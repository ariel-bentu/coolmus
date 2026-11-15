// Stroke geometry for the screen. Mirrors crates/sketch-render (tessellate.rs,
// paper.rs) and sketch-core (Pen::width_at, effective_color, hit testing) so
// what you see matches the PDF and PNG export.

export const PAPER_COLOR = '#fdfdfb';
const RULE_COLOR = '#c9d8ea';
const MARGIN_COLOR = '#f0b4b4';
const RULE_SPACING = 32;
const GRID_SPACING = 24;
export const DEFAULT_PAGE = { width: 794, height: 1123 };
/** Space between stacked sheets (sketch-core SHEET_GAP). */
export const SHEET_GAP = 24;

/**
 * The sheets as { x, y, w, h } rectangles in page units (sketch-core
 * Page::sheet_rect): page.width × page.height each, or for a page drawn on
 * another PDF, that PDF's pages at their own sizes.
 */
export function sheetRects(page) {
  const sizes = page.background?.sheets ?? Array.from({ length: Math.max(1, page.sheets ?? 1) }, () => ({ width: page.width, height: page.height }));
  let y = 0;
  return sizes.map((s) => {
    const r = { x: 0, y, w: s.width, h: s.height };
    y += s.height + SHEET_GAP;
    return r;
  });
}
export const sheetCount = (page) => sheetRects(page).length;
/** Top edge of sheet `i`. */
export const sheetTop = (page, i) => (i < sheetCount(page) ? sheetRects(page)[i].y : i * (page.height + SHEET_GAP));
/** Height of all sheets with the gaps between them. */
export const totalHeight = (page) => {
  const rects = sheetRects(page);
  const last = rects[rects.length - 1];
  return last.y + last.h;
};
/** True if (x, y) is on one of the sheets. */
export const onSheet = (page, x, y) => sheetRects(page).some((r) => x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h);

export function widthAt(pen, pressure) {
  if (!pen.pressure_sensitive || pen.style === 'Highlighter') return pen.width;
  const p = Math.min(1, Math.max(0, pressure));
  const min = pen.style === 'Pencil' ? 0.15 : 0.35;
  return pen.width * (min + (1 - min) * p);
}

export function effectiveColor(pen) {
  const c = pen.color;
  let a = c.a;
  if (pen.style === 'Highlighter') a = Math.min(a, 90);
  else if (pen.style === 'Pencil') a = Math.min(a, 200);
  return `rgba(${c.r},${c.g},${c.b},${a / 255})`;
}

/** Splits a stroke into pieces {pts: [[x,y]], radii: []} by pen style. */
export function pieces(stroke) {
  const pen = stroke.pen;
  const src = [];
  for (const [x, y, p] of stroke.points) {
    const last = src[src.length - 1];
    if (last && Math.hypot(last.x - x, last.y - y) < 0.01) continue;
    src.push({ x, y, r: widthAt(pen, p) * 0.5 });
  }
  if (!src.length) return [];
  if (pen.style === 'Dashed') return dash(src, pen.width * 3 + 6, pen.width * 2 + 5);
  if (pen.style === 'Dotted') return dots(src, pen.width * 2 + 3);
  return [{ pts: src.map((p) => [p.x, p.y]), radii: src.map((p) => p.r) }];
}

const lerp = (a, b, t) => [[a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t], a.r + (b.r - a.r) * t];

function dash(src, on, off) {
  const out = [];
  let cur = { pts: [[src[0].x, src[0].y]], radii: [src[0].r] };
  let drawing = true;
  let remaining = on;
  for (let i = 0; i + 1 < src.length; i++) {
    const a = src[i], b = src[i + 1];
    const seg = Math.hypot(b.x - a.x, b.y - a.y);
    let pos = 0;
    while (seg - pos > remaining) {
      pos += remaining;
      const [p, r] = lerp(a, b, pos / seg);
      cur.pts.push(p);
      cur.radii.push(r);
      if (drawing) {
        out.push(cur);
        cur = { pts: [], radii: [] };
        remaining = off;
      } else {
        remaining = on;
      }
      drawing = !drawing;
    }
    remaining -= seg - pos;
    if (drawing) {
      cur.pts.push([b.x, b.y]);
      cur.radii.push(b.r);
    }
  }
  if (drawing && cur.pts.length) out.push(cur);
  return out;
}

function dots(src, spacing) {
  const out = [{ pts: [[src[0].x, src[0].y]], radii: [src[0].r] }];
  let remaining = spacing;
  for (let i = 0; i + 1 < src.length; i++) {
    const a = src[i], b = src[i + 1];
    const seg = Math.hypot(b.x - a.x, b.y - a.y);
    let pos = 0;
    while (seg - pos >= remaining) {
      pos += remaining;
      const [p, r] = lerp(a, b, pos / seg);
      out.push({ pts: [p], radii: [r] });
      remaining = spacing;
    }
    remaining -= seg - pos;
  }
  return out;
}

function normals(pts) {
  const n = pts.length;
  return pts.map((_, i) => {
    const a = pts[Math.max(0, i - 1)], b = pts[Math.min(n - 1, i + 1)];
    const dx = b[0] - a[0], dy = b[1] - a[1];
    const len = Math.hypot(dx, dy);
    return len < 1e-6 ? [0, 1] : [-dy / len, dx / len];
  });
}

/** Builds a fillable Path2D (page coordinates) for a stroke. */
export function strokePath(stroke) {
  const path = new Path2D();
  const roundCaps = stroke.pen.style !== 'Highlighter';
  for (const piece of pieces(stroke)) {
    const n = piece.pts.length;
    if (n === 1) {
      const [x, y] = piece.pts[0];
      path.moveTo(x + Math.max(piece.radii[0], 0.5), y);
      path.arc(x, y, Math.max(piece.radii[0], 0.5), 0, Math.PI * 2);
      path.closePath();
      continue;
    }
    const nm = normals(piece.pts);
    const P = piece.pts, R = piece.radii;
    path.moveTo(P[0][0] + nm[0][0] * R[0], P[0][1] + nm[0][1] * R[0]);
    for (let i = 1; i < n; i++) path.lineTo(P[i][0] + nm[i][0] * R[i], P[i][1] + nm[i][1] * R[i]);
    const e = n - 1;
    if (roundCaps) {
      const a = Math.atan2(nm[e][1], nm[e][0]);
      path.arc(P[e][0], P[e][1], R[e], a, a - Math.PI, true);
    }
    for (let i = e; i >= 0; i--) path.lineTo(P[i][0] - nm[i][0] * R[i], P[i][1] - nm[i][1] * R[i]);
    if (roundCaps) {
      const a = Math.atan2(-nm[0][1], -nm[0][0]);
      path.arc(P[0][0], P[0][1], R[0], a, a - Math.PI, true);
    }
    path.closePath();
  }
  return path;
}

export function bounds(stroke) {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const [x, y] of stroke.points) {
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
  }
  const m = stroke.pen.width * 0.5 + 1;
  return { minX: minX - m, minY: minY - m, maxX: maxX + m, maxY: maxY + m };
}

function distSqSeg(px, py, a, b) {
  const dx = b[0] - a[0], dy = b[1] - a[1];
  const len = dx * dx + dy * dy;
  const t = len < 1e-9 ? 0 : Math.min(1, Math.max(0, ((px - a[0]) * dx + (py - a[1]) * dy) / len));
  const cx = a[0] + t * dx, cy = a[1] + t * dy;
  return (px - cx) ** 2 + (py - cy) ** 2;
}

export function hit(stroke, x, y, radius) {
  const reach = radius + stroke.pen.width * 0.5;
  const r2 = reach * reach;
  const pts = stroke.points;
  if (pts.length === 1) return (pts[0][0] - x) ** 2 + (pts[0][1] - y) ** 2 <= r2;
  for (let i = 0; i + 1 < pts.length; i++) if (distSqSeg(x, y, pts[i], pts[i + 1]) <= r2) return true;
  return false;
}

/** Low-latency exponential smoothing with a minimum-distance filter. */
export class Smoother {
  constructor(strength, minDistance) {
    this.a = 1 - Math.min(0.95, Math.max(0, strength));
    this.min = minDistance;
    this.last = null;
  }
  push([x, y, p]) {
    if (!this.last) return (this.last = [x, y, p]);
    const l = this.last;
    const q = [l[0] + (x - l[0]) * this.a, l[1] + (y - l[1]) * this.a, l[2] + (p - l[2]) * this.a];
    if (Math.hypot(q[0] - l[0], q[1] - l[1]) < this.min) return null;
    return (this.last = q);
  }
}

/** Draws the paper pattern of every visible sheet (view in page coordinates). */
export function drawPaper(ctx, page, view, zoom) {
  if (page.background) return; // the PDF underneath is the paper
  for (const r of sheetRects(page)) {
    if (r.y > view.maxY || r.y + r.h < view.minY) continue;
    ctx.save();
    ctx.translate(0, r.y);
    sheetPaper(ctx, page, { ...view, minY: view.minY - r.y, maxY: view.maxY - r.y }, zoom);
    ctx.restore();
  }
}

/** The paper pattern of one sheet, in sheet coordinates. */
function sheetPaper(ctx, page, view, zoom) {
  const lw = Math.max(0.5, 1) / zoom;
  const { minX, minY, maxX, maxY } = view;
  const W = page.width;
  const H = page.height;
  ctx.lineWidth = lw;
  if (page.paper === 'Lined') {
    ctx.strokeStyle = RULE_COLOR;
    ctx.beginPath();
    for (let y = RULE_SPACING * 3; y < H; y += RULE_SPACING) {
      if (y < minY || y > maxY) continue;
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
    }
    ctx.stroke();
    ctx.strokeStyle = MARGIN_COLOR;
    ctx.beginPath();
    ctx.moveTo(80, 0);
    ctx.lineTo(80, H);
    ctx.stroke();
  } else if (page.paper === 'Grid') {
    ctx.strokeStyle = RULE_COLOR;
    ctx.beginPath();
    for (let y = GRID_SPACING; y < H; y += GRID_SPACING) {
      if (y < minY || y > maxY) continue;
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
    }
    for (let x = GRID_SPACING; x < W; x += GRID_SPACING) {
      if (x < minX || x > maxX) continue;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, H);
    }
    ctx.stroke();
  } else if (page.paper === 'Dotted') {
    ctx.fillStyle = RULE_COLOR;
    const r = Math.max(1.2, 0.6 / zoom);
    ctx.beginPath();
    for (let y = GRID_SPACING; y < H; y += GRID_SPACING) {
      if (y < minY || y > maxY) continue;
      for (let x = GRID_SPACING; x < W; x += GRID_SPACING) {
        if (x < minX || x > maxX) continue;
        ctx.moveTo(x + r, y);
        ctx.arc(x, y, r, 0, Math.PI * 2);
      }
    }
    ctx.fill();
  }
}
