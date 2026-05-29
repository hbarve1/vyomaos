# FINAL Spec: Focus Management & Z-order (Round 28)

**Subsystem**: Focus Management & Z-order  
**macOS Analogue**: `NSApplication.keyWindow` / `makeKeyAndOrderFront`  
**Depends on**: R21 (PerSpaceZ, compact_z_order, apps-map lock), R22 (lifecycle), R25 (pending_focus, INPUT_LOCK_LEVEL), R26 (Stage Manager), R27 (FsRegistry, SplitView)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Focus state determines which app receives keyboard input. Z-order determines compositor
blit sequence within each space. The two are linked: focusing an app brings it to the top
of the z-order for its space. All focus transitions funnel through one canonical function:
`focus_transfer`.

---

## 2. Focus State Model

### 2.1 `FOCUSED_APP` — Wait-Free Reads

```rust
// supervisor/src/focus/state.rs

use arc_swap::ArcSwap;

/// App **name** string (not integer ID — see B5 fix).
/// Wait-free reads on TTY input thread and WIT handlers.
pub static FOCUSED_APP: Lazy<Arc<ArcSwap<String>>> =
    Lazy::new(|| Arc::new(ArcSwap::from_pointee(String::new())));
```

All focus notifications and `INPUT_LOCK_LEVEL`-bypass routing use `FOCUSED_APP.load()`.
Name is the manifest `app.name` — stable across restarts.

### 2.2 `last_focused` — Embedded in `SpaceState` (B3 Fix)

No standalone `Mutex<HashMap>`. Per-space last-focused tracking lives inside `SpaceState`
under the existing apps-map lock:

```rust
// supervisor/src/wm/spaces.rs

pub struct SpaceState {
    pub last_focused: Option<String>,  // most recently focused app in this space
    pub is_fs_space: bool,
}

pub struct SupervisorState {
    pub spaces: HashMap<u8, SpaceState>,
    // ...
}
```

Reads/writes to `last_focused` happen under the apps-map lock — no new lock introduced,
no new ordering constraint. This eliminates the ABBA risk from a standalone
`Lazy<Mutex<HashMap>>` static.

### 2.3 AppState Additions

```rust
// supervisor/src/display.rs — AppState additions

pub win_focused: bool,
pub focus_ring_active: bool,
pub last_focus_time: Instant,
```

---

## 3. `focus_transfer` — Canonical Focus-Change Function

### 3.1 `FocusReason`

```rust
#[derive(Clone, Copy, Debug)]
pub enum FocusReason {
    UserClick,          // R32 mouse input
    DockSelect,         // dock icon click
    AppSwitch,          // Ctrl-Tab switcher (R24)
    MissionControl,     // MC window selection (R25)
    StageActivation,    // Stage Manager swap (R26)
    FsEnter,            // FullScreen enter (R27)
    FsExit,             // FullScreen exit (R27)
    SplitViewActivate,
    SplitViewToggle,
    SpaceSwitchRestore, // focus restored on space switch
    AppExit,            // MRU fallback on focused-app exit
    Programmatic,       // app-initiated via WIT
}
```

### 3.2 `focus_transfer` Implementation

Called with apps-map lock held. Does NOT call notify_chrome/notify_dock under lock (B1 fix).
Does NOT call compact_z_order under lock (B2 fix).

```rust
// supervisor/src/focus/transfer.rs

pub fn focus_transfer(
    new_focus: &str,
    old_focus: &str,
    apps: &mut HashMap<String, AppState>,
    spaces: &mut HashMap<u8, SpaceState>,
    active_space: u8,
    reason: FocusReason,
    pending_compact: &AtomicU8,         // B2: deferred compact
    pending_notifs: &mut Vec<PendingFocusNotif>,  // B1: deferred notifications
) {
    if new_focus == old_focus { return; }

    // Step 1: Z-bump (O(N) scan — lightweight; compact deferred)
    if let Some(app) = apps.get(new_focus) {
        let space = app.win_space;
        if space != 0 {
            let max_z = apps.values()
                .filter(|a| a.win_space == space)
                .map(|a| a.win_z.get(space).unwrap_or(0))
                .max()
                .unwrap_or(0);
            if let Some(app_mut) = apps.get_mut(new_focus) {
                app_mut.win_z.set(space, max_z + 1);
                app_mut.win_focused = true;
                app_mut.focus_ring_active = true;
                app_mut.last_focus_time = Instant::now();
            }
            // Schedule compaction in compositor tick (B2 fix)
            if space >= 1 && space <= 8 {
                pending_compact.fetch_or(1 << (space - 1), Ordering::Release);
            }
        }
    }

    // Step 2: Clear old focus state
    if let Some(old_app) = apps.get_mut(old_focus) {
        old_app.win_focused = false;
        old_app.focus_ring_active = false;
    }

    // Step 3: Update FOCUSED_APP (ArcSwap — lock-free store)
    FOCUSED_APP.store(Arc::new(new_focus.to_string()));

    // Step 4: Update last_focused for active space (B3: under apps-map lock, no new mutex)
    if let Some(ss) = spaces.get_mut(&active_space) {
        ss.last_focused = Some(new_focus.to_string());
    }

    // Step 5: Enqueue notifications — dispatched AFTER apps-lock released (B1 fix)
    pending_notifs.push(PendingFocusNotif::AppFocusLost {
        app_name: old_focus.to_string(), reason,
    });
    pending_notifs.push(PendingFocusNotif::AppFocusGained {
        app_name: new_focus.to_string(), reason,
    });
    pending_notifs.push(PendingFocusNotif::ChromeFocusChanged {
        app_name: new_focus.to_string(),
    });
    pending_notifs.push(PendingFocusNotif::DockFocusChanged {
        app_name: new_focus.to_string(),
    });
}
```

### 3.3 Notification Dispatch (B1 Fix)

`pending_focus_notifications: Vec<PendingFocusNotif>` on `SupervisorState`. After the
IPC handler releases the apps-map lock, the main event loop drains and dispatches:

```rust
// supervisor/src/main.rs — after IPC handling, apps-lock released

fn drain_focus_notifications(
    state: &mut SupervisorState,
    chrome_router: &mut ChromeRouter,
    dock_router: &mut DockRouter,
) {
    for notif in state.pending_focus_notifications.drain(..) {
        match notif {
            PendingFocusNotif::ChromeFocusChanged { app_name } => {
                chrome_router.route(
                    format!("VYOMA_CHROME:focus_changed:{}\n", app_name),
                    &state.chrome_stdin_tx,
                );
            }
            PendingFocusNotif::DockFocusChanged { app_name } => {
                dock_router.route(
                    format!("VYOMA_DOCK:focus_changed:{}\n", app_name),
                    &state.dock_stdin_tx,
                );
            }
            PendingFocusNotif::AppFocusGained { app_name, reason } => {
                if let Some(tx) = state.app_stdin_txs.get(&app_name) {
                    let _ = tx.send(format!("VYOMA_FOCUS:gained:{:?}\n", reason));
                }
            }
            PendingFocusNotif::AppFocusLost { app_name, reason } => {
                if let Some(tx) = state.app_stdin_txs.get(&app_name) {
                    let _ = tx.send(format!("VYOMA_FOCUS:lost:{:?}\n", reason));
                }
            }
        }
    }
}
```

This mirrors the `pending_resize` (R21) and `pending_focus` (R25) proven patterns.

---

## 4. Deferred Z-Order Compaction (B2 Fix)

```rust
// supervisor/src/focus/state.rs

/// Bitmask of spaces needing compact_z_order (bits 0-7 = spaces 1-8)
pub static PENDING_COMPACT_SPACES: AtomicU8 = AtomicU8::new(0);
pub static PENDING_COMPACT_SPACE0: AtomicBool = AtomicBool::new(false);
```

Compositor `vsync_tick` drains at start of each frame (after space switch block):

```rust
// supervisor/src/compositor.rs — vsync_tick()

let compact_mask = PENDING_COMPACT_SPACES.swap(0, Ordering::AcqRel);
for bit in 0..8u8 {
    if compact_mask & (1 << bit) != 0 {
        let space = bit + 1;
        compact_z_order(&mut apps, space);
    }
}
if PENDING_COMPACT_SPACE0.swap(false, Ordering::AcqRel) {
    compact_z_order(&mut apps, 0);
}
```

`compact_z_order` is O(N log N) but runs in the compositor thread under `vsync_lock.write()`,
where it already must hold. Multiple rapid focus changes set the same bit — deduped to
one sort per space per compositor frame. 10 rapid app exits → 1 compaction, not 10.

---

## 5. Z-Order Model

### 5.1 `PerSpaceZ` Recap (R21)

Each app has `win_z: PerSpaceZ` where `PerSpaceZ` is `HashMap<u8, u32>`. Z-value is
per-space; compacting space N only modifies entries for that space. Space-0 apps use
a fixed high-range z (chrome=65535, dock=65534, MC=65533, stage-strip=65532) and are
never compacted by user focus changes.

### 5.2 Bring-to-Front Algorithm

```
1. scan active space apps → find max z value
2. set app.win_z[active_space] = max + 1
3. set PENDING_COMPACT_SPACES bit for active_space
4. compositor tick: compact_z_order renumbers 1..N gap-free ascending
```

After compaction, z-values are `1, 2, 3, ... N` where N = focused app.

### 5.3 Multi-Window Apps

Deferred to R35 (Text Input & Selection Model). Single logical window per app for now.

---

## 6. Focus Ring

Apps draw their own focus ring. Supervisor sends `VYOMA_WM:focused` / `VYOMA_WM:blurred`
on focus change (existing R21 messages). App redraws on receipt. `surface_dirty` set on
both old and new focused apps to trigger compositor re-blit.

The supervisor does NOT render a focus ring overlay — it cannot know app-specific chrome
geometry. Apps handle their own focus appearance.

---

## 7. Focus and INPUT_LOCK_LEVEL

`FOCUSED_APP` is always updated regardless of `INPUT_LOCK_LEVEL`. The lock only affects
keystroke routing:

```rust
fn route_keyboard(ev: KeyEvent) {
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        2 => send_key_to_app("chrome", ev),
        1 => send_key_to_app("mission-control", ev),
        3 | 4 => { /* swallow */ }
        _ => send_key_to_app(&FOCUSED_APP.load(), ev),
    }
}
```

When `INPUT_LOCK_LEVEL` returns to 0 (animation complete), keys immediately route to
`FOCUSED_APP` — no focus lag from stale state.

---

## 8. Focus and Spaces

### 8.1 Per-Space `last_focused` (B3 Fix)

`SpaceState.last_focused: Option<String>` updated by `focus_transfer`. Space-0 apps
excluded (`win_space == 0` check in `focus_transfer`).

### 8.2 Space Switch Restore

```rust
// supervisor/src/focus/transfer.rs

pub fn on_space_switched(new_space: u8, apps: &mut HashMap<String, AppState>,
                          spaces: &mut HashMap<u8, SpaceState>,
                          pending_compact: &AtomicU8,
                          pending_notifs: &mut Vec<PendingFocusNotif>) {
    let old_focus = FOCUSED_APP.load().as_ref().clone();

    let new_focus: String = spaces.get(&new_space)
        .and_then(|ss| ss.last_focused.clone())
        .or_else(|| {
            // MRU fallback: highest last_focus_time in new_space
            apps.values()
                .filter(|a| a.win_space == new_space && !a.is_space0)
                .max_by_key(|a| a.last_focus_time)
                .map(|a| a.name.clone())
        })
        .unwrap_or_default();

    if !new_focus.is_empty() {
        focus_transfer(&new_focus, &old_focus, apps, spaces, new_space,
                       FocusReason::SpaceSwitchRestore, pending_compact, pending_notifs);
    }
}
```

Called by compositor after `PENDING_SPACE_SWITCH` is applied (same tick as space switch).

---

## 9. Focus and Full Screen / Split View

### 9.1 FS Enter

After `PENDING_SPACE_SWITCH` is applied for the FS space, `focus_transfer` is called
with `FocusReason::FsEnter`. FS app gets focus. `INPUT_LOCK_LEVEL = FsTransition(4)` is
set before `focus_transfer` and cleared after animation completes.

### 9.2 FS Exit

Uses R25 `pending_focus` pattern: focus transfer deferred until space switch back to
`fs_pre_space` is committed. `FocusReason::FsExit`.

### 9.3 SplitView Focus Toggle (B4 Fix)

Replace one-directional `splitview_focus_secondary` with symmetric toggle:

```
IPC: @supervisor: splitview_toggle_focus   (sent by either primary or secondary)
```

```rust
// supervisor/src/focus/transfer.rs

fn handle_splitview_toggle_focus(
    sender: &str,
    fs_registry: &mut FsRegistry,
    apps: &mut HashMap<String, AppState>,
    spaces: &mut HashMap<u8, SpaceState>,
    pending_compact: &AtomicU8,
    pending_notifs: &mut Vec<PendingFocusNotif>,
) {
    let pair = fs_registry.pairs.iter_mut()
        .find(|p| p.primary == sender || p.secondary == sender);
    if let Some(pair) = pair {
        std::mem::swap(&mut pair.primary, &mut pair.secondary);
        let new_primary = pair.primary.clone();
        let old_primary = pair.secondary.clone();
        // update split_role on both apps
        if let Some(app) = apps.get_mut(&new_primary) { app.split_role = Some(SplitRole::Primary); }
        if let Some(app) = apps.get_mut(&old_primary) { app.split_role = Some(SplitRole::Secondary); }
        // focus the new primary
        focus_transfer(&new_primary, &old_primary, apps, spaces,
                       apps[&new_primary].win_space, FocusReason::SplitViewToggle,
                       pending_compact, pending_notifs);
        // Notify both apps of role change (B4 fix)
        pending_notifs.push(PendingFocusNotif::SplitViewRoleChanged {
            app_name: new_primary, new_role: SplitRole::Primary,
        });
        pending_notifs.push(PendingFocusNotif::SplitViewRoleChanged {
            app_name: old_primary, new_role: SplitRole::Secondary,
        });
    }
    // Silently ignore if sender is not in any pair
}
```

Both primary and secondary can call `splitview_toggle_focus`. Both receive
`VYOMA_FOCUS:splitview_role_changed:<primary|secondary>`.

---

## 10. Focus and Stage Manager

```rust
// supervisor/src/stage/events.rs

pub fn on_stage_activated(
    stage: &Stage,
    apps: &mut HashMap<String, AppState>,
    spaces: &mut HashMap<u8, SpaceState>,
    active_space: u8,
    pending_compact: &AtomicU8,
    pending_notifs: &mut Vec<PendingFocusNotif>,
) {
    // Focus the MRU member of the incoming stage
    let new_focus = stage.last_focused.as_ref()
        .or_else(|| stage.members.first())
        .cloned()
        .unwrap_or_default();
    if !new_focus.is_empty() {
        let old_focus = FOCUSED_APP.load().as_ref().clone();
        focus_transfer(&new_focus, &old_focus, apps, spaces, active_space,
                       FocusReason::StageActivation, pending_compact, pending_notifs);
    }
}
```

`Stage.last_focused: Option<String>` added to `Stage` struct (R26). Updated by
`focus_transfer` when the active stage changes.

---

## 11. Focus Starvation Prevention

```rust
// supervisor/src/focus/transfer.rs

pub fn on_app_exit(
    exited_app: &str,
    apps: &mut HashMap<String, AppState>,
    spaces: &mut HashMap<u8, SpaceState>,
    pending_compact: &AtomicU8,
    pending_notifs: &mut Vec<PendingFocusNotif>,
) {
    let was_focused = FOCUSED_APP.load().as_ref() == exited_app;
    // Clean up last_focused for all spaces
    for ss in spaces.values_mut() {
        if ss.last_focused.as_deref() == Some(exited_app) {
            ss.last_focused = None;
        }
    }
    if was_focused {
        // Find MRU replacement in active space
        let active = /* current active space */;
        let replacement = apps.values()
            .filter(|a| a.win_space == active && a.name != exited_app && !a.is_space0)
            .max_by_key(|a| a.last_focus_time)
            .map(|a| a.name.clone());
        if let Some(new_focus) = replacement {
            focus_transfer(&new_focus, exited_app, apps, spaces, active,
                           FocusReason::AppExit, pending_compact, pending_notifs);
        } else {
            FOCUSED_APP.store(Arc::new(String::new()));
        }
    }
}
```

---

## 12. VYOMA_FOCUS: Protocol

### 12.1 Supervisor → App (stdin push)

```
VYOMA_FOCUS:gained:<reason>                         ← this app now has keyboard focus
VYOMA_FOCUS:lost:<reason>                           ← this app lost keyboard focus
VYOMA_FOCUS:splitview_role_changed:<primary|secondary>  ← B4: sent to both pair members
```

### 12.2 App → Supervisor (stdout IPC)

```
@supervisor: focus                                  ← request focus for self
@supervisor: splitview_toggle_focus                 ← B4: toggle primary/secondary role
```

---

## 13. WIT Interface `vyoma:focus@1.0.0` (B5 Fix)

All identifiers are **app names** (strings), not integer IDs (B5 fix). No undefined
`name→id` map needed.

### 13.1 `focus-query` (any app with `display = true`)

```wit
package vyoma:focus@1.0.0;

interface focus-query {
    /// Name of the currently focused app, or "" if none.
    focused-window: func() -> string;

    /// True if the calling app currently has keyboard focus.
    is-focused: func() -> bool;

    /// Name of the app that previously held focus, or "".
    previous-focused: func() -> string;

    /// Name of the focused app in the given space, or "".
    focused-in-space: func(space: u8) -> string;
}
```

### 13.2 `focus-control` (apps with `display = true` and `shell = true`)

```wit
interface focus-control {
    /// Request keyboard focus for this app.
    request-focus: func() -> result<_, string>;

    /// In SplitView: toggle focus between primary and secondary.
    splitview-toggle-focus: func() -> result<_, string>;
}
```

Capability gate: `init_linker_for_app` registers `focus-control` only for apps with
both `display = true` and `shell = true` in their manifest.

---

## 14. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Focus management | yes | yes | no (headless) | no |
| Z-order compaction | yes | yes | no | no |
| Focus ring (app-drawn) | yes | yes | no | no |
| `VYOMA_FOCUS:` protocol | yes | yes | no | no |
| WIT `vyoma:focus` | yes | yes | no | no |

---

## 15. File Layout

```
supervisor/src/
├── focus/
│   ├── mod.rs         (re-exports)
│   ├── state.rs       (FOCUSED_APP ArcSwap; PENDING_COMPACT_SPACES AtomicU8)
│   ├── transfer.rs    (focus_transfer, on_app_exit, on_space_switched, FocusReason, PendingFocusNotif)
│   └── split.rs       (handle_splitview_toggle_focus)
├── wm/
│   └── spaces.rs      (SpaceState.last_focused; SpaceRegistry updated)
├── compositor.rs      (drain PENDING_COMPACT_SPACES; call compact_z_order; drain_focus_notifications)
└── wit_handlers.rs    (focus-query + focus-control WIT registrations)
```

---

## 16. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: notify_chrome/notify_dock under apps-map lock → ABBA deadlock | `pending_focus_notifications: Vec<PendingFocusNotif>` appended inside lock; drained by main loop AFTER lock released — mirrors proven pending_resize pattern |
| B2: compact_z_order O(N log N) under apps-map lock on every focus change | Z-bump only (O(N) scan) under lock; `PENDING_COMPACT_SPACES: AtomicU8` bitmask set; compositor tick calls compact_z_order per flagged space under vsync_lock.write() |
| B3: `LAST_FOCUSED` standalone Mutex creates ABBA with space_switch_commit | `SpaceState.last_focused: Option<String>` embedded in SupervisorState.spaces under existing apps-map lock — no new Mutex |
| B4: splitview_focus_secondary one-directional; no primary yield mechanism | `splitview_toggle_focus` symmetric command; both apps receive `VYOMA_FOCUS:splitview_role_changed:<role>` |
| B5: FOCUSED_APP stores names but WIT returns u32 app-id; name→id map undefined | Standardize on app names throughout; WIT `focused-window` returns `string`; `focus-query` interface uses names only; integer IDs never exposed |
