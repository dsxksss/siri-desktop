mod asr;
mod audio;
mod config;
mod events;
mod intent;
mod pipeline;
mod skills;
mod tts;
mod wake;

use events::emit_state;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, PhysicalPosition};

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

    /// Stop the current pipeline (releasing the mic), reload config, restart.
    fn restart(&self, app: &AppHandle) {
        let mut guard = self.current.lock().unwrap();
        if let Some(h) = guard.take() {
            let _ = h.control.send(pipeline::Control::Shutdown);
            let _ = h.join.join();
        }
        let cfg = Arc::new(config::load());
        *guard = Some(pipeline::start(app.clone(), cfg, self.on_text.clone()));
    }
}

/// Orb clicked: start listening immediately, skipping the wake word.
#[tauri::command]
fn manual_listen(mgr: tauri::State<VoiceManager>) {
    mgr.listen();
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
        .invoke_handler(tauri::generate_handler![manual_listen, list_microphones, get_config, save_config])
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
            let menu = Menu::with_items(app, &[&show, &mic_menu, &config_item, &quit])?;
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

            // ---- floating ball placement ----
            let handle = app.handle().clone();
            position_top_center(&handle);
            emit_state(&handle, "idle", None);

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

            let manager = VoiceManager {
                on_text: on_text.clone(),
                current: Mutex::new(None),
            };
            let handle = app.handle().clone();
            *manager.current.lock().unwrap() = Some(pipeline::start(handle, cfg, on_text));
            app.manage(manager);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
