//! Decoding source photographs and writing stills.
//!
//! The core works in a float canvas and knows nothing about file formats; this
//! is the whole of the conversion, in one place, so the wasm build can supply
//! its own without touching anything else.

use std::error::Error;
use std::path::Path;

use image::{ImageBuffer, Rgb as ImgRgb, RgbImage};
use pixelgen_core::pixel::Image;
use pixelgen_core::resample;

pub fn load(path: &Path) -> Result<Image, Box<dyn Error>> {
    let src = image::open(path).map_err(|e| format!("decoding {}: {e}", path.display()))?.to_rgb8();
    let (w, h) = (src.width() as usize, src.height() as usize);
    let mut im = Image::new(w, h);
    for (i, p) in src.pixels().enumerate() {
        im.pix[i * 3] = from8(p[0]);
        im.pix[i * 3 + 1] = from8(p[1]);
        im.pix[i * 3 + 2] = from8(p[2]);
    }
    Ok(im)
}

/// Saves a single frame, applying the integer upscale.
pub fn write_png(im: &Image, path: &Path, scale: usize) -> Result<(), Box<dyn Error>> {
    let scaled;
    let im = if scale > 1 {
        scaled = resample::upscale(im, scale);
        &scaled
    } else {
        im
    };
    to_rgb(im).save(path).map_err(|e| format!("writing {}: {e}", path.display()).into())
}

pub fn to_rgb(im: &Image) -> RgbImage {
    ImageBuffer::from_fn(im.w as u32, im.h as u32, |x, y| {
        let (r, g, b) = im.get(x as usize, y as usize);
        ImgRgb([to8(r), to8(g), to8(b)])
    })
}

#[inline]
pub fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

#[inline]
fn from8(v: u8) -> f32 {
    v as f32 / 255.0
}
