use std::path::PathBuf;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::leaderboard::DbState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: i64,
    pub name: String,
    pub photo_path: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStats {
    pub total_plays: i64,
    pub best_score: i64,
    pub avg_score: f64,
    pub songs_played: i64,
}

pub fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS players (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            photo_path TEXT,
            created_at INTEGER NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    // Best-effort additive migrations on the scores table.
    let _ = conn.execute("ALTER TABLE scores ADD COLUMN player_id INTEGER", []);
    let _ = conn.execute("ALTER TABLE scores ADD COLUMN player_ids TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_scores_player ON scores(player_id, score DESC)",
        [],
    );
    Ok(())
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn photos_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let p = base.join("players");
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

fn row_to_player(row: &rusqlite::Row) -> rusqlite::Result<Player> {
    Ok(Player {
        id: row.get(0)?,
        name: row.get(1)?,
        photo_path: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn copy_photo_into_storage(
    app: &AppHandle,
    player_id: i64,
    source: &str,
) -> Result<String, String> {
    let src = PathBuf::from(source);
    if !src.exists() {
        return Err(format!("photo not found: {}", source));
    }
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_lowercase();
    let dir = photos_dir(app)?;
    // Remove any prior photo for this id (any extension).
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if s.starts_with(&format!("{}.", player_id)) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let dest = dir.join(format!("{}.{}", player_id, ext));
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

#[tauri::command]
pub fn players_list(db: State<'_, DbState>) -> Result<Vec<Player>, String> {
    let conn = db.lock();
    let mut stmt = conn
        .prepare("SELECT id, name, photo_path, created_at FROM players ORDER BY name COLLATE NOCASE ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_player)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn players_get(id: i64, db: State<'_, DbState>) -> Result<Option<Player>, String> {
    let conn = db.lock();
    let mut stmt = conn
        .prepare("SELECT id, name, photo_path, created_at FROM players WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![id], row_to_player)
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(r) => Ok(Some(r.map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

#[tauri::command]
pub fn players_create(
    app: AppHandle,
    name: String,
    photo_path: Option<String>,
    db: State<'_, DbState>,
) -> Result<Player, String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err("name is required".into());
    }
    let conn = db.lock();
    conn.execute(
        "INSERT INTO players (name, photo_path, created_at) VALUES (?1, NULL, ?2)",
        params![trimmed, now_secs()],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    drop(conn);

    let stored_photo = if let Some(src) = photo_path.as_ref().filter(|s| !s.is_empty()) {
        match copy_photo_into_storage(&app, id, src) {
            Ok(p) => {
                let conn = db.lock();
                conn.execute(
                    "UPDATE players SET photo_path = ?1 WHERE id = ?2",
                    params![p, id],
                )
                .map_err(|e| e.to_string())?;
                Some(p)
            }
            Err(e) => {
                eprintln!("player photo copy failed: {}", e);
                None
            }
        }
    } else {
        None
    };

    Ok(Player {
        id,
        name: trimmed,
        photo_path: stored_photo,
        created_at: now_secs(),
    })
}

#[tauri::command]
pub fn players_update(
    app: AppHandle,
    id: i64,
    name: Option<String>,
    photo_path: Option<String>,
    clear_photo: Option<bool>,
    db: State<'_, DbState>,
) -> Result<Player, String> {
    if let Some(n) = name.as_ref() {
        let trimmed = n.trim();
        if trimmed.is_empty() {
            return Err("name cannot be empty".into());
        }
        let conn = db.lock();
        conn.execute(
            "UPDATE players SET name = ?1 WHERE id = ?2",
            params![trimmed, id],
        )
        .map_err(|e| e.to_string())?;
    }
    if clear_photo.unwrap_or(false) {
        let dir = photos_dir(&app)?;
        if let Ok(read) = std::fs::read_dir(&dir) {
            for entry in read.flatten() {
                let s = entry.file_name().to_string_lossy().to_string();
                if s.starts_with(&format!("{}.", id)) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let conn = db.lock();
        conn.execute(
            "UPDATE players SET photo_path = NULL WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
    } else if let Some(src) = photo_path.as_ref().filter(|s| !s.is_empty()) {
        let p = copy_photo_into_storage(&app, id, src)?;
        let conn = db.lock();
        conn.execute(
            "UPDATE players SET photo_path = ?1 WHERE id = ?2",
            params![p, id],
        )
        .map_err(|e| e.to_string())?;
    }

    players_get(id, db)?.ok_or_else(|| "player not found".into())
}

#[tauri::command]
pub fn players_delete(app: AppHandle, id: i64, db: State<'_, DbState>) -> Result<(), String> {
    let conn = db.lock();
    conn.execute("DELETE FROM players WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    drop(conn);
    if let Ok(dir) = photos_dir(&app) {
        if let Ok(read) = std::fs::read_dir(&dir) {
            for entry in read.flatten() {
                let s = entry.file_name().to_string_lossy().to_string();
                if s.starts_with(&format!("{}.", id)) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn players_stats(id: i64, db: State<'_, DbState>) -> Result<PlayerStats, String> {
    let conn = db.lock();
    let row: (i64, Option<i64>, Option<f64>, i64) = conn
        .query_row(
            "SELECT COUNT(*), MAX(score), AVG(score), COUNT(DISTINCT song_dir)
             FROM scores WHERE player_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| e.to_string())?;
    Ok(PlayerStats {
        total_plays: row.0,
        best_score: row.1.unwrap_or(0),
        avg_score: row.2.unwrap_or(0.0),
        songs_played: row.3,
    })
}

#[tauri::command]
pub fn players_history(
    id: i64,
    limit: i64,
    db: State<'_, DbState>,
) -> Result<Vec<crate::leaderboard::ScoreEntry>, String> {
    let conn = db.lock();
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.song_dir, s.song_title, s.player_name, s.score, s.hits, s.partials, s.misses, s.created_at, s.player_id, s.player_ids, p.photo_path
             FROM scores s LEFT JOIN players p ON p.id = s.player_id
             WHERE s.player_id = ?1 ORDER BY s.created_at DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![id, limit], crate::leaderboard::row_to_entry_pub)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}
