//! Pages drawn on a PDF from elsewhere (a worksheet, a scan, slides).
//!
//! The PDF stays as it was: every object of it is kept, and Sketcher adds
//! a layer on top of each page it drew on, plus its page data. The layer is
//! one Form XObject per page, drawn after the page's own content, which is
//! wrapped in `q … Q` so nothing it leaves set (a transform, a clip) leaks
//! into the layer. The page dictionary remembers its original `Contents` and
//! `Resources`, so [`strip`] can take the layer off again: for showing the
//! background under the editable layer, and before writing a new layer.

use std::collections::HashMap;

use lopdf::{dictionary, Dictionary, Object, ObjectId, Stream};
use sketch_core::{Background, Page, Rect, SheetSize};

use super::*;

/// The page's `Contents` before Sketcher's layer (`null`: it had none).
const ORIGINAL_CONTENTS: &[u8] = b"SketcherOriginalContents";
/// The page's own `Resources` before Sketcher's layer (`null`: inherited).
const ORIGINAL_RESOURCES: &[u8] = b"SketcherOriginalResources";
/// Resource name of the layer.
const LAYER: &[u8] = b"SketcherLayer";

/// The sheets for drawing on `bytes`: one per PDF page, sized as shown.
pub fn background(bytes: &[u8]) -> Result<Background> {
    let doc = lopdf::Document::load_mem(bytes)?;
    let sheets = doc
        .get_pages()
        .values()
        .map(|&id| {
            let [x0, y0, x1, y1] = visible_box(&doc, id);
            let (w, h) = if rotation(&doc, id) % 180 == 90 { (y1 - y0, x1 - x0) } else { (x1 - x0, y1 - y0) };
            SheetSize { width: w / PT_PER_PX, height: h / PT_PER_PX }
        })
        .collect();
    Ok(Background { sheets })
}

/// `original` with `page` drawn on top and its data attached. A layer from an
/// earlier save is replaced, not stacked.
pub fn write_over(original: &[u8], page: &Page, font: &[u8]) -> Result<Vec<u8>> {
    let mut doc = lopdf::Document::load_mem(original)?;
    if doc.is_encrypted() {
        return Err(Error::Encrypted);
    }
    strip_doc(&mut doc)?;

    // The layer: drawn like a Sketcher page, one transparent PDF page per sheet.
    let targets: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let sheets: Vec<Rect> = (0..page.sheet_count()).map(|i| page.sheet_rect(i)).take(targets.len()).collect();
    let mut layer = new_document("");
    let text_font = if page.texts.is_empty() { None } else { Some(TextFont::new(font)?) };
    for sheet in &sheets {
        draw_sheet(&mut layer, page, *sheet, text_font.as_ref(), false)?;
    }
    let layer_bytes = layer.finish().map_err(|e| Error::Write(format!("{e:?}")))?;
    let layer = lopdf::Document::load_mem(&layer_bytes)?;

    let mut imported = HashMap::new();
    for ((&target, &source), sheet) in targets.iter().zip(layer.get_pages().values()).zip(&sheets) {
        if has_content(page, sheet) {
            let form = form_xobject(&layer, source, &mut doc, &mut imported)?;
            add_layer(&mut doc, target, form)?;
        }
    }
    attach(&mut doc, serde_json::to_vec(page)?);
    stamp(&mut doc)?;
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::Write(e.to_string()))?;
    Ok(out)
}

/// The PDF without Sketcher's layer and data: what it was before Sketcher
/// drew on it. Returned as is if Sketcher never did.
pub fn strip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut doc = lopdf::Document::load_mem(bytes)?;
    if !strip_doc(&mut doc)? {
        return Ok(bytes.to_vec());
    }
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::Write(e.to_string()))?;
    Ok(out)
}

/// Takes off the layer and data; false if there were none.
fn strip_doc(doc: &mut lopdf::Document) -> Result<bool> {
    let mut found = false;
    for id in doc.get_pages().values().copied().collect::<Vec<_>>() {
        let page = doc.get_dictionary_mut(id)?;
        for (key, marker) in [(&b"Contents"[..], ORIGINAL_CONTENTS), (b"Resources", ORIGINAL_RESOURCES)] {
            let Ok(original) = page.get(marker).cloned() else { continue };
            found = true;
            match original {
                Object::Null => {
                    page.remove(key);
                }
                original => page.set(key, original),
            }
            page.remove(marker);
        }
        page.remove(FINGERPRINT_KEY);
        page.remove(ANNOTATIONS_KEY);
    }
    Ok(detach(doc) || found)
}

fn has_content(page: &Page, sheet: &Rect) -> bool {
    page.strokes.iter().any(|s| s.bounds().intersects(sheet))
        || page.images.iter().any(|im| im.bounds().intersects(sheet))
        || page.texts.iter().any(|t| t.y <= sheet.max_y && t.bounds().max_y + 20.0 * t.size >= sheet.min_y)
}

// ---- geometry ----

/// An attribute a page may inherit from its ancestors in the page tree.
fn inherited<'a>(doc: &'a lopdf::Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut node = doc.get_dictionary(page).ok()?;
    for _ in 0..64 {
        if let Ok(value) = node.get(key) {
            return doc.dereference(value).ok().map(|(_, o)| o);
        }
        node = deref_dict(doc, node.get(b"Parent").ok()?)?;
    }
    None
}

fn number(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

fn rect_of(o: Option<&Object>) -> Option<[f32; 4]> {
    let a = o?.as_array().ok()?;
    let [x0, y0, x1, y1] = [number(a.first()?)?, number(a.get(1)?)?, number(a.get(2)?)?, number(a.get(3)?)?];
    Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

/// The part of the page viewers show: the crop box within the media box.
fn visible_box(doc: &lopdf::Document, page: ObjectId) -> [f32; 4] {
    let media = rect_of(inherited(doc, page, b"MediaBox")).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    match rect_of(inherited(doc, page, b"CropBox")) {
        Some(c) => {
            let b = [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])];
            if b[2] > b[0] && b[3] > b[1] { b } else { media }
        }
        None => media,
    }
}

/// Clockwise rotation viewers apply: 0, 90, 180 or 270.
fn rotation(doc: &lopdf::Document, page: ObjectId) -> i64 {
    let r = inherited(doc, page, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0);
    (r.rem_euclid(360) + 45) / 90 % 4 * 90
}

/// Maps the layer (origin at the bottom left of the page as shown) into the
/// page's own space.
fn layer_matrix(doc: &lopdf::Document, page: ObjectId) -> [f32; 6] {
    let [x0, y0, x1, y1] = visible_box(doc, page);
    match rotation(doc, page) {
        90 => [0.0, 1.0, -1.0, 0.0, x1, y0],
        180 => [-1.0, 0.0, 0.0, -1.0, x1, y1],
        270 => [0.0, -1.0, 1.0, 0.0, x0, y1],
        _ => [1.0, 0.0, 0.0, 1.0, x0, y0],
    }
}

// ---- the layer ----

/// Copies `obj` from `src` into `dst`, with everything it refers to.
fn import(src: &lopdf::Document, obj: &Object, dst: &mut lopdf::Document, map: &mut HashMap<ObjectId, ObjectId>) -> Object {
    match obj {
        Object::Reference(id) => {
            if let Some(&new) = map.get(id) {
                return Object::Reference(new);
            }
            let new = dst.new_object_id();
            map.insert(*id, new);
            let copy = src.get_object(*id).map(|o| import(src, o, dst, map)).unwrap_or(Object::Null);
            dst.objects.insert(new, copy);
            Object::Reference(new)
        }
        Object::Array(items) => Object::Array(items.iter().map(|o| import(src, o, dst, map)).collect()),
        Object::Dictionary(d) => Object::Dictionary(import_dict(src, d, dst, map)),
        Object::Stream(s) => {
            let mut s = s.clone();
            s.dict = import_dict(src, &s.dict, dst, map);
            Object::Stream(s)
        }
        other => other.clone(),
    }
}

fn import_dict(src: &lopdf::Document, d: &Dictionary, dst: &mut lopdf::Document, map: &mut HashMap<ObjectId, ObjectId>) -> Dictionary {
    let mut out = Dictionary::new();
    for (k, v) in d.iter() {
        out.set(k.clone(), import(src, v, dst, map));
    }
    out
}

/// Page `source` of the layer document as a Form XObject in `doc`.
fn form_xobject(layer: &lopdf::Document, source: ObjectId, doc: &mut lopdf::Document, map: &mut HashMap<ObjectId, ObjectId>) -> Result<ObjectId> {
    let page = layer.get_dictionary(source)?;
    let content = layer.get_page_content_with_limit(source, MAX_DECODED)?;
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => page.get(b"MediaBox")?.clone(),
    };
    for key in [&b"Resources"[..], b"Group"] {
        if let Ok(value) = page.get(key) {
            dict.set(key, import(layer, value, doc, map));
        }
    }
    let mut stream = Stream::new(dict, content);
    let _ = stream.compress();
    Ok(doc.add_object(stream))
}

/// Draws `form` over page `id`, remembering what to restore in [`strip_doc`].
fn add_layer(doc: &mut lopdf::Document, id: ObjectId, form: ObjectId) -> Result<()> {
    let m = layer_matrix(doc, id);
    let page = doc.get_dictionary(id)?;
    let contents = page.get(b"Contents").cloned().unwrap_or(Object::Null);
    let own_resources = page.get(b"Resources").cloned().unwrap_or(Object::Null);
    // Resources are inherited whole, so the page gets its own copy with the layer added.
    let mut resources = inherited(doc, id, b"Resources").and_then(|o| o.as_dict().ok()).cloned().unwrap_or_default();
    let mut xobjects = resources
        .get(b"XObject")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    xobjects.set(LAYER, Object::Reference(form));
    resources.set("XObject", xobjects);

    let mut streams = Vec::new();
    match doc.dereference(&contents).map(|(_, o)| o) {
        Ok(Object::Array(items)) => streams.extend(items.iter().cloned()),
        Ok(Object::Null) | Err(_) => {}
        Ok(_) => streams.push(contents.clone()),
    }
    let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let draw = format!("\nQ q {} cm /SketcherLayer Do Q\n", m.map(|v| format!("{v}")).join(" "));
    let close = doc.add_object(Stream::new(Dictionary::new(), draw.into_bytes()));
    streams.insert(0, Object::Reference(open));
    streams.push(Object::Reference(close));

    let page = doc.get_dictionary_mut(id)?;
    page.set("Contents", streams);
    page.set("Resources", resources);
    page.set(ORIGINAL_CONTENTS, contents);
    page.set(ORIGINAL_RESOURCES, own_resources);
    Ok(())
}

// ---- page data ----

/// The catalog's `/Names /EmbeddedFiles` leaf names array, made if missing.
fn embedded_files(doc: &mut lopdf::Document) -> Option<(Dictionary, Dictionary)> {
    let catalog = doc.catalog().ok()?;
    let names = catalog.get(b"Names").ok().and_then(|o| deref_dict(doc, o)).cloned().unwrap_or_default();
    let files = names.get(b"EmbeddedFiles").ok().and_then(|o| deref_dict(doc, o)).cloned().unwrap_or_default();
    Some((names, files))
}

fn set_embedded_files(doc: &mut lopdf::Document, mut names: Dictionary, files: Dictionary) {
    let empty = files.get(b"Names").and_then(|n| n.as_array()).map_or(true, |a| a.is_empty()) && files.get(b"Kids").is_err();
    if empty {
        names.remove(b"EmbeddedFiles");
    } else {
        names.set("EmbeddedFiles", files);
    }
    if let Ok(catalog) = doc.catalog_mut() {
        if names.is_empty() {
            catalog.remove(b"Names");
        } else {
            catalog.set("Names", names);
        }
    }
}

fn attach(doc: &mut lopdf::Document, json: Vec<u8>) {
    let mut file = Stream::new(dictionary! { "Type" => "EmbeddedFile", "Subtype" => Object::Name(b"application/json".to_vec()) }, json);
    let _ = file.compress();
    let file = doc.add_object(file);
    let spec = doc.add_object(dictionary! {
        "Type" => "Filespec",
        "F" => Object::string_literal(PAGE_FILE),
        "UF" => Object::string_literal(PAGE_FILE),
        "EF" => dictionary! { "F" => file },
        "Desc" => Object::string_literal("Sketcher page data"),
        "AFRelationship" => "Source",
    });
    let Some((names, mut files)) = embedded_files(doc) else { return };
    let mut list = files.get(b"Names").and_then(|n| n.as_array()).cloned().unwrap_or_default();
    // Name trees are sorted by key.
    let key = PAGE_FILE.as_bytes();
    let at = list.chunks(2).position(|pair| pair[0].as_str().is_ok_and(|k| k > key)).map_or(list.len(), |i| i * 2);
    list.splice(at..at, [Object::string_literal(PAGE_FILE), Object::Reference(spec)]);
    files.set("Names", list);
    set_embedded_files(doc, names, files);
}

/// Removes Sketcher's page data; false if there was none.
fn detach(doc: &mut lopdf::Document) -> bool {
    let Some((names, mut files)) = embedded_files(doc) else { return false };
    let Ok(list) = files.get(b"Names").and_then(|n| n.as_array()) else { return false };
    let ours = |pair: &[Object]| doc.dereference(&pair[0]).ok().and_then(|(_, n)| n.as_str().ok()) == Some(PAGE_FILE.as_bytes());
    let kept: Vec<Object> = list.chunks(2).filter(|pair| pair.len() == 2 && !ours(pair)).flatten().cloned().collect();
    if kept.len() == list.len() {
        return false;
    }
    files.set("Names", kept);
    set_embedded_files(doc, names, files);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use krilla::page::PageSettings;

    /// Two pages drawn by "another program": the second rotated and cropped,
    /// the first with a comment, plus an attachment of its own and resources
    /// inherited from the page tree.
    fn foreign_pdf() -> Vec<u8> {
        let mut doc = Document::new();
        for _ in 0..2 {
            let mut page = doc.start_page_with(PageSettings::from_wh(200.0, 300.0).unwrap());
            let mut surface = page.surface();
            let mut pb = PathBuilder::new();
            pb.push_rect(krilla::geom::Rect::from_xywh(20.0, 20.0, 50.0, 50.0).unwrap());
            fill(&mut surface, &pb.finish().unwrap(), Color::BLACK);
            surface.finish();
            page.finish();
        }
        let mut d = lopdf::Document::load_mem(&doc.finish().unwrap()).unwrap();
        let ids: Vec<ObjectId> = d.get_pages().values().copied().collect();
        // Resources inherited from the page tree.
        let res = d.get_dictionary(ids[0]).unwrap().get(b"Resources").unwrap().clone();
        let parent = d.get_dictionary(ids[0]).unwrap().get(b"Parent").unwrap().as_reference().unwrap();
        d.get_dictionary_mut(parent).unwrap().set("Resources", res);
        d.get_dictionary_mut(ids[0]).unwrap().remove(b"Resources");
        let note = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()] });
        d.get_dictionary_mut(ids[0]).unwrap().set("Annots", vec![note.into()]);
        let second = d.get_dictionary_mut(ids[1]).unwrap();
        second.set("Rotate", 90);
        second.set("CropBox", vec![10.into(), 20.into(), 190.into(), 280.into()]);
        attach_other(&mut d);
        let mut out = Vec::new();
        d.save_to(&mut out).unwrap();
        out
    }

    fn attach_other(d: &mut lopdf::Document) {
        let file = d.add_object(Stream::new(dictionary! { "Type" => "EmbeddedFile" }, b"theirs".to_vec()));
        let spec = d.add_object(dictionary! { "Type" => "Filespec", "F" => Object::string_literal("a.txt"), "EF" => dictionary! { "F" => file } });
        d.catalog_mut().unwrap().set("Names", dictionary! { "EmbeddedFiles" => dictionary! { "Names" => vec![Object::string_literal("a.txt"), spec.into()] } });
    }

    fn page_on(original: &[u8]) -> Page {
        let mut page = Page::on_background("w", background(original).unwrap());
        for i in 0..2 {
            let top = page.sheet_top(i);
            let mut s = sketch_core::Stroke::new(sketch_core::Pen::default());
            s.points = vec![sketch_core::Point::new(10.0, top + 10.0, 1.0), sketch_core::Point::new(100.0, top + 60.0, 1.0)];
            page.strokes.push(s);
        }
        page
    }

    fn contents(bytes: &[u8]) -> Vec<Vec<u8>> {
        let d = lopdf::Document::load_mem(bytes).unwrap();
        d.get_pages().values().map(|&id| d.get_page_content(id)).collect()
    }

    #[test]
    fn sheets_are_the_pages_as_shown() {
        let bg = background(&foreign_pdf()).unwrap();
        let px = |pt: f32| pt / PT_PER_PX;
        assert_eq!(bg.sheets, vec![SheetSize { width: px(200.0), height: px(300.0) }, SheetSize { width: px(260.0), height: px(180.0) }]);
    }

    #[test]
    fn drawing_adds_a_layer_and_keeps_the_original() {
        let original = foreign_pdf();
        let page = page_on(&original);
        let written = write_over(&original, &page, &[]).unwrap();

        let Opened::Intact(back) = read(&written).unwrap() else { panic!("not intact") };
        assert_eq!(back, Page { title: String::new(), ..page.clone() });
        let d = lopdf::Document::load_mem(&written).unwrap();
        for (i, &id) in d.get_pages().values().enumerate() {
            let content = String::from_utf8_lossy(&d.get_page_content(id)).to_string();
            assert!(content.starts_with("q\n") && content.contains("/SketcherLayer Do"), "page {i}: {content}");
        }
        // Their comment and attachment are still there.
        assert_eq!(annotation_count(&d), 1);
        assert!(embedded_files(&mut d.clone()).unwrap().1.get(b"Names").unwrap().as_array().unwrap().len() == 4);

        // Saving again replaces the layer instead of adding one.
        let again = write_over(&written, &page, &[]).unwrap();
        assert_eq!(contents(&again), contents(&written));
        assert!(again.len() < written.len() + 512, "{} vs {}", again.len(), written.len());

        // Stripping gives back what they drew, without Sketcher's data.
        let stripped = strip(&again).unwrap();
        assert_eq!(contents(&stripped), contents(&original));
        assert!(page_data(&stripped).unwrap().is_none());
        assert_eq!(strip(&original).unwrap(), original, "untouched PDFs pass through");
    }

    #[test]
    fn layer_lands_on_rotated_pages_as_shown() {
        let original = foreign_pdf();
        let d = lopdf::Document::load_mem(&original).unwrap();
        let ids: Vec<ObjectId> = d.get_pages().values().copied().collect();
        // Rotated 90° clockwise and cropped to [10 20 190 280]: the page's
        // top edge is shown on the right, so a point (u, v) on the page as
        // shown is (190 - v, 20 + u) on the page itself.
        let m = layer_matrix(&d, ids[1]);
        let at = |u: f32, v: f32| (m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5]);
        assert_eq!(at(0.0, 0.0), (190.0, 20.0));
        assert_eq!(at(260.0, 180.0), (10.0, 280.0));
        assert_eq!(layer_matrix(&d, ids[0]), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
}
