// The browser front end. Every rendering decision lives in the Rust core; this
// file only moves pixels between the core and a canvas, and keeps the editor's
// state honest about what has actually been prepared.

import init, { Session, effects } from './pkg/pixelgen_wasm.js';
import { setLayerDisabled, setScalar } from './scene-text.js';

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
  /** Every layer in the scene, disabled ones too, as `Session.scene_layers`. */
  layers: [],
  /** Index into `layers` (the scene's own order) of the selected one, or null. */
  selected: null,
  /** The scene index of the row being dragged. */
  drag: null,
  tab: 'layers',
  shape: 'polygon',
  /** What the outline being drawn catches, as coverage, or null. */
  preview: null,
  points: [],
  name: 'image',
};

await init();
state.effects = JSON.parse(effects());

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

  // A scene is built for the photograph it was made on - its masks, palette
  // and title were all read off that image - so a new one gets the scene its
  // own analysis suggests. Whatever was selected, drawn or scrubbed to
  // belonged to the old scene and goes with it.
  state.selected = null;
  state.frame = 0;
  state.points = [];
  starter();
  emit();
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

// Builds the scene in the box. Returns whether it could.
function apply() {
  if (!state.session) return false;
  try {
    state.session.set_scene(el('yaml').value);
    el('error').hidden = true;
  } catch (e) {
    // A scene that fails to prepare leaves the previous one on screen. The
    // alternative - blanking the canvas - throws away the thing being edited
    // toward every time a half-typed line is momentarily invalid.
    el('error').hidden = false;
    el('error').textContent = e.message ?? String(e);
    return false;
  }
  return refresh();
}

// Everything that follows from a scene having been prepared, however it got
// there: typed into the Scene tab or edited in the layer panel.
function refresh() {
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
  el('add').inert = false;

  state.layers = JSON.parse(s.scene_layers());
  if (state.selected !== null && state.selected >= state.layers.length) {
    state.selected = state.layers.length ? state.layers.length - 1 : null;
  }
  if (state.selected === null && state.layers.length) state.selected = state.layers.length - 1;

  swatches(s.palette);
  layerPanel();

  titleblock(s);
  // The timer was set for the rate the loop had when Play was pressed.
  if (state.playing) play();
  draw();
  return true;
}

// ---------------------------------------------------------------- title block

// The figures over the YAML, which are also the quickest way to change them.
function titleblock(s) {
  for (const id of ['set-width', 'set-seconds', 'set-fps', 'set-colors']) el(id).disabled = false;
  el('set-width').value = s.width;
  el('grid-h').textContent = `\u00d7 ${s.height}`;
  el('set-seconds').value = s.seconds;
  el('set-fps').value = s.fps;
  el('set-seconds').parentElement.title = `${s.frames} frames`;

  // A fixed palette.hex has nothing to cluster, so the count is only a fact.
  el('set-colors').value = s.palette_derived ? s.colors : s.palette.length;
  el('set-colors').disabled = !s.palette_derived;
  el('set-colors').title = s.palette_derived ? 'palette.colors' : 'Fixed by palette.hex';

  const all = JSON.parse(s.scene_layers());
  const on = all.filter((l) => !l.disable).length;
  el('layer-count').textContent = all.length === on ? `${on}` : `${on}/${all.length}`;
  el('layer-menu').inert = !all.length;
  if (!all.length) el('layer-menu').open = false;
  el('layer-toggles').replaceChildren(...all.map((l, i) => {
    const label = document.createElement('label');
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = !l.disable;
    box.addEventListener('change', () => edit((t) => setLayerDisabled(t, i, !box.checked)));
    const chip = document.createElement('span');
    chip.className = 'type';
    chip.textContent = l.type;
    label.append(box, l.name, chip);
    return label;
  }));
}

// Rewrites the scene text and applies it straight away. The text is the one
// source of truth, so a control never touches the session directly.
function edit(change) {
  let text;
  try {
    text = change(el('yaml').value);
  } catch (e) {
    fail(e);
    // The browser has already moved the control - ticked the box, or taken
    // the typed number - so without this it would go on claiming a change
    // that never reached the text.
    titleblock(state.session);
    return;
  }
  write(text);
  clearTimeout(pending);
  // A text that no longer builds leaves the previous scene on screen, and the
  // controls say what that scene is rather than what was asked for.
  if (!apply()) titleblock(state.session);
}

// Replaces the scene text the way typing would, as one step in the box's own
// undo history. Assigning to .value wipes that history, which would make a
// header control the one edit Ctrl+Z cannot see past - and take everything
// typed before it along too.
function write(text) {
  if (text !== el('yaml').value) onBox((box) => replace(box, text));
}

// Runs `f` with the scene box able to take focus. It sits on the Scene tab,
// and a box on a hidden tab cannot be focused - so it could not be edited
// through execCommand, and an edit made from the Layers tab would wipe its
// history. The tab is shown for the length of the call only; nothing is
// painted in between, so it never appears.
function onBox(f) {
  const box = el('yaml');
  const tab = box.closest('[hidden]');
  if (tab) tab.hidden = false;
  try {
    return f(box);
  } finally {
    if (tab) tab.hidden = true;
  }
}

function replace(box, text) {
  const old = box.value;

  // Only the span that differs is replaced, so the rest of the text, and the
  // reader's place in it, is left alone.
  let a = 0;
  while (a < old.length && a < text.length && old[a] === text[a]) a++;
  let b = 0;
  while (b < old.length - a && b < text.length - a && old.at(-1 - b) === text.at(-1 - b)) b++;
  const mid = text.slice(a, text.length - b);

  const back = document.activeElement;
  const { selectionStart, selectionEnd, scrollTop } = box;
  box.focus({ preventScroll: true });
  box.setSelectionRange(a, old.length - b);
  // execCommand is deprecated, but it is still the only edit a textarea's undo
  // stack records. Should it be refused anyway, a plain assignment - which
  // loses the history - is better than losing the edit.
  const typed = document.activeElement === box
    && document.execCommand(mid ? 'insertText' : 'delete', false, mid);
  if (!typed) {
    box.value = text;
    return;
  }

  const moved = (p) => (p <= a ? p : p >= old.length - b ? p + text.length - old.length : a + mid.length);
  box.setSelectionRange(moved(selectionStart), moved(selectionEnd));
  box.scrollTop = scrollTop;
  if (back && back !== document.body) back.focus({ preventScroll: true });
  else box.blur();
}

// `change` rather than `input`: typing 320 would otherwise prepare a scene at
// width 3 and then 32 on the way there.
for (const [id, path, whole] of [
  ['set-width', ['width'], true],
  ['set-seconds', ['loop', 'seconds'], false],
  ['set-fps', ['loop', 'fps'], true],
  ['set-colors', ['palette', 'colors'], true],
]) {
  const input = el(id);
  input.addEventListener('change', () => {
    let v = input.valueAsNumber;
    // An emptied field puts back what the scene says rather than writing a
    // blank into it.
    if (!Number.isFinite(v)) return titleblock(state.session);
    if (whole) v = Math.round(v);
    // The browser holds a typed number to neither bound, and the renderer has
    // no ceiling of its own: a width of 999999 would try to build that grid.
    // The field's own min and max are the limits, so the page cannot offer one
    // range and write another.
    v = Math.min(+input.max, Math.max(+input.min, v));
    edit((t) => setScalar(t, path, v));
  });
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

// ---------------------------------------------------------------- layer panel

// Every change the panel makes goes through the parsed scene in the core and
// comes back as YAML for the Scene tab, so the text and the stack can never
// disagree. An edit the core refuses - a mask that catches nothing, a value
// the effect rejects - is reported and the working scene stays as it was.
function layerEdit(op) {
  if (!state.session) return false;
  let yaml;
  try {
    yaml = state.session.edit(JSON.stringify(op));
  } catch (e) {
    el('layer-error').hidden = false;
    el('layer-error').textContent = e.message ?? String(e);
    layerPanel();
    return false;
  }
  write(yaml);
  clearTimeout(pending);
  el('error').hidden = true;
  el('layer-error').hidden = true;
  refresh();
  return true;
}

// One history for the whole page: the scene box's own. The panel, the title
// block and typing all edit through it, so stepping back from any of them
// takes back the last change whatever made it.
function undo() {
  const back = document.activeElement;
  const undone = onBox((box) => {
    box.focus({ preventScroll: true });
    const ok = document.execCommand('undo');
    if (back && back !== document.body) back.focus({ preventScroll: true });
    else box.blur();
    return ok;
  });
  if (!undone) return;
  clearTimeout(pending);
  el('layer-error').hidden = true;
  apply();
}

document.addEventListener('keydown', (e) => {
  if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.key.toLowerCase() !== 'z') return;
  // Text fields keep their own undo; this one steps back through panel edits.
  if (e.target.closest('input, textarea, select')) return;
  e.preventDefault();
  undo();
});

/** The selected layer's index into the drawn layers, or null. */
function selectedDrawn() {
  return state.selected === null ? null : state.layers[state.selected]?.drawn ?? null;
}

function select(i) {
  state.selected = i;
  layerPanel();
  drawOverlay();
}

function layerPanel() {
  const ls = state.layers;
  const list = el('layers');
  // Top of the stack first, the way every layer panel reads: the row at the
  // top is composited last and so sits over everything below it.
  const rows = [];
  for (let i = ls.length - 1; i >= 0; i--) rows.push(layerRow(ls[i], i));
  list.replaceChildren(...rows);
  if (!ls.length) {
    const li = document.createElement('li');
    li.className = 'none';
    li.textContent = state.session
      ? 'No layers - the loop will be a still image. Add one above.'
      : 'Open an image to start.';
    list.append(li);
  }

  const sel = state.selected;
  el('remove').disabled = sel === null;
  el('up').disabled = sel === null || sel >= ls.length - 1;
  el('down').disabled = sel === null || sel <= 0;
  properties();
  maskTarget();
}

function layerRow(layer, i) {
  const li = document.createElement('li');
  li.dataset.i = i;
  li.setAttribute('role', 'option');
  li.setAttribute('aria-selected', state.selected === i);
  li.classList.toggle('off', layer.disable);
  li.draggable = true;

  const eye = document.createElement('button');
  eye.type = 'button';
  eye.className = 'eye';
  eye.setAttribute('aria-pressed', !layer.disable);
  eye.setAttribute('aria-label', layer.disable ? `Show ${layer.label}` : `Hide ${layer.label}`);
  eye.title = layer.disable ? 'Switched off - click to draw it' : 'Drawn - click to switch off';
  eye.addEventListener('click', (e) => {
    e.stopPropagation();
    layerEdit({ op: 'disable', i, disable: !layer.disable });
  });

  const name = document.createElement('span');
  name.className = 'name';
  name.textContent = layer.label;
  name.title = 'Double-click to rename';
  name.addEventListener('dblclick', (e) => {
    e.stopPropagation();
    rename(name, layer, i);
  });

  const chip = document.createElement('span');
  chip.className = 'type';
  chip.textContent = layer.type;

  li.append(eye, name, chip);
  li.addEventListener('click', () => select(i));

  li.addEventListener('dragstart', (e) => {
    state.drag = i;
    e.dataTransfer.effectAllowed = 'move';
    // Firefox starts no drag without data.
    e.dataTransfer.setData('text/plain', String(i));
    li.classList.add('dragging');
  });
  li.addEventListener('dragend', () => {
    state.drag = null;
    li.classList.remove('dragging');
    el('layers').querySelectorAll('.over').forEach((o) => o.classList.remove('over'));
  });
  li.addEventListener('dragover', (e) => {
    if (state.drag === null) return;
    // Handled here so the page-wide handler does not treat it as a file.
    e.preventDefault();
    e.stopPropagation();
    li.classList.toggle('over', state.drag !== i);
  });
  li.addEventListener('dragleave', () => li.classList.remove('over'));
  li.addEventListener('drop', (e) => {
    if (state.drag === null) return;
    e.preventDefault();
    e.stopPropagation();
    const from = state.drag;
    state.drag = null;
    if (from !== i && layerEdit({ op: 'move', from, to: i })) select(i);
  });
  return li;
}

function rename(span, layer, i) {
  const input = document.createElement('input');
  input.className = 'rename';
  input.value = layer.name;
  input.placeholder = layer.label;
  input.setAttribute('aria-label', 'Layer name');
  let done = false;
  const finish = (keep) => {
    if (done) return;
    done = true;
    if (keep && input.value.trim() !== layer.name) layerEdit({ op: 'rename', i, name: input.value });
    else layerPanel();
  };
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') finish(true);
    if (e.key === 'Escape') finish(false);
  });
  input.addEventListener('blur', () => finish(true));
  input.addEventListener('click', (e) => e.stopPropagation());
  span.replaceWith(input);
  input.focus();
  input.select();
}

el('remove').addEventListener('click', () => {
  const i = state.selected;
  if (i === null) return;
  if (layerEdit({ op: 'remove', i })) select(state.layers.length ? Math.min(i, state.layers.length - 1) : null);
});

// Up the stack is later in the scene's list, which is drawn in order.
for (const [id, d] of [['up', 1], ['down', -1]]) {
  el(id).addEventListener('click', () => {
    const i = state.selected;
    if (i === null) return;
    if (layerEdit({ op: 'move', from: i, to: i + d })) select(i + d);
  });
}

// The add menu, one entry per effect. A new layer goes directly over the
// selected one, which is where a panel's "new layer" always lands.
el('catalog').replaceChildren(...state.effects.map(({ name, description }) => {
  const b = document.createElement('button');
  b.type = 'button';
  b.append(name);
  const small = document.createElement('small');
  small.textContent = description;
  b.append(small);
  b.addEventListener('click', () => {
    el('add').open = false;
    const at = state.selected === null ? state.layers.length : state.selected + 1;
    if (layerEdit({ op: 'add', type: name, at })) select(at);
  });
  return b;
}));

function properties() {
  const layer = state.selected === null ? null : state.layers[state.selected];
  el('props').hidden = !layer;
  if (!layer) return;

  el('props-title').replaceChildren(layer.label);
  el('mask-summary').textContent = maskSummary(layer.mask);
  el('mask-summary').title = layer.mask ?? 'No mask: the whole frame';
  el('mask-clear').disabled = !layer.mask;
  el('mask-show').disabled = layer.drawn === null;
  el('mask-show').setAttribute('aria-pressed', el('showmask').checked && layer.drawn !== null);

  const spec = state.effects.find((f) => f.name === layer.type);
  const rows = (spec?.params ?? []).map((p) => paramRow(p, layer));
  if (!spec) {
    const p = document.createElement('p');
    p.className = 'quiet';
    p.textContent = `${layer.type} is not an effect this build knows.`;
    rows.push(p);
  }
  el('params').replaceChildren(...rows);
}

function paramRow(p, layer) {
  const i = state.selected;
  const value = layer.params?.[p.key] ?? null;
  const set = layer.set.includes(p.key);
  const send = (v) => layerEdit({ op: 'param', i, key: p.key, value: v });

  const row = document.createElement('div');
  row.className = 'param';
  row.classList.toggle('set', set);
  const id = `param-${p.key}`;
  const label = document.createElement('label');
  label.htmlFor = id;
  label.textContent = p.key.replace(/_/g, ' ');
  row.append(label);

  let input;
  if (p.kind === 'bool') {
    input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = !!value;
    input.addEventListener('change', () => send(input.checked));
  } else if (p.kind === 'choice') {
    input = document.createElement('select');
    for (const c of p.choices) input.append(new Option(c, c, false, c === value));
    input.addEventListener('change', () => send(input.value));
  } else if (p.kind === 'color') {
    input = document.createElement('input');
    input.type = 'color';
    input.value = /^#[0-9a-f]{6}$/i.test(value ?? '') ? value : '#808080';
    input.classList.toggle('auto', value === null);
    input.title = value ?? 'The effect’s own tint';
    input.addEventListener('change', () => send(input.value));
  } else if (p.kind === 'int' || p.kind === 'float') {
    input = document.createElement('input');
    input.type = 'number';
    input.step = p.kind === 'int' ? '1' : '0.01';
    input.value = value;
    // On change rather than input: a spinner held down would otherwise
    // re-prepare the scene on every step it passes through.
    input.addEventListener('change', () => {
      const v = input.valueAsNumber;
      if (Number.isFinite(v)) send(p.kind === 'int' ? Math.round(v) : v);
    });
  } else {
    input = document.createElement('input');
    input.value = value ?? '';
    input.addEventListener('change', () => send(input.value || null));
  }
  input.id = id;
  row.append(input);

  const reset = document.createElement('button');
  reset.type = 'button';
  reset.className = 'reset';
  reset.textContent = '↺';
  reset.title = p.kind === 'color' ? 'Back to the effect’s own tint' : `Back to the default (${p.default})`;
  reset.setAttribute('aria-label', `Reset ${p.key}`);
  reset.hidden = !set;
  reset.addEventListener('click', () => send(null));
  row.append(reset);
  return row;
}

// A mask as one line: what kind of selector it is, not its numbers. The full
// YAML is in the title and in the Scene tab.
function maskSummary(yaml) {
  if (!yaml) return 'full frame';
  const lines = yaml.split('\n');
  const top = lines.filter((l) => /^[a-z_]+:/.test(l));
  const kind = (l) => {
    const [k, v] = l.split(/:\s*/);
    if (k === 'ref') return `ref: ${v}`;
    if (k === 'invert') return 'inverted';
    if (k === 'feather' || k === 'gain') return `${k} ${v}`;
    if (k === 'polygon') return `polygon · ${lines.filter((x) => x.startsWith('- x:')).length} pts`;
    // A combinator names its members, which sit one level in as `- key:`.
    if (k === 'all' || k === 'any') {
      const members = lines.filter((x) => /^- [a-z_]+:/.test(x)).map((x) => kind(x.slice(2)));
      return `${k}(${members.join(', ')})`;
    }
    return k;
  };
  return top.map(kind).join(' · ');
}

el('mask-show').addEventListener('click', () => {
  el('showmask').checked = !el('showmask').checked;
  properties();
  drawOverlay();
});

el('mask-edit').addEventListener('click', () => {
  state.points = [];
  emit();
  showTab('draw');
});

el('mask-clear').addEventListener('click', () => {
  if (state.selected !== null) layerEdit({ op: 'mask', i: state.selected, mask: null });
});

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
  if (!state.session?.frames) return;
  // While drawing, what the outline catches is the thing being looked at -
  // never a selected layer's own mask, even before the outline has enough
  // points to preview.
  if (state.tab === 'draw') {
    if (state.preview) paintCoverage(state.preview);
    else drawShape();
    return;
  }
  const d = selectedDrawn();
  if (d !== null && el('showmask').checked) {
    paintCoverage(state.session.layer_mask(d), 0.55);
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
document.addEventListener('keydown', (e) => {
  if (e.key !== ' ' || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
  // Leave Space to whatever already owns it: typing, and pressing a focused
  // button or checkbox (the Play button included). The scrubber has no use
  // for it, so playback can resume straight after dragging.
  const t = e.target;
  if (t.closest?.('textarea, select, button, a, summary, [contenteditable]')) return;
  if (t.matches?.('input:not([type=range])')) return;
  if (el('play').disabled) return;
  e.preventDefault();
  state.playing ? stop() : play();
});
el('scrub').addEventListener('input', (e) => {
  stop();
  state.frame = +e.target.value;
  draw();
});
el('showmask').addEventListener('change', () => {
  properties();
  drawOverlay();
});

// ---------------------------------------------------------------- tabs and help

const tabs = document.querySelectorAll('.tabs button[data-tab]');
function showTab(name) {
  state.tab = name;
  tabs.forEach((o) => o.setAttribute('aria-pressed', o.dataset.tab === name));
  document.querySelectorAll('.tab').forEach((t) => {
    t.hidden = t.dataset.tab !== name;
  });
  el('viewport').classList.toggle('drawing', name === 'draw');
  maskTarget();
  drawOverlay();
}
tabs.forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));

// The explanations are for the first visit; after that they are in the way.
// Remembered per browser, which is all a preference like this needs.
function help(open) {
  el('help').setAttribute('aria-expanded', open);
  el('help').setAttribute('aria-label', open ? 'Hide help' : 'Show help');
  el('help').title = open ? 'Hide help' : 'Show help';
  document.querySelectorAll('.help').forEach((h) => (h.hidden = !open));
  try {
    localStorage.setItem('pixelgen.help', open ? '1' : '0');
  } catch { /* storage may be unavailable; the toggle still works */ }
}
el('help').addEventListener('click', () => help(el('help').getAttribute('aria-expanded') !== 'true'));
try {
  if (localStorage.getItem('pixelgen.help') === '1') help(true);
} catch { /* as above */ }

// ---------------------------------------------------------------- mask drawing

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

// The draw tab's Apply button: which layer it would write to, if any.
function maskTarget() {
  const layer = state.selected === null ? null : state.layers[state.selected];
  const b = el('apply-mask');
  b.disabled = !layer || !el('snippet').value;
  b.textContent = layer ? `Apply to ${layer.label}` : 'Apply';
  b.title = layer ? `Make this the mask of ${layer.label}` : 'Select a layer to apply a mask to';
}

el('apply-mask').addEventListener('click', () => {
  const i = state.selected;
  if (i === null || !el('snippet').value) return;
  if (layerEdit({ op: 'mask', i, mask: el('snippet').value })) {
    state.points = [];
    emit();
    el('showmask').checked = true;
    showTab('layers');
  }
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
  state.preview = null;
  if (text && state.session) {
    try {
      state.preview = state.session.preview_mask(text);
    } catch { /* an unfinished outline is not an error worth reporting */ }
  }
  maskTarget();
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
  for (const id of ['save', 'layer-menu', 'add']) if (!el(id).contains(e.target)) el(id).open = false;
});

function download(blob, name) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

// ---------------------------------------------------------------- odds and ends

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
window.pixelgen = { state, open, apply, edit: layerEdit, undo };
