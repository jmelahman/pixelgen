//! Whole-frame treatments applied last.
//!
//! `vignette` and `scanlines` are the two effects that do not animate. They are
//! effects rather than fixed pipeline stages so that a scene can place them in
//! the layer order, mask them, and leave them out entirely.

use core::f32::consts::TAU;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::{decode, whole, Context, Effect, Error};
use crate::pixel::{lerp, smooth_step, Image};

// ---------------------------------------------------------------- vignette

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct VignetteCfg {
    amount: f32,
    /// Where the darkening starts, as a fraction of the distance from the
    /// center to a corner.
    radius: f32,
}

impl Default for VignetteCfg {
    fn default() -> Self {
        VignetteCfg { amount: 0.3, radius: 0.75 }
    }
}

struct Vignette {
    cfg: VignetteCfg,
}

pub(super) fn vignette(params: &Value) -> Result<Box<dyn Effect>, Error> {
    Ok(Box::new(Vignette { cfg: decode(params)? }))
}

impl Effect for Vignette {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let (cx, cy) = (dst.w as f32 / 2.0, dst.h as f32 / 2.0);
        let max_d = cx.hypot(cy);
        for y in 0..dst.h {
            for x in 0..dst.w {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) / max_d;
                let f = smooth_step(self.cfg.radius, 1.0, d) * self.cfg.amount * cov;
                if f <= 0.0 {
                    continue;
                }
                let (r, g, b) = dst.get(x, y);
                dst.set(x, y, r * (1.0 - f), g * (1.0 - f), b * (1.0 - f));
            }
        }
    }
}

// ---------------------------------------------------------------- scanlines

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ScanlinesCfg {
    amount: f32,
    /// One row in every `period` is darkened.
    period: usize,
}

impl Default for ScanlinesCfg {
    fn default() -> Self {
        ScanlinesCfg { amount: 0.12, period: 2 }
    }
}

struct Scanlines {
    cfg: ScanlinesCfg,
}

pub(super) fn scanlines(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: ScanlinesCfg = decode(params)?;
    if cfg.period < 1 {
        return Err(Error::BadParams(
            "scanlines: period is 0, but it is the spacing between darkened rows".into(),
        ));
    }
    Ok(Box::new(Scanlines { cfg }))
}

impl Effect for Scanlines {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        for y in (0..dst.h).step_by(self.cfg.period) {
            for x in 0..dst.w {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let f = self.cfg.amount * cov;
                let (r, g, b) = dst.get(x, y);
                dst.set(x, y, r * (1.0 - f), g * (1.0 - f), b * (1.0 - f));
            }
        }
    }
}

// ---------------------------------------------------------------- breathe

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct BreatheCfg {
    amount: f32,
    speed: i32,
}

impl Default for BreatheCfg {
    fn default() -> Self {
        BreatheCfg { amount: 0.05, speed: 1 }
    }
}

struct Breathe {
    cfg: BreatheCfg,
}

pub(super) fn breathe(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: BreatheCfg = decode(params)?;
    whole("breathe", "speed", cfg.speed)?;
    Ok(Box::new(Breathe { cfg }))
}

impl Effect for Breathe {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let gain = 1.0 + self.cfg.amount * (ctx.t * self.cfg.speed as f32 * TAU).sin();
        let (x0, y0, x1, y1) = ctx.mask.bounds();
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let (r, g, b) = dst.get(x, y);
                let k = lerp(1.0, gain, cov);
                dst.set(
                    x,
                    y,
                    (r * k).clamp(0.0, 1.0),
                    (g * k).clamp(0.0, 1.0),
                    (b * k).clamp(0.0, 1.0),
                );
            }
        }
    }
}
