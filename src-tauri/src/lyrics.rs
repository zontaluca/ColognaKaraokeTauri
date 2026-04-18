use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

const LRCLIB_BASE: &str = "https://lrclib.net/api";

static TIMESTAMP_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[(\d{2}):(\d{2})\.(\d{2,3})\]\s*(.*)").unwrap());

// Strip common YouTube noise: "(Official Video)", "[HD]", "ft. X", etc.
static YT_NOISE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\s*[\(\[]\s*(official\b[^)\]]*|lyrics?|audio|video|hd|4k|remaster(?:ed)?|live[^)\]]*|feat\.[^)\]]*|ft\.[^)\]]*|explicit)\s*[\)\]]",
    )
    .unwrap()
});

fn clean_title(raw: &str) -> String {
    let s = YT_NOISE_RE.replace_all(raw, "");
    s.trim().trim_end_matches('-').trim().to_string()
}

/// If title looks like "Artist - Song Title", return just "Song Title".
fn strip_artist_prefix(title: &str, artist: &str) -> Option<String> {
    let prefix = format!("{} - ", artist);
    if title.to_lowercase().starts_with(&prefix.to_lowercase()) {
        Some(title[prefix.len()..].trim().to_string())
    } else if let Some(idx) = title.find(" - ") {
        Some(title[idx + 3..].trim().to_string())
    } else {
        None
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LrcLine {
    pub ts_ms: u64,
    pub text: String,
}

pub async fn fetch_lyrics(title: &str, artist: &str, duration_sec: Option<u64>) -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("ColognaKaraoke/0.1")
        .build()
        .ok()?;

    // Build candidate title variants: raw → cleaned → stripped of "Artist - " prefix
    let cleaned = clean_title(title);
    let stripped = strip_artist_prefix(&cleaned, artist)
        .or_else(|| strip_artist_prefix(title, artist));
    let mut title_candidates: Vec<&str> = vec![title, &cleaned];
    if let Some(ref s) = stripped {
        title_candidates.push(s.as_str());
    }
    title_candidates.dedup();

    let dur_dist = |v: &serde_json::Value| -> i64 {
        match (duration_sec, v.get("duration").and_then(|x| x.as_f64())) {
            (Some(d), Some(ld)) => (d as i64 - ld as i64).abs(),
            _ => 0,
        }
    };

    // Try exact GET for each candidate (title+artist match = high confidence, use as-is)
    for t in &title_candidates {
        let url = format!(
            "{}/get?track_name={}&artist_name={}",
            LRCLIB_BASE,
            urlencode(t),
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
    }

    // Search fallback: collect all candidates, pick closest by duration
    let mut best_synced: Option<(i64, String)> = None;
    let mut best_plain: Option<(i64, String)> = None;

    for t in &title_candidates {
        let q = format!("{} {}", t, artist);
        let url = format!("{}/search?q={}", LRCLIB_BASE, urlencode(&q));
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                    for it in &arr {
                        let dist = dur_dist(it);
                        if let Some(s) = it.get("syncedLyrics").and_then(|x| x.as_str()) {
                            if !s.is_empty() {
                                if best_synced.as_ref().map_or(true, |(d, _)| dist < *d) {
                                    best_synced = Some((dist, s.to_string()));
                                }
                            }
                        }
                        if let Some(s) = it.get("plainLyrics").and_then(|x| x.as_str()) {
                            if !s.is_empty() {
                                if best_plain.as_ref().map_or(true, |(d, _)| dist < *d) {
                                    best_plain = Some((dist, s.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some((_, s)) = best_synced {
        return Some(s);
    }
    if let Some((_, s)) = best_plain {
        return Some(s);
    }
    None
}

/// Shift all LRC timestamps by `offset_ms` milliseconds (can be negative).
/// Lines without timestamps are passed through unchanged.
pub fn shift_lrc(lrc: &str, offset_ms: i64) -> String {
    lrc.lines()
        .map(|line| {
            if let Some(caps) = TIMESTAMP_RE.captures(line) {
                let m: u64 = caps[1].parse().unwrap_or(0);
                let s: u64 = caps[2].parse().unwrap_or(0);
                let centis = &caps[3];
                let ms_frac: u64 = if centis.len() == 2 {
                    centis.parse::<u64>().unwrap_or(0) * 10
                } else {
                    centis.parse::<u64>().unwrap_or(0)
                };
                let ts_ms = (m * 60 + s) * 1000 + ms_frac;
                let new_ts = (ts_ms as i64 + offset_ms).max(0) as u64;
                let nm = new_ts / 60000;
                let ns = (new_ts % 60000) / 1000;
                let nms = new_ts % 1000;
                let text = caps[4].trim();
                format!("[{:02}:{:02}.{:03}] {}", nm, ns, nms, text)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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
pub async fn fetch_lyrics_cmd(title: String, artist: String, duration_sec: Option<u64>) -> Option<String> {
    fetch_lyrics(&title, &artist, duration_sec).await
}

#[tauri::command]
pub fn parse_lrc_cmd(text: String) -> Vec<LrcLine> {
    parse_lrc(&text)
}
