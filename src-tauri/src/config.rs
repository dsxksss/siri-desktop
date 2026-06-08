use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Inference threads to use: up to 4, based on available CPU cores. More threads
/// speed up ASR/TTS noticeably on multi-core machines.
pub fn worker_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| (n.get() as i32).clamp(1, 4))
        .unwrap_or(2)
}

/// Top-level user configuration, loaded from `config.toml` (see repo root).
/// Every field has a sensible default so the app still runs without a config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Display name of the wake word, shown in the UI only. The tokens the
    /// detector actually matches live in `models/kws/keywords.txt`.
    pub wake_word: String,
    /// Launch automatically at login (release builds only).
    pub autostart: bool,
    pub models: Models,
    pub audio: Audio,
    /// Spoken name -> executable path / command. Used by the "open app" skill.
    pub apps: HashMap<String, String>,
    pub netease: Netease,
    pub llm: Llm,
    pub tts: Tts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Models {
    /// Base directory containing `kws/`, `asr/` and `vad/` subfolders.
    /// May be relative (resolved against the working dir / exe dir) or absolute.
    pub dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    /// VAD speech probability threshold (0..1). Higher = less sensitive.
    pub vad_threshold: f32,
    /// Trailing silence (seconds) that marks the end of an utterance.
    pub vad_min_silence: f32,
    /// Seconds to wait for speech after waking before giving up.
    pub listen_timeout: f32,
    /// Microphone name substring (case-insensitive). Empty = system default.
    pub input_device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Netease {
    /// "direct" = resolve song ids in-process; "service" = use a local
    /// NeteaseCloudMusicApi instance at `service_url`.
    pub search_api: String,
    pub service_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Llm {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Tts {
    /// Speak replies aloud (offline). Needs the TTS model in models/tts/.
    pub enabled: bool,
    /// Speaking speed multiplier (1.0 = normal).
    pub speed: f32,
}

impl Default for Tts {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Must match a line in models/kws/keywords.txt. The bundled file
            // ships with 你好问问 / 小爱同学 / 你好军哥 out of the box.
            wake_word: "你好问问".into(),
            autostart: false,
            models: Models::default(),
            audio: Audio::default(),
            apps: HashMap::new(),
            netease: Netease::default(),
            llm: Llm::default(),
            tts: Tts::default(),
        }
    }
}

impl Default for Models {
    fn default() -> Self {
        Self { dir: "models".into() }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            vad_threshold: 0.5,
            vad_min_silence: 0.4,
            listen_timeout: 8.0,
            input_device: String::new(),
        }
    }
}

impl Default for Netease {
    fn default() -> Self {
        Self {
            search_api: "direct".into(),
            service_url: "http://127.0.0.1:3000".into(),
        }
    }
}

impl Default for Llm {
    fn default() -> Self {
        Self {
            provider: "deepseek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            api_key: String::new(),
        }
    }
}

/// Canonical model file locations (see `scripts/fetch-models.ps1`, which
/// downloads the upstream archives and renames files into this layout).
pub struct KwsPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
    pub keywords: PathBuf,
}

pub struct AsrPaths {
    pub model: PathBuf,
    pub tokens: PathBuf,
}

pub struct TtsPaths {
    pub model: PathBuf,
    pub lexicon: PathBuf,
    pub tokens: PathBuf,
    pub dict_dir: PathBuf,
    /// Comma-joined text-normalization rule FSTs that exist, if any.
    pub rule_fsts: Option<String>,
}

impl Config {
    /// Resolve the models base directory, trying a few common locations so the
    /// app works both in `tauri dev` (cwd = src-tauri) and when installed.
    pub fn models_dir(&self) -> PathBuf {
        let raw = PathBuf::from(&self.models.dir);
        if raw.is_absolute() {
            return raw;
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join(&raw));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(&raw));
                candidates.push(dir.join("..").join(&raw));
            }
        }
        candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .unwrap_or(raw)
    }

    pub fn kws_paths(&self) -> KwsPaths {
        let d = self.models_dir().join("kws");
        KwsPaths {
            encoder: d.join("encoder.onnx"),
            decoder: d.join("decoder.onnx"),
            joiner: d.join("joiner.onnx"),
            tokens: d.join("tokens.txt"),
            keywords: d.join("keywords.txt"),
        }
    }

    pub fn asr_paths(&self) -> AsrPaths {
        let d = self.models_dir().join("asr");
        AsrPaths {
            model: d.join("model.onnx"),
            tokens: d.join("tokens.txt"),
        }
    }

    pub fn vad_model(&self) -> PathBuf {
        self.models_dir().join("vad").join("silero_vad.onnx")
    }

    pub fn tts_paths(&self) -> TtsPaths {
        let d = self.models_dir().join("tts");
        let fsts: Vec<String> = ["date.fst", "number.fst", "phone.fst"]
            .iter()
            .map(|f| d.join(f))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        TtsPaths {
            model: d.join("model.onnx"),
            lexicon: d.join("lexicon.txt"),
            tokens: d.join("tokens.txt"),
            dict_dir: d.join("dict"),
            rule_fsts: if fsts.is_empty() {
                None
            } else {
                Some(fsts.join(","))
            },
        }
    }
}

/// Load configuration: `config.toml` is the base and `config.local.toml`
/// (gitignored, for secrets) is layered on top, key by key. Missing keys fall
/// back to [`Config::default`].
pub fn load() -> Config {
    load_dotenv();

    let base = read_first("config.toml");
    let overlay = read_first("config.local.toml");
    let merged = match (base, overlay) {
        (Some(mut b), Some(o)) => {
            log::info!("loaded config.toml + config.local.toml");
            merge(&mut b, o);
            Some(b)
        }
        (Some(b), None) => {
            log::info!("loaded config.toml");
            Some(b)
        }
        (None, Some(o)) => {
            log::info!("loaded config.local.toml");
            Some(o)
        }
        (None, None) => None,
    };
    let mut cfg = match merged {
        Some(value) => value.try_into().unwrap_or_else(|e| {
            log::warn!("invalid config, using defaults: {e}");
            Config::default()
        }),
        None => {
            log::info!("no config file found, using defaults");
            Config::default()
        }
    };

    // The API key comes from .env / the environment unless set in a config file.
    if cfg.llm.api_key.is_empty() {
        for var in ["SIRI_LLM_API_KEY", "DEEPSEEK_API_KEY", "LLM_API_KEY"] {
            if let Ok(v) = std::env::var(var) {
                if !v.trim().is_empty() {
                    cfg.llm.api_key = v;
                    log::info!("LLM api key loaded from {var}");
                    break;
                }
            }
        }
    }

    cfg
}

/// Load `.env` so `DEEPSEEK_API_KEY` etc. become available. Searches up from the
/// working dir and also the config base dirs (covers the installed exe dir).
fn load_dotenv() {
    let _ = dotenvy::dotenv();
    for base in config_base_dirs() {
        let path = base.join(".env");
        if path.is_file() {
            let _ = dotenvy::from_path(&path);
        }
    }
}

/// Read and parse the first existing file named `name` across candidate dirs.
fn read_first(name: &str) -> Option<toml::Value> {
    for base in config_base_dirs() {
        let path = base.join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            match toml::from_str::<toml::Value>(&text) {
                Ok(v) => return Some(v),
                Err(e) => log::warn!("invalid config at {}: {e}", path.display()),
            }
        }
    }
    None
}

/// Deep-merge `overlay` into `base` (overlay wins for leaf values).
fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(bv) => merge(bv, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

/// Directories to search for config files, in priority order.
pub fn config_base_dirs() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd.clone());
        bases.push(cwd.join("..")); // dev: src-tauri -> repo root
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bases.push(dir.to_path_buf());
            bases.push(dir.join(".."));
        }
    }
    bases
}

/// Persist the chosen microphone into config.local.toml's `[audio].input_device`
/// so it survives restarts. Returns the file written.
pub fn persist_input_device(name: &str) -> std::io::Result<PathBuf> {
    let dirs = config_base_dirs();
    let dir = dirs
        .iter()
        .find(|d| d.join("config.toml").exists())
        .or_else(|| dirs.first())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    let path = dir.join("config.local.toml");

    let mut root: toml::value::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    let audio = root
        .entry("audio".to_string())
        .or_insert_with(|| toml::Value::Table(Default::default()));
    if let toml::Value::Table(t) = audio {
        t.insert(
            "input_device".to_string(),
            toml::Value::String(name.to_string()),
        );
    }
    let text = toml::to_string_pretty(&toml::Value::Table(root))
        .unwrap_or_else(|_| format!("[audio]\ninput_device = \"{name}\"\n"));
    std::fs::write(&path, text)?;
    Ok(path)
}
