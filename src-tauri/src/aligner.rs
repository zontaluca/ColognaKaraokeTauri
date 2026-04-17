use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

fn lrc_to_plain_text(lrc: &str) -> String {
    lrc.lines()
        .filter_map(|line| {
            let line = line.trim();
            if let Some(pos) = line.find(']') {
                let plain = line[pos + 1..].trim();
                if !plain.is_empty() { Some(plain.to_string()) } else { None }
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run word-level alignment via bundled stable_whisper sidecar (`aligner`).
/// When `lrc` is provided, uses stable-ts forced alignment. Otherwise, whisper transcription.
pub async fn run_alignment(
    app: &AppHandle,
    dir: &Path,
    lrc: Option<&str>,
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

    let plain = lrc.map(lrc_to_plain_text).filter(|t| !t.is_empty());
    let mut args: Vec<String> = vec![
        "--audio".into(),
        vocals_path.to_string_lossy().into_owned(),
        "--out".into(),
        words_path.to_string_lossy().into_owned(),
        "--model".into(),
        "tiny".into(),
    ];
    if let Some(text) = plain.as_ref() {
        // Write lyrics to a temp file to avoid argv size limits
        let text_file = dir.join("_align_text.tmp");
        std::fs::write(&text_file, text).map_err(|e| e.to_string())?;
        args.push("--text-file".into());
        args.push(text_file.to_string_lossy().into_owned());
    }

    eprintln!("[aligner] invoking sidecar with args: {:?}", args);
    let out = app
        .shell()
        .sidecar("aligner")
        .map_err(|e| format!("aligner sidecar: {}", e))?
        .args(args)
        .output()
        .await
        .map_err(|e| format!("aligner exec: {}", e))?;

    let _ = std::fs::remove_file(dir.join("_align_text.tmp"));

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let code = out.status.code();
    eprintln!(
        "[aligner] exit={:?} stdout_len={} stderr_len={}",
        code,
        stdout.len(),
        stderr.len()
    );
    if !stdout.is_empty() {
        eprintln!("[aligner] stdout:\n{}", &stdout[..stdout.len().min(4000)]);
    }
    if !stderr.is_empty() {
        eprintln!("[aligner] stderr:\n{}", &stderr[..stderr.len().min(4000)]);
    }

    if !out.status.success() {
        return Err(format!(
            "aligner failed (exit {:?}). stderr: {}\nstdout: {}",
            code,
            &stderr[..stderr.len().min(2000)],
            &stdout[..stdout.len().min(2000)]
        ));
    }

    if !words_path.exists() {
        return Err(format!(
            "aligner produced no words.json (exit {:?}).\nstderr: {}\nstdout: {}\nHINT: if binary is a 0-byte placeholder, run ./scripts/build-aligner.sh to build the real PyInstaller bundle.",
            code,
            &stderr[..stderr.len().min(1500)],
            &stdout[..stdout.len().min(1500)]
        ));
    }
    let s = std::fs::read_to_string(&words_path).map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
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
