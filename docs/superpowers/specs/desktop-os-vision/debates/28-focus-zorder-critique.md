# CRITIQUE — 5 Blocking Issues

**Spec**: Round 28 — Focus Management & Z-Order  
**Reviewer**: Critic Agent  
**Date**: 2026-05-30

---

## B1: `focus_transfer` Holds Apps-Map Lock While Calling `notify_chrome` / `notify_dock` — Potential Deadlock with Compositor

**Problem**

Section 3.2 (`focus_transfer` Step 8) calls `notify_chrome(...)` and `notify_dock(...)`
while the `apps: Arc<Mutex<HashMap>>` lock is held by the IPC handler thread (stated
in §13.3: "protected by the apps: Arc<Mutex<HashMap>> (the apps map lock, held by the
IPC handler thread at this point in execution)"). The drop-oldest ring buffer (64 slots,
R23) is the delivery mechanism for chrome and dock notifications.

The chrome app and the dock app write their own `AppState` entries (e.g., updating
`surface_dirty`, processing `VYOMA_CHROME:focus_changed`). If either app's output-reading
thread is waiting to acquire the apps-map lock at the moment `notify_chrome` attempts
to push into the ring — and the ring's push path tries to acquire a lock that the output
thread's path also holds — a lock-order inversion can stall indefinitely.

Concretely: the ring buffer's `push` is described as non-blocking (drop-oldest), but
the ring's backing store is a `Mutex<VecDeque<...>>` (implied from R23). If the
compositor thread reads from that ring while also holding the apps-map lock (e.g., to
check `surface_dirty`), and the IPC thread holds the apps-map lock while pushing into
the ring, we have ABBA:

```
IPC thread:        apps_lock → ring_lock
Compositor thread: apps_lock → ring_lock  (if compositor reads ring under apps_lock)
```

Even if the current compositor doesn't hold apps_lock while reading the ring, this
assumption is fragile and undocumented. The spec has already encountered an ABBA deadlock
in commit `55fd121` ("eliminate ABBA deadlock in flush pass by using z from apps_sorted
snapshot"). The same failure pattern is being re-introduced here.

**Proposed Fix**

Decouple notification dispatch from the apps-lock critical section. Use a
**pending-notifications queue** on the `SupervisorState`:

```rust
pub struct SupervisorState {
    // ...
    pub pending_focus_notifications: Vec<PendingFocusNotif>,
}

pub enum PendingFocusNotif {
    ChromeFocusChanged { app_name: String, app_title: String },
    DockFocusChanged { app_name: String },
    StageFocusChanged { app_name: String },
    AppFocusGained { app_name: String, reason: FocusReason },
    AppFocusLost { app_name: String, reason: FocusReason },
}
```

`focus_transfer` appends to `pending_focus_notifications` (apps-lock still held —
this is just a `Vec::push`). After `focus_transfer` returns, the IPC handler releases
the apps-lock. The supervisor main loop then drains `pending_focus_notifications` and
dispatches to chrome/dock rings and app stdin pipes — **without** holding the apps-lock.
This is the same pattern as `pending_resize` (R21) and `pending_focus` (R25), which
are both proven lock-safe in VyomaOS.

`FOCUSED_APP.store()` and `LAST_FOCUSED.lock()` are still done inside `focus_transfer`
(they do not require the apps-map lock). Only the notification dispatch is deferred.

---

## B2: `compact_z_order` Called Inside `focus_transfer` Which Is Called Inside Apps-Lock — O(N) Under Lock on Every Keystroke Path

**Problem**

`focus_transfer` (§3.2 Step 3) calls `compact_z_order`, which iterates all apps twice
(space-N pass + space-0 pass), sorts them, and rewrites z-values. This is O(N log N)
where N is the number of apps in the active space.

`focus_transfer` is called from:
1. `route_keyboard` indirectly? No — focus is not changed per keystroke. But it IS
   called from the mouse-click handler (R32) and from IPC commands. More importantly,
   it is called from `on_stage_activated` (§9.1) which fires during every Stage Manager
   stage swap, which can be rapid (e.g., animated swipe gesture generating multiple
   stage activations per second during a drag).
2. `on_app_exit` is called synchronously in the app lifecycle handler. If many apps
   exit rapidly (e.g., `kill all` supervisor command), `compact_z_order` is called
   N times for N exits, each under the apps-lock — O(N²) total work under a single
   global lock.

The real problem is structural: **`compact_z_order` is a write to all apps' z-values,
but `focus_transfer` is meant to be a lightweight focus-state update.** Conflating them
inside the apps-lock is unnecessarily expensive and blocks the compositor thread from
acquiring the apps-lock for its own reads.

**Proposed Fix**

Extract the z-order bump (`set z = max + 1`) from `focus_transfer` and schedule the
compaction asynchronously, the same way `pending_resize` defers surface reallocation
to the compositor tick:

```rust
// In SupervisorState or SpaceRegistry:
pub pending_compact_spaces: AtomicU8; // bitmask of spaces needing compaction (spaces 1-8)
pub pending_compact_space0: AtomicBool; // separate flag for space-0
```

`focus_transfer` only:
1. Sets `app.win_z.set(target_space, max_z + 1)` — O(N) scan, no rewrite, still fast
2. Sets `pending_compact_spaces |= 1 << (target_space - 1)` — O(1) atomic write

The compositor `vsync_tick` checks `pending_compact_spaces` at the start of each frame
(same location as `PENDING_SPACE_SWITCH` check) and calls `compact_z_order` for each
flagged space under `vsync_lock.write()`. This defers the O(N log N) sort to the
compositor thread where it already holds the write-lock, and removes it from the
hot IPC path.

For `on_app_exit` (N rapid exits): the bitmask means even if 10 apps exit, only one
compaction per space per compositor frame is performed — O(N) total instead of O(N²).

---

## B3: `LAST_FOCUSED.lock()` Inside `focus_transfer` Creates a New Untracked Lock in the Lock Order

**Problem**

Section 2.2 introduces `LAST_FOCUSED: Lazy<Mutex<HashMap<u8, String>>>` as a module-level
static Mutex. Section 3.2 calls `LAST_FOCUSED.lock().unwrap()` inside `focus_transfer`,
which is invoked while the apps-map lock is held (§13.3). This adds a new lock to the
acquisition order:

```
apps_lock → last_focused_lock
```

Section 9.2 shows `focus_transfer` also called from `on_stage_activated`, which may be
called from the compositor thread (R26 stage swap during vsync) — meaning the compositor
thread would acquire `apps_lock → last_focused_lock`. Meanwhile, any code path that
acquires `last_focused_lock` first (e.g., `space_switch_commit` reading LAST_FOCUSED
to decide focus target, and then later acquiring apps_lock to call `focus_transfer`)
creates ABBA:

```
Compositor thread (stage activation): apps_lock → last_focused_lock
IPC thread (space_switch_commit):     last_focused_lock → apps_lock
```

`space_switch_commit` in §7.2 calls `LAST_FOCUSED.lock()` to read the target, then
calls `focus_transfer` which acquires apps_lock — this IS the ABBA ordering if the
compositor simultaneously runs `on_stage_activated`.

This is the same class of deadlock that was fixed in commit `55fd121`.

**Proposed Fix**

Remove `LAST_FOCUSED` as a standalone Mutex-protected static. Instead, store
`last_focused: HashMap<u8, String>` directly inside `SpaceRegistry`, which is already
protected by the apps-map lock when accessed through the normal IPC/compositor paths.
Alternatively, embed `last_focused_app: Option<String>` directly on each space's
entry in a `spaces_map: HashMap<u8, SpaceState>` that is owned by `SupervisorState`
and accessed under the same apps-map lock.

```rust
pub struct SpaceState {
    pub last_focused: Option<String>,
    pub is_fs_space: bool,
    // ...
}
pub struct SupervisorState {
    pub spaces: HashMap<u8, SpaceState>,
    // ...
}
```

This eliminates the `LAST_FOCUSED` Mutex entirely. All reads/writes to `last_focused`
happen under the existing apps-map lock — no new lock introduced, no new ordering
constraint.

---

## B4: SplitView Focus Protocol Has Undefined Behavior When the Sender of `splitview_focus_secondary` Is the Primary (Not the Secondary)

**Problem**

Section 8.2 states:

> The secondary app can request focus via: `@supervisor: splitview_focus_secondary`

and:

> If the sender is not currently the secondary of any SplitView pair, this command is ignored.

But the handler in §8.2 is named `handle_splitview_focus_secondary` and finds the pair
by `pair.secondary == sender_app`. The ambiguity is: if the **primary** app sends
`@supervisor: splitview_focus_secondary`, the spec says "ignored" — but the primary
may legitimately want to yield focus to its secondary partner (e.g., a primary text
editor yielding to a secondary terminal for shell commands). There is no IPC command
for the primary to voluntarily yield.

Worse, `splitview_focus_secondary` is also the command described as the way to toggle
focus within the pair. If the toggle is always initiated by the secondary, the primary
can never programmatically yield. And if the current primary attempts it, the command
is silently dropped — no error feedback, no documentation of expected behavior.

Additionally: `VYOMA_FOCUS:primary_to_secondary` (§11.1) is sent to the old primary
(now secondary) after the swap. But the new secondary (old primary) might not have
subscribed to `VYOMA_FOCUS:` events or might interpret `primary_to_secondary` as
unexpected if it thought it was sending a yield request.

**Proposed Fix**

Replace `splitview_focus_secondary` with a symmetric pair toggle command that can be
sent by either app in the pair:

```
@supervisor: splitview_toggle_focus
```

The handler finds the `FsPair` by checking if the sender is **either** the primary or
secondary of any registered pair:

```rust
fn handle_splitview_toggle_focus(sender: &str, fs_registry: &mut FsRegistry, ...) {
    let pair = fs_registry.pairs.iter_mut()
        .find(|p| p.primary == sender || p.secondary == sender);
    if let Some(pair) = pair {
        std::mem::swap(&mut pair.primary, &mut pair.secondary);
        // update split_role and focus_ring_active on both apps
        focus_transfer(&pair.primary, apps, spaces, FocusReason::Programmatic);
    }
    // Silently ignore if sender is not in any pair
}
```

Deprecate `splitview_focus_secondary`. Update §11.1 to replace
`VYOMA_FOCUS:secondary_active` / `VYOMA_FOCUS:primary_to_secondary` with a single
`VYOMA_FOCUS:splitview_role_changed:<primary|secondary>` sent to **both** apps after
a toggle, so both know their new role without special-casing who initiated.

---

## B5: `ArcSwap<String>` for `FOCUSED_APP` Stores App Names, But R22 Uses Integer App IDs — Name/ID Consistency Undefined

**Problem**

Section 2.1 declares `FOCUSED_APP: Arc<ArcSwap<String>>` — storing the app **name**
(a String). Section 12.2 notes that WIT `focused-window` needs to return an integer
app-id (R22), and the WIT layer "looks up app-id from name" via a `name → app_id` map.

This `name → app_id` map is not defined in this spec. R22 (App Lifecycle) established
integer app IDs but R28 does not specify:
1. Where the `name → app_id` map lives (is it on `SupervisorState`? a separate static?)
2. Who populates it (lifecycle handler at app launch?)
3. Thread-safety of the lookup (is it under the apps-map lock?)
4. What happens when an app is killed and relaunched with the same name — does it get
   a new app-id? Does `FOCUSED_APP` still hold the old name, which now maps to a
   different app-id?

Additionally, the `VYOMA_WM:list` response (R21 §9) uses `<app_id>` in the format
string `VYOMA_WM_WINDOW:<app_id>,<title>,...`. Focus notifications in R28
use app **names** (e.g., `VYOMA_CHROME:focus_changed:<app_name>`). This inconsistency
means consumers of the two protocols must maintain their own name↔id mapping — a
source of bugs as apps are killed and relaunched.

Standardizing on one identifier is necessary before the focus subsystem ships.

**Proposed Fix**

Standardize `FOCUSED_APP` on **app name** (String), not integer ID. Names are stable
for a given logical app (the manifest `app.name` field is fixed at compile time and
does not change on restart). Integer IDs are an internal R22 implementation detail;
they should not leak into the cross-subsystem focus protocol.

Concretely:
- `FOCUSED_APP` stores `String` (name) — as currently specced, confirmed.
- `VYOMA_CHROME:focus_changed:<app_name>,<app_title>` — uses name, confirmed.
- `VYOMA_FOCUS:gained:<reason>` — sent to the named app, confirmed.
- `focused-window` WIT function: change return type from `u32` (app-id) to `string`
  (app-name). This is more useful for apps that only know peer names from `boot.toml`
  or `VYOMA_WM:list` responses. Integer IDs are opaque and require an extra lookup.

```wit
interface focus-query {
    /// Returns the name of the currently focused app, or "" if none.
    focused-window: func() -> string;

    /// Returns true if the calling app is currently the keyboard-focus target.
    is-focused: func() -> bool;

    /// Returns the name of the app that last held focus, or "" if none.
    previous-focused: func() -> string;

    /// Returns the name of the focused app in the given space, or "" if empty.
    focused-in-space: func(space: u8) -> string;
}
```

This removes the undefined `name → app_id` lookup entirely. If integer IDs are needed
by a specific consumer (e.g., a future multi-window manager), R35 can add
`focused-window-id: func() -> u32` alongside `focused-window: func() -> string` without
breaking R28 callers.
