use super::{Intent, MediaAction};
use once_cell::sync::Lazy;
use regex::Regex;

/// Default step for relative "louder / dimmer" style commands.
const STEP: i8 = 10;

/// Parse a recognized utterance into an [`Intent`] using keyword/number rules.
/// Returns [`Intent::Unknown`] when nothing matches (handled by the LLM later).
pub fn parse(text: &str) -> Intent {
    let raw = text.trim();
    // Work on a whitespace-free copy; Chinese ASR output rarely needs spaces and
    // this makes substring matching robust.
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();

    // ---- mute ----
    if has_any(&s, &["取消静音", "解除静音", "打开声音", "恢复声音"]) {
        return Intent::Mute { on: false };
    }
    if has_any(&s, &["静音", "关闭声音", "闭嘴", "别说话"]) {
        return Intent::Mute { on: true };
    }

    // ---- media transport ----
    if has_any(&s, &["下一首", "下一曲", "下一个", "切歌", "换一首"]) {
        return Intent::MediaControl {
            action: MediaAction::Next,
        };
    }
    if has_any(&s, &["上一首", "上一曲", "上一个"]) {
        return Intent::MediaControl {
            action: MediaAction::Prev,
        };
    }
    if has_any(
        &s,
        &["暂停", "继续播放", "继续", "恢复播放", "播放暂停", "暂停一下"],
    ) && !has_play_target(&s)
    {
        return Intent::MediaControl {
            action: MediaAction::PlayPause,
        };
    }

    // ---- brightness ----
    if s.contains("亮度") || s.contains("屏幕") {
        if let Some(n) = first_number(&s) {
            return Intent::SetBrightness { percent: n };
        }
        if has_any(&s, &["亮一点", "亮一些", "调亮", "提高", "增加", "再亮"]) {
            return Intent::AdjustBrightness { delta: STEP };
        }
        if has_any(&s, &["暗一点", "暗一些", "调暗", "降低", "减少", "再暗"]) {
            return Intent::AdjustBrightness { delta: -STEP };
        }
    }
    if has_any(&s, &["亮一点", "亮一些", "调亮", "屏幕亮"]) {
        return Intent::AdjustBrightness { delta: STEP };
    }
    if has_any(&s, &["暗一点", "暗一些", "调暗", "屏幕暗"]) {
        return Intent::AdjustBrightness { delta: -STEP };
    }

    // ---- volume ----
    if s.contains("音量") || s.contains("声音") || s.contains("音响") {
        if let Some(n) = first_number(&s) {
            return Intent::SetVolume { percent: n };
        }
        if has_any(&s, &["大", "高", "增", "提", "响"]) {
            return Intent::AdjustVolume { delta: STEP };
        }
        if has_any(&s, &["小", "低", "减", "降", "轻"]) {
            return Intent::AdjustVolume { delta: -STEP };
        }
    }
    if has_any(&s, &["大声", "大点声", "响一点"]) {
        return Intent::AdjustVolume { delta: STEP };
    }
    if has_any(&s, &["小声", "小点声", "轻一点"]) {
        return Intent::AdjustVolume { delta: -STEP };
    }

    // ---- play music ----
    const PLAY_KW: &[&str] = &[
        "我想听", "我要听", "听一首", "点一首", "放一首", "来一首", "播放", "放首", "唱一首",
    ];
    for kw in PLAY_KW {
        if let Some(idx) = s.find(kw) {
            let rest = strip_song_prefix(&s[idx + kw.len()..]);
            if !rest.is_empty() {
                let (artist, song) = split_artist_song(rest);
                return Intent::PlayMusic { song, artist };
            }
        }
    }

    // ---- open app ----
    const OPEN_KW: &[&str] = &[
        "帮我打开", "幫我打開", "打开", "打開", "启动", "啟動", "运行", "運行", "开一下",
    ];
    for kw in OPEN_KW {
        if let Some(idx) = s.find(kw) {
            let name = s[idx + kw.len()..].trim_matches(|c| "的吧呀啊一下".contains(c));
            if !name.is_empty() {
                return Intent::OpenApp {
                    name: name.to_string(),
                };
            }
        }
    }

    Intent::Unknown {
        raw: raw.to_string(),
    }
}

fn has_any(s: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| s.contains(n))
}

/// True if the utterance names something to play (so "播放" is a music request,
/// not a transport play/pause toggle).
fn has_play_target(s: &str) -> bool {
    for kw in ["播放", "放一首", "来一首", "我想听"] {
        if let Some(idx) = s.find(kw) {
            if !strip_song_prefix(&s[idx + kw.len()..]).is_empty() {
                return true;
            }
        }
    }
    false
}

/// Strip leading filler ("的", "一首", "首", …) from a song phrase without
/// eating a bare "一" (so "一千年以后" survives).
fn strip_song_prefix(rest: &str) -> &str {
    let mut rest = rest;
    loop {
        let start = rest;
        for pre in ["的", "一首", "一支", "首", "歌曲"] {
            rest = rest.strip_prefix(pre).unwrap_or(rest);
        }
        if rest == start {
            return rest;
        }
    }
}

/// Split "周杰伦的晴天" -> (Some("周杰伦"), "晴天"). No separator -> (None, all).
fn split_artist_song(rest: &str) -> (Option<String>, String) {
    if let Some(pos) = rest.find('的') {
        let artist = &rest[..pos];
        let song = &rest[pos + '的'.len_utf8()..];
        if !artist.is_empty() && !song.is_empty() {
            return (Some(artist.to_string()), song.to_string());
        }
    }
    (None, rest.to_string())
}

static NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d{1,3}").unwrap());

/// Extract a 0..=100 value, accepting Arabic digits (preferred; SenseVoice ITN
/// emits these) or simple Chinese numerals.
fn first_number(s: &str) -> Option<u8> {
    // Drop "一点/一些/一下/一会" so "大一点" (= a little louder) isn't read as 1.
    let cleaned = s
        .replace("百分之", "")
        .replace("百分比", "")
        .replace('%', "")
        .replace("一点儿", "")
        .replace("一点", "")
        .replace("一些", "")
        .replace("一下", "")
        .replace("一会儿", "")
        .replace("一会", "");
    if let Some(m) = NUM_RE.find(&cleaned) {
        if let Ok(n) = m.as_str().parse::<u32>() {
            return Some(n.min(100) as u8);
        }
    }
    const ZH: &str = "零一二两三四五六七八九十百";
    if let Some(start) = cleaned.find(|c: char| ZH.contains(c)) {
        let run: String = cleaned[start..]
            .chars()
            .take_while(|c| ZH.contains(*c))
            .collect();
        if let Some(n) = zh_to_num(&run) {
            return Some(n.min(100) as u8);
        }
    }
    None
}

fn zh_to_num(s: &str) -> Option<u32> {
    let digit = |c: char| -> Option<u32> {
        match c {
            '零' => Some(0),
            '一' => Some(1),
            '二' | '两' => Some(2),
            '三' => Some(3),
            '四' => Some(4),
            '五' => Some(5),
            '六' => Some(6),
            '七' => Some(7),
            '八' => Some(8),
            '九' => Some(9),
            _ => None,
        }
    };
    if s.contains('百') {
        return Some(100);
    }
    if let Some(pos) = s.find('十') {
        let before = &s[..pos];
        let after = &s[pos + '十'.len_utf8()..];
        let tens = if before.is_empty() {
            1
        } else {
            digit(before.chars().next()?)?
        };
        let ones = if after.is_empty() {
            0
        } else {
            digit(after.chars().next()?)?
        };
        return Some(tens * 10 + ones);
    }
    digit(s.chars().next()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_commands() {
        assert_eq!(parse("音量调到30"), Intent::SetVolume { percent: 30 });
        assert_eq!(parse("把音量设为 50%"), Intent::SetVolume { percent: 50 });
        assert_eq!(parse("音量调到一百"), Intent::SetVolume { percent: 100 });
        assert_eq!(parse("声音调到三十"), Intent::SetVolume { percent: 30 });
        assert_eq!(parse("声音大一点"), Intent::AdjustVolume { delta: 10 });
        assert_eq!(parse("音量小一点"), Intent::AdjustVolume { delta: -10 });
        assert_eq!(parse("大声一点"), Intent::AdjustVolume { delta: 10 });
        assert_eq!(parse("静音"), Intent::Mute { on: true });
        assert_eq!(parse("取消静音"), Intent::Mute { on: false });
    }

    #[test]
    fn brightness_commands() {
        assert_eq!(parse("亮度调到80"), Intent::SetBrightness { percent: 80 });
        assert_eq!(parse("屏幕亮度调到五十"), Intent::SetBrightness { percent: 50 });
        assert_eq!(parse("屏幕暗一点"), Intent::AdjustBrightness { delta: -10 });
        assert_eq!(parse("亮一点"), Intent::AdjustBrightness { delta: 10 });
    }

    #[test]
    fn media_commands() {
        assert_eq!(
            parse("下一首"),
            Intent::MediaControl {
                action: MediaAction::Next
            }
        );
        assert_eq!(
            parse("上一首"),
            Intent::MediaControl {
                action: MediaAction::Prev
            }
        );
        assert_eq!(
            parse("暂停"),
            Intent::MediaControl {
                action: MediaAction::PlayPause
            }
        );
    }

    #[test]
    fn play_music_commands() {
        assert_eq!(
            parse("播放晴天"),
            Intent::PlayMusic {
                song: "晴天".into(),
                artist: None
            }
        );
        assert_eq!(
            parse("我想听周杰伦的晴天"),
            Intent::PlayMusic {
                song: "晴天".into(),
                artist: Some("周杰伦".into())
            }
        );
    }

    #[test]
    fn open_app_commands() {
        assert_eq!(
            parse("打开网易云音乐"),
            Intent::OpenApp {
                name: "网易云音乐".into()
            }
        );
        assert_eq!(
            parse("帮我打开微信"),
            Intent::OpenApp {
                name: "微信".into()
            }
        );
    }

    #[test]
    fn unknown_passthrough() {
        assert_eq!(
            parse("今天天气怎么样"),
            Intent::Unknown {
                raw: "今天天气怎么样".into()
            }
        );
    }
}
