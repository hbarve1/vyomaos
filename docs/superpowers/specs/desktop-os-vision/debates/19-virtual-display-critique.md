# Critique: Virtual Display & Screen Mirroring (Round 19)

## Verdict

The spec is the most complete virtual-display design in VyomaOS's history and resolves
several classes of bugs that were found in R18 (notably, the vsync lock type is now
correctly specified as `RwLock`, the link-time capability gate is explicitly provided,
and counter sharing across threads is handled with `Arc<AtomicU64>`). However, five
blocking issues remain before this spec is implementable: the transport backpressure
model has a kernel-buffer blind spot that defeats the non-blocking write strategy; extend
mode presents a coordinate-space gap that leaves WASM apps unable to render correctly; the
vsync read-lock tearing edge case is resolved for encoding but not for the less-obvious
case where a new vsync write-lock tries to pre-empt ongoing mirror snapshots; the TCP auth
model is acceptable for threat acceptance but is missing a handshake timeout that enables
a trivial denial-of-service; and the `virtual_display_any` runtime consent path omits
mid-session revocation semantics that could leave a mirror stream open after the user
denies it.

---

## Blocking Issues (must fix before implementation)

### B1: Transport Backpressure Blind Spot — Kernel Socket Buffer Fills Without Triggering WouldBlock on Partial Write

Section 4.3 defines the backpressure strategy: the `vdisp_worker` calls
`write_all(header + payload)` on a non-blocking socket; on `WouldBlock`, the frame is
dropped. This strategy is described as preventing "a slow consumer from filling kernel
socket buffers indefinitely." The description is not wrong, but it is incomplete in a way
that causes the stated invariant to fail under the most common slow-consumer scenario.

`write_all` is not a single syscall. It is a loop over `write` until all bytes are sent.
For a 250KB JPEG frame plus a 24-byte header, `write_all` may call `write` multiple times.
The first call succeeds (the kernel buffer has space), partially filling the send buffer.
The second call returns `WouldBlock` (buffer now full). But at this point `write_all` has
already written a partial frame header+payload into the kernel buffer. The socket is now
in a corrupted state from the consumer's perspective: it received a partial frame that
does not match the 24-byte header's `payload_length` field.

The spec's `write_all` + `WouldBlock` pattern is broken for frames larger than the kernel
socket send buffer. The default Linux send buffer is 212992 bytes (208KB), and a
1920×1080 JPEG at quality 70 averages 200–300KB. Frames right at the buffer boundary will
produce this partial-write condition on a significant fraction of sends.

The spec must address this with one of two approaches:

**Option A — Framed write with explicit length prefix and partial-write detection**:
Replace `write_all` with a two-step attempt: first try to write the header (24 bytes,
always fits); if successful, try to write the payload. If the payload write partially
succeeds, the transport must either buffer the remainder and complete it on the next
send opportunity, or close the connection (flagging it as corrupted) and drop the frame
with a `frames_dropped` increment. The spec must choose one of these and define the
state machine explicitly.

**Option B — Sendfile/vectored non-blocking write with pre-check**:
Before calling write, check `available_send_buffer_space` via `getsockopt(SO_SNDBUF)` and
compare against the full frame size. If insufficient space, drop the frame immediately
without attempting any write (preserving the socket's byte-stream integrity for the
consumer). This avoids partial writes entirely.

Without fixing B1, a slow consumer will receive a corrupted byte stream within the first
few seconds of connection, causing the consumer's frame parser to desync and fail to
recover. The non-blocking write model in section 4.3 appears correct but has a subtle
correctness gap at the system call boundary.

---

### B2: Extend Mode Coordinate Space — `VYOMA_VDISP_SCREEN` Sent Once, But App Assignment Is Dynamic and Can Change

Section 5.4 defines the provisional coordinate-space API for extend mode: when an app is
assigned to an extended display, the supervisor sends a single stdin line
`VYOMA_VDISP_SCREEN:<display_id>,<width>,<height>`. The app reads this line once at
initialization and adjusts its rendering dimensions accordingly.

This is correct for the initial assignment. The spec breaks down in three scenarios that
are not addressed:

**Scenario 1 — App is reassigned**: The supervisor command
`vdisp assign <display_id> <app_name>` can move an app between displays. If an app starts
on the physical display, gets assigned to the extended display, then gets reassigned back
to the physical display, the supervisor must send another `VYOMA_VDISP_SCREEN` update.
The spec says nothing about reassignment events. An app that does not receive the
reassignment notification will continue rendering at the extended display's resolution
on the physical display, producing distorted or wrong-sized output.

**Scenario 2 — App starts before virtual display exists**: If an app starts before any
virtual display of the `extend` type has been created, there is no `display_id` to
reference in the `VYOMA_VDISP_SCREEN` message. The app cannot know its canvas size
until the virtual display is created and the assignment is made. The spec does not define
what the app should render before receiving the screen dimensions notification, nor does
it define a WIT polling function that lets the app query its current display assignment.

**Scenario 3 — Multiple extended displays**: The platform matrix allows 2 simultaneous
virtual displays on desktop-full. Both could be in extend mode. An app that receives
`VYOMA_VDISP_SCREEN:0,1920,1080` does not know whether this is the primary or secondary
extended display. The spec provides no mechanism for the app to distinguish or to request
specific display placement.

The fundamental gap is that section 5.4 defines only a push-at-assignment notification,
not a pull API or an event subscription model. The WIT function `display-resolution(id)`
exists but requires the app to already know its `display_id` — which it only learns from
the `VYOMA_VDISP_SCREEN` push, creating a circular dependency.

The spec must define:
1. A `current-display: func() -> result<u32, string>` WIT function (or stdout equivalent
   `VYOMA_VDISP:my_display`) that an app can call at any time to learn which display it
   is currently assigned to, and its dimensions.
2. Reassignment notification: when an app is moved between displays, the supervisor sends
   a new `VYOMA_VDISP_SCREEN` (or a WIT callback, if async callbacks exist in v1).
3. A well-defined rendering behavior before assignment: apps should fall back to the
   physical display's dimensions (query via the existing framebuffer state) until a
   `VYOMA_VDISP_SCREEN` notification arrives.

---

### B3: Mirror Mode Tearing — vsync Read-Lock Window Does Not Prevent New Write-Lock Pre-emption During Multi-Session Snapshot

Section 7.1 states that after the physical display flush (steps 1–6), the virtual display
snapshot step acquires `vsync_lock.read()` and performs a memcpy. Section 7.5 correctly
notes that two simultaneous read-locks do not contend with each other and analyzes the
worst-case 4ms read-lock hold for 2 mirror sessions.

The analysis in 7.5 is correct but stops one step too early. The issue is not two readers
contending with each other — it is the next vsync tick's write-lock request arriving while
both mirror sessions' read-locks are still held.

The vsync timer fires at regular intervals (e.g., 16ms at 60fps or 33ms at 30fps). The
sequence of events:

```
T=0ms:   vsync write-lock acquired for flush
T=15ms:  flush completes; write-lock released
T=15ms:  mirror session 1 acquires read-lock; starts 2ms memcpy
T=15ms:  mirror session 2 acquires read-lock; starts 2ms memcpy (concurrent)
T=16ms:  vsync timer fires for next frame
T=16ms:  compositor tries to acquire vsync write-lock
          -- BLOCKED: read-locks from sessions 1 and 2 are still held
T=17ms:  sessions 1 and 2 finish memcpy; read-locks released
T=17ms:  compositor acquires write-lock; begins next flush
          -- 1ms late (frame is 1ms delayed)
```

At 60fps this is a 1ms stall on a 16ms budget — acceptable. But the spec allows 4
simultaneous sessions on server-headless. With 4 sessions doing sequential 2ms memcpy
each (if `std::sync::RwLock` is `std::sync::RwLock` from the standard library, which
does not guarantee fairness between concurrent readers — they may not all start at the
same moment):

```
T=15ms:  flush completes; write-lock released
T=15ms:  session 1 acquires read-lock
T=16ms:  vsync timer fires; compositor blocks on write-lock (readers hold it)
T=15-17ms: sessions 1-4 serially acquire read-lock, each holds 2ms
T=15+2+2+2+2 = T=23ms: all read-locks released
T=23ms:  compositor acquires write-lock; 7ms late on a 16ms budget
```

At 60fps physical display with 4 mirror sessions, the compositor is 7ms late — a
significant jitter that will produce visible stuttering on the physical display.

The spec must resolve this by either:
1. Serializing all mirror memcpy operations under a single read-lock acquisition
   (one combined memcpy step for all sessions, not N sequential acquisitions), reducing
   total read-lock hold to ~2ms regardless of session count.
2. Capping the post-flush read-lock hold to a deadline: if the total read-lock time
   would exceed a threshold (e.g., 2ms), pending sessions beyond the cap are skipped for
   this tick (drop policy).
3. Using a separate double-buffer for mirror mode: the compositor writes to a "capture
   buffer" alongside the physical fb (holding the write-lock for both writes, adding ~2ms
   to the flush pass itself), and mirror sessions read from the capture buffer without
   holding `vsync_lock` at all.

The spec's current analysis in section 7.5 concludes that "4ms of read-lock hold is
acceptable" but does not account for serial acquisition vs. concurrent acquisition, and
does not address the 60fps physical display case on server-headless. This is a blocking
issue because the implementation team will need to choose between the three options above,
and the wrong choice produces either visible display stutter or incorrect frame snapshots.

---

### B4: TCP Auth Handshake Has No Timeout — Enables Connection-Exhaustion DoS

Section 10.3 defines the TCP auth token exchange: the supervisor sends a 16-byte nonce
and waits for the consumer to send back `HMAC-SHA256(token, nonce)` (32 bytes). The
supervisor verifies and either continues with the handshake or closes the connection.

The spec does not define a timeout on this read. The `transport::accept_connection`
function (section 12.5) calls `read_exact(32 bytes)` waiting for the consumer's HMAC
response. If the consumer connects but never sends the 32 bytes (or sends them slowly),
the supervisor's transport worker thread blocks indefinitely on `read_exact`.

The consequence: since v1 is single-consumer per virtual display, the `accept()` loop
cannot proceed to the next consumer while the current connection is in the auth handshake
phase. A malicious or buggy consumer that connects and never sends the HMAC response
permanently occupies the virtual display's connection slot. Combined with the fact that
the supervisor has no way to distinguish a legitimately slow consumer from a DoS attempt
at the `accept_connection` level, this is a blocking resource exhaustion vulnerability.

Even on a loopback interface (where the threat model is "trusted local processes"), a
crashed consumer that established a TCP connection before crashing could leave the socket
in a half-open state indefinitely.

The fix is straightforward: `set_read_timeout(Some(Duration::from_secs(5)))` on the
accepted socket before attempting the auth read. If the consumer does not send the HMAC
within 5 seconds, close the socket and loop back to `accept()`. The spec must:

1. State the auth handshake timeout value (5 seconds is reasonable).
2. Specify the behavior on timeout: close the connection, log a WARNING, and return to
   the listening state.
3. Apply the same timeout to the initial handshake record send on **all** transport types
   (Unix socket and vsock included), not just TCP. A consumer that connects and then stops
   reading (network partition, crash) should not hold the session slot indefinitely.

This issue also applies to the non-auth TCP path (loopback-only): a consumer that
connects and never reads the handshake record will cause the worker thread's `write_all`
to block (or `WouldBlock` repeatedly) forever, depending on how the handshake is
implemented. The spec does not show whether the handshake write is also non-blocking.

---

### B5: Mid-Session Consent Revocation — Mirror Stream Remains Open After User Denies

Section 8.3 defines the runtime consent model for mirror mode and `virtual_display_any`:
the supervisor sends a consent request to the chrome app when an app first attempts mirror
mode, stores `VDisplayPermission::consent_granted = true` on acceptance, and subsequent
calls proceed without re-prompting. Denial sets `consent_granted = false`.

The spec addresses consent at session creation time but does not define what happens when
the user revokes consent mid-session:

**Scenario**: An app with `virtual_display_any = true` requests mirror mode. The user
grants consent. A `VirtualDisplay` session is created with `mode: Mirror` and
`active: true`. The `vdisp_worker` thread begins streaming frames. Five minutes later,
the user opens a Settings panel and revokes mirror consent (a standard feature in any
privacy-conscious OS — macOS lets you revoke screen recording permission from System
Settings while an app is actively recording).

When revocation occurs:
1. The chrome app sends `@supervisor: vdisp_consent_revoke:<app_id>`.
2. The supervisor sets `VDisplayPermission::consent_granted = false`.
3. The active `VirtualDisplay` session continues running — the `vdisp_worker` thread is
   not notified, and `active` remains `true`.
4. The mirror stream continues delivering frames to the consumer.

The spec provides no mechanism for the supervisor to stop an active session when consent
is revoked. Section 8.3 only describes the consent grant/deny flow for session creation.
The revocation IPC message `vdisp_consent_revoke` is not even defined in the spec — only
`vdisp_consent_grant` and `vdisp_consent_deny` are mentioned (section 8.3, by analogy
to R18 section 7.3 which has the same gap).

The spec must define:
1. `vdisp_consent_revoke:<app_id>` as an IPC message the chrome app can send.
2. The supervisor's response to receiving it: call `destroy-display` for all active mirror
   sessions and `set-mirror-source`-enabled sessions belonging to `app_id`.
3. The app is notified via `VYOMA_VDISP_ERROR:consent_revoked` on its stdin for each
   destroyed session.
4. Whether the app's `virtual_display_any` capability is temporarily suspended for the
   session (until the next boot), or permanently revoked until manually re-granted.

Without this, the consent model is a one-time gate, not a live privacy control. An app
that starts a mirror session with user consent then runs indefinitely — even if the user
later decides they do not want the screen mirrored — has no recourse except killing the
app. This is a privacy correctness issue, not merely a quality-of-life issue.

---

## Non-Blocking Issues (should fix, won't block)

### N1: `push-frame` WIT Function Passes Pixels as `list<u8>` — Large Copy on Every Call

Section 5's `push-frame` passes `pixels: list<u8>`. For 1920×1080 BGRA32, that is
8,294,400 bytes copied from WASM linear memory into the host on every call. At 30fps
this is a 250MB/s copy. Consider a shared-memory or surface-handle approach (app writes
to a pre-allocated R11 Surface, supervisor reads from it) to eliminate this copy for the
push-frame hot path.

### N2: virtio-vsock Requires Kernel Modules That allnoconfig Likely Excludes

Section 4.5 requires `CONFIG_VSOCKETS` and `CONFIG_VHOST_VSOCK` in the kernel build.
Neither is mentioned in CLAUDE.md's kernel config. Without them, vsock transport fails
at runtime with `EAFNOSUPPORT`. The spec must either add these to the required kernel
config for desktop-full and server-headless profiles, or mark vsock as a conditional
transport gated on a supervisor Cargo feature that checks for vsock availability at
startup.

### N3: `frames_dropped` Is `Arc<AtomicU64>` in `worker.rs` But `u64` in `VirtualDisplay` Struct — Type Inconsistency

Section 12.4 (`worker.rs`) says the worker holds "a clone of this `Arc<AtomicU64>`"
for `frames_dropped`. Section 3's `VirtualDisplay` struct definition shows:
`pub frames_dropped: u64`. These are incompatible: `u64` cannot be shared across threads
without a `Mutex` or `AtomicU64`. The struct definition in section 3 must be updated to
`pub frames_dropped: Arc<AtomicU64>` (and similarly for `frames_sent` which has the same
cross-thread write issue from `worker.rs`). The section 3 type definitions are the
authoritative API contract and must match the implementation description in section 12.4.

### N4: Remote-Only Mode Has No Defined Compositor Pass Driver

Section 7.2 describes the extend-mode compositor pass but does not explicitly state that
remote-only mode uses the same path. If no apps are assigned to a remote-only display,
the compositor still runs the pass every vsync tick, producing blank frames at 30fps
until an app is assigned. The spec should: (a) explicitly state that remote-only uses the
same compositor pass as extend mode, and (b) specify that the pass is skipped (no
try_send) when zero apps are assigned to the display.

### N5: Memory Budget in Section 11.4 Conflates Raw and JPEG Frame Queue Sizes

Section 4.6 defines a queue of `VDisplayFrame` structs whose `data` field holds
**encoded** bytes (post-JPEG), not raw pixels. JPEG at quality 70 is ~250KB per frame,
not 8MB. The section 11.4 "Frame queue (2 raw frames) = ~16MB" figure applies only to
raw encoding; for JPEG encoding the queue holds ~0.5MB per session. The total ~34MB
per-session estimate is also inflated because the raw frame buffer (encoder input) and
JPEG buffer (encoder output) are not both fully resident simultaneously. The spec should
differentiate by encoding type.

### N6: `push_frame` Stdout Protocol Mixes Binary Pixel Data into the Line-Oriented Parser

Section 6.1 states that after writing the `push_frame` command line, the app writes
`pixel_count * 4` bytes of raw binary to stdout. The supervisor's main stdout parser is
line-oriented (`read_line` / BufReader). Injecting raw binary bytes (which may contain
`\n` bytes — BGRA pixel values can be 0x0A which is ASCII newline) into the line-oriented
stream will corrupt subsequent line parses. The R11/R17/R18 protocol never mixes binary
data with line-oriented text for this reason. Section 6.1 acknowledges this as the "same
pattern as a potential future binary extension of the VYOMA_DRAW protocol" — but that
future extension does not exist yet. The `push_frame` stdout command should either be
removed from v1 (apps must use the WIT `push-frame` function for raw pixel writes) or
the protocol parser must switch to a binary framing mode for the duration of the pixel
data read. This is a design inconsistency that will cause a parse failure for any pixel
value containing `\n`, `\r`, or `\0`.

### N7: Auth Token Type Mismatch Between Rust (`[u8; 32]`) and WIT (`list<u8>`)

Section 3 uses `Option<[u8; 32]>` (compile-time enforced length) for the auth token.
Section 5 WIT uses `list<u8>` with a prose comment "Must be exactly 32 bytes." WIT has
no fixed-length array type, so the enforcement is runtime-only. The spec should add an
explicit WIT-level comment that `create-display` returns `Err("auth_token must be exactly
32 bytes")` for wrong-length tokens, to avoid implementers treating any-length as valid.

---

## What the Spec Got Right

### Strength 1: Link-Time Capability Gating Is Correctly Specified (Fixes R18 B1)

Section 8.2 explicitly defines `add_to_linker(linker, manifest)` where the manifest
controls whether `vyoma:virtual-display@1.0.0` host functions are registered at all. The
spec shows that `set-mirror-source` is further gated on `virtual_display_any` even within
an otherwise-registered linker. This directly addresses the R18 B1 blocking issue (WASM
capability wire-up was unspecified). The implementation note — "if the WASM binary
attempts to import them anyway, Wasmtime returns `InstantiationError::Trap` at link time
(before `_start` runs)" — correctly distinguishes link-time from runtime enforcement and
explains the failure mode. This is the right security model.

### Strength 2: Transport Backpressure Is Addressed at Two Independent Levels

Section 4.3 defines non-blocking writes at the transport socket level (drop on
`WouldBlock`), and section 4.6 defines a bounded queue (capacity 2) between the
compositor and the worker. Applying backpressure at both the internal queue boundary and
the external socket boundary means a slow consumer triggers a graceful drop cascade rather
than an unbounded memory growth. The separation of concerns — compositor never touches the
socket, worker never touches the compositor state — is clean and matches the R18 pattern.
The choice of capacity 2 (vs. R18's 4) is well-motivated by the latency argument:
"a consumer always receives frames at most 2 vsync ticks old." This is the right
engineering judgment for a live streaming use case.

### Strength 3: `Arc<AtomicU64>` for Cross-Thread Counter Sharing (Fixes R18 N2)

Section 12.4 explicitly uses `Arc<AtomicU64>` with `fetch_add(1, Ordering::Relaxed)` for
the `frames_dropped` counter shared between the main `VirtualDisplay` struct and the
`vdisp_worker` thread. The R18 critique's non-blocking issue N2 identified that
`RecordingSession::frames_dropped` as a plain `u64` would not compile because of
cross-thread mutation. R19 resolves this proactively. The `Ordering::Relaxed` choice is
correct here (no synchronization requirement — only monotonic counting) and avoids
unnecessary memory barriers.

### Strength 4: Wire Protocol Frame Header Is Well-Specified

Section 4.1 defines a complete 24-byte frame header with magic bytes, display ID,
sequence number, payload length, encoding, and pixel format. The header is self-describing
enough that a consumer can frame messages purely from it without needing to parse JPEG/PNG
structure. The magic byte sequence (`0x56445350` = "VDSP") and the 64-byte handshake
record (`0x56445348` = "VDSH") provide defense-in-depth against stream desyncs. The
sequence number field enables consumers to detect dropped frames on their side
independently of the supervisor's `frames_dropped` counter. This is a production-quality
wire protocol definition, not a sketch.

### Strength 5: Security Analysis Is Honest About v1 Limitations

Section 10.3 explicitly accepts that TCP auth provides authentication but not encryption
("the pixel stream is plaintext on the network") and defines the intended threat model
("trusted LAN environments"). The supervisor's WARNING log line at non-loopback TCP
creation time is the right user-facing signal. The explicit list of what is and is not
covered — "provides authentication (only token-holders can connect) but not encryption" —
is more honest than many security sections in system specs that claim security without
specifying the threat model. V1 accepting this is defensible; the path to TLS in v2 is
clear.

### Strength 6: Extend Mode Provisional API Acknowledges Its Own Incompleteness

Section 5.4 is labeled "Provisional Coordinate-Space API for Extend Mode" and states
explicitly that it is "pending R20 (HiDPI) and R21 (Window Manager)." The provisional
`VYOMA_VDISP_SCREEN` notification and the `display-resolution` WIT function are
explicitly identified as stopgap mechanisms. This intellectual honesty — defining a
working interim API while acknowledging it will be superseded — is the right approach for
a multi-round spec process. It enables implementation to proceed without waiting for the
full coordinate-model design from R20/R21.

---

## Questions for the Architect

### Q1: How Does the Compositor Know Which Apps Are Assigned to Which Extended Display?

Section 12.6 (`extend.rs`) defines `assign_app_to_display(app_id, display_id)` and
`app_display_target(app_id)`. But the compositor pass (section 12.2, `compositor.rs`)
must iterate apps sorted by Z-order for the extended display, the same way it does for
the physical display. The physical display compositor reads from `AppState` (R11). Does
`extend.rs` maintain a separate per-virtual-display `Vec<u32>` of assigned app IDs?
How is Z-order for the extended display determined — is it the same Z-order field as the
physical display, or a separate `vdisp_z_order` field in `AppState`? The spec's
`AppState` integration for extended display rendering is not shown anywhere; section 12.6
mentions the assignment functions but not the data layout that `compositor.rs` queries
to build the per-extended-display blit list.

### Q2: What Happens to the virtual_fb When `destroy-display` Is Called While the Worker Is Mid-Encode?

Section 12.1 defines `VirtualDisplayEngine::remove(id)` which returns the
`VirtualDisplay`. The worker thread holds `frame_rx` (the receive end of the bounded
queue) and may be in the middle of encoding a frame when `remove` is called from the main
event loop. Unlike R18's `stop_session` (which had a 5-second thread join), section 5's
`destroy-display` WIT function says only "closes the transport connection and stops the
worker thread." Does `destroy-display` join the worker thread (potentially blocking for
up to one encode cycle ~35ms)? Or does it signal the worker to stop and return
immediately (leaving the worker to finish and self-terminate)? The spec must define the
teardown sequence and its blocking behavior explicitly.

### Q3: On Server-Headless, Is There a Physical Framebuffer at All?

The CLAUDE.md says server-headless is a platform profile. Mirror mode on server-headless
reads the "physical composited framebuffer." But on a headless server with no GPU or
display output, `/dev/fb0` may not exist. Section 7.1 describes mirror mode reading from
`FramebufferState`. If `FramebufferState` is tied to a `/dev/fb0` open file descriptor,
mirror mode on server-headless would fail at the point where the framebuffer is opened.
The spec states mirror mode is supported on server-headless (section 9 platform matrix)
but does not address how the compositor operates when there is no physical display. Is
there an in-memory-only `FramebufferState` for headless mode? Is mirror mode on
server-headless always equivalent to remote-only mode with a virtual fb? The spec must
define the framebuffer model for the server-headless platform.

### Q4: Can the Same App Have Two Virtual Displays — One Mirror and One Remote-Only?

Section 10.6 enforces `max_displays = 2` on desktop-full. Is this per-app or global?
If it is per-app, an app with `virtual_display_any = true` could create 2 mirror sessions
and a second app creates 2 more, yielding 4 total on a desktop with a 2-display limit.
If it is global, two apps each requesting one display fill the limit and a third app's
request is rejected. The spec uses `VirtualDisplayEngine::max_displays` as a single
field, suggesting it is the total global count, but the error message "max virtual displays
reached" in section 8.2 does not clarify per-app vs. global semantics. Define the scope.

### Q5: How Is the `VYOMA_VDISP_SCREEN` Notification Delivered for Apps That Are Already Running?

Section 5.4 states the supervisor sends `VYOMA_VDISP_SCREEN` when an app is assigned to
an extended display. The supervisor's IPC model delivers messages to apps via their stdin.
For an app that is already running and waiting for input (e.g., a GUI app blocking on
`stdin.read_line`), receiving a `VYOMA_VDISP_SCREEN` line mid-input is unexpected. The
app may be using stdin for VYOMA_CAPTURE responses, VYOMA_DRAW ack messages, or user
keyboard input (if it declared `stdio = true`). How does the app distinguish a
`VYOMA_VDISP_SCREEN` notification from other stdin traffic? The spec should define whether
`VYOMA_VDISP_SCREEN` is injected into the app's regular stdin stream (parsed by the app's
main loop alongside other messages) or delivered via a separate mechanism. The existing
`VYOMA_CAPTURE_DONE:`, `VYOMA_CAPTURE_SESSION:` responses in R18 set the precedent for
supervisor-to-app responses on stdin — `VYOMA_VDISP_SCREEN` should follow the same
convention but the spec should confirm this explicitly.

### Q6: What Is the vsync Tick Source for Extended Display Compositor Pass on Server-Headless?

Section 7.1 states that virtual display mirror mode hooks in "after the physical display
flush." The physical display flush is driven by a vsync timer (the DRM vblank interrupt or
a timer-based approximation in the virtio-gpu path). On server-headless with no physical
display, there is no vsync source. The extend-mode and remote-only compositor passes in
section 7.2 must be driven by something on server-headless. Does the supervisor use a
periodic timer at the configured fps (e.g., 30fps = 33ms timer)? Is the timer per-virtual-
display or global? The spec says virtual display on server-headless is supported (remote-
only, mirror) but does not define the clock source for the compositor pass in the absence
of a physical vsync interrupt.

---

## Closing Assessment

Round 19 is a substantial and well-structured spec that shows clear architectural growth
from the patterns established in R11, R17, and R18. The wire protocol is production-quality,
the link-time capability gating directly fixes the most critical class of bug from R18, and
the backpressure model is correctly layered at both the internal queue and the transport
boundary. The five blocking issues are real implementation gaps — not documentation polish —
that require design decisions the spec currently defers or omits: the partial-write
hazard in transport.rs will cause stream corruption within seconds under any non-trivial
load, and the mid-session consent revocation gap means the privacy model is incomplete.
The spec should be returned for a targeted revision of B1–B5 before implementation begins,
with particular attention to B1 (transport write atomicity) and B3 (multi-session
read-lock serialization strategy). Non-blocking issues N6 (`push_frame` binary injection
into line parser) and N3 (struct type inconsistency) should also be addressed in the same
pass as they affect compilation correctness.
