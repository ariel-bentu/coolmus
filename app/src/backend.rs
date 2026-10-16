//! Application state shared by all Tauri commands, and handling of OneNote
//! worker events (imported pages are written into the folder tree before the
//! UI is notified).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use onenote::RemoteNotebook;
use onenote::RemotePage;
use serde::Serialize;
use sketch_core::Page;
use sketch_store::Store;
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::{config_dir, Settings};
use crate::sync::{Event, Job, SyncHandle};

pub struct AppState {
    pub store: Mutex<Store>,
    pub settings: Mutex<Settings>,
    pub sync: SyncHandle,
    pub signed_in: AtomicBool,
}

/// Messages pushed to the UI on the `sync` channel.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UiEvent {
    Busy { busy: bool },
    DeviceCode { user_code: String, verification_uri: String },
    SignedIn,
    SignedOut,
    Notebooks { notebooks: Vec<RemoteNotebook> },
    Pages { section_id: String, pages: Vec<RemotePage> },
    Info { key: String, args: Vec<String> },
    Error { message: String },
    TreeChanged,
    /// `existed`: the page had already been imported into that folder, so the
    /// local file (which may have local edits) was kept as is.
    PageImported { path: String, open: bool, existed: bool, source: onenote::ImportSource },
}

pub type CmdResult<T> = Result<T, String>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Opens the page folder from the settings (or the default), migrating a
/// library from versions before the folder layout the first time.
pub fn open_store(settings: &Settings) -> Store {
    let root = settings.root.as_ref().map(Into::into).unwrap_or_else(sketch_store::default_root);
    let mut store = Store::open(&root).unwrap_or_else(|e| {
        log::error!("could not open {}: {e}; using a temporary folder", root.display());
        Store::open(std::env::temp_dir().join("Sketcher")).expect("temp store")
    });
    if let Some(old) = sketch_store::legacy_root() {
        match store.migrate_legacy(&old) {
            Ok(0) => {}
            Ok(n) => log::info!("migrated {n} page(s) from {}", old.display()),
            Err(e) => log::error!("could not migrate the old library: {e}"),
        }
    }
    store
}

impl AppState {
    pub fn init(handle: AppHandle) -> Self {
        let settings = Settings::load();
        let store = open_store(&settings);
        let (sync, signed_in) = SyncHandle::spawn(settings.auth.clone(), config_dir().join("tokens.json"), move |ev| {
            on_sync_event(&handle, ev)
        });
        Self { store: Mutex::new(store), settings: Mutex::new(settings), sync, signed_in: AtomicBool::new(signed_in) }
    }

    pub fn store(&self) -> MutexGuard<'_, Store> {
        lock(&self.store)
    }

    pub fn settings(&self) -> MutexGuard<'_, Settings> {
        lock(&self.settings)
    }

    pub fn send(&self, job: Job) {
        self.sync.send(job);
    }
}

fn emit(handle: &AppHandle, ev: UiEvent) {
    if let Err(e) = handle.emit("sync", ev) {
        log::warn!("emit failed: {e}");
    }
}

/// Runs on the worker thread.
fn on_sync_event(handle: &AppHandle, ev: Event) {
    let state = handle.state::<AppState>();
    let ui = match ev {
        Event::Busy(busy) => UiEvent::Busy { busy },
        Event::DeviceCode(dc) => UiEvent::DeviceCode { user_code: dc.user_code, verification_uri: dc.verification_uri },
        Event::SignedIn => {
            state.signed_in.store(true, Ordering::Relaxed);
            UiEvent::SignedIn
        }
        Event::SignedOut => {
            state.signed_in.store(false, Ordering::Relaxed);
            UiEvent::SignedOut
        }
        Event::Notebooks(notebooks) => UiEvent::Notebooks { notebooks },
        Event::Pages { section_id, pages } => UiEvent::Pages { section_id, pages },
        Event::Info { key, args } => UiEvent::Info { key: key.into(), args },
        Event::Error(message) => UiEvent::Error { message },
        Event::Imported { folder, page, source, open } => match store_imported(&state, &folder, *page) {
            Ok((path, existed)) => {
                emit(handle, UiEvent::TreeChanged);
                UiEvent::PageImported { path, open, existed, source }
            }
            Err(message) => UiEvent::Error { message },
        },
    };
    emit(handle, ui);
}

/// Writes an imported page as a new file in `folder`, unless that OneNote page
/// was already imported there: the local copy always wins, so it is kept.
fn store_imported(state: &AppState, folder: &str, mut page: Page) -> Result<(String, bool), String> {
    let mut store = state.store();
    if let Some(existing) = page.onenote.as_ref().and_then(|l| store.find_imported(folder, &l.page_id)) {
        return Ok((existing, true));
    }
    if sketch_store::sanitize_name(&page.title).is_err() {
        page.title = "OneNote".into();
    }
    let path = store.create_page(folder, &page).map_err(|e| e.to_string())?;
    Ok((path, false))
}
