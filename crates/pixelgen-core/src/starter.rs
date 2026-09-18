//! A starter scene written from what can be measured in an image.
//!
//! The scene is emitted as a commented template rather than serialized YAML.
//! Without understanding the picture the geometry here is a guess, and the
//! guess is only useful if the author can see which numbers to move and why;
//! round-tripped YAML would strip exactly the explanation that makes it
//! editable.
//!
//! This lives in the core rather than in the CLI because the browser wants the
//! same starting point, and a second implementation of it would drift.

use core::fmt::Write as _;

use crate::mask::chroma_of;
use crate::palette::hex;
use crate::pixel::{luma, Image, Rgb};
use crate::scene::Scene;

/// Everything `init` needs from an image, measured in one pass.
pub fn starter(s: &Scene, source: &str, base: &Image) -> (String, Analysis) {
    let a = analyze(base);
    let text = template(s, source, &a);
    (text, a)
}

/// The scene `name:` for a source with no name of its own in the scene file.
/// Deliberately not `std::path`: `source` may be a bare upload name in the
/// browser, and the two must agree on the answer.
fn stem(source: &str) -> String {
    let file = source.rsplit(['/', '\\']).next().unwrap_or(source);
    let base = match file.rfind('.') {
        Some(i) if i > 0 => &file[..i],
        _ => file,
    };
    if base.is_empty() {
        "scene".into()
    } else {
        base.into()
    }
}

pub struct WarmLight {
    pub x: f32,
    pub y: f32,
    pub hex: String,
}

/// The small amount of structure that can be read out of an image without
/// understanding what it depicts.
pub struct Analysis {
    /// Normalized y of the strongest horizontal brightness edge.
    pub horizon: f32,
    /// Luma above which a pixel counts as a highlight, and how many do.
    pub threshold: f32,
    pub bright_frac: f32,
    pub warm_light: Option<WarmLight>,
    /// Fraction of the frame that reads as cool, when the image splits by
    /// temperature clearly enough for a region to be worth emitting.
    pub cool_frac: f32,
}

/// The chroma value the emitted `outside` region keys on. Empirically this
/// separates daylight beyond a window from a warm interior with room to spare
/// on both sides.
const COOL: f32 = 0.30;

pub fn analyze(base: &Image) -> Analysis {
    let (w, h) = (base.w, base.h);

    // Row brightness profile. The most negative step in the top two thirds is
    // usually where an open sky or window gives way to darker foreground,
    // which is the boundary most atmospheric effects want to respect.
    let row_luma: Vec<f32> =
        (0..h).map(|y| (0..w).map(|x| base.luma_at(x, y)).sum::<f32>() / w as f32).collect();
    let mut best = (0.0f32, h / 3);
    for y in 1..(h * 2 / 3) {
        let drop = row_luma[y - 1] - row_luma[y];
        if drop > best.0 {
            best = (drop, y);
        }
    }
    let mut horizon = best.1 as f32 / h as f32;
    if horizon < 0.15 {
        horizon = 0.45;
    }

    // Highlights, and the warm ones among them. A warm highlight is the
    // signature of a practical light in frame: a lamp, a lantern, a window.
    //
    // The threshold is a percentile of this image rather than a fixed value.
    // A dim night scene can have its brightest, most light-like pixels sitting
    // well below any absolute cutoff, and a fixed threshold simply reports
    // that such an image contains no lights at all.
    let mut sorted: Vec<f32> =
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| base.luma_at(x, y)).collect();
    sorted.sort_by(f32::total_cmp);
    // Guard against an almost entirely black frame, where the 98th percentile
    // is still shadow and everything would read as a highlight.
    let threshold = sorted[(sorted.len() as f32 * 0.98) as usize].max(0.35);

    let (mut bright, mut cool) = (0usize, 0usize);
    let (mut sum, mut pos, mut warm_n) = (Rgb { r: 0.0, g: 0.0, b: 0.0 }, (0.0f32, 0.0f32), 0.0f32);
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = base.get(x, y);
            if chroma_of(r, g, b) >= COOL {
                cool += 1;
            }
            if luma(r, g, b) < threshold {
                continue;
            }
            bright += 1;
            if r - b > 0.1 {
                sum.r += r;
                sum.g += g;
                sum.b += b;
                pos.0 += x as f32;
                pos.1 += y as f32;
                warm_n += 1.0;
            }
        }
    }

    let n = (w * h) as f32;
    // A handful of stray pixels is noise, not a light source.
    let warm_light = (warm_n >= 8.0).then(|| WarmLight {
        x: pos.0 / warm_n / w as f32,
        y: pos.1 / warm_n / h as f32,
        hex: hex(Rgb { r: sum.r / warm_n, g: sum.g / warm_n, b: sum.b / warm_n }),
    });

    // Only worth a region when the split is real: a frame that is almost all
    // cool, or barely cool at all, would produce a region covering everything
    // or nothing and teach the author the wrong thing about the selector.
    let cool_frac = cool as f32 / n;
    let cool_frac = if (0.05..0.7).contains(&cool_frac) { cool_frac } else { 0.0 };

    Analysis { horizon, threshold, bright_frac: bright as f32 / n, warm_light, cool_frac }
}

pub fn template(s: &Scene, source: &str, a: &Analysis) -> String {
    let name = if s.name.is_empty() { stem(source) } else { s.name.clone() };
    let mut b = String::new();

    let _ = write!(
        b,
        r#"# Generated by "pixelgen init". Every region below is a guess made from
# brightness and colour alone - open the image, and move the numbers to match
# what is actually in it. Preview a single frame at any time with:
#
#   pixelgen pixelate --scene THIS_FILE --scale 4
#
# Coordinates are fractions of the frame: x and y start at the top left, w and h
# are widths and heights, all in 0..1.

name: {name}
source: {source}

# Width of the pixel grid in cells. Lower is chunkier; 160-400 is the usual
# range for a wallpaper. Everything else is resolution-independent.
width: {width}
seed: {seed}

palette:
  # Colours are clustered out of the image itself. Dropping this towards 16
  # gives a harder, more graphic look; raising it past 48 starts to look like a
  # blurred photograph rather than pixel art.
  colors: {colors}
  # Dithering is applied once to the static base, never per frame, so the
  # pattern cannot crawl. 0 turns it off for flat colour fields.
  dither: {dither:.2}

prepare:
  # Median-filter the source before downscaling, to stop sensor noise and fine
  # texture surviving as isolated speckles.
  median: true
  # Photographs are usually too muted to survive a small palette; these push
  # colour and tone apart before the palette is chosen.
  saturation: {sat:.2}
  contrast: {con:.2}

loop:
  seconds: {secs}
  fps: {fps}
"#,
        width = s.width,
        seed = s.seed,
        colors = if s.palette.colors > 0 { s.palette.colors } else { 32 },
        dither = s.palette.dither,
        sat = s.prepare.saturation,
        con = s.prepare.contrast,
        secs = s.loop_.seconds,
        fps = s.loop_.fps,
    );

    // Named regions, when the image has a temperature split worth keying on.
    // This is the part worth reading first: a rectangle cannot say "outside the
    // window", and almost every depth mistake in a scene comes from pretending
    // that it can.
    let outside = if a.cool_frac > 0.0 {
        let _ = write!(
            b,
            r#"
# Named regions. A layer refers to one with "mask: {{ ref: NAME }}", so the
# hard part - working out where a thing actually is - is done once.
#
# {pct:.0}% of this frame reads as cool, so it probably splits into a warm
# interior and a cooler exterior. "chroma" keys on colour temperature rather
# than position: positive is cool, negative is warm. That gives correct
# occlusion for free, because anything warm standing in front of the exterior -
# a figure, a plant, the window frame - fails the test and is cut out.
regions:
  outside:
    all:
      # Narrow this rectangle to the opening the exterior is seen through.
      - rect: {{ x: 0.0, y: 0.0, w: 1.0, h: {h:.2} }}
      - chroma: {{ min: {cool:.2} }}
  indoors:
    ref: outside
    invert: true
"#,
            pct = a.cool_frac * 100.0,
            h = clamp01(a.horizon + 0.4),
            cool = COOL,
        );
        true
    } else {
        false
    };

    b.push_str("\nlayers:\n");

    let atmosphere_mask = if outside {
        "      ref: outside".to_string()
    } else {
        format!(
            "      band: {{ axis: y, start: {:.2}, end: {:.2} }}",
            clamp01(a.horizon + 0.22),
            clamp01(a.horizon - 0.18)
        )
    };

    let _ = write!(
        b,
        r#"  # Atmosphere over the open part of the frame. The brightness profile put
  # the horizon at y={horizon:.2}.
  - name: haze
    type: mist
    mask:
{atmosphere_mask}
    params:
      scale: 0.03        # smaller is larger, softer clouds
      period: 4          # lattice periods crossed per loop; must be a whole number
      dir_x: 1           # drift direction
      dir_y: 0
      threshold: 0.52    # raise for thinner, patchier fog
      softness: 0.3
      opacity: 0.22

  # Rain. Remove this whole entry for a dry scene.
  - name: rain
    type: rain
    mask:
{rain_mask}
    params:
      count: 260         # drops across the frame
      speed: 2           # full falls per loop; whole numbers only, or it jumps
      layers: 3          # parallax depth
      length: 7
      slant: 0.28
      opacity: 0.3
"#,
        horizon = a.horizon,
        rain_mask = if outside {
            "      ref: outside".to_string()
        } else {
            "      band: { axis: y, start: 0.0, end: 0.25 }".to_string()
        },
    );

    if let Some(l) = &a.warm_light {
        // Intersecting the lamp with "indoors" is what stops its bloom
        // spilling onto whatever is visible through the window behind it.
        let confine = if outside { "\n      - ref: indoors" } else { "" };
        let _ = write!(
            b,
            r#"
  # A warm highlight was found near ({lx:.2}, {ly:.2}) - probably a practical
  # light. The ellipse is a guess at its reach; widen it until the falloff
  # looks right.{note}
  - name: lamp-glow
    type: glow
    mask:
      all:
      - ellipse: {{ x: {gx:.2}, y: {gy:.2}, w: 0.24, h: 0.24 }}{confine}
    params:
      color: "{hex}"
      radius: 7
      threshold: 0.6
      intensity: 0.3
      pulse: 0.35
      speed: 1

  - name: lamp-flicker
    type: flicker
    mask:
      all:
      - ellipse: {{ x: {fx:.2}, y: {fy:.2}, w: 0.32, h: 0.32 }}{confine}
      feather: 3
    params:
      amount: 0.14
      speed: 3
      turbulence: 1.6
      warmth: 0.25
      color: "{hex}"
"#,
            lx = l.x,
            ly = l.y,
            gx = clamp01(l.x - 0.12),
            gy = clamp01(l.y - 0.12),
            fx = clamp01(l.x - 0.16),
            fy = clamp01(l.y - 0.16),
            hex = l.hex,
            note = if outside {
                "\n  # Intersected with \"indoors\" so the bloom stops at the window."
            } else {
                ""
            },
        );
    }

    if (0.002..0.08).contains(&a.bright_frac) {
        let _ = write!(
            b,
            r#"
  # {pct:.1}% of the image reads as highlights, above luma {t:.2} - a good
  # candidate for sparkle. Raise "threshold" until only the points you want
  # are moving.
  - name: highlights
    type: twinkle
    params:
      threshold: {t:.2}
      amount: 0.35
      speed: 2
"#,
            pct = a.bright_frac * 100.0,
            t = a.threshold,
        );
    }

    b.push_str(
        r#"
  # Foliage and hanging objects. The mask must include the empty space the
  # region swings through, or the movement gets clipped at the edges.
  # - name: leaves
  #   type: sway
  #   mask:
  #     rect: { x: 0.0, y: 0.0, w: 0.35, h: 0.5 }
  #   params:
  #     amplitude: 1.5     # cells of travel at the free end
  #     anchor: top        # "top" for hanging plants, "bottom" for grass
  #     speed: 1
  #     wavelength: 40

  # Water, reflections, heat.
  # - name: water
  #   type: shimmer
  #   mask:
  #     rect: { x: 0.0, y: 0.8, w: 1.0, h: 0.2 }
  #   params: { amplitude: 1, wavelength: 9, speed: 1, axis: x }

  - name: edges
    type: vignette
    params:
      amount: 0.25
      radius: 0.7
"#,
    );
    b
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}
