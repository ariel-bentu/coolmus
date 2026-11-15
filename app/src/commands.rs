//! Tauri commands invoked from the UI. Errors are returned as strings; short
//! camelCase ones (e.g. `invalidName`) are translation keys.
//!
//! Paths are absolute with `/` separators; the store refuses any outside the
//! roots. Commands that read or write PDFs are async so they run off the UI
//! thread.

use std::path::PathBuf;

use base64::Engine;
use clipboard_rs::{Clipboard, ClipboardContext};
use serde::Serialize;
use sketch_core::{Page, Paper};
use sketch_store::{Change, FolderLabel, Hit, Node};
use tauri::State;

use crate::backend::{AppState, CmdResult};
use crate::settings::Settings;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tree {
    tree: Vec<Node>,
    tags: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(flatten)]
    tree: Tree,
    settings: Settings,
}

fn tree_of(state: &AppState) -> Tree {
    let mut store = state.store();
    Tree { tree: store.tree(), tags: store.all_tags() }
}

#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> CmdResult<Snapshot> {
    Ok(Snapshot { tree: tree_of(&state), settings: state.settings().clone() })
}

/// Rescans the roots (they also change outside Sketcher: Explorer, OneDrive).
#[tauri::command]
pub async fn get_tree(state: State<'_, AppState>) -> CmdResult<Tree> {
    Ok(tree_of(&state))
}

#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String) -> CmdResult<Vec<Hit>> {
    Ok(state.store().search(&query))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedPage {
    page: Page,
    /// What other programs changed since Sketcher saved the page:
    /// `contentRedrawn`, `annotations`, `pagesAdded`.
    changes: Vec<&'static str>,
}

#[tauri::command]
pub async fn load_page(state: State<'_, AppState>, path: String) -> CmdResult<LoadedPage> {
    let loaded = state.store().load_page(&path).map_err(err)?;
    let changes = loaded
        .changes
        .iter()
        .map(|c| match c {
            Change::ContentRedrawn => "contentRedrawn",
            Change::Annotations(_) => "annotations",
            Change::PagesAdded(_) => "pagesAdded",
        })
        .collect();
    Ok(LoadedPage { page: loaded.page, changes })
}

/// The PDF a page is drawn on (without Sketcher's layer), as raw bytes.
#[tauri::command]
pub async fn page_background(state: State<'_, AppState>, path: String) -> CmdResult<tauri::ipc::Response> {
    Ok(tauri::ipc::Response::new(state.store().background(&path).map_err(err)?))
}

#[tauri::command]
pub async fn save_page(state: State<'_, AppState>, path: String, page: Page) -> CmdResult<()> {
    state.store().save_page(&path, &page).map_err(err)
}

/// Creates an empty page in `folder`; returns its path.
#[tauri::command]
pub async fn create_page(state: State<'_, AppState>, folder: String, name: String, paper: Paper) -> CmdResult<String> {
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

/// Moves a file or folder into `folder` (possibly in another root); returns
/// the new path.
#[tauri::command]
pub async fn move_item(state: State<'_, AppState>, path: String, folder: String) -> CmdResult<String> {
    state.store().move_into(&path, &folder).map_err(err)
}

/// Sets (or with `None` clears) a folder's calculated label.
#[tauri::command]
pub fn set_folder_label(state: State<AppState>, path: String, label: Option<FolderLabel>) -> CmdResult<()> {
    state.store().set_folder_label(&path, label).map_err(err)
}

/// Moves a file or folder to the system trash.
#[tauri::command]
pub async fn delete_item(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    state.store().delete(&path).map_err(err)
}

// ---------- clipboard (shared with Explorer / Finder) ----------

fn clipboard() -> CmdResult<ClipboardContext> {
    ClipboardContext::new().map_err(|_| "clipboard".to_string())
}

/// Puts files and folders on the system clipboard, so they can be pasted in
/// Explorer or back into Sketcher.
#[tauri::command]
pub fn copy_items(state: State<AppState>, paths: Vec<String>) -> CmdResult<()> {
    let store = state.store();
    let files = paths.iter().map(|p| store.native(p).map(|n| n.display().to_string())).collect::<Result<Vec<_>, _>>().map_err(err)?;
    clipboard()?.set_files(files).map_err(|_| "clipboard".to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pasted {
    paths: Vec<String>,
    /// The items were cut in Sketcher and moved (else copied).
    moved: bool,
}

/// Pastes the files on the system clipboard into `folder`: copied from
/// Explorer or Sketcher, they are copied; if they are exactly the items
/// `cut` in Sketcher, they are moved (paths in the same order as `cut`).
#[tauri::command]
pub async fn paste_items(state: State<'_, AppState>, folder: String, cut: Vec<String>) -> CmdResult<Pasted> {
    let files: Vec<PathBuf> = clipboard()?.get_files().unwrap_or_default().into_iter().map(PathBuf::from).collect();
    if files.is_empty() {
        return Err("clipboardNoFiles".into());
    }
    let mut store = state.store();
    let cut_files = cut.iter().map(|p| store.native(p)).collect::<Result<Vec<_>, _>>().map_err(err)?;
    if !cut.is_empty() && cut_files == files {
        let paths = cut.iter().map(|p| store.move_into(p, &folder)).collect::<Result<_, _>>().map_err(err)?;
        return Ok(Pasted { paths, moved: true });
    }
    Ok(Pasted { paths: store.copy_into(&files, &folder).map_err(err)?, moved: false })
}

/// Writes a file dropped on a folder: the body is its contents, the
/// URI-encoded `x-folder` and `x-name` headers say where. Returns its path.
#[tauri::command]
pub fn drop_file(state: State<AppState>, request: tauri::ipc::Request<'_>) -> CmdResult<String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected file contents".into());
    };
    let header = |key: &str| -> CmdResult<String> {
        let raw = request.headers().get(key).and_then(|v| v.to_str().ok()).ok_or_else(|| format!("missing {key}"))?;
        percent_encoding::percent_decode_str(raw).decode_utf8().map(|s| s.into_owned()).map_err(err)
    };
    state.store().write_file(&header("x-folder")?, &header("x-name")?, bytes).map_err(err)
}

// ---------- roots ----------

/// Lets the user add a folder to the tree. Returns the new tree, or None if
/// cancelled.
#[tauri::command]
pub async fn add_root(state: State<'_, AppState>) -> CmdResult<Option<Tree>> {
    let Some(dir) = rfd::AsyncFileDialog::new().pick_folder().await else {
        return Ok(None);
    };
    let roots = {
        let mut s = state.settings();
        let mut roots: Vec<String> = s.root_paths().iter().map(|p| p.display().to_string()).collect();
        let new = dir.path().display().to_string();
        if !roots.contains(&new) {
            roots.push(new);
        }
        s.roots = roots;
        s.save();
        s.root_paths()
    };
    state.store().set_roots(roots);
    Ok(Some(tree_of(&state)))
}

/// Removes a folder from the tree (its files stay on disk). Without any
/// folder left, the default folder is shown again.
#[tauri::command]
pub async fn remove_root(state: State<'_, AppState>, path: String) -> CmdResult<Tree> {
    let roots = {
        let mut s = state.settings();
        let gone = PathBuf::from(&path);
        s.roots = s.root_paths().into_iter().filter(|r| *r != gone).map(|r| r.display().to_string()).collect();
        s.save();
        s.root_paths()
    };
    state.store().set_roots(roots);
    Ok(tree_of(&state))
}

#[tauri::command]
pub fn update_settings(state: State<AppState>, mut settings: Settings) {
    let mut current = state.settings();
    // Roots change only through add_root / remove_root.
    settings.roots = current.roots.clone();
    settings.save();
    *current = settings;
}

// ---------- other programs ----------

/// Shows a file or folder in Explorer / Finder.
#[tauri::command]
pub fn reveal(state: State<AppState>, path: String) -> CmdResult<()> {
    let target = state.store().native(&path).map_err(err)?;
    if target.is_dir() {
        return open::that(target).map_err(err);
    }
    #[cfg(windows)]
    let shown = std::process::Command::new("explorer").arg(format!("/select,{}", target.display())).spawn().map(|_| ());
    #[cfg(target_os = "macos")]
    let shown = std::process::Command::new("open").arg("-R").arg(&target).spawn().map(|_| ());
    #[cfg(not(any(windows, target_os = "macos")))]
    let shown = open::that(target.parent().unwrap_or(&target));
    shown.map_err(err)
}

/// Opens a file with its default program (e.g. a PDF that isn't a Sketcher page).
#[tauri::command]
pub fn open_external(state: State<AppState>, path: String) -> CmdResult<()> {
    let target = state.store().native(&path).map_err(err)?;
    open::that(target).map_err(err)
}

/// Opens the system print dialog for the webview; the UI's print stylesheet
/// shows only the sheets it prepared.
#[tauri::command]
pub fn print_view(window: tauri::WebviewWindow) -> CmdResult<()> {
    window.print().map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    /// The folder created for the notebook or section.
    folder: String,
    pages: usize,
}

fn write_import(store: &mut sketch_store::Store, entry: &onenote_import::Entry, folder: &str) -> Result<String, sketch_store::StoreError> {
    use onenote_import::Entry;
    let fallback = |name: &str| if sketch_store::sanitize_name(name).is_ok() { name.to_string() } else { "OneNote".into() };
    match entry {
        Entry::Section { name, pages } => {
            let dir = store.create_folder(folder, &fallback(name))?;
            for page in pages {
                let mut page = page.clone();
                if sketch_store::sanitize_name(&page.title).is_err() {
                    page.title = "Untitled".into();
                }
                store.create_page(&dir, &page)?;
            }
            Ok(dir)
        }
        Entry::Group { name, entries } => {
            let dir = store.create_folder(folder, &fallback(name))?;
            for e in entries {
                write_import(store, e, &dir)?;
            }
            Ok(dir)
        }
    }
}

/// Imports a OneNote notebook (`.onetoc2`) or section (`.one`) file into
/// `folder`: a folder per notebook, section and section group, and a page
/// file per page. Returns None if cancelled.
#[tauri::command]
pub async fn import_onenote(state: State<'_, AppState>, folder: String) -> CmdResult<Option<Imported>> {
    let Some(file) = rfd::AsyncFileDialog::new().add_filter("OneNote", &["onetoc2", "one"]).pick_file().await else {
        return Ok(None);
    };
    let entry = onenote_import::read_file(file.path()).map_err(err)?;
    let folder = write_import(&mut state.store(), &entry, &folder).map_err(err)?;
    Ok(Some(Imported { folder, pages: entry.page_count() }))
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
