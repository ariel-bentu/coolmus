//! User preferences, stored as JSON in the platform config directory.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use coolmus_core::Pen;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Folders shown in the tree, in order; empty means the default
    /// (`Documents/Coolmus`).
    pub roots: Vec<String>,
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
        Self { roots: Vec::new(), smoothing: 0.35, pen: Pen::default(), language: "he".into(), touch_draws: false, text_size: 20.0 }
    }
}

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "Coolmus")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("coolmus-config"))
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
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

    /// The roots to show: the configured ones, or the default folder
    /// (created if needed).
    pub fn root_paths(&self) -> Vec<PathBuf> {
        if !self.roots.is_empty() {
            return self.roots.iter().map(PathBuf::from).collect();
        }
        let root = coolmus_store::default_root();
        if let Err(e) = std::fs::create_dir_all(&root) {
            log::error!("could not create {}: {e}", root.display());
        }
        vec![root]
    }
}
