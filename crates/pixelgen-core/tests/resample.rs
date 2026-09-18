//! The two resampling steps that give the style its hard cell edges.

use pixelgen_core::pixel::Image;
use pixelgen_core::resample::{downscale, fit_size, median3, upscale};

fn flat(w: usize, h: usize) -> Image {
    Image::new(w, h)
}

#[test]
fn downscale_averages_the_source_cells() {
    // Left half black, right half white: each 2x2 destination cell covers one
    // half exactly, so the result must be the two originals, not a blend.
    let mut src = flat(4, 4);
    for y in 0..4 {
        for x in 0..4 {
            let v = if x < 2 { 0.0 } else { 1.0 };
            src.set(x, y, v, v, v);
        }
    }
    let dst = downscale(&src, 2, 2);
    assert_eq!(dst.get(0, 0).0, 0.0);
    assert_eq!(dst.get(1, 0).0, 1.0);
}

#[test]
fn downscale_mixes_when_cells_straddle() {
    let mut src = flat(2, 1);
    src.set(0, 0, 0.0, 0.0, 0.0);
    src.set(1, 0, 1.0, 1.0, 1.0);
    assert_eq!(downscale(&src, 1, 1).get(0, 0).0, 0.5);
}

#[test]
fn upscale_replicates_exactly() {
    let mut src = flat(2, 1);
    src.set(0, 0, 1.0, 0.0, 0.0);
    src.set(1, 0, 0.0, 0.0, 1.0);
    let dst = upscale(&src, 3);
    assert_eq!((dst.w, dst.h), (6, 3));
    for y in 0..3 {
        for x in 0..3 {
            assert_eq!(dst.get(x, y).0, 1.0, "({x},{y}) should replicate the red pixel");
        }
        for x in 3..6 {
            assert_eq!(dst.get(x, y).2, 1.0, "({x},{y}) should replicate the blue pixel");
        }
    }
}

#[test]
fn fit_size_preserves_aspect_and_never_collapses() {
    assert_eq!(fit_size(1000, 500, 320), (320, 160));
    assert!(fit_size(100, 3, 8).1 >= 1, "height must stay at least 1");
}

#[test]
fn median3_removes_isolated_speckles() {
    let mut src = flat(5, 5);
    for y in 0..5 {
        for x in 0..5 {
            src.set(x, y, 0.5, 0.5, 0.5);
        }
    }
    src.set(2, 2, 1.0, 1.0, 1.0); // a single hot pixel
    assert_eq!(median3(&src).get(2, 2).0, 0.5, "the speckle survived");
}

#[test]
fn blend_clamps_alpha_and_ignores_out_of_bounds() {
    let mut im = flat(2, 2);
    im.blend(0, 0, 1.0, 1.0, 1.0, 0.5);
    assert_eq!(im.get(0, 0).0, 0.5);
    im.blend(0, 0, 1.0, 1.0, 1.0, 5.0); // alpha above 1 must clamp, not overshoot
    assert_eq!(im.get(0, 0).0, 1.0);
    im.blend(-1, 0, 1.0, 1.0, 1.0, 1.0); // must not panic
    im.blend(0, 99, 1.0, 1.0, 1.0, 1.0);
}

#[test]
fn sample_wrap_wraps_horizontally_and_clamps_vertically() {
    let mut im = flat(3, 2);
    im.set(0, 0, 1.0, 0.0, 0.0);
    im.set(2, 1, 0.0, 0.0, 1.0);
    assert_eq!(im.sample_wrap(3, 0).0, 1.0, "x=3 should wrap to x=0");
    assert_eq!(im.sample_wrap(-3, 0).0, 1.0, "x=-3 should wrap to x=0");
    assert_eq!(im.sample_wrap(2, 99).2, 1.0, "y past the bottom should clamp");
}
