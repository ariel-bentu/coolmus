use sketch_core::{Color, Page};
use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke as SkStroke, Transform,
};

use crate::paper::{self, Mark, PAPER_COLOR};
use crate::tessellate::outlines;

#[derive(Debug, Clone, Copy)]
pub struct RasterOptions {
    /// Output pixels per page unit.
    pub scale: f32,
    /// Draw paper color and pattern; otherwise transparent background.
    pub paper: bool,
    /// Crop to the ink (plus margin) instead of the full page.
    pub crop_to_ink: bool,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self { scale: 1.0, paper: true, crop_to_ink: false }
    }
}

#[derive(Debug)]
pub struct RasterError(pub String);

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RasterError {}

fn sk_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Renders the page to an RGBA pixmap.
pub fn render(page: &Page, opts: RasterOptions) -> Result<Pixmap, RasterError> {
    let (ox, oy, w, h) = if opts.crop_to_ink && !page.ink_bounds().is_empty() {
        let b = page.ink_bounds().expand(16.0);
        (b.min_x.max(0.0), b.min_y.max(0.0), b.width(), b.height())
    } else {
        (0.0, 0.0, page.width, page.height)
    };
    let pw = ((w * opts.scale).ceil() as u32).clamp(1, 16_384);
    let ph = ((h * opts.scale).ceil() as u32).clamp(1, 16_384);
    let mut pixmap = Pixmap::new(pw, ph).ok_or_else(|| RasterError("invalid image size".into()))?;
    let tf = Transform::from_scale(opts.scale, opts.scale).pre_translate(-ox, -oy);

    if opts.paper {
        pixmap.fill(sk_color(PAPER_COLOR));
        for mark in paper::marks(page.paper, page.width, page.height) {
            draw_mark(&mut pixmap, &mark, tf);
        }
    }

    for stroke in crate::paint_order(&page.strokes) {
        let mut pb = PathBuilder::new();
        for poly in outlines(stroke) {
            let mut it = poly.iter();
            if let Some(p) = it.next() {
                pb.move_to(p[0], p[1]);
                for p in it {
                    pb.line_to(p[0], p[1]);
                }
                pb.close();
            }
        }
        let Some(path) = pb.finish() else { continue };
        let color = stroke.pen.effective_color();
        let mut paint = Paint { anti_alias: true, ..Paint::default() };
        if color.a == 255 {
            paint.set_color(sk_color(color));
            pixmap.fill_path(&path, &paint, FillRule::Winding, tf, None);
        } else {
            // Translucent: draw opaque on a layer, then composite once so
            // self-overlaps do not darken.
            let mut layer = Pixmap::new(pw, ph).ok_or_else(|| RasterError("layer".into()))?;
            paint.set_color(sk_color(color.with_alpha(255)));
            layer.fill_path(&path, &paint, FillRule::Winding, tf, None);
            let pp = PixmapPaint { opacity: color.a as f32 / 255.0, ..PixmapPaint::default() };
            pixmap.draw_pixmap(0, 0, layer.as_ref(), &pp, Transform::identity(), None);
        }
    }
    Ok(pixmap)
}

fn draw_mark(pixmap: &mut Pixmap, mark: &Mark, tf: Transform) {
    let mut paint = Paint { anti_alias: true, ..Paint::default() };
    match *mark {
        Mark::Line { from, to, width, color } => {
            let mut pb = PathBuilder::new();
            pb.move_to(from[0], from[1]);
            pb.line_to(to[0], to[1]);
            if let Some(path) = pb.finish() {
                paint.set_color(sk_color(color));
                let st = SkStroke { width, ..SkStroke::default() };
                pixmap.stroke_path(&path, &paint, &st, tf, None);
            }
        }
        Mark::Dot { center, radius, color } => {
            if let Some(path) = PathBuilder::from_circle(center[0], center[1], radius) {
                paint.set_color(sk_color(color));
                pixmap.fill_path(&path, &paint, FillRule::Winding, tf, None);
            }
        }
    }
}

/// Renders the page to PNG bytes.
pub fn render_png(page: &Page, opts: RasterOptions) -> Result<Vec<u8>, RasterError> {
    render(page, opts)?.encode_png().map_err(|e| RasterError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tessellate::stroke_from_xy;
    use sketch_core::{Pen, PenStyle};

    #[test]
    fn renders_png() {
        let mut page = Page::new("t");
        for style in PenStyle::ALL {
            page.strokes.push(stroke_from_xy(
                Pen { style, width: 6.0, ..Pen::default() },
                &[(10.0, 10.0), (200.0, 80.0), (300.0, 20.0)],
            ));
        }
        let png = render_png(&page, RasterOptions::default()).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let px = render(&page, RasterOptions { paper: false, ..Default::default() }).unwrap();
        assert!(px.pixels().iter().any(|p| p.alpha() > 0));
    }
}
