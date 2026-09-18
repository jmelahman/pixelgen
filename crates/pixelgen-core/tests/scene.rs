//! Loading, defaults and validation of the scene file.

use pixelgen_core::scene::{Loop, Scene};

#[test]
fn parsing_applies_defaults() {
    let s = Scene::parse("name: tiny\nlayers:\n  - type: rain\n").unwrap();
    assert_eq!(s.width, 320);
    assert_eq!(s.loop_.fps, 20);
    assert_eq!(s.palette.colors, 32);
    assert_eq!(s.loop_.frames(), 120);
}

/// A mistyped key that is silently ignored shows up much later as an effect
/// that mysteriously does nothing, so loading must reject it.
#[test]
fn unknown_fields_are_rejected_by_name() {
    let err = match Scene::parse("widht: 320\n") {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected an error for an unknown field"),
    };
    assert!(err.contains("widht"), "the error should name the offending field: {err}");
}

#[test]
fn bad_values_are_rejected() {
    for (name, body) in [
        ("tiny width", "width: 2\n"),
        ("dither out of range", "palette:\n  dither: 2\n"),
        ("layer without a type", "layers:\n  - name: oops\n"),
    ] {
        assert!(Scene::parse(body).is_err(), "expected {name} to be rejected");
    }
}

/// An explicit zero is a value, not an absent field. Go tested every field
/// against its zero to decide whether it had been set, which made several
/// parameters impossible to turn off.
#[test]
fn an_explicit_zero_survives_loading() {
    let s = Scene::parse("prepare:\n  saturation: 0\n  contrast: 0\n").unwrap();
    assert_eq!(s.prepare.saturation, 0.0);
    assert_eq!(s.prepare.contrast, 0.0);
}

#[test]
fn frames_rounds_and_never_returns_zero() {
    assert_eq!(Loop { seconds: 0.01, fps: 1 }.frames(), 1);
    assert_eq!(Loop { seconds: 2.5, fps: 20 }.frames(), 50);
}

#[test]
fn regions_load_and_layers_refer_to_them() {
    let s = Scene::parse(
        r#"
regions:
  outside:
    all:
      - rect: { x: 0.1, y: 0, w: 0.8, h: 0.8 }
      - chroma: { min: 0.3 }
layers:
  - type: rain
    mask: { ref: outside }
  - type: glow
    mask: { ref: outside, invert: true }
"#,
    )
    .unwrap();
    assert_eq!(s.regions.len(), 1);
    assert_eq!(s.layers[0].mask.as_ref().unwrap().r#ref, "outside");
}

/// A layer pointing at a region that does not exist must fail at load, not
/// silently render with no mask at all.
#[test]
fn bad_region_references_are_rejected() {
    for (name, body) in [
        ("undefined", "layers:\n  - type: rain\n    mask: { ref: nowhere }\n"),
        ("cycle", "regions:\n  a: { ref: b }\n  b: { ref: a }\nlayers:\n  - type: rain\n"),
        (
            "ref with selector",
            "regions:\n  a: { rect: { x: 0, y: 0, w: 1, h: 1 } }\n\
             layers:\n  - type: rain\n    mask: { ref: a, rect: { x: 0, y: 0, w: 1, h: 1 } }\n",
        ),
    ] {
        assert!(Scene::parse(body).is_err(), "expected {name} to be rejected");
    }
}

/// A scene must survive the round trip through YAML, because the browser UI
/// edits a scene in memory and writes it back out.
#[test]
fn a_scene_round_trips_through_yaml() {
    let src = r#"
name: porch
width: 256
regions:
  outside:
    all:
      - rect: { x: 0.11, y: 0, w: 0.75, h: 0.82 }
      - chroma: { min: 0.3 }
layers:
  - name: rain
    type: rain
    mask: { ref: outside }
    params: { layers: 3, opacity: 0.4 }
"#;
    let a = Scene::parse(src).unwrap();
    let b = Scene::parse(&a.to_yaml().unwrap()).expect("re-parsing what we wrote");
    assert_eq!(b.name, "porch");
    assert_eq!(b.width, 256);
    assert_eq!(b.regions.len(), 1);
    assert_eq!(b.layers[0].mask.as_ref().unwrap().r#ref, "outside");
    assert_eq!(b.layers[0].params["opacity"].as_f64().unwrap(), 0.4);
}
