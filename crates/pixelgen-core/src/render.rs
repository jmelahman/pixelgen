//! Turns a scene plus a source photograph into the frames of one seamless loop.
//!
//! Nothing here touches the filesystem. The source image arrives already
//! decoded, so the same code path serves the CLI and the browser.

use std::fmt;

use crate::effect::{self, Context, Effect};
use crate::mask::{Builder, Mask};
use crate::palette::{self, Matcher, Palette};
use crate::pixel::Image;
use crate::resample;
use crate::scene::Scene;

#[derive(Debug)]
pub enum Error {
    Layer { label: String, source: effect::Error },
    Mask { label: String, source: crate::mask::Error },
    EmptyMask { label: String },
    Palette(palette::HexError),
    /// The scene names a palette file, which only the host can open.
    UnresolvedPaletteFile(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Layer { label, source } => write!(f, "layer {label:?}: {source}"),
            Error::Mask { label, source } => write!(f, "layer {label:?}: {source}"),
            Error::EmptyMask { label } => write!(f, "layer {label:?}: mask selects no pixels"),
            Error::Palette(e) => write!(f, "palette: {e}"),
            Error::UnresolvedPaletteFile(p) => write!(
                f,
                "palette file {p:?} was not loaded - call Scene::set_palette_hex with its contents"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Reduces a full-resolution source to the pixel grid and quantizes it.
///
/// This is the whole static half of the pipeline, and is also what `pixelgen
/// pixelate` exposes on its own.
pub fn pixelate(src: &Image, s: &Scene) -> Result<(Image, Matcher), Error> {
    let filtered;
    let work = if s.prepare.median {
        filtered = resample::median3(src);
        &filtered
    } else {
        src
    };

    let (w, h) = resample::fit_size(work.w, work.h, s.width);
    let mut base = resample::downscale(work, w, h);

    if s.prepare.saturation != 1.0 {
        resample::saturate(&mut base, s.prepare.saturation);
    }
    if s.prepare.contrast != 1.0 {
        resample::contrast(&mut base, s.prepare.contrast);
    }

    let pal = resolve_palette(&base, s)?;
    let matcher = Matcher::new(pal);
    matcher.dither(&mut base, s.palette.dither);
    Ok((base, matcher))
}

fn resolve_palette(base: &Image, s: &Scene) -> Result<Palette, Error> {
    if !s.palette.hex.is_empty() {
        let refs: Vec<&str> = s.palette.hex.iter().map(String::as_str).collect();
        return palette::parse_hex_all(&refs).map_err(Error::Palette);
    }
    if !s.palette.file.is_empty() {
        return Err(Error::UnresolvedPaletteFile(s.palette.file.clone()));
    }
    Ok(palette::extract(base, s.palette.colors, s.seed))
}

struct Layer {
    label: String,
    effect: Box<dyn Effect>,
    mask: Mask,
    seed: u32,
}

/// Everything that is computed once and shared by all frames.
pub struct Prepared {
    pub base: Image,
    pub matcher: Matcher,
    pub frames: usize,
    layers: Vec<Layer>,
}

/// Builds the base image, the palette and every layer's mask and effect.
pub fn prepare(src: &Image, s: &Scene) -> Result<Prepared, Error> {
    let (base, matcher) = pixelate(src, s)?;

    // One builder for the whole scene, so a region shared by several layers is
    // evaluated once.
    let mut builder = Builder::new(&base, &s.regions);
    let mut layers = Vec::new();
    for (i, l) in s.layers.iter().enumerate() {
        if l.disable {
            continue;
        }
        let label = l.label(i);
        let mut eff = effect::build(&l.r#type, &l.params)
            .map_err(|e| Error::Layer { label: label.clone(), source: e })?;
        let mask = builder
            .build(l.mask.as_ref())
            .map_err(|e| Error::Mask { label: label.clone(), source: e })?;
        if mask.is_empty() {
            return Err(Error::EmptyMask { label });
        }
        // Deriving each layer's seed from the scene seed and its index keeps
        // layers independent while the whole scene stays reproducible from one
        // number.
        let seed = s.seed.wrapping_mul(2654435761).wrapping_add(i as u32 * 40503);
        eff.prepare(&base, &mask, seed);
        layers.push(Layer { label, effect: eff, mask, seed });
    }

    Ok(Prepared { base, matcher, frames: s.loop_.frames(), layers })
}

impl Prepared {
    /// The resolved mask of the `i`th drawn layer, indexed as
    /// [`Prepared::layer_names`]. The editor draws these over the base so a
    /// region can be seen before it is animated.
    pub fn layer_mask(&self, i: usize) -> Option<&Mask> {
        self.layers.get(i).map(|l| &l.mask)
    }

    /// The names of the layers that will actually be drawn, in order.
    pub fn layer_names(&self) -> Vec<&str> {
        self.layers.iter().map(|l| l.label.as_str()).collect()
    }

    pub fn is_animated(&self) -> bool {
        !self.layers.is_empty()
    }

    /// Renders a single frame of the loop.
    pub fn frame(&self, i: usize) -> Image {
        let mut dst = self.base.clone();
        for l in &self.layers {
            let ctx = Context {
                t: i as f32 / self.frames as f32,
                frame: i,
                frames: self.frames,
                base: &self.base,
                mask: &l.mask,
                matcher: &self.matcher,
                seed: l.seed,
            };
            l.effect.render(&mut dst, &ctx);
        }
        // Effects blend in continuous colour; snapping afterwards is what keeps
        // every frame strictly inside the palette, so rain over a wall uses the
        // same handful of colours the wall was already made of.
        self.matcher.snap(&mut dst);
        dst
    }

    /// Renders the whole loop.
    ///
    /// Frames are fully independent by construction, which is why effects
    /// render through `&self` and derive everything from the context.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn all_frames(&self, progress: impl Fn(usize, usize) + Sync) -> Vec<Image> {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let done = AtomicUsize::new(0);
        (0..self.frames)
            .into_par_iter()
            .map(|i| {
                let f = self.frame(i);
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, self.frames);
                f
            })
            .collect()
    }

    /// The single-threaded path. wasm has no threads without cross-origin
    /// isolation, and the browser UI renders one frame at a time anyway.
    #[cfg(target_arch = "wasm32")]
    pub fn all_frames(&self, progress: impl Fn(usize, usize)) -> Vec<Image> {
        (0..self.frames)
            .map(|i| {
                let f = self.frame(i);
                progress(i + 1, self.frames);
                f
            })
            .collect()
    }
}
