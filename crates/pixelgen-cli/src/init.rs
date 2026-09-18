//! `pixelgen init` - a starter scene for an image.
//!
//! Path resolution lives here; the analysis and the template itself are
//! [`pixelgen_core::starter`], so the browser emits the identical scene.

use std::error::Error;
use std::path::{Path, PathBuf};

use pixelgen_core::render;
use pixelgen_core::scene::Scene;
use pixelgen_core::starter;

use crate::image_io;
use crate::Common;

pub fn run(
    s: &Scene,
    common: &Common,
    out: Option<PathBuf>,
    force: bool,
) -> Result<(), Box<dyn Error>> {
    let src_path = common.source(s)?;
    let src = image_io::load(&src_path)?;
    let (base, matcher) = render::pixelate(&src, s)?;

    let dst = out.unwrap_or_else(|| crate::derived(&src_path, ".scene.yaml"));
    if !force && dst.exists() {
        return Err(format!("{} already exists (pass --force to overwrite)", dst.display()).into());
    }

    let rel = relative(&src_path, &dst);
    let (text, a) = starter::starter(s, &rel, &base);
    std::fs::write(&dst, text)?;

    println!("wrote {}", dst.display());
    println!("  {}x{} cells, {} colours", base.w, base.h, matcher.palette.len());
    println!(
        "  horizon guessed at y={:.2}; highlights are luma>{:.2} ({:.1}% of pixels)",
        a.horizon,
        a.threshold,
        a.bright_frac * 100.0
    );
    if let Some(l) = &a.warm_light {
        println!("  warm light source near ({:.2}, {:.2}), colour {}", l.x, l.y, l.hex);
    }
    if a.cool_frac > 0.0 {
        println!(
            "  {:.0}% of the frame reads as cool; wrote an \"outside\" region keyed on it",
            a.cool_frac * 100.0
        );
    }
    println!("\nrender it with:\n  pixelgen animate --scene {}", dst.display());
    Ok(())
}

/// Expresses `target` relative to the directory holding `from`, so the scene's
/// `source:` keeps working wherever the pair is moved to together.
///
/// Both are made absolute first. Comparing the paths as written silently
/// produces a broken `source:` whenever one is relative and the other is not,
/// which is the normal case for `pixelgen init photo.jpg --out /somewhere/`.
fn relative(target: &Path, from: &Path) -> String {
    let abs = |p: &Path| -> PathBuf {
        p.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir().unwrap_or_default().join(p)
        })
    };
    let target = abs(target);
    let dir = abs(from.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")));
    match target.strip_prefix(&dir) {
        Ok(p) => p.display().to_string(),
        // Outside the scene's directory: an absolute path is the only spelling
        // that stays correct, and it is better than one that quietly does not.
        Err(_) => target.display().to_string(),
    }
}
