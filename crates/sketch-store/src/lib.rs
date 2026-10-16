//! Local persistence: pages are files in ordinary folders.
//!
//! ```text
//! <root>/                       a folder you choose (e.g. inside OneDrive)
//!   Work/
//!     Bridges/
//!       Plan.sketcher           one page: JSON with vector ink, text, pictures
//!   Ideas.sketcher
//! ```
//! The folder tree *is* the organization: there is no index file, so any sync
//! tool can copy the folder safely. The page name is the file name. Paths
//! exchanged with callers are relative to the root and use `/` separators.
//! Writes are atomic (temp file + rename) so a crash never corrupts a page.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sketch_core::tags::Query;
use sketch_core::{OneNotePageLink, Page};

/// File extension of page files.
pub const PAGE_EXT: &str = "sketcher";

/// Hidden per-folder settings file. It lives inside the folder, so it moves,
/// syncs and is deleted together with it.
pub const FOLDER_INFO: &str = ".sketcher-folder.json";

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
    #[error("corrupt page {path}: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    /// A path from the caller that is absolute or leaves the root.
    #[error("invalidPath")]
    InvalidPath,
    /// A name that is empty after removing characters files can't contain.
    #[error("invalidName")]
    InvalidName,
    #[error("could not move to the trash: {0}")]
    Trash(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// `SKETCHER_DATA_DIR` if set, else `Documents/Sketcher`.
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("SKETCHER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(|d| d.join("Sketcher")))
        .unwrap_or_else(|| PathBuf::from("Sketcher"))
}

/// Where versions before the folder layout kept their library
/// (`library.json` + `pages/`), for one-time migration.
pub fn legacy_root() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Sketcher").map(|d| d.data_dir().to_path_buf())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind {
    Folder,
    Page,
}

/// One entry of the folder tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub kind: NodeKind,
    /// Display name: the folder name, or the page file name without extension.
    pub name: String,
    /// Relative path, `/`-separated.
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Last modification, milliseconds since the Unix epoch.
    pub modified: u64,
    /// Calculated label (folders only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<FolderLabel>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    pub name: String,
    /// Relative path of the containing folder ("" for the root).
    pub folder: String,
    pub tags: Vec<String>,
}

/// What the store remembers about a page file without keeping it loaded.
#[derive(Debug, Clone)]
struct Entry {
    stamp: (u64, u64),
    tags: Vec<String>,
    onenote_page: Option<String>,
}

/// The few fields read when indexing; everything else in the file is skipped.
#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    onenote: Option<OneNotePageLink>,
}

pub struct Store {
    root: PathBuf,
    index: HashMap<String, Entry>,
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

fn is_page_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(PAGE_EXT))
}

fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('~') || name.ends_with(".tmp")
}

fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

fn join_rel(folder: &str, name: &str) -> String {
    if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") }
}

impl Store {
    /// Opens (creating if needed) the folder at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(io_err(&root))?;
        Ok(Self { root, index: HashMap::new() })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a caller-supplied relative path, refusing anything that could
    /// escape the root (`..`, absolute paths, drive prefixes).
    fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let mut out = self.root.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            let mut comps = Path::new(part).components();
            match (comps.next(), comps.next()) {
                (Some(Component::Normal(c)), None) if !part.contains('\\') => out.push(c),
                _ => return Err(StoreError::InvalidPath),
            }
        }
        Ok(out)
    }

    fn rel_of(&self, path: &Path) -> String {
        let rel = path.strip_prefix(&self.root).unwrap_or(path);
        rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
    }

    /// `folder/name.sketcher`, or `name (2).sketcher`… if that is taken.
    fn unique(&self, folder: &str, name: &str, page: bool, except: Option<&Path>) -> Result<(PathBuf, String)> {
        let name = sanitize_name(name)?;
        let dir = self.resolve(folder)?;
        for n in 1.. {
            let candidate = if n == 1 { name.clone() } else { format!("{name} ({n})") };
            let file = if page { format!("{candidate}.{PAGE_EXT}") } else { candidate };
            let path = dir.join(&file);
            // On case-insensitive disks a rename that only changes case "exists".
            let same = except.is_some_and(|e| e.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase());
            if same || !path.exists() {
                return Ok((path, join_rel(folder, &file)));
            }
        }
        unreachable!()
    }

    // ---------- tree & index ----------

    /// Scans the folder tree, refreshing the tag index for files that changed.
    pub fn tree(&mut self) -> Vec<Node> {
        let mut seen = Vec::new();
        let root = self.root.clone();
        let nodes = self.scan(&root, &mut seen);
        self.index.retain(|k, _| seen.contains(k));
        nodes
    }

    fn scan(&mut self, dir: &Path, seen: &mut Vec<String>) -> Vec<Node> {
        let Ok(read) = fs::read_dir(dir) else { return Vec::new() };
        let mut folders = Vec::new();
        let mut pages = Vec::new();
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if hidden(&name) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta.modified().map(millis).unwrap_or(0);
            let rel = self.rel_of(&path);
            if meta.is_dir() {
                let children = self.scan(&path, seen);
                let label = read_folder_info(&path).label;
                folders.push(Node { kind: NodeKind::Folder, name, path: rel, tags: Vec::new(), modified, label, children });
            } else if is_page_file(&path) {
                let stamp = (modified, meta.len());
                let fresh = self.index.get(&rel).is_some_and(|e| e.stamp == stamp);
                if !fresh {
                    let header = fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Header>(&b).ok());
                    let (tags, onenote_page) = header.map(|h| (h.tags, h.onenote.map(|l| l.page_id))).unwrap_or_default();
                    self.index.insert(rel.clone(), Entry { stamp, tags, onenote_page });
                }
                let tags = self.index[&rel].tags.clone();
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                seen.push(rel.clone());
                pages.push(Node { kind: NodeKind::Page, name: stem, path: rel, tags, modified, label: None, children: Vec::new() });
            }
        }
        let by_name = |a: &Node, b: &Node| a.name.to_lowercase().cmp(&b.name.to_lowercase());
        folders.sort_by(by_name);
        pages.sort_by(by_name);
        folders.extend(pages);
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

    /// Pages matching a query such as `"#work bridge"`, most recently modified first.
    pub fn search(&mut self, query: &str) -> Vec<Hit> {
        self.ensure_index();
        let q = Query::parse(query);
        let mut hits: Vec<(u64, Hit)> = self
            .index
            .iter()
            .filter_map(|(rel, e)| {
                let file = rel.rsplit('/').next().unwrap_or(rel);
                let name = file.strip_suffix(&format!(".{PAGE_EXT}")).unwrap_or(file).to_string();
                q.matches(&name, &e.tags).then(|| {
                    (e.stamp.0, Hit { path: rel.clone(), name, folder: parent_of(rel).to_string(), tags: e.tags.clone() })
                })
            })
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        hits.into_iter().map(|h| h.1).collect()
    }

    /// A page directly inside `folder` that was imported from this OneNote page.
    pub fn find_imported(&mut self, folder: &str, onenote_page: &str) -> Option<String> {
        self.ensure_index();
        self.index
            .iter()
            .find(|(rel, e)| parent_of(rel) == folder && e.onenote_page.as_deref() == Some(onenote_page))
            .map(|(rel, _)| rel.clone())
    }

    // ---------- pages ----------

    pub fn load_page(&self, rel: &str) -> Result<Page> {
        let path = self.resolve(rel)?;
        let bytes = fs::read(&path).map_err(io_err(&path))?;
        let mut page: Page =
            serde_json::from_slice(&bytes).map_err(|source| StoreError::Json { path: path.clone(), source })?;
        page.title = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        Ok(page)
    }

    /// Overwrites an existing page file (the name in `page.title` is ignored;
    /// use [`Store::rename`] to change it).
    pub fn save_page(&mut self, rel: &str, page: &Page) -> Result<()> {
        let path = self.resolve(rel)?;
        if !is_page_file(&path) {
            return Err(StoreError::InvalidPath);
        }
        let bytes = serde_json::to_vec(page).expect("page serializes");
        write_atomic(&path, &bytes)?;
        self.note_saved(rel, &path, page);
        Ok(())
    }

    fn note_saved(&mut self, rel: &str, path: &Path, page: &Page) {
        let stamp = fs::metadata(path).map(|m| (m.modified().map(millis).unwrap_or(0), m.len())).unwrap_or_default();
        let onenote_page = page.onenote.as_ref().map(|l| l.page_id.clone());
        self.index.insert(rel.to_string(), Entry { stamp, tags: page.tags.clone(), onenote_page });
    }

    /// Writes `page` as a new file named after `page.title` inside `folder`.
    /// Returns its relative path.
    pub fn create_page(&mut self, folder: &str, page: &Page) -> Result<String> {
        let (path, rel) = self.unique(folder, &page.title, true, None)?;
        let bytes = serde_json::to_vec(page).expect("page serializes");
        write_atomic(&path, &bytes)?;
        self.note_saved(&rel, &path, page);
        Ok(rel)
    }

    // ---------- folders & files ----------

    pub fn create_folder(&mut self, parent: &str, name: &str) -> Result<String> {
        let (path, rel) = self.unique(parent, name, false, None)?;
        fs::create_dir_all(&path).map_err(io_err(&path))?;
        Ok(rel)
    }

    /// Sets or clears a folder's calculated label.
    pub fn set_folder_label(&mut self, rel: &str, label: Option<FolderLabel>) -> Result<()> {
        let dir = self.resolve(rel)?;
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

    fn move_path(&mut self, from_rel: &str, to: &Path, to_rel: &str) -> Result<String> {
        let from = self.resolve(from_rel)?;
        if from == self.root {
            return Err(StoreError::InvalidPath);
        }
        if from != *to {
            fs::rename(&from, to).map_err(io_err(&from))?;
        }
        // Re-key index entries under the old path.
        let moved: Vec<(String, Entry)> = self
            .index
            .iter()
            .filter(|(k, _)| *k == from_rel || k.starts_with(&format!("{from_rel}/")))
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect();
        for (k, e) in moved {
            self.index.remove(&k);
            self.index.insert(format!("{to_rel}{}", &k[from_rel.len()..]), e);
        }
        Ok(to_rel.to_string())
    }

    /// Renames a page (give the name without extension) or a folder. Returns the new path.
    pub fn rename(&mut self, rel: &str, new_name: &str) -> Result<String> {
        let from = self.resolve(rel)?;
        let page = is_page_file(&from) && from.is_file();
        let (to, to_rel) = self.unique(parent_of(rel), new_name, page, Some(&from))?;
        self.move_path(rel, &to, &to_rel)
    }

    /// Moves a page or folder into `folder` ("" = the root). Returns the new path.
    pub fn move_into(&mut self, rel: &str, folder: &str) -> Result<String> {
        if folder == rel || folder.starts_with(&format!("{rel}/")) {
            return Err(StoreError::InvalidPath); // a folder can't go inside itself
        }
        if parent_of(rel) == folder {
            return Ok(rel.to_string());
        }
        let from = self.resolve(rel)?;
        let page = is_page_file(&from) && from.is_file();
        let name = from.file_name().ok_or(StoreError::InvalidPath)?.to_string_lossy().to_string();
        let name = if page { name.strip_suffix(&format!(".{PAGE_EXT}")).unwrap_or(&name).to_string() } else { name };
        let (to, to_rel) = self.unique(folder, &name, page, None)?;
        self.move_path(rel, &to, &to_rel)
    }

    /// Moves a page or folder to the system trash (Recycle Bin), never deleting outright.
    pub fn delete(&mut self, rel: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        if path == self.root {
            return Err(StoreError::InvalidPath);
        }
        trash::delete(&path).map_err(|e| StoreError::Trash(e.to_string()))?;
        self.index.retain(|k, _| k != rel && !k.starts_with(&format!("{rel}/")));
        Ok(())
    }

    // ---------- migration ----------

    /// Copies a library from versions before the folder layout
    /// (`library.json`: notebooks → sections → pages) into folders:
    /// notebook → folder, section → subfolder, page → file. The old
    /// `library.json` is renamed so this happens only once. Returns the
    /// number of pages copied.
    pub fn migrate_legacy(&mut self, old_root: &Path) -> Result<usize> {
        let lib_path = old_root.join("library.json");
        if !lib_path.is_file() || old_root == self.root {
            return Ok(0);
        }
        let bytes = fs::read(&lib_path).map_err(io_err(&lib_path))?;
        let lib: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|source| StoreError::Json { path: lib_path.clone(), source })?;
        let text = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let list = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
        let mut copied = 0;
        for nb in list(&lib, "notebooks") {
            let nb_rel = self.create_folder("", &text(&nb, "name")).or_else(|_| self.create_folder("", "Notebook"))?;
            for sec in list(&nb, "sections") {
                let sec_rel =
                    self.create_folder(&nb_rel, &text(&sec, "name")).or_else(|_| self.create_folder(&nb_rel, "Section"))?;
                for meta in list(&sec, "pages") {
                    let file = old_root.join("pages").join(format!("{}.json", text(&meta, "id")));
                    let Some(mut page) = fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Page>(&b).ok()) else {
                        continue;
                    };
                    page.title = text(&meta, "title");
                    if sanitize_name(&page.title).is_err() {
                        page.title = "Page".into();
                    }
                    page.move_ink_onto_page();
                    self.create_page(&sec_rel, &page)?;
                    copied += 1;
                }
            }
        }
        let done = old_root.join("library.json.migrated");
        fs::rename(&lib_path, &done).map_err(io_err(&lib_path))?;
        Ok(copied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sketcher-test-{name}-{}", sketch_core::new_id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn names(nodes: &[Node]) -> Vec<String> {
        nodes.iter().map(|n| n.path.clone()).collect()
    }

    #[test]
    fn folders_pages_rename_move_and_search() {
        let dir = temp_dir("tree");
        let mut store = Store::open(&dir).unwrap();
        let work = store.create_folder("", "Work").unwrap();
        let mut page = Page::new("גשר / Bridge");
        page.tags = vec!["work".into()];
        let rel = store.create_page(&work, &page).unwrap();
        assert_eq!(rel, "Work/גשר - Bridge.sketcher");
        assert_eq!(store.create_page(&work, &page).unwrap(), "Work/גשר - Bridge (2).sketcher");

        let loaded = store.load_page(&rel).unwrap();
        assert_eq!(loaded.title, "גשר - Bridge");
        assert_eq!(store.search("#work").len(), 2);
        assert_eq!(store.search("bridge #work").len(), 2);

        let renamed = store.rename(&rel, "Plan").unwrap();
        assert_eq!(renamed, "Work/Plan.sketcher");
        let moved = store.move_into(&renamed, "").unwrap();
        assert_eq!(moved, "Plan.sketcher");
        assert_eq!(store.search("plan")[0].path, "Plan.sketcher");

        let folder = store.rename(&work, "Job").unwrap();
        let tree = store.tree();
        assert_eq!(names(&tree), vec!["Job", "Plan.sketcher"]);
        assert_eq!(names(&tree[0].children), vec!["Job/גשר - Bridge (2).sketcher"]);
        // A folder can't be moved into itself or its own subfolder.
        let sub = store.create_folder(&folder, "Sub").unwrap();
        assert!(store.move_into(&folder, &folder).is_err());
        assert!(store.move_into(&folder, &sub).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_label_is_stored_in_the_folder_and_travels_with_it() {
        let dir = temp_dir("label");
        let mut store = Store::open(&dir).unwrap();
        let class = store.create_folder("", "class of 2021").unwrap();
        let label = FolderLabel::SchoolGrade { first_grade_year: 2024, class_number: "3".into(), template: "כיתה {grade}-{n}".into() };
        store.set_folder_label(&class, Some(label.clone())).unwrap();
        let moved = store.rename(&class, "class of 2021 born").unwrap();
        let tree = store.tree();
        assert_eq!(tree.len(), 1, "the settings file is hidden");
        assert_eq!((tree[0].path.as_str(), tree[0].label.as_ref()), (moved.as_str(), Some(&label)));
        store.set_folder_label(&moved, None).unwrap();
        assert_eq!(store.tree()[0].label, None);
        assert!(!dir.join(&moved).join(FOLDER_INFO).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_cannot_escape_the_root() {
        let dir = temp_dir("escape");
        let store = Store::open(&dir).unwrap();
        for bad in ["../x.sketcher", "a/../../x.sketcher", "C:\\x.sketcher", "a\\..\\..\\x", "./x"] {
            assert!(store.resolve(bad).is_err(), "{bad}");
        }
        // A leading slash is still relative to the root.
        assert!(store.resolve("/etc/passwd").unwrap().starts_with(&dir));
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

    #[test]
    fn legacy_library_is_migrated_once() {
        let old = temp_dir("legacy");
        fs::create_dir_all(old.join("pages")).unwrap();
        let page = Page::new("x");
        fs::write(old.join("pages").join(format!("{}.json", page.id)), serde_json::to_vec(&page).unwrap()).unwrap();
        let lib = serde_json::json!({ "notebooks": [{ "id": "n", "name": "מחברת", "sections": [
            { "id": "s", "name": "Quick Notes", "pages": [{ "id": page.id, "title": "First page", "tags": [] }] }
        ]}]});
        fs::write(old.join("library.json"), lib.to_string()).unwrap();

        let dir = temp_dir("migrated");
        let mut store = Store::open(&dir).unwrap();
        assert_eq!(store.migrate_legacy(&old).unwrap(), 1);
        assert!(store.load_page("מחברת/Quick Notes/First page.sketcher").is_ok());
        assert_eq!(store.migrate_legacy(&old).unwrap(), 0);
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&old).unwrap();
    }
}
