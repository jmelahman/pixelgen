// Loads web/ in headless Chromium, feeds it a synthetic photograph and drives
// the editor through one full cycle. Nothing here checks how the result looks;
// it checks that the page runs at all, which unit tests on the Rust side
// cannot tell you.
//
//   ./build-web.sh && node tools/web-smoke.mjs

import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';

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

const chrome = process.env.CHROME ?? 'chromium';
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
await send('Page.navigate', { url });
// The module does top-level await on the wasm init; poll rather than guess.
for (let i = 0; i < 100 && !(await evaluate('!!window.pixelgen').catch(() => false)); i++) {
  await new Promise((r) => setTimeout(r, 100));
}

const out = await evaluate(`(async () => {
  // A cool upper half, a warm lower half, a small bright warm lamp: the shape
  // the starter analysis is looking for.
  const c = document.createElement('canvas');
  c.width = 400; c.height = 300;
  const cx = c.getContext('2d');
  cx.fillStyle = '#466ebe'; cx.fillRect(0, 0, 400, 150);
  cx.fillStyle = '#785a3c'; cx.fillRect(0, 150, 400, 150);
  cx.fillStyle = '#fad278'; cx.beginPath(); cx.arc(120, 240, 14, 0, 7); cx.fill();
  const blob = await new Promise((r) => c.toBlob(r));

  await window.pixelgen.open(new File([blob], 'smoke.png'));
  document.getElementById('starter').click();

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
    frames: s.frames,
    fps: s.fps,
    colours: s.palette.length,
    layers: s.layers,
    counter: document.getElementById('counter').textContent,
    snippet: document.getElementById('snippet').value,
    error: document.getElementById('error').hidden ? null : document.getElementById('error').textContent,
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

console.log(out);
if (errors.length) console.log('console errors:', errors);

ws.close();
proc.kill();
server.close();

const bad = [];
if (!out.painted) bad.push('canvas is a flat colour');
if (out.error) bad.push('scene error: ' + out.error);
if (!out.layers.length) bad.push('no layers');
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
