// OneNote import panel. OneNote is only a source: pages are copied into the
// page folder as new files and are never written back. Imports go into the
// folder selected in the tree.

import { t } from './i18n.js';
import { $, el, bdi, error } from './ui.js';

const { invoke } = window.__TAURI__.core;

/** ctx: { state, saveSettings(), folderLabel(path) } */
export function renderOneNote(ctx) {
  const { state } = ctx;
  const o = state.onenote;
  const panel = $('onenote');
  if (!o.open) return;
  const s = state.settings;
  const parts = [el('div', { class: 'row' }, el('h3', { text: t('importFromOneNote') }), el('span', { class: 'spacer' }),
    el('button', { class: 'icon', text: '×', title: t('close'), onclick: () => ctx.toggle(false) }))];

  if (!state.signedIn && !o.deviceCode) {
    const id = el('input', { type: 'text', value: s.auth.clientId, dir: 'ltr' });
    const tenant = el('input', { type: 'text', value: s.auth.tenant, dir: 'ltr' });
    parts.push(
      el('p', { class: 'muted', text: t('importHelp') }),
      el('details', {},
        el('summary', { text: t('connection') }),
        el('p', { class: 'muted', text: t('connectionHelp') }),
        el('div', { class: 'field' }, el('span', { text: t('clientId') }), id, el('span', { text: t('tenant') }), tenant),
        el('div', { class: 'row' }, el('button', {
          text: t('apply'),
          onclick: () => {
            s.auth.clientId = id.value.trim();
            s.auth.tenant = tenant.value.trim() || 'common';
            ctx.saveSettings();
            renderOneNote(ctx);
          },
        })),
      ),
    );
  }
  if (o.deviceCode) {
    const dc = o.deviceCode;
    parts.push(
      el('h3', { text: t('signInTitle') }),
      el('div', {}, t('signInStep1', ''), el('bdi', { text: dc.verificationUri })),
      el('div', { class: 'row' }, t('signInStep2'), el('span', { class: 'code', text: dc.userCode }),
        el('button', { text: t('copy'), onclick: () => navigator.clipboard?.writeText(dc.userCode) })),
      el('div', { class: 'row' },
        el('button', { class: 'primary', text: t('openBrowser'), onclick: () => invoke('open_url', { url: dc.verificationUri }).catch(error) }),
        el('button', { text: t('cancel'), onclick: () => { invoke('onenote_cancel'); o.deviceCode = null; renderOneNote(ctx); } }),
        el('span', { class: 'spinner' })),
    );
  } else if (!state.signedIn) {
    parts.push(el('div', { class: 'row' }, el('button', { class: 'primary', text: t('signIn'), disabled: !s.auth.clientId || state.busy, onclick: () => invoke('onenote_sign_in') })));
  } else {
    parts.push(
      el('div', { class: 'dest' }, el('span', { text: t('importInto') }), ' ', el('b', {}, bdi(ctx.folderLabel(state.folder))),
        el('div', { class: 'muted', text: t('importIntoHint') })),
      el('div', { class: 'row' },
        el('button', { text: '⟳ ' + t('refresh'), disabled: state.busy, onclick: () => invoke('onenote_list_notebooks') }),
        el('button', { text: t('signOut'), onclick: () => invoke('onenote_sign_out') }),
        state.busy ? el('span', { class: 'spinner' }) : null,
        state.busy ? el('button', { class: 'link', text: t('stop'), onclick: () => invoke('onenote_cancel') }) : null),
    );
    const list = el('div', { class: 'list' });
    if (!o.notebooks.length) list.append(el('div', { class: 'muted', text: state.busy ? t('loading') : t('noNotebooks') }));
    for (const nb of o.notebooks) {
      const d = el('details', { open: o.selectedNotebook === nb.id });
      const sum = el('summary', { class: o.selectedNotebook === nb.id ? 'selected' : '' }, bdi(nb.name), el('span', { class: 'spacer' }),
        el('button', {
          class: 'link', text: t('importNotebook'), disabled: state.busy,
          onclick: (e) => { e.preventDefault(); e.stopPropagation(); invoke('onenote_import_notebook', { notebook: nb, folder: state.folder }); },
        }));
      sum.addEventListener('click', () => { o.selectedNotebook = nb.id; setTimeout(() => renderOneNote(ctx), 0); });
      d.append(sum, el('div', { class: 'children' },
        ...nb.sections.map((sec) =>
          el('button', {
            class: 'pageItem' + (o.selectedSection?.id === sec.id ? ' on' : ''),
            onclick: () => {
              o.selectedNotebook = nb.id;
              o.selectedSection = sec;
              o.pages = [];
              invoke('onenote_list_pages', { sectionId: sec.id });
              renderOneNote(ctx);
            },
          }, bdi(sec.name)),
        )));
      list.append(d);
    }
    parts.push(list);

    const sel = o.selectedSection;
    if (!sel) parts.push(el('p', { class: 'muted', text: t('selectSectionHint') }));
    else {
      parts.push(
        el('div', { class: 'row' }, el('h3', {}, bdi(sel.name)), el('span', { class: 'spacer' }),
          el('button', { text: t('importSection'), disabled: state.busy, onclick: () => invoke('onenote_import_section', { section: sel, folder: state.folder }) })),
        el('div', { class: 'list' },
          o.pages.length ? null : el('div', { class: 'muted', text: state.busy ? t('loading') : t('noPages') }),
          ...o.pages.map((p) =>
            el('div', { class: 'pageRow' },
              el('button', {
                text: t('import'),
                disabled: state.busy,
                onclick: () => invoke('onenote_import_page', { sectionId: sel.id, page: p, folder: state.folder, open: true }),
              }),
              bdi(p.title || t('untitled')),
              p.lastModified ? el('small', { text: p.lastModified.slice(0, 10) }) : null,
            ),
          ),
        ),
      );
    }
  }
  panel.replaceChildren(...parts);
}
