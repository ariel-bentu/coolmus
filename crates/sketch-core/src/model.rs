use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::geometry::{dist_sq_point_segment, Point, Rect};

pub type Id = Uuid;

pub fn new_id() -> Id {
    Uuid::new_v4()
}

/// Milliseconds since the Unix epoch.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0x1a, 0x1a, 0x1a);
    pub const WHITE: Color = Color::rgb(0xff, 0xff, 0xff);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// `#rrggbb` (alpha is dropped).
    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
        match s.len() {
            3 => {
                let n = |i: usize| u8::from_str_radix(s.get(i..i + 1)?, 16).ok().map(|v| v * 17);
                Some(Color::rgb(n(0)?, n(1)?, n(2)?))
            }
            6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?).with_alpha(byte(6)?)),
            _ => None,
        }
    }
}

/// Visual style of a stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PenStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    /// Wide, translucent, flat; drawn underneath ink.
    Highlighter,
    /// Thin, slightly translucent, very pressure-sensitive.
    Pencil,
}

impl PenStyle {
    pub const ALL: [PenStyle; 5] =
        [PenStyle::Solid, PenStyle::Dashed, PenStyle::Dotted, PenStyle::Highlighter, PenStyle::Pencil];

    pub fn label(&self) -> &'static str {
        match self {
            PenStyle::Solid => "Solid",
            PenStyle::Dashed => "Dashed",
            PenStyle::Dotted => "Dotted",
            PenStyle::Highlighter => "Highlighter",
            PenStyle::Pencil => "Pencil",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pen {
    pub color: Color,
    /// Nominal width in page units at full pressure.
    pub width: f32,
    pub style: PenStyle,
    /// Whether pressure modulates the width.
    pub pressure_sensitive: bool,
}

impl Default for Pen {
    fn default() -> Self {
        Self { color: Color::BLACK, width: 3.0, style: PenStyle::Solid, pressure_sensitive: true }
    }
}

impl Pen {
    /// Width at a given pressure.
    pub fn width_at(&self, pressure: f32) -> f32 {
        if !self.pressure_sensitive || self.style == PenStyle::Highlighter {
            return self.width;
        }
        let p = pressure.clamp(0.0, 1.0);
        let min = if self.style == PenStyle::Pencil { 0.15 } else { 0.35 };
        self.width * (min + (1.0 - min) * p)
    }

    /// Color actually painted (style may add translucency).
    pub fn effective_color(&self) -> Color {
        match self.style {
            PenStyle::Highlighter => self.color.with_alpha(self.color.a.min(90)),
            PenStyle::Pencil => self.color.with_alpha(self.color.a.min(200)),
            _ => self.color,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: Id,
    pub pen: Pen,
    pub points: Vec<Point>,
}

impl Stroke {
    pub fn new(pen: Pen) -> Self {
        Self { id: new_id(), pen, points: Vec::new() }
    }

    pub fn bounds(&self) -> Rect {
        let mut r = Rect::EMPTY;
        for p in &self.points {
            r.include(p.x, p.y);
        }
        r.expand(self.pen.width * 0.5 + 1.0)
    }

    /// True if the stroke passes within `radius` of (x, y).
    pub fn hit(&self, x: f32, y: f32, radius: f32) -> bool {
        let reach = radius + self.pen.width * 0.5;
        let reach_sq = reach * reach;
        match self.points.as_slice() {
            [] => false,
            [p] => (p.x - x).powi(2) + (p.y - y).powi(2) <= reach_sq,
            pts => pts.windows(2).any(|w| dist_sq_point_segment(x, y, &w[0], &w[1]) <= reach_sq),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Paper {
    #[default]
    Blank,
    Lined,
    Grid,
    Dotted,
}

impl Paper {
    pub const ALL: [Paper; 4] = [Paper::Blank, Paper::Lined, Paper::Grid, Paper::Dotted];

    pub fn label(&self) -> &'static str {
        match self {
            Paper::Blank => "Blank",
            Paper::Lined => "Lined",
            Paper::Grid => "Grid",
            Paper::Dotted => "Dotted",
        }
    }
}

/// Where a page lives in OneNote, if it was imported from or pushed to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OneNotePageLink {
    pub page_id: String,
    pub section_id: String,
    /// `lastModifiedDateTime` reported by OneNote at the last sync.
    pub last_modified: Option<String>,
    /// True if the page was written by Sketcher (has our image + data attachment),
    /// so it can be updated in place instead of re-created.
    pub owned_by_sketcher: bool,
}

/// Identifies a page file and its layout version.
pub const PAGE_FORMAT: &str = "sketcher/page";
pub const PAGE_VERSION: u32 = 2;

fn page_format() -> String {
    PAGE_FORMAT.to_string()
}

fn page_version() -> u32 {
    PAGE_VERSION
}

/// A text box placed anywhere on the page. `(x, y)` is its top-left corner;
/// text wraps at `width`. The text's own direction (Hebrew or English) decides
/// its alignment inside the box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBox {
    pub id: Id,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub text: String,
    /// Font size in page units.
    pub size: f32,
    pub color: Color,
}

impl TextBox {
    /// Approximate bounds (the exact height depends on the font's line wrapping).
    pub fn bounds(&self) -> Rect {
        let lines = self.text.lines().count().max(1) as f32;
        Rect { min_x: self.x, min_y: self.y, max_x: self.x + self.width, max_y: self.y + lines * self.size * 1.3 }
    }
}

/// A picture placed on the page, stored inside the page file as a data URL
/// (`data:image/jpeg;base64,…`) so a page is always one self-contained file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageItem {
    pub id: Id,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub src: String,
}

impl ImageItem {
    pub fn bounds(&self) -> Rect {
        Rect { min_x: self.x, min_y: self.y, max_x: self.x + self.width, max_y: self.y + self.height }
    }
}

/// A single sheet of paper: vector ink, text boxes and pictures.
///
/// Stored as one JSON file. The page's name is its file name, so `title` is
/// filled in when the file is loaded and never written into the file.
/// Drawing order: pictures, then highlighter ink, then ink, then text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    #[serde(default = "page_format")]
    pub format: String,
    #[serde(default = "page_version")]
    pub version: u32,
    pub id: Id,
    #[serde(skip)]
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub paper: Paper,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
    #[serde(default)]
    pub texts: Vec<TextBox>,
    #[serde(default)]
    pub images: Vec<ImageItem>,
    pub created: u64,
    pub modified: u64,
    /// The OneNote page this page was imported from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onenote: Option<OneNotePageLink>,
}

/// A4 at 96 DPI.
pub const DEFAULT_PAGE_SIZE: (f32, f32) = (794.0, 1123.0);

impl Page {
    pub fn new(title: impl Into<String>) -> Self {
        let now = now_millis();
        Self {
            format: page_format(),
            version: PAGE_VERSION,
            id: new_id(),
            title: title.into(),
            tags: Vec::new(),
            paper: Paper::default(),
            width: DEFAULT_PAGE_SIZE.0,
            height: DEFAULT_PAGE_SIZE.1,
            strokes: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            created: now,
            modified: now,
            onenote: None,
        }
    }

    pub fn touch(&mut self) {
        self.modified = now_millis();
    }

    /// Bounds of all ink (empty rect if none).
    pub fn ink_bounds(&self) -> Rect {
        self.strokes.iter().fold(Rect::EMPTY, |acc, s| acc.union(&s.bounds()))
    }

    /// Bounds of everything on the page: ink, text and pictures.
    pub fn content_bounds(&self) -> Rect {
        let texts = self.texts.iter().map(TextBox::bounds);
        let images = self.images.iter().map(ImageItem::bounds);
        texts.chain(images).fold(self.ink_bounds(), |acc, r| acc.union(&r))
    }

    /// Moves content that lies above or left of the page (negative coordinates)
    /// onto it, keeping its layout, then grows the page to fit. OneNote stores
    /// right-to-left pages' ink at negative X. Returns true if anything moved.
    pub fn move_ink_onto_page(&mut self) -> bool {
        const MARGIN: f32 = 40.0;
        let points = self.strokes.iter().flat_map(|s| &s.points).map(|p| (p.x, p.y));
        let corners = self.texts.iter().map(|t| (t.x, t.y)).chain(self.images.iter().map(|i| (i.x, i.y)));
        let (min_x, min_y) = points.chain(corners).fold((0.0f32, 0.0f32), |(x, y), (px, py)| (x.min(px), y.min(py)));
        let dx = if min_x < 0.0 { MARGIN - min_x } else { 0.0 };
        let dy = if min_y < 0.0 { MARGIN - min_y } else { 0.0 };
        if dx == 0.0 && dy == 0.0 {
            return false;
        }
        for s in &mut self.strokes {
            for p in &mut s.points {
                p.x += dx;
                p.y += dy;
            }
        }
        for t in &mut self.texts {
            t.x += dx;
            t.y += dy;
        }
        for i in &mut self.images {
            i.x += dx;
            i.y += dy;
        }
        self.fit_to_ink();
        true
    }

    /// Grows the page so all content fits, keeping a margin.
    pub fn fit_to_ink(&mut self) {
        let b = self.content_bounds();
        if !b.is_empty() {
            self.width = self.width.max(b.max_x + 40.0);
            self.height = self.height.max(b.max_y + 40.0);
        }
    }

    /// Indices of strokes hit at (x, y).
    pub fn strokes_hit(&self, x: f32, y: f32, radius: f32) -> Vec<usize> {
        self.strokes
            .iter()
            .enumerate()
            .filter(|(_, s)| s.hit(x, y, radius))
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ink_left_of_the_page_is_moved_onto_it() {
        let mut page = Page::new("rtl");
        let mut s = Stroke::new(Pen::default());
        s.points = vec![Point::new(-1000.0, 50.0, 0.5), Point::new(-10.0, 60.0, 0.5)];
        page.strokes.push(s);
        assert!(page.move_ink_onto_page());
        let p = &page.strokes[0].points;
        assert_eq!((p[0].x, p[0].y, p[1].x), (40.0, 50.0, 1030.0));
        assert!(page.width >= 1030.0);
        // Ink already on the page, even touching its edge, stays put.
        assert!(!page.move_ink_onto_page());
        page.strokes[0].points[0].x = 0.0;
        assert!(!page.move_ink_onto_page());
    }

    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Color::from_hex("#12abEF").unwrap();
        assert_eq!(c, Color::rgb(0x12, 0xab, 0xef));
        assert_eq!(c.to_hex(), "#12abef");
        assert_eq!(Color::from_hex("#fff"), Some(Color::rgb(255, 255, 255)));
        assert_eq!(Color::from_hex("zz"), None);
    }

    #[test]
    fn stroke_hit_test() {
        let mut s = Stroke::new(Pen { width: 2.0, ..Pen::default() });
        s.points = vec![Point::new(0.0, 0.0, 1.0), Point::new(100.0, 0.0, 1.0)];
        assert!(s.hit(50.0, 3.0, 3.0));
        assert!(!s.hit(50.0, 10.0, 3.0));
    }

    #[test]
    fn page_file_roundtrip_keeps_items_and_drops_title() {
        let mut page = Page::new("Plan");
        page.texts.push(TextBox { id: new_id(), x: 10.0, y: 20.0, width: 200.0, text: "שלום hello".into(), size: 18.0, color: Color::BLACK });
        page.images.push(ImageItem { id: new_id(), x: 0.0, y: 0.0, width: 50.0, height: 40.0, src: "data:image/png;base64,AAAA".into() });
        let json = serde_json::to_string(&page).unwrap();
        assert!(json.starts_with(r#"{"format":"sketcher/page","version":2,"#), "{json}");
        assert!(!json.contains("Plan"));
        let back: Page = serde_json::from_str(&json).unwrap();
        assert_eq!(back.texts, page.texts);
        assert_eq!(back.images, page.images);
        assert_eq!(back.title, "");
    }

    #[test]
    fn version_1_pages_still_load() {
        let old = r#"{"id":"6ca13678-8528-4b35-b9d7-e84c845ebb50","title":"Old","tags":["a"],"paper":"Lined","width":794.0,"height":1123.0,"strokes":[],"created":1,"modified":2,"onenote":null}"#;
        let page: Page = serde_json::from_str(old).unwrap();
        assert_eq!((page.paper, page.tags.len(), page.texts.len()), (Paper::Lined, 1, 0));
    }
}
