// The PDF under a page drawn on another PDF (a worksheet, a scan), rendered
// with pdf.js. Sheet i of the page is page i + 1 of the PDF, as shown (pdf.js
// applies the crop box and rotation, like crates/coolmus-pdf/src/background.rs).

import * as pdfjs from './vendor/pdfjs/pdf.min.mjs';

const vendor = (path) => new URL('./vendor/pdfjs/' + path, import.meta.url).href;
pdfjs.GlobalWorkerOptions.workerSrc = vendor('pdf.worker.min.mjs');
const OPTIONS = {
  cMapUrl: vendor('cmaps/'),
  cMapPacked: true,
  standardFontDataUrl: vendor('standard_fonts/'),
  wasmUrl: vendor('wasm/'),
  iccUrl: vendor('iccs/'),
  isEvalSupported: false,
};
/** Page units are 96 DPI, PDF units 72. */
const PX_PER_PT = 96 / 72;
/** Rendering resolutions (device pixels per page unit), so zooming re-renders only now and then. */
const SCALES = [0.5, 1, 1.5, 2, 3, 4];

/** page → { pdf: Promise<PDFDocumentProxy>, sheets: Map<i, { scale, canvas }>, jobs: Map<key, Promise> } */
const loaded = new WeakMap();

/** Gives `page` its background PDF (the bytes from the page_background command). */
export function attach(page, bytes) {
  loaded.set(page, { pdf: pdfjs.getDocument({ data: bytes, ...OPTIONS }).promise, sheets: new Map(), jobs: new Map() });
}

export const hasBackground = (page) => loaded.has(page);

/** The smallest rendering scale that is at least `want`. */
export const scaleFor = (want) => SCALES.find((s) => s >= want) ?? SCALES[SCALES.length - 1];

/** Renders sheet `i` at `scale` unless a sharp enough rendering exists; resolves with its canvas. */
export function render(page, i, scale) {
  const bg = loaded.get(page);
  if (!bg) return Promise.resolve(null);
  const have = bg.sheets.get(i);
  if (have && have.scale >= scale) return Promise.resolve(have.canvas);
  const key = `${i}@${scale}`;
  if (!bg.jobs.has(key)) {
    const job = (async () => {
      const pdfPage = await (await bg.pdf).getPage(i + 1);
      const viewport = pdfPage.getViewport({ scale: scale * PX_PER_PT });
      const canvas = document.createElement('canvas');
      canvas.width = Math.ceil(viewport.width);
      canvas.height = Math.ceil(viewport.height);
      await pdfPage.render({ canvas, viewport }).promise;
      const now = bg.sheets.get(i);
      if (!now || now.scale < scale) bg.sheets.set(i, { scale, canvas });
      return canvas;
    })();
    job.catch((e) => console.warn('background page', i + 1, e)).finally(() => bg.jobs.delete(key));
    bg.jobs.set(key, job);
  }
  return bg.jobs.get(key);
}

/**
 * The best rendering of sheet `i` there is now (null if none yet), starting a
 * sharper one if it is below `scale`; `onReady` is called when that is done.
 */
export function sheetImage(page, i, scale, onReady) {
  const bg = loaded.get(page);
  if (!bg) return null;
  const have = bg.sheets.get(i);
  if (!have || have.scale < scale) render(page, i, scale).then((c) => c && onReady(), () => {});
  return have?.canvas ?? null;
}
