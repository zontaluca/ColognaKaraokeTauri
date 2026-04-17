use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

pub fn library_dir(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    base.join("library")
}

pub fn ensure_library(app: &AppHandle) -> Result<(), String> {
    let dir = library_dir(app);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn song_dir(app: &AppHandle, safe_name: &str) -> PathBuf {
    library_dir(app).join(safe_name)
}

pub fn save_metadata(dir: &Path, meta: &serde_json::Value) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join("metadata.json");
    let json = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

fn load_metadata(dir: &Path) -> Option<serde_json::Value> {
    let path = dir.join("metadata.json");
    let s = fs::read_to_string(path).ok()?;
    serde_json::from_str(&s).ok()
}

#[tauri::command]
pub fn scan_library(app: AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let dir = library_dir(&app);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut songs: Vec<serde_json::Value> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(mut meta) = load_metadata(&path) {
            if let Some(obj) = meta.as_object_mut() {
                obj.insert(
                    "_dir".to_string(),
                    serde_json::Value::String(path.to_string_lossy().into_owned()),
                );
                obj.insert(
                    "has_instrumental".to_string(),
                    serde_json::Value::Bool(path.join("instrumental.mp3").exists() || path.join("instrumental.wav").exists()),
                );
                obj.insert(
                    "has_original".to_string(),
                    serde_json::Value::Bool(path.join("original.mp3").exists()),
                );
                let cover = path.join("cover.jpg");
                if cover.exists() && !obj.contains_key("cover_path") {
                    obj.insert(
                        "cover_path".to_string(),
                        serde_json::Value::String(cover.to_string_lossy().into_owned()),
                    );
                }
            }
            songs.push(meta);
        }
    }

    songs.sort_by(|a, b| {
        let ta = a.get("title").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        let tb = b.get("title").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        ta.cmp(&tb)
    });
    Ok(songs)
}

#[tauri::command]
pub fn delete_song(dir: String) -> Result<(), String> {
    let p = PathBuf::from(&dir);
    if p.exists() {
        fs::remove_dir_all(p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_library_dir(app: AppHandle) -> Result<String, String> {
    Ok(library_dir(&app).to_string_lossy().into_owned())
}

#[tauri::command]
pub fn get_song_audio_path(dir: String, prefer_instrumental: bool) -> Result<String, String> {
    let p = PathBuf::from(&dir);
    if prefer_instrumental {
        for name in ["instrumental.mp3", "instrumental.wav"] {
            let c = p.join(name);
            if c.exists() {
                return Ok(c.to_string_lossy().into_owned());
            }
        }
    }
    for name in ["original.mp3", "original.m4a", "original.wav"] {
        let c = p.join(name);
        if c.exists() {
            return Ok(c.to_string_lossy().into_owned());
        }
    }
    Err("No audio file found".into())
}
