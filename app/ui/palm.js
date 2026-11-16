// Palm rejection for the whole window. A Windows pen reports itself while it
// hovers above the screen, so a hand resting on the screen while the pen is
// near, or just after it leaves, is taken for a palm rather than a finger; so
// is a touch too big for a fingertip. The canvas asks isPalm() for each touch;
// elsewhere (the explorer, menus) a palm's taps and long presses are dropped
// here, and while the pen is near, touch doesn't scroll (body.penNear).

const GRACE_MS = 1000; // the pen counts as near this long after it leaves
const STILL_MS = 5000; // a hovering pen that sends nothing for this long has gone
const PALM_SIZE = 60; // CSS px across; a fingertip is about half that

let hovering = false;
let penSeen = -Infinity;
let expire = 0;
let palmUntil = 0; // taps from touch are dropped until then
const touches = new Set(); // touch pointers now on the screen
const palms = new Set(); // those taken for a palm
const listeners = [];

/** True if the pen is hovering or touching, or was a moment ago. */
export const penAround = () => performance.now() - penSeen < (hovering ? STILL_MS : GRACE_MS);

/** True if a touch pointer event is a palm rather than a finger. */
export const isPalm = (e) => e.pointerType === 'touch' && (palms.has(e.pointerId) || penAround() || Math.max(e.width, e.height) > PALM_SIZE);

/** Calls `fn` when the pen comes near the screen. */
export const onPenNear = (fn) => listeners.push(fn);

function pen(near) {
  const was = penAround();
  hovering = near;
  penSeen = performance.now();
  document.body.classList.add('penNear');
  clearTimeout(expire);
  expire = setTimeout(() => document.body.classList.remove('penNear'), near ? STILL_MS : GRACE_MS);
  if (near && !was) {
    // A hand that landed before the pen came near was a palm all along.
    for (const id of touches) palms.add(id);
    for (const fn of listeners) fn();
  }
}

export function bindPalmRejection() {
  const opts = { capture: true, passive: true };
  for (const type of ['pointerover', 'pointermove', 'pointerdown']) {
    window.addEventListener(type, (e) => e.pointerType === 'pen' && pen(true), opts);
  }
  // Out of hover range, or out of the window: relatedTarget is then null.
  window.addEventListener('pointerout', (e) => e.pointerType === 'pen' && !e.relatedTarget && pen(false), opts);
  window.addEventListener('pointerdown', (e) => {
    if (e.pointerType !== 'touch') return;
    if (isPalm(e)) palms.add(e.pointerId);
    touches.add(e.pointerId);
  }, opts);
  const lift = (e) => {
    touches.delete(e.pointerId);
    if (palms.delete(e.pointerId)) palmUntil = performance.now() + 500; // the click comes after the release
  };
  window.addEventListener('pointerup', lift, opts);
  window.addEventListener('pointercancel', lift, opts);
  // A palm's tap or long press doesn't open pages or menus. A pen or mouse
  // click while the palm rests still goes through.
  const drop = (e) => {
    const touch = e.pointerType === 'touch' || (!e.pointerType && e.sourceCapabilities?.firesTouchEvents);
    if (touch && (palms.size || performance.now() < palmUntil)) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  for (const type of ['click', 'dblclick', 'contextmenu', 'dragstart']) window.addEventListener(type, drop, { capture: true });
}
