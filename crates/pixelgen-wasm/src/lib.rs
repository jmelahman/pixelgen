//! Browser bindings for the renderer.
//!
//! This crate is a thin adapter and deliberately nothing more: no rendering
//! decision is made here, so the browser and the CLI cannot diverge. The core
//! was written for it - it opens no files, spawns no threads it cannot drop,
//! and reports `palette.file` back to the host rather than reading it - so
//! everything below is conversion between `Image` and the RGBA buffers a
//! canvas speaks.

use pixelgen_core::mask::{self, Builder, Mask, Spec};
use pixelgen_core::pixel::Image;
use pixelgen_core::render::{self, Prepared};
use pixelgen_core::scene::{Combine, Scene};
use pixelgen_core::{effect, resample, starter};
use serde::Deserialize;
use serde_json::json;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// Every effect type as JSON, for the editor's add-layer menu and for building
/// a selected layer's controls: its one-line description and each parameter's
/// default and kind.
///
/// The kind is decided here because JSON cannot carry it: `1.0` and `1` arrive
/// in JavaScript as the same number, but one is a float the editor should step
/// in hundredths and the other a whole count of loop steps.
#[wasm_bindgen]
pub fn effects() -> Result<String, JsError> {
    let mut out = Vec::new();
    for (name, description) in effect::CATALOG {
        let defaults = effect::defaults(name).map_err(err)?;
        let params: Vec<_> = defaults
            .as_mapping()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| {
                let key = k.as_str()?;
                let choices = effect::choices(name, key);
                let kind = match v {
                    _ if !choices.is_empty() => "choice",
                    serde_yaml::Value::Bool(_) => "bool",
                    serde_yaml::Value::Number(n) if n.is_f64() => "float",
                    serde_yaml::Value::Number(_) => "int",
                    // Every optional string parameter is a tint, absent until
                    // set so the effect can use its own.
                    serde_yaml::Value::Null => "color",
                    _ => "text",
                };
                Some(json!({ "key": key, "default": v, "kind": kind, "choices": choices }))
            })
            .collect();
        out.push(json!({ "name": name, "description": description, "params": params }));
    }
    Ok(serde_json::to_string(&out)?)
}

/// One edit from the layer panel. `i` is always an index into the scene's
/// own `layers:` list, disabled layers included.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Edit {
    Add {
        #[serde(rename = "type")]
        kind: String,
        at: Option<usize>,
    },
    Remove {
        i: usize,
    },
    Move {
        from: usize,
        to: usize,
    },
    Disable {
        i: usize,
        disable: bool,
    },
    Rename {
        i: usize,
        name: String,
    },
    Param {
        i: usize,
        key: String,
        value: serde_yaml::Value,
    },
    /// `mask` is a YAML mask block, as it would appear under `mask:`, or
    /// `null` for the whole frame.
    Mask {
        i: usize,
        mask: Option<String>,
    },
    /// Fold a selection into the layer's existing mask. `mask: null` is the
    /// whole frame.
    Combine {
        i: usize,
        mask: Option<String>,
        mode: Combine,
    },
    /// Define or replace a named region, or remove it with `mask: null`.
    Region {
        name: String,
        mask: Option<String>,
    },
    /// Rename a region, and every reference to it.
    #[serde(rename = "rename-region")]
    RenameRegion {
        from: String,
        to: String,
    },
}

fn parse_spec(yaml: Option<String>) -> Result<Option<Spec>, JsError> {
    yaml.map(|m| {
        serde_yaml::from_str::<Spec>(&m).map_err(|e| JsError::new(&format!("parsing mask: {e}")))
    })
    .transpose()
}

/// One source photograph, plus whatever has been prepared from it.
///
/// The expensive half of the pipeline - downscaling, palette extraction,
/// dithering, every layer's mask - happens in [`Session::set_scene`] and is
/// kept, so scrubbing the loop costs one effect pass per frame and nothing
/// else. Editing the scene text calls `set_scene` again.
#[wasm_bindgen]
pub struct Session {
    src: Image,
    prep: Option<Prepared>,
    scene: Scene,
}

#[wasm_bindgen]
impl Session {
    /// `rgba` is a canvas `ImageData.data`: four bytes per pixel, row-major.
    /// The alpha channel is ignored - a photograph has no transparency to
    /// preserve, and compositing it against an arbitrary page color here
    /// would change the palette the scene is built from.
    #[wasm_bindgen(constructor)]
    pub fn new(rgba: &[u8], w: usize, h: usize) -> Result<Session, JsError> {
        if rgba.len() < w * h * 4 {
            return Err(JsError::new("image buffer is shorter than width x height x 4"));
        }
        let mut src = Image::new(w, h);
        for i in 0..w * h {
            src.pix[i * 3] = rgba[i * 4] as f32 / 255.0;
            src.pix[i * 3 + 1] = rgba[i * 4 + 1] as f32 / 255.0;
            src.pix[i * 3 + 2] = rgba[i * 4 + 2] as f32 / 255.0;
        }
        Ok(Session { src, prep: None, scene: Scene::default() })
    }

    /// The commented starter scene `pixelgen init` writes, for this image.
    ///
    /// `name` is only used for the scene's `name:` and `source:` fields; the
    /// browser has no path to record.
    pub fn starter(&self, name: &str) -> Result<String, JsError> {
        let s = Scene { source: name.into(), ..Default::default() };
        let (base, _) = render::pixelate(&self.src, &s).map_err(err)?;
        Ok(starter::starter(&s, name, &base).0)
    }

    /// Parse a scene and prepare everything that does not depend on the frame.
    ///
    /// Errors carry the same messages the CLI prints, which is most of why the
    /// editor can show a useful complaint about a scene rather than "invalid".
    pub fn set_scene(&mut self, yaml: &str) -> Result<(), JsError> {
        let scene = Scene::parse(yaml).map_err(err)?;
        if let Some(f) = scene.palette_file() {
            return Err(JsError::new(&format!(
                "palette.file ({f}) cannot be read in the browser - paste the colors into \
                 palette.hex instead"
            )));
        }
        self.commit(scene)
    }

    /// Apply one layer-panel edit (a JSON [`Edit`]) and return the scene as
    /// YAML, for the editor to put back in its text box.
    ///
    /// The edit is made to a copy and only kept if the result prepares. A
    /// layer whose mask catches nothing, or a parameter its effect rejects,
    /// reports why and leaves the working scene on screen.
    pub fn edit(&mut self, op: &str) -> Result<String, JsError> {
        let op: Edit = serde_json::from_str(op)?;
        let mut s = self.scene.clone();
        match op {
            Edit::Add { kind, at } => s.add_layer(&kind, at).map(drop),
            Edit::Remove { i } => s.remove_layer(i).map(drop),
            Edit::Move { from, to } => s.move_layer(from, to),
            Edit::Disable { i, disable } => s.set_layer_disable(i, disable),
            Edit::Rename { i, name } => s.rename_layer(i, &name),
            Edit::Param { i, key, value } => s.set_layer_param(i, &key, value),
            Edit::Mask { i, mask } => s.set_layer_mask(i, parse_spec(mask)?),
            Edit::Combine { i, mask, mode } => s.combine_layer_mask(i, parse_spec(mask)?, mode),
            Edit::Region { name, mask } => s.set_region(&name, parse_spec(mask)?),
            Edit::RenameRegion { from, to } => s.rename_region(&from, &to),
        }
        .map_err(err)?;
        s.validate().map_err(err)?;
        self.commit(s)?;
        self.normalized()
    }

    /// Every layer in the scene as JSON, disabled ones included, for the layer
    /// panel: its parameters with defaults filled in, which of them the scene
    /// actually sets, its mask as YAML, and `drawn` - its index into
    /// [`Session::layers`] and [`Session::layer_mask`], or `null` when it is
    /// switched off.
    pub fn scene_layers(&self) -> Result<String, JsError> {
        let mut drawn = 0;
        let mut out = Vec::new();
        for (i, l) in self.scene.layers.iter().enumerate() {
            let mut params = effect::defaults(&l.r#type).unwrap_or(serde_yaml::Value::Null);
            let mut set = Vec::new();
            if let (Some(p), Some(given)) = (params.as_mapping_mut(), l.params.as_mapping()) {
                for (k, v) in given {
                    p.insert(k.clone(), v.clone());
                    set.extend(k.as_str().map(String::from));
                }
            }
            let mask = match &l.mask {
                Some(m) => Some(serde_yaml::to_string(m).map_err(err)?),
                None => None,
            };
            out.push(json!({
                "name": l.name,
                "label": l.label(i),
                "type": l.r#type,
                "disable": l.disable,
                "params": params,
                "set": set,
                "mask": mask,
                "drawn": (!l.disable).then_some(drawn),
            }));
            if !l.disable {
                drawn += 1;
            }
        }
        Ok(serde_json::to_string(&out)?)
    }

    /// Grid width in cells. Zero until a scene is set.
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> usize {
        self.prep.as_ref().map_or(0, |p| p.base.w)
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> usize {
        self.prep.as_ref().map_or(0, |p| p.base.h)
    }

    #[wasm_bindgen(getter)]
    pub fn frames(&self) -> usize {
        self.prep.as_ref().map_or(0, |p| p.frames)
    }

    #[wasm_bindgen(getter)]
    pub fn fps(&self) -> usize {
        self.scene.loop_.fps
    }

    /// The layers that will actually be drawn, in order, as JSON strings.
    /// Disabled ones are absent, which is what the editor should gray out.
    #[wasm_bindgen(getter)]
    pub fn layers(&self) -> Vec<String> {
        self.prep
            .as_ref()
            .map(|p| p.layer_names().into_iter().map(String::from).collect())
            .unwrap_or_default()
    }

    /// The effect type of each drawn layer, parallel to [`Session::layers`].
    ///
    /// Taken from the scene rather than from the prepared layers because the
    /// two are the same list: preparation drops the disabled entries and keeps
    /// the order.
    #[wasm_bindgen(getter)]
    pub fn layer_types(&self) -> Vec<String> {
        self.scene.layers.iter().filter(|l| !l.disable).map(|l| l.r#type.clone()).collect()
    }

    /// Loop length as written, which `frames` rounds to a whole frame count.
    #[wasm_bindgen(getter)]
    pub fn seconds(&self) -> f64 {
        self.scene.loop_.seconds
    }

    /// `palette.colors` as written. The palette itself can come out smaller:
    /// an image with fewer distinct colors cannot be clustered into more.
    #[wasm_bindgen(getter)]
    pub fn colors(&self) -> usize {
        self.scene.palette.colors
    }

    /// Whether the palette is derived from the image, and so whether
    /// `palette.colors` means anything. A fixed `hex` list ignores it.
    #[wasm_bindgen(getter)]
    pub fn palette_derived(&self) -> bool {
        self.scene.palette.hex.is_empty()
    }

    /// The scene palette as `#rrggbb`, in the renderer's own order - adjacent
    /// entries are adjacent shades, which is what makes `palette_cycle` read
    /// as flow and what the editor should preserve when showing swatches.
    #[wasm_bindgen(getter)]
    pub fn palette(&self) -> Vec<String> {
        self.prep
            .as_ref()
            .map(|p| p.matcher.palette.iter().map(|c| pixelgen_core::palette::hex(*c)).collect())
            .unwrap_or_default()
    }

    /// The static base, with no layers over it.
    pub fn base(&self, scale: usize) -> Result<Vec<u8>, JsError> {
        let p = self.prepared()?;
        Ok(rgba(&p.base, scale))
    }

    /// One frame of the loop, as RGBA at `scale` times the grid.
    ///
    /// The upscale happens here rather than in CSS because the browser's
    /// smoothing would blur exactly the hard cell edges the whole pipeline
    /// exists to produce. `image-rendering: pixelated` is still worth setting
    /// for the residual fractional scaling the layout does.
    pub fn frame(&self, i: usize, scale: usize) -> Result<Vec<u8>, JsError> {
        let p = self.prepared()?;
        if i >= p.frames {
            return Err(JsError::new(&format!("frame {i} out of range (loop is {})", p.frames)));
        }
        Ok(rgba(&p.frame(i), scale))
    }

    /// The whole loop as a GIF.
    ///
    /// The one moving format the page can write without a codec: the frames
    /// are already indices into a palette of at most 256 colors, which is
    /// what a GIF stores, so nothing is re-encoded and the loop stays exact.
    /// Video goes out through the browser's own recorder instead.
    pub fn gif(&self, scale: usize) -> Result<Vec<u8>, JsError> {
        let p = self.prepared()?;
        let frames: Vec<_> = (0..p.frames).map(|i| p.frame(i)).collect();
        pixelgen_core::encode::gif(&frames, &p.matcher.palette, scale, self.fps())
            .map_err(|e| JsError::new(&e))
    }

    /// A drawn layer's resolved mask as 8-bit coverage, one byte per cell.
    ///
    /// Indices match [`Session::layers`]. Drawing this over the base is the
    /// only honest way to see what a `chroma` or `luma` selector caught: the
    /// silhouette it cuts is the whole point and cannot be read off the YAML.
    pub fn layer_mask(&self, i: usize) -> Result<Vec<u8>, JsError> {
        let p = self.prepared()?;
        let m = p.layer_mask(i).ok_or_else(|| JsError::new("no such layer"))?;
        Ok(coverage(m, p.base.w, p.base.h))
    }

    /// The same, for a mask that is not in the scene yet: the editor writes a
    /// selector, this shows what it selects, and only then does it become a
    /// layer. `spec` is a YAML mask block, as it would appear under `mask:`.
    ///
    /// JSON is YAML, so the editor's selection goes in as it is kept.
    pub fn preview_mask(&self, spec: &str) -> Result<Vec<u8>, JsError> {
        let p = self.prepared()?;
        let spec: Spec =
            serde_yaml::from_str(spec).map_err(|e| JsError::new(&format!("parsing mask: {e}")))?;
        mask::validate(&self.scene.regions, &[Some(&spec)]).map_err(err)?;
        let mut b =
            Builder::new(&p.base, &self.scene.regions).dithered(self.scene.palette.dither > 0.0);
        let m = b.build(Some(&spec)).map_err(err)?;
        Ok(coverage(&m, p.base.w, p.base.h))
    }

    /// A YAML mask block as JSON, so the editor can load a layer's mask or a
    /// region back into its selection and keep working on it.
    pub fn spec_json(&self, yaml: &str) -> Result<String, JsError> {
        let spec: Spec =
            serde_yaml::from_str(yaml).map_err(|e| JsError::new(&format!("parsing mask: {e}")))?;
        Ok(serde_json::to_string(&spec)?)
    }

    /// A selection (JSON or YAML) as the YAML block it would be in the scene,
    /// for the editor's Copy button.
    pub fn spec_yaml(&self, spec: &str) -> Result<String, JsError> {
        let spec: Spec =
            serde_yaml::from_str(spec).map_err(|e| JsError::new(&format!("parsing mask: {e}")))?;
        serde_yaml::to_string(&spec).map_err(err)
    }

    /// The scene's regions as JSON, in name order: each one's `name`, its
    /// definition as a YAML `mask`, and the layers and regions that `use` it.
    pub fn regions(&self) -> Result<String, JsError> {
        let mut out = Vec::new();
        for (name, spec) in &self.scene.regions {
            out.push(json!({
                "name": name,
                "mask": serde_yaml::to_string(spec).map_err(err)?,
                "users": self.scene.region_users(name),
            }));
        }
        Ok(serde_json::to_string(&out)?)
    }

    /// The color of the base cell under a normalized point, as `#rrggbb`:
    /// what a magic wand clicked there records.
    pub fn sample(&self, x: f32, y: f32) -> Result<String, JsError> {
        Ok(mask::sample(&self.prepared()?.base, x, y))
    }

    /// Anchors the magnetic lasso at a normalized point. The expensive part
    /// happens here, once per anchor; following the pointer is cheap.
    pub fn livewire(&self, x: f32, y: f32) -> Result<LiveWire, JsError> {
        Ok(LiveWire(mask::LiveWire::new(&self.prepared()?.base, x, y)))
    }

    /// The scene as the renderer understands it, re-serialized. The editor
    /// uses this to show what a shorthand actually expanded to.
    pub fn normalized(&self) -> Result<String, JsError> {
        self.scene.to_yaml().map_err(err)
    }

    /// Prepare `scene` and make it the current one. Nothing changes if it
    /// fails, so the last working scene stays on screen.
    fn commit(&mut self, scene: Scene) -> Result<(), JsError> {
        self.prep = Some(render::prepare(&self.src, &scene).map_err(err)?);
        self.scene = scene;
        Ok(())
    }

    fn prepared(&self) -> Result<&Prepared, JsError> {
        self.prep.as_ref().ok_or_else(|| JsError::new("no scene set yet"))
    }
}

/// One magnetic-lasso anchor's paths, from [`Session::livewire`].
#[wasm_bindgen]
pub struct LiveWire(mask::LiveWire);

#[wasm_bindgen]
impl LiveWire {
    /// The path from the anchor to a normalized point that follows the
    /// strongest edges between them, as flat `x, y` pairs.
    pub fn path_to(&self, x: f32, y: f32) -> Vec<f32> {
        self.0.path_to(x, y).into_iter().flat_map(|(x, y)| [x, y]).collect()
    }
}

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn coverage(m: &Mask, w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = (m.at(x as i32, y as i32).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

fn rgba(im: &Image, scale: usize) -> Vec<u8> {
    let scaled;
    let im = if scale > 1 {
        scaled = resample::upscale(im, scale);
        &scaled
    } else {
        im
    };
    let mut out = vec![255u8; im.w * im.h * 4];
    for i in 0..im.w * im.h {
        for c in 0..3 {
            out[i * 4 + c] = (im.pix[i * 3 + c].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}
