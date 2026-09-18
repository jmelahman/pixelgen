//! `pixelgen` turns a photograph into a pixel-art background and animates it
//! into a seamless loop.

mod encode;
mod image_io;
mod init;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use pixelgen_core::effect::CATALOG;
use pixelgen_core::palette;
use pixelgen_core::render;
use pixelgen_core::scene::Scene;

#[derive(Parser)]
#[command(
    name = "pixelgen",
    about = "Animated pixel-art backgrounds from photographs",
    version,
    // Flags may follow positionals: naming the image first is the obvious way
    // to type these commands.
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Reduce an image to a pixel grid and write a still PNG
    Pixelate {
        #[command(flatten)]
        common: Common,
        /// Output PNG path (default <input>-pixel.png)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Integer nearest-neighbor upscale of the output
        #[arg(long, default_value_t = 1)]
        scale: usize,
    },
    /// Render a seamless animated loop
    Animate {
        #[command(flatten)]
        common: Common,
        /// Output file: .mp4, .webm, .gif or .png (default <input>-loop.mp4)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Integer upscale factor (default: the largest that fits 1080p)
        #[arg(long)]
        scale: Option<usize>,
        #[arg(long)]
        fps: Option<usize>,
        /// Loop length in seconds
        #[arg(long)]
        seconds: Option<f64>,
        /// Encoder CRF; lower is better quality
        #[arg(long)]
        quality: Option<u32>,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
    },
    /// Write a starter scene file for an image
    Init {
        #[command(flatten)]
        common: Common,
        /// Output scene path (default <input>.scene.yaml)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Overwrite an existing scene file
        #[arg(short, long)]
        force: bool,
    },
    /// Print the palette derived from an image
    Palette {
        #[command(flatten)]
        common: Common,
    },
    /// List the available effect types
    Effects,
}

/// The flags shared by every command that reduces an image, so the same knobs
/// mean the same thing whether the output is a still or a loop.
///
/// Each is an `Option`, and only a flag actually given overrides the scene.
/// Go used sentinel values for this, which made `-dither 0` indistinguishable
/// from not passing `-dither` at all.
#[derive(Args, Clone)]
struct Common {
    /// The image to work on; defaults to the scene's own `source`
    image: Option<PathBuf>,

    /// Pixel-grid width in cells
    #[arg(long)]
    width: Option<usize>,
    /// Palette size to derive from the image
    #[arg(long)]
    colors: Option<usize>,
    /// Dither strength 0..1 applied to the static base
    #[arg(long)]
    dither: Option<f32>,
    /// Median-filter the source before downscaling
    #[arg(long)]
    median: bool,
    /// Saturation multiplier applied before quantizing
    #[arg(long)]
    saturation: Option<f32>,
    /// Contrast multiplier applied before quantizing
    #[arg(long)]
    contrast: Option<f32>,
    /// Palette file of hex colors, one per line
    #[arg(long = "palette")]
    palette_file: Option<PathBuf>,
    #[arg(long)]
    seed: Option<u32>,
    /// Scene YAML file to load
    #[arg(long)]
    scene: Option<PathBuf>,
}

impl Common {
    /// Loads the scene file if given, then layers CLI overrides on top. Flags
    /// win over the file, so a scene can be tried out without editing it.
    fn scene(&self) -> Result<Scene, Box<dyn Error>> {
        let mut s = match &self.scene {
            Some(p) => {
                let text = std::fs::read_to_string(p)
                    .map_err(|e| format!("reading {}: {e}", p.display()))?;
                Scene::parse(&text).map_err(|e| format!("{}: {e}", p.display()))?
            }
            None => Scene::default(),
        };
        if let Some(v) = self.width {
            s.width = v;
        }
        if let Some(v) = self.colors {
            s.palette.colors = v;
            s.palette.hex.clear();
            s.palette.file.clear();
        }
        if let Some(v) = self.dither {
            s.palette.dither = v;
        }
        if self.median {
            s.prepare.median = true;
        }
        if let Some(v) = self.saturation {
            s.prepare.saturation = v;
        }
        if let Some(v) = self.contrast {
            s.prepare.contrast = v;
        }
        if let Some(p) = &self.palette_file {
            s.palette.file = p.display().to_string();
        }
        if let Some(v) = self.seed {
            s.seed = v;
        }
        // A palette file is the one part of a scene the core will not open for
        // itself, so that the renderer is identical in the browser.
        if let Some(f) = s.palette_file().map(str::to_string) {
            let p = self.relative_to_scene(Path::new(&f));
            let text =
                std::fs::read_to_string(&p).map_err(|e| format!("reading {}: {e}", p.display()))?;
            s.set_palette_hex(&text);
        }
        s.validate()?;
        Ok(s)
    }

    /// Paths inside a scene file are relative to that file, not to wherever
    /// the command happens to be run from.
    fn relative_to_scene(&self, p: &Path) -> PathBuf {
        match &self.scene {
            Some(sf) if p.is_relative() => sf.parent().unwrap_or(Path::new(".")).join(p),
            _ => p.to_path_buf(),
        }
    }

    /// The image to work on: the positional argument, else the scene's own
    /// `source` resolved relative to the scene file.
    fn source(&self, s: &Scene) -> Result<PathBuf, Box<dyn Error>> {
        if let Some(p) = &self.image {
            return Ok(p.clone());
        }
        if s.source.is_empty() {
            return Err("no input image given and the scene has no `source`".into());
        }
        Ok(self.relative_to_scene(Path::new(&s.source)))
    }
}

/// `photo.jpg` + `-pixel.png` -> `photo-pixel.png`.
fn derived(src: &Path, suffix: &str) -> PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    src.parent().unwrap_or(Path::new(".")).join(format!("{stem}{suffix}"))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("pixelgen: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Pixelate { common, out, scale } => pixelate(common, out, scale),
        Command::Animate { common, out, scale, fps, seconds, quality, quiet } => {
            animate(common, out, scale, fps, seconds, quality, quiet)
        }
        Command::Init { common, out, force } => init::run(&common.scene()?, &common, out, force),
        Command::Palette { common } => show_palette(common),
        Command::Effects => {
            let width = CATALOG.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
            for (name, summary) in CATALOG {
                println!("  {name:<width$}  {summary}");
            }
            Ok(())
        }
    }
}

fn pixelate(common: Common, out: Option<PathBuf>, scale: usize) -> Result<(), Box<dyn Error>> {
    let s = common.scene()?;
    let src_path = common.source(&s)?;
    let src = image_io::load(&src_path)?;
    let (base, _) = render::pixelate(&src, &s)?;

    let dst = out.unwrap_or_else(|| derived(&src_path, "-pixel.png"));
    image_io::write_png(&base, &dst, scale)?;
    println!("wrote {} ({}x{} cells, x{scale})", dst.display(), base.w, base.h);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn animate(
    common: Common,
    out: Option<PathBuf>,
    scale: Option<usize>,
    fps: Option<usize>,
    seconds: Option<f64>,
    quality: Option<u32>,
    quiet: bool,
) -> Result<(), Box<dyn Error>> {
    let mut s = common.scene()?;
    if let Some(v) = fps {
        s.loop_.fps = v;
    }
    if let Some(v) = seconds {
        s.loop_.seconds = v;
    }
    s.validate()?;

    let src_path = common.source(&s)?;
    let dst = out.unwrap_or_else(|| derived(&src_path, "-loop.mp4"));
    // Resolve the output format before rendering; finding out that the
    // extension is unusable after a few hundred frames is a poor trade.
    let format = encode::format_of(&dst)?;

    let src = image_io::load(&src_path)?;
    let start = Instant::now();
    let prep = render::prepare(&src, &s)?;
    // Say this before spending a minute on frames that will all be identical.
    if !prep.is_animated() {
        eprintln!("warning: scene has no enabled layers; the loop will be static");
    }

    let frames = prep.all_frames(encode::progress(quiet));
    let scale = scale.filter(|v| *v > 0).unwrap_or_else(|| {
        // The largest whole factor that still fits in 1080p. A fractional
        // scale would reintroduce the interpolation the pixel grid exists to
        // avoid.
        (1080 / prep.base.h).max(1)
    });
    encode::write(
        &frames,
        &prep.matcher.palette,
        &encode::Options { path: &dst, format, fps: s.loop_.fps, scale, quality },
    )?;

    println!(
        "wrote {} - {} frames, {}x{} cells at x{scale}, {} colors, {:.1}s",
        dst.display(),
        frames.len(),
        prep.base.w,
        prep.base.h,
        prep.matcher.palette.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn show_palette(common: Common) -> Result<(), Box<dyn Error>> {
    let s = common.scene()?;
    let src = image_io::load(&common.source(&s)?)?;
    let (_, m) = render::pixelate(&src, &s)?;
    for (i, c) in m.palette.iter().enumerate() {
        println!("{i:3}  {}", palette::hex(*c));
    }
    Ok(())
}
