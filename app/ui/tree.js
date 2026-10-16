// The sidebar: the page folder as a tree of folders and pages, plus search
// and the tag list. Paths are relative to the page folder, "/"-separated;
// "" is the page folder itself.

import { t } from './i18n.js';
import { $, el, bdi, showMenu } from './ui.js';
import { labelText } from './labels.js';

const FOLDER = '<svg viewBox="0 0 24 24"><path d="M3 6a1 1 0 011-1h5l2 2h9a1 1 0 011 1v10a1 1 0 01-1 1H4a1 1 0 01-1-1z"/></svg>';
const PAGE = '<svg viewBox="0 0 24 24"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/><path d="M9 14c2-3 3 1 6-2"/></svg>';
const CHEVRON = '<svg viewBox="0 0 24 24" class="chev"><path d="M9 6l6 6-6 6"/></svg>';
const DRAG_TYPE = 'text/x-sketcher-path';

export const parentOf = (path) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');
export const baseName = (path) => path.slice(path.lastIndexOf('/') + 1).replace(/\.sketcher$/i, '');

/** All folders, depth-first, as { path, name, depth } (name: the label if there is one). */
export function allFolders(nodes, depth = 1, out = []) {
  for (const n of nodes) {
    if (n.kind !== 'folder') continue;
    out.push({ path: n.path, name: labelText(n.label) || n.name, depth });
    allFolders(n.children ?? [], depth + 1, out);
  }
  return out;
}

/**
 * Renders the tree into #tree. `ctx` holds the app state and actions:
 * { state, open(path), newPage(folder), newFolder(parent), rename(node),
 *   move(node), moveTo(path, folder), remove(node), reveal() }
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
        el('div', { class: 'result' + (h.path === cur ? ' on' : ''), onclick: () => ctx.open(h.path) },
          bdi(h.name),
          el('small', {}, bdi(h.folder ? h.folder.replaceAll('/', ' › ') : t('rootFolder')), ' ', bdi(h.tags.map((x) => '#' + x).join(' '))),
        ),
      ),
    );
    return;
  }

  const rows = [];
  const rootName = state.root.split(/[\\/]/).filter(Boolean).pop() || t('rootFolder');
  rows.push(folderRow(ctx, { kind: 'folder', path: '', name: rootName, children: state.tree }, 0, true));
  const walk = (nodes, depth) => {
    for (const n of nodes) {
      if (n.kind === 'folder') {
        rows.push(folderRow(ctx, n, depth, state.openFolders.has(n.path)));
        if (state.openFolders.has(n.path)) walk(n.children ?? [], depth + 1);
      } else rows.push(pageRow(ctx, n, depth));
    }
  };
  walk(state.tree, 1);
  if (!state.tree.length) rows.push(el('p', { class: 'hint', text: t('emptyFolder') }));
  tree.replaceChildren(...rows);
}

function indent(depth) {
  return `padding-inline-start: ${6 + depth * 14}px`;
}

function folderRow(ctx, node, depth, open) {
  const { state } = ctx;
  const isRoot = node.path === '';
  const row = el('div', {
    class: 'row folder' + (state.folder === node.path ? ' on' : '') + (open ? ' open' : '') + (isRoot ? ' root' : ''),
    style: indent(depth),
    draggable: !isRoot,
    title: isRoot ? state.root : node.path,
    'data-path': node.path,
  });
  const toggle = (on) => {
    if (on) state.openFolders.add(node.path);
    else state.openFolders.delete(node.path);
    ctx.saveOpenFolders();
  };
  if (!isRoot) {
    const chev = el('span', { class: 'chevBtn', html: CHEVRON });
    chev.addEventListener('click', (e) => {
      e.stopPropagation();
      toggle(!open);
      renderTree(ctx);
    });
    row.append(chev);
  }
  // A calculated label (e.g. this year's class) is shown in place of the name.
  const label = labelText(node.label);
  row.append(el('span', { class: 'ico', html: FOLDER }), bdi(label || node.name));
  if (label) row.append(el('small', { class: 'techName' }, bdi(node.name)));
  // Clicking selects the folder (where new pages and imports go) and opens it;
  // clicking the selected, open folder again closes it.
  row.addEventListener('click', () => {
    const wasSelected = state.folder === node.path;
    state.folder = node.path;
    if (!isRoot) toggle(!(open && wasSelected));
    renderTree(ctx);
  });
  row.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    state.folder = node.path;
    renderTree(ctx);
    showMenu(e.clientX, e.clientY, [
      { text: t('newPageHere'), action: () => ctx.newPage(node.path) },
      { text: t('newFolderHere'), action: () => ctx.newFolder(node.path) },
      '-',
      !isRoot && { text: t('rename'), action: () => ctx.rename(node) },
      !isRoot && { text: t('moveTo'), action: () => ctx.move(node) },
      !isRoot && { text: t('folderLabelMenu'), action: () => ctx.editLabel(node) },
      isRoot && { text: t('revealFolder'), action: () => ctx.reveal() },
      !isRoot && '-',
      !isRoot && { text: t('delete'), danger: true, action: () => ctx.remove(node) },
    ]);
  });
  dropTarget(ctx, row, node.path);
  if (!isRoot) dragSource(row, node.path);
  return row;
}

function pageRow(ctx, node, depth) {
  const { state } = ctx;
  const row = el('div', {
    class: 'row page' + (node.path === state.current?.path ? ' on' : ''),
    style: indent(depth),
    draggable: true,
    title: node.tags?.length ? node.tags.map((x) => '#' + x).join(' ') : node.name,
  }, el('span', { class: 'ico', html: PAGE }), bdi(node.name));
  row.addEventListener('click', () => ctx.open(node.path));
  row.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    showMenu(e.clientX, e.clientY, [
      { text: t('rename'), action: () => ctx.rename(node) },
      { text: t('moveTo'), action: () => ctx.move(node) },
      '-',
      { text: t('delete'), danger: true, action: () => ctx.remove(node) },
    ]);
  });
  dragSource(row, node.path);
  return row;
}

function dragSource(row, path) {
  row.addEventListener('dragstart', (e) => {
    e.dataTransfer.setData(DRAG_TYPE, path);
    e.dataTransfer.effectAllowed = 'move';
  });
}

function dropTarget(ctx, row, folder) {
  row.addEventListener('dragover', (e) => {
    if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
    e.preventDefault();
    row.classList.add('drop');
  });
  row.addEventListener('dragleave', () => row.classList.remove('drop'));
  row.addEventListener('drop', (e) => {
    row.classList.remove('drop');
    const path = e.dataTransfer.getData(DRAG_TYPE);
    if (!path) return;
    e.preventDefault();
    ctx.moveTo(path, folder);
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
          renderTree(ctx);
        },
      }, bdi(token));
    }),
  );
}
