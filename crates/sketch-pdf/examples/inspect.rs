//! Prints what `sketch_pdf::read` finds in each PDF.
//!
//!     cargo run -p sketch-pdf --example inspect -- <file.pdf> …

use sketch_pdf::Opened;

fn main() {
    for path in std::env::args().skip(1) {
        let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| sketch_pdf::read(&b).map_err(|e| e.to_string()));
        let verdict = match result {
            Ok(Opened::Intact(p)) => format!("intact Sketcher page ({} strokes, {} texts, {} images)", p.strokes.len(), p.texts.len(), p.images.len()),
            Ok(Opened::Changed { page, changes }) => format!("Sketcher page ({} strokes) changed elsewhere: {changes:?}", page.strokes.len()),
            Ok(Opened::Foreign) => "not a Sketcher page".into(),
            Err(e) => format!("error: {e}"),
        };
        println!("{path}: {verdict}");
    }
}
