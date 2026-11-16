// Drawing surface. Layers, bottom to top:
//  * base  — paper, pictures, committed ink; redrawn only when the view or page changes;
//  * live  — the stroke being drawn (low-latency, desynchronized context), the
//            eraser outline, and the selection frame;
//  * text  — text boxes as HTML, so Hebrew/English layout and editing are native.
// Input uses Pointer Events with coalesced samples, so fast pen movement keeps
// every point and its pressure.
//
// The canvas never changes the page itself: every edit is reported through
// onCommit({ strokes?, texts?, images? }) with the new arrays, and the app
// applies it (which is what makes undo uniform).

import * as bg from './background.js';
import { PAPER_COLOR, bounds, cutStroke, drawPaper, effectiveAlpha, effectiveColor, hit, onSheet, sheetRects, SHEET_GAP, Smoother, strokePath, totalHeight, widthScale } from './ink.js';
import { t } from './i18n.js';
import { isPalm, onPenNear } from './palm.js';

const HANDLE = 16; // resize handle size, screen px
const ADD_SHEET_ROOM = 64; // below the last sheet, for its add button, screen px
const PICK = 6; // pick tolerance, screen px
const DOUBLE_TAP_MS = 450;

const uid = () => crypto.randomUUID();
const intersects = (a, b) => a.minX <= b.maxX && a.maxX >= b.minX && a.minY <= b.maxY && a.maxY >= b.minY;
const inside = (r, x, y, pad = 0) => x >= r.minX - pad && x <= r.maxX + pad && y >= r.minY - pad && y <= r.maxY + pad;
const union = (a, b) => (a ? { minX: Math.min(a.minX, b.minX), minY: Math.min(a.minY, b.minY), maxX: Math.max(a.maxX, b.maxX), maxY: Math.max(a.maxY, b.maxY) } : b);

export class SketchCanvas {
  constructor(container, { onCommit, onZoom, onSelection, onSheetChange, onAddSheet }) {
    this.container = container;
    this.onCommit = onCommit;
    this.onZoom = onZoom || (() => {});
    this.onSheetChange = onSheetChange || (() => {});
    this.onSelection = onSelection || (() => {});
    this.onAddSheet = onAddSheet;
    this.base = document.createElement('canvas');
    this.live = document.createElement('canvas');
    this.base.className = 'layer';
    this.live.className = 'layer live';
    this.textLayer = document.createElement('div');
    this.textLayer.className = 'textLayer';
    this.selbar = document.createElement('div');
    this.selbar.className = 'selbar';
    this.selbar.hidden = true;
    // Below the last sheet: adds one (pages drawn on a PDF have that PDF's sheets).
    this.addBtn = document.createElement('button');
    this.addBtn.className = 'addSheetBtn';
    this.addBtn.hidden = true;
    this.addBtn.addEventListener('click', () => this.onAddSheet?.());
    container.append(this.base, this.live, this.textLayer, this.selbar, this.addBtn);
    this.bctx = this.base.getContext('2d', { alpha: false });
    this.lctx = this.live.getContext('2d', { desynchronized: true }) || this.live.getContext('2d');

    this.page = null;
    this.zoom = 1;
    this.ox = 24;
    this.oy = 24;
    this.tool = 'pen';
    this.pen = null;
    this.smoothing = 0.35;
    this.touchDraws = false;
    this.eraserRadius = 10;
    this.textSize = 20;

    this.paths = new WeakMap(); // stroke object → { path, b, color }
    this.pictures = new Map(); // image src → HTMLImageElement
    this.textEls = new Map(); // text id → element
    this.textsRef = null;
    this.selection = new Set(); // ids of selected strokes, texts and images
    this.drag = null; // move/resize preview: { dx, dy, s, ax, ay, width? }
    this.marquee = null;
    this.editing = null; // text box being edited
    this.gesture = null;
    this.pointers = new Map();
    this.spaceDown = false;
    this.hover = null;
    this.lastTap = null;
    this.fitPending = true;
    this.frame = 0;
    this.fullPage = false;

    new ResizeObserver(() => this.resize()).observe(container);
    this.bindInput();
    onPenNear(() => this.rejectTouch());
  }

  // ---------- public API ----------

  /** `view` (from view()) restores a saved zoom and position instead of fitting. */
  setPage(page, view = null) {
    this.endTextEdit();
    this.page = page;
    this.selection.clear();
    this.drag = null;
    this.gesture = null;
    this.textsRef = null;
    this.fitPending = !view;
    if (view) {
      this.zoom = Math.min(8, Math.max(0.1, view.zoom));
      this.ox = -view.x * this.zoom;
      this.oy = -view.y * this.zoom;
      this.onZoom(this.zoom);
    }
    this.resize();
    this.selectionChanged();
  }

  /** Zoom and the page point at the view's top-left corner, for setPage. */
  view() {
    const r = (v, d) => Math.round(v * d) / d;
    return { zoom: r(this.zoom, 1e4), x: r(-this.ox / this.zoom, 100), y: r(-this.oy / this.zoom, 100) };
  }

  /** Call after the page's arrays were replaced (edit, undo, redo). */
  invalidate() {
    if (this.page) {
      const ids = new Set([...this.page.strokes, ...this.page.texts, ...this.page.images].map((x) => x.id));
      for (const id of this.selection) if (!ids.has(id)) this.selection.delete(id);
    }
    this.requestRender();
    this.selectionChanged();
  }

  fit() {
    const w = this.container.clientWidth;
    if (!this.page || w < 50) return;
    if (this.fullPage) {
      this.zoom = Math.min(2, Math.max(0.1, w / this.page.width));
      this.ox = 0;
      this.oy = 0;
    } else {
      this.zoom = Math.min(2, Math.max(0.1, (w - 48) / this.page.width));
      this.ox = Math.max(24, (w - this.page.width * this.zoom) / 2);
      this.oy = 24;
    }
    this.onZoom(this.zoom);
    this.requestRender();
  }

  zoomBy(factor, ax = this.container.clientWidth / 2, ay = this.container.clientHeight / 2) {
    const z = Math.min(8, Math.max(0.1, this.zoom * factor));
    const f = z / this.zoom;
    this.ox = ax - (ax - this.ox) * f;
    this.oy = ay - (ay - this.oy) * f;
    this.zoom = z;
    this.onZoom(z);
    this.requestRender();
  }

  /**
   * Keeps the sheets in view: no scrolling above the first sheet's top or
   * past the last one's bottom, and sheets narrower than the view stay
   * centered instead of drifting sideways.
   */
  clampView() {
    const w = this.container.clientWidth, h = this.container.clientHeight;
    const m = this.fullPage ? 0 : 24;
    const below = m + (this.canAddSheet() ? ADD_SHEET_ROOM : 0); // room for the add button
    const W = this.page.width * this.zoom, H = totalHeight(this.page) * this.zoom;
    const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
    this.ox = W + 2 * m <= w ? (w - W) / 2 : clamp(this.ox, w - W - m, m);
    this.oy = H + m + below <= h ? m : clamp(this.oy, h - H - below, m);
  }

  /** The PDF a page is drawn on, sheet by sheet, sharp at the current zoom. */
  drawBackground(ctx, sheets, view, dpr) {
    const scale = bg.scaleFor(this.zoom * dpr);
    sheets.forEach((r, i) => {
      if (r.y > view.maxY || r.y + r.h < view.minY) return;
      const img = bg.sheetImage(this.page, i, scale, () => this.requestRender());
      if (img) ctx.drawImage(img, r.x, r.y, r.w, r.h);
    });
  }

  /** Tells onSheetChange(index, count) about the sheet under the view's middle, when it changes. */
  reportSheet(sheets) {
    const mid = (this.container.clientHeight / 2 - this.oy) / this.zoom;
    let i = sheets.findIndex((r) => mid < r.y + r.h + SHEET_GAP / 2);
    if (i < 0) i = sheets.length - 1;
    const key = i + '/' + sheets.length;
    if (key === this.sheetKey) return;
    this.sheetKey = key;
    this.onSheetChange(i, sheets.length);
  }

  canAddSheet() {
    return !!(this.onAddSheet && this.page && !this.page.background);
  }

  /**
   * Glides down to a sheet just added, stopping with the gap above it near
   * the top of the view, so the end of the sheet before stays in sight.
   */
  revealSheet(i) {
    const r = sheetRects(this.page)[i];
    const from = this.oy, to = this.container.clientHeight * 0.15 - (r.y - SHEET_GAP / 2) * this.zoom;
    const start = performance.now();
    const step = (now) => {
      const k = Math.min(1, (now - start) / 350);
      this.oy = from + (to - from) * (1 - (1 - k) ** 3);
      this.requestRender();
      if (k < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  placeAddButton() {
    const b = this.addBtn;
    b.hidden = !this.canAddSheet();
    if (b.hidden) return;
    const label = t('addSheet');
    if (b.dataset.label !== label) {
      b.dataset.label = label;
      b.innerHTML = `<svg viewBox="0 0 24 24"><path d="M6 3h8l4 4v14H6z"/><path d="M12 10v7M8.5 13.5h7"/></svg><span>${label}</span>`;
    }
    const last = sheetRects(this.page).at(-1);
    b.style.left = this.ox + (last.x + last.w / 2) * this.zoom + 'px';
    b.style.top = this.oy + (last.y + last.h) * this.zoom + 16 + 'px';
  }

  /** Scrolls so sheet `i` starts at the top of the view. */
  showSheet(i) {
    this.oy = (this.fullPage ? 0 : 24) - sheetRects(this.page)[i].y * this.zoom;
    this.requestRender();
  }

  setFullPage(on) {
    this.fullPage = on;
    this.fitPending = true; // re-fit once the layout has settled at its new size
    requestAnimationFrame(() => this.resize());
  }

  setTool(tool) {
    if (tool === this.tool) return;
    this.endTextEdit();
    this.tool = tool;
    if (tool !== 'select' && this.selection.size) {
      this.selection.clear();
      this.selectionChanged();
    }
    this.requestRender();
  }

  isBusy() {
    return !!this.editing || (!!this.gesture && this.gesture.kind !== 'pan' && this.gesture.kind !== 'pinch');
  }

  /** Center of the visible part of the page, in page units. */
  viewCenter() {
    const w = this.container.clientWidth, h = this.container.clientHeight;
    return { x: (w / 2 - this.ox) / this.zoom, y: (Math.min(h, (this.page ? totalHeight(this.page) : h) * this.zoom + this.oy) / 2 - this.oy) / this.zoom };
  }

  /** Adds a picture centered at `at` (page units), selected. */
  insertImage(src, width, height, at = this.viewCenter()) {
    const max = Math.min(this.page.width * 0.7, 640);
    const s = Math.min(1, max / width, max / height);
    const w = Math.round(width * s), h = Math.round(height * s);
    const img = { id: uid(), x: Math.max(0, Math.round(at.x - w / 2)), y: Math.max(0, Math.round(at.y - h / 2)), width: w, height: h, src };
    this.onCommit({ images: [...this.page.images, img] });
    this.selection = new Set([img.id]);
    this.selectionChanged();
  }

  /** Starts a text box with `text` at `at` (e.g. pasted text). */
  insertText(text, at = this.viewCenter()) {
    const box = { id: uid(), x: Math.round(at.x - 150), y: Math.round(at.y), width: 300, text, size: this.textSize, color: this.pen.color };
    this.onCommit({ texts: [...this.page.texts, box] });
  }

  selectAll() {
    if (!this.page) return;
    this.selection = new Set([...this.page.strokes, ...this.page.texts, ...this.page.images].map((x) => x.id));
    this.requestRender();
    this.selectionChanged();
  }

  clearSelection() {
    if (!this.selection.size) return;
    this.selection.clear();
    this.requestRender();
    this.selectionChanged();
  }

  /** What is selected, by kind. */
  selected() {
    const p = this.page;
    const sel = (arr) => (p ? arr.filter((x) => this.selection.has(x.id)) : []);
    return p ? { strokes: sel(p.strokes), texts: sel(p.texts), images: sel(p.images) } : { strokes: [], texts: [], images: [] };
  }

  deleteSelection() {
    if (!this.selection.size) return;
    const keep = (arr) => arr.filter((x) => !this.selection.has(x.id));
    const p = this.page;
    // Filter before clearing: `keep` reads the selection.
    const change = { strokes: keep(p.strokes), texts: keep(p.texts), images: keep(p.images) };
    this.selection.clear();
    this.onCommit(change);
  }

  duplicateSelection() {
    if (!this.selection.size) return;
    const p = this.page;
    const fresh = new Set();
    const copy = (x) => {
      const id = uid();
      fresh.add(id);
      return { ...x, id };
    };
    const move = transformer({ dx: 24, dy: 24, s: 1, ax: 0, ay: 0 });
    const { strokes, texts, images } = this.selected();
    this.onCommit({
      strokes: [...p.strokes, ...strokes.map((s) => copy(move.stroke(s)))],
      texts: [...p.texts, ...texts.map((x) => copy(move.text(x)))],
      images: [...p.images, ...images.map((x) => copy(move.image(x)))],
    });
    this.selection = fresh;
    this.selectionChanged();
  }

  /** Applies pen properties (color, width, style) and/or a text size to the selection. */
  /** `width` is the size shown in the toolbar (see widthScale). */
  restyleSelection({ color, width, style, size }) {
    const { strokes, texts } = this.selected();
    if (!strokes.length && !texts.length) return false;
    const change = {};
    if (strokes.length && (color || width || style)) {
      change.strokes = this.page.strokes.map((s) =>
        this.selection.has(s.id)
          ? { ...s, pen: { ...s.pen, ...(color && { color }), ...(width && { width: width * widthScale(style || s.pen.style) }), ...(style && { style }) } }
          : s,
      );
    }
    if (texts.length && (color || size)) {
      change.texts = this.page.texts.map((x) => (this.selection.has(x.id) ? { ...x, ...(color && { color }), ...(size && { size }) } : x));
    }
    if (Object.keys(change).length) this.onCommit(change);
    return true;
  }

  // ---------- text editing ----------

  editText(id) {
    const box = this.page.texts.find((x) => x.id === id);
    if (box) this.beginTextEdit({ ...box });
  }

  beginTextEdit(box) {
    this.endTextEdit();
    this.clearSelection();
    const ta = document.createElement('textarea');
    ta.className = 'textEdit';
    ta.value = box.text ?? '';
    // The text's own direction once there is some; until then the
    // interface's, so the caret waits at the click (the right edge in Hebrew).
    const direction = () => (ta.dir = ta.value.trim() ? 'auto' : document.documentElement.dir || 'ltr');
    direction();
    ta.addEventListener('input', direction);
    ta.spellcheck = false;
    Object.assign(ta.style, {
      left: box.x + 'px', top: box.y + 'px', width: box.width + 'px',
      fontSize: box.size + 'px', color: colorCss(box.color),
    });
    const grow = () => {
      ta.style.height = 'auto';
      ta.style.height = ta.scrollHeight + 2 + 'px';
    };
    ta.addEventListener('input', grow);
    // Using the toolbar (a color, the text size) keeps the box open: those apply to
    // it. WebKit doesn't focus a clicked slider, so the app also flags such presses.
    ta.addEventListener('blur', (e) => this.holdEdit || e.relatedTarget?.closest?.('#toolbar, #menu') || this.endTextEdit());
    ta.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        this.endTextEdit();
      }
      e.stopPropagation();
    });
    ta.addEventListener('pointerdown', (e) => e.stopPropagation());
    this.textLayer.append(ta);
    const grip = this.textGrip();
    this.textLayer.append(grip);
    this.editing = { box, ta, grip };
    this.placeGrip();
    this.selectionChanged(); // the toolbar shows this text's color and size
    this.requestRender();
    grow();
    ta.focus();
  }

  /**
   * The handle for moving the text box being typed in: dragging it moves the
   * box while the typing goes on.
   */
  textGrip() {
    const grip = document.createElement('div');
    grip.className = 'textGrip';
    grip.title = t('moveText');
    grip.innerHTML = ICONS.move;
    // Keep the focus (and the caret) in the text box.
    grip.addEventListener('mousedown', (e) => e.preventDefault());
    grip.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      e.stopPropagation();
      const ed = this.editing;
      if (!ed) return;
      grip.setPointerCapture(e.pointerId);
      const start = { x: e.clientX, y: e.clientY, bx: ed.box.x, by: ed.box.y };
      const move = (ev) => {
        ed.box = { ...ed.box, x: Math.round(start.bx + (ev.clientX - start.x) / this.zoom), y: Math.round(start.by + (ev.clientY - start.y) / this.zoom) };
        Object.assign(ed.ta.style, { left: ed.box.x + 'px', top: ed.box.y + 'px' });
        this.placeGrip();
      };
      const up = () => {
        grip.removeEventListener('pointermove', move);
        ed.ta.focus();
      };
      grip.addEventListener('pointermove', move);
      grip.addEventListener('pointerup', up, { once: true });
      grip.addEventListener('pointercancel', up, { once: true });
    });
    return grip;
  }

  /**
   * Puts the move handle just outside the open box's top corner (left in a
   * left-to-right interface, right in Hebrew), the same size at any zoom.
   */
  placeGrip() {
    const ed = this.editing;
    if (!ed) return;
    const size = 24 / this.zoom;
    const rtl = document.documentElement.dir === 'rtl';
    Object.assign(ed.grip.style, {
      width: size + 'px',
      height: size + 'px',
      left: (rtl ? ed.box.x + ed.box.width + 2 / this.zoom : ed.box.x - size - 2 / this.zoom) + 'px',
      top: ed.box.y - size - 2 / this.zoom + 'px',
    });
  }

  /**
   * Applies a color or size to the text box being typed in, and gives it the
   * focus back unless `focus` is false (e.g. while a slider is still moving).
   * False if no box is open.
   */
  restyleEditing({ color, size }, { focus = true } = {}) {
    const e = this.editing;
    if (!e || (!color && !size)) return false;
    e.box = { ...e.box, ...(color && { color }), ...(size && { size }) };
    Object.assign(e.ta.style, { color: colorCss(e.box.color), fontSize: e.box.size + 'px' });
    e.ta.dispatchEvent(new Event('input')); // re-fit its height
    if (focus) e.ta.focus();
    this.selectionChanged();
    return true;
  }

  endTextEdit() {
    const e = this.editing;
    if (!e) return;
    this.editing = null; // first: removing the focused element fires blur again
    const text = e.ta.value.replace(/\s+$/, '');
    e.ta.remove();
    e.grip.remove();
    const p = this.page;
    const { box } = e;
    if (box.id) {
      const old = p.texts.find((x) => x.id === box.id);
      if (!text) this.onCommit({ texts: p.texts.filter((x) => x.id !== box.id) });
      else if (old && (old.text !== text || colorCss(old.color) !== colorCss(box.color) || old.size !== box.size || old.x !== box.x || old.y !== box.y)) {
        this.onCommit({ texts: p.texts.map((x) => (x.id === box.id ? { ...x, text, color: box.color, size: box.size, x: box.x, y: box.y } : x)) });
      }
    } else if (text) {
      this.onCommit({ texts: [...p.texts, { ...box, id: uid(), text }] });
    }
    this.textsRef = null;
    this.requestRender();
    this.selectionChanged(); // back to the pen's color and size
  }

  // ---------- rendering ----------

  resize() {
    const dpr = window.devicePixelRatio || 1;
    const w = this.container.clientWidth, h = this.container.clientHeight;
    for (const c of [this.base, this.live]) {
      c.width = Math.max(1, Math.round(w * dpr));
      c.height = Math.max(1, Math.round(h * dpr));
      c.style.width = w + 'px';
      c.style.height = h + 'px';
    }
    if (this.fitPending && this.page && w > 50) {
      this.fitPending = false;
      this.fit();
    }
    this.render();
  }

  requestRender() {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.render();
    });
  }

  entry(stroke) {
    let e = this.paths.get(stroke);
    if (!e) {
      e = { path: strokePath(stroke), b: bounds(stroke), color: effectiveColor(stroke.pen) };
      this.paths.set(stroke, e);
    }
    return e;
  }

  picture(src) {
    let img = this.pictures.get(src);
    if (!img) {
      img = new Image();
      img.onload = () => this.requestRender();
      img.src = src;
      this.pictures.set(src, img);
    }
    return img;
  }

  render() {
    const ctx = this.bctx;
    const dpr = window.devicePixelRatio || 1;
    const w = this.base.width / dpr, h = this.base.height / dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = getComputedStyle(this.container).getPropertyValue('--canvas-bg') || '#202124';
    ctx.fillRect(0, 0, w, h);
    const page = this.page;
    if (!page) {
      this.textLayer.replaceChildren();
      this.textEls.clear();
      this.addBtn.hidden = true;
      return;
    }

    this.clampView();
    const sheets = sheetRects(page);
    this.reportSheet(sheets);
    ctx.save();
    if (!this.fullPage) {
      ctx.shadowColor = 'rgba(0,0,0,0.28)';
      ctx.shadowBlur = 14;
      ctx.shadowOffsetY = 2;
    }
    ctx.fillStyle = page.background ? '#fff' : PAPER_COLOR;
    for (const r of sheets) ctx.fillRect(this.ox + r.x * this.zoom, this.oy + r.y * this.zoom, r.w * this.zoom, r.h * this.zoom);
    ctx.restore();

    ctx.setTransform(dpr * this.zoom, 0, 0, dpr * this.zoom, dpr * this.ox, dpr * this.oy);
    const view = { minX: -this.ox / this.zoom, minY: -this.oy / this.zoom, maxX: (w - this.ox) / this.zoom, maxY: (h - this.oy) / this.zoom };
    drawPaper(ctx, page, view, this.zoom);
    if (page.background) this.drawBackground(ctx, sheets, view, dpr);
    // Content shows only on the sheets, cut at their edges like in the PDF.
    ctx.save();
    ctx.clip(this.sheetClip(sheets));

    const dragged = (id) => this.drag && this.selection.has(id);
    for (const im of page.images) {
      const img = this.picture(im.src);
      if (!img.complete || !img.naturalWidth) continue;
      ctx.save();
      if (dragged(im.id)) applyDrag(ctx, this.drag);
      ctx.drawImage(img, im.x, im.y, im.width, im.height);
      ctx.restore();
    }
    if (this.pictures.size > page.images.length + 8) {
      const used = new Set(page.images.map((x) => x.src));
      for (const src of this.pictures.keys()) if (!used.has(src)) this.pictures.delete(src);
    }

    // Highlighters underneath, then everything else, each in original order.
    for (const pass of [true, false]) {
      for (const s of page.strokes) {
        if ((s.pen.style === 'Highlighter') !== pass) continue;
        const e = this.entry(s);
        const moving = dragged(s.id);
        if (!moving && !intersects(e.b, view)) continue;
        ctx.fillStyle = e.color;
        if (moving) {
          ctx.save();
          applyDrag(ctx, this.drag);
          ctx.fill(e.path);
          ctx.restore();
        } else ctx.fill(e.path);
      }
    }
    ctx.restore();

    this.syncTexts();
    this.placeAddButton();
    this.drawTop();
  }

  sheetClip(sheets = sheetRects(this.page)) {
    const clip = new Path2D();
    for (const r of sheets) clip.rect(r.x, r.y, r.w, r.h);
    return clip;
  }

  syncTexts() {
    const L = this.textLayer;
    L.style.transform = `translate(${this.ox}px, ${this.oy}px) scale(${this.zoom})`;
    this.placeGrip(); // keeps its size on screen when zooming
    const texts = this.page.texts;
    if (texts !== this.textsRef) {
      this.textsRef = texts;
      const live = new Set();
      for (const box of texts) {
        live.add(box.id);
        let el = this.textEls.get(box.id);
        if (!el) {
          el = document.createElement('div');
          el.className = 'textBox';
          el.dir = 'auto';
          this.textEls.set(box.id, el);
          L.append(el);
        }
        if (el.textContent !== box.text) el.textContent = box.text;
        Object.assign(el.style, { left: box.x + 'px', top: box.y + 'px', width: box.width + 'px', fontSize: box.size + 'px', color: colorCss(box.color) });
      }
      for (const [id, el] of this.textEls) if (!live.has(id)) (el.remove(), this.textEls.delete(id));
    }
    for (const [id, el] of this.textEls) {
      el.hidden = this.editing?.box.id === id;
      el.classList.toggle('selected', this.selection.has(id));
      const d = this.drag && this.selection.has(id) ? this.drag : null;
      if (d && d.width != null) {
        el.style.transform = '';
        el.style.width = d.width + 'px';
      } else {
        el.style.transform = d ? cssDrag(d, this.page.texts.find((x) => x.id === id)) : '';
        if (!d) el.style.width = (this.page.texts.find((x) => x.id === id)?.width ?? 0) + 'px';
      }
    }
  }

  /** Bounds of an item in page units. */
  boundsOf(item, kind) {
    if (kind === 'stroke') return this.entry(item).b;
    if (kind === 'image') return { minX: item.x, minY: item.y, maxX: item.x + item.width, maxY: item.y + item.height };
    const el = this.textEls.get(item.id);
    const h = el ? el.offsetHeight : item.size * 1.3;
    return { minX: item.x, minY: item.y, maxX: item.x + item.width, maxY: item.y + h };
  }

  selectionBounds() {
    if (!this.selection.size || !this.page) return null;
    const { strokes, texts, images } = this.selected();
    let r = null;
    for (const s of strokes) r = union(r, this.boundsOf(s, 'stroke'));
    for (const x of texts) r = union(r, this.boundsOf(x, 'text'));
    for (const x of images) r = union(r, this.boundsOf(x, 'image'));
    return r;
  }

  /** Selection bounds as previewed during a move/resize. */
  draggedBounds() {
    const b = this.selectionBounds();
    const d = this.drag;
    if (!b || !d) return b;
    if (d.width != null) return { ...b, maxX: b.minX + d.width };
    const f = (x, y) => [d.ax + (x - d.ax) * d.s + d.dx, d.ay + (y - d.ay) * d.s + d.dy];
    const [x0, y0] = f(b.minX, b.minY), [x1, y1] = f(b.maxX, b.maxY);
    return { minX: x0, minY: y0, maxX: x1, maxY: y1 };
  }

  clearLive() {
    const c = this.lctx;
    this.live.style.opacity = '';
    c.setTransform(1, 0, 0, 1, 0, 0);
    c.clearRect(0, 0, this.live.width, this.live.height);
  }

  /**
   * The stroke in progress, drawn so a long stroke costs no more per sample
   * than a short one: the part the smoother has settled is filled once into
   * an offscreen buffer, and each frame only adds the tail on top of it. Both
   * are opaque and the live layer takes the ink's opacity, so a translucent
   * pen doesn't darken where the pieces overlap. Dashed and dotted strokes
   * are redrawn whole, since their pattern runs from the start.
   */
  drawLiveStroke(g, dpr) {
    const c = this.lctx;
    const { pen, points } = g.stroke;
    const toPage = (ctx) => {
      ctx.setTransform(dpr * this.zoom, 0, 0, dpr * this.zoom, dpr * this.ox, dpr * this.oy);
      ctx.clip(this.sheetClip());
    };
    const fill = (ctx, from, to) => {
      const part = { pen, points: points.slice(from, to) };
      ctx.fill(strokePath(part));
      if (from > 0 && pen.style === 'Highlighter') {
        // a highlighter has square ends; round the joint so no notch shows
        const [x, y] = points[from];
        ctx.beginPath();
        ctx.arc(x, y, pen.width / 2, 0, Math.PI * 2);
        ctx.fill();
      }
    };
    const color = `rgb(${pen.color.r},${pen.color.g},${pen.color.b})`;
    this.live.style.opacity = effectiveAlpha(pen);
    let from = 0;
    if (pen.style !== 'Dashed' && pen.style !== 'Dotted') {
      if (!g.buffer) {
        g.buffer = new OffscreenCanvas(this.live.width, this.live.height);
        g.bctx = g.buffer.getContext('2d');
        toPage(g.bctx);
        g.bctx.fillStyle = color;
        g.frozen = 0;
      }
      const settled = g.smoother.settled;
      if (settled - 1 > g.frozen) {
        fill(g.bctx, g.frozen, settled);
        g.frozen = settled - 1;
      }
      from = g.frozen;
      c.setTransform(1, 0, 0, 1, 0, 0);
      c.drawImage(g.buffer, 0, 0);
    }
    c.save();
    toPage(c);
    c.fillStyle = color;
    fill(c, from, points.length);
    c.restore();
  }

  /** Redraws the live layer: stroke in progress, eraser outline, or selection frame. */
  drawTop() {
    const dpr = window.devicePixelRatio || 1;
    const c = this.lctx;
    const g = this.gesture;
    this.clearLive();
    if (g && g.kind === 'draw') return this.drawLiveStroke(g, dpr);
    c.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (this.tool === 'eraser' && this.hover) {
      c.strokeStyle = 'rgba(120,120,120,0.9)';
      c.lineWidth = 1;
      c.beginPath();
      c.arc(this.hover.x, this.hover.y, this.eraserRadius * this.zoom, 0, Math.PI * 2);
      c.stroke();
    }
    const scr = (r) => ({ x: this.ox + r.minX * this.zoom, y: this.oy + r.minY * this.zoom, w: (r.maxX - r.minX) * this.zoom, h: (r.maxY - r.minY) * this.zoom });
    if (this.marquee) {
      const m = this.marquee;
      const r = scr({ minX: Math.min(m.x0, m.x1), minY: Math.min(m.y0, m.y1), maxX: Math.max(m.x0, m.x1), maxY: Math.max(m.y0, m.y1) });
      c.fillStyle = 'rgba(26,115,232,0.08)';
      c.strokeStyle = 'rgba(26,115,232,0.9)';
      c.setLineDash([5, 4]);
      c.lineWidth = 1;
      c.fillRect(r.x, r.y, r.w, r.h);
      c.strokeRect(r.x + 0.5, r.y + 0.5, r.w, r.h);
      c.setLineDash([]);
    }
    const b = this.draggedBounds();
    if (b) {
      const r = scr(b);
      const pad = 4;
      c.strokeStyle = 'rgba(26,115,232,0.95)';
      c.lineWidth = 1.5;
      c.setLineDash([6, 4]);
      c.strokeRect(r.x - pad, r.y - pad, r.w + pad * 2, r.h + pad * 2);
      c.setLineDash([]);
      c.fillStyle = '#ffffff';
      c.fillRect(r.x + r.w + pad - HANDLE / 2, r.y + r.h + pad - HANDLE / 2, HANDLE, HANDLE);
      c.strokeRect(r.x + r.w + pad - HANDLE / 2, r.y + r.h + pad - HANDLE / 2, HANDLE, HANDLE);
    }
    this.placeSelbar(b);
  }

  // ---------- selection bar ----------

  selectionChanged() {
    const sel = this.selected();
    this.onSelection(sel);
    const bar = this.selbar;
    if (!this.selection.size) {
      bar.hidden = true;
      return;
    }
    const button = (icon, title, action, cls = '') => {
      const b = document.createElement('button');
      b.className = 'icon ' + cls;
      b.title = title;
      b.innerHTML = icon;
      b.addEventListener('pointerdown', (e) => e.stopPropagation());
      b.addEventListener('click', (e) => {
        e.stopPropagation();
        action();
      });
      return b;
    };
    const items = [
      button(ICONS.duplicate, t('duplicate'), () => this.duplicateSelection()),
      button(ICONS.trash, t('delete'), () => this.deleteSelection(), 'danger'),
    ];
    if (sel.texts.length === 1 && this.selection.size === 1) items.unshift(button(ICONS.edit, t('editText'), () => this.editText(sel.texts[0].id)));
    bar.replaceChildren(...items);
    this.requestRender();
  }

  placeSelbar(b) {
    const bar = this.selbar;
    if (!b || !this.selection.size || this.drag || this.marquee || this.editing) {
      bar.hidden = true;
      return;
    }
    bar.hidden = false;
    const x = this.ox + ((b.minX + b.maxX) / 2) * this.zoom;
    const top = this.oy + b.minY * this.zoom - 12;
    const below = top - bar.offsetHeight < 4;
    bar.style.left = Math.round(x - bar.offsetWidth / 2) + 'px';
    bar.style.top = Math.round(below ? this.oy + b.maxY * this.zoom + 14 : top - bar.offsetHeight) + 'px';
  }

  // ---------- picking ----------

  /** Topmost item at (x, y) in page units: text, then ink, then pictures. */
  pick(x, y) {
    const p = this.page;
    const tol = PICK / this.zoom;
    for (let i = p.texts.length - 1; i >= 0; i--) if (inside(this.boundsOf(p.texts[i], 'text'), x, y, tol)) return p.texts[i].id;
    for (let i = p.strokes.length - 1; i >= 0; i--) {
      const s = p.strokes[i];
      if (inside(this.entry(s).b, x, y, tol) && hit(s, x, y, tol)) return s.id;
    }
    for (let i = p.images.length - 1; i >= 0; i--) if (inside(this.boundsOf(p.images[i], 'image'), x, y)) return p.images[i].id;
    return null;
  }

  /** Items touched by a marquee rectangle (page units). */
  inRect(r) {
    const p = this.page;
    const ids = [];
    for (const s of p.strokes) if (intersects(this.entry(s).b, r) && s.points.some(([x, y]) => inside(r, x, y))) ids.push(s.id);
    for (const x of p.texts) if (intersects(this.boundsOf(x, 'text'), r)) ids.push(x.id);
    for (const x of p.images) if (intersects(this.boundsOf(x, 'image'), r)) ids.push(x.id);
    return ids;
  }

  // ---------- input ----------

  local(e) {
    const r = this.live.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  toPage(e) {
    const { x, y } = this.local(e);
    let p = 0.6;
    if (e.pointerType === 'pen') p = e.pressure > 0 ? e.pressure : 0.05;
    return [(x - this.ox) / this.zoom, (y - this.oy) / this.zoom, p];
  }

  bindInput() {
    const el = this.live;
    el.addEventListener('pointerdown', (e) => this.down(e));
    el.addEventListener('pointermove', (e) => this.move(e));
    el.addEventListener('pointerup', (e) => this.up(e, false));
    el.addEventListener('pointercancel', (e) => this.up(e, true));
    el.addEventListener('pointerleave', () => {
      this.hover = null;
      if (!this.gesture) this.drawTop();
    });
    el.addEventListener('contextmenu', (e) => e.preventDefault());
    el.addEventListener(
      'wheel',
      (e) => {
        e.preventDefault();
        const { x, y } = this.local(e);
        if (e.ctrlKey || e.metaKey) this.zoomBy(Math.exp(-e.deltaY * 0.01), x, y);
        else {
          this.ox -= e.shiftKey ? e.deltaY : e.deltaX;
          this.oy -= e.shiftKey ? 0 : e.deltaY;
          this.requestRender();
        }
      },
      { passive: false },
    );
    const typing = () => /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName);
    window.addEventListener('keydown', (e) => {
      if (e.code === 'Space' && !typing()) {
        this.spaceDown = true;
        e.preventDefault();
      }
    });
    window.addEventListener('keyup', (e) => {
      if (e.code === 'Space') this.spaceDown = false;
    });
  }

  down(e) {
    if (!this.page) return;
    if (isPalm(e)) return;
    try {
      this.live.setPointerCapture(e.pointerId);
    } catch {
      /* synthetic or already-released pointer */
    }
    this.pointers.set(e.pointerId, { ...this.local(e), type: e.pointerType });

    const touches = [...this.pointers.values()].filter((p) => p.type === 'touch');
    if (e.pointerType === 'touch' && touches.length >= 2) {
      if (this.gesture && this.gesture.kind !== 'pan') this.cancelGesture();
      this.startPinch();
      return;
    }
    if (this.gesture) {
      // A new press from the same pointer means we never saw its release
      // (e.g. focus moved to a native popup); drop the stale gesture.
      if (this.gesture.id !== e.pointerId) return;
      this.cancelGesture();
    }

    const panning = e.button === 1 || this.spaceDown || this.tool === 'pan' || (e.pointerType === 'touch' && !this.touchDraws);
    const erasing = this.tool === 'eraser' || (e.pointerType === 'pen' && (e.buttons & 32) !== 0);
    if (panning) {
      this.gesture = { kind: 'pan', id: e.pointerId, last: this.local(e), touchView: e.pointerType === 'touch' ? this.viewNow() : null };
      this.live.style.cursor = 'grabbing';
    } else if (this.editing) {
      // A tap outside the text box being edited finishes it.
      this.endTextEdit();
    } else if (erasing) {
      // Erasing works on the page's ink and text as it goes; the result is
      // one undoable change when the drag ends.
      this.gesture = { kind: 'erase', id: e.pointerId, before: { strokes: this.page.strokes, texts: this.page.texts }, last: null };
      this.eraseAt(e);
    } else if (this.tool === 'select') {
      this.selectDown(e);
    } else if (this.tool === 'text') {
      const [x, y] = this.toPage(e);
      const id = this.pick(x, y);
      if (id && this.page.texts.some((b) => b.id === id)) this.editText(id);
      else if (onSheet(this.page, x, y)) {
        // The click is where writing starts: the box's left edge, or its
        // right edge in a right-to-left (Hebrew) interface.
        const sheet = sheetRects(this.page).find((r) => y >= r.y && y <= r.y + r.h);
        const rtl = document.documentElement.dir === 'rtl';
        const room = rtl ? x - sheet.x - 16 : sheet.x + sheet.w - x - 16;
        const width = Math.max(120, Math.min(320, room));
        const left = rtl ? Math.max(sheet.x, x - width) : x;
        this.beginTextEdit({ id: null, x: Math.round(left), y: Math.round(y - this.textSize * 0.65), width, text: '', size: this.textSize, color: this.pen.color });
      }
      e.preventDefault(); // keep focus in the new text box
    } else if ((e.button === 0 || e.pointerType !== 'mouse') && onSheet(this.page, ...this.toPage(e))) {
      const smoother = new Smoother(this.smoothing, 0.4 / this.zoom, (8 * this.smoothing) / this.zoom);
      this.gesture = { kind: 'draw', id: e.pointerId, smoother, stroke: { id: uid(), pen: structuredClone(this.pen), points: smoother.points } };
      this.addPoint(this.toPage(e));
      this.drawTop();
    }
  }

  selectDown(e) {
    const [x, y] = this.toPage(e);
    const s = this.local(e);
    const b = this.selectionBounds();
    const start = { x, y };
    if (b) {
      const hx = this.ox + b.maxX * this.zoom + 4, hy = this.oy + b.maxY * this.zoom + 4;
      if (Math.abs(s.x - hx) <= HANDLE && Math.abs(s.y - hy) <= HANDLE) {
        const sel = this.selected();
        const textOnly = sel.texts.length === 1 && this.selection.size === 1;
        this.gesture = { kind: 'resize', id: e.pointerId, start, b, textWidth: textOnly ? sel.texts[0].width : null };
        return;
      }
    }
    const id = this.pick(x, y);
    const now = performance.now();
    const doubleTap = id && this.lastTap && this.lastTap.id === id && now - this.lastTap.time < DOUBLE_TAP_MS;
    this.lastTap = id ? { id, time: now } : null;
    if (doubleTap && this.page.texts.some((t) => t.id === id)) {
      this.editText(id);
      return;
    }
    if (b && inside(b, x, y, PICK / this.zoom) && (!id || this.selection.has(id))) {
      this.gesture = { kind: 'move', id: e.pointerId, start };
    } else if (id) {
      if (e.shiftKey || e.ctrlKey || e.metaKey) {
        if (this.selection.has(id)) this.selection.delete(id);
        else this.selection.add(id);
      } else this.selection = new Set([id]);
      this.selectionChanged();
      this.gesture = { kind: 'move', id: e.pointerId, start };
    } else {
      if (!(e.shiftKey || e.ctrlKey || e.metaKey)) this.clearSelection();
      this.gesture = { kind: 'marquee', id: e.pointerId };
      this.marquee = { x0: x, y0: y, x1: x, y1: y };
    }
  }

  addPoint(p, g = this.gesture) {
    g.smoother.push(p);
  }

  move(e) {
    if (isPalm(e)) return;
    if (this.pointers.has(e.pointerId)) this.pointers.set(e.pointerId, { ...this.local(e), type: e.pointerType });
    const g = this.gesture;
    if (!g) {
      this.hover = this.local(e);
      if (this.tool === 'eraser') this.drawTop();
      return;
    }
    if (g.kind === 'pinch') return this.updatePinch();
    if (g.id !== e.pointerId) return;
    const samples = e.getCoalescedEvents ? e.getCoalescedEvents() : [e];
    if (g.kind === 'draw') {
      for (const s of samples.length ? samples : [e]) this.addPoint(this.toPage(s));
      this.drawTop();
    } else if (g.kind === 'erase') {
      for (const s of samples.length ? samples : [e]) this.eraseAt(s);
      this.drawTop(); // the eraser circle stands in for the hidden cursor
    } else if (g.kind === 'pan') {
      const p = this.local(e);
      this.ox += p.x - g.last.x;
      this.oy += p.y - g.last.y;
      g.last = p;
      this.requestRender();
    } else if (g.kind === 'move') {
      const [x, y] = this.toPage(e);
      this.drag = { dx: x - g.start.x, dy: y - g.start.y, s: 1, ax: 0, ay: 0 };
      this.requestRender();
    } else if (g.kind === 'resize') {
      const [x, y] = this.toPage(e);
      if (g.textWidth != null) {
        this.drag = { dx: 0, dy: 0, s: 1, ax: 0, ay: 0, width: Math.max(40, g.textWidth + x - g.start.x) };
      } else {
        const { minX: ax, minY: ay, maxX, maxY } = g.b;
        const s = Math.max(0.05, (x - ax + (y - ay)) / Math.max(1, maxX - ax + (maxY - ay)));
        this.drag = { dx: 0, dy: 0, s, ax, ay };
      }
      this.requestRender();
    } else if (g.kind === 'marquee') {
      const [x, y] = this.toPage(e);
      this.marquee.x1 = x;
      this.marquee.y1 = y;
      this.drawTop();
    }
  }

  up(e, cancelled) {
    this.pointers.delete(e.pointerId);
    const g = this.gesture;
    if (!g) return;
    if (g.kind === 'pinch') {
      if ([...this.pointers.values()].filter((p) => p.type === 'touch').length < 2) this.gesture = null;
      return;
    }
    if (g.id !== e.pointerId) return;
    this.gesture = null;
    this.live.style.cursor = '';
    if (cancelled) return this.cancelGesture(g);
    if (g.kind === 'draw') {
      this.addPoint(this.toPage(e), g);
      if (g.stroke.points.length) this.onCommit({ strokes: [...this.page.strokes, g.stroke] });
      this.requestRender();
    } else if (g.kind === 'erase') {
      const { strokes, texts } = this.page;
      Object.assign(this.page, g.before);
      const change = {};
      if (strokes !== g.before.strokes) change.strokes = strokes;
      if (texts !== g.before.texts) change.texts = texts;
      if (Object.keys(change).length) this.onCommit(change);
      this.requestRender();
    } else if (g.kind === 'move' || g.kind === 'resize') {
      const d = this.drag;
      this.drag = null;
      const moved = d && (d.width != null || d.s !== 1 || Math.hypot(d.dx, d.dy) * this.zoom > 2);
      if (moved) this.commitTransform(d);
      this.requestRender();
    } else if (g.kind === 'marquee') {
      const m = this.marquee;
      this.marquee = null;
      const r = { minX: Math.min(m.x0, m.x1), minY: Math.min(m.y0, m.y1), maxX: Math.max(m.x0, m.x1), maxY: Math.max(m.y0, m.y1) };
      if ((r.maxX - r.minX) * this.zoom > 3 || (r.maxY - r.minY) * this.zoom > 3) for (const id of this.inRect(r)) this.selection.add(id);
      this.selectionChanged();
      this.requestRender();
    }
  }

  cancelGesture(g = this.gesture) {
    if (g?.kind === 'erase') Object.assign(this.page, g.before);
    this.gesture = null;
    this.drag = null;
    this.marquee = null;
    this.requestRender();
  }

  commitTransform(d) {
    const p = this.page;
    const sel = this.selection;
    if (d.width != null) {
      this.onCommit({ texts: p.texts.map((x) => (sel.has(x.id) ? { ...x, width: Math.round(d.width) } : x)) });
      return;
    }
    const f = transformer(d);
    const change = {};
    const { strokes, texts, images } = this.selected();
    if (strokes.length) change.strokes = p.strokes.map((s) => (sel.has(s.id) ? f.stroke(s) : s));
    if (texts.length) change.texts = p.texts.map((x) => (sel.has(x.id) ? f.text(x) : x));
    if (images.length) change.images = p.images.map((x) => (sel.has(x.id) ? f.image(x) : x));
    this.onCommit(change);
  }

  /**
   * Erases under the eraser at `e`: cuts away the ink it passes over and
   * removes any text box it touches. Fast moves are filled in, so no ink is
   * skipped between two samples.
   */
  eraseAt(e) {
    const [x, y] = this.toPage(e);
    const r = this.eraserRadius;
    const g = this.gesture;
    const from = g.last ?? [x, y];
    const steps = Math.max(1, Math.ceil(Math.hypot(x - from[0], y - from[1]) / (r * 0.5)));
    for (let i = 1; i <= steps; i++) this.eraseCircle(from[0] + ((x - from[0]) * i) / steps, from[1] + ((y - from[1]) * i) / steps, r);
    g.last = [x, y];
    this.hover = this.local(e);
  }

  /** True if a circle at (x, y) touches the text itself, not just the box around it. */
  touchesText(box, x, y, r) {
    const g = this.gesture;
    g.textRects ??= new Map();
    if (!g.textRects.has(box.id)) g.textRects.set(box.id, this.textLines(box));
    return g.textRects.get(box.id).some((b) => {
      const dx = x - Math.max(b.minX, Math.min(x, b.maxX)), dy = y - Math.max(b.minY, Math.min(y, b.maxY));
      return dx * dx + dy * dy <= r * r;
    });
  }

  /** The rectangles of a text box's laid-out lines, in page units (its box if it isn't shown). */
  textLines(box) {
    const el = this.textEls.get(box.id);
    if (!el || el.hidden) return [this.boundsOf(box, 'text')];
    const range = document.createRange();
    range.selectNodeContents(el);
    const c = this.container.getBoundingClientRect();
    const page = (sx, sy) => [(sx - c.left - this.ox) / this.zoom, (sy - c.top - this.oy) / this.zoom];
    return [...range.getClientRects()]
      .filter((q) => q.width > 0.5)
      .map((q) => {
        const [minX, minY] = page(q.left, q.top), [maxX, maxY] = page(q.right, q.bottom);
        return { minX, minY, maxX, maxY };
      });
  }

  eraseCircle(x, y, r) {
    const p = this.page;
    let cut = false;
    const strokes = [];
    for (const s of p.strokes) {
      const b = this.entry(s).b;
      const near = x >= b.minX - r && x <= b.maxX + r && y >= b.minY - r && y <= b.maxY + r;
      const pieces = near ? cutStroke(s, x, y, r) : null;
      if (!pieces) {
        strokes.push(s);
        continue;
      }
      cut = true;
      for (const points of pieces) strokes.push({ id: uid(), pen: s.pen, points });
    }
    if (cut) p.strokes = strokes;
    const texts = p.texts.filter((t) => !this.touchesText(t, x, y, r));
    const removed = texts.length !== p.texts.length;
    if (removed) p.texts = texts;
    if (cut || removed) this.requestRender();
  }


  startPinch() {
    const [a, b] = [...this.pointers.values()].filter((p) => p.type === 'touch');
    const touchView = this.gesture?.touchView ?? this.viewNow();
    this.gesture = { kind: 'pinch', touchView, dist: Math.hypot(a.x - b.x, a.y - b.y), mid: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 } };
    this.drawTop();
  }

  viewNow() {
    return { zoom: this.zoom, ox: this.ox, oy: this.oy };
  }

  /**
   * The pen came near while touch was panning or pinching: that touch was
   * the palm landing first, so the view goes back to where it was and the
   * touch is dropped.
   */
  rejectTouch() {
    const v = this.gesture?.touchView;
    if (!v) return;
    for (const [id, p] of this.pointers) if (p.type === 'touch') this.pointers.delete(id);
    this.gesture = null;
    this.live.style.cursor = '';
    Object.assign(this, v);
    this.onZoom(this.zoom);
    this.requestRender();
  }

  updatePinch() {
    const ts = [...this.pointers.values()].filter((p) => p.type === 'touch');
    if (ts.length < 2) return;
    const [a, b] = ts;
    const g = this.gesture;
    const dist = Math.hypot(a.x - b.x, a.y - b.y);
    const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
    this.ox += mid.x - g.mid.x;
    this.oy += mid.y - g.mid.y;
    if (g.dist > 10) this.zoomBy(dist / g.dist, mid.x, mid.y);
    g.dist = dist;
    g.mid = mid;
    this.requestRender();
  }
}

/** Point mapping for a move/resize: scale `s` around (ax, ay), then shift by (dx, dy). */
function transformer({ dx, dy, s, ax, ay }) {
  const X = (x) => Math.round((ax + (x - ax) * s + dx) * 100) / 100;
  const Y = (y) => Math.round((ay + (y - ay) * s + dy) * 100) / 100;
  return {
    stroke: (st) => ({ ...st, pen: s === 1 ? st.pen : { ...st.pen, width: Math.round(st.pen.width * s * 10) / 10 }, points: st.points.map(([x, y, p]) => [X(x), Y(y), p]) }),
    text: (b) => ({ ...b, x: X(b.x), y: Y(b.y), width: Math.round(b.width * s), size: Math.round(b.size * s * 10) / 10 }),
    image: (im) => ({ ...im, x: X(im.x), y: Y(im.y), width: Math.round(im.width * s), height: Math.round(im.height * s) }),
  };
}

function applyDrag(ctx, d) {
  ctx.translate(d.dx, d.dy);
  if (d.s !== 1) {
    ctx.translate(d.ax, d.ay);
    ctx.scale(d.s, d.s);
    ctx.translate(-d.ax, -d.ay);
  }
}

/** The same move/resize for a text element positioned at (box.x, box.y). */
function cssDrag(d, box) {
  if (!box) return '';
  const nx = d.ax + (box.x - d.ax) * d.s + d.dx, ny = d.ay + (box.y - d.ay) * d.s + d.dy;
  return `translate(${nx - box.x}px, ${ny - box.y}px) scale(${d.s})`;
}

export function colorCss(c) {
  return `rgba(${c.r},${c.g},${c.b},${c.a / 255})`;
}

const ICONS = {
  trash: '<svg viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/></svg>',
  duplicate: '<svg viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 00-1-1H5a1 1 0 00-1 1v10a1 1 0 001 1h3"/></svg>',
  edit: '<svg viewBox="0 0 24 24"><path d="M4 20h4L19 9l-4-4L4 16v4z"/></svg>',
  move: '<svg viewBox="0 0 24 24"><path d="M12 3v18M3 12h18M12 3l-3 3M12 3l3 3M12 21l-3-3M12 21l3-3M3 12l3-3M3 12l3 3M21 12l-3-3M21 12l-3 3"/></svg>',
};
