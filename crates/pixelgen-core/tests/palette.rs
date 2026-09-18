//! Palette extraction, matching and dithering.

use pixelgen_core::palette::{self, dist2, nearest_in, Matcher, Palette};
use pixelgen_core::pixel::{Image, Rgb};

fn rgb(r: f32, g: f32, b: f32) -> Rgb {
    Rgb { r, g, b }
}

fn in_palette(pal: &Palette, r: f32, g: f32, b: f32) -> bool {
    pal.iter().any(|c| c.r == r && c.g == g && c.b == b)
}

/// The lookup table rounds to 6 bits per channel, so a query sitting on the
/// boundary between two entries can legitimately fall to the other side. What
/// must hold is that the entry returned is never meaningfully worse than the
/// true nearest one - expressed as a bound on distance rather than squared
/// distance, so the number means something: within 5% of a unit cube diagonal.
#[test]
fn the_lookup_table_is_near_optimal() {
    let pal: Palette = vec![
        rgb(0.0, 0.0, 0.0),
        rgb(1.0, 1.0, 1.0),
        rgb(0.8, 0.2, 0.1),
        rgb(0.1, 0.3, 0.7),
        rgb(0.4, 0.5, 0.35),
    ];
    let m = Matcher::new(pal.clone());
    const TOLERANCE: f32 = 0.05;
    for r in 0..24 {
        for g in 0..24 {
            for b in 0..24 {
                let q = rgb(r as f32 / 23.0, g as f32 / 23.0, b as f32 / 23.0);
                let got = pal[m.index(q.r, q.g, q.b)];
                let want = pal[nearest_in(&pal, q)];
                let d_got = dist2(q, got).sqrt();
                let d_want = dist2(q, want).sqrt();
                assert!(
                    d_got <= d_want + TOLERANCE,
                    "at {q:?}: chose {got:?} (d={d_got}), nearest is {want:?} (d={d_want})"
                );
            }
        }
    }
}

#[test]
fn snap_leaves_only_palette_colours() {
    let pal: Palette = vec![rgb(0.0, 0.0, 0.0), rgb(1.0, 0.0, 0.0), rgb(0.0, 0.0, 1.0)];
    let m = Matcher::new(pal.clone());
    let mut im = Image::new(8, 8);
    for (i, v) in im.pix.iter_mut().enumerate() {
        *v = (i % 17) as f32 / 17.0;
    }
    m.snap(&mut im);
    for y in 0..im.h {
        for x in 0..im.w {
            let (r, g, b) = im.get(x, y);
            assert!(in_palette(&pal, r, g, b), "pixel ({x},{y}) = ({r},{g},{b}) is off-palette");
        }
    }
}

#[test]
fn dither_stays_in_palette_and_averages_out() {
    let pal: Palette = vec![rgb(0.0, 0.0, 0.0), rgb(1.0, 1.0, 1.0)];
    let m = Matcher::new(pal.clone());
    let mut im = Image::new(16, 16);
    im.pix.fill(0.5);
    m.dither(&mut im, 1.0);

    let mut white = 0;
    for y in 0..im.h {
        for x in 0..im.w {
            let (r, g, b) = im.get(x, y);
            assert!(in_palette(&pal, r, g, b), "dither produced ({r},{g},{b})");
            if r > 0.5 {
                white += 1;
            }
        }
    }
    // A flat mid grey on a black/white palette should come out roughly half
    // lit; error diffusion that had gone wrong would collapse to one value.
    assert!(
        (64..=192).contains(&white),
        "dithered 50% grey gave {white}/256 white pixels, expected near half"
    );
}

/// The palette is extracted once and every frame is matched against it, so an
/// extraction that varied run to run would make the whole render irreproducible.
#[test]
fn extract_is_deterministic() {
    let mut im = Image::new(32, 32);
    for y in 0..32 {
        for x in 0..32 {
            im.set(x, y, x as f32 / 32.0, y as f32 / 32.0, 0.25);
        }
    }
    assert_eq!(palette::extract(&im, 8, 99), palette::extract(&im, 8, 99));
}

#[test]
fn hex_parses_and_round_trips() {
    let pal = palette::parse_hex_all(&["#ff0000", "00ff00", " #0000ff "]).unwrap();
    assert_eq!(pal.len(), 3);
    assert_eq!(pal[0], rgb(1.0, 0.0, 0.0));
    assert_eq!(palette::hex(pal[2]), "#0000ff");
    assert!(palette::parse_hex("nope").is_err());
}
