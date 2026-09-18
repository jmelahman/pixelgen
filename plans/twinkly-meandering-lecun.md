# Photoshop-style mask selection

## Context

The Mask tab has three click-to-place shapes (polygon, rect, ellipse). "Apply" replaces
the selected layer's mask with the shape. The Layers-panel plan (`plans/declarative-dreaming-yeti.md`)
held back real selection tools and left "Apply to layer" as the place they would connect.
This plan adds that: a Photoshop-like selection workflow with magic wand, color range,
freehand/polygonal/magnetic lasso, a Bezier pen, a brush that paints and erases, and
refine modifiers.

Decisions you made:
- **Storage stays declarative.** Every tool writes a resolution-independent YAML
  selector in 0..1 coordinates:
  - The wand stores a seed point and a tolerance, and is re-run against the base.
  - The lasso tools and the pen store a path.
  - The brush stores stroke polylines with a radius.
  - There are no raster blobs.
- **Photoshop interaction model.**
  - A live selection shown with marching ants.
  - Shift adds, Alt subtracts, Shift+Alt intersects.
  - Select All, Deselect and Invert.
  - The selection is then **applied** to the selected layer's mask (replace, add,
    subtract or intersect), or **saved as a named region**.
- **All tools are in scope.** The work ships as a series of PRs that each stand alone.

Every mask is still evaluated at grid resolution against the quantized base
(`mask::Builder`, `crates/pixelgen-core/src/mask.rs`). The `#overlay` canvas is also
at grid resolution, so vector UI such as ants, handles and the brush cursor needs a new
screen-resolution canvas.

## Schema (core, `mask.rs`)

```yaml
mask:
  steps:                  # acc starts empty; first step must be `add`
    - add: { wand: { x: 0.41, y: 0.30, hex: "#3a5f8c", tolerance: 0.1, contiguous: true } }
    - add: { path: "M.1 .2 L.3 .22 L.28 .41 Z" }                  # lasso / magnetic
    - add: { path: "M.5 .5 C.55 .45 .62 .47 .65 .5 S.6 .6 .5 .5 Z" } # pen
    - sub: { stroke: { d: "M.2 .3 L.25 .31 L.3 .35", radius: 0.01 } } # brush erase
    - and: { color: { hex: "#223344", tolerance: 0.15 } }          # color range
  grow: -1     # cells; negative shrinks
  smooth: 1    # cells; rounds corners, drops specks
  feather: 1
```

- **`steps`.**
  - Each step is a struct with three `Option<Spec>` fields (`add`, `sub`, `and`), and
    exactly one of them must be set. A Rust enum is not used because serde_yaml 0.9
    would write `!add` tags.
  - `add` takes the maximum, `sub` takes `min(acc, 1-x)`, and `and` takes the minimum.
  - This keeps a Photoshop op sequence flat, where nested `any`/`all` would grow one
    level deeper on each change of mode. `any`/`all` stay for hand-written YAML.
- **`path`.** An SVG-style `d` string using `M L C S Q Z`, absolute and relative, filled
  with the even-odd rule.
  - One line of YAML per path. serde_yaml cannot write flow style, so a 300-point
    `polygon` would take 600 lines.
  - Curves are flattened adaptively, to 0.25 cell at build time.
  - The string is parsed in `validate`, and a bad one fails with a new `Error::BadPath`.
- **`stroke`.** `{ d, radius, hardness }`.
  - `radius` is a fraction of the width, so a stroke scales with the grid.
  - Coverage comes from the distance to the polyline segments, limited to the bbox.
  - Erasing is a `sub` step. JS merges consecutive strokes with the same op into one
    multi-subpath `d`.
- **`wand`.** `{ x, y, hex, tolerance, contiguous }`.
  - A 4-connected flood from the seed cell, using the same color distance as
    `from_color`.
  - `hex` records the sampled color. The flood compares against it and starts from the
    nearest matching cell within about 2 cells of the seed, so the wand still works
    after `width`, `palette` or `prepare` changes.
  - When `palette.dither > 0`, distances are taken on a 3×3 box-averaged copy of the
    base.
  - The seed cell is always included, so a wand is never empty.
- **Modifier order.** The shape is built first, then `grow`, then `smooth`, then the
  existing `invert`, `feather` and `gain`. The new modifiers come first so existing
  scenes keep their meaning.
  - `grow` is a disk dilation (or erosion when negative). It blends between the floor
    and the ceiling of the radius, and switches to a distance transform above 8 cells.
  - `smooth` is a box blur followed by `smooth_step(0.4, 0.6)`.
- **Validation.**
  - A new `Error::MultipleSelectors` fires when one spec sets two selectors. Today
    `shape()` silently drops one of them.
  - `conflict()` lists the new selectors.
  - `walk()` recurses into `steps`, so a ref cycle inside a step is reported.
- **Anti-aliased scanline fill.** It uses 4 sub-rows and exact span coverage. `polygon`
  and `path` share it, and it replaces the O(w·h·n) crossing test in `from_polygon`,
  which is too slow for 500-point lassos. Polygon edges become soft, which is intended.
- **`Registry`: `HashMap` → `BTreeMap`.** `regions:` then serializes in a stable order.
  Otherwise "Save as region" would shuffle the Scene text and clutter undo.

## Scene and wasm

- **`Scene::combine_layer_mask(i, sel: Option<Spec>, mode)`** in `scene.rs`, with
  `mode` one of replace, add, sub or and.
  - Select All with replace writes `mask: null`.
  - When the layer has no mask, `add` gives `null`, `sub` gives the inverted selection,
    and `and` gives the selection.
  - When the existing mask is a `steps` spec with no modifiers, the new step is appended
    to it. Otherwise the existing mask is wrapped as the first step.
- **`Scene::set_region(name, Option<Spec>)`.**
- **New `Edit` ops in `crates/pixelgen-wasm/src/lib.rs`:**
  - `MaskCombine { i, mask, mode }`
  - `Region { name, mask }`

  Both go through the existing clone → validate → commit path, so an `EmptyMask`
  result is refused and the scene is left unchanged.
- **New `Session` methods:**
  - `sample(x, y) -> hex`, which feeds the wand's `hex`.
  - `spec_json(yaml)`, which lets JS load a layer mask back into a selection. JSON is
    valid YAML, so `preview_mask` already accepts JSON selections.
  - `livewire(x, y) -> LiveWire` and `LiveWire::path_to(x, y) -> Float32Array`, for the
    magnetic lasso:
    - Nodes are cell corners, and edge cost is `1/(ε+ΔE)` of the two cells an edge
      separates.
    - Dijkstra runs once per anchor inside an ~80-cell window.
    - Each pointermove only walks back through the predecessor map.

## Web (`web/app.js`, `web/index.html`, `web/style.css`)

- **`#ui` canvas.**
  - Sized to the screen at `devicePixelRatio` and resized in `fit()`.
  - Sits over `#overlay` with `touch-action: none`.
  - Uses pointer events with pointer capture. This replaces the current `click`
    handler on `#overlay`.
- **Selection state.**
  - `state.sel` is a list of steps, or `null` for none. It is never `{}`, because an
    empty spec builds the full frame.
  - `state.selHistory` holds undo and redo.
  - While the Mask tab has a live selection, Ctrl+Z and Ctrl+Shift+Z step through the
    selection. Otherwise they fall through to the scene `undo()`.
- **Tool palette.** Replaces `.shapes`:
  - Marquee: rect or ellipse (M)
  - Lasso: freehand, polygonal or magnetic (L)
  - Wand: wand or color range (W)
  - Pen (P)
  - Brush (B)
- **Options bar** for the active tool:
  - tolerance and contiguous
  - brush radius and hardness
  - a sticky mode toggle (new/add/sub/and), for users whose window manager takes
    Alt-drag
- **Rendering.**
  - `#overlay` shows a quick-mask tint of coverage, reusing `paintCoverage`.
  - `#ui` draws the marching ants.
    - Coverage is thresholded at 0.5 and the cell-boundary edges are traced into
      loops, then collinear runs are merged.
    - The ants are drawn as a black/white dash whose `lineDashOffset` advances every
      120 ms. Only `#ui` is redrawn.
  - Pen anchors and handles, and the brush cursor ring, are also drawn on `#ui`.
- **Preview cost.**
  - The coverage of the committed selection is cached.
  - Only the operand still being drawn goes through `preview_mask`, throttled to once
    per animation frame.
  - The two are combined in JS with max/min.
- **Select menu.**
  - All (Ctrl+A)
  - Deselect (Ctrl+D)
  - Inverse (Shift+F7 or Ctrl+Alt+I). Ctrl+Shift+I cannot be used because it opens
    DevTools. Inverse wraps the selection unless it has no modifiers, because
    gain(1−B) ≠ 1−gain(B).
  - Load layer mask
  - Esc cancels the shape in progress, and Enter closes a polygon or path.
  - Space keeps playback, so there is no pan.
- **Apply.**
  - A split button: Apply to ‹layer›, with a menu of Replace / Add / Subtract /
    Intersect that sends `mask_combine`.
  - Apply is disabled, with a reason, when the preview coverage is all zero.
  - **Save as region…** asks for a name.
  - **Copy** stays: it copies the selection's YAML.
- **Refine section.** Grow/shrink, smooth and feather sliders, which write the
  top-level modifiers of the selection.
- **`maskSummary`** learns the new keys, for example `steps(3) · grow -1`.
- **Help.** Rewrite the Mask tab `.help` text to cover the tools and modifier keys.

## Phases (one PR each, each shippable)

1. **Core groundwork:**
   - the anti-aliased scanline fill for `polygon`
   - `Registry` → `BTreeMap`
   - `MultipleSelectors`
2. **`steps`, `grow` and `smooth`.** Includes the `walk`/`conflict` updates, plus
   `combine_layer_mask`, `set_region`, and the `MaskCombine` and `Region` edits.
3. **`path`, `stroke` and `wand` selectors:**
   - the path parser, in a new `mask/path.rs` or a private module
   - the wasm methods `sample` and `spec_json`
4. **Web selection core:**
   - the `#ui` canvas, pointer events and the selection stack
   - marching ants and the Shift/Alt modes
   - the Select menu
   - Marquee and the freehand/polygonal lasso
   - the Apply split button and Save as region
   - the old click-to-add path is removed
5. **Wand and color range** tools.
6. **Magnetic lasso:** the `LiveWire` handle and the tool.
7. **Pen:**
   - Editable anchors and handles.
   - Clicking a saved `path` step re-opens it: `d` is parsed back into anchors in JS,
     which only needs to handle the pen's own `M/C/Z` output.
8. **Brush and refine.** Paint and erase using coalesced pointer events, a cursor ring,
   and the refine sliders.
9. **Docs.** Update the README "Regions" section and the selector list.

## Tests

- **`crates/pixelgen-core/tests/mask.rs`:**
  - partial coverage at a polygon edge
  - a 1000-point polygon builds quickly
  - two selectors in one spec are rejected
  - steps apply add/sub/and in order
  - a first step that is not `add` is rejected
  - a ref cycle through a step is reported
  - grow(+1) then grow(−1) comes back close to a square
  - smooth removes a one-cell speck
  - wand is bounded by a color edge
  - contiguous and global wand give different results
  - a wand seed plus `hex` still hits at 2× width
  - a wand is never empty
  - the path parser handles M/L/C/Q/S/Z, both absolute and relative
  - `BadPath` is raised at validate
  - stroke coverage scales with width
  - the live-wire follows a two-color boundary
- **`crates/pixelgen-core/tests/edit.rs`:**
  - `combine_layer_mask` in every mode, with and without an existing mask, and when
    appending to an existing `steps`
  - `set_region`
  - YAML round-trips
- **`tools/web-smoke.mjs`:**
  - the `#ui`, `#overlay` and `#view` boxes agree
  - a drag, then a Shift-drag, then an Alt-drag give steps `add`, `add`, `sub`
  - Ctrl+Z pops a selection step before it touches the scene
  - Apply→Add changes the layer mask
  - a wand click on a synthetic region gives coverage only inside it
  - three pen anchors, one of them dragged, give a `C` command
  - an erase stroke inside a rect lowers coverage along it

## Verification

1. After each phase, run `prek run --all-files`: fmt, clippy, cargo test, build-web and
   web-smoke.
2. Run `prek run build-web --all-files`, then serve `web/` on port 8000 in the
   background.
3. Walk through it with the Playwright MCP:
   1. Open a synthetic photo through `window.pixelgen.open`.
   2. On the Mask tab, use each tool and check that the ants appear. Shift-add with
      the wand, then Alt-subtract with the lasso.
   3. Apply → Intersect onto a layer, and confirm with Show on the Layers tab.
   4. Save as a region and check that `regions:` appears in the Scene tab.
   5. Change the grid width and confirm the wand and path masks still select the same
      thing.
   6. Take a screenshot after each step, and check `browser_console_messages` at
      `error`, which must be empty.
