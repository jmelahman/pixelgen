//! Lamps, sparkle, bloom and palette animation.

use core::f32::consts::TAU;

use serde::Deserialize;
use serde_yaml::Value;

use super::{color, decode, whole, Context, Effect, Error};
use crate::mask::Mask;
use crate::noise;
use crate::pixel::{fract, lerp, luma, smooth_step, Image, Rgb};

// ---------------------------------------------------------------- flicker

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FlickerCfg {
    amount: f32,
    speed: i32,
    turbulence: f32,
    warmth: f32,
    color: Option<String>,
}

impl Default for FlickerCfg {
    fn default() -> Self {
        FlickerCfg { amount: 0.16, speed: 3, turbulence: 1.6, warmth: 0.0, color: None }
    }
}

struct Flicker {
    cfg: FlickerCfg,
    tint: Rgb,
}

pub(super) fn flicker(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: FlickerCfg = decode(params)?;
    whole("flicker", "speed", cfg.speed)?;
    let tint = color(&cfg.color, Rgb { r: 1.0, g: 0.82, b: 0.55 })?;
    Ok(Box::new(Flicker { cfg, tint }))
}

impl Effect for Flicker {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        // One scalar drives the whole region: a real flame lights everything it
        // touches in step, so per-pixel variation here would look like static.
        //
        // The octaves are amplitude-weighted rather than averaged. Averaging N
        // independent noise values pulls the result towards their common mean
        // and quietly throttles the flicker to a few percent no matter what
        // `amount` says; halving the weight each octave keeps the slow
        // component at full strength and lets the faster ones only roughen it.
        let (mut f, mut amp, mut norm) = (0.0f32, 1.0f32, 0.0f32);
        for h in 0..self.cfg.speed {
            f += amp * noise::loop1d(h, ctx.t * (h + 1) as f32, self.cfg.turbulence, ctx.seed);
            norm += amp;
            amp *= 0.5;
        }
        f /= norm;
        let gain = 1.0 + self.cfg.amount * (f - 0.5) * 2.0;

        let (x0, y0, x1, y1) = ctx.mask.bounds();
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let (r, g, b) = dst.get(x, y);
                let k = lerp(1.0, gain, cov);
                let (mut nr, mut ng, mut nb) =
                    ((r * k).clamp(0.0, 1.0), (g * k).clamp(0.0, 1.0), (b * k).clamp(0.0, 1.0));
                if self.cfg.warmth > 0.0 {
                    // Brighter flame is also warmer; pushing hue with intensity
                    // sells it far more than luminance alone.
                    let w = self.cfg.warmth
                        * cov
                        * ((gain - 1.0) / (self.cfg.amount + 1e-6).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                    nr = lerp(nr, self.tint.r, w).clamp(0.0, 1.0);
                    ng = lerp(ng, self.tint.g, w).clamp(0.0, 1.0);
                    nb = lerp(nb, self.tint.b, w).clamp(0.0, 1.0);
                }
                dst.set(x, y, nr, ng, nb);
            }
        }
    }
}

// ---------------------------------------------------------------- twinkle

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TwinkleCfg {
    amount: f32,
    speed: i32,
    threshold: f32,
    color: Option<String>,
}

impl Default for TwinkleCfg {
    fn default() -> Self {
        TwinkleCfg { amount: 0.5, speed: 2, threshold: 0.62, color: None }
    }
}

struct Twinkle {
    cfg: TwinkleCfg,
    tint: Rgb,
}

pub(super) fn twinkle(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: TwinkleCfg = decode(params)?;
    whole("twinkle", "speed", cfg.speed)?;
    let tint = color(&cfg.color, Rgb { r: 1.0, g: 0.95, b: 0.8 })?;
    Ok(Box::new(Twinkle { cfg, tint }))
}

impl Effect for Twinkle {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let (x0, y0, x1, y1) = ctx.mask.bounds();
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                // Key off the untouched base so the set of twinkling points is
                // fixed for the whole loop, rather than wandering as other
                // effects brighten and darken pixels beneath this one.
                let (br, bg, bb) = ctx.base.get(x, y);
                let l = luma(br, bg, bb);
                if l < self.cfg.threshold {
                    continue;
                }
                let weight = smooth_step(self.cfg.threshold, 1.0, l);
                // Each point gets its own phase offset, so the lights shimmer
                // independently instead of pulsing as one block.
                let off = noise::hash01(x as i32, y as i32, 0, 0, ctx.seed);
                let phase = fract(ctx.t * self.cfg.speed as f32 + off);
                let pulse = (phase * TAU).sin();
                let a = self.cfg.amount * weight * cov * pulse.clamp(0.0, 1.0);
                dst.blend(x as i32, y as i32, self.tint.r, self.tint.g, self.tint.b, a);
            }
        }
    }
}

// ---------------------------------------------------------------- glow

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GlowCfg {
    radius: f32,
    threshold: f32,
    intensity: f32,
    pulse: f32,
    speed: i32,
    color: Option<String>,
}

impl Default for GlowCfg {
    fn default() -> Self {
        GlowCfg { radius: 6.0, threshold: 0.65, intensity: 0.35, pulse: 0.3, speed: 1, color: None }
    }
}

struct Glow {
    cfg: GlowCfg,
    tint: Rgb,
    /// The bloom source depends only on the static base and the mask, so it is
    /// built once in `prepare` rather than reblurred for every frame.
    field: Vec<f32>,
}

pub(super) fn glow(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: GlowCfg = decode(params)?;
    whole("glow", "speed", cfg.speed)?;
    let tint = color(&cfg.color, Rgb { r: 1.0, g: 0.85, b: 0.6 })?;
    Ok(Box::new(Glow { cfg, tint, field: Vec::new() }))
}

impl Effect for Glow {
    fn prepare(&mut self, base: &Image, mask: &Mask, _seed: u32) {
        let (w, h) = (base.w, base.h);
        let mut src = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let cov = mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let l = base.luma_at(x, y);
                src[y * w + x] = smooth_step(self.cfg.threshold, 1.0, l) * cov;
            }
        }
        self.field = blur_field(src, w, h, self.cfg.radius as i32);
    }

    fn render(&self, dst: &mut Image, ctx: &Context) {
        let p = (fract(ctx.t * self.cfg.speed as f32) * TAU).sin();
        let amp = self.cfg.intensity * (1.0 + self.cfg.pulse * p);
        for y in 0..dst.h {
            for x in 0..dst.w {
                let v = self.field[y * dst.w + x];
                if v <= 0.0 {
                    continue;
                }
                dst.blend(
                    x as i32,
                    y as i32,
                    self.tint.r,
                    self.tint.g,
                    self.tint.b,
                    (v * amp).clamp(0.0, 1.0),
                );
            }
        }
    }
}

/// A three-pass separable box blur, the same Gaussian approximation the mask
/// feathering uses.
fn blur_field(src: Vec<f32>, w: usize, h: usize, r: i32) -> Vec<f32> {
    if r < 1 {
        return src;
    }
    let mut cur = src;
    for _ in 0..3 {
        cur = box_pass(&cur, w, h, r, true);
        cur = box_pass(&cur, w, h, r, false);
    }
    cur
}

fn box_pass(src: &[f32], w: usize, h: usize, r: i32, horizontal: bool) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for d in -r..=r {
                let (xx, yy) =
                    if horizontal { (x as i32 + d, y as i32) } else { (x as i32, y as i32 + d) };
                if xx < 0 || yy < 0 || xx as usize >= w || yy as usize >= h {
                    continue;
                }
                sum += src[yy as usize * w + xx as usize];
                n += 1.0;
            }
            out[y * w + x] = sum / n;
        }
    }
    out
}

// ---------------------------------------------------------------- palette cycle

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PaletteCycleCfg {
    start: usize,
    count: usize,
    /// Counted in complete rotations of the range per loop, which is the only
    /// unit guaranteed to bring every pixel back to its original entry.
    rotations: i32,
}

impl Default for PaletteCycleCfg {
    fn default() -> Self {
        PaletteCycleCfg { start: 0, count: 6, rotations: 1 }
    }
}

/// Rotates pixels through a contiguous run of palette entries.
///
/// This is the classic 1990s technique for animating water, neon and fire with
/// no extra frames, and it works here because the palette is sorted into
/// perceptual ramps: adjacent indices are adjacent shades, so rotation reads as
/// flow.
struct PaletteCycle {
    cfg: PaletteCycleCfg,
}

pub(super) fn palette_cycle(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: PaletteCycleCfg = decode(params)?;
    whole("palette_cycle", "rotations", cfg.rotations)?;
    if cfg.count < 2 {
        return Err(Error::BadParams(format!(
            "palette_cycle: count is {}, but rotating fewer than two entries changes nothing",
            cfg.count
        )));
    }
    Ok(Box::new(PaletteCycle { cfg }))
}

impl Effect for PaletteCycle {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let pal = &ctx.matcher.palette;
        if self.cfg.start >= pal.len() {
            return;
        }
        let count = self.cfg.count.min(pal.len() - self.cfg.start);
        if count < 2 {
            return;
        }
        let shift = (ctx.t * (self.cfg.rotations as usize * count) as f32) as usize % count;

        let (x0, y0, x1, y1) = ctx.mask.bounds();
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let (r, g, b) = dst.get(x, y);
                let idx = ctx.matcher.index(r, g, b);
                if idx < self.cfg.start || idx >= self.cfg.start + count {
                    continue;
                }
                let rel = (idx - self.cfg.start + shift) % count;
                let c = pal[self.cfg.start + rel];
                dst.blend(x as i32, y as i32, c.r, c.g, c.b, cov);
            }
        }
    }
}
