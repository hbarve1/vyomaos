# Round 19 — Virtual Display & Screen Mirroring (Architect)

**Status**: Draft
**Round**: 19
**Subsystem**: Virtual Display & Screen Mirroring
**Analogue**: macOS AirPlay Display, Sidecar (iPad as second display)
**Author**: Architect
**Date**: 2026-05-29

---

## 1. Overview and Motivation

VyomaOS has a composited framebuffer (R11), per-app Surface buffers (R11), a full 2D
rendering API (R17), and screen capture (R18). The next logical capability is the ability
to project a pixel stream of the display — or a region of it — to a remote or virtual
consumer. This is what macOS calls AirPlay Display (project your Mac screen to an Apple
TV or another Mac) and Sidecar (use an iPad as a second physical display).

In VyomaOS, the analogous use cases are:

- **Mirror mode**: Duplicate the composited framebuffer pixel stream to a Unix socket or
  TCP connection. A remote viewer, a CI screenshot harness, or a streaming tool consumes
  it. Analogous to AirPlay mirroring.
- **Extend mode**: A virtual second display with its own logical resolution and coordinate
  space. WASM apps can be assigned to the extended display and render to it independently.
  Analogous to AirPlay extended desktop or Sidecar.
- **Remote-only mode**: A headless virtual display — no physical framebuffer, just a
  logical rendering target for testing or remote-desktop scenarios. Useful for CI
  automation and server-side rendering.

Without this subsystem, remote access to a running VyomaOS instance requires external
tools (VNC, framebuffer screenshots). With it, VyomaOS natively exposes its compositor
output as a stream that any consumer can receive with a plain socket read.

### macOS Analogues and Mapping

| macOS Feature | VyomaOS Equivalent |
|---------------|--------------------|
| AirPlay mirror to Apple TV | Mirror mode over TCP |
| AirPlay extended desktop | Extend mode, second logical screen |
| Sidecar iPad secondary display | Extend mode with USB/local transport |
| QuickTime remote screen | Mirror mode, Unix socket, single consumer |
| ScreenShare (VNC-based) | Remote-only mode + TCP transport |

### In Scope (v1)

- Mirror mode: copy compositor output as JPEG/PNG frame sequence to socket
- Extend mode: second logical display with independent resolution; apps placed on it
- Remote-only mode: fully virtual display, no physical fb backing
- Transports: Unix domain socket (local), TCP stream (network), virtio-vsock (QEMU guest↔host)
- Frame encoding: JPEG frame sequences (primary); raw BGRA32; PNG (optional, slow)
- Pixel format negotiation at session handshake
- Capability model: `virtual_display = true` in vyoma.toml; `virtual_display_any` for
  mirroring other apps' windows
- Platform gating: desktop-full and server-headless only; all others compile stubs

### Out of Scope (v1)

- WebRTC or RTSP transport — reserved for v2 (requires a full ICE/STUN stack)
- Audio channel multiplexing — deferred to R20 (audio subsystem)
- TLS/DTLS encryption — transport security considerations are addressed in section 10;
  TCP is restricted to loopback or requires pre-shared token
- Cursor synchronization (local cursor rendering on the virtual display consumer side)
- Input injection from the virtual display consumer back to the supervisor
- Multi-consumer fan-out per virtual display — single consumer per session in v1;
  fan-out reserved for v2
- H264/H265 video encoding — JPEG sequences only in v1; inter-frame codecs reserved for v2
- Dynamic resolution change mid-session

---

## 2. Virtual Display Modes

### 2.1 Mirror Mode

In mirror mode the supervisor reads the fully composited framebuffer (the output of the
vsync flush pass) after each compositor tick and encodes it as a frame. The encoded frame
is pushed into a bounded queue consumed by a dedicated `vdisp_worker` thread that writes
it to the transport socket.

Mirror mode adds no new rendering surface — it reads from the existing `FramebufferState`
that R11 maintains. The consumer sees exactly what the physical display shows, with a
latency of one vsync tick plus encoding time (< 40ms at 30fps JPEG quality 70).

Mirror mode has a 1:1 resolution correspondence with the physical display. The consumer
receives frames at the physical display's pixel dimensions. Downscaling is available as a
session parameter (e.g., 50% scale for lower-bandwidth connections).

### 2.2 Extend Mode

In extend mode the supervisor maintains an additional logical framebuffer alongside the
physical one. The extended display has its own:

- Width and height (configured at session creation; defaults to 1920×1080)
- Z-ordered Surface list (apps assigned to the extended display)
- Independent vsync tick (driven by the same vsync timer but written to a separate buffer)
- Coordinate space with origin `(0, 0)` relative to the extended display's own pixel space

Apps that declare `display = true` in their vyoma.toml can be assigned to the extended
display by a supervisor command or IPC message. Once assigned, the compositor routes their
`VYOMA_DRAW:` commands to the extended display's framebuffer rather than the physical one.

The extended display's composited pixel buffer is encoded and streamed to the consumer
using the same transport and encoding path as mirror mode. The physical display is
unaffected.

**Coordinate space**: The extended display's coordinate space is independent of the
physical display. `(0, 0)` is the top-left of the extended display. The supervisor must
communicate this resolution to WASM apps assigned to the extended display so they can
render at the correct dimensions. Section 5.4 defines the provisional API for this.

### 2.3 Remote-Only Mode

Remote-only mode creates a fully virtual display with no physical framebuffer backing. The
supervisor allocates an in-memory pixel buffer of the specified dimensions. Apps assigned
to it render into this buffer. The buffer is encoded and streamed to a consumer.

This mode is the primary path for:
- CI/CD automated screenshot verification: a test harness connects via Unix socket,
  runs the app, captures rendered frames, disconnects
- Server-side rendering for remote desktop: a user connects to a headless VyomaOS server
  and receives a rendered desktop session
- Display testing without physical hardware: verify rendering output on platforms without
  a physical GPU

Remote-only mode is available on server-headless and desktop-full profiles.

### 2.4 Mode Summary

| Mode | Source buffer | Physical display | Use case |
|------|--------------|-----------------|----------|
| Mirror | Physical fb (R11) | Unchanged | AirPlay, VNC |
| Extend | New virtual fb | Unchanged | Sidecar, dual monitor |
| Remote-only | New virtual fb | None / headless | CI, remote desktop |

---

## 3. Core Rust Types

```rust
// supervisor/src/vdisp/mod.rs

/// Unique identifier for a virtual display session.
pub type VDisplayId = u32;

/// The mode a virtual display operates in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VDisplayMode {
    /// Mirror the physical composited framebuffer.
    Mirror,
    /// Extend: second logical screen with own resolution.
    Extend { width: u32, height: u32 },
    /// Fully virtual display, no physical fb backing.
    RemoteOnly { width: u32, height: u32 },
}

/// Pixel format used for raw transport frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VDisplayPixelFormat {
    /// 4 bytes per pixel, blue-green-red-alpha order. Native fb format.
    Bgra32,
    /// 4 bytes per pixel, red-green-blue-alpha order.
    Rgba32,
    /// 3 bytes per pixel, no alpha.
    Rgb24,
}

/// Frame encoding for the transport stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VDisplayEncoding {
    /// Raw pixels in the negotiated VDisplayPixelFormat. No compression.
    Raw(VDisplayPixelFormat),
    /// JPEG with quality 0–100. Frame-independent (no inter-frame refs).
    Jpeg(u8),
    /// Lossless PNG. High CPU cost; use for archival or CI comparison only.
    Png,
}

/// Transport used to deliver encoded frames to the consumer.
#[derive(Debug, Clone)]
pub enum VDisplayTransport {
    /// Unix domain socket. Path on the host filesystem.
    UnixSocket { path: String },
    /// TCP stream. On loopback (127.0.0.1) only unless auth token is set.
    Tcp {
        addr: std::net::SocketAddr,
        /// Pre-shared auth token, required for non-loopback addresses.
        auth_token: Option<[u8; 32]>,
    },
    /// virtio-vsock. Guest port number; host connects via vsock.
    VirtioVsock { port: u32 },
}

/// Negotiated session parameters exchanged in the handshake.
#[derive(Debug, Clone)]
pub struct VDisplaySessionParams {
    /// Width of the virtual display in pixels.
    pub width: u32,
    /// Height of the virtual display in pixels.
    pub height: u32,
    /// Frame rate requested by the consumer; supervisor may cap it.
    pub fps: u8,
    /// Encoding agreed for this session.
    pub encoding: VDisplayEncoding,
    /// Scale factor applied to output (0.25–1.0). 1.0 = native resolution.
    pub scale: f32,
}

/// An active virtual display session.
pub struct VirtualDisplay {
    /// Unique ID assigned by VirtualDisplayEngine.
    pub id: VDisplayId,
    /// Owning app's instance ID.
    pub owner_app_id: u32,
    /// Display mode.
    pub mode: VDisplayMode,
    /// Transport for encoded frames.
    pub transport: VDisplayTransport,
    /// Negotiated session parameters.
    pub params: VDisplaySessionParams,
    /// Whether this session is currently streaming.
    pub active: bool,
    /// Total frames pushed to the transport.
    pub frames_sent: u64,
    /// Frames dropped due to slow consumer (send queue full or transport stall).
    pub frames_dropped: u64,
    /// For Extend/RemoteOnly modes: the virtual framebuffer.
    /// Mirror mode: None (reads from physical fb).
    pub virtual_fb: Option<Vec<u8>>,
    /// Sender side of bounded frame queue to vdisp_worker thread.
    pub frame_tx: std::sync::mpsc::SyncSender<VDisplayFrame>,
    /// Join handle for the vdisp_worker thread.
    pub worker_thread: Option<std::thread::JoinHandle<()>>,
}

/// A single encoded frame ready for transport.
#[derive(Debug)]
pub struct VDisplayFrame {
    /// Display session this frame belongs to.
    pub display_id: VDisplayId,
    /// Encoded bytes (JPEG data, PNG data, or raw pixels).
    pub data: Vec<u8>,
    /// Width of this frame in pixels (after any scaling).
    pub width: u32,
    /// Height of this frame in pixels (after any scaling).
    pub height: u32,
    /// Encoding used for this frame's data.
    pub encoding: VDisplayEncoding,
    /// Monotonic microseconds since supervisor epoch.
    pub timestamp_us: u64,
    /// Sequence number, monotonically increasing per session.
    pub seq: u64,
}

/// The central engine owned by the supervisor's main state.
pub struct VirtualDisplayEngine {
    /// Next display ID counter.
    pub next_id: VDisplayId,
    /// Active virtual displays keyed by ID.
    pub displays: std::collections::HashMap<VDisplayId, VirtualDisplay>,
    /// Per-app permission records.
    pub permissions: std::collections::HashMap<u32, VDisplayPermission>,
    /// Maximum simultaneous virtual displays (platform-configured).
    pub max_displays: u8,
}

/// Per-app permission state for virtual display.
pub struct VDisplayPermission {
    /// App instance ID.
    pub app_id: u32,
    /// Can create a virtual display for own output.
    pub virtual_display_own: bool,
    /// Can mirror any app or the full screen (requires runtime consent).
    pub virtual_display_any: bool,
    /// Runtime consent granted (initialized false; set by chrome app response).
    pub consent_granted: bool,
}

impl VirtualDisplayEngine {
    pub fn new(max_displays: u8) -> Self;
    pub fn check_permission(&self, app_id: u32, mode: &VDisplayMode) -> Result<(), String>;
    pub fn alloc_id(&mut self) -> VDisplayId;
    pub fn insert(&mut self, display: VirtualDisplay);
    pub fn get_mut(&mut self, id: VDisplayId) -> Option<&mut VirtualDisplay>;
    pub fn remove(&mut self, id: VDisplayId) -> Option<VirtualDisplay>;
    pub fn displays_for_app(&self, app_id: u32) -> Vec<VDisplayId>;
}
```

---

## 4. Transport Layer

The transport layer is responsible for moving encoded `VDisplayFrame` bytes from the
`vdisp_worker` thread to the consumer. The transport is non-blocking from the worker's
perspective: if the consumer cannot receive a frame in time, the frame is dropped.

### 4.1 Frame Wire Format

Before sending encoded frame bytes, the transport prepends a fixed 24-byte frame header:

```
Offset  Size  Field
0       4     Magic: 0x56445350 ("VDSP")
4       4     Display ID (little-endian u32)
8       8     Sequence number (little-endian u64)
16      4     Payload length in bytes (little-endian u32)
20      1     Encoding: 0=Raw, 1=JPEG, 2=PNG
21      1     Pixel format (for Raw only): 0=BGRA32, 1=RGBA32, 2=RGB24
22      2     Reserved (zero)
```

The header is followed immediately by `payload_length` bytes of encoded frame data.
A consumer that reads this stream can frame messages purely from the header without
needing to parse JPEG/PNG structure.

### 4.2 Session Handshake

On connection, the supervisor sends a 64-byte handshake record before any frame headers:

```
Offset  Size  Field
0       4     Magic: 0x56445348 ("VDSH")
4       4     Protocol version: 1
8       4     Display width in pixels
12      4     Display height in pixels
16      1     Negotiated fps cap
17      1     Negotiated encoding (same codes as wire header byte 20)
18      1     Negotiated pixel format
19      1     Flags: bit 0 = mirror mode, bit 1 = extend mode, bit 2 = remote-only
20      44    Reserved (zero)
```

The consumer must read and validate the handshake before processing frame headers. If
the magic bytes do not match, the consumer must close the connection.

### 4.3 Unix Domain Socket Transport

The supervisor creates a Unix domain socket at the path specified in `VDisplayTransport::UnixSocket`.
The default path pattern is `/run/vyoma/vdisp/<display_id>.sock`. The supervisor creates
the `/run/vyoma/vdisp/` directory on first use.

The socket is a stream socket (`SOCK_STREAM`). The supervisor `accept()`s one connection per
virtual display (single-consumer model in v1). If a second connection arrives before the
first is closed, it is refused with `ECONNREFUSED`.

**Backpressure at the transport layer**: The `vdisp_worker` thread calls `write_all` with
a non-blocking socket in a try-write pattern. If the kernel socket send buffer is full
(consumer not reading), `write_all` returns `WouldBlock`. The worker increments
`frames_dropped` and discards the frame. This is the same drop-not-block model as the
R18 capture bounded queue, extended to the transport layer.

Specifically:
1. `vdisp_worker` receives a frame from the bounded `frame_rx` channel.
2. It calls `socket.set_nonblocking(true)` once at connection time.
3. It attempts `write_all(header + payload)`. On `WouldBlock`, the frame is dropped.
4. The worker does NOT retry on `WouldBlock` — it moves to the next frame tick.

This prevents a slow consumer from filling kernel socket buffers indefinitely, which
would exhaust kernel memory. The kernel socket send buffer size is left at its system
default (~128KB on Linux 5.10); at JPEG quality 70 the average 1080p frame is ~200KB,
so one partially-filled frame can fill the buffer. A consumer must read frames promptly
(within one frame interval = 33ms at 30fps) or risk drops.

### 4.4 TCP Stream Transport

The supervisor opens a `TcpListener`. For non-loopback addresses, an auth token exchange
is required before the handshake record is sent. See section 10 for the full security
analysis.

TCP transport uses the same non-blocking write pattern as Unix socket. The kernel TCP
send buffer (default ~64–128KB) provides minimal burst absorption.

**Loopback restriction**: If `VDisplayTransport::Tcp.auth_token` is `None`, the supervisor
enforces that `addr.ip()` is a loopback address (`127.0.0.1` or `::1`). Attempts to create
a TCP virtual display on a non-loopback address without an auth token are rejected at
`create-display` call time with error `"non-loopback TCP requires auth_token"`.

### 4.5 virtio-vsock Transport

`virtio-vsock` (AF_VSOCK) provides a bidirectional byte stream between a QEMU guest and the
host. The supervisor listens on the specified `port` using `SO_VMADDR_CID_ANY` (accept from
any CID, typically the host). The transport is otherwise identical to Unix socket: stream
socket, one consumer, non-blocking writes.

This transport is the recommended path for host-side tools (e.g., a macOS app that wants
to display a VyomaOS virtual desktop without opening a TCP port). The QEMU launch flags
must include `-device vhost-vsock-pci,guest-cid=3` for the guest CID to be stable.

### 4.6 Frame Queue Between Compositor and Worker

The compositor runs on the vsync thread. The `vdisp_worker` runs on a dedicated thread.
Between them is a `std::sync::mpsc::SyncSender<VDisplayFrame>` with capacity 2.

Capacity 2 is intentionally smaller than R18's capture queue (capacity 4). Virtual display
is a live streaming use case; accumulating stale frames does not help the consumer —
it adds latency. With capacity 2, the consumer always receives frames that are at most
2 vsync ticks old (33ms at 30fps = ~66ms maximum queue latency).

When the queue is full (`TrySendError::Full`), the compositor drops the new frame and
increments `VirtualDisplay::frames_dropped`. The compositor never blocks waiting for the
worker.

---

## 5. WIT Interface `vyoma:virtual-display@1.0.0`

```wit
package vyoma:virtual-display@1.0.0;

/// Virtual display operating mode.
variant display-mode {
    /// Mirror the physical composited framebuffer.
    mirror,
    /// Extend: second logical screen. Fields: width, height.
    extend(display-size),
    /// Fully virtual, no physical backing.
    remote-only(display-size),
}

/// Display dimensions.
record display-size {
    width: u32,
    height: u32,
}

/// Pixel format for raw transport frames.
enum pixel-format {
    bgra32,
    rgba32,
    rgb24,
}

/// Frame encoding for the transport stream.
variant frame-encoding {
    /// Uncompressed raw pixels in the specified format.
    raw(pixel-format),
    /// JPEG, quality 0–100.
    jpeg(u8),
    /// Lossless PNG.
    png,
}

/// Transport used to deliver frames to the consumer.
variant transport-spec {
    /// Unix domain socket at the given path.
    unix-socket(string),
    /// TCP loopback only (no auth token required).
    tcp-loopback(u16),
    /// TCP with pre-shared 32-byte auth token (allows non-loopback).
    tcp-auth(tcp-auth-params),
    /// virtio-vsock on the given port.
    vsock(u32),
}

record tcp-auth-params {
    addr: string,
    port: u16,
    /// Must be exactly 32 bytes; supervisor rejects other lengths.
    token: list<u8>,
}

/// Parameters for creating a virtual display.
record create-params {
    mode: display-mode,
    transport: transport-spec,
    /// Target frames per second (1–60; supervisor caps per platform).
    fps: u8,
    encoding: frame-encoding,
    /// Output scale factor 0.25–1.0; 1.0 = native resolution.
    scale: f32,
}

/// Information about an active virtual display.
record display-info {
    id: u32,
    mode: display-mode,
    width: u32,
    height: u32,
    fps: u8,
    encoding: frame-encoding,
    frames-sent: u64,
    frames-dropped: u64,
    active: bool,
}

interface virtual-display {
    /// Create a new virtual display session.
    /// Requires capability: virtual_display = true in vyoma.toml.
    /// For mirror or any-window capture, also requires virtual_display_any
    /// and runtime consent (desktop-full only).
    /// Returns display ID on success.
    create-display: func(params: create-params) -> result<u32, string>;

    /// Destroy a virtual display session.
    /// Closes the transport connection and stops the worker thread.
    destroy-display: func(id: u32) -> result<_, string>;

    /// In mirror mode: set a specific app's Surface as the mirror source
    /// instead of the full composited framebuffer.
    /// Requires virtual_display_any capability and consent.
    set-mirror-source: func(id: u32, app-id: u32) -> result<_, string>;

    /// In extend or remote-only mode: push a raw BGRA32 frame into the
    /// virtual display's framebuffer. Apps that render to an extended display
    /// via VYOMA_DRAW: do not need to call this — the compositor writes it.
    /// This function is for apps that want to push pixel data directly
    /// without going through the VYOMA_DRAW: protocol.
    push-frame: func(
        id: u32,
        width: u32,
        height: u32,
        pixels: list<u8>,
    ) -> result<_, string>;

    /// Connect the transport (begin accepting a consumer connection).
    /// After this call the supervisor begins listening on the transport socket.
    /// Returns immediately; the consumer connects asynchronously.
    connect: func(id: u32) -> result<_, string>;

    /// Disconnect the transport. Closes the consumer connection if active.
    /// The virtual display remains allocated; call connect again to reuse.
    disconnect: func(id: u32) -> result<_, string>;

    /// Return status information for a virtual display session.
    display-info: func(id: u32) -> result<display-info, string>;

    /// Query the resolution of the virtual display identified by id.
    /// Provisionally used by apps on extended displays to know their canvas size.
    /// Returns (width, height) in physical pixels.
    display-resolution: func(id: u32) -> result<tuple<u32, u32>, string>;

    /// Query dropped frame count since last call (destructive read).
    dropped-frames: func(id: u32) -> result<u64, string>;
}

world virtual-display-world {
    import virtual-display;
}
```

### 5.4 Provisional Coordinate-Space API for Extend Mode

Extend mode requires that WASM apps assigned to the extended display know its resolution
and that the supervisor routes their `VYOMA_DRAW:` commands to the correct framebuffer.
This is a provisional design pending R20 (HiDPI) and R21 (Window Manager), which will
define the full screen-space coordinate model.

**Assignment**: The supervisor command `vdisp assign <display_id> <app_name>` (or IPC
`@supervisor: vdisp_assign:<display_id>:<app_name>`) moves an app's rendering target from
the physical display to the extended display. The supervisor updates the app's
`AppState::target_display` field.

**Resolution advertisement**: When an app is assigned to an extended display, the supervisor
sends it a single stdin line:
```
VYOMA_VDISP_SCREEN:<display_id>,<width>,<height>
```
This tells the app its canvas dimensions. All subsequent `VYOMA_DRAW:` coordinate values
are interpreted in the extended display's pixel space.

**Coordinate space contract**: The extended display's coordinate space has origin `(0, 0)` at
its top-left corner. There is no global coordinate space spanning physical + extended
displays in v1. Apps on the extended display cannot reference coordinates on the physical
display and vice versa. Cross-display coordinate mapping is deferred to R21.

**WIT equivalent**: `display-resolution(id)` returns `(width, height)` for the specified
virtual display. An app assigned to an extended display calls this once at initialization
to determine its canvas size, then renders accordingly.

---

## 6. VYOMA_VDISP: Stdout Protocol

For apps that do not use WIT component model linking, the supervisor parses
`VYOMA_VDISP:` lines from stdout. This mirrors the `VYOMA_DRAW:` and `VYOMA_CAPTURE:`
patterns.

### 6.1 Commands (app → supervisor via stdout)

```
VYOMA_VDISP:create:<mode_spec>,<transport_spec>,<fps>,<encoding_spec>,<scale>
VYOMA_VDISP:destroy:<display_id>
VYOMA_VDISP:set_mirror_source:<display_id>,<app_id>
VYOMA_VDISP:push_frame:<display_id>,<width>,<height>,<pixel_count>
VYOMA_VDISP:connect:<display_id>
VYOMA_VDISP:disconnect:<display_id>
VYOMA_VDISP:info:<display_id>
VYOMA_VDISP:resolution:<display_id>
VYOMA_VDISP:dropped:<display_id>
```

Mode spec values:
- `mirror` — mirror physical fb
- `extend:<width>,<height>` — extended display
- `remote:<width>,<height>` — remote-only virtual display

Transport spec values:
- `unix:<path>` — Unix domain socket
- `tcp_lo:<port>` — TCP loopback
- `tcp_auth:<addr>:<port>:<token_hex>` — TCP with 32-byte hex auth token
- `vsock:<port>` — virtio-vsock

Encoding spec values:
- `raw_bgra` — raw BGRA32
- `raw_rgba` — raw RGBA32
- `raw_rgb` — raw RGB24
- `jpeg:<quality>` — JPEG quality 0–100
- `png` — PNG

**push_frame detail**: After writing the `push_frame` command line, the app immediately
writes exactly `pixel_count * 4` bytes (BGRA32 pixels) to stdout as raw binary. The
supervisor reads the pixel data before returning to line parsing. This is the same pattern
as a potential future binary extension of the VYOMA_DRAW protocol.

### 6.2 Responses (supervisor → app via stdin)

```
VYOMA_VDISP_CREATED:<display_id>
VYOMA_VDISP_DESTROYED:<display_id>
VYOMA_VDISP_CONNECTED:<display_id>
VYOMA_VDISP_DISCONNECTED:<display_id>
VYOMA_VDISP_INFO:<display_id>,<mode>,<width>,<height>,<fps>,<frames_sent>,<frames_dropped>,<active>
VYOMA_VDISP_SCREEN:<display_id>,<width>,<height>
VYOMA_VDISP_DROPPED:<display_id>,<count>
VYOMA_VDISP_ERROR:<reason>
```

### 6.3 Example (Rust app using protocol)

```rust
// Create a 30fps JPEG mirror over Unix socket
println!("VYOMA_VDISP:create:mirror,unix:/run/vyoma/vdisp/0.sock,30,jpeg:70,1.0");
let mut buf = String::new();
std::io::stdin().read_line(&mut buf).unwrap();
// buf: "VYOMA_VDISP_CREATED:0\n"

// Begin listening for consumer connection
println!("VYOMA_VDISP:connect:0");
std::io::stdin().read_line(&mut buf).unwrap();
// buf: "VYOMA_VDISP_CONNECTED:0\n"

// ... consumer connects and receives the frame stream ...

// Query drop stats
println!("VYOMA_VDISP:dropped:0");
std::io::stdin().read_line(&mut buf).unwrap();
// buf: "VYOMA_VDISP_DROPPED:0,12\n"  — 12 frames dropped

// Destroy on exit
println!("VYOMA_VDISP:destroy:0");
```

---

## 7. Compositor Integration

### 7.1 Where VirtualDisplayEngine Hooks In

The existing vsync flush pass in `supervisor/src/display/` (R11) follows this sequence:

1. Acquire `vsync_lock.write()` — exclusive compositor lock
2. Z-sort apps by Z-order
3. Blit each app's Surface onto the physical framebuffer in order
4. Blit chrome overlays
5. DRM/virtio-gpu blit to hardware
6. Release `vsync_lock.write()`

R18 screen capture reads from the framebuffer after step 6 by acquiring `vsync_lock.read()`.

Virtual display mirror mode inserts a step **after step 6, before the next vsync tick**:

7. Acquire `vsync_lock.read()` (shared — compatible with other readers, R18 capture)
8. Identify all active mirror-mode VirtualDisplay sessions
9. For each: snapshot the framebuffer into a raw `Vec<u8>` (memcpy, ~2ms for 1080p)
10. Release `vsync_lock.read()`
11. For each session: attempt `frame_tx.try_send(VDisplayFrame { data: raw_pixels, ... })`
    - On `TrySendError::Full`: increment `frames_dropped`, discard frame
    - On success: `vdisp_worker` thread encodes and writes to transport

Steps 9–11 happen on the vsync thread (after the hardware blit). The memcpy in step 9 is
the only operation that holds the read-lock; encoding and transport I/O are entirely
off-thread in the `vdisp_worker`.

### 7.2 Extend Mode Compositor Pass

For extend mode and remote-only mode, the compositor runs an additional pass:

After the physical display flush pass (steps 1–6 above):

A. Acquire the extended display's per-display write lock (separate from `vsync_lock`)
B. Z-sort apps assigned to the extended display
C. Blit their Surfaces onto the virtual framebuffer
D. Release the per-display write lock
E. Acquire `vsync_lock.read()` (shared with mirror read)
F. Snapshot the virtual framebuffer → `VDisplayFrame`
G. Release `vsync_lock.read()`
H. Try-send to `vdisp_worker`

The extended display uses a dedicated `RwLock<Vec<u8>>` for its pixel buffer, separate
from the physical framebuffer and from `vsync_lock`. This means the extended display
compositor pass does not contend with the physical display flush.

### 7.3 Encoding Does NOT Block the Compositor

The `vdisp_worker` thread handles all encoding. The compositor thread (vsync thread) only
performs:
- A `vsync_lock.read()` for the memcpy (< 2ms)
- A `try_send` into the bounded queue (< 1µs)

Encoding (JPEG at quality 70 for 1080p: ~25–35ms) runs entirely on `vdisp_worker` and
never holds the vsync lock.

### 7.4 Frame Skip When Encoder Is Behind

If the `vdisp_worker` thread has not finished encoding and sending the previous frame when
the next vsync tick produces a new snapshot:

1. The compositor tries `try_send` on the bounded queue (capacity 2).
2. If the queue is full (worker busy), `TrySendError::Full` is returned immediately.
3. The new frame is discarded; `frames_dropped` is incremented.
4. The compositor continues — it is never blocked.

This is the correct resolution of the tearing concern: the encoder is skipped for that
frame, not stalled. The consumer receives a slightly lower effective frame rate during
periods of high encoder load. The vsync compositor is unaffected.

**The read-lock is never held for longer than the memcpy.** The encoder works on a
copied buffer, not the live framebuffer. There is no possibility of the encoder holding
`vsync_lock.read()` while the physical compositor needs `vsync_lock.write()`.

### 7.5 Multiple Virtual Displays and vsync Lock Contention Analysis

With 2 simultaneous virtual displays (the desktop-full limit) both in mirror mode:

- Both sessions perform memcpy on the same vsync tick, both holding `vsync_lock.read()`.
- `std::sync::RwLock` allows concurrent readers — two simultaneous read-locks do NOT
  contend with each other.
- Total memcpy time: ~4ms (2 × ~2ms, but the second copy can begin immediately after
  the first since both hold the read-lock concurrently).
- The compositor's `vsync_lock.write()` for the next frame's flush pass must wait until
  all active read-locks are released.
- Worst case: both copies run sequentially on the vsync post-flush step, total ~4ms.
- With a 16ms vsync budget (60fps) or 33ms budget (30fps), 4ms of read-lock hold after
  each flush is acceptable: it means the next frame's write-lock acquisition is delayed
  by up to 4ms, compressing the next frame's compositor time budget to ~29ms (30fps case).
- At 4 simultaneous virtual displays (server-headless limit): ~8ms of total read-lock
  hold. Still within a 33ms budget at 30fps physical display rate.

The key invariant: `vsync_lock.read()` for virtual display snapshots is always acquired
**after** the physical display flush, never during. The write-lock for the current frame's
flush is already released before any virtual display read-lock is acquired.

---

## 8. Capability Model

### 8.1 Capability Declaration

```toml
# vyoma.toml — app declares what it needs

[capabilities]
stdio              = true
virtual_display    = true   # required for any virtual display operation
virtual_display_any = true  # required for mirror mode / cross-app pixel access
                             # requires runtime consent on desktop-full
```

`virtual_display` allows an app to:
- Create extend mode and remote-only virtual displays for its own output
- Stream its own rendered pixels via a virtual display transport

`virtual_display_any` additionally allows:
- Mirror mode (reads the full composited framebuffer — all apps' pixels)
- `set-mirror-source` (reads a specific other app's Surface)

### 8.2 Link-Time Capability Gating (same pattern as R18 B1 fix)

The `vyoma:virtual-display@1.0.0` WIT host functions are conditionally added to the
Wasmtime `Linker` at app instantiation time, not globally. The per-app `SupervisorCtx`
carries `app_id`, and the `add_to_linker` function gates registration on the manifest:

```rust
// supervisor/src/vdisp/wit_handlers.rs
pub fn add_to_linker(
    linker: &mut wasmtime::Linker<SupervisorCtx>,
    manifest: &AppManifest,
) -> anyhow::Result<()> {
    if !manifest.capabilities.virtual_display {
        // Do not register virtual display host functions for this app.
        // If the WASM binary imports them, Wasmtime will return an
        // instantiation error (unresolved import), not a runtime error.
        return Ok(());
    }
    // Register functions gated on virtual_display = true
    linker.func_wrap("vyoma:virtual-display/virtual-display", "create-display", ...)?;
    linker.func_wrap("vyoma:virtual-display/virtual-display", "destroy-display", ...)?;
    linker.func_wrap("vyoma:virtual-display/virtual-display", "connect", ...)?;
    linker.func_wrap("vyoma:virtual-display/virtual-display", "disconnect", ...)?;
    linker.func_wrap("vyoma:virtual-display/virtual-display", "display-info", ...)?;
    linker.func_wrap("vyoma:virtual-display/virtual-display", "display-resolution", ...)?;
    linker.func_wrap("vyoma:virtual-display/virtual-display", "dropped-frames", ...)?;
    linker.func_wrap("vyoma:virtual-display/virtual-display", "push-frame", ...)?;

    if manifest.capabilities.virtual_display_any {
        // set-mirror-source requires the higher capability.
        linker.func_wrap("vyoma:virtual-display/virtual-display", "set-mirror-source", ...)?;
    }
    Ok(())
}
```

An app that does not declare `virtual_display = true` in its manifest will not have these
imports registered. If the WASM binary attempts to import them anyway, Wasmtime returns
`InstantiationError::Trap` at link time (before `_start` runs). This is a stronger
guarantee than a runtime permission check.

### 8.3 Runtime Permission Check for Mirror Mode

Even with link-time gating, the supervisor performs a runtime check at `create-display`
call time for mirror mode and `set-mirror-source`:

```
if mode == Mirror or set_mirror_source is called:
    if not permission.virtual_display_any:
        return Err("virtual_display_any capability required")
    if not permission.consent_granted:
        post consent request to chrome app
        return Err("consent_pending")
```

Mirror mode exposes all apps' pixels to the stream consumer — equivalent to `capture_any`
in R18. The consent model follows the same chrome-app IPC pattern:
- Supervisor sends `@chrome: vdisp_consent_request:<app_name>:<app_id>` to the chrome app
- Chrome app draws a modal: "App `<name>` wants to mirror your screen. Allow?"
- User responds; chrome sends `@supervisor: vdisp_consent_grant:<app_id>` or `vdisp_consent_deny:<app_id>`
- Supervisor sets `VDisplayPermission::consent_granted = true/false`

Consent is stored in memory only (in-session). It is not persisted to disk in v1.

### 8.4 Manifest Parser Updates

`supervisor/src/manifest.rs` adds two new optional boolean fields:
- `capabilities.virtual_display` — defaults to `false`
- `capabilities.virtual_display_any` — defaults to `false`; implies `virtual_display`

The `check-manifests` Makefile target validates these fields.

---

## 9. Platform Matrix

| Feature | mcu-minimal | iot-edge | robotics-rt | server-headless | mobile | desktop-full |
|---------|:-----------:|:--------:|:-----------:|:---------------:|:------:|:------------:|
| Mirror mode | no | no | no | yes | no | yes |
| Extend mode | no | no | no | no | no | yes |
| Remote-only mode | no | no | no | yes | no | yes |
| Unix socket transport | no | no | no | yes | no | yes |
| TCP loopback transport | no | no | no | yes | no | yes |
| TCP auth transport | no | no | no | yes | no | yes |
| virtio-vsock transport | no | no | no | yes | no | yes |
| JPEG encoding | no | no | no | yes | no | yes |
| PNG encoding | no | no | no | yes (opt) | no | yes (opt) |
| Raw encoding | no | no | no | yes | no | yes |
| virtual_display_any perm | no | no | no | no | no | yes |
| Max simultaneous displays | 0 | 0 | 0 | 4 | 0 | 2 |
| Max fps (JPEG) | — | — | — | 30 | — | 60 |

**Platform compile-time gating**: MCU/IoT/Robotics/Mobile profiles compile the
`supervisor/src/vdisp/` module as a stub that returns `Err("virtual_display not
available on this platform")` from all functions. The stub is enforced by a Cargo feature
flag `vdisp` that is omitted from those platform profiles' feature sets. The vdisp module
is not compiled into the supervisor binary for these platforms.

**Server-headless specifics**: Server-headless allows mirror mode (useful for remote
desktop scenarios: a server process mirrors its own composited output, which may be the
output of headless WASM apps), remote-only mode, and all transport types. It does NOT
support extend mode (no second physical display to extend to; the "physical" display is
already virtual on server-headless). It does NOT grant `virtual_display_any` (no interactive
user to show consent dialog; admin-level server tasks use remote-only or mirror of own
output only).

**Mobile**: Mobile is excluded from virtual display in v1 due to the power and memory
implications of a continuous 30fps encoding loop. This is revisited in v2 when per-frame
dirty-rect encoding reduces CPU load.

---

## 10. Security Considerations

### 10.1 Threat Model

A virtual display consumer receives a continuous pixel stream of whatever the virtual
display is showing. In mirror mode, that is the entire composited framebuffer — every
app's rendered output, every UI dialog, every password field that renders as bullets.
The security threat is equivalent to `capture_any` in R18: an attacker who obtains a
connection to the mirror transport socket has passive screen surveillance capability.

### 10.2 Unix Socket Transport

The Unix socket is created in `/run/vyoma/vdisp/` with permissions `600` (owner-only
read/write). Only processes running as the same user as the supervisor (the init process,
UID 0 in VyomaOS's single-user model) can connect. This is equivalent to loopback-only
TCP for local consumers. No additional authentication is required for Unix socket transport.

### 10.3 TCP Transport — Loopback Restriction and Auth Token

TCP transport on non-loopback addresses exposes the pixel stream to any network client.
V1 addresses this with two mechanisms:

1. **Loopback-only default**: If no auth token is provided, the supervisor refuses
   to create a TCP virtual display that listens on a non-loopback address. The enforcement
   is at `create-display` call time (before the socket is even opened).

2. **Pre-shared auth token for non-loopback**: If `VDisplayTransport::Tcp.auth_token`
   is `Some([u8; 32])`, the supervisor requires the connecting consumer to prove knowledge
   of the token before sending the handshake record. The token exchange:
   - Supervisor sends a 16-byte random nonce on connection
   - Consumer sends `HMAC-SHA256(token, nonce)` (32 bytes)
   - Supervisor verifies; on mismatch, closes the connection with no data sent

   This is not TLS. It provides authentication (only token-holders can connect) but
   not encryption (the pixel stream is plaintext on the network). V1 accepts this: the
   use case for non-loopback TCP virtual display is trusted LAN environments (developer
   workstation mirroring to a local display server). TLS is deferred to v2.

3. **Explicit threat acceptance**: If an app creates a non-loopback TCP virtual display
   with an auth token, it has explicitly opted into network exposure. The token prevents
   casual interception but not a MITM on the same network segment. The supervisor logs a
   WARNING at creation time: `[vdisp] TCP non-loopback virtual display created on <addr>:<port>; stream is unencrypted`.

### 10.4 virtio-vsock Transport

vsock connections are mediated by the QEMU hypervisor. Only the host (CID=2) and the
guest (CID=3) can communicate via vsock; no external network entity can initiate a vsock
connection. This makes vsock the safest transport for local host-guest communication.
No additional authentication is required.

### 10.5 `virtual_display_any` Is Equivalent to `capture_any`

Mirror mode and `set-mirror-source` expose other apps' pixels. They require the same
runtime consent mechanism as `capture_any` (R18). The consent request, grant, and deny
flow is identical (chrome app modal dialog). Consent is stored per-app in memory for the
session duration.

An app with `virtual_display_any = true` that has not received consent will be denied
at every mirror create or set-mirror-source call. The supervisor sends the consent request
once; subsequent denials do not re-prompt (to prevent dialog spam).

### 10.6 Max Simultaneous Displays

The per-platform limit (2 on desktop-full, 4 on server-headless) is enforced at
`create-display` time. The `VirtualDisplayEngine::max_displays` field is set from the
platform profile at supervisor startup. Exceeding the limit returns `Err("max virtual
displays reached")`. This limits the CPU and memory load from concurrent encoding.

---

## 11. Performance Notes

### 11.1 Throughput Estimate at 30fps, 1920×1080, JPEG Quality 70

```
Frame size (raw):       1920 × 1080 × 4 bytes   = 8,294,400 bytes
JPEG quality 70 output: ~150–300KB per frame       (typical desktop UI content)
Frame rate:             30 fps
Encoded throughput:     30 × 250KB = ~7.5 MB/s    (network bandwidth required)
```

A standard 100Mbps LAN can sustain this throughput. For a Unix socket or vsock consumer
on the same machine, memory bandwidth is the limit — at ~7.5MB/s of JPEG data, well
within the ~10GB/s memory bandwidth of a modern CPU.

### 11.2 Frame Queue Sizing

Queue capacity is 2 frames (smaller than R18's 4 because latency matters more than burst
absorption for a live stream):

- At 30fps, one frame interval = 33ms
- With capacity 2: maximum queue latency = 66ms (2 frames buffered)
- If encoding takes > 66ms (uncommon at quality 70 but possible under load), frames are dropped
- Target: encoding time < 33ms per frame at quality 70 on desktop-full hardware

### 11.3 Drop Policy

When either the frame queue or the transport write stalls:
1. Frame queue full → drop incoming frame from compositor, increment `frames_dropped`
2. Transport write stall (`WouldBlock`) → drop frame from worker, increment `frames_dropped`
3. Both are non-blocking: neither the compositor nor the worker waits

There is no adaptive rate control in v1. An app that cares about drop rate should poll
`dropped-frames` and reduce its requested fps via `destroy-display` + `create-display`
with lower fps. Adaptive rate control (auto-reduce fps on sustained drops) is a v2
feature.

### 11.4 Memory Allocation Per Session

| Component | Size per session |
|-----------|-----------------|
| Raw frame buffer (memcpy destination) | ~8MB |
| Encode temp buffer (JPEG) | ~2MB |
| Frame queue (2 raw frames) | ~16MB |
| Virtual fb for extend/remote-only mode | ~8MB |
| Total per session (approx.) | ~34MB |

Two simultaneous sessions on desktop-full: ~68MB. Within the 512MB desktop RAM budget.
On server-headless with 4 sessions: ~136MB. Within the 1GB server RAM budget.

### 11.5 Thread Budget

Per session: 1 `vdisp_worker` thread (encodes + sends). The compositor tick that performs
the memcpy runs on the existing vsync thread — no new thread per session for the read step.

Worst case on desktop-full: 2 virtual display sessions = 2 additional threads beyond the
existing app threads. On server-headless: 4 sessions = 4 additional threads. Total
system thread count remains well under Linux's default per-process limit of 1024.

---

## 12. File Layout

The virtual display subsystem lives under `supervisor/src/vdisp/`. Each file is bounded
at 500 lines per the VyomaOS code size rule.

```
supervisor/src/vdisp/
├── mod.rs           — VirtualDisplayEngine, VirtualDisplay, all core types (~400 lines)
├── compositor.rs    — vsync integration: snapshot-after-flush, try_send to worker (~300 lines)
├── encoder.rs       — JPEG/PNG/raw encoding for vdisp frames; reuses logic from capture (~350 lines)
├── worker.rs        — vdisp_worker thread: receive from queue, send to transport (~350 lines)
├── transport.rs     — Unix socket / TCP / vsock accept, non-blocking write, handshake (~450 lines)
├── extend.rs        — extend and remote-only mode: virtual fb lifecycle, compositor pass (~400 lines)
├── wit_handlers.rs  — WIT linker: maps vyoma:virtual-display@1.0.0 host functions (~350 lines)
└── protocol.rs      — VYOMA_VDISP: line parser and response formatter (~300 lines)
```

### 12.1 `mod.rs` — Engine and Types (~400 lines)

Defines all public types from section 3. `VirtualDisplayEngine` owns the `displays`
HashMap and `permissions` HashMap. Provides methods: `new`, `check_permission`, `alloc_id`,
`insert`, `get_mut`, `remove`, `displays_for_app`.

### 12.2 `compositor.rs` — vsync Integration (~300 lines)

Called from the supervisor's vsync flush pass (after the physical fb write). Acquires
`vsync_lock.read()`, iterates active mirror-mode sessions, clones the framebuffer, releases
the lock, and calls `try_send` for each session.

Also contains the extend-mode compositor path (section 7.2): per-virtual-fb write lock,
app Z-sort, Surface blit to virtual fb, then snapshot and try_send.

### 12.3 `encoder.rs` — Frame Encoding (~350 lines)

JPEG encoding (via `jpeg-encoder` crate, already a dependency from R18), PNG encoding
(via `png` crate), and raw pixel copy. The encoder runs on the `vdisp_worker` thread, not
the vsync thread. BGRA→JPEG and BGRA→RGB24→JPEG conversion paths.

### 12.4 `worker.rs` — vdisp_worker Thread (~350 lines)

One thread per active VirtualDisplay. Receives `VDisplayFrame` from `frame_rx`, calls
`encoder::encode_frame` to produce the final bytes, then calls `transport::send_frame`.
On `TrySendError` or `WouldBlock` from the transport, increments `frames_dropped` via an
`Arc<AtomicU64>` shared with the main `VirtualDisplay` struct.

**Counter sharing**: `VirtualDisplay::frames_dropped` is backed by an `Arc<AtomicU64>`.
The worker thread holds a clone of this `Arc` and uses `fetch_add(1, Ordering::Relaxed)`
to increment. The main engine reads it via `Arc::load(Ordering::Relaxed)`. This avoids
the cross-thread mutation hazard that was identified as a non-blocking issue in R18's
critique (N2 in 18-screen-capture-critique.md).

### 12.5 `transport.rs` — Transport Layer (~450 lines)

Manages the lifecycle of each transport:
- `listen`: create socket (Unix/TCP/vsock) and begin `accept()`
- `accept_connection`: complete handshake, return `TransportHandle`
- `send_frame`: non-blocking write of header + payload to socket
- `close`: shutdown and cleanup

The `TransportHandle` wraps the connected socket with `set_nonblocking(true)` enforced at
handshake completion. No blocking writes anywhere in the transport layer.

### 12.6 `extend.rs` — Extend and Remote-Only Mode (~400 lines)

Virtual framebuffer lifecycle: `alloc_virtual_fb(width, height)` → `Vec<u8>` zero-filled;
`resize_virtual_fb(fb, new_w, new_h)` (resizing is not supported in v1 but the function is
defined for v2). App assignment tracking: `assign_app_to_display(app_id, display_id)`,
`unassign_app_from_display(app_id)`, `app_display_target(app_id) → Option<VDisplayId>`.
The extend-mode compositor pass (section 7.2 steps A–H) is implemented here.

### 12.7 `wit_handlers.rs` — WIT Linker (~350 lines)

Conditional registration (section 8.2). Maps each WIT function to a host closure. Closures
extract `app_id` from `SupervisorCtx`, call `engine.check_permission`, then dispatch to
the appropriate subsystem function.

### 12.8 `protocol.rs` — VYOMA_VDISP: Parser (~300 lines)

```rust
pub fn parse_vdisp_line(line: &str) -> Option<VdispCommand>;
pub fn format_vdisp_response(resp: &VdispResponse) -> String;

pub enum VdispCommand {
    Create { mode: VDisplayMode, transport: VDisplayTransport, fps: u8,
             encoding: VDisplayEncoding, scale: f32 },
    Destroy { id: VDisplayId },
    SetMirrorSource { id: VDisplayId, app_id: u32 },
    PushFrame { id: VDisplayId, width: u32, height: u32, pixel_count: u32 },
    Connect { id: VDisplayId },
    Disconnect { id: VDisplayId },
    Info { id: VDisplayId },
    Resolution { id: VDisplayId },
    Dropped { id: VDisplayId },
}

pub enum VdispResponse {
    Created { id: VDisplayId },
    Destroyed { id: VDisplayId },
    Connected { id: VDisplayId },
    Disconnected { id: VDisplayId },
    Info { id: VDisplayId, mode: String, width: u32, height: u32, fps: u8,
           frames_sent: u64, frames_dropped: u64, active: bool },
    Screen { id: VDisplayId, width: u32, height: u32 },
    Dropped { id: VDisplayId, count: u64 },
    Error { reason: String },
}
```

---

## 13. New Manifest Fields

```toml
# vyoma.toml additions for R19

[capabilities]
virtual_display     = false   # default; set true to create virtual displays
virtual_display_any = false   # default; set true for mirror/cross-app access
```

Both fields default to `false`. The manifest validator recognizes these fields.

---

## 14. Dependency Additions (`supervisor/Cargo.toml`)

No new dependencies required. R19 reuses:
- `jpeg-encoder` — already added in R18 for screen capture
- `png` — already added in R18 for screen capture
- `std::net::TcpListener`, `std::os::unix::net::UnixListener` — stdlib
- `nix` crate (already present for vsync/DRM operations) — for `AF_VSOCK` socket creation

The `hmac` and `sha2` crates are added for the TCP auth token challenge-response:
```toml
hmac = "0.12"
sha2 = "0.10"
```

These are lightweight pure-Rust crates with no C dependencies.

---

## 15. Test Plan

### Unit Tests (`supervisor/tests/vdisp_tests.rs`)

- `test_parse_create_mirror_unix` — protocol parser decodes mirror + Unix socket create
- `test_parse_create_extend` — extend mode with width/height
- `test_parse_create_remote_only` — remote-only mode
- `test_parse_tcp_loopback` — TCP loopback transport spec
- `test_parse_tcp_auth` — TCP auth transport with hex token
- `test_parse_vsock` — vsock transport spec
- `test_create_permission_denied` — app without virtual_display capability
- `test_mirror_requires_consent` — mirror mode without consent returns error
- `test_max_displays_exceeded` — 3rd session rejected on desktop-full
- `test_wire_header_roundtrip` — encode header, parse back, verify fields
- `test_handshake_bytes` — handshake record is exactly 64 bytes
- `test_auth_token_challenge_response` — valid token accepted, wrong token rejected
- `test_tcp_nonloopback_without_token_rejected` — `create-display` returns error
- `test_frame_drop_on_full_queue` — compositor try_send with full queue increments counter
- `test_concurrent_readers` — two mirror sessions can snapshot fb simultaneously
- `test_extend_fb_alloc` — `alloc_virtual_fb(1920, 1080)` allocates correct byte count
- `test_app_assignment` — `assign_app_to_display` routes draw commands correctly

### Integration Tests

- Mirror mode end-to-end: WASM app creates mirror, test harness connects via Unix socket,
  reads 10 frames, verifies JPEG SOI marker in each
- Extend mode: app assigned to extended display renders text; test harness reads frame,
  verifies pixel content
- Auth token: test harness connects with correct token (success) and wrong token (rejected)
- Graceful teardown: virtual display destroyed while worker is writing; verify no panic

---

## 16. Open Questions (v1)

1. **Cursor in mirror stream**: The compositor cursor is a separate overlay. Mirror mode
   reads the composited fb after the chrome/cursor overlay pass. Does the cursor appear in
   the stream? Current answer: yes (it is baked into the fb after the full compositor pass).
   A `cursor: bool` parameter can hide it by reading the pre-cursor fb in a future version.

2. **Multi-consumer fan-out**: V1 restricts each virtual display to one consumer. When
   should fan-out (multiple consumers reading the same stream) be added? Current answer:
   v2, after the streaming path is validated. Fan-out would require a ring buffer or
   broadcast channel, which is a more complex architecture.

3. **Adaptive rate control**: V1 drops frames silently. Should the supervisor auto-reduce
   fps when `frames_dropped / frames_sent > threshold`? Current answer: deferred to v2.
   In v1, the app must poll `dropped-frames` and manually adjust.

4. **extend mode on server-headless**: Server-headless has no physical display. Extend mode
   adds a second virtual display. Is extend mode on server-headless meaningfully different
   from remote-only mode? Current answer: exclude extend mode from server-headless in v1
   (only remote-only); revisit if a use case emerges.

5. **Input injection**: Remote desktop requires the consumer to send mouse/keyboard events
   back to the supervisor. V1 does not support this (virtual display is output-only). Input
   injection requires a separate IPC channel and is deferred to R22 (input subsystem).
