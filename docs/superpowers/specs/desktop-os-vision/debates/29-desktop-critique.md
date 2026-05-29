# Critique: Desktop & Wallpaper Engine (Round 29)

**Critiquing**: `29-desktop.md`  
**Critic role**: blocking issues only — correctness, threading safety, integration coherence

---

## CRITIQUE — 5 Blocking Issues

---

### B1: `fill_wallpaper` Called Under `vsync_lock.read()` While `WallpaperCache` Is Written by IPC Thread — Data Race

**Problem**:

Section 4.2 places `fill_wallpaper` in Step 3a of the compositor blit pass, which runs
under `vsync_lock.read()`. The `fill_wallpaper` call receives `&state.wallpaper_cache` as
a shared reference. Concurrently, the IPC handler `handle_wallpaper_set` (Section 9.1)
calls `state.wallpaper_cache.get_or_load(...)` which inserts into
`cache.images: HashMap<String, (u32, u32, Vec<u8>)>`. This is a classic read/write data
race on the `HashMap` — the IPC thread (writer) and the compositor thread (reader) access
the same `HashMap` without synchronization beyond `vsync_lock`, which the IPC thread does
**not** hold during `get_or_load`.

The spec states "Image loading always happens in IPC handler or boot (outside vsync lock).
Under read-lock, only already-loaded pixels are accessed" (Section 18, risk table). But
this is not enforced by the type system: `fill_wallpaper` receives `&WallpaperCache` and
the IPC thread holds `&mut WallpaperCache` — Rust's borrow checker would prevent
simultaneous mutable and shared borrows of the same struct, but only if they live in the
same mutex/lock scope. Because `WallpaperCache` is embedded in `SupervisorState` which is
shared across threads (IPC thread + compositor thread), a `Mutex` or `RwLock` wrapping
`WallpaperCache` is necessary.

The current spec embeds `wallpaper_cache: WallpaperCache` bare in `SupervisorState` with
no locking annotation, leaving the threading contract implicit and unenforceable.

**Proposed fix**:

Wrap `WallpaperCache` in `Arc<RwLock<WallpaperCache>>` stored on `SupervisorState`:

```rust
pub wallpaper_cache: Arc<RwLock<WallpaperCache>>,
```

`fill_wallpaper` acquires a `wallpaper_cache.read()` lock (non-blocking if IPC writer is
not currently inserting — typical case). `handle_wallpaper_set` acquires `write()` to
insert. The read-lock nests inside `vsync_lock.read()` — this is safe as long as no code
path holds `wallpaper_cache.write()` while also holding `vsync_lock` in any form. Document
the invariant: `wallpaper_cache.write()` is only acquired from the IPC thread; never from
inside a `vsync_lock` critical section.

---

### B2: Space Switch Replaces `fb.fill(BACKGROUND_COLOR)` — Wallpaper Not Yet in Cache on First Boot Causes a One-Frame Solid Fallback That Is Visible

**Problem**:

Section 5.1 and the `vsync_tick` code in Section 4.2 replace `fb.fill(BACKGROUND_COLOR)`
with `fill_wallpaper` inside the `vsync_lock.write()` space-switch block. Section 8.2
states that `load_wallpaper_config` pre-loads images at boot. However, the pre-load
function `preload_wallpaper_image` calls `cache.get_or_load(path, is_png)`, which may fail
silently if the file does not exist at boot (e.g. user configured a path that hasn't been
copied yet, or `/data` is not yet mounted when `load_wallpaper_config` runs).

More critically: the ordering of `load_wallpaper_config` relative to `/data` mount is
unspecified. If `load_wallpaper_config` runs before the 9P virtio `/data` mount is ready,
every `std::fs::read` in `WallpaperCache::load_png` returns an error. The images are never
inserted into the cache. The compositor then hits the `else { fb.fill(0x1E1E2EFF) }` fallback
on every frame until the next `wallpaper_reload` IPC command. Since the compositor has
already entered its loop, that command never comes automatically.

The spec's risk table (Section 18) acknowledges this case but says "Boot pre-loads all
configured images" without specifying the mount ordering. This is a blocking gap for
reliable image wallpapers at startup.

**Proposed fix**:

Add an explicit `wallpaper_preload_complete: AtomicBool` flag on `SupervisorState`,
initialized to `false`. After the `/data` 9P mount is confirmed ready (supervisor already
polls for mount readiness before spawning apps), call `load_wallpaper_config` and set
`wallpaper_preload_complete.store(true, Ordering::Release)`. In `fill_wallpaper`, if
`wallpaper_preload_complete` is still `false`, use `fb.fill(BACKGROUND_COLOR)` without
logging a warning (expected transient state). Add a `wallpaper_reload_all` path that is
called automatically once after mount readiness is confirmed, ensuring images are loaded
before the first user-visible frame.

Additionally: document in the supervisor boot sequence that `load_wallpaper_config` is
called **after** the 9P mount readiness check, not before.

---

### B3: `maybe_apply_time_variant` Mutates `wc.kind` In-Place — Active Space Gets Wallpaper Replacement Without `fill_wallpaper` Seeing It Until Next Tick; But More Critically, Mutation Inside the Compositor Tick Violates the Read-Lock Constraint

**Problem**:

Section 10.2 places `maybe_apply_time_variant` inside `vsync_tick` after Step 4 (DRM
blit). It takes `wallpapers: &mut HashMap<u8, WallpaperConfig>` and mutates `wc.kind` in
place. However, `vsync_tick` holds `vsync_lock.read()` in Step 3 (the blit pass), and the
`maybe_apply_time_variant` call occurs **after** the `vsync_lock.read()` guard has been
dropped (in Step 5).

The issue is subtler: `maybe_apply_time_variant` also calls `preload_wallpaper_image` to
pre-load the new image variant into the cache, which calls `cache.get_or_load` — a
mutable operation on `WallpaperCache`. If the fix from B1 wraps `WallpaperCache` in
`Arc<RwLock<WallpaperCache>>`, then `maybe_apply_time_variant` must acquire
`wallpaper_cache.write()` here. This is fine since it is outside `vsync_lock`. But the
spec as written passes `cache: &mut WallpaperCache` with no lock, making the invocation
order and lock state implicit.

Furthermore, `maybe_apply_time_variant` takes `wallpapers: &mut HashMap<u8,
WallpaperConfig>` — a mutable borrow of the entire wallpaper map — while `fill_wallpaper`
in Step 3a accessed the same map via `state.wallpapers.get(&active)`. If `SupervisorState`
is behind a single `Mutex<SupervisorState>` and both the compositor thread and IPC thread
hold the mutex lock in different phases, the mutation ordering is well-defined. But the
spec does not define how `SupervisorState` is shared or locked across the compositor and
IPC threads — this is a foundational gap.

**Proposed fix**:

Specify that `SupervisorState` is held behind `Arc<Mutex<SupervisorState>>` (consistent
with the existing R21 pattern of `apps: Arc<Mutex<HashMap<String, AppState>>>`). The
compositor thread acquires the mutex at the start of `vsync_tick` (or uses a snapshot
pattern for the blit pass). `maybe_apply_time_variant` is called while the mutex is held
at the end of `vsync_tick`, after the vsync blit. Pre-loading of the new image variant
into the cache calls `wallpaper_cache.write()` (B1 fix) inside the same mutex-guarded
section. This makes the entire variant-switch sequence atomic from the perspective of the
IPC thread.

---

### B4: `blit_wallpaper_image` for `ScaleMode::Fill` Uses Integer Division That Produces a 1-Pixel Gap on Odd-Width Screens — Visible Artifact on HiDPI Displays

**Problem**:

Section 15 computes the crop offset as:

```rust
let off_x = (scaled_w.saturating_sub(dst_w)) / 2;
let off_y = (scaled_h.saturating_sub(dst_h)) / 2;
```

`scaled_w` and `scaled_h` are computed by rounding `src_w as f32 * scale` to `u32` via
`as u32` truncation (which is `floor`). On HiDPI displays (e.g. 2560×1600 physical) with
a non-integer scale factor, the combination of `floor` scaling and integer halving can
produce a result where `scaled_w < dst_w` (the scaled image is one pixel narrower than the
framebuffer), leaving a 1-pixel vertical strip of uninitialized (or previous-frame) pixels
on the right edge of the wallpaper.

Example: `src_w=1920, dst_w=2560, scale=2560/1920=1.333...`. `scaled_w = (1920 *
1.333) as u32 = 2559` (float truncation). `2559 < 2560 = dst_w`. The right edge pixel
column is never written by `blit_scaled`, leaving a visible seam.

This is a real defect on any HiDPI profile (`desktop-full` at 2× scaling = physical 2560+
width) and any wallpaper image with a non-integer scale ratio.

**Proposed fix**:

Use `f32::ceil()` instead of implicit truncation when computing `scaled_w` and `scaled_h`
for `Fill` mode. The one extra pixel from ceiling ensures coverage:

```rust
let scaled_w = (src_w as f32 * scale).ceil() as u32;
let scaled_h = (src_h as f32 * scale).ceil() as u32;
```

For `Fit` mode, use `f32::floor()` (keeping the letterbox within bounds). For `Stretch`
mode, `scaled_w = dst_w` directly — no float arithmetic. Add a bounds check in
`blit_scaled` that clamps `src_x` to `src_w - 1` (already present as `min(src_w - 1)`)
so the ceiling-added extra pixel samples the edge pixel rather than out-of-bounds memory.
Add a unit test for the 1920→2560 case confirming `off_x + dst_w <= scaled_w`.

---

### B5: `VYOMA_DESKTOP:` Protocol Requires `display = true` Capability — But the Wallpaper Change Command Is a System Configuration Operation That Has Nothing to Do With Display Rendering

**Problem**:

Section 9.3 states: "`VYOMA_DESKTOP:` commands require `display = true` in the app's
manifest." This gates wallpaper changes (a system configuration operation) behind the
display rendering capability. The consequence:

1. A settings app that only manages configuration (no framebuffer rendering) must declare
   `display = true` to change the wallpaper. This over-provisions the settings app with
   framebuffer access it does not need and should not have.

2. A `shell`-only CLI tool (`capabilities.shell = true`) cannot change the wallpaper via
   `VYOMA_DESKTOP:` stdout even though the shell is the natural place for wallpaper config
   commands. The spec's own IPC example (`wallpaper_set <space> solid <rgba>`) implies
   shell access, but Section 9.2 uses `VYOMA_DESKTOP:` stdout (which requires `display`).

3. The capability-secure model (CLAUDE.md: "Capabilities not declared are not wired up")
   is violated in spirit: `display = true` is about framebuffer access, not system config.
   Granting it to get wallpaper-set access silently gives the app a framebuffer surface
   slot, surface memory allocation, and draw command routing that it may not need.

This is a security boundary design flaw, not just a naming issue. The correct capability
for "modify system UI configuration" is `shell = true` (already used for `@supervisor:`
IPC commands that manage process lifecycle). Wallpaper configuration is analogous.

**Proposed fix**:

Wallpaper change commands shall require `shell = true`, not `display = true`. Specifically:

- `VYOMA_DESKTOP:wallpaper_set:*` and related mutation commands are gated on `shell = true`.
- `VYOMA_DESKTOP:wallpaper_get:*` (read-only query) may be gated on `display = true` (an
  app rendering the wallpaper needs to know what wallpaper is configured).
- The capability check in `desktop_cmd.rs` is updated:

```rust
let is_mutation = line.starts_with("wallpaper_set")
    || line.starts_with("wallpaper_reload")
    || line.starts_with("wallpaper_reset")
    || line.starts_with("screensaver_set")
    || line.starts_with("screensaver_dismiss");

if is_mutation && !app.capabilities.shell {
    log::warn!(event = "desktop_cmd_denied", app = &app.name,
               "wallpaper mutation requires shell capability");
    continue;
}
if !is_mutation && !app.capabilities.display {
    log::warn!(event = "desktop_cmd_denied", app = &app.name,
               "wallpaper query requires display capability");
    continue;
}
```

- The WIT interface `vyoma:desktop@1.0.0` `set-wallpaper` / `reset-wallpaper` functions
  are only wired by `init_linker_for_app` for apps with `shell = true`. `get-wallpaper` is
  wired for apps with `display = true`.

- Update `vyoma.toml` documentation examples: a wallpaper settings app declares
  `shell = true` (not `display = true`) to use mutation commands.
