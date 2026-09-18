//! Scaling between the source photograph and the pixel grid.

use crate::pixel::{luma, Image};

/// Downscale with a box filter.
///
/// A box filter is the right choice precisely because it is the crude one.
/// Bilinear or Lanczos would preserve detail and gradients inside each
/// destination cell, which is the opposite of what is wanted: a cell has to
/// come out as one flat colour, because that is what makes it read as a pixel
/// rather than as a blurred photograph.
pub fn downscale(src: &Image, w: usize, h: usize) -> Image {
    let mut dst = Image::new(w, h);
    let sx = src.w as f32 / w as f32;
    let sy = src.h as f32 / h as f32;
    for y in 0..h {
        let y0 = (y as f32 * sy) as usize;
        let y1 = (((y + 1) as f32 * sy).ceil() as usize).min(src.h).max(y0 + 1);
        for x in 0..w {
            let x0 = (x as f32 * sx) as usize;
            let x1 = (((x + 1) as f32 * sx).ceil() as usize).min(src.w).max(x0 + 1);
            let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
            let mut n = 0.0;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let o = src.offset(xx, yy);
                    r += src.pix[o];
                    g += src.pix[o + 1];
                    b += src.pix[o + 2];
                    n += 1.0;
                }
            }
            dst.set(x, y, r / n, g / n, b / n);
        }
    }
    dst
}

/// Upscale by an integer factor, nearest-neighbour.
///
/// Nearest is not a fallback here, it is the point: every output pixel must be
/// an exact copy of a grid cell, with hard edges between cells.
pub fn upscale(src: &Image, factor: usize) -> Image {
    if factor <= 1 {
        return src.clone();
    }
    let mut dst = Image::new(src.w * factor, src.h * factor);
    for y in 0..dst.h {
        for x in 0..dst.w {
            let (r, g, b) = src.get(x / factor, y / factor);
            dst.set(x, y, r, g, b);
        }
    }
    dst
}

/// Fit a width to the source aspect ratio, returning at least one row.
pub fn fit_size(src_w: usize, src_h: usize, w: usize) -> (usize, usize) {
    let h = ((w as f32 * src_h as f32 / src_w as f32).round() as usize).max(1);
    (w, h)
}

/// A 3x3 per-channel median, run at full resolution before downscaling.
///
/// Photographic noise survives a box filter as speckle and then gets locked in
/// by quantization, where it reads as stray mis-coloured cells. Removing it
/// first is much cheaper than trying to clean it up afterwards.
pub fn median3(src: &Image) -> Image {
    let mut dst = Image::new(src.w, src.h);
    let mut win = [0.0f32; 9];
    for y in 0..src.h {
        for x in 0..src.w {
            for c in 0..3 {
                let mut n = 0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let xx = (x as i32 + dx).clamp(0, src.w as i32 - 1) as usize;
                        let yy = (y as i32 + dy).clamp(0, src.h as i32 - 1) as usize;
                        win[n] = src.pix[src.offset(xx, yy) + c];
                        n += 1;
                    }
                }
                win.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let o = dst.offset(x, y);
                dst.pix[o + c] = win[4];
            }
        }
    }
    dst
}

/// Push saturation around each pixel's own luma.
pub fn saturate(img: &mut Image, amount: f32) {
    if (amount - 1.0).abs() < f32::EPSILON {
        return;
    }
    for i in (0..img.pix.len()).step_by(3) {
        let l = luma(img.pix[i], img.pix[i + 1], img.pix[i + 2]);
        for c in 0..3 {
            img.pix[i + c] = (l + (img.pix[i + c] - l) * amount).clamp(0.0, 1.0);
        }
    }
}

/// Push contrast around mid grey.
pub fn contrast(img: &mut Image, amount: f32) {
    if (amount - 1.0).abs() < f32::EPSILON {
        return;
    }
    for v in img.pix.iter_mut() {
        *v = (0.5 + (*v - 0.5) * amount).clamp(0.0, 1.0);
    }
}
