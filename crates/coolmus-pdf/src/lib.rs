//! Coolmus pages as ordinary PDF files.
//!
//! [`write`] draws the page as vector PDF (ink outlines, real text, pictures,
//! paper) and attaches the page's own JSON. Any PDF viewer shows the page;
//! Coolmus reads the attachment back with [`read`] and edits it losslessly.
//!
//! A page drawn on a PDF from elsewhere keeps that PDF as it is and adds its
//! own layer on top; see [`background`].
//!
//! The page dictionary carries a fingerprint of the content Coolmus drew. If
//! another program changed what is visible (redrew the page or added
//! annotations), [`read`] reports it so the caller can keep those changes
//! instead of silently overwriting them.

mod background;

use std::ops::Range;

use base64::Engine;
use krilla::color::rgb;
use krilla::embed::{AssociationKind, EmbeddedFile, MimeType};
use krilla::geom::{Path, PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke as PdfStroke};
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};
use krilla::Document;
use coolmus_core::{Color, ImageItem, Page, Rect, TextBox};
use coolmus_render::paper::{self, Mark, PAPER_COLOR};
use coolmus_render::tessellate::outlines;

pub use background::{background, strip, write_over};

/// Name of the attached page data inside the PDF.
pub const PAGE_FILE: &str = "coolmus-page.json";
/// Page-dictionary key holding the fingerprint of the content Coolmus drew.
const FINGERPRINT_KEY: &[u8] = b"CoolmusContent";
/// Page-dictionary key holding how many annotations the PDF had when Coolmus
/// wrote it (a PDF from elsewhere may come with some).
const ANNOTATIONS_KEY: &[u8] = b"CoolmusAnnotations";
/// Decompression cap for content from PDFs other programs wrote.
const MAX_DECODED: usize = 512 * 1024 * 1024;
/// Page units are pixels at 96 DPI; PDF units are points (72 per inch).
const PT_PER_PX: f32 = 0.75;
/// Matches `.textBox { line-height: 1.3 }` in the UI.
const LINE_HEIGHT: f32 = 1.3;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the font can't be used")]
    Font,
    #[error("PDF could not be written: {0}")]
    Write(String),
    #[error("not a readable PDF: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("the attached page data is invalid: {0}")]
    PageData(#[from] serde_json::Error),
    #[error("the PDF is encrypted")]
    Encrypted,
}

pub type Result<T> = std::result::Result<T, Error>;

/// What [`read`] found in a PDF.
#[derive(Debug)]
pub enum Opened {
    /// Written by Coolmus and unchanged since.
    Intact(Page),
    /// Has Coolmus page data, but another program changed what is visible.
    Changed { page: Page, changes: Vec<Change> },
    /// Any other PDF.
    Foreign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The page's drawing no longer matches what Coolmus wrote.
    ContentRedrawn,
    /// Annotations (comments, highlights, ink) were added on top.
    Annotations(usize),
    /// The PDF has more than one page.
    PagesAdded(usize),
}

/// Writes `page` as a one-page PDF with its data attached. `font` is a
/// TrueType/OpenType font covering the page's text (Hebrew and Latin); it is
/// only read when the page has text.
pub fn write(page: &Page, font: &[u8]) -> Result<Vec<u8>> {
    let mut doc = new_document(&page.title);
    let mut fonts = None;
    draw_page(&mut doc, page, font, &mut fonts)?;
    doc.embed_file(EmbeddedFile {
        path: PAGE_FILE.into(),
        mime_type: MimeType::new("application/json"),
        description: Some("Coolmus page data".into()),
        association_kind: AssociationKind::Source,
        data: serde_json::to_vec(page)?.into(),
        modification_date: None,
        compress: Some(true),
        location: None,
    });
    let bytes = doc.finish().map_err(|e| Error::Write(format!("{e:?}")))?;
    stamp_fingerprint(&bytes)
}

/// One PDF of several pages for printing: drawn like [`write`], without the
/// page data.
pub fn print(pages: &[Page], title: &str, font: &[u8]) -> Result<Vec<u8>> {
    let mut doc = new_document(title);
    let mut fonts = None;
    for page in pages {
        draw_page(&mut doc, page, font, &mut fonts)?;
    }
    doc.finish().map_err(|e| Error::Write(format!("{e:?}")))
}

/// The attached page JSON, if this PDF carries one. Cheaper than [`read`]
/// when only a few fields are needed.
pub fn page_data(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    Ok(attached_page(&lopdf::Document::load_mem(bytes)?))
}

/// A font installed with the system that covers Hebrew and Latin text.
pub fn system_font() -> Option<Vec<u8>> {
    let windows = std::env::var_os("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts"));
    let candidates = windows
        .iter()
        .flat_map(|dir| [dir.join("segoeui.ttf"), dir.join("arial.ttf")])
        .chain(
            [
                "/System/Library/Fonts/Supplemental/Arial.ttf",
                "/Library/Fonts/Arial.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
            ]
            .map(std::path::PathBuf::from),
        );
    candidates.into_iter().find_map(|p| std::fs::read(p).ok())
}

fn new_document(title: &str) -> Document {
    let mut doc = Document::new();
    let mut metadata = Metadata::new().creator("Coolmus".into());
    if !title.is_empty() {
        metadata = metadata.title(title.to_string());
    }
    doc.set_metadata(metadata);
    doc
}

/// Adds `page` to `doc`. The font is parsed the first time a page has text.
fn draw_page<'f>(doc: &mut Document, page: &Page, font: &'f [u8], fonts: &mut Option<TextFont<'f>>) -> Result<()> {
    if !page.texts.is_empty() && fonts.is_none() {
        *fonts = Some(TextFont::new(font)?);
    }
    for i in 0..page.sheet_count() {
        draw_sheet(doc, page, page.sheet_rect(i), fonts.as_ref(), true)?;
    }
    Ok(())
}

/// One PDF page: `sheet` with the content that reaches onto it, cut at its
/// edges; on paper, or transparent for a layer over another PDF's page.
fn draw_sheet(doc: &mut Document, page: &Page, sheet: Rect, font: Option<&TextFont>, paper: bool) -> Result<()> {
    let size = PageSettings::from_wh(sheet.width() * PT_PER_PX, sheet.height() * PT_PER_PX)
        .ok_or_else(|| Error::Write("invalid page size".into()))?;
    let mut pdf_page = doc.start_page_with(size);
    let mut surface = pdf_page.surface();
    surface.push_transform(&Transform::from_scale(PT_PER_PX, PT_PER_PX));
    if paper {
        draw_paper(&mut surface, page);
    }
    surface.push_transform(&Transform::from_translate(-sheet.min_x, -sheet.min_y));
    let mut clip = PathBuilder::new();
    clip.push_rect(krilla::geom::Rect::from_ltrb(sheet.min_x, sheet.min_y, sheet.max_x, sheet.max_y).unwrap());
    surface.push_clip_path(&clip.finish().expect("sheet rect"), &FillRule::NonZero);
    for image in page.images.iter().filter(|im| im.bounds().intersects(&sheet)) {
        draw_image(&mut surface, image);
    }
    for stroke in coolmus_render::paint_order(&page.strokes).filter(|s| s.bounds().intersects(&sheet)) {
        let color = stroke.pen.effective_color();
        let Some(path) = polygons(&outlines(stroke)) else { continue };
        if color.a == 255 {
            fill(&mut surface, &path, color);
        } else {
            // Opaque inside a translucent group, so self-overlaps don't darken
            // (same as the raster renderer).
            surface.push_opacity(opacity(color.a));
            fill(&mut surface, &path, color.with_alpha(255));
            surface.pop();
        }
    }
    if let Some(font) = font {
        // A box's height depends on its wrapping, so its estimate gets slack.
        let reaches = |t: &&TextBox| t.y <= sheet.max_y && t.bounds().max_y + 20.0 * t.size >= sheet.min_y;
        for text in page.texts.iter().filter(reaches) {
            draw_text(&mut surface, font, text);
        }
    }
    surface.pop();
    surface.pop();
    surface.pop();
    surface.finish();
    pdf_page.finish();
    Ok(())
}

/// Reads a PDF: Coolmus's own page data if it carries some, and whether
/// another program has changed the page since.
pub fn read(bytes: &[u8]) -> Result<Opened> {
    let doc = lopdf::Document::load_mem(bytes)?;
    let Some(json) = attached_page(&doc) else { return Ok(Opened::Foreign) };
    let page: Page = serde_json::from_slice(&json)?;

    let pages = doc.get_pages();
    let mut changes = Vec::new();
    let sheets = page.sheet_count() as usize;
    if pages.len() > sheets {
        changes.push(Change::PagesAdded(pages.len() - sheets));
    }
    let mut had_annotations = 0;
    if let Some(&first) = pages.values().next() {
        let first = doc.get_dictionary(first)?;
        let stamped = first.get(FINGERPRINT_KEY).and_then(|o| o.as_str()).ok();
        if stamped != Some(fingerprint(&doc).as_bytes()) {
            changes.push(Change::ContentRedrawn);
        }
        had_annotations = first.get(ANNOTATIONS_KEY).and_then(|o| o.as_i64()).unwrap_or(0).max(0) as usize;
    }
    let added = annotation_count(&doc).saturating_sub(had_annotations);
    if added > 0 {
        changes.push(Change::Annotations(added));
    }
    Ok(if changes.is_empty() { Opened::Intact(page) } else { Opened::Changed { page, changes } })
}

// ---- fingerprint ----

/// FNV-1a over all pages' decoded content streams: stable across runs and
/// Rust versions, and blind to how a resaving program compresses streams.
fn fingerprint(doc: &lopdf::Document) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for &page_id in doc.get_pages().values() {
        let content = doc.get_page_content_with_limit(page_id, MAX_DECODED).unwrap_or_default();
        hash = content.iter().fold(hash, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    }
    format!("{hash:016x}")
}

fn annotation_count(doc: &lopdf::Document) -> usize {
    doc.get_pages()
        .values()
        .filter_map(|&id| doc.get_dictionary(id).ok()?.get(b"Annots").ok())
        .filter_map(|a| doc.dereference(a).ok()?.1.as_array().ok().map(Vec::len))
        .sum()
}

/// Records what the PDF looks like now, for [`read`] to compare against.
fn stamp(doc: &mut lopdf::Document) -> Result<()> {
    let page_id = *doc.get_pages().values().next().ok_or_else(|| Error::Write("no page".into()))?;
    let fp = fingerprint(doc);
    let annotations = annotation_count(doc) as i64;
    let first = doc.get_dictionary_mut(page_id)?;
    first.set(FINGERPRINT_KEY, lopdf::Object::string_literal(fp));
    first.set(ANNOTATIONS_KEY, annotations);
    Ok(())
}

fn stamp_fingerprint(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut doc = lopdf::Document::load_mem(bytes)?;
    stamp(&mut doc)?;
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::Write(e.to_string()))?;
    Ok(out)
}

// ---- attachment lookup ----

fn attached_page(doc: &lopdf::Document) -> Option<Vec<u8>> {
    let names = deref_dict(doc, doc.catalog().ok()?.get(b"Names").ok()?)?;
    let tree = deref_dict(doc, names.get(b"EmbeddedFiles").ok()?)?;
    find_in_name_tree(doc, tree, 0)
}

fn deref_dict<'a>(doc: &'a lopdf::Document, obj: &'a lopdf::Object) -> Option<&'a lopdf::Dictionary> {
    doc.dereference(obj).ok()?.1.as_dict().ok()
}

fn find_in_name_tree(doc: &lopdf::Document, node: &lopdf::Dictionary, depth: u32) -> Option<Vec<u8>> {
    if depth > 16 {
        return None;
    }
    if let Ok(names) = node.get(b"Names").and_then(|n| n.as_array()) {
        for pair in names.chunks(2) {
            let [name, spec] = pair else { continue };
            let is_ours = doc.dereference(name).ok().and_then(|(_, n)| n.as_str().ok()) == Some(PAGE_FILE.as_bytes());
            if is_ours {
                let spec = deref_dict(doc, spec)?;
                let ef = deref_dict(doc, spec.get(b"EF").ok()?)?;
                let stream = doc.dereference(ef.get(b"F").ok()?).ok()?.1.as_stream().ok()?;
                return stream.decompressed_content_with_limit(MAX_DECODED).ok().or_else(|| Some(stream.content.clone()));
            }
        }
    }
    let kids = node.get(b"Kids").and_then(|k| k.as_array()).ok()?;
    kids.iter().find_map(|kid| find_in_name_tree(doc, deref_dict(doc, kid)?, depth + 1))
}

// ---- drawing ----

fn pdf_color(c: Color) -> rgb::Color {
    rgb::Color::new(c.r, c.g, c.b)
}

fn opacity(a: u8) -> NormalizedF32 {
    NormalizedF32::new(a as f32 / 255.0).unwrap_or(NormalizedF32::ONE)
}

fn fill(surface: &mut Surface, path: &Path, color: Color) {
    surface.set_fill(Some(Fill { paint: pdf_color(color).into(), opacity: opacity(color.a), rule: FillRule::NonZero }));
    surface.set_stroke(None);
    surface.draw_path(path);
}

fn polygons(polys: &[Vec<[f32; 2]>]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for poly in polys {
        let mut it = poly.iter();
        let Some(p) = it.next() else { continue };
        pb.move_to(p[0], p[1]);
        for p in it {
            pb.line_to(p[0], p[1]);
        }
        pb.close();
    }
    pb.finish()
}

fn circle(pb: &mut PathBuilder, cx: f32, cy: f32, r: f32) {
    // Four cubic quarter-arcs.
    const K: f32 = 0.552_284_8;
    let k = r * K;
    pb.move_to(cx + r, cy);
    pb.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    pb.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    pb.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    pb.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    pb.close();
}

fn draw_paper(surface: &mut Surface, page: &Page) {
    let mut pb = PathBuilder::new();
    pb.push_rect(krilla::geom::Rect::from_xywh(0.0, 0.0, page.width, page.height).unwrap());
    if let Some(path) = pb.finish() {
        fill(surface, &path, PAPER_COLOR);
    }
    for mark in paper::marks(page.paper, page.width, page.height) {
        let mut pb = PathBuilder::new();
        match mark {
            Mark::Line { from, to, width, color } => {
                pb.move_to(from[0], from[1]);
                pb.line_to(to[0], to[1]);
                let Some(path) = pb.finish() else { continue };
                surface.set_fill(None);
                surface.set_stroke(Some(PdfStroke { paint: pdf_color(color).into(), width, ..PdfStroke::default() }));
                surface.draw_path(&path);
            }
            Mark::Dot { center, radius, color } => {
                circle(&mut pb, center[0], center[1], radius);
                let Some(path) = pb.finish() else { continue };
                fill(surface, &path, color);
            }
        }
    }
    surface.set_stroke(None);
}

fn draw_image(surface: &mut Surface, item: &ImageItem) {
    let Some((mime, data)) = decode_data_url(&item.src) else { return };
    let data: krilla::Data = data.into();
    let image = match mime.as_str() {
        "image/png" => Image::from_png(data, true),
        "image/jpeg" | "image/jpg" => Image::from_jpeg(data, true),
        "image/gif" => Image::from_gif(data, true),
        "image/webp" => Image::from_webp(data, true),
        _ => return,
    };
    let (Ok(image), Some(size)) = (image, Size::from_wh(item.width, item.height)) else { return };
    surface.push_transform(&Transform::from_translate(item.x, item.y));
    surface.draw_image(image, size);
    surface.pop();
}

/// `data:<mime>;base64,<payload>` → (mime, bytes).
fn decode_data_url(src: &str) -> Option<(String, Vec<u8>)> {
    let rest = src.strip_prefix("data:")?;
    let (header, payload) = rest.split_once(',')?;
    let mime = header.strip_suffix(";base64")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(payload.trim()).ok()?;
    Some((mime.to_ascii_lowercase(), bytes))
}

// ---- text ----

/// The font twice: krilla embeds it, rustybuzz measures with it.
struct TextFont<'a> {
    pdf: Font,
    face: rustybuzz::Face<'a>,
}

impl<'a> TextFont<'a> {
    fn new(data: &'a [u8]) -> Result<Self> {
        Ok(Self {
            pdf: Font::new(data.to_vec().into(), 0).ok_or(Error::Font)?,
            face: rustybuzz::Face::from_slice(data, 0).ok_or(Error::Font)?,
        })
    }

    /// Advance width of `text` at `size`.
    fn width(&self, text: &str, size: f32) -> f32 {
        let mut buf = rustybuzz::UnicodeBuffer::new();
        buf.push_str(text);
        buf.guess_segment_properties();
        let glyphs = rustybuzz::shape(&self.face, &[], buf);
        let units: i32 = glyphs.glyph_positions().iter().map(|p| p.x_advance).sum();
        units as f32 * size / self.face.units_per_em() as f32
    }

    /// Distance from a line box's top to its baseline, as CSS places it.
    fn baseline(&self, size: f32) -> f32 {
        let upem = self.face.units_per_em() as f32;
        let ascent = self.face.ascender() as f32 * size / upem;
        let descent = -(self.face.descender() as f32) * size / upem;
        (LINE_HEIGHT * size - (ascent + descent)) / 2.0 + ascent
    }
}

/// Draws a text box the way the UI lays it out: `white-space: pre-wrap`,
/// `overflow-wrap: anywhere`, `unicode-bidi: plaintext`, `text-align: start`.
fn draw_text(surface: &mut Surface, font: &TextFont, tb: &TextBox) {
    surface.set_fill(Some(Fill { paint: pdf_color(tb.color).into(), opacity: opacity(tb.color.a), rule: FillRule::NonZero }));
    surface.set_stroke(None);
    let line_height = LINE_HEIGHT * tb.size;
    let mut top = tb.y;
    for paragraph in tb.text.split('\n') {
        let bidi = unicode_bidi::BidiInfo::new(paragraph, None);
        let rtl = bidi.paragraphs.first().is_some_and(|p| p.level.is_rtl());
        for line in wrap(font, paragraph, tb.size, tb.width) {
            let visible = trim_end(paragraph, line.clone());
            let line_width = font.width(&paragraph[visible.clone()], tb.size);
            let mut x = if rtl { tb.x + tb.width - line_width } else { tb.x };
            let baseline = top + font.baseline(tb.size);
            if let (Some(para), false) = (bidi.paragraphs.first(), visible.is_empty()) {
                let (levels, runs) = bidi.visual_runs(para, visible);
                for run in runs {
                    let dir = if levels[run.start].is_rtl() { TextDirection::RightToLeft } else { TextDirection::LeftToRight };
                    let text = &paragraph[run];
                    surface.draw_text(Point::from_xy(x, baseline), font.pdf.clone(), tb.size, text, false, dir);
                    x += font.width(text, tb.size);
                }
            }
            top += line_height;
        }
    }
}

fn trim_end(text: &str, r: Range<usize>) -> Range<usize> {
    r.start..r.start + text[r].trim_end().len()
}

/// Greedy line breaking at spaces; a word wider than the box breaks anywhere.
/// Returns byte ranges into `text`; an empty paragraph is one empty line.
fn wrap(font: &TextFont, text: &str, size: f32, max: f32) -> Vec<Range<usize>> {
    // Words keep their trailing spaces, which may hang past the edge.
    let mut words = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            in_space = true;
        } else if in_space {
            words.push(start..i);
            start = i;
            in_space = false;
        }
    }
    words.push(start..text.len());

    let fits = |r: Range<usize>| font.width(text[trim_end(text, r)].as_ref(), size) <= max + 0.01;
    let mut lines = Vec::new();
    let mut line: Option<Range<usize>> = None;
    for word in words {
        if let Some(l) = &line {
            if fits(l.start..word.end) {
                line = Some(l.start..word.end);
                continue;
            }
            lines.push(l.clone());
        }
        // Start a new line with this word, breaking it if it alone is too wide.
        let mut s = word.start;
        while !fits(s..word.end) {
            let mut end = s;
            for (i, c) in text[s..word.end].char_indices() {
                let next = s + i + c.len_utf8();
                if end > s && !fits(s..next) {
                    break;
                }
                end = next;
            }
            lines.push(s..end);
            s = end;
        }
        line = Some(s..word.end);
    }
    lines.extend(line);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url() {
        let (mime, bytes) = decode_data_url("data:image/PNG;base64,aGk=").unwrap();
        assert_eq!((mime.as_str(), bytes.as_slice()), ("image/png", b"hi".as_slice()));
        assert!(decode_data_url("https://x/y.png").is_none());
    }

    fn ink_page() -> Page {
        let mut page = Page::new("t");
        let mut s = coolmus_core::Stroke::new(coolmus_core::Pen::default());
        s.points = vec![coolmus_core::Point::new(10.0, 10.0, 0.2), coolmus_core::Point::new(200.0, 90.0, 0.9)];
        page.strokes.push(s);
        let mut hl = coolmus_core::Stroke::new(coolmus_core::Pen { style: coolmus_core::PenStyle::Highlighter, ..Default::default() });
        hl.points = vec![coolmus_core::Point::new(10.0, 50.0, 1.0), coolmus_core::Point::new(200.0, 50.0, 1.0)];
        page.strokes.push(hl);
        page
    }

    #[test]
    fn page_survives_the_round_trip() {
        let page = ink_page();
        let pdf = write(&page, &[]).unwrap();
        let Opened::Intact(back) = read(&pdf).unwrap() else { panic!("not intact") };
        assert_eq!(back, Page { title: String::new(), ..page });
    }

    #[test]
    fn edits_by_other_programs_are_reported() {
        let pdf = write(&ink_page(), &[]).unwrap();
        let mut doc = lopdf::Document::load_mem(&pdf).unwrap();
        let page_id = *doc.get_pages().values().next().unwrap();
        let note = doc.add_object(lopdf::dictionary! { "Type" => "Annot", "Subtype" => "Text" });
        doc.get_dictionary_mut(page_id).unwrap().set("Annots", vec![note.into()]);
        let content = doc.get_page_contents(page_id)[0];
        doc.get_object_mut(content).unwrap().as_stream_mut().unwrap().set_plain_content(b"0 0 m 10 10 l S".to_vec());
        let mut edited = Vec::new();
        doc.save_to(&mut edited).unwrap();
        let Opened::Changed { changes, .. } = read(&edited).unwrap() else { panic!("change not seen") };
        assert_eq!(changes, vec![Change::ContentRedrawn, Change::Annotations(1)]);
    }

    #[test]
    fn every_sheet_is_a_pdf_page() {
        let mut page = ink_page();
        page.sheets = 3;
        let pdf = write(&page, &[]).unwrap();
        assert_eq!(lopdf::Document::load_mem(&pdf).unwrap().get_pages().len(), 3);
        let Opened::Intact(back) = read(&pdf).unwrap() else { panic!("not intact") };
        assert_eq!(back.sheets, 3);
    }

    #[test]
    fn print_has_one_pdf_page_per_page_and_no_data() {
        let pdf = print(&[ink_page(), ink_page(), ink_page()], "t", &[]).unwrap();
        let doc = lopdf::Document::load_mem(&pdf).unwrap();
        assert_eq!(doc.get_pages().len(), 3);
        assert!(page_data(&pdf).unwrap().is_none());
    }

    #[test]
    fn foreign_pdf_has_no_page() {
        let mut doc = Document::new();
        doc.start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap()).finish();
        assert!(matches!(read(&doc.finish().unwrap()).unwrap(), Opened::Foreign));
    }
}
