// The browser front end. Every rendering decision lives in the Rust core; this
// file only moves pixels between the core and a canvas, and keeps the editor's
// state honest about what has actually been prepared.

import init, { Session, effects } from './pkg/pixelgen_wasm.js';
import { setLayerDisabled, setScalar } from './scene-text.js';
import {
  combine, hasModifiers, outline, parsePen, penD, penPoints, polyD, simplify, withStep,
} from './select.js';

const el = (id) => document.getElementById(id);
const view = el('view');
const overlay = el('overlay');
const vctx = view.getContext('2d');
const octx = overlay.getContext('2d');
const ui = el('ui');
const uctx = ui.getContext('2d');

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
  /** The selection being made in the Select tab, as a mask spec, or null. */
  sel: null,
  /** What `sel` catches, as 8-bit coverage over the grid. */
  selCov: null,
  /**
   * The selection's own history, as JSON of `{sel, region}`: which region it
   * was loaded from travels with it, so stepping back past loading one does
   * not leave the page set to overwrite that region.
   */
  selUndo: [],
  selRedo: [],
  /** The scene's regions, as `Session.regions`. */
  regions: [],
  /**
   * The region whose definition was loaded as the selection, or null. The
   * name box holds it while it is set; only `editing` changes it.
   */
  region: null,
  tool: 'rect',
  mode: 'new',
  /** The shape the current tool is in the middle of, or null. */
  draft: null,
  /** The selection as it would be with the draft folded in, or null. */
  draftCov: null,
  /** The selection's outline in cell edges, for the ants. */
  runs: [],
  /** The pointer over the plate, normalized, or null. */
  hover: null,
  name: 'image',
  /** Set while a save runs, which may span many turns of the event loop. */
  saving: false,
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
  // A live-wire belongs to the session it was made in.
  cancelDraft();
  state.session = new Session(data, bitmap.width, bitmap.height);
  state.name = file.name || 'image';
  say(`${file.name || 'image'} - ${bitmap.width}x${bitmap.height}`);

  // A scene is built for the photograph it was made on - its masks, palette
  // and title were all read off that image - so a new one gets the scene its
  // own analysis suggests. Whatever was selected, drawn or scrubbed to
  // belonged to the old scene and goes with it.
  state.selected = null;
  state.frame = 0;
  resetSel();
  starter();
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
  // A video save renders across many turns of the event loop; swapping the
  // scene under it would change its size or length halfway through, so the
  // edit waits for the save to finish. The text already holds it, so the
  // controls may go on showing it.
  if (state.saving) {
    clearTimeout(pending);
    pending = setTimeout(apply, 300);
    return true;
  }
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
  regionMenu();
  // The selection is evaluated on the grid, which may just have changed.
  selChanged();

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
  for (const c of [view, overlay, ui]) {
    c.style.width = `${s.width * scale}px`;
    c.style.height = `${s.height * scale}px`;
  }
  // The ants and handles are drawn at the screen's own resolution.
  ui.width = Math.round(s.width * scale * devicePixelRatio);
  ui.height = Math.round(s.height * scale * devicePixelRatio);
  drawUi();
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
  // Edits the session in place, which a video save is still reading from.
  if (state.saving) {
    say('Wait for the save to finish', true);
    layerPanel();
    return false;
  }
  let yaml;
  try {
    yaml = state.session.edit(JSON.stringify(op));
  } catch (e) {
    // The panel's own error box is on the Layers tab; from anywhere else the
    // refusal would go unseen there, and still be showing on the way back.
    if (state.tab === 'layers') {
      el('layer-error').hidden = false;
      el('layer-error').textContent = e.message ?? String(e);
    } else {
      fail(e);
    }
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
  if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 'z') return;
  // Text fields keep their own undo; this one steps back through panel edits.
  if (e.target.closest('input, textarea, select')) return;
  // While a selection is being made, its own steps come first: Apply is the
  // only thing that reaches the scene from the Select tab.
  if (state.tab === 'draw' && selHistory(e.shiftKey)) {
    e.preventDefault();
    return;
  }
  if (e.shiftKey) return;
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
    if (['feather', 'gain', 'grow', 'smooth'].includes(k)) return `${k} ${v}`;
    if (k === 'steps') return `steps(${stepSummary(lines)})`;
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

// The operands of a top-level `steps` list, joined by what each step does.
function stepSummary(lines) {
  const out = [];
  lines.forEach((l, i) => {
    const m = l.match(/^- (add|sub|and):\s*(.*)$/);
    if (!m) return;
    const what = m[2] === '{}' ? 'all' : m[2] || (lines[i + 1] ?? '').trim().split(':')[0] || '?';
    out.push(out.length ? `${{ add: '+', sub: '\u2212', and: '\u2229' }[m[1]]} ${what}` : what);
  });
  return out.join(' ');
}

el('mask-show').addEventListener('click', () => {
  el('showmask').checked = !el('showmask').checked;
  properties();
  drawOverlay();
});

// Takes the layer's mask into the Select tab as the selection, to work on.
el('mask-edit').addEventListener('click', () => {
  loadLayerMask();
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
  // While selecting, the selection is the thing being looked at - never a
  // selected layer's own mask.
  if (state.tab === 'draw') {
    const cov = state.draftCov ?? state.selCov;
    if (cov) paintCoverage(cov, 0.35);
    return;
  }
  const d = selectedDrawn();
  if (d !== null && el('showmask').checked) paintCoverage(state.session.layer_mask(d), 0.55);
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
  if (name !== 'draw') cancelDraft();
  maskTarget();
  drawOverlay();
  drawUi();
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

// ---------------------------------------------------------------- selection

// The selection is a mask spec like any other, kept as the JSON the core
// reads, and null for nothing selected - never `{}`, which is the whole frame.
// It has its own history while it is being made: a dozen lasso strokes are
// not a dozen scene edits, and only Apply or Save writes to the scene.

const TOOLS = ['rect', 'ellipse', 'lasso', 'polygon', 'magnetic', 'wand', 'range', 'pen', 'brush'];
const r4 = (v) => Math.round(v * 1e4) / 1e4;
let antsOffset = 0;

// A step of the selection's history: the selection, and the region it came from.
function snapshot() {
  return JSON.stringify({ sel: state.sel, region: state.region });
}

function setSel(sel) {
  state.selUndo.push(snapshot());
  state.selRedo = [];
  state.sel = sel;
  selChanged();
}

// Steps back or forward through the selection. Returns whether there was
// anything to step through, so the scene's own undo can have the key if not.
function selHistory(redo) {
  if (state.draft && !redo) {
    cancelDraft();
    return true;
  }
  const [from, to] = redo ? [state.selRedo, state.selUndo] : [state.selUndo, state.selRedo];
  if (!from.length) return false;
  to.push(snapshot());
  const { sel, region } = JSON.parse(from.pop());
  state.sel = sel;
  selChanged();
  if (region !== state.region) editing(state.regions.some((r) => r.name === region) ? region : null);
  return true;
}

function resetSel() {
  cancelDraft();
  state.sel = null;
  state.selCov = null;
  state.selUndo = [];
  state.selRedo = [];
  // The regions belong to the old scene: a same-named one in the new scene
  // is not the one being edited, and a name gone from the list is not a
  // rename.
  state.regions = [];
  editing(null);
}

// Everything that follows from the selection having changed, or from the grid
// it is evaluated on having changed under it.
function selChanged() {
  state.selCov = null;
  if (state.sel && state.session?.width) {
    try {
      state.selCov = state.session.preview_mask(JSON.stringify(state.sel));
    } catch (e) {
      // A region the selection refers to may have gone with an undo.
      fail(e);
    }
  }
  let yaml = '';
  if (state.sel) {
    try {
      yaml = state.session.spec_yaml(JSON.stringify(state.sel));
    } catch { /* reported above */ }
  }
  el('snippet').value = yaml;
  const cov = state.selCov;
  let share = 0;
  if (cov) for (const v of cov) share += v;
  el('sel-summary').textContent = state.sel
    ? `${maskSummary(yaml)} · ${cov ? Math.round((share / 255 / cov.length) * 100) : 0}%`
    : 'Nothing selected';
  for (const key of ['grow', 'smooth', 'feather']) {
    const input = el(`refine-${key}`);
    input.disabled = !state.sel;
    input.value = state.sel?.[key] ?? 0;
    input.nextElementSibling.value = input.value;
  }
  el('pen-reopen').disabled = !lastPen();
  regionButtons();
  maskTarget();
  showSel();
}

// Repaints the tint and the ants for whatever is on screen: the selection, or
// the selection as the shape being drawn would leave it.
function showSel() {
  const s = state.session;
  const cov = state.draftCov ?? state.selCov;
  state.runs = cov && s?.width ? outline(cov, s.width, s.height) : [];
  drawOverlay();
  drawUi();
}

// Folds one more shape into the selection, the way the mode says.
function commit(op, operand) {
  const next = withStep(state.sel, op, operand);
  state.draft = null;
  state.draftCov = null;
  if (next === state.sel) return showSel();
  setSel(next);
}

function cancelDraft() {
  state.draft?.wire?.free();
  state.draft = null;
  state.draftCov = null;
  showSel();
}

// What a pointer press does to the selection: the modifier keys, as
// Photoshop has them, or the sticky mode for a window manager that takes
// Alt-drag for itself.
function modeFor(e) {
  if (e.shiftKey && e.altKey) return 'and';
  if (e.shiftKey) return 'add';
  if (e.altKey) return 'sub';
  return state.mode;
}

// At most one preview per frame: a drag moves the pointer far more often
// than the core can build a mask, and only the latest one is worth seeing.
let previewing = 0;
function schedulePreview() {
  if (!previewing) previewing = requestAnimationFrame(preview);
}

function preview() {
  previewing = 0;
  const d = state.draft;
  const operand = d && tools[state.tool].operand?.(d);
  let cov = null;
  if (operand) {
    try {
      cov = state.session.preview_mask(JSON.stringify(operand));
    } catch { /* an unfinished outline is not an error worth reporting */ }
  }
  state.draftCov = cov ? combine(state.selCov, d.op, cov) : null;
  showSel();
}

// ---------------------------------------------------------------- the tools

function cells() {
  return { w: state.session.width, h: state.session.height };
}

// Whether two normalized points are within `px` screen pixels.
function near(p, q, px = 7) {
  const r = ui.getBoundingClientRect();
  return Math.hypot((p.x - q.x) * r.width, (p.y - q.y) * r.height) <= px;
}

function box(a, b) {
  return {
    x: r4(Math.min(a.x, b.x)), y: r4(Math.min(a.y, b.y)),
    w: r4(Math.abs(a.x - b.x)), h: r4(Math.abs(a.y - b.y)),
  };
}

// A press with no drag. In New mode that deselects, as clicking outside a
// selection does in every editor; otherwise it does nothing.
function click(op) {
  state.draft = null;
  state.draftCov = null;
  if (op === 'new' && state.sel) setSel(null);
  else showSel();
}

// Drops points closer than half a cell to the one before, which a double
// click or a slow hand leaves behind.
function dedupe(pts) {
  const { w, h } = cells();
  return pts.filter((p, i) => !i || Math.hypot((p.x - pts[i - 1].x) * w, (p.y - pts[i - 1].y) * h) >= 0.5);
}

const marquee = (kind) => ({
  down(p, e) {
    state.draft = { op: modeFor(e), a: p, b: p };
  },
  move(p) {
    if (!state.draft) return;
    state.draft.b = p;
    schedulePreview();
  },
  up(p) {
    const d = state.draft;
    if (!d) return;
    d.b = p;
    const b = box(d.a, d.b);
    const { w, h } = cells();
    if (b.w * w < 0.5 || b.h * h < 0.5) return click(d.op);
    commit(d.op, this.operand(d));
  },
  operand: (d) => ({ [kind]: box(d.a, d.b) }),
  draw(d) {
    const b = box(d.a, d.b);
    outlinePath(() => {
      if (kind === 'rect') uctx.rect(b.x * ui.width, b.y * ui.height, b.w * ui.width, b.h * ui.height);
      else uctx.ellipse((b.x + b.w / 2) * ui.width, (b.y + b.h / 2) * ui.height, (b.w / 2) * ui.width, (b.h / 2) * ui.height, 0, 0, Math.PI * 2);
    });
  },
});

// An outline closed from a list of points, simplified to within `eps` cells.
function closedPath(pts, eps) {
  const { w, h } = cells();
  const s = simplify(dedupe(pts), eps, w, h);
  return s.length >= 3 ? { path: polyD(s) } : null;
}

const lasso = {
  down(p, e) {
    state.draft = { op: modeFor(e), pts: [p] };
  },
  move(p, e) {
    const d = state.draft;
    if (!d) return;
    // Every point the pointer passed through, not only the one this event
    // landed on: a fast stroke is otherwise a polygon.
    for (const c of e.getCoalescedEvents?.() ?? [e]) d.pts.push(pt(c));
    schedulePreview();
  },
  up() {
    const d = state.draft;
    if (!d) return;
    const operand = closedPath(d.pts, 0.5);
    if (!operand) return click(d.op);
    commit(d.op, operand);
  },
  operand: (d) => closedPath(d.pts, 0.5),
  draw(d) {
    outlinePath(() => polyline(d.pts, false));
  },
};

const polygon = {
  down(p, e) {
    const d = state.draft;
    if (!d) {
      state.draft = { op: modeFor(e), pts: [p] };
    } else if (d.pts.length >= 3 && near(p, d.pts[0])) {
      return this.close();
    } else {
      d.pts.push(p);
    }
    schedulePreview();
  },
  move() {
    if (state.draft) schedulePreview();
  },
  close() {
    const d = state.draft;
    if (!d) return;
    const operand = closedPath(d.pts, 0);
    if (!operand) return cancelDraft();
    commit(d.op, operand);
  },
  back() {
    const d = state.draft;
    d.pts.pop();
    if (!d.pts.length) return cancelDraft();
    schedulePreview();
  },
  operand: (d) => closedPath(state.hover ? [...d.pts, state.hover] : d.pts, 0),
  draw(d) {
    outlinePath(() => polyline(state.hover ? [...d.pts, state.hover] : d.pts, false));
    handles(d.pts.slice(0, 1), state.hover && d.pts.length >= 3 && near(state.hover, d.pts[0]));
  },
};

// The magnetic lasso: each click drops an anchor, and between the last one
// and the pointer the core finds the path that hugs the strongest edge.
const magnetic = {
  down(p, e) {
    const d = state.draft;
    if (!d) {
      state.draft = { op: modeFor(e), pts: [p], anchors: [p], live: [], wire: state.session.livewire(p.x, p.y) };
    } else if (d.anchors.length >= 2 && near(p, d.pts[0])) {
      return this.close();
    } else {
      this.anchor(p);
    }
    schedulePreview();
  },
  anchor(p) {
    const d = state.draft;
    d.pts.push(...pairs(d.wire.path_to(p.x, p.y)).slice(1));
    const end = d.pts.at(-1);
    d.anchors.push(end);
    d.wire.free();
    d.wire = state.session.livewire(end.x, end.y);
    d.live = [];
  },
  move(p) {
    const d = state.draft;
    if (!d) return;
    d.live = pairs(d.wire.path_to(p.x, p.y));
    // Anchors are dropped along the way, as Photoshop does, so a long drag
    // does not leave the whole outline to one search. Each goes on a bend
    // well behind the pointer: the last stretch is only the path's way off
    // the edge to wherever the pointer happens to be.
    const { w, h } = cells();
    const run = [0];
    for (let i = 1; i < d.live.length; i++) {
      run.push(run[i - 1] + Math.hypot((d.live[i].x - d.live[i - 1].x) * w, (d.live[i].y - d.live[i - 1].y) * h));
    }
    const total = run.at(-1) ?? 0;
    if (total >= 20) {
      let at = -1;
      for (let i = 1; i < d.live.length - 1; i++) if (run[i] >= 4 && run[i] <= total - 6) at = i;
      if (at > 0) {
        this.anchor(d.live[at]);
        d.live = pairs(d.wire.path_to(p.x, p.y));
      }
    }
    schedulePreview();
  },
  close() {
    const d = state.draft;
    if (!d) return;
    const start = d.pts[0];
    const pts = [...d.pts, ...pairs(d.wire.path_to(start.x, start.y)).slice(1)];
    d.wire.free();
    d.wire = null;
    const operand = closedPath(pts, 0.35);
    if (!operand) return cancelDraft();
    commit(d.op, operand);
  },
  back() {
    const d = state.draft;
    if (d.anchors.length <= 1) return cancelDraft();
    d.anchors.pop();
    const last = d.anchors.at(-1);
    d.pts.length = d.pts.lastIndexOf(last) + 1;
    d.wire.free();
    d.wire = state.session.livewire(last.x, last.y);
    d.live = [];
    schedulePreview();
  },
  operand: (d) => closedPath([...d.pts, ...d.live.slice(1)], 0.35),
  draw(d) {
    outlinePath(() => polyline([...d.pts, ...d.live.slice(1)], false));
    handles(d.anchors, state.hover && d.anchors.length >= 2 && near(state.hover, d.pts[0]));
  },
};

function pairs(flat) {
  const out = [];
  for (let i = 0; i + 1 < flat.length; i += 2) out.push({ x: flat[i], y: flat[i + 1] });
  return out;
}

function sampled(p, e, kind) {
  const hex = state.session.sample(p.x, p.y);
  const tolerance = +el('opt-tolerance').value;
  if (kind === 'range') return commit(modeFor(e), { color: { hex, tolerance } });
  const contiguous = el('opt-contiguous').checked;
  commit(modeFor(e), { wand: { x: r4(p.x), y: r4(p.y), hex, tolerance, contiguous } });
}

// The pen: a click places a corner, a drag pulls out its handles, and any
// anchor or handle already down can be dragged again until the path closes.
const pen = {
  down(p, e) {
    let d = state.draft;
    if (!d) d = state.draft = { op: modeFor(e), anchors: [], grab: null };
    const hit = this.hit(p);
    if (hit?.kind === 'anchor' && hit.a === d.anchors[0] && d.anchors.length >= 3) return this.close();
    if (hit) {
      d.grab = hit;
    } else {
      const a = { x: p.x, y: p.y, ix: 0, iy: 0, ox: 0, oy: 0 };
      d.anchors.push(a);
      d.grab = { kind: 'new', a };
    }
    schedulePreview();
  },
  hit(p) {
    const d = state.draft;
    for (const a of [...d.anchors].reverse()) {
      if ((a.ox || a.oy) && near(p, { x: a.x + a.ox, y: a.y + a.oy })) return { kind: 'out', a };
      if ((a.ix || a.iy) && near(p, { x: a.x + a.ix, y: a.y + a.iy })) return { kind: 'in', a };
      if (near(p, a)) return { kind: 'anchor', a };
    }
    return null;
  },
  move(p) {
    const d = state.draft;
    if (!d?.grab) return;
    const { kind, a } = d.grab;
    if (kind === 'anchor') {
      a.x = p.x;
      a.y = p.y;
    } else {
      // A handle is dragged with its opposite mirrored, which keeps the
      // curve smooth through the anchor.
      const [dx, dy] = [p.x - a.x, p.y - a.y];
      const out = kind !== 'in';
      [a.ox, a.oy, a.ix, a.iy] = out ? [dx, dy, -dx, -dy] : [-dx, -dy, dx, dy];
    }
    schedulePreview();
  },
  up() {
    if (state.draft) state.draft.grab = null;
  },
  close() {
    const d = state.draft;
    if (!d) return;
    if (d.anchors.length < 3) return cancelDraft();
    commit(d.op, { path: penD(d.anchors, true) });
  },
  back() {
    const d = state.draft;
    d.anchors.pop();
    if (!d.anchors.length) return cancelDraft();
    schedulePreview();
  },
  operand: (d) => (d.anchors.length >= 3 ? { path: penD(d.anchors, true) } : null),
  draw(d) {
    const open = penPoints(d.anchors, false);
    if (state.hover && !d.grab && d.anchors.length) open.push(state.hover);
    outlinePath(() => polyline(open, false));
    uctx.lineWidth = devicePixelRatio;
    uctx.strokeStyle = `rgb(${token('--accent').join(',')})`;
    for (const a of d.anchors) {
      for (const [hx, hy] of [[a.ix, a.iy], [a.ox, a.oy]]) {
        if (!hx && !hy) continue;
        const [x, y] = [(a.x + hx) * ui.width, (a.y + hy) * ui.height];
        uctx.beginPath();
        uctx.moveTo(a.x * ui.width, a.y * ui.height);
        uctx.lineTo(x, y);
        uctx.stroke();
        uctx.beginPath();
        uctx.arc(x, y, 3 * devicePixelRatio, 0, Math.PI * 2);
        uctx.fill();
      }
    }
    handles(d.anchors, state.hover && d.anchors.length >= 3 && near(state.hover, d.anchors[0]));
  },
};

// The last step of the selection, when it is a path the pen could have drawn
// and so can be taken back out to edit.
function lastPen() {
  const sel = state.sel;
  if (!sel?.steps?.length || hasModifiers(sel)) return null;
  const step = sel.steps.at(-1);
  const op = Object.keys(step)[0];
  const operand = step[op];
  if (Object.keys(operand).length !== 1 || typeof operand.path !== 'string') return null;
  const parsed = parsePen(operand.path);
  return parsed && { op, ...parsed };
}

el('pen-reopen').addEventListener('click', () => {
  const last = lastPen();
  if (!last) return;
  const steps = state.sel.steps.slice(0, -1);
  pickTool('pen');
  setSel(steps.length ? { ...state.sel, steps } : null);
  state.draft = { op: last.op, anchors: last.anchors, grab: null };
  schedulePreview();
});

// The brush paints a stroke, or erases one with Alt. Strokes of the same
// size in a row are one step with several subpaths rather than a step each.
const brush = {
  down(p, e) {
    let op = modeFor(e);
    if (op === 'new') op = 'add';
    state.draft = { op, pts: [p] };
    schedulePreview();
  },
  move(p, e) {
    const d = state.draft;
    if (!d) return;
    for (const c of e.getCoalescedEvents?.() ?? [e]) d.pts.push(pt(c));
    schedulePreview();
  },
  up() {
    const d = state.draft;
    if (!d) return;
    const stroke = this.operand(d).stroke;
    state.draft = null;
    state.draftCov = null;
    const sel = state.sel;
    const last = sel && !hasModifiers(sel) && sel.steps?.at(-1);
    const prev = last?.[d.op]?.stroke;
    if (prev && Object.keys(last[d.op]).length === 1 && prev.radius === stroke.radius
        && (prev.hardness ?? 1) === (stroke.hardness ?? 1)) {
      const steps = [...sel.steps.slice(0, -1), { [d.op]: { stroke: { ...prev, d: prev.d + stroke.d } } }];
      return setSel({ ...sel, steps });
    }
    commit(d.op, { stroke });
  },
  operand(d) {
    const { w, h } = cells();
    const pts = simplify(dedupe(d.pts), 0.25, w, h);
    const hardness = +el('opt-hardness').value;
    const stroke = { d: polyD(pts, false), radius: r4(+el('opt-radius').value / w) };
    if (hardness < 1) stroke.hardness = hardness;
    return { stroke };
  },
  draw() {},
};

const tools = {
  rect: marquee('rect'),
  ellipse: marquee('ellipse'),
  lasso,
  polygon,
  magnetic,
  wand: { down: (p, e) => sampled(p, e, 'wand') },
  range: { down: (p, e) => sampled(p, e, 'range') },
  pen,
  brush,
};

function pt(e) {
  const r = ui.getBoundingClientRect();
  return { x: clamp01((e.clientX - r.left) / r.width), y: clamp01((e.clientY - r.top) / r.height) };
}

ui.addEventListener('pointerdown', (e) => {
  if (!state.session?.width || e.button !== 0) return;
  e.preventDefault();
  try {
    ui.setPointerCapture(e.pointerId);
  } catch { /* a synthetic pointer has nothing to capture */ }
  tools[state.tool].down(pt(e), e);
});
ui.addEventListener('pointermove', (e) => {
  if (!state.session?.width) return;
  state.hover = pt(e);
  tools[state.tool].move?.(state.hover, e);
  drawUi();
});
ui.addEventListener('pointerup', (e) => {
  if (state.session?.width) tools[state.tool].up?.(pt(e), e);
});
ui.addEventListener('dblclick', () => tools[state.tool].close?.());
ui.addEventListener('pointerleave', () => {
  state.hover = null;
  drawUi();
});

function pickTool(tool) {
  if (tool !== state.tool) cancelDraft();
  state.tool = tool;
  document.querySelectorAll('.tools [data-tool]').forEach((b) => b.setAttribute('aria-pressed', b.dataset.tool === tool));
  document.querySelectorAll('.options [data-for]').forEach((o) => {
    o.hidden = !o.dataset.for.split(' ').includes(tool);
  });
  ui.classList.toggle('brush', tool === 'brush');
  drawUi();
}
document.querySelectorAll('.tools [data-tool]').forEach((b) => b.addEventListener('click', () => pickTool(b.dataset.tool)));
pickTool('rect');

function pickMode(mode) {
  state.mode = mode;
  document.querySelectorAll('.modes [data-mode]').forEach((b) => b.setAttribute('aria-pressed', b.dataset.mode === mode));
}
document.querySelectorAll('.modes [data-mode]').forEach((b) => b.addEventListener('click', () => pickMode(b.dataset.mode)));

for (const id of ['opt-tolerance', 'opt-radius', 'opt-hardness']) {
  const input = el(id);
  const show = () => (input.nextElementSibling.value = input.value);
  input.addEventListener('input', () => {
    show();
    drawUi();
  });
  show();
}

// ---------------------------------------------------------------- select menu

el('sel-all').addEventListener('click', selectAll);
el('sel-none').addEventListener('click', deselect);
el('sel-invert').addEventListener('click', invert);

function selectAll() {
  setSel({ steps: [{ add: {} }] });
}

function deselect() {
  cancelDraft();
  if (state.sel) setSel(null);
  editing(null);
}

function invert() {
  const sel = state.sel;
  if (!sel) return selectAll();
  // Gain comes after invert, and 1 - gain(x) is not gain(1 - x), so a gained
  // selection is wrapped rather than flipped in place.
  if (sel.gain && sel.gain !== 1) return setSel({ steps: [{ add: sel }], invert: true });
  const next = { ...sel };
  if (next.invert) delete next.invert;
  else next.invert = true;
  setSel(next);
}

el('sel-layer').addEventListener('click', () => loadLayerMask());

function loadLayerMask() {
  const layer = state.selected === null ? null : state.layers[state.selected];
  if (!layer) return;
  try {
    setSel(layer.mask ? JSON.parse(state.session.spec_json(layer.mask)) : { steps: [{ add: {} }] });
  } catch (e) {
    fail(e);
  }
  editing(null);
}

// The scene's regions, in the Select menu - where picking one adds a
// reference to it - and in the list under the tab, where picking one loads
// its definition to be worked on and saved back.
function regionMenu() {
  const before = state.regions;
  try {
    state.regions = state.session ? JSON.parse(state.session.regions()) : [];
  } catch {
    state.regions = []; // no scene yet
  }
  const renames = renaming ? [renaming] : guessRenames(before, state.regions);
  for (const [from, to] of renames) followRename(from, to);
  // The region being edited may have gone with an undo.
  if (state.region !== null && !state.regions.some((r) => r.name === state.region)) editing(null);

  el('sel-regions').replaceChildren(...state.regions.map(({ name }) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = name;
    const small = document.createElement('small');
    small.textContent = 'region';
    b.append(' ', small);
    b.addEventListener('click', () => {
      setSel({ ref: name });
      editing(null);
    });
    return b;
  }));
  // Rebuilt only when there is something new to show: a rebuild takes any
  // rename being typed along with it.
  const shown = JSON.stringify(state.regions);
  if (shown !== regionsShown) regionList();
  regionsShown = shown;
}
let regionsShown = null;

function regionList() {
  const list = el('region-list');
  el('nregions').textContent = state.regions.length || '';
  list.replaceChildren(...state.regions.map(regionRow));
  if (!state.regions.length) {
    const li = document.createElement('li');
    li.className = 'none';
    li.textContent = 'None yet. Name a selection above to save one.';
    list.append(li);
  }
  regionButtons();
}

// What depends on the selection rather than on the regions: the selection
// counts as a user too, since deleting a region it refers to would leave it
// selecting nothing.
function regionButtons() {
  for (const li of el('region-list').querySelectorAll('li[data-name]')) {
    const r = state.regions.find((x) => x.name === li.dataset.name);
    if (!r) continue;
    const users = refersTo(state.sel, r.name) ? [...r.users, 'the selection'] : r.users;
    const del = li.querySelector('.del');
    del.disabled = users.length > 0;
    del.title = users.length ? `Still used by ${users.join(', ')}` : 'Delete the region';
    li.classList.toggle('current', r.name === state.region);
  }
}

function regionRow(r) {
  const li = document.createElement('li');
  li.dataset.name = r.name;

  // A button, so a region can be loaded from the keyboard; the rest of the
  // row does the same for the pointer.
  const name = document.createElement('button');
  name.type = 'button';
  name.className = 'name';
  name.textContent = r.name;
  name.title = 'Load as the selection';

  const used = document.createElement('span');
  used.className = 'type';
  used.textContent = r.users.length ? `used \u00d7${r.users.length}` : 'unused';
  used.title = r.users.length ? `Used by ${r.users.join(', ')}` : 'Nothing refers to it yet';

  // Its own button rather than a double-click on the name: a click on the
  // row loads the region, so the first half of a double-click would already
  // have replaced the selection.
  const ren = document.createElement('button');
  ren.type = 'button';
  ren.className = 'ctl ren';
  ren.textContent = 'Rename';
  ren.setAttribute('aria-label', `Rename region ${r.name}`);
  ren.addEventListener('click', (e) => {
    e.stopPropagation();
    renameRegion(name, r.name);
  });

  const del = document.createElement('button');
  del.type = 'button';
  del.className = 'ctl del';
  del.textContent = '\u2212';
  del.setAttribute('aria-label', `Delete region ${r.name}`);
  del.addEventListener('click', (e) => {
    e.stopPropagation();
    if (!layerEdit({ op: 'region', name: r.name, mask: null })) return;
    if (el('region-name').value.trim() === r.name) el('region-name').value = '';
    maskTarget();
    say(`Deleted region ${r.name}`);
  });

  li.append(name, used, ren, del);
  li.addEventListener('click', () => editRegion(r));
  return li;
}

// A rename made from the list, as `[from, to]`, while its edit is applied.
let renaming = null;

// A region renamed some other way - by an undo or redo of a rename, or by
// hand in the Scene tab - shows as a name gone and a name come with the same
// definition. Only pairs that match one to one are taken for renames.
function guessRenames(before, after) {
  const has = (list, n) => list.some((r) => r.name === n);
  const gone = before.filter((r) => !has(after, r.name));
  const come = after.filter((r) => !has(before, r.name));
  const out = [];
  for (const g of gone) {
    const to = come.filter((c) => c.mask === g.mask);
    if (to.length === 1 && gone.filter((x) => x.mask === g.mask).length === 1) out.push([g.name, to[0].name]);
  }
  return out;
}

// Everything the page holds that refers to a renamed region follows it, as
// the scene's own references do: the selection, its history, and the region
// being edited.
function followRename(from, to) {
  const move = (json) => {
    const { sel, region } = JSON.parse(json);
    return JSON.stringify({ sel: renameRef(sel, from, to), region: region === from ? to : region });
  };
  state.sel = renameRef(state.sel, from, to);
  state.selUndo = state.selUndo.map(move);
  state.selRedo = state.selRedo.map(move);
  if (state.region === from) editing(to);
  else if (el('region-name').value.trim() === from) el('region-name').value = to;
}

// Whether `spec` refers to region `name` anywhere in it.
function refersTo(spec, name) {
  if (!spec || typeof spec !== 'object' || !name) return false;
  if (spec.ref === name) return true;
  return Object.values(spec).some((v) => refersTo(v, name));
}

// A copy of `spec` with every `ref: from` made `ref: to`.
function renameRef(spec, from, to) {
  if (Array.isArray(spec)) return spec.map((v) => renameRef(v, from, to));
  if (!spec || typeof spec !== 'object') return spec;
  const out = {};
  for (const [k, v] of Object.entries(spec)) out[k] = k === 'ref' && v === from ? to : renameRef(v, from, to);
  return out;
}

// Loads a region's own definition - not a reference to it - so that what is
// saved back replaces it rather than wrapping it.
function editRegion(r) {
  try {
    setSel(JSON.parse(state.session.spec_json(r.mask)));
  } catch (e) {
    return fail(e);
  }
  editing(r.name);
}

// Marks which region the selection came from, if any, and makes the name box
// say so: the box is what Save writes to, so it must never go on naming a
// region the selection no longer comes from.
function editing(name) {
  state.region = name;
  el('region-name').value = name ?? '';
  regionButtons();
  maskTarget();
}

function renameRegion(span, from) {
  const input = document.createElement('input');
  input.className = 'rename';
  input.value = from;
  input.setAttribute('aria-label', 'Region name');
  let done = false;
  const finish = (keep) => {
    if (done) return;
    done = true;
    const to = input.value.trim();
    if (!keep || !to || to === from) return regionList();
    // The selection, and whatever else names the region, follow it by way
    // of regionMenu.
    renaming = [from, to];
    let ok;
    try {
      ok = layerEdit({ op: 'rename-region', from, to });
    } finally {
      renaming = null;
    }
    if (ok) say(`Renamed region ${from} to ${to}`);
    else regionList();
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

for (const b of document.querySelectorAll('#select-menu button')) {
  b.addEventListener('click', () => (el('select-menu').open = false));
}

// ---------------------------------------------------------------- refine

// Dragging a slider previews on every step and is one step of history when
// let go.
let refineFrom = null;
for (const key of ['grow', 'smooth', 'feather']) {
  const input = el(`refine-${key}`);
  input.addEventListener('input', () => {
    if (!state.sel) return;
    refineFrom ??= snapshot();
    const next = { ...state.sel };
    const v = +input.value;
    if (v) next[key] = v;
    else delete next[key];
    state.sel = next;
    selChanged();
  });
  input.addEventListener('change', () => {
    if (refineFrom === null) return;
    state.selUndo.push(refineFrom);
    state.selRedo = [];
    refineFrom = null;
  });
}

// ---------------------------------------------------------------- keys

document.addEventListener('keydown', (e) => {
  if (state.tab !== 'draw' || !state.session?.width) return;
  if (e.target.closest?.('input, textarea, select')) return;
  const mod = e.ctrlKey || e.metaKey;
  const act = (f) => {
    e.preventDefault();
    f();
  };
  if (mod && !e.altKey && e.code === 'KeyA') return act(selectAll);
  if (mod && !e.altKey && e.code === 'KeyD') return act(deselect);
  // Ctrl+Shift+I is the browser's own, so Photoshop's other binding is used.
  if ((e.shiftKey && e.key === 'F7') || (mod && e.altKey && e.code === 'KeyI')) return act(invert);
  if (mod || e.altKey) return;
  const tool = tools[state.tool];
  if (e.key === 'Escape' && state.draft) return act(cancelDraft);
  if (e.key === 'Enter' && state.draft && tool.close) return act(() => tool.close());
  if (e.key === 'Backspace' && state.draft && tool.back) return act(() => tool.back());
  // A tool's key again steps through the tools that share it.
  const group = [...document.querySelectorAll(`.tools [data-key="${e.key.toLowerCase()}"]`)].map((b) => b.dataset.tool);
  if (group.length && !e.shiftKey) {
    const at = group.indexOf(state.tool);
    act(() => pickTool(group[(at + 1) % group.length]));
  }
});

// ---------------------------------------------------------------- drawing the ui

function outlinePath(build) {
  uctx.beginPath();
  build();
  uctx.lineJoin = 'round';
  uctx.lineWidth = 3 * devicePixelRatio;
  uctx.strokeStyle = 'rgba(0, 0, 0, 0.55)';
  uctx.stroke();
  uctx.lineWidth = 1.25 * devicePixelRatio;
  uctx.strokeStyle = '#fff';
  uctx.stroke();
}

function polyline(pts, closed) {
  pts.forEach((p, i) => uctx[i ? 'lineTo' : 'moveTo'](p.x * ui.width, p.y * ui.height));
  if (closed) uctx.closePath();
}

// Anchor squares; the first grows when the pointer is close enough to close
// the outline on it.
function handles(pts, closing) {
  const dpr = devicePixelRatio;
  uctx.fillStyle = `rgb(${token('--accent').join(',')})`;
  uctx.strokeStyle = '#000';
  uctx.lineWidth = dpr;
  pts.forEach((p, i) => {
    const r = (i === 0 && closing ? 5 : 3) * dpr;
    uctx.fillRect(p.x * ui.width - r, p.y * ui.height - r, r * 2, r * 2);
    uctx.strokeRect(p.x * ui.width - r, p.y * ui.height - r, r * 2, r * 2);
  });
}

function drawUi() {
  uctx.clearRect(0, 0, ui.width, ui.height);
  const s = state.session;
  if (state.tab !== 'draw' || !s?.width) return;
  const dpr = devicePixelRatio;
  const [cx, cy] = [ui.width / s.width, ui.height / s.height];

  // Marching ants: a white dash over a black one, walked along by the timer.
  if (state.runs.length) {
    uctx.beginPath();
    for (const [x0, y0, x1, y1] of state.runs) {
      uctx.moveTo(x0 * cx, y0 * cy);
      uctx.lineTo(x1 * cx, y1 * cy);
    }
    uctx.lineWidth = dpr;
    uctx.setLineDash([]);
    uctx.strokeStyle = '#000';
    uctx.stroke();
    uctx.setLineDash([4 * dpr, 4 * dpr]);
    uctx.lineDashOffset = -antsOffset * dpr;
    uctx.strokeStyle = '#fff';
    uctx.stroke();
    uctx.setLineDash([]);
  }

  if (state.draft) tools[state.tool].draw?.(state.draft);

  if (state.tool === 'brush' && state.hover) {
    const r = +el('opt-radius').value * cx;
    uctx.beginPath();
    uctx.arc(state.hover.x * ui.width, state.hover.y * ui.height, r, 0, Math.PI * 2);
    uctx.lineWidth = 3 * dpr;
    uctx.strokeStyle = 'rgba(0, 0, 0, 0.55)';
    uctx.stroke();
    uctx.lineWidth = dpr;
    uctx.strokeStyle = '#fff';
    uctx.stroke();
  }
}

setInterval(() => {
  if (state.tab !== 'draw' || !state.runs.length || document.hidden) return;
  antsOffset = (antsOffset + 1) % 8;
  drawUi();
}, 120);

// ---------------------------------------------------------------- apply

// The Apply button: which layer it would write to, and if not, why not.
function maskTarget() {
  const layer = state.selected === null ? null : state.layers[state.selected];
  const b = el('apply-mask');
  const any = state.selCov?.some((v) => v > 0);
  const why = !state.sel ? 'Select something first'
    : !any ? 'The selection is empty'
      : !layer ? 'Select a layer in the Layers tab' : null;
  b.disabled = !!why;
  el('apply-menu').inert = !!why;
  if (why) el('apply-menu').open = false;
  b.textContent = layer ? `Apply to ${layer.label}` : 'Apply';
  b.title = why ?? `Make the selection the mask of ${layer.label}`;
  el('sel-layer').disabled = !layer;
  el('sel-layer-name').textContent = layer?.label ?? '';
  const name = el('region-name').value.trim();
  const save = el('save-region');
  save.disabled = !state.sel || !any || !name;
  const exists = state.regions.some((r) => r.name === name);
  save.textContent = exists ? `Update ${name}` : 'Save as region';
  save.title = exists ? `Replace the definition of ${name} with the selection` : 'Save the selection under this name';
  el('copy').disabled = !state.sel;
}

function applySel(mode) {
  const i = state.selected;
  if (i === null || !state.sel) return;
  const verb = { replace: 'Set as', add: 'Added to', sub: 'Subtracted from', and: 'Intersected with' }[mode];
  if (layerEdit({ op: 'combine', i, mask: JSON.stringify(state.sel), mode })) {
    say(`${verb} the mask of ${state.layers[i].label}`);
  }
}

el('apply-mask').addEventListener('click', () => applySel('replace'));
for (const b of document.querySelectorAll('#apply-menu [data-combine]')) {
  b.addEventListener('click', () => {
    el('apply-menu').open = false;
    applySel(b.dataset.combine);
  });
}

el('region-name').addEventListener('input', maskTarget);
el('region-form').addEventListener('submit', (e) => {
  e.preventDefault();
  const name = el('region-name').value.trim();
  if (!name || !state.sel) return;
  const exists = state.regions.some((r) => r.name === name);
  if (layerEdit({ op: 'region', name, mask: JSON.stringify(state.sel) })) {
    // Saved, it is the region being worked on: another Update replaces it.
    editing(name);
    say(`${exists ? 'Updated' : 'Saved'} region ${name}`);
  }
});

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
  state.saving = true;
  try {
    await run();
  } catch (e) {
    fail(e);
  } finally {
    state.saving = false;
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
  const summary = el('save').querySelector('summary');
  const session = state.session;
  const { width, height, frames } = session;
  const scale = 3;
  const fps = Math.max(1, session.fps);

  // The recorder stamps each frame with the moment it arrives, so the file is
  // exactly as long as the recording took. Rendering inside the timed loop
  // let a big scene's render time stretch the video far past frames/fps; the
  // loop is rendered first, at grid size so it stays small, and the timed
  // pass only has to blit.
  const loop = [];
  for (let i = 0; i < frames; i++) {
    summary.textContent = `Rendering ${i + 1}/${frames}…`;
    // A timeout rather than an animation frame: enough for the label to
    // paint, and it keeps going if the tab is sent to the background.
    await new Promise((r) => setTimeout(r, 0));
    loop.push(new ImageData(new Uint8ClampedArray(session.frame(i, 1)), width, height));
  }
  summary.textContent = 'Recording…';

  const grid = document.createElement('canvas');
  grid.width = width;
  grid.height = height;
  const gx = grid.getContext('2d');
  const c = document.createElement('canvas');
  c.width = width * scale;
  c.height = height * scale;
  const cx = c.getContext('2d');
  // Nearest-neighbour, so the upscale is the same hard-edged one the
  // renderer's own `scale` does.
  cx.imageSmoothingEnabled = false;

  const stream = c.captureStream(0);
  const track = stream.getVideoTracks()[0];
  // The standard puts requestFrame on the track; Firefox only has it on the
  // stream.
  const requestFrame = track.requestFrame ? () => track.requestFrame() : () => stream.requestFrame();
  const chunks = [];
  const rec = new MediaRecorder(stream, { mimeType: VIDEO, videoBitsPerSecond: 12e6 });
  rec.ondataavailable = (e) => e.data.size && chunks.push(e.data);
  const done = new Promise((r) => (rec.onstop = r));
  rec.start();

  const start = performance.now();
  for (let i = 0; i < frames; i++) {
    gx.putImageData(loop[i], 0, 0);
    cx.drawImage(grid, 0, 0, c.width, c.height);
    // Pushing frames explicitly rather than letting the stream sample the
    // canvas is what keeps the recording frame-exact, and so still a loop.
    requestFrame();
    // Against a fixed schedule rather than a fixed pause, so the time the
    // blit itself takes does not accumulate into the length.
    const due = start + ((i + 1) * 1000) / fps;
    await new Promise((r) => setTimeout(r, Math.max(0, due - performance.now())));
  }

  rec.stop();
  await done;
  download(new Blob(chunks, { type: VIDEO }), `${stem(state.name)}-loop.${VIDEO_EXT}`);
}));

// A menu left open after the pointer has gone elsewhere is just a panel in
// the way.
document.addEventListener('click', (e) => {
  for (const id of ['save', 'layer-menu', 'add', 'select-menu', 'apply-menu']) if (!el(id).contains(e.target)) el(id).open = false;
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
window.pixelgen = { state, open, apply, edit: layerEdit, undo, setSel };
