use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use crossbeam_channel::Sender;

/// Sample rate expected by the sherpa-onnx KWS / ASR / VAD models.
pub const TARGET_RATE: u32 = 16_000;

/// Keeps the cpal input stream alive. Dropping this stops capture.
/// Note: `cpal::Stream` is `!Send` on Windows, so this must stay on the thread
/// that created it (the voice pipeline thread).
pub struct MicStream {
    #[allow(dead_code)]
    stream: cpal::Stream,
    pub device_rate: u32,
}

/// List available input device names (for the tray "选择麦克风" menu / settings).
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    if let Ok(devices) = host.input_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                names.push(name);
            }
        }
    }
    names
}

/// Pick an input device: by name substring if `preferred` is set, else the
/// system default. Errors list the available devices to guide the user.
fn select_device(host: &cpal::Host, preferred: Option<&str>) -> Result<cpal::Device> {
    if let Some(want) = preferred.filter(|s| !s.trim().is_empty()) {
        let want_lc = want.to_lowercase();
        if let Ok(devices) = host.input_devices() {
            for d in devices {
                if let Ok(name) = d.name() {
                    if name.to_lowercase().contains(&want_lc) {
                        log::info!("using input device: {name}");
                        return Ok(d);
                    }
                }
            }
        }
        log::warn!("input device matching '{want}' not found; falling back to default");
    }
    host.default_input_device().ok_or_else(|| {
        anyhow!(
            "未找到麦克风。可用输入设备: [{}]。请在托盘菜单“选择麦克风”里选择，或在 config.toml 的 [audio] input_device 指定",
            list_input_devices().join(", ")
        )
    })
}

/// Open an input device and start streaming mono `f32` samples (at the device's
/// native rate) into `tx`. Resample to 16 kHz with [`Resampler`].
pub fn start_capture(tx: Sender<Vec<f32>>, preferred: Option<&str>) -> Result<MicStream> {
    let host = cpal::default_host();
    log::info!("available input devices: {:?}", list_input_devices());
    let device = select_device(&host, preferred)?;
    let supported = device.default_input_config()?;
    let device_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    log::info!(
        "microphone: {} Hz, {} channel(s), {:?}",
        device_rate,
        channels,
        sample_format
    );

    let err_fn = |e| log::error!("audio input stream error: {e}");

    let stream = match sample_format {
        SampleFormat::F32 => {
            let tx = tx.clone();
            device.build_input_stream(
                &config,
                move |data: &[f32], _| send_mono(data, channels, &tx),
                err_fn,
                None,
            )?
        }
        SampleFormat::I16 => {
            let tx = tx.clone();
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|s| *s as f32 / i16::MAX as f32)
                        .collect();
                    send_mono(&f, channels, &tx);
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::U16 => {
            let tx = tx.clone();
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|s| (*s as f32 - 32768.0) / 32768.0)
                        .collect();
                    send_mono(&f, channels, &tx);
                },
                err_fn,
                None,
            )?
        }
        other => return Err(anyhow!("unsupported sample format: {other:?}")),
    };

    stream.play()?;
    Ok(MicStream {
        stream,
        device_rate,
    })
}

/// Downmix interleaved frames to mono and forward to the worker thread.
fn send_mono(data: &[f32], channels: usize, tx: &Sender<Vec<f32>>) {
    let mono: Vec<f32> = if channels <= 1 {
        data.to_vec()
    } else {
        data.chunks(channels)
            .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
            .collect()
    };
    // Drop samples if the consumer is gone; never block the audio callback.
    let _ = tx.send(mono);
}

/// Streaming linear resampler from the device rate to 16 kHz mono.
///
/// Linear interpolation keeps continuity across chunks via a carried tail
/// sample and a fractional read position. It is intentionally simple and
/// dependency-free; good enough for the robust KWS/ASR models. Swap in a
/// polyphase/sinc resampler later if recognition quality needs it.
pub struct Resampler {
    src_rate: u32,
    step: f64, // input samples advanced per output sample
    pos: f64,  // next read position (index space of the current chunk; <0 = tail)
    last: f32, // last input sample of the previous chunk (index -1)
}

impl Resampler {
    pub fn new(src_rate: u32) -> Self {
        Self {
            src_rate,
            step: src_rate as f64 / TARGET_RATE as f64,
            pos: 0.0,
            last: 0.0,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.src_rate == TARGET_RATE {
            return input.to_vec();
        }
        if input.is_empty() {
            return Vec::new();
        }
        let n = input.len();
        let mut out = Vec::with_capacity(((n as f64) / self.step) as usize + 2);
        let mut p = self.pos;
        loop {
            let i = p.floor();
            let idx = i as isize;
            // need input[idx] and input[idx + 1]; stop if the right sample is
            // not in this chunk yet (carry position to the next chunk).
            if idx + 1 >= n as isize {
                break;
            }
            let a = if idx < 0 { self.last } else { input[idx as usize] };
            let b = input[(idx + 1) as usize];
            let frac = (p - i) as f32;
            out.push(a + (b - a) * frac);
            p += self.step;
        }
        self.last = input[n - 1];
        self.pos = p - n as f64;
        out
    }
}
