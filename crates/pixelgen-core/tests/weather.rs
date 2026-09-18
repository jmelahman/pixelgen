//! Weather sizes follow the grid width, so a scene looks the same at any
//! `width` instead of its rain and fog shrinking as the grid grows.

use pixelgen_core::effect::{self, Context};
use pixelgen_core::mask::Mask;
use pixelgen_core::palette::Matcher;
use pixelgen_core::pixel::{Image, Rgb};

/// Renders one frame of an effect over black, on a full mask.
fn frame(name: &str, grid_w: usize, grid_h: usize, t: f32, params: &str) -> Image {
    seeded(name, grid_w, grid_h, t, 1, params)
}

fn seeded(name: &str, grid_w: usize, grid_h: usize, t: f32, seed: u32, params: &str) -> Image {
    let fx = effect::build(name, &serde_yaml::from_str(params).unwrap()).unwrap();
    let mut dst = Image::new(grid_w, grid_h);
    let base = dst.clone();
    let mask = Mask::full(grid_w, grid_h);
    let matcher = Matcher::new(vec![Rgb { r: 0.0, g: 0.0, b: 0.0 }]);
    let ctx = Context { t, frame: 0, frames: 1, base: &base, mask: &mask, matcher: &matcher, seed };
    fx.render(&mut dst, &ctx);
    dst
}

/// Draws a single vertical streak and returns how many cells wide its widest
/// row is and how many rows it covers.
fn streak(grid_w: usize, params: &str) -> (usize, usize) {
    let params = format!("{{count: 1, layers: 1, slant: 0, opacity: 1, {params}}}");
    // With seed 1 this phase puts the one drop whole and on screen at every
    // grid size the tests use.
    let dst = frame("rain", grid_w, grid_w, 0.5, &params);
    let mut widest = 0;
    let mut rows = 0;
    for y in 0..dst.h {
        let lit = (0..dst.w).filter(|&x| dst.get(x, y).0 > 0.0).count();
        widest = widest.max(lit);
        rows += (lit > 0) as usize;
    }
    (widest, rows)
}

#[test]
fn thickness_thickens_the_streak() {
    assert_eq!(streak(320, "thickness: 1").0, 1);
    assert_eq!(streak(320, "thickness: 3").0, 3);
    // The fractional part is a fainter extra cell, not rounded away.
    assert_eq!(streak(320, "thickness: 1.5").0, 2);
}

#[test]
fn streaks_scale_with_the_grid() {
    let (w1, l1) = streak(320, "thickness: 1, length: 10");
    let (w2, l2) = streak(640, "thickness: 1, length: 10");
    assert_eq!((w1, w2), (1, 2));
    assert!(l2.abs_diff(2 * l1) <= 1, "320 grid: {l1} rows, 640 grid: {l2} rows");
}

/// Scaled down past a cell, a streak would taper away to nothing; a narrow
/// grid still has to show its rain.
#[test]
fn narrow_grids_still_draw_rain() {
    for w in [8, 32, 80] {
        let (thick, rows) = streak(w, "thickness: 1");
        assert_eq!(thick, 1, "{w}-wide grid");
        assert!(rows >= 1, "{w}-wide grid drew no rain");
    }
}

#[test]
fn a_huge_thickness_is_capped_at_the_grid() {
    assert_eq!(streak(64, "thickness: 1e12").0, 64);
}

#[test]
fn a_size_that_cannot_be_drawn_is_reported() {
    for param in ["length", "thickness"] {
        for bad in ["0", "-5", ".nan", ".inf"] {
            let params = serde_yaml::from_str(&format!("{{{param}: {bad}}}")).unwrap();
            let msg = match effect::build("rain", &params) {
                Err(e) => e.to_string(),
                Ok(_) => panic!("{param} {bad} was accepted"),
            };
            assert!(msg.contains(param), "{msg}");
        }
    }
}

/// The grid can shrink a size down to the floor, but a size written below it
/// is the scene's own choice and is kept.
#[test]
fn a_thin_thickness_draws_a_fainter_cell() {
    let bright = |p: &str| {
        let dst = frame("rain", 320, 320, 0.5, &format!("{{count: 1, layers: 1, slant: 0, {p}}}"));
        dst.pix.iter().cloned().fold(0.0f32, f32::max)
    };
    let (thin, full) = (bright("thickness: 0.3"), bright("thickness: 1"));
    assert!(thin > 0.0 && thin < full, "0.3 drew {thin}, 1 drew {full}");
    assert_eq!(streak(320, "thickness: 0.3").0, 1);
}

/// Neither is bounded by the scene, so both have to stop at the frame rather
/// than hang the renderer.
#[test]
fn a_huge_length_stops_at_the_frame() {
    let (_, rows) = streak(64, "length: 1e30");
    assert!(rows <= 64, "{rows}");
}

/// The drop's column saturates at the integer limit here, where widening it
/// used to overflow.
#[test]
fn a_huge_slant_does_not_overflow() {
    frame("rain", 64, 64, 0.5, "{slant: 1e10, thickness: 2}");
}

#[test]
fn an_empty_frame_draws_nothing() {
    for (w, h) in [(0, 0), (0, 16), (16, 0)] {
        frame("rain", w, h, 0.5, "{thickness: 3}");
    }
}

/// A thick streak near an edge is pushed inside rather than wrapped, so no
/// row shows it on both sides of the frame.
#[test]
fn a_thick_streak_never_splits_across_the_frame() {
    for seed in 0..200 {
        let dst =
            seeded("rain", 24, 24, 0.5, seed, "{count: 1, layers: 1, slant: 0, thickness: 5}");
        for y in 0..dst.h {
            let lit: Vec<usize> = (0..dst.w).filter(|&x| dst.get(x, y).0 > 0.0).collect();
            if let (Some(a), Some(b)) = (lit.first(), lit.last()) {
                assert_eq!(b - a + 1, lit.len(), "seed {seed} row {y} is split: {lit:?}");
            }
        }
    }
}

/// A drop has to leave the frame a row at a time before it wraps back to the
/// top; one longer than average used to still show its top and vanish.
#[test]
fn a_drop_is_gone_before_it_wraps() {
    let params = serde_yaml::from_str("{count: 1, layers: 1, speed: 2, length: 50}").unwrap();
    let fx = effect::build("rain", &params).unwrap();
    let base = Image::new(64, 64);
    let mask = Mask::full(64, 64);
    let matcher = Matcher::new(vec![Rgb { r: 0.0, g: 0.0, b: 0.0 }]);
    // Long enough that a popping drop loses several rows at once. A step
    // moves a drop about a fifth of a row, so between neighbouring steps a
    // streak that leaves properly gains or loses at most one.
    let steps = 800;
    for seed in 0..16 {
        let mut last = None;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let mut dst = base.clone();
            let ctx = Context {
                t,
                frame: 0,
                frames: 1,
                base: &base,
                mask: &mask,
                matcher: &matcher,
                seed,
            };
            fx.render(&mut dst, &ctx);
            let rows = (0..dst.h).filter(|&y| (0..dst.w).any(|x| dst.get(x, y).0 > 0.0)).count();
            if let Some(last) = last {
                assert!(
                    rows.abs_diff(last) <= 1,
                    "seed {seed} at t {t}: the drop jumped from {last} rows to {rows}"
                );
            }
            last = Some(rows);
        }
    }
}

/// Doubling the grid samples the same noise field twice as finely, so every
/// cell of the small frame reappears at the matching cell of the large one.
#[test]
fn mist_keeps_its_shape_across_grid_sizes() {
    let small = frame("mist", 160, 90, 0.3, "{}");
    let large = frame("mist", 320, 180, 0.3, "{}");
    assert!(large.pix.iter().any(|&v| v > 0.0), "no mist was drawn");
    for y in 0..small.h {
        for x in 0..small.w {
            assert_eq!(small.get(x, y), large.get(2 * x, 2 * y), "cell ({x},{y})");
        }
    }
}

/// Steam also fades with height through its region, which lands on slightly
/// different rows at each size, so it only has to match closely.
#[test]
fn steam_keeps_its_shape_across_grid_sizes() {
    let small = frame("steam", 160, 90, 0.3, "{}");
    let large = frame("steam", 320, 180, 0.3, "{}");
    assert!(large.pix.iter().any(|&v| v > 0.0), "no steam was drawn");
    let mut worst = 0.0f32;
    for y in 0..small.h {
        for x in 0..small.w {
            worst = worst.max((small.get(x, y).0 - large.get(2 * x, 2 * y).0).abs());
        }
    }
    assert!(worst < 0.05, "steam differs by {worst} between grid sizes");
}
