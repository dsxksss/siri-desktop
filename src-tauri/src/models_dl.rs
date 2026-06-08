//! In-app downloader for the offline models, mirroring `scripts/fetch-models.ps1`
//! but with live progress events for the settings UI. Downloads each archive with
//! `ureq` (byte-level progress), extracts with the Windows-bundled `tar.exe`, and
//! arranges files into the canonical `models/{kws,asr,vad,tts}/` layout.

use crate::config::Config;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

/// Event the settings window listens on for download progress.
pub const EVENT_PROGRESS: &str = "model://progress";

// Only one download may run at a time; `CANCEL` requests an early stop.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

// --- model manifest (URLs + extracted sub-dir names) -------------------------
const KWS_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01.tar.bz2";
const KWS_SUB: &str = "sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01";
const ASR_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2";
const ASR_SUB: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";
const VAD_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
const TTS_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-melo-tts-zh_en.tar.bz2";
const TTS_SUB: &str = "vits-melo-tts-zh_en";

/// Status of one model group, sent to the settings UI.
#[derive(Serialize, Clone)]
pub struct ModelGroup {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub approx_mb: u32,
    pub required: bool,
    pub installed: bool,
}

/// All model groups with their current install status.
pub fn groups(cfg: &Config) -> Vec<ModelGroup> {
    vec![
        ModelGroup {
            id: "kws",
            name: "唤醒词 (KWS)",
            desc: "离线唤醒词检测",
            approx_mb: 13,
            required: true,
            installed: cfg.kws_paths().encoder.exists(),
        },
        ModelGroup {
            id: "asr",
            name: "语音识别 (ASR)",
            desc: "SenseVoice 多语种识别",
            approx_mb: 230,
            required: true,
            installed: cfg.asr_paths().model.exists(),
        },
        ModelGroup {
            id: "vad",
            name: "静音检测 (VAD)",
            desc: "Silero 语音端点检测",
            approx_mb: 2,
            required: true,
            installed: cfg.vad_model().exists(),
        },
        ModelGroup {
            id: "tts",
            name: "语音合成 (TTS)",
            desc: "MeloTTS 朗读回复（可选）",
            approx_mb: 160,
            required: false,
            installed: cfg.tts_paths().model.exists(),
        },
    ]
}

#[derive(Clone, Serialize)]
struct Progress {
    /// Current group id, or "" when idle/done.
    group: String,
    group_name: String,
    group_index: u32,
    group_count: u32,
    /// downloading | extracting | arranging | done | error | cancelled
    phase: String,
    received: u64,
    total: u64,
    message: Option<String>,
}

fn emit(
    app: &AppHandle,
    group: &str,
    group_name: &str,
    index: u32,
    count: u32,
    phase: &str,
    received: u64,
    total: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        EVENT_PROGRESS,
        Progress {
            group: group.to_string(),
            group_name: group_name.to_string(),
            group_index: index,
            group_count: count,
            phase: phase.to_string(),
            received,
            total,
            message,
        },
    );
}

/// Request cancellation of an in-flight download.
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

fn check_cancel() -> Result<()> {
    if CANCEL.load(Ordering::SeqCst) {
        bail!("已取消");
    }
    Ok(())
}

/// Kick off downloading every missing model group on a background thread. No-op
/// if a download is already running. Emits [`EVENT_PROGRESS`] throughout and
/// reloads the voice pipeline once required models land.
pub fn start_download(app: AppHandle, cfg: Arc<Config>) {
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        log::info!("model download already in progress");
        return;
    }
    CANCEL.store(false, Ordering::SeqCst);
    std::thread::Builder::new()
        .name("model-download".into())
        .spawn(move || {
            let result = run(&app, &cfg);
            DOWNLOADING.store(false, Ordering::SeqCst);
            match result {
                Ok(pipeline_changed) => {
                    emit(&app, "", "", 0, 0, "done", 0, 0, Some("模型已就绪".into()));
                    if pipeline_changed {
                        // Reload so wake word / ASR start working without an app restart.
                        crate::reload_pipeline_models(&app);
                    }
                }
                Err(e) => {
                    let cancelled = CANCEL.load(Ordering::SeqCst);
                    log::warn!("model download stopped: {e:#}");
                    emit(
                        &app,
                        "",
                        "",
                        0,
                        0,
                        if cancelled { "cancelled" } else { "error" },
                        0,
                        0,
                        Some(format!("{e}")),
                    );
                }
            }
        })
        .expect("failed to spawn model-download thread");
}

fn run(app: &AppHandle, cfg: &Config) -> Result<bool> {
    let missing: Vec<ModelGroup> = groups(cfg).into_iter().filter(|g| !g.installed).collect();
    if missing.is_empty() {
        return Ok(false);
    }
    let count = missing.len() as u32;
    let models = cfg.models_dir();
    let tmp = std::env::temp_dir().join("siri-models");
    fs::create_dir_all(&tmp)?;

    let mut pipeline_changed = false;
    for (i, g) in missing.iter().enumerate() {
        check_cancel()?;
        let idx = i as u32 + 1;
        match g.id {
            "kws" => {
                install_kws(app, &tmp, &models, idx, count, g.name)?;
                pipeline_changed = true;
            }
            "asr" => {
                install_asr(app, &tmp, &models, idx, count, g.name)?;
                pipeline_changed = true;
            }
            "vad" => {
                install_vad(app, &models, idx, count, g.name)?;
                pipeline_changed = true;
            }
            "tts" => install_tts(app, &tmp, &models, idx, count, g.name)?,
            _ => {}
        }
    }
    Ok(pipeline_changed)
}

// --- per-group installers ----------------------------------------------------

fn install_kws(
    app: &AppHandle,
    tmp: &Path,
    models: &Path,
    idx: u32,
    count: u32,
    name: &str,
) -> Result<()> {
    let archive = tmp.join("kws.tar.bz2");
    fetch(app, KWS_URL, &archive, "kws", name, idx, count)?;
    extract(app, &archive, tmp, "kws", name, idx, count)?;
    emit(app, "kws", name, idx, count, "arranging", 0, 0, None);
    let src = tmp.join(KWS_SUB);
    let dst = models.join("kws");
    fs::create_dir_all(&dst)?;
    copy_first(&src, "encoder", &dst.join("encoder.onnx"))?;
    copy_first(&src, "decoder", &dst.join("decoder.onnx"))?;
    copy_first(&src, "joiner", &dst.join("joiner.onnx"))?;
    fs::copy(src.join("tokens.txt"), dst.join("tokens.txt"))?;
    // Don't clobber a customized wake-word list if one already exists.
    let keywords = dst.join("keywords.txt");
    if !keywords.exists() {
        fs::copy(src.join("keywords.txt"), keywords)?;
    }
    Ok(())
}

fn install_asr(
    app: &AppHandle,
    tmp: &Path,
    models: &Path,
    idx: u32,
    count: u32,
    name: &str,
) -> Result<()> {
    let archive = tmp.join("asr.tar.bz2");
    fetch(app, ASR_URL, &archive, "asr", name, idx, count)?;
    extract(app, &archive, tmp, "asr", name, idx, count)?;
    emit(app, "asr", name, idx, count, "arranging", 0, 0, None);
    let src = tmp.join(ASR_SUB);
    let dst = models.join("asr");
    fs::create_dir_all(&dst)?;
    // int8 model: ~5x smaller, accuracy is still excellent for commands.
    fs::copy(src.join("model.int8.onnx"), dst.join("model.onnx"))?;
    fs::copy(src.join("tokens.txt"), dst.join("tokens.txt"))?;
    Ok(())
}

fn install_vad(app: &AppHandle, models: &Path, idx: u32, count: u32, name: &str) -> Result<()> {
    let dst = models.join("vad");
    fs::create_dir_all(&dst)?;
    // A single .onnx file: download straight to its final location.
    fetch(app, VAD_URL, &dst.join("silero_vad.onnx"), "vad", name, idx, count)?;
    Ok(())
}

fn install_tts(
    app: &AppHandle,
    tmp: &Path,
    models: &Path,
    idx: u32,
    count: u32,
    name: &str,
) -> Result<()> {
    let archive = tmp.join("tts.tar.bz2");
    fetch(app, TTS_URL, &archive, "tts", name, idx, count)?;
    extract(app, &archive, tmp, "tts", name, idx, count)?;
    emit(app, "tts", name, idx, count, "arranging", 0, 0, None);
    let src = tmp.join(TTS_SUB);
    let dst = models.join("tts");
    fs::create_dir_all(&dst)?;
    fs::copy(src.join("model.onnx"), dst.join("model.onnx"))?;
    fs::copy(src.join("lexicon.txt"), dst.join("lexicon.txt"))?;
    fs::copy(src.join("tokens.txt"), dst.join("tokens.txt"))?;
    let dict = src.join("dict");
    if dict.exists() {
        copy_dir_all(&dict, &dst.join("dict"))?;
    }
    // Text-normalization rule FSTs (date/number/phone/…).
    for entry in fs::read_dir(&src)? {
        let p = entry?.path();
        if p.extension().and_then(|e| e.to_str()) == Some("fst") {
            if let Some(fname) = p.file_name() {
                fs::copy(&p, dst.join(fname))?;
            }
        }
    }
    Ok(())
}

// --- shared helpers ----------------------------------------------------------

/// Stream `url` to `dest` (via a `.part` temp file), emitting download progress.
/// Reuses a fully downloaded `dest` as a cache, so re-runs after a failure are cheap.
fn fetch(
    app: &AppHandle,
    url: &str,
    dest: &Path,
    group: &str,
    name: &str,
    idx: u32,
    count: u32,
) -> Result<()> {
    if dest.exists() && fs::metadata(dest)?.len() > 0 {
        log::info!("cached: {}", dest.display());
        return Ok(());
    }
    log::info!("downloading {url}");
    let resp = ureq::get(url)
        .call()
        .map_err(|e| anyhow!("下载失败：{e}"))?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let part = dest.with_extension("part");
    let mut file = fs::File::create(&part)?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut received: u64 = 0;
    let mut last = Instant::now();
    emit(app, group, name, idx, count, "downloading", 0, total, None);
    loop {
        check_cancel()?;
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        received += n as u64;
        if last.elapsed().as_millis() >= 100 {
            emit(app, group, name, idx, count, "downloading", received, total, None);
            last = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    fs::rename(&part, dest)?;
    emit(app, group, name, idx, count, "downloading", received, total.max(received), None);
    Ok(())
}

/// Extract a `.tar.bz2` with the Windows-bundled bsdtar (auto-detects bzip2).
fn extract(
    app: &AppHandle,
    archive: &Path,
    dest: &Path,
    group: &str,
    name: &str,
    idx: u32,
    count: u32,
) -> Result<()> {
    emit(app, group, name, idx, count, "extracting", 0, 0, None);
    let mut tar = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()));
    tar.push("System32");
    tar.push("tar.exe");
    let tar = if tar.exists() {
        tar
    } else {
        PathBuf::from("tar")
    };
    let status = std::process::Command::new(tar)
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .map_err(|e| anyhow!("无法运行 tar：{e}"))?;
    if !status.success() {
        bail!("解压失败：{}", archive.display());
    }
    Ok(())
}

/// Copy the first `*.onnx` whose name contains `needle` (skipping int8 variants).
fn copy_first(dir: &Path, needle: &str, dest: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if !p.is_file() {
            continue;
        }
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.contains(needle) && name.ends_with(".onnx") && !name.contains("int8") {
            fs::copy(&p, dest)?;
            return Ok(());
        }
    }
    bail!("缺少 {needle} 模型文件")
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
