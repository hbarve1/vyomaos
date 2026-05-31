# FINAL Spec: Screen Capture & Recording (Round 18)

**Subsystem**: Screen Capture & Recording  
**macOS Analogue**: ScreenCaptureKit / QuickTime capture  
**Depends on**: R11 (Surface buffers, vsync mutex), R14 (image table), R17 (WIT draw)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

VyomaOS Screen Capture provides one-shot screenshots and continuous screen recording for
WASM apps. The design reads from the CPU-visible framebuffer or per-app Surface buffers;
no GPU readback is required. This round defines the permission model, synchronization
contract with the vsync compositor, the capture_worker thread architecture, and output
path safety.

### Design Goals

- One-shot PNG/JPEG screenshot via single WIT call or stdout protocol line
- Per-window capture from per-app Surface (R11) without full compositor pass
- Region capture from composited framebuffer
- Bounded frame-queue screen recording (raw/MJPEG/optional H264)
- Link-time WASM capability gating (not runtime-only)
- Push-notification backpressure when encoder falls behind
- Hard-coded output root — no path traversal possible

### Non-Goals (v1)

- Audio capture — reserved for R20 (audio subsystem)
- Hardware-accelerated video encoding — software only
- Cursor overlay in capture — compositor cursor is not in per-app Surfaces in R11
- Frame delta / dirty-rect streaming — full frames only; delta reserved for R21
- Multi-subscriber streams — single consumer per session; sharing reserved for R21

---

## 2. Capability Model — Link-Time Gating (B1 fix)

### 2.1 vyoma.toml Fields

```toml
[capabilities]
capture     = true   # can screenshot/record own window; requires no consent dialog
capture_any = true   # can screenshot/record any window or full screen; requires runtime consent
```

`capture_any` implies `capture`. An app that declares only `capture` may not call
`screenshot(target: screen)` or `screenshot(target: window(other_id))` — those calls
return `Err("permission denied")` at the runtime check.

### 2.2 Link-Time Gate (enforced in supervisor)

The `vyoma:capture@1.0.0` WIT host functions are registered into each app's `Linker`
**conditionally** — only when the app's parsed manifest declares the capability. An app
that does not declare `capture = true` does not receive the import in its `Linker`, and
Wasmtime returns an instantiation error if the WASM binary tries to import it. This is
identical to how R17's `vyoma:draw@3.0.0` is conditionally registered.

```rust
// supervisor/src/wit_handlers.rs  (init_linker_for_app)

pub fn init_linker_for_app(
    linker: &mut Linker<SupervisorCtx>,
    manifest: &AppManifest,
) -> Result<()> {
    // ... other capabilities ...

    if manifest.capabilities.capture || manifest.capabilities.capture_any {
        vyoma::capture::capture::add_to_linker(linker, |ctx| ctx)?;
    }
    // If neither flag is set, no capture imports are registered.
    // A WASM binary declaring a capture import without the manifest flag
    // receives Wasmtime instantiation error: "unknown import: vyoma:capture/capture#screenshot"
    Ok(())
}
```

### 2.3 SupervisorCtx.app_id

`SupervisorCtx` (defined in `supervisor/src/runtime.rs`) carries the app's instance ID:

```rust
pub struct SupervisorCtx {
    pub app_id: u32,           // supervisor-assigned instance ID (not Linux PID)
    pub manifest: Arc<AppManifest>,
    pub ipc_tx: mpsc::Sender<IpcMessage>,
    // ... other fields ...
}
```

`app_id` is set when the app's `Linker` and `Store<SupervisorCtx>` are constructed at
launch time — the same point where `init_linker_for_app` runs. The `app_id` is the
supervisor's internal instance counter, not the Linux PID of the Wasmtime child process.

### 2.4 Runtime Permission Check (secondary gate)

Even after link-time gating, `wit_handlers.rs` checks `CapturePermission` at call time
for `capture_any` operations to enforce the consent requirement:

```rust
// Inside the `screenshot` WIT closure
let perm = engine.permissions.get(&ctx.app_id)
    .ok_or("capture not initialized")?;

if target.requires_any_access() && !perm.consent_granted {
    // Trigger consent dialog (R23 menu bar chrome handles this)
    // Returns error immediately; app must retry after consent
    return Err("capture_any requires user consent".to_string());
}
```

The two-gate model:
1. **Link-time**: `Linker` registration — prevents instantiation without capability
2. **Runtime**: `CapturePermission` check — enforces consent for `capture_any`

---

## 3. Core Rust Types

```rust
// supervisor/src/capture/mod.rs

#[derive(Debug, Clone)]
pub enum CaptureTarget {
    Screen,
    Window(u32),   // supervisor app_id (not Linux PID)
    Region(f32, f32, f32, f32),  // x, y, w, h in physical pixels
}

#[derive(Debug, Clone, Copy)]
pub enum PixelFormat { Bgra32, Rgba32, Rgb24 }

#[derive(Debug, Clone, Copy)]
pub enum CaptureFormat {
    Png,
    Raw(PixelFormat),
    Jpeg(u8),  // quality 0–100; panics if > 100 (caller must clamp)
}

#[derive(Debug)]
pub struct CaptureFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub timestamp_us: u64,
    pub format: CaptureFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Raw,
    Mjpeg(u8),
    #[cfg(feature = "h264")]
    H264,
}

pub struct RecordingSession {
    pub session_id: u32,
    pub app_id: u32,           // owning app's supervisor instance ID
    pub target: CaptureTarget,
    pub fps: u8,
    pub encoder: Encoder,
    pub output_path: String,
    pub frames_captured: Arc<AtomicU64>,   // shared with frame pump thread
    pub frames_dropped: Arc<AtomicU64>,    // shared with frame pump thread
    pub active: bool,
    pub frame_tx: std::sync::mpsc::SyncSender<CaptureFrame>,
    pub pump_thread: Option<std::thread::JoinHandle<()>>,
    pub encoder_thread: Option<std::thread::JoinHandle<()>>,
}

pub struct CapturePermission {
    pub app_id: u32,
    pub capture_own: bool,
    pub capture_any: bool,
    pub consent_granted: bool,
    pub last_screenshot_us: u64,
    pub screenshots_this_second: u32,
}

pub struct CaptureEngine {
    pub next_session_id: u32,
    pub sessions: std::collections::HashMap<u32, RecordingSession>,
    pub permissions: std::collections::HashMap<u32, CapturePermission>,
    // captures_dir is a compile-time constant — NOT a constructor parameter (B5 fix)
}

/// Hard-coded output root. Never sourced from boot.toml or any runtime config.
pub const CAPTURES_DIR: &str = "/data/captures";
```

**B5 fix**: `captures_dir` is a `const &str`, not a constructor parameter. `CaptureEngine`
uses `CAPTURES_DIR` directly. There is no path to inject a different root through
configuration. Boot-time TOML cannot influence this value.

**N2 fix**: `frames_captured` and `frames_dropped` are `Arc<AtomicU64>`, shared between
the `RecordingSession` owned by the main event loop and the frame pump thread. No
`Mutex` required for counter updates across thread boundaries.

---

## 4. vsync Mutex Contract (B2 fix)

The vsync mutex (defined in R11's `supervisor/src/compositor.rs`) is upgraded from
`Mutex<()>` to `RwLock<()>` to enable concurrent reads by the capture subsystem.

### 4.1 Lock Type

```rust
// supervisor/src/compositor.rs — updated from R11
pub struct Compositor {
    pub fb: FrameBuffer,
    /// Write-locked by vsync flush; read-locked by capture pixel copy.
    pub vsync_lock: Arc<std::sync::RwLock<()>>,
    // ...
}
```

### 4.2 Compositor Usage (write-lock during flush only)

```rust
// compositor.rs: flush_to_fb()
fn flush_to_fb(&self) {
    let _write_guard = self.vsync_lock.write();
    // Copy composited pixels to /dev/fb0
    self.fb.blit_to_device();
    // write_guard drops here — ~2ms held
}
```

The write-lock is held only for the DRM/fb0 blit step, not for the entire compositor
pass (Z-sort, alpha blend). This minimizes writer-hold time.

### 4.3 Capture Usage (read-lock during pixel copy only)

```rust
// capture/screenshot.rs
pub fn capture_screen(
    fb: &FrameBuffer,
    vsync_lock: &Arc<RwLock<()>>,
    request: &CaptureRequest,
    now_us: u64,
) -> CaptureFrame {
    let pixels: Vec<u8> = {
        let _read_guard = vsync_lock.read();
        fb.pixels.clone()          // ~2ms memcpy
        // _read_guard drops here
    };
    // encode runs outside the lock — arbitrary duration, no contention
    let data = encode_frame(&pixels, fb.width, fb.height, &request.format);
    CaptureFrame { width: fb.width, height: fb.height, data, timestamp_us: now_us, format: request.format }
}
```

Multiple simultaneous read-locks are allowed. A pending write-lock request from the
compositor will block until all in-progress reads complete (~2ms per capture call).
This is acceptable: at 60fps, the compositor write-lock recurs every 16ms; a 2ms read-lock
contention window is 12.5% of the vsync budget.

### 4.4 R11 Compatibility Note

R11 specified `Mutex<()>` for `vsync_lock`. This FINAL upgrades it to `RwLock<()>`.
The compositor's existing `lock()` calls must be updated to `write()`. This is a
mechanical change: `let _g = vsync_lock.lock().unwrap()` → `let _g = vsync_lock.write().unwrap()`.
No semantic change for the compositor; behaviorally identical single-writer access.

---

## 5. Capture Worker Thread Architecture (B3 fix)

### 5.1 Thread Model

The supervisor spawns a **single named `capture_worker` thread** at startup (not per-session).
It runs a `crossbeam_channel` receive loop. This thread handles:
1. One-shot screenshot encoding (PNG/JPEG)
2. Per-session frame pump ticks

```rust
// supervisor/src/capture/worker.rs

pub enum WorkerTask {
    Screenshot {
        frame: CaptureFrame,
        result_tx: std::sync::mpsc::SyncSender<Result<String, String>>,
    },
    FramePumpTick {
        session_id: u32,
        frame_tx: std::sync::mpsc::SyncSender<CaptureFrame>,
        frames_dropped: Arc<AtomicU64>,
        notify_tx: mpsc::Sender<IpcMessage>,  // for VYOMA_CAPTURE_DROPPING
        app_id: u32,
    },
    Shutdown,
}

pub fn run_capture_worker(rx: crossbeam_channel::Receiver<WorkerTask>) {
    for task in rx {
        match task {
            WorkerTask::Screenshot { frame, result_tx } => {
                // encode synchronously on this thread — does NOT block supervisor main thread
                let path = encode_to_file(&frame);
                let _ = result_tx.send(path);
            }
            WorkerTask::FramePumpTick { session_id, frame_tx, frames_dropped, notify_tx, app_id } => {
                // attempt to push frame; drop and notify on full queue
                match frame_tx.try_send(frame) {
                    Ok(_) => {},
                    Err(mpsc::TrySendError::Full(_)) => {
                        frames_dropped.fetch_add(1, Ordering::Relaxed);
                        // push notification to app stdin (B4 fix)
                        let _ = notify_tx.send(IpcMessage::CaptureDropping { app_id, session_id });
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => { /* session ended */ }
                }
            }
            WorkerTask::Shutdown => break,
        }
    }
}
```

### 5.2 WIT Screenshot: Synchronous App Call → Async Dispatch

The WIT `screenshot` function is synchronous from the WASM app's perspective (the
app's Wasmtime thread blocks until it returns). PNG encoding takes 80–250ms; this must
not run on the supervisor's main event loop thread.

Resolution: the WIT closure dispatches to `capture_worker` via a oneshot-style
`SyncSender<Result<String,String>>` with capacity 1, then parks the calling thread
on `result_rx.recv()`. Encoding runs on `capture_worker`; the calling app's Wasmtime
thread is parked (not spinning, not blocking the event loop):

```rust
// wit_handlers.rs — screenshot WIT closure

fn screenshot(ctx: &mut SupervisorCtx, target: CaptureTarget, format: CaptureFormat, scale: f32)
    -> Result<u32, String>
{
    // 1. Permission check (link-time already done; runtime check for capture_any)
    let perm = ctx.engine.capture.permissions.get(&ctx.app_id)
        .ok_or("not initialized")?;
    rate_limit_check(perm)?;
    if target.requires_any_access() && !perm.consent_granted {
        return Err("capture_any requires consent".to_string());
    }

    // 2. Pixel copy — done on THIS thread, which IS the app's Wasmtime thread.
    //    This is acceptable: memcpy(8MB) ≈ 2ms, well within watchdog.
    let frame = do_pixel_copy(&ctx.compositor, &target, scale);

    // 3. Dispatch encoding to capture_worker — do NOT encode on this thread.
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    ctx.capture_worker_tx.send(WorkerTask::Screenshot {
        frame,
        result_tx,
    }).map_err(|_| "capture worker unavailable")?;

    // 4. Park this thread until encoding completes (80–250ms).
    //    This thread is the app's dedicated Wasmtime thread — parking it is
    //    correct. The supervisor main event loop is NOT blocked.
    let path = result_rx.recv().map_err(|_| "capture worker died")??;

    // 5. Insert into R14 image table and return handle
    let handle = ctx.engine.images.insert(path);
    Ok(handle)
}
```

**Concurrency of screenshot calls**: If the same app calls `screenshot` twice
concurrently (impossible in single-threaded WASM, but documented for completeness),
the second call is blocked at `result_rx.recv()` until the first completes, because each
call creates its own `(result_tx, result_rx)` pair. `capture_worker` processes tasks in
order; there is no starvation, but throughput is serial.

### 5.3 Encoder Thread (per recording session)

Each `RecordingSession` has its own `encoder_thread` which runs independently of
`capture_worker`. Frame pump ticks are dispatched via `capture_worker` (to share the
pixel-copy path), but encoded frames travel from `frame_tx` channel to the encoder
thread directly.

```
App Wasmtime thread  →  WIT start-recording  →  spawn encoder_thread
capture_worker       →  tick every 1/fps      →  try_send to frame_tx
encoder_thread       →  recv frame_tx         →  encode + write to disk
```

Maximum threads at full load (10 apps, 2 sessions each):
- 10 Wasmtime app threads
- 1 capture_worker thread
- 20 encoder threads (2 per app)
- 1 supervisor main event loop thread
- **Total: 32 threads** — well within Linux's default 1024 per-process limit on the
  1-vCPU QEMU desktop-full target. Scheduler pressure at 32 threads is low; most threads
  are blocked on channel receive the majority of the time.

---

## 6. Frame Drop Backpressure (B4 fix)

### 6.1 Push Notification Model

When the frame queue is full and a frame must be dropped, the supervisor sends a
push notification to the app's stdin. This does not require the app to poll:

```
VYOMA_CAPTURE_DROPPING:<session_id>
```

The supervisor sends this line **at most once per second per session** to avoid
flooding the app's stdin. The notification is a hint; it does not carry a drop count.
The app calls `dropped-frames` to get the exact count.

Implementation in `run_capture_worker` (section 5.1):
- `last_drop_notify_us: HashMap<u32, u64>` tracks last notify time per session
- If `now_us - last_drop_notify_us[session_id] < 1_000_000`, suppress the notification
- Otherwise, send `IpcMessage::CaptureDropping { app_id, session_id }` and update timestamp

The IPC message is routed by the supervisor's existing IPC broker to the app's stdin,
identical to how `VYOMA_CAPTURE_SAVED:` results are delivered.

### 6.2 Well-Behaved App Response

On receiving `VYOMA_CAPTURE_DROPPING:<session_id>`:
1. App reads `dropped-frames(session_id)` to get the drop count and reset the counter
2. App calls `stop-recording(session_id)` and `start-recording` with a lower fps
   (e.g., 30fps → 15fps → 10fps)

The notification latency is at most 1 second (the suppress window). An app should
expect to learn of drops within 1 second of them beginning.

### 6.3 `dropped-frames` Destructive-Read Semantics

The `dropped-frames` WIT function resets the counter after reading. This is a
**destructive read**: each call atomically reads the current value and resets to zero.
Implementation:

```rust
// CapturePermission::read_and_reset_dropped(session_id)
fn dropped_frames(&self, session_id: u32) -> Result<u64, String> {
    let session = self.sessions.get(&session_id).ok_or("session not found")?;
    // fetch_swap is load + store zero in one atomic op
    let count = session.frames_dropped.swap(0, Ordering::Relaxed);
    Ok(count)
}
```

Only one consumer per session is supported in v1. Concurrent callers would race on the
reset. This is documented; multi-consumer support is a v2 concern.

---

## 7. WIT Interface `vyoma:capture@1.0.0`

### 7.1 Proper WIT Variant Types (N1 fix)

The original spec used `type format-code = u8` (C-style magic numbers). FINAL uses
proper WIT variants:

```wit
package vyoma:capture@1.0.0;

variant pixel-format { bgra32, rgba32, rgb24 }

variant capture-format {
    png,
    raw(pixel-format),
    jpeg(u8),      // quality 0–100 (supervisor clamps: min(q, 100))
    mjpeg(u8),     // video recording: quality 0–100
}

variant capture-target {
    screen,
    window(u32),
    region(tuple<f32, f32, f32, f32>),
}

interface capture {
    screenshot: func(
        target: capture-target,
        format: capture-format,
        scale: f32,
    ) -> result<u32, string>;

    start-recording: func(
        target: capture-target,
        fps: u8,
        format: capture-format,
    ) -> result<u32, string>;

    stop-recording: func(session: u32) -> result<string, string>;

    capture-frame: func(session: u32) -> result<u32, string>;

    /// Destructive read: resets counter to 0 after returning.
    /// Single-consumer only in v1.
    dropped-frames: func(session: u32) -> result<u64, string>;

    h264-available: func() -> bool;
}

world capture-world {
    import capture;
}
```

### 7.2 Capability Matrix

| Operation | Requires |
|-----------|---------|
| `screenshot(target: screen)` | `capture_any = true` + consent |
| `screenshot(target: window(own_id))` | `capture = true` |
| `screenshot(target: window(other_id))` | `capture_any = true` + consent |
| `screenshot(target: region(_))` | `capture_any = true` + consent |
| `start-recording(target: screen)` | `capture_any = true` + consent |
| `start-recording(target: window(own_id))` | `capture = true` |
| `h264-available` | `capture = true` (any) |

---

## 8. VYOMA_CAPTURE: Stdout Protocol

```
VYOMA_CAPTURE:screenshot:<target>,<format>,<scale>
VYOMA_CAPTURE:record_start:<target>,<fps>,<format>
VYOMA_CAPTURE:record_stop:<session_id>
VYOMA_CAPTURE:frame_peek:<session_id>
VYOMA_CAPTURE:dropped_frames:<session_id>
```

Target encoding:
- `screen`
- `window:<app_id>`
- `region:<x>,<y>,<w>,<h>`

Format encoding (named, not integer codes):
- `png` / `raw_bgra32` / `raw_rgba32` / `raw_rgb24`
- `jpeg:<quality>` (e.g. `jpeg:85`)
- `mjpeg:<quality>` (e.g. `mjpeg:70`)

Responses (supervisor → app stdin):
```
VYOMA_CAPTURE_DONE:<image_handle>
VYOMA_CAPTURE_SESSION:<session_id>
VYOMA_CAPTURE_SAVED:<absolute_path>
VYOMA_CAPTURE_FRAME:<image_handle>
VYOMA_CAPTURE_DROPPED:<count>
VYOMA_CAPTURE_DROPPING:<session_id>    ← push notification (B4 fix)
VYOMA_CAPTURE_ERROR:<reason>
```

---

## 9. Output Path Safety (B5 fix confirmed)

### 9.1 Path Construction

```rust
// supervisor/src/capture/paths.rs

/// CAPTURES_DIR is a compile-time constant. It is never read from configuration.
/// No runtime string can change where capture output lands.
const CAPTURES_DIR: &str = "/data/captures";

pub fn make_output_path(app_id: u32, session_id: u32, encoder: Encoder) -> String {
    let ext = match encoder {
        Encoder::Raw    => "raw",
        Encoder::Mjpeg(_) => "mjpeg",
        #[cfg(feature = "h264")]
        Encoder::H264   => "mp4",
    };
    // Path: /data/captures/<app_id>/<session_id>.<ext>
    // app_id: supervisor-assigned u32 instance counter (not Linux PID)
    // session_id: supervisor-assigned u32 counter
    // ext: from Rust enum — no user string in path
    format!("{CAPTURES_DIR}/{app_id}/{session_id}.{ext}")
}
```

**Why traversal is impossible**:
- `CAPTURES_DIR` = compile-time constant, never from config
- `app_id` = `u32` decimal, supervisor-assigned
- `session_id` = `u32` decimal, supervisor-assigned
- `ext` = from Rust `match` on enum, never from user input

No string from WASM app, vyoma.toml, or boot.toml touches the path. The directory
`/data/captures/<app_id>/` is created with `std::fs::create_dir_all` before the
first session opens.

### 9.2 Filesystem Capability Prerequisite

Screen recording output requires `filesystem = true` in the app's vyoma.toml to
access `/data`. An app with only `capture = true` and no `filesystem = true` can take
screenshots (in-memory image handles) but cannot create recording sessions that write
to disk. The supervisor enforces this at `start-recording` time:

```rust
if !manifest.capabilities.filesystem {
    return Err("recording requires filesystem capability".to_string());
}
```

---

## 10. App Teardown — Recording Session Cleanup

When the supervisor detects app exit (Wasmtime child thread terminates), it calls
`CaptureEngine::cleanup_app(app_id)`:

```rust
pub fn cleanup_app(&mut self, app_id: u32) {
    let session_ids: Vec<u32> = self.sessions.values()
        .filter(|s| s.app_id == app_id)
        .map(|s| s.session_id)
        .collect();
    for sid in session_ids {
        // Drop frame_tx — frame pump observes Disconnected and exits
        // Encoder thread drains remaining frames in queue and closes file
        // join with 5s timeout; if timeout, detach and log partial file warning
        if let Some(session) = self.sessions.remove(&sid) {
            drop(session.frame_tx);
            if let Some(h) = session.encoder_thread {
                let _ = h.join(); // best-effort; encoder thread exits on channel close
            }
            if let Some(h) = session.pump_thread {
                let _ = h.join();
            }
        }
    }
    // Remove orphaned permission entry
    self.permissions.remove(&app_id);
}
```

Partial files: If the encoder thread is mid-write when the app dies and the 5s join
succeeds, the file is complete (MJPEG is self-delimiting; each frame is independent).
If the join times out and the thread is detached, the file is truncated at the last
complete frame boundary. The file is not deleted — callers can recover all complete
frames from a truncated MJPEG file. A `VYOMA_CAPTURE_ERROR:partial_file:<path>` line
is written to the dead app's log (not stdin, since the app is dead).

---

## 11. stop-recording Blocking Duration (N5 note)

`stop-recording` blocks the calling app's Wasmtime thread until the encoder thread
drains and closes the file. For large recordings, this can be up to 5 seconds.

**Requirement**: Apps using screen recording must set `watchdog_secs = 0` or
`watchdog_secs >= 10` in their vyoma.toml. The supervisor logs a warning at startup
if it detects `capture = true` with `watchdog_secs` between 1 and 9.

Future option (v2): add an async `stop-recording-async` that returns immediately and
delivers `VYOMA_CAPTURE_SAVED:` via stdin when flushing is complete.

---

## 12. Platform Matrix

| Feature | desktop-full | mobile | server-headless | robotics-rt | iot-edge | mcu-minimal |
|---------|-------------|--------|----------------|-------------|----------|-------------|
| PNG screenshot | yes | yes | yes | no | no | no |
| JPEG screenshot | yes | yes | yes | no | no | no |
| Raw screenshot | yes | yes | yes | no | no | no |
| MJPEG recording | yes | yes | yes | no | no | no |
| H264 recording | opt-in | opt-in | opt-in | no | no | no |
| `capture_any` perm | yes | no | no | no | no | no |
| Max fps | 60 | 30 | 30 | — | — | — |
| Max concurrent sessions | 4 | 2 | 2 | — | — | — |

On profiles where capture is disabled, all `vyoma:capture@1.0.0` WIT functions are
stub implementations returning `Err("capture not available on this platform")`. The
`capture_worker` thread is not spawned. These stubs are compiled in at all times;
the entire capture module is never `#[cfg]`-excluded, to avoid dead-code warnings and
linker confusion when building cross-profile.

On mobile and server-headless, `capture_any` is not honored. If a manifest declares
`capture_any = true` on these profiles, the manifest parser emits a **capability
mismatch error** (not a warning) at load time:

```
ERROR [manifest] app "screen-recorder": capture_any declared but not available on profile "mobile"
      Fix: remove capture_any from vyoma.toml or build for desktop-full
```

The app fails to launch. This surfaces the cross-profile incompatibility at load time,
not at runtime.

---

## 13. File Layout

```
supervisor/src/capture/
├── mod.rs          (CaptureEngine, types, CAPTURES_DIR const)
├── screenshot.rs   (capture_screen, capture_window, capture_region — pixel copy functions)
├── recorder.rs     (RecordingSession, encoder_thread main loop)
├── worker.rs       (capture_worker thread, WorkerTask enum)
├── paths.rs        (make_output_path, create_session_dir)
└── permissions.rs  (CapturePermission, rate_limit_check, consent_check)

supervisor/src/wit_handlers.rs  (screenshot, start_recording, stop_recording WIT closures)
supervisor/src/compositor.rs    (vsync_lock type updated: Mutex → RwLock — R11 compat)
```

Each file is bounded well under the 500-line limit.

---

## 14. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: WASM capability wire-up | `init_linker_for_app` conditionally registers WIT imports per manifest; link-time gate prevents instantiation without capability |
| B2: vsync mutex type | `RwLock<()>` — compositor write-locks during fb blit; capture read-locks during memcpy; both coexist without mutual exclusion for reads |
| B3: Encoding thread model | `capture_worker` named thread; one-shot screenshots dispatch via `WorkerTask::Screenshot` with `SyncSender<Result>` channel; WIT closure parks on `result_rx.recv()` without blocking event loop |
| B4: Frame drop backpressure | `VYOMA_CAPTURE_DROPPING:<session_id>` push notification (rate-limited 1/s per session); app adjusts fps on receipt; `dropped-frames` remains available for exact count |
| B5: captures_dir trust | `CAPTURES_DIR` is `const &str` — no constructor parameter, no boot.toml field, no runtime config; path traversal is structurally impossible |

---

## 15. Open Questions (deferred to future rounds)

1. **Audio interleaving**: R20 (audio subsystem) will define whether audio capture can
   be muxed into recording sessions. R18 is video-only.
2. **Cursor overlay**: The system compositor cursor is not in per-app Surfaces (R11).
   Including the cursor in screen capture requires cursor-compositing in the capture path.
   Deferred to R21 (Window Manager & Spaces) which will own cursor layer management.
3. **Consent persistence**: Currently in-memory only; cleared on app restart. Persisting
   consent across boots (e.g., to `/data/captures/.consent`) is a privacy-sensitive
   decision deferred to R61 (Permissions & Privacy).
4. **Thumbnail API**: A `thumbnail(session_id) -> image_handle` call for recording
   previews is useful for R24 (Dock & App Switcher). Deferred to that round.
