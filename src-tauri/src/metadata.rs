use std::path::Path;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct AlbumMeta {
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub release_year: Option<String>,
    pub cover_path: Option<String>,
    pub genre: Option<String>,
}

/// Fetch album metadata + download cover. Best-effort: returns empty on failure.
pub async fn fetch_album_meta(
    title: &str,
    artist: &str,
    song_dir: &Path,
) -> AlbumMeta {
    let client = match reqwest::Client::builder()
        .user_agent("ColognaKaraoke/0.1 (local app)")
        .build()
    {
        Ok(c) => c,
        Err(_) => return AlbumMeta::default(),
    };

    // iTunes Search API
    let term = format!("{} {}", title, artist);
    let url = format!(
        "https://itunes.apple.com/search?term={}&entity=song&limit=1",
        urlencode(&term)
    );
    let mut meta = AlbumMeta::default();
    let mut cover_url: Option<String> = None;

    if let Ok(resp) = client.get(&url).send().await {
        if let Ok(v) = resp.json::<serde_json::Value>().await {
            if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
                if let Some(first) = arr.first() {
                    meta.album = first
                        .get("collectionName")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    meta.album_artist = first
                        .get("artistName")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    meta.release_year = first
                        .get("releaseDate")
                        .and_then(|x| x.as_str())
                        .map(|s| s.chars().take(4).collect());
                    meta.genre = first
                        .get("primaryGenreName")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    // artworkUrl100 → upscale to 500
                    if let Some(art) = first.get("artworkUrl100").and_then(|x| x.as_str()) {
                        cover_url = Some(art.replace("100x100", "500x500"));
                    }
                }
            }
        }
    }

    // MusicBrainz fallback for cover
    if cover_url.is_none() {
        let q = format!("recording:\"{}\" AND artist:\"{}\"", title, artist);
        let mb = format!(
            "https://musicbrainz.org/ws/2/recording?query={}&fmt=json&limit=1",
            urlencode(&q)
        );
        if let Ok(resp) = client.get(&mb).send().await {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                if let Some(rec) = v
                    .get("recordings")
                    .and_then(|r| r.as_array())
                    .and_then(|a| a.first())
                {
                    if let Some(releases) = rec.get("releases").and_then(|r| r.as_array()) {
                        if let Some(rel) = releases.first() {
                            if let Some(mbid) = rel.get("id").and_then(|i| i.as_str()) {
                                cover_url = Some(format!(
                                    "https://coverartarchive.org/release/{}/front-500",
                                    mbid
                                ));
                            }
                            if meta.album.is_none() {
                                meta.album = rel
                                    .get("title")
                                    .and_then(|x| x.as_str())
                                    .map(String::from);
                            }
                        }
                    }
                }
            }
        }
    }

    // Download cover
    if let Some(url) = cover_url {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes().await {
                    let out = song_dir.join("cover.jpg");
                    if std::fs::write(&out, &bytes).is_ok() {
                        meta.cover_path = Some(out.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    meta
}
