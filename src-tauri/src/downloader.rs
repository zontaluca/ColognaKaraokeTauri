use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadResult {
    pub title: String,
    pub artist: String,
    pub duration_sec: u64,
    pub youtube_url: String,
    pub safe_name: String,
    pub audio_path: String,
}

fn safe(s: &str) -> String {
    let re = Regex::new(r"[^\w\s-]").unwrap();
    re.replace_all(s, "").trim().to_string()
}

pub async fn download_audio(
    app: &AppHandle,
    url: &str,
    output_dir: &Path,
    mut on_progress: impl FnMut(&str, f32),
) -> Result<DownloadResult, String> {
    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    on_progress("Fetching video info...", 0.0);

    // Get JSON metadata
    let sidecar = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| e.to_string())?
        .args(["--dump-json", "--no-download", "--no-playlist", url]);

    let out = sidecar.output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("yt-dlp info failed: {}", stderr));
    }
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let info: serde_json::Value =
        serde_json::from_str(stdout.lines().next().unwrap_or("{}")).map_err(|e| e.to_string())?;

    let title = info
        .get("track")
        .and_then(|v| v.as_str())
        .or_else(|| info.get("title").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_string();

    let artist = info
        .get("artist")
        .and_then(|v| v.as_str())
        .or_else(|| info.get("uploader").and_then(|v| v.as_str()))
        .or_else(|| info.get("channel").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_string();

    let duration = info.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;

    let safe_name = format!("{}_{}", safe(&artist), safe(&title));
    let output_template = output_dir.join("original.%(ext)s");
    let output_template_str = output_template.to_string_lossy().to_string();

    on_progress("Downloading audio...", 0.1);

    let percent_re = Regex::new(r"(\d{1,3}(?:\.\d+)?)%").unwrap();
    let (mut rx, _child) = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| e.to_string())?
        .args([
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "-o",
            &output_template_str,
            "--no-playlist",
            "--newline",
            url,
        ])
        .spawn()
        .map_err(|e| e.to_string())?;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                let line = String::from_utf8_lossy(&bytes).to_string();
                if let Some(m) = percent_re.captures_iter(&line).last() {
                    if let Ok(p) = m[1].parse::<f32>() {
                        on_progress(&format!("Downloading... {:.0}%", p), (p / 100.0).clamp(0.0, 1.0));
                    }
                }
            }
            CommandEvent::Error(err) => return Err(err),
            CommandEvent::Terminated(payload) => {
                if payload.code.unwrap_or(-1) != 0 {
                    return Err(format!("yt-dlp exited with code {:?}", payload.code));
                }
                break;
            }
            _ => {}
        }
    }

    let audio_path: PathBuf = output_dir.join("original.mp3");
    if !audio_path.exists() {
        return Err(format!(
            "Expected output not found: {}",
            audio_path.display()
        ));
    }

    on_progress("Download complete", 1.0);

    Ok(DownloadResult {
        title,
        artist,
        duration_sec: duration,
        youtube_url: url.to_string(),
        safe_name,
        audio_path: audio_path.to_string_lossy().into_owned(),
    })
}
