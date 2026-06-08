//! Launch applications by spoken name, using the configured name->path map with
//! a automatic system shortcut scanning, fuzzy local matching, and LLM semantic matching.
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use serde::Deserialize;

/// Resolve a spoken name to a target (path / URI / PATH command) and launch it.
/// Returns the friendly name that was launched.
pub fn open_app(cfg: &crate::config::Config, name: &str) -> Result<String> {
    // 1. Try to resolve via custom user config first
    if let Some(target) = resolve_custom(&cfg.apps, name) {
        if launch(&target).is_ok() {
            return Ok(name.to_string());
        }
    }

    // 2. Scan installed applications (Start Menu and Desktop shortcuts)
    let installed = scan_installed_apps();
    
    // 3. Search for a local match (fuzzy string containment / exact matching)
    if let Some(matched_path) = resolve_local(&installed, name) {
        log::info!("local match found for app '{}': {:?}", name, matched_path);
        launch(&matched_path.to_string_lossy())?;
        return Ok(name.to_string());
    }

    // 4. Try LLM matching if LLM configuration is available
    if !cfg.llm.api_key.is_empty() {
        let app_names: Vec<&str> = installed.keys().map(|s| s.as_str()).collect();
        if let Some(matched_name) = llm_match_app(&cfg.llm, name, &app_names) {
            if let Some(matched_path) = installed.get(&matched_name) {
                log::info!("LLM matched app '{}' to '{}': {:?}", name, matched_name, matched_path);
                launch(&matched_path.to_string_lossy())?;
                return Ok(matched_name);
            }
        }
    }

    // 5. Fallback: let the shell resolve the raw spoken name (e.g. system commands like notepad, calc)
    let norm = name.trim().to_string();
    if launch(&norm).is_ok() {
        return Ok(norm);
    }

    Err(anyhow!(
        "找不到程序或快捷方式：{}，且未配置或无法匹配到对应的可执行文件",
        name
    ))
}

fn resolve_custom(apps: &HashMap<String, String>, name: &str) -> Option<String> {
    let n = name.trim();
    if let Some(p) = apps.get(n) {
        return Some(p.clone());
    }
    // fuzzy: spoken name contains a configured key or vice versa
    for (key, path) in apps {
        if n.contains(key.as_str()) || key.contains(n) {
            return Some(path.clone());
        }
    }
    None
}

fn resolve_local(installed: &HashMap<String, PathBuf>, name: &str) -> Option<PathBuf> {
    let n = name.trim().to_lowercase();
    if n.is_empty() {
        return None;
    }

    // 1. Exact match (case insensitive)
    for (app_name, path) in installed {
        if app_name.to_lowercase() == n {
            return Some(path.clone());
        }
    }

    // 2. Substring match (either input is in app_name or vice versa)
    let mut best_match: Option<(&String, &PathBuf)> = None;
    for (app_name, path) in installed {
        let app_lower = app_name.to_lowercase();
        if app_lower.contains(&n) || n.contains(&app_lower) {
            // Prefer the shorter name to reduce false positives (e.g. "WeChat" over "WeChat DevTools")
            if let Some((best_name, _)) = best_match {
                if app_name.len() < best_name.len() {
                    best_match = Some((app_name, path));
                }
            } else {
                best_match = Some((app_name, path));
            }
        }
    }

    best_match.map(|(_, path)| path.clone())
}

pub fn scan_installed_apps() -> HashMap<String, PathBuf> {
    let mut apps = HashMap::new();
    
    // 1. User Start Menu
    if let Ok(appdata) = std::env::var("APPDATA") {
        let user_start = PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs");
        scan_dir_shortcuts(&user_start, &mut apps);
    }
    
    // 2. System Start Menu
    if let Ok(programdata) = std::env::var("PROGRAMDATA") {
        let system_start = PathBuf::from(programdata).join(r"Microsoft\Windows\Start Menu\Programs");
        scan_dir_shortcuts(&system_start, &mut apps);
    }

    // 3. Desktop Shortcuts (User + Public)
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let user_desktop = PathBuf::from(userprofile).join("Desktop");
        scan_dir_shortcuts(&user_desktop, &mut apps);
    }
    if let Ok(public) = std::env::var("PUBLIC") {
        let public_desktop = PathBuf::from(public).join("Desktop");
        scan_dir_shortcuts(&public_desktop, &mut apps);
    }
    
    apps
}

fn scan_dir_shortcuts(dir: &Path, apps: &mut HashMap<String, PathBuf>) {
    if !dir.exists() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_dir_shortcuts(&path, apps);
            } else if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext.eq_ignore_ascii_case("lnk") {
                        if let Some(stem) = path.file_stem() {
                            if let Some(name) = stem.to_str() {
                                apps.insert(name.to_string(), path.clone());
                            }
                        }
                    }
                }
            }
        }
    }
}

fn llm_match_app(llm_cfg: &crate::config::Llm, input_name: &str, app_names: &[&str]) -> Option<String> {
    if llm_cfg.api_key.is_empty() || app_names.is_empty() {
        return None;
    }
    
    let url = format!("{}/chat/completions", llm_cfg.base_url.trim_end_matches('/'));
    
    let system_prompt = r#"You are a system assistant. The user wants to open an application. Match their spoken name to the most likely installed application name from the provided list.
Return a JSON object with a single field: {"matched": "exact name from the list"} or {"matched": null} if no likely match exists.
Only return the JSON object, no markdown formatting, no extra text."#;
    
    let user_content = format!(
        "Spoken application name: \"{}\"\nInstalled applications list: {:?}",
        input_name, app_names
    );
    
    let body = serde_json::json!({
        "model": llm_cfg.model,
        "temperature": 0.1,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_content },
        ],
    });
    
    let resp: serde_json::Value = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", llm_cfg.api_key))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(10))
        .send_json(body)
        .ok()?
        .into_json()
        .ok()?;
        
    let content = resp["choices"][0]["message"]["content"].as_str()?;
    
    // Parse the JSON outcome
    #[derive(Deserialize)]
    struct MatchResult {
        matched: Option<String>,
    }
    let res: MatchResult = serde_json::from_str(content).ok()?;
    res.matched
}

fn launch(target: &str) -> Result<()> {
    if target.contains("://") {
        return shell_start(target);
    }
    let norm = target.replace('/', "\\");
    let path = Path::new(&norm);
    let looks_like_path = norm.contains('\\') || norm.contains(':');
    if looks_like_path {
        if !path.is_file() {
            return Err(anyhow!("找不到文件：{norm}"));
        }
        let is_shortcut = norm.to_lowercase().ends_with(".lnk") || norm.to_lowercase().ends_with(".url");
        if is_shortcut {
            shell_start(&norm)
        } else {
            let mut cmd = Command::new(&norm);
            if let Some(dir) = path.parent() {
                if !dir.as_os_str().is_empty() {
                    cmd.current_dir(dir);
                }
            }
            cmd.spawn().map_err(|e| anyhow!("启动失败: {e}"))?;
            Ok(())
        }
    } else {
        shell_start(&norm)
    }
}

fn shell_start(target: &str) -> Result<()> {
    Command::new("cmd")
        .args(&["/C", "start", "", target])
        .spawn()
        .map_err(|e| anyhow!("启动失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_path_errors_cleanly() {
        assert!(launch("C:/no/such/place/app.exe").is_err());
    }

    #[test]
    fn resolve_prefers_exact_then_fuzzy() {
        let mut apps = std::collections::HashMap::new();
        apps.insert("网易云音乐".to_string(), "D:/CloudMusic/cloudmusic.exe".to_string());
        assert_eq!(resolve_custom(&apps, "网易云音乐"), Some("D:/CloudMusic/cloudmusic.exe".to_string()));
        assert_eq!(resolve_custom(&apps, "打开网易云音乐播放"), Some("D:/CloudMusic/cloudmusic.exe".to_string()));
    }
}
