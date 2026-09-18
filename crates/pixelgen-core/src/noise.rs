//! Noise that closes into a seamless loop.
//!
//! Every effect in a background loop has to return to its starting state, and
//! the usual fixes - cross-fading the end into the beginning, or playing the
//! sequence forwards then backwards - either show a soft spot or make the
//! motion visibly reverse. Both are avoided here by never generating a sequence
//! that needs repairing.
//!
//! The trick is dimensional. Time is not a line but a circle traced through two
//! extra dimensions of a 4D noise field: as the phase `t` runs from 0 to 1 the
//! sample point goes around that circle and arrives exactly where it started,
//! so the first and last frames agree by construction. Drifting fields add a
//! second requirement, that the field itself tile, which [`value4_tiling`]
//! provides by wrapping lattice coordinates at a whole-number period.

use crate::pixel::lerp;
use core::f32::consts::TAU;

#[inline]
fn hash4(x: i32, y: i32, z: i32, w: i32, seed: u32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((y as u32).wrapping_mul(668_265_263))
        .wrapping_add((z as u32).wrapping_mul(2_147_483_647))
        .wrapping_add((w as u32).wrapping_mul(1_274_126_177))
        .wrapping_add(seed.wrapping_mul(362_437));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0xff_ffff) as f32 / 0x100_0000 as f32
}

/// Perlin's `6t^5-15t^4+10t^3` fade. Its first and second derivatives vanish at
/// the ends, so lattice boundaries leave no visible creases.
#[inline]
fn quintic(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
pub fn ifloor(v: f32) -> i32 {
    v.floor() as i32
}

/// 4D value noise in `[0,1)`.
pub fn value4(x: f32, y: f32, z: f32, w: f32, seed: u32) -> f32 {
    let (xi, yi, zi, wi) = (ifloor(x), ifloor(y), ifloor(z), ifloor(w));
    let u = quintic(x - xi as f32);
    let v = quintic(y - yi as f32);
    let s = quintic(z - zi as f32);
    let r = quintic(w - wi as f32);

    let mut acc = [0.0f32; 2];
    for (dw, slot) in acc.iter_mut().enumerate() {
        let mut acc_z = [0.0f32; 2];
        for (dz, zslot) in acc_z.iter_mut().enumerate() {
            let (dz, dw) = (dz as i32, dw as i32);
            let c00 = hash4(xi, yi, zi + dz, wi + dw, seed);
            let c10 = hash4(xi + 1, yi, zi + dz, wi + dw, seed);
            let c01 = hash4(xi, yi + 1, zi + dz, wi + dw, seed);
            let c11 = hash4(xi + 1, yi + 1, zi + dz, wi + dw, seed);
            *zslot = lerp(lerp(c00, c10, u), lerp(c01, c11, u), v);
        }
        *slot = lerp(acc_z[0], acc_z[1], s);
    }
    lerp(acc[0], acc[1], r)
}

/// Sample a 2D noise field at spatial `(x,y)` and looping phase `t` in `[0,1)`.
///
/// `scale` is in lattice cells; `turbulence` controls how much the field churns
/// over one loop versus merely drifting.
#[inline]
pub fn looped(x: f32, y: f32, t: f32, scale: f32, turbulence: f32, seed: u32) -> f32 {
    let ang = t * TAU;
    value4(x * scale, y * scale, ang.cos() * turbulence, ang.sin() * turbulence, seed)
}

/// Stack octaves of [`looped`] for a field with detail at several scales, the
/// shape smoke and cloud effects need.
pub fn fbm(x: f32, y: f32, t: f32, scale: f32, turbulence: f32, octaves: i32, seed: u32) -> f32 {
    let octaves = octaves.max(1);
    let (mut sum, mut amp, mut norm) = (0.0f32, 1.0f32, 0.0f32);
    let mut freq = scale;
    for i in 0..octaves {
        sum += amp * looped(x, y, t, freq, turbulence, seed.wrapping_add(i as u32 * 7919));
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// A periodic scalar in `[0,1)` driven only by phase, used for per-element
/// flicker where each element gets its own id.
#[inline]
pub fn loop1d(id: i32, t: f32, turbulence: f32, seed: u32) -> f32 {
    let ang = t * TAU;
    value4(id as f32 * 13.7, 0.0, ang.cos() * turbulence, ang.sin() * turbulence, seed)
}

/// Reduce `v` into `[0, period)`.
#[inline]
fn wrap(v: i32, period: i32) -> i32 {
    if period <= 0 {
        return v;
    }
    v.rem_euclid(period)
}

/// [`value4`] with the x and y lattice wrapped to a period.
///
/// Wrapping the lattice is what allows a field to be scrolled: translating by
/// exactly one period lands on identical values, so an effect that drifts a
/// whole period over one loop returns precisely to its starting state.
pub fn value4_tiling(x: f32, y: f32, z: f32, w: f32, px: i32, py: i32, seed: u32) -> f32 {
    let (xi, yi, zi, wi) = (ifloor(x), ifloor(y), ifloor(z), ifloor(w));
    let u = quintic(x - xi as f32);
    let v = quintic(y - yi as f32);
    let s = quintic(z - zi as f32);
    let r = quintic(w - wi as f32);

    let (x0, x1) = (wrap(xi, px), wrap(xi + 1, px));
    let (y0, y1) = (wrap(yi, py), wrap(yi + 1, py));

    let mut acc = [0.0f32; 2];
    for (dw, slot) in acc.iter_mut().enumerate() {
        let mut acc_z = [0.0f32; 2];
        for (dz, zslot) in acc_z.iter_mut().enumerate() {
            let (dz, dw) = (dz as i32, dw as i32);
            let c00 = hash4(x0, y0, zi + dz, wi + dw, seed);
            let c10 = hash4(x1, y0, zi + dz, wi + dw, seed);
            let c01 = hash4(x0, y1, zi + dz, wi + dw, seed);
            let c11 = hash4(x1, y1, zi + dz, wi + dw, seed);
            *zslot = lerp(lerp(c00, c10, u), lerp(c01, c11, u), v);
        }
        *slot = lerp(acc_z[0], acc_z[1], s);
    }
    lerp(acc[0], acc[1], r)
}

/// Tiling fractal noise that has been scrolled by phase `t`.
///
/// Both kinds of looping are combined here. The field drifts by exactly
/// `period` lattice cells over one loop, which the tiling lattice makes
/// seamless, and it simultaneously churns around the time circle. The result
/// reads as fog moving in a direction while also evolving, yet returns exactly
/// to frame zero.
#[allow(clippy::too_many_arguments)]
pub fn drift_fbm(
    x: f32,
    y: f32,
    t: f32,
    scale: f32,
    period: i32,
    dir_x: f32,
    dir_y: f32,
    turbulence: f32,
    octaves: i32,
    seed: u32,
) -> f32 {
    let period = period.max(1);
    let octaves = octaves.max(1);
    let ang = t * TAU;
    let cz = ang.cos() * turbulence;
    let cw = ang.sin() * turbulence;

    let (mut sum, mut amp, mut norm) = (0.0f32, 1.0f32, 0.0f32);
    let mut freq = scale;
    let mut per = period;
    for i in 0..octaves {
        // The drift distance scales with the octave's period so every octave
        // completes a whole number of wraps over the loop.
        let ox = t * per as f32 * dir_x;
        let oy = t * per as f32 * dir_y;
        sum += amp
            * value4_tiling(
                x * freq + ox,
                y * freq + oy,
                cz,
                cw,
                per,
                per,
                seed.wrapping_add(i as u32 * 7919),
            );
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
        per *= 2;
    }
    sum / norm
}

/// The lattice hash, for callers that need a reproducible random value keyed on
/// integers rather than a smooth field: per-drop positions, per-light phase
/// offsets, and similar.
///
/// It avoids keeping a seeded generator alive across frames, which parallel
/// rendering would make order-dependent.
#[inline]
pub fn hash01(a: i32, b: i32, c: i32, d: i32, seed: u32) -> f32 {
    hash4(a, b, c, d, seed)
}
