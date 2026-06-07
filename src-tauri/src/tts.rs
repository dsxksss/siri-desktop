//! Offline text-to-speech (sherpa-onnx VITS / MeloTTS) with playback via rodio.
use crate::config::Config;
use anyhow::{anyhow, Result};
use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsVitsModelConfig,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A loaded offline TTS engine.
pub struct Tts {
    inner: OfflineTts,
    speed: f32,
}

impl Tts {
    pub fn new(cfg: &Config) -> Result<Self> {
        let p = cfg.tts_paths();
        for f in [&p.model, &p.lexicon, &p.tokens] {
            if !f.exists() {
                return Err(anyhow!("missing TTS model file: {}", f.display()));
            }
        }
        let vits = OfflineTtsVitsModelConfig {
            model: Some(path(&p.model)),
            lexicon: Some(path(&p.lexicon)),
            tokens: Some(path(&p.tokens)),
            dict_dir: if p.dict_dir.exists() {
                Some(path(&p.dict_dir))
            } else {
                None
            },
            ..Default::default()
        };
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                vits,
                num_threads: crate::config::worker_threads(),
                provider: Some("cpu".into()),
                ..Default::default()
            },
            rule_fsts: p.rule_fsts.clone(),
            // 1 = emit audio after each sentence, so streaming playback starts ASAP.
            max_num_sentences: 1,
            ..Default::default()
        };
        let inner = OfflineTts::create(&config)
            .ok_or_else(|| anyhow!("failed to create OfflineTts (check model files)"))?;
        Ok(Self {
            inner,
            speed: cfg.tts.speed.max(0.3),
        })
    }

    /// Generate speech for `text` and play it as it streams in: each generated
    /// chunk is queued to the audio sink immediately, so the first words start
    /// playing well before the whole clip is synthesized. Blocks until done.
    pub fn say(&self, text: &str) -> Result<()> {
        let rate = self.inner.sample_rate() as u32;
        let (_stream, handle) =
            OutputStream::try_default().map_err(|e| anyhow!("打开音频输出失败：{e}"))?;
        let sink = Arc::new(Sink::try_new(&handle).map_err(|e| anyhow!("创建播放器失败：{e}"))?);

        let t0 = std::time::Instant::now();
        let sink_cb = sink.clone();
        let logged = Arc::new(AtomicBool::new(false));
        let cb = move |chunk: &[f32], _progress: f32| -> bool {
            if !chunk.is_empty() {
                if !logged.swap(true, Ordering::Relaxed) {
                    log::info!("TTS first audio {} ms", t0.elapsed().as_millis());
                }
                sink_cb.append(SamplesBuffer::new(1, rate, chunk.to_vec()));
            }
            true // keep generating
        };

        let gc = GenerationConfig {
            speed: self.speed,
            sid: 0,
            ..Default::default()
        };
        let audio = self
            .inner
            .generate_with_config(text, &gc, Some(cb))
            .ok_or_else(|| anyhow!("TTS 生成失败"))?;
        log::info!(
            "TTS total {} ms ({} samples)",
            t0.elapsed().as_millis(),
            audio.samples().len()
        );
        sink.sleep_until_end();
        Ok(())
    }

    /// One tiny generation to warm the onnx graph so the first reply is fast.
    fn warmup(&self) {
        let gc = GenerationConfig {
            speed: self.speed,
            sid: 0,
            ..Default::default()
        };
        let _ = self
            .inner
            .generate_with_config::<fn(&[f32], f32) -> bool>("你好", &gc, None);
    }
}

/// Loads the TTS engine in the background and serializes playback so replies
/// never overlap.
#[derive(Clone)]
pub struct TtsHandle {
    inner: Arc<Mutex<Option<Tts>>>,
    enabled: bool,
}

impl TtsHandle {
    /// A handle that never speaks.
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            enabled: false,
        }
    }

    /// Begin loading the model on a background thread (non-blocking).
    pub fn spawn_load(cfg: Arc<Config>) -> Self {
        let inner = Arc::new(Mutex::new(None));
        let slot = inner.clone();
        std::thread::Builder::new()
            .name("tts-load".into())
            .spawn(move || match Tts::new(&cfg) {
                Ok(t) => {
                    t.warmup();
                    *slot.lock().unwrap() = Some(t);
                    log::info!("TTS ready");
                }
                Err(e) => log::warn!("TTS disabled: {e:#}"),
            })
            .ok();
        Self {
            inner,
            enabled: true,
        }
    }

    /// Speak `text` if the engine is loaded; no-op otherwise. Blocks until done.
    pub fn say(&self, text: &str) {
        if !self.enabled || text.trim().is_empty() {
            return;
        }
        let guard = self.inner.lock().unwrap();
        if let Some(tts) = guard.as_ref() {
            if let Err(e) = tts.say(text) {
                log::warn!("TTS playback failed: {e:#}");
            }
        }
    }
}

fn path(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exercises model load + generation (no playback). Skips if model absent.
    #[test]
    fn generate_nonempty() {
        let cfg = Config::default();
        let tts = match Tts::new(&cfg) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("skip (no TTS model): {e:#}");
                return;
            }
        };
        let gc = GenerationConfig {
            speed: 1.0,
            sid: 0,
            ..Default::default()
        };
        let audio = tts
            .inner
            .generate_with_config::<fn(&[f32], f32) -> bool>("你好，我是小问。", &gc, None)
            .expect("generate");
        println!(
            "tts samples={} rate={}",
            audio.samples().len(),
            audio.sample_rate()
        );
        assert!(!audio.samples().is_empty());
        assert!(audio.sample_rate() > 0);
    }
}
