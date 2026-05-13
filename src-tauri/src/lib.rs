mod aligner;
mod audio;
mod cloud;
mod downloader;
mod jobs;
mod leaderboard;
mod library;
mod lyrics;
mod metadata;
mod pipeline;
mod pitch;
mod players;
mod recorder;
mod recognizer;
mod separator;
mod settings;
mod vad;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

#[tauri::command]
async fn close_presentation_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("presentation") {
        win.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn open_presentation_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("presentation") {
        let _ = win.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(
        &app,
        "presentation",
        WebviewUrl::App("index.html#presentation".into()),
    )
    .title("Cologna Karaoke — Presentation")
    .inner_size(1280.0, 720.0)
    .min_inner_size(640.0, 360.0)
    .resizable(true)
    .decorations(true)
    .fullscreen(false)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn is_presentation_open(app: AppHandle) -> bool {
    app.get_webview_window("presentation").is_some()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            library::scan_library,
            library::delete_song,
            library::get_library_dir,
            library::get_song_audio_path,
            library::set_lrc_offset,
            lyrics::fetch_lyrics_cmd,
            lyrics::parse_lrc_cmd,
            pipeline::process_youtube_url,
            pipeline::reprocess_song,
            aligner::get_words,
            aligner::get_whisper_model,
            aligner::set_whisper_model,
            aligner::get_cookie_browser,
            aligner::set_cookie_browser,
            aligner::get_cookies_file,
            aligner::set_cookies_file,
            aligner::get_alignment_mode,
            aligner::set_alignment_mode,
            jobs::jobs_enqueue,
            jobs::jobs_list,
            jobs::jobs_cancel,
            recorder::recorder_start,
            recorder::recorder_stop,
            recorder::list_mic_devices,
            recorder::get_mic_device,
            recorder::set_mic_device,
            pitch::pitch_start,
            pitch::pitch_stop,
            pitch::pitch_sync,
            leaderboard::leaderboard_insert,
            leaderboard::leaderboard_top,
            leaderboard::leaderboard_global_top,
            leaderboard::leaderboard_reset,
            leaderboard::leaderboard_reset_song,
            open_presentation_window,
            close_presentation_window,
            is_presentation_open,
            cloud::cloud_get_settings,
            cloud::cloud_save_credentials,
            cloud::cloud_check_status,
            cloud::cloud_login,
            cloud::cloud_logout,
            cloud::cloud_sync_song,
            cloud::cloud_sync_all,
            cloud::cloud_download_song,
            cloud::cloud_delete_song,
            cloud::cloud_make_local_only,
            cloud::cloud_list_remote_songs,
            cloud::cloud_restore_song,
            players::players_load,
            players::players_save,
            players::active_queue_load,
            players::active_queue_save,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            library::ensure_library(&handle)?;
            jobs::init(&handle);
            cloud::init(&handle);
            recorder::init(&handle);
            pitch::init(&handle);
            if let Err(e) = leaderboard::init(&handle) {
                eprintln!("leaderboard init failed: {}", e);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
