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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    #[serde(default)]
    pub cookie_browser: CookieBrowser,
    /// Path to a Netscape-format cookies.txt file (overrides cookie_browser when set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookies_file: Option<String>,
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
