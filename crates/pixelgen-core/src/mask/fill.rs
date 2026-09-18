//! Rasterizing outlines into coverage.

use super::path::{Polyline, Pt};
use super::Mask;
use crate::pixel::smooth_step;

/// Horizontal samples are exact; vertical ones are this many per cell.
const SUBROWS: usize = 4;

/// Fills closed polylines, given in grid cells, with the even-odd rule.
///
/// A scanline fill rather than a per-pixel crossing test: the test costs every
/// edge for every cell, which a freehand lasso of a few hundred points turns
/// into tens of millions of steps on every rebuild of the scene. Coverage is
/// exact across each span and sampled four times down each cell, so an edge
/// fades over one cell instead of stepping.
pub fn polygons(rings: &[Vec<Pt>], w: usize, h: usize) -> Mask {
    let mut m = Mask::new(w, h);
    // Every edge as (y0, y1, x0, x1), closing each ring.
    let mut edges: Vec<(f32, f32, f32, f32)> = Vec::new();
    for r in rings {
        if r.len() < 3 {
            continue;
        }
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if a.1 != b.1 {
                edges.push((a.1, b.1, a.0, b.0));
            }
        }
    }
    if edges.is_empty() {
        return m;
    }
    let (lo, hi) = edges
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), e| (lo.min(e.0).min(e.1), hi.max(e.0).max(e.1)));
    let y0 = (lo.floor().max(0.0) as usize).min(h);
    let y1 = (hi.ceil().max(0.0) as usize).min(h);
    let wt = 1.0 / SUBROWS as f32;
    let mut xs: Vec<f32> = Vec::new();
    for y in y0..y1 {
        let row = &mut m.a[y * w..(y + 1) * w];
        for k in 0..SUBROWS {
            let sy = y as f32 + (k as f32 + 0.5) * wt;
            xs.clear();
            for &(ya, yb, xa, xb) in &edges {
                if (ya <= sy) != (yb <= sy) {
                    xs.push(xa + (sy - ya) / (yb - ya) * (xb - xa));
                }
            }
            xs.sort_by(f32::total_cmp);
            for &[a, b] in xs.as_chunks::<2>().0 {
                span(row, a, b, wt);
            }
        }
        // Snap float noise: an edge written as 0.6 lands a hair past cell 60
        // at width 100, and would otherwise leave that column a sliver.
        for v in row.iter_mut() {
            *v = if *v < 1e-4 {
                0.0
            } else if *v > 1.0 - 1e-4 {
                1.0
            } else {
                *v
            };
        }
    }
    m
}

/// Adds `wt` times the fraction of each cell that `[a, b)` covers.
fn span(row: &mut [f32], a: f32, b: f32, wt: f32) {
    let w = row.len() as f32;
    let (a, b) = (a.clamp(0.0, w), b.clamp(0.0, w));
    if b <= a {
        return;
    }
    let (ia, ib) = (a.floor() as usize, b.floor() as usize);
    if ia == ib {
        row[ia] += (b - a) * wt;
        return;
    }
    row[ia] += (ia as f32 + 1.0 - a) * wt;
    for v in &mut row[ia + 1..ib] {
        *v += wt;
    }
    if ib < row.len() {
        row[ib] += (b - ib as f32) * wt;
    }
}

/// Coverage within `radius` cells of each polyline, with the edge softened
/// inward by `hardness` below 1 - a brush, and so round at its ends.
pub fn strokes(lines: &[Polyline], radius: f32, hardness: f32, w: usize, h: usize) -> Mask {
    let mut m = Mask::new(w, h);
    let r = radius.max(0.0);
    // A hard brush still gets one cell of antialiasing, centered on its edge.
    let outer = r + 0.5;
    let inner = (r * hardness.clamp(0.0, 1.0)).min(r) - 0.5;
    for l in lines {
        let mut segs: Vec<(Pt, Pt)> = l.pts.windows(2).map(|p| (p[0], p[1])).collect();
        if l.closed && l.pts.len() > 2 {
            segs.push((*l.pts.last().unwrap(), l.pts[0]));
        }
        if segs.is_empty() {
            // A click without a drag is a dot.
            segs.extend(l.pts.first().map(|&p| (p, p)));
        }
        for (a, b) in segs {
            let bx0 = ((a.0.min(b.0) - outer).floor().max(0.0) as usize).min(w);
            let bx1 = ((a.0.max(b.0) + outer).ceil().max(0.0) as usize).min(w);
            let by0 = ((a.1.min(b.1) - outer).floor().max(0.0) as usize).min(h);
            let by1 = ((a.1.max(b.1) + outer).ceil().max(0.0) as usize).min(h);
            for y in by0..by1 {
                for x in bx0..bx1 {
                    let d = dist_to_segment((x as f32 + 0.5, y as f32 + 0.5), a, b);
                    if d < outer {
                        let v = &mut m.a[y * w + x];
                        *v = v.max(smooth_step(outer, inner, d));
                    }
                }
            }
        }
    }
    m
}

fn dist_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-12 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}
