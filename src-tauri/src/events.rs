use tauri::{AppHandle, Emitter};

/// Event channel the floating-ball frontend listens on.
pub const EVENT_STATE: &str = "assistant://state";

/// Payload pushed to the frontend to drive the orb's visual state.
#[derive(Clone, serde::Serialize)]
pub struct StatePayload {
    pub state: String,
    pub text: Option<String>,
}

/// Emit an assistant state change to the floating ball.
/// `state` is one of: idle | waking | listening | thinking | acting | error.
pub fn emit_state(app: &AppHandle, state: &str, text: Option<&str>) {
    let _ = app.emit(
        EVENT_STATE,
        StatePayload {
            state: state.to_string(),
            text: text.map(|s| s.to_string()),
        },
    );
}
