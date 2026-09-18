//! The animated layers composited over the static pixelated base.

use std::fmt;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_yaml::Value;

use crate::mask::Mask;
use crate::palette::{parse_hex, Matcher};
use crate::pixel::{Image, Rgb};

mod light;
mod motion;
mod post;
mod weather;

/// Everything an effect needs to draw one frame.
pub struct Context<'a> {
    /// The loop phase in `[0,1)`.
    ///
    /// Every effect must be periodic in `t`, so that the last frame hands off
    /// to the first with no visible seam.
    pub t: f32,
    /// The same information in integer form, for effects that advance in
    /// discrete steps.
    pub frame: usize,
    pub frames: usize,

    /// The static pixelated, palette-snapped source image. Effects read it to
    /// sample original colors; it is shared and immutable.
    pub base: &'a Image,
    /// The region the effect is confined to.
    pub mask: &'a Mask,
    /// The scene palette, for effects that want to stay strictly in-palette
    /// while drawing rather than relying on the final snap.
    pub matcher: &'a Matcher,
    /// The scene seed combined with the effect's index, so each effect gets a
    /// distinct but reproducible random stream.
    pub seed: u32,
}

/// Draws one frame onto `dst`.
///
/// `render` takes `&self` rather than `&mut self` deliberately: the renderer
/// computes frames in parallel and out of order, so an effect that carried
/// state between frames would produce different output run to run. Anything an
/// effect wants to precompute goes in [`Effect::prepare`], which runs once,
/// before any frame, and can see the base and the mask.
pub trait Effect: Send + Sync {
    fn render(&self, dst: &mut Image, ctx: &Context);

    /// Precompute whatever depends on the static base and the mask but not on
    /// the frame. Called once by the renderer during preparation.
    fn prepare(&mut self, _base: &Image, _mask: &Mask, _seed: u32) {}
}

#[derive(Debug)]
pub enum Error {
    UnknownType {
        name: String,
        known: Vec<&'static str>,
    },
    BadParams(String),
    /// A parameter counted per loop that was given a value which cannot close.
    NotWhole {
        effect: &'static str,
        param: &'static str,
        value: i32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownType { name, known } => {
                write!(f, "unknown effect type {name:?} (known: {})", known.join(", "))
            }
            Error::BadParams(m) => write!(f, "bad parameters: {m}"),
            Error::NotWhole { effect, param, value } => write!(
                f,
                "{effect}: {param} is {value}, but it counts whole steps per loop and must be at least 1 \
                 - a fractional or zero count leaves the effect mid-motion at the loop point"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Every effect type, with the one-line description `pixelgen effects` prints.
pub const CATALOG: &[(&str, &str)] = &[
    ("breathe", "Slow global brightness swell"),
    ("drift", "Scroll a region with wraparound, for clouds and parallax"),
    ("flicker", "Brightness/temperature wobble over a light source"),
    ("glow", "Pulsing bloom radiating from bright pixels"),
    ("mist", "Drifting fog/haze from tiling fractal noise"),
    ("palette_cycle", "Rotate a contiguous range of palette entries"),
    ("rain", "Falling rain streaks in parallax layers"),
    ("scanlines", "CRT-style horizontal line darkening"),
    ("shimmer", "Rippling displacement, for water and reflections"),
    ("steam", "Wisps rising out of the masked region"),
    ("sway", "Bend a region side to side, for foliage and hanging things"),
    ("twinkle", "Per-pixel sparkle on the bright points in a region"),
    ("vignette", "Darken toward the edges of the frame"),
];

pub fn names() -> Vec<&'static str> {
    CATALOG.iter().map(|(n, _)| *n).collect()
}

/// Instantiate an effect by the type name a scene file uses.
///
/// A match rather than a registry populated by module initializers: the set of
/// effects is fixed at compile time, so a table that can only be wrong at
/// runtime buys nothing, and this works identically under wasm where link-time
/// registration tricks do not.
pub fn build(name: &str, params: &Value) -> Result<Box<dyn Effect>, Error> {
    match name {
        "breathe" => post::breathe(params),
        "drift" => motion::drift(params),
        "flicker" => light::flicker(params),
        "glow" => light::glow(params),
        "mist" => weather::mist(params),
        "palette_cycle" => light::palette_cycle(params),
        "rain" => weather::rain(params),
        "scanlines" => post::scanlines(params),
        "shimmer" => motion::shimmer(params),
        "steam" => weather::steam(params),
        "sway" => motion::sway(params),
        "twinkle" => light::twinkle(params),
        "vignette" => post::vignette(params),
        _ => Err(Error::UnknownType { name: name.into(), known: names() }),
    }
}

/// Every parameter an effect accepts, at its default value, as the mapping a
/// scene would write under `params:`.
///
/// The editor builds its controls from this, so it has to name exactly the
/// keys [`build`] accepts - the two matches are kept in step by a test. An
/// optional color is present as `null`: absent, with the effect's own tint.
pub fn defaults(name: &str) -> Result<Value, Error> {
    fn of<T: Serialize + Default>() -> Result<Value, Error> {
        let mut v =
            serde_yaml::to_value(T::default()).map_err(|e| Error::BadParams(e.to_string()))?;
        if let Value::Mapping(m) = &mut v {
            for (_, x) in m.iter_mut() {
                *x = shortest(x.clone());
            }
        }
        Ok(v)
    }
    match name {
        "breathe" => of::<post::BreatheCfg>(),
        "drift" => of::<motion::DriftCfg>(),
        "flicker" => of::<light::FlickerCfg>(),
        "glow" => of::<light::GlowCfg>(),
        "mist" => of::<weather::MistCfg>(),
        "palette_cycle" => of::<light::PaletteCycleCfg>(),
        "rain" => of::<weather::RainCfg>(),
        "scanlines" => of::<post::ScanlinesCfg>(),
        "shimmer" => of::<motion::ShimmerCfg>(),
        "steam" => of::<weather::SteamCfg>(),
        "sway" => of::<motion::SwayCfg>(),
        "twinkle" => of::<light::TwinkleCfg>(),
        "vignette" => of::<post::VignetteCfg>(),
        _ => Err(Error::UnknownType { name: name.into(), known: names() }),
    }
}

/// Every parameter is an `f32`, and widening one to the `f64` YAML stores
/// writes `0.3` as `0.30000001192092896`. Going through the `f32`'s own
/// shortest decimal gives back the number that was written in the source.
fn shortest(v: Value) -> Value {
    match v.as_f64() {
        Some(f) if !v.is_i64() && !v.is_u64() => {
            Value::from((f as f32).to_string().parse::<f64>().unwrap_or(f))
        }
        _ => v,
    }
}

/// The values a string parameter is limited to, for the few that are an enum
/// rather than free text or a color. Empty for everything else.
pub fn choices(name: &str, param: &str) -> &'static [&'static str] {
    match (name, param) {
        ("sway", "anchor") => &["top", "bottom"],
        ("shimmer", "axis") => &["x", "y"],
        _ => &[],
    }
}

/// Deserialize an effect's parameters, tolerating an absent block so every
/// effect can be listed with nothing but its type.
pub(crate) fn decode<T: DeserializeOwned + Default>(params: &Value) -> Result<T, Error> {
    if params.is_null() {
        return Ok(T::default());
    }
    serde_yaml::from_value(params.clone()).map_err(|e| Error::BadParams(e.to_string()))
}

/// Parse an optional hex parameter, falling back when unset.
pub(crate) fn color(hex: &Option<String>, fallback: Rgb) -> Result<Rgb, Error> {
    match hex {
        None => Ok(fallback),
        Some(h) => parse_hex(h).map_err(|e| Error::BadParams(e.to_string())),
    }
}

/// Check a parameter that counts whole steps per loop.
///
/// Go silently clamped these to 1, which turned a scene-file mistake into a
/// loop that ran at the wrong speed for no stated reason. Reporting it names
/// the parameter instead.
pub(crate) fn whole(effect: &'static str, param: &'static str, v: i32) -> Result<i32, Error> {
    if v < 1 {
        return Err(Error::NotWhole { effect, param, value: v });
    }
    Ok(v)
}
