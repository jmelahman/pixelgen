//! The magnetic lasso's path finder ("live-wire").
//!
//! Shortest paths over the corners between cells, where walking along an edge
//! between two cells is cheap if their colors differ and dear if they match.
//! The cheapest route from the last anchor to the pointer therefore follows
//! whatever boundary runs between them.
//!
//! Corners rather than cell centers: the path then runs exactly along cell
//! boundaries, which is where a pixel-art region's edge actually is, and the
//! polygon it closes into needs no antialiasing to sit on it.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::palette::dist2;
use crate::pixel::{Image, Rgb};

/// How far from the anchor, in cells, paths are searched. A lasso places a new
/// anchor long before the pointer gets this far.
pub const WINDOW: usize = 80;

/// One anchor's shortest-path tree. Built once when the anchor is placed, so
/// following the pointer afterwards is a walk back through `prev`.
pub struct LiveWire {
    /// Grid size in cells; corners are one more in each direction.
    w: usize,
    h: usize,
    /// The window, in corner coordinates, inclusive.
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    seed: usize,
    prev: Vec<u32>,
}

#[derive(PartialEq)]
struct Node(f32, u32);

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        // Reversed, for a min-heap.
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl LiveWire {
    /// Anchors at the corner nearest the normalized point `(x, y)`.
    pub fn new(base: &Image, x: f32, y: f32) -> LiveWire {
        let (w, h) = (base.w, base.h);
        let cw = w + 1;
        let (sx, sy) = corner(x, y, w, h);
        let x0 = sx.saturating_sub(WINDOW);
        let y0 = sy.saturating_sub(WINDOW);
        let x1 = (sx + WINDOW).min(w);
        let y1 = (sy + WINDOW).min(h);
        let seed = sy * cw + sx;

        let color = |x: i64, y: i64| -> Option<Rgb> {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                return None;
            }
            let (r, g, b) = base.get(x as usize, y as usize);
            Some(Rgb { r, g, b })
        };
        // The edge between two cells, as a cost to walk along. The frame's
        // own edge counts as a strong one: an outline that reaches it should
        // be able to run along it.
        let cost = |a: Option<Rgb>, b: Option<Rgb>| -> f32 {
            let d = match (a, b) {
                (Some(a), Some(b)) => dist2(a, b).sqrt() / 3.0,
                _ => 1.0,
            };
            0.05 + 1.0 / (1.0 + 40.0 * d)
        };

        let n = cw * (h + 1);
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        dist[seed] = 0.0;
        heap.push(Node(0.0, seed as u32));
        while let Some(Node(d, i)) = heap.pop() {
            let i = i as usize;
            if d > dist[i] {
                continue;
            }
            let (cx, cy) = (i % cw, i / cw);
            let (ix, iy) = (cx as i64, cy as i64);
            // Moving along a horizontal edge from corner (cx, cy) to (cx+1, cy)
            // runs between cell (cx, cy-1) above and (cx, cy) below; a
            // vertical one between (cx-1, cy) and (cx, cy).
            let mut go = |nx: usize, ny: usize, c: f32| {
                if nx < x0 || nx > x1 || ny < y0 || ny > y1 {
                    return;
                }
                let j = ny * cw + nx;
                let nd = d + c;
                if nd < dist[j] {
                    dist[j] = nd;
                    prev[j] = i as u32;
                    heap.push(Node(nd, j as u32));
                }
            };
            if cx < w {
                go(cx + 1, cy, cost(color(ix, iy - 1), color(ix, iy)));
            }
            if cx > 0 {
                go(cx - 1, cy, cost(color(ix - 1, iy - 1), color(ix - 1, iy)));
            }
            if cy < h {
                go(cx, cy + 1, cost(color(ix - 1, iy), color(ix, iy)));
            }
            if cy > 0 {
                go(cx, cy - 1, cost(color(ix - 1, iy - 1), color(ix, iy - 1)));
            }
        }
        LiveWire { w, h, x0, y0, x1, y1, seed, prev }
    }

    /// The path from the anchor to the corner nearest `(x, y)`, clamped into
    /// the search window, as normalized points with straight runs merged.
    pub fn path_to(&self, x: f32, y: f32) -> Vec<(f32, f32)> {
        let cw = self.w + 1;
        let (tx, ty) = corner(x, y, self.w, self.h);
        let (tx, ty) = (tx.clamp(self.x0, self.x1), ty.clamp(self.y0, self.y1));
        let mut i = ty * cw + tx;
        let mut idx = vec![i];
        while i != self.seed {
            let p = self.prev[i];
            if p == u32::MAX {
                break;
            }
            i = p as usize;
            idx.push(i);
        }
        idx.reverse();

        let pts: Vec<(usize, usize)> = idx.iter().map(|&i| (i % cw, i / cw)).collect();
        let mut out: Vec<(usize, usize)> = Vec::with_capacity(pts.len());
        for &p in &pts {
            // Drop the middle of any three in a line.
            if out.len() >= 2 {
                let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
                if (a.0 == b.0 && b.0 == p.0) || (a.1 == b.1 && b.1 == p.1) {
                    out.pop();
                }
            }
            out.push(p);
        }
        out.into_iter().map(|(x, y)| (x as f32 / self.w as f32, y as f32 / self.h as f32)).collect()
    }
}

fn corner(x: f32, y: f32, w: usize, h: usize) -> (usize, usize) {
    let cx = (x * w as f32).round().clamp(0.0, w as f32) as usize;
    let cy = (y * h as f32).round().clamp(0.0, h as f32) as usize;
    (cx, cy)
}
