// Small DOM helpers shared by the UI modules: element builder, toasts,
// dialogs and the context menu.

import { t, hasKey } from './i18n.js';

export const $ = (id) => document.getElementById(id);

export const el = (tag, props = {}, ...children) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') e.className = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (k === 'text') e.textContent = v;
    else if (k === 'html') { if (v != null) e.innerHTML = v; }
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c != null && c !== false) e.append(c);
  return e;
};

/** Wraps user text so Hebrew/English mixes render in the right order. */
export const bdi = (text) => el('bdi', { text });

/** Shows an error toast; short keys (e.g. "invalidName") are translated. */
export function error(msg) {
  const raw = msg?.message ?? msg;
  const key = 'err_' + raw;
  const text = hasKey(key) ? t(key) : String(raw);
  const toast = el('div', { class: 'toast' }, el('b', { text: '⚠' }), el('span', { text }), el('button', { class: 'link', text: '×', onclick: () => toast.remove() }));
  $('toasts').append(toast);
  while ($('toasts').children.length > 4) $('toasts').firstChild.remove();
  setTimeout(() => toast.remove(), 12000);
}

export function modal(build) {
  return new Promise((resolve) => {
    const d = $('dialog');
    d.replaceChildren();
    let settled = false;
    const done = (v) => {
      if (settled) return;
      settled = true;
      d.close();
      resolve(v);
    };
    build(d, done);
    d.onclose = () => done(null);
    d.showModal();
  });
}

export function prompt(title, value = '') {
  return modal((d, done) => {
    const input = el('input', { type: 'text', dir: 'auto', value });
    input.addEventListener('keydown', (e) => e.key === 'Enter' && done(input.value));
    d.append(
      el('h3', { text: title }),
      input,
      el('div', { class: 'buttons' }, el('button', { class: 'primary', text: t('ok'), onclick: () => done(input.value) }), el('button', { text: t('cancel'), onclick: () => done(null) })),
    );
    setTimeout(() => input.select(), 0);
  });
}

export function confirmDialog(text, sub, action = t('delete')) {
  return modal((d, done) => {
    d.append(
      el('p', {}, bdi(text)),
      sub ? el('p', { class: 'muted', text: sub }) : null,
      el('div', { class: 'buttons' }, el('button', { class: 'danger', text: action, onclick: () => done(true) }), el('button', { text: t('cancel'), onclick: () => done(false) })),
    );
  });
}

const escapeHtml = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

/** An ✕ at the top of every menu: easy to hit with a pen (no right click or Esc needed). */
const closeRow = () =>
  el('div', { class: 'menuHead' }, el('button', {
    class: 'menuClose',
    title: t('closeMenu'),
    html: '<svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18"/></svg>',
    onclick: () => hideMenu(),
  }));

export function showMenu(x, y, items) {
  const m = $('menu');
  m.replaceChildren(
    closeRow(),
    ...items.filter(Boolean).map((it) =>
      it === '-'
        ? el('hr')
        : it.label
          ? el('div', { class: 'label', text: it.label })
          : el('button', {
              class: (it.danger ? 'danger' : '') + (it.on ? ' on' : ''),
              disabled: it.disabled,
              // `icon`: an SVG shown before the text.
              html: it.icon ? `${it.icon}<span>${escapeHtml(it.text ?? '')}</span>` : it.html,
              onclick: () => { hideMenu(); it.action(); },
            }, it.icon ? '' : it.text ?? ''),
    ),
  );
  place(m, x, y);
}

/** Shows `content` (an element) in the popup menus use, at (x, y). */
export function showPanel(x, y, content) {
  const m = $('menu');
  m.replaceChildren(closeRow(), content);
  place(m, x, y);
}

function place(m, x, y) {
  m.hidden = false;
  const r = m.getBoundingClientRect();
  m.style.left = Math.max(4, Math.min(x, innerWidth - r.width - 4)) + 'px';
  m.style.top = Math.max(4, Math.min(y, innerHeight - r.height - 4)) + 'px';
}

export const hideMenu = () => ($('menu').hidden = true);
document.addEventListener('pointerdown', (e) => !$('menu').contains(e.target) && hideMenu());
document.addEventListener('keydown', (e) => e.key === 'Escape' && hideMenu());
