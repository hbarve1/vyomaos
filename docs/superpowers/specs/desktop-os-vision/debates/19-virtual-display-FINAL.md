# FINAL Spec: Virtual Display & Screen Mirroring (Round 19)

**Subsystem**: Virtual Display & Screen Mirroring  
**macOS Analogue**: AirPlay display (mirror/extend to Apple TV/Mac), Sidecar (iPad as second display)  
**Depends on**: R11 (Surface buffers, vsync RwLock), R18 (capture pattern, link-time WIT gating)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

VyomaOS Virtual Display enables WASM apps and the compositor to stream display output
to a non-physical consumer — another process over a Unix socket, a remote host over TCP,
or a QEMU host over virtio-vsock. Three modes: mirror (duplicate the physical screen),
extend (independent second logical display), and remote-only (headless virtual screen
with no physical counterpart).

### Non-Goals (v1)

- WebRTC, RTSP, RTMP — no media-streaming protocols; raw framed byte streams only
- Audio/video mux — video only; audio integration deferred to R20
- Hardware-accelerated encode — software JPEG/MJPEG only; H264 optional compile flag
- Multi-subscriber per session — single consumer per virtual display in v1

---

## 2. Capability Model (link-time gating, same pattern as R18)

```toml
[capabilities]
virtual_display     = true   # can create/push to own virtual display
virtual_display_any = true   # can mirror full compositor output (requires consent)
```

`vyoma:virtual-display@1.0.0` WIT imports are conditionally added to each app's Linker
in `init_linker_for_app`. No manifest flag = no import = Wasmtime instantiation error.

---

## 3. Core Rust Types

```rust
// supervisor/src/vdisp/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VDisplayMode { Mirror, Extend, RemoteOnly }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VDisplayTransport { UnixSocket, Tcp, Vsock }

#[derive(Debug, Clone, Copy)]
pub enum VDisplayFormat { JpegQuality(u8), Png, RawBgra32 }

pub struct VirtualDisplay {
    pub display_id: u32,
    pub owner_app_id: u32,
    pub mode: VDisplayMode,
    pub width: u32,
    pub height: u32,
    pub fps: u8,
    pub format: VDisplayFormat,
    pub transport: VDisplayTransport,
    pub active: bool,
    pub assigned_app_ids: Vec<u32>,         // extend mode only
    pub frames_sent: Arc<AtomicU64>,
    pub frames_dropped: Arc<AtomicU64>,     // Arc shared with vdisp_worker thread
    pub frame_tx: SyncSender<VDisplayFrame>,
    pub worker_thread: Option<JoinHandle<()>>,
}

pub struct VDisplayFrame {
    pub display_id: u32,
    pub data: Vec<u8>,          // encoded bytes (JPEG/PNG/raw)
    pub width: u32,
    pub height: u32,
    pub timestamp_us: u64,
}

pub struct VDisplayPermission {
    pub app_id: u32,
    pub virtual_display_own: bool,
    pub virtual_display_any: bool,
    pub consent_granted: bool,
    pub consent_suspended: bool,   // true after revocation; cleared only on reboot
}

pub struct VirtualDisplayEngine {
    pub next_display_id: u32,
    pub displays: HashMap<u32, VirtualDisplay>,
    pub permissions: HashMap<u32, VDisplayPermission>,
}
```

---

## 4. Transport Layer (B1 fix — atomic frame write)

### 4.1 Frame Wire Format

```
[8 bytes magic: 0x56 0x44 0x49 0x53 0x50 0x31 0x00 0x00]
[4 bytes: display_id (LE u32)]
[4 bytes: width (LE u32)]
[4 bytes: height (LE u32)]
[4 bytes: payload_length (LE u32)]
[4 bytes: timestamp_us_hi (LE u32)]
[4 bytes: timestamp_us_lo (LE u32)]
[payload_length bytes: encoded frame]
```

Total header: 32 bytes. Payload: encoded frame data (JPEG/PNG/raw).

### 4.2 Non-Blocking Atomic Write (B1 fix)

The original `write_all` + `WouldBlock` pattern is broken: `write_all` loops over
`write(2)`, and a partial write (first chunk succeeds, second chunk returns `WouldBlock`)
leaves a corrupted partial frame in the kernel socket buffer. The consumer's frame parser
desyncs and cannot recover.

**Fix**: Pre-check available send buffer space before writing. If insufficient, drop the
frame without writing any bytes — preserving socket byte-stream integrity:

```rust
// supervisor/src/vdisp/transport.rs

fn try_send_frame(stream: &UnixStream, frame: &VDisplayFrame) -> SendResult {
    let header = build_frame_header(frame);
    let total = header.len() + frame.data.len();

    // Query kernel send buffer available space
    let sndbuf = get_sndbuf_available(stream.as_raw_fd());

    if sndbuf < total {
        // Not enough buffer — drop frame atomically (zero bytes written)
        return SendResult::Dropped;
    }

    // Buffer has capacity — do vectored write (header + payload in one syscall)
    let iov = [IoSlice::new(&header), IoSlice::new(&frame.data)];
    match stream.write_vectored(&iov) {
        Ok(n) if n == total => SendResult::Sent,
        Ok(_) => {
            // Partial vectored write is possible only if socket went non-blocking
            // mid-write. Close the connection — consumer receives EOF.
            SendResult::ConnectionError("partial write — closing".to_string())
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock => SendResult::Dropped,
        Err(e) => SendResult::ConnectionError(e.to_string()),
    }
}

fn get_sndbuf_available(fd: RawFd) -> usize {
    // Linux: ioctl(SIOCOUTQ) returns bytes already in send buffer.
    // Available = SO_SNDBUF - SIOCOUTQ.
    let sndbuf = get_sockopt_sndbuf(fd);
    let queued = ioctl_siocoutq(fd);
    sndbuf.saturating_sub(queued)
}
```

**Why vectored write**: `write_vectored` with two `IoSlice` values is issued as a single
`writev(2)` syscall, which is atomic at the kernel level for a socket that has sufficient
buffer. If `writev` returns `n < total`, the connection is corrupted and must be closed.
Pre-checking buffer space makes this case extremely rare (only if kernel buffer shrinks
between check and write, which requires a competing sender — not possible for a single
thread).

**SendResult**:
- `Sent` — frame delivered
- `Dropped` — buffer full, frame skipped (increments `frames_dropped`)
- `ConnectionError(msg)` — socket closed; consumer disconnects

On `ConnectionError`, the vdisp_worker calls `VirtualDisplay::disconnect()`, which stops
the worker and leaves the `VirtualDisplay` in an idle state ready for a new connection.

### 4.3 Transport Types

**Unix socket** (local IPC): default. Path: `/tmp/vdisp-<display_id>.sock`. No auth
required — access controlled by Unix socket file permissions (owner-only, mode 0600).

**TCP** (network): binds to specified `host:port`. Auth handshake required (see section 9).

**virtio-vsock**: requires kernel `CONFIG_VSOCKETS` + `CONFIG_VHOST_VSOCK`. Gated behind
Cargo feature `vsock`. At startup, supervisor checks for `/dev/vsock` availability; if
absent and vsock transport is requested, returns `Err("vsock not available on this kernel")`.

---

## 5. WIT Interface `vyoma:virtual-display@1.0.0`

```wit
package vyoma:virtual-display@1.0.0;

variant vdisplay-mode { mirror, extend, remote-only }
variant vdisplay-transport { unix-socket, tcp(tuple<string, u16>), vsock(u32) }
variant vdisplay-format { jpeg(u8), png, raw-bgra32 }

record display-info {
    display-id: u32,
    width: u32,
    height: u32,
    fps: u8,
    frames-sent: u64,
    frames-dropped: u64,
    active: bool,
}

interface virtual-display {
    create-display: func(
        mode: vdisplay-mode,
        width: u32,
        height: u32,
        fps: u8,
        format: vdisplay-format,
        transport: vdisplay-transport,
    ) -> result<u32, string>;

    destroy-display: func(display-id: u32) -> result<_, string>;

    set-mirror-source: func(display-id: u32, source: u32) -> result<_, string>;

    push-frame: func(display-id: u32, surface-handle: u32) -> result<_, string>;

    display-info: func(display-id: u32) -> result<display-info, string>;

    /// Returns the display_id this app is currently assigned to, and its dimensions.
    /// Returns Err if app is not assigned to any virtual display.
    current-display: func() -> result<tuple<u32, u32, u32>, string>;

    dropped-frames: func(display-id: u32) -> result<u64, string>;

    h264-available: func() -> bool;
}

world virtual-display-world {
    import virtual-display;
}
```

**`push-frame` takes a surface handle** (R11 Surface ID), not raw pixels — eliminates the
250MB/s pixel copy from WASM linear memory. The supervisor reads from the app's Surface
buffer using the same `vsync_lock.read()` pattern. (N1 fix)

---

## 6. Extend Mode Coordinate Space (B2 fix)

### 6.1 App Assignment Lifecycle

When an app is assigned to an extended virtual display:
1. Supervisor sends `VYOMA_VDISP_SCREEN:<display_id>,<width>,<height>` to app stdin
2. This is sent at **every assignment event**: initial assignment, reassignment to different
   display, and reassignment back to physical (where width/height are the physical fb dims)

### 6.2 `VYOMA_VDISP_SCREEN` Semantics

```
VYOMA_VDISP_SCREEN:<display_id>,<width>,<height>
```

- `display_id = 0` means physical display
- `display_id > 0` means a virtual display

Sent whenever the app's display assignment changes. App MUST re-read its canvas dimensions
on each receipt. Apps that do not read this notification continue rendering at previous
dimensions — this is accepted behavior (no forced rerender from supervisor side).

### 6.3 `current-display` WIT Function

Apps can poll their current display assignment at any time:

```rust
// Returns (display_id, width, height)
// display_id 0 = physical display
fn current_display(ctx: &mut SupervisorCtx) -> Result<(u32, u32, u32), String> {
    let state = ctx.engine.apps.get(&ctx.app_id).ok_or("app not found")?;
    Ok((state.current_display_id, state.current_display_width, state.current_display_height))
}
```

Stdout protocol equivalent: `VYOMA_VDISP:my_display` → `VYOMA_VDISP_SCREEN:<id>,<w>,<h>`

### 6.4 Rendering Before Assignment

Apps that have not received a `VYOMA_VDISP_SCREEN` notification (they started before any
virtual display was created) should fall back to physical display dimensions. The supervisor
initializes `AppState::current_display_id = 0` with physical fb width/height at launch.
The app can call `current-display` to get this initial state without waiting for a push
notification.

### 6.5 Multiple Extended Displays

On desktop-full with 2 extended displays, each has a distinct `display_id`. The
`VYOMA_VDISP_SCREEN` notification always includes the `display_id`, so the app knows
which screen it is on. Apps may only be assigned to one display at a time.

---

## 7. Mirror Mode Compositor Integration (B3 fix)

### 7.1 Single Combined Read-Lock for All Mirror Sessions

The original per-session read-lock pattern (each mirror session independently acquires
`vsync_lock.read()`, holds it for ~2ms, then releases) causes sequential stalling on the
next vsync write-lock. With 4 sessions on server-headless, total stall = ~8ms on a 33ms
vsync budget at 30fps physical display — acceptable. But at 60fps (16ms budget), stall =
8ms (50% of budget) — unacceptable.

**Fix**: A single read-lock acquisition copies the framebuffer **once** into a shared
`Arc<Vec<u8>>` snapshot. All active mirror sessions receive the same snapshot. Encoding
runs per-session on `vdisp_worker` threads without holding `vsync_lock`:

```rust
// supervisor/src/vdisp/compositor_hook.rs

pub fn capture_mirror_snapshot(
    fb: &FrameBuffer,
    vsync_lock: &Arc<RwLock<()>>,
    active_mirror_displays: &[u32],
) -> Option<Arc<Vec<u8>>> {
    if active_mirror_displays.is_empty() {
        return None;
    }
    // One read-lock, one clone, ~2ms total regardless of session count
    let snapshot: Vec<u8> = {
        let _read_guard = vsync_lock.read().unwrap();
        fb.pixels.clone()
    };
    Some(Arc::new(snapshot))
}
```

After the vsync flush pass completes, `capture_mirror_snapshot` runs once. The resulting
`Arc<Vec<u8>>` is distributed to all active mirror sessions' frame queues. Each
`vdisp_worker` thread encodes from the shared snapshot independently — no lock held.

**Total vsync stall for mirror sessions**: ~2ms (one combined read-lock), regardless of
session count. Flush completes at T+X ms; snapshot taken at T+X+2ms; write-lock available
for next frame at T+X+2ms.

### 7.2 Extend Mode Compositor Pass

For each extended virtual display with `assigned_app_ids.len() > 0`:
1. Supervisor runs a mini-compositor pass over the assigned apps' Surfaces (same Z-sort
   logic as physical display but rendering into a `VDisplayFrame` buffer)
2. The mini-compositor does NOT use `vsync_lock` (it reads from per-app Surfaces, not `fb`)
3. The rendered frame is sent to the session's bounded frame queue

For remote-only mode with zero assigned apps: skip the compositor pass entirely (no
try_send). No blank frames are generated for unoccupied remote-only displays.

---

## 8. VYOMA_VDISP: Stdout Protocol

```
VYOMA_VDISP:create:<mode>,<width>,<height>,<fps>,<format>,<transport>
VYOMA_VDISP:destroy:<display_id>
VYOMA_VDISP:mirror:<display_id>,<source_app_id>
VYOMA_VDISP:push_frame:<display_id>,<surface_handle>
VYOMA_VDISP:info:<display_id>
VYOMA_VDISP:my_display
VYOMA_VDISP:dropped:<display_id>
```

Responses:
```
VYOMA_VDISP_CREATED:<display_id>
VYOMA_VDISP_DESTROYED:<display_id>
VYOMA_VDISP_INFO:<display_id>,<w>,<h>,<fps>,<sent>,<dropped>,<active>
VYOMA_VDISP_SCREEN:<display_id>,<width>,<height>    ← push notification on assignment
VYOMA_VDISP_DROPPED:<display_id>,<count>
VYOMA_VDISP_DROPPING:<display_id>                  ← push on frame drop (1/s rate-limit)
VYOMA_VDISP_ERROR:<reason>
```

Frame push: app writes `VYOMA_VDISP:push_frame:<display_id>,<surface_handle>\n`
where `surface_handle` references an existing R11 Surface (set via `VYOMA_DRAW:set_surface`).
No raw binary bytes in the line-oriented protocol — all data flows through Surface handles.
(N6 fix)

---

## 9. TCP Authentication (B4 fix — handshake timeout)

### 9.1 Token Configuration

Virtual display TCP auth uses a pre-shared token specified at `create-display` time:

```wit
vdisplay-transport::tcp-auth(tuple<string, u16, string>)  // host, port, token
```

Token is a 32-character hex string. On the loopback interface (`127.0.0.1`), auth is
optional (no-token creates an unauthenticated loopback-only virtual display).

### 9.2 Handshake with Timeout (B4 fix)

```rust
// supervisor/src/vdisp/transport.rs — accept_connection

fn accept_connection(listener: &TcpListener, auth_token: Option<&str>) -> Result<TcpStream, String> {
    let (mut stream, _addr) = listener.accept().map_err(|e| e.to_string())?;

    // Mandatory 5-second timeout on auth handshake read AND write
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    if let Some(token) = auth_token {
        // Send 16-byte nonce
        let nonce: [u8; 16] = rand::random();
        stream.write_all(&nonce)?;  // timeout applies; returns Err if consumer slow

        // Read 32-byte HMAC-SHA256 response
        let mut response = [0u8; 32];
        stream.read_exact(&mut response)?;  // returns TimedOut after 5s if no response

        let expected = hmac_sha256(token.as_bytes(), &nonce);
        if response != expected {
            return Err("auth failed: incorrect token".to_string());
        }
    }

    // Remove timeouts after successful auth — data phase uses non-blocking mode
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    stream.set_nonblocking(true)?;
    Ok(stream)
}
```

**Timeout applies to all transports**: Unix socket and vsock accept the same 5-second
deadline for the initial handshake record exchange. On timeout: close connection, log
`WARN [vdisp] auth handshake timeout from <addr>`, return to listen loop.

**Unix socket auth**: No token required — file permissions (mode 0600, owner only) provide
access control. Handshake record is a 32-byte magic + display_id (no HMAC).

---

## 10. Consent Revocation (B5 fix)

### 10.1 Revocation IPC Message

The chrome app (R23) can revoke consent mid-session by sending:
```
@supervisor: vdisp_consent_revoke:<app_id>
```

### 10.2 Supervisor Response

```rust
// supervisor/src/ipc_handlers.rs

fn handle_vdisp_consent_revoke(engine: &mut VirtualDisplayEngine, app_id: u32) {
    // Suspend consent — cleared only on reboot
    if let Some(perm) = engine.permissions.get_mut(&app_id) {
        perm.consent_granted = false;
        perm.consent_suspended = true;
    }

    // Terminate all active mirror/virtual_display_any sessions for this app
    let to_destroy: Vec<u32> = engine.displays.values()
        .filter(|d| d.owner_app_id == app_id && d.mode == VDisplayMode::Mirror)
        .map(|d| d.display_id)
        .collect();

    for display_id in to_destroy {
        destroy_display(engine, display_id);
        // Notify the app
        send_stdin_to_app(app_id, format!("VYOMA_VDISP_ERROR:consent_revoked:{display_id}\n"));
    }
}
```

### 10.3 `consent_suspended` Semantics

- `consent_suspended = true`: any future `create-display(mode: Mirror)` call returns
  `Err("mirror consent suspended until reboot")` without showing a dialog
- Cleared only on supervisor restart (next boot)
- The `virtual_display_any` manifest capability remains declared — the app's binary is
  not affected; only the runtime permission state is suspended

This mirrors R18's approach: consent is session-scoped plus boot-scoped after revocation.
Persistent consent storage across reboots is deferred to R61 (Permissions & Privacy).

---

## 11. Platform Matrix

| Feature | desktop-full | mobile | server-headless | robotics-rt | iot-edge | mcu-minimal |
|---------|-------------|--------|----------------|-------------|----------|-------------|
| Mirror mode | yes | no | no | no | no | no |
| Extend mode | yes | yes | yes | no | no | no |
| Remote-only | yes | no | yes | no | no | no |
| Unix socket transport | yes | yes | yes | no | no | no |
| TCP transport | yes | no | yes | no | no | no |
| vsock transport | opt | no | opt | no | no | no |
| Max concurrent sessions | 2 | 1 | 4 | — | — | — |
| Max fps | 60 | 30 | 30 | — | — | — |
| `virtual_display_any` | yes | no | no | no | no | no |

Profiles that don't support virtual display get stub WIT implementations returning
`Err("virtual display not available on this profile")`. Capability mismatch
(`virtual_display_any = true` on mobile) is a load-time error, not a warning.

**vsock**: requires kernel `CONFIG_VSOCKETS` + `CONFIG_VHOST_VSOCK`. Supervisor checks
`/dev/vsock` at startup; if absent, vsock transport requests return
`Err("vsock not available — rebuild kernel with CONFIG_VSOCKETS")`.

---

## 12. File Layout

```
supervisor/src/vdisp/
├── mod.rs           (VirtualDisplayEngine, VirtualDisplay types, FINALS consts)
├── compositor_hook.rs (capture_mirror_snapshot, extend_compositor_pass)
├── worker.rs        (vdisp_worker thread, WorkerTask enum)
├── transport.rs     (try_send_frame, get_sndbuf_available, accept_connection)
├── permissions.rs   (VDisplayPermission, consent grant/revoke)
└── paths.rs         (socket path construction for Unix transport)

supervisor/src/wit_handlers.rs   (virtual-display WIT closures)
supervisor/src/ipc_handlers.rs   (vdisp_consent_revoke handler)
```

---

## 13. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Partial write corrupts stream | Pre-check `SIOCOUTQ` vs `SO_SNDBUF`; drop atomically if insufficient buffer; use `write_vectored` for header+payload in single syscall |
| B2: Extend mode coordinate space | `current-display` WIT function + `VYOMA_VDISP:my_display` poll; `VYOMA_VDISP_SCREEN` sent on every assignment change; fallback to physical dims before first assignment |
| B3: Mirror mode read-lock stall | Single combined read-lock captures one `Arc<Vec<u8>>` snapshot shared by all sessions; total vsync stall = ~2ms regardless of session count |
| B4: TCP auth handshake no timeout | `set_read_timeout(5s)` + `set_write_timeout(5s)` before auth read; applies to all transport types; on timeout: close + log + re-listen |
| B5: Consent revocation mid-session | `vdisp_consent_revoke:<app_id>` IPC defined; supervisor sets `consent_suspended = true`, destroys all active mirror sessions, notifies app via `VYOMA_VDISP_ERROR:consent_revoked:<id>` |

---

## 14. Open Questions (deferred to future rounds)

1. **HiDPI virtual displays**: Virtual displays may have different pixel density than the
   physical display. R20 (HiDPI) will define the backing scale API; virtual display should
   expose `scale_factor` in `display-info` once R20 is complete.
2. **Multi-monitor extend mode UX**: Which app is on which screen is currently supervisor-
   command driven. R21 (Window Manager) will define drag-to-move-window-between-displays.
3. **vsock kernel config**: The required kernel config changes for vsock transport need
   to be added to CLAUDE.md's kernel config section once vsock is enabled.
4. **Remote desktop protocol**: For serious remote access, a lossless mode (PNG) at lower
   fps plus input event forwarding (mouse/keyboard) is needed. Input forwarding is deferred
   to R32 (Mouse & Gestures) and R31 (Keyboard Input).
