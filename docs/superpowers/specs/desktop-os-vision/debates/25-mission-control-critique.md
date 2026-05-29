# CRITIQUE — 5 Blocking Issues
# Round 25: Mission Control & Exposé

**Reviewer**: Architect/Critic combined agent  
**Spec**: `25-mission-control.md`  
**Status**: CRITIQUE — blocks FINAL spec

---

## B1: Thumbnail Filesystem Capability Grants Overly-Broad `/data` Access

### Problem

The spec delivers thumbnails to the mission-control WASM app via
`/tmp/mc_thumbnails/<app_name>.raw` files (Section 4.2). This requires the
`mission-control` app to have `filesystem = true` in its `vyoma.toml`. However,
per the established capability model (CLAUDE.md), `filesystem = true` mounts the
persistent `/data` host directory — not just `/tmp`. This gives `mission-control`
read/write access to all persistent user data on the host. That is an unacceptable
capability grant for a UI overlay app.

Furthermore, `/tmp` is an **in-initramfs path** (not the 9P-mounted `/data`). The
`filesystem` capability only controls the `/data` 9P mount — there is no separate
capability for `/tmp`. This means:
- If `filesystem = false`, `/tmp/mc_thumbnails/` is inaccessible (the directory is
  not writeable by the supervisor and not readable by the WASM app via WASI).
- If `filesystem = true`, the app gets the whole `/data` mount, which is excessive.

A WASM app cannot read supervisor-written files unless either (a) the file is on a
mounted filesystem the app has WASI access to, or (b) the data is delivered via stdin.
Neither path is correctly specified.

### Proposed Fix

Remove the filesystem-based thumbnail delivery entirely. Instead, use the existing
**stdin delivery channel**: the supervisor serializes thumbnail pixel data as a
base64-encoded block delivered on the mission-control app's stdin, framed with a new
`VYOMA_MC:` protocol message:

```
VYOMA_MC:thumb_begin:<app_name>,<w_px>,<h_px>
VYOMA_MC:thumb_data:<base64_chunk>   (repeated, 64 chars/line)
VYOMA_MC:thumb_end
```

The MC app accumulates chunks into an in-memory buffer. This requires no `filesystem`
capability and no `/tmp` path — only `stdio = true`. The `draw_image` command (Section 13)
is then replaced by a simpler `draw_thumb:<thumb_id>,<x>,<y>,<w>,<h>` command where
`thumb_id` is an integer the MC app tracks in-memory after deserializing the stdin stream.

The supervisor's `draw_image` command (filesystem I/O in compositor loop, Section 13)
is also eliminated — avoiding the I/O latency problem identified in the spec's own
pre-critique (Section 22.3).

---

## B2: `mc_input_locked` and `chrome_input_locked` Can Be Simultaneously True — No Priority Order Defined

### Problem

The TTY input routing in Section 2.1 checks `mc_input_locked` first, then
`chrome_input_locked`. However, `chrome_input_locked` is set by the chrome app when it
displays a consent dialog (R23, Section 7). There is no mechanism that prevents the
following sequence:

1. User presses F3 → MC opens → `mc_input_locked = true`
2. A screen-capture consent dialog triggers → `chrome_input_locked = true`
3. Both flags are now `true`

The TTY routing code as written routes all keys to `mission-control` when both are true
(because `mc_input_locked` is checked first). This means:
- The consent dialog is visible (chrome draws it) but receives no keyboard input
- The user cannot accept or reject the consent dialog
- The consent WIT call blocks indefinitely (waiting for chrome's response that can never arrive)
- The system is soft-deadlocked: MC cannot exit via Escape (it could be handled, but only
  if the MC app sends `mc_exit` via its own logic, which it has no reason to do)

This is an undefined interaction between two independently-gating input locks.

### Proposed Fix

Introduce a **lock priority stack** in `SupervisorState`. Rather than two independent
`AtomicBool` flags, replace with a single `Arc<AtomicU8>` encoding priority levels:

```rust
// supervisor/src/input.rs
pub enum InputLockLevel {
    None          = 0,
    MissionControl = 1,
    ChromeConsent  = 2,  // consent always wins over MC
}

pub static INPUT_LOCK_LEVEL: AtomicU8 = AtomicU8::new(0);
```

`ChromeConsent` (level 2) always overrides `MissionControl` (level 1). TTY routing reads
the single atomic value and dispatches accordingly:

```rust
fn route_keyboard(ev: KeyEvent) {
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        2 => send_to_app("chrome", ev),
        1 => send_to_app("mission-control", ev),
        _ => send_to_focused_app(&focused_app.load(), ev),
    }
}
```

When a consent dialog opens, the supervisor sets `INPUT_LOCK_LEVEL = 2` regardless of MC
state. When the consent dialog closes, it restores to the previous level (1 if MC was open,
0 otherwise). The supervisor keeps a `prev_lock_level: u8` field set before raising to
level 2.

Auto-clear rules: on any privileged app's lifecycle transition away from Running, reset
the lock level to `None` (same as the existing per-flag auto-clear pattern in R23/R24).

---

## B3: Compositor DRM Blit Step Races with Thumbnail Capture's `vsync_lock.read()`

### Problem

Section 4.3 describes the thumbnail capture sequence. Step 3 acquires `vsync_lock.read()`
per-app for a memcopy. However, the compositor's DRM blit step (Step 4 in R21 Section 10)
acquires `vsync_lock.write()`. The capture is described as running "after DRM blit (while
vsync_lock is not held)" — but this is executed as a response to an IPC command
(`@supervisor: mc_capture_request`), which arrives on the main event loop. There is no
guarantee that the capture loop completes before the next compositor tick starts.

The concrete race:
1. Event loop processes `mc_capture_request`, starts capture loop (acquires `vsync_lock.read()`)
2. Compositor thread attempts `vsync_lock.write()` for DRM blit → **blocks**
3. If the surface loop iterates slowly (many apps, large surfaces), the compositor is
   stalled for multiple frames, causing visible frame drops / jank

Additionally, the spec says thumbnails are written per-app with the read-lock released
between writes: "releasing the lock between writes to avoid long read-lock holds." But
`RwLock` in Rust's std (and most implementations) does not guarantee that a pending
write-lock acquisition starves between successive read-lock acquisitions. If the event
loop re-acquires `vsync_lock.read()` immediately for the next app while the compositor
thread is waiting for write access, the compositor thread may be stalled indefinitely
(writer starvation in a reader-favoring `RwLock`).

### Proposed Fix

Move thumbnail capture entirely **off the main event loop** into a dedicated
**capture thread**. Add a `CaptureRequest` channel:

```rust
// supervisor/src/mc_capture.rs

pub struct CaptureRequest {
    pub all_spaces: bool,
    pub mc_tx: Sender<String>,
}

/// Spawned once at supervisor startup, sleeps on receiver
pub fn capture_thread_main(
    req_rx: Receiver<CaptureRequest>,
    apps: Arc<RwLock<AppsMap>>,
    vsync_lock: Arc<RwLock<()>>,
    config: DisplayConfig,
) {
    for req in req_rx {
        // Acquire a single long read-lock snapshot (not per-app)
        let snapshot = {
            let _r = vsync_lock.read();
            snapshot_all_surfaces_locked(&apps.read(), req.all_spaces, &config)
        };
        // Release vsync_lock BEFORE writing files (I/O outside lock)
        let count = write_thumbnails_to_memory(snapshot, &config);
        let _ = req.mc_tx.send(format!("VYOMA_MC:windows_ready:{}\n", count));
    }
}
```

Key properties:
- The `vsync_lock.read()` is held for exactly one atomic snapshot of all surfaces — not
  released and re-acquired per-app (eliminates writer starvation window).
- The snapshot is a `Vec<(String, Vec<u8>, u32, u32)>` (app_name, pixel_data, w, h) taken
  in one pass. The lock is released before any I/O.
- Capture runs on its own thread; the main event loop posts to `req_tx` and returns
  immediately. No stall. If a second `mc_capture_request` arrives before the first
  completes, it is silently dropped (MC app will see no second `windows_ready` — an
  acceptable constraint noted in spec comments).

---

## B4: Space Strip Space-Thumbnail Capture Deadlocks on Non-Active Spaces With Apps in `Suspended` State

### Problem

Section 6.3 defines `mc_capture_request_all_spaces` which captures surfaces from all
spaces, including non-active spaces where apps are `Suspended`. The problem is that
`Suspended` apps are in the "receives no input, no IPC" state (R22) and have stopped
updating their surface. Their surface data may be:

1. **Partially rendered**: if the app was suspended mid-draw (before a `flush` command
   committed the frame), the surface contains an incomplete frame
2. **Stale**: the surface contains the last committed frame, which may be from seconds
   or minutes ago

More critically, the spec does not define the surface ownership contract for Suspended
apps. Specifically: when an app is `Suspended`, does the supervisor continue to allow
surface writes from the app's output-reading thread? If yes, there is a race between:
- The MC capture thread reading the surface under `vsync_lock.read()`
- The app's output-reading thread writing a pending `VYOMA_DRAW:` command into the surface

The app's output-reading thread does NOT hold `vsync_lock` while parsing draw commands —
it only acquires it when processing a `flush` (to blit into compositor). If `Suspended`
apps can still have pending draw commands in their output buffer being processed, the
surface state during capture is non-deterministic.

Furthermore, the R22 spec says `Suspended` apps "receive no input" but does NOT say they
stop producing output. An app suspended mid-output-stream may continue writing
`VYOMA_DRAW:` commands that the supervisor processes — mutating the surface outside of
`vsync_lock`.

### Proposed Fix

Define a clear **surface quiescence invariant** for Suspended apps:

> When an app transitions to `Suspended`, the supervisor drains all pending `VYOMA_DRAW:`
> commands from the app's current output-reading buffer **before** transitioning the
> lifecycle state. After the transition, the app's surface is "quiesced" — no further
> surface mutations occur until the app transitions back to `Running`.

Implementation:
- In `lifecycle::events::on_app_suspended`, after setting `lifecycle = Suspended`:
  - Set `app.surface_quiesced = true` (new boolean flag on AppState)
  - The draw_cmd handler checks this flag: if `surface_quiesced = true`, all `VYOMA_DRAW:`
    commands from this app are discarded (not applied to the surface)
  - On `Running` (resume): clear `surface_quiesced = false` before resuming normal draw processing

This guarantees that during MC capture, any `Suspended` app's surface is stable (no
concurrent mutations). The capture loop can safely read it under `vsync_lock.read()`.

A secondary fix: for the space-strip thumbnail specifically, use the **last committed
frame** (post-flush state) as the thumbnail source. Track `last_flushed_snapshot:
Option<Arc<Vec<u8>>>` on AppState — a reference-counted copy of the surface pixel data
taken at each successful `flush`. The MC capture reads `last_flushed_snapshot` rather
than live surface data. This is safe without `vsync_lock` and avoids the Suspended-app
mutation race entirely for the common case.

---

## B5: Mission Control Exit While Space Switch Is Pending Corrupts Focus State

### Problem

Section 9.2 (Exit Transition) and Section 11.1 (Window Selection) describe the MC exit
sequence as:
1. MC sends `@supervisor: focus <selected_app>`
2. MC sends `@supervisor: space <N>` (if different space)
3. MC runs exit animation (~200ms, 5 frames)
4. MC collapses surface
5. MC sends `@supervisor: mc_exit`

Between steps 2 and 5, the supervisor sets `PENDING_SPACE_SWITCH = N` (AtomicU8). The
compositor applies the space switch at the start of the next tick (R21 Section 5.3).
The compositor tick interval is typically ~16ms at 60fps. The MC exit animation takes
~200ms.

During the 200ms animation:
- `PENDING_SPACE_SWITCH` is set but not yet applied (will be applied on next compositor tick)
- MC sends `focus <selected_app>` **before** the space switch is applied
- The focus command is processed by the WM focus handler, which calls `compact_z_order`
  for `spaces.active` (the OLD active space, not the target space N)

When the compositor tick applies the space switch:
- `compact_z_order` is called for space N
- The `selected_app` may have a different z-index in space N than what was set by the
  focus handler (which compacted space `old_active`)

Result: the selected app may not be brought to front in the new space. Additionally,
if the selected app is in space N and the focus command ran on space `old_active`,
the `win_focused = true` flag is set on an app in the wrong space. The next tiling
recompute may place the wrong app at the front.

**Concrete example**:
- Active space = 1, target space = 2
- MC sends `focus app-b` (in space 2), then `space 2`
- Focus handler: compact_z_order(space=1) → app-b not in space 1, z unchanged in space 2
- Compositor tick: PENDING_SPACE_SWITCH applies, compact_z_order(space=2) → app-b gets
  z relative to other space-2 apps' current z, not boosted to front
- app-b is NOT at front of space 2

### Proposed Fix

Reverse the ordering: the MC app must send `space <N>` **before** `focus <selected_app>`.
The supervisor's `handle_mc_exit` (or the focus command handler) must defer the focus
change until after the space switch is applied.

Implement a **deferred focus** mechanism: add `pending_focus: Option<String>` to
`SupervisorState`. When the focus command arrives while `PENDING_SPACE_SWITCH != 0`,
store it in `pending_focus` instead of applying immediately. In the compositor tick, after
applying `PENDING_SPACE_SWITCH` and `compact_z_order`, check and apply `pending_focus`:

```rust
// supervisor/src/compositor.rs — in vsync_tick(), after space switch block

if let Some(focus_name) = state.pending_focus.take() {
    wm::focus::handle_focus(&focus_name, &mut state.apps, spaces.active);
    wm::zorder::compact_z_order(&mut state.apps, spaces.active);
}
```

The focus handler is thus always applied in the compositor thread's context, under the
correct `spaces.active` value, after the space switch has been committed. This eliminates
the race between space switch and focus change ordering.

Additionally, the MC app's exit sequence is simplified: it sends `space <N>` and
`focus <app>` in any order, then `mc_exit`. The supervisor buffers the focus change
in `pending_focus` and applies it atomically after the space switch. If no space change
is pending, `pending_focus` is applied immediately on the next compositor tick (no delay).
