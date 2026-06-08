use crate::config::Config;
use crate::events::emit_state;
use crate::{asr, audio, wake};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;

/// Control messages sent to the voice pipeline thread.
pub enum Control {
    /// Skip the wake word and start listening immediately (e.g. orb clicked).
    Listen,
    /// Cancel listening and return to wake mode.
    Cancel,
    /// Stop the pipeline and release the microphone (used when restarting).
    Shutdown,
}

/// Callback invoked with the final transcript. Wired to the intent dispatcher
/// in later phases; the assistant state is already set to `thinking` when called.
pub type OnText = Arc<dyn Fn(&AppHandle, String) + Send + Sync>;

/// Handle for talking to the running pipeline.
pub struct PipelineHandle {
    pub control: Sender<Control>,
    pub join: std::thread::JoinHandle<()>,
}

/// Spawn the voice pipeline on its own thread. Audio capture and all model
/// inference run here; the cpal stream is `!Send` so it must stay on this thread.
pub fn start(app: AppHandle, cfg: Arc<Config>, on_text: OnText) -> PipelineHandle {
    let (ctrl_tx, ctrl_rx) = unbounded::<Control>();
    let app_err = app.clone();
    let join = std::thread::Builder::new()
        .name("voice-pipeline".into())
        .spawn(move || {
            if let Err(e) = run(app, cfg, on_text, ctrl_rx) {
                log::error!("voice pipeline stopped: {e:#}");
                emit_state(&app_err, "error", Some(&friendly_error(&e)));
            }
        })
        .expect("failed to spawn voice pipeline thread");
    PipelineHandle {
        control: ctrl_tx,
        join,
    }
}

/// Turn a pipeline startup error into a short, actionable message for the orb.
/// The full technical error is already in the log; this is what the user sees.
fn friendly_error(e: &anyhow::Error) -> String {
    let s = e.to_string();
    if s.contains("model") && (s.contains(".onnx") || s.contains("model file")) {
        "缺少语音模型 · 请运行 scripts/fetch-models.ps1 下载后重启".to_string()
    } else if s.contains("input device") || s.contains("device") {
        "麦克风不可用 · 请检查录音设备或在托盘菜单重新选择".to_string()
    } else {
        s
    }
}

enum Mode {
    Wake,
    Listen { since: Instant },
}

fn run(
    app: AppHandle,
    cfg: Arc<Config>,
    on_text: OnText,
    ctrl: Receiver<Control>,
) -> anyhow::Result<()> {
    // Load models first so a missing-model error surfaces before we open the mic.
    let wake = wake::WakeWord::new(&cfg)?;
    let vad = asr::Vad::new(&cfg)?;
    let recognizer = asr::Recognizer::new(&cfg)?;

    // Warm up the onnx graphs on silence so the first real command isn't slow.
    let tw = std::time::Instant::now();
    let warm = vec![0.0f32; (audio::TARGET_RATE / 2) as usize];
    let _ = wake.accept(&warm);
    let _ = recognizer.transcribe(&warm);
    log::info!(
        "warmup {} ms (asr threads={})",
        tw.elapsed().as_millis(),
        crate::config::worker_threads()
    );

    let (tx, rx) = unbounded::<Vec<f32>>();
    let mic = audio::start_capture(tx, Some(cfg.audio.input_device.as_str()))?;
    let mut resampler = audio::Resampler::new(mic.device_rate);

    log::info!("voice pipeline ready; say the wake word to begin");
    emit_state(&app, "idle", None);

    let listen_timeout = Duration::from_secs_f32(cfg.audio.listen_timeout.max(2.0));
    let mut mode = Mode::Wake;

    loop {
        // Handle control messages (non-blocking).
        match ctrl.try_recv() {
            Ok(Control::Listen) => enter_listen(&app, &vad, &mut mode),
            Ok(Control::Cancel) => {
                if let Mode::Listen { .. } = mode {
                    log::info!("listening cancelled by user");
                    vad.reset();
                    mode = Mode::Wake;
                    emit_state(&app, "idle", None);
                }
            }
            Ok(Control::Shutdown) => break,
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        // Timeout keeps the control channel responsive even if audio stalls.
        let raw = match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(chunk) => chunk,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let chunk = resampler.process(&raw);
        if chunk.is_empty() {
            continue;
        }

        match &mode {
            Mode::Wake => {
                if let Some(keyword) = wake.accept(&chunk) {
                    log::info!("wake word detected: {keyword}");
                    enter_listen(&app, &vad, &mut mode);
                }
            }
            Mode::Listen { since } => {
                vad.accept(&chunk);
                if let Some(segment) = vad.pop_segment() {
                    let t0 = std::time::Instant::now();
                    let text = recognizer.transcribe(&segment);
                    log::info!(
                        "ASR {} ms ({:.1}s audio) -> {text}",
                        t0.elapsed().as_millis(),
                        segment.len() as f32 / audio::TARGET_RATE as f32
                    );
                    mode = Mode::Wake;
                    if text.is_empty() {
                        emit_state(&app, "idle", None);
                    } else {
                        emit_state(&app, "thinking", Some(&text));
                        on_text(&app, text);
                    }
                } else if since.elapsed() > listen_timeout {
                    log::info!("listen timed out with no speech");
                    vad.reset();
                    mode = Mode::Wake;
                    emit_state(&app, "idle", None);
                }
            }
        }
    }

    drop(mic);
    Ok(())
}

fn enter_listen(app: &AppHandle, vad: &asr::Vad, mode: &mut Mode) {
    vad.reset();
    *mode = Mode::Listen {
        since: Instant::now(),
    };
    emit_state(app, "listening", Some("聆听中…"));
}
