//! Draws a test layer on a PDF: `draw_over <in.pdf> <out.pdf> <font.ttf>`.
//! Writes `<out>` and `<out>.stripped.pdf` (the layer taken off again).

use sketch_core::{Color, Page, Pen, Point, Stroke, TextBox};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let original = std::fs::read(&args[1]).expect("input PDF");
    let font = std::fs::read(&args[3]).expect("font");
    let bg = sketch_pdf::background(&original).expect("readable PDF");
    println!("{} sheets: {:?}", bg.sheets.len(), bg.sheets);
    let mut page = Page::on_background("test", bg);
    for i in 0..page.sheet_count() {
        let r = page.sheet_rect(i);
        // A frame just inside the sheet, a diagonal, a highlighter band and a note.
        let mut frame = Stroke::new(Pen { color: Color::rgb(220, 30, 30), width: 3.0, ..Default::default() });
        frame.points = [(8.0, 8.0), (r.width() - 8.0, 8.0), (r.width() - 8.0, r.height() - 8.0), (8.0, r.height() - 8.0), (8.0, 8.0)]
            .map(|(x, y)| Point::new(x, r.min_y + y, 1.0))
            .to_vec();
        page.strokes.push(frame);
        let mut hl = Stroke::new(Pen { style: sketch_core::PenStyle::Highlighter, color: Color::rgb(255, 230, 0), width: 18.0, ..Default::default() });
        hl.points = vec![Point::new(60.0, r.min_y + 150.0, 1.0), Point::new(r.width() - 60.0, r.min_y + 150.0, 1.0)];
        page.strokes.push(hl);
        page.texts.push(TextBox {
            id: sketch_core::new_id(),
            x: r.width() - 360.0,
            y: r.min_y + 30.0,
            width: 320.0,
            text: format!("הערה בעמוד {} — note on page {}", i + 1, i + 1),
            size: 20.0,
            color: Color::rgb(30, 60, 200),
        });
    }
    let written = sketch_pdf::write_over(&original, &page, &font).expect("write");
    std::fs::write(&args[2], &written).unwrap();
    let again = sketch_pdf::write_over(&written, &page, &font).expect("write again");
    println!("original {} B, with layer {} B, saved again {} B", original.len(), written.len(), again.len());
    let stripped = sketch_pdf::strip(&written).unwrap();
    std::fs::write(format!("{}.stripped.pdf", args[2]), &stripped).unwrap();
    println!("read back: {:?}", match sketch_pdf::read(&written).unwrap() {
        sketch_pdf::Opened::Intact(p) => format!("intact, {} strokes, {} texts", p.strokes.len(), p.texts.len()),
        other => format!("{other:?}"),
    });
}
