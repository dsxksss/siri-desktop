use crate::audio::TARGET_RATE;
use crate::config::Config;
use anyhow::{anyhow, Result};
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};

/// Offline wake-word detector backed by a sherpa-onnx zipformer KWS model.
pub struct WakeWord {
    spotter: KeywordSpotter,
    stream: OnlineStream,
}

impl WakeWord {
    pub fn new(cfg: &Config) -> Result<Self> {
        let k = cfg.kws_paths();
        for p in [&k.encoder, &k.decoder, &k.joiner, &k.tokens, &k.keywords] {
            if !p.exists() {
                return Err(anyhow!("missing KWS model file: {}", p.display()));
            }
        }

        let mut config = KeywordSpotterConfig::default();
        config.model_config.transducer.encoder = Some(path(&k.encoder));
        config.model_config.transducer.decoder = Some(path(&k.decoder));
        config.model_config.transducer.joiner = Some(path(&k.joiner));
        config.model_config.tokens = Some(path(&k.tokens));
        config.model_config.provider = Some("cpu".into());
        config.model_config.num_threads = 1;
        config.keywords_file = Some(path(&k.keywords));

        let spotter = KeywordSpotter::create(&config)
            .ok_or_else(|| anyhow!("failed to create KeywordSpotter (check model files)"))?;
        let stream = spotter.create_stream();
        Ok(Self { spotter, stream })
    }

    /// Feed 16 kHz mono samples. Returns the matched keyword when one fires.
    pub fn accept(&self, samples: &[f32]) -> Option<String> {
        self.stream.accept_waveform(TARGET_RATE as i32, samples);
        let mut hit = None;
        while self.spotter.is_ready(&self.stream) {
            self.spotter.decode(&self.stream);
            if let Some(result) = self.spotter.get_result(&self.stream) {
                if !result.keyword.is_empty() {
                    hit = Some(result.keyword.clone());
                    // clear decoder state so the next utterance starts fresh
                    self.spotter.reset(&self.stream);
                }
            }
        }
        hit
    }
}

fn path(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}
