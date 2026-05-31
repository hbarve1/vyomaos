# Round 24 — Dock & App Switcher (Architect)

**Status**: Draft
**Round**: 24
**Subsystem**: Dock & App Switcher
**Analogue**: macOS Dock (bottom bar, running indicators) and Cmd-Tab App Switcher
**Author**: Architect
**Date**: 2026-05-29

---

## 1. Overview and Motivation

A desktop operating system communicates the set of running and launchable applications to the
user through two canonical mechanisms: a persistent visual strip showing application icons at
all times (the Dock), and an on-demand transient overlay that lets the user rapidly cycle
through running apps without lifting their hands from the keyboard (the App Switcher). On
macOS these are the Dock and the Cmd-Tab switcher respectively.

VyomaOS through Round 23 has neither. The system chrome introduced in R23 reserves the top
24 logical points for the menu bar. Application windows fill the remaining area according to
the tiling layout from R21. There is no persistent visual indicator of which apps are running,
no running dot below app icons, no quick-switch overlay, and no standard user gesture for
moving focus between applications. The shell command `focus <app>` is the only non-trivial
mechanism, but it requires the user to know the app name and type it in full.

Round 24 introduces the **Dock**: a second privileged WASM app named `dock` that lives in
space 0, positioned at the bottom of the logical screen, occupying the full logical width at
48 logical points of height. Like `chrome`, the dock app draws entirely via the `VYOMA_DRAW:`
protocol and communicates with the supervisor through a line-oriented event protocol named
`VYOMA_DOCK:`. It receives lifecycle and focus events routed to it by the supervisor through
a new `dock_router.rs` module, maintaining an internal list of `DockSlot` entries that it
redraws on every state change.

Round 24 also introduces the **App Switcher**: an overlay drawn by the dock app itself,
triggered by the global keyboard shortcut Ctrl-Tab. When active, the overlay presents a
centred panel listing all running apps in MRU (most-recently-used) order. Repeated Ctrl-Tab
presses cycle forward through the list; Ctrl-Shift-Tab cycles backward. Releasing Ctrl
commits the selection by sending `@supervisor: focus <app_name>` to the supervisor.

### 1.1 What R24 Does Not Do

- No dock icon images (PNG icon support deferred to R14 integration into the dock rendering
  path; for now, each slot is a coloured rectangle with a text label).
- No dock magnification on hover (deferred to R32 mouse events and R16 animations).
- No pinned (non-running) app slots — v1 shows only running apps; a pinned launcher is
  deferred to R74 Launch Services.
- No dock position preference (left/right/bottom) — always bottom in v1; deferred to R78
  System Preferences.
- No dock hide/auto-hide behaviour — always visible in v1.
- No spring-loading (drag a file over a dock icon to open the app with that file).
- No Expose / Mission Control thumbnail preview in switcher — deferred to R25.
- No sound effects on app launch/exit indicator.
- No dock badges or notification counts (deferred to R73 Notification Center).
- No Cmd-Tab (uses Ctrl-Tab instead, because macOS Cmd-Tab requires a modifier that
  conflicts with terminal emulators where Cmd maps to a WASM app; R35 Global Shortcuts
  will formalise modifier remapping).

### 1.2 Relationship to Prior Rounds

| Round | Contribution reused or extended |
|-------|--------------------------------|
| R11   | IPC broker `@<app>: msg` pattern; dock sends focus transfers via `@supervisor:` |
| R17   | `VYOMA_DRAW:` protocol; dock renders all UI through standard draw commands |
| R20   | `DisplayConfig` logical points and `scale_factor`; dock height is 48 logical pts |
| R21   | space=0 always-on-top model; `PerSpaceZ`; dock uses `win_space = 0` |
| R22   | `LifecycleState` (Launching/Running/Suspended/Background/Terminating/Terminated); dock |
|       | tracks lifecycle per slot |
| R23   | `chrome_router.rs` pattern; `chrome = true` manifest flag; y-clip in compositor; |
|       | `src_y_offset` technique extended to bottom-clip for dock |

---

## 2. Dock Architecture

### 2.1 The Dock App as a Second Privileged WASM App

The dock is implemented as a `wasm32-wasip2` binary located at `apps/dock/`. It shares
exactly the same privilege mechanism as `chrome` from R23: its power comes entirely from
its supervisor registration in `boot.toml`, not from any capability the WASM runtime grants
it directly. From Wasmtime's perspective, `dock.wasm` and `gui-demo.wasm` are indistinguishable
processes.

What makes dock privileged is the `dock = true` field in its `boot.toml` entry:

```toml
[[app]]
name        = "dock"
wasm        = "dock.wasm"
space       = 0
dock        = true    # privileged flag: supervisor enables VYOMA_DOCK: routing

[app.capabilities]
stdio       = true
display     = true
shell       = true    # needed to send `@supervisor: focus <app>` and query app list
```

The `dock = true` flag causes the supervisor to:

1. Register this app as the system dock recipient. At most one app may have `dock = true`.
   If two entries declare `dock = true`, the supervisor logs an error and boots with only
   the first entry honoured.
2. Route `VYOMA_DOCK:` lines from any app's stdout to dock's stdin, prefixed with the sender:
   `VYOMA_DOCK_FROM:<sender>:<rest_of_line>`.
3. Send supervisor-generated dock events (app lifecycle changes, focus changes, initial app
   list) directly to dock's stdin without the `VYOMA_DOCK_FROM:` prefix, using the same
   direct stdin-write mechanism used by `chrome_router` for supervisor-generated chrome
   events.
4. Enforce a bottom-edge clip: in the compositor's `flush_pass`, all non-dock surfaces are
   clipped to `y + h <= logical_h - DOCK_HEIGHT_PTS` (see §10 for exact clipping arithmetic).
5. Assign dock's surface `z_order = 65534` (one below chrome's 65535), as the second
   highest Z in space 0. See §4 for the z-order model and the Ctrl-Tab overlay exception.

### 2.2 Window Configuration

The dock app declares its window in `vyoma.toml`:

```toml
[app]
name    = "dock"
version = "1.0.0"
wasm    = "dock.wasm"

[capabilities]
stdio   = true
display = true
shell   = true

[window]
space       = 0
x           = 0
y           = 0        # resolved at startup to (logical_h - 48) by supervisor
width       = 0        # 0 = full logical screen width, resolved at startup
height      = 48       # logical points
manual      = true     # not managed by tiling WM; dock controls its own geometry
z_order     = 65534    # one below chrome (65535)
```

The supervisor resolves `y = 0` in the dock manifest to `DisplayConfig.logical_height - 48`
at app spawn time. This is distinct from the `width = 0` → full-width resolution used for
chrome. The sentinel value `y = 0` in a `manual = true`, `dock = true` app is treated as
`y = logical_h - height`. If the screen is resized, the supervisor sends
`VYOMA_DOCK:display_resized:<new_w>,<new_h>` to dock's stdin so it can update its layout.

### 2.3 Surface Allocation

Dock receives a `Surface` of dimensions `(logical_w * scale_factor, 48 * scale_factor)`
physical pixels. The surface is allocated at dock spawn time by the same path used for any
display-capable app. The dock clears the surface with the dock background colour on startup,
renders its initial state (empty or populated from the initial app list message), and calls
`VYOMA_DRAW:flush`. Subsequent redraws are triggered by incoming stdin events.

The dock surface is positioned at physical y = `(logical_h - 48) * scale_factor`. The
compositor blits the dock surface with its `win_y_px` computed from the supervisor-resolved
window position. Because `z_order = 65534`, dock is blitted second-to-last in the Z sort
(chrome at 65535 is blitted last). This means chrome's 24px strip at the top still overlays
dock if dock's logical y were to overlap it — but since dock is at `logical_h - 48`, there
is no overlap under normal geometry.

### 2.4 Supervisor Changes Required for Dock

```
supervisor/src/
  dock_router.rs     NEW — route VYOMA_DOCK: lines from any app stdout to dock stdin
                     startup queue (capacity 64, drop-oldest policy); drain on dock Running
                     send supervisor-generated events (lifecycle, focus, initial_list) to dock stdin
  display/
    compositor.rs    MODIFIED — apply bottom-edge clip (y + h <= dock_clip_y_px) to non-dock
                     non-chrome blit calls; dock_clip_y_px = (logical_h - 48) * scale_factor
  process.rs         MODIFIED — parse `dock = true` field from boot.toml; register dock_app_name
  manifest.rs        MODIFIED — parse and validate `dock = true`; at most one dock app allowed
  ipc.rs             MODIFIED — lifecycle state changes, focus changes sent to dock_router;
                     handle `@supervisor: focus <app>` from dock stdout (already handled)
  keyboard.rs        MODIFIED — Ctrl-Tab intercept before focused-app routing (§7 provisional
                     global shortcut mechanism)
```

---

## 3. Icon Data Model

### 3.1 DockSlot Struct

The dock app maintains an ordered list of `DockSlot` entries, one per running (or recently
exited) app. The list is the dock's internal state; it is not shared with the supervisor.

```rust
// apps/dock/src/state.rs

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockSlot {
    /// Internal app name, as reported by the supervisor (e.g. "gui-demo").
    pub app_name: String,
    /// Display name for the slot label (human-readable).
    pub display_name: String,
    /// Current lifecycle state of the app.
    pub lifecycle: LifecyclePhase,
    /// Whether this app currently holds keyboard focus.
    pub is_focused: bool,
    /// Position in the MRU list (0 = most recently focused).
    pub mru_rank: usize,
}

/// Simplified lifecycle phase for dock display purposes.
/// Maps from R22's full LifecycleState.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecyclePhase {
    Launching,
    Running,
    Suspended,
    Background,
    Terminating,
    Terminated,
}
```

### 3.2 Slot List Ordering

The `DockSlot` list stored in `DockState` is ordered by **spawn order** — the order in
which `VYOMA_DOCK:app_launched:<app>` events were received. This is the canonical display
order in the dock strip. The MRU list (used exclusively by the switcher, §6) is a separate
`Vec<String>` of app names in most-recently-focused order; it is derived from the same slot
list but ordered differently. The dock strip always shows slots in spawn order; the switcher
overlay always shows names in MRU order. These are two independent orderings over the same
set of app names.

### 3.3 Display Name Mapping

Display names for system apps are derived from the same mapping convention as R23's chrome
app, extended to cover dock-visible apps:

| Internal name | Display name   |
|---------------|----------------|
| `chrome`      | (excluded from dock; dock does not show space-0 privileged apps) |
| `dock`        | (excluded from dock; self-exclusion) |
| `shell`       | `Terminal`     |
| `http-server` | `HTTP Server`  |
| `gui-demo`    | `Demo`         |
| `calculator`  | `Calculator`   |
| `ping`        | `Ping`         |
| `pong`        | `Pong`         |
| (any other)   | raw `app_name` with first letter capitalised |

This mapping is hardcoded in `apps/dock/src/state.rs` in v1. R24's branding field
(`display_name` in `vyoma.toml`) will replace this in R74 Launch Services.

### 3.4 Lifecycle Rendering State

Each `DockSlot` renders differently based on `lifecycle`:

| LifecyclePhase | Running dot | Slot background  | Label colour     |
|----------------|-------------|------------------|------------------|
| Launching      | Pulsing*    | `0x313244FF`     | `0x6C7086FF`     |
| Running        | Visible     | `0x313244FF`     | `0xCDD6F4FF`     |
| Suspended      | Dim dot     | `0x1E1E2EFF`     | `0x6C7086FF`     |
| Background     | Dim dot     | `0x1E1E2EFF`     | `0x9399B2FF`     |
| Terminating    | Fading*     | `0x1E1E2EFF`     | `0x6C7086FF`     |
| Terminated     | None        | `0x1E1E2EFF`     | `0x45475AFF`     |

*Animation (pulsing/fading) is advisory in v1; actual animation deferred to R16 integration.
In v1, Launching shows a solid dot and Terminating shows no dot.

A slot for a Terminated app is displayed for 2 seconds after the `app_exited` event, then
removed from the list. The 2-second display window is driven by counting `VYOMA_DOCK:tick:`
events (see §9.2). This gives the user visual feedback that an app exited without the slot
vanishing instantaneously.

---

## 4. Dock Z-Order and Multi-App Space-0 Model

### 4.1 Two Simultaneous Space-0 Apps

R21 defines space 0 as the "always-on-top" space whose windows appear on every space. R23
placed chrome at space=0 with `z_order = 65535`. R24 places dock at space=0 with
`z_order = 65534`. The compositor's `flush_pass` already handles multiple apps in the same
space: all space-0 windows are included in the Z-sorted blit list on every frame, alongside
the windows from the active space. Having two space-0 apps is therefore a natural extension
of the existing model, not a special case.

The definitive Z-order policy for space-0 privileged apps is:

| z_order | Owner | Blit position |
|---------|-------|---------------|
| 65535   | chrome | Last (topmost) |
| 65534   | dock  | Second-to-last |
| 65533   | (reserved for switcher overlay; see §4.2) | |
| 65532   | (reserved for future privileged apps) | |
| 0–65531 | Normal applications | In ascending Z order |

This table is the canonical Z-order reservation policy for VyomaOS. It is enforced in the
supervisor's manifest parser: any non-privileged app declaring `z_order >= 65532` has its
value clamped to `65531` with a warning log. Privileged apps (those with `chrome = true` or
`dock = true`) use the reserved values assigned in their supervisor registration code.

### 4.2 Switcher Overlay Z-Order

The Ctrl-Tab switcher overlay is drawn by the dock app itself into dock's own surface. When
the switcher is active, the dock temporarily resizes its surface to cover the full screen
using `VYOMA_DRAW:resize_surface:<logical_w>,<logical_h>` (the same command introduced in
R23 for chrome's consent modal). In this expanded state, dock's surface covers the full
screen but dock's `z_order` remains 65534.

Because dock's `z_order` is 65534, it is composited below chrome's surface (z=65535).
Chrome is 24pt tall at the top. The switcher overlay panel is drawn starting from
`y = 24` (below the menu bar) and extends to `y = logical_h - 48` (above the dock strip
itself). Chrome's surface at `z=65535` still appears above the switcher overlay, which is
the correct visual result: the menu bar remains visible above the switcher.

The dock strip at the bottom of the full-screen surface is also drawn during switcher mode,
showing which app will be selected. The result is that the switcher overlay occupies the
middle region of the screen while chrome and dock strip remain at their normal positions —
without any z_order change. This is a clean and architecturally sound solution: the switcher
overlay is just dock's surface temporarily expanded, not a separate entity.

When the switcher is dismissed (Ctrl released), dock issues
`VYOMA_DRAW:resize_surface:<logical_w>,48` to collapse back to the normal dock surface,
then redraws and flushes.

---

## 5. Focus Indicator

### 5.1 Focus Event Reception

When any app receives keyboard focus, the supervisor sends to dock's stdin:

```
VYOMA_DOCK:focus_changed:<app_name>
```

This is a supervisor-generated event written directly to dock's stdin by `dock_router.rs`,
without any `VYOMA_DOCK_FROM:` prefix. It uses the same direct write path as chrome's
`focus_changed` events.

### 5.2 Focused Slot Rendering

On receiving `focus_changed:<app_name>`, the dock:

1. Sets `is_focused = false` on all existing slots.
2. Sets `is_focused = true` on the slot whose `app_name` matches.
3. Updates the MRU list (see §6.2).
4. Redraws the full dock surface.
5. Calls `VYOMA_DRAW:flush`.

The focused slot is rendered with a distinct background colour `0x45475AFF` (Catppuccin
Surface2, slightly brighter than the unfocused slot background) and a larger running
indicator dot. The focused slot label uses colour `0xCDD6F4FF` in bold weight (or, in v1
without bold font support, with an underline drawn as a 1px `fill_rect` below the text).

### 5.3 No-App-Focused State

If `focus_changed:` is received with an empty app name (supervisor sends this when no app
holds focus, e.g. after the focused app terminates and before focus is reassigned), dock
resets all `is_focused` flags and redraws with no highlighted slot.

---

## 6. Ctrl-Tab App Switcher

### 6.1 Switcher State Machine

The switcher is a modal state managed by the dock app. It has three states:

```
Inactive → [Ctrl-Tab received] → Active (index=0 in MRU)
Active   → [Ctrl-Tab received] → Active (index += 1, wrap to 0 at end)
Active   → [Ctrl-Shift-Tab received] → Active (index -= 1, wrap to end at 0)
Active   → [Ctrl released (KeyUp event)] → Committing
Committing → [focus command sent] → Inactive
```

### 6.2 MRU List Management

The dock maintains a `mru_list: Vec<String>` of app names in most-recently-focused order.
The list:

- Contains only apps in `Running`, `Suspended`, or `Background` lifecycle phases.
  Apps in `Launching`, `Terminating`, or `Terminated` are excluded from the MRU list.
- Is updated when `VYOMA_DOCK:focus_changed:<app_name>` is received:
  - Remove `<app_name>` from wherever it currently appears in `mru_list`.
  - Prepend `<app_name>` to the front (index 0 = most recently focused).
- Is updated when `VYOMA_DOCK:app_exited:<app_name>` is received:
  - Remove `<app_name>` from `mru_list`.
- Is updated when `VYOMA_DOCK:app_launched:<app_name>` is received:
  - Append `<app_name>` to the end of `mru_list` (newly launched apps appear last until
    they receive focus).

Minimizing an app does NOT affect its MRU position. A minimized app is still in the MRU
list at whatever position it held before being minimized. This matches macOS Cmd-Tab
behaviour where minimized apps appear in the switcher.

### 6.3 Switcher Overlay Rendering

When the switcher becomes Active, the dock:

1. Issues `VYOMA_DRAW:resize_surface:<logical_w>,<logical_h>` to expand to full screen.
2. Fills the full surface with dock background: `VYOMA_DRAW:fill_rect:0,0,<w>,<h>,0x1E1E2ECC`
   (semi-transparent dark overlay; alpha `0xCC` = ~80% opacity).
3. Draws the dock strip at the bottom of the surface as normal (unchanged from non-switcher
   rendering, occupying the bottom 48pt of the surface).
4. Draws the switcher panel centred horizontally between `y = 24` (below chrome menu bar)
   and `y = logical_h - 48` (above dock strip):
   - Panel background: `fill_rect` at `(logical_w/2 - panel_w/2, panel_y, panel_w, panel_h)`
     in colour `0x313244FF` (Catppuccin Surface0).
   - Panel width: `min(logical_w - 64, max(320, mru_list.len() * 96))` logical points.
   - Panel height: 96 logical points (fixed in v1; single row of slots).
   - Panel y: `(logical_h - 48 - 24 - 96) / 2 + 24` (vertically centred in the usable area).
5. For each app in `mru_list`, draws a slot within the panel:
   - Slot width: 88 logical points, with 4pt gap between slots.
   - Slot `fill_rect` background: `0x45475AFF` if selected (current `mru_index`), else
     `0x313244FF`.
   - App name label: `draw_text` at slot centre, size `s`, colour `0xCDD6F4FF`.
   - Coloured rectangle placeholder (no icons in v1): 48×48pt rectangle centred in slot,
     using a colour derived from a hash of the app name (deterministic, repeatable).
6. Calls `VYOMA_DRAW:flush`.

On each subsequent Ctrl-Tab event while switcher is Active, dock increments `mru_index`,
redraws only the panel region (or the full surface for simplicity in v1), and flushes.

### 6.4 Committing a Selection

The switcher commits when Ctrl is released. Dock receives a `VYOMA_DOCK:key:ctrl_up` event
(see §7 for how this is delivered). On commit:

1. Look up `app_name = mru_list[mru_index]`.
2. Write to stdout: `@supervisor: focus <app_name>`.
3. Issue `VYOMA_DRAW:resize_surface:<logical_w>,48` to collapse back to dock surface.
4. Redraw the dock strip.
5. Call `VYOMA_DRAW:flush`.
6. Transition switcher state to Inactive.

If `mru_list` is empty (no apps in switchable states), steps 1 and 2 are skipped.

### 6.5 Switcher Cancellation

If the user presses Escape while the switcher is Active, dock:

1. Writes nothing to the supervisor (no focus change).
2. Collapses the surface to `48pt` height.
3. Redraws the dock strip.
4. Calls `VYOMA_DRAW:flush`.
5. Transitions switcher state to Inactive.

The Escape key event is delivered to dock via the same provisional global shortcut mechanism
described in §7, since Escape should cancel the switcher regardless of which app notionally
holds focus while the switcher is open.

---

## 7. Input Handling and Provisional Global Shortcut Mechanism

### 7.1 The Problem

R35 (Global Shortcuts & Hotkeys) defines the formal system for apps to register keyboard
shortcuts that the supervisor intercepts before routing to the focused app. R35 is not yet
specced. But the dock needs Ctrl-Tab delivery before R35 exists, because the App Switcher
is a core desktop interaction that cannot be deferred.

This section defines a **provisional mechanism** for keyboard intercept that is compatible
with the eventual R35 design and does not require structural changes to implement R35 later.

### 7.2 Provisional Intercept Table

The supervisor maintains a static `GLOBAL_SHORTCUTS: &[(KeyCombo, &str)]` table, consulted
on every raw keypress before the normal focused-app routing. This table is defined at compile
time in `supervisor/src/keyboard.rs`:

```rust
// supervisor/src/keyboard.rs

pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

/// Static global shortcut table.
/// Evaluated before focused-app keyboard routing on every keypress.
/// When a combo matches, the key event is sent to `target_app` instead of the focused app.
/// The key event is NOT sent to the focused app if it matches a global shortcut.
pub static GLOBAL_SHORTCUTS: &[(KeyCombo, &str)] = &[
    // Ctrl-Tab → dock (switcher forward)
    (KeyCombo { ctrl: true, shift: false, alt: false, key: Key::Tab }, "dock"),
    // Ctrl-Shift-Tab → dock (switcher backward)
    (KeyCombo { ctrl: true, shift: true,  alt: false, key: Key::Tab }, "dock"),
    // Ctrl key-up event → dock (to detect switcher commit)
    // Note: key-up events for modifier keys are always routed to dock when switcher is Active.
    // This is handled by a separate dock_active flag (see §7.3).
];
```

The supervisor's TTY input thread evaluates `GLOBAL_SHORTCUTS` on each keypress. If the
combo matches, the key event is written to the target app's (`dock`'s) stdin as:

```
VYOMA_DOCK:key:<modifiers>_<keyname>
```

Examples:
- Ctrl-Tab arrives as: `VYOMA_DOCK:key:ctrl_tab`
- Ctrl-Shift-Tab arrives as: `VYOMA_DOCK:key:ctrl_shift_tab`
- Ctrl key-up (when switcher active): `VYOMA_DOCK:key:ctrl_up`
- Escape (when switcher active): `VYOMA_DOCK:key:escape`

When a global shortcut fires, the key event is NOT forwarded to the focused app. This is the
correct behaviour: the focused app should never see a Ctrl-Tab that the dock is handling.

### 7.3 Ctrl Key-Up Delivery

Detecting "Ctrl released" requires the supervisor to track the state of modifier keys and
deliver key-up events. In the current VyomaOS TTY input model (raw mode `/dev/tty`), key-up
events for modifier keys are not directly reported by the Linux TTY driver in the ASCII escape
sequence model used by QEMU's virtio-console. Instead, the supervisor uses a **Ctrl timeout**
heuristic:

After a Ctrl-Tab or Ctrl-Shift-Tab key event is delivered to dock, the supervisor starts a
**modifier hold timer** of 500ms. If no further Ctrl-Tab or Ctrl-Shift-Tab event arrives
within 500ms, the supervisor synthesises a `VYOMA_DOCK:key:ctrl_up` event to dock's stdin.

This is an approximation that works for human interaction speeds (a user who presses Ctrl-Tab
and releases Ctrl within 500ms gets correct switcher commit behaviour). The timer is reset on
each new Ctrl-Tab event, allowing the user to hold Ctrl and tab through apps without
committing prematurely. R35 will replace this with proper modifier-key tracking using the
`TIOCLINUX` or `KDGKBTYPE` ioctl on the kernel side.

The 500ms timer is driven by the supervisor's main event loop, not a separate thread. On each
event loop iteration, the supervisor checks:

```rust
if dock_ctrl_held && last_ctrl_tab.elapsed() >= Duration::from_millis(500) {
    dock_router.send_supervisor_event("VYOMA_DOCK:key:ctrl_up\n");
    dock_ctrl_held = false;
}
```

### 7.4 Escape Key During Switcher

When the switcher is Active (`dock_ctrl_held = true`), Escape is also routed to dock
rather than the focused app. The supervisor checks: if `dock_ctrl_held` is true, any
Escape keypress is sent to dock as `VYOMA_DOCK:key:escape`. This requires one additional
check in the TTY input routing path before the normal focused-app dispatch.

### 7.5 Compatibility with R35

The provisional mechanism is designed to be replaced in R35. R35 will introduce a dynamic
shortcut registration API where an app sends `@supervisor: register_shortcut <combo> <app>`
and the supervisor adds the combo to a runtime-modifiable table. The static
`GLOBAL_SHORTCUTS` table in v1 becomes the seed content of that runtime table. R35 can
migrate the Ctrl-Tab entries to dock's `register_shortcut` call at startup without
changing the shortcut routing semantics.

---

## 8. Dock Routing Protocol (Supervisor-to-Dock Events)

### 8.1 App Lifecycle Events

The supervisor's `dock_router.rs` sends lifecycle events to dock's stdin whenever an app's
`LifecycleState` changes:

```
VYOMA_DOCK:app_launched:<app_name>
VYOMA_DOCK:app_exited:<app_name>
VYOMA_DOCK:app_lifecycle:<app_name>,<state>
```

`app_launched` is sent when an app transitions from `Launching` to `Running` (not on
`Launching` itself, to avoid showing a slot that may immediately disappear for short-lived
apps). `app_exited` is sent when an app reaches `Terminated`. `app_lifecycle` is sent on
every other state transition (e.g. `Running` → `Suspended`), with `<state>` being one of:
`launching`, `running`, `suspended`, `background`, `terminating`, `terminated`.

### 8.2 Focus Events

```
VYOMA_DOCK:focus_changed:<app_name>
```

Sent by the supervisor to dock's stdin whenever focus changes, using the same dispatch
point as the chrome `focus_changed` event (both chrome_router and dock_router are called
from the same focus-change code path in `ipc.rs`).

### 8.3 Initial App List

On dock startup, the supervisor sends a snapshot of all currently running apps to dock's
stdin, before dock enters its event loop. This allows dock to render the correct initial
state even if it starts after other apps are already Running:

```
VYOMA_DOCK:initial_list:<app1>,<app2>,<app3>,...
VYOMA_DOCK:focused:<app_name>
```

`initial_list` is a comma-separated list of running app names (those in `Running`,
`Suspended`, or `Background` state). `focused` gives the current focused app name.
These two events are sent consecutively to dock's stdin during dock's startup queue drain
(§dock_router queue), generated by the supervisor when dock transitions to `Running` and
the queue is drained.

### 8.4 Tick Events

```
VYOMA_DOCK:tick:<iso_timestamp>
```

Sent once per second by the same tick mechanism introduced for chrome in R23. Dock uses
this to drive the 2-second Terminated-slot display timer (§3.4) by incrementing a counter
per terminated slot on each tick.

### 8.5 Display Resize

```
VYOMA_DOCK:display_resized:<new_w>,<new_h>
```

Sent by the supervisor when the logical screen dimensions change. Dock must recalculate its
`win_y` (which is `new_h - 48`) and its slot layout geometry, then resize its surface and
redraw.

---

## 9. VYOMA_DOCK: Protocol Reference

### 9.1 Lines from App Stdout (routed to dock by supervisor)

| Command | Format | Description |
|---------|--------|-------------|
| `dock_badge` | `VYOMA_DOCK:dock_badge:<count>` | (reserved for R73; dock badge count) |
| `dock_progress` | `VYOMA_DOCK:dock_progress:<pct>` | (reserved; progress bar in dock icon) |

These are the only lines that ordinary apps may send with `VYOMA_DOCK:` prefix. In v1 they
are received by dock but result in no visual action (reserved for future rounds). All
`VYOMA_DOCK:` lines are intercepted by `dock_router.rs` and stripped from normal stdout
processing (they are not displayed on serial console and not passed to the draw subsystem).

### 9.2 Supervisor-Generated Events to Dock Stdin

| Event | Format | Description |
|-------|--------|-------------|
| App launched | `VYOMA_DOCK:app_launched:<app>` | App reached Running state |
| App exited | `VYOMA_DOCK:app_exited:<app>` | App reached Terminated state |
| Lifecycle change | `VYOMA_DOCK:app_lifecycle:<app>,<state>` | Other state transitions |
| Focus changed | `VYOMA_DOCK:focus_changed:<app>` | Focused app changed |
| Initial list | `VYOMA_DOCK:initial_list:<app1>,...` | Snapshot of running apps on startup |
| Initial focus | `VYOMA_DOCK:focused:<app>` | Currently focused app on startup |
| Tick | `VYOMA_DOCK:tick:<iso_ts>` | 1-second timer tick |
| Display resize | `VYOMA_DOCK:display_resized:<w>,<h>` | Screen geometry changed |
| Key event | `VYOMA_DOCK:key:<combo>` | Global shortcut or modifier key event for dock |

### 9.3 Commands from Dock Stdout (processed by supervisor)

| Command | Format | Effect |
|---------|--------|--------|
| Focus transfer | `@supervisor: focus <app>` | Standard WM focus (R21); used by switcher |
| Resize surface | `VYOMA_DRAW:resize_surface:<w>,<h>` | Expand/collapse to switcher overlay size |

---

## 10. Window Clip at Bottom (Bottom-Edge Exclusion Zone)

### 10.1 Exclusion Zone Geometry

Just as chrome reserves the top 24 logical points from non-chrome apps (R23 §10), dock
reserves the bottom 48 logical points from non-dock, non-chrome apps. The combined exclusion
zones mean the usable application window region is:

```
┌─────────────────────────────────────────────────────┐  y=0
│  Menu Bar (chrome)              24pt reserved        │
├─────────────────────────────────────────────────────┤  y=24
│                                                     │
│  Application Windows                                │
│  (y: 24 to logical_h-48 in logical points)          │
│                                                     │
├─────────────────────────────────────────────────────┤  y=logical_h-48
│  Dock                           48pt reserved        │
└─────────────────────────────────────────────────────┘  y=logical_h
```

### 10.2 Bottom-Edge Clip in `flush_pass`

The bottom-edge clip is applied in `supervisor/src/display/compositor.rs` in the
`flush_pass` function, parallel to the top-edge clip for chrome. The clip is applied to
all surfaces that are not the dock app and not the chrome app.

The clip in physical pixels: `dock_clip_y_px = (logical_h - 48) * scale_factor`.

For a non-dock, non-chrome surface at destination position `(dest_x_px, dest_y_px)` with
physical dimensions `(surface_width_px, surface_height_px)`:

```rust
// Bottom-edge clip in flush_pass:
let blittable_h_from_bottom = if is_dock_or_chrome {
    surface_height_px
} else {
    let max_dest_bottom = dock_clip_y_px as i32;
    let dest_bottom = dest_y_px + surface_height_px as i32;
    if dest_bottom <= max_dest_bottom {
        surface_height_px   // fully above dock; no bottom clip needed
    } else if dest_y_px >= max_dest_bottom {
        0  // entire surface is in dock zone; blit nothing
    } else {
        (max_dest_bottom - dest_y_px) as u32  // partially overlapping; clip rows
    }
};
// Combined with the top-edge src_y_offset from R23:
blit_surface(
    fb, &surface.buffer,
    dest_x_px, effective_dest_y,
    surface_width_px, blittable_h_from_bottom.min(
        blittable_h_from_top  // the R23 top-edge clip height already computed
    ),
    src_y_offset,
);
```

**Critical note on src_y_offset**: The bottom clip does NOT require a `src_y_offset`
adjustment, because it clips rows from the *end* of the source surface (the bottom rows),
not the beginning. The `src_y_offset` correction from R23 applies only to the top-edge clip.
Bottom-edge clipping simply reduces the `height` parameter passed to `blit_surface`, leaving
`src_y_offset` unchanged. This is the exact dual of the top-edge clip: top-clip skips
leading source rows (adjusts `src_y_offset`); bottom-clip truncates trailing source rows
(adjusts `height`).

### 10.3 Combined Top and Bottom Clip

After R23 and R24, the compositor applies two simultaneous clips to every non-privileged
non-chrome, non-dock surface:

1. **Top clip**: `effective_dest_y = dest_y_px.max(chrome_height_px)`; adjusts `src_y_offset`.
2. **Bottom clip**: `blittable_h = min(original_h - src_y_offset, dock_clip_y_px - effective_dest_y).max(0)`.

If both clips apply simultaneously (a surface spans from above chrome to below dock, which
would require a surface taller than `logical_h - 72` points), both reductions apply. The
`blit_surface` call receives the intersection of the clipped range.

### 10.4 Defence in Depth for Dock Region

Like chrome, dock uses two independent mechanisms to prevent other apps from drawing in its
region:

**Mechanism 1 — Compositor Z-order**: Dock's `z_order = 65534` places it second-to-last
in the blit order. Any app pixel that leaks into the bottom 48px is overwritten by dock
on the same frame.

**Mechanism 2 — Bottom-edge clip**: The `flush_pass` clip prevents non-dock surfaces from
blitting into the bottom `DOCK_HEIGHT_PX` rows. Combined with the Z-order guarantee, the
dock region is protected by two independent layers.

---

## 11. WIT Interface `vyoma:dock@1.0.0`

The dock WIT interface is defined here for documentation and forward compatibility. Like
chrome's WIT interface in R23, no host or guest bindings are generated in R24. The interface
documents the intended structured API for future WIT-capable dock interactions.

```wit
// wit/dock.wit
package vyoma:dock@1.0.0;

interface dock-state {
    /// Get the list of running app names in dock order (spawn order).
    get-running-apps: func() -> list<string>;

    /// Get the name of the currently focused application.
    get-focused-app: func() -> option<string>;

    /// Get the MRU list of app names (most-recently-focused first).
    get-mru-list: func() -> list<string>;

    /// Query the lifecycle state of a specific app (as seen by the dock).
    get-app-lifecycle: func(app-name: string) -> option<dock-lifecycle>;

    /// Returns true if the app switcher overlay is currently visible.
    is-switcher-active: func() -> bool;
}

/// Simplified lifecycle as reported by the dock.
enum dock-lifecycle {
    launching,
    running,
    suspended,
    background,
    terminating,
    terminated,
}

world dock-world {
    import dock-state;
}
```

This WIT file is checked into `wit/dock.wit` but is not wired to code generation in R24.

---

## 12. Platform Matrix

Dock behaviour per platform profile:

| Platform        | Dock | Height | App Switcher | Running Indicators |
|-----------------|------|--------|--------------|--------------------|
| `desktop-full`  | Yes  | 48pt   | Ctrl-Tab     | Yes                |
| `mobile`        | No   | —      | No           | —                  |
| `server-headless` | No | —    | No           | —                  |
| `iot-edge`      | No   | —      | No           | —                  |
| `robotics-rt`   | No   | —      | No           | —                  |
| `mcu-minimal`   | No   | —      | No           | —                  |

### 12.1 Mobile (No Dock)

On the `mobile` platform profile, the dock is not included in `boot.toml`. The mobile
chrome (R23 §12.1) positions its status bar at the bottom of the screen at 20pt height. If
dock were also present on mobile, their bottom-position would conflict geometrically. The
mobile form factor does not benefit from a persistent dock (on iOS, the dock appears only
as part of app launch and the Home Screen). R33 (Touch & Stylus Input) will introduce a
mobile-appropriate launcher mechanism.

On mobile, because the dock app is absent, the `dock_clip_y_px` bottom-edge clip in the
compositor is set to `logical_h` (no-op bottom clip). The supervisor must handle the absence
of the dock app gracefully: `dock_app_name: Option<String>` is `None` on mobile, and the
compositor skips the bottom-edge clip when `dock_app_name.is_none()`.

### 12.2 Server, IoT, Robotics, MCU

On headless platforms, the `dock` app is not in `boot.toml`. The `dock = true` flag is
absent. The supervisor's `dock_router` discards all `VYOMA_DOCK:` lines from apps silently
(same pattern as chrome on headless, R23 §12.2). Apps that issue `VYOMA_DOCK:dock_badge:3`
on a headless server run without error; the dock_router logs at debug level and discards.

---

## 13. File Layout

### 13.1 New Files

```
apps/dock/
  Cargo.toml             — [package], [[bin]], [dependencies]
  vyoma.toml             — capabilities: stdio, display, shell; window: space=0, z=65534
  src/
    main.rs              — event loop: read stdin, dispatch to handlers (<=500 lines)
    slots.rs             — DockSlot, DockState, spawn-order list, MRU list management
    draw.rs              — draw_dock_strip(), draw_slot(), draw_running_dot()
    switcher.rs          — switcher state machine, draw_switcher_overlay()
    state.rs             — DockState struct: slots, mru_list, switcher_index, switcher_active

wit/
  dock.wit               — vyoma:dock@1.0.0 WIT definition (documentation only in R24)
```

### 13.2 Modified Supervisor Files

```
supervisor/src/
  dock_router.rs         NEW: intercept VYOMA_DOCK: from stdout; route to dock stdin
                         startup queue (capacity 64, drop-oldest); drain on dock Running
                         send lifecycle/focus/initial_list/tick events to dock stdin
  process.rs             parse `dock = true` field from boot.toml; register dock_app_name
  manifest.rs            parse `dock = true`; validate at most one dock app
  ipc.rs                 add dock lifecycle/focus event dispatch; Ctrl-Tab modifier hold timer
  keyboard.rs            GLOBAL_SHORTCUTS table; Ctrl-Tab intercept; dock_ctrl_held flag
  display/compositor.rs  bottom-edge clip (dock_clip_y_px); combined top+bottom clip logic

base/rootfs.sh           add dock.wasm to initramfs; add to boot.toml with dock=true
```

### 13.3 Line Budget

| File | Estimated lines |
|------|----------------|
| `apps/dock/src/main.rs` | ~180 |
| `apps/dock/src/slots.rs` | ~120 |
| `apps/dock/src/draw.rs` | ~150 |
| `apps/dock/src/switcher.rs` | ~130 |
| `apps/dock/src/state.rs` | ~80 |
| `supervisor/src/dock_router.rs` | ~130 |
| `supervisor/src/keyboard.rs` (modified) | ~60 new lines |
| Total dock app | ~660 → split across 5 files, each under 500 ✓ |

---

## 14. Invariants and Guarantees

| ID  | Invariant |
|-----|-----------|
| I1  | At most one app has `dock = true` in `boot.toml`. |
| I2  | The dock app's `z_order` is 65534; chrome's is 65535; no ordinary app may reach >= 65532. |
| I3  | The dock app's window is always at `y = logical_h - 48`; it is the supervisor that resolves the manifest `y = 0` sentinel. |
| I4  | All `VYOMA_DOCK:` lines from app stdout are stripped before reaching the display subsystem. |
| I5  | Supervisor-generated dock events are written directly to dock's stdin, not routed through app stdout interception. |
| I6  | Ctrl-Tab is consumed by the supervisor before being forwarded to the focused app; the focused app never sees Ctrl-Tab while the global shortcut table includes it. |
| I7  | The 64-message startup queue for dock uses drop-oldest overflow policy, same as chrome. |
| I8  | On headless and mobile platforms, `VYOMA_DOCK:` lines from apps are silently discarded. |
| I9  | The bottom-edge clip in `flush_pass` does not require a `src_y_offset` adjustment (unlike the top-edge clip); it reduces `height` only. |
| I10 | The `dock_ctrl_held` flag is cleared unconditionally when dock transitions away from Running, preventing phantom switcher state on dock restart. |
| I11 | Chrome (z=65535) is always blitted after dock (z=65534); during switcher overlay mode, chrome still appears above the switcher overlay because dock's z remains 65534. |
| I12 | MRU list entries are only apps in Running, Suspended, or Background states. |

---

## 15. Open Questions and Deferred Decisions

1. **Dock with multiple monitors (R+future)**: On a dual-monitor setup, does the dock appear
   on only the primary monitor, or on each monitor's bottom edge? Deferred. The
   `display_resized` event carries `<w>,<h>` for the primary logical screen only.

2. **Dock app crash and restart**: If dock crashes, the bottom 48px shows the fallback from
   the dock_clip_y_px compositor layer (raw framebuffer or whatever was previously there).
   The supervisor should restart dock with `restart = "always"` and a 500ms backoff. This
   requires the same `dock_surface_ready: bool` fallback fill introduced for chrome (fill the
   bottom 48px with `0x1E1E2EFF` when no dock surface is registered). Deferred to R24 FINAL.

3. **Pinned apps**: Users may want apps that do not currently have a running process to
   appear as launchable slots in the dock. This requires the launch services model (R74).
   For now, the dock is "running apps only." The `VYOMA_DOCK:dock_badge` reservation (§9.1)
   anticipates the pinned-slot extension.

4. **Ctrl-Tab modifier ambiguity in raw TTY**: The `ctrl_up` detection via 500ms timeout is
   an approximation. On real keyboard hardware exposed through a Linux VT, modifier key events
   are tracked differently from character keypresses. R35 should specify the authoritative
   modifier tracking mechanism for the VyomaOS TTY input model.

5. **Dock height on HiDPI**: At `scale_factor = 2.0`, the dock occupies 96 physical pixels.
   At 1x it occupies 48 physical pixels. The dock renders all geometry in logical points and
   the compositor converts to physical pixels. No special handling is needed; the existing
   `scale_factor` conversion in `flush_pass` handles this automatically.

---

## 16. Implementation Sequence

### Step 1 — Supervisor: parse `dock = true`, register dock app name

**Files**: `supervisor/src/manifest.rs`, `supervisor/src/process.rs`

Mirror the chrome registration work from R23. Add `dock: Option<bool>` to `AppManifest`.
Store `dock_app_name: Option<String>` in supervisor state. If two apps declare `dock = true`,
log error and keep only the first.

**Verification**: `boot.toml` with `dock = true` on the shell app produces
`[dock] registered dock app: shell` in supervisor log.

### Step 2 — Supervisor: dock_router.rs stub + startup queue

**Files**: `supervisor/src/dock_router.rs` (new)

Mirror `chrome_router.rs`. Create `DockRouter` struct with startup queue (capacity 64,
drop-oldest). `route_line(sender, line)` intercepts `VYOMA_DOCK:` prefix. `drain_queue()`
delivers queued messages when dock transitions to Running. Unit test queue overflow and drain
ordering.

### Step 3 — Supervisor: bottom-edge clip in compositor flush_pass

**Files**: `supervisor/src/display/compositor.rs`

Add `dock_clip_y_px` parameter to `flush_pass`. Apply bottom-clip to non-dock, non-chrome
surfaces: reduce `height` without adjusting `src_y_offset`. Unit test that a surface whose
bottom overlaps the dock zone is truncated at the correct row. Confirm top+bottom combined
clip handles a surface spanning both exclusion zones correctly.

### Step 4 — Supervisor: Ctrl-Tab intercept in keyboard.rs

**Files**: `supervisor/src/keyboard.rs`

Add `GLOBAL_SHORTCUTS` static table with Ctrl-Tab and Ctrl-Shift-Tab entries targeting
`dock`. Add `dock_ctrl_held: bool` and `last_ctrl_tab: Instant` to keyboard state. On
matching keypress, route to dock's stdin as `VYOMA_DOCK:key:ctrl_tab`. Check modifier
hold timer on each event loop iteration.

**Verification**: Send Ctrl-Tab to the QEMU console. Confirm dock's stdin receives
`VYOMA_DOCK:key:ctrl_tab`. Confirm the focused app's stdin does NOT receive Ctrl-Tab.

### Step 5 — Build dock WASM app (dock strip, no switcher)

**Files**: `apps/dock/src/`

Implement in order: `state.rs` → `slots.rs` → `draw.rs` → `main.rs`. Initial version
draws an empty dock strip (no slots yet). Wire to `VYOMA_DOCK:app_launched` and
`VYOMA_DOCK:app_exited` to add/remove slots.

**Verification**: Boot VyomaOS. Confirm dock strip visible at bottom. Confirm slots appear
for running apps.

### Step 6 — Wire lifecycle and focus events from supervisor

**Files**: `supervisor/src/ipc.rs`

On every lifecycle state transition that dock cares about, call `dock_router.send_supervisor_event`.
On every focus change, call dock_router and chrome_router (both from the same dispatch point).
On dock startup (transition to Running + queue drain), inject `initial_list` and `focused`.

### Step 7 — Implement App Switcher overlay

**Files**: `apps/dock/src/switcher.rs`

Implement the switcher state machine. On `VYOMA_DOCK:key:ctrl_tab`: expand surface to full
screen, draw overlay. On repeated Ctrl-Tab: cycle selection. On `ctrl_up`: commit and
collapse. On `escape`: cancel and collapse.

**Verification**: Press Ctrl-Tab in QEMU. Confirm switcher overlay appears. Cycle through
apps. Release Ctrl (wait 500ms). Confirm focus changes to selected app.

---

## 17. Testing Plan

### Unit Tests (supervisor)

| Test | File | What it verifies |
|------|------|-----------------|
| `dock_router_queue_drop_oldest` | `tests/dock_router.rs` | Queue overflow drops oldest |
| `dock_router_drain_order` | `tests/dock_router.rs` | Drain delivers messages in arrival order |
| `compositor_bottom_clip_partial` | `tests/compositor.rs` | Surface partially in dock zone truncated |
| `compositor_bottom_clip_full` | `tests/compositor.rs` | Surface entirely in dock zone: blit nothing |
| `compositor_combined_clip` | `tests/compositor.rs` | Top + bottom clip applied simultaneously |
| `compositor_src_y_offset_unchanged_by_bottom_clip` | `tests/compositor.rs` | Bottom clip does not alter src_y_offset |
| `keyboard_ctrl_tab_intercepted` | `tests/keyboard.rs` | Ctrl-Tab goes to dock, not focused app |
| `keyboard_ctrl_up_timeout` | `tests/keyboard.rs` | ctrl_up synthetic event after 500ms |
| `keyboard_escape_during_switcher` | `tests/keyboard.rs` | Escape routed to dock when dock_ctrl_held |

### Integration Tests (QEMU smoke)

- Boot: dock strip visible at bottom within 5 seconds.
- Running apps show slots in spawn order.
- Focused app's slot has distinct background.
- Exited app's slot disappears within 2 tick events.
- Ctrl-Tab produces switcher overlay visible above dock strip.
- Switching apps via Ctrl-Tab changes focus correctly.
- No visual corruption in bottom 48px from non-dock apps.

### Manual Verification Checklist

- [ ] Dock strip visible at bottom of QEMU GUI window (`make run-gui`).
- [ ] Dock background colour is `#1E1E2E`.
- [ ] Each running app has a named slot with a coloured rectangle placeholder.
- [ ] Running indicator dot appears below each Running app's slot.
- [ ] Focused app's slot has brighter background (`#45475A`).
- [ ] After `focus calculator` in shell, calculator slot becomes highlighted in dock.
- [ ] Pressing Ctrl-Tab shows the switcher overlay.
- [ ] Repeated Ctrl-Tab cycles selection in MRU order.
- [ ] Releasing Ctrl (waiting ~600ms) commits the selection and hides the overlay.
- [ ] No visual corruption in dock region from other apps drawing at `y > logical_h-48`.
- [ ] Menu bar (chrome) remains visible above the switcher overlay during Ctrl-Tab.
