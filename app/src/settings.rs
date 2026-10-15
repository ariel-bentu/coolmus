//! User preferences, stored as JSON in the platform config directory.

use std::path::PathBuf;

use onenote::{AuthConfig, ExportMode};
use serde::{Deserialize, Serialize};
use sketch_core::Pen;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub auth: AuthConfig,
    pub export_mode: ExportMode,
    /// When saving an already-linked page, delete the previous OneNote page.
    pub replace_old: bool,
    pub smoothing: f32,
    pub pen: Pen,
    pub recent_colors: Vec<[u8; 4]>,
}

impl Default for Settings {
    fn default() -> Self {
        let mut auth = AuthConfig::default();
        if let Ok(id) = std::env::var("SKETCHER_CLIENT_ID") {
            auth.client_id = id;
        }
        Self {
            auth,
            export_mode: ExportMode::NativeInk,
            replace_old: true,
            smoothing: 0.35,
            pen: Pen::default(),
            recent_colors: Vec::new(),
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
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
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
