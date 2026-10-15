//! Sketcher page ⇄ OneNote page.
//!
//! A page written by Sketcher contains, inside one `<div data-id="sketcher">`:
//! * a tags paragraph (`data-id="sketcher-tags"`) — readable and searchable in OneNote,
//! * either native OneNote ink (InkML) or a rendered PNG (`data-id="sketcher-image"`),
//! * a JSON attachment (`*.sketcher.json`) holding the lossless Sketcher page
//!   (pen styles, paper, pressure), so round-trips lose nothing.
//!
//! Pages *not* written by Sketcher are imported from their native ink.

use serde::{Deserialize, Serialize};
use serde_json::json;
use sketch_core::{tags, OneNotePageLink, Page, Stroke};
use sketch_render::raster::{render_png, RasterOptions};

use crate::client::{OneNoteClient, RemotePage};
use crate::html::{self, escape};
use crate::inkml::{self, Fingerprint};
use crate::multipart::Part;
use crate::{Error, Result};

pub const DATA_SUFFIX: &str = ".sketcher.json";
const FORMAT: &str = "sketcher/1";
/// Native ink is shifted down so it sits below OneNote's title and our tags/attachment block.
pub const INK_TOP_OFFSET: f32 = 200.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportMode {
    /// Real OneNote ink strokes — editable with OneNote's own pens.
    /// Updating re-creates the page (the API cannot modify existing ink).
    #[default]
    NativeInk,
    /// A rendered picture. Updates are patched in place.
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Payload {
    format: String,
    page: Page,
    /// Ink offset used when the page was written.
    ink_offset_y: f32,
    /// Signature of the ink as written, to detect edits made in OneNote.
    ink: Option<Fingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSource {
    /// Lossless Sketcher data attached to the page.
    SketcherData,
    /// OneNote's native ink.
    NativeInk,
    /// The page has no ink.
    Empty,
}

#[derive(Debug, Clone)]
pub struct ImportedPage {
    pub page: Page,
    pub source: ImportSource,
}

fn shifted(strokes: &[Stroke], dy: f32) -> Vec<Stroke> {
    strokes
        .iter()
        .map(|s| {
            let mut s = s.clone();
            for p in &mut s.points {
                p.y += dy;
            }
            s
        })
        .collect()
}

fn file_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    if s.is_empty() { "sketch".into() } else { s }
}

fn tags_paragraph(page: &Page) -> String {
    let tags = if page.tags.is_empty() {
        "(none)".to_string()
    } else {
        page.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")
    };
    format!("<p data-id=\"sketcher-tags\" style=\"color:#707070;font-size:9pt\">Tags: {}</p>", escape(&tags))
}

fn image_tag(page: &Page) -> String {
    format!(
        "<img data-id=\"sketcher-image\" src=\"name:sketchimg\" width=\"{}\" height=\"{}\" alt=\"{}\" />",
        page.width.round(),
        page.height.round(),
        escape(&format!("Sketch: {}", page.title))
    )
}

fn data_tag(page: &Page) -> String {
    format!(
        "<object data-id=\"sketcher-data\" data-attachment=\"{}{DATA_SUFFIX}\" data=\"name:sketchdata\" type=\"application/json\" />",
        escape(&file_stem(&page.title))
    )
}

fn payload_bytes(page: &Page, mode: ExportMode) -> (Vec<u8>, Option<String>) {
    let mut clean = page.clone();
    clean.onenote = None;
    let (inkml, ink) = match mode {
        ExportMode::NativeInk => {
            let xml = inkml::write(&shifted(&page.strokes, INK_TOP_OFFSET));
            // Fingerprint what OneNote will parse back, not what we hold in memory.
            let fp = inkml::parse(&xml).ok().map(|s| inkml::fingerprint(&s));
            (Some(xml), fp)
        }
        ExportMode::Image => (None, None),
    };
    let payload = Payload { format: FORMAT.into(), page: clean, ink_offset_y: INK_TOP_OFFSET, ink };
    (serde_json::to_vec(&payload).expect("payload serializes"), inkml)
}

fn render_image(page: &Page) -> Result<Vec<u8>> {
    render_png(page, RasterOptions { scale: 1.5, ..Default::default() }).map_err(|e| Error::Render(e.to_string()))
}

/// Writes `page` to OneNote `section_id`. Returns the new link to store on the page.
///
/// If the page is already linked to a page in the same section it is updated:
/// image-mode pages are patched in place; otherwise a fresh page is created and
/// the old one deleted (when `replace_old`) because the API cannot edit ink.
pub fn export_page(
    client: &mut OneNoteClient,
    section_id: &str,
    page: &Page,
    mode: ExportMode,
    replace_old: bool,
) -> Result<OneNotePageLink> {
    let existing = page.onenote.as_ref().filter(|l| l.section_id == section_id);

    if let (ExportMode::Image, Some(link)) = (mode, existing) {
        if link.owned_by_sketcher {
            match patch_in_place(client, link, page) {
                Ok(remote) => {
                    return Ok(OneNotePageLink {
                        last_modified: remote.last_modified,
                        ..link.clone()
                    })
                }
                Err(e) => log::warn!("in-place update failed, re-creating page: {e}"),
            }
        }
    }

    let (data, inkml) = payload_bytes(page, mode);
    let png = if mode == ExportMode::Image { Some(render_image(page)?) } else { None };
    let visual = if png.is_some() { image_tag(page) } else { String::new() };
    let html = format!(
        "<!DOCTYPE html>\n<html><head><title>{}</title><meta name=\"created\" content=\"{}\" /></head>\n<body>\n<div data-id=\"sketcher\">\n{}\n{}\n{}\n</div>\n</body></html>",
        escape(&page.title),
        iso8601(page.created),
        tags_paragraph(page),
        data_tag(page),
        visual,
    );

    let mut parts = vec![Part { name: "presentation", content_type: "text/html", body: html.as_bytes() }];
    if let Some(xml) = &inkml {
        parts.push(Part { name: "presentation-onenote-inkml", content_type: "application/inkml+xml", body: xml.as_bytes() });
    }
    parts.push(Part { name: "sketchdata", content_type: "application/json", body: &data });
    if let Some(png) = &png {
        parts.push(Part { name: "sketchimg", content_type: "image/png", body: png });
    }
    let created = client.create_page(section_id, &parts)?;

    if let (Some(old), true) = (existing, replace_old) {
        if old.page_id != created.id {
            if let Err(e) = client.delete_page(&old.page_id) {
                log::warn!("created the new page but could not delete the old one: {e}");
            }
        }
    }
    Ok(OneNotePageLink {
        page_id: created.id,
        section_id: section_id.to_string(),
        last_modified: created.last_modified,
        owned_by_sketcher: true,
    })
}

fn patch_in_place(client: &mut OneNoteClient, link: &OneNotePageLink, page: &Page) -> Result<RemotePage> {
    let content = client.get_page_content(&link.page_id)?;
    let has = |id: &str| html::find_tags(&content.html, "img").iter().chain(html::find_tags(&content.html, "object").iter()).any(|t| t.get("data-id") == Some(id));
    if !has("sketcher-image") || !has("sketcher-data") {
        return Err(Error::Parse("page no longer has Sketcher elements".into()));
    }
    let (data, _) = payload_bytes(page, ExportMode::Image);
    let png = render_image(page)?;
    let mut commands = vec![
        json!({ "target": "title", "action": "replace", "content": page.title }),
        json!({ "target": "#sketcher-image", "action": "replace", "content": image_tag(page) }),
        json!({ "target": "#sketcher-data", "action": "replace", "content": data_tag(page) }),
    ];
    // Paragraphs can only be replaced by their generated id.
    if let Some(id) = html::find_tags(&content.html, "p")
        .iter()
        .find(|t| t.get("data-id") == Some("sketcher-tags"))
        .and_then(|t| t.get("id"))
    {
        commands.push(json!({ "target": id, "action": "replace", "content": tags_paragraph(page) }));
    }
    client.patch_page(
        &link.page_id,
        &serde_json::Value::Array(commands),
        &[
            Part { name: "sketchimg", content_type: "image/png", body: &png },
            Part { name: "sketchdata", content_type: "application/json", body: &data },
        ],
    )?;
    client.get_page(&link.page_id)
}

/// Reads a OneNote page into a Sketcher page.
pub fn import_page(client: &mut OneNoteClient, section_id: &str, remote: &RemotePage) -> Result<ImportedPage> {
    let content = client.get_page_content(&remote.id)?;
    let payload = html::find_tags(&content.html, "object")
        .into_iter()
        .filter(|t| t.get("data-attachment").is_some_and(|a| a.ends_with(DATA_SUFFIX)))
        .filter_map(|t| t.get("data").map(str::to_string))
        .next()
        .and_then(|url| match client.get_resource(&url) {
            Ok(bytes) => serde_json::from_slice::<Payload>(&bytes).ok().filter(|p| p.format == FORMAT),
            Err(e) => {
                log::warn!("could not download Sketcher data: {e}");
                None
            }
        });
    let native = match &content.inkml {
        Some(xml) => inkml::parse(xml)?,
        None => Vec::new(),
    };
    let has_image = html::find_tags(&content.html, "img").iter().any(|t| t.get("data-id") == Some("sketcher-image"));
    let mut imported = interpret(&content.html, native, payload, has_image, remote);
    imported.page.onenote = Some(OneNotePageLink {
        page_id: remote.id.clone(),
        section_id: section_id.to_string(),
        last_modified: remote.last_modified.clone(),
        owned_by_sketcher: imported.source == ImportSource::SketcherData || has_image,
    });
    Ok(imported)
}

fn interpret(
    html_text: &str,
    native: Vec<Stroke>,
    payload: Option<Payload>,
    has_image: bool,
    remote: &RemotePage,
) -> ImportedPage {
    let title = html::title(html_text).filter(|t| !t.is_empty()).unwrap_or_else(|| remote.title.clone());
    // Tags paragraph text is authoritative: the user may have edited it in OneNote.
    let tags_text = html::find_tags(html_text, "p")
        .iter()
        .find(|t| t.get("data-id") == Some("sketcher-tags"))
        .map(|t| html::inner_text(html_text, t, "p"));

    let (mut page, source) = match payload {
        Some(p) => {
            let unchanged = if p.ink.is_none() {
                native.is_empty() && has_image
            } else {
                p.ink == Some(inkml::fingerprint(&native))
            };
            if unchanged {
                (p.page, ImportSource::SketcherData)
            } else {
                // Ink was edited in OneNote: native ink wins, keep our page settings.
                let mut page = p.page;
                page.strokes = shifted(&native, -p.ink_offset_y);
                (page, ImportSource::NativeInk)
            }
        }
        None => {
            let mut page = Page::new(title.clone());
            let empty = native.is_empty();
            page.strokes = native;
            (page, if empty { ImportSource::Empty } else { ImportSource::NativeInk })
        }
    };
    page.title = title;
    if let Some(text) = tags_text {
        let mut parsed = Vec::new();
        for word in text.split_whitespace().filter(|w| w.starts_with('#')) {
            tags::add_tag(&mut parsed, word);
        }
        page.tags = parsed;
    }
    page.fit_to_ink();
    page.touch();
    ImportedPage { page, source }
}

/// Unix millis → `YYYY-MM-DDTHH:MM:SSZ` (UTC), without a date library.
fn iso8601(millis: u64) -> String {
    let secs = millis / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sketch_core::{Pen, PenStyle, Point};

    fn sample_page() -> Page {
        let mut page = Page::new("Plan");
        page.tags = vec!["work".into(), "ideas".into()];
        let mut s = Stroke::new(Pen { style: PenStyle::Dashed, ..Pen::default() });
        s.points = vec![Point::new(10.0, 10.0, 1.0), Point::new(300.0, 40.0, 1.0)];
        page.strokes.push(s);
        page
    }

    fn remote() -> RemotePage {
        RemotePage { id: "p1".into(), title: "Plan".into(), last_modified: None }
    }

    const HTML: &str = r#"<html><head><title>Plan</title></head><body><div data-id="sketcher">
        <p data-id="sketcher-tags" id="p:{x}{1}">Tags: #work #ideas #new</p></div></body></html>"#;

    #[test]
    fn iso_dates() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_760_486_400_000), "2025-10-15T00:00:00Z");
    }

    #[test]
    fn unchanged_ink_uses_lossless_payload() {
        let page = sample_page();
        let (bytes, xml) = payload_bytes(&page, ExportMode::NativeInk);
        let payload: Payload = serde_json::from_slice(&bytes).unwrap();
        let native = inkml::parse(&xml.unwrap()).unwrap();
        let imported = interpret(HTML, native, Some(payload), false, &remote());
        assert_eq!(imported.source, ImportSource::SketcherData);
        assert_eq!(imported.page.strokes[0].pen.style, PenStyle::Dashed);
        assert_eq!(imported.page.tags, vec!["work", "ideas", "new"]);
    }

    #[test]
    fn edited_ink_wins_and_offset_is_removed() {
        let page = sample_page();
        let (bytes, xml) = payload_bytes(&page, ExportMode::NativeInk);
        let payload: Payload = serde_json::from_slice(&bytes).unwrap();
        let mut native = inkml::parse(&xml.unwrap()).unwrap();
        native.truncate(1); // someone erased ink in OneNote
        let imported = interpret(HTML, native, Some(payload), false, &remote());
        assert_eq!(imported.source, ImportSource::NativeInk);
        assert!((imported.page.strokes[0].points[0].y - 10.0).abs() < 0.1);
    }

    #[test]
    fn foreign_page_imports_native_ink() {
        let mut s = Stroke::new(Pen::default());
        s.points = vec![Point::new(0.0, 0.0, 1.0), Point::new(5.0, 5.0, 1.0)];
        let imported = interpret("<title>Foreign</title>", vec![s], None, false, &remote());
        assert_eq!(imported.source, ImportSource::NativeInk);
        assert_eq!(imported.page.title, "Foreign");
    }
}
