use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

/// Convert whisper-apr TranscriptionResult JSON segments → [{word, start_ms, end_ms}].
/// Handles both per-word segments (when --split-on-word used) and multi-word segments
/// (interpolates timing proportionally by character count).
fn transcription_to_words(result: &serde_json::Value) -> Result<Vec<serde_json::Value>, String> {
    let segments = result["segments"]
        .as_array()
        .ok_or("missing segments in whisper-apr output")?;

    let mut words: Vec<serde_json::Value> = Vec::new();
    for seg in segments {
        let text = seg["text"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() {
            continue;
        }
        // Skip whisper hallucination segments: "[Music]", "[BLANK_AUDIO]", etc.
        if text.starts_with('[') && text.ends_with(']') {
            continue;
        }
        let start_s = seg["start"].as_f64().unwrap_or(0.0);
        let end_s = seg["end"].as_f64().unwrap_or(start_s);
        let duration = (end_s - start_s).max(0.0);

        let tokens: Vec<&str> = text.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        let total_chars: usize = tokens.iter().map(|w| w.len()).sum::<usize>().max(1);
        let mut t = start_s;

        for w in &tokens {
            let word_duration = duration * (w.len() as f64 / total_chars as f64);
            let ws = t;
            let we = t + word_duration;
            t = we;
            // Strip leading/trailing punctuation but keep apostrophes and hyphens
            let clean: String = w
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
                .to_string();
            if clean.is_empty() {
                continue;
            }
            words.push(serde_json::json!({
                "word": clean,
                "start_ms": (ws * 1000.0) as u64,
                "end_ms": (we * 1000.0) as u64,
            }));
        }
    }

    if words.is_empty() {
        return Err("whisper-apr produced no words".into());
    }
    Ok(words)
}

/// Run word-level alignment via whisper-apr Rust sidecar (pure Rust, no Python required).
/// Uses base model. On first run, the model is downloaded automatically (~150 MB).
async fn run_alignment_rust(
    app: &AppHandle,
    dir: &Path,
) -> Result<serde_json::Value, String> {
    let vocals_path = dir.join("vocals.wav");
    let words_path = dir.join("words.json");

    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        return serde_json::from_str(&s).map_err(|e| e.to_string());
    }

    if !vocals_path.exists() {
        return Err("vocals.wav not found — re-process song to generate it".into());
    }

    let args: Vec<String> = vec![
        "transcribe".into(),
        "-f".into(),
        vocals_path.to_string_lossy().into_owned(),
        "--model".into(),
        "small".into(),
        "--split-on-word".into(),
        "--no-prints".into(),
        "-o".into(),
        "json".into(),
    ];

    eprintln!("[aligner/whisper-apr] invoking sidecar with args: {:?}", args);
    let out = app
        .shell()
        .sidecar("whisper-apr")
        .map_err(|e| format!("whisper-apr sidecar: {}", e))?
        .args(args)
        .output()
        .await
        .map_err(|e| format!("whisper-apr exec: {}", e))?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!(
        "[aligner/whisper-apr] exit={:?} stdout_len={} stderr_len={}",
        out.status.code(),
        stdout.len(),
        stderr.len()
    );
    if !stderr.is_empty() {
        eprintln!("[aligner/whisper-apr] stderr:\n{}", &stderr[..stderr.len().min(4000)]);
    }

    if !out.status.success() {
        return Err(format!(
            "whisper-apr failed (exit {:?}).\nstderr: {}\nstdout: {}",
            out.status.code(),
            &stderr[..stderr.len().min(2000)],
            &stdout[..stdout.len().min(2000)]
        ));
    }

    let result: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("parse whisper-apr JSON: {} (stdout: {})", e, &stdout[..stdout.len().min(200)]))?;

    let words_array = transcription_to_words(&result)?;
    let words_json = serde_json::to_string_pretty(&words_array).map_err(|e| e.to_string())?;
    std::fs::write(&words_path, &words_json).map_err(|e| e.to_string())?;

    Ok(serde_json::Value::Array(words_array))
}

/// Build words.json from LRC text using proportional timing between lines.
/// Used when LRC is available — preserves the original language and avoids
/// whisper transcribing/translating into English.
fn build_words_from_lrc(lrc: &str, dir: &Path) -> Result<serde_json::Value, String> {
    let words_path = dir.join("words.json");
    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        return serde_json::from_str(&s).map_err(|e| e.to_string());
    }

    let lines = crate::lyrics::parse_lrc(lrc);
    if lines.is_empty() {
        return Err("LRC has no timestamped lines".into());
    }

    let mut words: Vec<serde_json::Value> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let line_start = line.ts_ms;
        let line_end = lines.get(i + 1).map(|l| l.ts_ms).unwrap_or(line_start + 4000);
        let duration = line_end.saturating_sub(line_start).max(1);

        let tokens: Vec<&str> = line.text.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let n = tokens.len() as u64;
        for (j, word) in tokens.iter().enumerate() {
            let start_ms = line_start + (j as u64 * duration) / n;
            let end_ms = line_start + ((j as u64 + 1) * duration) / n;
            words.push(serde_json::json!({
                "word": word,
                "start_ms": start_ms,
                "end_ms": end_ms,
            }));
        }
    }

    if words.is_empty() {
        return Err("LRC produced no words".into());
    }

    let json = serde_json::to_string_pretty(&words).map_err(|e| e.to_string())?;
    std::fs::write(&words_path, &json).map_err(|e| e.to_string())?;
    Ok(serde_json::Value::Array(words))
}

/// Use LRC when available (preserves original language); fall back to whisper when absent.
pub async fn run_alignment(
    app: &AppHandle,
    dir: &Path,
    lrc: Option<&str>,
) -> Result<serde_json::Value, String> {
    if let Some(lrc_text) = lrc.filter(|t| t.contains('[')) {
        return build_words_from_lrc(lrc_text, dir);
    }
    run_alignment_rust(app, dir).await
}

#[tauri::command]
pub fn get_words(dir: String) -> Result<serde_json::Value, String> {
    let words_path = PathBuf::from(&dir).join("words.json");
    if words_path.exists() {
        let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::Value::Null)
    }
}

#[tauri::command]
pub fn get_cookies_file(app: AppHandle) -> Option<String> {
    crate::settings::load_settings(&app).cookies_file
}

#[tauri::command]
pub fn set_cookies_file(app: AppHandle, path: Option<String>) -> Result<(), String> {
    let mut settings = crate::settings::load_settings(&app);
    settings.cookies_file = path;
    crate::settings::save_settings(&app, &settings)
}

#[tauri::command]
pub fn get_cookie_browser(app: AppHandle) -> String {
    match crate::settings::load_settings(&app).cookie_browser {
        crate::settings::CookieBrowser::None => "none".into(),
        crate::settings::CookieBrowser::Safari => "safari".into(),
        crate::settings::CookieBrowser::Chrome => "chrome".into(),
        crate::settings::CookieBrowser::Firefox => "firefox".into(),
        crate::settings::CookieBrowser::Chromium => "chromium".into(),
    }
}

#[tauri::command]
pub fn set_cookie_browser(app: AppHandle, browser: String) -> Result<(), String> {
    let mut settings = crate::settings::load_settings(&app);
    settings.cookie_browser = match browser.as_str() {
        "safari" => crate::settings::CookieBrowser::Safari,
        "chrome" => crate::settings::CookieBrowser::Chrome,
        "firefox" => crate::settings::CookieBrowser::Firefox,
        "chromium" => crate::settings::CookieBrowser::Chromium,
        _ => crate::settings::CookieBrowser::None,
    };
    crate::settings::save_settings(&app, &settings)
}

