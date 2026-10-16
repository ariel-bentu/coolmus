//! User preferences, stored as JSON in the platform config directory.

use std::path::PathBuf;

use onenote::AuthConfig;
use serde::{Deserialize, Serialize};
use sketch_core::Pen;

/// Application (client) ID of the project's own Entra app registration.
/// Not a secret: public clients (device-code flow) ship their ID inside the app.
pub const DEFAULT_CLIENT_ID: &str = "3f6a4493-f7eb-4548-9e76-53e0e5fc2f84";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub auth: AuthConfig,
    /// The folder holding all pages; `None` means the default (`Documents/Sketcher`).
    pub root: Option<String>,
    pub smoothing: f32,
    pub pen: Pen,
    /// UI language: "he" (right-to-left) or "en".
    pub language: String,
    /// Draw with a finger; otherwise touch pans/zooms and only pen/mouse draw.
    pub touch_draws: bool,
    /// Font size for new text boxes, in page units.
    pub text_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        let auth = AuthConfig {
            client_id: std::env::var("SKETCHER_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_string()),
            ..AuthConfig::default()
        };
        Self {
            auth,
            root: None,
            smoothing: 0.35,
            pen: Pen::default(),
            language: "he".into(),
            touch_draws: false,
            text_size: 20.0,
        }
    }
}

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "Sketcher")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("sketcher-config"))
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let mut s: Self = std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Settings saved before a client ID was configured fall back to the built-in one.
        if s.auth.client_id.trim().is_empty() {
            s.auth.client_id = Self::default().auth.client_id;
        }
        s
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, serde_json::to_vec_pretty(self).unwrap_or_default()) {
            log::warn!("could not save settings: {e}");
        }
    }
}
