// Renders pages (paper, pictures, ink, text) the same way the screen draws
// them: as one PNG, or as sheet images for printing.

import { PAPER_COLOR, drawPaper, effectiveColor, sheetRects, strokePath, totalHeight } from './ink.js';
import { colorCss } from './canvas.js';

const FONT = getComputedStyle(document.documentElement).fontFamily || 'sans-serif';
const RTL_FIRST = /^[^A-Za-zÀ-ɏ]*[֐-׿؀-ۿ]/;

const load = (src) =>
  new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = src;
  });

/** Splits a paragraph into lines that fit `width`. */
function wrap(ctx, text, width) {
  const lines = [];
  for (const para of text.split('\n')) {
    let line = '';
    for (const word of para.split(/(\s+)/)) {
      const next = line + word;
      if (line && ctx.measureText(next.trimEnd()).width > width) {
        lines.push(line.trimEnd());
        line = word.trimStart();
      } else line = next;
    }
    lines.push(line);
  }
  return lines;
}

function drawText(ctx, box) {
  ctx.font = `${box.size}px ${FONT}`;
  ctx.fillStyle = colorCss(box.color);
  ctx.textBaseline = 'top';
  const lh = box.size * 1.3;
  let y = box.y + (lh - box.size) / 2;
  for (const para of box.text.split('\n')) {
    const rtl = RTL_FIRST.test(para);
    ctx.direction = rtl ? 'rtl' : 'ltr';
    ctx.textAlign = rtl ? 'right' : 'left';
    const x = rtl ? box.x + box.width : box.x;
    for (const line of wrap(ctx, para, box.width)) {
      ctx.fillText(line, x, y);
      y += lh;
    }
  }
}

/** Draws the page's sheets and content between y and y + h into a new canvas. */
async function renderRegion(page, y, h, scale) {
  const c = document.createElement('canvas');
  c.width = Math.ceil(page.width * scale);
  c.height = Math.ceil(h * scale);
  const ctx = c.getContext('2d');
  ctx.setTransform(scale, 0, 0, scale, 0, -y * scale);
  const sheets = sheetRects(page);
  const clip = new Path2D();
  ctx.fillStyle = '#d5d7db'; // the gaps between sheets
  ctx.fillRect(0, y, page.width, h);
  ctx.fillStyle = PAPER_COLOR;
  for (const r of sheets) {
    ctx.fillRect(r.x, r.y, r.w, r.h);
    clip.rect(r.x, r.y, r.w, r.h);
  }
  drawPaper(ctx, page, { minX: 0, minY: y, maxX: page.width, maxY: y + h }, scale);
  ctx.clip(clip);
  for (const im of page.images) {
    const img = await load(im.src);
    if (img) ctx.drawImage(img, im.x, im.y, im.width, im.height);
  }
  for (const pass of [true, false]) {
    for (const s of page.strokes) {
      if ((s.pen.style === 'Highlighter') !== pass) continue;
      ctx.fillStyle = effectiveColor(s.pen);
      ctx.fill(strokePath(s));
    }
  }
  for (const box of page.texts) drawText(ctx, box);
  return c;
}

/** Returns the page as a base64 PNG (no data: prefix). */
export async function pagePng(page, scale = 2) {
  const c = await renderRegion(page, 0, totalHeight(page), scale);
  return c.toDataURL('image/png').split(',')[1];
}

/**
 * The page as printed sheets: one canvas per sheet, and sheets taller than
 * A4's proportions (long OneNote imports) cut into A4-shaped pieces.
 */
export async function printSheets(page, scale = 3) {
  const out = [];
  const a4 = page.width * Math.SQRT2;
  for (const r of sheetRects(page)) {
    const n = Math.max(1, Math.ceil(r.h / a4 - 0.01));
    for (let i = 0; i < n; i++) {
      const y = r.y + i * a4;
      out.push(await renderRegion(page, y, Math.min(a4, r.y + r.h - y), scale));
    }
  }
  return out;
}
