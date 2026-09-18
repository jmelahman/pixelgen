//! The YAML description of a background: how the source photograph is reduced
//! to a pixel grid, and which animated layers are drawn over it.
//!
//! The format is deliberately declarative and free of any executable fragment.
//! Every layer names a known effect type, a region, and plain scalar
//! parameters, which keeps a scene file something a person can hand-edit and
//! something a model could later emit without the renderer having to trust it.
//!
//! Defaults are expressed as `impl Default` plus `#[serde(default)]`, so an
//! explicitly written `0` means zero. The Go version tested fields against
//! their zero value to decide whether they had been set, which made
//! `saturation: 0` silently mean 1.15 and left several parameters impossible
//! to turn off.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::effect;
use crate::mask::{self, Registry, Spec};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scene {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The source photograph, relative to the scene file. A scene is still
    /// usable without one; the CLI can be pointed at an image directly.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Width of the pixel grid in cells. Height follows from the aspect ratio.
    pub width: usize,
    pub seed: u32,
    pub palette: Palette,
    pub prepare: Prepare,
    #[serde(rename = "loop")]
    pub loop_: Loop,

    /// Named masks that layers refer to with `ref`. Working out where a thing
    /// actually is - the opening of a window, the lit half of a room - is most
    /// of the work of writing a scene, and several layers usually need the
    /// same answer or its inverse.
    #[serde(skip_serializing_if = "Registry::is_empty")]
    pub regions: Registry,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            name: String::new(),
            source: String::new(),
            width: 320,
            seed: 1,
            palette: Palette::default(),
            prepare: Prepare::default(),
            loop_: Loop::default(),
            regions: Registry::new(),
            layers: Vec::new(),
        }
    }
}

/// Chooses the color set. At most one source may be given: `colors` derives
/// one from the image, `hex` and `file` supply a fixed one.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Palette {
    pub colors: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hex: Vec<String>,
    /// A file of whitespace-separated hex colors, relative to the scene file.
    ///
    /// The core never opens it: [`Scene::palette_file`] reports it and
    /// [`Scene::set_palette_hex`] folds the contents into `hex`, so loading is
    /// the host's job and the renderer works unchanged in the browser.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// Dither strength in `[0,1]`, applied once to the static base. Frames are
    /// never dithered, so the pattern cannot crawl.
    pub dither: f32,
}

impl Default for Palette {
    fn default() -> Self {
        Palette { colors: 32, hex: Vec::new(), file: String::new(), dither: 0.0 }
    }
}

/// Photographic corrections applied before quantization. They exist because a
/// photograph's noise and muted color rarely survive a small palette intact.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Prepare {
    /// A 3x3 median filter at full resolution, before averaging. Once pixels
    /// are merged into cells, speckles have already contaminated their
    /// neighborhood and cannot be isolated.
    pub median: bool,
    pub saturation: f32,
    pub contrast: f32,
}

impl Default for Prepare {
    fn default() -> Self {
        Prepare { median: false, saturation: 1.15, contrast: 1.05 }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Loop {
    pub seconds: f64,
    pub fps: usize,
}

impl Default for Loop {
    fn default() -> Self {
        // Twenty is the reference cadence for this style, and low enough that
        // motion still reads as stepped rather than smooth.
        Loop { seconds: 6.0, fps: 20 }
    }
}

impl Loop {
    /// The number of rendered frames in one loop.
    pub fn frames(&self) -> usize {
        ((self.seconds * self.fps as f64 + 0.5) as usize).max(1)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layer {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<Spec>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub params: Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disable: bool,
}

impl Layer {
    /// How the layer is named in diagnostics.
    pub fn label(&self, i: usize) -> String {
        if self.name.is_empty() {
            format!("{}[{}]", self.r#type, i)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Parse(serde_yaml::Error),
    TooNarrow(usize),
    NoFps,
    Dither(f32),
    NoType(usize),
    Mask(mask::Error),
    /// An edit named a layer index the scene does not have.
    NoLayer(usize),
    /// An edit set a parameter the layer's effect does not take.
    Param(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(e) => write!(f, "scene: {e}"),
            Error::TooNarrow(w) => write!(f, "scene: width {w} is too small"),
            Error::NoFps => write!(f, "scene: fps must be at least 1"),
            Error::Dither(v) => write!(f, "scene: dither must be within [0,1], got {v}"),
            Error::NoType(i) => write!(f, "scene: layer {i} has no type"),
            Error::Mask(e) => write!(f, "scene: {e}"),
            Error::NoLayer(i) => write!(f, "scene: there is no layer {i}"),
            Error::Param(m) => write!(f, "scene: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<mask::Error> for Error {
    fn from(e: mask::Error) -> Self {
        Error::Mask(e)
    }
}

impl Scene {
    /// Parse and validate a scene.
    ///
    /// Unknown fields are rejected: a silently ignored typo in a scene file
    /// shows up as an effect mysteriously doing nothing, which is painful to
    /// debug.
    pub fn parse(yaml: &str) -> Result<Scene, Error> {
        let s: Scene = serde_yaml::from_str(yaml).map_err(Error::Parse)?;
        s.validate()?;
        Ok(s)
    }

    pub fn to_yaml(&self) -> Result<String, Error> {
        serde_yaml::to_string(self).map_err(Error::Parse)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.width < 8 {
            return Err(Error::TooNarrow(self.width));
        }
        if self.loop_.fps < 1 {
            return Err(Error::NoFps);
        }
        if !(0.0..=1.0).contains(&self.palette.dither) {
            return Err(Error::Dither(self.palette.dither));
        }
        for (i, l) in self.layers.iter().enumerate() {
            if l.r#type.trim().is_empty() {
                return Err(Error::NoType(i));
            }
        }
        let specs: Vec<Option<&Spec>> = self.layers.iter().map(|l| l.mask.as_ref()).collect();
        mask::validate(&self.regions, &specs)?;
        Ok(())
    }

    /// The palette file the scene asks for, if any. The host resolves it
    /// against the scene's own directory and hands the contents back to
    /// [`Scene::set_palette_hex`].
    pub fn palette_file(&self) -> Option<&str> {
        if self.palette.file.is_empty() {
            None
        } else {
            Some(&self.palette.file)
        }
    }

    /// Fold the contents of a palette file into `palette.hex`.
    pub fn set_palette_hex(&mut self, contents: &str) {
        self.palette.hex = contents.split_whitespace().map(str::to_string).collect();
        self.palette.file.clear();
    }

    // ------------------------------------------------------------ layer edits
    //
    // What the editor's layer panel does to a scene. They are plain mutations
    // that leave checking to the caller: the host validates and prepares the
    // result, and keeps the old scene if that fails, so a half-made edit never
    // replaces a working one.

    fn layer_mut(&mut self, i: usize) -> Result<&mut Layer, Error> {
        self.layers.get_mut(i).ok_or(Error::NoLayer(i))
    }

    /// Insert a layer of effect `kind` at `at`, or on top when `at` is `None`.
    /// It starts with no parameters and no mask, which is the whole frame.
    pub fn add_layer(&mut self, kind: &str, at: Option<usize>) -> Result<usize, Error> {
        let at = at.unwrap_or(self.layers.len());
        if at > self.layers.len() {
            return Err(Error::NoLayer(at));
        }
        self.layers.insert(at, Layer { r#type: kind.into(), ..Default::default() });
        Ok(at)
    }

    pub fn remove_layer(&mut self, i: usize) -> Result<Layer, Error> {
        if i >= self.layers.len() {
            return Err(Error::NoLayer(i));
        }
        Ok(self.layers.remove(i))
    }

    /// Move layer `from` so that it ends up at index `to`.
    pub fn move_layer(&mut self, from: usize, to: usize) -> Result<(), Error> {
        if from >= self.layers.len() {
            return Err(Error::NoLayer(from));
        }
        if to >= self.layers.len() {
            return Err(Error::NoLayer(to));
        }
        let l = self.layers.remove(from);
        self.layers.insert(to, l);
        Ok(())
    }

    pub fn set_layer_disable(&mut self, i: usize, disable: bool) -> Result<(), Error> {
        self.layer_mut(i)?.disable = disable;
        Ok(())
    }

    pub fn rename_layer(&mut self, i: usize, name: &str) -> Result<(), Error> {
        self.layer_mut(i)?.name = name.trim().into();
        Ok(())
    }

    /// Set one of a layer's effect parameters.
    ///
    /// `null`, or the effect's own default, removes the key instead: the scene
    /// file should record what was changed, not restate every default, or it
    /// stops being something a person can read.
    pub fn set_layer_param(&mut self, i: usize, key: &str, value: Value) -> Result<(), Error> {
        let l = self.layer_mut(i)?;
        let defaults = effect::defaults(&l.r#type).map_err(|e| Error::Param(e.to_string()))?;
        let default = defaults
            .get(key)
            .ok_or_else(|| Error::Param(format!("{} takes no parameter {key:?}", l.r#type)))?;
        // A number input hands back `1` for a default of `1.0`; both mean the
        // same parameter value, so they compare as numbers.
        let same = match (default.as_f64(), value.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => *default == value,
        };
        let unset = value.is_null() || same;
        if !l.params.is_mapping() {
            if unset {
                return Ok(());
            }
            l.params = Value::Mapping(Default::default());
        }
        let m = l.params.as_mapping_mut().expect("params was just made a mapping");
        if unset {
            m.remove(key);
        } else {
            m.insert(key.into(), value);
        }
        if m.is_empty() {
            l.params = Value::Null;
        }
        Ok(())
    }

    /// Replace a layer's mask; `None` is the whole frame.
    pub fn set_layer_mask(&mut self, i: usize, mask: Option<Spec>) -> Result<(), Error> {
        self.layer_mut(i)?.mask = mask;
        Ok(())
    }
}
