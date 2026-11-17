// Calculated folder labels: the school year turns on September 1.

import test from 'node:test';
import assert from 'node:assert/strict';
// labels.js brings in ui.js, which listens on the document as it loads.
globalThis.document ??= { addEventListener() {} };
const { labelText, schoolYear } = await import('../ui/labels.js');

const rule = (over) => ({ kind: 'schoolGrade', firstGradeYear: 2024, classNumber: '3', ...over });

test('the school year begins in September', () => {
  assert.equal(schoolYear(new Date(2026, 7, 31)), 2025);
  assert.equal(schoolYear(new Date(2026, 8, 1)), 2026);
  assert.equal(schoolYear(new Date(2027, 0, 15)), 2026);
});

test('the grade moves up on September 1', () => {
  assert.equal(labelText(rule(), new Date(2026, 7, 31)), 'כיתה ב-3');
  assert.equal(labelText(rule(), new Date(2026, 8, 1)), 'כיתה ג-3');
});

test('before first grade and after twelfth have their own words', () => {
  assert.equal(labelText(rule(), new Date(2024, 0, 1)), 'כיתה גן-3');
  assert.equal(labelText(rule({ firstGradeYear: 2010 }), new Date(2022, 8, 1)), 'כיתה בוגרים-3');
  assert.equal(labelText(rule({ firstGradeYear: 2011 }), new Date(2022, 8, 1)), 'כיתה יב-3');
});

test('without a class number its placeholder and separator go', () => {
  const d = new Date(2026, 8, 1);
  assert.equal(labelText(rule({ classNumber: '' }), d), 'כיתה ג');
  assert.equal(labelText(rule({ classNumber: '  ', template: 'Class {grade} / {n}' }), d), 'Class ג');
});

test('templates fill every placeholder', () => {
  assert.equal(labelText(rule({ template: '{grade} ({gradeNum}) #{n}' }), new Date(2026, 8, 1)), 'ג (3) #3');
});

test('unknown kinds have no label', () => {
  assert.equal(labelText({ kind: 'other' }), null);
  assert.equal(labelText(null), null);
});
