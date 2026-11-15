//! Prints what `coolmus_pdf::read` finds in each PDF.
//!
//!     cargo run -p coolmus-pdf --example inspect -- <file.pdf> …

use coolmus_pdf::Opened;

fn main() {
    for path in std::env::args().skip(1) {
        let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| coolmus_pdf::read(&b).map_err(|e| e.to_string()));
        let verdict = match result {
            Ok(Opened::Intact(p)) => format!("intact Coolmus page ({} strokes, {} texts, {} images)", p.strokes.len(), p.texts.len(), p.images.len()),
            Ok(Opened::Changed { page, changes }) => format!("Coolmus page ({} strokes) changed elsewhere: {changes:?}", page.strokes.len()),
            Ok(Opened::Foreign) => "not a Coolmus page".into(),
            Err(e) => format!("error: {e}"),
        };
        println!("{path}: {verdict}");
    }
}
