//! Rain, fog and steam.

use serde::Deserialize;
use serde_yaml::Value;

use super::{color, decode, whole, Context, Effect, Error};
use crate::noise;
use crate::pixel::{fract, smooth_step, Image, Rgb};

// ---------------------------------------------------------------- rain

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RainCfg {
    count: i32,
    /// Whole traversals of the frame per loop. A fractional count would leave
    /// every drop mid-fall at the loop point and produce a visible jump.
    speed: i32,
    length: f32,
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
    let c = color(&cfg.color, Rgb { r: 0.78, g: 0.85, b: 1.0 })?;
    Ok(Box::new(Rain { cfg, color: c }))
}

impl Effect for Rain {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let height = dst.h as f32;
        for layer in 0..self.cfg.layers {
            // Depth cues: distant layers are slower, shorter and fainter.
            // Three layers moving at one speed would read as a flat sheet.
            let depth = layer as f32 / self.cfg.layers as f32;
            let len_l = self.cfg.length * (1.0 - 0.45 * depth);
            let alpha_l = self.cfg.opacity * (1.0 - 0.55 * depth);
            let cycles = (self.cfg.speed - layer).max(1);
            let count_l = self.cfg.count / self.cfg.layers;
            let travel = height + len_l;

            for i in 0..count_l {
                let x0 = noise::hash01(i, layer, 0, 0, ctx.seed) * dst.w as f32;
                let phase = noise::hash01(i, layer, 1, 0, ctx.seed);
                // Slight per-drop length jitter so streaks do not look stamped.
                let jitter = 0.7 + 0.6 * noise::hash01(i, layer, 2, 0, ctx.seed);
                let l = len_l * jitter;

                let head = fract(phase + ctx.t * cycles as f32) * travel - l;
                let mut s = 0.0f32;
                while s < l {
                    let y = head + s;
                    let yi = noise::ifloor(y);
                    if yi < 0 || yi >= dst.h as i32 {
                        s += 1.0;
                        continue;
                    }
                    let xi = noise::ifloor(x0 + s * self.cfg.slant).rem_euclid(dst.w as i32);
                    let cov = ctx.mask.at(xi, yi);
                    if cov > 0.0 {
                        // Taper both ends of the streak; a constant-alpha
                        // segment reads as a stick rather than motion blur.
                        let taper = smooth_step(0.0, l * 0.35, s) * smooth_step(l, l * 0.6, s);
                        dst.blend(xi, yi, self.color.r, self.color.g, self.color.b, alpha_l * taper * cov);
                    }
                    s += 1.0;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- mist

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MistCfg {
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
                    self.cfg.scale,
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

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SteamCfg {
    scale: f32,
    rise: i32,
    threshold: f32,
    softness: f32,
    opacity: f32,
    octaves: i32,
    turbulence: f32,
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
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                // A slow horizontal wobble keeps the column from looking
                // extruded.
                let wob = self.cfg.wobble
                    * (noise::looped(0.0, y as f32 * 0.1, ctx.t, 1.0, 1.0, ctx.seed.wrapping_add(17)) - 0.5);
                let v = noise::drift_fbm(
                    x as f32 + wob,
                    y as f32,
                    ctx.t,
                    self.cfg.scale,
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
