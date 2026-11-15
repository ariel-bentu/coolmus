//! Writes a page as `.sketcher.pdf`, reads it back and checks nothing was lost.
//!
//!     cargo run -p sketch-pdf --example roundtrip -- <font.ttf> <out dir> [page.sketcher …]
//!
//! Without page files it writes a demo page with every kind of content.

use std::path::{Path, PathBuf};
use std::time::Instant;

use base64::Engine;
use sketch_core::{Color, ImageItem, Page, Paper, Pen, PenStyle, Point, Stroke, TextBox};
use sketch_pdf::Opened;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [font, out, pages @ ..] = args.as_slice() else {
        eprintln!("usage: roundtrip <font.ttf> <out dir> [page.sketcher …]");
        std::process::exit(2);
    };
    let font = std::fs::read(font).expect("read font");
    std::fs::create_dir_all(out).unwrap();
    let pages: Vec<(String, Page)> = if pages.is_empty() {
        vec![("demo".into(), demo_page())]
    } else {
        pages.iter().map(|p| load(Path::new(p))).collect()
    };
    let mut ok = true;
    for (name, page) in pages {
        ok &= roundtrip(&name, &page, &font, Path::new(out));
    }
    std::process::exit(if ok { 0 } else { 1 });
}

fn load(path: &Path) -> (String, Page) {
    let mut page: Page = serde_json::from_slice(&std::fs::read(path).expect("read page")).expect("parse page");
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    page.title = name.clone();
    (name, page)
}

fn roundtrip(name: &str, page: &Page, font: &[u8], out: &Path) -> bool {
    let t = Instant::now();
    let pdf = sketch_pdf::write(page, font).expect("write");
    let write_ms = t.elapsed().as_secs_f32() * 1000.0;
    let file: PathBuf = out.join(format!("{name}.sketcher.pdf"));
    std::fs::write(&file, &pdf).unwrap();
    let json_len = serde_json::to_vec(page).unwrap().len();

    let t = Instant::now();
    let opened = sketch_pdf::read(&pdf).expect("read");
    let read_ms = t.elapsed().as_secs_f32() * 1000.0;
    let same = match &opened {
        Opened::Intact(back) => {
            // `title` comes from the file name, not the file.
            let back = Page { title: page.title.clone(), ..back.clone() };
            serde_json::to_value(&back).unwrap() == serde_json::to_value(page).unwrap()
        }
        _ => false,
    };
    println!(
        "{name}: {} strokes, {} texts, {} images → {} ({} KB pdf, {} KB json) write {write_ms:.0} ms, read {read_ms:.0} ms, {}",
        page.strokes.len(),
        page.texts.len(),
        page.images.len(),
        file.display(),
        pdf.len() / 1024,
        json_len / 1024,
        match &opened {
            Opened::Intact(_) if same => "round trip OK".to_string(),
            Opened::Intact(_) => "DATA DIFFERS".to_string(),
            other => format!("UNEXPECTED {other:?}"),
        }
    );
    same
}

fn demo_page() -> Page {
    let mut page = Page::new("demo");
    page.paper = Paper::Lined;

    // Pressure-sensitive wave: thin → thick → thin.
    let mut wave = Stroke::new(Pen { color: Color::rgb(26, 115, 232), width: 8.0, ..Pen::default() });
    wave.points = (0..=200)
        .map(|i| {
            let t = i as f32 / 200.0;
            Point::new(100.0 + t * 600.0, 140.0 + (t * 12.0).sin() * 30.0, (t * std::f32::consts::PI).sin())
        })
        .collect();
    page.strokes.push(wave);

    // A crossing loop with a translucent pencil, to check self-overlaps.
    let mut pencil = Stroke::new(Pen { color: Color::rgb(80, 80, 80), width: 6.0, style: PenStyle::Pencil, ..Pen::default() });
    pencil.points = (0..=300)
        .map(|i| {
            let a = i as f32 / 300.0 * std::f32::consts::TAU * 1.5;
            Point::new(600.0 + a.cos() * 60.0 + a * 10.0, 900.0 + a.sin() * 60.0, 0.8)
        })
        .collect();
    page.strokes.push(pencil);

    for (style, y) in [(PenStyle::Dashed, 230.0), (PenStyle::Dotted, 260.0)] {
        let mut s = Stroke::new(Pen { color: Color::rgb(200, 40, 40), width: 3.0, style, ..Pen::default() });
        s.points = vec![Point::new(100.0, y, 1.0), Point::new(700.0, y, 1.0)];
        page.strokes.push(s);
    }

    let text = |x, y, width, size, text: &str| TextBox {
        id: sketch_core::new_id(),
        x,
        y,
        width,
        text: text.into(),
        size,
        color: Color::BLACK,
    };
    page.texts.push(text(100.0, 300.0, 600.0, 22.0, "שלום עולם! זו פסקה בעברית שמספיק ארוכה כדי להישבר לכמה שורות בתוך התיבה."));
    page.texts.push(text(100.0, 420.0, 600.0, 22.0, "Sketcher גרסה 2.0 עובד עם Hebrew ו-English באותה שורה."));
    page.texts.push(text(100.0, 480.0, 300.0, 18.0, "An English paragraph that wraps inside a narrower box.\n\nAfter an empty line."));
    page.texts.push(text(450.0, 480.0, 250.0, 18.0, "שורה ראשונה\nשורה שנייה (2)"));

    // Highlighter over the first Hebrew line (drawn under the text).
    let mut hl = Stroke::new(Pen { color: Color::rgb(255, 230, 0), width: 24.0, style: PenStyle::Highlighter, ..Pen::default() });
    hl.points = vec![Point::new(400.0, 315.0, 1.0), Point::new(700.0, 315.0, 1.0)];
    page.strokes.push(hl);

    // A picture: a small raster render of a page with one stroke.
    let mut pic = Page::new("pic");
    pic.width = 200.0;
    pic.height = 120.0;
    pic.paper = Paper::Grid;
    let mut s = Stroke::new(Pen { color: Color::rgb(0, 140, 70), width: 10.0, ..Pen::default() });
    s.points = vec![Point::new(20.0, 100.0, 1.0), Point::new(100.0, 20.0, 1.0), Point::new(180.0, 100.0, 1.0)];
    pic.strokes.push(s);
    let png = sketch_render::raster::render_png(&pic, Default::default()).unwrap();
    page.images.push(ImageItem {
        id: sketch_core::new_id(),
        x: 100.0,
        y: 620.0,
        width: 300.0,
        height: 180.0,
        src: format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)),
    });
    page
}
