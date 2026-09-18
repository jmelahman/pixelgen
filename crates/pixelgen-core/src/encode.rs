//! Encodes rendered frames as a GIF.
//!
//! Here rather than in the CLI because the browser needs the same file: the
//! encoder writes to a byte buffer and opens nothing, which is what lets both
//! callers hand the result wherever they keep output.

use crate::palette::{Matcher, Palette};
use crate::pixel::Image;
use crate::resample;

/// Encodes the frames as a looping GIF.
///
/// Every frame is already exactly on a palette of at most 256 colors, which
/// is precisely GIF's own model, so the frames go in as indices and no color
/// is re-decided on the way out.
pub fn gif(frames: &[Image], pal: &Palette, scale: usize, fps: usize) -> Result<Vec<u8>, String> {
    if frames.is_empty() {
        return Err("no frames to encode".into());
    }
    if pal.is_empty() {
        return Err("gif output needs a palette".into());
    }
    if pal.len() > 256 {
        return Err(format!("gif supports at most 256 colors, the scene has {}", pal.len()));
    }

    let table: Vec<u8> = pal.iter().flat_map(|c| [to8(c.r), to8(c.g), to8(c.b)]).collect();
    let matcher = Matcher::new(pal.clone());
    let scale = scale.max(1);
    let (w, h) = ((frames[0].w * scale) as u16, (frames[0].h * scale) as u16);

    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, w, h, &table).map_err(err)?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(err)?;

        // GIF delays are in hundredths of a second, so only a subset of frame
        // rates is representable exactly. Two is the floor most viewers honour.
        let delay = (((100.0 / fps.max(1) as f32) + 0.5) as u16).max(2);

        for f in frames {
            let up = resample::upscale(f, scale);
            let idx: Vec<u8> = (0..up.w * up.h)
                .map(|i| {
                    let (x, y) = (i % up.w, i / up.w);
                    let (r, g, b) = up.get(x, y);
                    matcher.index(r, g, b) as u8
                })
                .collect();
            let mut frame = gif::Frame::from_indexed_pixels(w, h, idx, None);
            frame.delay = delay;
            frame.dispose = gif::DisposalMethod::Keep;
            enc.write_frame(&frame).map_err(err)?;
        }
    }
    Ok(out)
}

fn err(e: gif::EncodingError) -> String {
    e.to_string()
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}
