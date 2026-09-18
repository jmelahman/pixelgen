//! The renderer's invariants, checked against every registered effect.

use pixelgen_core::effect;
use pixelgen_core::pixel::Image;
use pixelgen_core::render;
use pixelgen_core::scene::{Layer, Loop, Palette, Scene};
use serde_yaml::Value;

/// A synthetic source with a bright warm patch, a dark region and a gradient,
/// so that luma- and color-keyed masks all select something.
fn test_image(w: usize, h: usize) -> Image {
    let mut im = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut r = x as f32 / w as f32;
            let mut g = y as f32 / h as f32;
            let mut b = 0.5 - 0.3 * y as f32 / h as f32;
            if x > w / 2 && y > h / 2 {
                // A highlight, for twinkle and glow.
                (r, g, b) = (0.98, 0.9, 0.6);
            }
            im.set(x, y, r, g, b);
        }
    }
    im
}

/// Gives effects whose defaults are static something to animate, so the wrap
/// check below is not trivially satisfied by an effect that never moves.
fn params_for(name: &str) -> Value {
    let src = match name {
        "drift" => "{speed_x: 1, speed_y: 0}",
        "palette_cycle" => "{start: 0, count: 5, rotations: 2}",
        "mist" => "{dir_x: 1, period: 4}",
        "shimmer" => "{amplitude: 2, speed: 1}",
        "sway" => "{amplitude: 3, speed: 1}",
        "twinkle" => "{threshold: 0.4, amount: 0.8}",
        "glow" => "{threshold: 0.4, pulse: 0.5, speed: 1}",
        // Subtle by design, so drive them hard enough to survive quantization
        // onto the small palette these tests use.
        "breathe" => "{amount: 0.35}",
        "flicker" => "{amount: 0.4}",
        _ => return Value::Null,
    };
    serde_yaml::from_str(src).expect("bad test params")
}

/// The layers that are deliberately the same on every frame: they exist to
/// shape the still image, not to animate it.
fn is_static(name: &str) -> bool {
    matches!(name, "vignette" | "scanlines")
}

fn diff(a: &Image, b: &Image) -> usize {
    a.pix.iter().zip(&b.pix).filter(|(x, y)| (*x - *y).abs() > 1e-6).count()
}

fn scene_with(layers: Vec<Layer>, width: usize, colors: usize, fps: usize) -> Scene {
    Scene {
        width,
        seed: 7,
        palette: Palette { colors, ..Palette::default() },
        loop_: Loop { seconds: 1.0, fps },
        layers,
        ..Scene::default()
    }
}

fn layer(name: &str) -> Layer {
    Layer { r#type: name.to_string(), params: params_for(name), ..Layer::default() }
}

/// The central invariant of the renderer: phase 1 must reproduce phase 0
/// exactly. Anything else shows up as a visible jump every time the wallpaper
/// restarts, which is the one artifact a looping background cannot have.
#[test]
fn loop_closes_for_every_effect() {
    for name in effect::names() {
        let s = scene_with(vec![layer(name)], 64, 16, 12);
        let p = render::prepare(&test_image(128, 96), &s)
            .unwrap_or_else(|e| panic!("prepare {name}: {e}"));

        let first = p.frame(0);
        // Frame index `frames` is phase 1.0, the frame that would follow the
        // last one; it must be identical to frame 0.
        let wrapped = p.frame(p.frames);
        let d = diff(&first, &wrapped);
        assert_eq!(d, 0, "effect {name:?} does not close its loop: {d} pixels differ at the wrap");

        // Guard against the check passing for the wrong reason: an effect that
        // drew nothing at all would also wrap perfectly.
        if !is_static(name) {
            let moved = (1..p.frames).any(|i| diff(&first, &p.frame(i)) > 0);
            assert!(moved, "effect {name:?} produced an identical image on every frame");
        }
    }
}

/// Frames are rendered in parallel, so an effect that leaked state between
/// frames would produce different output run to run.
#[test]
fn render_is_deterministic() {
    let layers = effect::names().into_iter().map(layer).collect();
    let s = scene_with(layers, 48, 12, 10);
    let src = test_image(96, 72);

    let a = render::prepare(&src, &s).unwrap().all_frames(|_, _| {});
    let b = render::prepare(&src, &s).unwrap().all_frames(|_, _| {});
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        assert_eq!(diff(x, y), 0, "frame {i} differs between runs");
    }
}

/// Effects blend in continuous color, so the final snap is the only thing
/// keeping a frame inside the palette. If it ever stops running, the encoder
/// silently starts producing colors the still image never had.
#[test]
fn frames_are_all_in_palette() {
    let layers = ["rain", "mist", "vignette"].into_iter().map(layer).collect();
    let s = scene_with(layers, 48, 8, 6);
    let p = render::prepare(&test_image(96, 72), &s).unwrap();
    let pal = p.matcher.palette.clone();

    for (fi, f) in p.all_frames(|_, _| {}).iter().enumerate() {
        for y in 0..f.h {
            for x in 0..f.w {
                let (r, g, b) = f.get(x, y);
                assert!(
                    pal.iter().any(|c| c.r == r && c.g == g && c.b == b),
                    "frame {fi} pixel ({x},{y}) is off-palette: {r} {g} {b}"
                );
            }
        }
    }
}

#[test]
fn unknown_effect_names_the_layer_and_lists_what_is_known() {
    let l = Layer { name: "oops".into(), r#type: "no_such_effect".into(), ..Layer::default() };
    let msg = match render::prepare(&test_image(32, 32), &scene_with(vec![l], 32, 8, 6)) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected an error for an unknown effect type"),
    };
    assert!(msg.contains("oops"), "{msg}");
    assert!(msg.contains("rain"), "the message should list the known effects: {msg}");
}

/// Go clamped these counts to 1 in silence, so a scene asking for half a
/// rotation ran at a speed nothing in the file explained.
#[test]
fn a_count_that_cannot_close_the_loop_is_reported() {
    let l = Layer {
        r#type: "breathe".into(),
        params: serde_yaml::from_str("{speed: 0}").unwrap(),
        ..Layer::default()
    };
    let msg = match render::prepare(&test_image(32, 32), &scene_with(vec![l], 32, 8, 6)) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected an error for speed: 0"),
    };
    assert!(msg.contains("speed"), "{msg}");
}
