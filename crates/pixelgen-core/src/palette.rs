//! Palette extraction, matching and dithering.

use crate::pixel::{luma, Image, Rgb};
use serde::{Deserialize, Serialize};

pub type Palette = Vec<Rgb>;

/// Bits per channel of the lookup table. Six gives 262144 entries, which is
/// small enough to build in a few milliseconds and fine enough that the color
/// it returns differs from the true nearest only for colors sitting almost
/// exactly on a boundary between two palette entries.
const LUT_BITS: u32 = 6;
const LUT_SIZE: usize = 1 << (LUT_BITS * 3);

/// Matcher snaps arbitrary colors onto a fixed palette.
///
/// Every frame is snapped, so this is the hottest path in the renderer; a
/// precomputed table turns a linear search over the palette into one index.
#[derive(Clone)]
pub struct Matcher {
    pub palette: Palette,
    lut: Vec<u16>,
}

/// Weighted RGB distance ("redmean"), a cheap approximation of perceptual
/// difference that is markedly better than plain Euclidean RGB at the thing
/// that matters here: not letting a saturated color collapse onto a gray.
pub fn dist2(a: Rgb, b: Rgb) -> f32 {
    let rmean = (a.r + b.r) * 0.5;
    let (dr, dg, db) = (a.r - b.r, a.g - b.g, a.b - b.b);
    (2.0 + rmean) * dr * dr + 4.0 * dg * dg + (3.0 - rmean) * db * db
}

impl Matcher {
    pub fn new(palette: Palette) -> Self {
        let n = (1 << LUT_BITS) as f32 - 1.0;
        let mut lut = vec![0u16; LUT_SIZE];
        for (i, slot) in lut.iter_mut().enumerate() {
            let r = ((i >> (LUT_BITS * 2)) & ((1 << LUT_BITS) - 1)) as f32 / n;
            let g = ((i >> LUT_BITS) & ((1 << LUT_BITS) - 1)) as f32 / n;
            let b = (i & ((1 << LUT_BITS) - 1)) as f32 / n;
            *slot = nearest_in(&palette, Rgb { r, g, b }) as u16;
        }
        Matcher { palette, lut }
    }

    #[inline]
    pub fn index(&self, r: f32, g: f32, b: f32) -> usize {
        let n = ((1 << LUT_BITS) - 1) as f32;
        let q = |v: f32| (v.clamp(0.0, 1.0) * n + 0.5) as usize;
        self.lut[(q(r) << (LUT_BITS * 2)) | (q(g) << LUT_BITS) | q(b)] as usize
    }

    #[inline]
    pub fn nearest(&self, r: f32, g: f32, b: f32) -> Rgb {
        self.palette[self.index(r, g, b)]
    }

    /// Snap every pixel onto the palette, with no dithering.
    ///
    /// Frames are snapped this way rather than dithered because a dither
    /// pattern recomputed per frame crawls: the pattern shifts wherever a pixel
    /// lands on the other side of a threshold, and a field of flat color
    /// appears to boil.
    pub fn snap(&self, img: &mut Image) {
        for i in (0..img.pix.len()).step_by(3) {
            let c = self.nearest(img.pix[i], img.pix[i + 1], img.pix[i + 2]);
            img.pix[i] = c.r;
            img.pix[i + 1] = c.g;
            img.pix[i + 2] = c.b;
        }
    }

    /// Floyd-Steinberg error diffusion, applied once to the static base.
    pub fn dither(&self, img: &mut Image, strength: f32) {
        if strength <= 0.0 {
            self.snap(img);
            return;
        }
        let s = strength.clamp(0.0, 1.0);
        let (w, h) = (img.w, img.h);
        for y in 0..h {
            for x in 0..w {
                let o = img.offset(x, y);
                let old = Rgb { r: img.pix[o], g: img.pix[o + 1], b: img.pix[o + 2] };
                let new = self.nearest(old.r, old.g, old.b);
                img.pix[o] = new.r;
                img.pix[o + 1] = new.g;
                img.pix[o + 2] = new.b;
                let err = [(old.r - new.r) * s, (old.g - new.g) * s, (old.b - new.b) * s];
                for (dx, dy, f) in [
                    (1, 0, 7.0 / 16.0),
                    (-1, 1, 3.0 / 16.0),
                    (0, 1, 5.0 / 16.0),
                    (1, 1, 1.0 / 16.0),
                ] {
                    spread(img, x as i32 + dx, y as i32 + dy, &err, f);
                }
            }
        }
    }
}

#[inline]
fn spread(img: &mut Image, x: i32, y: i32, err: &[f32; 3], f: f32) {
    if !img.contains(x, y) {
        return;
    }
    let o = img.offset(x as usize, y as usize);
    for (c, e) in err.iter().enumerate().take(3) {
        img.pix[o + c] = (img.pix[o + c] + e * f).clamp(0.0, 1.0);
    }
}

pub fn nearest_in(pal: &[Rgb], c: Rgb) -> usize {
    let mut best = 0;
    let mut bd = f32::MAX;
    for (i, p) in pal.iter().enumerate() {
        let d = dist2(c, *p);
        if d < bd {
            bd = d;
            best = i;
        }
    }
    best
}

/// Extract a palette from an image by k-means, seeded with k-means++.
///
/// Plain random seeding regularly loses a small bright element (a lamp, a sign)
/// into a cluster dominated by the background, and that element is usually the
/// subject. k-means++ picks seeds spread apart by distance, so a small but
/// distinct color survives.
pub fn extract(img: &Image, k: usize, seed: u32) -> Palette {
    let samples = sample_pixels(img, 20_000);
    let k = k.clamp(2, 256).min(samples.len());
    let mut rng = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        (rng & 0x00FF_FFFF) as f32 / 16_777_215.0
    };

    let mut centers: Vec<Rgb> = Vec::with_capacity(k);
    centers.push(samples[(rand() * (samples.len() - 1) as f32) as usize]);
    let mut d2: Vec<f32> = samples.iter().map(|s| dist2(*s, centers[0])).collect();
    while centers.len() < k {
        let total: f32 = d2.iter().sum();
        let mut target = rand() * total;
        let mut pick = samples.len() - 1;
        for (i, d) in d2.iter().enumerate() {
            target -= *d;
            if target <= 0.0 {
                pick = i;
                break;
            }
        }
        let c = samples[pick];
        centers.push(c);
        for (i, s) in samples.iter().enumerate() {
            d2[i] = d2[i].min(dist2(*s, c));
        }
    }

    let mut assign = vec![0usize; samples.len()];
    for _ in 0..24 {
        let mut moved = false;
        for (i, s) in samples.iter().enumerate() {
            let a = nearest_in(&centers, *s);
            if a != assign[i] {
                assign[i] = a;
                moved = true;
            }
        }
        let mut sums = vec![[0.0f64; 3]; centers.len()];
        let mut counts = vec![0usize; centers.len()];
        for (i, s) in samples.iter().enumerate() {
            let a = assign[i];
            sums[a][0] += s.r as f64;
            sums[a][1] += s.g as f64;
            sums[a][2] += s.b as f64;
            counts[a] += 1;
        }
        for (i, c) in centers.iter_mut().enumerate() {
            if counts[i] == 0 {
                // An empty cluster is wasted palette capacity; respawn it on a
                // random sample rather than leave a color unused.
                *c = samples[(rand() * (samples.len() - 1) as f32) as usize];
                moved = true;
                continue;
            }
            let n = counts[i] as f64;
            *c = Rgb {
                r: (sums[i][0] / n) as f32,
                g: (sums[i][1] / n) as f32,
                b: (sums[i][2] / n) as f32,
            };
        }
        if !moved {
            break;
        }
    }
    sort(centers)
}

/// Sample by a fixed stride rather than randomly, so the same image always
/// yields the same palette.
fn sample_pixels(img: &Image, want: usize) -> Vec<Rgb> {
    let total = img.w * img.h;
    let stride = (total / want.max(1)).max(1);
    let mut out = Vec::with_capacity(total / stride + 1);
    let mut i = 0;
    while i < total {
        let o = i * 3;
        out.push(Rgb { r: img.pix[o], g: img.pix[o + 1], b: img.pix[o + 2] });
        i += stride;
    }
    out
}

/// Order a palette into grays first, then hue buckets, each by brightness.
///
/// The order is not cosmetic. `palette_cycle` animates by rotating a contiguous
/// range of indices, which only reads as flowing color if neighboring indices
/// are neighboring shades.
pub fn sort(mut pal: Palette) -> Palette {
    pal.sort_by(|a, b| key(*a).partial_cmp(&key(*b)).unwrap());
    pal
}

fn key(c: Rgb) -> (u8, u8, f32) {
    let (h, s, l) = hsl(c);
    if s < 0.12 {
        (0, 0, l) // grays lead, ordered by lightness
    } else {
        (1, (h * 12.0) as u8, luma(c.r, c.g, c.b))
    }
}

fn hsl(c: Rgb) -> (f32, f32, f32) {
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == c.r {
        ((c.g - c.b) / d).rem_euclid(6.0)
    } else if max == c.g {
        (c.b - c.r) / d + 2.0
    } else {
        (c.r - c.g) / d + 4.0
    };
    (h / 6.0, s, l)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HexError(pub String);

impl std::fmt::Display for HexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid hex color {:?}", self.0)
    }
}

impl std::error::Error for HexError {}

pub fn parse_hex(s: &str) -> Result<Rgb, HexError> {
    let t = s.trim().trim_start_matches('#');
    let expand = |v: u8| v as f32 / 255.0;
    match t.len() {
        6 => {
            let n = u32::from_str_radix(t, 16).map_err(|_| HexError(s.into()))?;
            Ok(Rgb { r: expand((n >> 16) as u8), g: expand((n >> 8) as u8), b: expand(n as u8) })
        }
        3 => {
            let n = u32::from_str_radix(t, 16).map_err(|_| HexError(s.into()))?;
            let c = |v: u32| expand(((v << 4) | v) as u8);
            Ok(Rgb { r: c((n >> 8) & 0xF), g: c((n >> 4) & 0xF), b: c(n & 0xF) })
        }
        _ => Err(HexError(s.into())),
    }
}

/// Parse a whole palette, reporting the first color that will not parse.
pub fn parse_hex_all(list: &[&str]) -> Result<Palette, HexError> {
    list.iter().map(|s| parse_hex(s)).collect()
}

pub fn hex(c: Rgb) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    format!("#{:02x}{:02x}{:02x}", q(c.r), q(c.g), q(c.b))
}
