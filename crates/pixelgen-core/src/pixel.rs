//! The canvas every stage of the renderer operates on.

/// A non-premultiplied sRGB canvas with one `f32` per channel in the range
/// `[0,1]`.
///
/// Effects blend directly in sRGB space, which is what pixel-art palettes are
/// authored in; going through linear light would wash out the flat color steps
/// the style depends on.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub pix: Vec<f32>, // len == w * h * 3
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Image {
    pub fn new(w: usize, h: usize) -> Self {
        Image { w, h, pix: vec![0.0; w * h * 3] }
    }

    #[inline]
    pub fn offset(&self, x: usize, y: usize) -> usize {
        (y * self.w + x) * 3
    }

    #[inline]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> (f32, f32, f32) {
        let o = self.offset(x, y);
        (self.pix[o], self.pix[o + 1], self.pix[o + 2])
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, r: f32, g: f32, b: f32) {
        let o = self.offset(x, y);
        self.pix[o] = r;
        self.pix[o + 1] = g;
        self.pix[o + 2] = b;
    }

    /// Blend a color over a pixel at coverage `a`.
    ///
    /// Bounds and range are handled here so that effects, which mostly compute
    /// positions from noise and phase, never have to special-case the edge of
    /// the canvas.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, r: f32, g: f32, b: f32, a: f32) {
        if !self.contains(x, y) || a <= 0.0 {
            return;
        }
        let a = a.clamp(0.0, 1.0);
        let o = self.offset(x as usize, y as usize);
        self.pix[o] += (r - self.pix[o]) * a;
        self.pix[o + 1] += (g - self.pix[o + 1]) * a;
        self.pix[o + 2] += (b - self.pix[o + 2]) * a;
    }

    /// Sample with wrapping in x and clamping in y.
    ///
    /// Horizontal motion in these scenes is a loop that should come back
    /// around, while vertical motion runs into the ground or the sky and should
    /// hold at the edge instead.
    pub fn sample_wrap(&self, x: i32, y: i32) -> (f32, f32, f32) {
        let xx = x.rem_euclid(self.w as i32) as usize;
        let yy = y.clamp(0, self.h as i32 - 1) as usize;
        self.get(xx, yy)
    }

    pub fn luma_at(&self, x: usize, y: usize) -> f32 {
        let (r, g, b) = self.get(x, y);
        luma(r, g, b)
    }
}

/// Rec. 601 luma. The renderer uses it for brightness-derived masks and for
/// ordering a palette, both of which want perceived brightness rather than a
/// channel average.
#[inline]
pub fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Hermite interpolation between two edges, which may be given in either order
/// so that a caller can reverse a gradient by swapping them.
#[inline]
pub fn smooth_step(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < f32::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn fract(v: f32) -> f32 {
    v - v.floor()
}
