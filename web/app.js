// The browser front end. Every rendering decision lives in the Rust core; this
// file only moves pixels between the core and a canvas, and keeps the editor's
// state honest about what has actually been prepared.

import init, { Session, effects } from './pkg/pixelgen_wasm.js';

const el = (id) => document.getElementById(id);
const view = el('view');
const overlay = el('overlay');
const vctx = view.getContext('2d');
const octx = overlay.getContext('2d');

const state = {
  session: null,
  /** Frames are rendered on demand and kept, so scrubbing is instant. */
  cache: [],
  frame: 0,
  playing: false,
  timer: null,
  /** Index into session.layers whose mask is drawn over the frame, or null. */
  maskLayer: null,
  shape: 'polygon',
  points: [],
  name: 'image',
};

await init();

// ---------------------------------------------------------------- image input

async function open(file) {
  const bitmap = await createImageBitmap(file);
  const c = new OffscreenCanvas(bitmap.width, bitmap.height);
  const cx = c.getContext('2d', { willReadFrequently: true });
  cx.drawImage(bitmap, 0, 0);
  const data = cx.getImageData(0, 0, bitmap.width, bitmap.height).data;

  stop();
  state.session = new Session(data, bitmap.width, bitmap.height);
  state.name = file.name || 'image';
  say(`${file.name || 'image'} - ${bitmap.width}x${bitmap.height}`);

  // A scene already in the box is almost always meant for this image too -
  // the usual move is to try the same scene on a second photograph. With an
  // empty box there is nothing to preserve, so the photograph goes straight to
  // the scene its own analysis suggests.
  if (el('yaml').value.trim()) apply();
  else starter();
}

// Replaces the scene with the one this photograph suggests.
function starter() {
  try {
    el('yaml').value = state.session.starter(state.name);
    apply();
  } catch (e) {
    fail(e);
  }
}

el('drop').addEventListener('click', () => el('file').click());

el('file').addEventListener('change', (e) => {
  if (e.target.files[0]) open(e.target.files[0]).catch(fail);
});

for (const type of ['dragover', 'drop']) {
  document.addEventListener(type, (e) => {
    e.preventDefault();
    if (type === 'drop' && e.dataTransfer.files[0]) open(e.dataTransfer.files[0]).catch(fail);
  });
}

// ---------------------------------------------------------------- the scene

let pending = null;
el('yaml').addEventListener('input', () => {
  clearTimeout(pending);
  // Long enough that a scene is not re-prepared on every keystroke, short
  // enough that it still feels like the preview is following the text.
  pending = setTimeout(apply, 300);
});

function apply() {
  if (!state.session) return;
  try {
    state.session.set_scene(el('yaml').value);
    el('error').hidden = true;
  } catch (e) {
    // A scene that fails to prepare leaves the previous one on screen. The
    // alternative - blanking the canvas - throws away the thing being edited
    // toward every time a half-typed line is momentarily invalid.
    el('error').hidden = false;
    el('error').textContent = e.message ?? String(e);
    return;
  }

  const s = state.session;
  view.width = overlay.width = s.width;
  view.height = overlay.height = s.height;
  el('viewport').classList.remove('empty');

  fit();
  state.cache = new Array(s.frames);
  state.frame = Math.min(state.frame, s.frames - 1);
  el('scrub').max = s.frames - 1;
  el('scrub').value = state.frame;
  for (const id of ['play', 'scrub']) el(id).disabled = false;
  el('save').inert = false;

  swatches(s.palette);
  layerList(s.layers, s.layer_types);
  if (state.maskLayer !== null && state.maskLayer >= s.layers.length) state.maskLayer = null;

  stat('grid', `${s.width}x${s.height}`);
  stat('loop', `${s.frames}f @ ${s.fps}`);
  stat('colors', s.palette.length);
  stat('layers', s.layers.length || '-');
  draw();
}

function stat(name, value) {
  document.querySelector(`[data-stat="${name}"]`).textContent = value;
}

// Scales the plate to fill the space it has.
//
// Whole multiples while the image fits: nearest-neighbor at 3.4x gives some
// cells three screen pixels and some four, which reads as a grid that cannot
// hold its rhythm. Only when the frame is larger than the viewport does it
// fall back to a fractional fit, where there is no whole ratio to have.
function fit() {
  const s = state.session;
  if (!s?.width) return;
  const box = el('viewport').getBoundingClientRect();
  const room = { w: box.width - 32, h: box.height - 32 };
  const exact = Math.min(room.w / s.width, room.h / s.height);
  const scale = exact >= 1 ? Math.floor(exact) : exact;
  for (const c of [view, overlay]) {
    c.style.width = `${s.width * scale}px`;
    c.style.height = `${s.height * scale}px`;
  }
}

// The plate is sized against the viewport, so a resize has to re-fit it.
let refit = null;
window.addEventListener('resize', () => {
  clearTimeout(refit);
  refit = setTimeout(fit, 100);
});

function swatches(hexes) {
  el('ncolors').textContent = hexes.length;
  el('palette').replaceChildren(...hexes.map((h) => {
    const i = document.createElement('i');
    i.style.background = h;
    i.title = h;
    return i;
  }));
}

function layerList(names, types = []) {
  el('layers').replaceChildren(...names.map((name, i) => {
    const li = document.createElement('li');
    li.append(name);
    if (types[i]) {
      const chip = document.createElement('span');
      chip.className = 'type';
      chip.textContent = types[i];
      li.append(chip);
    }
    li.setAttribute('aria-pressed', state.maskLayer === i);
    li.title = 'Draw this layer’s resolved mask over the frame';
    li.addEventListener('click', () => {
      state.maskLayer = state.maskLayer === i ? null : i;
      el('showmask').checked = state.maskLayer !== null;
      layerList(names, types);
      draw();
    });
    return li;
  }));
  if (!names.length) {
    const li = document.createElement('li');
    li.className = 'none';
    li.textContent = 'No enabled layers - the loop will be a still image.';
    el('layers').append(li);
  }
}

// ---------------------------------------------------------------- playback

function frame(i) {
  if (!state.cache[i]) {
    const rgba = state.session.frame(i, 1);
    state.cache[i] = new ImageData(new Uint8ClampedArray(rgba), state.session.width, state.session.height);
  }
  return state.cache[i];
}

function draw() {
  if (!state.session || !state.session.frames) return;
  vctx.putImageData(frame(state.frame), 0, 0);
  el('counter').textContent = `${state.frame + 1}/${state.session.frames}`;
  drawOverlay();
}

function drawOverlay() {
  octx.clearRect(0, 0, overlay.width, overlay.height);
  if (state.maskLayer !== null && el('showmask').checked) {
    paintCoverage(state.session.layer_mask(state.maskLayer), 0.55);
    return;
  }
  drawShape();
}

// The one saturated thing the page adds to the image, so it is the page's own
// accent rather than a color invented here - and it is read from the
// stylesheet, which is what keeps it right in both themes.
function token(name) {
  const hex = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function tick() {
  state.frame = (state.frame + 1) % state.session.frames;
  el('scrub').value = state.frame;
  draw();
}

function play() {
  if (!state.session?.frames) return;
  state.playing = true;
  el('play').classList.add('playing');
  clearInterval(state.timer);
  state.timer = setInterval(tick, 1000 / Math.max(1, state.session.fps));
}

function stop() {
  state.playing = false;
  clearInterval(state.timer);
  el('play').classList.remove('playing');
}

el('play').addEventListener('click', () => (state.playing ? stop() : play()));
el('scrub').addEventListener('input', (e) => {
  stop();
  state.frame = +e.target.value;
  draw();
});
el('showmask').addEventListener('change', (e) => {
  if (e.target.checked && state.maskLayer === null && state.session?.layers.length) {
    state.maskLayer = 0;
    layerList(state.session.layers, state.session.layer_types);
  }
  drawOverlay();
});

// ---------------------------------------------------------------- mask drawing

const tabs = document.querySelectorAll('.tabs button');
tabs.forEach((b) => b.addEventListener('click', () => {
  tabs.forEach((o) => o.setAttribute('aria-pressed', o === b));
  document.querySelectorAll('.tab').forEach((t) => {
    t.hidden = t.dataset.tab !== b.dataset.tab;
  });
  el('viewport').classList.toggle('drawing', b.dataset.tab === 'draw');
}));

for (const shape of ['polygon', 'rect', 'ellipse']) {
  el(`shape-${shape}`).addEventListener('click', () => {
    state.shape = shape;
    state.points = [];
    ['polygon', 'rect', 'ellipse'].forEach((s) => el(`shape-${s}`).setAttribute('aria-pressed', s === shape));
    emit();
    drawOverlay();
  });
}

el('clear').addEventListener('click', () => {
  state.points = [];
  emit();
  drawOverlay();
});

overlay.addEventListener('click', (e) => {
  if (!el('viewport').classList.contains('drawing') || !state.session) return;
  const r = overlay.getBoundingClientRect();
  const p = { x: clamp01((e.clientX - r.left) / r.width), y: clamp01((e.clientY - r.top) / r.height) };
  // A box needs two corners and nothing more, so a third click starts a new
  // one rather than silently ignoring it.
  if (state.shape !== 'polygon' && state.points.length >= 2) state.points = [];
  state.points.push(p);
  emit();
  drawOverlay();
});

function drawShape() {
  const pts = state.points;
  if (!pts.length) return;
  const W = overlay.width, H = overlay.height;
  octx.strokeStyle = `rgb(${token('--chalk').join(',')})`;
  octx.lineWidth = 1;
  octx.beginPath();
  if (state.shape === 'polygon') {
    pts.forEach((p, i) => octx[i ? 'lineTo' : 'moveTo'](p.x * W, p.y * H));
    if (pts.length > 2) octx.closePath();
  } else if (pts.length === 2) {
    const b = box(pts);
    if (state.shape === 'rect') {
      octx.rect(b.x * W, b.y * H, b.w * W, b.h * H);
    } else {
      octx.ellipse((b.x + b.w / 2) * W, (b.y + b.h / 2) * H, (b.w / 2) * W, (b.h / 2) * H, 0, 0, Math.PI * 2);
    }
  }
  octx.stroke();
  octx.fillStyle = `rgb(${token('--accent').join(',')})`;
  for (const p of pts) octx.fillRect(p.x * W - 1, p.y * H - 1, 3, 3);
}

function box(pts) {
  const [a, b] = pts;
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
}

const f = (v) => v.toFixed(3).replace(/0+$/, '').replace(/\.$/, '.0');

function emit() {
  const pts = state.points;
  let text = '';
  if (state.shape === 'polygon' && pts.length >= 3) {
    text = 'polygon:\n' + pts.map((p) => `  - { x: ${f(p.x)}, y: ${f(p.y)} }`).join('\n');
  } else if (state.shape !== 'polygon' && pts.length === 2) {
    const b = box(pts);
    text = `${state.shape}: { x: ${f(b.x)}, y: ${f(b.y)}, w: ${f(b.w)}, h: ${f(b.h)} }`;
  }
  el('snippet').value = text;
  // Showing what the selector actually catches is the point of drawing it in
  // the first place; feather and gain change the answer, so the core is asked
  // rather than the outline being trusted.
  if (text && state.session) {
    try {
      const cov = state.session.preview_mask(text);
      state.maskLayer = null;
      paintCoverage(cov);
    } catch { /* an unfinished outline is not an error worth reporting */ }
  }
}

function paintCoverage(cov, alpha = 0.45) {
  const [r, g, b] = token('--accent');
  const img = octx.createImageData(overlay.width, overlay.height);
  for (let i = 0; i < cov.length; i++) {
    img.data[i * 4] = r;
    img.data[i * 4 + 1] = g;
    img.data[i * 4 + 2] = b;
    img.data[i * 4 + 3] = cov[i] * alpha;
  }
  octx.putImageData(img, 0, 0);
  drawShape();
}

el('copy').addEventListener('click', () => navigator.clipboard.writeText(el('snippet').value));

// ---------------------------------------------------------------- export

// The recorder is the only part of this that is not the renderer's own output:
// MP4 where the browser can write one, WebM where it cannot. The menu says
// which, rather than offering a format that will arrive named something else.
const VIDEO = ['video/mp4;codecs=avc1.42E01E', 'video/mp4', 'video/webm;codecs=vp9', 'video/webm']
  .find((t) => MediaRecorder.isTypeSupported(t)) ?? '';
const VIDEO_EXT = VIDEO.startsWith('video/mp4') ? 'mp4' : 'webm';
el('save-video').firstChild.nodeValue = `${VIDEO_EXT.toUpperCase()} `;

// Saving is the one thing here that can take a visible moment, so the menu
// label says what is happening and the menu closes: leaving it open over a
// frozen page reads as a click that did not land.
async function saving(label, run) {
  const summary = el('save').querySelector('summary');
  el('save').open = false;
  summary.textContent = label;
  // A frame for the label to paint before the encoder takes the thread.
  await new Promise(requestAnimationFrame);
  try {
    await run();
  } catch (e) {
    fail(e);
  }
  summary.textContent = 'Save';
}

el('save-png').addEventListener('click', () => saving('Writing…', async () => {
  const scale = 4;
  const rgba = state.session.frame(state.frame, scale);
  const c = document.createElement('canvas');
  c.width = state.session.width * scale;
  c.height = state.session.height * scale;
  c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(rgba), c.width, c.height), 0, 0);
  const blob = await new Promise((r) => c.toBlob(r));
  download(blob, `${stem(state.name)}-pixel.png`);
}));

// Written by the renderer itself rather than re-encoded: the frames are
// already indices into the scene's palette, which is what a GIF stores, so
// every color survives exactly.
el('save-gif').addEventListener('click', () => saving('Writing GIF…', async () => {
  const bytes = state.session.gif(2);
  download(new Blob([bytes], { type: 'image/gif' }), `${stem(state.name)}-loop.gif`);
}));

// A preview-quality loop, written with what the browser already has. The CLI
// is still the way to get video at wallpaper size: this hands frames the
// renderer produced exactly to a lossy codec.
el('save-video').addEventListener('click', () => saving('Recording…', async () => {
  stop();
  const scale = 3;
  const c = document.createElement('canvas');
  c.width = state.session.width * scale;
  c.height = state.session.height * scale;
  const cx = c.getContext('2d');

  const fps = Math.max(1, state.session.fps);
  const stream = c.captureStream(0);
  const track = stream.getVideoTracks()[0];
  const chunks = [];
  const rec = new MediaRecorder(stream, { mimeType: VIDEO, videoBitsPerSecond: 12e6 });
  rec.ondataavailable = (e) => e.data.size && chunks.push(e.data);
  const done = new Promise((r) => (rec.onstop = r));
  rec.start();

  for (let i = 0; i < state.session.frames; i++) {
    const rgba = state.session.frame(i, scale);
    cx.putImageData(new ImageData(new Uint8ClampedArray(rgba), c.width, c.height), 0, 0);
    // Pushing frames explicitly rather than letting the stream sample the
    // canvas is what keeps the recording frame-exact, and so still a loop.
    track.requestFrame();
    await new Promise((r) => setTimeout(r, 1000 / fps));
  }

  rec.stop();
  await done;
  download(new Blob(chunks, { type: VIDEO }), `${stem(state.name)}-loop.${VIDEO_EXT}`);
}));

// A menu left open after the pointer has gone elsewhere is just a panel in
// the way.
document.addEventListener('click', (e) => {
  if (!el('save').contains(e.target)) el('save').open = false;
});

function download(blob, name) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

// ---------------------------------------------------------------- odds and ends

el('catalog').replaceChildren(...JSON.parse(effects()).map(({ name, description }) => {
  const li = document.createElement('li');
  li.innerHTML = `<b><code>${name}</code></b> <span></span>`;
  li.querySelector('span').textContent = description;
  return li;
}));

function say(msg, bad = false) {
  el('status').textContent = msg;
  el('status').classList.toggle('bad', bad);
}

function fail(e) {
  say(e.message ?? String(e), true);
}

function clamp01(v) {
  return Math.min(1, Math.max(0, v));
}

function stem(name) {
  return name.replace(/\.[^.]+$/, '') || 'scene';
}

// Exposed so the page can be driven from the console, and by the headless
// smoke test in tools/, which has no way to work a file picker.
window.pixelgen = { state, open, apply };
