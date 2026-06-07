//! NetEase Cloud Music "play by song name": resolve the name to a song id, then
//! hand `orpheus://song/{id}` to the client via the shell.
use crate::config::Config;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::process::Command;
use std::time::Duration;

#[derive(Deserialize)]
struct SearchResp {
    result: Option<SearchResult>,
}
#[derive(Deserialize)]
struct SearchResult {
    songs: Option<Vec<Song>>,
}
#[derive(Deserialize)]
struct Song {
    id: i64,
    name: Option<String>,
}

/// Search for `song` (optionally by `artist`) and start playback. Returns a
/// short description of what was launched.
pub fn play(cfg: &Config, song: &str, artist: Option<&str>) -> Result<String> {
    let query = match artist {
        Some(a) => format!("{a} {song}"),
        None => song.to_string(),
    };
    let (id, name) = resolve_song(cfg, &query)?;
    launch_uri(&format!("orpheus://song/{id}"))?;
    Ok(name.unwrap_or_else(|| song.to_string()))
}

fn resolve_song(cfg: &Config, query: &str) -> Result<(i64, Option<String>)> {
    let resp: SearchResp = if cfg.netease.search_api == "service" {
        let url = format!(
            "{}/cloudsearch?keywords={}&limit=1",
            cfg.netease.service_url.trim_end_matches('/'),
            urlencoding::encode(query)
        );
        ureq::get(&url)
            .timeout(Duration::from_secs(8))
            .call()
            .map_err(|e| anyhow!("网易云搜索服务请求失败：{e}"))?
            .into_json()
            .map_err(|e| anyhow!("解析搜索结果失败：{e}"))?
    } else {
        // Direct legacy web API. Browser-like headers reduce anti-crawl blocks.
        let url = format!(
            "https://music.163.com/api/search/get/web?type=1&offset=0&total=true&limit=5&s={}",
            urlencoding::encode(query)
        );
        ureq::get(&url)
            .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .set("Referer", "https://music.163.com/")
            .set("Cookie", "os=pc; appver=2.0.2")
            .timeout(Duration::from_secs(8))
            .call()
            .map_err(|e| anyhow!("网易云搜索请求失败：{e}（可改用本地 service 接口）"))?
            .into_json()
            .map_err(|e| anyhow!("解析搜索结果失败：{e}"))?
    };

    let song = resp
        .result
        .and_then(|r| r.songs)
        .and_then(|mut s| if s.is_empty() { None } else { Some(s.remove(0)) })
        .ok_or_else(|| anyhow!("没有找到歌曲：{query}"))?;
    Ok((song.id, song.name))
}

fn launch_uri(uri: &str) -> Result<()> {
    Command::new("cmd")
        .arg("/C")
        .arg(format!("start \"\" \"{uri}\""))
        .spawn()
        .map_err(|e| anyhow!("无法启动网易云音乐：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    // Hits the live NetEase search API. Lenient: anti-crawl/network failures are
    // logged and skipped so the suite stays green offline.
    #[test]
    fn direct_search_resolves() {
        let cfg = Config::default();
        match resolve_song(&cfg, "周杰伦 晴天") {
            Ok((id, name)) => {
                println!("resolved id={id} name={name:?}");
                assert!(id > 0);
            }
            Err(e) => eprintln!("skipping (search unavailable): {e:#}"),
        }
    }
}
