//! Renderer-agnostic stroke geometry plus a CPU rasterizer.
//!
//! * [`tessellate`] produces triangle meshes that any GPU front-end can upload.
//! * [`raster`] renders a whole page to PNG (used for picture export).
//! * [`paper`] describes the background pattern so every front-end draws the same paper.

pub mod paper;
pub mod raster;
pub mod tessellate;

pub use tessellate::{tessellate_stroke, Mesh};

use coolmus_core::{PenStyle, Stroke};

/// Strokes in paint order: highlighters underneath, everything else on top,
/// preserving the original order within each group.
pub fn paint_order(strokes: &[Stroke]) -> impl Iterator<Item = &Stroke> {
    let hl = |s: &&Stroke| s.pen.style == PenStyle::Highlighter;
    strokes.iter().filter(hl).chain(strokes.iter().filter(move |s| !hl(s)))
}
