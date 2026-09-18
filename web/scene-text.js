// Edits to the scene as text, for the controls that sit over the YAML.
//
// The renderer can re-serialize a scene, but that throws away every comment -
// and the comments in a starter scene are most of what makes it editable. So a
// control changes the one line it owns and leaves the rest of the file exactly
// as it was typed. This understands only as much YAML as that needs: block
// mappings and sequences, flow mappings on a single line, and comments.

const indent = (line) => line.match(/^ */)[0].length;
const blank = (line) => /^\s*(#.*)?$/.test(line);

// A value as YAML writes it. Numbers only, which is all the controls produce.
const scalar = (v) => String(v);

// The lines of a top-level key's block, as [start, end) - start is the key's
// own line - or null when the key is absent.
function block(lines, key) {
  const head = new RegExp(`^${key}\\s*:`);
  const start = lines.findIndex((l) => head.test(l));
  if (start === -1) return null;
  let end = start + 1;
  // A block ends at the next top-level key. A sequence may sit at column 0
  // under its key, so a dash there is still inside it.
  while (end < lines.length && (blank(lines[end]) || indent(lines[end]) > 0 || lines[end].startsWith('-'))) end++;
  // Trailing blank lines and comments belong to whatever follows.
  while (end > start + 1 && blank(lines[end - 1])) end--;
  return [start, end];
}

// Replaces `key: value` on one line, keeping any trailing comment and its
// alignment. A flow mapping - `{ seconds: 6, fps: 20 }` - is edited in place.
function replaceOn(line, key, value) {
  const re = new RegExp(`(^|[\\s{,])(${key}\\s*:[ \\t]*)([^,}#]*?)(\\s*(?:[,}#]|$))`);
  if (!re.test(line)) return null;
  return line.replace(re, (_, pre, k, _old, post) => `${pre}${k.endsWith(' ') ? k : `${k} `}${scalar(value)}${post}`);
}

/**
 * Sets a scalar at `path` - `['width']` or `['loop', 'fps']` - adding the key,
 * and its parent block, if the scene does not have them yet.
 */
export function setScalar(text, path, value) {
  const lines = text.split('\n');
  const [top, child] = path;

  if (!child) {
    const at = block(lines, top);
    if (at) {
      lines[at[0]] = replaceOn(lines[at[0]], top, value);
    } else {
      lines.splice(firstKey(lines), 0, `${top}: ${scalar(value)}`);
    }
    return lines.join('\n');
  }

  const at = block(lines, top);
  if (!at) {
    // Appended after the last content line, so it does not land inside a
    // trailing commented-out example.
    let end = lines.length;
    while (end > 0 && !lines[end - 1].trim()) end--;
    lines.splice(end, 0, '', `${top}:`, `  ${child}: ${scalar(value)}`);
    return lines.join('\n');
  }

  const [start, end] = at;
  // `loop: { seconds: 6, fps: 20 }`
  if (/:\s*\{/.test(lines[start])) {
    const edited = replaceOn(lines[start], child, value);
    if (edited === null) {
      lines[start] = lines[start].replace(/\s*\}/, `, ${child}: ${scalar(value)} }`);
    } else {
      lines[start] = edited;
    }
    return lines.join('\n');
  }

  // The first content line fixes the block's indent; anything deeper belongs
  // to a nested mapping and is not ours to touch.
  const body = [];
  for (let i = start + 1; i < end; i++) if (!blank(lines[i])) body.push(i);
  const depth = body.length ? indent(lines[body[0]]) : 2;
  const re = new RegExp(`^ {${depth}}${child}\\s*:`);
  const hit = body.find((i) => re.test(lines[i]));
  if (hit !== undefined) {
    lines[hit] = replaceOn(lines[hit], child, value);
  } else {
    lines.splice(body.length ? body[body.length - 1] + 1 : start + 1, 0, `${' '.repeat(depth)}${child}: ${scalar(value)}`);
  }
  return lines.join('\n');
}

// Where a new top-level key goes: before the first one, so that it lands under
// the file's opening comment rather than above it.
function firstKey(lines) {
  const i = lines.findIndex((l) => !blank(l) && indent(l) === 0);
  return i === -1 ? lines.length : i;
}

/**
 * Sets or clears `disable: true` on the `index`th entry under `layers:`,
 * counting every entry, disabled or not.
 */
export function setLayerDisabled(text, index, disabled) {
  const lines = text.split('\n');
  const at = block(lines, 'layers');
  if (!at) throw new Error('the scene has no layers: block');
  const [start, end] = at;

  const items = [];
  let depth = null;
  for (let i = start + 1; i < end; i++) {
    if (blank(lines[i])) continue;
    const d = indent(lines[i]);
    if (depth === null) depth = d;
    if (d === depth && lines[i].slice(d).startsWith('-')) items.push(i);
  }
  if (index >= items.length) throw new Error(`no layer ${index} in the scene text`);

  const first = items[index];
  let last = (items[index + 1] ?? end) - 1;
  while (last > first && blank(lines[last])) last--;

  const dash = lines[first].match(/^ *- */)[0];
  const rest = lines[first].slice(dash.length);
  if (rest.startsWith('{')) {
    // `- { type: vignette, params: { ... } }`, on one line.
    if (last !== first) throw new Error('cannot toggle a flow-style layer that spans lines - edit its disable: by hand');
    const edited = replaceOn(lines[first], 'disable', disabled);
    if (edited !== null) lines[first] = disabled ? edited : edited.replace(/,\s*disable\s*:\s*false/, '');
    else if (disabled) lines[first] = lines[first].replace(/\s*\}(\s*(#.*)?)$/, ', disable: true }$1');
    return lines.join('\n');
  }

  // Keys of the entry sit where the first one does, just past the dash - on
  // the dash line itself, or on a line of their own at that indent.
  const keys = dash.length;
  const own = new RegExp(`^ {${keys}}disable\\s*:`);
  let hit = /^disable\s*:/.test(rest) ? first : -1;
  for (let i = first + 1; i <= last && hit === -1; i++) if (own.test(lines[i])) hit = i;

  if (disabled) {
    if (hit !== -1) lines[hit] = replaceOn(lines[hit], 'disable', true);
    else lines.splice(last + 1, 0, `${' '.repeat(keys)}disable: true`);
  } else if (hit === first) {
    // On the dash line itself, dropping the key would take the dash with it.
    lines[hit] = replaceOn(lines[hit], 'disable', false);
  } else if (hit !== -1) {
    lines.splice(hit, 1);
  }
  return lines.join('\n');
}
