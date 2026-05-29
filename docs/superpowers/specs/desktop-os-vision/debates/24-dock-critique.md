# Critique: Dock & App Switcher (Round 24)

## Verdict

The R24 spec builds competently on the R23 chrome foundation: the `dock = true` registration
mirror of `chrome = true`, the `dock_router.rs` pattern copy from `chrome_router.rs`, and the
bottom-edge clip complement to chrome's top-edge clip are all structurally sound. The slot data
model is well-defined. The MRU ordering is clear and distinguishes correctly between the
dock-strip display order (spawn order) and the switcher display order (MRU). The switcher state
machine is complete. The decision to use `VYOMA_DRAW:resize_surface` for the switcher overlay is
an elegant reuse of R23 infrastructure that avoids introducing a new compositor concept.

However, five blocking issues prevent safe implementation as written: two simultaneous space-0
apps require an explicit z_order protocol that is defined here but never wired to the compositor's
space-0 filter logic, leaving an ambiguity in how the blit order is resolved when two apps share
the same space; the Ctrl-Tab provisional mechanism uses a 500ms synthetic `ctrl_up` timeout that
races with the modifier hold timer check in the event loop, and the spec does not define the
atomicity of `dock_ctrl_held` between the TTY input thread and the event loop's timer check; the
bottom clip arithmetic in §10.2 is correct in isolation but contradicts the R23 `blit_surface`
call signature (which the critique established takes `(fb, buffer, x, y, w, h, src_y_offset)`)
because the combined top+bottom code sample passes `blittable_h_from_bottom.min(blittable_h_from_top)`
as `height` but never recomputes what `blittable_h_from_top` equals in the R23 codepath —
leaving an implementation gap; the DockSlot `mru_rank` field is defined on the struct but is
never updated anywhere in the spec (MRU ordering is on `mru_list: Vec<String>` not on the slot
struct), creating a confusing divergence between the struct definition and the actual MRU model;
and the switcher overlay `resize_surface` to full screen and back relies on the same R23
`resize_surface` command that R23's critic identified as needing careful vsync_lock synchronisation,
but R24 performs this resize twice per switcher invocation (expand + collapse) without specifying
the synchronisation requirement or whether two rapid resize commands can be safely issued.

---

## Blocking Issues

### B1: Two Space-0 Apps — Z-Order Blit Sequence Is Underspecified at the Compositor Level

Section 4.1 defines the z_order reservation table (chrome=65535, dock=65534) and asserts that
"the compositor's flush_pass already handles multiple apps in the same space." This is
architecturally correct — the flush_pass sorts `apps_sorted` by `win_z` ascending and blits in
that order. However, the compositor's space-filter logic in R21 reads: "blit all space-0 windows
plus all windows in the active space." It does not specify the interleaving of space-0 windows
with active-space windows in the Z-sorted list.

Two possible interleaving models exist:

**Model A (separate passes)**: First blit all active-space windows in Z order, then blit all
space-0 windows in Z order. Space-0 windows always appear above all active-space windows.

**Model B (unified Z sort)**: Include both space-0 and active-space windows in a single Z-sorted
list and blit in that order. A space-0 window with z=5 would be below an active-space window
with z=100.

The spec assumes Model A (the phrasing "dock at 65534 is blitted second-to-last, chrome at 65535
is blitted last" only makes sense if space-0 windows are isolated from the normal Z range). But
the R21 spec does not define which model the existing compositor uses. R23 implicitly assumed
Model A (chrome is always on top because it has the highest Z in space-0, which is a separate
pass above all apps). But R23 never had to share space-0 with another app; the behaviour under
Model B with chrome=65535 and no other space-0 apps is identical to Model A.

R24 introduces a second space-0 app, making the model observable: if a normal app has z=70000
(which is clamped to 65531 by the manifest validator, but what if the supervisor itself
synthesises a higher-than-65531 z for some future system app?), would it appear above dock
(z=65534)? Under Model A, no. Under Model B, yes.

The fix: R24 must state explicitly that the compositor uses **Model A** (separate passes: active-
space windows at their native Z values, then space-0 windows in Z order above all of them). The
`flush_pass` code change for R24 must sort the `apps_sorted` snapshot in two sub-lists: non-
space-0 apps first (sorted by z ascending), then space-0 apps (sorted by z ascending). The
blit loop processes both sub-lists in sequence. This must be stated as a required change to
`compositor.rs`, not left as an assumed consequence of the z_order table.

---

### B2: `dock_ctrl_held` Is Shared Between TTY Thread and Event Loop Without Synchronisation

Section 7.3 defines `dock_ctrl_held: bool` as a variable in the supervisor's main event loop
state, checked on each iteration to decide whether to synthesise `ctrl_up`. Section 7.2 defines
that the TTY input thread sets `dock_ctrl_held = true` when Ctrl-Tab is received.

The TTY input thread and the main event loop run concurrently. `dock_ctrl_held` is written by
the TTY thread on every Ctrl-Tab keypress and read (and written) by the main event loop on every
vsync iteration. The spec does not define what synchronisation primitive protects this field.

If `dock_ctrl_held` is a plain `bool`, the concurrent write from the TTY thread and read from
the main event loop is a data race — undefined behaviour in Rust (the language requires either
a single-writer or explicit synchronisation for shared state). The two plausible options are:

**Option 1 (channel)**: The TTY thread sends Ctrl-Tab events over a `crossbeam::channel` or
`std::sync::mpsc::Sender<KeyEvent>` to the main event loop. The main event loop drains the
channel on each iteration and processes Ctrl-Tab events from there, updating `dock_ctrl_held`
in a single thread. This eliminates the shared state entirely and is the preferred pattern for
the VyomaOS architecture (the TTY thread already sends key events over a channel per R22's
model of event dispatch).

**Option 2 (atomic)**: `dock_ctrl_held` is an `Arc<AtomicBool>`, mirroring the
`chrome_input_locked` fix specified in R23 B4. The TTY thread calls `.store(true, Ordering::Release)`;
the main event loop calls `.load(Ordering::Acquire)`.

The spec must choose one option and state it explicitly, alongside the corresponding change to the
`last_ctrl_tab: Instant` timestamp (which also needs synchronisation if updated from the TTY
thread). Recommended: use Option 1 (channel), since the TTY thread channel model is established
by R22. The `dock_ctrl_held` flag and `last_ctrl_tab` then live entirely in the main event loop
and require no atomic primitives.

Additionally: the spec must clarify that `dock_ctrl_held` is reset to `false` when dock
transitions away from `Running` (same pattern as `chrome_input_locked` auto-release in R23 B4).
If dock crashes mid-switcher, `dock_ctrl_held` must be cleared so the supervisor does not
continue sending Ctrl-Tab events to a dead dock process.

---

### B3: Combined Top+Bottom Clip Code Sample Has an Unresolved Variable

Section 10.3 presents a combined clip pseudocode that calls:

```
blit_surface(fb, &surface.buffer, dest_x_px, effective_dest_y,
             surface_width_px,
             blittable_h_from_bottom.min(blittable_h_from_top),
             src_y_offset);
```

The variable `blittable_h_from_top` is not defined anywhere in §10. It is the height remaining
after the top-edge clip from R23 is applied — that is, `surface_height_px - src_y_offset`. The
spec shows `blittable_h_from_bottom` computed explicitly in §10.2 as a function of `dest_y_px`,
`surface_height_px`, and `dock_clip_y_px`. But it references `blittable_h_from_top` without
defining it in the same code context, making the combined clip sample incomplete for an
implementer reading §10.3 in isolation.

More importantly, the R23 critique (B3) established that the top-edge clip produces:

```rust
let src_y_offset = (effective_dest_y - win_y_px).max(0) as u32;
let blittable_h_after_top_clip = surface.height_px.saturating_sub(src_y_offset);
```

The combined clip height must be:

```rust
let blittable_h_combined = blittable_h_after_top_clip
    .min(blittable_h_from_bottom);
```

where `blittable_h_from_bottom` is computed as shown in §10.2 but using `surface_height_px`
replaced by `blittable_h_after_top_clip` (not the original `surface_height_px`) to avoid
double-counting the top-clipped rows. The spec's version passes the original `surface_height_px`
to the bottom-clip formula, which gives an incorrect result when the surface top has been clipped:
the bottom-clip calculation would operate on a height that includes already-clipped-away top rows.

The fix: §10.3 must restate the combined clip with all intermediate variables defined in order:

1. Compute `effective_dest_y` and `src_y_offset` from the top-edge clip (R23 formula).
2. Compute `blittable_h_after_top_clip = surface.height_px.saturating_sub(src_y_offset)`.
3. Compute the max bottom physical pixel of the blittable region: `blittable_bottom = effective_dest_y + blittable_h_after_top_clip as i32`.
4. Clip to `dock_clip_y_px`: `clipped_bottom = blittable_bottom.min(dock_clip_y_px as i32)`.
5. `blittable_h_combined = (clipped_bottom - effective_dest_y).max(0) as u32`.
6. Call `blit_surface(fb, &surface.buffer, dest_x_px, effective_dest_y, surface.width_px, blittable_h_combined, src_y_offset)`.

This is the single correct formulation. The §10.3 pseudocode must be replaced with these six
explicit steps.

---

### B4: `DockSlot.mru_rank` Field Is Defined but Never Maintained

Section 3.1 defines `DockSlot` with a field `mru_rank: usize` described as "Position in the MRU
list (0 = most recently focused)." Section 6.2 defines MRU ordering entirely in terms of
`mru_list: Vec<String>` operations (remove-and-prepend on focus, remove on exit, append on
launch). The `mru_rank` field on `DockSlot` is never updated anywhere in the spec.

This creates two sources of truth for MRU ordering: the `mru_list: Vec<String>` (which is
authoritative and correct per §6.2) and `DockSlot.mru_rank` (which is defined but never
written after initial creation). An implementer reading `state.rs` would see the `mru_rank` field
and either:

1. Attempt to maintain it in sync with `mru_list`, introducing a redundant update step that the
   spec never mentions and that is easy to get wrong (especially when multiple slots are re-ranked
   simultaneously after a focus change).
2. Leave it at its initial value of 0 for all slots, making it silently wrong.
3. Use it instead of `mru_list` for switcher ordering, producing incorrect behaviour when the two
   diverge (which they always will after the first focus change, since `mru_list` is updated but
   `mru_rank` is not).

The fix is unambiguous: **remove `mru_rank` from the `DockSlot` struct entirely**. The MRU
ordering is the responsibility of `mru_list: Vec<String>` in `DockState`. The switcher index
(`mru_index: usize`) is a field on `DockState` (or the switcher sub-state), not on individual
slots. The `DockSlot` struct should contain only information about a single app's state
(`app_name`, `display_name`, `lifecycle`, `is_focused`). MRU position is a property of the
ordering of the `mru_list` vector, not a property of a slot.

After removing `mru_rank`, the `DockState` struct becomes:

```rust
pub struct DockState {
    pub slots: Vec<DockSlot>,   // spawn order; drives dock strip rendering
    pub mru_list: Vec<String>,  // MRU order; drives switcher rendering
    pub switcher_index: usize,  // current selection in mru_list; 0 when switcher inactive
    pub switcher_active: bool,
    pub focused_app: Option<String>,
}
```

This is cleaner and has a single place to update for each ordering.

---

### B5: Switcher Double `resize_surface` Is Not Synchronised and Risks Compositor Race

Section 6.3 and §6.4 together describe two `resize_surface` calls per switcher session:
expand to `(logical_w, logical_h)` when switcher opens, and collapse to `(logical_w, 48)`
when switcher closes. R23's critique (B5 of the closing section, Questions §5) identified that
`resize_surface` requires the vsync_lock to be held for the duration of the buffer swap, and
that the old buffer must not be freed until the flush pass holding it completes.

R24 adds a new constraint: if the user presses Ctrl-Tab and releases Ctrl in very rapid
succession (within the same 16ms vsync frame), the dock may issue expand-then-collapse before
the compositor has processed the expand. The sequence would be:

1. Dock issues `resize_surface:<w>,<h>` (expand) to stdout.
2. Supervisor's output-reading thread processes the command, reallocates the surface under
   vsync_lock. The compositor on the next flush sees the large surface.
3. Before the next flush, dock has already issued `resize_surface:<w>,48` (collapse) and
   drawn the collapsed surface.
4. Supervisor processes the collapse resize, reallocates again under vsync_lock.
5. The compositor sees only the collapsed surface — the expand was effectively skipped.

In this case the visual result is correct (switcher blink without visible overlay), but the
double-reallocation under vsync_lock in the same frame is a surface-allocator pressure point.
More seriously, if `resize_surface` is processed asynchronously (e.g., via the main event loop
rather than the stdout-reading thread), the expand and collapse may be processed out of order
or batched, causing undefined surface dimensions for the compositor's next flush.

The fix requires two additions to §6.3 and §6.4:

**Addition A (ordering guarantee)**: `resize_surface` commands issued by dock must be processed
in the order they appear in dock's stdout stream. The supervisor must not batch or reorder them.
Since dock's stdout is read sequentially by one reader thread, this is already implied by the
architecture, but the spec must state it explicitly to prevent an implementer from "optimising"
the resize by coalescing multiple resize commands in the same frame.

**Addition B (minimum frame hold)**: After issuing the expand `resize_surface`, the dock must
call `VYOMA_DRAW:flush` before issuing any further `resize_surface` commands. The `flush`
command acts as a synchronisation barrier: it ensures the supervisor has committed the expanded
surface to the compositor before dock proceeds. Without this barrier, the collapse resize may
be processed before the first flush of the expanded surface, making the expand a no-op. The
spec's §6.3 step 5 already calls `VYOMA_DRAW:flush` after drawing the switcher overlay —
the requirement is that no collapse `resize_surface` may be issued before that flush completes.
Dock's event loop naturally serialises this (it processes one stdin event at a time), but the
spec should state: "the collapse `resize_surface` in §6.4 step 3 must be issued only after the
switcher overlay flush in §6.3 step 6 has been written to stdout."

---

## Non-Blocking Issues

**N1: The `dock_surface_ready: bool` fallback fill for the bottom 48px is mentioned in §15
(Open Questions) but not defined in §10 (Clipping) or §2 (Architecture).** The chrome spec
defined this in §10 and §2 explicitly. The dock spec should add the same fallback: before the
apps_sorted blit loop in `flush_pass`, if `dock_surface_ready` is false, fill the bottom 48px
rows with `0x1E1E2EFF` as a supervisor-side raw fill. This prevents boot-time visual corruption
in the dock zone during the ~200ms before dock's first flush.

**N2: §8.3 defines `initial_list` as sent "during dock's startup queue drain."** But the startup
queue is populated before dock reaches Running; the drain happens on the Running transition. The
`initial_list` event is a snapshot of the apps map at that moment. If apps are added or removed
between queue population and the drain, the initial_list may be stale by the time dock processes
it. The spec should note that `initial_list` is generated dynamically at drain time (when dock
transitions to Running), not at the time the queue was initially populated.

**N3: The `display_name` fallback for unknown apps (§3.3) says "first letter capitalised."**
But Rust's `String::to_uppercase()` operates on Unicode scalars, not ASCII. For all current
VyomaOS app names (ASCII only), this is fine. A note clarifying ASCII-only assumption in v1
prevents a future maintainer from using `chars().next().unwrap().to_uppercase()` on a UTF-8
app name and getting a multi-byte first char unexpectedly.

**N4: The switcher panel width formula in §6.3 is `min(logical_w - 64, max(320, mru_list.len() * 96))`.** For `mru_list.len() = 0`, this gives `max(320, 0) = 320`, and the switcher renders an empty 320pt panel. The spec should note explicitly that if `mru_list` is empty (no switchable apps), the switcher overlay is not shown at all (transition directly back to Inactive with no surface expand). Rendering an empty switcher panel is confusing UX and wastes a surface reallocation.

**N5: §9.1 reserves `VYOMA_DOCK:dock_badge` and `VYOMA_DOCK:dock_progress` as app-sendable
commands.** Both are described as "received by dock but result in no visual action." This creates
a protocol surface that is parsed but ignored in v1. Apps are not warned that these commands are
no-ops. A comment in `dock_router.rs` and a note in the protocol table should state that these
commands are logged at debug level and discarded in v1, exactly as `VYOMA_CHROME:` lines are
handled on headless platforms.

**N6: §7.3 uses `Duration::from_millis(500)` for the Ctrl-key release timeout.** The value 500ms
is not motivated. macOS uses approximately 1 second for Cmd-Tab hold-to-repeat behaviour. 500ms
means a user who pauses for half a second between Ctrl-Tab presses will accidentally commit the
current selection mid-cycle. The spec should note this trade-off and explain the 500ms choice
(presumably: faster than macOS to account for the approximation's inaccuracy). Alternatively,
the timeout should be made configurable via a future R78 System Preferences setting.

**N7: §13.3 (line budget) lists `supervisor/src/keyboard.rs` as "modified" with "~60 new lines."**
If `keyboard.rs` does not currently exist in the supervisor (the CLAUDE.md file structure does
not mention it explicitly), this should be noted as a NEW file rather than MODIFIED, with the
full estimated line count. The keyboard input routing currently lives in the TTY input thread
within `ipc.rs` or `main.rs`. If `keyboard.rs` is new, its initial contents will exceed 60 lines
once the `GLOBAL_SHORTCUTS` table, the `KeyCombo` struct, and the intercept logic are added.

**N8: The WIT `dock-state` interface in §11 exposes `get-mru-list` as returning `list<string>`.** In
WIT, `list<string>` is passed by value (copied). If the MRU list has many entries (up to N running
apps), this is a copy-on-call. For the query frequency expected (rare; debugging or future
integration), this is fine. But the interface should document the return order: "most-recently-
focused first." Without this documentation, a caller cannot distinguish MRU order from alphabetical
or spawn order.

**N9: The dock strip does not specify what happens when there are more running apps than fit in
`logical_w / slot_width` horizontal slots.** At 960pt logical width and 88pt+4pt=92pt per slot,
the dock fits `floor(960 / 92) = 10` slots. VyomaOS currently boots with 10 concurrent apps.
Adding more would overflow the dock. The spec should define an overflow policy for v1: either
truncate (show only the first N slots), scroll (not feasible without mouse in v1), or compress
(reduce slot width below 88pt). Recommended: truncate with a `+N more` indicator in the last
visible slot position.

**N10: §5.3 says the supervisor sends `focus_changed:` with an empty app name when no app holds
focus.** But `VYOMA_DOCK:focus_changed:` with no trailing app name would parse identically to
`VYOMA_DOCK:focus_changed:` with an empty string as the app name. Dock's stdin parser must handle
the empty-name case explicitly. The spec should define the exact wire format:
`VYOMA_DOCK:focus_changed:` (nothing after the colon) for no-focus, and confirm that an
empty string is a valid parse result for "no focused app." Adding a test case for this edge case
to the unit test plan would prevent a subtle parse bug. This same edge case applies to the chrome
router's `focus_changed` event from R23; the solution adopted there (an empty trailing field
treated as "no focused app") should be replicated verbatim in dock_router.

**N11: The implementation sequence in §16 does not include a step for the `dock_surface_ready`
fallback fill (mentioned in §15 as deferred).** Given that B-level issue N1 notes this omission,
the sequence should include it as Step 2.5 (between the dock_router stub and the compositor
bottom-clip), since it requires a change to `compositor.rs` and is most naturally implemented
alongside the dock clip logic. Deferring it to the FINAL round risks the boot-time visual glitch
shipping with an early implementation build.

**N12: The unit test list in §17 does not include a test for the `initial_list` + `focused`
startup sequence.** A unit test that: (a) starts the dock_router with three pre-existing Running
apps and one focused app, (b) triggers the dock Running transition, (c) confirms that dock's
stdin receives `initial_list:app1,app2,app3` followed by `focused:app2` would validate the most
complex startup code path (the queue drain combined with the snapshot injection). Without this
test, the initial_list logic is covered only by the QEMU integration test, which is slower and
harder to debug.

---

## Got Right

**The bottom-edge clip as a structural dual of the top-edge chrome clip is the correct design.**
R23 established a clean pattern for privileged UI regions: a y-clip in `flush_pass` plus a
Z-order guarantee provides defence in depth. R24 correctly applies this pattern to the bottom
edge without inventing a new mechanism. The key insight — that bottom-clip reduces `height`
without adjusting `src_y_offset` (unlike top-clip, which adjusts `src_y_offset` without reducing
`height`) — is stated explicitly in §10.2 and is correct. Once the B3 variable-definition gap is
fixed, the clip arithmetic is sound.

**The MRU list separation from the spawn-order slot list is the correct data model choice.**
Maintaining two independent orderings (spawn order for the dock strip, MRU order for the
switcher) avoids the tension between "stable UI" (dock strip should not reorder as focus
changes) and "recency-first UI" (switcher should put the most recently used app first).
The dock strip showing apps in spawn order matches macOS Dock behaviour (app order in the
dock does not change as you switch between apps). The switcher showing apps in MRU order
matches macOS Cmd-Tab behaviour (the last app you were in is the first in the switcher cycle).

**The `resize_surface` reuse for the switcher overlay is architecturally elegant.** Rather than
defining a new compositor primitive (a full-screen overlay layer owned by a separate process),
the spec reuses the R23 `resize_surface` command to expand dock's own surface temporarily. This
means the switcher overlay shares dock's Z-order (65534), so chrome at 65535 correctly stays
above the switcher without any special-casing. The menu bar remaining visible during the
switcher is a consequence of Z-order arithmetic, not a separate rule. Clean.

**The provisional Ctrl-Tab mechanism in §7 is properly scoped and R35-compatible.** Rather than
hard-wiring the Ctrl-Tab intercept in a way that would require rework when R35 defines the
formal global shortcut registration API, the spec uses a static `GLOBAL_SHORTCUTS` table that is
explicitly positioned as "the seed content of R35's runtime table." R35 can migrate the Ctrl-Tab
entries to dock's `register_shortcut` call without changing the semantics. The spec is honest
about the 500ms timeout approximation and defers proper modifier tracking to R35.

**The `initial_list` + `focused` startup sequence solves the dock startup race correctly.**
A dock that starts after other apps are already running needs a point-in-time snapshot, not
just a stream of lifecycle events. Sending `initial_list` and `focused` at queue-drain time
provides this snapshot without requiring the dock to issue a synchronous query (which would
require a different IPC model). The dock can reconstruct its full initial state from these two
events plus any queued lifecycle events that arrived during dock's Launching phase.

**The Z-order reservation table in §4.1 is the right place to formalise the space-0 z_order
budget.** Defining chrome=65535, dock=65534, reserved=65533, and clamping ordinary apps to
65531 gives future privileged apps a clear lane (65532–65533) and prevents z_order collisions
without requiring a dynamic allocation system. This table should be added to the supervisor's
`manifest.rs` validation logic as a named constant block, making future privileged app
additions self-documenting.

**The exclusion of `chrome` and `dock` from the dock's slot list is stated clearly.** §3.3
explicitly marks both `chrome` and `dock` as excluded from the visible slot set. This prevents
the confusing visual situation where the dock shows an icon for the dock itself or the menu bar
process. Excluding all space-0 privileged apps as a class (rather than by hard-coded name list)
would be even cleaner, but the current approach is correct for v1 where the privileged app set
is small and fixed.

**The 2-second Terminated-slot grace period driven by tick events is the right mechanism.**
Rather than using a real timer (which would require a second background thread or an async
runtime), the spec reuses the 1-second tick event already flowing to dock's stdin for the
slot-expiry counter. This means the grace period is approximately 2 seconds (2 tick events)
with at most 1 second of clock jitter. For a visual grace period on an exited app's slot, this
precision is more than adequate. The pattern of deriving timers from tick counts rather than
wall-clock measurements keeps the dock app purely reactive to its stdin stream, with no
background threads.

**The `GLOBAL_SHORTCUTS` table being a compile-time static is the correct choice for v1.**
A runtime-modifiable shortcut table would require a synchronisation mechanism between the TTY
input thread (which consults the table on every keypress) and any thread that modifies the
table. Deferring modification to R35 and using a static table in v1 eliminates this complexity
entirely. The static table cannot be corrupted by a misbehaving app, cannot be accidentally
cleared, and has zero runtime lookup overhead beyond a small linear scan over at most a dozen
entries.

---

## Questions for the Architect

1. **Dock and `input_lock_chrome`**: When chrome is in consent modal with `input_lock_chrome`
   active, Ctrl-Tab events are suppressed (all keyboard input goes to chrome). This means the
   user cannot use the switcher while a consent dialog is displayed, which is correct security
   behaviour. But what about the reverse: should dock be able to suppress keyboard input to all
   other apps while the switcher is active, analogous to chrome's `input_lock`? Or is the current
   model (Ctrl-Tab is intercepted globally, other keys still go to the focused app) sufficient?
   The current model means a focused app receives all keys except Ctrl-Tab during switcher mode,
   which could lead to accidental keystrokes in the focused app while the user cycles through the
   switcher.

2. **MRU and apps that have never received focus**: An app that is Running but has never been
   focused (e.g., `http-server` starts running in the background and receives no focus because
   the user uses only the shell) appears last in `mru_list` at its append position from
   `app_launched`. Over time, as other apps receive focus and move to the front, background
   apps accumulate at the back of the MRU list. Is this the desired behaviour? An alternative
   is to exclude apps from the MRU list entirely until they have received at least one `focus_changed`
   event. The spec should clarify this case explicitly.

3. **Dock behaviour when the active space has no Running apps**: If the user has moved all
   apps to Space 2 and switches to Space 1 (which has no apps), the dock strip would show an
   empty set of slots (since slots are for all running apps, regardless of their space, per the
   current model). Or should dock filter slots by the active space? macOS Dock shows all apps
   regardless of space. The spec should confirm: dock shows all running apps on all spaces, not
   just the active space's apps.

4. **Ctrl-Tab during a chrome consent modal**: Section 7.2 says "Ctrl-Tab is consumed by the
   supervisor before being forwarded to the focused app." But R23's `input_lock_chrome` redirects
   all keyboard input to chrome. Does the global shortcut intercept run before or after the
   `input_lock_chrome` check? If it runs before (global shortcuts win), the user can open the
   switcher during a consent dialog. If it runs after (input_lock wins), Ctrl-Tab is blocked
   during consent. Correct answer is: `input_lock_chrome` should take priority over global
   shortcuts. The spec should document this precedence explicitly.

5. **Dock's `display_name` for the `chrome` app itself**: §3.3 says chrome is "excluded from
   dock; dock does not show space-0 privileged apps." But dock receives `app_launched:chrome`
   events from the supervisor because the lifecycle routing does not distinguish privileged from
   ordinary apps. Dock's implementation must explicitly filter out apps whose names match the
   known privileged app list (`chrome`, `dock`). This filter should be documented in §3.3 as a
   required implementation step, not just as a table note.

---

## Closing Assessment

Round 24 is a well-structured extension of the R23 chrome architecture. The symmetry between
`chrome_router.rs` and `dock_router.rs`, between the top-edge and bottom-edge clips, and between
chrome's z=65535 and dock's z=65534 gives the spec a consistent internal logic that makes it
easy to reason about. The switcher design is clean. The data model for MRU vs spawn-order is
sound once `mru_rank` is removed from `DockSlot`.

The five blocking issues are all implementation precision gaps. B1 (Model A vs Model B space-0
Z sort) requires one sentence clarifying the two-pass blit structure and one corresponding change
to `compositor.rs`. B2 (`dock_ctrl_held` thread safety) requires naming a synchronisation
primitive and adding an auto-clear on dock exit. B3 (combined clip variable gap) requires replacing
the §10.3 pseudocode with six explicit sequential steps. B4 (`mru_rank` removal) requires deleting
one field from a struct. B5 (double resize synchronisation) requires adding a flush-ordering
guarantee and a note about no-op resize when expand+collapse occur within one frame.

None of these require architectural redesign. Applied together, they close all known implementation
ambiguities. After resolution, R24 is ready for implementation planning.

Estimated implementation cost: 3–4 days of focused Rust development. The dock WASM app (5 files)
and `dock_router.rs` are the largest new pieces. The compositor bottom-clip is the highest-risk
change due to its interaction with the existing R23 top-clip; the unit tests in §17 (specifically
`compositor_combined_clip` and `compositor_src_y_offset_unchanged_by_bottom_clip`) should be
written first (TDD) before the clip implementation.

The questions section surfaces one policy decision requiring architect input before implementation
begins: Ctrl-Tab precedence during `input_lock_chrome` (Question 4). This is a one-sentence
decision (`input_lock_chrome` wins, global shortcuts are suppressed during input lock) and should
be added to §7.2 of the spec as an explicit rule before the implementation sequence starts.

The non-blocking issues are lightweight. N1 (dock_surface_ready fallback fill) and N9 (dock
strip overflow policy) are the two most operationally visible: without N1, the dock zone shows
framebuffer garbage for ~200ms on every boot; without N9, a system with 11 running apps silently
renders only 10 slots with no indication that one is missing. Both are one-paragraph additions
to §10 and §6.3 respectively and should be resolved before the implementation sequence begins.
The remaining non-blocking issues (N2–N8, N10) are documentation clarifications that can be
resolved inline during development without a spec revision cycle.

Overall quality: strong. The R24 spec is a sound and well-scoped extension of R23's privileged
WASM app pattern. After the five blocking issues are resolved and the two operational non-blocking
issues are addressed, R24 is ready for implementation planning.
