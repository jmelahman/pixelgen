//! Refining a mask's edge: growing or shrinking it, and rounding it off.

use super::{box_pass, Mask};
use crate::pixel::{lerp, smooth_step};

/// Dilates by `r` cells, or erodes when `r` is negative.
///
/// Grayscale, so a soft edge moves as a soft edge rather than being thresholded
/// first. Erosion is dilation of the complement, and cells beyond the frame
/// take no part in either: the frame's edge is not an edge of the region, so a
/// shrunk selection still reaches it.
///
/// A fractional radius blends the two whole radii around it, which keeps a
/// slider from moving the edge in visible one-cell jumps.
pub fn grow(m: &Mask, r: f32) -> Mask {
    if r == 0.0 {
        return m.clone();
    }
    let erode = r < 0.0;
    let r = r.abs();
    let mut cur = m.clone();
    if erode {
        complement(&mut cur);
    }
    let whole = r.floor() as usize;
    cur = dilate(&cur, whole);
    let frac = r - whole as f32;
    if frac > 0.0 {
        let more = dilate(&cur, 1);
        for (v, n) in cur.a.iter_mut().zip(&more.a) {
            *v = lerp(*v, *n, frac);
        }
    }
    if erode {
        complement(&mut cur);
    }
    cur
}

fn complement(m: &mut Mask) {
    for v in m.a.iter_mut() {
        *v = 1.0 - *v;
    }
}

/// Dilation by a disk of `r` cells, done as repeated dilation by a disk of at
/// most four: the sum of two disks is a disk, and the small one keeps the cost
/// linear in the radius rather than quadratic.
fn dilate(m: &Mask, r: usize) -> Mask {
    let mut cur = m.clone();
    let mut left = r;
    while left > 0 {
        let step = left.min(4);
        cur = dilate_disk(&cur, step);
        left -= step;
    }
    cur
}

fn dilate_disk(m: &Mask, r: usize) -> Mask {
    let ri = r as i32;
    // A disk of radius 1 is the plus shape: the 3x3 square would grow
    // diagonals faster than edges, and a closing by it squares off corners.
    let lim = (r as f32 + 0.35).powi(2);
    let offs: Vec<(i32, i32)> = (-ri..=ri)
        .flat_map(|dy| (-ri..=ri).map(move |dx| (dx, dy)))
        .filter(|&(dx, dy)| ((dx * dx + dy * dy) as f32) <= lim)
        .collect();
    let mut out = Mask::new(m.w, m.h);
    for y in 0..m.h as i32 {
        for x in 0..m.w as i32 {
            let mut best = 0.0f32;
            for &(dx, dy) in &offs {
                best = best.max(m.at(x + dx, y + dy));
                if best >= 1.0 {
                    break;
                }
            }
            out.a[y as usize * m.w + x as usize] = best;
        }
    }
    out
}

/// Rounds corners and drops anything narrower than about `r` cells: a box blur
/// of that radius, pulled back to a hard-ish edge around one half.
pub fn smooth(m: &Mask, r: f32) -> Mask {
    let ri = r.round() as i32;
    if ri < 1 {
        return m.clone();
    }
    let mut out = box_pass(&box_pass(m, ri, true), ri, false);
    for v in out.a.iter_mut() {
        *v = smooth_step(0.4, 0.6, *v);
    }
    out
}
