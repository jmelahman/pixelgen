//! SVG-style path data, in normalized `0..1` coordinates.
//!
//! A drawn outline is stored as one `d` string rather than as a list of
//! points: the scene file is written in block style, where every point of a
//! `polygon` costs two lines, and a freehand lasso has hundreds of them.

use std::fmt;

/// A point in whatever space the caller is working in: normalized while
/// parsing, grid cells once flattened.
pub type Pt = (f32, f32);

#[derive(Clone, Debug)]
enum Seg {
    Line(Pt),
    Quad(Pt, Pt),
    Cubic(Pt, Pt, Pt),
}

#[derive(Clone, Debug)]
struct Subpath {
    start: Pt,
    segs: Vec<Seg>,
    closed: bool,
}

/// A parsed path: one or more subpaths of lines and Bezier curves.
#[derive(Clone, Debug, Default)]
pub struct Path {
    subs: Vec<Subpath>,
}

#[derive(Debug)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A flattened subpath, in the scaled space `flatten` was asked for.
pub struct Polyline {
    pub pts: Vec<Pt>,
    pub closed: bool,
}

impl Path {
    /// Parses `M L H V C S Q T Z`, absolute and relative. Arcs are left out:
    /// nothing in the editor writes them, and a hand-written arc is better
    /// expressed as an `ellipse`.
    pub fn parse(d: &str) -> Result<Path, ParseError> {
        let mut t = Tokens { s: d.as_bytes(), i: 0 };
        let mut subs: Vec<Subpath> = Vec::new();
        let mut cur: Pt = (0.0, 0.0);
        // The reflection point for S and T: the previous segment's last
        // control point, when that segment was the same kind of curve.
        let mut last_ctrl: Option<(u8, Pt)> = None;
        let mut cmd = 0u8;
        loop {
            match t.command() {
                Some(c) => cmd = c,
                None if t.at_end() => break,
                None if cmd == 0 => {
                    return Err(ParseError(format!("path {d:?}: expected a command")))
                }
                // A bare number repeats the previous command; after a moveto
                // it is an implicit lineto, as SVG has it.
                None if cmd == b'M' => cmd = b'L',
                None if cmd == b'm' => cmd = b'l',
                None if cmd.eq_ignore_ascii_case(&b'z') => {
                    return Err(ParseError(format!("path {d:?}: numbers after Z")));
                }
                None => {}
            }
            let rel = cmd.is_ascii_lowercase();
            let off = |p: Pt, cur: Pt| if rel { (p.0 + cur.0, p.1 + cur.1) } else { p };
            let upper = cmd.to_ascii_uppercase();
            if upper != b'M' && upper != b'Z' && subs.is_empty() {
                return Err(ParseError(format!("path {d:?}: must start with M")));
            }
            let e = |what: &str| ParseError(format!("path {d:?}: {what}"));
            match upper {
                b'M' => {
                    let p = off(t.pair().ok_or_else(|| e("M needs x y"))?, cur);
                    subs.push(Subpath { start: p, segs: Vec::new(), closed: false });
                    cur = p;
                    last_ctrl = None;
                }
                b'L' => {
                    let p = off(t.pair().ok_or_else(|| e("L needs x y"))?, cur);
                    push(&mut subs, cur, Seg::Line(p));
                    cur = p;
                    last_ctrl = None;
                }
                b'H' => {
                    let x = t.number().ok_or_else(|| e("H needs x"))?;
                    let p = (if rel { cur.0 + x } else { x }, cur.1);
                    push(&mut subs, cur, Seg::Line(p));
                    cur = p;
                    last_ctrl = None;
                }
                b'V' => {
                    let y = t.number().ok_or_else(|| e("V needs y"))?;
                    let p = (cur.0, if rel { cur.1 + y } else { y });
                    push(&mut subs, cur, Seg::Line(p));
                    cur = p;
                    last_ctrl = None;
                }
                b'C' => {
                    let c1 = off(t.pair().ok_or_else(|| e("C needs three points"))?, cur);
                    let c2 = off(t.pair().ok_or_else(|| e("C needs three points"))?, cur);
                    let p = off(t.pair().ok_or_else(|| e("C needs three points"))?, cur);
                    push(&mut subs, cur, Seg::Cubic(c1, c2, p));
                    cur = p;
                    last_ctrl = Some((b'C', c2));
                }
                b'S' => {
                    let c1 = reflect(last_ctrl, b'C', cur);
                    let c2 = off(t.pair().ok_or_else(|| e("S needs two points"))?, cur);
                    let p = off(t.pair().ok_or_else(|| e("S needs two points"))?, cur);
                    push(&mut subs, cur, Seg::Cubic(c1, c2, p));
                    cur = p;
                    last_ctrl = Some((b'C', c2));
                }
                b'Q' => {
                    let c = off(t.pair().ok_or_else(|| e("Q needs two points"))?, cur);
                    let p = off(t.pair().ok_or_else(|| e("Q needs two points"))?, cur);
                    push(&mut subs, cur, Seg::Quad(c, p));
                    cur = p;
                    last_ctrl = Some((b'Q', c));
                }
                b'T' => {
                    let c = reflect(last_ctrl, b'Q', cur);
                    let p = off(t.pair().ok_or_else(|| e("T needs a point"))?, cur);
                    push(&mut subs, cur, Seg::Quad(c, p));
                    cur = p;
                    last_ctrl = Some((b'Q', c));
                }
                b'Z' => {
                    if let Some(s) = subs.last_mut() {
                        s.closed = true;
                        cur = s.start;
                    }
                    last_ctrl = None;
                }
                _ => return Err(e(&format!("unsupported command {:?}", cmd as char))),
            }
        }
        if subs.is_empty() {
            return Err(ParseError(format!("path {d:?} is empty")));
        }
        Ok(Path { subs })
    }

    /// Every subpath as a polyline in `(x * sx, y * sy)` space, with curves
    /// split until they are within `tol` of straight.
    pub fn flatten(&self, sx: f32, sy: f32, tol: f32) -> Vec<Polyline> {
        let sc = |p: Pt| (p.0 * sx, p.1 * sy);
        self.subs
            .iter()
            .map(|s| {
                let mut pts = vec![sc(s.start)];
                let mut cur = sc(s.start);
                for seg in &s.segs {
                    match *seg {
                        Seg::Line(p) => pts.push(sc(p)),
                        Seg::Quad(c, p) => {
                            // Degree elevation: a quadratic is the cubic with
                            // both controls two-thirds of the way to `c`.
                            let (c, p) = (sc(c), sc(p));
                            let c1 = (
                                cur.0 + (c.0 - cur.0) * 2.0 / 3.0,
                                cur.1 + (c.1 - cur.1) * 2.0 / 3.0,
                            );
                            let c2 = (p.0 + (c.0 - p.0) * 2.0 / 3.0, p.1 + (c.1 - p.1) * 2.0 / 3.0);
                            cubic(&mut pts, cur, c1, c2, p, tol, 0);
                        }
                        Seg::Cubic(c1, c2, p) => {
                            cubic(&mut pts, cur, sc(c1), sc(c2), sc(p), tol, 0)
                        }
                    }
                    cur = *pts.last().expect("a polyline always has its start");
                }
                Polyline { pts, closed: s.closed }
            })
            .collect()
    }
}

/// Appends to the current subpath. A drawing command straight after `Z`
/// starts a new one where the closed one began, as SVG has it.
fn push(subs: &mut Vec<Subpath>, cur: Pt, seg: Seg) {
    if subs.last().is_some_and(|s| s.closed) {
        subs.push(Subpath { start: cur, segs: Vec::new(), closed: false });
    }
    if let Some(s) = subs.last_mut() {
        s.segs.push(seg);
    }
}

fn reflect(last: Option<(u8, Pt)>, kind: u8, cur: Pt) -> Pt {
    match last {
        Some((k, c)) if k == kind => (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1),
        _ => cur,
    }
}

/// Recursive subdivision, stopping once both controls are within `tol` of the
/// chord. The depth limit only matters for degenerate input.
fn cubic(out: &mut Vec<Pt>, p0: Pt, c1: Pt, c2: Pt, p3: Pt, tol: f32, depth: u32) {
    if depth >= 12 || (dist_to_line(c1, p0, p3) <= tol && dist_to_line(c2, p0, p3) <= tol) {
        out.push(p3);
        return;
    }
    let mid = |a: Pt, b: Pt| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let (ab, bc, cd) = (mid(p0, c1), mid(c1, c2), mid(c2, p3));
    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
    let m = mid(abc, bcd);
    cubic(out, p0, ab, abc, m, tol, depth + 1);
    cubic(out, m, bcd, cd, p3, tol, depth + 1);
}

fn dist_to_line(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 {
        return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    }
    ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tokens<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',')
        {
            self.i += 1;
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    /// An SVG number: `-.5.5` is two numbers, which is how a compact path
    /// writer saves bytes.
    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let s = self.s;
        let mut i = self.i;
        if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
            i += 1;
        }
        let mut digits = false;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
            digits = true;
        }
        if i < s.len() && s[i] == b'.' {
            i += 1;
            while i < s.len() && s[i].is_ascii_digit() {
                i += 1;
                digits = true;
            }
        }
        if !digits {
            return None;
        }
        if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
            let mut j = i + 1;
            if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
                j += 1;
            }
            if j < s.len() && s[j].is_ascii_digit() {
                while j < s.len() && s[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        let v = std::str::from_utf8(&s[start..i]).ok()?.parse().ok()?;
        self.i = i;
        Some(v)
    }

    fn pair(&mut self) -> Option<Pt> {
        Some((self.number()?, self.number()?))
    }
}
