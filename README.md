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
resolution with ffmpeg doing the integer nearest-neighbour upscale.

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
`width`. Masks can be rectangles, ellipses, polygons, soft bands, a luma range, "pixels
near this colour", or a colour-temperature range, combined with `all`
(intersect) and `any` (union), and modified by `invert`, `feather` and `gain`.

### Depth, without a depth map

Rectangles cannot express depth, and most scenes have some: rain belongs outside
the window and behind everything standing in front of it, and lamplight must
stop at the wall. Drawing either with a box puts rain indoors and light on the
rocks.

The `chroma` selector solves this for the common case, because a scene that has
an inside and an outside almost always splits by colour temperature — warm lamp,
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
| `crates/pixelgen-core` | The whole renderer: masks, palette, effects, scene format. No file, process or network I/O; rayon is the only non-`serde` dependency and it is compiled out on wasm |
| `crates/pixelgen-cli`  | The `pixelgen` binary: image decoding, ffmpeg, GIF writing                                                                                                          |
| `crates/pixelgen-wasm` | `wasm-bindgen` wrapper around the core, for the browser UI                                                                                                          |

The split is load-bearing rather than cosmetic. The core never opens a file —
`palette.file` is reported to the host and folded back in by
`Scene::set_palette_hex` — so the browser runs the identical renderer instead of
a reimplementation that drifts.

## In the browser

The renderer compiles to wasm, and `web/` is a small editor built on it: open a
photograph, generate a starter scene, edit the YAML with the loop playing beside
it, and draw or preview masks over the frame.

```
cargo install wasm-bindgen-cli
./build-web.sh
python3 -m http.server -d web 8000
```

Nothing is uploaded and there is no build step beyond the wasm: the page is
three static files and a `<script type="module">`.

The editor is worth using for one thing in particular. A `chroma` or `luma`
selector is defined by what it catches in _this_ image, which cannot be read off
the YAML — clicking a layer draws its resolved mask over the frame, so the
silhouette it actually cuts is visible before anything is animated.

`tools/web-smoke.mjs` drives the page in headless Chromium — loads an image,
generates a scene, plays it, scrubs, overlays a mask, draws a polygon — and
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
`--dither 0` means flat colour fields rather than "unset".

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
| `band: {axis, start, end}`  | A soft gradient along `x` or `y`; reversed if `end < start` |
| `luma: {min, max}`          | Pixels in a brightness range                                |
| `color: {hex, tolerance}`   | Pixels near one colour                                      |
| `chroma: {min, max, soft}`  | Pixels in a colour-temperature range                        |
| `ref: <name>`               | A region declared in `regions:`                             |
| `all: [...]` / `any: [...]` | Intersection / union                                        |

Modifiers `invert`, `feather` (blur radius in cells) and `gain` apply to any of
them, in that order.

Unknown keys are rejected at load time, so a typo fails immediately instead of
silently disabling a layer.

## Effects

Parameters named `speed`, `rotations`, `period` and `dir_*` are counted **per
loop** and must be whole numbers; that is what makes the loop close.

| Type            | Purpose                                      | Key parameters                                                                     |
| --------------- | -------------------------------------------- | ---------------------------------------------------------------------------------- |
| `rain`          | Falling streaks in parallax layers           | `count`, `speed`, `layers`, `length`, `slant`, `opacity`, `color`                  |
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
