# Critique: Screen Capture & Recording (Round 18)

## Verdict

The spec is architecturally coherent at a surface level and correctly identifies the
relevant integration points (R11 Surface buffers, vsync mutex, R14 image table, R17 WIT).
However, it contains five blocking implementation gaps — two concurrency hazards, one
missing WIT capability wire-up, one unbounded resource risk, and one subtle path issue —
any of which would cause a silent bug or system hang in production. The spec is not
implementable as written without resolving those gaps first.

---

## Blocking Issues (must fix before implementation)

### B1: WASM Capability Wire-Up Is Never Specified

Section 7 describes the capability model in terms of `vyoma.toml` fields (`capture`,
`capture_any`) and the `CapturePermission` struct, but the spec never explains the
mechanism by which a WASM app actually gains access to the `vyoma:capture@1.0.0`
host functions at Wasmtime link time.

For comparison, R17's rendering API explicitly states that `wit_handlers.rs` adds host
functions to the `Linker` conditionally — only if the app manifest declares the relevant
capability. The conditional wiring is the enforcement point. Without it, an app that
does NOT declare `capture = true` in its vyoma.toml can still call `screenshot` if the
linker unconditionally registers the host function, because Wasmtime does not know about
vyoma.toml fields.

Section 6.6 shows `wit_handlers.rs` calling `engine.check_permission(app_id, &target)?`
at runtime inside each WIT closure. That is a runtime check, not a link-time gate. The
distinction matters: a link-time gate (omit the host import from the `Linker`) means an
app without `capture = true` receives an instantiation error and never runs; a runtime
check means the app runs, calls the function, and receives an error string. Runtime
checking is the weaker model and, more importantly, the spec never reconciles how the
runtime check works when `app_id` is extracted from `SupervisorCtx`.

The spec must answer:
1. Where is `SupervisorCtx.app_id` set, and by which component of the supervisor?
2. Is `vyoma:capture@1.0.0` conditionally added to the Linker (per-app) or globally
   registered once for all apps?
3. If globally registered, how does the runtime permission check prevent an app from
   bypassing it by issuing a raw WASM import call that aliases the host function?

Without this, the permission model is a paper tiger. A WASM app can declare no
capabilities and still receive a linked `capture` import if the linker is set up globally,
which is the pattern implied by the single `add_to_linker` signature in section 6.6.

### B2: vsync Mutex Type Mismatch — Read-Lock on a `Mutex<()>` Does Not Exist

Section 6.3 states: "capture_screen and capture_region acquire vsync_lock for the
minimum time needed to memcpy pixels into a local Vec<u8>." The pseudocode in that
section calls `vsync_lock.lock()`, which is correct for `std::sync::Mutex<()>`.

But section 8.1 contradicts this with: "screenshot.rs acquires the vsync mutex
read-lock for the pixel copy only." A `Mutex<()>` has no read-lock; that is
`RwLock<()>`. The spec uses both terms interchangeably across sections 6.3 and 8.1,
and the distinction is not cosmetic.

If `vsync_lock` is a `Mutex<()>`, then:
- Only one thread can hold it at a time.
- `capture_screen` holding the lock for a 2ms memcpy would prevent the vsync compositor
  from flushing for that entire duration, stalling frame delivery to the user.
- The spec's own performance table (section 9.1) says "Wait for vsync_lock: 0–16ms",
  which implies the lock can be held by the vsync thread for up to 16ms. If capture
  tries to acquire during vsync flush, it waits up to 16ms just for the lock. This is
  a one-way hazard: vsync blocks capture. But if capture holds the lock during the
  2ms memcpy, vsync also blocks on capture. The spec claims this is safe because
  "encoding runs after releasing the lock" but says nothing about the reverse contention.

If `vsync_lock` is an `RwLock<()>`:
- The compositor must use `write()` during flush (exclusive) and capture must use
  `read()` (shared). But the spec never defines which lock type the compositor holds.
  R11 (section 8.1) describes the vsync mutex as a `Mutex<()>` — a plain mutex —
  not an RwLock.

The spec must:
1. Define the exact Rust type of `vsync_lock` (`Mutex<()>` or `RwLock<()>`).
2. Define whether the compositor takes an exclusive lock for the full flush pass or
   just for the fb-write step.
3. If using `Mutex`, acknowledge and quantify the mutual-exclusion stall during
   capture memcpy, and specify whether 2ms of compositor stall is acceptable at the
   system level.

The ambiguity between "lock" and "read-lock" across two sections is not a typo — it
reflects an unresolved design decision that must be resolved before writing `screenshot.rs`.

### B3: Encoding on the capture_worker Thread Blocks the Thread for All Subsequent Ops

Section 6.2 states: "There is no separate lock for CaptureEngine — it is accessed only
from the supervisor's main event loop, keeping the concurrency model simple." Section 6.4
implies that encoding runs on the "capture_worker thread". Section 6.5 says the frame pump
in `recorder.rs` spawns "one std::thread per session" for the frame pump loop.

These three statements cannot simultaneously be true for the one-shot screenshot path.
Here is the contradiction:

The WIT call `screenshot(target, format, scale)` (section 4) returns a `result<u32, string>`.
The WIT binding is synchronous from the WASM app's perspective: the app calls the function
and blocks until it returns. The host closure in `wit_handlers.rs` runs on the Wasmtime
thread executing that app (i.e., the app's dedicated supervisor thread). If the closure
calls `encode_png` synchronously, PNG encoding (~80–250ms) blocks that app's supervisor
thread for the full encode duration.

The spec attempts to address this in section 9.1 by saying "the supervisor does not block
on this; encoding runs on the capture_worker thread". But the WIT interface is synchronous
— the app blocks on the WIT call. Offloading to capture_worker requires the supervisor
app thread to wait for a response channel from that worker. The spec does not define:

1. What is `capture_worker`? Is it a named thread? A thread pool? A tokio task?
2. How does the synchronous WIT call block-wait on capture_worker without spinning or
   deadlocking? Is there a `std::sync::mpsc::channel` involved?
3. If the same `capture_worker` thread handles both one-shot screenshots and is the
   frame pump for active recording sessions, what is its scheduling model? Two concurrent
   screenshot requests plus an active recording session would queue on the same thread.

Without a defined thread model for the one-shot screenshot path through the WIT linker,
the spec's claim that "the supervisor does not block on this" is unverifiable. Either
encoding is synchronous on the app's supervisor thread (blocking that app for 80–250ms,
which may be acceptable for a screenshot tool but must be stated), or there is an async
dispatch mechanism that must be specified.

### B4: Frame Queue Overflow Drop Policy Is Incomplete — Silent Data Loss Without Notification

Section 6.5 defines the frame pump loop with `try_send` and increments `frames_dropped`
on `TrySendError::Full`. Section 4 provides a `dropped-frames` WIT function that lets
apps poll the drop count. Section 9.2 says "the app can monitor dropped-frames to detect
this and reduce fps."

This drop model has three specification gaps:

**Gap 1: No notification mechanism.** The app is not notified when drops start occurring.
It must poll `dropped-frames` periodically. If it polls at 1-second intervals, up to 30
frames (one full second at 30fps) can drop silently before the app learns about it and
adjusts. The spec does not define a recommended polling frequency or acknowledge this
latency.

**Gap 2: No backpressure path.** The only responses to a full queue are "drop the frame"
or "the app polls and reduces fps". There is no mechanism for the frame pump to slow
itself (e.g., adaptive sleep timing, dynamic fps reduction by the supervisor, or a
push-notification to the app's stdin like `VYOMA_CAPTURE_DROPPING:<session_id>`). The
spec should either specify an explicit backpressure protocol or justify why drop-and-poll
is sufficient for all use cases.

**Gap 3: What happens when frames_dropped overflows u64?** This is pedantic but
`dropped-frames` returns `u64` and "resets counter after read" per section 4. If the
app never calls `dropped-frames`, the counter accumulates indefinitely. At 30fps with
a consistently full queue, `u64::MAX / 30` seconds = ~19 billion years, so overflow is
not a runtime concern. However, "resets counter after read" is a stateful side-effect
on a read operation, which is an unusual API design. The spec should document this
explicitly as a destructive-read semantic and warn that two concurrent callers (if ever
supported) would race on the counter reset. In v1 with a single consumer this is fine,
but it should be stated.

The blocking concern is Gap 1 and Gap 2: the spec claims the recording subsystem is
usable without specifying how a well-behaved app detects and responds to encoder lag in
a timely manner. This is a liveness gap, not just a quality-of-life issue.

### B5: Output Path Construction Uses app_pid But app_pid Is Never Defined or Validated

Section 6.5 shows `make_output_path(captures_dir, app_pid, session_id, encoder)` and
section 11.1 claims "the app PID and session ID are integers assigned by the supervisor"
and "path traversal is structurally impossible."

The path traversal argument is sound for session_id (it is a supervisor-assigned u32
counter). But `app_pid` is described as "the app's PID" and section 6.5 shows it coming
from `start_session`'s parameter `app_pid: u32`. This raises the question: is `app_pid`
the Linux PID of the Wasmtime child process, or is it the VyomaOS app instance ID?

These are different values with different security properties:

- If `app_pid` is the Linux PID (assigned by the kernel), it is in the range 1–32768
  on most Linux systems, and the supervisor cannot control its value. A Wasmtime process
  might get PID 1000 on one boot and PID 2048 on the next. The path would be
  `/data/captures/1000/1.mjpeg`. This is deterministic per-session but not predictable.

- If `app_pid` is the VyomaOS app instance ID (supervisor-assigned), it is controlled
  by the supervisor and bounded, but the spec never connects this to the `app_pid`
  parameter name in `start_session`.

The actual path traversal risk that section 11.1 overlooks: `captures_dir` itself is
described as "a supervisor-controlled constant (`/data/captures`)" but section 6.2 shows
`CaptureEngine::new(captures_dir: impl Into<String>)` taking it as a constructor
parameter. Who calls `CaptureEngine::new`? If it is called from the boot.toml
configuration parser (which reads TOML from a user-editable file), then `captures_dir`
is not a compile-time constant — it is a runtime string from a config file. A modified
`boot.toml` with `captures_dir = "/etc"` would cause all recording output to land under
`/etc/<pid>/`. The spec must either:

1. Hard-code `captures_dir` as a compile-time constant (not a constructor parameter), or
2. Validate `captures_dir` at construction time against an allowlist of permitted roots
   (e.g., must start with `/data/`), or
3. Explicitly state that `boot.toml` is a trusted input and that modifying it requires
   the same trust as modifying the supervisor binary.

The claim "path traversal is structurally impossible" is overstated given the above.

---

## Non-Blocking Issues (should fix, won't block)

### N1: `format-code` and `encoder-code` Are Magic Numbers Packed Into `u8`

Section 4 defines `type format-code = u8` and `type encoder-code = u8` with a comment
explaining the encoding: "0 = PNG, 1 = Raw BGRA32, ... 10–109 = JPEG quality (value - 10)".
This is a stringly-typed API baked into a binary encoding. JPEG quality range 0–99 is
encoded as values 10–109, meaning 110 and above are undefined. H264 is encoded as 200.
Values 101–199 and 201–255 are unspecified.

WIT supports proper variants and enum types precisely to avoid this. The correct WIT
definition would be:

```
variant format {
    png,
    raw(pixel-format),
    jpeg(u8),  // quality 0–100
}
```

Using `type format-code = u8` with a prose comment is not a WIT interface — it is a
C-style integer protocol dressed up in WIT syntax. Any caller that passes value 110 will
get undefined behavior (neither JPEG nor anything else). The spec should use proper WIT
variants or at minimum define the full u8 space with explicit error handling for
out-of-range values.

### N2: `RecordingSession` Is Not `Send` Due to `SyncSender` But Is Stored in `HashMap`

Section 3 shows `RecordingSession` containing `frame_tx: std::sync::mpsc::SyncSender<CaptureFrame>`
and `encoder_thread: Option<std::thread::JoinHandle<()>>`. The `CaptureEngine` stores
sessions in `HashMap<u32, RecordingSession>` and is owned by `SupervisorState`.

`JoinHandle<()>` is `Send` (it can be moved to another thread). `SyncSender<CaptureFrame>`
is `Send` only if `CaptureFrame` is `Send`. `CaptureFrame` contains `Vec<u8>` and `CaptureFormat`,
both of which are `Send`. So the struct is `Send`. However, `CaptureEngine` is
accessed "only from the supervisor's main event loop" (section 6.2), while the frame pump
thread (which runs in `recorder.rs`) also needs access to the `RecordingSession` to update
`frames_captured` and `frames_dropped` counters.

Section 6.5 shows the pump thread doing `session.frames_captured += 1` and
`session.frames_dropped += 1`. But `session` is stored in `CaptureEngine::sessions` which
is owned by the main event loop. How does the pump thread mutate `session` fields without
a shared mutable reference? The spec never addresses this. Either the counters live inside
the `RecordingSession` (requiring a `Mutex` or `AtomicU64` for cross-thread mutation), or
they live in a separate per-session `Arc<AtomicU64>` pair that both the pump thread and the
engine can access. The pseudocode as written would not compile.

### N3: `capture_window` Read-Lock Duration Is Unquantified and May Stall Compositor

Section 6.3 states "window capture holds the RwLock<AppTable> read-lock for the pixel
copy duration only." An AppState surface for a 1920×1080 window is ~8MB. A `memcpy`
of 8MB takes ~2ms. The compositor also reads AppTable (read-lock) during rendering.

The issue is not contention between two readers (both hold read-locks simultaneously).
The issue arises when a write-lock is pending. In Rust's `std::sync::RwLock`, a pending
write waiter blocks all subsequent read-lock acquisitions once it is waiting (to prevent
writer starvation). If the compositor or another subsystem attempts to write-lock
AppTable (e.g., to insert a new app, update window positions, or apply Z-sort), and
capture holds a read-lock for 2ms, the write attempt is queued and all subsequent
read-lock attempts (including future compositor frames) stall behind it.

The spec should quantify whether 2ms of AppTable read-lock during window capture is
acceptable given the compositor's read-lock acquisition frequency, or specify that a
separate per-app surface lock (rather than AppTable-level) should be used.

### N4: H264 via x264 FFI in a musl Static Binary Is Not Straightforward

Section 6.4 states: "H264 via optional compile-time dependency (x264 via FFI). Feature-
gated: compile with --features h264." The supervisor binary is compiled as
`x86_64-unknown-linux-musl` (static, from CLAUDE.md). The `x264-sys` crate links to
`libx264.so` (a shared library). Static linking of x264 into a musl binary requires either:
(a) a static `libx264.a` compiled with musl, or (b) the x264 source included in the build
as a `cc` build-script dependency.

Neither option is trivial. `x264-sys` on crates.io expects a system-installed libx264.
The Docker build environment (Ubuntu 22.04) provides `libx264-dev` which targets glibc,
not musl. Cross-compiling x264 to musl requires patching the build.

The spec says "requires x264 shared lib on host" in the Cargo.toml comment, which
contradicts the supervisor build model (static musl binary). A shared-lib dependency would
force the supervisor to be dynamically linked when H264 is enabled, breaking the static
musl build and increasing the initramfs by the size of libx264 (~2MB). This is a
real integration cost that the spec glosses over with a single comment.

### N5: `stop-recording` Blocking for Up to 5 Seconds Deadlocks the WASM App

Section 6.5 states: "`stop_session` joins both threads with a 5-second timeout (if threads
do not finish in 5s, they are detached and an error is logged)." Section 4 defines
`stop-recording` as a synchronous WIT call that "blocks until the encoder thread flushes
and closes the file."

If the encoder thread is processing a large MJPEG file and the encoder is behind (frame
queue is full), the encoder thread may need up to 5 seconds to drain and close. For the
duration of this call, the WASM app's Wasmtime thread is blocked inside the WIT host
function. This means:
- The app cannot receive keyboard or mouse input during stop.
- The supervisor's stdout parser for that app is also blocked (depending on threading model).
- If the app has a watchdog (section in CLAUDE.md describes `watchdog_secs`), a 5-second
  blocking call could trigger the watchdog and kill the app before stop completes.

The spec should either make `stop-recording` asynchronous (returns immediately; a
`VYOMA_CAPTURE_SAVED:` line arrives later via stdin), or document the 5-second blocking
behavior explicitly and require that recording apps set `watchdog_secs` to at least 10.

### N6: Rate Limit Applies to `capture-frame` But Not to `start-recording`

Section 7.4 states the 10/s rate limit applies to `screenshot` and `capture-frame` but
not to `start-recording`. An app can call `start-recording` at high frequency: start a
session, immediately stop it (capturing one frame to disk), start again, repeat. Each
start+stop cycle captures one JPEG to `/data/captures/<pid>/`. At 1 cycle per 100ms,
this produces 10 files per second — equivalent to 10 screenshots per second, bypassing
the rate limit entirely.

The spec should either apply the rate limit to session creation (max N sessions started
per second, separate from the max-concurrent-sessions check), or clarify why this bypass
is acceptable (e.g., because disk I/O provides natural throttling and `filesystem = true`
is a separate capability gate).

### N7: Consent State Is Cleared on App Restart But Not on Supervisor Reload

Section 7.3 states "consent is stored only in memory for the current session" and is
"not persisted to disk". The `CapturePermission::consent_granted` field is initialized
to `false` at app launch. However, the supervisor supports `restart <name>` and `reload`
commands (CLAUDE.md). When an app is restarted by the supervisor:
- A new `app_id` (u32) may be assigned.
- The old `CapturePermission` entry (keyed by old `app_id`) is orphaned in
  `CaptureEngine::permissions`.
- The new `app_id` has `consent_granted = false`.

This means a restarted screen-recording app would show the consent dialog again on every
restart, even within the same boot session. This may be the intended behavior ("always
re-ask after restart"), but the spec should state it explicitly rather than leaving it
as an implicit consequence of the ID-keyed permission map. The orphaned permission entries
also constitute a minor memory leak that should be addressed by cleaning up on app exit.

### N8: The `capture_any` Warning-and-Ignore on Non-Desktop Profiles Is a Misleading UX

Section 7.3 states: "On server, mobile profiles, `capture_any` is not listed in the
platform capability matrix. If an app on those profiles declares `capture_any = true`,
the manifest parser logs a warning and ignores the field."

But the platform matrix in section 10 shows `capture_any perm = no` for server-headless
and mobile. An app developer writing a cross-platform app that declares `capture_any = true`
on desktop will see the runtime consent dialog; on mobile, the same capability declaration
is silently ignored and the app's full-screen capture attempts will fail with a runtime
permission error. The developer receives no compile-time or load-time signal that their app
will behave differently across profiles. The spec should define a platform-capability
mismatch error (distinct from the warning) that surfaces to the developer toolchain,
not just a supervisor log line.

---

## What the Spec Got Right

### Strength 1: Lock-Minimize Pattern for vsync Contention

Despite the type ambiguity (see B2), the spec's core insight in section 6.3 is correct:
hold the vsync lock for the minimum duration needed to memcpy pixels, then release before
encoding. The pseudocode illustrates this clearly with `let pixels = { let _guard = lock; fb.pixels.clone() };`. This is the right pattern — encoding outside the lock is non-negotiable
for a system with a 16ms vsync budget, and the spec gets this right.

### Strength 2: Structural Path Traversal Prevention

Section 11.1's argument that path traversal is structurally impossible for session output
paths is largely correct: `app_pid` and `session_id` are both u32 decimal values assigned
by the supervisor, and the extension is from a fixed Rust enum. No user-supplied string
ever touches the path construction in `make_output_path`. This is the right design. The
critique in B5 is about `captures_dir` being a constructor parameter, not about
`make_output_path` itself, which is solid.

### Strength 3: Dual Interface (WIT + stdout Protocol) Is Properly Symmetric

Sections 4 and 5 define WIT and stdout protocol variants that map 1:1 in semantics. The
same operations are available through both interfaces with the same capability enforcement.
This mirrors the existing `VYOMA_DRAW:` / `vyoma:draw@3.0.0` pattern successfully established
in earlier rounds. The protocol design is consistent with VyomaOS conventions and requires
no special-casing in the supervisor's main parsing loop.

### Strength 4: Bounded Queue with Explicit Drop Accounting

Using `SyncSender` with capacity 4 (section 6.5) is the right call for a soft-real-time
recording loop. The spec does not try to make the queue unbounded (which would cause
memory exhaustion on a slow encoder). Exposing `frames_dropped` via both WIT and the
stdout protocol gives callers a diagnostic signal. The distinction between `frames_captured`
and `frames_dropped` being tracked separately in `RecordingSession` is good operational
visibility. The spec acknowledges the performance limits honestly (section 9.2: "at 30fps
MJPEG with quality 70, the encoder is near capacity").

### Strength 5: Platform Matrix Is Explicit and Compile-Time Enforced

Section 10 presents a clear feature matrix with per-row feature flags and per-column
platform profiles. More importantly, section 10's final paragraph specifies that platform
gating is done "at compile time in the platform profile loader" — disabled platforms
produce stub implementations that return `Err("capture not available on this platform")`,
not panics or undefined behavior. This is the correct approach for a multi-profile OS
where binary size and feature surface must be controlled per deployment target.

### Strength 6: Open Questions Are Genuinely Open (Not Deferred Problems)

Section 16 lists four open questions: cursor inclusion, audio interleaving, consent
persistence, and thumbnail API. Each is a real design question with non-obvious tradeoffs,
and the spec's provisional answers are defensible. The cursor question (exclude from window
capture, include in full-screen) is the correct default for the R18 model. The "video-only
in R18; R20 decides on container format" is the right way to avoid premature coupling to
the audio subsystem. This section demonstrates architectural self-awareness.

---

## Questions for the Architect

### Q1: What Is the Concurrency Model for the One-Shot Screenshot WIT Path?

Section 6.6 says WIT closures dispatch to `screenshot::*` or `recorder::*`. But WIT host
functions execute on the Wasmtime thread for that app. If encoding is synchronous inside
the WIT closure, the app's thread blocks for 80–250ms (PNG) or 30–60ms (JPEG). Is this
the intended model, or is there an async dispatch to `capture_worker` with a channel
park? If async, what prevents two concurrent screenshot calls from the same app from
queuing behind each other indefinitely on `capture_worker`?

### Q2: How Does `CaptureEngine::check_permission` Access `capture_any` Consent State When Called From a Frame Pump Thread?

Section 6.5 says the frame pump thread calls `screenshot::capture_*`. But `capture_*`
functions in section 6.3 do not accept a `CapturePermission` parameter — they take
`fb`, `vsync_lock`, `request`, and `now_us`. Permission checking happens in `wit_handlers.rs`
before dispatching. In the recording path, permission was checked once at `start-recording`
time (section 6.6). If consent is revoked mid-session (user denies via a second consent
dialog), the running frame pump thread continues capturing because there is no per-frame
permission check. Should the frame pump re-check consent on each frame, or is revocation
only effective at the next `start-recording` call? The spec does not address revocation.

### Q3: What Is the Maximum Total Thread Count at Full Load?

The supervisor already spawns one thread per WASM app (up to 10 apps per CLAUDE.md
current state). Section 6.5 spawns two threads per recording session (frame pump +
encoder). With 2 sessions per app and 10 apps, worst case is 20 + 10 = 30 threads plus
the main event loop. Does this fit within the supervisor's threading budget? Is there a
global thread limit defined anywhere? The spec should enumerate the maximum thread count
at full load and confirm it does not trigger Linux's per-process thread limit (default
1024) or cause scheduler thrash on the 1-vCPU QEMU target.

### Q4: How Are `CaptureFrame` Allocations Managed Across the Frame Pump and Encoder Threads?

Section 6.5's frame pump loop calls `screenshot::capture_*` on each tick, which
allocates a `Vec<u8>` of ~8MB (raw 1080p frame) or ~300KB (JPEG). With a bounded queue
of capacity 4, up to 5 frames can be in-flight simultaneously (4 in queue + 1 being
encoded). That is 40MB of frame buffers per session (raw), or more realistically 1.5MB
per session (JPEG). When the encoder thread writes the frame to disk and drops the
`CaptureFrame`, the allocation is freed. Is there a frame buffer pool (pre-allocated
`Vec<u8>` reuse) to avoid repeated large allocations at 30fps? At 30 allocs/sec of
8MB each, the allocator is under non-trivial pressure. The spec should address whether
a pool is used or whether the allocator is expected to handle this without observable
latency spikes.

### Q5: Why Is `capture_any` Restricted to Desktop-Full But Server-Headless Allows `screenshot` Without Consent?

Section 10 shows `capture_any perm = no` for server-headless but `Screenshot PNG/JPEG/raw`
all `yes`. Section 7.1 states `capture = true` allows capturing own window and
`capture_any = true` allows capturing "any window or full screen (requires runtime consent
on desktop)." On server-headless, there is no interactive display and no chrome app to
show a consent dialog. A server process with `capture = true` can capture the full screen
(which on server-headless is just the supervisor's own composited output). Is this the
intended permission model? If so, the spec should clarify that on server-headless, full-
screen capture is implicitly granted with `capture = true` because there is no user to
prompt, and the only "screen" is the app's own supervisor-managed output. The permission
table in section 7.2 does not cover this server case.

### Q6: What Happens to Active Recording Sessions When the Capturing App Exits?

Section 6.5 describes `stop_session` as an explicit call. But what if the WASM app exits
abnormally (crash, `process::exit(1)`, or killed by the supervisor via the `kill` command)?
The supervisor detects app exit via the Wasmtime child thread terminating. At that point:
- The `frame_tx` in `RecordingSession` has one live reference (in the session stored in
  `CaptureEngine`).
- The frame pump thread is still running, calling `screenshot::capture_*` and pushing
  to `frame_tx`.
- The encoder thread is still running, writing to disk.

Neither the frame pump nor the encoder thread observes app exit directly; they only observe
channel disconnection. The spec must specify whether the supervisor calls `stop_session`
for all sessions belonging to a dead app as part of app teardown, and if so, whether the
5-second thread join timeout applies. If an encoder thread is mid-write on a 4GB raw
recording file at the time of forced shutdown, the output file will be truncated. Is a
partial file acceptable, or should the supervisor write a trailer/index frame to allow
partial file recovery?

---

## Closing Assessment

Round 18 is a well-structured spec that handles the easy parts well: dual interface
symmetry, platform matrix, bounded queuing, and structural path safety for session output.
The depth of detail in sections 9 and 10 demonstrates genuine engineering thought rather
than hand-waving. However, the five blocking issues are not minor oversights — B1 (WASM
capability enforcement), B2 (mutex type ambiguity), B3 (encoding thread model), B4
(backpressure specification), and B5 (captures_dir trust assumption) each represent a
gap where an implementer would either make a wrong choice or need to invent architecture
not present in the spec. B1 in particular is the most dangerous: a globally-registered
WIT linker without conditional per-app capability gating undermines the entire VyomaOS
capability security model. The spec should be returned to the architect for a revision
pass addressing B1–B5 before implementation begins. Non-blocking issues N1, N2, and N5
should also be addressed in the same revision as they involve API design and compilation
correctness, not just documentation clarity.
