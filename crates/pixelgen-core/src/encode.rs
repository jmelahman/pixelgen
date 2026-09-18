//! Encodes rendered frames as a GIF, and wraps already-encoded video frames
//! in a WebM file.
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

/// One compressed video frame, as a codec handed it back.
pub struct Chunk<'a> {
    pub data: &'a [u8],
    /// Whether the frame decodes on its own.
    pub key: bool,
}

/// Wraps VP9 frames, one per loop step and in order, in a WebM file.
///
/// Only the container: the browser's encoder does the compression. The point
/// of writing it here is the timestamps. A recorder stamps each frame with the
/// moment it arrived, so an encoder that falls behind stretches or stutters
/// the video; here frame `i` is at `i / fps` however long it took to encode,
/// and the file declares exactly `frames / fps` seconds.
pub fn webm(width: u32, height: u32, fps: usize, chunks: &[Chunk]) -> Result<Vec<u8>, String> {
    if chunks.is_empty() {
        return Err("no frames to encode".into());
    }
    if !chunks[0].key {
        return Err("the first frame must be a key frame".into());
    }
    if width == 0 || height == 0 {
        return Err(format!("video size {width}x{height} is empty"));
    }
    let fps = fps.max(1) as f64;
    // Milliseconds, the Matroska default: fine enough for any frame rate a
    // loop plays at, and what every player assumes.
    let at = |i: usize| (i as f64 * 1000.0 / fps).round() as u64;

    let header = element(
        EBML,
        &[
            uint(EBML_VERSION, 1),
            uint(EBML_READ_VERSION, 1),
            uint(EBML_MAX_ID_LENGTH, 4),
            uint(EBML_MAX_SIZE_LENGTH, 8),
            element(DOC_TYPE, b"webm"),
            uint(DOC_TYPE_VERSION, 4),
            uint(DOC_TYPE_READ_VERSION, 2),
        ]
        .concat(),
    );

    let info = element(
        INFO,
        &[
            uint(TIMECODE_SCALE, 1_000_000),
            float(DURATION, chunks.len() as f64 * 1000.0 / fps),
            element(MUXING_APP, b"pixelgen"),
            element(WRITING_APP, b"pixelgen"),
        ]
        .concat(),
    );

    let size = [uint(PIXEL_WIDTH, width.into()), uint(PIXEL_HEIGHT, height.into())];
    let video = element(VIDEO, &size.concat());
    let tracks = element(
        TRACKS,
        &element(
            TRACK_ENTRY,
            &[
                uint(TRACK_NUMBER, 1),
                uint(TRACK_UID, 1),
                uint(TRACK_TYPE, 1),
                uint(FLAG_LACING, 0),
                element(CODEC_ID, b"V_VP9"),
                uint(DEFAULT_DURATION, (1e9 / fps).round() as u64),
                video,
            ]
            .concat(),
        ),
    );

    // A cluster starts at every key frame, so a player seeking into the loop
    // lands on one, and whenever the next frame's offset from the cluster's
    // start would not fit the block's signed 16-bit field.
    let mut clusters = Vec::new();
    let mut i = 0;
    while i < chunks.len() {
        let start = at(i);
        let mut body = uint(TIMECODE, start);
        let mut j = i;
        while j < chunks.len() && (j == i || !chunks[j].key) && at(j) - start <= i16::MAX as u64 {
            let mut block = vec![0x81]; // track 1, as a one-byte vint
            block.extend_from_slice(&((at(j) - start) as i16).to_be_bytes());
            block.push(if chunks[j].key { 0x80 } else { 0 });
            block.extend_from_slice(chunks[j].data);
            body.extend(element(SIMPLE_BLOCK, &block));
            j += 1;
        }
        clusters.extend(element(CLUSTER, &body));
        i = j;
    }

    let mut out = header;
    out.extend(element(SEGMENT, &[info, tracks, clusters].concat()));
    Ok(out)
}

// Matroska element IDs, written with their length marker as the spec lists them.
const EBML: u32 = 0x1A45_DFA3;
const EBML_VERSION: u32 = 0x4286;
const EBML_READ_VERSION: u32 = 0x42F7;
const EBML_MAX_ID_LENGTH: u32 = 0x42F2;
const EBML_MAX_SIZE_LENGTH: u32 = 0x42F3;
const DOC_TYPE: u32 = 0x4282;
const DOC_TYPE_VERSION: u32 = 0x4287;
const DOC_TYPE_READ_VERSION: u32 = 0x4285;
const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const TIMECODE_SCALE: u32 = 0x2A_D7B1;
const DURATION: u32 = 0x4489;
const MUXING_APP: u32 = 0x4D80;
const WRITING_APP: u32 = 0x5741;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_UID: u32 = 0x73C5;
const TRACK_TYPE: u32 = 0x83;
const FLAG_LACING: u32 = 0x9C;
const CODEC_ID: u32 = 0x86;
const DEFAULT_DURATION: u32 = 0x23_E383;
const VIDEO: u32 = 0xE0;
const PIXEL_WIDTH: u32 = 0xB0;
const PIXEL_HEIGHT: u32 = 0xBA;
const CLUSTER: u32 = 0x1F43_B675;
const TIMECODE: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;

/// An element: its ID, its body's length as a variable-length integer, then
/// the body. Every size is known up front because the whole file is built in
/// memory, so nothing is left open-ended for a player to guess at.
fn element(id: u32, body: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = id.to_be_bytes().into_iter().skip_while(|&b| b == 0).collect();
    let len = body.len() as u64;
    // The shortest vint that holds the length; all-ones is reserved for
    // "unknown", hence the strict comparison.
    let n = (1..=8).find(|&n| len < (1u64 << (7 * n)) - 1).expect("element under 2^56 bytes");
    let size = len | 1 << (7 * n);
    out.extend_from_slice(&size.to_be_bytes()[8 - n..]);
    out.extend_from_slice(body);
    out
}

fn uint(id: u32, v: u64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let skip = bytes.iter().take(7).take_while(|&&b| b == 0).count();
    element(id, &bytes[skip..])
}

fn float(id: u32, v: f64) -> Vec<u8> {
    element(id, &v.to_be_bytes())
}

fn err(e: gif::EncodingError) -> String {
    e.to_string()
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}
