// UI strings: Hebrew and English have the same keys and arguments, and every
// key the interface names exists.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dicts } from '../ui/i18n.js';

const ui = new URL('../ui/', import.meta.url);
const read = (name) => readFileSync(new URL(name, ui), 'utf8');
const args = (s) => [...new Set(s.match(/\{\d\}/g) ?? [])].sort();

test('Hebrew and English have the same keys', () => {
  const he = Object.keys(dicts.he), en = Object.keys(dicts.en);
  assert.deepEqual(he.filter((k) => !(k in dicts.en)), [], 'missing in English');
  assert.deepEqual(en.filter((k) => !(k in dicts.he)), [], 'missing in Hebrew');
});

test('translations take the same arguments', () => {
  const differ = Object.keys(dicts.en).filter((k) => k in dicts.he && args(dicts.he[k]).join() !== args(dicts.en[k]).join());
  assert.deepEqual(differ, []);
});

// Keys built at run time, such as t('style_' + style), aren't checked.
test('every key the interface uses is translated', () => {
  const used = new Set();
  for (const f of readdirSync(ui).filter((f) => f.endsWith('.js'))) {
    for (const [, k] of read(f).matchAll(/\bt\(\s*['"]([A-Za-z0-9_]+)['"]\s*[,)]/g)) used.add(k);
  }
  for (const [, k] of read('index.html').matchAll(/data-i18n(?:-title|-placeholder)?="([^"]+)"/g)) used.add(k);
  assert.ok(used.size > 50);
  assert.deepEqual([...used].filter((k) => !(k in dicts.en) || !(k in dicts.he)), []);
});
