//! Region selection: the shapes, the keyed selectors, and named regions.

use pixelgen_core::mask::{
    self, Axis, Band, Builder, Chroma, ColorIn, Point, Range, Rect, Registry, Spec,
};
use pixelgen_core::pixel::Image;

fn flat(w: usize, h: usize, r: f32, g: f32, b: f32) -> Image {
    let mut im = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            im.set(x, y, r, g, b);
        }
    }
    im
}

fn build(spec: Spec, base: &Image) -> mask::Mask {
    Builder::plain(base).build(Some(&spec)).expect("build")
}

#[test]
fn rect_covers_the_normalized_region() {
    let base = flat(100, 100, 0.5, 0.5, 0.5);
    let m = build(
        Spec { rect: Some(Rect { x: 0.25, y: 0.5, w: 0.5, h: 0.25 }), ..Spec::default() },
        &base,
    );
    assert_eq!(m.at(50, 60), 1.0, "inside the rect");
    assert_eq!(m.at(10, 60), 0.0, "left of the rect");
    assert_eq!(m.at(50, 20), 0.0, "above the rect");
    assert_eq!(m.bounds(), (25, 50, 75, 75));
}

#[test]
fn all_intersects_and_any_unions() {
    let base = flat(100, 100, 0.5, 0.5, 0.5);
    let left = Spec { rect: Some(Rect { x: 0.0, y: 0.0, w: 0.6, h: 1.0 }), ..Spec::default() };
    let top = Spec { rect: Some(Rect { x: 0.0, y: 0.0, w: 1.0, h: 0.6 }), ..Spec::default() };

    let and = build(Spec { all: vec![left.clone(), top.clone()], ..Spec::default() }, &base);
    assert_eq!((and.at(30, 30), and.at(80, 30), and.at(30, 80)), (1.0, 0.0, 0.0));

    let or = build(Spec { any: vec![left, top], ..Spec::default() }, &base);
    assert_eq!((or.at(30, 30), or.at(80, 30), or.at(30, 80), or.at(80, 80)), (1.0, 1.0, 1.0, 0.0));
}

#[test]
fn invert_flips_coverage_and_gain_scales_it() {
    let base = flat(10, 10, 0.5, 0.5, 0.5);
    let full = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let m =
        build(Spec { rect: Some(Rect { w: 0.5, ..full }), invert: true, ..Spec::default() }, &base);
    assert_eq!((m.at(1, 1), m.at(8, 1)), (0.0, 1.0));

    let g = build(Spec { rect: Some(full), gain: 0.25, ..Spec::default() }, &base);
    assert_eq!(g.at(5, 5), 0.25);
}

#[test]
fn luma_selects_bright_pixels() {
    let mut base = Image::new(10, 10);
    for y in 0..10 {
        for x in 0..10 {
            let v = y as f32 / 9.0;
            base.set(x, y, v, v, v);
        }
    }
    let m = build(Spec { luma: Some(Range { min: 0.8, max: 1.0 }), ..Spec::default() }, &base);
    assert_eq!(m.at(5, 0), 0.0, "dark row was selected");
    assert_eq!(m.at(5, 9), 1.0, "brightest row was not selected");
}

#[test]
fn color_matches_nearby_colours_only() {
    let mut base = flat(10, 10, 0.9, 0.2, 0.2);
    base.set(0, 0, 0.1, 0.1, 0.9);
    let m = build(
        Spec { color: Some(ColorIn { hex: "#e63333".into(), tolerance: 0.2 }), ..Spec::default() },
        &base,
    );
    assert!(m.at(5, 5) > 0.0, "the matching colour was not selected");
    assert_eq!(m.at(0, 0), 0.0, "a distant colour was selected");
}

#[test]
fn band_ramps_across_its_axis_in_either_direction() {
    let base = flat(10, 100, 0.5, 0.5, 0.5);
    let m = build(
        Spec { band: Some(Band { axis: Axis::Y, start: 0.2, end: 0.8 }), ..Spec::default() },
        &base,
    );
    assert_eq!(m.at(5, 5), 0.0, "before the band start");
    assert_eq!(m.at(5, 95), 1.0, "after the band end");
    let mid = m.at(5, 50);
    assert!(mid > 0.2 && mid < 0.8, "mid-band coverage = {mid}, expected an intermediate value");

    let rev = build(
        Spec { band: Some(Band { axis: Axis::Y, start: 0.8, end: 0.2 }), ..Spec::default() },
        &base,
    );
    assert_eq!((rev.at(5, 5), rev.at(5, 95)), (1.0, 0.0), "a reversed band should ramp downwards");
}

/// An omitted `end` used to mean 1.0 only because zero was read as "unset";
/// now it is a real default, so a band with nothing but an axis still covers.
#[test]
fn band_defaults_to_a_full_ramp() {
    let base = flat(10, 100, 0.5, 0.5, 0.5);
    let m = build(Spec { band: Some(Band::default()), ..Spec::default() }, &base);
    assert!(m.at(5, 99) > 0.99);
    assert!(m.at(5, 0) < 0.01);
}

#[test]
fn polygon_selects_its_interior() {
    let base = flat(100, 100, 0.5, 0.5, 0.5);
    let tri = vec![Point { x: 0.5, y: 0.0 }, Point { x: 1.0, y: 1.0 }, Point { x: 0.0, y: 1.0 }];
    let m = build(Spec { polygon: tri, ..Spec::default() }, &base);
    assert_eq!(m.at(50, 80), 1.0, "inside the triangle");
    assert_eq!(m.at(5, 5), 0.0, "outside the triangle");
}

#[test]
fn an_empty_mask_is_detected() {
    let base = flat(10, 10, 0.5, 0.5, 0.5);
    let m = build(Spec { luma: Some(Range { min: 0.99, max: 1.0 }), ..Spec::default() }, &base);
    assert!(m.is_empty());
}

// ---------------------------------------------------------------- chroma

/// An image whose left half is warm and right half is cool, at two very
/// different brightnesses per side. This is the case the normalization in
/// `chroma_of` exists for: the dark cool half must still read as cool.
fn split(w: usize, h: usize) -> Image {
    let mut im = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let dim = if y >= h / 2 { 0.25 } else { 1.0 };
            if x < w / 2 {
                im.set(x, y, 0.75 * dim, 0.55 * dim, 0.30 * dim); // warm
            } else {
                im.set(x, y, 0.35 * dim, 0.50 * dim, 0.75 * dim); // cool
            }
        }
    }
    im
}

#[test]
fn chroma_selects_cool_regardless_of_brightness() {
    let base = split(100, 100);
    let m = build(
        Spec { chroma: Some(Chroma { min: Some(0.2), ..Chroma::default() }), ..Spec::default() },
        &base,
    );
    for y in [25, 75] {
        // bright row, dark row
        assert!(m.at(75, y) > 0.99, "cool side at y={y}: coverage {}", m.at(75, y));
        assert!(m.at(25, y) < 0.01, "warm side at y={y}: coverage {}", m.at(25, y));
    }
}

#[test]
fn chroma_max_selects_warm() {
    let base = split(100, 100);
    let m = build(
        Spec { chroma: Some(Chroma { max: Some(0.0), ..Chroma::default() }), ..Spec::default() },
        &base,
    );
    assert!(m.at(25, 25) > 0.99, "warm side: coverage {}", m.at(25, 25));
    assert!(m.at(75, 25) < 0.01, "cool side: coverage {}", m.at(75, 25));
}

/// A zero bound must be honoured rather than read as "unset", because zero is
/// the natural neutral point of the scale and so the most likely bound to
/// write. `Option` makes the distinction explicit instead of conventional.
#[test]
fn chroma_zero_bound_is_not_treated_as_unset() {
    let base = split(100, 100);
    let bounded = build(
        Spec { chroma: Some(Chroma { min: Some(0.0), ..Chroma::default() }), ..Spec::default() },
        &base,
    );
    assert!(bounded.at(25, 25) < 0.01, "warm side with min=0: {}", bounded.at(25, 25));

    let unbounded = build(Spec { chroma: Some(Chroma::default()), ..Spec::default() }, &base);
    assert_eq!(unbounded.at(25, 25), 1.0, "with no bounds everything should be selected");
}

// ---------------------------------------------------------------- regions

fn region_of(rect: Rect) -> Spec {
    Spec { rect: Some(rect), ..Spec::default() }
}

#[test]
fn ref_resolves_a_region_and_modifiers_apply_to_the_copy() {
    let base = flat(100, 100, 0.5, 0.5, 0.5);
    let regions: Registry =
        [("left".to_string(), region_of(Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }))].into();
    let mut b = Builder::new(&base, &regions);

    let m = b.build(Some(&Spec { r#ref: "left".into(), ..Spec::default() })).unwrap();
    assert_eq!((m.at(25, 50), m.at(75, 50)), (1.0, 0.0));

    let inv =
        b.build(Some(&Spec { r#ref: "left".into(), invert: true, ..Spec::default() })).unwrap();
    assert_eq!((inv.at(25, 50), inv.at(75, 50)), (0.0, 1.0));

    // The previous use inverted its own copy; the cached region must not have
    // been inverted along with it.
    let again = b.build(Some(&Spec { r#ref: "left".into(), ..Spec::default() })).unwrap();
    assert_eq!((again.at(25, 50), again.at(75, 50)), (1.0, 0.0), "the cache was mutated");
}

#[test]
fn an_undefined_region_is_reported() {
    let base = flat(10, 10, 0.5, 0.5, 0.5);
    let empty = Registry::new();
    let spec = Spec { r#ref: "nope".into(), ..Spec::default() };
    assert!(Builder::new(&base, &empty).build(Some(&spec)).is_err());
    assert!(mask::validate(&empty, &[Some(&spec)]).is_err());
}

#[test]
fn a_region_cycle_is_reported_rather_than_recursed_into() {
    let regions: Registry = [
        (
            "a".to_string(),
            Spec { all: vec![Spec { r#ref: "b".into(), ..Spec::default() }], ..Spec::default() },
        ),
        ("b".to_string(), Spec { r#ref: "a".into(), ..Spec::default() }),
    ]
    .into();
    assert!(mask::validate(&regions, &[]).is_err());

    let base = flat(10, 10, 0.5, 0.5, 0.5);
    let spec = Spec { r#ref: "a".into(), ..Spec::default() };
    assert!(Builder::new(&base, &regions).build(Some(&spec)).is_err());
}

/// `ref` replaces the selector rather than combining with one, so writing both
/// is a misunderstanding worth naming instead of silently resolving.
#[test]
fn a_ref_combined_with_a_selector_is_reported() {
    let regions: Registry =
        [("a".to_string(), region_of(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }))].into();
    let spec = Spec {
        r#ref: "a".into(),
        rect: Some(Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }),
        ..Spec::default()
    };
    let err = mask::validate(&regions, &[Some(&spec)]).expect_err("expected a conflict");
    assert!(err.to_string().contains("rect"), "{err}");
}
