//! Local persistence: pages are PDF files in ordinary folders.
//!
//! ```text
//! <root>/                       any folder the user adds (several are allowed)
//!   Work/
//!     Bridges/
//!       Plan.pdf                one page: a normal PDF that carries its own
//!                               editable page data (see `coolmus-pdf`)
//!   Manual.pdf                  any other PDF is listed too
//! ```
//! The folder tree *is* the organization: there is no index file, so Explorer,
//! OneDrive or any sync tool can change the folders safely. The page name is
//! the file name. Paths exchanged with callers are absolute, use `/`
//! separators, and must lie inside one of the roots. Writes are atomic
//! (temp file + rename) so a crash never corrupts a page.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use coolmus_core::tags::Query;
use coolmus_core::Page;
pub use coolmus_pdf::Change;

/// File extension of page files.
pub const PAGE_EXT: &str = "pdf";

/// Hidden per-folder settings file. It lives inside the folder, so it moves,
/// syncs and is deleted together with it.
pub const FOLDER_INFO: &str = ".coolmus-folder.json";

/// A display label for a folder that is calculated rather than typed, so it
/// can change over time while the folder's own name stays fixed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FolderLabel {
    /// A school class: the grade follows the school year (September–August),
    /// counted from the year the class started first grade. The UI fills the
    /// template's `{grade}` and `{n}` (class number) placeholders.
    SchoolGrade { first_grade_year: i32, class_number: String, template: String },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FolderInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<FolderLabel>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Pdf { path: PathBuf, source: coolmus_pdf::Error },
    /// A path from the caller that is not inside one of the roots.
    #[error("invalidPath")]
    InvalidPath,
    /// A name that is empty after removing characters files can't contain.
    #[error("invalidName")]
    InvalidName,
    /// A folder can't be moved or copied into itself.
    #[error("intoItself")]
    IntoItself,
    #[error("could not move to the trash: {0}")]
    Trash(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// `COOLMUS_DATA_DIR` if set, else `Documents/Coolmus`.
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("COOLMUS_DATA_DIR") {
        return PathBuf::from(dir);
    }
    directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(|d| d.join("Coolmus")))
        .unwrap_or_else(|| PathBuf::from("Coolmus"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind {
    Root,
    Folder,
    /// A PDF with Coolmus page data.
    Page,
    /// Any other PDF.
    Pdf,
}

/// One entry of the folder tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub kind: NodeKind,
    /// Display name: the folder name, or the file name without extension.
    pub name: String,
    /// Absolute path, `/`-separated.
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Last modification, milliseconds since the Unix epoch.
    pub modified: u64,
    /// Calculated label (folders only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<FolderLabel>,
    /// A root that doesn't exist (e.g. an unplugged drive).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub kind: NodeKind,
    pub path: String,
    pub name: String,
    /// Path of the containing folder.
    pub folder: String,
    pub tags: Vec<String>,
}

/// A page as loaded, with what other programs changed since Coolmus saved it.
#[derive(Debug)]
pub struct Loaded {
    pub page: Page,
    pub changes: Vec<Change>,
}

/// What the store remembers about a PDF without keeping it loaded.
#[derive(Debug, Clone)]
struct Entry {
    stamp: (u64, u64),
    is_page: bool,
    tags: Vec<String>,
}

/// The fields read when indexing; everything else in the page data is skipped.
#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    tags: Vec<String>,
}

pub struct Store {
    roots: Vec<PathBuf>,
    index: HashMap<String, Entry>,
    /// Font for text in written PDFs (see [`coolmus_pdf::system_font`]).
    font: Vec<u8>,
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.to_path_buf(), source }
}

fn millis(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or(StoreError::InvalidPath)?;
    fs::create_dir_all(dir).map_err(io_err(dir))?;
    let name = path.file_name().ok_or(StoreError::InvalidPath)?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp"));
    let mut f = fs::File::create(&tmp).map_err(io_err(&tmp))?;
    f.write_all(bytes).map_err(io_err(&tmp))?;
    f.sync_all().map_err(io_err(&tmp))?;
    drop(f);
    fs::rename(&tmp, path).map_err(io_err(path))
}

/// Makes `name` safe as a file or folder name on Windows, macOS and Linux.
pub fn sanitize_name(name: &str) -> Result<String> {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '-' } else { c })
        .collect();
    let mut cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if cleaned.chars().count() > 120 {
        cleaned = cleaned.chars().take(120).collect::<String>().trim_end().to_string();
    }
    if cleaned.is_empty() || cleaned.starts_with('.') && cleaned.trim_start_matches('.').is_empty() {
        return Err(StoreError::InvalidName);
    }
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        cleaned.push('_');
    }
    Ok(cleaned)
}

/// A folder's settings; a missing or unreadable file means none.
fn read_folder_info(dir: &Path) -> FolderInfo {
    fs::read(dir.join(FOLDER_INFO)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(PAGE_EXT))
}

fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('~') || name.ends_with(".tmp")
}

/// The path as the UI sees it: absolute with `/` separators.
pub fn api_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if cfg!(windows) { s.replace('\\', "/") } else { s.into_owned() }
}

/// `name` without a trailing `.pdf`.
fn stem_of(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case(PAGE_EXT) => stem,
        _ => name,
    }
}

/// Copies a file or a whole folder.
/// "notes.pdf" → ("notes", ".pdf"); a name without a stem keeps it whole.
fn split_ext(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    }
}

fn copy_all(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        fs::create_dir(to).map_err(io_err(to))?;
        for entry in fs::read_dir(from).map_err(io_err(from))?.flatten() {
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ()).map_err(io_err(from))
    }
}

fn index_entry(path: &Path, stamp: (u64, u64)) -> Entry {
    let data = fs::read(path).ok().and_then(|b| coolmus_pdf::page_data(&b).ok().flatten());
    let header = data.and_then(|json| serde_json::from_slice::<Header>(&json).ok());
    Entry { stamp, is_page: header.is_some(), tags: header.map(|h| h.tags).unwrap_or_default() }
}

fn stamp_of(meta: &fs::Metadata) -> (u64, u64) {
    (meta.modified().map(millis).unwrap_or(0), meta.len())
}

impl Store {
    pub fn new(roots: Vec<PathBuf>, font: Vec<u8>) -> Self {
        Self { roots, index: HashMap::new(), font }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
        self.roots = roots;
        self.index.clear();
    }

    /// Resolves a caller-supplied path, refusing anything outside the roots
    /// (relative paths, `..`, other folders).
    fn resolve(&self, path: &str) -> Result<PathBuf> {
        let p = PathBuf::from(path);
        let plain = p.components().all(|c| !matches!(c, Component::ParentDir | Component::CurDir));
        if !p.is_absolute() || !plain || !self.roots.iter().any(|r| p.starts_with(r)) {
            return Err(StoreError::InvalidPath);
        }
        Ok(p)
    }

    /// Like [`Store::resolve`], but not a root itself.
    fn resolve_item(&self, path: &str) -> Result<PathBuf> {
        let p = self.resolve(path)?;
        if self.roots.contains(&p) {
            return Err(StoreError::InvalidPath);
        }
        Ok(p)
    }

    /// `dir/name`, or `name (2)`… if that is taken. `ext` is appended to the
    /// name, `except` is the item being renamed.
    fn unique(dir: &Path, name: &str, ext: &str, except: Option<&Path>) -> Result<PathBuf> {
        let name = sanitize_name(name)?;
        for n in 1.. {
            let candidate = if n == 1 { name.clone() } else { format!("{name} ({n})") };
            let path = dir.join(format!("{candidate}{ext}"));
            // On case-insensitive disks a rename that only changes case "exists".
            let same = except.is_some_and(|e| e.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase());
            if same || !path.exists() {
                return Ok(path);
            }
        }
        unreachable!()
    }

    // ---------- tree & index ----------

    /// Scans all roots, refreshing the index for PDFs that changed.
    pub fn tree(&mut self) -> Vec<Node> {
        let mut seen = Vec::new();
        let nodes = self
            .roots
            .clone()
            .iter()
            .map(|root| {
                let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| api_path(root));
                let modified = fs::metadata(root).and_then(|m| m.modified()).map(millis).unwrap_or(0);
                let missing = !root.is_dir();
                let label = read_folder_info(root).label;
                let children = self.scan(root, &mut seen);
                Node { kind: NodeKind::Root, name, path: api_path(root), tags: Vec::new(), modified, label, missing, children }
            })
            .collect();
        self.index.retain(|k, _| seen.contains(k));
        nodes
    }

    fn scan(&mut self, dir: &Path, seen: &mut Vec<String>) -> Vec<Node> {
        let Ok(read) = fs::read_dir(dir) else { return Vec::new() };
        let mut folders = Vec::new();
        let mut files = Vec::new();
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if hidden(&name) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta.modified().map(millis).unwrap_or(0);
            let key = api_path(&path);
            if meta.is_dir() {
                let children = self.scan(&path, seen);
                let label = read_folder_info(&path).label;
                folders.push(Node { kind: NodeKind::Folder, name, path: key, tags: Vec::new(), modified, label, missing: false, children });
            } else if is_pdf(&path) {
                let stamp = stamp_of(&meta);
                if !self.index.get(&key).is_some_and(|e| e.stamp == stamp) {
                    self.index.insert(key.clone(), index_entry(&path, stamp));
                }
                let e = &self.index[&key];
                let kind = if e.is_page { NodeKind::Page } else { NodeKind::Pdf };
                let node = Node { kind, name: stem_of(&name).to_string(), path: key.clone(), tags: e.tags.clone(), modified, label: None, missing: false, children: Vec::new() };
                seen.push(key);
                files.push(node);
            }
        }
        let by_name = |a: &Node, b: &Node| a.name.to_lowercase().cmp(&b.name.to_lowercase());
        folders.sort_by(by_name);
        files.sort_by(by_name);
        folders.extend(files);
        folders
    }

    fn ensure_index(&mut self) {
        if self.index.is_empty() {
            self.tree();
        }
    }

    /// All tags in use, sorted case-insensitively.
    pub fn all_tags(&mut self) -> Vec<String> {
        self.ensure_index();
        let mut tags: Vec<String> = self.index.values().flat_map(|e| e.tags.iter().cloned()).collect();
        tags.sort_by_key(|t| t.to_lowercase());
        tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        tags
    }

    /// PDFs matching a query such as `"#work bridge"`, most recently modified first.
    pub fn search(&mut self, query: &str) -> Vec<Hit> {
        self.ensure_index();
        let q = Query::parse(query);
        let mut hits: Vec<(u64, Hit)> = self
            .index
            .iter()
            .filter_map(|(key, e)| {
                let (folder, file) = key.rsplit_once('/').unwrap_or(("", key));
                let name = stem_of(file).to_string();
                q.matches(&name, &e.tags).then(|| {
                    let kind = if e.is_page { NodeKind::Page } else { NodeKind::Pdf };
                    (e.stamp.0, Hit { kind, path: key.clone(), name, folder: folder.to_string(), tags: e.tags.clone() })
                })
            })
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        hits.into_iter().map(|h| h.1).collect()
    }

    /// Coolmus pages directly inside `folder`, in tree order.
    pub fn pages_in(&mut self, folder: &str) -> Result<Vec<String>> {
        let dir = self.resolve(folder)?;
        let prefix = format!("{}/", api_path(&dir));
        self.ensure_index();
        let mut pages: Vec<String> = self
            .index
            .iter()
            .filter(|(k, e)| e.is_page && k.strip_prefix(&prefix).is_some_and(|rest| !rest.contains('/')))
            .map(|(k, _)| k.clone())
            .collect();
        pages.sort_by_key(|k| k.to_lowercase());
        Ok(pages)
    }

    // ---------- pages ----------

    pub fn load_page(&self, path: &str) -> Result<Loaded> {
        let file = self.resolve(path)?;
        let bytes = fs::read(&file).map_err(io_err(&file))?;
        let opened = coolmus_pdf::read(&bytes).map_err(|source| StoreError::Pdf { path: file.clone(), source })?;
        let (mut page, changes) = match opened {
            coolmus_pdf::Opened::Intact(page) => (page, Vec::new()),
            coolmus_pdf::Opened::Changed { page, changes } => (page, changes),
            // Any other PDF opens as a page to draw on; the file changes only when saved.
            coolmus_pdf::Opened::Foreign => {
                let background = coolmus_pdf::background(&bytes).map_err(|source| StoreError::Pdf { path: file.clone(), source })?;
                (Page::on_background("", background), Vec::new())
            }
        };
        page.title = stem_of(&file.file_name().unwrap_or_default().to_string_lossy()).to_string();
        Ok(Loaded { page, changes })
    }

    fn write_page(&mut self, file: &Path, page: &Page) -> Result<()> {
        let pdf_err = |source| StoreError::Pdf { path: file.to_path_buf(), source };
        let bytes = match page.background {
            // Drawn on another PDF: the file itself is the background.
            Some(_) => coolmus_pdf::write_over(&fs::read(file).map_err(io_err(file))?, page, &self.font).map_err(pdf_err)?,
            None => coolmus_pdf::write(page, &self.font).map_err(pdf_err)?,
        };
        write_atomic(file, &bytes)?;
        let stamp = fs::metadata(file).map(|m| stamp_of(&m)).unwrap_or_default();
        self.index.insert(api_path(file), Entry { stamp, is_page: true, tags: page.tags.clone() });
        Ok(())
    }

    /// Overwrites an existing page file (the name in `page.title` is ignored;
    /// use [`Store::rename`] to change it).
    pub fn save_page(&mut self, path: &str, page: &Page) -> Result<()> {
        let file = self.resolve_item(path)?;
        if !is_pdf(&file) {
            return Err(StoreError::InvalidPath);
        }
        self.write_page(&file, page)
    }

    /// The PDF a page is drawn on, without Coolmus's layer, for showing
    /// under the editable one.
    pub fn background(&self, path: &str) -> Result<Vec<u8>> {
        let file = self.resolve_item(path)?;
        let bytes = fs::read(&file).map_err(io_err(&file))?;
        coolmus_pdf::strip(&bytes).map_err(|source| StoreError::Pdf { path: file, source })
    }

    /// Writes `page` as a new file named after `page.title` inside `folder`.
    /// Returns its path.
    pub fn create_page(&mut self, folder: &str, page: &Page) -> Result<String> {
        let file = Self::unique(&self.resolve(folder)?, &page.title, ".pdf", None)?;
        self.write_page(&file, page)?;
        Ok(api_path(&file))
    }

    /// One printable PDF of the given pages, in order.
    pub fn print(&self, paths: &[String], title: &str) -> Result<Vec<u8>> {
        let pages = paths.iter().map(|p| self.load_page(p).map(|l| l.page)).collect::<Result<Vec<_>>>()?;
        coolmus_pdf::print(&pages, title, &self.font).map_err(|source| StoreError::Pdf { path: PathBuf::from(title), source })
    }

    // ---------- folders & files ----------

    pub fn create_folder(&mut self, parent: &str, name: &str) -> Result<String> {
        let dir = Self::unique(&self.resolve(parent)?, name, "", None)?;
        fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        Ok(api_path(&dir))
    }

    /// Sets or clears a folder's calculated label.
    pub fn set_folder_label(&mut self, path: &str, label: Option<FolderLabel>) -> Result<()> {
        let dir = self.resolve(path)?;
        if !dir.is_dir() {
            return Err(StoreError::InvalidPath);
        }
        let file = dir.join(FOLDER_INFO);
        let mut info = read_folder_info(&dir);
        info.label = label;
        if info.label.is_none() {
            if file.exists() {
                fs::remove_file(&file).map_err(io_err(&file))?;
            }
            return Ok(());
        }
        write_atomic(&file, &serde_json::to_vec_pretty(&info).expect("folder info serializes"))
    }

    /// Re-keys index entries at or below `from` after a move.
    fn reindex(&mut self, from: &Path, to: &Path) {
        let (from, to) = (api_path(from), api_path(to));
        let moved: Vec<String> =
            self.index.keys().filter(|k| **k == from || k.starts_with(&format!("{from}/"))).cloned().collect();
        for k in moved {
            let e = self.index.remove(&k).expect("key exists");
            self.index.insert(format!("{to}{}", &k[from.len()..]), e);
        }
    }

    fn move_path(&mut self, from: &Path, to: &Path) -> Result<String> {
        if from != to {
            if fs::rename(from, to).is_err() {
                // Another drive: copy, then remove the original.
                copy_all(from, to)?;
                let removed = if from.is_dir() { fs::remove_dir_all(from) } else { fs::remove_file(from) };
                removed.map_err(io_err(from))?;
            }
            self.reindex(from, to);
        }
        Ok(api_path(to))
    }

    /// Splits a file or folder name into (name to keep unique, extension).
    fn name_parts(path: &Path) -> Result<(String, String)> {
        let name = path.file_name().ok_or(StoreError::InvalidPath)?.to_string_lossy().to_string();
        if path.is_dir() {
            return Ok((name, String::new()));
        }
        Ok(split_ext(&name))
    }

    /// Renames a page or folder (give a page's name without extension).
    /// Returns the new path.
    pub fn rename(&mut self, path: &str, new_name: &str) -> Result<String> {
        let from = self.resolve_item(path)?;
        let (_, ext) = Self::name_parts(&from)?;
        let to = Self::unique(from.parent().ok_or(StoreError::InvalidPath)?, new_name, &ext, Some(&from))?;
        self.move_path(&from, &to)
    }

    /// Moves a file or folder into `folder`, which may be in another root.
    /// Returns the new path.
    pub fn move_into(&mut self, path: &str, folder: &str) -> Result<String> {
        let from = self.resolve_item(path)?;
        let dir = self.resolve(folder)?;
        if dir.starts_with(&from) {
            return Err(StoreError::IntoItself);
        }
        if from.parent() == Some(dir.as_path()) {
            return Ok(api_path(&from));
        }
        let (name, ext) = Self::name_parts(&from)?;
        let to = Self::unique(&dir, &name, &ext, None)?;
        self.move_path(&from, &to)
    }

    /// Copies files or folders (from anywhere, e.g. pasted from Explorer) into
    /// `folder`; a name already taken gets " (2)". Returns the new paths.
    pub fn copy_into(&mut self, sources: &[PathBuf], folder: &str) -> Result<Vec<String>> {
        let dir = self.resolve(folder)?;
        let mut out = Vec::new();
        for from in sources {
            if !from.exists() {
                return Err(io_err(from)(std::io::ErrorKind::NotFound.into()));
            }
            if dir.starts_with(from) {
                return Err(StoreError::IntoItself);
            }
            let (name, ext) = Self::name_parts(from)?;
            let to = Self::unique(&dir, &name, &ext, None)?;
            copy_all(from, &to)?;
            out.push(api_path(&to));
        }
        Ok(out)
    }

    /// Writes a new file into `folder` (e.g. dropped from Finder or Explorer,
    /// which give the contents but not the path); a name already taken gets
    /// " (2)". Returns its path.
    pub fn write_file(&mut self, folder: &str, name: &str, bytes: &[u8]) -> Result<String> {
        let (stem, ext) = split_ext(name);
        let file = Self::unique(&self.resolve(folder)?, &stem, &ext, None)?;
        write_atomic(&file, bytes)?;
        Ok(api_path(&file))
    }

    /// Moves a file or folder to the system trash (Recycle Bin), never deleting outright.
    pub fn delete(&mut self, path: &str) -> Result<()> {
        let file = self.resolve_item(path)?;
        trash::delete(&file).map_err(|e| StoreError::Trash(e.to_string()))?;
        let key = api_path(&file);
        self.index.retain(|k, _| *k != key && !k.starts_with(&format!("{key}/")));
        Ok(())
    }

    /// The filesystem path for `path` if it is inside a root (for opening it
    /// in another program).
    pub fn native(&self, path: &str) -> Result<PathBuf> {
        // Normalizes separators to the platform's own.
        Ok(self.resolve(path)?.components().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("coolmus-test-{name}-{}", coolmus_core::new_id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn store(roots: &[&Path]) -> Store {
        Store::new(roots.iter().map(|r| r.to_path_buf()).collect(), Vec::new())
    }

    fn names(nodes: &[Node]) -> Vec<&str> {
        nodes.iter().map(|n| n.name.as_str()).collect()
    }

    #[test]
    fn folders_pages_rename_move_and_search() {
        let dir = temp_dir("tree");
        let root = api_path(&dir);
        let mut store = store(&[&dir]);
        let work = store.create_folder(&root, "Work").unwrap();
        let mut page = Page::new("גשר / Bridge");
        page.tags = vec!["work".into()];
        let path = store.create_page(&work, &page).unwrap();
        assert_eq!(path, format!("{root}/Work/גשר - Bridge.pdf"));
        assert_eq!(store.create_page(&work, &page).unwrap(), format!("{root}/Work/גשר - Bridge (2).pdf"));

        let loaded = store.load_page(&path).unwrap();
        assert_eq!((loaded.page.title.as_str(), loaded.changes.len()), ("גשר - Bridge", 0));
        assert_eq!(store.search("#work").len(), 2);
        assert_eq!(store.search("bridge #work").len(), 2);

        let renamed = store.rename(&path, "Plan").unwrap();
        assert_eq!(renamed, format!("{work}/Plan.pdf"));
        let moved = store.move_into(&renamed, &root).unwrap();
        assert_eq!(moved, format!("{root}/Plan.pdf"));
        assert_eq!(store.search("plan")[0].path, moved);

        let folder = store.rename(&work, "Job").unwrap();
        let tree = store.tree();
        assert_eq!((tree.len(), tree[0].kind), (1, NodeKind::Root));
        assert_eq!(names(&tree[0].children), vec!["Job", "Plan"]);
        assert_eq!(tree[0].children[1].kind, NodeKind::Page);
        assert_eq!(names(&tree[0].children[0].children), vec!["גשר - Bridge (2)"]);
        // A folder can't be moved into itself or its own subfolder.
        let sub = store.create_folder(&folder, "Sub").unwrap();
        assert!(matches!(store.move_into(&folder, &folder), Err(StoreError::IntoItself)));
        assert!(matches!(store.move_into(&folder, &sub), Err(StoreError::IntoItself)));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn several_roots_with_moves_and_copies_between_them() {
        let (a, b, outside) = (temp_dir("a"), temp_dir("b"), temp_dir("outside"));
        let mut store = store(&[&a, &b]);
        let page = store.create_page(&api_path(&a), &Page::new("p")).unwrap();
        let folder = store.create_folder(&api_path(&a), "f").unwrap();

        let moved = store.move_into(&folder, &api_path(&b)).unwrap();
        assert_eq!(moved, api_path(&b.join("f")));
        // Copies of the same item get distinct names.
        let copies = store.copy_into(&[a.join("p.pdf"), a.join("p.pdf")], &moved).unwrap();
        assert_eq!(copies, vec![api_path(&b.join("f/p.pdf")), api_path(&b.join("f/p (2).pdf"))]);
        assert!(store.load_page(&copies[1]).is_ok());

        // Files from outside the roots can be copied in, e.g. pasted from Explorer.
        fs::write(outside.join("notes.txt"), "x").unwrap();
        assert_eq!(store.copy_into(&[outside.join("notes.txt")], &api_path(&a)).unwrap(), vec![api_path(&a.join("notes.txt"))]);
        // Dropped files arrive as contents and get a free name.
        assert_eq!(store.write_file(&api_path(&a), "notes.txt", b"y").unwrap(), api_path(&a.join("notes (2).txt")));
        assert!(matches!(store.write_file(&api_path(&outside), "x.txt", b"y"), Err(StoreError::InvalidPath)));
        assert_eq!(store.write_file(&api_path(&a), "../x.txt", b"y").unwrap(), api_path(&a.join("..-x.txt")), "stays in the folder");
        // …but nothing outside the roots can be changed.
        let foreign = api_path(&outside.join("notes.txt"));
        assert!(matches!(store.rename(&foreign, "x"), Err(StoreError::InvalidPath)));
        assert!(matches!(store.move_into(&page, &api_path(&outside)), Err(StoreError::InvalidPath)));
        assert!(matches!(store.delete(&api_path(&a)), Err(StoreError::InvalidPath)), "a root itself");

        let tree = store.tree();
        assert_eq!(names(&tree), vec![a.file_name().unwrap().to_str().unwrap(), b.file_name().unwrap().to_str().unwrap()]);
        assert_eq!(names(&tree[1].children[0].children), vec!["p", "p (2)"]);
        for d in [a, b, outside] {
            fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn other_pdfs_open_as_backgrounds_to_draw_on() {
        let dir = temp_dir("pdfs");
        let mut doc = lopdf::Document::with_version("1.5");
        let pages = doc.new_object_id();
        let page = doc.add_object(lopdf::dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 10.into(), 10.into()] });
        doc.objects.insert(pages, lopdf::Object::Dictionary(lopdf::dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
        let catalog = doc.add_object(lopdf::dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        doc.save(dir.join("Manual.PDF")).unwrap();

        let mut store = store(&[&dir]);
        let tree = store.tree();
        assert_eq!((tree[0].children[0].name.as_str(), tree[0].children[0].kind), ("Manual", NodeKind::Pdf));
        let path = tree[0].children[0].path.clone();
        let original = fs::read(dir.join("Manual.PDF")).unwrap();

        // Opening doesn't change the file; it's a page whose one sheet is the PDF's page.
        let mut page = store.load_page(&path).unwrap().page;
        assert_eq!(page.background.as_ref().map(|b| b.sheets.len()), Some(1));
        assert_eq!(fs::read(dir.join("Manual.PDF")).unwrap(), original);
        assert_eq!(store.background(&path).unwrap(), original);

        // Saving adds Coolmus's layer; the file becomes a page and still shows the original under it.
        let mut s = coolmus_core::Stroke::new(coolmus_core::Pen::default());
        s.points = vec![coolmus_core::Point::new(1.0, 1.0, 1.0), coolmus_core::Point::new(9.0, 9.0, 1.0)];
        page.strokes.push(s);
        store.save_page(&path, &page).unwrap();
        assert_eq!(store.tree()[0].children[0].kind, NodeKind::Page);
        assert_eq!(store.load_page(&path).unwrap().page.strokes.len(), 1);
        let under = store.background(&path).unwrap();
        assert!(coolmus_pdf::page_data(&under).unwrap().is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_label_is_stored_in_the_folder_and_travels_with_it() {
        let dir = temp_dir("label");
        let mut store = store(&[&dir]);
        let class = store.create_folder(&api_path(&dir), "class of 2021").unwrap();
        let label = FolderLabel::SchoolGrade { first_grade_year: 2024, class_number: "3".into(), template: "כיתה {grade}-{n}".into() };
        store.set_folder_label(&class, Some(label.clone())).unwrap();
        let moved = store.rename(&class, "class of 2021 born").unwrap();
        let tree = &store.tree()[0].children;
        assert_eq!(tree.len(), 1, "the settings file is hidden");
        assert_eq!((tree[0].path.as_str(), tree[0].label.as_ref()), (moved.as_str(), Some(&label)));
        store.set_folder_label(&moved, None).unwrap();
        assert_eq!(store.tree()[0].children[0].label, None);
        assert!(!PathBuf::from(&moved).join(FOLDER_INFO).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_must_stay_inside_a_root() {
        let dir = temp_dir("escape");
        let store = store(&[&dir]);
        let root = api_path(&dir);
        for bad in [format!("{root}/../x.pdf"), format!("{root}/a/../../x.pdf"), "x.pdf".into(), api_path(&std::env::temp_dir())] {
            assert!(store.resolve(&bad).is_err(), "{bad}");
        }
        assert!(store.resolve(&format!("{root}/a/b.pdf")).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(sanitize_name("a:b?c").unwrap(), "a-b-c");
        assert_eq!(sanitize_name("  notes.  ").unwrap(), "notes");
        assert_eq!(sanitize_name("CON").unwrap(), "CON_");
        assert_eq!(sanitize_name("com1.txt").unwrap(), "com1.txt_");
        assert!(sanitize_name(" / ").is_ok());
        assert!(sanitize_name("   ").is_err());
        assert!(sanitize_name("..").is_err());
    }
}
