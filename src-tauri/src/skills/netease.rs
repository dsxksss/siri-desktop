//! NetEase Cloud Music "play by song name": resolve the name to a song id, then
//! hand `orpheus://song/{id}` to the client via the shell.
use crate::config::Config;
use anyhow::{anyhow, Result};
use serde::Deserialize;
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
    // Artist names differ between the two APIs: "artists" (web) vs "ar" (cloudsearch).
    #[serde(default)]
    artists: Vec<Artist>,
    #[serde(default)]
    ar: Vec<Artist>,
}
#[derive(Deserialize)]
struct Artist {
    name: Option<String>,
}

impl Song {
    fn artist_names(&self) -> Vec<&str> {
        self.artists
            .iter()
            .chain(self.ar.iter())
            .filter_map(|a| a.name.as_deref())
            .collect()
    }
}

/// Strip whitespace and lowercase for loose comparison.
fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect()
}

/// Score how well a search result matches the requested song (and artist).
/// Exact title match dominates; artist match and search rank break ties.
fn match_score(s: &Song, want_song: &str, want_artist: Option<&str>, idx: usize, total: usize) -> i32 {
    let mut score = 0i32;
    let name = s.name.as_deref().map(norm).unwrap_or_default();
    let target = norm(want_song);
    if !target.is_empty() {
        if name == target {
            score += 100;
        } else if name.contains(&target) || target.contains(&name) {
            score += 50;
        }
    }
    if let Some(a) = want_artist {
        let want = norm(a);
        if !want.is_empty()
            && s.artist_names().iter().any(|n| {
                let n = norm(n);
                n == want || n.contains(&want) || want.contains(&n)
            })
        {
            score += 30;
        }
    }
    // Search relevance: earlier results slightly preferred on ties.
    score + total.saturating_sub(idx) as i32
}

/// Search for `song` (optionally by `artist`) and start playback. Returns a
/// short description of what was launched.
///
/// Resolves the song id via NetEase's search API, then opens the desktop client
/// through the `orpheus://` protocol.
pub fn play(cfg: &Config, song: &str, artist: Option<&str>) -> Result<String> {
    let query = match artist {
        Some(a) => format!("{a} {song}"),
        None => song.to_string(),
    };

    let (id, name) = resolve_song(cfg, &query, song, artist)?;
    launch_uri(&format!("orpheus://song/{id}/?autoplay=1"))?;
    Ok(name.unwrap_or_else(|| song.to_string()))
}

/// Search NetEase for `query`, then return the id+name of the result that best
/// matches the requested `want_song` / `want_artist` (not just the first hit).
fn resolve_song(
    cfg: &Config,
    query: &str,
    want_song: &str,
    want_artist: Option<&str>,
) -> Result<(i64, Option<String>)> {
    let resp: SearchResp = if cfg.netease.search_api == "service" {
        let url = format!(
            "{}/cloudsearch?keywords={}&limit=10",
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
            "https://music.163.com/api/search/get/web?type=1&offset=0&total=true&limit=10&s={}",
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

    let songs = resp.result.and_then(|r| r.songs).unwrap_or_default();
    let total = songs.len();
    let best = songs
        .iter()
        .enumerate()
        .max_by_key(|(i, s)| match_score(s, want_song, want_artist, *i, total))
        .map(|(_, s)| s)
        .ok_or_else(|| anyhow!("没有找到歌曲：{query}"))?;

    log::info!(
        "netease matched \"{}\" -> 《{}》- {} (id={})",
        query,
        best.name.as_deref().unwrap_or("?"),
        best.artist_names().join("/"),
        best.id
    );
    Ok((best.id, best.name.clone()))
}

fn launch_uri(uri: &str) -> Result<()> {
    log::info!("Launching via protocol handler: {}", uri);
    open::that(uri).map_err(|e| anyhow!("启动 orpheus 协议失败：{e}"))
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
        match resolve_song(&cfg, "周杰伦 晴天", "晴天", Some("周杰伦")) {
            Ok((id, name)) => {
                println!("resolved id={id} name={name:?}");
                assert!(id > 0);
            }
            Err(e) => eprintln!("skipping (search unavailable): {e:#}"),
        }
    }
}
