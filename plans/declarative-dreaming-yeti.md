# Layers panel as the primary editor

## Context

The page is built around a YAML textarea (Scene tab). The Layers tab only lists the enabled
layers and toggles a mask overlay. You want Layers to be the main way to edit a scene, and
you want it to feel like a Photoshop layers and properties panel: add and remove layers,
toggle visibility, reorder, and edit parameters through controls. Help text should sit behind
a button. A proper selection and masking system (magic wand, lasso) is **deferred**. This
change sets up the per-layer mask entry point that it will plug into later.

Decisions you made:
- **Re-serialize.** Every structural edit goes through the parsed `Scene` in Rust and writes
  the YAML back out. The starter comments are lost after the first panel edit. Scene becomes
  the raw/advanced view.
- **Generated controls.** Rust exposes each effect's default parameters, and the panel builds
  inputs from them.
- **Keep the tabs, with Layers first.** The tab order becomes Layers | Mask | Scene. Mask
  drawing turns into a per-layer "Edit mask" action.

## Core (`crates/pixelgen-core`)

1. **Effect param defaults.** `src/effect/{weather,light,motion,post}.rs`
   - Add `Serialize` to every `*Cfg` struct. They already derive `Deserialize` and
     implement `Default`, as `decode` requires.
   - Keep the existing `#[serde(...)]` attributes so the field names serialize to the same
     keys the YAML uses.
   - In `effect/mod.rs`, add `pub fn defaults(name) -> Result<Value, Error>`. It uses a
     match that mirrors `build()` and returns `serde_yaml::to_value(Cfg::default())`.
   - Add a test that every entry in `CATALOG` has defaults, and that
     `build(name, &defaults(name))` succeeds. That test stops the two match arms from
     drifting apart.

2. **Scene layer edits.** `src/scene.rs`, added to `impl Scene`. These are pure mutations,
   and the host validates and prepares afterwards.
   - `add_layer(type, at: Option<usize>)`: inserts `Layer { type, ..Default }` with no
     params and no mask, which means the full frame.
   - `remove_layer(i)`, `move_layer(from, to)`, `set_layer_disable(i, bool)`,
     `rename_layer(i, name)`.
   - `set_layer_param(i, key, Value)`: turns a null `params` into a mapping first.
     Writing a value equal to the default removes the key, so the YAML only records what
     actually changed.
   - `set_layer_mask(i, Option<Spec>)`.
   - Unit tests cover index bounds and confirm that `to_yaml` → `parse` round-trips.

3. **Drawn index.** `render.rs` skips disabled layers, so `Prepared::layer_mask` is indexed
   by drawn position. Keep it that way. The wasm side reports each scene layer's drawn
   index (or null), so the panel can list disabled layers too and grey them out.

## Wasm (`crates/pixelgen-wasm/src/lib.rs`)

- `effects()` also returns each effect's `defaults`. Add `serde_json` to this crate only
  and use it to build that JSON instead of the hand-rolled `quote()`. Remove `quote()`.
- `Session::scene_layers() -> String` returns JSON for **all** scene layers:
  `{ name, label, type, disable, params (defaults merged with set values), set: [keys], mask: yaml|null, drawn: idx|null }`.
- Add one edit entry point: `Session::edit(op_json) -> Result<String, JsError>`. It works on
  a clone of `self.scene`:
  1. Apply the op.
  2. `validate`, then `render::prepare`.
  3. On success, commit and return `to_yaml()`.
  4. On failure, return the error and leave the session unchanged. For example, an
     `EmptyMask` or bad-param error must not wipe out the current scene.

  The ops are `add | remove | move | disable | rename | param | mask`. A single
  dispatcher keeps the wasm surface small.

## Web (`web/index.html`, `web/app.js`, `web/style.css`)

**Tabs.** Layers is the default tab (`aria-pressed="true"`). The order is Layers, Mask,
Scene.

**Layers tab layout**, from top to bottom:
- **Toolbar:** `+` (add), `−` (remove selected), `?` (help), plus up/down move buttons.
  - `+` opens a `<details class="menu">` popover. It reuses the existing `.menu` styles and
    lists the effect catalog with descriptions, so the separate "Effects" list goes away.
- **Layer stack**, shown Photoshop-style with the top-most layer at the top of the list.
  The list is reversed relative to render order and the index mapping stays in JS.
  - Each row: eye toggle (`disable`), a name you can rename by double-clicking, and a type
    chip.
  - Clicking a row selects it. The selected row is highlighted.
  - Disabled rows are dimmed.
  - Drag-to-reorder uses native HTML5 drag events. The up/down buttons are the keyboard
    fallback.
- **Properties** for the selected layer:
  - Controls are generated from the merged params. Numbers become `<input type=number>`
    with `step` inferred (integers step by 1, floats by 0.05). Booleans become checkboxes.
    Hex strings, or null fields whose name suggests a color, become `<input type=color>`
    plus a clear button.
  - Params you've set are marked, and each has a reset control.
  - A **Mask** row shows the mask as a one-line summary (for example "full frame",
    "ref: sky", or "polygon · 5 pts") with **Show** (the overlay), **Edit** and **Clear**
    buttons.
    - **Edit** switches to the Mask tab, targeting that layer.

**Mask tab.** It keeps its current draw tools. When a target layer is set, a new "Apply to
‹layer›" button next to Copy sends `edit({op:'mask', ...})`, and the Copy/snippet flow
stays for pasting into `regions:`. This is the seam where wand and lasso tools will go
later.

**Flow for every panel edit:** `yaml = session.edit(op)` →
`el('yaml').value = yaml` → the same redraw path as `apply()` (factor the post-`set_scene`
half of `apply()` into `refresh()`) → re-render the panel from `scene_layers()`.
- Errors show in the existing `#error` / `say(..., true)`, and the scene stays as it was.
- Parameter inputs fire on `change` (not `input`), so a spinner doesn't re-prepare on every
  tick.

**Undo.** Keep a small stack of YAML strings in `state`, pushed before each panel edit.
Ctrl/Cmd+Z restores the previous string through `apply()`. Plain text edits in the Scene
textarea keep the textarea's own native undo.

**Help behind a button.** Move the explanatory `.quiet` paragraphs from the Layers and Mask
tabs, and the "When not to draw one" `<details>`, into a `.help` block on each tab.
- The block is hidden by default.
- The `?` toggle has `aria-expanded` and remembers its state in `localStorage` inside a
  try/catch.

**Other cleanup:**
- The "Mask" checkbox in the transport and `state.maskLayer` stay, but are keyed to the
  selected layer's drawn index.
- The stat for `layers` counts drawn/total.

## Tests and docs

- `tools/web-smoke.mjs` changes:
  - Select a layer via `#layers li`, which is still valid.
  - Also click `+` → add a layer (e.g. `vignette`), then assert that `session.layers`
    grew and that the YAML contains it.
  - Toggle a layer's eye and assert the drawn count dropped.
  - Change one param.
  - Keep the draw-tab section. The `data-tab="draw"` selector stays valid.
- README "In the browser" section: say briefly that the layer panel is the main editor and
  YAML is the raw view.

## Deferred (not in this change)

Selection-based masks: magic wand (flood fill on palette index or color tolerance from the
clicked cell), freehand lasso and magnetic lasso, and add/subtract/intersect modes. These
map onto the existing `any` / `all` / `invert` combinators and a future `bitmap` or `cells`
selector in `mask::Spec`. The Mask tab's "Apply to layer" path built here is where they
plug in.

## Verification

1. `prek run --all-files`. This runs fmt, clippy, cargo test (including the new defaults
   and scene-edit tests), build-web and web-smoke.
2. `prek run build-web --all-files`, then serve `web/` on port 8000 in the background.
3. Walk through it with the Playwright MCP:
   1. Open the page and wait for `window.pixelgen`.
   2. Open a synthetic image through `window.pixelgen.open`.
   3. Confirm the Layers tab is active.
   4. Add a layer, toggle its eye, drag to reorder, change a numeric param, reset it,
      remove the layer, then press Ctrl+Z.
   5. After each step, check the YAML textarea and take a screenshot.
   6. Click Edit mask, draw a rectangle, Apply, and confirm the layer's mask summary
      updates and the overlay shows coverage.
   7. Toggle `?` help.
   8. Check `browser_console_messages` at `error` and confirm it is empty.
