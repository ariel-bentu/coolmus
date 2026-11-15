//! Imports OneNote files into a folder as Sketcher pages, like the app does.
//!
//!     cargo run -p onenote-import --example import -- <out dir> <file.onetoc2|file.one> …

use std::path::PathBuf;

use onenote_import::Entry;
use sketch_store::{api_path, Store};

fn write(store: &mut Store, entry: &Entry, folder: &str) {
    match entry {
        Entry::Section { name, pages } => {
            let dir = store.create_folder(folder, name).unwrap();
            for page in pages {
                let path = store.create_page(&dir, page).unwrap();
                println!("  {path}: {} strokes, {} texts, {} images, {}×{}", page.strokes.len(), page.texts.len(), page.images.len(), page.width, page.height);
            }
        }
        Entry::Group { name, entries } => {
            let dir = store.create_folder(folder, name).unwrap();
            for e in entries {
                write(store, e, &dir);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(&args[0]);
    std::fs::create_dir_all(&out).unwrap();
    let mut store = Store::new(vec![out.clone()], sketch_pdf::system_font().unwrap_or_default());
    for file in &args[1..] {
        let entry = onenote_import::read_file(file.as_ref()).unwrap();
        println!("{file}: {} pages", entry.page_count());
        write(&mut store, &entry, &api_path(&out));
    }
}
