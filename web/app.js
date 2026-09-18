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
  el('starter').disabled = false;
  say(`${bitmap.width}x${bitmap.height} loaded. Press "Starter scene", or paste one.`);

  // A scene already in the box is almost always meant for this image too -
  // the usual move is to try the same scene on a second photograph.
  if (el('yaml').value.trim()) apply();
}

el('file').addEventListener('change', (e) => {
  if (e.target.files[0]) open(e.target.files[0]).catch(fail);
});

for (const type of ['dragover', 'drop']) {
  document.addEventListener(type, (e) => {
    e.preventDefault();
    if (type === 'drop' && e.dataTransfer.files[0]) open(e.dataTransfer.files[0]).catch(fail);
  });
}

el('starter').addEventListener('click', () => {
  try {
    el('yaml').value = state.session.starter(state.name);
    apply();
  } catch (e) {
    fail(e);
  }
});

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
    // towards every time a half-typed line is momentarily invalid.
    el('error').hidden = false;
    el('error').textContent = e.message ?? String(e);
    return;
  }

  const s = state.session;
  view.width = overlay.width = s.width;
  view.height = overlay.height = s.height;
  el('viewport').classList.remove('empty');

  state.cache = new Array(s.frames);
  state.frame = Math.min(state.frame, s.frames - 1);
  el('scrub').max = s.frames - 1;
  el('scrub').value = state.frame;
  for (const id of ['play', 'scrub', 'png', 'record']) el(id).disabled = false;

  swatches(s.palette);
  layerList(s.layers);
  if (state.maskLayer !== null && state.maskLayer >= s.layers.length) state.maskLayer = null;

  say(`${s.width}x${s.height} cells, ${s.frames} frames at ${s.fps} fps, ${s.palette.length} colours`);
  draw();
}

function swatches(hexes) {
  el('palette').replaceChildren(...hexes.map((h) => {
    const i = document.createElement('i');
    i.style.background = h;
    i.title = h;
    return i;
  }));
}

function layerList(names) {
  el('layers').replaceChildren(...names.map((name, i) => {
    const li = document.createElement('li');
    li.textContent = name;
    li.className = state.maskLayer === i ? 'on' : '';
    li.title = 'Show this layer’s resolved mask';
    li.addEventListener('click', () => {
      state.maskLayer = state.maskLayer === i ? null : i;
      el('showmask').checked = state.maskLayer !== null;
      layerList(names);
      draw();
    });
    return li;
  }));
  if (!names.length) {
    const li = document.createElement('li');
    li.textContent = 'No enabled layers - the loop will be a still image.';
    li.style.color = 'var(--dim)';
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
    const cov = state.session.layer_mask(state.maskLayer);
    const img = octx.createImageData(overlay.width, overlay.height);
    for (let i = 0; i < cov.length; i++) {
      img.data[i * 4] = 255;
      img.data[i * 4 + 1] = 64;
      img.data[i * 4 + 2] = 96;
      img.data[i * 4 + 3] = cov[i] * 0.55;
    }
    octx.putImageData(img, 0, 0);
  }
  drawShape();
}

function tick() {
  state.frame = (state.frame + 1) % state.session.frames;
  el('scrub').value = state.frame;
  draw();
}

function play() {
  if (!state.session?.frames) return;
  state.playing = true;
  el('play').textContent = 'Pause';
  clearInterval(state.timer);
  state.timer = setInterval(tick, 1000 / Math.max(1, state.session.fps));
}

function stop() {
  state.playing = false;
  clearInterval(state.timer);
  el('play').textContent = 'Play';
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
    layerList(state.session.layers);
  }
  drawOverlay();
});

// ---------------------------------------------------------------- mask drawing

const tabs = document.querySelectorAll('.tabs button');
tabs.forEach((b) => b.addEventListener('click', () => {
  tabs.forEach((o) => o.classList.toggle('on', o === b));
  document.querySelectorAll('.tab').forEach((t) => {
    t.hidden = t.dataset.tab !== b.dataset.tab;
  });
  el('viewport').classList.toggle('drawing', b.dataset.tab === 'draw');
}));

for (const shape of ['polygon', 'rect', 'ellipse']) {
  el(`shape-${shape}`).addEventListener('click', () => {
    state.shape = shape;
    state.points = [];
    ['polygon', 'rect', 'ellipse'].forEach((s) => el(`shape-${s}`).classList.toggle('on', s === shape));
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
  octx.strokeStyle = '#7fb2ff';
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
  octx.fillStyle = '#7fb2ff';
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

function paintCoverage(cov) {
  const img = octx.createImageData(overlay.width, overlay.height);
  for (let i = 0; i < cov.length; i++) {
    img.data[i * 4] = 127;
    img.data[i * 4 + 1] = 178;
    img.data[i * 4 + 2] = 255;
    img.data[i * 4 + 3] = cov[i] * 0.45;
  }
  octx.putImageData(img, 0, 0);
  drawShape();
}

el('copy').addEventListener('click', () => navigator.clipboard.writeText(el('snippet').value));

// ---------------------------------------------------------------- export

el('png').addEventListener('click', () => {
  const scale = 4;
  const rgba = state.session.frame(state.frame, scale);
  const c = document.createElement('canvas');
  c.width = state.session.width * scale;
  c.height = state.session.height * scale;
  c.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(rgba), c.width, c.height), 0, 0);
  c.toBlob((b) => download(b, `${stem(state.name)}-pixel.png`));
});

// A preview-quality loop, written with what the browser already has. The CLI
// is still the way to get an mp4 at wallpaper size: this re-encodes frames the
// renderer produced exactly, and hands the result to a lossy codec.
el('record').addEventListener('click', async () => {
  stop();
  const btn = el('record');
  btn.disabled = true;
  btn.textContent = 'Recording…';

  const scale = 3;
  const c = document.createElement('canvas');
  c.width = state.session.width * scale;
  c.height = state.session.height * scale;
  const cx = c.getContext('2d');

  const fps = Math.max(1, state.session.fps);
  const stream = c.captureStream(0);
  const track = stream.getVideoTracks()[0];
  const chunks = [];
  const rec = new MediaRecorder(stream, { mimeType: 'video/webm', videoBitsPerSecond: 12e6 });
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
  download(new Blob(chunks, { type: 'video/webm' }), `${stem(state.name)}-loop.webm`);
  btn.disabled = false;
  btn.textContent = 'Record loop';
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
