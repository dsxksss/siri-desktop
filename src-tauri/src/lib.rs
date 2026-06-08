mod asr;
mod audio;
mod config;
mod events;
mod intent;
mod models_dl;
mod pipeline;
mod skills;
mod tts;
mod wake;

use events::emit_state;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

/// The current interactive area of the orb, in window-local logical pixels,
/// reported by the frontend. The cursor-watch loop makes the (otherwise fully
/// transparent) window click-through everywhere outside this box, so the empty
/// area around the island stops blocking clicks to whatever is behind it.
#[derive(Default)]
struct HitArea(Mutex<Option<(i32, i32, i32, i32)>>);

/// Owns the running voice pipeline and can restart it (e.g. after the user picks
/// a different microphone).
struct VoiceManager {
    on_text: pipeline::OnText,
    current: Mutex<Option<pipeline::PipelineHandle>>,
}

impl VoiceManager {
    fn listen(&self) {
        if let Some(h) = self.current.lock().unwrap().as_ref() {
            let _ = h.control.send(pipeline::Control::Listen);
        }
    }

    fn cancel(&self) {
        if let Some(h) = self.current.lock().unwrap().as_ref() {
            let _ = h.control.send(pipeline::Control::Cancel);
        }
    }

    /// Stop the current pipeline (releasing the mic), reload config, restart.
    fn restart(&self, app: &AppHandle) {
        let mut guard = self.current.lock().unwrap();
        if let Some(h) = guard.take() {
            let _ = h.control.send(pipeline::Control::Shutdown);
            let _ = h.join.join();
        }
        let cfg = Arc::new(config::load());
        let on_wake: pipeline::OnWake = Arc::new(|| {});
        *guard = Some(pipeline::start(app.clone(), cfg, self.on_text.clone(), on_wake));
    }
}

/// Stop and restart the voice pipeline with freshly-loaded config/models. Used
/// after an in-app model download so wake word / ASR start working immediately.
pub(crate) fn reload_pipeline_models(app: &AppHandle) {
    if let Some(mgr) = app.try_state::<VoiceManager>() {
        mgr.restart(app);
    }
}

/// Orb clicked: start listening immediately, skipping the wake word.
#[tauri::command]
fn manual_listen(mgr: tauri::State<VoiceManager>) {
    mgr.listen();
}

/// Stop button clicked: cancel listening and return to wake mode.
#[tauri::command]
fn cancel_listen(mgr: tauri::State<VoiceManager>) {
    mgr.cancel();
}

/// Status of every offline model group (for the settings "模型管理" tab).
#[tauri::command]
fn model_groups() -> Vec<models_dl::ModelGroup> {
    models_dl::groups(&config::load())
}

/// Start downloading any missing model groups; progress streams over the
/// `model://progress` event. No-op if a download is already running.
#[tauri::command]
fn download_models(app: AppHandle) {
    models_dl::start_download(app, Arc::new(config::load()));
}

/// Request cancellation of an in-flight model download.
#[tauri::command]
fn cancel_model_download() {
    models_dl::cancel();
}

/// Frontend reports the orb's current interactive box (logical px, window-local).
#[tauri::command]
fn set_hit_rect(x: i32, y: i32, w: i32, h: i32, hit: tauri::State<HitArea>) {
    *hit.0.lock().unwrap() = Some((x, y, w, h));
}

/// Poll the global cursor and make the main window click-through whenever the
/// cursor is outside the reported interactive box. Runs on its own thread.
fn spawn_cursor_watch(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    std::thread::Builder::new()
        .name("cursor-watch".into())
        .spawn(move || {
            // Start interactive until the frontend reports a box.
            let mut ignoring = false;
            let _ = win.set_ignore_cursor_events(false);
            loop {
                std::thread::sleep(Duration::from_millis(80));
                let rect = *handle.state::<HitArea>().0.lock().unwrap();
                let Some((rx, ry, rw, rh)) = rect else { continue };
                let (Ok(cursor), Ok(pos)) = (win.cursor_position(), win.inner_position()) else {
                    continue;
                };
                let scale = win.scale_factor().unwrap_or(1.0);
                // Interactive box in physical/global coordinates.
                let left = pos.x as f64 + rx as f64 * scale;
                let top = pos.y as f64 + ry as f64 * scale;
                let right = left + rw as f64 * scale;
                let bottom = top + rh as f64 * scale;
                let inside =
                    cursor.x >= left && cursor.x <= right && cursor.y >= top && cursor.y <= bottom;
                let want_ignore = !inside;
                if want_ignore != ignoring {
                    let _ = win.set_ignore_cursor_events(want_ignore);
                    ignoring = want_ignore;
                }
            }
        })
        .expect("failed to spawn cursor-watch thread");
}

/// Debug helper: feed typed text into the exact same path a spoken command takes
/// (skip the mic/ASR, reuse the pipeline's `on_text` dispatch). Lets you test
/// intents/skills/LLM without speaking. Wired to the orb's hidden text input.
#[tauri::command]
fn simulate_text(text: String, app: AppHandle, mgr: tauri::State<VoiceManager>) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    log::info!("simulate_text (debug): {text}");
    // Mirror pipeline.rs: show the transcript as `thinking`, then dispatch.
    emit_state(&app, "thinking", Some(&text));
    // Run off the IPC thread; the LLM fallback can block for seconds.
    let on_text = mgr.on_text.clone();
    std::thread::spawn(move || on_text(&app, text));
}

/// Available microphone names (for a future settings UI / debugging).
#[tauri::command]
fn list_microphones() -> Vec<String> {
    audio::list_input_devices()
}

#[tauri::command]
fn get_config() -> config::Config {
    config::load()
}

#[tauri::command]
fn save_config(cfg: config::Config, app: tauri::AppHandle) -> Result<(), String> {
    let text = toml::to_string_pretty(&cfg)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;
    
    let dirs = config::config_base_dirs();
    let dir = dirs
        .iter()
        .find(|d| d.join("config.toml").exists())
        .or_else(|| dirs.first())
        .cloned()
        .ok_or_else(|| "Could not find config directory".to_string())?;
    
    let path = dir.join("config.local.toml");
    std::fs::write(&path, text)
        .map_err(|e| format!("Failed to write config file: {e}"))?;

    // Restart VoiceManager to apply new config
    if let Some(mgr) = app.try_state::<VoiceManager>() {
        mgr.restart(&app);
    }
    
    // Manage autostart dynamically (on release builds)
    #[cfg(not(debug_assertions))]
    {
        use tauri_plugin_autostart::ManagerExt;
        if let Ok(autostart_mgr) = app.autolaunch() {
            if cfg.autostart {
                let _ = autostart_mgr.enable();
            } else {
                let _ = autostart_mgr.disable();
            }
        }
    }
    
    Ok(())
}

/// Show the first-run setup wizard, creating its window if needed.
fn open_onboarding(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("onboarding") {
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        let _ = tauri::WebviewWindowBuilder::new(
            app,
            "onboarding",
            tauri::WebviewUrl::App("onboarding.html".into()),
        )
        .title("Siri Desktop 设置向导")
        .inner_size(780.0, 560.0)
        .min_inner_size(680.0, 480.0)
        .resizable(true)
        .center()
        .build();
    }
}

/// Centered horizontally at the top of the primary monitor.
fn position_top_center(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(Some(monitor)) = window.primary_monitor() {
            let screen = monitor.size();
            if let Ok(win) = window.outer_size() {
                let x = (screen.width as i32 - win.width as i32) / 2;
                let y = 0; // flush to the top edge of screen
                log::info!("positioning window: screen={:?}, win={:?}, pos=({},{})", screen, win, x, y);
                let _ = window.set_position(PhysicalPosition::new(x.max(0), y));
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // single-instance must be registered first; focus the orb if relaunched
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(HitArea::default())
        .invoke_handler(tauri::generate_handler![manual_listen, cancel_listen, simulate_text, list_microphones, get_config, save_config, model_groups, download_models, cancel_model_download, set_hit_rect])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // ---- system tray ----
            let show = MenuItem::with_id(app, "show", "显示 / 隐藏", true, None::<&str>)?;
            let wizard_item = MenuItem::with_id(app, "onboarding", "设置向导", true, None::<&str>)?;
            let config_item = MenuItem::with_id(app, "config", "设置", true, None::<&str>)?;

            // microphone picker submenu (ids are "mic::<name>", "" = default)
            let mut mic_items = Vec::new();
            mic_items.push(MenuItem::with_id(
                app,
                "mic::",
                "（系统默认）",
                true,
                None::<&str>,
            )?);
            for name in audio::list_input_devices() {
                mic_items.push(MenuItem::with_id(
                    app,
                    format!("mic::{name}"),
                    name.as_str(),
                    true,
                    None::<&str>,
                )?);
            }
            let mic_refs: Vec<&dyn tauri::menu::IsMenuItem<_>> = mic_items
                .iter()
                .map(|m| m as &dyn tauri::menu::IsMenuItem<_>)
                .collect();
            let mic_menu = Submenu::with_items(app, "选择麦克风", true, &mic_refs)?;

            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &mic_menu, &wizard_item, &config_item, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Siri Desktop")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id.as_ref();
                    if let Some(name) = id.strip_prefix("mic::") {
                        let name = name.to_string();
                        match config::persist_input_device(&name) {
                            Ok(p) => log::info!("microphone set to '{name}' ({})", p.display()),
                            Err(e) => log::warn!("failed to persist microphone: {e}"),
                        }
                        if let Some(mgr) = app.try_state::<VoiceManager>() {
                            mgr.restart(app);
                        }
                        let label = if name.is_empty() {
                            "系统默认".to_string()
                        } else {
                            name
                        };
                        emit_state(app, "acting", Some(&format!("已切换麦克风：{label}")));
                        let app2 = app.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(2500));
                            emit_state(&app2, "idle", None);
                        });
                        return;
                    }
                    match id {
                        "quit" => app.exit(0),
                        "onboarding" => open_onboarding(app),
                        "config" => {
                            if let Some(w) = app.get_webview_window("settings") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            } else {
                                let _ = tauri::WebviewWindowBuilder::new(
                                    app,
                                    "settings",
                                    tauri::WebviewUrl::App("settings.html".into()),
                                )
                                .title("Siri Desktop 设置")
                                .inner_size(680.0, 580.0)
                                .resizable(true)
                                .build();
                            }
                        }
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                if w.is_visible().unwrap_or(false) {
                                    let _ = w.hide();
                                } else {
                                    let _ = w.show();
                                    let _ = w.set_focus();
                                }
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // ---- debug hotkey: Ctrl+Shift+K summons the text-input on the orb ----
            // Lets you type a command (instead of speaking) when a mic isn't handy.
            {
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
                let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyK);
                let hk_handle = app.handle().clone();
                if let Err(e) = app.global_shortcut().on_shortcut(shortcut, move |_app, _scut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if let Some(w) = hk_handle.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    // Tell the frontend to reveal & focus the debug input.
                    let _ = hk_handle.emit("debug://toggle-input", ());
                }) {
                    log::warn!("failed to register debug hotkey (Ctrl+Shift+K): {e}");
                }
            }

            // ---- floating ball placement ----
            let handle = app.handle().clone();
            position_top_center(&handle);
            emit_state(&handle, "idle", None);

            // Make the transparent area around the orb click-through.
            spawn_cursor_watch(&handle);

            // ---- voice pipeline ----
            let cfg = Arc::new(config::load());

            // Launch at login for the installed app (never during dev, so we
            // don't register the debug exe in the user's startup).
            #[cfg(not(debug_assertions))]
            if cfg.autostart {
                use tauri_plugin_autostart::ManagerExt;
                if let Err(e) = app.autolaunch().enable() {
                    log::warn!("autostart enable failed: {e}");
                }
            }

            // Offline TTS: loads in the background; speaks replies if enabled.
            let tts_handle = if cfg.tts.enabled {
                tts::TtsHandle::spawn_load(cfg.clone())
            } else {
                tts::TtsHandle::disabled()
            };

            // Wake word callback: say "我在" briefly, then start listening.
            let tts_wake = tts_handle.clone();
            let on_wake: pipeline::OnWake = Arc::new(move || {
                log::info!("wake acknowledged: 我在");
                tts_wake.say("我在");
            });

            // Parse the transcript (rules first, LLM fallback) and run the
            // matching skill, then reflect the result on the orb.
            let cfg_dispatch = cfg.clone();
            let on_text: pipeline::OnText = Arc::new(move |app, text| {
                // Commands go through rules first. Anything the rules miss is sent
                // to the LLM, which either returns a command or answers as chat.
                let reply: skills::Reply = match intent::parse(&text) {
                    intent::Intent::Unknown { .. } => {
                        if cfg_dispatch.llm.api_key.is_empty() {
                            skills::Reply {
                                ok: false,
                                message: "未配置大模型，无法回答（请在 .env 设置 DEEPSEEK_API_KEY）"
                                    .into(),
                            }
                        } else {
                            log::info!("rule miss → querying LLM: {text}");
                            match intent::llm::classify(&cfg_dispatch.llm, &text) {
                                Ok(intent::llm::LlmOutcome::Command(i)) => {
                                    skills::dispatch(app, &cfg_dispatch, i)
                                }
                                Ok(intent::llm::LlmOutcome::Chat(ans)) => skills::Reply {
                                    ok: true,
                                    message: if ans.trim().is_empty() {
                                        "我不太确定，可以换种说法吗？".into()
                                    } else {
                                        ans
                                    },
                                },
                                Err(e) => {
                                    log::warn!("LLM fallback failed: {e:#}");
                                    skills::Reply {
                                        ok: false,
                                        message: "没听懂，再说一次试试".into(),
                                    }
                                }
                            }
                        }
                    }
                    cmd => skills::dispatch(app, &cfg_dispatch, cmd),
                };

                emit_state(
                    app,
                    if reply.ok { "acting" } else { "error" },
                    Some(&reply.message),
                );
                // Speak the reply aloud (background thread; serialized in TtsHandle).
                if reply.ok {
                    let t = tts_handle.clone();
                    let msg = reply.message.clone();
                    std::thread::spawn(move || t.say(&msg));
                }
                // Chat answers are longer; keep them on screen a little longer.
                let hold = if reply.message.chars().count() > 14 {
                    7000
                } else {
                    2800
                };
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(hold));
                    emit_state(&app, "idle", None);
                });
            });

            // First run (or models cleared): guide the user through setup.
            if models_dl::groups(&cfg)
                .iter()
                .any(|g| g.required && !g.installed)
            {
                open_onboarding(app.handle());
            }

            let manager = VoiceManager {
                on_text: on_text.clone(),
                current: Mutex::new(None),
            };
            let handle = app.handle().clone();
            *manager.current.lock().unwrap() = Some(pipeline::start(handle, cfg, on_text, on_wake));
            app.manage(manager);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
