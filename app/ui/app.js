// Coolmus UI. Pages are PDF files in the user's folders, read and written by
// the Rust backend; this module keeps the open page and its undo history,
// autosaves, and wires the toolbar and the folder tree.

import { setLanguage, t, translateDom, getLanguage } from './i18n.js';
import { SketchCanvas, colorCss } from './canvas.js';
import { bounds, sheetCount, sheetTop, SHEET_GAP, widthScale } from './ink.js';
import { $, el, bdi, error, modal, prompt, confirmDialog, showMenu, showPanel, hideMenu } from './ui.js';
import { renderTree, allFolders, findNode, parentOf, baseName, isInside, isFlat, bindTreeSpace } from './tree.js';
import { pagePng, printSheets } from './export.js';
import * as bg from './background.js';
import { fromBlob, pickFile, takePhoto } from './media.js';
import { editLabel, labelText } from './labels.js';
import { bindPalmRejection } from './palm.js';

const { invoke } = window.__TAURI__.core;

const PALETTE = ['#1a1a1a', '#5f6368', '#d93025', '#f28b00', '#f9d400', '#1e8e3e', '#009688', '#1a73e8', '#7b1fa2', '#e91e63'];
const STYLES = ['Solid', 'Dashed', 'Dotted', 'Highlighter', 'Pencil'];
/** A marker is this many times wider than the pen it was switched from. */

const hslHex = (h, s, l) => {
  const f = (n) => {
    const k = (n + h / 30) % 12;
    const c = l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255).toString(16).padStart(2, '0');
  };
  return `#${f(0)}${f(8)}${f(4)}`;
};
/** The color panel's grid: greys from black to white, then 12 hues from dark to light. */
const COLOR_GRID = [
  ...Array.from({ length: 12 }, (_, i) => hslHex(0, 0, i / 11)),
  ...[0.25, 0.35, 0.45, 0.55, 0.65, 0.75, 0.85].flatMap((l) => Array.from({ length: 12 }, (_, i) => hslHex(i * 30, 0.8, l))),
];
const RECENT_COLORS_KEY = 'coolmus.recentColors';
const RECENT_COLORS = 5;
const PAPERS = ['Blank', 'Lined', 'Grid', 'Dotted'];

// A sample line in the given style, width and color, drawn on a strip of paper.
function styleSvg(style, width, color) {
  const w = Math.max(1, Math.min(width, 12));
  let attrs = `stroke="${color}" stroke-width="${w}"`;
  if (style === 'Dashed') attrs += ` stroke-dasharray="${2.5 * w + 2} ${1.5 * w + 3}" stroke-linecap="butt"`;
  if (style === 'Dotted') attrs += ` stroke-dasharray="0.01 ${w + 4}"`;
  if (style === 'Highlighter') attrs = `stroke="${color}" stroke-width="${Math.max(w * 2, 8)}" stroke-opacity="0.4" stroke-linecap="butt"`;
  if (style === 'Pencil') attrs += ' stroke-opacity="0.7"';
  return `<svg viewBox="0 0 64 24" class="styleLine"><rect width="64" height="24" rx="4" class="paper"/><path d="M8 15C18 5 26 19 36 12S52 7 56 10" ${attrs}/></svg>`;
}
const icon = (paths) => `<svg viewBox="0 0 24 24" class="menuIcon">${paths}</svg>`;
const SHEET = '<path d="M6 3h12v18H6z"/>';
const ICONS = {
  paper_Blank: icon(SHEET),
  paper_Lined: icon(SHEET + '<path d="M9 9h6M9 13h6M9 17h6"/>'),
  paper_Grid: icon(SHEET + '<path d="M6 9h12M6 15h12M10 3v18M14 3v18"/>'),
  paper_Dotted: icon(SHEET + '<path d="M9.5 8.5h.01M14.5 8.5h.01M9.5 12h.01M14.5 12h.01M9.5 15.5h.01M14.5 15.5h.01" stroke-width="2.6"/>'),
  addSheet: icon('<path d="M6 3h8l4 4v14H6z"/><path d="M12 10v7M8.5 13.5h7"/>'),
  removeSheet: icon('<path d="M6 3h8l4 4v14H6z"/><path d="M8.5 13.5h7"/>'),
  clearPage: icon('<path d="M4 7h16M9 7V4h6v3M6.5 7l1 13h9l1-13M10 11v6M14 11v6"/>'),
  print: icon('<path d="M7 9V3h10v6"/><rect x="3" y="9" width="18" height="8" rx="2"/><path d="M7 14h10v7H7z"/>'),
  openInViewer: icon('<path d="M14 4h6v6M20 4l-9 9"/><path d="M18 14v5a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h5"/>'),
  revealFolder: icon('<path d="M3 6a1 1 0 011-1h5l2 2h9a1 1 0 011 1v10a1 1 0 01-1 1H4a1 1 0 01-1-1z"/>'),
  exportPng: icon('<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="9.5" r="1.6"/><path d="M4 19l5-5 4 4 3-3 4 4"/>'),
};
const OPEN_FOLDERS_KEY = 'coolmus.openFolders';
const SIDEBAR_KEY = 'coolmus.sidebarHidden';
const SIDEBAR_WIDTH_KEY = 'coolmus.sidebarWidth';

const state = {
  tree: [], // root nodes
  tags: [],
  settings: null,
  current: null, // { path, page, undo: [], redo: [], dirty, lastChange }
  folder: '', // selected folder: where new pages go
  reveal: null, // path to scroll to and flash on the next tree render
  treeSel: null, // last tree item clicked: target of Ctrl+C / Ctrl+X / Ctrl+V
  focus: 'page', // 'tree' or 'page': where the keyboard's copy/paste applies
  cut: [], // paths cut in Coolmus, moved by the next paste
  openFolders: new Set(),
  search: '',
  tool: 'pen',
  fullPage: false,
  busy: false,
  sheet: 0, // the sheet in view, from the canvas
  version: '', // Coolmus's own
  selection: { strokes: [], texts: [], images: [] },
};

let firstRun = true;
try {
  const saved = localStorage.getItem(OPEN_FOLDERS_KEY);
  firstRun = saved == null;
  state.openFolders = new Set(JSON.parse(saved || '[]'));
} catch {
  /* storage unavailable: start with only the roots open */
}

// ---------- helpers ----------

const hexToColor = (hex) => ({ r: parseInt(hex.slice(1, 3), 16), g: parseInt(hex.slice(3, 5), 16), b: parseInt(hex.slice(5, 7), 16), a: 255 });
const colorToHex = (c) => '#' + [c.r, c.g, c.b].map((v) => v.toString(16).padStart(2, '0')).join('');
const widthFromSlider = (v) => Math.round(0.5 * Math.pow(80, v / 100) * 10) / 10;
const sliderFromWidth = (w) => Math.round((100 * Math.log(w / 0.5)) / Math.log(80));
/** A pen's width as the toolbar shows it: a marker's is scaled down (widthScale). */
const shownWidth = (pen) => Math.round((pen.width / widthScale(pen.style)) * 10) / 10;
const typing = () => /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName) || $('dialog').open;

async function call(cmd, args, options) {
  try {
    return await invoke(cmd, args, options);
  } catch (e) {
    error(e);
    throw e;
  }
}

function setStatus(s) {
  $('statusText').textContent = s;
}

/** The names from the root down to `path`, with folder labels in place of names. */
function folderParts(path) {
  const root = state.tree.find((r) => isInside(path, r.path));
  if (!root) return [path];
  const parts = [];
  let at = root.path;
  for (const name of ['', ...path.slice(root.path.length).split('/').filter(Boolean)]) {
    if (name) at += '/' + name;
    const node = findNode(state.tree, at);
    parts.push(labelText(node?.label) || node?.name || name);
  }
  // A root shown flat isn't in the tree, so it isn't in the path either.
  return isFlat(state, root.path) && parts.length > 1 ? parts.slice(1) : parts;
}

/** "Root › sub › folder" for a folder path. */
const folderLabel = (path) => folderParts(path).join(' › ');

/** The same as nodes, each name isolated so the line keeps the UI's direction. */
const pathNodes = (parts) => parts.flatMap((p, i) => (i ? [' › ', bdi(p)] : [bdi(p)]));

const firstRoot = () => state.tree[0]?.path ?? '';

/** Path after `old` was renamed/moved to `nu` (for paths at or below `old`). */
function remap(path, old, nu) {
  if (path === old) return nu;
  if (path.startsWith(old + '/')) return nu + path.slice(old.length);
  return path;
}

function remapAll(old, nu) {
  if (state.current) state.current.path = remap(state.current.path, old, nu);
  state.folder = remap(state.folder, old, nu);
  if (state.treeSel) state.treeSel.path = remap(state.treeSel.path, old, nu);
  state.openFolders = new Set([...state.openFolders].map((p) => remap(p, old, nu)));
  saveOpenFolders();
  views = Object.fromEntries(Object.entries(views).map(([p, v]) => [remap(p, old, nu), v]));
  storeViews();
}

// Each page's zoom and scroll position, kept on this computer (not in the
// file: just looking at a page shouldn't rewrite it, and positions depend on
// the screen). Keyed by path, most recently used last.
const VIEWS_KEY = 'coolmus.views';
const MAX_VIEWS = 1000;
let views = {};
try {
  views = JSON.parse(localStorage.getItem(VIEWS_KEY) || '{}');
} catch {
  /* storage unavailable: pages open fitted */
}

function rememberView() {
  const c = state.current;
  if (!c) return;
  const v = canvas.view();
  const old = views[c.path];
  if (old && old.zoom === v.zoom && old.x === v.x && old.y === v.y) return;
  delete views[c.path];
  views[c.path] = v;
  const keys = Object.keys(views);
  for (const k of keys.slice(0, Math.max(0, keys.length - MAX_VIEWS))) delete views[k];
  storeViews();
}

function storeViews() {
  try {
    localStorage.setItem(VIEWS_KEY, JSON.stringify(views));
  } catch {
    /* not essential */
  }
}

function saveOpenFolders() {
  try {
    localStorage.setItem(OPEN_FOLDERS_KEY, JSON.stringify([...state.openFolders]));
  } catch {
    /* not essential */
  }
}

// ---------- canvas & editing ----------

const canvas = new SketchCanvas($('stage'), {
  onCommit: (change) => commit(change),
  onAddSheet: () => addSheet(),
  onZoom: (z) => ($('zoomFit').textContent = Math.round(z * 100) + '%'),
  onSheetChange: (i) => {
    state.sheet = i;
    renderStatus();
  },
  onSelection: (sel) => {
    state.selection = sel;
    renderToolbar();
  },
});

function changed() {
  const c = state.current;
  if (!c) return;
  c.dirty = true;
  c.lastChange = performance.now();
  c.page.modified = Date.now();
  renderStatus();
}

/** True if anything on the page reaches below `y`. */
function contentBelow(page, y) {
  return (
    page.strokes.some((s) => bounds(s).maxY > y) ||
    page.texts.some((b) => b.y + b.size * 1.3 > y) ||
    page.images.some((im) => im.y + im.height > y)
  );
}

function addSheet() {
  const c = state.current;
  if (!c) return;
  const n = sheetCount(c.page);
  commit({ sheets: n + 1 });
  canvas.revealSheet(n);
}

function removeLastSheet() {
  const c = state.current;
  const n = c ? sheetCount(c.page) : 1;
  if (n < 2) return;
  if (contentBelow(c.page, sheetTop(c.page, n - 1) - SHEET_GAP)) return error('lastPageNotEmpty');
  commit({ sheets: n - 1 });
}

/** Applies an edit from the canvas: new arrays for some of strokes/texts/images. */
function commit(change) {
  const c = state.current;
  if (!c) return;
  const before = {};
  for (const k of Object.keys(change)) {
    before[k] = c.page[k];
    c.page[k] = change[k];
  }
  c.undo.push({ before, after: change });
  if (c.undo.length > 500) c.undo.shift();
  c.redo = [];
  canvas.invalidate();
  changed();
  renderUndo();
}

function undo() {
  canvas.endTextEdit();
  const c = state.current;
  const e = c?.undo.pop();
  if (!e) return;
  Object.assign(c.page, e.before);
  c.redo.push(e);
  canvas.invalidate();
  changed();
  renderUndo();
}

function redo() {
  const c = state.current;
  const e = c?.redo.pop();
  if (!e) return;
  Object.assign(c.page, e.after);
  c.undo.push(e);
  canvas.invalidate();
  changed();
  renderUndo();
}

async function setFullPage(on) {
  state.fullPage = on;
  document.body.classList.toggle('full', on);
  $('fullBtn').classList.toggle('on', on);
  canvas.setFullPage(on);
  try {
    await window.__TAURI__.window?.getCurrentWindow().setFullscreen(on);
  } catch (e) {
    console.warn('fullscreen unavailable', e);
  }
}

function setSidebar(show) {
  document.body.classList.toggle('noSidebar', !show);
  $('sidebarBtn').classList.toggle('on', show);
  try {
    localStorage.setItem(SIDEBAR_KEY, show ? '' : '1');
  } catch {
    /* not essential */
  }
}

/** Dragging the sidebar's edge resizes it; double-click restores the default width. */
function bindSidebarResizer() {
  const bar = $('sidebarResizer'), side = $('sidebar');
  const store = (w) => {
    try {
      if (w) localStorage.setItem(SIDEBAR_WIDTH_KEY, w);
      else localStorage.removeItem(SIDEBAR_WIDTH_KEY);
    } catch {
      /* not essential */
    }
  };
  try {
    const w = localStorage.getItem(SIDEBAR_WIDTH_KEY);
    if (w) side.style.width = w;
  } catch {
    /* default width */
  }
  bar.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    bar.setPointerCapture(e.pointerId);
    bar.classList.add('drag');
    const r = side.getBoundingClientRect();
    const move = (ev) => {
      const w = document.dir === 'rtl' ? r.right - ev.clientX : ev.clientX - r.left;
      side.style.width = Math.round(w) + 'px';
    };
    const up = () => {
      bar.classList.remove('drag');
      bar.removeEventListener('pointermove', move);
      store(side.style.width);
    };
    bar.addEventListener('pointermove', move);
    bar.addEventListener('pointerup', up, { once: true });
    bar.addEventListener('pointercancel', up, { once: true });
  });
  bar.addEventListener('dblclick', () => {
    side.style.width = '';
    store(null);
  });
}

function setTool(tool) {
  state.tool = tool;
  canvas.setTool(tool);
  renderToolbar();
}

// ---------- persistence ----------

async function saveNow() {
  const c = state.current;
  if (!c || !c.dirty) return;
  c.dirty = false;
  try {
    await invoke('save_page', { path: c.path, page: c.page });
  } catch (e) {
    c.dirty = true;
    error(e);
  }
  renderStatus();
}

setInterval(() => {
  const c = state.current;
  if (c && !canvas.gesture) rememberView();
  if (c && c.dirty && !canvas.isBusy() && performance.now() - c.lastChange > 3000) saveNow().then(refreshTree);
}, 400);
// "today" becomes "yesterday" at midnight.
setInterval(renderStatus, 60000);
window.addEventListener('beforeunload', () => {
  rememberView();
  saveNow();
});
// The folder may change outside Coolmus (Explorer, OneDrive): rescan on focus.
window.addEventListener('focus', () => refreshTree());

function setTree(tr) {
  state.tree = tr.tree;
  state.tags = tr.tags;
  if (!state.tree.some((r) => isInside(state.folder, r.path))) state.folder = firstRoot();
}

let treeVersion = 0;

/** Shows a tree the backend returned after changing the roots; a refresh still on its way is dropped. */
function applyTree(tr) {
  treeVersion++;
  setTree(tr);
  renderSidebar();
}
async function refreshTree() {
  const v = ++treeVersion;
  const tr = await invoke('get_tree');
  if (v !== treeVersion) return;
  setTree(tr);
  renderSidebar();
  renderTagSuggestions();
}

const treeCtx = {
  state,
  open: (node) => openPage(node.path),
  newPage: (folder) => newPage(folder),
  newFolder: (parent) => newFolder(parent),
  rename: (node) => renameItem(node),
  move: (node) => moveDialog(node),
  moveTo: (path, folder) => moveItem(path, folder),
  remove: (node) => removeItem(node),
  editLabel: async (node) => {
    const label = await editLabel(node);
    if (label === undefined) return;
    await call('set_folder_label', { path: node.path, label });
    await refreshTree();
  },
  reveal: (path) => call('reveal', { path }),
  openExternal: (path) => call('open_external', { path }),
  copy: (node) => copyItems([node.path], false),
  cut: (node) => copyItems([node.path], true),
  paste: (folder) => pasteInto(folder),
  print: (node) => (node.kind === 'folder' || node.kind === 'root' ? printFolder(node) : printPages([node.path], node.name)),
  removeRoot: (node) => removeRoot(node),
  importOneNote: (folder) => importOneNote(folder),
  select: (node) => (state.treeSel = { path: node.path, kind: node.kind }),
  folderLabel,
  folderPath: (path) => pathNodes(folderParts(path)),
  dropFiles,
  toggleFlat,
  moveRoot,
  reorder: reorderItem,
  saveOpenFolders,
};

function renderSidebar() {
  renderTree(treeCtx);
}

/** Loads a page, with the PDF it is drawn on if it has one. */
async function loadPage(path) {
  const loaded = await call('load_page', { path });
  if (loaded.page.background) bg.attach(loaded.page, new Uint8Array(await call('page_background', { path })));
  return loaded;
}

async function openPage(path, { force = false } = {}) {
  if (!force && state.current?.path === path) return;
  canvas.endTextEdit();
  rememberView();
  await saveNow();
  const { page, changes } = await loadPage(path);
  // Changed in another program (e.g. annotated in Acrobat): saving from here
  // would drop those changes, so ask first.
  if (changes.length) {
    const choice = await modal((d, done) => {
      d.append(
        el('h3', { text: t('changedTitle') }),
        el('p', {}, t('changedText', baseName(path), changes.map((c) => t('change_' + c)).join(', '))),
        el('div', { class: 'buttons' },
          el('button', { text: t('openInViewer'), class: 'primary', onclick: () => done('viewer') }),
          el('button', { text: t('editAnyway'), class: 'danger', onclick: () => done('edit') }),
          el('button', { text: t('cancel'), onclick: () => done(null) }),
        ),
      );
    });
    if (choice === 'viewer') call('open_external', { path });
    if (choice !== 'edit') return;
  }
  state.current = { path, page, undo: [], redo: [], dirty: false, lastChange: 0 };
  state.folder = parentOf(path);
  // Reveal the opened page; the user may collapse its folders afterwards.
  for (let p = state.folder; state.tree.some((r) => isInside(p, r.path)); p = parentOf(p)) state.openFolders.add(p);
  saveOpenFolders();
  canvas.setPage(page, views[path]);
  renderPage();
  renderSidebar();
}

function closePage() {
  rememberView();
  state.current = null;
  canvas.setPage(null);
  renderPage();
}

/** Opens the most recently changed page, or creates a first one. */
async function openSomething() {
  const hits = await invoke('search', { query: '' });
  const page = hits.find((h) => h.kind === 'page');
  if (page) return openPage(page.path).catch(() => {});
  if (firstRoot()) return newPage(firstRoot());
}

function pagesIn(folder) {
  return (findNode(state.tree, folder)?.children ?? []).filter((n) => n.kind === 'page').length;
}

async function newPage(folder = state.folder) {
  canvas.endTextEdit();
  await saveNow();
  const paper = state.current?.page.paper ?? 'Blank';
  const path = await call('create_page', { folder, name: t('defaultPage', pagesIn(folder) + 1), paper });
  state.search = '';
  $('search').value = '';
  await refreshTree();
  state.reveal = path;
  await openPage(path);
  $('title').focus();
  $('title').select();
}

async function newFolder(parent = state.folder) {
  const name = await prompt(t('newFolderName'), t('defaultFolder'));
  if (name == null || !name.trim()) return;
  const path = await call('create_folder', { parent, name });
  state.openFolders.add(parent);
  state.folder = path;
  state.reveal = path;
  saveOpenFolders();
  await refreshTree();
}

async function renameItem(node) {
  const name = await prompt(t('rename'), node.name);
  if (name == null || !name.trim() || name.trim() === node.name) return;
  await renamePath(node.path, name);
}

async function renamePath(path, name) {
  await saveNow();
  const nu = await call('rename', { path, name });
  remapAll(path, nu);
  if (state.current) state.current.page.title = baseName(state.current.path);
  await refreshTree();
  renderPage();
  return nu;
}

async function moveItem(path, folder) {
  if (parentOf(path) === folder) return;
  await saveNow();
  const nu = await call('move_item', { path, folder });
  state.openFolders.add(folder);
  remapAll(path, nu);
  await refreshTree();
  renderStatus();
}

async function moveDialog(node) {
  const folders = allFolders(state.tree).filter((f) => !isInside(f.path, node.path));
  const target = await modal((d, done) => {
    d.append(
      el('h3', {}, t('moveItem', ''), bdi(node.name)),
      el('div', { class: 'list folderPick' },
        ...folders.map((f) =>
          el('button', { class: 'pageItem', style: `padding-inline-start:${8 + f.depth * 16}px`, disabled: f.path === parentOf(node.path), onclick: () => done(f.path) }, '📁 ', bdi(f.name)),
        ),
      ),
      el('div', { class: 'buttons' }, el('button', { text: t('cancel'), onclick: () => done(null) })),
    );
  });
  if (target != null) await moveItem(node.path, target);
}

async function removeItem(node) {
  const key = node.kind === 'folder' ? 'confirmDeleteFolder' : 'confirmDeletePage';
  if (!(await confirmDialog(t(key, node.name), t('deleteToTrash')))) return;
  const cur = state.current?.path;
  const closes = cur && (cur === node.path || cur.startsWith(node.path + '/'));
  if (closes) {
    canvas.endTextEdit();
    state.current.dirty = false; // the file is going to the trash
    closePage();
  }
  await call('delete_item', { path: node.path });
  if (isInside(state.folder, node.path)) state.folder = parentOf(node.path);
  await refreshTree();
  if (!state.current) await openSomething();
}

// ---------- files: clipboard, folders, printing ----------

/** Puts items on the system clipboard (pasteable in Explorer too); a cut moves them on the next paste in Coolmus. */
async function copyItems(paths, cut) {
  await saveNow();
  await call('copy_items', { paths });
  state.cut = cut ? paths : [];
  setStatus(t('copied'));
}

async function pasteInto(folder) {
  await saveNow();
  const { paths, moved } = await call('paste_items', { folder, cut: state.cut });
  if (moved) {
    state.cut.forEach((old, i) => remapAll(old, paths[i]));
    state.cut = [];
  }
  state.openFolders.add(folder);
  saveOpenFolders();
  await refreshTree();
  renderStatus();
  setStatus(t('pasted', paths.length));
}

/** The folder Ctrl+V pastes into: the selected tree folder, or the selected file's folder. */
/** Copies files and folders dropped from Finder or Explorer into `folder`. */
async function dropFiles(entries, folder) {
  const readDir = async (dir) => {
    const reader = dir.createReader();
    const all = [];
    for (;;) {
      const batch = await new Promise((ok, fail) => reader.readEntries(ok, fail));
      if (!batch.length) return all;
      all.push(...batch);
    }
  };
  let count = 0;
  const copy = async (entry, into) => {
    if (entry.name.startsWith('.')) return; // .DS_Store and other hidden files
    if (entry.isDirectory) {
      const sub = await call('create_folder', { parent: into, name: entry.name });
      for (const child of await readDir(entry)) await copy(child, sub);
    } else {
      const file = await new Promise((ok, fail) => entry.file(ok, fail));
      const headers = { 'x-folder': encodeURIComponent(into), 'x-name': encodeURIComponent(file.name) };
      await call('drop_file', new Uint8Array(await file.arrayBuffer()), { headers });
    }
    count++;
  };
  if (!entries.length) return;
  state.busy = true;
  renderStatus();
  try {
    for (const entry of entries) await copy(entry, folder);
  } finally {
    state.busy = false;
    state.openFolders.add(folder);
    saveOpenFolders();
    await refreshTree();
    setStatus(t('droppedFiles', count));
  }
}

function pasteTarget() {
  const sel = state.treeSel;
  if (!sel) return state.folder;
  return sel.kind === 'folder' || sel.kind === 'root' ? sel.path : parentOf(sel.path);
}

/**
 * Prints pages through the system print dialog: each sheet becomes an image
 * in #printArea, the only thing the print stylesheet shows.
 */
async function printPages(paths, title) {
  if (!paths.length) return error('nothingToPrint');
  canvas.endTextEdit();
  await saveNow();
  state.busy = true;
  renderStatus();
  try {
    const area = $('printArea');
    for (const img of area.querySelectorAll('img')) URL.revokeObjectURL(img.src);
    const imgs = [];
    const sheets = [];
    for (const path of paths) {
      const page = path === state.current?.path ? state.current.page : (await loadPage(path)).page;
      for (const c of await printSheets(page)) {
        const blob = await new Promise((r) => c.toBlob(r, 'image/png'));
        const img = el('img', { src: URL.createObjectURL(blob) });
        imgs.push(img);
        sheets.push(el('div', { class: 'sheet' }, img));
      }
    }
    area.replaceChildren(...sheets);
    await Promise.all(imgs.map((i) => i.decode().catch(() => {})));
    // The title is the suggested name when printing to PDF.
    const before = document.title;
    document.title = title;
    try {
      await call('print_view');
    } finally {
      setTimeout(() => (document.title = before), 1000);
    }
  } finally {
    state.busy = false;
    renderStatus();
  }
}

/** Prints the pages directly inside a folder, in tree order. */
function printFolder(node) {
  const pages = (findNode(state.tree, node.path)?.children ?? []).filter((n) => n.kind === 'page');
  return printPages(pages.map((n) => n.path), labelText(node.label) || node.name);
}

/**
 * Shows a long task's progress in the status bar: `text`, and a bar filled
 * `done` of `total` (moving back and forth when total is 0). null hides it.
 */
function showProgress(p) {
  $('progressSpin').hidden = !p;
  $('progress').hidden = !p;
  if (!p) return;
  setStatus(p.text);
  if (p.total) {
    $('progress').max = p.total;
    $('progress').value = p.done;
  } else $('progress').removeAttribute('value');
}

/** Imports OneNote files the user picks, each into its own folder inside `folder`. */
async function importOneNote(folder) {
  const files = await call('pick_onenote_files');
  if (!files.length) return;
  await saveNow();
  const name = (path) => path.split(/[\\/]/).pop().replace(/\.(one|onetoc2)$/i, '');
  let current = { index: 0, name: '' };
  const show = (done = 0, total = 0) =>
    showProgress({
      text: t('importingOneNote', current.name, current.index, files.length) + (total ? ' · ' + t('importingPages', done, total) : ''),
      done,
      total,
    });
  const unlisten = await window.__TAURI__.event.listen('onenote-progress', (e) => show(e.payload.pagesDone, e.payload.pages));
  let pages = 0;
  let last = null;
  const failed = [];
  try {
    for (const [i, file] of files.entries()) {
      current = { index: i + 1, name: name(file) };
      show(); // reading the file, before its pages are known
      try {
        const done = await invoke('import_onenote', { file, folder });
        pages += done.pages;
        last = done.folder;
        state.openFolders.add(done.folder);
      } catch (e) {
        failed.push(`${current.name}: ${e}`);
      }
    }
  } finally {
    unlisten();
    showProgress(null);
    state.openFolders.add(folder);
    if (files.length === 1 && last) state.folder = last;
    saveOpenFolders();
    await refreshTree();
    setStatus(t('importedOneNote', pages, files.length - failed.length, folderLabel(folder)));
    if (failed.length) error(t('importFailed', failed.join('\n')));
  }
}

async function addRoot() {
  await saveNow();
  const unlisten = await window.__TAURI__.event.listen('scan-progress', ({ payload: [done, total] }) =>
    showProgress({ text: t('readingPages', done, total), done, total }),
  );
  const tr = await call('add_root').finally(() => {
    unlisten();
    showProgress(null);
    setStatus('');
  });
  if (!tr) return null;
  for (const r of tr.tree) if (!state.tree.some((o) => o.path === r.path)) state.openFolders.add(r.path);
  saveOpenFolders();
  applyTree(tr);
  return tr;
}

/** Shows a root's contents at the top level of the tree, or the root itself again. */
function toggleFlat(root) {
  const flat = new Set(state.settings.flatRoots ?? []);
  if (!flat.delete(root.path)) flat.add(root.path);
  state.settings.flatRoots = [...flat];
  state.openFolders.add(root.path);
  saveSettings();
  saveOpenFolders();
  renderSidebar();
  renderStatus();
}

async function removeRoot(node) {
  if (!(await confirmDialog(t('confirmRemoveRoot', node.name), t('removeRootSub'), t('removeRoot')))) return;
  if (isFlat(state, node.path)) toggleFlat(node);
  if (state.current && isInside(state.current.path, node.path)) {
    await saveNow();
    closePage();
  }
  applyTree(await call('remove_root', { path: node.path }));
  if (!state.current) await openSomething();
}

/** Moves a folder or file up (-1) or down (+1) among its siblings. */
async function reorderItem(node, delta) {
  applyTree(await call('reorder', { path: node.path, delta }));
}

/** Moves a root up (-1) or down (+1) in the tree. */
async function moveRoot(node, delta) {
  applyTree(await call('move_root', { path: node.path, delta }));
}

// ---------- pictures ----------

async function addPicture(blob, at) {
  if (!state.current || !blob) return;
  try {
    const { src, width, height } = await fromBlob(blob);
    setTool('select');
    canvas.insertImage(src, width, height, at);
  } catch (e) {
    error(e);
  }
}

async function addPhoto() {
  if (!state.current) return;
  try {
    const shot = await takePhoto($('dialog'));
    if (!shot) return;
    setTool('select');
    canvas.insertImage(shot.src, shot.width, shot.height);
  } catch (e) {
    error(e);
  }
}

async function pasteFromClipboard() {
  try {
    for (const item of await navigator.clipboard.read()) {
      const type = item.types.find((x) => x.startsWith('image/'));
      if (type) return addPicture(await item.getType(type));
    }
    const text = await navigator.clipboard.readText();
    if (text.trim()) canvas.insertText(text.trim());
  } catch {
    error('clipboard');
  }
}

// ---------- rendering ----------

function renderToolbar() {
  const pen = state.settings.pen;
  const sel = state.selection;
  const selecting = state.tool === 'select' && sel.strokes.length + sel.texts.length + sel.images.length > 0;
  document.querySelectorAll('.tool').forEach((b) => b.classList.toggle('on', b.dataset.tool === state.tool));
  $('stage').className = state.tool === 'pen' ? '' : state.tool;
  // A text box being typed in, or a selection, shows (and is changed by) the
  // pen controls; otherwise they show the pen.
  const editing = canvas.editing?.box;
  const shown = editing ? { ...pen, color: editing.color } : selecting && sel.strokes.length ? sel.strokes[0].pen : selecting && sel.texts.length ? { ...pen, color: sel.texts[0].color } : pen;
  const hex = colorToHex(shown.color);
  $('swatches').querySelectorAll('.swatch').forEach((s) => s.classList.toggle('on', s.dataset.color === hex));
  // The color button shows a custom color; for the toolbar's own it shows a rainbow.
  $('colorChip').style.background = PALETTE.includes(hex) ? '' : hex;
  $('width').value = sliderFromWidth(shownWidth(shown));
  $('widthOut').textContent = shownWidth(shown);
  $('stylePreview').innerHTML = styleSvg(shown.style, shownWidth(shown), colorCss(shown.color));
  $('styleBtn').title = `${t('style')}: ${t('style_' + shown.style)}`;
  state.shownPen = shown;
  $('pressure').classList.toggle('on', pen.pressure_sensitive);
  $('eraserGroup').hidden = state.tool !== 'eraser';
  $('eraserSize').value = canvas.eraserRadius;
  const textSize = editing ? editing.size : selecting && sel.texts.length ? sel.texts[0].size : state.settings.textSize;
  $('textGroup').hidden = !(state.tool === 'text' || editing || (selecting && sel.texts.length));
  $('textSize').value = textSize;
  $('textSizeOut').textContent = Math.round(textSize);
  canvas.pen = pen;
  canvas.smoothing = state.settings.smoothing;
  canvas.touchDraws = state.settings.touchDraws;
  canvas.textSize = state.settings.textSize;
}

function renderUndo() {
  $('undo').disabled = !state.current?.undo.length;
  $('redo').disabled = !state.current?.redo.length;
}

function renderPage() {
  const c = state.current;
  $('title').value = c ? baseName(c.path) : '';
  autoDirection($('title'));
  $('title').disabled = !c;
  $('tagInput').disabled = !c;
  renderTags();
  renderUndo();
  renderStatus();
}

function renderTags() {
  const c = state.current;
  $('tags').replaceChildren(
    ...(c?.page.tags ?? []).map((tag, i) =>
      el('span', { class: 'chip' }, bdi('#' + tag), el('button', { title: t('removeTag'), text: '×', onclick: () => { c.page.tags.splice(i, 1); changed(); renderTags(); } })),
    ),
  );
}

function renderTagSuggestions() {
  $('tagSuggestions').replaceChildren(...state.tags.map((tag) => el('option', { value: tag })));
}

/** "today 14:32", "yesterday 14:32", "3 days ago" (up to a week), else "03/11/26 14:32". */
function editedTime(ms) {
  const d = new Date(ms);
  const pad = (n) => String(n).padStart(2, '0');
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  const midnight = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((midnight(new Date()) - midnight(d)) / 86400000);
  if (days === 0) return t('today', time);
  if (days === 1) return t('yesterday', time);
  if (days > 1 && days <= 7) return t('daysAgo', days);
  return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${pad(d.getFullYear() % 100)} ${time}`;
}

function renderStatus() {
  const c = state.current;
  $('statusPage').replaceChildren(
    ...(c
      ? [...pathNodes([...folderParts(parentOf(c.path)), baseName(c.path)]), ' · ', t('pageOf', state.sheet + 1, sheetCount(c.page)), c.dirty ? ' · ' + t('unsaved') : '']
      : []),
  );
  $('edited').textContent = c ? t('edited', editedTime(c.page.modified)) : '';
  $('spinner').hidden = !state.busy;
}

function normalizeTag(raw) {
  const s = raw.trim().replace(/^#+/, '').trim();
  return s ? s.split(/\s+/).join('-') : null;
}

function addTags(text) {
  const c = state.current;
  if (!c) return;
  let added = false;
  for (const word of text.split(/[,\s]+/)) {
    const tag = normalizeTag(word);
    if (tag && !c.page.tags.some((x) => x.toLowerCase() === tag.toLowerCase())) {
      c.page.tags.push(tag);
      added = true;
    }
  }
  if (added) {
    changed();
    renderTags();
  }
}

// ---------- updates ----------

/** Looks for a newer version; offers it, never installs it unasked. */
async function checkForUpdate({ quiet = true } = {}) {
  let update;
  try {
    update = await invoke('check_update');
  } catch (e) {
    if (!quiet) error(e);
    return;
  }
  if (update) offerUpdate(update.version);
  else if (!quiet) setStatus(t('upToDate', state.version));
}

function offerUpdate(version) {
  const toast = el('div', { class: 'toast update' },
    el('span', { text: t('updateAvailable', version) }),
    el('button', { class: 'primary', text: t('updateNow'), onclick: () => { toast.remove(); installUpdate(); } }),
    el('button', { class: 'link', text: t('later'), onclick: () => toast.remove() }),
  );
  $('toasts').append(toast);
}

/** Saves the page, then downloads and installs; the installer restarts Coolmus. */
async function installUpdate() {
  await saveNow();
  const unlisten = await window.__TAURI__.event.listen('update-progress', ({ payload: [done, total] }) =>
    showProgress({ text: t('downloadingUpdate'), done, total: total ?? 0 }),
  );
  showProgress({ text: t('downloadingUpdate') });
  try {
    await invoke('install_update');
  } catch (e) {
    error(e);
  } finally {
    unlisten();
    showProgress(null);
    setStatus('');
  }
}

// ---------- settings ----------

function saveSettings() {
  invoke('update_settings', { settings: state.settings }).catch(error);
}

function openSettings() {
  const s = state.settings;
  modal((d, done) => {
    const smooth = el('input', { type: 'range', min: 0, max: 0.9, step: 0.05, value: s.smoothing });
    const touch = el('input', { type: 'checkbox', checked: s.touchDraws });
    const roots = el('div');
    const renderRoots = () =>
      roots.replaceChildren(
        ...state.tree.map((r) =>
          el('div', { class: 'pathRow rootRow' },
            el('bdi', { class: 'path', dir: 'ltr', text: r.path }),
            el('label', {}, el('input', { type: 'checkbox', checked: isFlat(state, r.path), onchange: () => toggleFlat(r) }), t('flatRoot')),
            el('button', { class: 'link', text: '↑', title: t('moveUp'), disabled: r === state.tree[0], onclick: async () => { await moveRoot(r, -1); renderRoots(); } }),
            el('button', { class: 'link', text: '↓', title: t('moveDown'), disabled: r === state.tree[state.tree.length - 1], onclick: async () => { await moveRoot(r, 1); renderRoots(); } }),
            el('button', { class: 'link', text: t('revealFolder'), onclick: () => call('reveal', { path: r.path }) }),
            el('button', { class: 'link', text: t('removeRoot'), onclick: async () => { d.close(); await removeRoot(r); } }),
          ),
        ),
      );
    renderRoots();
    d.append(
      el('h3', { text: t('settings') }),
      el('b', { text: t('drawing') }),
      el('label', {}, t('smoothing'), smooth),
      el('label', {}, touch, t('touchDraws')),
      el('hr'),
      el('b', { text: t('folders') }),
      el('p', { class: 'muted', text: t('foldersHelp') }),
      roots,
      el('div', { class: 'row' },
        el('button', { text: t('addRoot'), onclick: async () => { if (await addRoot()) renderRoots(); } }),
      ),
      el('hr'),
      el('b', { text: t('updates') }),
      el('div', { class: 'row' },
        el('span', { class: 'muted', text: t('version', state.version) }),
        el('button', { text: t('checkUpdates'), onclick: () => { done(true); checkForUpdate({ quiet: false }); } }),
      ),
      el('div', { class: 'buttons' }, el('button', {
        class: 'primary',
        text: t('close'),
        onclick: () => {
          s.smoothing = Number(smooth.value);
          s.touchDraws = touch.checked;
          saveSettings();
          renderToolbar();
          done(true);
        },
      })),
    );
  });
}

// ---------- wiring ----------

/** Pen controls change the selection when there is one, else the pen. */
/** `props.width` is the width as the toolbar shows it (see shownWidth). */
function penChange(props, { switchToPen = true } = {}) {
  if (state.tool === 'select' && canvas.restyleSelection(props)) return renderToolbar();
  // Typing in a text box: the color goes to it (and stays the pen's for the next one).
  if (canvas.restyleEditing(props)) switchToPen = false;
  const { width, ...rest } = props;
  const pen = state.settings.pen;
  Object.assign(pen, rest);
  if (width) pen.width = width * widthScale(pen.style);
  if (switchToPen && state.tool !== 'text') setTool('pen');
  renderToolbar();
  saveSettings();
}

function loadRecentColors() {
  try {
    const list = JSON.parse(localStorage.getItem(RECENT_COLORS_KEY) || '[]');
    return Array.isArray(list) ? list.filter((c) => /^#[0-9a-f]{6}$/.test(c)).slice(0, RECENT_COLORS) : [];
  } catch {
    return [];
  }
}

/** Remembers a color picked from the panel (not one of the toolbar's), newest first. */
function rememberColor(hex) {
  if (PALETTE.includes(hex)) return;
  const list = [hex, ...loadRecentColors().filter((c) => c !== hex)].slice(0, RECENT_COLORS);
  try {
    localStorage.setItem(RECENT_COLORS_KEY, JSON.stringify(list));
  } catch {
    /* not essential */
  }
}

/** The color panel under the color button: recent custom colors, then the grid. */
function openColorPanel(anchor) {
  const current = colorToHex((state.shownPen ?? state.settings.pen).color);
  const chip = (hex) =>
    el('button', {
      class: 'chip' + (hex === current ? ' on' : ''),
      title: hex,
      style: `background:${hex}`,
      onclick: () => {
        hideMenu();
        rememberColor(hex);
        penChange({ color: hexToColor(hex) });
      },
    });
  const recent = loadRecentColors();
  const panel = el('div', { class: 'colorPanel' },
    ...(recent.length ? [el('div', { class: 'label', text: t('recentColors') }), el('div', { class: 'colorRow' }, ...recent.map(chip))] : []),
    el('div', { class: 'colorGrid' }, ...COLOR_GRID.map(chip)),
  );
  const rtl = document.documentElement.dir === 'rtl';
  showPanel(rtl ? anchor.right - 290 : anchor.left, anchor.bottom + 4, panel);
}

function bind() {
  document.querySelectorAll('.tool').forEach((b) => b.addEventListener('click', () => setTool(b.dataset.tool)));
  $('swatches').replaceChildren(
    ...PALETTE.map((hex) =>
      el('button', { class: 'swatch', 'data-color': hex, title: hex, style: `background:${hex}`, onclick: () => penChange({ color: hexToColor(hex) }) }),
    ),
  );
  $('styleBtn').addEventListener('click', (e) => {
    e.stopPropagation();
    const shown = state.shownPen ?? state.settings.pen;
    const r = $('styleBtn').getBoundingClientRect();
    const rtl = document.documentElement.dir === 'rtl';
    showMenu(rtl ? r.right - 180 : r.left, r.bottom + 4, STYLES.map((s) => ({
      html: styleSvg(s, shownWidth(shown), colorCss(shown.color)) + `<span>${t('style_' + s)}</span>`,
      on: s === shown.style,
      // The shown width stays; a marker draws it wider.
      action: () => penChange({ style: s, width: shownWidth(shown) }),
    })));
  });
  $('color').addEventListener('click', (e) => {
    e.stopPropagation();
    openColorPanel(e.currentTarget.getBoundingClientRect());
  });
  $('width').addEventListener('input', (e) => {
    $('widthOut').textContent = widthFromSlider(Number(e.target.value));
    if (state.tool !== 'select') {
      state.settings.pen.width = widthFromSlider(Number(e.target.value)) * widthScale(state.settings.pen.style);
      canvas.pen = state.settings.pen;
    }
  });
  $('width').addEventListener('change', (e) => {
    penChange({ width: widthFromSlider(Number(e.target.value)) }, { switchToPen: false });
    e.target.blur();
  });
  $('pressure').addEventListener('click', () => {
    state.settings.pen.pressure_sensitive = !state.settings.pen.pressure_sensitive;
    renderToolbar();
    saveSettings();
  });
  $('eraserSize').addEventListener('input', (e) => (canvas.eraserRadius = Number(e.target.value)));
  $('textSize').addEventListener('input', (e) => {
    $('textSizeOut').textContent = e.target.value;
    canvas.restyleEditing({ size: Number(e.target.value) }, { focus: false }); // live, while dragging
  });
  $('textSize').addEventListener('change', (e) => {
    const size = Number(e.target.value);
    canvas.restyleEditing({ size });
    if (!(state.tool === 'select' && canvas.restyleSelection({ size }))) {
      state.settings.textSize = size;
      saveSettings();
    }
    renderToolbar();
    e.target.blur();
  });
  $('imageBtn').addEventListener('click', (e) => {
    if (!state.current) return;
    const r = e.currentTarget.getBoundingClientRect();
    showMenu(r.left, r.bottom + 4, [
      { text: t('imageFromFile'), action: async () => addPicture(await pickFile()) },
      { text: t('takePhoto'), action: addPhoto },
      { text: t('paste'), action: pasteFromClipboard },
    ]);
  });
  $('undo').addEventListener('click', undo);
  $('redo').addEventListener('click', redo);
  $('zoomIn').addEventListener('click', () => canvas.zoomBy(1.2));
  $('zoomOut').addEventListener('click', () => canvas.zoomBy(1 / 1.2));
  $('zoomFit').addEventListener('click', () => canvas.fit());
  $('fullBtn').addEventListener('click', () => setFullPage(!state.fullPage));
  $('sidebarBtn').addEventListener('click', () => setSidebar(document.body.classList.contains('noSidebar')));
  let sidebarHidden = false;
  try {
    sidebarHidden = !!localStorage.getItem(SIDEBAR_KEY);
  } catch {
    /* storage unavailable: show it */
  }
  setSidebar(!sidebarHidden);
  bindSidebarResizer();
  $('exitFull').addEventListener('click', () => setFullPage(false));
  $('settingsBtn').addEventListener('click', openSettings);
  $('newPageBtn').addEventListener('click', () => newPage());
  $('newFolderBtn').addEventListener('click', () => newFolder());
  $('langBtn').addEventListener('click', () => {
    state.settings.language = getLanguage() === 'he' ? 'en' : 'he';
    saveSettings();
    applyLanguage();
  });
  $('pageMenu').addEventListener('click', (e) => {
    const c = state.current;
    if (!c) return;
    const r = e.currentTarget.getBoundingClientRect();
    showMenu(r.left, r.bottom + 4, [
      // A page drawn on a PDF has that PDF's pages as its paper.
      ...(c.page.background
        ? []
        : [
            { label: t('paper') },
            ...PAPERS.map((p) => ({ icon: ICONS['paper_' + p], text: t('paper_' + p), on: c.page.paper === p, action: () => { c.page.paper = p; changed(); canvas.invalidate(); } })),
            '-',
            { icon: ICONS.addSheet, text: t('addSheet'), action: addSheet },
            { icon: ICONS.removeSheet, text: t('removeSheet'), disabled: sheetCount(c.page) < 2, action: removeLastSheet },
          ]),
      {
        icon: ICONS.clearPage, text: t('clearPage'), danger: true,
        action: async () => {
          if (!(c.page.strokes.length || c.page.texts.length || c.page.images.length)) return;
          if (!(await confirmDialog(t('confirmClearPage', baseName(c.path)), t('clearPageSub'), t('clearPage')))) return;
          commit({ strokes: [], texts: [], images: [] });
        },
      },
      '-',
      { icon: ICONS.print, text: t('print'), action: () => printPages([c.path], baseName(c.path)) },
      { icon: ICONS.openInViewer, text: t('openInViewer'), action: () => call('open_external', { path: c.path }) },
      { icon: ICONS.revealFolder, text: t('revealFolder'), action: () => call('reveal', { path: c.path }) },
      {
        icon: ICONS.exportPng,
        text: t('exportPng'),
        action: async () => {
          const p = await call('save_png', { name: baseName(c.path), data: await pagePng(c.page) });
          if (p) setStatus(t('exported', p));
        },
      },
    ]);
  });

  $('title').addEventListener('change', async (e) => {
    const c = state.current;
    if (!c) return;
    const name = e.target.value.trim();
    if (!name || name === baseName(c.path)) return renderPage();
    try {
      await renamePath(c.path, name);
    } catch {
      renderPage();
    }
  });
  $('title').addEventListener('keydown', (e) => e.key === 'Enter' && e.target.blur());
  $('tagInput').addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      addTags(e.target.value);
      e.target.value = '';
      autoDirection(e.target);
    }
  });
  $('tagInput').addEventListener('change', (e) => {
    // Picking a datalist suggestion fires `change` without Enter.
    if (state.tags.includes(e.target.value)) {
      addTags(e.target.value);
      e.target.value = '';
      autoDirection(e.target);
    }
  });
  let searchTimer;
  $('search').addEventListener('input', (e) => {
    state.search = e.target.value;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(renderSidebar, 120);
  });

  // Pictures and text from the clipboard, and image files dropped on the page.
  // Copy and paste act on the tree after clicking in it, else on the page.
  $('sidebar').addEventListener('pointerdown', () => (state.focus = 'tree'));
  bindTreeSpace(treeCtx);
  bindPalmRejection();
  document.querySelectorAll('[data-dir="auto"]').forEach((f) => {
    f.addEventListener('input', () => autoDirection(f));
    f.addEventListener('change', () => autoDirection(f));
  });
  // While a text box is being typed in, toolbar and menu buttons don't take
  // the focus from it, so a color picked there applies to that text.
  for (const id of ['toolbar', 'menu']) {
    $(id).addEventListener('mousedown', (e) => canvas.editing && !e.target.closest('input') && e.preventDefault());
    // A slider can't keep the focus away like that, so the box is told to stay
    // open until the press is over.
    $(id).addEventListener('pointerdown', () => {
      if (!canvas.editing) return;
      canvas.holdEdit = true;
      const release = () => setTimeout(() => (canvas.holdEdit = false));
      window.addEventListener('pointerup', release, { once: true });
      window.addEventListener('pointercancel', release, { once: true });
    }, true);
  }
  $('stage').addEventListener('pointerdown', () => (state.focus = 'page'));
  document.addEventListener('paste', (e) => {
    if (typing() || !state.current || state.focus === 'tree') return;
    const file = [...(e.clipboardData?.files ?? [])].find((f) => f.type.startsWith('image/'));
    if (file) {
      e.preventDefault();
      addPicture(file);
      return;
    }
    const text = e.clipboardData?.getData('text/plain')?.trim();
    if (text) {
      e.preventDefault();
      canvas.insertText(text);
    }
  });
  $('stage').addEventListener('dragover', (e) => {
    if (e.dataTransfer.types.includes('Files')) e.preventDefault();
  });
  // Anywhere else a dropped file would be opened in place of the app.
  document.addEventListener('dragover', (e) => {
    if (e.defaultPrevented || !e.dataTransfer.types.includes('Files')) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'none';
  });
  document.addEventListener('drop', (e) => {
    if (e.dataTransfer.types.includes('Files')) e.preventDefault();
  });
  $('stage').addEventListener('drop', (e) => {
    const files = [...e.dataTransfer.files].filter((f) => f.type.startsWith('image/'));
    if (!files.length) return;
    e.preventDefault();
    const r = $('stage').getBoundingClientRect();
    const at = { x: (e.clientX - r.left - canvas.ox) / canvas.zoom, y: (e.clientY - r.top - canvas.oy) / canvas.zoom };
    files.forEach((f, i) => addPicture(f, { x: at.x + i * 24, y: at.y + i * 24 }));
  });

  window.addEventListener('keydown', (e) => {
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    if (mod && k === 's') { e.preventDefault(); if (state.current) state.current.dirty = true; saveNow(); return; }
    if (e.key === 'Escape' && !$('dialog').open) {
      if (state.selection.strokes.length + state.selection.texts.length + state.selection.images.length) canvas.clearSelection();
      else if (state.fullPage) setFullPage(false);
      return;
    }
    if (mod && k === 'p') { e.preventDefault(); if (state.current) printPages([state.current.path], baseName(state.current.path)); return; }
    if (typing()) return;
    if (mod && state.focus === 'tree' && ['c', 'x', 'v'].includes(k)) {
      e.preventDefault();
      const sel = state.treeSel ?? (state.current && { path: state.current.path, kind: 'page' });
      if (k === 'v') pasteInto(pasteTarget());
      else if (sel && sel.kind !== 'root') copyItems([sel.path], k === 'x');
      return;
    }
    const tool = { v: 'select', p: 'pen', t: 'text', e: 'eraser', h: 'pan' }[k] ?? { KeyV: 'select', KeyP: 'pen', KeyT: 'text', KeyE: 'eraser', KeyH: 'pan' }[e.code];
    if (mod && k === 'z' && !e.shiftKey) { e.preventDefault(); undo(); }
    else if (mod && (k === 'y' || (k === 'z' && e.shiftKey))) { e.preventDefault(); redo(); }
    else if (mod && k === 'n') { e.preventDefault(); newPage(); }
    else if (mod && k === '0') { e.preventDefault(); canvas.fit(); }
    else if (mod && k === 'b') { e.preventDefault(); setSidebar(document.body.classList.contains('noSidebar')); }
    else if (mod && k === 'a') { e.preventDefault(); setTool('select'); canvas.selectAll(); }
    else if (mod && k === 'd') { e.preventDefault(); canvas.duplicateSelection(); }
    else if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); canvas.deleteSelection(); }
    else if (!mod && tool) setTool(tool);
    else if (!mod && (k === 'f' || e.code === 'KeyF')) setFullPage(!state.fullPage);
  });
}

/**
 * Text fields marked data-dir="auto" take the direction of what is typed in
 * them, and the interface's while empty, so the placeholder and the caret
 * start on the right in Hebrew.
 */
function autoDirection(field) {
  field.dir = field.value.trim() ? 'auto' : document.documentElement.dir || 'ltr';
}

function applyLanguage() {
  setLanguage(state.settings.language);
  document.querySelectorAll('[data-dir="auto"]').forEach(autoDirection);
  translateDom();
  document.title = t('appName');
  renderToolbar();
  renderPage();
  renderSidebar();
  canvas.selectionChanged();
}

/** The time in the toolbar (handy in a meeting), updated on the minute. */
function tickClock() {
  const now = new Date();
  $('clock').textContent = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  setTimeout(tickClock, 60000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 50);
}

async function start() {
  const loader = $('loader');
  const unlisten = await window.__TAURI__.event.listen('scan-progress', ({ payload: [done, total] }) => {
    loader.classList.add('counting');
    loader.style.setProperty('--done', `${(100 * done) / total}%`);
    $('loaderCount').textContent = `${done} / ${total}`;
  });
  try {
    const snap = await invoke('get_state');
    state.version = await window.__TAURI__.app.getVersion();
    state.settings = snap.settings;
    setTree(snap);
    if (firstRun) state.tree.forEach((r) => state.openFolders.add(r.path));
    bind();
    tickClock();
    applyLanguage();
    renderTagSuggestions();
    await openSomething();
  } finally {
    unlisten();
    loader.classList.add('done');
    setTimeout(() => loader.remove(), 400);
  }
  setTimeout(checkForUpdate, 5000);
}

start().catch((e) => error(e));
