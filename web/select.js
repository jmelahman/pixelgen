// The selection editor's arithmetic, kept apart from the page so none of it
// needs a canvas: outlines to path data and back, coverage combined the way
// the core combines `steps`, and the outline the marching ants walk.
//
// Every point here is normalized to the frame, as the scene format has it.

// Four decimals is a tenth of a cell at a width of a thousand, which no grid
// here reaches; more only makes the scene file longer.
export const num = (v) => {
  const s = (Math.round(v * 1e4) / 1e4).toFixed(4).replace(/0+$/, '').replace(/\.$/, '');
  return s.replace(/^(-?)0\./, '$1.') || '0';
};

// Numbers separated by a space, or by nothing where a minus sign already
// separates them.
function join(nums) {
  return nums.map(num).reduce((out, s) => (out && !s.startsWith('-') ? `${out} ${s}` : out + s), '');
}

/** Path data for a polyline, closed with `Z` when asked. */
export function polyD(pts, closed = true) {
  if (!pts.length) return '';
  let d = `M${join([pts[0].x, pts[0].y])}`;
  for (const p of pts.slice(1)) d += `L${join([p.x, p.y])}`;
  return closed ? `${d}Z` : d;
}

/**
 * Ramer-Douglas-Peucker: drops points within `eps` of the line through their
 * neighbors. `sx`/`sy` scale to cells, so `eps` is in cells whatever the
 * frame's aspect.
 */
export function simplify(pts, eps, sx = 1, sy = 1) {
  if (pts.length < 3) return pts.slice();
  const keep = new Uint8Array(pts.length);
  keep[0] = keep[pts.length - 1] = 1;
  const stack = [[0, pts.length - 1]];
  while (stack.length) {
    const [a, b] = stack.pop();
    const ax = pts[a].x * sx, ay = pts[a].y * sy;
    const dx = pts[b].x * sx - ax, dy = pts[b].y * sy - ay;
    const len = Math.hypot(dx, dy);
    let far = -1, best = eps;
    for (let i = a + 1; i < b; i++) {
      const px = pts[i].x * sx - ax, py = pts[i].y * sy - ay;
      const d = len < 1e-9 ? Math.hypot(px, py) : Math.abs(px * dy - py * dx) / len;
      if (d > best) { best = d; far = i; }
    }
    if (far >= 0) {
      keep[far] = 1;
      stack.push([a, far], [far, b]);
    }
  }
  return pts.filter((_, i) => keep[i]);
}

/**
 * The selection with one more operand, as the core's `steps` would have it.
 * `mode` is new, add, sub or and. A selection whose own modifiers would
 * otherwise reach the new step is wrapped first, so they keep applying to
 * what they were set on.
 */
export function withStep(sel, op, operand) {
  if (op === 'new' || !sel) {
    // Taking away from, or intersecting with, nothing leaves nothing.
    return op === 'new' || op === 'add' ? { steps: [{ add: operand }] } : sel;
  }
  // A spec loaded from a layer may be a bare selector rather than a list.
  const flat = sel.steps && !hasModifiers(sel);
  const base = flat ? { ...sel, steps: [...sel.steps] } : { steps: [{ add: sel }] };
  base.steps.push({ [op]: operand });
  return base;
}

export function hasModifiers(s) {
  return !!(s.grow || s.smooth || s.invert || s.feather || (s.gain && s.gain !== 1));
}

/** The core's step arithmetic, on 8-bit coverage. `base` null is empty. */
export function combine(base, op, cov) {
  if (!base || op === 'new') {
    return op === 'new' || op === 'add' ? cov : base ?? new Uint8Array(cov.length);
  }
  const out = new Uint8Array(cov.length);
  for (let i = 0; i < cov.length; i++) {
    const a = base[i], b = cov[i];
    out[i] = op === 'add' ? Math.max(a, b) : op === 'sub' ? Math.min(a, 255 - b) : Math.min(a, b);
  }
  return out;
}

/**
 * The boundary of what is at least half covered, as runs along cell edges:
 * `[x0, y0, x1, y1]` in cells, horizontal and vertical, each as long as it
 * can be. The ants walk these; joining them into loops would only matter for
 * a dash that has to run unbroken round a corner, which at cell scale no one
 * can see.
 */
export function outline(cov, w, h) {
  const on = (x, y) => x >= 0 && y >= 0 && x < w && y < h && cov[y * w + x] >= 128;
  const runs = [];
  // Horizontal edges lie on corner row y, between cell rows y-1 and y.
  for (let y = 0; y <= h; y++) {
    let start = -1;
    for (let x = 0; x <= w; x++) {
      const edge = x < w && on(x, y - 1) !== on(x, y);
      if (edge && start < 0) start = x;
      if (!edge && start >= 0) {
        runs.push([start, y, x, y]);
        start = -1;
      }
    }
  }
  for (let x = 0; x <= w; x++) {
    let start = -1;
    for (let y = 0; y <= h; y++) {
      const edge = y < h && on(x - 1, y) !== on(x, y);
      if (edge && start < 0) start = y;
      if (!edge && start >= 0) {
        runs.push([x, start, x, y]);
        start = -1;
      }
    }
  }
  return runs;
}

// ---------------------------------------------------------------- the pen

/**
 * Pen anchors as path data. Each anchor is `{x, y, ix, iy, ox, oy}`: its
 * point and the offsets to its incoming and outgoing handles. A segment with
 * no handles at either end is written as a line.
 */
export function penD(anchors, closed) {
  if (!anchors.length) return '';
  const a0 = anchors[0];
  let d = `M${join([a0.x, a0.y])}`;
  const seg = (a, b) => {
    if (!a.ox && !a.oy && !b.ix && !b.iy) return `L${join([b.x, b.y])}`;
    return `C${join([a.x + a.ox, a.y + a.oy, b.x + b.ix, b.y + b.iy, b.x, b.y])}`;
  };
  for (let i = 1; i < anchors.length; i++) d += seg(anchors[i - 1], anchors[i]);
  if (closed && anchors.length > 2) d += `${seg(anchors.at(-1), a0)}Z`;
  return d;
}

/**
 * The pen's own output read back into anchors, or null for anything it could
 * not have written: more than one subpath, or a command other than M, L, C
 * and a closing Z. That is enough to re-open a path drawn here; anything
 * hand-written stays as it is.
 */
export function parsePen(d) {
  const tokens = d.match(/[MLCZmlcz]|-?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?/gi);
  if (!tokens || !/^M$/.test(tokens[0])) return null;
  let i = 1;
  const nums = (n) => {
    const out = [];
    for (let k = 0; k < n; k++) {
      const v = Number(tokens[i++]);
      if (!Number.isFinite(v)) return null;
      out.push(v);
    }
    return out;
  };
  const start = nums(2);
  if (!start) return null;
  const anchors = [{ x: start[0], y: start[1], ix: 0, iy: 0, ox: 0, oy: 0 }];
  let closed = false;
  let cmd = 'L';
  while (i < tokens.length) {
    if (/^[A-Za-z]$/.test(tokens[i])) cmd = tokens[i++];
    if (cmd === 'Z') {
      if (i < tokens.length) return null;
      closed = true;
      break;
    }
    const last = anchors.at(-1);
    if (cmd === 'L') {
      const p = nums(2);
      if (!p) return null;
      anchors.push({ x: p[0], y: p[1], ix: 0, iy: 0, ox: 0, oy: 0 });
    } else if (cmd === 'C') {
      const c = nums(6);
      if (!c) return null;
      last.ox = c[0] - last.x;
      last.oy = c[1] - last.y;
      anchors.push({ x: c[4], y: c[5], ix: c[2] - c[4], iy: c[3] - c[5], ox: 0, oy: 0 });
    } else {
      return null;
    }
  }
  // A closed path ends with a segment back onto its first anchor; that
  // segment's handles belong to the last and first anchors, not a new one.
  if (closed && anchors.length > 2) {
    const end = anchors.at(-1), a0 = anchors[0];
    if (Math.abs(end.x - a0.x) < 1e-6 && Math.abs(end.y - a0.y) < 1e-6) {
      anchors.pop();
      a0.ix = end.ix;
      a0.iy = end.iy;
    }
  }
  return { anchors, closed };
}

/** Flattened pen curve for drawing, in the same normalized space. */
export function penPoints(anchors, closed, steps = 16) {
  const out = [];
  const seg = (a, b) => {
    const p0 = [a.x, a.y], p1 = [a.x + a.ox, a.y + a.oy], p2 = [b.x + b.ix, b.y + b.iy], p3 = [b.x, b.y];
    for (let k = 1; k <= steps; k++) {
      const t = k / steps, u = 1 - t;
      const w = [u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t];
      out.push({
        x: w[0] * p0[0] + w[1] * p1[0] + w[2] * p2[0] + w[3] * p3[0],
        y: w[0] * p0[1] + w[1] * p1[1] + w[2] * p2[1] + w[3] * p3[1],
      });
    }
  };
  if (!anchors.length) return out;
  out.push({ x: anchors[0].x, y: anchors[0].y });
  for (let i = 1; i < anchors.length; i++) seg(anchors[i - 1], anchors[i]);
  if (closed && anchors.length > 2) seg(anchors.at(-1), anchors[0]);
  return out;
}
