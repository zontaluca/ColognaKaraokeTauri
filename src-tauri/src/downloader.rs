use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Find a JS runtime for yt-dlp's signature solving.
/// Returns e.g. `"deno:/opt/homebrew/bin/deno"` or `"nodejs:/usr/local/bin/node"`.
fn find_js_runtime() -> Option<String> {
    let deno_paths = [
        "/opt/homebrew/bin/deno",
        "/usr/local/bin/deno",
        "/usr/bin/deno",
    ];
    for p in &deno_paths {
        if std::fs::metadata(p).is_ok() {
            return Some(format!("deno:{}", p));
        }
    }
    // Try PATH via `which`
    if let Ok(out) = Command::new("which").arg("deno").output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() && std::fs::metadata(&s).is_ok() {
            return Some(format!("deno:{}", s));
        }
    }

    let node_paths = [
        "/opt/homebrew/bin/node",
        "/usr/local/bin/node",
        "/usr/bin/node",
    ];
    for p in &node_paths {
        if std::fs::metadata(p).is_ok() {
            return Some(format!("nodejs:{}", p));
        }
    }
    if let Ok(out) = Command::new("which").arg("node").output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() && std::fs::metadata(&s).is_ok() {
            return Some(format!("nodejs:{}", s));
        }
    }

    None
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

    let settings = crate::settings::load_settings(app);

    // cookies_file takes priority; fallback to browser extraction
    enum CookieMode { File(String), Browser(String), None }
    let cookie_mode = if let Some(ref f) = settings.cookies_file {
        CookieMode::File(f.clone())
    } else if let Some(b) = settings.cookie_browser.as_str() {
        CookieMode::Browser(b.to_string())
    } else {
        CookieMode::None
    };

    let js_runtime = find_js_runtime();

    let mut info_args: Vec<String> = vec![
        "--dump-json".into(), "--no-download".into(), "--no-playlist".into(),
    ];
    if let Some(ref rt) = js_runtime {
        info_args.push("--js-runtimes".into());
        info_args.push(rt.clone());
    }
    match &cookie_mode {
        CookieMode::File(f) => { info_args.push("--cookies".into()); info_args.push(f.clone()); }
        CookieMode::Browser(b) => { info_args.push("--cookies-from-browser".into()); info_args.push(b.clone()); }
        CookieMode::None => {}
    }
    info_args.push(url.to_string());

    if js_runtime.is_none() {
        eprintln!("[downloader] WARNING: no JS runtime found — install deno with: brew install deno");
    }

    // Get JSON metadata
    let sidecar = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| e.to_string())?
        .args(info_args);

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
    let mut dl_args: Vec<String> = vec![
        "-x".into(),
        "--audio-format".into(), "mp3".into(),
        "--audio-quality".into(), "0".into(),
        "-o".into(), output_template_str.clone(),
        "--no-playlist".into(),
        "--newline".into(),
    ];
    if let Some(ref rt) = js_runtime {
        dl_args.push("--js-runtimes".into());
        dl_args.push(rt.clone());
    }
    match &cookie_mode {
        CookieMode::File(f) => { dl_args.push("--cookies".into()); dl_args.push(f.clone()); }
        CookieMode::Browser(b) => { dl_args.push("--cookies-from-browser".into()); dl_args.push(b.clone()); }
        CookieMode::None => {}
    }
    dl_args.push(url.to_string());

    let (mut rx, _child) = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| e.to_string())?
        .args(dl_args)
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
