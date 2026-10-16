// Renders a whole page (paper, pictures, ink, text) into a PNG, the same way
// the screen draws it.

import { PAPER_COLOR, drawPaper, effectiveColor, strokePath } from './ink.js';
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

/** Returns the page as a base64 PNG (no data: prefix). */
export async function pagePng(page, scale = 2) {
  const c = document.createElement('canvas');
  c.width = Math.ceil(page.width * scale);
  c.height = Math.ceil(page.height * scale);
  const ctx = c.getContext('2d');
  ctx.fillStyle = PAPER_COLOR;
  ctx.fillRect(0, 0, c.width, c.height);
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  drawPaper(ctx, page, { minX: 0, minY: 0, maxX: page.width, maxY: page.height }, scale);
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
  return c.toDataURL('image/png').split(',')[1];
}
