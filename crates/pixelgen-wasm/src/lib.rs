//! Browser bindings for the renderer.
//!
//! This crate is a thin adapter and deliberately nothing more: no rendering
//! decision is made here, so the browser and the CLI cannot diverge. The core
//! was written for it - it opens no files, spawns no threads it cannot drop,
//! and reports `palette.file` back to the host rather than reading it - so
//! everything below is conversion between `Image` and the RGBA buffers a
//! canvas speaks.

use pixelgen_core::mask::{Builder, Mask, Spec};
use pixelgen_core::pixel::Image;
use pixelgen_core::render::{self, Prepared};
use pixelgen_core::scene::Scene;
use pixelgen_core::{effect, resample, starter};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// Every effect type and its one-line description, as JSON, for populating the
/// editor's palette of layers.
#[wasm_bindgen]
pub fn effects() -> String {
    let body: Vec<String> = effect::CATALOG
        .iter()
        .map(|(n, d)| format!("{{\"name\":{},\"description\":{}}}", quote(n), quote(d)))
        .collect();
    format!("[{}]", body.join(","))
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
    /// preserve, and compositing it against an arbitrary page colour here
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
        let mut s = Scene::default();
        s.source = name.into();
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
                "palette.file ({f}) cannot be read in the browser - paste the colours into \
                 palette.hex instead"
            )));
        }
        self.prep = Some(render::prepare(&self.src, &scene).map_err(err)?);
        self.scene = scene;
        Ok(())
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
    /// Disabled ones are absent, which is what the editor should grey out.
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
    pub fn preview_mask(&self, spec: &str) -> Result<Vec<u8>, JsError> {
        let p = self.prepared()?;
        let spec: Spec = serde_yaml::from_str(spec)
            .map_err(|e| JsError::new(&format!("parsing mask: {e}")))?;
        let mut b = Builder::new(&p.base, &self.scene.regions);
        let m = b.build(Some(&spec)).map_err(err)?;
        Ok(coverage(&m, p.base.w, p.base.h))
    }

    /// The scene as the renderer understands it, re-serialized. The editor
    /// uses this to show what a shorthand actually expanded to.
    pub fn normalized(&self) -> Result<String, JsError> {
        self.scene.to_yaml().map_err(err)
    }

    fn prepared(&self) -> Result<&Prepared, JsError> {
        self.prep.as_ref().ok_or_else(|| JsError::new("no scene set yet"))
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
