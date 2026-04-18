use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::aligner::run_alignment;
use crate::downloader::download_audio;
use crate::library::{ensure_library, library_dir, save_metadata, song_dir};
use crate::lyrics::{fetch_lyrics, parse_lrc, shift_lrc};
use crate::metadata::fetch_album_meta;
use crate::pitch::precompute_reference_pitch;
use crate::recognizer::recognize_song;
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

    // Step 0b — Shazam recognition (best-effort; overrides YouTube title/artist if found)
    on_progress(1, "active", "Recognizing song...", 0.21);
    let (song_title, song_artist) =
        match recognize_song(std::path::Path::new(&download.audio_path)).await {
            Some(info) => {
                eprintln!(
                    "[pipeline] Shazam match: {} — {}",
                    info.artist, info.title
                );
                (info.title, info.artist)
            }
            None => {
                eprintln!("[pipeline] Shazam: no match, using YouTube metadata");
                (download.title.clone(), download.artist.clone())
            }
        };

    // Step 1 — lyrics
    on_progress(1, "active", "Fetching lyrics...", 0.22);
    let lrc = fetch_lyrics(&song_title, &song_artist, Some(download.duration_sec)).await;
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
    let album_meta = fetch_album_meta(&song_title, &song_artist, &final_song_dir).await;
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
    let words_result = run_alignment(&app, &final_song_dir, lrc_for_align.as_deref()).await;
    match &words_result {
        Ok(_) => on_progress(4, "done", "Words aligned", 0.88),
        Err(e) => {
            on_progress(4, "error", e, 0.0);
            return Err(e.clone());
        }
    }

    // Correct LRC timing using vocal onset from aligner (fixes video intro offset)
    let lrc = lrc.and_then(|lrc_text| {
        // Only shift synced LRC (plain lyrics have no timestamps)
        if !lrc_text.contains('[') {
            return Some(lrc_text);
        }
        let words_val = words_result.as_ref().ok()?;
        let first_vocal_ms = words_val.as_array()?.first()?["start_ms"].as_u64()?;
        let lrc_lines = parse_lrc(&lrc_text);
        let first_lrc_ms = lrc_lines.first()?.ts_ms;
        let offset_ms = first_vocal_ms as i64 - first_lrc_ms as i64;
        // Only shift LRC forward (positive): video has a longer intro than the LRC expects.
        // A negative offset means whisper reported t≈0 for hallucinated intro segments — never shift backward.
        if offset_ms > 3000 {
            eprintln!(
                "[pipeline] LRC offset correction: +{}ms (vocal_onset={}ms, first_lrc={}ms)",
                offset_ms, first_vocal_ms, first_lrc_ms
            );
            Some(shift_lrc(&lrc_text, offset_ms))
        } else {
            Some(lrc_text)
        }
    });

    // Step 5 — pitch contour (non-fatal)
    on_progress(5, "active", "Computing pitch...", 0.90);
    match precompute_reference_pitch(&final_song_dir).await {
        Ok(_) => on_progress(5, "done", "Pitch contour cached", 0.96),
        Err(e) => on_progress(5, "done", &format!("Pitch skipped: {}", e), 0.96),
    }

    // Step 6 — save metadata
    on_progress(6, "active", "Saving to library...", 0.97);
    let mut meta = serde_json::json!({
        "title": song_title,
        "artist": song_artist,
        "duration_sec": download.duration_sec,
        "youtube_url": download.youtube_url,
        "youtube_title": download.title,
        "youtube_artist": download.artist,
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

/// Re-run the processing pipeline for an already-downloaded song.
/// Skips download and vocal separation if vocals.wav already exists.
/// Always re-fetches lyrics and re-runs alignment (deletes words.json first).
pub async fn run_reprocess<F>(
    app: AppHandle,
    dir: String,
    mut on_progress: F,
) -> Result<serde_json::Value, String>
where
    F: FnMut(usize, &str, &str, f32) + Send + Sync + Clone + 'static,
{
    let dir = PathBuf::from(&dir);

    let meta_str = std::fs::read_to_string(dir.join("metadata.json"))
        .map_err(|e| format!("metadata.json not found: {}", e))?;
    let existing_meta: serde_json::Value =
        serde_json::from_str(&meta_str).map_err(|e| format!("parse metadata.json: {}", e))?;

    let song_title = existing_meta["title"].as_str().unwrap_or("").to_string();
    let song_artist = existing_meta["artist"].as_str().unwrap_or("").to_string();
    let duration_sec = existing_meta["duration_sec"].as_u64();

    // Step 1 — re-fetch lyrics
    on_progress(1, "active", "Fetching lyrics...", 0.05);
    let lrc = fetch_lyrics(&song_title, &song_artist, duration_sec).await;
    on_progress(1, "done", if lrc.is_some() { "Lyrics found" } else { "No lyrics (continuing)" }, 0.15);

    // Step 2 — album art
    on_progress(2, "active", "Fetching album art...", 0.17);
    let album_meta = fetch_album_meta(&song_title, &song_artist, &dir).await;
    on_progress(2, "done", if album_meta.cover_path.is_some() { "Cover downloaded" } else { "No cover (continuing)" }, 0.22);

    // Step 3 — separate vocals (skip if vocals.wav already exists)
    if !dir.join("vocals.wav").exists() {
        let original = dir.join("original.mp3");
        if !original.exists() {
            return Err("original.mp3 not found — cannot re-process without re-downloading".into());
        }
        on_progress(3, "active", "Separating vocals...", 0.24);
        let mut cb = on_progress.clone();
        separate_vocals(&app, &original, &dir, move |msg, p| {
            cb(3, "active", msg, 0.24 + p * 0.50);
        })
        .await
        .map_err(|e| { on_progress(3, "error", &e, 0.0); e })?;
        on_progress(3, "done", "Vocals separated", 0.74);
    } else {
        on_progress(3, "done", "Vocals already separated", 0.22);
    }

    // Step 4 — delete words.json, re-align
    let _ = std::fs::remove_file(dir.join("words.json"));
    on_progress(4, "active", "Aligning words...", 0.76);
    let lrc_for_align = lrc.clone();
    let words_result = run_alignment(&app, &dir, lrc_for_align.as_deref()).await;
    match &words_result {
        Ok(_) => on_progress(4, "done", "Words aligned", 0.88),
        Err(e) => { on_progress(4, "error", e, 0.0); return Err(e.clone()); }
    }

    // Correct LRC timing using vocal onset
    let lrc = lrc.and_then(|lrc_text| {
        if !lrc_text.contains('[') { return Some(lrc_text); }
        let words_val = words_result.as_ref().ok()?;
        let first_vocal_ms = words_val.as_array()?.first()?["start_ms"].as_u64()?;
        let lrc_lines = parse_lrc(&lrc_text);
        let first_lrc_ms = lrc_lines.first()?.ts_ms;
        let offset_ms = first_vocal_ms as i64 - first_lrc_ms as i64;
        if offset_ms > 3000 {
            eprintln!("[reprocess] LRC offset correction: +{}ms", offset_ms);
            Some(shift_lrc(&lrc_text, offset_ms))
        } else {
            Some(lrc_text)
        }
    });

    // Step 5 — pitch (force recompute)
    let _ = std::fs::remove_file(dir.join("pitch.json"));
    on_progress(5, "active", "Computing pitch...", 0.90);
    match precompute_reference_pitch(&dir).await {
        Ok(_) => on_progress(5, "done", "Pitch contour cached", 0.96),
        Err(e) => on_progress(5, "done", &format!("Pitch skipped: {}", e), 0.96),
    }

    // Step 6 — merge and save metadata
    on_progress(6, "active", "Saving to library...", 0.97);
    let mut meta = existing_meta;
    let obj = meta.as_object_mut().unwrap();
    if let Some(lrc_text) = lrc {
        obj.insert("lrc".into(), lrc_text.into());
    } else {
        obj.remove("lrc");
    }
    if let Some(v) = album_meta.album { obj.insert("album".into(), v.into()); }
    if let Some(v) = album_meta.album_artist { obj.insert("album_artist".into(), v.into()); }
    if let Some(v) = album_meta.release_year { obj.insert("release_year".into(), v.into()); }
    if let Some(v) = album_meta.cover_path { obj.insert("cover_path".into(), v.into()); }
    if let Some(v) = album_meta.genre { obj.insert("genre".into(), v.into()); }
    save_metadata(&dir, &meta)?;

    on_progress(6, "done", "Done!", 1.0);
    Ok(meta)
}

#[tauri::command]
pub async fn reprocess_song(
    app: AppHandle,
    dir: String,
) -> Result<serde_json::Value, String> {
    let app2 = app.clone();
    run_reprocess(app, dir, move |step, status, message, progress| {
        let _ = tauri::Emitter::emit(
            &app2,
            "karaoke://reprocess-progress",
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
