// Calculated folder labels. A folder keeps its own (technical) name, e.g.
// "class of 2021", and can show a label computed from today's date, e.g.
// "כיתה ג-3" this school year and "כיתה ד-3" from next September.

import { t } from './i18n.js';
import { el, modal } from './ui.js';

export const GRADES = ['א', 'ב', 'ג', 'ד', 'ה', 'ו', 'ז', 'ח', 'ט', 'י', 'יא', 'יב'];
export const DEFAULT_TEMPLATE = 'כיתה {grade}-{n}';

/** The calendar year in which the school year containing `date` began (September–August). */
export const schoolYear = (date = new Date()) => (date.getMonth() >= 8 ? date.getFullYear() : date.getFullYear() - 1);

/** Grade text for a grade number: 1 → א … 12 → יב; before and after school have their own words. */
function gradeText(n) {
  if (n < 1) return t('gradeBefore');
  if (n > GRADES.length) return t('gradeAfter');
  return GRADES[n - 1];
}

/** The label a rule produces on `date`, or null for unknown rule kinds. */
export function labelText(rule, date = new Date()) {
  if (rule?.kind !== 'schoolGrade') return null;
  const grade = schoolYear(date) - rule.firstGradeYear + 1;
  const n = (rule.classNumber ?? '').trim();
  let s = rule.template || DEFAULT_TEMPLATE;
  // Without a class number, drop the placeholder and the separator before it.
  s = n ? s.replaceAll('{n}', n) : s.replace(/[\s\-–־/]*\{n\}/g, '');
  return s.replaceAll('{grade}', gradeText(grade)).replaceAll('{gradeNum}', String(grade)).trim();
}

/** Dialog to set or remove a folder's label. Resolves with a rule, `null` (remove) or undefined (cancel). */
export function editLabel(node) {
  const rule = node.label?.kind === 'schoolGrade' ? node.label : null;
  const year = schoolYear();
  const currentGrade = rule ? year - rule.firstGradeYear + 1 : 1;
  return new Promise((resolve) => {
    modal((d, done) => {
      const grade = el('select', {}, ...GRADES.map((g, i) => el('option', { value: i + 1, selected: i + 1 === currentGrade, text: g })));
      if (currentGrade < 1 || currentGrade > GRADES.length) grade.append(el('option', { value: currentGrade, selected: true, text: String(currentGrade) }));
      const num = el('input', { type: 'text', dir: 'auto', value: rule?.classNumber ?? '', placeholder: '3', style: 'width:6em' });
      const tpl = el('input', { type: 'text', dir: 'auto', value: rule?.template ?? DEFAULT_TEMPLATE });
      const preview = el('div', { class: 'labelPreview' });
      const build = () => ({ kind: 'schoolGrade', firstGradeYear: year - Number(grade.value) + 1, classNumber: num.value.trim(), template: tpl.value.trim() || DEFAULT_TEMPLATE });
      const update = () => {
        const r = build();
        const next = new Date(year + 1, 8, 1);
        preview.replaceChildren(
          el('div', {}, t('labelThisYear'), ' ', el('bdi', { dir: 'ltr', text: `(${year}–${String(year + 1).slice(2)})` }), ': ', el('b', {}, el('bdi', { text: labelText(r) }))),
          el('div', { class: 'muted' }, t('labelNextYear'), ' ', el('bdi', { text: labelText(r, next) })),
        );
      };
      for (const i of [grade, num, tpl]) i.addEventListener('input', update);
      update();
      d.append(
        el('h3', {}, t('folderLabelTitle'), ' ', el('bdi', { text: node.name })),
        el('p', { class: 'muted', text: t('folderLabelHelp') }),
        el('label', {}, t('labelPattern'), el('b', { text: t('pattern_schoolGrade') })),
        el('label', {}, t('gradeThisYear'), grade),
        el('label', {}, t('classNumber'), num),
        el('label', {}, t('labelTemplate'), tpl),
        el('p', { class: 'muted', text: t('labelTemplateHelp') }),
        preview,
        el('div', { class: 'buttons' },
          el('button', { class: 'primary', text: t('ok'), onclick: () => done({ value: build() }) }),
          rule ? el('button', { class: 'danger', text: t('removeLabel'), onclick: () => done({ value: null }) }) : null,
          el('button', { text: t('cancel'), onclick: () => done(null) }),
        ),
      );
    }).then((r) => resolve(r ? r.value : undefined));
  });
}
