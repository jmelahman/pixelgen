# pixelgen

Turn a photograph into an animated pixel-art background that loops seamlessly.

The input is an ordinary image. The output is a still PNG, or an MP4/WebM/GIF
loop in the style of the lofi pixel-art wallpapers that pair a hand-painted
scene with a few small moving layers: rain in the window, fog through the trees,
a lamp that flickers, plants that sway.

```
pixelgen init    photo.jpg          # write a starter scene file
pixelgen animate --scene photo.scene.yaml
```

## How it works

The pipeline splits into a static half and an animated half, and the split is
what keeps the result looking like pixel art rather than like a filtered video.

**Static.** The source is median-filtered, box-downscaled to a pixel grid,
pushed in saturation and contrast, and quantized to a palette derived from the
image by k-means. The base is dithered **once**. Everything here happens a
single time.

**Animated.** Each frame clones that base, runs the scene's effect layers over
it, and snaps the result back onto the same palette. The palette never changes
and frames are never dithered, so flat areas stay perfectly still and no
dither pattern crawls between frames.

Frames are rendered in parallel, one per core, and encoded at the pixel-grid
resolution with ffmpeg doing the integer nearest-neighbor upscale.

### Seamless looping

Every effect is a pure function of a loop phase `t` in `[0,1)` and must be
periodic in it, so that the last frame hands off to the first with no jump.
This is enforced by construction rather than by cross-fading:

- **Noise** is sampled in 4D, with `t` traced around a circle in the two extra
  dimensions. Because the circle closes, `t=0` and `t=1` are the same point.
- **Drifting** noise additionally uses a lattice that wraps every `period`
  cells, and scrolls by exactly a whole number of periods over one loop.
- **Discrete motion** — rain falling, palettes cycling, regions scrolling — is
  parameterized in whole traversals or rotations per loop, never in cells per
  loop, because only a whole traversal returns to the start.

`loop_closes_for_every_effect` asserts this for every registered effect: frame
`n` (phase 1.0) must be pixel-identical to frame 0, and the effect must also
have actually moved in between.

### Regions

There is no scene understanding here. Effects are confined by masks declared in
the scene file, in normalized `0..1` coordinates so they survive a change of
`width`. Masks can be rectangles, ellipses, polygons, SVG-style paths, brush
strokes, soft bands, a luma range, "pixels near this color", a magic-wand flood
or a color-temperature range. They are combined with `steps` (add, subtract,
intersect, in order), `all` (intersect) and `any` (union), and modified by
`grow`, `smooth`, `invert`, `feather` and `gain`.

### Depth, without a depth map

Rectangles cannot express depth, and most scenes have some: rain belongs outside
the window and behind everything standing in front of it, and lamplight must
stop at the wall. Drawing either with a box puts rain indoors and light on the
rocks.

The `chroma` selector solves this for the common case, because a scene that has
an inside and an outside almost always splits by color temperature — warm lamp,
cool daylight. Selecting the cool half cuts the exact silhouette of whatever
stands in the way:

```yaml
regions:
  outside:
    all:
      - rect: { x: 0.11, y: 0.0, w: 0.75, h: 0.82 }
      - chroma: { min: 0.30 } # positive is cool, negative is warm
  indoors:
    ref: outside
    invert: true
```

`chroma` measures blue minus red normalized by brightness, so a shadowed blue
trunk and a brightly lit patch of the same fog both read as equally cool. The
un-normalized difference between them is a factor of three, and a threshold
picked for the fog would drop every shadow beside it.

Regions declared at the top level are referred to by layers with `ref`, and
carry modifiers, so `indoors` above is just the complement. Naming them matters
more than it looks: the aperture is most of the work in a scene, several layers
need it or its inverse, and restating it per layer means it drifts as you tune.
A region is evaluated once however many layers use it. Unresolvable references
and reference cycles are rejected at load time.

This also gives occlusion for free. `glow` blurs outward but is clamped to its
mask, so intersecting it with `indoors` is what stops the bloom at the window
edge — no depth buffer involved.

`pixelgen init` guesses a few of these from brightness alone — where the horizon
is, whether there is a warm light source — and writes them into a commented
starter file for you to correct.

## Install

```
cargo install --path crates/pixelgen-cli
```

or just `cargo build --release` and use `target/release/pixelgen`.

Requires `ffmpeg` on `PATH` for video output. GIF and PNG output are written
natively and need nothing.

### Layout

| Crate                  | Contents                                                                                                                                                            |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/pixelgen-core` | The whole renderer: masks, palette, effects, scene format, GIF encoding. No file, process or network I/O; rayon is the only dependency beyond `serde` and `gif`, and it is compiled out on wasm |
| `crates/pixelgen-cli`  | The `pixelgen` binary: image decoding, ffmpeg, writing files                                                                                                        |
| `crates/pixelgen-wasm` | `wasm-bindgen` wrapper around the core, for the browser UI                                                                                                          |

The split is load-bearing rather than cosmetic. The core never opens a file —
`palette.file` is reported to the host and folded back in by
`Scene::set_palette_hex` — so the browser runs the identical renderer instead of
a reimplementation that drifts.

## In the browser

The renderer compiles to wasm, and `web/` is a small editor built on it: open a
photograph and it lands in the scene its own analysis suggests, with the loop
playing beside it. The **Layers** tab is where it is edited: add, remove,
reorder and hide layers, and set each one's parameters. **Mask** is a selection
editor in the manner of Photoshop's: marquee, lasso (freehand, polygonal and
magnetic), magic wand and color range, a Bézier pen and a brush, with
<kbd>Shift</kbd> to add, <kbd>Alt</kbd> to subtract and both to intersect, and
grow, smooth and feather to refine the edge. The selection is applied to the
selected layer's mask (replacing, adding to, subtracting from or intersecting
it) or saved as a named region. What is kept is the selector, never the pixels
it caught, so a selection survives a change of `width` or palette. **Scene** is the same thing as raw YAML, for anything the panel does not
cover; an edit in either shows up in the other.

**Save** writes a PNG of the current frame, a GIF of the loop, or a recording
of it. The GIF comes from the same encoder the CLI uses — frames are already
indices into a palette of at most 256 colors, which is exactly what a GIF
stores, so no color is re-decided. Video is the browser's own recorder, which
means MP4 in Chrome and Safari and WebM in Firefox; the menu says which you are
getting. For video at wallpaper size, use the CLI and its ffmpeg.

```
cargo install wasm-bindgen-cli
./build-web.sh
python3 -m http.server -d web 8000
```

Nothing is uploaded and there is no build step beyond the wasm: the page is
three static files and a `<script type="module">`. The same bundle is published
to GitHub Pages by `.github/workflows/pages.yml` on every push to `master`, at
<https://jamison.lahman.dev/pixelgen/>.

The photograph never leaves the browser - it is decoded to a canvas and handed
straight to the wasm module, and there is no server to send it to.

The editor is worth using for one thing in particular. A `chroma` or `luma`
selector is defined by what it catches in _this_ image, which cannot be read off
the YAML — a layer's **Show** draws its resolved mask over the frame, so the
silhouette it actually cuts is visible before anything is animated.

`tools/web-smoke.mjs` drives the page in headless Chromium — loads an image,
generates a scene, plays it, scrubs, overlays a mask, adds, edits and hides a
layer from the panel, and works each selection tool and applies the result — and
fails if the canvas comes out blank or anything reaches the console.

Recording from the page re-encodes the frames with `MediaRecorder`, which is
lossy and browser-dependent; the CLI is still the way to produce a final loop.

## Commands

| Command                     | What it does                              |
| --------------------------- | ----------------------------------------- |
| `pixelgen pixelate <image>` | Reduce to a pixel grid, write a still PNG |
| `pixelgen animate <image>`  | Render a seamless loop                    |
| `pixelgen init <image>`     | Write a starter scene file                |
| `pixelgen palette <image>`  | Print the derived palette as hex          |
| `pixelgen effects`          | List available effect types               |

Common flags: `--width` (grid width in cells), `--colors` (palette size),
`--dither` (0..1), `--median`, `--saturation`, `--contrast`,
`--palette <file.hex>`, `--seed`, `--scene <file.yaml>`.

`animate` adds `--out` (`-o`), `--scale`, `--fps`, `--seconds`, `--quality`,
`--quiet`. Output format follows the extension of `--out`: `.mp4`, `.webm`,
`.gif`, or `.png` for a numbered frame sequence.

Flags may appear before or after the image argument. Every override is optional
in the strict sense — only a flag actually passed overrides the scene file, so
`--dither 0` means flat color fields rather than "unset".

### Tuning the still first

The palette and grid are usually worth settling before touching animation:

```
pixelgen pixelate photo.jpg --width 240 --colors 24 --median --dither 0.6 --scale 4
```

Lower `--colors` for a harder, more graphic look; past about 48 the result
starts to read as a blurred photograph rather than pixel art.

## Scene files

```yaml
name: porch
source: demo.jpg
width: 320
seed: 1

palette:
  colors: 32 # or: hex: ["#1a1c2c", ...] / file: palette.hex
  dither: 0.6

prepare:
  median: true
  saturation: 1.18
  contrast: 1.06

loop:
  seconds: 8
  fps: 20

regions:
  outside:
    all:
      - rect: { x: 0.11, y: 0.0, w: 0.75, h: 0.82 }
      - chroma: { min: 0.30 }

layers:
  - name: rain
    type: rain
    mask: { ref: outside }
    params:
      count: 300
      speed: 3
      opacity: 0.26
```

### Mask selectors

| Selector                    | Selects                                                     |
| --------------------------- | ----------------------------------------------------------- |
| `rect: {x, y, w, h}`        | A box, in normalized `0..1` coordinates                     |
| `ellipse: {x, y, w, h}`     | An ellipse inscribed in that box                            |
| `polygon: [{x, y}, ...]`    | An arbitrary outline                                        |
| `path: "M.1 .2 L.3 .2 ..."` | SVG path data (`M L H V C S Q T Z`), filled even-odd        |
| `stroke: {d, radius, hardness}` | A brush stroke along path data; `radius` is a fraction of the width |
| `wand: {x, y, hex, tolerance, contiguous}` | Cells near the color `hex`, flooded from the point `x, y` |
| `band: {axis, start, end}`  | A soft gradient along `x` or `y`; reversed if `end < start` |
| `luma: {min, max}`          | Pixels in a brightness range                                |
| `color: {hex, tolerance}`   | Pixels near one color                                       |
| `chroma: {min, max, soft}`  | Pixels in a color-temperature range                         |
| `ref: <name>`               | A region declared in `regions:`                             |
| `all: [...]` / `any: [...]` | Intersection / union                                        |
| `steps: [{add: ...}, {sub: ...}, {and: ...}]` | Each step folded into the ones before: union, difference, intersection; the first must be `add` |

Modifiers `grow` (cells; negative shrinks), `smooth` (cells), `invert`,
`feather` (blur radius in cells) and `gain` apply to any of them, in that
order.

A wand records the color it was clicked on as well as where, and floods from
the nearest cell of that color, so it keeps selecting the same thing when a
change of `width` or palette moves what lies under the point. This is what the
editor's selection tools write:

```yaml
mask:
  steps:
    - add: { wand: { x: 0.41, y: 0.3, hex: "#3a5f8c", tolerance: 0.1 } }
    - add: { path: "M.1 .2L.3 .22L.28 .41Z" }
    - sub: { stroke: { d: "M.2 .3L.25 .31L.3 .35", radius: 0.01 } }
  grow: -1
  feather: 1
```

Unknown keys are rejected at load time, so a typo fails immediately instead of
silently disabling a layer.

## Effects

Parameters named `speed`, `rotations`, `period` and `dir_*` are counted **per
loop** and must be whole numbers; that is what makes the loop close.

The weather effects are sized for a 320-wide grid and scale with the actual
one, so a scene looks the same at any `width`: rain's `length` and `thickness`
are in cells of that reference grid, and mist and steam's `scale` (and steam's
`wobble`) are measured against it too.

| Type            | Purpose                                      | Key parameters                                                                     |
| --------------- | -------------------------------------------- | ---------------------------------------------------------------------------------- |
| `rain`          | Falling streaks in parallax layers           | `count`, `speed`, `layers`, `length`, `thickness`, `slant`, `opacity`, `color`     |
| `mist`          | Drifting fog from tiling fractal noise       | `scale`, `period`, `dir_x`, `dir_y`, `threshold`, `softness`, `opacity`, `octaves` |
| `steam`         | Wisps rising out of the region               | `scale`, `rise`, `threshold`, `opacity`, `wobble`                                  |
| `flicker`       | Brightness and temperature wobble on a light | `amount`, `speed`, `turbulence`, `warmth`, `color`                                 |
| `glow`          | Pulsing bloom radiating from bright pixels   | `radius`, `threshold`, `intensity`, `pulse`, `speed`, `color`                      |
| `twinkle`       | Per-pixel sparkle on highlights              | `threshold`, `amount`, `speed`, `color`                                            |
| `sway`          | Bend a region side to side                   | `amplitude`, `anchor`, `speed`, `wavelength`, `noise`                              |
| `shimmer`       | Rippling displacement for water              | `amplitude`, `wavelength`, `speed`, `axis`                                         |
| `drift`         | Scroll a region, wrapping inside itself      | `speed_x`, `speed_y`, `wrap`                                                       |
| `palette_cycle` | Rotate a contiguous palette range            | `start`, `count`, `rotations`                                                      |
| `vignette`      | Darken the frame edges                       | `amount`, `radius`                                                                 |
| `scanlines`     | CRT line darkening                           | `amount`, `period`                                                                 |
| `breathe`       | Slow global brightness swell                 | `amount`, `speed`                                                                  |

`palette_cycle` works because the palette is sorted into hue-then-luma ramps, so
adjacent indices are adjacent shades and rotating a range reads as flow.

The displacement effects (`sway`, `shimmer`, `drift`) sample from a snapshot
taken before they run. Give them a mask that includes the empty space the region
moves **through** — a mask cropped tightly to the object will clip the motion.

## Adding an effect

Implement `Effect` and register it in `effect::build`:

```rust
impl Effect for Ripple {
    /// Optional. Called once with the finished base and the resolved mask;
    /// anything that does not depend on `t` belongs here, not in `render`.
    fn prepare(&mut self, base: &Image, mask: &Mask, seed: u32) {}

    fn render(&self, dst: &mut Image, ctx: &Context) {}
}
```

Two rules. Be periodic in `ctx.t`. Take `&self` and mean it — frames are
rendered in parallel and out of order, so per-element randomness comes from
`hash01` rather than a live RNG. Both are covered automatically by the tests in
`crates/pixelgen-core/tests/render.rs` as soon as the effect is registered.

## Status

No LLM is involved. The scene format is deliberately declarative and contains no
executable fragment, which leaves room for a planning step later that emits the
same YAML — region proposals from a vision model, checked and rendered by the
same deterministic pipeline.
