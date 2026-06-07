//! Screen brightness via the `brightness` crate (WMI on Windows). The crate is
//! async; we drive it to completion with a small blocking executor.
use anyhow::Result;
use brightness::Brightness;
use futures::{executor::block_on, TryStreamExt};

pub fn set_brightness(percent: u8) -> Result<()> {
    block_on(async {
        let mut devices = brightness::brightness_devices();
        let mut any = false;
        while let Some(mut dev) = devices.try_next().await? {
            dev.set(percent.min(100) as u32).await?;
            any = true;
        }
        if !any {
            anyhow::bail!("未找到可调节亮度的显示器");
        }
        Ok(())
    })
}

pub fn adjust_brightness(delta: i8) -> Result<u8> {
    block_on(async {
        let mut devices = brightness::brightness_devices();
        let mut last: Option<u32> = None;
        while let Some(mut dev) = devices.try_next().await? {
            let cur = dev.get().await? as i32;
            let next = (cur + delta as i32).clamp(0, 100) as u32;
            dev.set(next).await?;
            last = Some(next);
        }
        last.map(|v| v as u8)
            .ok_or_else(|| anyhow::anyhow!("未找到可调节亮度的显示器"))
    })
}
