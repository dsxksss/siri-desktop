mod brightness;
mod media;
mod netease;
mod open_app;
mod volume;

use crate::config::Config;
use crate::intent::{Intent, MediaAction};
use tauri::AppHandle;

/// Result of executing an [`Intent`], surfaced on the floating ball.
pub struct Reply {
    pub ok: bool,
    pub message: String,
}

fn ok(message: String) -> Reply {
    Reply { ok: true, message }
}
fn err(message: String) -> Reply {
    Reply { ok: false, message }
}

/// Execute an intent and return a short user-facing reply.
pub fn dispatch(_app: &AppHandle, cfg: &Config, intent: Intent) -> Reply {
    match intent {
        Intent::SetVolume { percent } => match volume::set_volume(percent) {
            Ok(_) => ok(format!("音量已设为 {percent}%")),
            Err(e) => err(format!("音量设置失败：{e}")),
        },
        Intent::AdjustVolume { delta } => match volume::adjust_volume(delta) {
            Ok(v) => ok(format!("音量 {v}%")),
            Err(e) => err(format!("音量调节失败：{e}")),
        },
        Intent::Mute { on } => match volume::set_mute(on) {
            Ok(_) => ok(if on { "已静音" } else { "已取消静音" }.to_string()),
            Err(e) => err(format!("操作失败：{e}")),
        },
        Intent::SetBrightness { percent } => match brightness::set_brightness(percent) {
            Ok(_) => ok(format!("亮度已设为 {percent}%")),
            Err(e) => err(format!("亮度设置失败：{e}")),
        },
        Intent::AdjustBrightness { delta } => match brightness::adjust_brightness(delta) {
            Ok(v) => ok(format!("亮度 {v}%")),
            Err(e) => err(format!("亮度调节失败：{e}")),
        },
        Intent::OpenApp { name } => match open_app::open_app(&cfg.apps, &name) {
            Ok(n) => ok(format!("正在打开 {n}")),
            Err(e) => err(format!("{e}")),
        },
        Intent::MediaControl { action } => {
            let label = match action {
                MediaAction::PlayPause => "播放 / 暂停",
                MediaAction::Next => "下一首",
                MediaAction::Prev => "上一首",
            };
            media::control(action);
            ok(label.to_string())
        }
        Intent::PlayMusic { song, artist } => {
            match netease::play(cfg, &song, artist.as_deref()) {
                Ok(name) => ok(format!("正在播放：{name}")),
                Err(e) => err(format!("{e}")),
            }
        }
        // Phase 6: LLM fallback.
        Intent::Unknown { raw } => err(format!("没听懂：{raw}")),
    }
}
