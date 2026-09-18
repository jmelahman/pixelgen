//! The editor's layer panel: effect defaults it builds controls from, and the
//! scene edits it makes.

use pixelgen_core::effect::{self, CATALOG};
use pixelgen_core::mask::{Spec, Step};
use pixelgen_core::scene::{Combine, Scene};
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

// ---------------------------------------------------------------- selections

fn spec(yaml: &str) -> Spec {
    serde_yaml::from_str(yaml).unwrap()
}

fn ops(s: &Spec) -> Vec<&str> {
    s.steps.iter().map(|st| st.op().unwrap().0).collect()
}

const LEFT: &str = "rect: { x: 0, y: 0, w: 0.5, h: 1 }";
const TOP: &str = "rect: { x: 0, y: 0, w: 1, h: 0.5 }";

#[test]
fn a_selection_combines_into_a_layer_without_a_mask() {
    let mut s = scene();
    s.combine_layer_mask(0, Some(spec(LEFT)), Combine::Add).unwrap();
    assert!(s.layers[0].mask.is_none(), "adding to the whole frame is the whole frame");

    s.combine_layer_mask(0, Some(spec(LEFT)), Combine::And).unwrap();
    assert!(s.layers[0].mask.as_ref().unwrap().rect.is_some());

    s.set_layer_mask(0, None).unwrap();
    s.combine_layer_mask(0, Some(spec(LEFT)), Combine::Sub).unwrap();
    let m = s.layers[0].mask.as_ref().unwrap();
    assert_eq!(ops(m), ["add", "sub"]);
    assert!(m.steps[0].add.as_ref().unwrap().is_full_frame());
    s.validate().unwrap();

    s.combine_layer_mask(0, Some(spec(TOP)), Combine::Replace).unwrap();
    assert_eq!(s.layers[0].mask.as_ref().unwrap().rect.unwrap().h, 0.5);
}

#[test]
fn every_mode_folds_a_selection_into_an_existing_mask() {
    for (mode, op) in [(Combine::Add, "add"), (Combine::Sub, "sub"), (Combine::And, "and")] {
        let mut s = scene();
        s.set_layer_mask(0, Some(spec(LEFT))).unwrap();
        s.combine_layer_mask(0, Some(spec(TOP)), mode).unwrap();
        let m = s.layers[0].mask.as_ref().unwrap();
        assert_eq!(ops(m), ["add", op], "{mode:?}");
        assert!(m.steps[0].add.as_ref().unwrap().rect.is_some());
        s.validate().unwrap();
    }
}

/// Repeated edits keep one flat list rather than nesting a level each time,
/// unless a modifier on the list would then apply to the new step too.
#[test]
fn combining_appends_to_a_plain_steps_list() {
    let mut s = scene();
    s.set_layer_mask(0, Some(spec(LEFT))).unwrap();
    s.combine_layer_mask(0, Some(spec(TOP)), Combine::Add).unwrap();
    s.combine_layer_mask(
        0,
        Some(spec("ellipse: { x: 0.4, y: 0.4, w: 0.2, h: 0.2 }")),
        Combine::Sub,
    )
    .unwrap();
    assert_eq!(ops(s.layers[0].mask.as_ref().unwrap()), ["add", "add", "sub"]);

    let mut feathered = s.layers[0].mask.clone().unwrap();
    feathered.feather = 2.0;
    s.set_layer_mask(0, Some(feathered)).unwrap();
    s.combine_layer_mask(0, Some(spec(TOP)), Combine::And).unwrap();
    let m = s.layers[0].mask.as_ref().unwrap();
    assert_eq!(ops(m), ["add", "and"]);
    assert_eq!(m.steps[0].add.as_ref().unwrap().feather, 2.0);
}

/// A selection of one `add` step is written as just its operand, and Select
/// All applied as a replacement clears the mask.
#[test]
fn a_selection_is_simplified_before_it_is_stored() {
    let mut s = scene();
    let one = Spec { steps: vec![Step::add(spec(LEFT))], ..Spec::default() };
    s.combine_layer_mask(0, Some(one.clone()), Combine::Replace).unwrap();
    let m = s.layers[0].mask.as_ref().unwrap();
    assert!(m.steps.is_empty() && m.rect.is_some());

    s.combine_layer_mask(0, Some(Spec::default()), Combine::Replace).unwrap();
    assert!(s.layers[0].mask.is_none());

    s.set_region("left", Some(one)).unwrap();
    assert!(s.regions["left"].rect.is_some());
}

#[test]
fn regions_are_set_replaced_and_removed() {
    let mut s = scene();
    s.set_region(" sky ", Some(spec(TOP))).unwrap();
    s.set_region("ground", Some(spec(LEFT))).unwrap();
    assert_eq!(s.regions.keys().collect::<Vec<_>>(), ["ground", "sky"]);
    s.set_layer_mask(0, Some(spec("ref: sky"))).unwrap();
    s.validate().unwrap();

    s.set_region("sky", Some(spec(LEFT))).unwrap();
    assert_eq!(s.regions["sky"].rect.unwrap().w, 0.5);
    assert!(s.set_region("  ", Some(spec(LEFT))).is_err());

    let used = s.set_region("sky", None).unwrap_err().to_string();
    assert!(used.contains("layer rain[0]"), "{used}");
    assert!(s.regions.contains_key("sky"), "a region in use was removed");

    s.set_layer_mask(0, None).unwrap();
    s.set_region("sky", None).unwrap();
    assert!(!s.regions.contains_key("sky"));
    s.validate().unwrap();
}

#[test]
fn renaming_a_region_renames_every_reference_to_it() {
    let mut s = scene();
    s.set_region("sky", Some(spec(TOP))).unwrap();
    s.set_region("upper-left", Some(spec(&format!("all: [{{ref: sky}}, {{{LEFT}}}]")))).unwrap();
    s.set_region("ground", Some(spec("ref: sky\ninvert: true"))).unwrap();
    s.set_layer_mask(0, Some(spec("steps: [{add: {ref: sky}}, {sub: {ref: ground}}]"))).unwrap();
    assert_eq!(s.region_users("sky"), ["layer rain[0]", "region ground", "region upper-left"]);

    s.rename_region("sky", " heavens ").unwrap();
    s.validate().unwrap();
    assert!(!s.regions.contains_key("sky"));
    assert!(s.region_users("sky").is_empty());
    assert_eq!(s.region_users("heavens").len(), 3);
    assert_eq!(s.layers[0].mask.as_ref().unwrap().steps[0].add.as_ref().unwrap().r#ref, "heavens");
    assert_eq!(s.regions["ground"].r#ref, "heavens");

    assert!(s.rename_region("heavens", "ground").is_err(), "renamed onto another region");
    assert!(s.rename_region("heavens", " ").is_err());
    assert!(s.rename_region("nowhere", "elsewhere").is_err());
    assert!(s.rename_region("nowhere", "nowhere").is_err());
    s.rename_region("heavens", "heavens").unwrap();
    s.rename_region(" heavens", "heavens ").unwrap();
    assert!(s.regions.contains_key("heavens"));
}

#[test]
fn an_empty_region_name_is_not_every_reference() {
    let yaml = "
regions:
  '': { rect: { x: 0, y: 0, w: 1, h: 0.5 } }
  sky: { rect: { x: 0, y: 0, w: 0.5, h: 1 } }
layers:
  - { type: rain, mask: { ref: sky } }
";
    let mut s = Scene::parse(yaml).unwrap();
    assert!(s.region_users("").is_empty());
    assert!(s.rename_region("", "x").is_err());
    s.set_region("", None).unwrap_err();
    assert_eq!(s.region_users("sky"), ["layer rain[0]"]);
}

#[test]
fn a_region_saved_over_itself_keeps_its_old_definition() {
    let mut s = scene();
    s.set_region("sky", Some(spec(TOP))).unwrap();

    // Loaded as a selection and saved back unchanged.
    s.set_region("sky", Some(spec("ref: sky"))).unwrap();
    assert!(s.regions["sky"].rect.is_some());
    s.validate().unwrap();

    // Refined, and with a step added.
    s.set_region(
        "sky",
        Some(spec(
            "steps: [{add: {ref: sky, grow: 2}}, {sub: {rect: {x: 0, y: 0, w: 0.1, h: 0.1}}}]",
        )),
    )
    .unwrap();
    s.validate().unwrap();
    let sky = &s.regions["sky"];
    let first = sky.steps[0].add.as_ref().unwrap();
    assert!(first.r#ref.is_empty() && first.grow == 2.0);
    assert!(first.steps[0].add.as_ref().unwrap().rect.is_some());
}

#[test]
fn selections_round_trip_through_the_scene_file() {
    let mut s = scene();
    let sel = Spec {
        steps: vec![
            Step::add(spec("wand: { x: 0.4, y: 0.3, hex: '#3a5f8c', tolerance: 0.1 }")),
            Step::add(spec("path: M.1 .2 L.3 .22 L.28 .41 Z")),
            Step::sub(spec("stroke: { d: M.2 .3 L.25 .31, radius: 0.02, hardness: 0.5 }")),
        ],
        grow: -1.0,
        smooth: 1.0,
        ..Spec::default()
    };
    s.combine_layer_mask(0, Some(sel), Combine::Replace).unwrap();
    s.set_region("b", Some(spec(TOP))).unwrap();
    s.set_region("a", Some(spec(LEFT))).unwrap();
    let yaml = s.to_yaml().unwrap();
    assert!(!yaml.contains('!'), "an enum tag was written:\n{yaml}");
    assert!(yaml.find("a:").unwrap() < yaml.find("b:").unwrap(), "regions out of order");

    let back = Scene::parse(&yaml).unwrap();
    back.validate().unwrap();
    assert_eq!(back.to_yaml().unwrap(), yaml);
    let m = back.layers[0].mask.as_ref().unwrap();
    assert_eq!(ops(m), ["add", "add", "sub"]);
    assert_eq!((m.grow, m.smooth), (-1.0, 1.0));
    assert!(m.steps[0].add.as_ref().unwrap().wand.as_ref().unwrap().contiguous);
}
