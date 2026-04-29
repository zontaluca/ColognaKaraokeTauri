use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreEntry {
    pub id: Option<i64>,
    pub song_dir: String,
    pub song_title: String,
    pub player_name: String,
    pub score: i64,
    pub hits: i64,
    pub partials: i64,
    pub misses: i64,
    pub created_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_id: Option<i64>,
    /// JSON-encoded list of player ids for duets/group sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_ids: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo_path: Option<String>,
}

pub type DbState = Arc<Mutex<Connection>>;

pub fn init(app: &AppHandle) -> Result<(), String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let db_path = base.join("leaderboard.db");
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS scores (
            id INTEGER PRIMARY KEY,
            song_dir TEXT NOT NULL,
            song_title TEXT NOT NULL,
            player_name TEXT NOT NULL,
            score INTEGER NOT NULL,
            hits INTEGER NOT NULL DEFAULT 0,
            partials INTEGER NOT NULL DEFAULT 0,
            misses INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_song_score ON scores(song_dir, score DESC);
        CREATE INDEX IF NOT EXISTS idx_global_score ON scores(score DESC);
        "#,
    )
    .map_err(|e| e.to_string())?;
    crate::players::init_schema(&conn)?;
    let state: DbState = Arc::new(Mutex::new(conn));
    app.manage(state);
    Ok(())
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn leaderboard_insert(entry: ScoreEntry, db: State<'_, DbState>) -> Result<i64, String> {
    let conn = db.lock();
    conn.execute(
        "INSERT INTO scores (song_dir, song_title, player_name, score, hits, partials, misses, created_at, player_id, player_ids)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            entry.song_dir,
            entry.song_title,
            entry.player_name,
            entry.score,
            entry.hits,
            entry.partials,
            entry.misses,
            now_secs(),
            entry.player_id,
            entry.player_ids,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<ScoreEntry> {
    Ok(ScoreEntry {
        id: row.get(0)?,
        song_dir: row.get(1)?,
        song_title: row.get(2)?,
        player_name: row.get(3)?,
        score: row.get(4)?,
        hits: row.get(5)?,
        partials: row.get(6)?,
        misses: row.get(7)?,
        created_at: row.get(8)?,
        player_id: row.get(9).ok(),
        player_ids: row.get(10).ok(),
        photo_path: row.get(11).ok(),
    })
}

pub fn row_to_entry_pub(row: &rusqlite::Row) -> rusqlite::Result<ScoreEntry> {
    row_to_entry(row)
}

const SELECT_COLS: &str =
    "s.id, s.song_dir, s.song_title, s.player_name, s.score, s.hits, s.partials, s.misses, s.created_at, s.player_id, s.player_ids, p.photo_path";

#[tauri::command]
pub fn leaderboard_top(
    song_dir: String,
    limit: i64,
    db: State<'_, DbState>,
) -> Result<Vec<ScoreEntry>, String> {
    let conn = db.lock();
    let sql = format!(
        "SELECT {cols} FROM scores s LEFT JOIN players p ON p.id = s.player_id
         WHERE s.song_dir = ?1 ORDER BY s.score DESC, s.created_at ASC LIMIT ?2",
        cols = SELECT_COLS
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![song_dir, limit], row_to_entry)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn leaderboard_global_top(
    limit: i64,
    db: State<'_, DbState>,
) -> Result<Vec<ScoreEntry>, String> {
    let conn = db.lock();
    let sql = format!(
        "SELECT {cols} FROM scores s LEFT JOIN players p ON p.id = s.player_id
         ORDER BY s.score DESC, s.created_at ASC LIMIT ?1",
        cols = SELECT_COLS
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit], row_to_entry)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}
