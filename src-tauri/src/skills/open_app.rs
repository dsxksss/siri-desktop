//! Launch applications by spoken name, using the configured name->path map with
//! a fuzzy fallback, then letting the shell resolve anything unknown.
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Resolve a spoken name to a target (path / URI / PATH command) and launch it.
/// Returns the friendly name that was launched.
pub fn open_app(apps: &HashMap<String, String>, name: &str) -> Result<String> {
    let target = resolve(apps, name);
    launch(&target)?;
    Ok(name.to_string())
}

fn resolve(apps: &HashMap<String, String>, name: &str) -> String {
    let n = name.trim();
    if let Some(p) = apps.get(n) {
        return p.clone();
    }
    // fuzzy: spoken name contains a configured key or vice versa
    for (key, path) in apps {
        if n.contains(key.as_str()) || key.contains(n) {
            return path.clone();
        }
    }
    // fall back to letting the shell resolve it (PATH command, registered app)
    n.to_string()
}

fn launch(target: &str) -> Result<()> {
    // URI (e.g. orpheus://...) — hand to the shell handler.
    if target.contains("://") {
        return shell_start(target);
    }
    // Config often uses forward slashes; Windows launching wants backslashes.
    let norm = target.replace('/', "\\");
    let path = Path::new(&norm);
    let looks_like_path = norm.contains('\\') || norm.contains(':');
    if looks_like_path {
        // An explicit file path: spawn it directly, with a clear error if it's
        // missing (so a wrong config path doesn't become a cryptic shell error).
        if !path.is_file() {
            return Err(anyhow!(
                "找不到程序文件：{norm}（请在 config.toml 的 [apps] 中改为正确路径）"
            ));
        }
        let mut cmd = Command::new(&norm);
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                cmd.current_dir(dir);
            }
        }
        cmd.spawn().map_err(|e| anyhow!("启动失败: {e}"))?;
        Ok(())
    } else {
        // Bare command resolvable via PATH / App Paths (notepad, calc, msedge).
        shell_start(&norm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_path_errors_cleanly() {
        // A wrong configured path must yield a clear error, not a shell `\\` dialog.
        assert!(launch("C:/no/such/place/app.exe").is_err());
    }

    #[test]
    fn resolve_prefers_exact_then_fuzzy() {
        let mut apps = std::collections::HashMap::new();
        apps.insert("网易云音乐".to_string(), "D:/CloudMusic/cloudmusic.exe".to_string());
        assert_eq!(resolve(&apps, "网易云音乐"), "D:/CloudMusic/cloudmusic.exe");
        // fuzzy: spoken name contains the configured key
        assert_eq!(resolve(&apps, "打开网易云音乐播放"), "D:/CloudMusic/cloudmusic.exe");
    }
}

/// `cmd /C start "" "<target>"` — works for URIs, PATH commands and paths.
fn shell_start(target: &str) -> Result<()> {
    Command::new("cmd")
        .arg("/C")
        .arg(format!("start \"\" \"{target}\""))
        .spawn()
        .map_err(|e| anyhow!("启动失败: {e}"))?;
    Ok(())
}
