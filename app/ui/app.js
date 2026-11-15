// Sketcher UI. Pages are PDF files in the user's folders, read and written by
// the Rust backend; this module keeps the open page and its undo history,
// autosaves, and wires the toolbar and the folder tree.

import { setLanguage, t, translateDom, getLanguage } from './i18n.js';
import { SketchCanvas, colorCss } from './canvas.js';
import { bounds, sheetCount, sheetTop, SHEET_GAP } from './ink.js';
import { $, el, bdi, error, modal, prompt, confirmDialog, showMenu } from './ui.js';
import { renderTree, allFolders, findNode, parentOf, baseName, isInside } from './tree.js';
import { pagePng, printSheets } from './export.js';
import { fromBlob, pickFile, takePhoto } from './media.js';
import { editLabel, labelText } from './labels.js';

const { invoke } = window.__TAURI__.core;

const PALETTE = ['#1a1a1a', '#5f6368', '#d93025', '#f28b00', '#f9d400', '#1e8e3e', '#009688', '#1a73e8', '#7b1fa2', '#e91e63'];
const STYLES = ['Solid', 'Dashed', 'Dotted', 'Highlighter', 'Pencil'];
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
const OPEN_FOLDERS_KEY = 'sketcher.openFolders';
const SIDEBAR_KEY = 'sketcher.sidebarHidden';
const SIDEBAR_WIDTH_KEY = 'sketcher.sidebarWidth';

const state = {
  tree: [], // root nodes
  tags: [],
  settings: null,
  current: null, // { path, page, undo: [], redo: [], dirty, lastChange }
  folder: '', // selected folder: where new pages go
  treeSel: null, // last tree item clicked: target of Ctrl+C / Ctrl+X / Ctrl+V
  focus: 'page', // 'tree' or 'page': where the keyboard's copy/paste applies
  cut: [], // paths cut in Sketcher, moved by the next paste
  openFolders: new Set(),
  search: '',
  tool: 'pen',
  fullPage: false,
  busy: false,
  sheet: 0, // the sheet in view, from the canvas
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
  return parts;
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
const VIEWS_KEY = 'sketcher.views';
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
  canvas.showSheet(n);
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
// The folder may change outside Sketcher (Explorer, OneDrive): rescan on focus.
window.addEventListener('focus', () => refreshTree());

function setTree(tr) {
  state.tree = tr.tree;
  state.tags = tr.tags;
  if (!state.tree.some((r) => isInside(state.folder, r.path))) state.folder = firstRoot();
}

let treeVersion = 0;
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
  open: (node) => (node.kind === 'page' ? openPage(node.path) : call('open_external', { path: node.path })),
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
  print: (node) => (node.kind === 'page' ? printPages([node.path], node.name) : printFolder(node)),
  removeRoot: (node) => removeRoot(node),
  importOneNote: (folder) => importOneNote(folder),
  select: (node) => (state.treeSel = { path: node.path, kind: node.kind }),
  folderLabel,
  folderPath: (path) => pathNodes(folderParts(path)),
  dropFiles,
  saveOpenFolders,
};

function renderSidebar() {
  renderTree(treeCtx);
}

async function openPage(path, { force = false } = {}) {
  if (!force && state.current?.path === path) return;
  canvas.endTextEdit();
  rememberView();
  await saveNow();
  const { page, changes } = await call('load_page', { path });
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

/** Puts items on the system clipboard (pasteable in Explorer too); a cut moves them on the next paste in Sketcher. */
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
      const page = path === state.current?.path ? state.current.page : (await call('load_page', { path })).page;
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

async function importOneNote(folder) {
  await saveNow();
  state.busy = true;
  renderStatus();
  setStatus(t('importingOneNote'));
  try {
    const done = await call('import_onenote', { folder });
    setStatus('');
    if (!done) return;
    state.openFolders.add(folder);
    state.openFolders.add(done.folder);
    state.folder = done.folder;
    saveOpenFolders();
    await refreshTree();
    setStatus(t('importedOneNote', done.pages, folderLabel(done.folder)));
  } finally {
    state.busy = false;
    renderStatus();
  }
}

async function addRoot() {
  await saveNow();
  const tr = await call('add_root');
  if (!tr) return null;
  for (const r of tr.tree) if (!state.tree.some((o) => o.path === r.path)) state.openFolders.add(r.path);
  saveOpenFolders();
  setTree(tr);
  renderSidebar();
  return tr;
}

async function removeRoot(node) {
  if (!(await confirmDialog(t('confirmRemoveRoot', node.name), t('removeRootSub'), t('removeRoot')))) return;
  if (state.current && isInside(state.current.path, node.path)) {
    await saveNow();
    closePage();
  }
  setTree(await call('remove_root', { path: node.path }));
  renderSidebar();
  if (!state.current) await openSomething();
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
  // With a selection, the pen controls show (and change) the selected ink.
  const shown = selecting && sel.strokes.length ? sel.strokes[0].pen : selecting && sel.texts.length ? { ...pen, color: sel.texts[0].color } : pen;
  const hex = colorToHex(shown.color);
  $('swatches').querySelectorAll('.swatch').forEach((s) => s.classList.toggle('on', s.dataset.color === hex));
  $('color').value = hex;
  $('width').value = sliderFromWidth(shown.width);
  $('widthOut').textContent = shown.width;
  $('stylePreview').innerHTML = styleSvg(shown.style, shown.width, colorCss(shown.color));
  $('styleBtn').title = `${t('style')}: ${t('style_' + shown.style)}`;
  state.shownPen = shown;
  $('pressure').classList.toggle('on', pen.pressure_sensitive);
  $('eraserGroup').hidden = state.tool !== 'eraser';
  $('eraserSize').value = canvas.eraserRadius;
  const textSize = selecting && sel.texts.length ? sel.texts[0].size : state.settings.textSize;
  $('textGroup').hidden = !(state.tool === 'text' || (selecting && sel.texts.length));
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
            el('bdi', { class: 'path', text: r.path }),
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
function penChange(props, { switchToPen = true } = {}) {
  if (state.tool === 'select' && canvas.restyleSelection(props)) return renderToolbar();
  Object.assign(state.settings.pen, props);
  if (switchToPen && state.tool !== 'text') setTool('pen');
  renderToolbar();
  saveSettings();
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
      html: styleSvg(s, shown.width, colorCss(shown.color)) + `<span>${t('style_' + s)}</span>`,
      on: s === shown.style,
      action: () => penChange({ style: s }),
    })));
  });
  $('color').addEventListener('input', (e) => {
    if (state.tool === 'select' && state.selection.strokes.length + state.selection.texts.length) return;
    state.settings.pen.color = hexToColor(e.target.value);
    renderToolbar();
  });
  $('color').addEventListener('change', (e) => {
    penChange({ color: hexToColor(e.target.value) }, { switchToPen: false });
    e.target.blur();
  });
  $('width').addEventListener('input', (e) => {
    $('widthOut').textContent = widthFromSlider(Number(e.target.value));
    if (state.tool !== 'select') {
      state.settings.pen.width = widthFromSlider(Number(e.target.value));
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
  $('textSize').addEventListener('input', (e) => ($('textSizeOut').textContent = e.target.value));
  $('textSize').addEventListener('change', (e) => {
    const size = Number(e.target.value);
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
      { label: t('paper') },
      ...PAPERS.map((p) => ({ text: (c.page.paper === p ? '✓ ' : ' ') + t('paper_' + p), action: () => { c.page.paper = p; changed(); canvas.invalidate(); } })),
      '-',
      { text: t('addSheet'), action: addSheet },
      { text: t('removeSheet'), disabled: sheetCount(c.page) < 2, action: removeLastSheet },
      {
        text: t('clearPage'), danger: true,
        action: () => (c.page.strokes.length || c.page.texts.length || c.page.images.length) && commit({ strokes: [], texts: [], images: [] }),
      },
      '-',
      { text: t('print'), action: () => printPages([c.path], baseName(c.path)) },
      { text: t('openInViewer'), action: () => call('open_external', { path: c.path }) },
      { text: t('revealFolder'), action: () => call('reveal', { path: c.path }) },
      {
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
    }
  });
  $('tagInput').addEventListener('change', (e) => {
    // Picking a datalist suggestion fires `change` without Enter.
    if (state.tags.includes(e.target.value)) {
      addTags(e.target.value);
      e.target.value = '';
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

function applyLanguage() {
  setLanguage(state.settings.language);
  translateDom();
  document.title = t('appName');
  renderToolbar();
  renderPage();
  renderSidebar();
  canvas.selectionChanged();
}

async function start() {
  const snap = await invoke('get_state');
  state.settings = snap.settings;
  setTree(snap);
  if (firstRun) state.tree.forEach((r) => state.openFolders.add(r.path));
  bind();
  applyLanguage();
  renderTagSuggestions();
  await openSomething();
}

start().catch((e) => error(e));
