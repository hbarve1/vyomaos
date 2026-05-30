# FINAL Spec: Video Playback & Codecs (Round 68)

**Subsystem**: Video Playback & Codecs
**macOS Analogue**: AVFoundation / VideoToolbox / CoreMedia
**Depends on**: R11 (display compositor, surface protocol), R67 (audio system)
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Video decode runs entirely in the WASM app. The supervisor is a thin coordinator: it manages a shared-memory surface per video window, enforces frame timing, and synchronizes audio/video presentation timestamps. No codec logic lives in the supervisor.

```
┌─────────────────────────────────────────────────────────────────┐
│  WASM Video App (wasm32-wasip2)                                 │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────┐   │
│  │  Demuxer    │→ │  Codec       │→ │  Frame Scaler/Color  │   │
│  │  (mp4/webm) │  │  (dav1d-wasm │  │  (RGB→RGBA, YUV→RGB) │   │
│  │  pure-Rust) │  │   / openh264 │  └──────────┬───────────┘   │
│  └─────────────┘  │   / rav1e)   │             │ decoded RGBA  │
│                   └──────────────┘             ↓               │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  VyomaVideoClient (SDK, pure-Rust, wasm32-wasip2)        │   │
│  │  - mmap shared surface at /dev/shm/vyoma_vid_<sid>       │   │
│  │  - writes RGBA frame into shm, flips sequence number     │   │
│  │  - sends VYOMA_VIDEO:present:<sid>:<pts_us> via stdout   │   │
│  │  - awaits VYOMA_VIDEO:ack:<sid>:<deadline_us> on stdin   │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                           │stdout/stdin IPC (control only)
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  Supervisor: supervisor/src/video/                               │
│  ┌──────────────────┐  ┌─────────────────┐  ┌───────────────┐  │
│  │  session.rs      │  │  timing.rs      │  │  shm.rs       │  │
│  │  VideoSession    │  │  FrameClock     │  │  ShmSurface   │  │
│  │  open/close/seek │  │  PTS scheduler  │  │  mmap manage  │  │
│  └──────────────────┘  └─────────────────┘  └───────────────┘  │
│  ┌──────────────────┐  ┌─────────────────┐                      │
│  │  protocol.rs     │  │  av_sync.rs     │                      │
│  │  parse VYOMA_VID │  │  clock master   │                      │
│  └──────────────────┘  └─────────────────┘                      │
└─────────────────────────────────────────────────────────────────┘
```

File structure (500-line rule enforced):

```
supervisor/src/video/
├── mod.rs          # re-exports, VideoSubsystem struct, ≤120 lines
├── session.rs      # VideoSession: open/close/seek/state machine, ≤300 lines
├── shm.rs          # ShmSurface: mmap lifecycle, frame flip, ≤200 lines
├── timing.rs       # FrameClock: PTS→wall-clock scheduling, ≤250 lines
├── av_sync.rs      # AvSyncClock: master clock, drift correction, ≤300 lines
└── protocol.rs     # parse/emit VYOMA_VIDEO lines, ≤150 lines
```

---

## 2. Core Types

```rust
// supervisor/src/video/session.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Opening,
    Playing,
    Paused,
    Seeking,
    Closed,
}

pub struct VideoSession {
    pub session_id: u32,
    pub app_name:   String,
    pub frame_interval_us: u64,
    pub width:  u32,
    pub height: u32,
    pub state:  SessionState,
    pub surface: ShmSurface,
    pub clock:   Arc<Mutex<AvSyncClock>>,
    pub frame_clock: FrameClock,
    pub last_ack_pts_us: u64,
}

impl VideoSession {
    pub fn new(
        session_id: u32,
        app_name: String,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        clock: Arc<Mutex<AvSyncClock>>,
    ) -> anyhow::Result<Self> {
        let frame_interval_us =
            (fps_den as u64 * 1_000_000) / fps_num.max(1) as u64;
        let surface = ShmSurface::create(session_id, width, height)?;
        let frame_clock = FrameClock::new(frame_interval_us);
        Ok(Self {
            session_id, app_name, frame_interval_us,
            width, height, state: SessionState::Opening,
            surface, clock, frame_clock, last_ack_pts_us: 0,
        })
    }
}
```

```rust
// supervisor/src/video/shm.rs

use memmap2::{MmapMut, MmapOptions};

#[repr(C)]
pub struct ShmHeader {
    pub seq_write: std::sync::atomic::AtomicU64,
    pub seq_read:  std::sync::atomic::AtomicU64,
    pub width:     u32,
    pub height:    u32,
    _pad: [u8; 40],
}

const HEADER_SIZE: usize = 64;

pub struct ShmSurface {
    pub session_id: u32,
    pub width:  u32,
    pub height: u32,
    shm_path:   String,
    mmap:       MmapMut,
}

impl ShmSurface {
    pub fn create(session_id: u32, width: u32, height: u32) -> anyhow::Result<Self> {
        let path = format!("/dev/shm/vyoma_vid_{}", session_id);
        let frame_bytes = (width * height * 4) as usize;
        let total = HEADER_SIZE + frame_bytes;
        let file = std::fs::OpenOptions::new()
            .read(true).write(true).create(true)
            .mode(0o600)
            .open(&path)?;
        file.set_len(total as u64)?;
        let mmap = unsafe { MmapOptions::new().map_mut(&file)? };
        let surf = Self { session_id, width, height, shm_path: path, mmap };
        surf.header().width = width;
        surf.header().height = height;
        Ok(surf)
    }

    fn header(&self) -> &ShmHeader {
        unsafe { &*(self.mmap.as_ptr() as *const ShmHeader) }
    }

    pub fn rgba_slice(&self) -> &[u8] { &self.mmap[HEADER_SIZE..] }

    pub fn has_new_frame(&self) -> bool {
        use std::sync::atomic::Ordering::Acquire;
        let w = self.header().seq_write.load(Acquire);
        let r = self.header().seq_read.load(Acquire);
        w != r && w % 2 == 0
    }

    pub fn mark_consumed(&mut self) {
        use std::sync::atomic::Ordering::{Acquire, Release};
        let w = self.header().seq_write.load(Acquire);
        self.header().seq_read.store(w, Release);
    }
}

impl Drop for ShmSurface {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.shm_path); }
}
```

---

## 3. VYOMA_VIDEO Protocol

All messages are UTF-8 text lines over existing stdout/stdin IPC. No binary data crosses this channel — only control messages. Frame pixels travel exclusively via shared memory.

### App → Supervisor (stdout)

```
VYOMA_VIDEO:open:<width>,<height>,<fps_num>/<fps_den>
VYOMA_VIDEO:present:<sid>:<pts_us>
VYOMA_VIDEO:seek:<sid>:<target_pts_us>
VYOMA_VIDEO:audio_pts:<sid>:<pts_us>
VYOMA_VIDEO:pause:<sid>
VYOMA_VIDEO:resume:<sid>:<resume_pts_us>
VYOMA_VIDEO:close:<sid>
```

### Supervisor → App (stdin)

```
VYOMA_VIDEO:session:<sid>:<shm_path>
VYOMA_VIDEO:ack:<sid>:<next_deadline_us>
VYOMA_VIDEO:sync_correction:<sid>:<delta_us>
VYOMA_VIDEO:seek_ack:<sid>:<pts_us>
VYOMA_VIDEO:error:<sid>:<code>:<message>
```

```rust
// supervisor/src/video/protocol.rs

#[derive(Debug)]
pub enum VideoCmd {
    Open { width: u32, height: u32, fps_num: u32, fps_den: u32 },
    Present { sid: u32, pts_us: u64 },
    Seek    { sid: u32, target_pts_us: u64 },
    AudioPts{ sid: u32, pts_us: u64 },
    Pause   { sid: u32 },
    Resume  { sid: u32, resume_pts_us: u64 },
    Close   { sid: u32 },
}

pub fn parse_video_cmd(line: &str) -> Option<VideoCmd> {
    let rest = line.strip_prefix("VYOMA_VIDEO:")?;
    let mut parts = rest.splitn(2, ':');
    let verb = parts.next()?;
    let args = parts.next().unwrap_or("");

    match verb {
        "open" => {
            let mut it = args.splitn(3, ',');
            let w = it.next()?.parse().ok()?;
            let h = it.next()?.parse().ok()?;
            let fps_str = it.next()?;
            let (n, d) = fps_str.split_once('/')?;
            Some(VideoCmd::Open { width: w, height: h,
                fps_num: n.parse().ok()?, fps_den: d.parse().ok()? })
        }
        "present" => {
            let (sid_s, pts_s) = args.split_once(':')?;
            Some(VideoCmd::Present { sid: sid_s.parse().ok()?, pts_us: pts_s.parse().ok()? })
        }
        "seek" => {
            let (sid_s, pts_s) = args.split_once(':')?;
            Some(VideoCmd::Seek { sid: sid_s.parse().ok()?, target_pts_us: pts_s.parse().ok()? })
        }
        "audio_pts" => {
            let (sid_s, pts_s) = args.split_once(':')?;
            Some(VideoCmd::AudioPts { sid: sid_s.parse().ok()?, pts_us: pts_s.parse().ok()? })
        }
        "pause"  => Some(VideoCmd::Pause  { sid: args.parse().ok()? }),
        "resume" => {
            let (sid_s, pts_s) = args.split_once(':')?;
            Some(VideoCmd::Resume { sid: sid_s.parse().ok()?, resume_pts_us: pts_s.parse().ok()? })
        }
        "close" => Some(VideoCmd::Close { sid: args.parse().ok()? }),
        _ => None,
    }
}

pub fn emit_session(sid: u32, shm_path: &str) -> String {
    format!("VYOMA_VIDEO:session:{}:{}\n", sid, shm_path)
}
pub fn emit_ack(sid: u32, next_deadline_us: u64) -> String {
    format!("VYOMA_VIDEO:ack:{}:{}\n", sid, next_deadline_us)
}
pub fn emit_sync_correction(sid: u32, delta_us: i64) -> String {
    format!("VYOMA_VIDEO:sync_correction:{}:{}\n", sid, delta_us)
}
```

---

## 4. Frame Delivery — Shared Memory Surface

This resolves B1 (bandwidth) and B4 (binary data in text protocol).

The WASM app and supervisor share a `/dev/shm/vyoma_vid_<sid>` file. The app mmaps it. The VYOMA_VIDEO protocol carries only control lines — never pixel data.

Frame write protocol (app side, double-buffered with atomic sequence counter):

```rust
// apps/video-player/src/shm_client.rs

use std::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
struct ShmHeader {
    seq_write: AtomicU64,
    seq_read:  AtomicU64,
    width:  u32,
    height: u32,
    _pad:   [u8; 40],
}

const HEADER_SIZE: usize = 64;

pub struct ShmFrameWriter { ptr: *mut u8, len: usize, width: u32, height: u32 }

impl ShmFrameWriter {
    /// Write a decoded RGBA frame. Returns false if previous frame not yet consumed.
    pub fn write_frame(&mut self, rgba: &[u8]) -> bool {
        assert_eq!(rgba.len(), (self.width * self.height * 4) as usize);
        let hdr = unsafe { &*(self.ptr as *const ShmHeader) };
        let seq_w = hdr.seq_write.load(Ordering::Acquire);
        let seq_r = hdr.seq_read.load(Ordering::Acquire);
        if seq_w != seq_r { return false; }  // supervisor still reading; drop frame

        hdr.seq_write.store(seq_w + 1, Ordering::Release);  // mark write-in-progress (odd)
        let pixel_slice = unsafe {
            std::slice::from_raw_parts_mut(self.ptr.add(HEADER_SIZE), rgba.len())
        };
        pixel_slice.copy_from_slice(rgba);
        hdr.seq_write.store(seq_w + 2, Ordering::Release);  // mark write-complete (even)
        true
    }
}
```

The supervisor compositor integrates `ShmSurface` into the existing R11 display pass:

```rust
// supervisor/src/video/mod.rs

impl VideoSubsystem {
    pub fn composite_pending_frames(&mut self, fb: &mut Framebuffer) {
        for session in self.sessions.values_mut() {
            if session.state != SessionState::Playing { continue; }
            if !session.surface.has_new_frame() { continue; }
            let now_us = timing::monotonic_us();
            if !session.frame_clock.is_due(now_us) { continue; }
            let rgba = session.surface.rgba_slice();
            fb.blit_rgba(session.surface.width, session.surface.height,
                rgba, session.window_x, session.window_y);
            session.surface.mark_consumed();
            session.frame_clock.advance();
            let next_deadline = session.frame_clock.next_deadline_us();
            self.send_to_app(&session.app_name, &emit_ack(session.session_id, next_deadline));
        }
    }
}
```

---

## 5. Audio/Video Sync

```rust
// supervisor/src/video/av_sync.rs

use std::time::Instant;

const DRIFT_THRESHOLD_US: i64 = 50_000;
const MAX_CORRECTION_US:  i64 = 500_000;

pub struct AvSyncClock {
    origin_wall:           Instant,
    origin_pts_us:         u64,
    last_audio_pts_us:     Option<u64>,
    last_audio_wall:       Option<Instant>,
    paused_at:             Option<Instant>,
    accumulated_pause_us:  u64,
}

impl AvSyncClock {
    pub fn new() -> Self {
        Self {
            origin_wall: Instant::now(), origin_pts_us: 0,
            last_audio_pts_us: None, last_audio_wall: None,
            paused_at: None, accumulated_pause_us: 0,
        }
    }

    pub fn start(&mut self, start_pts_us: u64) {
        self.origin_wall = Instant::now();
        self.origin_pts_us = start_pts_us;
        self.accumulated_pause_us = 0;
    }

    pub fn seek(&mut self, new_pts_us: u64) {
        self.origin_wall = Instant::now();
        self.origin_pts_us = new_pts_us;
        self.last_audio_pts_us = None;
        self.accumulated_pause_us = 0;
    }

    pub fn expected_video_pts_us(&self) -> u64 {
        let elapsed = self.origin_wall.elapsed().as_micros() as u64;
        self.origin_pts_us + elapsed - self.accumulated_pause_us
    }

    /// Returns Some(correction_us) if drift > threshold.
    /// positive correction → video is ahead → app should delay next frame
    pub fn update_audio_pts(&mut self, audio_pts_us: u64) -> Option<i64> {
        self.last_audio_pts_us = Some(audio_pts_us);
        self.last_audio_wall   = Some(Instant::now());
        let drift: i64 = self.expected_video_pts_us() as i64 - audio_pts_us as i64;
        if drift.abs() > DRIFT_THRESHOLD_US {
            let correction = drift.clamp(-MAX_CORRECTION_US, MAX_CORRECTION_US);
            let adjust = -(correction / 2);
            if adjust > 0 {
                self.origin_pts_us = self.origin_pts_us.saturating_add(adjust as u64);
            } else {
                self.origin_pts_us = self.origin_pts_us.saturating_sub((-adjust) as u64);
            }
            Some(correction)
        } else { None }
    }
}
```

---

## 6. Frame Rate Control

```rust
// supervisor/src/video/timing.rs

use std::time::Instant;

pub struct FrameClock {
    frame_interval_us: u64,
    session_start:     Instant,
    next_frame_index:  u64,
}

impl FrameClock {
    pub fn new(frame_interval_us: u64) -> Self {
        Self { frame_interval_us, session_start: Instant::now(), next_frame_index: 0 }
    }

    pub fn advance(&mut self) { self.next_frame_index += 1; }

    pub fn is_due(&self, now_us: u64) -> bool {
        now_us >= self.scheduled_us(self.next_frame_index)
    }

    fn scheduled_us(&self, frame_index: u64) -> u64 {
        let elapsed = self.session_start.elapsed().as_micros() as u64;
        let frame_offset = frame_index * self.frame_interval_us;
        monotonic_us() - elapsed + frame_offset
    }

    pub fn next_deadline_us(&self) -> u64 {
        self.scheduled_us(self.next_frame_index + 1)
    }
}

pub fn monotonic_us() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}
```

App-side paced decode loop:

```rust
// apps/video-player/src/decode_loop.rs

pub fn run_decode_loop(
    demuxer: &mut impl Demuxer,
    decoder: &mut impl Decoder,
    shm: &mut ShmFrameWriter,
    session_id: u32,
    stdout: &mut impl Write,
    stdin:  &mut impl BufRead,
) -> anyhow::Result<()> {
    let mut line_buf = String::new();
    loop {
        let packet = match demuxer.next_packet()? {
            Some(p) => p,
            None    => break,
        };
        let rgba = decoder.decode_rgba(&packet)?;
        let written = shm.write_frame(&rgba);
        if !written { continue; }  // supervisor busy; skip frame
        writeln!(stdout, "VYOMA_VIDEO:present:{}:{}", session_id, packet.pts_us)?;
        stdout.flush()?;
        line_buf.clear();
        stdin.read_line(&mut line_buf)?;
        let deadline_us = parse_ack_deadline(&line_buf, session_id)?;
        let now_us = SystemTime::now().duration_since(UNIX_EPOCH)
            .unwrap_or_default().as_micros() as u64;
        if deadline_us > now_us + 1_000 {
            std::thread::sleep(Duration::from_micros(deadline_us - now_us));
        }
    }
    Ok(())
}
```

---

## 7. Codec SDK for WASM Apps

Three tiers chosen by app based on format support needed:

### Tier 1 — Pure-Rust, small (recommended default)
- `mp4 = "0.14"` (~25 KB wasm) — ISO BMFF/MP4 demuxer
- `matroska = "0.14"` (~30 KB wasm) — MKV/WebM demuxer
- `symphonia = { version = "0.5", default-features = false, features = ["aac","mp3","vorbis"] }` — audio

### Tier 2 — WASM-compiled C, moderate size
- `openh264` compiled to wasm32 (~600 KB) — H.264
- `dav1d` via wasm32 (~350 KB) — AV1/VP9
- `yuv = "0.1"` — pure Rust YCbCr→RGBA

### Max WASM binary size enforcement

```toml
# apps/video-player/vyoma.toml
[app]
name           = "video-player"
max_wasm_kb    = 1500   # CI enforces: error if .wasm > this value

[capabilities]
stdio        = true
filesystem   = true
display      = true
video_decode = true    # R59 capability gate
```

---

## 8. Protocol State Machine

```
App                                    Supervisor
 |-- VYOMA_VIDEO:open:1920,1080,30/1 →|
 |← VYOMA_VIDEO:session:1:/dev/shm/...|
 | (app mmaps /dev/shm/vyoma_vid_1)   |
 |-- VYOMA_VIDEO:audio_pts:1:0 ------→|
 | [write frame to shm]               |
 |-- VYOMA_VIDEO:present:1:33333 ----→|
 |← VYOMA_VIDEO:ack:1:66666 ---------|
 | [sleep until 66666]                |
 | ...                                |
 |-- VYOMA_VIDEO:audio_pts:1:1200000 →|  drift > 50ms
 |← VYOMA_VIDEO:sync_correction:1:... |
 |-- VYOMA_VIDEO:close:1 -----------→|
```

---

## 9. Cargo Additions (supervisor)

```toml
[dependencies]
memmap2 = "0.9"   # safe mmap wrapper for ShmSurface; pure Rust, no C deps
```

No codec libraries. No FFmpeg. Total supervisor binary size impact: ~8 KB.

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|------------|
| **B1** — Bandwidth: 1080p@30fps = ~250 MB/s via stdout | Pixel data never crosses the IPC channel. A `/dev/shm/vyoma_vid_<sid>` mmap is shared between app and supervisor. Only a 1-line `VYOMA_VIDEO:present` control message is sent per frame. Bandwidth on IPC channel: ~40 bytes/frame × 30fps = 1.2 KB/s. |
| **B2** — A/V sync across separate channels | App reports audio PTS every ~500ms via `VYOMA_VIDEO:audio_pts`. Supervisor `AvSyncClock` computes drift and emits `VYOMA_VIDEO:sync_correction` when drift exceeds 50ms. Half-step convergence avoids overcorrection oscillation. |
| **B3** — WASM scheduling jitter | App does not self-time frames. After each `present`, it blocks on stdin for `ack:<next_deadline_us>`. Supervisor is the clock authority. App sleeps precisely until `next_deadline_us` using WASI `poll_oneoff`. |
| **B4** — Binary data in text protocol | Resolved by shm design: protocol lines are pure UTF-8 ASCII. RGBA pixels written directly into mmap region by app, read by supervisor without serialization. seq_write/seq_read atomic pair provides lock-free ISR-safe handoff. |
| **B5** — Codec WASM binary size | Three-tier codec menu. Apps enforce `max_wasm_kb` limit in `vyoma.toml` validated by `make check-manifests`. Default recommendation (VP8+WebM) fits under 200 KB. H.264 apps capped at 1500 KB. |
