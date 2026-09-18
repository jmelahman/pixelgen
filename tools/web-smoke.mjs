// Loads web/ in headless Chromium, feeds it a synthetic photograph and drives
// the editor through one full cycle. Nothing here checks how the result looks;
// it checks that the page runs at all, which unit tests on the Rust side
// cannot tell you.
//
//   ./build-web.sh && node tools/web-smoke.mjs

import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, writeFile } from 'node:fs/promises';
import { delimiter, extname, join, normalize } from 'node:path';
import { homedir } from 'node:os';
import { existsSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const ROOT = new URL('../web/', import.meta.url).pathname;
const TYPES = {
  '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css',
  '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.png': 'image/png',
};

const server = createServer(async (req, res) => {
  const path = normalize(decodeURI(req.url.split('?')[0])).replace(/^(\.\.[/\\])+/, '');
  const file = join(ROOT, path === '/' ? 'index.html' : path);
  try {
    const body = await readFile(file);
    res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404).end('not found');
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/`;

// prek installs a hook's packages into its own env prefix rather than a
// node_modules beside us, and an ESM bare specifier looks in neither (NODE_PATH
// is a CommonJS-only mechanism). So fall back to walking the prefixes prek does
// put on PATH.
async function importBrowsers() {
  try {
    return await import('@puppeteer/browsers');
  } catch (e) {
    if (e.code !== 'ERR_MODULE_NOT_FOUND') throw e;
  }
  const roots = [
    ...(process.env.NODE_PATH ?? '').split(delimiter),
    ...(process.env.PATH ?? '').split(delimiter).map((p) => join(p, '..', 'lib', 'node_modules')),
  ].filter(Boolean);
  for (const root of roots) {
    const entry = join(root, '@puppeteer', 'browsers', 'lib', 'main.js');
    if (existsSync(entry)) return await import(pathToFileURL(entry).href);
  }
  throw new Error('no browser found and @puppeteer/browsers is not installed; set $CHROME');
}

// A browser, in order of preference: $CHROME, a system install, or a
// chrome-headless-shell downloaded into ~/.cache/pixelgen/browsers. The
// download is what makes this runnable on a machine with no Chrome at all;
// @puppeteer/browsers is supplied by the hook (and by `npm i` for a manual
// run), so its absence is only fatal when we actually need to download.
async function findChrome() {
  if (process.env.CHROME) return process.env.CHROME;
  for (const name of ['chromium', 'chromium-browser', 'google-chrome', 'google-chrome-stable']) {
    for (const dir of (process.env.PATH ?? '').split(delimiter)) {
      const candidate = join(dir, name);
      if (existsSync(candidate)) return candidate;
    }
  }
  const { Browser, install, resolveBuildId, detectBrowserPlatform } = await importBrowsers();
  const cacheDir = join(homedir(), '.cache', 'pixelgen', 'browsers');
  const browser = Browser.CHROMEHEADLESSSHELL;
  const platform = detectBrowserPlatform();
  const buildId = await resolveBuildId(browser, platform, 'stable');
  const installed = await install({ browser, buildId, cacheDir, platform });
  return installed.executablePath;
}

const chrome = await findChrome();
const proc = spawn(chrome, [
  '--headless=new', '--disable-gpu', '--no-sandbox',
  '--remote-debugging-port=0', '--user-data-dir=' + (process.env.TMPDIR ?? '/tmp') + '/pixelgen-smoke',
  'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });

// Chromium announces the port it actually took on stderr.
const wsUrl = await new Promise((resolve, reject) => {
  let buf = '';
  proc.stderr.on('data', (d) => {
    buf += d;
    const m = buf.match(/ws:\/\/[^\s]+/);
    if (m) resolve(m[0]);
  });
  proc.on('exit', (c) => reject(new Error(`${chrome} exited with ${c}`)));
  setTimeout(() => reject(new Error('chromium did not report a debugging url')), 15000);
});

// That endpoint is the browser itself, which has no Runtime domain. The page
// target has its own, listed over HTTP on the same port.
const port = new URL(wsUrl).port;
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = targets.find((t) => t.type === 'page');
if (!page) throw new Error('chromium exposed no page target');

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));

let id = 0;
const waiting = new Map();
const errors = [];
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id) waiting.get(msg.id)?.(msg);
  else if (msg.method === 'Runtime.consoleAPICalled' && msg.params.type === 'error') {
    errors.push(msg.params.args.map((a) => a.value ?? a.description).join(' '));
  } else if (msg.method === 'Runtime.exceptionThrown') {
    errors.push(msg.params.exceptionDetails.exception?.description ?? 'exception');
  }
};

const send = (method, params = {}) =>
  new Promise((resolve) => {
    const n = ++id;
    waiting.set(n, resolve);
    ws.send(JSON.stringify({ id: n, method, params }));
  });

const evaluate = async (expression) => {
  const { result } = await send('Runtime.evaluate', {
    expression, awaitPromise: true, returnByValue: true,
  });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception.description);
  if (result.result.subtype === 'error') throw new Error(result.result.description);
  return result.result.value;
};

await send('Runtime.enable');
await send('Page.enable');
// Size and theme before the first paint: the editor scales the plate to the
// room it has, and a theme switched after the page has been painted leaves
// headless compositing tiles from the old one in the screenshot.
await send('Emulation.setDeviceMetricsOverride', {
  width: Number(process.env.SHOT_WIDTH ?? 1440),
  height: Number(process.env.SHOT_HEIGHT ?? 900),
  deviceScaleFactor: 1,
  mobile: false,
});
await send('Emulation.setEmulatedMedia', {
  features: [{ name: 'prefers-color-scheme', value: process.argv.includes('--light') ? 'light' : 'dark' }],
});
await send('Page.navigate', { url });
// The module does top-level await on the wasm init; poll rather than guess.
let ready = false;
for (let i = 0; i < 100 && !ready; i++) {
  ready = await evaluate('!!window.pixelgen').catch(() => false);
  if (!ready) await new Promise((r) => setTimeout(r, 100));
}
// Falling through to the driving below would report the page's failure to
// start as an unrelated TypeError on `undefined`. Whatever the page logged on
// its way down is the thing worth reading.
if (!ready) {
  console.error('the page never started; console said:', errors.length ? errors : '(nothing)');
  process.exit(1);
}

// --empty stops before an image is opened, which is the only way to look at
// the state the page is actually first seen in.
const empty = process.argv.includes('--empty');

const out = empty ? { boxes: [[0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]], grid: [0, 0] } : await evaluate(`(async () => {
  // A cool upper half, a warm lower half, a small bright warm lamp: the shape
  // the starter analysis is looking for.
  const c = document.createElement('canvas');
  c.width = 400; c.height = 300;
  const cx = c.getContext('2d');
  cx.fillStyle = '#466ebe'; cx.fillRect(0, 0, 400, 150);
  cx.fillStyle = '#785a3c'; cx.fillRect(0, 150, 400, 150);
  cx.fillStyle = '#fad278'; cx.beginPath(); cx.arc(120, 240, 14, 0, 7); cx.fill();
  const blob = await new Promise((r) => c.toBlob(r));

  // Opening an image is supposed to leave a scene on screen by itself.
  await window.pixelgen.open(new File([blob], 'smoke.png'));

  const s = window.pixelgen.state.session;
  const view = document.getElementById('view');

  // The title block's controls, which write to the YAML rather than to the
  // session: each has to land in the text and come back out of the renderer.
  const set = (id, v) => {
    const input = document.getElementById(id);
    input.value = v;
    input.dispatchEvent(new Event('change'));
  };
  set('set-width', 96);
  set('set-seconds', 2);
  set('set-fps', 10);
  set('set-colors', 12);
  document.querySelector('#layer-toggles input').click();
  const yaml = document.getElementById('yaml').value;
  const titleblock = {
    width: s.width, frames: s.frames, colors: s.colors,
    count: document.getElementById('layer-count').textContent,
    text: ['width: 96', 'seconds: 2', 'fps: 10', 'colors: 12', 'disable: true'].filter((l) => !yaml.includes(l)),
    comments: yaml.startsWith('# Generated'),
  };
  document.querySelector('#layer-toggles input').click();
  titleblock.reenabled = !document.getElementById('yaml').value.includes('disable: true');

  // A typed number is held to the field's own bounds rather than written as
  // typed, and the edit is a step in the box's undo history rather than the
  // end of it.
  set('set-fps', 1000);
  titleblock.clamped = s.fps;
  // Undone from the box itself, which means going to the tab it is on - the
  // Layers tab is the one open by default.
  document.querySelector('.tabs [data-tab="scene"]').click();
  const box = document.getElementById('yaml');
  box.focus();
  document.execCommand('undo');
  box.blur();
  window.pixelgen.apply();
  titleblock.undone = s.fps;
  document.querySelector('.tabs [data-tab="layers"]').click();

  // Playback, a scrub, and a mask overlay: the three things that touch the
  // canvas from different directions.
  document.getElementById('play').click();
  await new Promise((r) => setTimeout(r, 400));
  document.getElementById('play').click();
  document.getElementById('scrub').value = 5;
  document.getElementById('scrub').dispatchEvent(new Event('input'));
  document.querySelector('#layers li').click();
  document.getElementById('showmask').checked = true;
  document.getElementById('showmask').dispatchEvent(new Event('change'));

  // The layer panel, which edits the scene without the YAML being touched:
  // add a layer from the menu, change one of its parameters, then switch a
  // layer off. Each is read back from the session, not the panel, so a
  // panel that only looks edited fails.
  const panel = { before: s.layers.length };
  document.querySelector('#add summary').click();
  [...document.querySelectorAll('#catalog button')]
    .find((b) => b.firstChild.textContent === 'vignette').click();
  panel.added = s.layers.length;
  panel.yaml = document.getElementById('yaml').value;
  const amount = document.getElementById('param-amount');
  amount.value = '0.55';
  amount.dispatchEvent(new Event('change'));
  panel.param = document.getElementById('yaml').value;
  document.querySelector('#layers li .eye').click();
  panel.hidden = s.layers.length;
  panel.rows = document.querySelectorAll('#layers li').length;
  panel.layerError = document.getElementById('layer-error').hidden
    ? null : document.getElementById('layer-error').textContent;

  // The selection tools, driven by pointer events on the canvas they listen
  // to, so the mapping from screen space back to the 0..1 the scene format
  // uses is exercised rather than bypassed.
  document.querySelector('.tabs [data-tab="draw"]').click();
  const ui = document.getElementById('ui');
  const st = window.pixelgen.state;
  const ptr = (type, [x, y], mods = {}) => {
    const r = ui.getBoundingClientRect();
    ui.dispatchEvent(new PointerEvent(type, {
      clientX: r.left + x * r.width, clientY: r.top + y * r.height,
      bubbles: true, button: 0, pointerId: 1, ...mods,
    }));
  };
  const drag = (pts, mods) => {
    ptr('pointerdown', pts[0], mods);
    for (const p of pts.slice(1)) ptr('pointermove', p, mods);
    ptr('pointerup', pts.at(-1), mods);
  };
  const key = (k, mods = {}) => document.body.dispatchEvent(new KeyboardEvent('keydown', {
    key: k, code: k.length === 1 ? 'Key' + k.toUpperCase() : k, bubbles: true, ...mods,
  }));
  const tool = (t) => document.querySelector('.tools [data-tool="' + t + '"]').click();
  const ops = () => st.sel?.steps?.map((x) => Object.keys(x)[0]) ?? [];
  const covered = () => (st.selCov ?? []).reduce((a, v) => a + v, 0);
  const layerMask = () => st.layers[st.selected]?.mask ?? null;
  const sel = {};

  tool('rect');
  drag([[0.1, 0.1], [0.3, 0.2], [0.4, 0.4]]);
  drag([[0.5, 0.5], [0.7, 0.7]], { shiftKey: true });
  drag([[0.15, 0.15], [0.2, 0.2]], { altKey: true });
  sel.ops = ops();
  sel.ants = st.runs.length;

  // Ctrl+Z takes back a selection step and leaves the scene alone.
  const text = document.getElementById('yaml').value;
  key('z', { ctrlKey: true });
  sel.undone = ops();
  sel.sceneKept = document.getElementById('yaml').value === text;

  document.getElementById('apply-mask').click();
  sel.replaced = layerMask();
  drag([[0.6, 0.1], [0.8, 0.3]]);
  document.querySelector('#apply-menu [data-combine="add"]').click();
  sel.added = layerMask();

  document.getElementById('region-name').value = 'smoke';
  document.getElementById('region-name').dispatchEvent(new Event('input'));
  document.getElementById('region-form').requestSubmit();
  const region = (n) => JSON.parse(s.regions()).find((r) => r.name === n);
  const row = (n) => [...document.querySelectorAll('#region-list li[data-name]')]
    .find((li) => li.dataset.name === n);
  const nameBox = document.getElementById('region-name');
  const saveLabel = () => document.getElementById('save-region').textContent;
  // Only the page's own guards here: a missing row or button is reported by
  // the checks below rather than thrown.
  const pick = (n) => [...document.querySelectorAll('#sel-regions button')]
    .find((b) => b.firstChild.textContent === n)?.click();
  sel.region = !!region('smoke') && document.getElementById('yaml').value.includes('smoke:');

  // Editing it in place: its own definition comes back as the selection, and
  // saving under the same name replaces it.
  const saved = region('smoke')?.mask;
  const count = JSON.parse(s.regions()).length;
  // Saving made it the region being edited; start from nothing loaded.
  key('d', { ctrlKey: true });
  row('smoke')?.querySelector('.name')?.click();
  const regions = {
    loaded: !!st.sel && !st.sel.ref && st.region === 'smoke',
    label: saveLabel(),
  };
  // Stepping back past the load leaves nothing set to be overwritten, and
  // stepping forward again sets it back.
  key('z', { ctrlKey: true });
  regions.undoDisarms = st.region === null && nameBox.value === '' && saveLabel() === 'Save as region';
  key('z', { ctrlKey: true, shiftKey: true });
  regions.redoRearms = st.region === 'smoke' && saveLabel() === 'Update smoke';
  drag([[0.05, 0.8], [0.15, 0.9]], { shiftKey: true });
  document.getElementById('region-form').requestSubmit();
  regions.updated = region('smoke')?.mask !== saved && JSON.parse(s.regions()).length === count;

  // Renamed while a layer uses it: the layer's reference follows, and so do
  // the selection and its history, which the rename must not replace.
  pick('smoke');
  regions.pickDisarms = st.region === null && nameBox.value === '';
  document.getElementById('apply-mask').click();
  drag([[0.4, 0.8], [0.5, 0.9]], { shiftKey: true });
  const held = JSON.stringify(st.sel);
  row('smoke')?.querySelector('.ren')?.click();
  const rename = document.querySelector('#region-list input.rename');
  if (rename) {
    rename.value = 'haze';
    rename.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  }
  regions.renamed = !region('smoke') && /ref: haze/.test(layerMask() ?? '');
  regions.selFollowed = JSON.stringify(st.sel) === held.replace('"smoke"', '"haze"');
  key('z', { ctrlKey: true });
  regions.undoFollowed = st.sel?.ref === 'haze' && covered() > 0;
  regions.inUse = row('haze')?.querySelector('.del')?.disabled;

  // One the selection uses cannot be deleted; once nothing uses it, it can.
  nameBox.value = 'spare';
  nameBox.dispatchEvent(new Event('input'));
  document.getElementById('region-form').requestSubmit();
  pick('spare');
  regions.selBlocks = row('spare')?.querySelector('.del')?.disabled;
  key('d', { ctrlKey: true });
  regions.deselectDisarms = nameBox.value === '' && saveLabel() === 'Save as region';
  row('spare')?.querySelector('.del')?.click();
  regions.deleted = !!region('haze') && !region('spare');
  sel.regions = regions;

  // The wand, on the flat cool upper half: nothing below the horizon.
  tool('wand');
  ptr('pointerdown', [0.5, 0.2]);
  ptr('pointerup', [0.5, 0.2]);
  {
    const w = s.width, h = s.height, c = st.selCov;
    let top = 0, n = 0, low = 0;
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      if (y < h * 0.4) { top += c[y * w + x] > 127; n++; }
      if (y > h * 0.6) low += c[y * w + x] > 0;
    }
    sel.wand = { top: top / n, low, spec: st.sel.steps[0].add.wand };
  }

  // The pen: three corners, the middle one dragged out into a curve.
  tool('pen');
  ptr('pointerdown', [0.2, 0.2]); ptr('pointerup', [0.2, 0.2]);
  drag([[0.5, 0.2], [0.6, 0.1]]);
  ptr('pointerdown', [0.4, 0.5]); ptr('pointerup', [0.4, 0.5]);
  key('Enter');
  sel.pen = st.sel?.steps?.[0]?.add?.path ?? null;

  // The brush, erasing across a box.
  tool('rect');
  drag([[0.1, 0.1], [0.6, 0.6]]);
  const full = covered();
  tool('brush');
  drag([[0.2, 0.35], [0.3, 0.35], [0.5, 0.35]], { altKey: true });
  sel.erase = { before: full, after: covered(), ops: ops() };
  sel.summary = document.getElementById('sel-summary').textContent;

  return {
    titleblock,
    grid: [view.width, view.height],
    // The drawn size of both canvases. They must agree: a click on the overlay
    // becomes a fraction of the frame by way of its box, so the two boxes
    // drifting apart would put every drawn point in the wrong place - and a
    // plate left at 1x in a large viewport is a defect no assertion above sees.
    // The room the plate had to fill, for the fit assertion below.
    room: (() => {
      const b = document.getElementById('viewport').getBoundingClientRect();
      return [Math.round(b.width), Math.round(b.height)];
    })(),
    boxes: [view, overlay, ui].map((c) => {
      const b = c.getBoundingClientRect();
      return [Math.round(b.left), Math.round(b.top), Math.round(b.width), Math.round(b.height)];
    }),
    frames: s.frames,
    fps: s.fps,
    colors: s.palette.length,
    layers: s.layers,
    panel,
    counter: document.getElementById('counter').textContent,
    sel,
    error: document.getElementById('error').hidden ? null : document.getElementById('error').textContent,
    scene: document.getElementById('yaml').value.slice(0, 40),
    // The GIF encoder is the renderer's own, so it is worth knowing it still
    // produces a GIF rather than only that the button exists.
    gif: (() => {
      const b = s.gif(1);
      return { bytes: b.length, magic: String.fromCharCode(...b.slice(0, 6)) };
    })(),
    // Relabeled at load to whatever the browser can actually record.
    video: document.getElementById('save-video').textContent.trim(),
    // Wide enough that a realtime recorder would fall behind and stutter:
    // the file has to be exactly frames / fps long anyway.
    videoFile: await (async () => {
      const scale = Math.ceil(1100 / s.width);
      // A hung encoder or player would otherwise hang the hook with it.
      const within = (p, what) => Promise.race([
        p, new Promise((_, j) => setTimeout(() => j(new Error(what + ' timed out')), 60000)),
      ]);
      const blob = await within(window.pixelgen.video(scale), 'video export');
      const head = new Uint8Array(await blob.slice(0, 4).arrayBuffer());
      const v = document.createElement('video');
      v.src = URL.createObjectURL(blob);
      await within(new Promise((r, j) => {
        v.onloadedmetadata = r;
        v.onerror = () => j(new Error('video: ' + v.error?.message));
      }), 'loading the video');
      URL.revokeObjectURL(v.src);
      return {
        type: blob.type,
        magic: [...head].map((b) => b.toString(16).padStart(2, '0')).join(''),
        size: [v.videoWidth, v.videoHeight],
        // Trimmed to even, as the codec needs.
        expect: [(s.width * scale) & ~1, (s.height * scale) & ~1],
        duration: v.duration,
        expected: s.frames / s.fps,
        frame: 1 / s.fps,
      };
    })(),
    status: document.getElementById('status').textContent,
    // A blank canvas is the failure this whole test exists to catch.
    painted: (() => {
      const d = view.getContext('2d').getImageData(0, 0, view.width, view.height).data;
      const first = d.slice(0, 3).join();
      for (let i = 0; i < d.length; i += 4) if (d.slice(i, i + 3).join() !== first) return true;
      return false;
    })(),
  };
})()`);

if (!empty) console.log(out);
if (errors.length) console.log('console errors:', errors);

// --shot writes a full-page screenshot, which is the only way to actually
// look at a change to the stylesheet without a browser open.
const shot = process.argv.indexOf('--shot');
if (shot !== -1) {
  const path = process.argv[shot + 1] ?? 'shot.png';
  const { data } = (await send('Page.captureScreenshot', { format: 'png' })).result;
  await writeFile(path, Buffer.from(data, 'base64'));
  console.log('wrote', path);
}

ws.close();
proc.kill();
server.close();

const bad = [];
if (empty) { console.log('ok (empty, no assertions run)'); process.exit(0); }
if (!out.painted) bad.push('canvas is a flat color');
if (out.error) bad.push('scene error: ' + out.error);
if (!out.layers.length) bad.push('no layers');
if (!out.scene.trim()) bad.push('opening an image left the scene box empty');
if (out.gif.magic !== 'GIF89a') bad.push('gif export produced ' + JSON.stringify(out.gif));
if (!/^(MP4|WEBM) /.test(out.video)) bad.push('video item is labeled ' + JSON.stringify(out.video));
const vf = out.videoFile;
if (vf.type !== 'video/webm' || vf.magic !== '1a45dfa3') bad.push('video export is not a WebM file: ' + JSON.stringify(vf));
if (vf.size.join() !== vf.expect.join()) bad.push('video export has the wrong size: ' + JSON.stringify(vf));
if (!(Math.abs(vf.duration - vf.expected) < vf.frame)) bad.push('video export is not frames / fps long: ' + JSON.stringify(vf));
if (new Set(out.boxes.map((b) => b.join())).size !== 1) bad.push('overlays are not over the plate: ' + JSON.stringify(out.boxes));
// The plate should be the largest whole multiple of the frame that fits: not
// left at 1x in a large viewport, and not blown past the edges of a small one.
const scale = out.boxes[0][2] / out.grid[0];
if (!Number.isInteger(scale) || scale < 1) bad.push('plate is not a whole multiple of the frame: ' + scale);
else if ((scale + 1) * out.grid[0] <= out.room[0] - 32 && (scale + 1) * out.grid[1] <= out.room[1] - 32) {
  bad.push(`plate sat at ${scale}x in a viewport with room for more: ` + JSON.stringify(out.room));
}
const p = out.panel;
if (p.layerError) bad.push('layer panel error: ' + p.layerError);
if (p.added !== p.before + 1) bad.push(`adding a layer went from ${p.before} to ${p.added} drawn`);
if (!/type: vignette/.test(p.yaml)) bad.push('the added layer is not in the scene');
if (!/amount: 0\.55/.test(p.param)) bad.push('the parameter edit is not in the scene');
if (p.hidden !== p.added - 1) bad.push(`hiding a layer left ${p.hidden} of ${p.added} drawn`);
if (p.rows !== p.added) bad.push(`a hidden layer should stay listed: ${p.rows} rows for ${p.added}`);
if (out.counter !== `6/${out.frames}`) bad.push('scrub did not move the frame: ' + out.counter);
const sel = out.sel;
if (sel.ops.join() !== 'add,add,sub') bad.push('drag, Shift-drag, Alt-drag gave steps ' + JSON.stringify(sel.ops));
if (!sel.ants) bad.push('a selection drew no marching ants');
if (sel.undone.join() !== 'add,add') bad.push('Ctrl+Z left the selection at ' + JSON.stringify(sel.undone));
if (!sel.sceneKept) bad.push('Ctrl+Z on the Select tab undid a scene edit rather than a selection step');
if (!/steps:/.test(sel.replaced ?? '')) bad.push('Apply did not set the layer mask: ' + JSON.stringify(sel.replaced));
if (!sel.added || sel.added === sel.replaced || !/- add:\s+rect:\s+x: 0\.6/.test(sel.added)) {
  bad.push('Apply > Add did not add to the layer mask: ' + JSON.stringify(sel.added));
}
if (!sel.region) bad.push('Save as region did not write regions.smoke');
const rg = sel.regions;
if (!rg.loaded || rg.label !== 'Update smoke') bad.push('clicking a region did not load it for editing: ' + JSON.stringify(rg));
if (!rg.updated) bad.push('Update did not replace the region in place: ' + JSON.stringify(rg));
if (!rg.renamed) bad.push('renaming a region did not carry its references along: ' + JSON.stringify(rg));
if (!rg.inUse) bad.push('a region in use could be deleted');
if (!rg.selFollowed || !rg.undoFollowed) bad.push('the selection did not follow a region rename: ' + JSON.stringify(rg));
if (!rg.selBlocks) bad.push('a region the selection uses could be deleted');
for (const k of ['undoDisarms', 'redoRearms', 'pickDisarms', 'deselectDisarms']) {
  if (!rg[k]) bad.push(`the save button still named a region it should not (${k}): ` + JSON.stringify(rg));
}
if (!rg.deleted) bad.push('deleting an unused region did not remove it: ' + JSON.stringify(rg));
if (sel.wand.top < 0.9 || sel.wand.low) bad.push('the wand leaked across the horizon: ' + JSON.stringify(sel.wand));
if (!/^M[^Z]*C[^Z]*Z$/.test(sel.pen ?? '')) bad.push('the pen wrote ' + JSON.stringify(sel.pen));
if (!(sel.erase.after < sel.erase.before) || sel.erase.ops.join() !== 'add,sub') {
  bad.push('an erase stroke did not take anything away: ' + JSON.stringify(sel.erase));
}
const tb = out.titleblock;
if (tb.text.length) bad.push('title block did not write ' + JSON.stringify(tb.text));
if (tb.width !== 96 || tb.frames !== 20 || tb.colors !== 12) bad.push('title block edits did not reach the renderer: ' + JSON.stringify(tb));
if (!/^\d+\/\d+$/.test(tb.count)) bad.push('disabling a layer left the count at ' + tb.count);
if (!tb.comments) bad.push('title block edits lost the scene comments');
if (!tb.reenabled) bad.push('re-enabling a layer left disable: true in the scene');
if (tb.clamped !== 60) bad.push('fps 1000 was written as ' + tb.clamped + ', not held to the field max of 60');
if (tb.undone !== 10) bad.push('undo in the scene box did not take back a title block edit: fps is ' + tb.undone);
if (errors.length) bad.push(errors.length + ' console errors');

if (bad.length) {
  console.error('FAIL:', bad.join('; '));
  process.exit(1);
}
console.log('ok');
