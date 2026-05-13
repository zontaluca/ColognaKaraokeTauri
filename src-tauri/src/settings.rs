use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::library::library_dir;

/// Browser to use for yt-dlp cookie extraction.
/// "none" = no cookies (may hit 429), "safari"/"chrome"/"firefox"/"chromium" = read from browser.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CookieBrowser {
    None,
    Safari,
    Chrome,
    Firefox,
    Chromium,
}

impl Default for CookieBrowser {
    fn default() -> Self {
        // Safari is the system browser on macOS
        #[cfg(target_os = "macos")]
        return Self::Safari;
        #[cfg(not(target_os = "macos"))]
        return Self::None;
    }
}

impl CookieBrowser {
    pub fn as_str(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Safari => Some("safari"),
            Self::Chrome => Some("chrome"),
            Self::Firefox => Some("firefox"),
            Self::Chromium => Some("chromium"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentMode {
    Wav2vec2Ctc,
}

impl Default for AlignmentMode {
    fn default() -> Self {
        Self::Wav2vec2Ctc
    }
}

impl AlignmentMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wav2vec2Ctc => "wav2vec2_ctc",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "wav2vec2_ctc" => Some(Self::Wav2vec2Ctc),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MegaSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Stored plaintext in app_settings.json; user is informed in Settings UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// TOTP 2FA code (6-digit); only needed once per device, MEGAcmd stores the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mfa: Option<String>,
    #[serde(default)]
    pub auto_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    #[serde(default)]
    pub cookie_browser: CookieBrowser,
    /// Path to a Netscape-format cookies.txt file (overrides cookie_browser when set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookies_file: Option<String>,
    #[serde(default)]
    pub alignment_mode: AlignmentMode,
    /// Preferred microphone device name; None = system default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic_device: Option<String>,
    #[serde(default)]
    pub mega: MegaSettings,
}

fn settings_path(app: &AppHandle) -> PathBuf {
    library_dir(app).join("app_settings.json")
}

pub fn load_settings(app: &AppHandle) -> AppSettings {
    let path = settings_path(app);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app);
    let s = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| e.to_string())
}
