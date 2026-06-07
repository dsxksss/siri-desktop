//! LLM fallback. A single call decides whether the utterance is a device
//! command (returns a structured [`Intent`]) or ordinary conversation (returns
//! a spoken-style answer). Uses any OpenAI-compatible chat endpoint (DeepSeek).
use super::{Intent, MediaAction};
use crate::config::Llm;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::time::Duration;

/// Outcome of an LLM call: a device command to run, or a chat answer to show.
pub enum LlmOutcome {
    Command(Intent),
    Chat(String),
}

const SYSTEM_PROMPT: &str = r#"你是“小问”，一个运行在 Windows 上的中文桌面语音助手（用户用唤醒词“你好问问”唤醒你）。
判断用户这句话是「设备指令」还是「普通对话/提问」，只输出一个 JSON 对象，不要任何多余文字或解释。

如果是设备指令，用下面的 action 之一：
- {"action":"open_app","name":"应用名"}
- {"action":"play_music","song":"歌名","artist":"歌手(可选)"}
- {"action":"set_volume","percent":0-100}
- {"action":"adjust_volume","delta":正数调大/负数调小}
- {"action":"mute","on":true 静音/false 取消静音}
- {"action":"set_brightness","percent":0-100}
- {"action":"adjust_brightness","delta":正数/负数}
- {"action":"media","control":"play_pause"|"next"|"prev"}

如果不是设备指令（闲聊、提问、问你是谁等），用：
- {"action":"chat","reply":"用一两句话、口语化地中文回答用户"}

示例：
"把声音弄小一点" -> {"action":"adjust_volume","delta":-10}
"你是谁" -> {"action":"chat","reply":"我是小问，你的桌面语音助手，可以帮你打开应用、点歌、调音量和亮度。"}
"讲个冷笑话" -> {"action":"chat","reply":"为什么程序员分不清万圣节和圣诞节？因为 Oct 31 == Dec 25。"}"#;

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    message: Message,
}
#[derive(Deserialize)]
struct Message {
    content: String,
}

#[derive(Deserialize)]
struct IntentJson {
    action: String,
    name: Option<String>,
    song: Option<String>,
    artist: Option<String>,
    percent: Option<i64>,
    delta: Option<i64>,
    on: Option<bool>,
    control: Option<String>,
    reply: Option<String>,
}

/// Send `text` to the LLM and interpret the reply.
pub fn classify(cfg: &Llm, text: &str) -> Result<LlmOutcome> {
    if cfg.api_key.is_empty() {
        return Err(anyhow!("未配置 LLM api_key"));
    }
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": cfg.model,
        "temperature": 0.3,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": text },
        ],
    });

    let resp: ChatResp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", cfg.api_key))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(20))
        .send_json(body)
        .map_err(|e| anyhow!("LLM 请求失败：{e}"))?
        .into_json()
        .map_err(|e| anyhow!("解析 LLM 响应失败：{e}"))?;

    let content = resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("LLM 无返回"))?
        .message
        .content;

    parse_outcome(&content)
}

fn parse_outcome(content: &str) -> Result<LlmOutcome> {
    // Tolerate stray text around the JSON object.
    let json = match (content.find('{'), content.rfind('}')) {
        (Some(a), Some(b)) if b > a => &content[a..=b],
        _ => content,
    };
    let v: IntentJson =
        serde_json::from_str(json).map_err(|e| anyhow!("LLM 返回的 JSON 无法解析：{e}"))?;

    let pct = |p: Option<i64>| p.unwrap_or(0).clamp(0, 100) as u8;
    let delta = |d: Option<i64>| d.unwrap_or(0).clamp(-100, 100) as i8;

    let outcome = match v.action.as_str() {
        "open_app" => LlmOutcome::Command(Intent::OpenApp {
            name: v.name.unwrap_or_default(),
        }),
        "play_music" => LlmOutcome::Command(Intent::PlayMusic {
            song: v.song.unwrap_or_default(),
            artist: v.artist.filter(|s| !s.is_empty()),
        }),
        "set_volume" => LlmOutcome::Command(Intent::SetVolume { percent: pct(v.percent) }),
        "adjust_volume" => LlmOutcome::Command(Intent::AdjustVolume { delta: delta(v.delta) }),
        "mute" => LlmOutcome::Command(Intent::Mute {
            on: v.on.unwrap_or(true),
        }),
        "set_brightness" => LlmOutcome::Command(Intent::SetBrightness { percent: pct(v.percent) }),
        "adjust_brightness" => {
            LlmOutcome::Command(Intent::AdjustBrightness { delta: delta(v.delta) })
        }
        "media" => LlmOutcome::Command(Intent::MediaControl {
            action: match v.control.as_deref() {
                Some("next") => MediaAction::Next,
                Some("prev") => MediaAction::Prev,
                _ => MediaAction::PlayPause,
            },
        }),
        // "chat" and anything unrecognized fall back to a conversational answer.
        _ => LlmOutcome::Chat(v.reply.unwrap_or_default()),
    };
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Live call. Lenient: skips if no key / network. Confirms "你是谁" is chat.
    #[test]
    fn classify_live() {
        let _ = dotenvy::dotenv();
        let mut c = Llm::default();
        c.model = "deepseek-v4-flash".into();
        if let Ok(k) = std::env::var("DEEPSEEK_API_KEY") {
            c.api_key = k;
        }
        if c.api_key.is_empty() {
            eprintln!("skip: no DEEPSEEK_API_KEY");
            return;
        }

        match classify(&c, "你是谁") {
            Ok(LlmOutcome::Chat(a)) => {
                println!("chat reply: {a}");
                assert!(!a.trim().is_empty());
            }
            Ok(LlmOutcome::Command(i)) => panic!("expected chat for '你是谁', got {i:?}"),
            Err(e) => eprintln!("skip (network): {e:#}"),
        }
        match classify(&c, "把音量调到20") {
            Ok(LlmOutcome::Command(i)) => println!("command: {i:?}"),
            Ok(LlmOutcome::Chat(a)) => eprintln!("note: chat instead of command: {a}"),
            Err(e) => eprintln!("skip (network): {e:#}"),
        }
    }
}
