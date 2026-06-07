pub mod llm;
mod rules;

pub use rules::parse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    PlayPause,
    Next,
    Prev,
}

/// A single understood command. Produced by the rule engine ([`parse`]) and, in
/// Phase 6, by the LLM fallback. Consumed uniformly by `skills::dispatch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    OpenApp { name: String },
    PlayMusic { song: String, artist: Option<String> },
    SetVolume { percent: u8 },
    AdjustVolume { delta: i8 },
    Mute { on: bool },
    SetBrightness { percent: u8 },
    AdjustBrightness { delta: i8 },
    MediaControl { action: MediaAction },
    /// No rule matched; routed to the LLM fallback (Phase 6).
    Unknown { raw: String },
}
