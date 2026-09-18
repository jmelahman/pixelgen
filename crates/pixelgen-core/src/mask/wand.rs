//! The magic wand: cells like the one clicked on.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::{Error, Mask};
use crate::palette::{dist2, parse_hex};
use crate::pixel::{Image, Rgb};

/// Selects the cells whose color is within `tolerance` of a seed's.
///
/// The seed is where the author clicked, but a width, palette or prepare
/// change moves which cell lands under that point - and the wand would then
/// quietly flood something else. So the color clicked on is recorded in
/// `hex`, compared against instead of whatever is now under the point, and
/// the flood starts from the nearest cell of that color close by.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Wand {
    pub x: f32,
    pub y: f32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hex: String,
    /// On the same scale as `color`'s; zero means that selector's default.
    #[serde(skip_serializing_if = "super::is_zero")]
    pub tolerance: f32,
    /// Only cells connected to the seed, rather than every matching cell.
    #[serde(skip_serializing_if = "is_true")]
    pub contiguous: bool,
}

impl Default for Wand {
    fn default() -> Self {
        Wand { x: 0.0, y: 0.0, hex: String::new(), tolerance: 0.0, contiguous: true }
    }
}

fn is_true(b: &bool) -> bool {
    *b
}

/// How far, in cells, a recorded color is looked for around the seed point.
const REACH: i32 = 2;

pub fn build(wd: &Wand, base: &Image, dither: bool) -> Result<Mask, Error> {
    let (w, h) = (base.w, base.h);
    let mut m = Mask::new(w, h);
    if w == 0 || h == 0 {
        return Ok(m);
    }
    // A dithered base scatters two colors through every blend between them,
    // which a flood cannot cross. Its local average is what the eye sees.
    let img = if dither { box3(base) } else { base.clone() };
    let at = |x: usize, y: usize| {
        let (r, g, b) = img.get(x, y);
        Rgb { r, g, b }
    };

    let sx = ((wd.x * w as f32) as i32).clamp(0, w as i32 - 1);
    let sy = ((wd.y * h as f32) as i32).clamp(0, h as i32 - 1);
    let (target, seed) = if wd.hex.is_empty() {
        (at(sx as usize, sy as usize), (sx as usize, sy as usize))
    } else {
        let t = parse_hex(&wd.hex).map_err(|_| Error::BadColor(wd.hex.clone()))?;
        let mut best = (f32::MAX, (sx as usize, sy as usize));
        for dy in -REACH..=REACH {
            for dx in -REACH..=REACH {
                let (x, y) = (sx + dx, sy + dy);
                if !base.contains(x, y) {
                    continue;
                }
                // Nearer cells win ties, so an exact match under the point is
                // always the one taken.
                let d = dist2(at(x as usize, y as usize), t) + (dx * dx + dy * dy) as f32 * 1e-6;
                if d < best.0 {
                    best = (d, (x as usize, y as usize));
                }
            }
        }
        (t, best.1)
    };

    let tol = if wd.tolerance <= 0.0 { 0.12 } else { wd.tolerance };
    // The same scale as `color`: squared distance summed over three channels.
    // `dist2` weights the channels to sum to 9 rather than 3.
    let tol2 = tol * tol * 9.0;
    let hit = |x: usize, y: usize| dist2(at(x, y), target) <= tol2;

    if wd.contiguous {
        let mut q = VecDeque::from([seed]);
        m.a[seed.1 * w + seed.0] = 1.0;
        while let Some((x, y)) = q.pop_front() {
            let n = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
            for (nx, ny) in n {
                if nx >= w || ny >= h || m.a[ny * w + nx] > 0.0 || !hit(nx, ny) {
                    continue;
                }
                m.a[ny * w + nx] = 1.0;
                q.push_back((nx, ny));
            }
        }
    } else {
        for y in 0..h {
            for x in 0..w {
                if hit(x, y) {
                    m.a[y * w + x] = 1.0;
                }
            }
        }
        // Clicking on something always selects it, however strict the
        // tolerance: a wand that can come back empty fails the whole scene.
        m.a[seed.1 * w + seed.0] = 1.0;
    }
    Ok(m)
}

fn box3(im: &Image) -> Image {
    let mut out = Image::new(im.w, im.h);
    for y in 0..im.h as i32 {
        for x in 0..im.w as i32 {
            let (mut s, mut n) = ([0.0f32; 3], 0.0);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if im.contains(x + dx, y + dy) {
                        let (r, g, b) = im.get((x + dx) as usize, (y + dy) as usize);
                        s[0] += r;
                        s[1] += g;
                        s[2] += b;
                        n += 1.0;
                    }
                }
            }
            out.set(x as usize, y as usize, s[0] / n, s[1] / n, s[2] / n);
        }
    }
    out
}

/// The color of the cell under a normalized point, as `#rrggbb` - what a wand
/// placed there records.
pub fn sample(base: &Image, x: f32, y: f32) -> String {
    if base.w == 0 || base.h == 0 {
        return "#000000".into();
    }
    let cx = ((x * base.w as f32) as usize).min(base.w - 1);
    let cy = ((y * base.h as f32) as usize).min(base.h - 1);
    let (r, g, b) = base.get(cx, cy);
    crate::palette::hex(Rgb { r, g, b })
}
