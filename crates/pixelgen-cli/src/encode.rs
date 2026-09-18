//! Writes rendered frames out as video, GIF or a PNG sequence.

use std::error::Error;
use std::fmt;
use std::io::{BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use pixelgen_core::palette::Palette;
use pixelgen_core::pixel::Image;

use crate::image_io::{to8, write_png};

/// The output formats, recognised from the file extension.
#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Video(&'static str),
    Gif,
    Png,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Format::Video(e) => write!(f, "{e}"),
            Format::Gif => write!(f, "gif"),
            Format::Png => write!(f, "png"),
        }
    }
}

/// Resolves an output path's format without rendering anything.
///
/// Callers check this before starting work, so an unusable filename costs a
/// moment rather than a full render.
pub fn format_of(path: &Path) -> Result<Format, Box<dyn Error>> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
    match ext.as_deref() {
        Some("mp4") => Ok(Format::Video("mp4")),
        Some("webm") => Ok(Format::Video("webm")),
        Some("mkv") => Ok(Format::Video("mkv")),
        Some("mov") => Ok(Format::Video("mov")),
        Some("gif") => Ok(Format::Gif),
        Some("png") => Ok(Format::Png),
        None => Err(format!(
            "output {} has no extension; use .mp4, .webm, .gif or .png",
            path.display()
        )
        .into()),
        Some(e) => {
            Err(format!("unsupported output extension {e:?}; use .mp4, .webm, .gif or .png").into())
        }
    }
}

pub struct Options<'a> {
    pub path: &'a Path,
    pub format: Format,
    pub fps: usize,
    /// Integer nearest-neighbour upscale factor.
    pub scale: usize,
    /// x264/VP9 CRF; lower is better. `None` takes the per-codec default.
    pub quality: Option<u32>,
}

pub fn write(frames: &[Image], pal: &Palette, o: &Options) -> Result<(), Box<dyn Error>> {
    if frames.is_empty() {
        return Err("no frames to encode".into());
    }
    match o.format {
        Format::Video(ext) => write_video(frames, o, ext),
        Format::Gif => write_gif(frames, pal, o),
        Format::Png => write_png_sequence(frames, o),
    }
}

/// Reports rendering progress only when stderr is a terminal. Piped output
/// would otherwise accumulate one line per frame, since the carriage return
/// that makes it a single updating line means nothing in a file.
pub fn progress(quiet: bool) -> impl Fn(usize, usize) + Sync {
    let show = !quiet && std::io::stderr().is_terminal();
    move |done, total| {
        if !show {
            return;
        }
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "\rrendering {done}/{total} frames");
        if done == total {
            let _ = writeln!(err);
        }
    }
}

/// Streams raw frames at the pixel-grid resolution into ffmpeg and lets ffmpeg
/// do the nearest-neighbour upscale. Sending small frames and scaling at the
/// far end moves a fraction of the bytes through the pipe and produces output
/// identical to upscaling first.
fn write_video(frames: &[Image], o: &Options, ext: &str) -> Result<(), Box<dyn Error>> {
    let (w, h) = (frames[0].w, frames[0].h);
    // H.264 and VP9 need even dimensions in the subsampled chroma planes.
    let out_w = (w * o.scale) & !1;
    let out_h = (h * o.scale) & !1;

    let mut args: Vec<String> = [
        "-y", "-hide_banner", "-loglevel", "error", "-f", "rawvideo", "-pixel_format", "rgb24",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend([
        "-video_size".into(),
        format!("{w}x{h}"),
        "-framerate".into(),
        o.fps.to_string(),
        "-i".into(),
        "-".into(),
        "-vf".into(),
        format!("scale={out_w}:{out_h}:flags=neighbor"),
    ]);

    if ext == "webm" {
        let crf = o.quality.unwrap_or(30);
        args.extend(
            ["-c:v", "libvpx-vp9", "-crf", &crf.to_string(), "-b:v", "0"].map(str::to_string),
        );
    } else {
        let crf = o.quality.unwrap_or(16);
        args.extend(
            ["-c:v", "libx264", "-preset", "slow", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p"]
                .map(str::to_string),
        );
        // Wallpapers are looped by the player, so make every frame a valid
        // seek point and keep the loop point clean.
        args.extend(
            ["-g", &frames.len().to_string(), "-movflags", "+faststart"].map(str::to_string),
        );
    }
    args.push(o.path.display().to_string());

    let mut child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("starting ffmpeg (is it installed?): {e}"))?;

    {
        let mut stdin = BufWriter::with_capacity(1 << 20, child.stdin.take().expect("piped"));
        let mut buf = vec![0u8; w * h * 3];
        for f in frames {
            for (o, v) in buf.iter_mut().zip(&f.pix) {
                *o = to8(*v);
            }
            stdin.write_all(&buf)?;
        }
        stdin.flush()?;
    }

    let out = child.wait_with_output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(format!("ffmpeg failed: {}", msg.trim()).into());
    }
    Ok(())
}

/// Encodes GIF directly rather than through ffmpeg: the encoder lives in the
/// core, where the browser can reach it too.
fn write_gif(frames: &[Image], pal: &Palette, o: &Options) -> Result<(), Box<dyn Error>> {
    let bytes = pixelgen_core::encode::gif(frames, pal, o.scale, o.fps)?;
    std::fs::write(o.path, bytes)?;
    Ok(())
}

/// Writes `name_0000.png` next to the named file, for feeding a sprite packer
/// or an external encoder.
fn write_png_sequence(frames: &[Image], o: &Options) -> Result<(), Box<dyn Error>> {
    if frames.len() == 1 {
        return write_png(&frames[0], o.path, o.scale);
    }
    let dir = o.path.parent().unwrap_or(Path::new("."));
    let stem = o.path.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
    for (i, f) in frames.iter().enumerate() {
        let p: PathBuf = dir.join(format!("{stem}_{i:04}.png"));
        write_png(f, &p, o.scale)?;
    }
    Ok(())
}
