# Round 18 — Screen Capture & Recording (Architect)

**Status**: Draft  
**Round**: 18  
**Subsystem**: Screen Capture & Recording  
**Analogue**: macOS ScreenCaptureKit / QuickTime capture  
**Author**: Architect  
**Date**: 2026-05-29

---

## 1. Overview and Motivation

VyomaOS has a composited framebuffer (R11), per-app Surface buffers (R11), a bitmap-font renderer (R09), and a WIT rendering API (R17). What is missing is the ability for an app or a user tool to capture what is on screen — either as a one-shot screenshot or as a continuous recording stream.

This subsystem is equivalent in scope to macOS ScreenCaptureKit (screenshot, window capture, stream capture) and QuickTime screen recording. It is NOT a GPU-accelerated path; VyomaOS runs on virtio-gpu and does all compositing in software, so capture reads from the CPU-visible framebuffer or from the per-app Surface that R11 maintains.

### Design Goals

- **Low-dependency PNG screenshot** — single call, returns file handle or image handle; no external C libs
- **Per-window capture** — reads from the per-app Surface (R11) without compositing the full scene
- **Region capture** — arbitrary rectangle of the composited framebuffer
- **Screen recording** — bounded frame queue, pluggable encoder (raw, MJPEG, optional H264)
- **Dual interface** — `VYOMA_CAPTURE:` stdout protocol for simple apps; `vyoma:capture@1.0.0` WIT for typed apps
- **Permission model** — static capability (`capture = true` in vyoma.toml) + runtime consent prompt for `capture_any`
- **Platform-gated** — capture is only wired up on server, mobile, and desktop profiles; MCU/IoT/Robotics profiles compile capture module out entirely

### Non-Goals (v1)

- Audio capture alongside video — out of scope for R18; reserved for R20 (audio subsystem)
- Hardware-accelerated video encoding — software only; H264 is optional via compile flag
- Cursor inclusion in capture — the system compositor cursor is not composited into per-app Surfaces in R11; cursor overlay is a future enhancement
- Frame delta / dirty-rect streaming — full frames only in v1; delta encoding reserved for R21 (remote display)
- Multi-subscriber capture streams — single consumer per session; stream-sharing reserved for R21

---

## 2. Capture Modes

### 2.1 Full-Screen Capture

Reads the composited framebuffer at the time of the call. The supervisor's `CaptureEngine` acquires a read-lock on the vsync mutex (R11), copies the framebuffer pixels into a `CaptureFrame`, then releases the lock. Encoding (PNG/JPEG/raw) runs on a dedicated `capture_worker` thread so the vsync thread is not stalled.

```
Full screen: 1920×1080 × 4 bytes/pixel = 8,294,400 bytes per frame
Copy time on musl/x86_64 (memcpy): ~2ms
PNG encode (miniz_oxide): ~80-250ms depending on content complexity
```

### 2.2 Window Capture

Reads a specific app's `Surface` buffer (as introduced in R11). Each running app that has `display = true` has an associated `Surface { width, height, pixels: Vec<u8> }` stored in `AppState`. Window capture copies from that Surface directly — bypassing the compositor — so it captures the app's own render output before chrome overlays are applied.

This is the correct semantic: callers want "what this app drew", not "what the compositor stamped on top of it". Chrome decorations (title bar, shadow) are not included in window capture by default. A future flag `include_chrome: bool` can add them.

### 2.3 Region Capture

A rectangle `(x, y, w, h)` of the composited framebuffer. Same synchronization as full-screen: lock, copy the rows, unlock, encode off-thread. The supervisor validates that the region is within bounds (clamped to screen dimensions, no negative sizes).

### 2.4 Screen Recording

A `RecordingSession` is opened with a target, frame rate, and encoder. The session spawns an internal frame-pump loop: at each tick (1/fps seconds), the supervisor samples the current frame from the target and pushes it into a `bounded(4)` channel to the encoder thread. The encoder writes encoded frames to an output file under `/data/captures/<pid>/`.

The recording loop runs on the capture_worker thread (shared with screenshot encoding, but screenshots are one-shot and return before the recording loop starts on the same session).

### 2.5 Screenshot (one-shot WIT call)

The simplest entry point: `screenshot(target, format)` → `image-handle`. The result is inserted into the R14 image table so it can immediately be referenced by `vyoma:draw@3.0.0` draw-image commands. This is the path used by screenshot tools, share-sheet actions, etc.

---

## 3. Core Rust Types

```rust
// supervisor/src/capture/mod.rs

/// What to capture.
#[derive(Debug, Clone)]
pub enum CaptureTarget {
    /// Composited framebuffer — the full screen as the user sees it.
    Screen,
    /// A specific app's Surface buffer, identified by app instance ID.
    Window(u32),
    /// An arbitrary rectangle of the composited framebuffer.
    /// Fields: x, y, width, height — in physical pixels, f32 for sub-pixel spec.
    Region(f32, f32, f32, f32),
}

/// Output pixel format for raw captures.
#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    Bgra32,
    Rgba32,
    Rgb24,
}

/// Output encoding.
#[derive(Debug, Clone, Copy)]
pub enum CaptureFormat {
    /// Lossless PNG via miniz_oxide. Best for screenshots.
    Png,
    /// Raw pixel data in the specified PixelFormat. No header.
    Raw(PixelFormat),
    /// JPEG with quality 0–100. Smallest output.
    Jpeg(u8),
}

/// A single captured frame.
#[derive(Debug)]
pub struct CaptureFrame {
    pub width: u32,
    pub height: u32,
    /// Encoded bytes (PNG/JPEG data, or raw pixels depending on CaptureFormat).
    pub data: Vec<u8>,
    /// Microseconds since supervisor epoch (monotonic clock).
    pub timestamp_us: u64,
    /// Which capture format produced this data.
    pub format: CaptureFormat,
}

/// Input parameters for a one-shot capture.
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub target: CaptureTarget,
    pub format: CaptureFormat,
    /// Scale factor applied to output dimensions. 1.0 = native resolution.
    /// Values < 1.0 downsample; > 1.0 upscale. Must be in range [0.1, 4.0].
    pub scale: f32,
}

/// Video encoder selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    /// Uncompressed raw frames. Very large files; useful for post-processing.
    Raw,
    /// Motion JPEG: each frame is an independent JPEG. No inter-frame dependency.
    Mjpeg(u8), // quality 0–100
    /// H.264 via optional compile-time dependency (x264 via FFI).
    /// Feature-gated: compile with --features h264.
    #[cfg(feature = "h264")]
    H264,
}

/// An active screen recording session.
pub struct RecordingSession {
    pub session_id: u32,
    pub target: CaptureTarget,
    pub fps: u8,
    pub encoder: Encoder,
    /// Absolute path being written to: /data/captures/<pid>/<session_id>.<ext>
    pub output_path: String,
    /// Total frames captured so far.
    pub frames_captured: u64,
    /// Total frames dropped (encoder fell behind bounded queue).
    pub frames_dropped: u64,
    /// Whether the session is actively recording.
    pub active: bool,
    /// Sender side of bounded frame queue to encoder thread.
    pub frame_tx: std::sync::mpsc::SyncSender<CaptureFrame>,
    /// Join handle for encoder thread (None until session starts).
    pub encoder_thread: Option<std::thread::JoinHandle<()>>,
}

/// Per-app capture permission state, tracked by CaptureEngine.
pub struct CapturePermission {
    /// app instance ID
    pub app_id: u32,
    /// can capture own window
    pub capture_own: bool,
    /// can capture any window / full screen (requires runtime consent)
    pub capture_any: bool,
    /// consent was explicitly granted by user at runtime
    pub consent_granted: bool,
    /// monotonic timestamp of last screenshot (for rate limiting)
    pub last_screenshot_us: u64,
    /// screenshots taken in the current second (for rate limiting)
    pub screenshots_this_second: u32,
}

/// Central engine owned by the supervisor's main state.
pub struct CaptureEngine {
    /// next session ID counter
    pub next_session_id: u32,
    /// active sessions, keyed by session_id
    pub sessions: std::collections::HashMap<u32, RecordingSession>,
    /// per-app permission records, keyed by app_id
    pub permissions: std::collections::HashMap<u32, CapturePermission>,
    /// output directory root: /data/captures
    pub captures_dir: String,
}
```

---

## 4. WIT Interface `vyoma:capture@1.0.0`

```wit
package vyoma:capture@1.0.0;

/// Identifies what to capture.
variant capture-target {
    /// Full composited screen.
    screen,
    /// One app's Surface, identified by instance ID.
    window(u32),
    /// Rectangular region of screen: x, y, w, h in pixels.
    region(tuple<f32, f32, f32, f32>),
}

/// Wire encoding for formats.
/// 0 = PNG, 1 = Raw BGRA32, 2 = Raw RGBA32, 3 = Raw RGB24, 10–109 = JPEG quality (value - 10)
type format-code = u8;

/// Wire encoding for encoders.
/// 0 = Raw, 1–100 = MJPEG quality, 200 = H264 (optional)
type encoder-code = u8;

interface capture {
    /// Take a one-shot screenshot.
    /// Returns an image handle (compatible with vyoma:images@1.0.0 R14).
    /// Requires capability: capture = true in vyoma.toml.
    /// Error if rate limit exceeded (> 10/s) or permission denied.
    screenshot: func(
        target: capture-target,
        format: format-code,
        scale: f32,
    ) -> result<u32, string>;

    /// Start a recording session.
    /// fps: frames per second (1–60; capped at 30 on mobile, 15 on server).
    /// Returns session handle.
    start-recording: func(
        target: capture-target,
        fps: u8,
        encoder: encoder-code,
    ) -> result<u32, string>;

    /// Stop a recording session.
    /// Returns the absolute path of the output file under /data/captures/<pid>/.
    /// Blocks until the encoder thread flushes and closes the file.
    stop-recording: func(session: u32) -> result<string, string>;

    /// Capture the current frame of an active recording session without
    /// storing to disk. Returns a one-time image handle (R14).
    /// Useful for live preview thumbnails.
    capture-frame: func(session: u32) -> result<u32, string>;

    /// Query dropped frame count for a session (since start or last query).
    /// Resets counter after read.
    dropped-frames: func(session: u32) -> result<u64, string>;

    /// Query whether the H264 encoder is available at runtime.
    h264-available: func() -> bool;
}

world capture-world {
    import capture;
}
```

---

## 5. VYOMA_CAPTURE: Stdout Protocol

For apps that do not use WIT component model linking, the supervisor parses
`VYOMA_CAPTURE:` lines from stdout. This mirrors the `VYOMA_DRAW:` pattern.

### 5.1 Commands (app → supervisor via stdout)

```
VYOMA_CAPTURE:screenshot:<target_code>,<format_code>,<scale>
VYOMA_CAPTURE:record_start:<target_code>,<fps>,<encoder_code>
VYOMA_CAPTURE:record_stop:<session_id>
VYOMA_CAPTURE:frame_peek:<session_id>
VYOMA_CAPTURE:dropped_frames:<session_id>
```

Target codes:
- `screen` — full screen
- `window:<id>` — app window by ID
- `region:<x>,<y>,<w>,<h>` — pixel rectangle

Format codes (same as WIT format-code):
- `0` = PNG
- `1` = Raw BGRA32
- `2` = Raw RGBA32
- `3` = Raw RGB24
- `10`–`109` = JPEG quality (code − 10)

Encoder codes:
- `0` = Raw
- `1`–`100` = MJPEG quality
- `200` = H264 (optional; supervisor returns error if not compiled in)

### 5.2 Responses (supervisor → app via stdin)

```
VYOMA_CAPTURE_DONE:<image_handle>
VYOMA_CAPTURE_SESSION:<session_id>
VYOMA_CAPTURE_SAVED:<absolute_path>
VYOMA_CAPTURE_FRAME:<image_handle>
VYOMA_CAPTURE_DROPPED:<count>
VYOMA_CAPTURE_ERROR:<reason>
```

### 5.3 Example Session (Rust app using protocol)

```rust
// One-shot PNG screenshot of full screen
println!("VYOMA_CAPTURE:screenshot:screen,0,1.0");
// Read response from stdin
let mut line = String::new();
std::io::stdin().read_line(&mut line).unwrap();
// line: "VYOMA_CAPTURE_DONE:42\n"  — image handle 42

// Start 30fps MJPEG recording of window 7
println!("VYOMA_CAPTURE:record_start:window:7,30,80");
// line: "VYOMA_CAPTURE_SESSION:1"

// ... some time later ...
println!("VYOMA_CAPTURE:record_stop:1");
// line: "VYOMA_CAPTURE_SAVED:/data/captures/1234/1.mjpeg"
```

---

## 6. Supervisor Implementation

### 6.1 File Layout

The capture subsystem lives entirely under `supervisor/src/capture/`. Each file is bounded at 500 lines per the VyomaOS code size rule.

```
supervisor/src/capture/
├── mod.rs           — CaptureEngine, public types (CaptureTarget, CaptureFormat, ...)
├── screenshot.rs    — one-shot capture: framebuffer snapshot, surface copy, region crop
├── encoder.rs       — frame encoding: raw copy, miniz_oxide PNG, JPEG, optional H264
├── recorder.rs      — RecordingSession lifecycle, frame pump loop, output writer
├── wit_handlers.rs  — WIT linker: maps vyoma:capture@1.0.0 host functions
└── protocol.rs      — VYOMA_CAPTURE: line parser and response formatter
```

### 6.2 `mod.rs` — Engine and Types (~350 lines)

Defines all public types (section 3). Also defines `CaptureEngine` with methods:

```rust
impl CaptureEngine {
    pub fn new(captures_dir: impl Into<String>) -> Self;
    pub fn check_permission(&self, app_id: u32, target: &CaptureTarget) -> Result<(), String>;
    pub fn rate_limit_check(&mut self, app_id: u32) -> Result<(), String>;
    pub fn alloc_session_id(&mut self) -> u32;
    pub fn insert_session(&mut self, session: RecordingSession);
    pub fn get_session_mut(&mut self, id: u32) -> Option<&mut RecordingSession>;
    pub fn remove_session(&mut self, id: u32) -> Option<RecordingSession>;
}
```

The `CaptureEngine` is owned inside `SupervisorState` alongside `AppTable` and `DisplayState`. There is no separate lock for `CaptureEngine` — it is accessed only from the supervisor's main event loop, keeping the concurrency model simple.

### 6.3 `screenshot.rs` — One-Shot Capture (~420 lines)

```rust
pub fn capture_screen(
    fb: &FramebufferState,   // R11 type
    vsync_lock: &std::sync::Mutex<()>,
    request: &CaptureRequest,
    now_us: u64,
) -> Result<CaptureFrame, String>;

pub fn capture_window(
    app_state: &AppState,    // R11 type — contains Surface
    request: &CaptureRequest,
    now_us: u64,
) -> Result<CaptureFrame, String>;

pub fn capture_region(
    fb: &FramebufferState,
    vsync_lock: &std::sync::Mutex<()>,
    rect: (f32, f32, f32, f32),
    request: &CaptureRequest,
    now_us: u64,
) -> Result<CaptureFrame, String>;
```

**Synchronization detail**: `capture_screen` and `capture_region` acquire `vsync_lock` for the minimum time needed to `memcpy` pixels into a local `Vec<u8>`. The encode step runs *after* releasing the lock. This means encoding never contends with the vsync compositor.

```rust
// Pseudocode for capture_screen
pub fn capture_screen(...) -> Result<CaptureFrame, String> {
    let pixels = {
        let _guard = vsync_lock.lock().expect("vsync_lock poisoned");
        fb.pixels.clone()  // copy while holding lock
    };
    // lock released; now encode on this thread (capture_worker thread, not vsync)
    let encoded = encode_frame(&pixels, fb.width, fb.height, &request.format)?;
    Ok(CaptureFrame { width: fb.width, height: fb.height, data: encoded, ... })
}
```

**Window capture** does not need the vsync_lock because `AppState::surface` is protected by the supervisor's `RwLock<AppTable>` — a read-lock is held for the duration of the pixel copy. The compositor also holds a read-lock during rendering; only the Z-sort step takes a write-lock.

**Scale factor**: after copying raw pixels, `screenshot.rs` applies bilinear downscale if `request.scale < 1.0`. For `scale == 1.0` (the common case), no scaling is performed.

### 6.4 `encoder.rs` — Frame Encoding (~450 lines)

Provides synchronous encoding functions called either from `screenshot.rs` (one-shot) or the encoder thread in `recorder.rs`.

```rust
/// Encode raw BGRA pixels to PNG using miniz_oxide.
pub fn encode_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String>;

/// Encode raw BGRA pixels to JPEG.
/// Uses a lightweight pure-Rust JPEG encoder (jpeg-encoder crate, no C deps).
pub fn encode_jpeg(pixels: &[u8], width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String>;

/// Convert BGRA to RGB24 in-place for encoders that require RGB input.
pub fn bgra_to_rgb24(pixels: &[u8]) -> Vec<u8>;

/// Write a raw frame with a simple binary header:
/// [4 bytes magic 0x56594F4D] [4 bytes width] [4 bytes height]
/// [4 bytes pixel_format as u32] [8 bytes timestamp_us] [data bytes]
pub fn write_raw_frame(
    writer: &mut dyn std::io::Write,
    frame: &CaptureFrame,
) -> Result<(), String>;
```

**PNG encoding detail**: `miniz_oxide` provides `deflate` compression. VyomaOS uses it via the `png` crate (which depends on `miniz_oxide`) for correct PNG file structure including IHDR, IDAT, and IEND chunks. The `png` crate is already a transitive dependency of several WASM apps; adding it to the supervisor Cargo.toml does not introduce a new unique dependency.

**JPEG encoding**: The `jpeg-encoder` crate provides a pure-Rust JPEG encoder with zero C FFI. At quality 80, a 1920×1080 image encodes in ~30–60ms on a modern CPU, well within the 33ms-per-frame budget for 30fps recording if CPU is lightly loaded. In practice, MJPEG recording targets 15–24fps unless on desktop profile.

**H264** (compile-time optional): gated behind `--features h264`. When enabled, links `x264` via a thin FFI wrapper in `encoder.rs`. The feature is NOT enabled by default; the supervisor binary on server/desktop profiles can opt in via the Makefile. The WIT function `h264-available` returns false when the feature is absent.

### 6.5 `recorder.rs` — Recording Session (~480 lines)

Manages the full lifecycle of a `RecordingSession`.

```rust
pub fn start_session(
    engine: &mut CaptureEngine,
    target: CaptureTarget,
    fps: u8,
    encoder: Encoder,
    app_pid: u32,
    fb_handle: FbHandle,  // Arc clone of framebuffer state for reader
    app_table: AppTableHandle,
    vsync_lock: Arc<Mutex<()>>,
) -> Result<u32, String>;

pub fn stop_session(
    engine: &mut CaptureEngine,
    session_id: u32,
) -> Result<String, String>;
```

**Frame pump**: `start_session` spawns one `std::thread` per session. The thread runs:

```
loop {
    sleep until next_tick (computed from fps);
    attempt to capture frame (calls screenshot::capture_*);
    match frame_tx.try_send(frame) {
        Ok(()) => session.frames_captured += 1,
        Err(TrySendError::Full(_)) => session.frames_dropped += 1,
        Err(TrySendError::Disconnected(_)) => break,
    }
}
```

**Encoder thread**: A second thread per session reads from `frame_rx` (the other end of the bounded channel) and calls the appropriate `encoder.rs` function, then writes to the output file. The two threads per session are the frame pump and the encoder; they communicate via a `SyncSender<CaptureFrame>` with capacity 4.

**Max concurrent sessions per app**: The `CaptureEngine` enforces a limit of 2 active sessions per `app_id`. This is checked in `start_session` before spawning threads.

**Output path construction**:

```rust
fn make_output_path(captures_dir: &str, app_pid: u32, session_id: u32, encoder: Encoder) -> String {
    let ext = match encoder {
        Encoder::Raw     => "raw",
        Encoder::Mjpeg(_) => "mjpeg",
        #[cfg(feature = "h264")]
        Encoder::H264    => "mp4",
    };
    // Safe: components are u32 decimal, no user-supplied strings
    format!("{}/{}/{}.{}", captures_dir, app_pid, session_id, ext)
}
```

The `app_pid` directory is created by the supervisor on first capture request for that app. The path contains only decimal integers and the extension; no user-supplied string is ever interpolated into the path.

**`stop_session`**: Sets `session.active = false`, drops `frame_tx` (causing the encoder thread's `recv()` to return `Disconnected`), joins both threads with a 5-second timeout (if threads do not finish in 5s, they are detached and an error is logged), then removes the session from the engine and returns the `output_path`.

### 6.6 `wit_handlers.rs` — WIT Linker (~300 lines)

Wires the `vyoma:capture@1.0.0` WIT host functions into the Wasmtime `Linker`. Follows the same pattern as R17's `wit_handlers.rs` for `vyoma:draw@3.0.0`.

```rust
pub fn add_to_linker(
    linker: &mut wasmtime::Linker<SupervisorCtx>,
) -> anyhow::Result<()>;
```

Each WIT function maps to a closure that:
1. Extracts `app_id` from `SupervisorCtx`
2. Calls `engine.check_permission(app_id, &target)?`
3. Calls `engine.rate_limit_check(app_id)?`
4. Dispatches to `screenshot::*` or `recorder::*`
5. For screenshot results: stores `CaptureFrame::data` in the R14 image table and returns the handle

### 6.7 `protocol.rs` — VYOMA_CAPTURE: Parser (~250 lines)

```rust
/// Returns Some(CaptureCommand) if the line starts with "VYOMA_CAPTURE:"
pub fn parse_capture_line(line: &str) -> Option<CaptureCommand>;

pub enum CaptureCommand {
    Screenshot { target: CaptureTarget, format: CaptureFormat, scale: f32 },
    RecordStart { target: CaptureTarget, fps: u8, encoder: Encoder },
    RecordStop  { session_id: u32 },
    FramePeek   { session_id: u32 },
    DroppedFrames { session_id: u32 },
}

pub fn format_response(resp: &CaptureResponse) -> String;

pub enum CaptureResponse {
    Done     { image_handle: u32 },
    Session  { session_id: u32 },
    Saved    { path: String },
    Frame    { image_handle: u32 },
    Dropped  { count: u64 },
    Error    { reason: String },
}
```

The parser is integrated into the supervisor's main stdout-parsing loop (in `main.rs`) alongside `parse_draw_line` and `parse_anim_line`.

---

## 7. Permission Model

### 7.1 Capability Declaration

```toml
# vyoma.toml — app declares what it needs

[capabilities]
stdio      = true
filesystem = true   # required if recording (to write to /data/captures/)
capture    = true   # required for any capture operation

# Optional: allow capturing other apps' windows or full screen
capture_any = true  # requires runtime consent dialog (desktop only)
```

The supervisor's manifest parser (`manifest.rs`) adds two new optional boolean fields:
- `capabilities.capture` — defaults to `false`
- `capabilities.capture_any` — defaults to `false`; `capture_any` implies `capture`

### 7.2 Enforcement

In `CaptureEngine::check_permission`:

```
if not permission.capture_own:
    return Err("capture capability not declared")

if target is Window(id) and id != self_app_id:
    if not permission.capture_any:
        return Err("capture_any capability required to capture other windows")
    if not permission.consent_granted:
        return Err("runtime consent required for capture_any; prompt pending")

if target is Screen or Region:
    if not permission.capture_any:
        return Err("capture_any capability required to capture full screen")
    if not permission.consent_granted:
        return Err("runtime consent required for capture_any")
```

### 7.3 Runtime Consent for `capture_any`

When an app with `capture_any = true` first attempts a full-screen or cross-window capture, the supervisor:

1. Pauses the capture request (returns `VYOMA_CAPTURE_ERROR:consent_pending` immediately)
2. Posts a system notification to the chrome app (via IPC: `@chrome: capture_consent_request:<app_name>:<app_id>`)
3. The chrome app draws a modal consent dialog: "App `<name>` wants to record your screen. Allow?"
4. User responds; chrome sends `@supervisor: capture_consent_grant:<app_id>` or `capture_consent_deny:<app_id>`
5. Supervisor sets `permission.consent_granted = true/false`
6. Subsequent requests proceed or fail based on the stored consent

This is equivalent to macOS TCC. The UI mechanism is the chrome app (already implemented in the compositor layer), not a new UI thread in the supervisor. The consent is stored only in memory for the current session; it is not persisted to disk (no TCC-style database in v1 — consent must be re-granted after reboot).

**Note on non-desktop profiles**: On server, mobile profiles, `capture_any` is not listed in the platform capability matrix. If an app on those profiles declares `capture_any = true`, the manifest parser logs a warning and ignores the field. On server-headless, screen recording is allowed but only of the app's own output (there is no interactive display).

### 7.4 Rate Limiting

Enforced in `CaptureEngine::rate_limit_check`:

```rust
let now_us = monotonic_now_us();
let one_second_us = 1_000_000u64;
if now_us - permission.last_screenshot_us > one_second_us {
    // new second window
    permission.last_screenshot_us = now_us;
    permission.screenshots_this_second = 0;
}
permission.screenshots_this_second += 1;
if permission.screenshots_this_second > 10 {
    return Err("rate limit exceeded: max 10 screenshots per second");
}
Ok(())
```

Rate limiting applies to one-shot `screenshot` calls only. `capture-frame` (frame peek during recording) is also rate-limited using the same counter, since it is semantically a screenshot. `start-recording` and `stop-recording` are not rate-limited (sessions are bounded by max count instead).

---

## 8. Integration with Previous Rounds

### 8.1 R11 — Display Compositor

`CaptureEngine` receives two handles at construction time:
- `Arc<RwLock<FramebufferState>>` — shared reference to the composited framebuffer
- `Arc<Mutex<()>>` — the vsync mutex (same mutex the compositor uses to guard the flush pass)

`screenshot.rs` acquires the vsync mutex read-lock for the pixel copy only. It does NOT hold the lock during encoding, which can take hundreds of milliseconds.

Per-app Surface buffers are accessed via `Arc<RwLock<AppTable>>`. A read-lock gives access to any app's `Surface`. Window capture holds this read-lock for the pixel copy duration only.

### 8.2 R14 — Image Handles

After a one-shot `screenshot` or a `capture-frame` call, the encoded bytes are inserted into the R14 image table:

```rust
let handle = image_table.insert(ImageEntry {
    width: frame.width,
    height: frame.height,
    data: frame.data,
    format: ImageFormat::from_capture_format(frame.format),
});
return Ok(handle);
```

The returned `u32` handle is immediately usable in `vyoma:draw@3.0.0` `draw-image` calls or in subsequent R14 image operations (scale, crop, export). This makes the screenshot → display pipeline a single round-trip.

### 8.3 R17 — WIT Rendering API

The `vyoma:capture@1.0.0` WIT world is a separate package from `vyoma:draw@3.0.0`. An app that wants both rendering and capture imports both worlds. The two WIT packages share no types (image handles are just `u32`; the R14 table is the shared state). This keeps the WIT interfaces independently versioned.

### 8.4 R16 — Animation (No Dependency)

Screen capture reads from the composited framebuffer, which is the output of R16's animation subsystem. Capture does not need to know about animation internals: it captures whatever pixels the vsync compositor has most recently written. If a frame is captured mid-animation, the captured pixels reflect the animation's current interpolated state. This is correct behavior — it mirrors what the user sees.

---

## 9. Performance Analysis

### 9.1 Screenshot Latency Budget (target: < 50ms end-to-end)

| Step | Time estimate | Thread |
|------|--------------|--------|
| Wait for vsync_lock (usually < 1ms unless vsync is in flush) | 0–16ms | capture_worker |
| Pixel copy (1920×1080 × 4B = 8MB, ~memcpy) | ~2ms | capture_worker |
| Release vsync_lock | — | — |
| PNG encode (miniz_oxide level 1 — fast) | ~80ms | capture_worker |
| Insert into R14 image table | ~0.1ms | capture_worker |
| Return image handle to app | — | — |

**Problem**: PNG at level 1 still takes ~80ms for a 1080p frame, which exceeds the 50ms target.

**Resolution**: Default screenshot format is **JPEG at quality 85**, not PNG. JPEG at quality 85 encodes a 1920×1080 frame in ~30–40ms and produces files of 300–600KB. PNG is available as an explicit format option but is not the default.

For apps that need lossless PNG, they accept the ~80–250ms latency. The supervisor does not block on this; encoding runs on the capture_worker thread, not the vsync thread. The app waits for the `VYOMA_CAPTURE_DONE:` response or the WIT call to return, which includes the encode time.

**Raw format** (no encoding) returns in < 5ms including copy. Useful for post-processing pipelines where the app will do its own encoding.

### 9.2 Recording Frame Budget (30fps = 33ms per frame)

| Component | Time | Headroom |
|-----------|------|---------|
| Frame pump: pixel copy | ~2ms | — |
| Frame pump: scale (if != 1.0) | ~5ms | — |
| Frame pump: push to queue | ~0.1ms | — |
| Encoder: JPEG at quality 70 | ~25ms | ~6ms |
| Encoder: file write (SSD/9P) | ~3ms | ~3ms |

At 30fps MJPEG with quality 70, the encoder is near capacity. Default recording fps is 24 (for desktop) and 15 (for server/mobile). Quality defaults to 70 for recording (vs 85 for screenshots).

The bounded queue (capacity 4) provides ~133ms of burst absorption at 30fps. If encoding consistently takes > 33ms, frames are dropped after the queue fills. The app can monitor `dropped-frames` to detect this and reduce fps.

### 9.3 Memory Usage

| Scenario | Memory |
|---------|--------|
| One 1080p raw frame buffer | ~8MB |
| Bounded queue of 4 raw frames | ~32MB |
| PNG encode temp buffer | ~10MB |
| JPEG encode temp buffer | ~2MB |

Total per active recording session: ~42MB including the frame queue. Two concurrent sessions: ~84MB. This is acceptable on desktop (512MB RAM floor) but tight on mobile (256MB). On mobile, max fps is capped at 15 and queue capacity is reduced to 2 frames (16MB per session).

---

## 10. Platform Matrix

| Feature | mcu-minimal | iot-edge | robotics-rt | server-headless | mobile | desktop-full |
|---------|:-----------:|:--------:|:-----------:|:---------------:|:------:|:------------:|
| Screenshot raw | no | no | no | yes | yes | yes |
| Screenshot PNG | no | no | no | yes | yes | yes |
| Screenshot JPEG | no | no | no | yes | yes | yes |
| Region capture | no | no | no | yes | yes | yes |
| Window capture | no | no | no | yes | yes | yes |
| Recording raw | no | no | no | yes | no | yes |
| Recording MJPEG | no | no | no | no | yes (≤15fps) | yes (≤30fps) |
| Recording H264 | no | no | no | no | opt | opt |
| capture_any perm | no | no | no | no | no | yes |
| Max sessions | 0 | 0 | 0 | 2 | 1 | 2 |
| Frame queue cap | — | — | — | 4 | 2 | 4 |

Platform gating is done at compile time in the platform profile loader (R43 supervisor subsystem). When the capture module is disabled for a platform, it is not compiled in; calls return `Err("capture not available on this platform")` from a stub implementation.

The server-headless profile allows raw recording (useful for CI screenshot testing, automated workflows) but not MJPEG (no display server). Mobile allows MJPEG at ≤15fps but not raw (memory constraint). Desktop is the full-featured tier.

---

## 11. Security Analysis

### 11.1 Path Traversal Prevention

All output paths are constructed exclusively by the supervisor, never by the app:

```rust
// The app never supplies a path string
// Session ID and app PID are u32 values — no special characters
let path = format!("{}/{}/{}.{}", captures_dir, app_pid, session_id, ext);
```

The `captures_dir` is a supervisor-controlled constant (`/data/captures`). The app PID and session ID are integers assigned by the supervisor. The extension is selected from a fixed enum. There is no user-supplied string component. Path traversal is structurally impossible.

The `stop-recording` WIT function returns the supervisor-generated path to the app. The app can read this path to know where its file was written. The app does NOT supply a destination path.

### 11.2 `capture_any` Runtime Consent

As described in section 7.3: consent is mandatory at the supervisor level before any cross-window or full-screen capture proceeds. The consent state is stored in `CapturePermission::consent_granted`, which is initialized to `false` at app launch regardless of the static capability declaration.

An app that declares `capture_any = true` but has not yet received runtime consent will get `Err("runtime consent required")` on every cross-window capture attempt. The supervisor also sends a one-time consent request to the chrome app (see section 7.3). The chrome app is a trusted system app (PID 2); the supervisor only accepts `capture_consent_grant/deny` IPC from that app.

### 11.3 Max Screenshot Rate

10 screenshots per second per app. This prevents denial-of-service via capture (e.g., an app that screenshots in a tight loop to fill disk or saturate CPU). The rate limit is per-app (keyed by `app_id`), not global.

### 11.4 Max Concurrent Sessions

2 sessions per app. The global limit across all apps is not explicitly set in v1; in practice, each session uses two threads and ~42MB of RAM. The supervisor's process model (one thread per WASM app) means excessive concurrent recordings would degrade overall system responsiveness. A future improvement would add a global cap of 4 total sessions.

### 11.5 Output Storage

Recording output is stored under `/data/captures/<pid>/`. The `/data` filesystem is the 9P-virtio mount shared between host and VM. Apps must declare `filesystem = true` to record; without it, `start-recording` returns an error. The supervisor creates the `<pid>` subdirectory on first recording start and does not share it with other apps.

Apps cannot access each other's capture directories through the VyomaOS capability model (each app gets a separate filesystem capability; directory isolation is enforced by the supervisor's WASI preopened-dir list). In v1, the captures directory is shared at the host level but this is acceptable — it is outside the WASM trust boundary.

---

## 12. New Manifest Fields

```toml
# vyoma.toml additions for R18

[capabilities]
capture     = false   # default; set true to allow any capture
capture_any = false   # default; set true to request cross-window/full-screen
```

Both fields are optional (default false). The manifest validator (`check-manifests` target) is updated to recognize these fields. Unknown-field errors will NOT be raised for apps that do not declare them.

---

## 13. App Manifest Capability Table (updated)

| Capability | Effect |
|-----------|--------|
| `capture` | App can screenshot/record its own window |
| `capture_any` | App can screenshot/record any window or full screen (requires runtime consent on desktop) |

---

## 14. Dependency Additions (`supervisor/Cargo.toml`)

```toml
[dependencies]
png              = "0.17"     # PNG encode/decode; depends on miniz_oxide
jpeg-encoder     = "0.6"     # Pure Rust JPEG encoder; no C deps

[features]
default = []
h264    = ["x264-sys"]       # Optional; requires x264 shared lib on host

[target.'cfg(feature = "h264")'.dependencies]
x264-sys = "0.4"
```

The `png` crate version 0.17 is already present in several WASM app Cargo.toml files. The supervisor adding it as a direct dependency does not increase the total unique dependency count significantly. `jpeg-encoder` is new but small (< 3000 lines of Rust, zero C dependencies, pure Rust JPEG implementation).

---

## 15. Test Plan

### Unit Tests (`supervisor/tests/capture_tests.rs`)

- `test_parse_screenshot_command` — protocol parser correctly decodes all target/format combinations
- `test_parse_record_start` — parses fps and encoder codes including edge cases (fps=0, fps=255)
- `test_parse_invalid_command` — malformed VYOMA_CAPTURE: lines return None (not panics)
- `test_path_construction` — `make_output_path` for all Encoder variants produces correct extensions
- `test_rate_limit_10_per_second` — 11th call in one second returns error
- `test_rate_limit_resets` — counter resets after 1 second window
- `test_permission_no_capture` — app without capture capability gets error
- `test_permission_capture_any_without_consent` — app with capture_any but no consent gets error
- `test_png_encode_roundtrip` — encode small frame, verify PNG header bytes
- `test_jpeg_encode_basic` — encode small frame, verify JPEG SOI marker
- `test_region_bounds_clamp` — region that extends beyond screen dimensions is clamped
- `test_max_sessions_per_app` — 3rd session start returns error

### Integration Tests

- Screenshot via VYOMA_CAPTURE: protocol from a mock WASM app
- Recording start/stop cycle producing a valid MJPEG file
- Consent flow: chrome mock accepting and denying consent requests

---

## 16. Open Questions (v1)

1. **Cursor in capture**: The compositor cursor is a separate overlay layer in R11. Should window capture include or exclude it? Current answer: exclude (capture the raw Surface). Full-screen capture includes it (it's part of the composited fb). A `cursor: bool` flag can be added to `CaptureRequest` in a follow-up.

2. **Audio interleaving**: When R20 (audio subsystem) lands, should MJPEG files be upgraded to AVI with PCM audio? Or leave video-only in R18 and add audio tracks in R20? Current answer: video-only in R18; R20 decides on the container format.

3. **Consent persistence**: Currently consent is in-memory (lost on reboot). Should it be persisted to `/data/.vyoma/capture_consent.toml`? Persisting it introduces a new attack surface (app writes fake consent). In v1: in-memory only.

4. **Thumbnail API**: The `capture-frame` WIT call for frame preview returns a full-resolution image handle. Should there be a `capture-thumbnail: func(session, max_width, max_height)` that returns a downscaled image? Deferred to v2.
