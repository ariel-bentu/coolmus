// Sketcher UI. Pages are files in folders, read and written by the Rust
// backend; this module keeps the open page and its undo history, autosaves,
// and wires the toolbar, the folder tree and the OneNote import panel.

import { setLanguage, t, translateDom, getLanguage } from './i18n.js';
import { SketchCanvas, colorCss } from './canvas.js';
import { DEFAULT_PAGE } from './ink.js';
import { $, el, bdi, error, modal, prompt, confirmDialog, showMenu } from './ui.js';
import { renderTree, allFolders, parentOf, baseName } from './tree.js';
import { renderOneNote } from './onenote.js';
import { pagePng } from './export.js';
import { fromBlob, pickFile, takePhoto } from './media.js';
import { editLabel } from './labels.js';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

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

const state = {
  tree: [],
  tags: [],
  root: '',
  settings: null,
  signedIn: false,
  current: null, // { path, page, undo: [], redo: [], dirty, lastChange }
  folder: '', // selected folder: where new pages and imports go
  openFolders: new Set(),
  search: '',
  tool: 'pen',
  fullPage: false,
  busy: false,
  selection: { strokes: [], texts: [], images: [] },
  onenote: { open: false, deviceCode: null, notebooks: [], selectedNotebook: null, selectedSection: null, pages: [] },
};

try {
  state.openFolders = new Set(JSON.parse(localStorage.getItem(OPEN_FOLDERS_KEY) || '[]'));
} catch {
  /* storage unavailable: start with everything collapsed */
}

// ---------- helpers ----------

const hexToColor = (hex) => ({ r: parseInt(hex.slice(1, 3), 16), g: parseInt(hex.slice(3, 5), 16), b: parseInt(hex.slice(5, 7), 16), a: 255 });
const colorToHex = (c) => '#' + [c.r, c.g, c.b].map((v) => v.toString(16).padStart(2, '0')).join('');
const widthFromSlider = (v) => Math.round(0.5 * Math.pow(80, v / 100) * 10) / 10;
const sliderFromWidth = (w) => Math.round((100 * Math.log(w / 0.5)) / Math.log(80));
const typing = () => /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName) || $('dialog').open;

async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    error(e);
    throw e;
  }
}

function setStatus(s) {
  $('statusText').textContent = s;
}

const folderLabel = (path) => (path ? path.replaceAll('/', ' › ') : state.root.split(/[\\/]/).filter(Boolean).pop() || t('rootFolder'));

/** Path after `old` was renamed/moved to `nu` (for paths at or below `old`). */
function remap(path, old, nu) {
  if (path === old) return nu;
  if (path.startsWith(old + '/')) return nu + path.slice(old.length);
  return path;
}

function remapAll(old, nu) {
  if (state.current) state.current.path = remap(state.current.path, old, nu);
  state.folder = remap(state.folder, old, nu);
  state.openFolders = new Set([...state.openFolders].map((p) => remap(p, old, nu)));
  saveOpenFolders();
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

/** Grows the page so content never falls outside it (PNG export uses the page size). */
function growPage(page) {
  let maxX = 0, maxY = 0;
  for (const s of page.strokes) for (const [x, y] of s.points) {
    if (x + s.pen.width > maxX) maxX = x + s.pen.width;
    if (y + s.pen.width > maxY) maxY = y + s.pen.width;
  }
  for (const b of page.texts) {
    maxX = Math.max(maxX, b.x + b.width);
    maxY = Math.max(maxY, b.y + b.size * 1.3 * Math.max(1, b.text.split('\n').length));
  }
  for (const im of page.images) {
    maxX = Math.max(maxX, im.x + im.width);
    maxY = Math.max(maxY, im.y + im.height);
  }
  page.width = Math.max(page.width, Math.ceil(maxX + 40));
  page.height = Math.max(page.height, Math.ceil(maxY + 40));
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
  growPage(c.page);
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
  if (c && c.dirty && !canvas.isBusy() && performance.now() - c.lastChange > 1000) saveNow().then(refreshTree);
}, 400);
window.addEventListener('beforeunload', () => saveNow());
// The folder may change outside Sketcher (Explorer, OneDrive): rescan on focus.
window.addEventListener('focus', () => refreshTree());

let treeVersion = 0;
async function refreshTree() {
  const v = ++treeVersion;
  const tr = await invoke('get_tree');
  if (v !== treeVersion) return;
  state.tree = tr.tree;
  state.tags = tr.tags;
  state.root = tr.root;
  renderSidebar();
  renderTagSuggestions();
}

const treeCtx = {
  state,
  open: (path) => openPage(path),
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
  reveal: () => call('reveal_root'),
  saveOpenFolders,
};

function renderSidebar() {
  renderTree(treeCtx);
}

async function openPage(path, { force = false } = {}) {
  if (!force && state.current?.path === path) return;
  canvas.endTextEdit();
  await saveNow();
  const page = await call('load_page', { path });
  state.current = { path, page, undo: [], redo: [], dirty: false, lastChange: 0 };
  state.folder = parentOf(path);
  // Reveal the opened page; the user may collapse its folders afterwards.
  for (let p = state.folder; p; p = parentOf(p)) state.openFolders.add(p);
  saveOpenFolders();
  canvas.setPage(page);
  renderPage();
  renderSidebar();
  renderOneNote(oneNoteCtx);
}

function closePage() {
  state.current = null;
  canvas.setPage(null);
  renderPage();
}

/** Opens the most recently changed page, or creates a first one. */
async function openSomething() {
  const hits = await invoke('search', { query: '' });
  if (hits.length) return openPage(hits[0].path);
  return newPage('');
}

function pagesIn(folder) {
  let nodes = state.tree;
  for (const part of folder.split('/').filter(Boolean)) nodes = nodes.find((n) => n.kind === 'folder' && n.name === part)?.children ?? [];
  return nodes.filter((n) => n.kind === 'page').length;
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
  const folders = [{ path: '', name: folderLabel(''), depth: 0 }, ...allFolders(state.tree)].filter(
    (f) => f.path !== node.path && !f.path.startsWith(node.path + '/'),
  );
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
  if (state.folder === node.path || state.folder.startsWith(node.path + '/')) state.folder = parentOf(node.path);
  await refreshTree();
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
  $('imported').hidden = !c?.page.onenote;
}

function renderTagSuggestions() {
  $('tagSuggestions').replaceChildren(...state.tags.map((tag) => el('option', { value: tag })));
}

function renderStatus() {
  const c = state.current;
  $('statusPage').replaceChildren(
    ...(c
      ? [bdi(folderLabel(parentOf(c.path))), ' · ', t('counts', c.page.strokes.length, c.page.texts.length, c.page.images.length), ' · ', c.dirty ? t('unsaved') : t('saved')]
      : []),
  );
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

// ---------- OneNote ----------

const oneNoteCtx = {
  state,
  saveSettings: () => saveSettings(),
  folderLabel,
  toggle: (on) => toggleOneNote(on),
};

function toggleOneNote(force) {
  state.onenote.open = force ?? !state.onenote.open;
  $('onenote').hidden = !state.onenote.open;
  $('oneNoteBtn').classList.toggle('on', state.onenote.open);
  if (state.onenote.open && state.signedIn && !state.onenote.notebooks.length && !state.busy) invoke('onenote_list_notebooks');
  renderOneNote(oneNoteCtx);
}

async function onSync(ev) {
  const o = state.onenote;
  switch (ev.kind) {
    case 'busy':
      state.busy = ev.busy;
      renderStatus();
      break;
    case 'deviceCode':
      o.deviceCode = { userCode: ev.userCode, verificationUri: ev.verificationUri };
      toggleOneNote(true);
      break;
    case 'signedIn':
      state.signedIn = true;
      o.deviceCode = null;
      setStatus(t('signedIn'));
      break;
    case 'signedOut':
      state.signedIn = false;
      Object.assign(o, { deviceCode: null, notebooks: [], pages: [], selectedNotebook: null, selectedSection: null });
      break;
    case 'notebooks':
      o.notebooks = ev.notebooks;
      break;
    case 'pages':
      if (o.selectedSection?.id === ev.sectionId) o.pages = ev.pages;
      break;
    case 'info':
      setStatus(t('info_' + ev.key, ...ev.args.map((a, i) => (ev.key === 'importedN' && i === 1 ? folderLabel(a) : a))));
      break;
    case 'error':
      o.deviceCode = null;
      error(ev.message);
      break;
    case 'treeChanged':
      await refreshTree();
      break;
    case 'pageImported':
      if (ev.open) {
        await openPage(ev.path);
        const name = baseName(ev.path);
        setStatus(ev.existed ? t('alreadyImported', name) : t({ NativeInk: 'importedInk', Empty: 'importedEmpty', SketcherData: 'importedOk' }[ev.source], name));
      }
      break;
  }
  renderOneNote(oneNoteCtx);
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
    const rootText = el('bdi', { class: 'path', text: state.root });
    d.append(
      el('h3', { text: t('settings') }),
      el('b', { text: t('drawing') }),
      el('label', {}, t('smoothing'), smooth),
      el('label', {}, touch, t('touchDraws')),
      el('hr'),
      el('b', { text: t('pageFolder') }),
      el('p', { class: 'muted', text: t('pageFolderHelp') }),
      el('div', { class: 'pathRow' }, rootText),
      el('div', { class: 'row' },
        el('button', {
          text: t('changeFolder'),
          onclick: async () => {
            await saveNow();
            const tr = await call('choose_root');
            if (!tr) return;
            s.root = tr.root;
            rootText.textContent = tr.root;
            Object.assign(state, { tree: tr.tree, tags: tr.tags, root: tr.root, folder: '' });
            closePage();
            renderSidebar();
            await openSomething();
          },
        }),
        el('button', { text: t('revealFolder'), onclick: () => call('reveal_root') }),
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
  $('exitFull').addEventListener('click', () => setFullPage(false));
  $('oneNoteBtn').addEventListener('click', () => toggleOneNote());
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
      { text: t('extendPage'), action: () => { c.page.height += DEFAULT_PAGE.height / 2; changed(); canvas.invalidate(); } },
      {
        text: t('clearPage'), danger: true,
        action: () => (c.page.strokes.length || c.page.texts.length || c.page.images.length) && commit({ strokes: [], texts: [], images: [] }),
      },
      '-',
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
  document.addEventListener('paste', (e) => {
    if (typing() || !state.current) return;
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
    if (typing()) return;
    const tool = { v: 'select', p: 'pen', t: 'text', e: 'eraser', h: 'pan' }[k] ?? { KeyV: 'select', KeyP: 'pen', KeyT: 'text', KeyE: 'eraser', KeyH: 'pan' }[e.code];
    if (mod && k === 'z' && !e.shiftKey) { e.preventDefault(); undo(); }
    else if (mod && (k === 'y' || (k === 'z' && e.shiftKey))) { e.preventDefault(); redo(); }
    else if (mod && k === 'n') { e.preventDefault(); newPage(); }
    else if (mod && k === '0') { e.preventDefault(); canvas.fit(); }
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
  renderOneNote(oneNoteCtx);
  canvas.selectionChanged();
}

async function start() {
  await listen('sync', (e) => onSync(e.payload));
  const snap = await invoke('get_state');
  Object.assign(state, { tree: snap.tree, tags: snap.tags, root: snap.root, settings: snap.settings, signedIn: snap.signedIn });
  bind();
  applyLanguage();
  renderTagSuggestions();
  await openSomething();
}

start().catch((e) => error(e));
