use std::f32::consts::PI;

use sketch_core::{Pen, PenStyle, Point, Stroke};

/// A single-colored triangle mesh in page coordinates.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn clear(&mut self) {
        self.positions.clear();
        self.indices.clear();
    }

    fn push(&mut self, x: f32, y: f32) -> u32 {
        self.positions.push([x, y]);
        (self.positions.len() - 1) as u32
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }
}

/// A continuous run of centerline points with per-point radii.
#[derive(Debug, Clone, Default)]
pub struct Piece {
    pub pts: Vec<[f32; 2]>,
    pub radii: Vec<f32>,
}

/// Splits a stroke into drawable pieces according to its pen style
/// (dashes, dots, or one continuous piece).
pub fn pieces(stroke: &Stroke) -> Vec<Piece> {
    let pen = &stroke.pen;
    let mut src: Vec<(Point, f32)> = Vec::with_capacity(stroke.points.len());
    for p in &stroke.points {
        if src.last().is_some_and(|(q, _)| q.distance(p) < 0.01) {
            continue;
        }
        src.push((*p, pen.width_at(p.pressure) * 0.5));
    }
    if src.is_empty() {
        return Vec::new();
    }
    match pen.style {
        PenStyle::Dashed => dash(&src, pen.width * 3.0 + 6.0, pen.width * 2.0 + 5.0),
        PenStyle::Dotted => dots(&src, pen.width * 2.0 + 3.0),
        _ => vec![Piece {
            pts: src.iter().map(|(p, _)| [p.x, p.y]).collect(),
            radii: src.iter().map(|(_, r)| *r).collect(),
        }],
    }
}

fn lerp(a: &(Point, f32), b: &(Point, f32), t: f32) -> ([f32; 2], f32) {
    (
        [a.0.x + (b.0.x - a.0.x) * t, a.0.y + (b.0.y - a.0.y) * t],
        a.1 + (b.1 - a.1) * t,
    )
}

/// Splits the polyline into `on`-long dashes separated by `off`-long gaps.
fn dash(src: &[(Point, f32)], on: f32, off: f32) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut cur = Piece::default();
    let mut drawing = true;
    let mut remaining = on;
    cur.pts.push([src[0].0.x, src[0].0.y]);
    cur.radii.push(src[0].1);
    for w in src.windows(2) {
        let seg = w[0].0.distance(&w[1].0);
        let mut pos = 0.0;
        while seg - pos > remaining {
            pos += remaining;
            let (p, r) = lerp(&w[0], &w[1], pos / seg);
            if drawing {
                cur.pts.push(p);
                cur.radii.push(r);
                out.push(std::mem::take(&mut cur));
                remaining = off;
            } else {
                cur.pts.push(p);
                cur.radii.push(r);
                remaining = on;
            }
            drawing = !drawing;
        }
        remaining -= seg - pos;
        if drawing {
            cur.pts.push([w[1].0.x, w[1].0.y]);
            cur.radii.push(w[1].1);
        }
    }
    if drawing && !cur.pts.is_empty() {
        out.push(cur);
    }
    out
}

fn dots(src: &[(Point, f32)], spacing: f32) -> Vec<Piece> {
    let dot = |p: [f32; 2], r: f32| Piece { pts: vec![p], radii: vec![r] };
    let mut out = vec![dot([src[0].0.x, src[0].0.y], src[0].1)];
    let mut remaining = spacing;
    for w in src.windows(2) {
        let seg = w[0].0.distance(&w[1].0);
        let mut pos = 0.0;
        while seg - pos >= remaining {
            pos += remaining;
            let (p, r) = lerp(&w[0], &w[1], pos / seg);
            out.push(dot(p, r));
            remaining = spacing;
        }
        remaining -= seg - pos;
    }
    out
}

fn circle_segments(r: f32) -> usize {
    ((r * 2.0).sqrt() * 4.0).clamp(8.0, 40.0) as usize
}

/// Appends the stroke's triangles to `mesh`.
pub fn tessellate_stroke(stroke: &Stroke, mesh: &mut Mesh) {
    let round_caps = stroke.pen.style != PenStyle::Highlighter;
    for piece in pieces(stroke) {
        tessellate_piece(&piece, round_caps, mesh);
    }
}

#[allow(clippy::needless_range_loop)] // parallel arrays
fn tessellate_piece(piece: &Piece, round_caps: bool, mesh: &mut Mesh) {
    let n = piece.pts.len();
    if n == 1 {
        let [x, y] = piece.pts[0];
        fan(mesh, x, y, piece.radii[0].max(0.5), 0.0, 2.0 * PI);
        return;
    }
    let normals = normals(&piece.pts);
    let base = mesh.positions.len() as u32;
    for i in 0..n {
        let [x, y] = piece.pts[i];
        let [nx, ny] = normals[i];
        let r = piece.radii[i];
        mesh.push(x + nx * r, y + ny * r);
        mesh.push(x - nx * r, y - ny * r);
    }
    for i in 0..(n as u32 - 1) {
        let (l0, r0, l1, r1) = (base + 2 * i, base + 2 * i + 1, base + 2 * i + 2, base + 2 * i + 3);
        mesh.tri(l0, r0, l1);
        mesh.tri(r0, r1, l1);
    }
    if round_caps {
        // Half-discs beyond each end, so caps do not overlap the body
        // (matters for translucent pens).
        let [nx, ny] = normals[0];
        let a = ny.atan2(nx);
        fan(mesh, piece.pts[0][0], piece.pts[0][1], piece.radii[0], a, a + PI);
        let [nx, ny] = normals[n - 1];
        let a = (-ny).atan2(-nx);
        fan(mesh, piece.pts[n - 1][0], piece.pts[n - 1][1], piece.radii[n - 1], a, a + PI);
    }
}

/// Unit normals per vertex using the averaged direction of neighbors.
fn normals(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let a = pts[i.saturating_sub(1)];
            let b = pts[(i + 1).min(n - 1)];
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy);
            if len <= f32::EPSILON {
                [0.0, 1.0]
            } else {
                [-dy / len, dx / len]
            }
        })
        .collect()
}

/// Triangle fan from angle `a0` to `a1` (counter-clockwise in math coordinates).
fn fan(mesh: &mut Mesh, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
    let segs = ((circle_segments(r) as f32) * (a1 - a0) / (2.0 * PI)).ceil().max(2.0) as usize;
    let c = mesh.push(cx, cy);
    let mut prev = mesh.push(cx + r * a0.cos(), cy + r * a0.sin());
    for k in 1..=segs {
        let a = a0 + (a1 - a0) * k as f32 / segs as f32;
        let v = mesh.push(cx + r * a.cos(), cy + r * a.sin());
        mesh.tri(c, prev, v);
        prev = v;
    }
}

/// Closed outline polygon(s) of a stroke, for path-based rasterizers.
#[allow(clippy::needless_range_loop)] // parallel arrays
pub fn outlines(stroke: &Stroke) -> Vec<Vec<[f32; 2]>> {
    let round_caps = stroke.pen.style != PenStyle::Highlighter;
    pieces(stroke)
        .iter()
        .map(|piece| {
            let n = piece.pts.len();
            // Half circle sweeping clockwise (decreasing angle) from `a0`, which
            // keeps caps on the outside when walking the outline left-side first.
            let arc = |out: &mut Vec<[f32; 2]>, c: [f32; 2], r: f32, a0: f32| {
                let segs = circle_segments(r) / 2 + 1;
                for k in 0..=segs {
                    let a = a0 - PI * k as f32 / segs as f32;
                    out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
                }
            };
            if n == 1 {
                let mut out = Vec::new();
                let r = piece.radii[0].max(0.5);
                arc(&mut out, piece.pts[0], r, 0.0);
                arc(&mut out, piece.pts[0], r, PI);
                return out;
            }
            let nm = normals(&piece.pts);
            let mut out = Vec::with_capacity(n * 2 + 32);
            for i in 0..n {
                let r = piece.radii[i];
                out.push([piece.pts[i][0] + nm[i][0] * r, piece.pts[i][1] + nm[i][1] * r]);
            }
            if round_caps {
                let [nx, ny] = nm[n - 1];
                arc(&mut out, piece.pts[n - 1], piece.radii[n - 1], ny.atan2(nx));
            }
            for i in (0..n).rev() {
                let r = piece.radii[i];
                out.push([piece.pts[i][0] - nm[i][0] * r, piece.pts[i][1] - nm[i][1] * r]);
            }
            if round_caps {
                let [nx, ny] = nm[0];
                arc(&mut out, piece.pts[0], piece.radii[0], (-ny).atan2(-nx));
            }
            out
        })
        .collect()
}

/// Used by tests and callers that need a quick stroke.
pub fn stroke_from_xy(pen: Pen, pts: &[(f32, f32)]) -> Stroke {
    let mut s = Stroke::new(pen);
    s.points = pts.iter().map(|&(x, y)| Point::new(x, y, 1.0)).collect();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(style: PenStyle) -> Stroke {
        stroke_from_xy(Pen { style, width: 4.0, ..Pen::default() }, &[(0.0, 0.0), (100.0, 0.0)])
    }

    #[test]
    fn solid_is_one_piece() {
        assert_eq!(pieces(&line(PenStyle::Solid)).len(), 1);
    }

    #[test]
    fn dashed_splits() {
        // on = 18, off = 13 → 100 / 31 ≈ 3.2 dashes → 4 pieces (last partial)
        assert_eq!(pieces(&line(PenStyle::Dashed)).len(), 4);
    }

    #[test]
    fn dotted_spacing() {
        // spacing 11 → dots at 0, 11, ..., 99 → 10 dots
        assert_eq!(pieces(&line(PenStyle::Dotted)).len(), 10);
    }

    #[test]
    fn mesh_indices_valid() {
        let mut m = Mesh::default();
        for style in PenStyle::ALL {
            tessellate_stroke(&line(style), &mut m);
        }
        assert!(!m.indices.is_empty());
        assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()));
    }

    #[test]
    fn single_point_is_a_dot() {
        let s = stroke_from_xy(Pen::default(), &[(5.0, 5.0)]);
        let mut m = Mesh::default();
        tessellate_stroke(&s, &mut m);
        assert!(m.indices.len() >= 8 * 3);
    }
}
