// The sidebar: the root folders as a tree of folders, pages and other PDFs,
// plus search and the tag list. Paths are absolute, "/"-separated.

import { t } from './i18n.js';
import { $, el, bdi, showMenu } from './ui.js';
import { labelText } from './labels.js';

const FOLDER = '<svg viewBox="0 0 24 24"><path d="M3 6a1 1 0 011-1h5l2 2h9a1 1 0 011 1v10a1 1 0 01-1 1H4a1 1 0 01-1-1z"/></svg>';
const PAGE = '<svg viewBox="0 0 24 24"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/><path d="M9 14c2-3 3 1 6-2"/></svg>';
const PDF = '<svg viewBox="0 0 24 24"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/><path d="M9 13h6M9 16h6M9 10h2"/></svg>';
const DOTS = '<svg viewBox="0 0 24 24"><circle cx="12" cy="5" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="12" cy="19" r="1.6"/></svg>';
const CHEVRON = '<svg viewBox="0 0 24 24" class="chev"><path d="M9 6l6 6-6 6"/></svg>';
const DRAG_TYPE = 'text/x-coolmus-path';

export const parentOf = (path) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');
export const baseName = (path) => path.slice(path.lastIndexOf('/') + 1).replace(/\.pdf$/i, '');
export const isInside = (path, folder) => path === folder || path.startsWith(folder + '/');

/** True if the root at `path` shows its contents at the top level of the tree. */
export const isFlat = (state, path) => (state.settings?.flatRoots ?? []).includes(path);

/**
 * "Move up" / "Move down" for a root among the roots, a folder among its
 * sibling folders, or a file among its sibling files.
 */
function orderItems(ctx, node) {
  const isFolder = (n) => n.kind === 'folder' || n.kind === 'root';
  const siblings = node.kind === 'root' ? ctx.state.tree : (findNode(ctx.state.tree, parentOf(node.path))?.children ?? []).filter((n) => isFolder(n) === isFolder(node));
  const i = siblings.findIndex((n) => n.path === node.path);
  if (siblings.length < 2) return [];
  const move = (delta) => (node.kind === 'root' ? ctx.moveRoot(node, delta) : ctx.reorder(node, delta));
  return [
    { text: t('moveUp'), disabled: i === 0, action: () => move(-1) },
    { text: t('moveDown'), disabled: i === siblings.length - 1, action: () => move(1) },
  ];
}

/** Roots and folders, depth-first, as { path, name, depth } (name: the label if there is one). */
export function allFolders(nodes, depth = 0, out = []) {
  for (const n of nodes) {
    if (n.kind !== 'folder' && n.kind !== 'root') continue;
    out.push({ path: n.path, name: labelText(n.label) || n.name, depth });
    allFolders(n.children ?? [], depth + 1, out);
  }
  return out;
}

/** The node at `path`, or null. */
export function findNode(nodes, path) {
  for (const n of nodes) {
    if (n.path === path) return n;
    if (n.children && isInside(path, n.path)) return findNode(n.children, path);
  }
  return null;
}

/**
 * Renders the tree into #tree. `ctx` holds the app state and actions:
 * { state, open(node), newPage(folder), newFolder(parent), rename(node),
 *   move(node), moveTo(path, folder), remove(node), editLabel(node),
 *   reveal(path), openExternal(path), copy(node), cut(node), paste(folder),
 *   print(node), importOneNote(folder), removeRoot(node), toggleFlat(root), dropFiles(entries, folder), folderLabel(path), folderPath(path), select(node), saveOpenFolders() }
 */
export async function renderTree(ctx) {
  const { state } = ctx;
  renderTagCloud(ctx);
  const tree = $('tree');
  const cur = state.current?.path;

  if (state.search.trim()) {
    const hits = await window.__TAURI__.core.invoke('search', { query: state.search });
    tree.replaceChildren(
      el('div', { class: 'muted', text: hits.length ? t('results', hits.length) : t('noResults') }),
      ...hits.map((h) =>
        el('div', { class: 'result' + (h.path === cur ? ' on' : ''), onclick: () => ctx.open(h) },
          bdi(h.name),
          el('small', {}, ...ctx.folderPath(h.folder), ' ', bdi(h.tags.map((x) => '#' + x).join(' '))),
        ),
      ),
    );
    return;
  }

  const rows = [];
  const walk = (nodes, depth) => {
    for (const n of nodes) {
      // A root set to show its contents at the top level has no row of its own.
      if (n.kind === 'root' && isFlat(state, n.path)) walk(n.children ?? [], depth);
      else if (n.kind === 'folder' || n.kind === 'root') {
        const open = state.openFolders.has(n.path);
        rows.push(folderRow(ctx, n, depth, open));
        if (open) walk(n.children ?? [], depth + 1);
      } else rows.push(fileRow(ctx, n, depth));
    }
  };
  walk(state.tree, 0);
  tree.replaceChildren(...rows);
}

function indent(depth) {
  return `padding-inline-start: ${6 + depth * 14}px`;
}

const selected = (state, node) => state.treeSel?.path === node.path;

function folderRow(ctx, node, depth, open) {
  const { state } = ctx;
  const isRoot = node.kind === 'root';
  const row = el('div', {
    class: 'row folder' + (state.folder === node.path ? ' on' : '') + (open ? ' open' : '') + (isRoot ? ' root' : '') + (node.missing ? ' missing' : ''),
    style: indent(depth),
    draggable: !isRoot,
    title: node.path,
    'data-path': node.path,
  });
  const toggle = (on) => {
    if (on) state.openFolders.add(node.path);
    else state.openFolders.delete(node.path);
    ctx.saveOpenFolders();
  };
  const chev = el('span', { class: 'chevBtn', html: CHEVRON });
  chev.addEventListener('click', (e) => {
    e.stopPropagation();
    toggle(!open);
    renderTree(ctx);
  });
  row.append(chev);
  // A calculated label (e.g. this year's class) is shown in place of the name.
  const label = labelText(node.label);
  row.append(el('span', { class: 'ico', html: FOLDER }), bdi(label || node.name));
  if (label) row.append(el('small', { class: 'techName' }, bdi(node.name)));
  if (node.missing) row.append(el('small', { class: 'techName', text: t('missingRoot') }));
  // A ⋮ button, shown on hover and on the selected folder, for the common actions.
  const btn = el('button', { class: 'rowMenu hover', title: t('moreActions'), html: DOTS });
  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    state.folder = node.path;
    ctx.select(node);
    const r = btn.getBoundingClientRect();
    showMenu(document.dir === 'rtl' ? r.left : r.right, r.bottom + 2, [
      { text: t('newPageHere'), action: () => ctx.newPage(node.path) },
      { text: t('newFolderHere'), action: () => ctx.newFolder(node.path) },
      '-',
      ...orderItems(ctx, node),
      ...(isRoot
        ? [{ text: t('removeRoot'), action: () => ctx.removeRoot(node) }]
        : [
            { text: t('rename'), action: () => ctx.rename(node) },
            { text: t('delete'), danger: true, action: () => ctx.remove(node) },
          ]),
    ]);
  });
  row.append(btn);
  // Clicking selects the folder (where new pages go) and opens it; clicking
  // the selected, open folder again closes it.
  row.addEventListener('click', () => {
    const wasSelected = state.folder === node.path;
    state.folder = node.path;
    ctx.select(node);
    toggle(!(open && wasSelected));
    renderTree(ctx);
  });
  row.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    state.folder = node.path;
    ctx.select(node);
    renderTree(ctx);
    folderMenu(ctx, node, e.clientX, e.clientY);
  });
  dropTarget(ctx, row, node.path);
  if (!isRoot) dragSource(row, node.path);
  return row;
}

/** The menu for a folder or root (also for a root shown flat, from the tree's empty space). */
export function folderMenu(ctx, node, x, y) {
  const isRoot = node.kind === 'root';
  showMenu(x, y, [
      { text: t('newPageHere'), action: () => ctx.newPage(node.path) },
      { text: t('newFolderHere'), action: () => ctx.newFolder(node.path) },
      { text: t('pasteItems'), action: () => ctx.paste(node.path) },
      { text: t('importOneNote'), action: () => ctx.importOneNote(node.path) },
      '-',
      !isRoot && { text: t('copyItem'), action: () => ctx.copy(node) },
      !isRoot && { text: t('cutItem'), action: () => ctx.cut(node) },
      !isRoot && { text: t('rename'), action: () => ctx.rename(node) },
      !isRoot && { text: t('moveTo'), action: () => ctx.move(node) },
      { text: t('folderLabelMenu'), action: () => ctx.editLabel(node) },
      { text: t('printFolder'), action: () => ctx.print(node) },
      { text: t('revealFolder'), action: () => ctx.reveal(node.path) },
      isRoot && { text: (isFlat(ctx.state, node.path) ? '✓ ' : '') + t('flatRoot'), action: () => ctx.toggleFlat(node) },
      ...orderItems(ctx, node),
      '-',
      isRoot
        ? { text: t('removeRoot'), action: () => ctx.removeRoot(node) }
        : { text: t('delete'), danger: true, action: () => ctx.remove(node) },
  ]);
}

function fileRow(ctx, node, depth) {
  const { state } = ctx;
  const isPage = node.kind === 'page';
  const row = el('div', {
    class: 'row ' + node.kind + (node.path === state.current?.path ? ' on' : selected(state, node) ? ' sel' : ''),
    style: indent(depth),
    draggable: true,
    title: node.tags?.length ? node.tags.map((x) => '#' + x).join(' ') : node.name,
  }, el('span', { class: 'ico', html: isPage ? PAGE : PDF }), bdi(node.name));
  row.addEventListener('click', () => {
    ctx.select(node);
    ctx.open(node);
  });
  const menu = (x, y) => showMenu(x, y, [
    { text: t('print'), action: () => ctx.print(node) },
    { text: t('openInViewer'), action: () => ctx.openExternal(node.path) },
    { text: t('revealFolder'), action: () => ctx.reveal(node.path) },
    '-',
    { text: t('copyItem'), action: () => ctx.copy(node) },
    { text: t('cutItem'), action: () => ctx.cut(node) },
    { text: t('rename'), action: () => ctx.rename(node) },
    { text: t('moveTo'), action: () => ctx.move(node) },
    ...orderItems(ctx, node),
    '-',
    { text: t('delete'), danger: true, action: () => ctx.remove(node) },
  ]);
  row.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    ctx.select(node);
    menu(e.clientX, e.clientY);
  });
  // A ⋮ button, shown on hover and on the open or selected file (touch has no hover).
  const btn = el('button', { class: 'rowMenu hover', title: t('moreActions'), html: DOTS });
  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    ctx.select(node);
    const r = btn.getBoundingClientRect();
    menu(document.dir === 'rtl' ? r.left : r.right, r.bottom + 2);
  });
  row.append(btn);
  dropTarget(ctx, row, parentOf(node.path));
  dragSource(row, node.path);
  return row;
}

function dragSource(row, path) {
  row.addEventListener('dragstart', (e) => {
    e.dataTransfer.setData(DRAG_TYPE, path);
    e.dataTransfer.effectAllowed = 'move';
  });
}

const accepts = (e) => e.dataTransfer.types.includes(DRAG_TYPE) || e.dataTransfer.types.includes('Files');

/** A drop onto `folder`: a tree item moves there, files from Finder or Explorer are copied in. */
function drop(ctx, e, folder) {
  e.preventDefault();
  const path = e.dataTransfer.getData(DRAG_TYPE);
  if (path) return ctx.moveTo(path, folder);
  // The entries must be taken now: the drop's data is gone once it returns.
  const entries = [...e.dataTransfer.items].map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
  ctx.dropFiles(entries, folder);
}

function dragOver(e) {
  e.preventDefault();
  e.dataTransfer.dropEffect = e.dataTransfer.types.includes('Files') ? 'copy' : 'move';
}

function dropTarget(ctx, row, folder) {
  row.addEventListener('dragover', (e) => {
    if (!accepts(e)) return;
    dragOver(e);
    row.classList.add('drop');
  });
  row.addEventListener('dragleave', () => row.classList.remove('drop'));
  row.addEventListener('drop', (e) => {
    row.classList.remove('drop');
    if (accepts(e)) drop(ctx, e, folder);
  });
}

/**
 * The tree's empty space stands for a root shown flat, whose own row is
 * hidden: right-click there for its menu, drop there to put things in it.
 */
export function bindTreeSpace(ctx) {
  const tree = $('tree');
  const flatRoot = (e) => (e.target.closest('.row, .result') ? null : ctx.state.tree.find((r) => isFlat(ctx.state, r.path)));
  tree.addEventListener('contextmenu', (e) => {
    const root = flatRoot(e);
    if (!root) return;
    e.preventDefault();
    ctx.state.folder = root.path;
    folderMenu(ctx, root, e.clientX, e.clientY);
  });
  tree.addEventListener('dragover', (e) => flatRoot(e) && accepts(e) && dragOver(e));
  tree.addEventListener('drop', (e) => {
    const root = flatRoot(e);
    if (root && accepts(e)) drop(ctx, e, root.path);
  });
}

function renderTagCloud(ctx) {
  const { state } = ctx;
  const active = new Set(state.search.split(/\s+/).filter((w) => w.startsWith('#')).map((w) => w.toLowerCase()));
  $('tagCloud').replaceChildren(
    ...state.tags.slice(0, 40).map((tag) => {
      const token = '#' + tag;
      const on = active.has(token.toLowerCase());
      return el('button', {
        class: on ? 'on' : '',
        onclick: () => {
          const words = state.search.split(/\s+/).filter(Boolean);
          state.search = on ? words.filter((w) => w.toLowerCase() !== token.toLowerCase()).join(' ') : [...words, token].join(' ');
          $('search').value = state.search;
          $('search').dispatchEvent(new Event('change')); // its direction follows the text
          renderTree(ctx);
        },
      }, bdi(token));
    }),
  );
}
