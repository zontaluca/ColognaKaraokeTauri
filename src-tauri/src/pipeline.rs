use std::path::{Path, PathBuf};
use std::sync::Arc;

use aligner_pipeline::AudioBuffer;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::aligner::{alignment_needed, run_alignment};
use crate::downloader::download_audio;
use crate::library::{ensure_library, library_dir, save_metadata, song_dir};
use crate::lyrics::fetch_lyrics;
use crate::metadata::{fetch_album_meta, AlbumMeta};
use crate::pitch::{has_reference_pitch, write_reference_pitch};
use crate::recognizer::recognize_song;
use crate::separator::separate_vocals;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StageUpdate {
    pub step: usize,
    pub status: String,
    pub message: String,
    pub progress: f32,
}

/// Decode vocals.mp3 once (native rate, mono) on a blocking thread.
async fn decode_vocals(dir: &Path) -> Result<Arc<AudioBuffer>, String> {
    let path = dir.join("vocals.mp3");
    if !path.exists() {
        return Err("vocals.mp3 not found".into());
    }
    let (samples, sample_rate) = tokio::task::spawn_blocking(move || crate::audio::load_wav_mono(&path))
        .await
        .map_err(|e| e.to_string())??;
    Ok(Arc::new(AudioBuffer { samples, sample_rate }))
}

/// Steps 4 + 5: word alignment and reference pitch contour. vocals.mp3 is
/// decoded once and shared by both, and the pitch contour (independent of the
/// alignment) is computed on its own thread while alignment runs.
/// Alignment errors are fatal; pitch errors are reported and skipped.
/// Returns the LRC generated from the word timings (with its source) when
/// `lrc` was plain text or missing.
async fn align_and_pitch<F>(
    dir: &Path,
    lrc: Option<&str>,
    on_progress: &mut F,
    align_start: f32,
) -> Result<Option<(String, &'static str)>, String>
where
    F: FnMut(usize, &str, &str, f32) + Send + Sync + Clone + 'static,
{
    on_progress(4, "active", "Aligning words...", align_start);

    let need_words = alignment_needed(dir, lrc);
    let need_pitch = !has_reference_pitch(dir);
    let mut decode_error: Option<String> = None;
    let vocals = if need_words || need_pitch {
        match decode_vocals(dir).await {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("[pipeline] vocals decode failed (non-fatal): {}", e);
                decode_error = Some(e);
                None
            }
        }
    } else {
        None
    };

    let pitch_job = if need_pitch {
        vocals.clone().map(|v| {
            let dir = dir.to_path_buf();
            tokio::task::spawn_blocking(move || write_reference_pitch(&dir, &v))
        })
    } else {
        None
    };

    let phrase_cb = std::sync::Mutex::new(on_progress.clone());
    let on_phrase = move |done: usize, total: usize| {
        if total == 0 {
            return;
        }
        let frac = (done as f32 / total as f32).clamp(0.0, 1.0);
        let progress = align_start + frac * (0.88 - align_start);
        let msg = format!("Aligning {}/{}", done, total);
        if let Ok(mut cb) = phrase_cb.lock() {
            cb(4, "active", &msg, progress);
        }
    };
    let words_result = run_alignment(dir, lrc, vocals, &on_phrase).await;
    let generated_lrc = match words_result {
        Ok(out) => {
            on_progress(4, "done", "Words aligned", 0.88);
            let source = out.lrc_source;
            out.generated_lrc.map(|lrc| (lrc, source))
        }
        Err(e) => {
            on_progress(4, "error", &e, 0.0);
            return Err(e);
        }
    };

    // Step 5 — pitch contour (non-fatal)
    on_progress(5, "active", "Computing pitch...", 0.90);
    let pitch_result: Result<(), String> = match pitch_job {
        Some(job) => job.await.map_err(|e| e.to_string()).and_then(|r| r),
        None if need_pitch => Err(decode_error.unwrap_or_else(|| "vocals.mp3 not found".into())),
        None => Ok(()),
    };
    match pitch_result {
        Ok(_) => on_progress(5, "done", "Pitch contour cached", 0.96),
        Err(e) => on_progress(5, "done", &format!("Pitch skipped: {}", e), 0.96),
    }
    Ok(generated_lrc)
}

/// Store the lyrics in metadata: the generated synced LRC when there is one
/// (flagged with `lrc_generated`), otherwise the fetched text.
fn apply_lyrics(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    fetched: Option<String>,
    generated: Option<(String, &'static str)>,
) {
    obj.remove("lrc_generated");
    match (generated, fetched) {
        (Some((g, source)), _) => {
            obj.insert("lrc".into(), g.into());
            obj.insert("lrc_generated".into(), source.into());
        }
        (None, Some(f)) => {
            obj.insert("lrc".into(), f.into());
        }
        (None, None) => {
            obj.remove("lrc");
        }
    }
}

fn apply_album_meta(obj: &mut serde_json::Map<String, serde_json::Value>, album_meta: AlbumMeta) {
    if let Some(v) = album_meta.album { obj.insert("album".into(), v.into()); }
    if let Some(v) = album_meta.album_artist { obj.insert("album_artist".into(), v.into()); }
    if let Some(v) = album_meta.release_year { obj.insert("release_year".into(), v.into()); }
    if let Some(v) = album_meta.cover_path { obj.insert("cover_path".into(), v.into()); }
    if let Some(v) = album_meta.genre { obj.insert("genre".into(), v.into()); }
}

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

    // Steps 4 + 5 — align words (mandatory) + pitch contour
    let generated_lrc = align_and_pitch(&final_song_dir, lrc.as_deref(), &mut on_progress, 0.78).await?;

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
    apply_lyrics(meta.as_object_mut().unwrap(), lrc, generated_lrc);
    apply_album_meta(meta.as_object_mut().unwrap(), album_meta);
    save_metadata(&final_song_dir, &meta)?;

    on_progress(6, "done", "Done!", 1.0);
    // Embed dir for post-pipeline hooks (jobs.rs auto-sync); not written to disk
    meta.as_object_mut().unwrap().insert(
        "_pipeline_dir".into(),
        final_song_dir.to_string_lossy().into_owned().into(),
    );
    Ok(meta)
}

/// Re-run the processing pipeline for an already-downloaded song.
/// Skips download and vocal separation if vocals.mp3 already exists.
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

    // Step 3 — separate vocals (skip if vocals.mp3 already exists)
    if !dir.join("vocals.mp3").exists() {
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

    // vad.json is no longer produced: drop the stale copy left by older versions.
    let _ = std::fs::remove_file(dir.join("vad.json"));

    // Steps 4 + 5 — delete words.json / pitch.json, re-align + recompute pitch
    let _ = std::fs::remove_file(dir.join("words.json"));
    let _ = std::fs::remove_file(dir.join("pitch.json"));
    let generated_lrc = align_and_pitch(&dir, lrc.as_deref(), &mut on_progress, 0.76).await?;

    // Step 6 — merge and save metadata
    on_progress(6, "active", "Saving to library...", 0.97);
    let mut meta = existing_meta;
    let obj = meta.as_object_mut().unwrap();
    apply_lyrics(obj, lrc, generated_lrc);
    apply_album_meta(obj, album_meta);
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
    let res = run_reprocess(app.clone(), dir.clone(), move |step, status, message, progress| {
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
    .await;
    // Outside the job queue nobody else will reuse the loaded model soon.
    if !crate::jobs::worker_alive(&app) {
        crate::aligner::release_model_cache();
    }
    let res = res?;

    // Auto-resync to MEGA if configured (force-replace old files with new ones)
    let settings = crate::settings::load_settings(&app);
    if settings.mega.auto_sync && settings.mega.email.is_some() {
        let app3 = app.clone();
        let dir3 = dir.clone();
        tauri::async_runtime::spawn(async move {
            if crate::cloud::is_network_available().await {
                let _ = crate::cloud::resync_song(&app3, dir3).await;
            }
        });
    }

    Ok(res)
}

/// Legacy direct command (kept for compat). Prefer jobs_enqueue.
#[tauri::command]
pub async fn process_youtube_url(
    app: AppHandle,
    url: String,
) -> Result<serde_json::Value, String> {
    let app2 = app.clone();
    let app3 = app.clone();
    let res = run_pipeline(app, url, move |step, status, message, progress| {
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
    .await;
    if !crate::jobs::worker_alive(&app3) {
        crate::aligner::release_model_cache();
    }
    res
}
