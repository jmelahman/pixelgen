# Frame-exact browser video export (WebCodecs + WebM)

## Context

Large browser video exports (output width over about 1000 px) stutter partway through, and in the worst cases the file runs longer than `frames / fps`. #2668 fixed the case where rendering was slow: it now renders every frame first and the timed loop only draws them. The recording step is still timed by the wall clock, though. `web/app.js:1943` feeds a `MediaRecorder` through `canvas.captureStream(0)` + `requestFrame()`, and the recorder stamps each frame with the time it arrives. At large sizes, reading back the canvas and running the realtime VP9/H.264 encoder takes longer than one frame interval (at 12 Mbps). So the recorder drops frames (stutter), or `setTimeout` fires late and the catch-up schedule sends frames in bursts, and stalls end up in the timestamps (longer duration). No amount of pacing fixes this while the timestamps come from the clock.

Fix: encode offline with WebCodecs `VideoEncoder`, giving each frame an explicit timestamp `i * 1e6 / fps`, and write the chunks into a WebM (VP9) file ourselves. Encoder speed then only affects how long the save takes, not the file. Decision (from the user): the output is always WebM/VP9. `MediaRecorder` stays as the fallback only where `VideoEncoder` or a VP9 config isn't available.

## Changes

### 1. WebM muxer in Rust: `crates/pixelgen-core/src/encode.rs`
It goes next to `gif()`, following the existing pattern: "the encoder writes to a byte buffer and opens nothing".

- `pub fn webm(width: u32, height: u32, fps: usize, chunks: &[Chunk]) -> Result<Vec<u8>, String>`, where `Chunk { data: &[u8], key: bool }`, with frames in order.
- It writes a minimal EBML document, with all sizes known because everything is in memory:
  - EBML header: DocType `webm`, DocTypeVersion 4, DocTypeReadVersion 2.
  - Segment containing:
    - Info: TimecodeScale 1 000 000 (1 ms), MuxingApp/WritingApp `pixelgen`, and Duration (float) = `frames * 1000 / fps`. This makes the length exact.
    - Tracks: one video track, CodecID `V_VP9`, DefaultDuration `1e9 / fps` ns, and PixelWidth/PixelHeight.
    - Clusters: a new one starts on every keyframe, and also when a block's timecode relative to the cluster would overflow i16. Each frame is a SimpleBlock with timecode `round(i * 1000 / fps)` and the keyframe flag.
- Small private helpers: `ebml_id`, `ebml_size` (vint), `uint`, `float`, `element(id, body)`.
- It errors on empty chunks or a first chunk that isn't a keyframe, matching `gif()`'s guard style.
- Tests go in a new file, `crates/pixelgen-core/tests/encode.rs` (or extend an existing one if gif tests live somewhere). They cover:
  - the output starts with `1A 45 DF A3`
  - Duration equals frames/fps
  - the block count equals the chunk count
  - keyframes start clusters
  - a >32 s run at 1 ms scale splits clusters
  - error cases

### 2. wasm binding: `crates/pixelgen-wasm/src/lib.rs`
- A free `#[wasm_bindgen] pub fn webm(width, height, fps, data: &[u8], sizes: &[u32], keys: &[u8]) -> Result<Vec<u8>, JsError>`. It splits `data` by `sizes` into `Chunk`s and calls `encode::webm`. It's a free function rather than a `Session` method because it doesn't read the scene.

### 3. Export path: `web/app.js` (save-video handler, ~1891–2005)
- Detect support once at load: `VideoEncoder` exists and `VideoEncoder.isConfigSupported({ codec: 'vp09.00.51.08', ... })` passes for the requested size. Level 5.1 covers up to 4096×2176. Try `vp09.00.61.08` for anything bigger, then fall back.
  - The menu label (`save-video`, `VIDEO_EXT`) becomes `WEBM` when WebCodecs is used. The existing MP4/WebM `MediaRecorder` pick is kept only for the fallback.
- New `async function encodeLoop(session, scale)` that returns a `Blob`:
  - Render each frame with `session.frame(i, 1)` → `putImageData` on the grid canvas → nearest-neighbour `drawImage` onto the scaled canvas. This reuses the current grid/upscale code and its `imageSmoothingEnabled = false`.
  - `new VideoFrame(c, { timestamp: i * 1e6 / fps, duration: 1e6 / fps })`, then `encoder.encode(frame, { keyFrame: i % (2 * fps) === 0 })`, then `frame.close()`.
  - Backpressure: if `encoder.encodeQueueSize > 4`, await the encoder's `dequeue` event. Memory then stays bounded and the up-front `loop` array of ImageData isn't needed, so frames are rendered inline.
  - Progress: `summary.textContent = \`Encoding ${i + 1}/${frames}…\``, yielding with `setTimeout(0)` as the current code does (it keeps working in a background tab).
  - The `output` callback copies each `EncodedVideoChunk` into a growing list, recording `byteLength` and `type === 'key'`.
  - After `await encoder.flush()`, concatenate the chunks and call the wasm `webm(...)`, which returns `new Blob([bytes], { type: 'video/webm' })`.
  - Same bitrate as today (`12e6`), plus `framerate: fps` and `latencyMode: 'quality'`.
- The save-video handler calls `encodeLoop` when supported and the existing `MediaRecorder` body otherwise. That body moves unchanged into `recordLoop(session, scale)`. The handler still sets the file name, and the extension comes from whichever path ran.
- Update the comments on the export path. The "recorder stamps each frame with the moment it arrives" note now explains why the fallback exists.
- Add `video: encodeLoop`-style access to `window.pixelgen` (e.g. `video: (scale) => …` returning the Blob) so the smoke test can check it without a download.

### 4. Smoke test: `tools/web-smoke.mjs`
- After a scene is built, call the exposed video function at a scale that makes the output >1000 px wide. Check that:
  - the Blob starts with the EBML magic
  - loading it into a `<video>` (blob URL, `loadedmetadata`) gives `duration ≈ frames / fps` (within one frame)
- The `/^(MP4|WEBM) /` label check at line ~467 still passes.

## Verification
1. `prek run --all-files` (fmt, clippy, cargo tests including the new muxer tests, build-web, web-smoke).
2. Manual check via the Playwright MCP, as in CLAUDE.md: serve `web/`, open a large synthetic photo so that `width * 3 > 1000`, and run `window.pixelgen.video(3)` via `browser_evaluate`. Load the result into a `<video>` and check that `duration` and `getVideoPlaybackQuality()` report no dropped frames. Also check `browser_console_messages` for errors.
3. Compare against current master on the same scene to confirm the old path's duration drift/stutter and that the new file is exact.
