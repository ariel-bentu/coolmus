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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OneNoteSectionLink {
    pub notebook_id: String,
    pub section_id: String,
}

/// A single sheet of paper with ink on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: Id,
    pub title: String,
    pub tags: Vec<String>,
    pub paper: Paper,
    pub width: f32,
    pub height: f32,
    pub strokes: Vec<Stroke>,
    pub created: u64,
    pub modified: u64,
    #[serde(default)]
    pub onenote: Option<OneNotePageLink>,
}

/// A4 at 96 DPI.
pub const DEFAULT_PAGE_SIZE: (f32, f32) = (794.0, 1123.0);

impl Page {
    pub fn new(title: impl Into<String>) -> Self {
        let now = now_millis();
        Self {
            id: new_id(),
            title: title.into(),
            tags: Vec::new(),
            paper: Paper::default(),
            width: DEFAULT_PAGE_SIZE.0,
            height: DEFAULT_PAGE_SIZE.1,
            strokes: Vec::new(),
            created: now,
            modified: now,
            onenote: None,
        }
    }

    pub fn touch(&mut self) {
        self.modified = now_millis();
    }

    pub fn meta(&self) -> PageMeta {
        PageMeta {
            id: self.id,
            title: self.title.clone(),
            tags: self.tags.clone(),
            created: self.created,
            modified: self.modified,
            onenote_page: self.onenote.as_ref().map(|l| l.page_id.clone()),
        }
    }

    /// Bounds of all ink (empty rect if none).
    pub fn ink_bounds(&self) -> Rect {
        self.strokes.iter().fold(Rect::EMPTY, |acc, s| acc.union(&s.bounds()))
    }

    /// Grows the page so all ink fits, keeping a margin.
    pub fn fit_to_ink(&mut self) {
        let b = self.ink_bounds();
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

/// Lightweight page summary kept in the library index (used for listing and search).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    pub id: Id,
    pub title: String,
    pub tags: Vec<String>,
    pub created: u64,
    pub modified: u64,
    /// OneNote page this page is linked to, if any.
    #[serde(default)]
    pub onenote_page: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: Id,
    pub name: String,
    pub pages: Vec<PageMeta>,
    #[serde(default)]
    pub onenote: Option<OneNoteSectionLink>,
}

impl Section {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: new_id(), name: name.into(), pages: Vec::new(), onenote: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notebook {
    pub id: Id,
    pub name: String,
    pub sections: Vec<Section>,
}

impl Notebook {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: new_id(), name: name.into(), sections: Vec::new() }
    }
}

/// The whole organizational tree: notebooks → sections → pages.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub notebooks: Vec<Notebook>,
}

/// Address of a page inside the library tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageLocation {
    pub notebook: Id,
    pub section: Id,
    pub page: Id,
}

impl Library {
    pub fn notebook_mut(&mut self, id: Id) -> Option<&mut Notebook> {
        self.notebooks.iter_mut().find(|n| n.id == id)
    }

    pub fn section(&self, id: Id) -> Option<&Section> {
        self.notebooks.iter().flat_map(|n| &n.sections).find(|s| s.id == id)
    }

    pub fn section_mut(&mut self, id: Id) -> Option<&mut Section> {
        self.notebooks.iter_mut().flat_map(|n| &mut n.sections).find(|s| s.id == id)
    }

    pub fn locate_page(&self, page: Id) -> Option<PageLocation> {
        self.notebooks.iter().find_map(|n| {
            n.sections.iter().find_map(|s| {
                s.pages.iter().any(|p| p.id == page).then_some(PageLocation {
                    notebook: n.id,
                    section: s.id,
                    page,
                })
            })
        })
    }

    /// Inserts or refreshes the index entry for `page` inside `section`.
    pub fn upsert_page_meta(&mut self, section: Id, meta: PageMeta) -> bool {
        let Some(sec) = self.section_mut(section) else { return false };
        match sec.pages.iter_mut().find(|p| p.id == meta.id) {
            Some(existing) => *existing = meta,
            None => sec.pages.push(meta),
        }
        true
    }

    /// Removes a page from the index; returns true if found.
    pub fn remove_page(&mut self, page: Id) -> bool {
        for sec in self.notebooks.iter_mut().flat_map(|n| &mut n.sections) {
            if let Some(i) = sec.pages.iter().position(|p| p.id == page) {
                sec.pages.remove(i);
                return true;
            }
        }
        false
    }

    /// Finds the local page linked to a OneNote page.
    pub fn find_by_onenote_page(&self, onenote_page: &str) -> Option<PageLocation> {
        let page = self
            .notebooks
            .iter()
            .flat_map(|n| &n.sections)
            .flat_map(|s| &s.pages)
            .find(|p| p.onenote_page.as_deref() == Some(onenote_page))?;
        self.locate_page(page.id)
    }

    /// Finds the local section linked to a OneNote section.
    pub fn find_by_onenote_section(&self, onenote_section: &str) -> Option<(Id, Id)> {
        self.notebooks.iter().find_map(|n| {
            n.sections
                .iter()
                .find(|s| s.onenote.as_ref().is_some_and(|l| l.section_id == onenote_section))
                .map(|s| (n.id, s.id))
        })
    }

    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .notebooks
            .iter()
            .flat_map(|n| &n.sections)
            .flat_map(|s| &s.pages)
            .flat_map(|p| p.tags.iter().cloned())
            .collect();
        tags.sort_by_key(|t| t.to_lowercase());
        tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        tags
    }
}

#[cfg(test)]
mod tests {
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
    fn library_locate_and_remove() {
        let mut lib = Library::default();
        let mut nb = Notebook::new("NB");
        let mut sec = Section::new("S");
        let page = Page::new("P");
        sec.pages.push(page.meta());
        let sec_id = sec.id;
        nb.sections.push(sec);
        lib.notebooks.push(nb);
        assert_eq!(lib.locate_page(page.id).unwrap().section, sec_id);
        assert!(lib.remove_page(page.id));
        assert!(lib.locate_page(page.id).is_none());
    }
}
