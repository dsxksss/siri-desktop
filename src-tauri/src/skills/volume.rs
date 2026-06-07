//! System master volume via the Windows Core Audio `IAudioEndpointVolume` API.
use anyhow::{Context, Result};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

fn endpoint() -> Result<IAudioEndpointVolume> {
    unsafe {
        // Safe to call repeatedly; returns S_FALSE if already initialised on
        // this thread, RPC_E_CHANGED_MODE if another mode was set. Ignore both.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("create device enumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .context("get default audio endpoint")?;
        let volume: IAudioEndpointVolume =
            device.Activate(CLSCTX_ALL, None).context("activate endpoint volume")?;
        Ok(volume)
    }
}

pub fn get_volume() -> Result<u8> {
    let v = endpoint()?;
    let level = unsafe { v.GetMasterVolumeLevelScalar()? };
    Ok((level * 100.0).round().clamp(0.0, 100.0) as u8)
}

pub fn set_volume(percent: u8) -> Result<()> {
    let v = endpoint()?;
    let level = (percent.min(100) as f32) / 100.0;
    unsafe { v.SetMasterVolumeLevelScalar(level, std::ptr::null())? };
    Ok(())
}

pub fn adjust_volume(delta: i8) -> Result<u8> {
    let cur = get_volume()? as i32;
    let next = (cur + delta as i32).clamp(0, 100) as u8;
    set_volume(next)?;
    Ok(next)
}

pub fn set_mute(on: bool) -> Result<()> {
    let v = endpoint()?;
    unsafe { v.SetMute(on, std::ptr::null())? };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Actually exercises the Core Audio API: set a level, read it back, restore.
    #[test]
    fn set_and_get_roundtrip() {
        let original = match get_volume() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("skipping (no audio endpoint): {e:#}");
                return;
            }
        };
        set_volume(25).expect("set volume");
        let read = get_volume().expect("get volume");
        assert!((read as i32 - 25).abs() <= 2, "expected ~25, got {read}");
        set_volume(original).expect("restore volume");
    }
}
