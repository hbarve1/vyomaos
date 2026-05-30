# FINAL Spec: App Lifecycle Management (Round 22)

**Subsystem**: App Lifecycle Management  
**macOS Analogue**: `NSApplicationDelegate`, `UIApplication`, app launch/quit/background/foreground  
**Depends on**: R11 (AppState, supervisor), R21 (WM space/tiling integration)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

App Lifecycle Management formalizes the states a WASM app passes through from spawn to
death, defines graceful termination with a 2-second grace period, adds restart backoff
logic, and introduces a `reload` command for zero-kill hot manifest updates.

---

## 2. App States

```rust
// supervisor/src/lifecycle/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    Launching,     // Wasmtime spawned; waiting for ready signal or first flush
    Running,       // Active and producing output
    Background,    // In non-active space; opted into background policy; still receives IPC
    Suspended,     // In non-active space OR minimized; receives no input, no IPC
    Terminating,   // will_terminate sent; 2s grace period active
    Terminated,    // Process exited or SIGKILL issued
}
```

Added to `AppState`:
```rust
pub lifecycle: LifecycleState,
pub background_policy: bool,       // set via set-background-policy WIT or stdout
pub termination_deadline: Option<Instant>,   // set when Terminating begins
pub spawn_time: Option<Instant>,    // set when Wasmtime process spawned
pub restart_count_window: u8,       // restarts within current 60-second window
pub next_restart_at: Option<Instant>,
pub restart_total: u32,             // lifetime restart count
```

---

## 3. Complete State Transition Table (B1 fix)

All valid transitions are listed. Unhandled combinations are explicitly noted as no-ops.

| Current State | Event | Next State | Action |
|--------------|-------|-----------|--------|
| — (not spawned) | Boot / `spawn` | Launching | Wasmtime starts; `spawn_time = now` |
| Launching | First `VYOMA_DRAW:flush` OR `VYOMA_LIFECYCLE:ready` OR WIT `notify-ready` | Running | `win_visible = true`; broadcast `running` event |
| Launching | `kill` issued | Terminating | Write `will_terminate` optimistically; start 2s timer |
| Running | Window minimized (`minimize`) | Suspended | `win_visible = false`; send `suspended` event; pause watchdog |
| Running | Space → inactive; no background policy | Suspended | `win_visible = false`; send `suspended` event; pause watchdog |
| Running | Space → inactive; background policy set | Background | `win_visible = false`; send `background` event; watchdog active |
| Running | `kill` issued | Terminating | Send `will_terminate`; start 2s timer; stop IPC delivery |
| Suspended | Window restored (`restore`) | Running | `win_visible = true`; send `resumed` event; resume watchdog |
| Suspended | Space → active | Running | `win_visible = true`; send `foreground` event; resume watchdog |
| Suspended | `set-background-policy(true)` | Background | Send `background` event; resume watchdog |
| Suspended | `kill` issued | Terminating | Send `will_terminate`; start 2s timer |
| Background | Space → active | Running | Send `foreground` event |
| Background | `set-background-policy(false)` | Suspended | Send `suspended` event; pause watchdog |
| Background | `kill` issued | Terminating | Send `will_terminate`; start 2s timer; stop IPC delivery |
| Terminating | Process exits voluntarily | Terminated | Cancel 2s timer; run post-exit cleanup |
| Terminating | 2s timer expires | Terminated | SIGKILL Wasmtime process; run post-exit cleanup |
| Terminating | Any event (IPC, flush, etc.) | Terminating | **No-op** — all events discarded in Terminating state |
| Terminated | `restart = "always"` policy | Launching | After backoff delay; `spawn_time = now` |
| Terminated | `restart = "never"` policy | Terminated | Permanent |

### 3.1 IPC Delivery in Terminating State

When an app transitions to `Terminating`, the IPC broker atomically stops enqueuing new
messages for that app. Messages already queued at the moment of state flip are dropped.
`will_terminate` is the LAST write to stdin. The stdin pipe remains open so the app can
receive the `will_terminate` event, but nothing else is written after it.

```rust
// supervisor/src/ipc_handlers.rs

fn handle_kill(app_name: &str, apps: &mut AppsMap) {
    let Some(app) = apps.get_mut(app_name) else { return; };
    match app.lifecycle {
        Terminating | Terminated => return,  // already dying
        _ => {}
    }
    // Stop IPC delivery FIRST, before writing will_terminate
    app.ipc_delivery_stopped = true;
    app.lifecycle = LifecycleState::Terminating;
    app.termination_deadline = Some(Instant::now() + Duration::from_secs(2));
    // Write will_terminate as last stdin event
    let _ = app.stdin_tx.send("VYOMA_LIFECYCLE:will_terminate\n".to_string());
}
```

---

## 4. `Launching → Running` Transition (B5 fix)

### 4.1 Two Trigger Paths

Both paths call the same state machine function under `APPS_MAP` write lock:

```rust
// supervisor/src/lifecycle/events.rs

pub fn on_app_ready(app_name: &str, apps: &mut AppsMap) {
    let Some(app) = apps.get_mut(app_name) else { return; };
    if app.lifecycle != LifecycleState::Launching {
        return;  // No-op: already Running or further along — idempotent
    }
    app.lifecycle = LifecycleState::Running;
    app.win_visible = true;
    let _ = app.stdin_tx.send("VYOMA_LIFECYCLE:running\n".to_string());
    // Trigger WM tiling recompute
    wm::apply_tiling_layout(apps_ref, &config, spaces.active);
}
```

**Path 1: stdout `VYOMA_LIFECYCLE:ready`** — output-reading thread calls
`on_app_ready(app_name, &mut apps_map.write())`.

**Path 2: WIT `notify-ready`** — Wasmtime thread calls
`on_app_ready(app_name, &mut apps_map.write())`.

**Path 3: First `VYOMA_DRAW:flush`** — `draw_cmd.rs` calls
`on_app_ready(app_name, &mut apps_map.write())` if lifecycle is `Launching`.

All three paths take the `APPS_MAP` write lock and are no-ops when state ≠ Launching.
The `Launching → Running` transition is therefore idempotent regardless of which path
fires first. No race is possible.

### 4.2 Display-less Apps

For apps with `display = false`, only path 1 or path 2 applies (no flush). Daemon apps
SHOULD write `VYOMA_LIFECYCLE:ready` to stdout when initialization is complete. If they
never write it, they stay in `Launching` indefinitely (watchdog applies at normal interval
since watchdog is active during `Launching`).

---

## 5. Graceful Termination

```rust
// supervisor/src/lifecycle/termination.rs

/// Called on each supervisor event loop tick.
pub fn check_termination_deadlines(apps: &mut AppsMap, now: Instant) {
    let expired: Vec<String> = apps.values()
        .filter(|a| a.lifecycle == LifecycleState::Terminating)
        .filter(|a| a.termination_deadline.map(|d| now >= d).unwrap_or(false))
        .map(|a| a.name.clone())
        .collect();

    for name in expired {
        force_kill(&name, apps);
    }
}

fn force_kill(app_name: &str, apps: &mut AppsMap) {
    if let Some(app) = apps.get_mut(app_name) {
        app.lifecycle = LifecycleState::Terminated;
        // SIGKILL the Wasmtime child process
        if let Some(pid) = app.wasmtime_pid {
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        }
        post_exit_cleanup(app_name, apps);
    }
}
```

`post_exit_cleanup`:
1. Call `wm::compact_z_order` for the app's space
2. Call `wm::apply_tiling_layout` for the app's space
3. Remove app from `CaptureEngine::permissions` (R18)
4. Close any `RecordingSession`s owned by the app (R18 cleanup)
5. If `restart = "always"`, schedule restart (section 6)

---

## 6. Watchdog Integration (B2 fix — dual-check)

### 6.1 Watchdog Pause States

The watchdog is **paused** (skip kill) when lifecycle is `Suspended`, `Terminating`, or
`Launching`. It is **active** in `Running` and `Background`.

### 6.2 In-Thread State Re-Check

The watchdog runs in each app's output-reading thread. Before firing, it re-reads the
lifecycle state under `APPS_MAP` read lock — this closes the race between the event loop
thread setting `watchdog_paused` and the output-reading thread reading it:

```rust
// supervisor/src/lifecycle/watchdog.rs — inside output-reading thread

fn check_watchdog(app_name: &str, apps: &Arc<RwLock<AppsMap>>, last_output_us: u64, now_us: u64) {
    let (watchdog_secs, lifecycle) = {
        let apps_r = apps.read();
        let app = apps_r.get(app_name).map(|a| (a.watchdog_secs, a.lifecycle));
        app.unwrap_or((0, LifecycleState::Terminated))
    };

    if watchdog_secs == 0 { return; }

    // Dual-check: skip watchdog for states where silence is expected
    match lifecycle {
        LifecycleState::Suspended | LifecycleState::Terminating | LifecycleState::Launching => return,
        _ => {}
    }

    let silence_us = now_us.saturating_sub(last_output_us);
    if silence_us >= watchdog_secs as u64 * 1_000_000 {
        // Watchdog fires — app has been silent too long in Running/Background state
        let mut apps_w = apps.write();
        handle_kill(app_name, &mut apps_w);
    }
}
```

This in-thread check is the primary guard. The `watchdog_paused` field on `AppState` is
removed — lifecycle state itself is the guard. Single source of truth.

---

## 7. Restart Backoff

```rust
// supervisor/src/lifecycle/restart.rs

const MAX_RESTARTS_PER_HOUR: u32 = 10;
const BACKOFF_WINDOW_SECS: u64   = 60;
const MAX_BACKOFF_SECS: u64      = 16;

pub fn schedule_restart(app: &mut AppState, now: Instant) {
    if app.restart_total >= MAX_RESTARTS_PER_HOUR {
        log::error!("[lifecycle] {} exceeded restart limit; staying Terminated", app.name);
        return;
    }
    let delay = match app.restart_count_window {
        0 => Duration::ZERO,
        1 => Duration::from_secs(1),
        2 => Duration::from_secs(2),
        3 => Duration::from_secs(4),
        4 => Duration::from_secs(8),
        _ => Duration::from_secs(MAX_BACKOFF_SECS),
    };
    app.restart_count_window += 1;
    app.restart_total += 1;
    app.next_restart_at = Some(now + delay);
}

/// Called on EVERY supervisor event loop tick — NOT only on exit events (B4 fix).
pub fn check_backoff_resets(apps: &mut AppsMap, now: Instant) {
    for app in apps.values_mut() {
        if app.restart_count_window == 0 { continue; }
        // Guard: skip apps that haven't been spawned yet
        let Some(spawn_time) = app.spawn_time else { continue; };
        if now.duration_since(spawn_time) >= Duration::from_secs(BACKOFF_WINDOW_SECS) {
            app.restart_count_window = 0;
            app.next_restart_at = None;
        }
    }
}

pub fn check_pending_restarts(apps: &mut AppsMap, now: Instant) {
    let to_restart: Vec<String> = apps.values()
        .filter(|a| a.lifecycle == LifecycleState::Terminated)
        .filter(|a| a.next_restart_at.map(|t| now >= t).unwrap_or(false))
        .map(|a| a.name.clone())
        .collect();
    for name in to_restart {
        spawn_app(&name, apps);
    }
}
```

**Nominal crash-reset-crash trace**:
- t=0: App crashes → `restart_count_window=1`, delay=1s, `next_restart_at=t+1s`
- t=1s: Restart → `spawn_time=t+1s`, `lifecycle=Launching`
- t=61s (app alive 60s): `check_backoff_resets` → `restart_count_window=0`
- t=62s: App crashes → `restart_count_window=1`, delay=1s (treated as fresh first crash)

`check_backoff_resets` and `check_pending_restarts` are called from the main event loop
tick function unconditionally — not from exit handlers.

---

## 8. `reload` Command (B3 fix — all-or-nothing validation)

### 8.1 Reload Sequence

```
Step 1: Parse boot.toml → abort on TOML parse error (no state change)
Step 2: For each app listed in new boot.toml, parse its vyoma.toml
         → If ANY manifest fails to parse: abort reload entirely (Policy A)
         → Return error listing which manifest failed
Step 3: Diff new app set vs running app set
         - Added apps: apps in new set but not currently running
         - Removed apps: apps currently running but not in new set
         - Changed apps: apps in both sets but with different manifest fields
Step 4: Gracefully terminate all removed apps (send will_terminate; wait up to 5s; SIGKILL)
Step 5: Restart changed apps (graceful terminate → respawn with new manifest)
Step 6: Spawn added apps
```

**All-or-nothing at Step 2**: if any vyoma.toml in the new boot.toml fails to parse,
the entire reload aborts before any app is killed. This is invariant I8.

**Known limitation**: if a removed app's Wasmtime process survives SIGKILL (extremely
rare), the new instance may fail to start if it requires an exclusive resource. Log error.
This is an OS-level limitation; no mitigation in R22.

### 8.2 "Changed" Definition

An app is considered changed if any field in `[app]` or `[capabilities]` differs between
old and new manifest. Hash-compare the manifest structs; no partial-field diffing.

---

## 9. WIT Interface `vyoma:lifecycle@1.0.0`

```wit
package vyoma:lifecycle@1.0.0;

enum lifecycle-state {
    launching, running, background, suspended, terminating, terminated,
}

interface lifecycle {
    app-state: func() -> result<lifecycle-state, string>;

    /// Signal that the app has finished launching.
    /// No-op if already in Running or later state.
    notify-ready: func() -> result<_, string>;

    /// Opt into Background state for inactive spaces.
    /// If called while Suspended, transitions immediately to Background.
    set-background-policy: func(enabled: bool) -> result<_, string>;

    /// Request voluntary termination of this app.
    request-terminate: func() -> result<_, string>;
}

world lifecycle-world {
    import lifecycle;
}
```

---

## 10. VYOMA_LIFECYCLE: Stdout Protocol

```
VYOMA_LIFECYCLE:ready              → supervisor transitions app Launching → Running
VYOMA_LIFECYCLE:background_opt_in  → set background_policy = true
VYOMA_LIFECYCLE:background_opt_out → set background_policy = false
VYOMA_LIFECYCLE:quit               → graceful self-termination request
```

Push notifications (supervisor → app stdin):
```
VYOMA_LIFECYCLE:launching          ← at spawn (for logging/debugging)
VYOMA_LIFECYCLE:running            ← after Launching → Running
VYOMA_LIFECYCLE:suspended          ← app is now suspended
VYOMA_LIFECYCLE:resumed            ← Suspended → Running
VYOMA_LIFECYCLE:background         ← Running/Suspended → Background
VYOMA_LIFECYCLE:foreground         ← Background → Running
VYOMA_LIFECYCLE:will_terminate     ← 2s warning before kill (LAST stdin write)
```

---

## 11. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Background state | yes | yes | yes | no |
| Suspended state | yes | yes | no | no |
| Spaces-driven transitions | yes | no | no | no |
| Graceful termination | yes | yes | yes | yes |
| Restart backoff | yes | yes | yes | yes |
| `reload` command | yes | no | yes | no |

---

## 12. Event Loop Integration

These functions are called on EVERY supervisor event loop tick (unconditionally):

```rust
// supervisor/src/main.rs — main event loop tick

fn supervisor_tick(state: &mut SupervisorState, now: Instant) {
    // ... existing event processing ...

    // Lifecycle tick functions (B4 fix: unconditional)
    lifecycle::restart::check_backoff_resets(&mut state.apps, now);
    lifecycle::restart::check_pending_restarts(&mut state.apps, now);
    lifecycle::termination::check_termination_deadlines(&mut state.apps, now);
}
```

---

## 13. File Layout

```
supervisor/src/lifecycle/
├── mod.rs           (LifecycleState, on_app_ready, state machine)
├── termination.rs   (check_termination_deadlines, force_kill, post_exit_cleanup)
├── restart.rs       (schedule_restart, check_backoff_resets, check_pending_restarts)
├── watchdog.rs      (check_watchdog with dual state re-check)
├── reload.rs        (reload command handler, all-or-nothing validation)
└── events.rs        (push notification helpers)

supervisor/src/app_state.rs   (new lifecycle fields)
supervisor/src/main.rs        (tick integration)
supervisor/src/wit_handlers.rs (lifecycle WIT closures)
```

---

## 14. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Incomplete transition graph | Complete table with all 16 transitions; Launching→Terminating, Suspended↔Background, IPC-stop-on-Terminating all defined |
| B2: Watchdog double-kill race | Dual-check in output-reading thread: re-read lifecycle state under read-lock before firing; `Suspended/Terminating/Launching` states cause skip regardless of `watchdog_paused` field (removed) |
| B3: Reload partial failure | Policy A: validate ALL manifests before killing any app; any parse failure → abort entire reload; returns error listing failed manifests |
| B4: Backoff reset not evaluated | `check_backoff_resets` explicitly added to main event loop tick (unconditional); `spawn_time = None` guard present; nominal crash-reset-crash trace documented |
| B5: Dual ready-trigger race | Single `on_app_ready` function under APPS_MAP write lock; all three trigger paths (stdout ready, WIT notify-ready, first flush) are no-ops when state ≠ Launching; idempotent by design |
