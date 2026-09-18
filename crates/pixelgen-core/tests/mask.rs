//! Region selection: the shapes, the keyed selectors, and named regions.

use pixelgen_core::mask::{
    self, Axis, Band, Builder, Chroma, ColorIn, LiveWire, Point, Range, Rect, Registry, Spec, Step,
    Stroke,
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
    assert!(m.at(5, 5) > 0.0, "the matching color was not selected");
    assert_eq!(m.at(0, 0), 0.0, "a distant color was selected");
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

// ---------------------------------------------------------------- fill

fn rect_spec(x: f32, y: f32, w: f32, h: f32) -> Spec {
    Spec { rect: Some(Rect { x, y, w, h }), ..Spec::default() }
}

fn path(d: &str) -> Spec {
    Spec { path: d.into(), ..Spec::default() }
}

/// An edge through the middle of a cell covers half of it, rather than all or
/// nothing as a crossing test at the cell's center would have it.
#[test]
fn a_polygon_edge_partially_covers_the_cells_it_crosses() {
    let base = flat(10, 10, 0.5, 0.5, 0.5);
    let pts = [(0.0, 0.0), (0.55, 0.0), (0.55, 1.0), (0.0, 1.0)];
    let poly = pts.iter().map(|&(x, y)| Point { x, y }).collect();
    let m = build(Spec { polygon: poly, ..Spec::default() }, &base);
    assert_eq!(m.at(2, 5), 1.0);
    assert!((m.at(5, 5) - 0.5).abs() < 1e-4, "edge cell coverage {}", m.at(5, 5));
    assert_eq!(m.at(7, 5), 0.0);
}

/// A freehand lasso has hundreds of points; building one must not cost every
/// edge for every cell.
#[test]
fn a_long_polygon_builds_quickly() {
    let base = flat(400, 400, 0.5, 0.5, 0.5);
    let poly: Vec<Point> = (0..1000)
        .map(|i| {
            let t = i as f32 / 1000.0 * std::f32::consts::TAU;
            let r = 0.4 + 0.02 * (t * 37.0).sin();
            Point { x: 0.5 + r * t.cos(), y: 0.5 + r * t.sin() }
        })
        .collect();
    let start = std::time::Instant::now();
    let m = build(Spec { polygon: poly, ..Spec::default() }, &base);
    let took = start.elapsed();
    assert_eq!(m.at(200, 200), 1.0);
    assert_eq!(m.at(2, 2), 0.0);
    assert!(took.as_millis() < 500, "took {took:?}");
}

// ---------------------------------------------------------------- selectors

#[test]
fn two_selectors_in_one_spec_are_reported() {
    let spec = Spec {
        rect: Some(Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }),
        path: "M0 0 L1 0 L1 1 Z".into(),
        ..Spec::default()
    };
    let err = mask::validate(&Registry::new(), &[Some(&spec)]).expect_err("expected a conflict");
    assert!(matches!(err, mask::Error::MultipleSelectors(..)), "{err}");
}

#[test]
fn steps_apply_in_order() {
    let base = flat(100, 100, 0.5, 0.5, 0.5);
    let spec = Spec {
        steps: vec![
            Step::add(rect_spec(0.0, 0.0, 0.5, 1.0)),
            Step::add(rect_spec(0.5, 0.0, 0.5, 0.5)),
            Step::sub(rect_spec(0.0, 0.0, 0.25, 0.25)),
            Step::and(rect_spec(0.0, 0.0, 1.0, 0.75)),
        ],
        ..Spec::default()
    };
    mask::validate(&Registry::new(), &[Some(&spec)]).unwrap();
    let m = build(spec, &base);
    assert_eq!(m.at(10, 10), 0.0, "subtracted");
    assert_eq!(m.at(40, 40), 1.0, "first add");
    assert_eq!(m.at(75, 25), 1.0, "second add");
    assert_eq!(m.at(75, 60), 0.0, "never added");
    assert_eq!(m.at(40, 90), 0.0, "outside the intersection");
}

#[test]
fn a_first_step_that_is_not_add_is_reported() {
    let spec = Spec { steps: vec![Step::sub(rect_spec(0.0, 0.0, 0.5, 0.5))], ..Spec::default() };
    let err = mask::validate(&Registry::new(), &[Some(&spec)]).expect_err("expected BadStep");
    assert!(matches!(err, mask::Error::BadStep(_)), "{err}");

    let two = Step { add: Some(Spec::default()), sub: Some(Spec::default()), and: None };
    let spec = Spec { steps: vec![two], ..Spec::default() };
    assert!(matches!(
        mask::validate(&Registry::new(), &[Some(&spec)]),
        Err(mask::Error::BadStep(_))
    ));
}

#[test]
fn a_region_cycle_through_a_step_is_reported() {
    let regions: Registry = [(
        "a".to_string(),
        Spec {
            steps: vec![Step::add(Spec { r#ref: "a".into(), ..Spec::default() })],
            ..Spec::default()
        },
    )]
    .into();
    assert!(matches!(mask::validate(&regions, &[]), Err(mask::Error::Cycle(_))));
}

#[test]
fn steps_round_trip_as_plain_keys() {
    let yaml = "steps:\n- add:\n    path: M0 0 L1 0 L1 1 Z\n- sub:\n    wand:\n      x: 0.5\n      y: 0.5\ngrow: -1.0\n";
    let spec: Spec = serde_yaml::from_str(yaml).unwrap();
    let out = serde_yaml::to_string(&spec).unwrap();
    assert!(!out.contains('!'), "an enum tag was written: {out}");
    assert_eq!(out, yaml);
}

// ---------------------------------------------------------------- refine

fn count(m: &mask::Mask) -> f32 {
    m.a.iter().sum()
}

#[test]
fn grow_and_shrink_move_the_edge_by_whole_cells() {
    let base = flat(20, 20, 0.5, 0.5, 0.5);
    let sq = rect_spec(0.25, 0.25, 0.5, 0.5); // cells 5..15
    let grown = build(Spec { grow: 1.0, ..sq.clone() }, &base);
    assert_eq!(grown.at(4, 10), 1.0);
    assert_eq!(grown.at(3, 10), 0.0);
    let shrunk = build(Spec { grow: -1.0, ..sq.clone() }, &base);
    assert_eq!(shrunk.at(5, 10), 0.0);
    assert_eq!(shrunk.at(6, 10), 1.0);

    // A frame-filling mask shrinks away from nothing: the frame is not an edge.
    let full = build(Spec { grow: -2.0, ..rect_spec(0.0, 0.0, 1.0, 1.0) }, &base);
    assert_eq!(full.at(0, 0), 1.0);

    // Growing then shrinking a square gives back about the square.
    let there_and_back = Spec {
        steps: vec![Step::add(Spec { grow: 1.0, ..sq.clone() })],
        grow: -1.0,
        ..Spec::default()
    };
    let back = build(there_and_back, &base);
    let orig = build(sq, &base);
    assert!((count(&back) - count(&orig)).abs() <= 4.0, "{} vs {}", count(&back), count(&orig));
}

#[test]
fn smooth_drops_a_speck_and_keeps_a_block() {
    let base = flat(40, 40, 0.5, 0.5, 0.5);
    let spec = Spec {
        steps: vec![
            Step::add(rect_spec(0.1, 0.1, 0.4, 0.4)),
            Step::add(rect_spec(0.8, 0.8, 0.025, 0.025)), // one cell
        ],
        smooth: 1.0,
        ..Spec::default()
    };
    let m = build(spec, &base);
    assert_eq!(m.at(12, 12), 1.0);
    assert_eq!(m.at(32, 32), 0.0, "the speck survived");
}

// ---------------------------------------------------------------- wand

/// Left third red, right two thirds blue, plus a red block on the far right
/// that the blue separates from the left.
fn islands(w: usize, h: usize) -> Image {
    let mut im = flat(w, h, 0.1, 0.2, 0.8);
    for y in 0..h {
        for x in 0..w {
            let far = x >= w * 3 / 4 && y >= h / 4 && y < h / 2;
            if x < w / 3 || far {
                im.set(x, y, 0.9, 0.1, 0.1);
            }
        }
    }
    im
}

fn wand(x: f32, y: f32) -> mask::Wand {
    mask::Wand { x, y, ..mask::Wand::default() }
}

#[test]
fn a_wand_floods_up_to_a_color_edge() {
    let base = islands(30, 20);
    let m = build(Spec { wand: Some(wand(0.1, 0.5)), ..Spec::default() }, &base);
    assert_eq!(m.at(0, 0), 1.0);
    assert_eq!(m.at(9, 19), 1.0);
    assert_eq!(m.at(10, 10), 0.0, "crossed into the blue");
    assert_eq!(m.at(25, 7), 0.0, "reached the unconnected block");
}

#[test]
fn a_global_wand_reaches_unconnected_cells() {
    let base = islands(30, 20);
    let w = mask::Wand { contiguous: false, ..wand(0.1, 0.5) };
    let m = build(Spec { wand: Some(w), ..Spec::default() }, &base);
    assert_eq!(m.at(0, 0), 1.0);
    assert_eq!(m.at(25, 7), 1.0);
    assert_eq!(m.at(15, 15), 0.0);
}

/// The recorded color is what the wand looks for: when a grid change moves
/// the seed point just off the region, it still finds it close by.
#[test]
fn a_wand_with_a_recorded_color_survives_a_grid_change() {
    let small = islands(30, 20);
    let hex = mask::sample(&small, 0.33, 0.5);
    assert_eq!(hex, mask::sample(&small, 0.0, 0.0), "the seed should be on the red");

    // At double width the seed lands on the blue side of the edge.
    let big = islands(60, 40);
    let w = mask::Wand { hex: hex.clone(), ..wand(0.34, 0.5) };
    let m = build(Spec { wand: Some(w), ..Spec::default() }, &big);
    assert_eq!(m.at(0, 0), 1.0, "the red was not found");
    assert_eq!(m.at(30, 30), 0.0, "the blue was flooded");
}

#[test]
fn a_wand_is_never_empty() {
    let mut base = Image::new(10, 10);
    for y in 0..10 {
        for x in 0..10 {
            let v = (x * 10 + y) as f32 / 100.0;
            base.set(x, y, v, 1.0 - v, (v * 7.0).fract());
        }
    }
    for contiguous in [true, false] {
        let w = mask::Wand { tolerance: 1e-6, contiguous, ..wand(0.55, 0.55) };
        let m = build(Spec { wand: Some(w), ..Spec::default() }, &base);
        assert!(!m.is_empty());
        assert_eq!(m.at(5, 5), 1.0);
    }
}

// ---------------------------------------------------------------- path

#[test]
fn paths_parse_every_command_absolute_and_relative() {
    use pixelgen_core::mask::path::Path;
    for d in [
        "M0 0 L1 0 L1 1 Z",
        "m.1.1 l.5 0 v.5 h-.5 z",
        "M0,0 H1 V1 H0 Z M.2 .2 L.4 .2 .4 .4Z",
        "M0 0 C.3 0 .6 .2 1 1 S.2 .8 0 1 Z",
        "M0 0 Q.5 0 1 1 T0 1 z",
        "M0 0 c.1 .1 .2 .2 .3 .3 s.1 .1 .2 .2 q.1 0 .1 .1 t.1 .1 Z",
        "M1e-1 2E-1 L.5 .5 L.1 .5 Z",
    ] {
        let p = Path::parse(d).unwrap_or_else(|e| panic!("{d}: {e}"));
        assert!(!p.flatten(10.0, 10.0, 0.25).is_empty(), "{d}");
    }
    for d in ["", "L0 0 1 1", "M0", "M0 0 Z 1 1", "M0 0 A1 1 0 0 0 1 1", "0 0"] {
        assert!(Path::parse(d).is_err(), "{d:?} should not parse");
    }
}

#[test]
fn relative_commands_offset_from_the_current_point() {
    let base = flat(100, 100, 0.5, 0.5, 0.5);
    let abs = build(path("M.2 .2 L.6 .2 L.6 .6 L.2 .6 Z"), &base);
    let rel = build(path("m.2 .2 h.4 v.4 h-.4 z"), &base);
    let diff = abs.a.iter().zip(&rel.a).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(diff < 1e-3, "absolute and relative differ by {diff}");
    assert_eq!(abs.bounds(), (20, 20, 60, 60));
}

#[test]
fn a_curved_path_bulges_past_its_chord() {
    let base = flat(100, 100, 0.5, 0.5, 0.5);
    let m = build(path("M.2 .5 C.2 .1 .8 .1 .8 .5 Z"), &base);
    assert_eq!(m.at(50, 35), 1.0, "under the curve");
    assert_eq!(m.at(50, 60), 0.0, "below the chord");
    assert_eq!(m.at(50, 15), 0.0, "above the curve");
}

#[test]
fn a_bad_path_is_reported_at_validate() {
    let spec = path("M0 0 X1 1");
    let err = mask::validate(&Registry::new(), &[Some(&spec)]).expect_err("expected BadPath");
    assert!(matches!(err, mask::Error::BadPath(_)), "{err}");

    let spec = Spec {
        steps: vec![Step::add(Spec {
            stroke: Some(Stroke { d: "nope".into(), ..Stroke::default() }),
            ..Spec::default()
        })],
        ..Spec::default()
    };
    assert!(matches!(
        mask::validate(&Registry::new(), &[Some(&spec)]),
        Err(mask::Error::BadPath(_))
    ));
}

#[test]
fn a_stroke_scales_with_the_width() {
    let stroke = |w: usize| {
        let base = flat(w, w, 0.5, 0.5, 0.5);
        let s = Stroke { d: "M.2 .5 L.8 .5".into(), radius: 0.05, hardness: 1.0 };
        build(Spec { stroke: Some(s), ..Spec::default() }, &base)
    };
    let (a, b) = (stroke(100), stroke(200));
    assert_eq!(a.at(50, 50), 1.0);
    assert_eq!(a.at(50, 58), 0.0);
    assert_eq!(b.at(100, 108), 1.0, "the radius should double with the width");
    assert_eq!(b.at(100, 116), 0.0);
    let ratio = count(&b) / count(&a);
    assert!((ratio - 4.0).abs() < 0.2, "area ratio {ratio}");
}

// ---------------------------------------------------------------- live-wire

#[test]
fn the_live_wire_follows_a_color_boundary() {
    // Red above row 10, blue below: the boundary is the corner row y = 10.
    let mut base = flat(40, 20, 0.1, 0.2, 0.8);
    for y in 0..10 {
        for x in 0..40 {
            base.set(x, y, 0.9, 0.1, 0.1);
        }
    }
    // Both ends a few cells off the boundary; the path should dive to it,
    // run along it, and come back, rather than cut straight across.
    let lw = LiveWire::new(&base, 5.0 / 40.0, 7.0 / 20.0);
    let pts = lw.path_to(35.0 / 40.0, 13.0 / 20.0);
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    assert_eq!((first.0 * 40.0, first.1 * 20.0), (5.0, 7.0));
    assert_eq!((last.0 * 40.0, last.1 * 20.0), (35.0, 13.0));
    let on_edge: f32 = pts
        .windows(2)
        .filter(|p| p[0].1 * 20.0 == 10.0 && p[1].1 * 20.0 == 10.0)
        .map(|p| (p[1].0 - p[0].0).abs() * 40.0)
        .sum();
    assert!(on_edge >= 25.0, "only {on_edge} cells along the boundary: {pts:?}");
}
