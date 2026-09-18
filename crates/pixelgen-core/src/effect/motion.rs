//! Displacement effects: things that move pixels rather than tint them.
//!
//! All three sample from a snapshot taken before they run, not from the live
//! canvas. Reading and writing the same buffer would smear pixels along the
//! displacement direction as the loop overwrote cells it had yet to read.
//!
//! The mask for these effects should include the empty space the region moves
//! through. Pixels are only written where the mask covers, so a mask cropped
//! tightly to the object leaves the swept area untouched and the object appears
//! to be clipped rather than to move.

use core::f32::consts::TAU;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::{decode, whole, Context, Effect, Error};
use crate::noise;
use crate::pixel::Image;

/// Which end of the region stays put.
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Anchor {
    Top,
    Bottom,
}

/// The axis pixels are displaced along. The wave itself travels along the
/// other one.
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Axis {
    X,
    Y,
}

// ---------------------------------------------------------------- sway

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SwayCfg {
    amplitude: f32,
    speed: i32,
    wavelength: f32,
    anchor: Anchor,
    noise: f32,
}

impl Default for SwayCfg {
    fn default() -> Self {
        SwayCfg { amplitude: 1.5, speed: 1, wavelength: 40.0, anchor: Anchor::Top, noise: 0.0 }
    }
}

struct Sway {
    cfg: SwayCfg,
}

pub(super) fn sway(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: SwayCfg = decode(params)?;
    whole("sway", "speed", cfg.speed)?;
    if cfg.wavelength <= 0.0 {
        return Err(Error::BadParams(format!(
            "sway: wavelength is {}, but it divides the row index and must be positive",
            cfg.wavelength
        )));
    }
    Ok(Box::new(Sway { cfg }))
}

impl Effect for Sway {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let src = dst.clone();
        let (x0, y0, x1, y1) = ctx.mask.bounds();
        let h = (y1 - y0) as f32;
        if h <= 0.0 {
            return;
        }
        for y in y0..y1 {
            // Distance from the anchor: a hanging plant pivots at its top and
            // swings most at the tip, grass is the other way around.
            let along = ((y - y0) as f32 + 0.5) / h;
            let lever = if self.cfg.anchor == Anchor::Bottom { 1.0 - along } else { along };

            let phase = ctx.t * self.cfg.speed as f32 * TAU + y as f32 / self.cfg.wavelength * TAU;
            let mut d = self.cfg.amplitude * lever * lever * phase.sin();
            if self.cfg.noise > 0.0 {
                let n = noise::loop1d(y as i32, ctx.t * self.cfg.speed as f32, 1.2, ctx.seed);
                d += self.cfg.noise * lever * (n - 0.5) * 2.0;
            }
            // Snap to whole cells: a smoothly interpolated shift would blur the
            // hard pixel edges the whole pipeline is built to preserve.
            let di = d.round() as i32;
            if di == 0 {
                continue;
            }
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let sx = x as i32 - di;
                if !src.contains(sx, y as i32) {
                    continue;
                }
                let (r, g, b) = src.get(sx as usize, y);
                dst.blend(x as i32, y as i32, r, g, b, cov);
            }
        }
    }
}

// ---------------------------------------------------------------- shimmer

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ShimmerCfg {
    amplitude: f32,
    wavelength: f32,
    speed: i32,
    axis: Axis,
}

impl Default for ShimmerCfg {
    fn default() -> Self {
        ShimmerCfg { amplitude: 1.0, wavelength: 9.0, speed: 1, axis: Axis::X }
    }
}

struct Shimmer {
    cfg: ShimmerCfg,
}

pub(super) fn shimmer(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: ShimmerCfg = decode(params)?;
    whole("shimmer", "speed", cfg.speed)?;
    if cfg.wavelength <= 0.0 {
        return Err(Error::BadParams(format!(
            "shimmer: wavelength is {}, but it divides a coordinate and must be positive",
            cfg.wavelength
        )));
    }
    Ok(Box::new(Shimmer { cfg }))
}

impl Effect for Shimmer {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let src = dst.clone();
        let (x0, y0, x1, y1) = ctx.mask.bounds();
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                // The wave travels along the perpendicular axis, which is what
                // makes it read as a surface rippling rather than a whole block
                // sliding back and forth.
                let along = if self.cfg.axis == Axis::Y { x as f32 } else { y as f32 };
                let phase = along / self.cfg.wavelength * TAU + ctx.t * self.cfg.speed as f32 * TAU;
                let d = (self.cfg.amplitude * phase.sin()).round() as i32;
                if d == 0 {
                    continue;
                }
                let (mut sx, mut sy) = (x as i32, y as i32);
                match self.cfg.axis {
                    Axis::Y => sy -= d,
                    Axis::X => sx -= d,
                }
                if !src.contains(sx, sy) {
                    continue;
                }
                let (r, g, b) = src.get(sx as usize, sy as usize);
                dst.blend(x as i32, y as i32, r, g, b, cov);
            }
        }
    }
}

// ---------------------------------------------------------------- drift

/// Scrolls a region.
///
/// Speed is counted in whole traversals of the masked region per loop, not in
/// cells per loop: the region wraps around its own bounding box, so only a
/// whole number of traversals returns it to its starting position, and any
/// other unit would let the author request a scroll that cannot close. The
/// sign gives the direction.
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct DriftCfg {
    speed_x: i32,
    speed_y: i32,
    wrap: bool,
}

impl Default for DriftCfg {
    fn default() -> Self {
        DriftCfg { speed_x: 1, speed_y: 0, wrap: true }
    }
}

struct Drift {
    cfg: DriftCfg,
}

pub(super) fn drift(params: &Value) -> Result<Box<dyn Effect>, Error> {
    let cfg: DriftCfg = decode(params)?;
    // Go quietly set speed_x to 1 here, so a scene that asked for a still
    // region got a scrolling one. Both zero is far more likely to be a mistake
    // than a request for a layer that does nothing.
    if cfg.speed_x == 0 && cfg.speed_y == 0 {
        return Err(Error::BadParams(
            "drift: speed_x and speed_y are both 0, so the region would not move - \
             set one of them, or remove the layer"
                .into(),
        ));
    }
    Ok(Box::new(Drift { cfg }))
}

impl Effect for Drift {
    fn render(&self, dst: &mut Image, ctx: &Context) {
        let (x0, y0, x1, y1) = ctx.mask.bounds();
        let (rw, rh) = (x1 as i32 - x0 as i32, y1 as i32 - y0 as i32);
        if rw <= 0 || rh <= 0 {
            return;
        }
        let ox = (ctx.t * (self.cfg.speed_x * rw) as f32).round() as i32;
        let oy = (ctx.t * (self.cfg.speed_y * rh) as f32).round() as i32;
        if ox == 0 && oy == 0 {
            return;
        }
        let src = dst.clone();
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = ctx.mask.at(x as i32, y as i32);
                if cov <= 0.0 {
                    continue;
                }
                let (mut sx, mut sy) = (x as i32 - ox, y as i32 - oy);
                if self.cfg.wrap {
                    // Wrap inside the region itself, so the seam is hidden by
                    // the region's own content rather than by the frame edge.
                    sx = x0 as i32 + wrap_int(sx - x0 as i32, rw);
                    sy = y0 as i32 + wrap_int(sy - y0 as i32, rh);
                } else if !src.contains(sx, sy) {
                    continue;
                }
                let (r, g, b) = src.get(sx as usize, sy as usize);
                dst.blend(x as i32, y as i32, r, g, b, cov);
            }
        }
    }
}

fn wrap_int(v: i32, n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    v.rem_euclid(n)
}
