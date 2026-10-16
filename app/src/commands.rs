//! Tauri commands invoked from the UI. Errors are returned as strings; short
//! camelCase ones (e.g. `invalidName`) are translation keys.

use std::sync::atomic::Ordering;

use base64::Engine;
use onenote::{RemoteNotebook, RemotePage, RemoteSection};
use serde::Serialize;
use sketch_core::{Page, Paper};
use sketch_store::{FolderLabel, Hit, Node};
use tauri::State;

use crate::backend::{open_store, AppState, CmdResult};
use crate::settings::Settings;
use crate::sync::Job;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tree {
    tree: Vec<Node>,
    tags: Vec<String>,
    root: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(flatten)]
    tree: Tree,
    settings: Settings,
    signed_in: bool,
}

fn tree_of(state: &AppState) -> Tree {
    let mut store = state.store();
    Tree { tree: store.tree(), tags: store.all_tags(), root: store.root().display().to_string() }
}

#[tauri::command]
pub fn get_state(state: State<AppState>) -> Snapshot {
    Snapshot { tree: tree_of(&state), settings: state.settings().clone(), signed_in: state.signed_in.load(Ordering::Relaxed) }
}

/// Rescans the folder (it may also change outside Sketcher, e.g. by OneDrive).
#[tauri::command]
pub fn get_tree(state: State<AppState>) -> Tree {
    tree_of(&state)
}

#[tauri::command]
pub fn search(state: State<AppState>, query: String) -> Vec<Hit> {
    state.store().search(&query)
}

#[tauri::command]
pub fn load_page(state: State<AppState>, path: String) -> CmdResult<Page> {
    let mut store = state.store();
    let mut page = store.load_page(&path).map_err(err)?;
    // Repairs pages imported before off-page OneNote ink was handled.
    if page.move_ink_onto_page() {
        store.save_page(&path, &page).map_err(err)?;
    }
    Ok(page)
}

#[tauri::command]
pub fn save_page(state: State<AppState>, path: String, page: Page) -> CmdResult<()> {
    state.store().save_page(&path, &page).map_err(err)
}

/// Creates an empty page in `folder`; returns its path.
#[tauri::command]
pub fn create_page(state: State<AppState>, folder: String, name: String, paper: Paper) -> CmdResult<String> {
    let mut page = Page::new(name);
    page.paper = paper;
    state.store().create_page(&folder, &page).map_err(err)
}

#[tauri::command]
pub fn create_folder(state: State<AppState>, parent: String, name: String) -> CmdResult<String> {
    state.store().create_folder(&parent, &name).map_err(err)
}

/// Renames a page or folder; returns the new path.
#[tauri::command]
pub fn rename(state: State<AppState>, path: String, name: String) -> CmdResult<String> {
    state.store().rename(&path, &name).map_err(err)
}

/// Moves a page or folder into `folder` ("" = the root); returns the new path.
#[tauri::command]
pub fn move_item(state: State<AppState>, path: String, folder: String) -> CmdResult<String> {
    state.store().move_into(&path, &folder).map_err(err)
}

/// Sets (or with `None` clears) a folder's calculated label.
#[tauri::command]
pub fn set_folder_label(state: State<AppState>, path: String, label: Option<FolderLabel>) -> CmdResult<()> {
    state.store().set_folder_label(&path, label).map_err(err)
}

/// Moves a page or folder to the system trash.
#[tauri::command]
pub fn delete_item(state: State<AppState>, path: String) -> CmdResult<()> {
    state.store().delete(&path).map_err(err)
}

#[tauri::command]
pub fn update_settings(state: State<AppState>, settings: Settings) {
    let auth_changed = state.settings().auth != settings.auth;
    if auth_changed {
        state.send(Job::Configure(settings.auth.clone()));
    }
    settings.save();
    *state.settings() = settings;
}

/// Lets the user pick the folder that holds the pages. Returns the new tree,
/// or None if cancelled.
#[tauri::command]
pub async fn choose_root(state: State<'_, AppState>) -> CmdResult<Option<Tree>> {
    let current = state.store().root().to_path_buf();
    let Some(dir) = rfd::AsyncFileDialog::new().set_directory(&current).pick_folder().await else {
        return Ok(None);
    };
    let settings = {
        let mut s = state.settings();
        s.root = Some(dir.path().display().to_string());
        s.save();
        s.clone()
    };
    *state.store() = open_store(&settings);
    Ok(Some(tree_of(&state)))
}

/// Asks for a file name and writes a PNG rendered by the UI (base64).
/// Returns the path, or None if cancelled.
#[tauri::command]
pub async fn save_png(name: String, data: String) -> CmdResult<Option<String>> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(err)?;
    let Some(file) = rfd::AsyncFileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{name}.png")).save_file().await
    else {
        return Ok(None);
    };
    std::fs::write(file.path(), bytes).map_err(err)?;
    Ok(Some(file.path().display().to_string()))
}

#[tauri::command]
pub fn open_url(url: String) -> CmdResult<()> {
    if !url.starts_with("https://") {
        return Err("only https links can be opened".into());
    }
    open::that(url).map_err(err)
}

/// Shows the page folder in Explorer / Finder.
#[tauri::command]
pub fn reveal_root(state: State<AppState>) -> CmdResult<()> {
    let root = state.store().root().to_path_buf();
    open::that(root).map_err(err)
}

// ---------- OneNote (import only) ----------

#[tauri::command]
pub fn onenote_sign_in(state: State<AppState>) {
    state.send(Job::SignIn);
}

#[tauri::command]
pub fn onenote_cancel(state: State<AppState>) {
    state.sync.cancel();
}

#[tauri::command]
pub fn onenote_sign_out(state: State<AppState>) {
    state.send(Job::SignOut);
}

#[tauri::command]
pub fn onenote_list_notebooks(state: State<AppState>) {
    state.send(Job::ListNotebooks);
}

#[tauri::command]
pub fn onenote_list_pages(state: State<AppState>, section_id: String) {
    state.send(Job::ListPages { section_id });
}

#[tauri::command]
pub fn onenote_import_page(state: State<AppState>, section_id: String, page: RemotePage, folder: String, open: bool) {
    state.send(Job::ImportPage { section_id, page, folder, open });
}

#[tauri::command]
pub fn onenote_import_section(state: State<AppState>, section: RemoteSection, folder: String) {
    state.send(Job::ImportSection { section, folder });
}

#[tauri::command]
pub fn onenote_import_notebook(state: State<AppState>, notebook: RemoteNotebook, folder: String) {
    state.send(Job::ImportNotebook { notebook, folder });
}
