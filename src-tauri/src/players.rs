use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::library::library_dir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntry {
    pub id: String,
    pub player_name: String,
    pub song_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveQueueState {
    pub entries: Vec<QueueEntry>,
    pub queue_idx: i32,
}

fn queue_path(app: &AppHandle) -> PathBuf {
    library_dir(app).join("players_queue.json")
}

fn active_queue_path(app: &AppHandle) -> PathBuf {
    library_dir(app).join("active_queue.json")
}

pub fn load_queue(app: &AppHandle) -> Vec<QueueEntry> {
    let path = queue_path(app);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_queue(app: &AppHandle, entries: &[QueueEntry]) -> Result<(), String> {
    let path = queue_path(app);
    let s = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn players_load(app: AppHandle) -> Vec<QueueEntry> {
    load_queue(&app)
}

#[tauri::command]
pub async fn players_save(app: AppHandle, players: Vec<QueueEntry>) -> Result<(), String> {
    save_queue(&app, &players)
}

#[tauri::command]
pub async fn active_queue_load(app: AppHandle) -> Option<ActiveQueueState> {
    let path = active_queue_path(&app);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

#[tauri::command]
pub async fn active_queue_save(app: AppHandle, state: Option<ActiveQueueState>) -> Result<(), String> {
    let path = active_queue_path(&app);
    match state {
        None => { let _ = std::fs::remove_file(&path); Ok(()) }
        Some(s) => {
            let json = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| e.to_string())
        }
    }
}
