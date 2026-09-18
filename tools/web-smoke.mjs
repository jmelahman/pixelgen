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
  '.wasm': 'application/wasm',
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

const out = empty ? { boxes: [[0, 0, 0, 0], [0, 0, 0, 0]], grid: [0, 0] } : await evaluate(`(async () => {
  // A cool upper half, a warm lower half, a small bright warm lamp: the shape
  // the starter analysis is looking for.
  const c = document.createElement('canvas');
  c.width = 400; c.height = 300;
  const cx = c.getContext('2d');
  cx.fillStyle = '#466ebe'; cx.fillRect(0, 0, 400, 150);
  cx.fillStyle = '#785a3c'; cx.fillRect(0, 150, 400, 150);
  cx.fillStyle = '#fad278'; cx.beginPath(); cx.arc(120, 240, 14, 0, 7); cx.fill();
  const blob = await new Promise((r) => c.toBlob(r));

  // No click on #starter: opening an image is supposed to leave a scene on
  // screen by itself. The button only repeats it.
  await window.pixelgen.open(new File([blob], 'smoke.png'));

  const s = window.pixelgen.state.session;
  const view = document.getElementById('view');

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

  // The mask editor. Real clicks on the overlay, so the coordinate mapping
  // from screen space back to the 0..1 the scene format uses is exercised
  // rather than bypassed.
  document.querySelector('.tabs [data-tab="draw"]').click();
  const overlay = document.getElementById('overlay');
  const r = overlay.getBoundingClientRect();
  for (const [x, y] of [[0.1, 0.1], [0.9, 0.2], [0.5, 0.8]]) {
    overlay.dispatchEvent(new MouseEvent('click', {
      clientX: r.left + x * r.width, clientY: r.top + y * r.height, bubbles: true,
    }));
  }

  return {
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
    boxes: [view, overlay].map((c) => {
      const b = c.getBoundingClientRect();
      return [Math.round(b.left), Math.round(b.top), Math.round(b.width), Math.round(b.height)];
    }),
    frames: s.frames,
    fps: s.fps,
    colours: s.palette.length,
    layers: s.layers,
    counter: document.getElementById('counter').textContent,
    snippet: document.getElementById('snippet').value,
    error: document.getElementById('error').hidden ? null : document.getElementById('error').textContent,
    scene: document.getElementById('yaml').value.slice(0, 40),
    // The GIF encoder is the renderer's own, so it is worth knowing it still
    // produces a GIF rather than only that the button exists.
    gif: (() => {
      const b = s.gif(1);
      return { bytes: b.length, magic: String.fromCharCode(...b.slice(0, 6)) };
    })(),
    // Relabelled at load to whatever the browser can actually record.
    video: document.getElementById('save-video').textContent.trim(),
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
if (!out.painted) bad.push('canvas is a flat colour');
if (out.error) bad.push('scene error: ' + out.error);
if (!out.layers.length) bad.push('no layers');
if (!out.scene.trim()) bad.push('opening an image left the scene box empty');
if (out.gif.magic !== 'GIF89a') bad.push('gif export produced ' + JSON.stringify(out.gif));
if (!/^(MP4|WEBM) /.test(out.video)) bad.push('video item is labelled ' + JSON.stringify(out.video));
if (out.boxes[0].join() !== out.boxes[1].join()) bad.push('overlay is not over the plate: ' + JSON.stringify(out.boxes));
// The plate should be the largest whole multiple of the frame that fits: not
// left at 1x in a large viewport, and not blown past the edges of a small one.
const scale = out.boxes[0][2] / out.grid[0];
if (!Number.isInteger(scale) || scale < 1) bad.push('plate is not a whole multiple of the frame: ' + scale);
else if ((scale + 1) * out.grid[0] <= out.room[0] - 32 && (scale + 1) * out.grid[1] <= out.room[1] - 32) {
  bad.push(`plate sat at ${scale}x in a viewport with room for more: ` + JSON.stringify(out.room));
}
if (out.counter !== `6/${out.frames}`) bad.push('scrub did not move the frame: ' + out.counter);
if (!/^polygon:(\n\s+- \{ x: [\d.]+, y: [\d.]+ \}){3}$/.test(out.snippet)) {
  bad.push('mask editor emitted: ' + JSON.stringify(out.snippet));
}
if (errors.length) bad.push(errors.length + ' console errors');

if (bad.length) {
  console.error('FAIL:', bad.join('; '));
  process.exit(1);
}
console.log('ok');
