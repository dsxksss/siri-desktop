use crate::audio::TARGET_RATE;
use crate::config::Config;
use anyhow::{anyhow, Result};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, VadModelConfig,
    VoiceActivityDetector,
};

/// Offline speech recognizer backed by a sherpa-onnx SenseVoice model.
pub struct Recognizer {
    inner: OfflineRecognizer,
}

impl Recognizer {
    pub fn new(cfg: &Config) -> Result<Self> {
        let a = cfg.asr_paths();
        for p in [&a.model, &a.tokens] {
            if !p.exists() {
                return Err(anyhow!("missing ASR model file: {}", p.display()));
            }
        }

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(path(&a.model)),
            language: Some("auto".into()),
            use_itn: true,
        };
        config.model_config.tokens = Some(path(&a.tokens));
        config.model_config.provider = Some("cpu".into());
        config.model_config.num_threads = crate::config::worker_threads();

        let inner = OfflineRecognizer::create(&config)
            .ok_or_else(|| anyhow!("failed to create OfflineRecognizer (check model files)"))?;
        Ok(Self { inner })
    }

    /// Transcribe a 16 kHz mono utterance into text.
    pub fn transcribe(&self, samples: &[f32]) -> String {
        let stream = self.inner.create_stream();
        stream.accept_waveform(TARGET_RATE as i32, samples);
        self.inner.decode(&stream);
        match stream.get_result() {
            Some(r) => r.text.trim().to_string(),
            None => String::new(),
        }
    }
}

/// Silero voice-activity detector used to segment an utterance after wake.
pub struct Vad {
    inner: VoiceActivityDetector,
}

impl Vad {
    pub fn new(cfg: &Config) -> Result<Self> {
        let model = cfg.vad_model();
        if !model.exists() {
            return Err(anyhow!("missing VAD model: {}", model.display()));
        }

        let mut config = VadModelConfig::default();
        config.silero_vad.model = Some(path(&model));
        config.silero_vad.threshold = cfg.audio.vad_threshold;
        config.silero_vad.min_silence_duration = cfg.audio.vad_min_silence;
        config.silero_vad.min_speech_duration = 0.25;
        config.silero_vad.window_size = 512;
        config.sample_rate = TARGET_RATE as i32;

        // 30 s rolling buffer is plenty for a single command.
        let inner = VoiceActivityDetector::create(&config, 30.0)
            .ok_or_else(|| anyhow!("failed to create VoiceActivityDetector"))?;
        Ok(Self { inner })
    }

    pub fn accept(&self, samples: &[f32]) {
        self.inner.accept_waveform(samples);
    }

    /// True while speech is currently being detected.
    #[allow(dead_code)]
    pub fn detected(&self) -> bool {
        self.inner.detected()
    }

    /// Return the next completed speech segment's samples, if any.
    pub fn pop_segment(&self) -> Option<Vec<f32>> {
        if self.inner.is_empty() {
            return None;
        }
        let segment = self.inner.front()?;
        let samples = segment.samples().to_vec();
        self.inner.pop();
        Some(samples)
    }

    pub fn reset(&self) {
        self.inner.reset();
    }
}

fn path(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}
