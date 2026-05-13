use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSyncEvent {
    pub song_dir: String,
    pub status: String, // "active" | "done" | "error"
    pub message: String,
    pub progress: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudStatus {
    pub available: bool,
    pub logged_in: bool,
    pub account: Option<String>,
    pub path: Option<String>,  // binary path found, for debug
}

fn emit_cloud(app: &AppHandle, ev: CloudSyncEvent) {
    let _ = app.emit("karaoke://cloud-sync", ev);
}

// ─── MEGAcmd binary resolution ──────────────────────────────────────────────

fn find_mega_exec() -> String {
    // macOS app sandbox may not have full PATH; check known install locations first
    for path in [
        "/usr/local/bin/mega-exec",          // MEGAcmd official .pkg / cask (Intel + AS)
        "/opt/homebrew/bin/mega-exec",       // Homebrew Apple Silicon
        "/Applications/MEGAcmd.app/Contents/MacOS/mega-exec",
    ] {
        if Path::new(path).exists() {
            return path.to_string();
        }
    }
    "mega-exec".to_string()
}

/// Returns the path used for mega-exec, or None if not found anywhere.
pub fn mega_exec_path() -> Option<String> {
    for path in [
        "/usr/local/bin/mega-exec",
        "/opt/homebrew/bin/mega-exec",
        "/Applications/MEGAcmd.app/Contents/MacOS/mega-exec",
    ] {
        if Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    // Last resort: check PATH via which
    None
}

async fn run_mega(args: &[&str]) -> Result<String, String> {
    let bin = find_mega_exec();
    let output = tokio::process::Command::new(&bin)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("MEGAcmd non trovato ({}). Installa con: brew install megacmd", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

// ─── Connectivity helpers ────────────────────────────────────────────────────

pub async fn is_network_available() -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .get("https://g.api.mega.co.nz/")
        .send()
        .await
        .map(|r| r.status().is_success() || r.status().as_u16() == 400)
        .unwrap_or(false)
}

// ─── Auth helpers ────────────────────────────────────────────────────────────

async fn is_logged_in() -> Option<String> {
    run_mega(&["whoami"]).await.ok().map(|s| {
        // whoami returns something like "Account e-mail: user@example.com"
        // or just "user@example.com" depending on version
        s.lines()
            .last()
            .unwrap_or(&s)
            .trim()
            .trim_start_matches("Account e-mail: ")
            .to_string()
    })
}

async fn mega_login(app: &AppHandle) -> Result<(), String> {
    if is_logged_in().await.is_some() {
        return Ok(());
    }
    let settings = crate::settings::load_settings(app);
    let mega = &settings.mega;
    let email = mega.email.as_deref().ok_or("Email MEGA non configurata")?;
    let password = mega.password.as_deref().ok_or("Password MEGA non configurata")?;

    let result = if let Some(mfa) = mega.mfa.as_deref().filter(|s| !s.is_empty()) {
        // Pass MFA code as third arg: mega-exec login email password mfa_code
        run_mega(&["login", email, password, mfa]).await
    } else {
        run_mega(&["login", email, password]).await
    };

    result
        .map(|_| ())
        .map_err(|e| format!("Login MEGA fallito: {}", e))
}

// ─── MEGA path helpers ───────────────────────────────────────────────────────

fn safe_name_from_dir(song_dir: &str) -> String {
    Path::new(song_dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into())
}

fn remote_path_for(song_dir: &str) -> String {
    format!("/ColognaKaraoke/{}", safe_name_from_dir(song_dir))
}

async fn ensure_remote_folder(remote_path: &str) -> Result<(), String> {
    if run_mega(&["ls", remote_path]).await.is_err() {
        run_mega(&["mkdir", "-p", remote_path])
            .await
            .map_err(|e| format!("Errore creazione cartella MEGA: {}", e))?;
    }
    Ok(())
}

// ─── metadata.json patch helper ─────────────────────────────────────────────

fn patch_metadata(
    dir: &Path,
    f: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> Result<(), String> {
    let path = dir.join("metadata.json");
    let s = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut v: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    if let Some(obj) = v.as_object_mut() {
        f(obj);
    }
    crate::library::save_metadata(dir, &v)
}

// ─── Core operations ─────────────────────────────────────────────────────────

const SYNC_FILES: &[&str] = &[
    "metadata.json",
    "words.json",
    "lrc.json",
    "cover.jpg",
    "instrumental.mp3",
    "original.mp3",
    "vocals.wav",
    "_pitch_contour.bin",
];

pub async fn upload_song(app: &AppHandle, song_dir: String) -> Result<(), String> {
    emit_cloud(
        app,
        CloudSyncEvent {
            song_dir: song_dir.clone(),
            status: "active".into(),
            message: "Connessione a MEGA...".into(),
            progress: 0.0,
        },
    );

    if !is_network_available().await {
        let msg = "Nessuna connessione di rete".to_string();
        emit_cloud(
            app,
            CloudSyncEvent {
                song_dir: song_dir.clone(),
                status: "error".into(),
                message: msg.clone(),
                progress: 0.0,
            },
        );
        return Err(msg);
    }

    mega_login(app).await.map_err(|e| {
        emit_cloud(
            app,
            CloudSyncEvent {
                song_dir: song_dir.clone(),
                status: "error".into(),
                message: e.clone(),
                progress: 0.0,
            },
        );
        e
    })?;

    let remote_path = remote_path_for(&song_dir);
    ensure_remote_folder(&remote_path).await?;

    let dir = PathBuf::from(&song_dir);
    let files: Vec<&str> = SYNC_FILES
        .iter()
        .filter(|&&f| dir.join(f).exists())
        .copied()
        .collect();

    let total = files.len() as f32;
    for (i, filename) in files.iter().enumerate() {
        emit_cloud(
            app,
            CloudSyncEvent {
                song_dir: song_dir.clone(),
                status: "active".into(),
                message: format!("Upload {}...", filename),
                progress: i as f32 / total,
            },
        );
        let local_path = dir.join(filename).to_string_lossy().into_owned();
        // trailing slash on remote = upload into that folder
        run_mega(&["put", &local_path, &format!("{}/", remote_path)])
            .await
            .map_err(|e| {
                let msg = format!("Errore upload {}: {}", filename, e);
                emit_cloud(
                    app,
                    CloudSyncEvent {
                        song_dir: song_dir.clone(),
                        status: "error".into(),
                        message: msg.clone(),
                        progress: 0.0,
                    },
                );
                msg
            })?;
    }

    patch_metadata(&dir, |obj| {
        obj.insert("cloud_synced".into(), true.into());
        obj.insert("local_deleted".into(), false.into());
        obj.insert("mega_remote_path".into(), remote_path.clone().into());
    })?;

    emit_cloud(
        app,
        CloudSyncEvent {
            song_dir: song_dir.clone(),
            status: "done".into(),
            message: "Sync completato".into(),
            progress: 1.0,
        },
    );
    Ok(())
}

pub async fn download_song(app: &AppHandle, song_dir: String) -> Result<(), String> {
    emit_cloud(
        app,
        CloudSyncEvent {
            song_dir: song_dir.clone(),
            status: "active".into(),
            message: "Download da MEGA...".into(),
            progress: 0.0,
        },
    );

    mega_login(app).await?;

    let remote_path = remote_path_for(&song_dir);
    let dir = PathBuf::from(&song_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    emit_cloud(
        app,
        CloudSyncEvent {
            song_dir: song_dir.clone(),
            status: "active".into(),
            message: "Scaricamento file...".into(),
            progress: 0.3,
        },
    );

    // `mega-exec get /remote/folder /local/parent/` downloads the folder into parent,
    // creating parent/folder_name/ — so use dir's parent as destination.
    let parent = dir.parent().ok_or("song_dir has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    run_mega(&["get", &remote_path, &format!("{}/", parent.to_string_lossy())])
        .await
        .map_err(|e| format!("Errore download MEGA: {}", e))?;

    patch_metadata(&dir, |obj| {
        obj.insert("local_deleted".into(), false.into());
    })?;

    emit_cloud(
        app,
        CloudSyncEvent {
            song_dir: song_dir.clone(),
            status: "done".into(),
            message: "Download completato".into(),
            progress: 1.0,
        },
    );
    Ok(())
}

pub async fn delete_from_cloud(app: &AppHandle, song_dir: String) -> Result<(), String> {
    mega_login(app).await?;

    let remote_path = remote_path_for(&song_dir);
    // -f = force (no error if not found)
    run_mega(&["rm", "-rf", &remote_path])
        .await
        .map_err(|e| format!("Errore delete MEGA: {}", e))?;

    let dir = PathBuf::from(&song_dir);
    if dir.join("metadata.json").exists() {
        patch_metadata(&dir, |obj| {
            obj.insert("cloud_synced".into(), false.into());
            obj.insert("mega_remote_path".into(), serde_json::Value::Null);
        })?;
    }
    Ok(())
}

pub fn make_cloud_only(song_dir: &str) -> Result<(), String> {
    const DELETABLE: &[&str] = &[
        "original.mp3",
        "instrumental.mp3",
        "instrumental.wav",
        "vocals.wav",
        "cover.jpg",
        "_pitch_contour.bin",
        "words.json",
        "lrc.json",
        "vad.json",
    ];
    let dir = PathBuf::from(song_dir);
    for f in DELETABLE {
        let p = dir.join(f);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
    }
    patch_metadata(&dir, |obj| {
        obj.insert("local_deleted".into(), true.into());
    })
}

// ─── Tauri commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn cloud_get_settings(app: AppHandle) -> crate::settings::MegaSettings {
    crate::settings::load_settings(&app).mega
}

#[tauri::command]
pub fn cloud_save_credentials(
    app: AppHandle,
    email: String,
    password: String,
    mfa: String,
    auto_sync: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::load_settings(&app);
    settings.mega.email = Some(email);
    settings.mega.password = Some(password);
    settings.mega.mfa = if mfa.is_empty() { None } else { Some(mfa) };
    settings.mega.auto_sync = auto_sync;
    crate::settings::save_settings(&app, &settings)
}

#[tauri::command]
pub async fn cloud_check_status() -> CloudStatus {
    let path = mega_exec_path();
    let available = path.is_some();
    if !available {
        return CloudStatus {
            available: false,
            logged_in: false,
            account: None,
            path: None,
        };
    }
    let account = is_logged_in().await;
    CloudStatus {
        available: true,
        logged_in: account.is_some(),
        account,
        path,
    }
}

#[tauri::command]
pub async fn cloud_login(app: AppHandle) -> Result<String, String> {
    mega_login(&app).await?;
    is_logged_in()
        .await
        .ok_or_else(|| "Login riuscito ma whoami fallito".into())
}

#[tauri::command]
pub async fn cloud_logout() -> Result<(), String> {
    run_mega(&["logout"]).await.map(|_| ())
}

#[tauri::command]
pub async fn cloud_sync_song(app: AppHandle, dir: String) -> Result<(), String> {
    upload_song(&app, dir).await
}

#[tauri::command]
pub async fn cloud_sync_all(app: AppHandle) -> Result<(), String> {
    let settings = crate::settings::load_settings(&app);
    if settings.mega.email.is_none() {
        return Err("MEGA non configurato".into());
    }

    let lib_dir = crate::library::library_dir(&app);
    let entries = std::fs::read_dir(&lib_dir).map_err(|e| e.to_string())?;
    let mut errors: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let meta_path = p.join("metadata.json");
        if !meta_path.exists() {
            continue;
        }
        let already_synced = std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("cloud_synced").and_then(|x| x.as_bool()))
            .unwrap_or(false);
        if already_synced {
            continue;
        }
        let song_dir = p.to_string_lossy().into_owned();
        if let Err(e) = upload_song(&app, song_dir).await {
            errors.push(e);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[tauri::command]
pub async fn cloud_download_song(app: AppHandle, dir: String) -> Result<(), String> {
    download_song(&app, dir).await
}

#[tauri::command]
pub async fn cloud_delete_song(app: AppHandle, dir: String) -> Result<(), String> {
    delete_from_cloud(&app, dir).await
}

#[tauri::command]
pub fn cloud_make_local_only(dir: String) -> Result<(), String> {
    make_cloud_only(&dir)
}

/// Returns folder names under /ColognaKaraoke/ on MEGA that have no corresponding
/// local library entry. Used to surface "orphan" cloud songs the user can re-download.
#[tauri::command]
pub async fn cloud_list_remote_songs(app: AppHandle) -> Result<Vec<String>, String> {
    mega_login(&app).await?;

    let output = run_mega(&["ls", "/ColognaKaraoke/"])
        .await
        .map_err(|e| format!("Impossibile listare MEGA: {}", e))?;

    // mega-exec ls prints one entry per line; entries may have trailing '/' for folders
    let remote_names: Vec<String> = output
        .lines()
        .map(|l| l.trim().trim_end_matches('/').to_string())
        .filter(|l| !l.is_empty())
        .collect();

    // Collect local song dir basenames so we can exclude already-present entries
    let lib_dir = crate::library::library_dir(&app);
    let local_names: std::collections::HashSet<String> = std::fs::read_dir(&lib_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(remote_names
        .into_iter()
        .filter(|name| !local_names.contains(name))
        .collect())
}

/// Download an orphan cloud song (no local metadata.json) by safe folder name.
#[tauri::command]
pub async fn cloud_restore_song(app: AppHandle, safe_name: String) -> Result<(), String> {
    mega_login(&app).await?;

    let lib_dir = crate::library::library_dir(&app);
    let song_dir = lib_dir.join(&safe_name);
    let song_dir_str = song_dir.to_string_lossy().into_owned();

    emit_cloud(
        &app,
        CloudSyncEvent {
            song_dir: song_dir_str.clone(),
            status: "active".into(),
            message: format!("Ripristino {}...", safe_name),
            progress: 0.0,
        },
    );

    // Download folder into lib_dir — MEGAcmd creates lib_dir/safe_name/ automatically.
    let remote_path = format!("/ColognaKaraoke/{}", safe_name);
    run_mega(&["get", &remote_path, &format!("{}/", lib_dir.to_string_lossy())])
        .await
        .map_err(|e| format!("Errore ripristino: {}", e))?;

    // Clear local_deleted flag if metadata was downloaded
    let meta_path = song_dir.join("metadata.json");
    if meta_path.exists() {
        patch_metadata(&song_dir, |obj| {
            obj.insert("local_deleted".into(), false.into());
        })?;
    }

    emit_cloud(
        &app,
        CloudSyncEvent {
            song_dir: song_dir_str.clone(),
            status: "done".into(),
            message: "Ripristinato".into(),
            progress: 1.0,
        },
    );
    Ok(())
}

// ─── Module init ─────────────────────────────────────────────────────────────

pub fn init(_app: &AppHandle) {
    // No managed state needed: MEGAcmd is stateless from our perspective
    // (the daemon manages the session externally)
}
