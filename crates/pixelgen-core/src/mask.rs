//! Declarative regions.
//!
//! Regions are the substitute for scene understanding: instead of a model
//! telling the renderer where the sky or the lamp is, the author states it once
//! in normalized coordinates or as a property of the base image - bright
//! pixels, pixels near a colour, pixels cooler than their surroundings.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::palette::parse_hex;
use crate::pixel::{luma, smooth_step, Image};

/// Coverage in `[0,1]` per pixel.
///
/// Fractional coverage matters: it lets effects fade out at region edges
/// instead of stopping on a hard line, which would advertise the rectangle the
/// author drew.
#[derive(Clone, Debug)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
}

impl Mask {
    pub fn new(w: usize, h: usize) -> Self {
        Mask { w, h, a: vec![0.0; w * h] }
    }

    pub fn full(w: usize, h: usize) -> Self {
        Mask { w, h, a: vec![1.0; w * h] }
    }

    #[inline]
    pub fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return 0.0;
        }
        self.a[y as usize * self.w + x as usize]
    }

    /// Whether the mask covers nothing, letting the renderer skip an effect
    /// entirely rather than iterate every pixel for no result.
    pub fn is_empty(&self) -> bool {
        self.a.iter().all(|v| *v <= 0.0)
    }

    /// The tight bounding box of non-zero coverage, so effects iterate only the
    /// region they affect.
    pub fn bounds(&self) -> (usize, usize, usize, usize) {
        let (mut x0, mut y0, mut x1, mut y1) = (self.w, self.h, 0usize, 0usize);
        let mut any = false;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.a[y * self.w + x] > 0.0 {
                    any = true;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if !any {
            return (0, 0, 0, 0);
        }
        (x0, y0, x1 + 1, y1 + 1)
    }
}

/// The serialized form of a region. Exactly one selector should be set, or
/// `all`/`any` for composition; modifiers apply to the result.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Names a region declared in the scene's top-level `regions` block.
    ///
    /// It is the only selector that is not self-contained, and it exists so a
    /// region worked out once - which is most of the effort in a scene - can be
    /// used by several layers without being restated and drifting out of sync.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub r#ref: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ellipse: Option<Rect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub polygon: Vec<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luma: Option<Range>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorIn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chroma: Option<Chroma>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<Band>,

    /// `all` intersects (minimum coverage), `any` unions (maximum).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<Spec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<Spec>,

    /// Modifiers, applied in this order.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
    /// Blur radius in pixel-grid cells.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub feather: f32,
    /// Multiplies coverage; 0 means 1.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gain: f32,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// Normalized `[0,1]` coordinates, so a scene file survives a change of width
/// without every region drifting.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub h: f32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    #[serde(default)]
    pub min: f32,
    #[serde(default)]
    pub max: f32,
}

/// Selects pixels near a colour, the practical way to grab "the neon sign" or
/// "the water" out of an already-quantized base.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorIn {
    pub hex: String,
    #[serde(default)]
    pub tolerance: f32,
}

/// Selects by colour temperature rather than by a particular colour. Positive
/// values are cool, negative values are warm, and zero is neutral.
///
/// This is the practical way to separate an interior from what is outside it,
/// which is a distinction rectangles cannot draw: a warm lamplit room and the
/// blue daylight beyond its window overlap completely in screen position but
/// not at all in temperature. Selecting the exterior that way also yields
/// correct occlusion for free, because anything in the room - a figure, a
/// plant, the frame itself - fails the test and is cut out of the region.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chroma {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    /// Width of the soft edge at each bound.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub soft: f32,
}

/// How cool a colour is: blue minus red, normalized by brightness.
///
/// The normalization is what makes the measure usable. A shadowed blue tree
/// trunk and a brightly lit patch of the same fog are equally cool, but their
/// raw blue-minus-red differs by a factor of three, so an un-normalized
/// threshold that catches the fog drops every shadow in the same region.
/// Dividing by brightness - offset so that near-black pixels, where the hue is
/// mostly quantization noise, stay bounded - puts both at the same value.
#[inline]
pub fn chroma_of(r: f32, g: f32, b: f32) -> f32 {
    (b - r) / (luma(r, g, b) + 0.1)
}

/// A soft horizontal or vertical gradient, for things like "sky fading into the
/// horizon" where a hard edge would be obvious.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    #[default]
    Y,
    X,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Band {
    pub axis: Axis,
    /// Where coverage begins and where it reaches full. Writing `end` below
    /// `start` reverses the ramp, which is how the generated scene template
    /// fades fog in above a horizon.
    pub start: f32,
    pub end: f32,
}

impl Default for Band {
    fn default() -> Self {
        Band { axis: Axis::Y, start: 0.0, end: 1.0 }
    }
}

/// The named regions a `ref` selector can resolve.
pub type Registry = HashMap<String, Spec>;

#[derive(Debug)]
pub enum Error {
    UnknownRegion(String),
    Cycle(Vec<String>),
    RefWithSelector { name: String, selector: &'static str },
    BadColor(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownRegion(n) => write!(f, "no region named {n:?} is defined"),
            Error::Cycle(path) => write!(f, "region {:?} refers to itself via {:?}", path[0], path),
            Error::RefWithSelector { name, selector } => write!(
                f,
                "region reference {name:?} cannot also set {selector:?}; wrap both in an \"all\" instead"
            ),
            Error::BadColor(s) => write!(f, "color selector: invalid hex colour {s:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// Evaluates specs against one base image, resolving named regions and caching
/// them so a region shared by several layers is computed once.
pub struct Builder<'a> {
    pub base: &'a Image,
    pub regions: &'a Registry,
    cache: HashMap<String, Mask>,
}

static EMPTY_REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();

impl<'a> Builder<'a> {
    pub fn new(base: &'a Image, regions: &'a Registry) -> Self {
        Builder { base, regions, cache: HashMap::new() }
    }

    /// A builder for specs that contain no `ref`.
    pub fn plain(base: &'a Image) -> Self {
        Builder::new(base, EMPTY_REGISTRY.get_or_init(Registry::new))
    }

    pub fn build(&mut self, spec: Option<&Spec>) -> Result<Mask, Error> {
        match spec {
            None => Ok(Mask::full(self.base.w, self.base.h)),
            Some(s) => self.build_inner(s, &mut Vec::new()),
        }
    }

    /// `stack` carries the regions currently being resolved, which is what lets
    /// a region that refers to itself be reported instead of overflowing.
    fn build_inner(&mut self, s: &Spec, stack: &mut Vec<String>) -> Result<Mask, Error> {
        let mut m = self.shape(s, stack)?;
        if s.invert {
            for v in m.a.iter_mut() {
                *v = 1.0 - *v;
            }
        }
        if s.feather > 0.0 {
            m = blur(&m, s.feather);
        }
        if s.gain != 0.0 && s.gain != 1.0 {
            for v in m.a.iter_mut() {
                *v = (*v * s.gain).clamp(0.0, 1.0);
            }
        }
        Ok(m)
    }

    fn shape(&mut self, s: &Spec, stack: &mut Vec<String>) -> Result<Mask, Error> {
        let (w, h) = (self.base.w, self.base.h);
        if !s.r#ref.is_empty() {
            return self.region(&s.r#ref, stack);
        }
        if !s.all.is_empty() {
            let mut m = Mask::full(w, h);
            for sub in &s.all {
                let x = self.build_inner(sub, stack)?;
                for (i, v) in m.a.iter_mut().enumerate() {
                    *v = v.min(x.a[i]);
                }
            }
            return Ok(m);
        }
        if !s.any.is_empty() {
            let mut m = Mask::new(w, h);
            for sub in &s.any {
                let x = self.build_inner(sub, stack)?;
                for (i, v) in m.a.iter_mut().enumerate() {
                    *v = v.max(x.a[i]);
                }
            }
            return Ok(m);
        }
        if let Some(r) = &s.rect {
            return Ok(from_rect(*r, w, h));
        }
        if let Some(r) = &s.ellipse {
            return Ok(from_ellipse(*r, w, h));
        }
        if !s.polygon.is_empty() {
            return Ok(from_polygon(&s.polygon, w, h));
        }
        if let Some(r) = &s.luma {
            return Ok(from_luma(*r, self.base));
        }
        if let Some(c) = &s.color {
            return from_color(c, self.base);
        }
        if let Some(c) = &s.chroma {
            return Ok(from_chroma(*c, self.base));
        }
        if let Some(b) = &s.band {
            return Ok(from_band(b, w, h));
        }
        Ok(Mask::full(w, h))
    }

    fn region(&mut self, name: &str, stack: &mut Vec<String>) -> Result<Mask, Error> {
        if stack.iter().any(|n| n == name) {
            let mut path = stack.clone();
            path.push(name.to_string());
            return Err(Error::Cycle(path));
        }
        if let Some(m) = self.cache.get(name) {
            // Handed out as a copy: the caller's modifiers write in place, and
            // a shared region must not be altered by whichever layer used it
            // first.
            return Ok(m.clone());
        }
        let spec = self
            .regions
            .get(name)
            .ok_or_else(|| Error::UnknownRegion(name.to_string()))?
            .clone();
        stack.push(name.to_string());
        let m = self.build_inner(&spec, stack)?;
        stack.pop();
        self.cache.insert(name.to_string(), m.clone());
        Ok(m)
    }
}

/// Reports unresolvable references and reference cycles. It is separate from
/// building so a scene file can be rejected at load time, before an image has
/// been read, rather than partway through a render.
pub fn validate(regions: &Registry, specs: &[Option<&Spec>]) -> Result<(), Error> {
    let mut names: Vec<&String> = regions.keys().collect();
    names.sort(); // deterministic error for a scene with several faults
    for name in names {
        walk(regions, &regions[name], &mut vec![name.clone()])?;
    }
    for s in specs.iter().flatten() {
        walk(regions, s, &mut Vec::new())?;
    }
    Ok(())
}

fn walk(regions: &Registry, s: &Spec, stack: &mut Vec<String>) -> Result<(), Error> {
    if !s.r#ref.is_empty() {
        if let Some(sel) = conflict(s) {
            return Err(Error::RefWithSelector { name: s.r#ref.clone(), selector: sel });
        }
        if stack.iter().any(|n| *n == s.r#ref) {
            let mut path = stack.clone();
            path.push(s.r#ref.clone());
            return Err(Error::Cycle(path));
        }
        let sub = regions
            .get(&s.r#ref)
            .ok_or_else(|| Error::UnknownRegion(s.r#ref.clone()))?;
        stack.push(s.r#ref.clone());
        walk(regions, sub, stack)?;
        stack.pop();
    }
    for sub in s.all.iter().chain(s.any.iter()) {
        walk(regions, sub, stack)?;
    }
    Ok(())
}

/// Names a selector set alongside `ref`, which building would silently ignore.
/// Modifiers are not conflicts: they are meant to apply to the region.
fn conflict(s: &Spec) -> Option<&'static str> {
    if s.rect.is_some() {
        Some("rect")
    } else if s.ellipse.is_some() {
        Some("ellipse")
    } else if !s.polygon.is_empty() {
        Some("polygon")
    } else if s.luma.is_some() {
        Some("luma")
    } else if s.color.is_some() {
        Some("color")
    } else if s.chroma.is_some() {
        Some("chroma")
    } else if s.band.is_some() {
        Some("band")
    } else if !s.all.is_empty() {
        Some("all")
    } else if !s.any.is_empty() {
        Some("any")
    } else {
        None
    }
}

fn from_rect(r: Rect, w: usize, h: usize) -> Mask {
    let mut m = Mask::new(w, h);
    let x0 = (r.x * w as f32) as i32;
    let y0 = (r.y * h as f32) as i32;
    let x1 = ((r.x + r.w) * w as f32 + 0.5) as i32;
    let y1 = ((r.y + r.h) * h as f32 + 0.5) as i32;
    for y in y0.max(0)..y1.min(h as i32) {
        for x in x0.max(0)..x1.min(w as i32) {
            m.a[y as usize * w + x as usize] = 1.0;
        }
    }
    m
}

fn from_ellipse(r: Rect, w: usize, h: usize) -> Mask {
    let mut m = Mask::new(w, h);
    let cx = (r.x + r.w / 2.0) * w as f32;
    let cy = (r.y + r.h / 2.0) * h as f32;
    let rx = r.w / 2.0 * w as f32;
    let ry = r.h / 2.0 * h as f32;
    if rx <= 0.0 || ry <= 0.0 {
        return m;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 + 0.5 - cx) / rx;
            let dy = (y as f32 + 0.5 - cy) / ry;
            let d = dx * dx + dy * dy;
            if d <= 1.0 {
                // Soften the outer 20% so the ellipse does not alias into a
                // staircase at the pixel-grid resolution.
                m.a[y * w + x] = smooth_step(1.0, 0.8, d);
            }
        }
    }
    m
}

fn from_polygon(pts: &[Point], w: usize, h: usize) -> Mask {
    let mut m = Mask::new(w, h);
    let n = pts.len();
    if n < 3 {
        return m;
    }
    let px: Vec<f32> = pts.iter().map(|p| p.x * w as f32).collect();
    let py: Vec<f32> = pts.iter().map(|p| p.y * h as f32).collect();
    for y in 0..h {
        let fy = y as f32 + 0.5;
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let mut inside = false;
            let mut j = n - 1;
            for i in 0..n {
                // Standard even-odd crossing test.
                if (py[i] > fy) != (py[j] > fy) {
                    let xi = px[i] + (fy - py[i]) / (py[j] - py[i]) * (px[j] - px[i]);
                    if fx < xi {
                        inside = !inside;
                    }
                }
                j = i;
            }
            if inside {
                m.a[y * w + x] = 1.0;
            }
        }
    }
    m
}

fn from_luma(r: Range, base: &Image) -> Mask {
    let max = if r.max == 0.0 { 1.0 } else { r.max };
    let mut m = Mask::new(base.w, base.h);
    for i in 0..m.a.len() {
        let o = i * 3;
        let l = luma(base.pix[o], base.pix[o + 1], base.pix[o + 2]);
        if l >= r.min && l <= max {
            m.a[i] = 1.0;
        }
    }
    m
}

fn from_color(c: &ColorIn, base: &Image) -> Result<Mask, Error> {
    let target = parse_hex(&c.hex).map_err(|_| Error::BadColor(c.hex.clone()))?;
    let tol = if c.tolerance <= 0.0 { 0.12 } else { c.tolerance };
    // Compare against squared distance summed over three channels.
    let tol2 = tol * tol * 3.0;
    let mut m = Mask::new(base.w, base.h);
    for i in 0..m.a.len() {
        let o = i * 3;
        let dr = base.pix[o] - target.r;
        let dg = base.pix[o + 1] - target.g;
        let db = base.pix[o + 2] - target.b;
        let d2 = dr * dr + dg * dg + db * db;
        if d2 <= tol2 {
            m.a[i] = smooth_step(tol2, tol2 * 0.5, d2);
        }
    }
    Ok(m)
}

fn from_chroma(c: Chroma, base: &Image) -> Mask {
    let soft = if c.soft <= 0.0 { 0.08 } else { c.soft };
    let mut m = Mask::new(base.w, base.h);
    for i in 0..m.a.len() {
        let o = i * 3;
        let v = chroma_of(base.pix[o], base.pix[o + 1], base.pix[o + 2]);
        let mut a = 1.0f32;
        if let Some(lo) = c.min {
            a = a.min(smooth_step(lo - soft, lo + soft, v));
        }
        if let Some(hi) = c.max {
            a = a.min(smooth_step(hi + soft, hi - soft, v));
        }
        m.a[i] = a;
    }
    m
}

fn from_band(b: &Band, w: usize, h: usize) -> Mask {
    let mut m = Mask::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = if b.axis == Axis::X {
                (x as f32 + 0.5) / w as f32
            } else {
                (y as f32 + 0.5) / h as f32
            };
            m.a[y * w + x] = smooth_step(b.start, b.end, p);
        }
    }
    m
}

/// A separable box blur repeated three times, which approximates a Gaussian
/// closely enough for feathering at a fraction of the cost.
fn blur(m: &Mask, radius: f32) -> Mask {
    let r = radius.round() as i32;
    if r < 1 {
        return m.clone();
    }
    let mut cur = m.clone();
    for _ in 0..3 {
        cur = box_pass(&cur, r, true);
        cur = box_pass(&cur, r, false);
    }
    cur
}

fn box_pass(m: &Mask, r: i32, horizontal: bool) -> Mask {
    let mut out = Mask::new(m.w, m.h);
    for y in 0..m.h {
        for x in 0..m.w {
            let (mut sum, mut n) = (0.0, 0.0);
            for d in -r..=r {
                let (xx, yy) = if horizontal {
                    (x as i32 + d, y as i32)
                } else {
                    (x as i32, y as i32 + d)
                };
                if xx < 0 || yy < 0 || xx as usize >= m.w || yy as usize >= m.h {
                    continue;
                }
                sum += m.a[yy as usize * m.w + xx as usize];
                n += 1.0;
            }
            out.a[y * m.w + x] = sum / n;
        }
    }
    out
}
