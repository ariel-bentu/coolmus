// Palm rejection, driven by fake pointer events on a stand-in window and a
// clock the test moves forward.

import test from 'node:test';
import assert from 'node:assert/strict';

let now = 0;
performance.now = () => now;
globalThis.window = new EventTarget();
const classes = new Set();
globalThis.document = { body: { classList: { add: (c) => classes.add(c), remove: (c) => classes.delete(c) } } };
const { bindPalmRejection, isPalm, onPenNear, penAround } = await import('../ui/palm.js');
bindPalmRejection();

let nextId = 1;
const pointer = (type, pointerType, over = {}) => Object.assign(new Event(type, { cancelable: true }), { pointerType, pointerId: nextId, width: 20, height: 20, relatedTarget: null, ...over });
const send = (type, pointerType, over) => {
  const e = pointer(type, pointerType, over);
  window.dispatchEvent(e);
  return e;
};
const finger = (over) => pointer('pointerdown', 'touch', over);
/** The pen leaves, and enough time passes for it to count as gone. */
const penGone = () => {
  send('pointerout', 'pen');
  now += 5000;
};

test('a fingertip with no pen around is a finger', () => {
  assert.ok(!penAround());
  assert.ok(!isPalm(finger()));
  assert.ok(!isPalm(pointer('pointerdown', 'pen')));
  assert.ok(!isPalm(pointer('pointerdown', 'mouse', { width: 200, height: 200 })));
});

test('a touch too big for a fingertip is always a palm', () => {
  assert.ok(isPalm(finger({ width: 80, height: 40 })));
});

test('while the pen hovers, and a second after it leaves, touch is a palm', () => {
  send('pointermove', 'pen');
  assert.ok(penAround());
  assert.ok(classes.has('penNear'));
  assert.ok(isPalm(finger()));
  now += 4000; // still hovering, sending nothing yet
  assert.ok(isPalm(finger()));
  send('pointerout', 'pen');
  now += 900;
  assert.ok(isPalm(finger()));
  now += 200;
  assert.ok(!isPalm(finger()));
});

test('a hovering pen that goes silent counts as gone', () => {
  send('pointerover', 'pen');
  now += 5100;
  assert.ok(!penAround());
  penGone();
});

test('a hand that landed before the pen came near becomes a palm', () => {
  let near = 0;
  onPenNear(() => near++);
  nextId = 10;
  send('pointerdown', 'touch');
  assert.ok(!isPalm(finger()), 'a finger until the pen shows up');
  send('pointermove', 'pen');
  assert.equal(near, 1);
  penGone();
  assert.ok(isPalm(finger()), 'still a palm after the pen leaves');
  send('pointerup', 'touch');
  assert.ok(!isPalm(finger()));
  nextId = 11;
});

test("a palm's tap is dropped, a pen's click goes through", () => {
  nextId = 20;
  send('pointerdown', 'touch', { width: 100, height: 100 });
  const tap = send('click', 'touch');
  assert.ok(tap.defaultPrevented);
  assert.ok(!send('click', 'pen').defaultPrevented);
  send('pointerup', 'touch');
  now += 400;
  assert.ok(send('click', 'touch').defaultPrevented, 'the click that follows the release');
  now += 200;
  assert.ok(!send('click', 'touch').defaultPrevented);
});
