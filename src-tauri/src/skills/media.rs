//! Global media transport control via synthetic media-key presses.
use crate::intent::MediaAction;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, KEYEVENTF_KEYUP, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
    VIRTUAL_KEY,
};

pub fn control(action: MediaAction) {
    let vk: VIRTUAL_KEY = match action {
        MediaAction::PlayPause => VK_MEDIA_PLAY_PAUSE,
        MediaAction::Next => VK_MEDIA_NEXT_TRACK,
        MediaAction::Prev => VK_MEDIA_PREV_TRACK,
    };
    unsafe {
        keybd_event(vk.0 as u8, 0, Default::default(), 0);
        keybd_event(vk.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}
