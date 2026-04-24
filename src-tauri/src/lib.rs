mod aligner;
mod audio;
mod downloader;
mod jobs;
mod leaderboard;
mod library;
mod lyrics;
mod metadata;
mod pipeline;
mod pitch;
mod recorder;
mod recognizer;
mod separator;
mod settings;
mod vad;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

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
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
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
            lyrics::fetch_lyrics_cmd,
            lyrics::parse_lrc_cmd,
            pipeline::process_youtube_url,
            pipeline::reprocess_song,
            aligner::get_words,
            aligner::get_cookie_browser,
            aligner::set_cookie_browser,
            aligner::get_cookies_file,
            aligner::set_cookies_file,
            jobs::jobs_enqueue,
            jobs::jobs_list,
            jobs::jobs_cancel,
            recorder::recorder_start,
            recorder::recorder_stop,
            pitch::pitch_start,
            pitch::pitch_stop,
            leaderboard::leaderboard_insert,
            leaderboard::leaderboard_top,
            leaderboard::leaderboard_global_top,
            open_presentation_window,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            library::ensure_library(&handle)?;
            jobs::init(&handle);
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
