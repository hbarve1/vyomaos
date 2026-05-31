# Round 22 — App Lifecycle Management (Architect)

**Status**: Draft
**Round**: 22
**Subsystem**: App Lifecycle Management
**Analogue**: macOS `NSApplicationDelegate`, `UIApplication` (launch / foreground / background / suspend / terminate states)
**Author**: Architect
**Date**: 2026-05-29

---

## 1. Overview and Motivation

VyomaOS through Round 21 treats an app as a binary state machine: either a Wasmtime
process is running or it is not. The supervisor starts apps listed in `boot.toml`, routes
their output to the display and IPC broker, and kills them on request. This model is
sufficient for a demonstration OS but fails to support three scenarios that define a real
desktop experience.

**Scenario 1: Graceful quit.** On macOS, pressing Cmd+Q sends `applicationWillTerminate`
to the delegate. The app saves its state, closes file handles, and flushes buffered output
before the process exits. In VyomaOS today, `kill <app>` sends SIGKILL immediately. The
app loses any pending writes to `/data`, any in-progress IPC messages it was composing, and
any partially completed `VYOMA_DRAW:` command sequence. For apps that open SQLite-style
append-only log files in `/data`, this is silent data corruption.

**Scenario 2: Space-aware suspension.** Round 21 introduced spaces. When a user switches
from Space 1 to Space 2, apps on Space 1 should stop receiving keyboard and mouse events.
A running terminal app on Space 1 should not consume CPU polling for keystrokes that will
never arrive. However, the supervisor has no formal mechanism to notify an app that it has
been moved out of the active space, nor a mechanism for the app to opt into a background
processing mode versus a fully suspended mode.

**Scenario 3: Restart policy with backoff.** Several apps use `restart = "always"` in
their `vyoma.toml`. If a daemon crashes instantly on startup due to a dependency not yet
available, it will restart in a tight loop: the supervisor restarts it on every exit with
zero delay. This wastes CPU and fills the log buffer. A production supervisor needs
exponential backoff with a cap on restarts per hour.

Round 22 introduces a formal **App Lifecycle** subsystem. It adds six explicit app states,
a protocol for pushing lifecycle events to apps via stdin, a graceful termination sequence
with a 2-second flush window, a refined restart policy with exponential backoff and an
hourly cap, a `reload` command for hot-applying manifest changes, a WIT interface for apps
to interact with lifecycle events programmatically, and integration points with both the
watchdog and the window manager introduced in R21.

### Relationship to Prior Rounds

| Round | Contribution reused or extended |
|-------|---------------------------------|
| R13   | `watchdog_secs` in `AppState`; timeout kills on stdout silence |
| R18   | `capture` capability; app output routing model |
| R21   | `win_space`, `win_visible`, `win_manual_layout`; `wm::compact_z_order` |
| R17   | `VYOMA_DRAW:flush` as the primary app liveness signal |
| R11   | IPC broker routing `@<app>: msg` messages via supervisor stdin writes |

### What R22 Does Not Do

- No graphical "app is launching" spinner (deferred to chrome R23).
- No process sandboxing beyond existing capability enforcement.
- No cross-app lifecycle dependencies (app A waiting for app B to reach `Running`).
- No checkpoint / restore (freeze app state to disk and resume later).
- No background network fetch policy (that is a capability concern for a later round).
- No GUI for viewing lifecycle state (deferred to process inspector R25).

---

## 2. App States

### 2.1 State Enumeration

Every app in VyomaOS has exactly one lifecycle state at all times. The state is stored in
`AppState` as a new field `lifecycle: LifecycleState`. The six states are:

```rust
// supervisor/src/lifecycle/state.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    /// Wasmtime process has been started; supervisor is waiting for the app to
    /// signal readiness via VYOMA_LIFECYCLE:ready on stdout or for the first
    /// VYOMA_DRAW:flush (for display apps). No keyboard or mouse events are
    /// delivered during Launching.
    Launching,

    /// App is active, in the foreground space, producing output. All input
    /// events are delivered normally. Watchdog timer is active.
    Running,

    /// App's window is in a non-active space and the app has NOT opted into
    /// background policy. No keyboard or mouse events are delivered. Watchdog
    /// timer is PAUSED (suspended apps are expected to be silent). IPC messages
    /// (via @<app>:) are still delivered to stdin.
    Suspended,

    /// App's window is in a non-active space and the app HAS opted into
    /// background policy via notify-ready + set-background-policy. IPC messages
    /// are delivered. No keyboard or mouse events are delivered. Watchdog timer
    /// remains ACTIVE (background apps must remain responsive).
    Background,

    /// kill <app> was issued, or the app exceeded the hourly restart cap.
    /// VYOMA_LIFECYCLE:will_terminate has been written to app stdin. The
    /// supervisor waits up to 2 seconds for voluntary exit, then SIGKILL.
    Terminating,

    /// The Wasmtime process has exited (voluntarily or via SIGKILL). The
    /// AppState record is retained for log access and potential restart, but
    /// the process handle is None. WM resources are released.
    Terminated,
}
```

### 2.2 State Semantics Summary

| State        | Process alive | Input delivered | IPC delivered | Watchdog active | Display composited |
|--------------|---------------|-----------------|---------------|-----------------|-------------------|
| Launching    | Yes           | No              | No            | No              | Yes (partial)     |
| Running      | Yes           | Yes             | Yes           | Yes             | Yes               |
| Suspended    | Yes           | No              | Yes           | No              | No                |
| Background   | Yes           | No              | Yes           | Yes             | No                |
| Terminating  | Yes           | No              | No            | No              | Yes (last frame)  |
| Terminated   | No            | —               | —             | —               | No                |

Note on "Display composited" during Launching: an app may emit `VYOMA_DRAW:` commands
before reaching `Running` (e.g., a splash screen). The compositor blits the surface as
normal; the lifecycle state does not gate rendering. The distinction is that the supervisor
does not forward keyboard or mouse events until the app is `Running`.

### 2.3 Initial State

Every app enters `Launching` when its Wasmtime child process is started. The transition
to `Running` occurs on one of two signals, whichever arrives first:

1. The app writes `VYOMA_LIFECYCLE:ready` to stdout (explicit readiness declaration).
2. The app writes `VYOMA_DRAW:flush` to stdout (implicit readiness for display apps).

For apps that have neither `display = true` nor ever emit `VYOMA_LIFECYCLE:ready`, the
supervisor transitions them to `Running` after 500 ms as a fallback to avoid apps being
permanently stuck in `Launching` state. This fallback is logged at `WARN` level.

---

## 3. Lifecycle Events — Push Protocol

The supervisor communicates lifecycle state changes to apps by writing event lines to the
app's stdin. Apps read these lines alongside normal IPC messages. The format is a reserved
prefix `VYOMA_LIFECYCLE:` followed by the event name.

### 3.1 Event Catalogue

| Event                         | Sent when                                                     |
|-------------------------------|---------------------------------------------------------------|
| `VYOMA_LIFECYCLE:launching`   | Process has just started (written before any other stdin)     |
| `VYOMA_LIFECYCLE:running`     | Supervisor transitions app to Running state                   |
| `VYOMA_LIFECYCLE:suspended`   | App's space becomes inactive; background policy NOT set       |
| `VYOMA_LIFECYCLE:resumed`     | App's space becomes active again after Suspended              |
| `VYOMA_LIFECYCLE:background`  | App's space becomes inactive; background policy IS set        |
| `VYOMA_LIFECYCLE:foreground`  | App returns to active space from Background                   |
| `VYOMA_LIFECYCLE:will_terminate` | kill issued; app has 2 seconds to flush and exit           |
| `VYOMA_LIFECYCLE:ready`       | App → supervisor signal (on stdout, not stdin); see §8        |

`VYOMA_LIFECYCLE:terminated` is intentionally absent: the app is dead and cannot receive
it. The supervisor logs the termination to its own structured log.

### 3.2 Stdin Delivery Guarantees

The supervisor writes lifecycle events to the app's stdin pipe using a non-blocking write
with a 100 ms timeout. If the write would block (app's stdin buffer is full — unlikely but
possible if the app is not consuming stdin), the supervisor logs a warning and proceeds
without writing the event. The app's lifecycle state transitions regardless of whether the
event was successfully delivered: state transitions are authoritative in the supervisor,
not in the app. An app that ignores lifecycle events is still subject to all state-based
restrictions (no input delivery, watchdog paused, etc.).

### 3.3 Event Ordering Guarantee

Lifecycle events are always written to stdin before other events that follow from the same
state transition. For example, when an app transitions Suspended → Resumed and a keyboard
event is also pending: the supervisor writes `VYOMA_LIFECYCLE:resumed` first, then delivers
the keyboard event. This ensures the app can prepare its rendering pipeline before
receiving input.

### 3.4 Rust App Example

```rust
use std::io::{self, BufRead};

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        match line.as_str() {
            "VYOMA_LIFECYCLE:launching" => on_launching(),
            "VYOMA_LIFECYCLE:running"   => on_running(),
            "VYOMA_LIFECYCLE:suspended" => on_suspended(),
            "VYOMA_LIFECYCLE:resumed"   => on_resumed(),
            "VYOMA_LIFECYCLE:background"  => on_background(),
            "VYOMA_LIFECYCLE:foreground"  => on_foreground(),
            "VYOMA_LIFECYCLE:will_terminate" => {
                on_will_terminate();
                // App must exit within 2 seconds after this point.
                std::process::exit(0);
            }
            _ => handle_ipc_or_input(&line),
        }
    }
}
```

### 3.5 Protocol Prefix Reservation

Lines beginning with `VYOMA_LIFECYCLE:` are reserved by the supervisor and must not be
produced by apps on their own stdout (doing so would cause the supervisor to misinterpret
the app's output as a supervisor-generated event echo). Apps that need to emit lifecycle-
like debug strings must use a different prefix (e.g., `LOG:lifecycle:`).

---

## 4. Graceful Termination

### 4.1 Termination Sequence

When `kill <app>` is issued (either via the IPC shell command or via the `request-terminate`
WIT call), the supervisor executes the following sequence:

```
Step 1: Validate app exists and is not already Terminating or Terminated.
         If already Terminating, no-op and return "already terminating".
         If already Terminated, no-op and return "already terminated".

Step 2: Transition app state to Terminating.
         Record termination_deadline = Instant::now() + Duration::from_secs(2).

Step 3: Write "VYOMA_LIFECYCLE:will_terminate\n" to app stdin (non-blocking, 100ms timeout).

Step 4: Pause watchdog for this app (set watchdog_paused = true in AppState).

Step 5: Stop delivering new keyboard and mouse events to this app.

Step 6: Return immediately to the supervisor event loop.
         Do NOT block waiting for the app to exit here.
```

The supervisor's main event loop checks all `Terminating` apps on every tick. On each
tick, for every app in `Terminating` state, the loop checks:

```
If app process has exited voluntarily:
    → Transition to Terminated.
    → Run post-termination cleanup (§4.3).
    → Done.

If Instant::now() >= termination_deadline:
    → Send SIGKILL to Wasmtime process.
    → Wait with waitpid(WNOHANG) for up to 100ms.
    → Transition to Terminated.
    → Run post-termination cleanup (§4.3).
    → Log: "[lifecycle] <app> forcibly killed after 2s grace period expired"
    → Done.
```

This design ensures the 2-second wait never blocks the event loop. The supervisor continues
compositing frames, routing IPC for other apps, and processing keyboard input for the
focused app while waiting for the terminating app to exit.

### 4.2 App Responsibilities During Termination

When an app receives `VYOMA_LIFECYCLE:will_terminate`, it MUST:

1. Stop accepting new work (flush any queued IPC outbox).
2. Close file handles to `/data` (triggering OS-level flush of 9P mmap buffers).
3. Emit a final `VYOMA_DRAW:flush` if it wants its last frame to be composited.
4. Call `std::process::exit(0)` or return from `main()` within 2 seconds.

An app that does not implement `will_terminate` handling (ignores the lifecycle event) will
be SIGKILL'd after 2 seconds. This is acceptable for stateless apps (e.g., clocks, system
monitors). It is NOT acceptable for apps with open file handles in `/data`. The manifest
for such apps SHOULD set `watchdog_secs` and SHOULD handle `will_terminate`.

### 4.3 Post-Termination Cleanup

After an app transitions to `Terminated`, the supervisor performs the following cleanup
unconditionally (regardless of voluntary vs forced exit):

1. **WM cleanup**: Call `wm::on_app_terminated(app_name)`:
   - Remove app from `Z_ORDER` vector.
   - Call `wm::compact_z_order()` to close gaps.
   - If app was in `active_space`, call `apply_tiling_layout()` to redistribute screen.
   - Set `win_visible = false` in `AppState` (record preserved for log access).

2. **Input cleanup**: If app held keyboard focus, reassign focus to the next app in
   Z_ORDER (topmost visible app in active space). If no such app exists, focus is unset.

3. **IPC cleanup**: Remove app name from IPC routing table so subsequent `@<app>:` messages
   are dropped with a supervisor warning rather than silently buffering forever.

4. **Surface cleanup**: Drop `AppState.surface` Arc. If the refcount reaches zero (no
   compositor pass currently reading it), the surface buffer is freed.

5. **Process handle**: Set `AppState.process_handle = None`.

### 4.4 Forced Termination Edge Cases

**App ignores SIGKILL**: On Linux, SIGKILL cannot be ignored or caught. If Wasmtime itself
is in an uninterruptible kernel sleep (D state), SIGKILL will not immediately deliver. The
supervisor logs this condition and retries `kill(SIGKILL)` every 500 ms for up to 5 seconds,
then logs a fatal error and marks the app `Terminated` anyway (accepting the zombie process).

**App stdin pipe is closed before `will_terminate` write**: This can happen if the app
has already exited by the time kill is issued. The write will return EPIPE. The supervisor
catches this error, skips steps 3 and 6 of the termination sequence, and immediately
transitions to Terminated.

---

## 5. Restart Policy

### 5.1 Manifest Restart Field

The existing `restart` field in `vyoma.toml` is extended:

```toml
[app]
name    = "my-daemon"
version = "1.0.0"
wasm    = "my-daemon.wasm"
restart = "always"          # "never" | "always" (unchanged from prior spec)
```

`restart = "never"` (one-shot): app is not restarted after exit. Default.
`restart = "always"`: app is restarted on any exit (including clean exit with code 0),
subject to the backoff and hourly cap defined below.

### 5.2 Backoff Algorithm

When `restart = "always"` and an app exits, the supervisor schedules a restart using
an exponential backoff. New fields on `AppState`:

```rust
pub struct AppState {
    // ... existing fields ...

    /// Monotonic timestamp of the most recent spawn of this app.
    pub spawn_time: Option<Instant>,

    /// Number of restarts since the last backoff reset.
    pub restart_count_window: u32,

    /// Monotonic timestamp of the last restart count window start.
    pub restart_window_start: Instant,

    /// Monotonic timestamp when the next restart is allowed.
    pub next_restart_at: Option<Instant>,

    /// Total restarts in the current rolling 3600-second window.
    pub restarts_this_hour: u32,

    /// Monotonic timestamp of the start of the current hourly window.
    pub hourly_window_start: Instant,
}
```

**Backoff table**:

| restart_count_window | Delay before next restart |
|----------------------|---------------------------|
| 0 (first restart)    | 0 s                       |
| 1                    | 1 s                       |
| 2                    | 2 s                       |
| 3                    | 4 s                       |
| 4                    | 8 s                       |
| ≥ 5                  | 16 s (cap)                |

**Backoff reset rule**: When an app has been alive (in `Running`, `Suspended`, or
`Background` state) continuously for ≥ 60 seconds, `restart_count_window` is reset to 0
and `next_restart_at` is cleared. "Alive continuously" is measured from `spawn_time`:
`Instant::now() - spawn_time >= Duration::from_secs(60)`. The reset is checked on each
supervisor tick; it is NOT event-driven.

The choice of `spawn_time` (not `last_output_time`) is intentional. An app that starts
successfully and then goes silent for 61 seconds (e.g., a daemon waiting on a timer) should
have its backoff reset. Using `last_output_time` would penalise well-behaved quiet daemons.

**Why `spawn_time` not `exit_time`**: The 60-second window is measured from the spawn, not
from the previous exit. This ensures that an app which crashes at t=59s does NOT get its
backoff reset, even if it was technically alive for 59s. The window requirement is 60s of
continuous life in the current invocation.

### 5.3 Hourly Cap

The hourly restart cap prevents a persistently crashing app from consuming unbounded
resources. Implementation:

```
On each restart attempt:
    If Instant::now() - hourly_window_start >= 3600s:
        Reset hourly_window_start = Instant::now()
        Reset restarts_this_hour = 0

    If restarts_this_hour >= 10:
        Do NOT restart.
        Transition app to Terminated state (permanent).
        Log: "[lifecycle] <app> exceeded 10 restarts/hour; giving up"
        Return.

    restarts_this_hour += 1
    Schedule restart.
```

An app that has been given up on (exceeded hourly cap) can only be revived by an explicit
`restart <app>` command from the operator, which resets all backoff counters.

### 5.4 Restart Scheduling Without a Timer Thread

Restarts are not implemented with a background timer thread. Instead, the supervisor's
main event loop maintains a list of pending restart entries:

```rust
struct PendingRestart {
    app_name: String,
    restart_at: Instant,
}
```

On each tick, the event loop scans `pending_restarts` and spawns any apps whose
`restart_at <= Instant::now()`. This is O(N) in the number of apps, which is acceptable
given VyomaOS targets ≤ 20 simultaneous apps in the current phase.

---

## 6. The `reload` Command

### 6.1 Purpose

`reload` allows an operator to modify `boot.toml` (add or remove apps, change manifests)
and apply the change without rebooting. The command is issued via the IPC shell or as
`@supervisor: reload`.

### 6.2 Reload Sequence

```
Step 1: Re-read /etc/vyoma/boot.toml from disk.
         On parse error: abort reload, log error, return "reload failed: <parse error>".
         Do NOT modify any running app state.

Step 2: For each app path listed in the new boot.toml, re-read its vyoma.toml manifest.
         On per-app manifest parse error: skip that app, log warning, continue with others.
         The reload is partial (best-effort per app) rather than all-or-nothing.

Step 3: Compute the diff between current running apps and new manifest set:
         - New apps (in new boot.toml but not currently running): will be started.
         - Removed apps (currently running but not in new boot.toml): will be killed.
         - Unchanged apps (same name, identical manifest): left as-is.
         - Changed apps (same name, modified manifest): killed and restarted.

Step 4: For removed and changed apps: initiate graceful termination (§4.1).
         Wait for ALL terminating apps to reach Terminated state before starting new apps.
         Timeout: 5 seconds total. After 5s, SIGKILL any still-Terminating apps.

Step 5: Start new and changed apps in the order they appear in boot.toml.
```

### 6.3 Manifest Change Detection

A manifest is considered "changed" if ANY field in `[app]` or `[capabilities]` differs
between the on-disk version and the in-memory version loaded at last boot. This includes:

- `[app].name`, `[app].version`, `[app].wasm` (binary path change → always restart)
- `[app].restart` policy change
- Any `[capabilities]` boolean toggle
- `[app].watchdog_secs` change
- Any peripheral capability field (`gpio_pins`, `i2c_bus`, etc.)

Changes to `[app].version` alone do not trigger a restart unless the wasm binary path
also changed. Version is a metadata field for human inspection.

### 6.4 Reload Failure Model

**Truncated boot.toml**: If the file is being edited and contains a partial write (e.g.,
truncated mid-TOML), the TOML parser will return a parse error at Step 1. Reload aborts.
No app state is changed. The operator fixes the file and re-issues `reload`.

**Missing wasm binary**: If a new app's vyoma.toml references a `.wasm` binary that does
not exist in the rootfs, the app is skipped with a warning. Other new apps proceed normally.

**Concurrent reload**: If a second `reload` command arrives while the first is in its
Step 4 wait (terminating old apps), the second command is rejected with "reload in progress".
A `busy` flag on the supervisor prevents concurrent reloads.

### 6.5 Reload Atomicity Guarantee

The reload is NOT atomic at the level of the running system — some apps will be down
between Step 4 and Step 5. The guarantee is only: "if boot.toml cannot be parsed, no app
state changes." The spec explicitly does NOT guarantee zero-downtime reload. Zero-downtime
hot-reload (keeping old app running until new app signals ready) is deferred to a future
round (R28 tentative).

---

## 7. WIT Interface `vyoma:lifecycle@1.0.0`

The lifecycle subsystem exposes a WIT interface that apps compiled against the
`wasm32-wasip2` target can use to interact with the supervisor programmatically. The
interface is in addition to (not replacing) the stdout/stdin protocol described in §3 and §8.

### 7.1 WIT Definition

```wit
// supervisor/wit/vyoma-lifecycle.wit

package vyoma:lifecycle@1.0.0;

interface app-lifecycle {
    /// The six lifecycle states an app can be in.
    enum app-state {
        launching,
        running,
        suspended,
        background,
        terminating,
        terminated,
    }

    /// Query the current lifecycle state of THIS app (self-query only).
    /// Apps cannot query the state of other apps via this interface.
    get-state: func() -> app-state;

    /// App calls this to signal it has finished initialisation.
    /// Equivalent to writing "VYOMA_LIFECYCLE:ready" on stdout.
    /// Calling this transitions the app from Launching to Running.
    /// Calling this when already Running is a no-op.
    notify-ready: func();

    /// Request graceful termination of THIS app.
    /// The supervisor will begin the will_terminate sequence (§4.1) for this app.
    /// The app will receive a VYOMA_LIFECYCLE:will_terminate event on stdin shortly
    /// after this call returns. The call does not block until exit.
    request-terminate: func();

    /// Opt into Background state instead of Suspended when the app's space becomes
    /// inactive. Must be called during Launching or Running state to take effect.
    /// If called while already Suspended, the app transitions immediately to Background
    /// and the supervisor writes VYOMA_LIFECYCLE:background to stdin.
    /// If not called, the default behaviour is Suspended.
    set-background-policy: func(enabled: bool);

    /// Query whether background policy is currently enabled for this app.
    get-background-policy: func() -> bool;
}

world lifecycle-world {
    import app-lifecycle;
}
```

### 7.2 Implementation Notes

WIT imports are wired up by the supervisor at app launch as a WASI Preview 2 component.
The host-side implementation lives in `supervisor/src/lifecycle/wit_host.rs`. Each WIT
call is serviced synchronously from the Wasmtime calling thread.

`get-state` reads `AppState.lifecycle` under a read lock on `APPS_MAP` (the same global
map used by the compositor). It does not block the supervisor event loop.

`notify-ready` sets `AppState.lifecycle = LifecycleState::Running` if the current state
is `Launching`, then writes `VYOMA_LIFECYCLE:running` to the app's stdin (via the reverse
pipe used for supervisor-to-app messages). This is the same pipe that delivers keyboard
events. The write is non-blocking.

`set-background-policy` sets `AppState.background_policy = enabled`. If the app is
currently `Suspended` and `enabled = true`, the supervisor immediately transitions the app
to `Background` state (updates `AppState.lifecycle`) and writes `VYOMA_LIFECYCLE:background`
to stdin.

### 7.3 Access Control

The lifecycle WIT interface is available to ALL apps unconditionally — it does not require
a specific capability declaration in `vyoma.toml`. The reasoning: every app needs to be
able to signal readiness and request termination. These are fundamental process operations,
not privileged capabilities. The only operation that could be considered privileged —
requesting termination of another app — is intentionally excluded from the interface (apps
can only terminate themselves via `request-terminate`).

---

## 8. VYOMA_LIFECYCLE Stdout Protocol

### 8.1 App-to-Supervisor Events (stdout)

Apps communicate with the supervisor by writing line-oriented text to stdout. Two
lifecycle-relevant lines are defined:

| Line                       | Meaning                                                     |
|----------------------------|-------------------------------------------------------------|
| `VYOMA_LIFECYCLE:ready`    | App has finished launching; equivalent to `notify-ready`   |
| `VYOMA_LIFECYCLE:background_opt_in` | App requests Background policy; equivalent to `set-background-policy(true)` |

`VYOMA_LIFECYCLE:background_opt_in` is provided for apps that do not link against the WIT
interface and prefer the simpler stdout protocol. It is treated identically to
`set-background-policy(true)` in the WIT interface.

### 8.2 Supervisor-to-App Events (stdin)

These are the same events listed in §3.1. The stdout and stdin protocols are
complementary: apps that link the WIT interface use WIT function calls for outbound
communication and can still receive inbound lifecycle events via stdin (the supervisor
always writes to stdin regardless of whether the app uses WIT or stdout protocol).

### 8.3 Protocol Versions

The lifecycle protocol version is encoded in the launching event, reserved for future
extension:

```
VYOMA_LIFECYCLE:launching:v1
```

Current parsers MUST handle both `VYOMA_LIFECYCLE:launching` (version-less) and
`VYOMA_LIFECYCLE:launching:v1` as identical. Version negotiation is out of scope for R22.

---

## 9. Background vs Suspended — Policy Rules

### 9.1 Default Behavior (No Policy Set)

An app that has NOT called `set-background-policy(true)` or written
`VYOMA_LIFECYCLE:background_opt_in` to stdout is subject to the default policy:

- When its space becomes inactive (space switch away from the app's space): Suspended.
- When its space becomes active again: Resumed.

A Suspended app receives no keyboard events, no mouse events, and no new `VYOMA_DRAW:flush`
compositing (its last-rendered frame remains visible if and only if the space is visible,
which it is not — the space is inactive). The watchdog is paused.

### 9.2 Background Policy

An app that HAS set background policy follows the alternative rule:

- When its space becomes inactive: Background (not Suspended).
- When its space becomes active again: Foreground (not Resumed).

A Background app:
- Still receives IPC messages (`@<app>: msg` routing continues).
- Does NOT receive keyboard or mouse events.
- Continues producing stdout output (e.g., network polling, log tailing).
- Has its watchdog remain active with the same `watchdog_secs` as during Running.
- May emit `VYOMA_DRAW:` commands, which are parsed but not composited (space is inactive).
  Draw commands during Background state are discarded after parsing. This prevents the
  app's surface buffer from consuming unbounded memory (draw-only-when-visible rule).

### 9.3 Transition Table

The complete state transition table for space-switch events:

| Current State | Space Switch Away         | Space Switch Back        |
|---------------|---------------------------|--------------------------|
| Running       | → Suspended OR Background | (n/a)                    |
| Suspended     | (n/a)                     | → Running                |
| Background    | (n/a)                     | → Running                |
| Launching     | No transition (wait for Running first) | (n/a)      |
| Terminating   | No transition             | No transition            |
| Terminated    | No transition             | No transition            |

When a space switch brings a previously-Suspended or Background app back to the foreground,
the supervisor always transitions it to `Running` (not back to Suspended or Background).
Running is the canonical "app is in foreground" state.

---

## 10. Watchdog Integration

### 10.1 Existing Watchdog Behavior (R13)

`watchdog_secs` in `vyoma.toml` specifies a silence timeout: if the app produces no
stdout output for N seconds, the supervisor kills it. This is implemented in the supervisor's
output-reading thread: the last output time is updated on each line read; if the gap exceeds
`watchdog_secs`, the process is killed.

### 10.2 R22 Watchdog Extensions

R22 adds lifecycle-state-awareness to the watchdog. New field on `AppState`:

```rust
pub watchdog_paused: bool,
```

The watchdog silence timer is only checked when `watchdog_paused == false`. The watchdog
is paused (`watchdog_paused = true`) when:

- The app is in `Suspended` state. Rationale: suspended apps are expected to be silent;
  killing them for silence defeats the purpose of suspension.
- The app is in `Terminating` state. Rationale: the app has received `will_terminate` and
  is executing shutdown logic; it may go silent for up to 2 seconds. The dedicated
  termination timeout (§4.1) handles the 2-second kill, not the watchdog.
- The app is in `Launching` state. Rationale: apps may take time to initialise before
  producing their first output; they should not be killed before reaching Running.

The watchdog remains active (`watchdog_paused = false`) when the app is in `Running` or
`Background` state.

### 10.3 Watchdog Pause/Resume on State Transition

On each lifecycle state transition, the supervisor updates `watchdog_paused` as follows:

```rust
fn update_watchdog_pause(state: LifecycleState) -> bool {
    matches!(state, LifecycleState::Suspended
                  | LifecycleState::Terminating
                  | LifecycleState::Launching)
}
```

The `last_output_time` is also reset to `Instant::now()` when the watchdog is un-paused
(i.e., on transition from Suspended → Running or Terminating → [impossible, but for
correctness] or Launching → Running). This prevents a stale `last_output_time` from
triggering an immediate watchdog kill the moment the app resumes.

---

## 11. WM Integration

### 11.1 Lifecycle State → WM Actions

The WM subsystem introduced in R21 tracks `win_visible` per app. R22 extends WM integration:

**`Running` → `Suspended`**:
- Set `AppState.win_visible = false`.
- The compositor skips this app's surface in the active-space blit pass.
- Do NOT call `apply_tiling_layout()` — suspended apps retain their tiling slot.
  Rationale: if the user switches back quickly, they expect windows to be in the same
  positions. Retiling on suspend would cause visible layout churn.

**`Suspended` → `Running` (Resumed)**:
- Set `AppState.win_visible = true`.
- Call `apply_tiling_layout()` only if the app's tiling slot was taken by another app
  during suspension (slot-stealing is not currently implemented, so this check is
  always false in R22; the call is a no-op added for forward compatibility).

**`Running` → `Background`**:
- Set `AppState.win_visible = false`.
- Same compositor skip as Suspended.
- Do NOT call `apply_tiling_layout()`.

**`Background` → `Running` (Foreground)**:
- Set `AppState.win_visible = true`.
- Same forward-compatibility layout check as Resumed.

**Any state → `Terminated`**:
- Call `wm::on_app_terminated(app_name)`:
  - Remove from `Z_ORDER`.
  - Call `wm::compact_z_order()`.
  - Call `apply_tiling_layout()` unconditionally (a window has permanently left).
- Set `AppState.win_visible = false`.

### 11.2 Focus Management on Termination

When the focused app terminates:

1. Remove it from the focus queue.
2. Find the topmost app in Z_ORDER that is (a) in Running state and (b) in the active space.
3. Assign keyboard focus to that app.
4. Write `VYOMA_LIFECYCLE:running` to the new focus app's stdin as a focus-gain notification.
   (Reusing the `running` event here is intentional: the app is already running; the event
   signals it has regained focus. A dedicated `VYOMA_LIFECYCLE:focus_gained` event is
   deferred to R23 when chrome and focus rings are introduced.)

If no Running app exists in the active space, focus is unset. The supervisor logs this.

---

## 12. File Layout

The lifecycle subsystem is implemented as a dedicated submodule of the supervisor, split
into focused files respecting the 500-line limit:

```
supervisor/src/lifecycle/
├── mod.rs            # Re-exports; LifecycleState enum; module registration (< 80 lines)
├── state.rs          # LifecycleState definition + update_watchdog_pause helper (< 60 lines)
├── events.rs         # Event delivery: write_lifecycle_event(app, event) (< 120 lines)
├── termination.rs    # graceful_terminate(app), poll_terminating_apps() (< 200 lines)
├── restart.rs        # backoff table, PendingRestart, check_pending_restarts() (< 180 lines)
├── reload.rs         # reload_boot_config(), diff_manifests() (< 250 lines)
└── wit_host.rs       # WIT interface host implementation for Wasmtime (< 200 lines)
```

Total new code budget: ~1090 lines across 7 files, well within the 500-line-per-file limit.

### 12.1 Integration Points in Existing Files

The following existing supervisor source files receive additions:

**`supervisor/src/main.rs`** (event loop):
- Import `lifecycle::termination::poll_terminating_apps` — called on each tick.
- Import `lifecycle::restart::check_pending_restarts` — called on each tick.
- On app exit event: call `lifecycle::restart::on_app_exit(app_name, exit_code)`.

**`supervisor/src/windows.rs`** (WM):
- Add `on_app_terminated(app_name)` function called from `lifecycle::termination`.
- Existing `compact_z_order` and `apply_tiling_layout` are unchanged.

**`supervisor/src/ipc_handlers.rs`** (IPC commands):
- Wire `kill <app>` → `lifecycle::termination::graceful_terminate`.
- Wire `restart <app>` → resets backoff counters + `lifecycle::termination::graceful_terminate`
  + schedules immediate restart (delay = 0).
- Wire `reload` → `lifecycle::reload::reload_boot_config`.

**`supervisor/src/display.rs`** (compositor):
- On `VYOMA_LIFECYCLE:ready` parsed from app stdout: call `lifecycle::events::on_ready(app)`.
- On `VYOMA_LIFECYCLE:background_opt_in` parsed from app stdout: call
  `lifecycle::events::on_background_opt_in(app)`.

---

## 13. Appendix: State Transition Diagram (ASCII)

```
                        ┌─────────────────────────────────┐
                        │          LAUNCHING               │
                        │  (watchdog paused, no input)     │
                        └────────────┬────────────────────-┘
                                     │ ready / first flush / 500ms timeout
                                     ▼
                   ┌─────────────────────────────────────┐
                   │              RUNNING                  │
                   │  (watchdog active, input delivered)   │
                   └──────┬──────────────────────┬────────┘
                          │ space switch away     │ kill issued
              bg policy?  │                       │
           ┌──────────────┴────────┐              │
           │ yes            no     │              ▼
           ▼                ▼      │     ┌─────────────────┐
    ┌────────────┐  ┌──────────┐  │     │  TERMINATING     │
    │ BACKGROUND │  │ SUSPENDED│  │     │  (2s grace,      │
    │ (watchdog) │  │ (paused) │  │     │   watchdog off)  │
    └─────┬──────┘  └────┬─────┘  │     └────────┬────────┘
          │ switch back  │        │              │ exit or SIGKILL
          └──────────────┘        │              ▼
                  │               │     ┌─────────────────┐
                  └───────────────┘     │   TERMINATED     │
                                        │  (process dead,  │
                                        │   WM cleanup)    │
                                        └─────────────────┘
```

---

## 14. Operator Commands — Extended Reference

R22 adds or modifies the following operator commands accessible via IPC:

### 14.1 `kill <app>`

Initiates graceful termination (§4.1). Returns immediately with one of:
- `"killing <app>"` — termination sequence started.
- `"already terminating <app>"` — a previous kill is still in the 2s grace window.
- `"already terminated <app>"` — app has already exited; nothing to do.
- `"unknown app <app>"` — no app with that name in `APPS_MAP`.

**Example**:
```
@supervisor: kill gui-demo
supervisor: killing gui-demo
```

### 14.2 `restart <app>`

Forces an immediate restart regardless of `restart` policy. Resets all backoff counters
and hourly cap counters for this app. Sequence:
1. If app is Running/Suspended/Background: initiate graceful termination (§4.1).
2. After termination: schedule a restart with delay = 0 (bypasses backoff table).
3. Reset `restart_count_window = 0`, `restarts_this_hour = 0`, `hourly_window_start = now`.

Returns `"restarting <app>"` immediately. The actual restart occurs after the termination
grace period (≤2s).

### 14.3 `lifecycle <app>`

New command in R22. Returns the current lifecycle state and relevant metadata:

```
@supervisor: lifecycle gui-demo
supervisor: gui-demo state=Running watchdog_active=true bg_policy=false restarts=2
```

Output format:
```
<app> state=<state> watchdog_active=<bool> bg_policy=<bool> restarts=<n>
```

Where `restarts` is `restart_count_window` (restarts in current backoff window).

### 14.4 `reload`

Described fully in §6. Returns one of:
- `"reload ok: started=N killed=M unchanged=K"` on success.
- `"reload failed: <parse error>"` if boot.toml is unparseable.
- `"reload in progress"` if a concurrent reload is running.

---

## 15. Testing Strategy

### 15.1 Unit Tests

All lifecycle logic is unit-testable without a running Wasmtime process. New test files:

**`supervisor/tests/lifecycle_state.rs`**:
- Verify all valid state transitions are accepted.
- Verify illegal transitions (e.g., Terminated → Running) are rejected or no-op'd.
- Verify watchdog pause/resume logic matches the truth table in §10.

**`supervisor/tests/lifecycle_termination.rs`**:
- Mock app process handle with a FakeProcess that exits after N milliseconds.
- Test: app exits voluntarily within 2s → transitions to Terminated cleanly.
- Test: app does not exit within 2s → SIGKILL issued; transitions to Terminated.
- Test: kill issued when app is already Terminating → returns "already terminating".
- Test: kill issued when app is already Terminated → returns "already terminated".
- Test: `will_terminate` write to broken pipe (EPIPE) → immediate Terminated transition.

**`supervisor/tests/lifecycle_restart.rs`**:
- Test backoff table: first restart = 0s, second = 1s, ..., sixth = 16s (capped).
- Test backoff reset: app alive ≥60s → `restart_count_window` resets to 0.
- Test backoff non-reset: app alive 59s → `restart_count_window` not reset.
- Test hourly cap: 10 restarts → next restart is rejected; app stays Terminated.
- Test `restart <app>` operator command resets hourly cap.

**`supervisor/tests/lifecycle_reload.rs`**:
- Test: valid boot.toml with new app → new app is started.
- Test: valid boot.toml with removed app → removed app gracefully terminated.
- Test: invalid TOML (truncated) → reload aborts, running apps unchanged.
- Test: concurrent reload → second `reload` returns "reload in progress".
- Test: changed manifest (capability added) → app restarted with new manifest.
- Test: changed version only (wasm path unchanged) → app NOT restarted.

### 15.2 Integration Points with Existing Tests

`supervisor/tests/display.rs` (R17 tests) must be updated to handle the fact that
`VYOMA_LIFECYCLE:ready` is now parsed from app stdout before `VYOMA_DRAW:` commands can
be processed. Test apps that write `VYOMA_DRAW:` must first write
`VYOMA_LIFECYCLE:ready` or wait for the 500ms fallback.

### 15.3 Smoke Test Extensions

The smoke test (`make smoke`) verifies `[lifecycle] all apps spawned` on serial output.
R22 extends this to also verify no app stays in `Launching` state beyond 5 seconds:
```
[lifecycle] all apps running within 2s
```
This log line is emitted by the supervisor once all apps listed in boot.toml have
transitioned from `Launching` to either `Running`, `Suspended`, `Background`, or
`Terminated` (one-shot apps may terminate quickly).

---

## 16. Forward Compatibility Notes

### 16.1 R23 (Window Chrome)

R23 will introduce window chrome (title bar with close/minimise/maximise buttons). The
close button maps to `kill <app>` → `graceful_terminate`. R23's chrome module calls
`lifecycle::termination::graceful_terminate` directly; no API change needed.

R23 will also introduce `VYOMA_LIFECYCLE:focus_gained` and `VYOMA_LIFECYCLE:focus_lost`
events. These are not in scope for R22 but the event delivery infrastructure in
`lifecycle/events.rs` is designed to accommodate additional event strings trivially.

### 16.2 R28 (Zero-Downtime Reload)

R28 tentatively introduces a zero-downtime hot-reload for daemon apps. The flow:
1. Start new instance of the app (parallel to the old one).
2. Wait for new instance to emit `VYOMA_LIFECYCLE:ready`.
3. Route new IPC traffic to the new instance; drain remaining IPC to old instance.
4. Gracefully terminate old instance.

R22's lifecycle infrastructure is compatible with this model: `notify-ready` / `ready`
event already triggers the `Launching → Running` transition, which R28 can listen to.
The `reload` command in R22 does NOT implement zero-downtime; it is a simpler sequential
kill-then-start. R28 will add a `hot_reload = true` flag to the manifest.

### 16.3 R25 (Process Inspector)

R25 will add a visual process inspector app (similar to macOS Activity Monitor) that
displays lifecycle state for all running apps. It will use the `lifecycle <app>` IPC
command (§14.3) to poll state. The `LifecycleState` enum is already `Debug`-derived,
making it trivially serialisable for display.

---

## 17. Summary of Invariants

| Invariant | Description |
|-----------|-------------|
| I1 | Every app has exactly one LifecycleState at all times. |
| I2 | Watchdog timer is active iff state ∈ {Running, Background}. |
| I3 | Keyboard and mouse events are delivered iff state == Running. |
| I4 | IPC messages are delivered iff state ∈ {Running, Suspended, Background}. |
| I5 | The 2-second termination wait never blocks the supervisor event loop. |
| I6 | restart=always apps are never restarted more than 10 times per hour. |
| I7 | Backoff is measured from spawn_time, not last_output_time. |
| I8 | reload aborts cleanly on TOML parse error with no state change. |
| I9 | Post-termination WM cleanup always runs, even after SIGKILL. |
| I10 | Lifecycle events are always written to stdin before subsequent input events. |
| I11 | Background apps may not accumulate draw commands; draw-only-when-visible enforced. |
| I12 | The reload busy-flag prevents concurrent reloads from interleaving state changes. |
