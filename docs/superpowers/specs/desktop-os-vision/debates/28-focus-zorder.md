# ARCHITECT Spec: Focus Management & Z-Order (Round 28)

**Subsystem**: Focus Management & Z-Order  
**macOS Analogue**: `NSApplication.keyWindow` / `makeKeyAndOrderFront` / window focus rings  
**Depends on**: R21 (PerSpaceZ, compact_z_order, SpaceRegistry), R25 (INPUT_LOCK_LEVEL,
pending_focus), R26 (Stage Manager, StageSwitch), R27 (FullScreenMode, SplitView, FsTransition)  
**Status**: DRAFT — pending critique

---

## 1. Overview

Focus management in VyomaOS answers two questions per frame:
1. **Who receives keyboard input?** (key routing target)
2. **Who is on top?** (z-order of the focused window)

macOS conflates "key window" (keyboard focus) with "main window" (menu bar authority) into
a single concept. VyomaOS mirrors this: the focused app drives both keyboard routing and
the chrome menu bar title. Z-order is the visible confirmation — the focused window rises
to the top of its space.

This round defines the full focus state machine, the `makeKeyAndOrderFront` analogue,
cross-cutting interactions with INPUT_LOCK_LEVEL, spaces, full screen, split view, and
Stage Manager, plus the `VYOMA_FOCUS:` notification protocol and a WIT query interface.

---

## 2. Focus State Model

### 2.1 Primary Focused-App Atom

```rust
// supervisor/src/focus/state.rs

use arc_swap::ArcSwap;
use std::sync::Arc;

/// The single source of truth for which app currently holds keyboard focus.
/// Written by: focus_transfer() (called from IPC handlers, space switch commit,
///   app lifecycle, FS/SplitView transitions).
/// Read by: TTY input router, chrome menu-bar renderer, dock badge renderer,
///   WIT focus query handlers, IPC `focused_window` responses.
pub static FOCUSED_APP: Lazy<Arc<ArcSwap<String>>> =
    Lazy::new(|| Arc::new(ArcSwap::from_pointee(String::new())));
```

`ArcSwap<String>` is chosen because:
- **Wait-free reads**: the TTY input router reads `FOCUSED_APP` on every keystroke with no
  locking. A plain `Arc<Mutex<String>>` would introduce contention between the input thread
  and the compositor thread.
- **Atomic writes**: `focus_transfer()` calls `FOCUSED_APP.store(Arc::new(new_name))`.
  The old `Arc` is dropped after all readers are done; no use-after-free.
- **No circular Arc ownership**: `FOCUSED_APP` is a module-level `Lazy`; `AppState` does
  not embed it. There is exactly one owner.

The empty string `""` means "no app is focused" — valid during early boot or after
the last app exits.

### 2.2 Per-Space Last-Focused Map

```rust
// supervisor/src/focus/state.rs

use std::collections::HashMap;
use std::sync::Mutex;

/// Tracks the most-recently-focused app in each space.
/// Key = space number (0 = space-0 overlay, 1-9 = user spaces).
/// Written by: focus_transfer() under last_focused.lock().
/// Read by: space_switch_commit() to restore focus when switching spaces.
pub static LAST_FOCUSED: Lazy<Mutex<HashMap<u8, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
```

`Mutex<HashMap>` is appropriate here: `last_focused` is only written during a focus
transition (low frequency) and read during space switches (also low frequency). The
hot path (keystroke routing) never touches this map — it reads `FOCUSED_APP` instead.

### 2.3 AppState Focus Fields

```rust
// supervisor/src/app_state.rs

pub struct AppState {
    // ... existing fields ...

    /// True when this app currently holds keyboard focus.
    /// Mirrored from FOCUSED_APP for O(1) lookup without string comparison.
    pub win_focused: bool,

    /// True when this app should draw a focus ring / active title bar decoration.
    /// Equals win_focused when the app is in a Normal or TileWindow mode.
    /// For SplitView: primary = true, secondary = false (B5: secondary still
    /// receives VYOMA_FOCUS:secondary_active for custom drawing).
    pub focus_ring_active: bool,
}
```

### 2.4 Relationship to INPUT_LOCK_LEVEL

`FOCUSED_APP` tracks *intent* — which app **would** receive keys in a fully unlocked
system. `INPUT_LOCK_LEVEL` is an *override* layer on top. The routing rule (from R27):

```rust
// supervisor/src/input.rs

fn route_keyboard(ev: KeyEvent) {
    let focused = FOCUSED_APP.load();
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        2 => send_key_to_app("chrome", ev),          // ChromeConsent dialog
        1 => send_key_to_app("mission-control", ev), // MC overlay
        3 | 4 => { /* StageSwitch | FsTransition: swallow */ }
        _ => {
            if focused.is_empty() { return; }         // no focus yet: drop key
            send_key_to_app(&focused, ev);
        }
    }
}
```

Focus state is **always updated** regardless of INPUT_LOCK_LEVEL. If a user clicks
an app while a Mission Control animation runs, focus transfers logically; when the
animation completes and lock drops to 0, keys immediately go to the newly-focused app.

---

## 3. `makeKeyAndOrderFront` Equivalent

### 3.1 IPC Command

```
@supervisor: focus <app_name>
```

Also triggered internally via:
- Mouse click on a window (R32, deferred)
- `VYOMA_WM:focus:<app_id>` stdout protocol (R21)
- `focus-window` WIT call (§12)
- Space switch commit (restore last-focused)
- App lifecycle transitions (FS enter/exit, SplitView exit, stage activation)

### 3.2 `focus_transfer` — The Central Function

All focus changes route through this single function:

```rust
// supervisor/src/focus/commands.rs

pub fn focus_transfer(
    new_focus: &str,
    apps: &mut HashMap<String, AppState>,
    spaces: &SpaceRegistry,
    reason: FocusReason,
) {
    // Step 1: Identify previous focused app
    let old_focus: String = FOCUSED_APP.load().as_ref().clone();
    if old_focus == new_focus { return; } // idempotent

    // Step 2: Validate new_focus exists and is in the active space (or space-0)
    let app = match apps.get(new_focus) {
        Some(a) if a.win_visible && !a.win_minimized
            && (a.win_space == spaces.active || a.win_space == 0) => a,
        _ => return, // silently ignore invalid targets
    };

    // Step 3: Bring new_focus to front in its space
    //   Set z to MAX+1, then compact (gap-free renumber)
    let target_space = if app.win_space == 0 { 0 } else { spaces.active };
    if let Some(app) = apps.get_mut(new_focus) {
        let max_z = apps.values()
            .filter(|a| a.win_space == target_space || (target_space == spaces.active && a.win_space == spaces.active))
            .map(|a| a.win_z.get(target_space))
            .max()
            .unwrap_or(0);
        app.win_z.set(target_space, max_z + 1);
    }
    compact_z_order(apps, target_space);

    // Step 4: Update win_focused mirror on AppState
    if let Some(old) = apps.get_mut(&old_focus) {
        old.win_focused = false;
        old.focus_ring_active = false;
    }
    if let Some(new_app) = apps.get_mut(new_focus) {
        new_app.win_focused = true;
        new_app.focus_ring_active = (new_app.fs_mode != FullScreenMode::SplitView
            || new_app.split_role == Some(SplitRole::Primary));
    }

    // Step 5: Atomic update of FOCUSED_APP
    FOCUSED_APP.store(Arc::new(new_focus.to_string()));

    // Step 6: Update last_focused for the active space
    {
        let mut lf = LAST_FOCUSED.lock().unwrap();
        lf.insert(spaces.active, new_focus.to_string());
    }

    // Step 7: Send VYOMA_FOCUS: notifications (non-blocking, buffered)
    if !old_focus.is_empty() {
        send_focus_event(&old_focus, FocusEvent::Lost { reason });
    }
    send_focus_event(new_focus, FocusEvent::Gained { reason });

    // Step 8: Notify chrome and dock (privileged drop-oldest ring, 64 slots)
    notify_chrome(ChromeEvent::FocusChanged {
        app_name: new_focus.to_string(),
        app_title: apps.get(new_focus).map(|a| a.win_title.clone()).unwrap_or_default(),
    });
    notify_dock(DockEvent::FocusChanged { app_name: new_focus.to_string() });
}
```

### 3.3 `FocusReason` Enum

```rust
// supervisor/src/focus/commands.rs

pub enum FocusReason {
    UserClick,          // R32 mouse click
    IpcCommand,         // @supervisor: focus <app>
    SpaceSwitch,        // space restored focus on switch
    AppLaunch,          // new app gets focus on launch
    AppExit,            // previous app exited; MRU replacement
    FsEnter,            // fullscreen_enter: FS app auto-focused
    FsExit,             // fullscreen_exit: restored pre-FS focus
    SplitViewEnter,     // primary of new SplitView pair
    SplitViewExit,      // remaining app promoted to FS
    StageActivation,    // Stage Manager stage swapped
    WitCall,            // focus-window WIT API
    Programmatic,       // catch-all for supervisor-internal calls
}
```

`FocusReason` is included in `VYOMA_FOCUS:gained` and `VYOMA_FOCUS:lost` events so
apps can distinguish user-driven focus (draw attention animation) from programmatic
focus (suppress animation for seamless transitions).

---

## 4. Z-Order Model

### 4.1 Recap of R21 PerSpaceZ

From R21:

```rust
pub struct PerSpaceZ {
    pub by_space: HashMap<u8, u32>,
}
impl PerSpaceZ {
    pub fn get(&self, space: u8) -> u32 { self.by_space.get(&space).copied().unwrap_or(0) }
    pub fn set(&mut self, space: u8, z: u32) { self.by_space.insert(space, z); }
}
```

Key invariants (R21):
- Space-N apps are composited first (z ascending within `spaces.active`)
- Space-0 apps are composited second (always on top of all space-N apps)
- After `compact_z_order`, z-values are multiples of 10 starting at 10 (10, 20, 30…)
- Space-0 compact base = `space_n_count * 10 + 10`

### 4.2 Bring-to-Front Algorithm

```
Focus <app> in space S:
  1. max_z = max(win_z.get(S) for all visible, non-minimized apps in space S)
  2. app.win_z.set(S, max_z + 1)        // temporarily above all others
  3. compact_z_order(apps, S)           // renumber to 10, 20, 30…; the app
                                         //  ends up at N * 10 where N = count
```

This is equivalent to macOS `makeKeyAndOrderFront`: the window is moved to the front
of the z-order for its layer (normal or floating). The compaction step prevents
unbounded growth of z-values.

### 4.3 compact_z_order — Canonical Reference

```rust
// supervisor/src/wm/zorder.rs

pub fn compact_z_order(apps: &mut HashMap<String, AppState>, space: u8) {
    // --- Pass 1: space-N apps in `space` ---
    let mut space_n: Vec<String> = apps.values()
        .filter(|a| a.win_space == space && a.win_visible && !a.win_minimized)
        .sorted_by_key(|a| a.win_z.get(space))
        .map(|a| a.name.clone())
        .collect();
    for (i, name) in space_n.iter().enumerate() {
        if let Some(a) = apps.get_mut(name) {
            a.win_z.set(space, (i as u32 + 1) * 10);
        }
    }

    // --- Pass 2: space-0 overlay apps (always on top) ---
    let base = space_n.len() as u32 * 10 + 10;
    let mut space_0: Vec<String> = apps.values()
        .filter(|a| a.win_space == 0 && a.win_visible && !a.win_minimized)
        .sorted_by_key(|a| a.win_z.get(0))
        .map(|a| a.name.clone())
        .collect();
    for (i, name) in space_0.iter().enumerate() {
        if let Some(a) = apps.get_mut(name) {
            a.win_z.set(0, base + (i as u32 + 1) * 10);
        }
    }
}
```

`compact_z_order` is called from:
1. `focus_transfer` — after bring-to-front z bump
2. `space_switch_commit` — after PENDING_SPACE_SWITCH applied
3. Window add: new app assigned `z = 0`; compact places it at the front of its arrival
   order (other windows unchanged since sorting is stable)
4. Window remove: exit / minimize. Compact closes the gap.
5. `fullscreen_enter` / `splitview_enter`: both call `compact_z_order` on the FS
   dedicated space after all geometry is set.

### 4.4 Multi-Window Apps

VyomaOS R28 is **one-app = one-window**. Each WASM process has exactly one surface.
Multi-window apps (a single app binary managing multiple logical windows) are deferred
to R35 (Multi-Window Apps). Reasoning: VyomaOS's WASM security model requires each
app to manage its own surface via `VYOMA_DRAW:`; the supervisor assigns one surface per
process. If an app needs multiple panels, it spawns multiple WASM processes with IPC
coordination (already supported via `@<app>: <message>`).

No `win_z` changes needed for R35 — each logical sub-window would be a separate
`AppState` entry with its own `PerSpaceZ`, and `compact_z_order` would treat them
as independent windows.

---

## 5. Focus Ring / Active Window Indicator

### 5.1 Design Decision: App-Drawn Focus Indicator

The supervisor does **not** draw a focus ring overlay on top of app surfaces. Rationale:
- VyomaOS apps own their entire surface; adding a supervisor-drawn border would require
  the supervisor to know each app's chrome decoration area, which is app-specific.
- The WASM boundary means the supervisor cannot safely introspect app-internal coordinates.
- macOS moved to app-drawn window chrome in AppKit; we follow the same model.

Instead, the supervisor **tells** the app about its focus state change via `VYOMA_FOCUS:`,
and the app redraws its title-bar / header area to indicate active/inactive state.

### 5.2 Focus State Communicated via VYOMA_WM Notifications

The R21 `VYOMA_WM:focused` and `VYOMA_WM:blurred` push notifications remain the
canonical mechanism for apps that only need a simple boolean:

```
VYOMA_WM:focused          → app draws active title bar (e.g., bright white text)
VYOMA_WM:blurred          → app draws inactive title bar (e.g., gray text)
```

`VYOMA_FOCUS:gained` and `VYOMA_FOCUS:lost` (§11) carry additional metadata (`reason`,
`split_role`) for richer interactions.

### 5.3 Focus Ring Convention

Apps should follow this convention (non-enforced, documented in `docs/app-guidelines.md`):
- Active window: title bar background uses `BG_ACTIVE` from the color palette; title
  text weight = medium; resize handle opacity = 1.0
- Inactive window: title bar background uses `BG_INACTIVE`; title text opacity = 0.5;
  resize handle opacity = 0.3

The VyomaOS design token for `BG_ACTIVE` and `BG_INACTIVE` is emitted via the
`VYOMA_THEME:` protocol (deferred to R29: Theming).

### 5.4 Chrome Menu Bar

The chrome app displays the focused app's name and title in the menu bar. On
`VYOMA_CHROME:focus_changed:<app_name>,<app_title>` push:
- Update the left-side menu bar text to `<app_title>`
- Update the application menu items for the newly focused app

This notification is sent from `focus_transfer` Step 8 via the privileged
drop-oldest ring buffer (64 slots, established in R23).

---

## 6. Focus and INPUT_LOCK_LEVEL

### 6.1 Lock Levels Recap (R25 + R27)

| Level | Constant | Keys go to |
|-------|----------|-----------|
| 0 | `None` | `FOCUSED_APP` |
| 1 | `MissionControl` | `mission-control` app |
| 2 | `ChromeConsent` | `chrome` app |
| 3 | `StageSwitch` | swallowed |
| 4 | `FsTransition` | swallowed |

### 6.2 Focus Transfer During Lock

Focus can transfer logically while a lock is active. The rule:

```
Lock active → FOCUSED_APP.store() still executes (focus tracked).
              win_focused / focus_ring_active still update.
              VYOMA_FOCUS:gained / VYOMA_FOCUS:lost still sent to apps.
              Chrome + dock notifications still sent.
              VYOMA_WM:focused / VYOMA_WM:blurred still sent to apps.
              But route_keyboard ignores FOCUSED_APP until lock == 0.
```

This allows the dock (running under `StageSwitch` lock during stage animation) to
perform a focus change, so that when the animation completes and INPUT_LOCK_LEVEL drops
to 0, the first keystroke immediately goes to the correct app. No "focus lag" on
animation completion.

### 6.3 No Re-entrant Lock Promotion

`focus_transfer` does not raise `INPUT_LOCK_LEVEL`. Only specialized sequences
(FS enter, MC enter, etc.) raise the lock. `focus_transfer` is a pure-state-update
function.

### 6.4 ChromeConsent Focus Override (Level 2)

When level = 2, the `chrome` app receives all keys. This is used for system dialogs
(password prompt, permission dialog). The chrome app must dismiss the dialog before
releasing the lock. The chrome app tracks its own internal focus for the dialog fields;
the supervisor's `FOCUSED_APP` remains pointing at the background app that will resume
focus after the dialog closes.

---

## 7. Focus and Spaces

### 7.1 Focus is Per-Space

Each space maintains its own most-recently-focused app via `LAST_FOCUSED: HashMap<u8, String>`.
When the user switches from space 1 to space 2, the previously-focused app in space 2
is automatically focused — no explicit focus command required.

### 7.2 Space Switch Focus Restoration

```rust
// supervisor/src/wm/spaces.rs — called from compositor space-switch block

pub fn space_switch_commit(
    new_space: u8,
    apps: &mut HashMap<String, AppState>,
    spaces: &mut SpaceRegistry,
) {
    spaces.active = new_space;

    // Restore last-focused app in new space
    let target = {
        let lf = LAST_FOCUSED.lock().unwrap();
        lf.get(&new_space).cloned()
    };

    let focus_target = match target {
        Some(name) if apps.get(&name).map_or(false, |a| a.win_visible && !a.win_minimized) => name,
        _ => {
            // No history or app no longer visible: fall back to MRU in new space
            find_mru_in_space(apps, new_space)
                .map(|a| a.name.clone())
                .unwrap_or_default()
        }
    };

    if !focus_target.is_empty() {
        focus_transfer(&focus_target, apps, spaces, FocusReason::SpaceSwitch);
    } else {
        // No apps in new space: clear focus
        FOCUSED_APP.store(Arc::new(String::new()));
    }

    compact_z_order(apps, new_space);
}
```

### 7.3 `find_mru_in_space` Helper

```rust
// supervisor/src/focus/state.rs

fn find_mru_in_space<'a>(
    apps: &'a HashMap<String, AppState>,
    space: u8,
) -> Option<&'a AppState> {
    apps.values()
        .filter(|a| (a.win_space == space || a.win_space == 0)
            && a.win_visible && !a.win_minimized)
        .max_by_key(|a| a.last_focus_time)  // last_focus_time: Instant on AppState
}
```

`last_focus_time: Instant` is added to `AppState` — updated in `focus_transfer`
when that app gains focus.

### 7.4 Space-0 and Focus

Space-0 apps (chrome, dock, notification center) can receive focus via INPUT_LOCK_LEVEL
overrides (levels 1 and 2) or never (most overlays are non-interactive). They do **not**
appear in `LAST_FOCUSED`. A space-0 app gaining focus via lock level does not update
`LAST_FOCUSED`; when the lock releases, the previous user-space app in `FOCUSED_APP` is
retained, restoring natural key routing.

Exception: the notification center (when an action button is focused) uses `ChromeConsent`
lock (level 2) rather than updating `FOCUSED_APP`.

---

## 8. Focus and Full Screen / Split View

### 8.1 Full Screen Focus

**Enter**: `fullscreen_enter` sequence (R27 §6.1) calls `focus_transfer` in step 9
(after `INPUT_LOCK_LEVEL = FsTransition`):

```
fullscreen_enter step 9 (new):
  focus_transfer(app_name, apps, spaces, FocusReason::FsEnter);
```

`FocusReason::FsEnter` suppresses the app's focus ring entrance animation (the FS
animation itself is visually sufficient).

**Exit**: `fullscreen_exit` sequence (R27 §6.2) uses `pending_focus: Option<String>`
(R25 pattern). The pending focus is applied after `PENDING_SPACE_SWITCH` commits:

```
fullscreen_exit step 4 (existing in R27):
  state.pending_focus = Some(app.name.clone())
```

`space_switch_commit` checks `pending_focus` before falling back to `LAST_FOCUSED`:

```rust
// supervisor/src/wm/spaces.rs — space_switch_commit()

if let Some(pending) = state.pending_focus.take() {
    focus_transfer(&pending, apps, spaces, FocusReason::FsExit);
} else {
    // ... normal LAST_FOCUSED restoration
}
```

### 8.2 Split View Focus

**Primary app** receives keyboard focus at `splitview_enter`. The primary is the app
that issued `@supervisor: splitview_enter <other_app>`:

```
splitview_enter step 12 (new):
  focus_transfer(primary_app, apps, spaces, FocusReason::SplitViewEnter);
  primary_app.focus_ring_active = true;
  secondary_app.focus_ring_active = false;
  send_focus_event(secondary_app, FocusEvent::SecondaryActive);
```

**Switching focus within SplitView**: The secondary app can request focus via:

```
@supervisor: splitview_focus_secondary
```

This swaps primary/secondary keyboard routing within the pair's dedicated space. The
IPC handler:

```rust
// supervisor/src/fs/commands.rs

fn handle_splitview_focus_secondary(pair: &mut FsPair, apps: &mut HashMap<String, AppState>) {
    std::mem::swap(&mut pair.primary, &mut pair.secondary);
    // Update split_role on both apps
    if let Some(a) = apps.get_mut(&pair.primary) {
        a.split_role = Some(SplitRole::Primary);
        a.focus_ring_active = true;
    }
    if let Some(a) = apps.get_mut(&pair.secondary) {
        a.split_role = Some(SplitRole::Secondary);
        a.focus_ring_active = false;
    }
    focus_transfer(&pair.primary, apps, spaces, FocusReason::Programmatic);
}
```

**Keyboard routing in SplitView**: Only the primary app of the pair receives keys via
`FOCUSED_APP`. The secondary app can draw differently (dimmed title bar) but receives
no keyboard input unless it becomes primary.

**Why not Command-Tab between SplitView pair?** Command-Tab is handled by the app
switcher (R24/Dock). In a SplitView space, Command-Tab cycles through all visible
app groups, not just the pair. The SplitView-internal focus toggle uses a distinct
gesture (`@supervisor: splitview_focus_secondary`) rather than Command-Tab to avoid
conflict.

### 8.3 Tile Window Focus

TileWindow apps remain in their original space. Focus works normally — `focus_transfer`
targets the tile app by name, brings it to front in its space, and routes keys to it.
The non-tiled apps in the same space can still receive focus via click or IPC.

---

## 9. Focus and Stage Manager

### 9.1 Stage Activation Focus

When Stage Manager activates a new stage (R26), the supervisor must decide which app in
the incoming stage receives focus. Policy: **most-recently-focused app in the stage**.

Each stage (R26 `Stage` struct) should track a `last_focused: Option<String>`:

```rust
// supervisor/src/stage/registry.rs — extends Stage from R26

pub struct Stage {
    pub id: u32,
    pub members: Vec<String>,        // app names in this stage
    pub last_focused: Option<String>, // MRU focus within this stage (NEW)
}
```

The stage activation sequence calls:

```rust
// supervisor/src/stage/events.rs — on_stage_activated()

pub fn on_stage_activated(
    stage: &Stage,
    apps: &mut HashMap<String, AppState>,
    spaces: &mut SpaceRegistry,
) {
    // Candidate: last_focused if still visible in this stage
    let target = stage.last_focused.as_ref()
        .and_then(|name| apps.get(name))
        .filter(|a| a.win_visible && !a.win_minimized)
        .map(|a| a.name.clone())
        .or_else(|| {
            // Fall back: MRU by last_focus_time among stage members
            stage.members.iter()
                .filter_map(|n| apps.get(n))
                .filter(|a| a.win_visible && !a.win_minimized)
                .max_by_key(|a| a.last_focus_time)
                .map(|a| a.name.clone())
        });

    if let Some(name) = target {
        focus_transfer(&name, apps, spaces, FocusReason::StageActivation);
    }
}
```

### 9.2 Stage Focus Tracking

`Stage.last_focused` is updated whenever `focus_transfer` runs and the newly focused app
is a member of the currently active stage:

```rust
// supervisor/src/focus/commands.rs — focus_transfer(), after Step 6

if let Some(stage) = active_stage_for_app(new_focus, stage_reg, spaces) {
    stage.last_focused = Some(new_focus.to_string());
}
```

`active_stage_for_app(name, stage_reg, spaces)` returns a mutable reference to the
stage containing `name` in the current space's stage set, or `None` if SM is disabled
or the app is not in a stage.

### 9.3 Stage Manager Thumbnail Focus Highlight

The Stage Manager thumbnail strip (R26) highlights the thumbnail of the focused app's
stage. On `VYOMA_STAGE:focus_changed:<app_name>` push:
- Stage Manager marks the active stage's thumbnail with a highlight border.
- The specific thumbnail of the focused app within the strip is not individually
  highlighted (strip thumbnails are group-level, not per-app).

```rust
// supervisor/src/focus/commands.rs — focus_transfer(), Step 8 extension

notify_stage_manager(StageEvent::FocusChanged { app_name: new_focus.to_string() });
```

---

## 10. Focus Starvation Prevention

### 10.1 Problem

When the focused app exits unexpectedly (crash or normal exit), `FOCUSED_APP` points
to a non-existent app. Subsequent keystrokes are dropped. The system appears "frozen"
to the user until they click another window.

### 10.2 MRU Focus Inheritance Policy

On app exit, the process lifecycle handler calls `on_app_exit`:

```rust
// supervisor/src/focus/commands.rs

pub fn on_app_exit(
    exiting_app: &str,
    apps: &mut HashMap<String, AppState>,
    spaces: &SpaceRegistry,
) {
    let was_focused = {
        let f = FOCUSED_APP.load();
        f.as_str() == exiting_app
    };

    if !was_focused { return; } // another app already focused; nothing to do

    // Find MRU replacement in active space
    let replacement = find_mru_in_space(apps, spaces.active)
        .filter(|a| a.name != exiting_app)
        .map(|a| a.name.clone());

    match replacement {
        Some(name) => {
            focus_transfer(&name, apps, spaces, FocusReason::AppExit);
        }
        None => {
            // No apps remain in active space
            FOCUSED_APP.store(Arc::new(String::new()));
            // Update LAST_FOCUSED for active space too
            let mut lf = LAST_FOCUSED.lock().unwrap();
            lf.remove(&spaces.active);
        }
    }

    // Compact z-order to close the gap left by the exiting window
    compact_z_order(apps, spaces.active);
}
```

Called from the existing app lifecycle handler before the `AppState` is removed from
the `apps` HashMap, so that `find_mru_in_space` can exclude the exiting app.

### 10.3 `last_focus_time` Ordering

`last_focus_time: std::time::Instant` is set to `Instant::now()` in `focus_transfer`
when an app gains focus. For apps that have never been focused, `last_focus_time` is
set to `Instant::UNIX_EPOCH` equivalent via `Instant::now() - Duration::from_secs(u64::MAX / 2)`
(pre-set in `AppState::new`). This ensures `find_mru_in_space` returns the most recently
used app, not a never-focused one, as the replacement candidate.

### 10.4 LAST_FOCUSED Cleanup

When an app exits, its name is removed from `LAST_FOCUSED` for all spaces it was ever
assigned to:

```rust
// supervisor/src/focus/commands.rs — on_app_exit(), called after replacement focus

{
    let mut lf = LAST_FOCUSED.lock().unwrap();
    lf.values_mut().for_each(|name| {
        if name == exiting_app { name.clear(); }
    });
    lf.retain(|_, v| !v.is_empty());
}
```

---

## 11. VYOMA_FOCUS: Protocol

### 11.1 Supervisor → App (stdin push)

All events are line-terminated (`\n`). Fields separated by `:`.

```
VYOMA_FOCUS:gained:<reason>
```
Sent to the newly focused app. `<reason>` is a snake_case string matching
`FocusReason` variants: `user_click`, `ipc_command`, `space_switch`, `app_launch`,
`app_exit`, `fs_enter`, `fs_exit`, `splitview_enter`, `splitview_exit`, `stage_activation`,
`wit_call`, `programmatic`.

```
VYOMA_FOCUS:lost:<reason>
```
Sent to the previously focused app.

```
VYOMA_FOCUS:secondary_active
```
Sent to the secondary app in a SplitView pair when it becomes the secondary but a
focus-swap occurred (e.g., the user requested `splitview_focus_secondary`). The
secondary app knows it is the *unfocused* partner in the pair.

```
VYOMA_FOCUS:primary_active
```
Sent to the primary app of a SplitView pair after a `splitview_focus_secondary`
reassignment — confirms it is now the unfocused secondary (i.e., the old secondary
is now primary). Note: this is sent to the app that **lost** primary status, so it
can dim its title bar.

Wait — more precisely:
- `splitview_focus_secondary` swaps primary ↔ secondary.
- New primary (was secondary): receives `VYOMA_FOCUS:gained:programmatic`
- New secondary (was primary): receives `VYOMA_FOCUS:primary_to_secondary`

```
VYOMA_FOCUS:primary_to_secondary
```
Sent to the app that was primary in a SplitView pair but is now secondary after
a focus-toggle. The app should draw its title bar as inactive.

### 11.2 App → Supervisor

```
@supervisor: splitview_focus_secondary
```
The secondary app of a pair requests to become primary. If the sender is not currently
the secondary of any SplitView pair, this command is ignored.

No other focus commands from apps — apps request focus implicitly by writing
`VYOMA_WM:focus:<app_id>` (R21) or calling the WIT `focus-window` function (§12).

### 11.3 VYOMA_FOCUS: Delivery Guarantees

Focus events are delivered via the same non-blocking pipe that carries `VYOMA_WM:`
notifications. The `send_focus_event` function uses the drop-oldest ring buffer
(64 slots, R23) shared with `VYOMA_CHROME:` notifications. Focus events are marked
with priority `High`; if the ring is full, oldest `Low`-priority events are dropped
first.

---

## 12. WIT Interface `vyoma:focus@1.0.0`

```wit
package vyoma:focus@1.0.0;

/// Reason why focus changed. Used in focus-event notifications.
enum focus-reason {
    user-click,
    ipc-command,
    space-switch,
    app-launch,
    app-exit,
    fs-enter,
    fs-exit,
    splitview-enter,
    splitview-exit,
    stage-activation,
    wit-call,
    programmatic,
}

/// Focus event delivered to apps that subscribe.
variant focus-event {
    gained(focus-reason),
    lost(focus-reason),
    secondary-active,
    primary-to-secondary,
}

interface focus-query {
    /// Returns the app-id of the currently focused window, or 0 if none.
    focused-window: func() -> u32;

    /// Returns true if the calling app is currently the keyboard-focus target.
    is-focused: func() -> bool;

    /// Returns the app-id that last held focus before the current one.
    /// Returns 0 if no previous focus recorded.
    previous-focused: func() -> u32;

    /// Returns the app-id of the focused app in the given space (1-9).
    /// Returns 0 if the space has no focus history or the space is empty.
    focused-in-space: func(space: u8) -> u32;
}

interface focus-control {
    /// Bring this app to front and give it keyboard focus.
    /// Equivalent to @supervisor: focus <self>.
    /// Returns error if the calling app is minimized or in a hidden space.
    request-focus: func() -> result<_, string>;

    /// Bring another app to front by app-id.
    /// Only permitted for apps with the `shell` capability declared.
    /// Returns error if target not visible or permission denied.
    focus-window: func(app-id: u32) -> result<_, string>;
}

world focus-world {
    import focus-query;
    import focus-control;
}
```

### 12.1 `init_linker_for_app` Registration

```rust
// supervisor/src/wit_handlers.rs

fn init_linker_for_app(linker: &mut Linker<AppCtx>, manifest: &AppManifest) {
    // focus-query: always registered (read-only, no capability gate)
    register_focus_query(linker);

    // focus-control: requires display or shell capability
    if manifest.capabilities.display || manifest.capabilities.shell {
        register_focus_control(linker);
    }
}
```

`focus-query` is unconditionally available — every app can ask "am I focused?" without
declaring special capabilities. `focus-control` (requesting focus for other apps) requires
`display` or `shell` to prevent background-only apps from stealing focus.

### 12.2 `focused-window` vs `FOCUSED_APP`

The WIT `focused-window` function calls:
```rust
FOCUSED_APP.load().parse::<u32>() // or look up app-id from name
```
App IDs are stable integer handles assigned at launch (R22). The WIT layer keeps a
`name → app_id` map. `0` is reserved for "no focus".

---

## 13. Compositor Integration

### 13.1 Focus Ring Dirty Mark

When `focus_transfer` runs, the apps that gained and lost focus need to redraw their
title bars. The compositor needs to be informed:

```rust
// supervisor/src/focus/commands.rs — focus_transfer(), after Step 4

if let Some(old) = apps.get_mut(&old_focus) {
    old.surface_dirty = true;  // trigger repaint for focus ring removal
}
if let Some(new_app) = apps.get_mut(new_focus) {
    new_app.surface_dirty = true;  // trigger repaint for focus ring draw
}
```

`surface_dirty` is checked by the compositor pass; dirty apps are repainted from their
surface buffer. Setting `surface_dirty = true` here ensures the apps' own repaint
cycles (triggered by `VYOMA_WM:focused` / `VYOMA_WM:blurred`) are composited on the
next frame.

### 13.2 No Compositor-Level Focus Decoration

The compositor does **not** draw any focus ring or border overlay. All visual focus
feedback is app-drawn. The supervisor only communicates state changes.

### 13.3 vsync_tick Focus Integration

`focus_transfer` does not acquire `vsync_lock`. It only sets:
- `FOCUSED_APP` (atomic ArcSwap store)
- `LAST_FOCUSED` (Mutex, fast insert)
- `win_focused`, `focus_ring_active`, `surface_dirty` on AppState (plain field writes)

These writes are safe from the IPC handler thread because:
- `FOCUSED_APP` is lock-free
- `LAST_FOCUSED` uses its own Mutex (not vsync_lock)
- `AppState` field writes are protected by the `apps: Arc<Mutex<HashMap>>` (the apps
  map lock, held by the IPC handler thread at this point in execution)

The compositor thread reads `win_focused` and `focus_ring_active` only under its own
`vsync_lock.read()` pass, which it acquires after the IPC handler releases the apps
map lock. No deadlock.

---

## 14. Platform Matrix

| Feature | desktop-full | mobile | server-headless | iot-edge | mcu-minimal |
|---------|-------------|--------|----------------|----------|-------------|
| FOCUSED_APP (ArcSwap) | yes | yes | no (no display) | no | no |
| LAST_FOCUSED per-space | yes | no (1 space) | no | no | no |
| VYOMA_FOCUS: protocol | yes | yes | no | no | no |
| focus-query WIT | yes | yes | no | no | no |
| focus-control WIT | yes | yes | no | no | no |
| SplitView focus | yes | yes | no | no | no |
| Stage Manager focus | yes | no | no | no | no |
| compact_z_order | yes | yes (1 space) | no | no | no |

**Mobile**: Single space (space 1). `LAST_FOCUSED` keyed at space 1 only. No Stage
Manager. SplitView uses same focus model as desktop. Full-screen focus is always the
foreground app; switching apps via the dock transfers focus.

**server-headless**: No display subsystem. Focus tracking disabled. `FOCUSED_APP`
is a no-op stub. No keyboard routing (headless apps use stdin/stdout directly).

**iot-edge / robotics-rt / mcu-minimal**: No interactive display. Focus subsystem
compiled out via `#[cfg(feature = "display")]` gate on the focus module.

---

## 15. File Layout

```
supervisor/src/
├── focus/
│   ├── mod.rs           (pub re-exports: FOCUSED_APP, LAST_FOCUSED, focus_transfer,
│   │                      on_app_exit, FocusReason, FocusEvent)
│   ├── state.rs         (FOCUSED_APP: ArcSwap, LAST_FOCUSED: Mutex<HashMap>,
│   │                      last_focus_time on AppState, find_mru_in_space)
│   └── commands.rs      (focus_transfer, on_app_exit, send_focus_event,
│                          notify_chrome, notify_dock, notify_stage_manager)
├── wm/
│   ├── zorder.rs        (compact_z_order — extended to call from focus module)
│   └── spaces.rs        (space_switch_commit: integrates pending_focus + LAST_FOCUSED)
├── fs/
│   └── commands.rs      (splitview_focus_secondary handler)
├── stage/
│   └── events.rs        (on_stage_activated: last_focused in Stage struct)
├── app_state.rs         (win_focused: bool, focus_ring_active: bool,
│                          last_focus_time: Instant — new fields)
├── input.rs             (route_keyboard: reads FOCUSED_APP for level-0 routing)
└── wit_handlers.rs      (init_linker_for_app: register focus-query / focus-control)

supervisor/wit/
└── focus.wit            (vyoma:focus@1.0.0 — focus-query, focus-control interfaces)
```

**Line budget**: `focus/state.rs` ≈ 120 lines, `focus/commands.rs` ≈ 200 lines,
`focus/mod.rs` ≈ 20 lines. All well within the 500-line limit. `zorder.rs` (existing,
≈ 60 lines) and `spaces.rs` (existing, modified) stay under limit.

---

## 16. Edge Cases

### 16.1 Focus Request for Hidden App

If `focus_transfer` receives a target app that is in a different space than `spaces.active`,
the call is **rejected silently**. The caller should first issue a space switch
(`PENDING_SPACE_SWITCH`) and use `pending_focus` (R25 pattern) to apply focus after
the space switch commits. This is the existing pattern from R27 FS exit.

### 16.2 Focus During App Launch

New apps enter the `Launching` lifecycle state. They do not receive focus until they
transition to `Running`. The lifecycle state machine calls:

```rust
// supervisor/src/lifecycle.rs — on_app_running(name)

focus_transfer(name, apps, spaces, FocusReason::AppLaunch);
```

The transition from `Launching` → `Running` is triggered by the first `VYOMA_DRAW:flush`
(or a heartbeat timeout). This matches macOS behavior: an app gets the key window only
after its first frame is drawn.

### 16.3 Minimized App Focus Request

A minimized app cannot receive focus. `focus_transfer` checks `!a.win_minimized` in
its validation step. If a minimized app is the target of `FocusReason::AppLaunch` (e.g.,
restart of a previously minimized app), the supervisor first restores the window
(`win_minimized = false`, `pending_resize` triggered), then calls `focus_transfer`.

### 16.4 Concurrent Focus Requests

Two IPC threads could both call `focus_transfer` simultaneously (e.g., two apps racing
to request focus). The `ArcSwap.store()` is atomic; whichever write executes last wins.
`win_focused` writes are protected by the `apps` Mutex; the second writer will see
`win_focused = false` already set by the first and still overwrite it correctly.
This is acceptable: focus races are extremely rare in normal use and the final state is
always consistent.

---

## 17. Summary of Key Decisions

| Decision | Rationale |
|----------|-----------|
| `ArcSwap<String>` for FOCUSED_APP | Wait-free reads on the hot keystroke path |
| `Mutex<HashMap>` for LAST_FOCUSED | Low-frequency writes; space-switch is not on the hot path |
| App-drawn focus ring | Supervisor cannot safely introspect per-app chrome geometry |
| Focus tracked during lock | Eliminates "focus lag" when lock drops |
| MRU replacement on app exit | Predictable behavior; avoids focus starvation |
| One-app-one-window | WASM process isolation; multi-window deferred to R35 |
| `focus_transfer` always via one function | Single location for all side effects; no missed notifications |
| `FocusReason` in notifications | Apps can suppress animation for programmatic focus |
| `last_focus_time: Instant` on AppState | Enables MRU ordering without a separate data structure |
| `focus-query` unconditional | Read-only; safe for all apps including sandboxed ones |
| `focus-control` requires display/shell | Prevents background apps from stealing focus |
