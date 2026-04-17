use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

const LRCLIB_BASE: &str = "https://lrclib.net/api";

static TIMESTAMP_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[(\d{2}):(\d{2})\.(\d{2,3})\]\s*(.*)").unwrap());

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LrcLine {
    pub ts_ms: u64,
    pub text: String,
}

pub async fn fetch_lyrics(title: &str, artist: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("ColognaKaraoke/0.1")
        .build()
        .ok()?;

    // Exact match
    let url = format!(
        "{}/get?track_name={}&artist_name={}",
        LRCLIB_BASE,
        urlencode(title),
        urlencode(artist)
    );
    if let Ok(resp) = client.get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                if let Some(s) = v.get("syncedLyrics").and_then(|x| x.as_str()) {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
                if let Some(s) = v.get("plainLyrics").and_then(|x| x.as_str()) {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }

    // Search fallback
    let url = format!("{}/search?q={}", LRCLIB_BASE, urlencode(title));
    if let Ok(resp) = client.get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                for it in &arr {
                    if let Some(s) = it.get("syncedLyrics").and_then(|x| x.as_str()) {
                        if !s.is_empty() {
                            return Some(s.to_string());
                        }
                    }
                }
                if let Some(first) = arr.first() {
                    if let Some(s) = first.get("plainLyrics").and_then(|x| x.as_str()) {
                        if !s.is_empty() {
                            return Some(s.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                c.to_string()
            } else {
                let mut buf = [0u8; 4];
                let bytes = c.encode_utf8(&mut buf).as_bytes().to_vec();
                bytes.iter().map(|b| format!("%{:02X}", b)).collect::<String>()
            }
        })
        .collect()
}

pub fn parse_lrc(text: &str) -> Vec<LrcLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim();
        if let Some(c) = TIMESTAMP_RE.captures(raw) {
            let m: u64 = c[1].parse().unwrap_or(0);
            let s: u64 = c[2].parse().unwrap_or(0);
            let centis = &c[3];
            let ms_fraction: u64 = if centis.len() == 2 {
                centis.parse::<u64>().unwrap_or(0) * 10
            } else {
                centis.parse::<u64>().unwrap_or(0)
            };
            let ts_ms = (m * 60 + s) * 1000 + ms_fraction;
            let t = c[4].trim().to_string();
            if !t.is_empty() {
                lines.push(LrcLine { ts_ms, text: t });
            }
        }
    }
    lines.sort_by_key(|l| l.ts_ms);
    lines
}

#[tauri::command]
pub async fn fetch_lyrics_cmd(title: String, artist: String) -> Option<String> {
    fetch_lyrics(&title, &artist).await
}

#[tauri::command]
pub fn parse_lrc_cmd(text: String) -> Vec<LrcLine> {
    parse_lrc(&text)
}
