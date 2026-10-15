//! Local persistence.
//!
//! Layout under the data directory:
//! ```text
//! library.json          notebooks → sections → page summaries (titles, tags)
//! pages/<uuid>.json     full page content (strokes)
//! ```
//! Writes are atomic (temp file + rename) so a crash never corrupts data.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sketch_core::tags::Query;
use sketch_core::{Id, Library, Notebook, Page, PageLocation, PageMeta, Section};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("corrupt file {path}: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("section {0} not found")]
    NoSection(Id),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    root: PathBuf,
    pub library: Library,
}

/// `SKETCHER_DATA_DIR` if set, else the platform data directory
/// (e.g. `%APPDATA%\Sketcher\data` on Windows).
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("SKETCHER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("", "", "Sketcher")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("sketcher-data"))
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.to_path_buf(), source }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = fs::File::create(&tmp).map_err(io_err(&tmp))?;
    f.write_all(bytes).map_err(io_err(&tmp))?;
    f.sync_all().map_err(io_err(&tmp))?;
    drop(f);
    fs::rename(&tmp, path).map_err(io_err(path))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(io_err(path))?;
    serde_json::from_slice(&bytes).map_err(|source| StoreError::Json { path: path.to_path_buf(), source })
}

impl Store {
    /// Opens (or initializes) a store at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("pages")).map_err(io_err(&root))?;
        let lib_path = root.join("library.json");
        let library = if lib_path.exists() { read_json(&lib_path)? } else { Library::default() };
        let mut store = Self { root, library };
        if store.library.notebooks.is_empty() {
            let mut nb = Notebook::new("My Notebook");
            nb.sections.push(Section::new("Quick Notes"));
            store.library.notebooks.push(nb);
            store.save_library()?;
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn page_path(&self, id: Id) -> PathBuf {
        self.root.join("pages").join(format!("{id}.json"))
    }

    pub fn save_library(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.library).expect("library serializes");
        write_atomic(&self.root.join("library.json"), &bytes)
    }

    pub fn load_page(&self, id: Id) -> Result<Page> {
        read_json(&self.page_path(id))
    }

    /// Writes the page and refreshes its summary in `section`.
    pub fn save_page(&mut self, section: Id, page: &Page) -> Result<()> {
        let bytes = serde_json::to_vec(page).expect("page serializes");
        write_atomic(&self.page_path(page.id), &bytes)?;
        if !self.library.upsert_page_meta(section, page.meta()) {
            return Err(StoreError::NoSection(section));
        }
        self.save_library()
    }

    pub fn delete_page(&mut self, id: Id) -> Result<()> {
        self.library.remove_page(id);
        let path = self.page_path(id);
        if path.exists() {
            fs::remove_file(&path).map_err(io_err(&path))?;
        }
        self.save_library()
    }

    pub fn delete_section(&mut self, section: Id) -> Result<()> {
        let pages: Vec<Id> =
            self.library.section(section).map(|s| s.pages.iter().map(|p| p.id).collect()).unwrap_or_default();
        for id in pages {
            self.delete_page(id)?;
        }
        for nb in &mut self.library.notebooks {
            nb.sections.retain(|s| s.id != section);
        }
        self.save_library()
    }

    pub fn delete_notebook(&mut self, notebook: Id) -> Result<()> {
        let sections: Vec<Id> = self
            .library
            .notebooks
            .iter()
            .filter(|n| n.id == notebook)
            .flat_map(|n| n.sections.iter().map(|s| s.id))
            .collect();
        for s in sections {
            self.delete_section(s)?;
        }
        self.library.notebooks.retain(|n| n.id != notebook);
        self.save_library()
    }

    /// Pages matching a query such as `"#work sketch"`, most recently modified first.
    pub fn search(&self, query: &str) -> Vec<(PageLocation, PageMeta)> {
        let q = Query::parse(query);
        let mut hits: Vec<(PageLocation, PageMeta)> = self
            .library
            .notebooks
            .iter()
            .flat_map(|n| {
                n.sections.iter().flat_map(move |s| {
                    s.pages.iter().map(move |p| {
                        (PageLocation { notebook: n.id, section: s.id, page: p.id }, p.clone())
                    })
                })
            })
            .filter(|(_, m)| q.matches(m))
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.1.modified));
        hits
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

    #[test]
    fn roundtrip_and_search() {
        let dir = temp_dir("rt");
        let mut store = Store::open(&dir).unwrap();
        let section = store.library.notebooks[0].sections[0].id;
        let mut page = Page::new("Bridge design");
        page.tags = vec!["work".into()];
        store.save_page(section, &page).unwrap();

        let reopened = Store::open(&dir).unwrap();
        assert_eq!(reopened.load_page(page.id).unwrap(), page);
        assert_eq!(reopened.search("#work").len(), 1);
        assert_eq!(reopened.search("bridge").len(), 1);
        assert_eq!(reopened.search("#home").len(), 0);

        let mut store = reopened;
        store.delete_section(section).unwrap();
        assert!(store.load_page(page.id).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
