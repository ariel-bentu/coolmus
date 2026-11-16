//! Application state shared by all Tauri commands.

use std::sync::{Mutex, MutexGuard};

use coolmus_store::Store;

use crate::settings::Settings;

pub struct AppState {
    pub store: Mutex<Store>,
    pub settings: Mutex<Settings>,
}

pub type CmdResult<T> = Result<T, String>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl AppState {
    pub fn init() -> Self {
        let settings = Settings::load();
        let font = coolmus_pdf::system_font().unwrap_or_else(|| {
            log::error!("no system font found: text boxes can't be saved");
            Vec::new()
        });
        let store = Store::new(settings.root_paths(), font).with_cache(crate::settings::config_dir().join("index.json"));
        Self { store: Mutex::new(store), settings: Mutex::new(settings) }
    }

    pub fn store(&self) -> MutexGuard<'_, Store> {
        lock(&self.store)
    }

    pub fn settings(&self) -> MutexGuard<'_, Settings> {
        lock(&self.settings)
    }
}
