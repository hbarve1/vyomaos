# Critique: Accessibility Tree & AX API (Round 30)

**Critiquing**: `30-accessibility.md`
**Critic role**: blocking issues only — correctness, threading safety, integration coherence

---

## CRITIQUE — 5 Blocking Issues

---

### B1: `AXParseState` Is Per-App Mutable State Stored Outside the Registry — No Home Defined, Creating a Data Race

**Problem**:

Section 15 defines `AXParseState { Idle | InTree { revision, partial } }` as per-app
mutable state held while a `begin_tree` / `end_tree` block is in flight. Section 16
describes `handle_app_output` as the code path that transitions this state machine.

`handle_app_output` runs on the **IPC/output reader thread** for a specific app — this
thread owns the app's stdout pipe. The spec never states where `AXParseState` is stored.
The only plausible storage locations are:

1. Inside `AppAXTree` (in `ax_registry`) — but `ax_registry.write()` would need to be held
   for the entire duration of a `begin_tree` … `end_tree` block (potentially many lines),
   blocking all AX client reads for the duration. Unacceptable.

2. Inside `AppState` in `apps_map` — but the output reader thread cannot acquire
   `apps_map.write()` without risking ABBA deadlock with the compositor thread (which
   acquires `apps_map.read()` under `vsync_lock.write()`).

3. As a local variable owned by the per-app reader thread — this is thread-safe by
   construction since each app has exactly one reader thread. But the spec does not state
   this, and the struct definition in Section 15 suggests it is part of a shared context.

Without a clear ownership declaration, an implementer will likely embed `AXParseState`
into `AppState` (option 2), which creates a deadlock hazard: the output reader thread
acquires `apps_map.write()` to update `AXParseState`, while the compositor thread holds
`apps_map.read()` for the flush pass.

**Proposed fix**:

State explicitly in the spec that `AXParseState` is a **local variable on the per-app
output reader thread**, not stored in any shared struct:

```rust
// supervisor/src/accessibility/parser.rs

pub fn run_app_output_reader(
    app_name: String,
    stdout: ChildStdout,
    ax_registry: Arc<RwLock<AXRegistry>>,
    // ...
) {
    let mut ax_state = AXParseState::Idle;   // thread-local; no shared ownership
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap_or_default();
        if line.starts_with("VYOMA_AX:") {
            handle_ax_line(&app_name, &line, &mut ax_state, &ax_registry);
        } else {
            // ... existing VYOMA_DRAW: / IPC routing ...
        }
    }
}
```

`handle_ax_line` accumulates into `ax_state.partial` (stack-local `AppAXTree`) and only
acquires `ax_registry.write()` for the brief atomic swap on `end_tree`. Add a section to
the spec: "**AX Parse State Ownership**" that makes this explicit and calls out why it must
not be stored in `AppState` or `AXRegistry`.

---

### B2: Bounds Encoding Is App-Local but AX Clients Need Screen Coordinates — Coordinate Translation Is Unspecified

**Problem**:

Section 16.3 states: "App-local pixel coordinates (origin at app window top-left). The
supervisor does NOT translate to screen coordinates in the registry — AX clients that need
screen coordinates add the app window origin (available from the compositor's
`AppState.position` field)."

This creates a two-step lookup that is never specified:

1. AX client calls `get-node("calculator", 7)` via WIT → gets bounds `(x=20, y=44, w=300, h=28)`.
2. AX client needs screen coordinates → must look up the window's screen position.

But the `accessibility-client` WIT interface defined in Section 5.2 contains only AX tree
queries. There is no WIT call to retrieve a window's screen position. The `AppState`
struct is a supervisor-internal type; WASM apps cannot access it.

A screen reader drawing a highlight rectangle (to indicate the focused element visually)
cannot compute its screen position without window position data. VoiceOver on macOS uses
`AXFrame` (screen-relative) for exactly this purpose.

Furthermore, window positions change during window dragging. An AX client that caches
bounds will display stale highlights during a drag unless it re-queries. The spec provides
no invalidation mechanism for position changes.

**Proposed fix**:

Add a `get-window-bounds` function to the `accessibility-client` WIT interface:

```wit
record window-bounds {
    screen-x: s32,
    screen-y: s32,
    width: u32,
    height: u32,
}

get-window-bounds: func(app-name: string) -> option<window-bounds>;
```

The supervisor implements this by reading `AppState.position` from `apps_map.read()` — a
cheap read-lock. The AX client computes element screen position as:
`(window.screen_x + node.x, window.screen_y + node.y)`.

Additionally, add a `VYOMA_AX_EVENT:window_moved:<app_name>:<x>,<y>` push event, emitted
by the compositor when a window's position changes, so AX clients can invalidate cached
screen coordinates without polling.

---

### B3: `begin_tree` / `end_tree` Block Can Be Interleaved with `VYOMA_DRAW:flush` — Compositor Deadlock

**Problem**:

Section 3.1 defines a multi-line `begin_tree` … `end_tree` protocol. An app writes these
lines to stdout sequentially. However, stdout is a single pipe — the app may interleave
`VYOMA_DRAW:` and `VYOMA_AX:` commands in any order. A well-behaved app would not do this,
but the spec makes no guarantee.

More critically: Section 14 shows the compositor tick draining `pending_ax_events` at
phase 3b — **after** the compositor blit pass (phases 1–2). The blit pass acquires
`vsync_lock.write()`. Phase 3b dispatch calls `send_to_stdin()` which acquires
`apps_map.read()`.

Now consider this sequence:
1. Compositor tick phase 1–2: acquires `vsync_lock.write()` → completes → releases.
2. An app's output reader thread (running concurrently) writes to `ax_registry` (acquires
   `ax_registry.write()`) and then tries to deliver a `tree_changed` event by calling
   `send_to_stdin()` which acquires `apps_map.read()`.
3. Simultaneously, the compositor tick phase 3b acquires `apps_map.read()`.

This is not a deadlock in itself, but there is a lurking ordering issue: if the output
reader thread calls `send_to_stdin` directly (bypassing the pending_ax_events queue) while
the compositor tick is also calling `send_to_stdin` for AX events, the stdin pipe to the
AX client app can receive interleaved writes from two threads, corrupting the event stream.

The spec says events are "enqueued" and "drained" but Section 15 shows `handle_ax_line`
directly acting on `ax_registry`. The enqueue path for resulting events is not connected to
the parse path — the `pending_ax_events` queue is described in Section 14 but no code in
Section 15 pushes to it.

**Proposed fix**:

Make the enqueue path explicit in the parse handler. `handle_ax_line` must push events
onto a **thread-safe queue** rather than calling `send_to_stdin` directly:

```rust
// AX event queue: producer = output reader threads; consumer = compositor tick
pub type AXEventQueue = Arc<Mutex<Vec<(String, String)>>>;
//                                  ^client   ^event_line
```

`handle_ax_line` pushes `(client_name, event_line)` onto `AXEventQueue` (lock held
briefly). The compositor tick phase 3b drains this queue and calls `send_to_stdin`. This
serializes all AX event delivery through a single thread (the compositor tick), eliminating
the concurrent stdin write hazard.

Add a section "**AX Event Delivery Queue**" specifying the `Arc<Mutex<Vec<...>>>` type,
its location in `SupervisorState`, and the producer/consumer threading invariant.

---

### B4: `voiceover` at z=65531 in Space-0 Participates in the Compositor Flush — No Space-0 Slot Defined

**Problem**:

Section 8.3 assigns `voiceover` z=65531 in space-0. The established space-0 occupant
table (normative, R24) lists:

| App | z |
|-----|---|
| chrome | 65535 |
| dock | 65534 |
| mission-control | 65533 |
| stage-strip | 65532 |

z=65531 is not in this table. More importantly, the space-0 compositor pass in Model A
(R24) iterates `apps_sorted` for space-0 apps in z-ascending order — it does not maintain
a hardcoded list. The pass will naturally include `voiceover` at z=65531 as long as it is
placed in space-0 and has a surface.

However, the `voiceover` app draws only when focus changes (caption update) and flushes
once. Between caption updates, its surface may contain stale content from a previous
caption or blank pixels. The space-0 pass composites `voiceover`'s surface on every
vsync tick regardless of whether the content changed. If `voiceover`'s surface is
uninitialized (all zeros, transparent) the compositor correctly blits nothing visible. But
if `voiceover` has drawn a caption and then the focused app changes to one with no label
(e.g., `role=image` with empty label), `voiceover` must actively clear its surface —
there is no mechanism described for this.

Furthermore, z=65531 places `voiceover` **below** `stage-strip` (65532). On a display
running Stage Manager, the stage-strip is visible at the left edge. The voiceover caption
bar at the bottom of the screen (y=670) does not conflict spatially, but the z-ordering
means stage-strip renders on top of voiceover. If Stage Manager covers the bottom of the
screen (e.g., on a small display), the caption is obscured.

**Proposed fix**:

1. Formally add `voiceover: z=65531` to the space-0 occupant table in the spec (and note
   this as a normative update to R24's table).

2. Specify the surface lifecycle for `voiceover`: on receiving a `focus_changed` event
   with an empty label, `voiceover` must clear its caption region:
   ```
   VYOMA_DRAW:fill_rect:0,670,960,30,0x00000000
   VYOMA_DRAW:flush
   ```
   Add this explicitly to Section 8.3 as a required behavior.

3. Address z vs Stage Manager: specify that the caption bar occupies y=650–680 on
   `desktop-full` (1280×800+) but y=590–620 on displays shorter than 700px. Add a
   `VYOMA_AX:config_set:caption_y,<y>` command that the supervisor can issue to `voiceover`
   on startup (pushed as a stdin message), letting the supervisor compute the safe y
   position based on display height and stage-strip presence.

---

### B5: AX Client Bootstrap Sends `app_appeared` for Every Existing App — No Ordering Guarantee Relative to `end_tree` Events Means Client May Query Before Trees Are Available

**Problem**:

Section 5.3 states: "Delivers a synthetic `VYOMA_AX_EVENT:app_appeared:<name>` for every
app already in the registry, so the AX client can bootstrap its internal state."

The delivery mechanism is `send_to_stdin` (or via `pending_ax_events` queue if B3 is
fixed). The AX client receives these events and is expected to call `get-tree(<name>)` to
populate its internal model.

However, the spec allows apps to start with `revision = 0` (Section 13: "No disk
persistence — apps re-publish their AX trees each time they start"). A newly started AX
client (e.g., user launches `voiceover` after all other apps are already running) will
receive `app_appeared` for every running app, but each app's `AppAXTree` may have
`revision = 0` (empty) because the app has not yet re-published its tree since it started.

The `voiceover` app calls `get-tree("calculator")` and receives an empty list. It has no
way to know whether the tree is genuinely empty (the app has no UI) or not yet published.

Worse: `app_appeared` may arrive at the AX client **before** the app's `tree_changed`
event (which would arrive after the app publishes its first full tree). The AX client must
handle a common pattern: receive `app_appeared`, get empty tree, then later receive
`tree_changed` with actual data. If the AX client assumes `app_appeared` means a valid
tree is available, it will display incorrect information.

This is not just a "quality of implementation" issue — it is a protocol ambiguity that will
cause incorrect behavior in the reference `voiceover` implementation. An implementer
reading Section 5.3 and Section 8.1 in isolation will write code that queries the tree
immediately on `app_appeared` and fail silently on apps that haven't published yet.

**Proposed fix**:

1. **Clarify protocol semantics**: Add a note to Section 5.3 that `app_appeared` means "an
   app is registered in the AX registry" — **not** "a valid AX tree is available." The AX
   client must use `ax_tree_summary.revision` to determine if a tree has been published
   (`revision == 0` → tree is empty/unpublished; `revision > 0` → tree is valid).

2. **Add a `tree_ready` event**: After an app publishes its first full tree
   (`begin_tree`/`end_tree` block with `revision >= 1`), the supervisor emits a distinct
   event to AX clients:
   ```
   VYOMA_AX_EVENT:tree_ready:<app_name>:<revision>
   ```
   This is the signal for an AX client to call `get-tree()` for the first time. Subsequent
   updates use `tree_changed`.

3. **`voiceover` implementation note**: In Section 8, specify that `voiceover` only
   processes focus-change events for apps whose `AppAXTree.revision > 0`. It silently
   ignores `focus_changed` events for apps with unpublished trees, instead of querying and
   displaying nothing.

4. **Protocol table update**: Add `VYOMA_AX_EVENT:tree_ready:<app>,<revision>` to the
   event table in Section 10.
