// Getting pictures onto a page: from a file (or the photo gallery), from the
// camera, or from the clipboard. Large photos are scaled down and re-encoded
// so page files stay small enough to save and sync quickly.

import { t } from './i18n.js';

const MAX_SIDE = 1600;
const KEEP_BYTES = 600_000; // small PNG/JPEG/GIF/WebP files are kept as they are

const readAsDataUrl = (blob) =>
  new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(r.result);
    r.onerror = () => reject(r.error);
    r.readAsDataURL(blob);
  });

const decode = (src) =>
  new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error('imageUnreadable'));
    img.src = src;
  });

/** Scales `source` (image, video or canvas) to fit MAX_SIDE and encodes it. */
function encode(source, w, h, type = 'image/jpeg') {
  const s = Math.min(1, MAX_SIDE / Math.max(w, h));
  const c = document.createElement('canvas');
  c.width = Math.round(w * s);
  c.height = Math.round(h * s);
  const ctx = c.getContext('2d');
  if (type === 'image/jpeg') {
    ctx.fillStyle = '#ffffff'; // JPEG has no transparency
    ctx.fillRect(0, 0, c.width, c.height);
  }
  ctx.drawImage(source, 0, 0, c.width, c.height);
  return { src: c.toDataURL(type, 0.85), width: c.width, height: c.height };
}

/** Turns an image file or blob into { src (data URL), width, height }. */
export async function fromBlob(blob) {
  if (!blob.type.startsWith('image/')) throw new Error('notAnImage');
  const original = await readAsDataUrl(blob);
  const img = await decode(original);
  const w = img.naturalWidth, h = img.naturalHeight;
  if (blob.size <= KEEP_BYTES && Math.max(w, h) <= MAX_SIDE && /image\/(png|jpeg|gif|webp)/.test(blob.type)) {
    return { src: original, width: w, height: h };
  }
  // PNGs may be diagrams with transparency; photos become JPEG.
  return encode(img, w, h, blob.type === 'image/png' && blob.size < 3_000_000 ? 'image/png' : 'image/jpeg');
}

/** Opens the system file picker (on tablets this includes the photo gallery). */
export function pickFile() {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/*';
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.oncancel = () => resolve(null);
    input.click();
  });
}

/**
 * Shows a camera preview in `dialog` and resolves with the captured picture,
 * or null if cancelled.
 */
export async function takePhoto(dialog) {
  if (!navigator.mediaDevices?.getUserMedia) throw new Error('noCamera');
  let facing = 'environment';
  let stream;
  const video = document.createElement('video');
  video.autoplay = true;
  video.playsInline = true;
  video.muted = true;
  video.className = 'cameraView';
  const start = async () => {
    stream?.getTracks().forEach((tr) => tr.stop());
    stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: facing, width: { ideal: 1920 }, height: { ideal: 1080 } }, audio: false });
    video.srcObject = stream;
  };
  try {
    await start();
  } catch (e) {
    throw new Error(e?.name === 'NotAllowedError' ? 'cameraDenied' : 'noCamera');
  }
  const cams = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'videoinput');
  return new Promise((resolve) => {
    const finish = (result) => {
      stream?.getTracks().forEach((tr) => tr.stop());
      dialog.onclose = null;
      dialog.close();
      resolve(result);
    };
    const button = (text, cls, action) => {
      const b = document.createElement('button');
      b.textContent = text;
      if (cls) b.className = cls;
      b.onclick = action;
      return b;
    };
    const buttons = document.createElement('div');
    buttons.className = 'buttons';
    buttons.append(
      button(t('capture'), 'primary', () => finish(encode(video, video.videoWidth, video.videoHeight))),
      cams.length > 1 ? button(t('switchCamera'), '', () => { facing = facing === 'environment' ? 'user' : 'environment'; start().catch(() => {}); }) : '',
      button(t('cancel'), '', () => finish(null)),
    );
    const title = document.createElement('h3');
    title.textContent = t('takePhoto');
    dialog.replaceChildren(title, video, buttons);
    dialog.onclose = () => finish(null);
    dialog.showModal();
  });
}
