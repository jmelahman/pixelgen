//! The editor's layer panel: effect defaults it builds controls from, and the
//! scene edits it makes.

use pixelgen_core::effect::{self, CATALOG};
use pixelgen_core::mask::Spec;
use pixelgen_core::scene::Scene;
use serde_yaml::Value;

/// `defaults` and `build` are two matches over the same names. A missing arm,
/// or a default that `build` itself rejects, would give the editor a control
/// that breaks the scene the moment it is touched.
#[test]
fn every_effect_has_defaults_that_build() {
    for (name, _) in CATALOG {
        let d = effect::defaults(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(d.is_mapping(), "{name}: defaults should be a mapping, got {d:?}");
        effect::build(name, &d).unwrap_or_else(|e| panic!("{name} rejects its own defaults: {e}"));
    }
}

#[test]
fn float_defaults_read_as_written() {
    let d = effect::defaults("vignette").unwrap();
    assert_eq!(d.get("amount"), Some(&Value::from(0.3)));
}

fn scene() -> Scene {
    Scene::parse("layers:\n  - type: rain\n  - type: mist\n").unwrap()
}

fn types(s: &Scene) -> Vec<&str> {
    s.layers.iter().map(|l| l.r#type.as_str()).collect()
}

#[test]
fn layers_are_added_removed_and_moved() {
    let mut s = scene();
    assert_eq!(s.add_layer("vignette", None).unwrap(), 2);
    s.add_layer("glow", Some(0)).unwrap();
    assert_eq!(types(&s), ["glow", "rain", "mist", "vignette"]);

    s.move_layer(0, 3).unwrap();
    assert_eq!(types(&s), ["rain", "mist", "vignette", "glow"]);
    s.move_layer(2, 0).unwrap();
    assert_eq!(types(&s), ["vignette", "rain", "mist", "glow"]);

    assert_eq!(s.remove_layer(1).unwrap().r#type, "rain");
    assert_eq!(types(&s), ["vignette", "mist", "glow"]);
}

#[test]
fn edits_out_of_range_are_rejected() {
    let mut s = scene();
    assert!(s.add_layer("rain", Some(3)).is_err());
    assert!(s.remove_layer(2).is_err());
    assert!(s.move_layer(0, 2).is_err());
    assert!(s.set_layer_disable(5, true).is_err());
    assert!(s.rename_layer(5, "x").is_err());
    assert!(s.set_layer_mask(5, None).is_err());
    assert_eq!(types(&s), ["rain", "mist"]);
}

/// The file records what was changed and nothing else: a value put back to
/// its default disappears, and so does a params block left empty.
#[test]
fn params_record_only_what_differs_from_the_default() {
    let mut s = scene();
    s.set_layer_param(0, "count", Value::from(40)).unwrap();
    s.set_layer_param(0, "opacity", Value::from(0.5)).unwrap();
    assert_eq!(s.layers[0].params.as_mapping().unwrap().len(), 2);

    s.set_layer_param(0, "opacity", Value::from(0.34)).unwrap();
    assert_eq!(s.layers[0].params.as_mapping().unwrap().len(), 1);
    s.set_layer_param(0, "count", Value::from(220.0)).unwrap();
    assert!(s.layers[0].params.is_null());

    s.set_layer_param(0, "color", Value::from("#aabbcc")).unwrap();
    s.set_layer_param(0, "color", Value::Null).unwrap();
    assert!(s.layers[0].params.is_null());

    let err = s.set_layer_param(0, "cuont", Value::from(1)).unwrap_err().to_string();
    assert!(err.contains("cuont"), "the error should name the parameter: {err}");
}

#[test]
fn edited_scenes_round_trip_through_yaml() {
    let mut s = scene();
    s.rename_layer(0, "  window rain ").unwrap();
    s.set_layer_disable(1, true).unwrap();
    s.set_layer_param(0, "color", Value::from("#aabbcc")).unwrap();
    let spec: Spec = serde_yaml::from_str("rect: { x: 0.1, y: 0.2, w: 0.3, h: 0.4 }").unwrap();
    s.set_layer_mask(0, Some(spec)).unwrap();

    let back = Scene::parse(&s.to_yaml().unwrap()).unwrap();
    assert_eq!(back.layers[0].name, "window rain");
    assert!(back.layers[1].disable);
    assert_eq!(back.layers[0].params.get("color"), Some(&Value::from("#aabbcc")));
    assert!(back.layers[0].mask.as_ref().unwrap().rect.is_some());

    s.set_layer_mask(0, None).unwrap();
    assert!(Scene::parse(&s.to_yaml().unwrap()).unwrap().layers[0].mask.is_none());
}
