// Stroke geometry: sheets, pen widths, dashes, hit-testing, the eraser's cut,
// input smoothing and straight lines.

import test from 'node:test';
import assert from 'node:assert/strict';
import { bounds, cutStroke, effectiveAlpha, hit, linePoints, onSheet, pieces, sheetCount, sheetRects, sheetTop, Smoother, SHEET_GAP, straightLine, totalHeight, widthAt } from '../ui/ink.js';

const pen = (over = {}) => ({ color: { r: 0, g: 0, b: 0, a: 255 }, width: 2, style: 'Solid', pressure_sensitive: true, ...over });
const stroke = (points, over) => ({ id: 's', pen: pen(over), points });
/** A horizontal line from x0 to x1 at y, a point every `step`. */
const hline = (x0, x1, y = 0, step = 1) => Array.from({ length: Math.floor((x1 - x0) / step) + 1 }, (_, i) => [x0 + i * step, y, 0.5]);
const close = (a, b, eps = 1e-6) => assert.ok(Math.abs(a - b) <= eps, `${a} ≉ ${b}`);

test('sheets stack top to bottom with a gap between them', () => {
  const page = { width: 794, height: 1123, sheets: 3 };
  const rects = sheetRects(page);
  assert.equal(sheetCount(page), 3);
  assert.deepEqual(rects[1], { x: 0, y: 1123 + SHEET_GAP, w: 794, h: 1123 });
  assert.equal(totalHeight(page), 3 * 1123 + 2 * SHEET_GAP);
  assert.equal(sheetTop(page, 2), rects[2].y);
  assert.equal(sheetTop(page, 3), 3 * (1123 + SHEET_GAP)); // where the next one would go
  assert.ok(onSheet(page, 10, 10));
  assert.ok(!onSheet(page, 10, 1123 + SHEET_GAP / 2)); // in the gap
  assert.ok(!onSheet(page, 800, 10));
});

test('a page without a sheet count has one sheet', () => {
  assert.equal(sheetCount({ width: 100, height: 100 }), 1);
  assert.equal(sheetCount({ width: 100, height: 100, sheets: 0 }), 1);
});

test("a page on another PDF takes that PDF's page sizes", () => {
  const page = { width: 794, height: 1123, background: { sheets: [{ width: 600, height: 800 }, { width: 800, height: 600 }] } };
  assert.deepEqual(sheetRects(page).map((r) => [r.y, r.w, r.h]), [[0, 600, 800], [800 + SHEET_GAP, 800, 600]]);
});

test('pressure changes the width, but not for a highlighter or without pressure', () => {
  assert.equal(widthAt(pen(), 1), 2);
  assert.ok(widthAt(pen(), 0) < widthAt(pen(), 0.5));
  assert.ok(widthAt(pen(), 0) > 0);
  assert.ok(widthAt(pen({ style: 'Pencil' }), 0) < widthAt(pen(), 0)); // a pencil gets finer
  assert.equal(widthAt(pen({ style: 'Highlighter' }), 0), 2);
  assert.equal(widthAt(pen({ pressure_sensitive: false }), 0), 2);
  assert.equal(widthAt(pen(), 5), widthAt(pen(), 1)); // clamped
});

test('highlighter and pencil are see-through', () => {
  assert.equal(effectiveAlpha(pen()), 1);
  assert.ok(effectiveAlpha(pen({ style: 'Highlighter' })) < 0.5);
  assert.ok(effectiveAlpha(pen({ style: 'Pencil' })) < 1);
});

test('dashed and dotted strokes split into pieces along the line', () => {
  const line = hline(0, 100);
  assert.equal(pieces(stroke(line)).length, 1);
  const dashes = pieces(stroke(line, { style: 'Dashed' }));
  assert.ok(dashes.length > 3);
  // Each dash is `on` long (width 2: 3·2 + 6 = 12); the last may be cut short.
  for (const d of dashes.slice(0, -1)) close(d.pts.at(-1)[0] - d.pts[0][0], 12);
  const dots = pieces(stroke(line, { style: 'Dotted' }));
  assert.ok(dots.every((d) => d.pts.length === 1));
  close(dots[1].pts[0][0] - dots[0].pts[0][0], 7); // 2·2 + 3 apart
});

test('repeated points are dropped', () => {
  const [piece] = pieces(stroke([[0, 0, 0.5], [0, 0, 0.5], [10, 0, 0.5]]));
  assert.equal(piece.pts.length, 2);
});

test('bounds include half the pen width', () => {
  assert.deepEqual(bounds(stroke(hline(10, 20, 5), { width: 4 })), { minX: 7, minY: 2, maxX: 23, maxY: 8 });
});

test('hit reaches the radius plus half the pen width', () => {
  const s = stroke(hline(0, 100), { width: 2 });
  assert.ok(hit(s, 50, 0, 0));
  assert.ok(hit(s, 50, 3.9, 3)); // 3 + 1
  assert.ok(!hit(s, 50, 4.1, 3));
  assert.ok(hit(s, 103.9, 0, 3)); // round the end
  assert.ok(!hit(s, 105, 0, 3));
  const dot = stroke([[0, 0, 0.5]]);
  assert.ok(hit(dot, 2, 0, 1));
  assert.ok(!hit(dot, 3, 0, 1));
});

test('the eraser cuts a stroke in two at its edge', () => {
  const s = stroke(hline(0, 100, 0, 10), { width: 2 });
  assert.equal(cutStroke(s, 50, 20, 5), null); // misses
  const parts = cutStroke(s, 50, 0, 5);
  assert.equal(parts.length, 2);
  const [left, right] = parts;
  assert.deepEqual(left[0], [0, 0, 0.5]);
  assert.deepEqual(right.at(-1), [100, 0, 0.5]);
  close(left.at(-1)[0], 44); // 50 - (5 + 1)
  close(right[0][0], 56);
  // The cut ends keep the pressure along the line.
  close(left.at(-1)[2], 0.5);
});

test('erasing an end leaves one piece, erasing it all leaves none', () => {
  const s = stroke(hline(0, 100, 0, 10), { width: 2 });
  const [rest, ...more] = cutStroke(s, 0, 0, 10);
  assert.equal(more.length, 0);
  close(rest[0][0], 11);
  assert.deepEqual(cutStroke(stroke(hline(0, 10, 0, 5)), 5, 0, 50), []);
});

test('crumbs left by the eraser are dropped', () => {
  // Cuts at 0.2 and 9.8 would leave slivers of 0.2 at either end.
  assert.deepEqual(cutStroke(stroke(hline(0, 10, 0, 1), { width: 0 }), 5, 0, 4.8), []);
});

test('smoothing evens out jitter but keeps the ends', () => {
  const sm = new Smoother(0, 0.1, 3);
  const raw = Array.from({ length: 60 }, (_, i) => [i * 2, i % 2 ? 1 : -1, 0.5]);
  for (const p of raw) sm.push(p);
  assert.deepEqual(sm.points[0], raw[0]);
  assert.deepEqual(sm.points.at(-1), raw.at(-1));
  const middle = sm.points.slice(15, 45);
  assert.ok(middle.every(([, y]) => Math.abs(y) < 0.2), 'the zigzag is ironed out');
});

test('smoothing drops samples closer than the minimum distance', () => {
  const sm = new Smoother(0, 1, 0);
  for (const p of [[0, 0, 0.5], [0.2, 0, 0.5], [0.4, 0, 0.5], [5, 0, 0.5]]) sm.push(p);
  assert.equal(sm.points.length, 2);
});

test('smoothed points before `settled` no longer change', () => {
  const sm = new Smoother(0.3, 0.1, 4);
  let settled = [];
  for (let i = 0; i < 80; i++) {
    sm.push([i * 1.5, Math.sin(i / 3) * 5, 0.5]);
    assert.deepEqual(sm.points.slice(0, settled.length), settled);
    settled = sm.points.slice(0, sm.settled).map((p) => [...p]);
  }
  assert.ok(sm.settled > 50);
});

test('a nearly straight stroke straightens to its end', () => {
  const wobbly = Array.from({ length: 50 }, (_, i) => [i * 4, 100 + Math.sin(i) * 3, i % 2 ? 0.4 : 0.6]);
  const line = straightLine(wobbly, [196, 101, 0.5], 24);
  assert.deepEqual(line[0], [0, 100, 0.5]);
  close(line.at(-1)[0], Math.hypot(196, 1)); // levelled, same length
  assert.ok(line.every(([, y, p]) => y === 100 && Math.abs(p - 0.5) < 1e-9));
});

test('curves and short strokes stay as drawn', () => {
  const curve = Array.from({ length: 50 }, (_, i) => [i * 4, 100 + (i - 25) ** 2 / 10, 0.5]);
  assert.equal(straightLine(curve, [196, 156, 0.5], 24), null);
  assert.equal(straightLine(hline(0, 10), [10, 0, 0.5], 24), null);
  // Out and back along a line is still straight: it becomes the line from
  // the start to where the pen stopped.
  const back = [...hline(0, 100), ...hline(0, 100).reverse()];
  assert.equal(straightLine(back, [50, 0, 0.5], 24).at(-1)[0], 50);
});

test('lines snap level or upright only when close', () => {
  const end = (to) => linePoints([0, 0], to, 0.5).at(-1);
  const deg = (a) => (a * Math.PI) / 180;
  const at = (a) => [100 * Math.cos(deg(a)), 100 * Math.sin(deg(a))];
  const same = ([x, y], [u, v]) => (close(x, u), close(y, v));
  for (const [a, want] of [[2, 0], [-2, 0], [88, 90], [181, 180], [-91, -90]]) same(end(at(a)), at(want));
  same(end(at(10)), at(10)); // a slope stays
  assert.equal(linePoints([0, 0], [100, 0], 0.5).length, 26); // a point every 4 units
  assert.equal(linePoints([0, 0], [0, 0], 0.5).length, 2);
});
