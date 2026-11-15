//! Imports OneNote notebooks and sections from files as Coolmus pages.
//!
//! Reads `.onetoc2` (a notebook: its folder holds one `.one` per section) and
//! `.one` (a single section), as downloaded from OneDrive or saved by OneNote
//! desktop, with the `onenote_parser` crate. Ink becomes strokes, typed text
//! becomes text boxes and pictures stay pictures, at their positions.
//!
//! OneNote units: layout offsets and sizes are in half-inches, ink in
//! 0.01 mm, font sizes in half-points. Coolmus uses pixels at 96 DPI.

use std::io::Read;
use std::path::Path;

use base64::Engine;
use onenote_parser::contents::{Content, EmbeddedObject, Image, Ink, Outline, OutlineElement, OutlineItem, RichText};
use onenote_parser::page::PageContent;
use onenote_parser::property::common::ColorRef;
use onenote_parser::section::{Section, SectionEntry};
use onenote_parser::Parser;
use coolmus_core::{new_id, Color, ImageItem, Page, Pen, PenStyle, Point, Stroke, TextBox};
use typed_path::TypedPath;

const PX_PER_HALF_INCH: f32 = 48.0;
const PX_PER_INK_UNIT: f32 = 96.0 / 2540.0;
/// Ink offsets are in half-inches; ink points in 0.01 mm.
const INK_UNITS_PER_HALF_INCH: f32 = 1270.0;
/// OneNote's default text size, 11 pt.
const DEFAULT_TEXT_PX: f32 = 11.0 * 96.0 / 72.0;
/// Matches the UI's text line height.
const LINE_HEIGHT: f32 = 1.3;
const MARGIN: f32 = 40.0;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a OneNote file (.one or .onetoc2)")]
    NotOneNote,
    #[error("could not read the OneNote file: {0}")]
    Parse(String),
}

/// A notebook or section as folders of pages, ready to be written.
#[derive(Debug)]
pub enum Entry {
    Section { name: String, pages: Vec<Page> },
    Group { name: String, entries: Vec<Entry> },
}

impl Entry {
    pub fn page_count(&self) -> usize {
        match self {
            Entry::Section { pages, .. } => pages.len(),
            Entry::Group { entries, .. } => entries.iter().map(Entry::page_count).sum(),
        }
    }
}

/// Reads a `.onetoc2` notebook (as a group named after its folder) or a
/// `.one` section.
pub fn read_file(path: &Path) -> Result<Entry, Error> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let display = path.to_string_lossy();
    let typed = TypedPath::derive(display.as_ref());
    let parser = Parser::new();
    let parse_err = |e: onenote_parser::errors::Error| Error::Parse(e.to_string());
    match ext.as_deref() {
        Some("onetoc2") => {
            let notebook = parser.parse_notebook(typed).map_err(parse_err)?;
            let name = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "OneNote".into());
            Ok(Entry::Group { name, entries: entries(notebook.entries()) })
        }
        Some("one") => {
            // Named after the file, as the user sees it.
            let mut entry = section(&parser.parse_section(typed).map_err(parse_err)?);
            if let (Entry::Section { name, .. }, Some(stem)) = (&mut entry, path.file_stem()) {
                *name = stem.to_string_lossy().into_owned();
            }
            Ok(entry)
        }
        _ => Err(Error::NotOneNote),
    }
}

/// OneNote's own recycle bin, which a notebook download includes.
fn is_recycle_bin(name: &str) -> bool {
    name.starts_with("OneNote_RecycleBin") || name.starts_with("OneNote_DeletedPages")
}

fn entries(list: &[SectionEntry]) -> Vec<Entry> {
    list.iter()
        .filter_map(|e| match e {
            SectionEntry::Section(s) if !is_recycle_bin(s.display_name()) => Some(section(s)),
            SectionEntry::SectionGroup(g) if !is_recycle_bin(g.display_name()) => {
                Some(Entry::Group { name: g.display_name().to_string(), entries: entries(g.entries()) })
            }
            _ => None,
        })
        .collect()
}

fn section(s: &Section) -> Entry {
    let pages = s.page_series().iter().flat_map(|ps| ps.pages()).map(convert_page).collect();
    Entry::Section { name: s.display_name().to_string(), pages }
}

fn millis(t: time::UtcDateTime) -> u64 {
    (t.unix_timestamp_nanos() / 1_000_000).max(0) as u64
}

/// Converts one OneNote page. Its title becomes the page name.
pub fn convert_page(src: &onenote_parser::page::Page) -> Page {
    let title = src.title_text().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("Untitled");
    let mut page = Page::new(title);
    page.created = millis(src.created_time());
    page.modified = millis(src.updated_time());
    for content in src.contents() {
        match content {
            PageContent::Outline(o) => outline(&mut page, o),
            PageContent::Image(i) => {
                image(&mut page, i, None);
            }
            PageContent::Ink(i) => ink(&mut page, i, (0.0, 0.0)),
            _ => {}
        }
    }
    place_on_page(&mut page);
    page
}

// ---- ink ----

/// Adds an ink object's strokes; `origin` is its container's position in ink units.
fn ink(page: &mut Page, src: &Ink, origin: (f32, f32)) {
    let origin = (
        origin.0 + src.offset_horizontal().unwrap_or(0.0) * INK_UNITS_PER_HALF_INCH,
        origin.1 + src.offset_vertical().unwrap_or(0.0) * INK_UNITS_PER_HALF_INCH,
    );
    for s in src.ink_strokes() {
        // OneNote stores the color as 0x00BBGGRR.
        let color = s.color().map(|c| Color::rgb(c as u8, (c >> 8) as u8, (c >> 16) as u8)).unwrap_or(Color::BLACK);
        let highlighter = s.transparency().is_some();
        let pen = Pen {
            color,
            width: (s.width() * PX_PER_INK_UNIT).max(0.5),
            style: if highlighter { PenStyle::Highlighter } else { PenStyle::Solid },
            // The parser reads no pressure, so strokes keep the width OneNote gives them.
            pressure_sensitive: false,
        };
        let mut stroke = Stroke::new(pen);
        stroke.points = s
            .absolute_path()
            .iter()
            .map(|p| Point::new((p.x() + origin.0) * PX_PER_INK_UNIT, (p.y() + origin.1) * PX_PER_INK_UNIT, 1.0))
            .collect();
        if !stroke.points.is_empty() {
            page.strokes.push(stroke);
        }
    }
    for group in src.child_groups() {
        ink(page, group, origin);
    }
}

// ---- outlines: text, plus ink and pictures placed inside them ----

/// Lays out an outline top to bottom, one text box per run of paragraphs.
struct Flow<'p> {
    page: &'p mut Page,
    x: f32,
    y: f32,
    width: f32,
    /// Paragraphs not yet written as a text box: (text, size, color).
    pending: Vec<(String, f32, Color)>,
}

impl Flow<'_> {
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let (_, size, color) = self.pending[0];
        let text: Vec<String> = self.pending.drain(..).map(|(t, ..)| t).collect();
        let lines = text.len() as f32;
        self.page.texts.push(TextBox { id: new_id(), x: self.x, y: self.y, width: self.width, text: text.join("\n"), size, color });
        self.y += lines * size * LINE_HEIGHT;
    }

    fn items(&mut self, items: &[OutlineItem], depth: usize) {
        for item in items {
            match item {
                OutlineItem::Group(g) => self.items(g.outlines(), depth),
                OutlineItem::Element(e) => {
                    self.element(e, depth);
                    self.items(e.children(), depth + 1);
                }
            }
        }
    }

    fn element(&mut self, e: &OutlineElement, depth: usize) {
        for content in e.contents() {
            match content {
                Content::RichText(r) => self.rich_text(r, depth),
                Content::Table(t) => {
                    for row in t.contents() {
                        let cells: Vec<String> = row.contents().iter().map(|c| cell_text(c.contents())).collect();
                        self.pending.push((cells.join(" | "), DEFAULT_TEXT_PX, Color::BLACK));
                    }
                }
                Content::Image(i) => {
                    self.flush();
                    let (_, h) = image(self.page, i, Some((self.x, self.y)));
                    self.y += h;
                }
                Content::Ink(i) => {
                    self.flush();
                    ink(self.page, i, (self.x / PX_PER_INK_UNIT, self.y / PX_PER_INK_UNIT));
                }
                _ => {}
            }
        }
    }

    fn rich_text(&mut self, r: &RichText, depth: usize) {
        for obj in r.embedded_objects() {
            if let EmbeddedObject::Ink(c) = obj {
                ink(self.page, c.ink(), (self.x / PX_PER_INK_UNIT, self.y / PX_PER_INK_UNIT));
            }
        }
        let text = r.text().trim_end();
        if text.is_empty() {
            return;
        }
        let style = r.text_run_formatting().first().unwrap_or(r.paragraph_style());
        let size = style.font_size().map(|hp| hp as f32 / 2.0 * 96.0 / 72.0).unwrap_or(DEFAULT_TEXT_PX);
        let color = match style.font_color() {
            Some(ColorRef::Manual { r, g, b }) => Color::rgb(r, g, b),
            _ => Color::BLACK,
        };
        let indent = "    ".repeat(depth);
        for line in text.split(['\n', '\r', '\u{b}']) {
            self.pending.push((format!("{indent}{line}"), size, color));
        }
    }
}

fn cell_text(elements: &[OutlineElement]) -> String {
    let texts = elements.iter().flat_map(|e| e.contents()).filter_map(|c| c.rich_text()).map(|r| r.text().trim().to_string());
    texts.filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

fn outline(page: &mut Page, o: &Outline) {
    let mut flow = Flow {
        x: o.offset_horizontal().unwrap_or(0.0) * PX_PER_HALF_INCH,
        y: o.offset_vertical().unwrap_or(0.0) * PX_PER_HALF_INCH,
        width: o.layout_max_width().map(|w| w * PX_PER_HALF_INCH).unwrap_or(400.0).max(80.0),
        page,
        pending: Vec::new(),
    };
    flow.items(o.items(), 0);
    flow.flush();
}

// ---- pictures ----

/// Adds a picture at its own offset, or at `at` inside an outline. Returns its size.
fn image(page: &mut Page, src: &Image, at: Option<(f32, f32)>) -> (f32, f32) {
    let Some(mut reader) = src.read() else { return (0.0, 0.0) };
    let mut bytes = Vec::new();
    if reader.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
        return (0.0, 0.0);
    }
    let mime = match src.extension().map(|e| e.trim_start_matches('.').to_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "image/png",
    };
    let half_inches = |a: Option<f32>, b: Option<f32>| a.or(b).filter(|v| *v > 0.0).map(|v| v * PX_PER_HALF_INCH);
    let width = half_inches(src.picture_width(), src.layout_max_width()).unwrap_or(320.0);
    let height = half_inches(src.picture_height(), src.layout_max_height()).unwrap_or(width * 0.75);
    let (x, y) = at.unwrap_or((
        src.offset_horizontal().unwrap_or(0.0) * PX_PER_HALF_INCH,
        src.offset_vertical().unwrap_or(0.0) * PX_PER_HALF_INCH,
    ));
    let src = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    page.images.push(ImageItem { id: new_id(), x, y, width, height, src });
    (width, height)
}

// ---- placement ----

/// Right-to-left OneNote pages put content at negative X: shifts everything
/// onto the page, then grows the page (at least A4) to fit it.
fn place_on_page(page: &mut Page) {
    let points = page.strokes.iter().flat_map(|s| &s.points).map(|p| (p.x, p.y));
    let corners = page.texts.iter().map(|t| (t.x, t.y)).chain(page.images.iter().map(|i| (i.x, i.y)));
    let (min_x, min_y) = points.chain(corners).fold((0.0f32, 0.0f32), |(x, y), (px, py)| (x.min(px), y.min(py)));
    let dx = if min_x < 0.0 { MARGIN - min_x } else { 0.0 };
    let dy = if min_y < 0.0 { MARGIN - min_y } else { 0.0 };
    for p in page.strokes.iter_mut().flat_map(|s| s.points.iter_mut()) {
        p.x += dx;
        p.y += dy;
    }
    for t in &mut page.texts {
        t.x += dx;
        t.y += dy;
    }
    for i in &mut page.images {
        i.x += dx;
        i.y += dy;
    }
    let mut max = (0.0f32, 0.0f32);
    for s in &page.strokes {
        let b = s.bounds();
        max = (max.0.max(b.max_x), max.1.max(b.max_y));
    }
    for b in page.texts.iter().map(TextBox::bounds).chain(page.images.iter().map(ImageItem::bounds)) {
        max = (max.0.max(b.max_x), max.1.max(b.max_y));
    }
    page.width = page.width.max((max.0 + MARGIN).ceil());
    page.height = page.height.max((max.1 + MARGIN).ceil());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_left_of_the_page_is_moved_onto_it() {
        let mut page = Page::new("rtl");
        let mut s = Stroke::new(Pen::default());
        s.points = vec![Point::new(-1000.0, 50.0, 1.0), Point::new(-10.0, 60.0, 1.0)];
        page.strokes.push(s);
        place_on_page(&mut page);
        let p = &page.strokes[0].points;
        assert_eq!((p[0].x, p[0].y, p[1].x), (40.0, 50.0, 1030.0));
        assert!(page.width >= 1030.0 + MARGIN);
        // Content already on the page stays put.
        place_on_page(&mut page);
        assert_eq!(page.strokes[0].points[0].x, 40.0);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(matches!(read_file(Path::new("notes.txt")), Err(Error::NotOneNote)));
    }
}
