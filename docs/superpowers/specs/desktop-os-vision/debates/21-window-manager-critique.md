# Critique: Window Manager & Spaces (Round 21)

## Verdict

The spec is architecturally sound for a first-generation window manager on a WASM OS. The
decision to embed window properties directly in `AppState` avoids a separate registry-of-
registries abstraction that would add lock nesting without benefit at this scale. The
`SpaceRegistry` bitmask design is clean and the compaction algorithm for z-order is well
motivated. The partial auto-tiling model (manual layout locks individual windows, leaving
others auto-tiled) is the right tradeoff between flexibility and predictability.

However, five blocking issues prevent safe implementation: a Surface reallocation race
with the compositor; z-order compaction producing non-deterministic results at the boundary
between space-N and space-0 windows; the space switching flush being underspecified at the
level of which lock is held in what order; an ambiguity in the z-order rule for space-0
windows that contradicts the no-forced-ordering design principle stated one paragraph
earlier; and a partial auto-tiling edge case where a manual-layout window can permanently
fragment the screen after its neighbour is removed. Each of these will cause either data
races, visual corruption, or user-visible layout glitches in production. None require
redesigning the core model — they are precision gaps that a revised spec can close with
targeted additions to §4, §6, and §10 without invalidating any other section.

---

## Blocking Issues

### B1: Surface Reallocation Race During Resize

Section 6.3 proposes the correct fix — acquire `vsync_lock.write()` before swapping the
Surface pointer — but leaves a critical gap in the call path. The `resize` IPC command
arrives on an app-output-reading thread. That thread does not ordinarily hold
`vsync_lock`. However, the IPC handler in `ipc_handlers.rs` may be called from within
the same execution context as `draw_cmd.rs`, which calls `blit_surface` while holding
`vsync_lock.read()`.

The specific race: the app output thread calls `ipc_handlers::handle_command("resize
shell 800 600")`. Inside the handler, the spec says acquire `vsync_lock.write()`. But
if the same thread earlier in the same call stack is already holding `vsync_lock.read()`
(e.g., because a `VYOMA_DRAW:flush` on the same line batch triggered a compositor run),
the write-lock acquisition will deadlock on a non-reentrant `RwLock`. Rust's
`std::sync::RwLock` does not support upgrading a read lock to a write lock, and
attempting `write()` while the current thread holds `read()` is undefined behaviour
(implementation-defined panic or deadlock depending on the OS futex implementation).

The fix must be one of:
- (A) Ensure the resize command handler is never called from within a read-lock context.
  This means the `resize` IPC command must be dispatched to a dedicated resize thread or
  deferred to the compositor tick (processed between frames, after `vsync_lock.read()` is
  released).
- (B) Use `Arc<ArcSwap<Surface>>` (from the `arc-swap` crate) instead of
  `Arc<Mutex<Surface>>` for the surface pointer, enabling a lock-free atomic swap. The
  compositor reads the current pointer via `load()` which is safe concurrently with a
  `store()` from the resize handler.
- (C) Split the surface pointer from the surface content: store an `Arc<Mutex<Option<
  Surface>>>` where `None` signals "in progress" and the compositor skips blitting
  `None` surfaces for that frame.

The spec must select one of these and describe it precisely. Option A (deferred resize)
is the safest because it requires no new dependencies and preserves the existing lock
discipline. The spec should state: "resize commands are enqueued in a per-app
`PendingResize` channel; the compositor thread drains this channel between frames while
not holding vsync_lock.read()."

A secondary concern in B1 is the ordering of the size update relative to the Surface
swap. In the spec's §6.3 protocol, `win_w` and `win_h` are updated (step 5) after the
Surface is replaced (step 4). If the compositor reads `win_w`/`win_h` to determine the
blit dimensions but accesses the new Surface (which uses its own `.width`/`.height`
fields), the size update ordering is irrelevant to the blit. However, if any other code
path reads `win_w`/`win_h` to compute clip bounds or HiDPI scale factors before the
Surface is swapped, a tearing window exists where `win_w` and `win_h` reflect the new
size but the Surface still reflects the old size. The spec must state that `win_w`,
`win_h`, `win_region`, and the Surface pointer are all updated atomically within the
same vsync_lock.write() critical section, and that no code outside this section reads
`win_w`/`win_h` without first acquiring the vsync_lock.

### B2: Z-Order Compaction Non-Determinism for Space-0 Windows

Section 4.4 states that space-0 windows "are included in the compaction pass for every
space" and their z-indices "are interleaved with the active space's windows based on
whatever win_z value they were given when last compacted." This creates a non-determinism
problem.

Consider: space-0 app `overlay` was last compacted in the context of space 1 where it
got `win_z = 30`. Now the user switches to space 2, which has two apps with `win_z = 10`
and `win_z = 20`. The compaction for space 2 includes `overlay` (space-0) with its stale
`win_z = 30`. Compaction step 3 reassigns: app-A gets 10, app-B gets 20, overlay gets 30.
Overlay is on top. So far predictable.

But now the user runs `compact_z_order` on space 1 (triggered by a focus change in space
1 while space 2 is active). Overlay's `win_z` is reassigned in the context of space 1's
compaction to, say, `20` (middle of 3 apps). Then the user switches back to space 2.
Space 2 has its stale `win_z` values from the previous compaction, and overlay now has
`win_z = 20` from the space-1 compaction — it is now sandwiched between space 2's app-A
(10) and app-B's stale value which may also be 20 or some other value. The z-order for
space 2 is now undefined.

The root cause: `win_z` is a single field shared across all spaces. Compacting for one
space mutates `win_z` values that are also used in another space's compaction.

The fix: `win_z` must be **per-space**. Replace `win_z: u32` on `AppState` with a
`win_z: HashMap<u8, u32>` (keyed by space number, with space 0 as special). Alternatively,
for space-0 apps, maintain a single canonical z-index used in every space's compaction
(i.e., space-0 apps are always placed at the top, with a separate `space0_z: u32` field
that is only modified when explicitly set). The spec must choose one of these models and
update the compaction algorithm description accordingly.

There is also a correctness issue with the compaction trigger. The spec says compaction is
called "after any window is added or removed from a space." When an app exits (process
death), the app-lifecycle thread removes the entry from `AppRegistry`. This does not
currently trigger a compaction call because the lifecycle code in `lifecycle.rs` is
unaware of the WM. A separate trigger path is needed: when `AppState` is removed from
the registry, the lifecycle handler must call `wm::compact_z_order` for the space that
window occupied. The spec lists the three compaction trigger points (§4.2) but does not
mention process death as a trigger. A fourth trigger must be added: "when an app's
process exits and its `AppState` is removed from the registry."

### B3: Space Switching Flush — Lock Order and Atomicity

Section 3.4 says the space switch completes asynchronously: "the supervisor sets
`active = N` and marks the compositor dirty; the next compositor tick will render the new
space." Section 6.4 says "Set APP_DIRTY for every app in the new active space." These two
descriptions are consistent, but neither specifies the lock order for the operation.

The space switch handler runs on an IPC handler thread. It acquires `spaces().lock()` to
update `active`. The compositor thread, on its next tick, acquires `spaces().lock()` to
read `active` during the space-filter check (§6.2 step A). Between these two operations,
there is a window where `active` has been updated but `APP_DIRTY` has not yet been set.
If the compositor runs its tick in that window, it will composite the new space's windows
without having cleared the framebuffer back-buffer first (§6.4 requires a background fill
before compositing the new space). The result is ghost pixels from the old space visible
behind the new space's windows.

The fix: the space switch must be atomic from the compositor's perspective. The simplest
implementation is to acquire `vsync_lock.write()` for the entire switch operation: update
`active`, clear the back-buffer to background, mark all new-space apps dirty, then release.
The compositor's next `vsync_lock.read()` acquire will see a fully consistent state.

Alternatively, introduce a `PENDING_SPACE_SWITCH: AtomicU8` global. The compositor, at
the start of each tick (before acquiring any surface locks), checks if a pending space
switch is set and if so, performs the background fill and dirty-marking atomically before
proceeding with the surface blit loop. This avoids taking a write lock from a non-
compositor thread.

The spec must select one approach and describe the lock sequence precisely, listing all
globals touched in the switch operation in the order they are locked.

A related issue: §3.4 says the space switch broadcasts `VYOMA_WM:space_changed:<N>` to
"every app that has `display = true`." The broadcast calls `send_reply(target, msg,
inbox)` for each app. `send_reply` acquires `inbox.lock()` to look up the app's sender
channel. If the broadcast is done while holding `spaces().lock()`, and any app's inbox
processing also acquires `spaces().lock()` (e.g., to read the active space for a query
command), a lock-order inversion deadlock is possible. The spec must either (a) release
`spaces().lock()` before performing the broadcast, or (b) document that `spaces().lock()`
is never acquired while holding `inbox.lock()` and enforce this as a lock-ordering
invariant in the codebase comments.

### B4: Space-0 Z-Order Contradiction

Section 4.4 contains an internal contradiction. The first paragraph states: "Their
z-indices are interleaved with the active space's windows based on whatever win_z value
they were given when last compacted." This is the no-forced-ordering design: space-0
windows can be low or high z depending on how they were positioned.

The second paragraph states: "In most cases they should be positioned above the active
space's regular windows to function as 'always on top' overlays (e.g., a system
notification window)." This suggests space-0 is primarily an "always on top" mechanism.

The third paragraph states: "There is no automatic 'space-0 windows are always above
space-N windows' rule." This contradicts the previous sentence's implication.

The contradiction matters because it leaves the compositor implementation undefined for
the common case. If a developer implements space-0 as "always on top" (the motivated use
case), they must violate the no-forced-ordering principle. If they implement it as
interleaved-by-win_z (the stated rule), the "system notification" use case requires manual
`focus overlay` calls after every space switch to keep the overlay on top, which is
fragile.

The fix: the spec must commit to one of two models and describe the implementation
precisely:

**Model A (Always-On-Top)**: Space-0 windows are always composited after (on top of) all
space-N windows. Within the space-0 set, z-order among themselves is determined by their
relative `win_z` values. The compaction algorithm produces: space-N windows get z=10*i,
space-0 windows get z=N_count*10 + 10*j. This is simple and matches the motivated use
case.

**Model B (Interleaved)**: Space-0 windows participate in the per-space z-order as equals.
A space-0 window can be placed anywhere in the stack via the `focus` and `assign` commands.
The compaction algorithm is unchanged. The spec must note that space-0 "background
wallpaper" apps must manually be assigned a very low z-index and that no system invariant
enforces this.

The spec should adopt Model A. It is the cleaner semantic (space-0 = "sticky overlay
layer"), it aligns with the macOS analogue (system notification panels above all app
windows), and it simplifies the compositor: the blit loop naturally places space-0 windows
last regardless of their numeric z-index.

### B5: Partial Auto-Tiling Creates Permanent Screen Fragmentation

Section 10.4 describes the partial auto-tiling model: apps with `win_manual_layout = true`
keep their positions; newly added or removed non-manual apps are tiled into "the remaining
space." The problem is that the spec does not define what "remaining space" means when
manual-layout windows occupy non-contiguous or non-aligned regions.

Consider: screen is 1440×900 pts. Three apps A, B, C start in auto-tiling: A=(0,0,480,900),
B=(480,0,480,900), C=(960,0,480,900). The user runs `move B 200 200` — B becomes
manual-layout at (200,200,480,900), overlapping with A and C. Now app D is added. The
tiling engine must re-tile {A, C, D} (non-manual apps) into the "remaining space." But
the remaining space is not a rectangle — it is the full screen minus B's manually
positioned region. The current `compute_tiling` function takes `(n, sw, sh)` and
returns n rectangles filling the full `sw × sh` rectangle. It has no concept of "excluded
regions." Calling `compute_tiling(3, 1440, 900)` will produce three rectangles that
overlap with B's manual position, creating z-order-dependent visual corruption.

The spec states this works correctly but does not define the tiling algorithm for the case
where manual windows are present. This is a specification gap that will produce visual bugs
in the first real-world use.

The fix: the spec must define partial auto-tiling explicitly. The simplest correct model
is: **when any app in the active space has `win_manual_layout = true`, auto-tiling is
globally disabled for that space** (not per-window disabled). The non-manual apps are
tiled into the full screen as if the manual-layout windows do not exist; the manual windows
appear on top due to z-order. This is the "layers" model: auto-tiled apps form the
background layer; manually positioned apps float above. It is visually predictable, and
it matches how most real window managers behave (floating vs tiling modes are mutually
exclusive per workspace). The `reset_tiling <app>` command remains as described, and
`reset_tiling` for all apps simultaneously returns to pure auto-tiling.

The "layers" fix also resolves a secondary ambiguity in §10.2: the spec says the tiling
engine "only tiles apps belonging to the active space." But the tiling function
`compute_tiling(n, sw, sh)` takes a count `n`, not a list of apps. The caller
(`apply_tiling_layout`) must pre-filter the registry to count only the visible,
non-manual, active-space apps, then call `compute_tiling` with that count, and apply
the resulting regions back to those same apps in the same order. The spec must define the
ordering of this pre-filtered list (currently: "left-to-right, top-to-bottom" from
§10.1, which implies some deterministic sort of the app names or spawn order). If the
ordering of apps in the tiling grid changes between app add/remove events, existing apps
will swap positions on screen, which is disorienting. The spec should state that tiling
order is determined by app spawn time (the order apps appear in `boot.toml`), is
stable across recomputes, and that newly added apps are appended to the end of the order.

---

## Non-Blocking Issues

**N1: `win_region` redundancy.** The spec adds `win_x, win_y, win_w, win_h` (logical
points) alongside the existing `win_region: Option<(u32, u32, u32, u32)>` (physical
pixels). Two sources of truth for the same data will drift. The spec acknowledges this and
says "both are kept in sync by apply_window_geometry." This is acceptable for R21 but the
spec should flag this as a technical debt item to be resolved in R22 (deprecate
`win_region`, derive physical bounds from logical fields on demand). The migration path is:
introduce a method `fn physical_rect(state: &AppState, sf: u32) -> (u32, u32, u32, u32)`
on the WM module, then migrate all callers of `win_region` to use this method. In R22,
remove the `win_region` field from `AppState`. The R21 spec should include a comment in
the `AppState` struct definition: `// TODO(R22): deprecate win_region in favour of
win_x/win_y/win_w/win_h * scale_factor`.

**N2: `win_title` default initialisation.** Section 5.9 says `win_title` defaults to "the
app name from the manifest." But `AppState` is constructed in `app_threads.rs` from a
`BootEntry`, which contains the app name. The spec does not update `AppState` construction
to initialise `win_title = entry.name.clone()`. This is a one-line fix but it must be
called out explicitly to avoid `win_title` defaulting to an empty string in the
implementation.

**N3: `wm_list_windows` response ordering.** Section 8.3 defines the response format as
"one line per window" but does not specify the ordering. It should be: all windows sorted
by space number ascending (space 1 first, space 9 last, space 0 last of all), then within
each space, sorted by ascending z-index. This makes the response both human-readable (all
windows in space 1 are grouped together) and machine-parseable (the topmost window in the
active space is the last line in that space's group). Deterministic ordering also makes
unit-testing the IPC response straightforward via string comparison.

**N4: `space_del` focus handling.** Section 5.6 says that if the active space was N,
switch to space 1. It does not specify what happens to the focused app if it was on space
N (now reassigned to space 1). The focus transfer logic from §9.4 should be explicitly
called by the `space_del` handler after reassigning windows, to ensure `FocusedApp` is
consistent with the new state. Specifically: after all `win_space == N` apps are
reassigned to space 1, and after the active space is set to 1, call the focus transfer
logic for space 1. If the previously focused app was on space N, it is now on space 1 and
can retain focus — no focus transfer needed. If the previously focused app was already on
space 1 (the case where `space_del N` is called while viewing space M ≠ N), the focus is
also already consistent. The only case requiring action is `space_del N` where N was
the active space and none of space N's windows existed before the reassignment completed;
in this case the focus transfer correctly selects the topmost window in space 1.

**N5: `reset_tiling` command not in §5.** The command `@supervisor: reset_tiling <app>`
is mentioned in §10.4 but not listed in §5 (Window Commands). It needs a formal entry in
§5 with parameter description, behaviour, and reply format for completeness.

**N6: Mobile profile `minimize` behaviour.** Section 12.1 says on mobile "only one app is
visible at a time; others are effectively minimized behind it." But the `minimize` command
is rejected as "not supported on mobile profile." This creates an inconsistency: the WM
internally minimizes apps to implement the single-visible-app model, but external callers
cannot use `minimize`. The spec should clarify that internal WM-driven minimization is
allowed on mobile; only externally-issued `minimize` commands from IPC are rejected. The
mobile focus-switch path must be described: when `focus <app>` is called on mobile, the
currently visible app is internally minimized (WM sets `minimized = true` without the
external `minimize` command path) and the new app is un-minimized. This is a two-step
atomic update from the compositor's perspective — both changes should be made while
holding `vsync_lock.write()` to avoid a single-frame flash where both apps are visible
simultaneously.

**N7: Fullscreen and tiling interaction.** Section 2.1 says "at most one window per space
can be fullscreen at a time; the WM enforces this by clearing the flag on all other
windows." When a second app goes fullscreen, all other apps in the space have
`win_fullscreen` cleared. But the tiling algorithm (§10.3) says "the auto-tiling algorithm
treats a fullscreen window as if it is the only window." If a fullscreen window is the
only app being tiled, the other apps (whose fullscreen was cleared) need to be re-tiled
into their previous positions. The spec does not describe the position restoration logic
when fullscreen is cleared. A `pre_fullscreen_region` field similar to
`pre_minimize_region` is implied but not specified.

**N8: `assign` to a non-existent space.** Section 5.8 says "if `space >= 1`, verify
`SpaceRegistry::exists(space)`." But it does not specify the reply format on error. All
other error replies in the WM commands follow the pattern "error: <reason>". The
`assign` error case should explicitly state: reply `error: space <N> does not exist`.
This is a documentation gap, not a functional issue.

**N9: `win_space` for non-display apps.** The spec adds `win_space: u8` to `AppState`,
which is created for every app including non-display apps (http-server, ping, etc.). For
non-display apps, `win_space` is meaningless — they have no Surface and are never
composited. The spec should state that `win_space` defaults to 0 for non-display apps and
that the WM command `assign <non-display-app> <space>` is rejected with an error. Without
this clarification, an implementer may allow the `assign` command on non-display apps,
which would silently update a field that has no effect, confusing operators.

**N10: `wm_list_windows` includes minimized windows.** Section 8.3 defines the response
format with a `<minimized>` boolean field. This is correct and useful. However, it is
unclear whether `wm_list_windows` returns windows from all spaces or only the active
space. It should return all windows from all spaces (with their `<space>` field populated)
to give a complete system view, consistent with `ps` which lists all running processes
regardless of their display state. The spec should explicitly state the scope of
`wm_list_windows` to prevent implementations that only return active-space windows.

---

## What the Spec Got Right

**Clean `SpaceRegistry` design.** The bitmask representation for allocated spaces is
compact, correct, and easy to test. The `exists()` / `add()` / `del()` API is small and
has no hidden state. The choice of `u16` for the bitmask (9 spaces = 9 bits) is precise
and self-documenting.

**Partial auto-tiling philosophy.** Despite the implementation gap identified in B5, the
motivation for partial auto-tiling (skip manual windows, tile the rest) is the right long-
term model. The spec correctly identifies "always override manual positions" as disruptive
and "disable tiling after any manual move" as too conservative. The stated fix (floating
vs tiling layers) is a well-known pattern in WM literature.

**Surface reallocation using vsync_lock.write().** Section 6.3 correctly identifies
`vsync_lock.write()` as the serialisation point for surface swaps. The reasoning (write
lock blocks until compositor read lock releases) is correct given the existing
`Arc<RwLock<()>>` topology. The spec just needs to close the re-entrancy gap raised in B1.

**WIT capability gating on `shell`.** Gating `vyoma:window-manager` behind `shell = true`
is the right capability model. A display-only app should not be able to move other apps'
windows. Reusing the existing `shell` capability avoids adding a new manifest field for a
narrow use case.

**`VYOMA_WM:` stdout protocol mirroring IPC commands.** Providing both a stdout protocol
and an IPC command interface for the same operations follows the established VyomaOS
pattern (R20 provides both `VYOMA_DISPLAY:` stdout and `vyoma:display-info@1.0.0` WIT).
This ensures apps compiled before WIT support was added can still use WM features.

**500-line file limit discipline.** The proposed `supervisor/src/wm/` layout (6 files, 80–
200 lines each) is consistent with the repo's file size rule. The commands file at ~200
lines is the largest and is appropriately bounded.

**Logical-point coordinate system.** The decision to store all WM state in logical points
and convert to physical pixels only at compositor time (§2.4, §6.2) is correct and
consistent with R20's HiDPI model. It means the WM algorithms (tiling, z-order, focus
transfer) are all scale-factor-agnostic, which simplifies testing and future extension
to fractional scale factors. The alternative (store physical pixels and convert to points
on query) would require all WM callers to carry a `DisplayConfig` reference, coupling
the WM tightly to the display stack.

**Event delivery via existing IPC inbox.** Using `send_reply` (the existing IPC inbox
channel) for WM events (`VYOMA_WM:focused`, `VYOMA_WM:resized`, etc.) avoids introducing
a second per-app channel. This is the right choice: WM events are low-frequency and their
delivery latency requirements are the same as IPC replies. A dedicated WM event channel
would add complexity for no observable benefit at this scale.

**Deferred mouse dragging.** Explicitly deferring drag-based window repositioning to R32
is the right call. Adding mouse drag support in R21 would require the WM to register for
mouse move events, track drag state, and handle edge cases (drag off screen, drag past
another window's edge). That scope belongs in a focused mouse interaction round. The R21
WM is complete and useful without dragging: the `move` command provides full programmatic
control, and apps like `shell` can use it to reposition their own windows via
`VYOMA_WM:move:shell,x,y` from their stdout.

---

## Questions for the Architect

**Q1**: The spec defers window chrome (title bars, close/minimize/maximise buttons) to R23.
But §11 introduces `VYOMA_WM:minimized` and `VYOMA_WM:restored` events, and §2.1 adds
`win_fullscreen` to `AppState`. If chrome is deferred to R23, what component in R21
actually triggers `minimize` and `fullscreen` state changes — only external IPC commands?
Will there be a gap where the minimize event exists but there is no visual affordance (no
button) to trigger it from within the GUI? Clarify the intended user-visible path for R21
minimize vs R23 chrome minimize.

A practical follow-on: the `shell` app currently provides the primary UI for supervisor
commands. In R21 the intended path for minimize is presumably `@supervisor: minimize
<app>` typed at the shell prompt. This is functional but requires a keyboard-accessible
shell. For headless CI environments this is fine. For interactive demos, the absence of a
clickable minimize button means window management cannot be demonstrated visually until
R23. The spec should acknowledge this gap explicitly and note it as a known limitation of
the R21 scope rather than leaving it implied.

**Q2**: Section 9.4 says focus transfers to "the topmost window in space N" when switching
spaces. "Topmost" means highest `win_z`. But what if the topmost window in space N is
minimized? The focus transfer logic should skip minimized windows and walk down the z-order
until it finds a non-minimized window. The spec does not describe this. Is the intent to
never focus a minimized window, and if so, what is the fallback when all windows in space N
are minimized? The expected behaviour (by analogy with macOS) is: when switching to a
space where all windows are minimized, `FocusedApp` becomes `None` and no app receives
keyboard events. The keyboard input router already handles `FocusedApp == None` (it
discards keypresses). This should be stated explicitly so the focus transfer function has
a defined terminal case rather than leaving the `FocusedApp` pointing at a minimized
window that cannot receive input events meaningfully.

**Q3**: The `assign <app> 0` command places an app on "all spaces." What happens to a
space-0 app's `win_space` when `space_del` is called? Section 5.6 says "re-assign all apps
with `win_space == N` to space 1." A space-0 app has `win_space == 0`, so it is unaffected.
This is correct behaviour (space-0 apps should survive space deletion), but it should be
stated explicitly in §5.6 to prevent an implementer from writing a loop that
unintentionally catches space-0 apps. The implementation guard is a single condition:
`if app.win_space == n { app.win_space = 1; }` — note `n >= 1` is a precondition of
`space_del` so `n == 0` is already unreachable, but an explicit assert or comment in the
code clarifies intent. The spec should add: "Apps with `win_space == 0` (all-spaces) are
not reassigned by `space_del`; they remain on space 0 and continue appearing on all
remaining spaces."

**Q4**: Can a space-0 app receive `VYOMA_WM:space_changed:<N>` events? Section 11.1 says
the event is sent to "every app that has `display = true`." A space-0 app has `display =
true` (otherwise it would not have a Surface). So yes, it would receive space change events.
Is this the intended behaviour? A pinned overlay app receiving space change events might
use them to update its display (e.g., a taskbar showing which space is active). This should
be stated explicitly as a feature rather than a side effect.

**Q5**: The `resize` command specifies "clamp to `[min_size.0, display_pts_w]`." What if
a window is on a virtual display (R19) whose `width_pts` differs from the physical
display? The clamping upper bound should use the display width for the display the window
is on, not a global `display_pts_w`. For R21, since virtual displays are separate composit
targets, does the WM need to know which display a window is on? This interaction with R19
is unaddressed.

More broadly: R19 introduced virtual displays but `AppState` does not currently have a
`display_id` field associating a window with a specific display. The compositor in R19
presumably uses some per-display routing logic. R21's WM tiling algorithm (`compute_tiling`
called with `DisplayConfig.width_pts()`/`height_pts()`) will use the physical display
dimensions for all windows, including those on virtual displays with different resolutions.
For R21 this is probably acceptable as an out-of-scope limitation, but the spec should
explicitly state: "R21 WM operates against the primary physical display dimensions;
virtual display support is deferred to the multi-monitor round." This prevents implementers
from silently producing incorrect tiling on R19 virtual display setups.

---

## Closing Assessment

Round 21 correctly identifies the three structural problems with the current compositor
model (no logical window abstraction, no spaces, unmanaged z-order) and proposes coherent
solutions to all three. The `SpaceRegistry`, the compaction algorithm, and the partial auto-
tiling model are all production-quality designs. The WIT interface is well-scoped and the
file layout respects the repo's 500-line constraint.

The spec is not implementable as written due to the five blocking issues: B1 (reallocation
re-entrancy) and B3 (space switch atomicity) are data-race hazards; B2 (z-order shared
across spaces) is a correctness bug; B4 (space-0 z-order contradiction) leaves the
compositor undefined; and B5 (partial tiling fragmentation) will produce visual corruption
on the first non-trivial window arrangement. All five are fixable without structural changes
to the design — they require precision additions to the spec, not redesigns. The total
estimated text addition for the revised spec to resolve all blocking issues is fewer than
60 lines, distributed across §4.2, §4.4, §6.3, §6.4, and §10.4.

The non-blocking issues (N1–N10) are documentation and edge-case gaps. None of them
blocks implementation, but N2 (`win_title` default), N4 (`space_del` focus), and N9
(`assign` on non-display apps) are close to blocking in practice because they will produce
incorrect or confusing observable behaviour in the first integration test. They should be
resolved in the same revision pass as B1–B5.

The questions for the architect (Q1–Q5) reveal three areas where R21 interfaces with
earlier rounds in underspecified ways: the R19 virtual display interaction (Q5), the R23
chrome affordance gap (Q1), and the focus-on-minimized-window edge case (Q2). These do
not require answers before implementation can begin, but Q2's answer should be reflected
in the focus-transfer logic before the first test of multi-space switching.

With B1–B5 addressed and the N2/N4/N9 gaps closed, R21 delivers a solid foundation for
R22 (window chrome) and R24 (dock/taskbar), both of which depend on `win_title`, `win_z`,
and the space model introduced here.

**Approve with revisions**: implement after B1–B5 are addressed in a revised spec.
Non-blocking issues N1–N10 should be resolved in the same revision pass where feasible.
