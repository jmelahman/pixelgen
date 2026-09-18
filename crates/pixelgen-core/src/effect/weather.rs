//! Rain, fog and steam.

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::{color, decode, grid_scale, positive, whole, Context, Effect, Error};
use crate::noise;
use crate::pixel::{fract, smooth_step, Image, Rgb};

// ---------------------------------------------------------------- rain

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RainCfg {
    count: i32,
    /// Whole traversals of the frame per loop. A fractional count would leave
    /// every drop mid-fall at the loop point and produce a visible jump.
    speed: i32,
    /// `length` and `thickness` are in cells of the reference grid and scale
    /// with the actual one; see [`scaled`].
    length: f32,
    /// A fractional part is drawn as a fainter edge cell, and a thickness
    /// written under one cell as a single faint one.
    thickness: f32,
    slant: f32,
    opacity: f32,
    layers: i32,
    color: Option<String>,
}

impl Default for RainCfg {
    fn default() -> Self {
        RainCfg {
            count: 220,
            speed: 2,
            length: 7.0,
            thickness: 1.0,
            slant: 0.28,
            opacity: 0.34,
            layers: 3,
            color: None,
        }
    }
}

struct Rain {
    cfg: RainCfg,
    color: Rgb,
}

pub(super) fn rain(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: RainCfg = decode(params)?;
    whole("rain", "speed", cfg.speed)?;
    whole("rain", "layers", cfg.layers)?;
    positive("rain", "length", cfg.length)?;
    positive("rain", "thickness", cfg.thickness)?;
    let c = color(&cfg.color, Rgb { r: 0.78, g: 0.85, b: 1.0 })?;
    Ok(Box::new(Rain { cfg, color: c }))
}

impl Effect for Rain {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let (w, h) = (dst.w as i32, dst.h as f32);
        if w == 0 {
            return;
        }
        let k = grid_scale(dst);
        for layer in 0..self.cfg.layers {
            // Depth cues: distant layers are slower, shorter, thinner and
            // fainter. Three layers moving at one speed would read as a flat
            // sheet.
            let depth = layer as f32 / self.cfg.layers as f32;
            let near = 1.0 - 0.45 * depth;
            // A streak longer than the frame looks no different and only
            // costs rows, and one wider than the frame cannot be drawn, so
            // both stop there and a huge value cannot run away.
            let len_l = scaled(self.cfg.length, near, k, 2.0).min(h);
            let thick = scaled(self.cfg.thickness, near, k, 1.0).min(w as f32);
            let full = thick.floor() as i32;
            let part = thick - full as f32;
            let span = full + (part > 0.0) as i32;
            let alpha_l = self.cfg.opacity * (1.0 - 0.55 * depth);
            let cycles = (self.cfg.speed - layer).max(1);
            let count_l = self.cfg.count / self.cfg.layers;

            for i in 0..count_l {
                let x0 = noise::hash01(i, layer, 0, 0, ctx.seed) * w as f32;
                let phase = noise::hash01(i, layer, 1, 0, ctx.seed);
                // Slight per-drop length jitter so streaks do not look stamped.
                let jitter = 0.7 + 0.6 * noise::hash01(i, layer, 2, 0, ctx.seed);
                let l = len_l * jitter;
                // Each drop travels its own length past the bottom, so that it
                // has fully left the frame before it wraps back to the top.
                let travel = h + l;

                let head = fract(phase + ctx.t * cycles as f32) * travel - l;
                let mut s = 0.0f32;
                while s < l {
                    let yi = noise::ifloor(head + s);
                    if yi < 0 || yi as f32 >= h {
                        s += 1.0;
                        continue;
                    }
                    // Taper both ends of the streak; a constant-alpha
                    // segment reads as a stick rather than motion blur.
                    let taper = smooth_step(0.0, l * 0.35, s) * smooth_step(l, l * 0.6, s);
                    // The drop's column wraps with the slant, as it always
                    // has. Its thickness is centred on that column and
                    // pushed back inside at the frame edges rather than
                    // wrapped, so a streak never splits across the frame.
                    let centre = noise::ifloor(x0 + s * self.cfg.slant).rem_euclid(w);
                    let start = (centre - (span - 1) / 2).clamp(0, w - span);
                    for dx in 0..span {
                        let xi = start + dx;
                        let cov = ctx.mask.at(xi, yi);
                        if cov > 0.0 {
                            let edge = if dx < full { 1.0 } else { part };
                            dst.blend(
                                xi,
                                yi,
                                self.color.r,
                                self.color.g,
                                self.color.b,
                                alpha_l * taper * cov * edge,
                            );
                        }
                    }
                    s += 1.0;
                }
            }
        }
    }
}

/// A written size at the given depth, on a grid `k` times the reference.
///
/// Neither the grid nor the depth takes it below `least`, so shrinking a scene
/// leaves short, thin rain rather than none; only a value written smaller than
/// that goes lower.
fn scaled(written: f32, depth: f32, k: f32, least: f32) -> f32 {
    (written * depth * k).max(written.min(least))
}

// ---------------------------------------------------------------- mist

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct MistCfg {
    /// Noise frequency per cell of the reference grid, so the clouds keep
    /// their size relative to the frame at any grid width.
    scale: f32,
    /// The noise lattice wraps every `period` cells and the field is scrolled
    /// by `period * dir` cells over one loop, so the directions must be whole
    /// numbers for the scroll to land back on the lattice and the loop to
    /// close.
    period: i32,
    dir_x: i32,
    dir_y: i32,
    threshold: f32,
    softness: f32,
    opacity: f32,
    octaves: i32,
    turbulence: f32,
    color: Option<String>,
}

impl Default for MistCfg {
    fn default() -> Self {
        MistCfg {
            scale: 0.035,
            period: 4,
            dir_x: 1,
            dir_y: 0,
            threshold: 0.5,
            softness: 0.28,
            opacity: 0.3,
            octaves: 3,
            turbulence: 0.7,
            color: None,
        }
    }
}

struct Mist {
    cfg: MistCfg,
    color: Rgb,
}

pub(super) fn mist(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: MistCfg = decode(params)?;
    whole("mist", "period", cfg.period)?;
    whole("mist", "octaves", cfg.octaves)?;
    if cfg.dir_x == 0 && cfg.dir_y == 0 {
        return Err(Error::BadParams(
            "mist: dir_x and dir_y are both 0, so the fog would never move".into(),
        ));
    }
    let c = color(&cfg.color, Rgb { r: 0.82, g: 0.88, b: 0.95 })?;
    Ok(Box::new(Mist { cfg, color: c }))
}

impl Effect for Mist {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        // The drift is in lattice cells, so rescaling the frequency leaves
        // the loop closing exactly as before.
        let scale = self.cfg.scale / grid_scale(dst);
        let (x0, y0, x1, y1) = ctx.mask.bounds();
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let v = noise::drift_fbm(
                    x as f32,
                    y as f32,
                    ctx.t,
                    scale,
                    self.cfg.period,
                    self.cfg.dir_x as f32,
                    self.cfg.dir_y as f32,
                    self.cfg.turbulence,
                    self.cfg.octaves,
                    ctx.seed,
                );
                let a = smooth_step(self.cfg.threshold, self.cfg.threshold + self.cfg.softness, v);
                dst.blend(
                    x as i32,
                    y as i32,
                    self.color.r,
                    self.color.g,
                    self.color.b,
                    a * self.cfg.opacity * cov,
                );
            }
        }
    }
}

// ---------------------------------------------------------------- steam

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SteamCfg {
    /// Like mist's, per cell of the reference grid.
    scale: f32,
    rise: i32,
    threshold: f32,
    softness: f32,
    opacity: f32,
    octaves: i32,
    turbulence: f32,
    /// Sideways sway in cells of the reference grid.
    wobble: f32,
    color: Option<String>,
}

impl Default for SteamCfg {
    fn default() -> Self {
        SteamCfg {
            scale: 0.09,
            rise: 3,
            threshold: 0.55,
            softness: 0.2,
            opacity: 0.45,
            octaves: 2,
            turbulence: 1.1,
            wobble: 2.5,
            color: None,
        }
    }
}

struct Steam {
    cfg: SteamCfg,
    color: Rgb,
}

pub(super) fn steam(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: SteamCfg = decode(params)?;
    whole("steam", "rise", cfg.rise)?;
    whole("steam", "octaves", cfg.octaves)?;
    let c = color(&cfg.color, Rgb { r: 0.92, g: 0.93, b: 0.95 })?;
    Ok(Box::new(Steam { cfg, color: c }))
}

impl Effect for Steam {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let k = grid_scale(dst);
        let scale = self.cfg.scale / k;
        let (x0, y0, x1, y1) = ctx.mask.bounds();
        let h = (y1 - y0) as f32;
        if h <= 0.0 {
            return;
        }
        for y in y0..y1 {
            // Steam is dense where it leaves the source and dissipates as it
            // climbs, so coverage falls off with height through the region.
            let up = 1.0 - ((y - y0) as f32 + 0.5) / h;
            let fade = up * up;
            // A slow horizontal wobble keeps the column from looking
            // extruded. It moves whole rows, so it is sampled once per row.
            let wob = self.cfg.wobble
                * k
                * (noise::looped(
                    0.0,
                    y as f32 * 0.1 / k,
                    ctx.t,
                    1.0,
                    1.0,
                    ctx.seed.wrapping_add(17),
                ) - 0.5);
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let v = noise::drift_fbm(
                    x as f32 + wob,
                    y as f32,
                    ctx.t,
                    scale,
                    4,
                    0.0,
                    -(self.cfg.rise as f32),
                    self.cfg.turbulence,
                    self.cfg.octaves,
                    ctx.seed,
                );
                let a = smooth_step(self.cfg.threshold, self.cfg.threshold + self.cfg.softness, v);
                dst.blend(
                    x as i32,
                    y as i32,
                    self.color.r,
                    self.color.g,
                    self.color.b,
                    a * self.cfg.opacity * fade * cov,
                );
            }
        }
    }
}
