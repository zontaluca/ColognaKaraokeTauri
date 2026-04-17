use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::aligner::run_alignment;
use crate::downloader::download_audio;
use crate::library::{ensure_library, library_dir, save_metadata, song_dir};
use crate::lyrics::fetch_lyrics;
use crate::metadata::fetch_album_meta;
use crate::pitch::precompute_reference_pitch;
use crate::separator::separate_vocals;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StageUpdate {
    pub step: usize,
    pub status: String,
    pub message: String,
    pub progress: f32,
}

/// Stage indices:
/// 0 = Download, 1 = Lyrics, 2 = Album art, 3 = Separate vocals,
/// 4 = Align words, 5 = Pitch contour, 6 = Save
pub const STAGES: &[&str] = &[
    "Download audio",
    "Fetch lyrics",
    "Fetch album art",
    "Separate vocals",
    "Align words",
    "Compute pitch",
    "Save",
];

/// Run full pipeline with progress callback (step, status, message, progress 0-1).
pub async fn run_pipeline<F>(
    app: AppHandle,
    url: String,
    mut on_progress: F,
) -> Result<serde_json::Value, String>
where
    F: FnMut(usize, &str, &str, f32) + Send + Sync + Clone + 'static,
{
    ensure_library(&app)?;
    let lib_dir = library_dir(&app);
    let temp_dir: PathBuf = lib_dir.join("_temp");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    // Step 0 — download
    on_progress(0, "active", "Downloading audio...", 0.0);
    let mut cb = on_progress.clone();
    let download = download_audio(&app, &url, &temp_dir, |msg, p| {
        cb(0, "active", msg, 0.02 + p * 0.18);
    })
    .await
    .map_err(|e| {
        on_progress(0, "error", &e, 0.0);
        e
    })?;
    on_progress(0, "done", "Audio downloaded", 0.20);

    // Step 1 — lyrics
    on_progress(1, "active", "Fetching lyrics...", 0.22);
    let lrc = fetch_lyrics(&download.title, &download.artist).await;
    on_progress(
        1,
        "done",
        if lrc.is_some() { "Lyrics found" } else { "No lyrics (continuing)" },
        0.28,
    );

    let final_song_dir = song_dir(&app, &download.safe_name);
    std::fs::create_dir_all(&final_song_dir).map_err(|e| e.to_string())?;

    // Step 2 — album art
    on_progress(2, "active", "Fetching album art...", 0.30);
    let album_meta =
        fetch_album_meta(&download.title, &download.artist, &final_song_dir).await;
    on_progress(
        2,
        "done",
        if album_meta.cover_path.is_some() { "Cover downloaded" } else { "No cover (continuing)" },
        0.34,
    );

    // Step 3 — separate
    on_progress(3, "active", "Separating vocals...", 0.36);
    let mut cb = on_progress.clone();
    let _instrumental = separate_vocals(
        &app,
        &PathBuf::from(&download.audio_path),
        &final_song_dir,
        move |msg, p| {
            cb(3, "active", msg, 0.36 + p * 0.40);
        },
    )
    .await
    .map_err(|e| {
        on_progress(3, "error", &e, 0.0);
        e
    })?;
    on_progress(3, "done", "Vocals separated", 0.76);

    // Move original into song dir
    let final_original = final_song_dir.join("original.mp3");
    let _ = std::fs::rename(&download.audio_path, &final_original);
    let _ = std::fs::remove_dir_all(&temp_dir);

    // Step 4 — align words (mandatory)
    on_progress(4, "active", "Aligning words...", 0.78);
    let lrc_for_align = lrc.clone();
    match run_alignment(&app, &final_song_dir, lrc_for_align.as_deref()).await {
        Ok(_) => on_progress(4, "done", "Words aligned", 0.88),
        Err(e) => {
            on_progress(4, "error", &e, 0.0);
            return Err(e);
        }
    }

    // Step 5 — pitch contour (non-fatal)
    on_progress(5, "active", "Computing pitch...", 0.90);
    match precompute_reference_pitch(&final_song_dir).await {
        Ok(_) => on_progress(5, "done", "Pitch contour cached", 0.96),
        Err(e) => on_progress(5, "done", &format!("Pitch skipped: {}", e), 0.96),
    }

    // Step 6 — save metadata
    on_progress(6, "active", "Saving to library...", 0.97);
    let mut meta = serde_json::json!({
        "title": download.title,
        "artist": download.artist,
        "duration_sec": download.duration_sec,
        "youtube_url": download.youtube_url,
    });
    if let Some(lrc_text) = lrc {
        meta.as_object_mut()
            .unwrap()
            .insert("lrc".into(), serde_json::Value::String(lrc_text));
    }
    let obj = meta.as_object_mut().unwrap();
    if let Some(v) = album_meta.album { obj.insert("album".into(), v.into()); }
    if let Some(v) = album_meta.album_artist { obj.insert("album_artist".into(), v.into()); }
    if let Some(v) = album_meta.release_year { obj.insert("release_year".into(), v.into()); }
    if let Some(v) = album_meta.cover_path { obj.insert("cover_path".into(), v.into()); }
    if let Some(v) = album_meta.genre { obj.insert("genre".into(), v.into()); }
    save_metadata(&final_song_dir, &meta)?;

    on_progress(6, "done", "Done!", 1.0);
    Ok(meta)
}

/// Legacy direct command (kept for compat). Prefer jobs_enqueue.
#[tauri::command]
pub async fn process_youtube_url(
    app: AppHandle,
    url: String,
) -> Result<serde_json::Value, String> {
    let app2 = app.clone();
    run_pipeline(app, url, move |step, status, message, progress| {
        let _ = tauri::Emitter::emit(
            &app2,
            "karaoke://progress",
            &StageUpdate {
                step,
                status: status.into(),
                message: message.into(),
                progress,
            },
        );
    })
    .await
}
