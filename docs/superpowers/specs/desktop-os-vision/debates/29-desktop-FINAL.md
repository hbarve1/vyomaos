# FINAL Spec: Desktop & Wallpaper Engine (Round 29)

**Subsystem**: Desktop & Wallpaper Engine  
**macOS Analogue**: Finder desktop / wallpaper daemon  
**Depends on**: R21 (compositor, vsync_lock, fb.fill), R26 (Stage Manager strip), R27 (FS suppression flags pattern)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture: Supervisor-Rendered Wallpaper

Wallpaper is rendered by the supervisor compositor — `fill_wallpaper()` replaces
`fb.fill(BACKGROUND_COLOR)` in `vsync_tick`. No dedicated WASM app needed. Rationale:
- No startup sync race (WASM app vs compositor first frame)
- No z=1 slot reservation per space
- No 9 full-screen surface allocations
- Existing `fb.fill` in space-switch block is the natural hook

Space-0 apps (chrome, dock, MC, stage-strip) composite on top as normal.

Desktop icons: deferred to R41 (File Manager). This spec covers background rendering only.

---

## 2. Data Model

```rust
// supervisor/src/desktop/wallpaper.rs

#[derive(Clone)]
pub enum WallpaperKind {
    Solid { rgba: u32 },
    LinearGradient { from_rgba: u32, to_rgba: u32, angle_deg: u16 },
    Image { path: String, scale_mode: ScaleMode },
}

#[derive(Clone, Copy)]
pub enum ScaleMode { Fill, Fit, Stretch, Center }

#[derive(Clone)]
pub struct WallpaperConfig {
    pub kind: WallpaperKind,
    pub time_variants: Vec<(u8, WallpaperKind)>,  // (hour_utc, kind); sorted
}

pub struct WallpaperCache {
    pub images: HashMap<String, (u32, u32, Vec<u8>)>,  // path → (w, h, BGRA pixels)
}
```

`SupervisorState` additions:
```rust
pub wallpapers: HashMap<u8, WallpaperConfig>,         // space → config
pub wallpaper_cache: Arc<RwLock<WallpaperCache>>,     // B1: RwLock-protected
pub wallpaper_preload_complete: AtomicBool,            // B2: mount-ordering guard
pub screensaver_idle_ticks: u32,
pub screensaver_active: bool,
```

---

## 3. `WallpaperCache` Thread Safety (B1 Fix)

```rust
pub wallpaper_cache: Arc<RwLock<WallpaperCache>>,
```

- `fill_wallpaper` acquires `wallpaper_cache.read()` — non-blocking in typical case
- `handle_wallpaper_set` and `preload_wallpaper_image` acquire `wallpaper_cache.write()`
- **Invariant**: `wallpaper_cache.write()` is NEVER acquired from inside a `vsync_lock` critical section. Only IPC thread or boot path writes to cache.

---

## 4. Boot Ordering — Mount-Aware Preload (B2 Fix)

```rust
// supervisor/src/main.rs — after 9P mount readiness confirmed

fn on_data_mount_ready(state: &mut SupervisorState) {
    load_wallpaper_config(state);   // reads /data/wallpaper.toml
    preload_all_configured_images(state);
    state.wallpaper_preload_complete.store(true, Ordering::Release);
}
```

`fill_wallpaper` fallback when not ready:
```rust
pub fn fill_wallpaper(fb: &mut FrameBuffer, space: u8, state: &SupervisorState,
                       config: &DisplayConfig) {
    if !state.wallpaper_preload_complete.load(Ordering::Acquire) {
        fb.fill_rows(0, config.physical_height, BACKGROUND_COLOR);
        return;
    }
    // normal rendering below
}
```

This ensures solid-color `#1E1E2E` fills on boot frames before `/data` is mounted —
correct and expected behavior, not a bug.

---

## 5. Compositor Integration

### 5.1 Space-Switch Block (Inside `vsync_lock.write()`)

```rust
// supervisor/src/compositor.rs — vsync_tick() space switch block

if pending != 0 {
    let _write = vsync_lock.write();
    spaces.active = pending;
    // Replace fb.fill(BACKGROUND_COLOR) with fill_wallpaper:
    fill_wallpaper(fb, pending, &state, &config);
    mark_new_space_dirty(&mut apps, pending);
    compact_z_order(&mut apps, pending);
    // FS suppression flags (R27) ...
}
```

### 5.2 Normal Frame Dirty Redraw

```rust
// supervisor/src/compositor.rs — vsync_tick() blit pass

let _read = vsync_lock.read();
// Fill wallpaper before blitting any app surfaces (Step 3a)
if frame_needs_wallpaper_redraw(&state) {
    let wc = state.wallpaper_cache.read();
    fill_wallpaper_locked(fb, spaces.active, &state.wallpapers, &wc, &config);
}
// then blit all app surfaces (7-step blit_clipped)
```

`frame_needs_wallpaper_redraw` returns true on space switch, wallpaper change, or any
app surface dirtied (uncovered region may need wallpaper fill).

---

## 6. `fill_wallpaper` Implementation

```rust
// supervisor/src/desktop/render.rs

pub fn fill_wallpaper_locked(
    fb: &mut FrameBuffer,
    space: u8,
    wallpapers: &HashMap<u8, WallpaperConfig>,
    cache: &WallpaperCache,
    config: &DisplayConfig,
) {
    let wc = wallpapers.get(&space)
        .unwrap_or(&WallpaperConfig { kind: WallpaperKind::Solid { rgba: BACKGROUND_COLOR },
                                       time_variants: vec![] });
    match &wc.kind {
        WallpaperKind::Solid { rgba } => fb.fill_rows(0, config.physical_height, *rgba),
        WallpaperKind::LinearGradient { from_rgba, to_rgba, angle_deg } =>
            fill_gradient(fb, *from_rgba, *to_rgba, *angle_deg, config),
        WallpaperKind::Image { path, scale_mode } => {
            if let Some((iw, ih, pixels)) = cache.images.get(path) {
                blit_wallpaper_image(fb, pixels, *iw, *ih, *scale_mode, config);
            } else {
                fb.fill_rows(0, config.physical_height, BACKGROUND_COLOR);
            }
        }
    }
}
```

### 6.1 `blit_wallpaper_image` — HiDPI Fill Fix (B4 Fix)

```rust
fn blit_wallpaper_image(fb: &mut FrameBuffer, pixels: &[u8], src_w: u32, src_h: u32,
                         mode: ScaleMode, config: &DisplayConfig) {
    let dst_w = config.physical_width;
    let dst_h = config.physical_height;
    match mode {
        ScaleMode::Fill => {
            let scale = f32::max(dst_w as f32 / src_w as f32, dst_h as f32 / src_h as f32);
            // B4 fix: ceil() ensures scaled_w >= dst_w — no right-edge seam
            let scaled_w = (src_w as f32 * scale).ceil() as u32;
            let scaled_h = (src_h as f32 * scale).ceil() as u32;
            let off_x = scaled_w.saturating_sub(dst_w) / 2;
            let off_y = scaled_h.saturating_sub(dst_h) / 2;
            blit_scaled(fb, pixels, src_w, src_h, off_x, off_y, scaled_w, scaled_h, dst_w, dst_h);
        }
        ScaleMode::Fit => {
            let scale = f32::min(dst_w as f32 / src_w as f32, dst_h as f32 / src_h as f32);
            // Fit: floor() keeps within bounds; letterbox fills with BACKGROUND_COLOR
            let scaled_w = (src_w as f32 * scale).floor() as u32;
            let scaled_h = (src_h as f32 * scale).floor() as u32;
            fb.fill_rows(0, dst_h, BACKGROUND_COLOR);
            let dest_x = (dst_w.saturating_sub(scaled_w)) / 2;
            let dest_y = (dst_h.saturating_sub(scaled_h)) / 2;
            blit_scaled_at(fb, pixels, src_w, src_h, dest_x, dest_y, scaled_w, scaled_h);
        }
        ScaleMode::Stretch => {
            // No float arithmetic — direct dst dimensions
            blit_scaled(fb, pixels, src_w, src_h, 0, 0, dst_w, dst_h, dst_w, dst_h);
        }
        ScaleMode::Center => {
            fb.fill_rows(0, dst_h, BACKGROUND_COLOR);
            let x = (dst_w.saturating_sub(src_w)) / 2;
            let y = (dst_h.saturating_sub(src_h)) / 2;
            blit_region(fb, pixels, src_w, src_h, x, y, src_w.min(dst_w), src_h.min(dst_h));
        }
    }
}
```

---

## 7. Persistence — `/data/wallpaper.toml`

```toml
# /data/wallpaper.toml — per-space wallpaper config

[default]
kind = "solid"
rgba = 0x1E1E2EFF

[space.1]
kind = "image"
path = "/data/wallpapers/sonoma-dawn.bgra"
scale_mode = "fill"

[space.1.time_variants]
# hour_utc = 0..23
[[space.1.time_variants]]
hour = 6
kind = "image"
path = "/data/wallpapers/sonoma-dawn.bgra"

[[space.1.time_variants]]
hour = 18
kind = "image"
path = "/data/wallpapers/sonoma-dusk.bgra"
```

Loaded at boot after `/data` mount. Write-through on every `wallpaper_set`: supervisor
serializes the updated config and writes to `/data/wallpaper.toml` immediately.

---

## 8. Dynamic Wallpapers — Time Variants

```rust
// supervisor/src/desktop/time.rs

pub fn maybe_apply_time_variant(
    space: u8,
    wallpapers: &mut HashMap<u8, WallpaperConfig>,
    cache: &Arc<RwLock<WallpaperCache>>,
    now_hour_utc: u8,
) -> bool {  // returns true if wallpaper changed
    let wc = match wallpapers.get_mut(&space) {
        Some(w) => w,
        None => return false,
    };
    let target = wc.time_variants.iter()
        .rev()
        .find(|(h, _)| *h <= now_hour_utc)
        .or_else(|| wc.time_variants.last())
        .map(|(_, k)| k.clone());
    if let Some(kind) = target {
        if std::mem::discriminant(&kind) != std::mem::discriminant(&wc.kind)
            || !wallpaper_kinds_eq(&kind, &wc.kind)
        {
            wc.kind = kind.clone();
            // Pre-load image if needed (outside vsync_lock — B3 fix)
            if let WallpaperKind::Image { path, .. } = &kind {
                let mut cache_w = cache.write();
                if !cache_w.images.contains_key(path) {
                    load_image_into_cache(&mut cache_w, path);
                }
            }
            return true;
        }
    }
    false
}
```

Called in `vsync_tick` after DRM blit (outside `vsync_lock`), once per minute
(`tick_counter % 3600 == 0` at 60fps). If returns true, sets `wallpaper_dirty` flag
for next frame.

**B3 Fix**: `wallpapers` and `wallpaper_cache` both accessed after `vsync_lock` is dropped.
`SupervisorState` is behind `Arc<Mutex<SupervisorState>>`; `maybe_apply_time_variant` is
called while the state mutex is held by the compositor thread at end of `vsync_tick`.
`wallpaper_cache.write()` does not nest inside any `vsync_lock` section.

---

## 9. Capability Gating — Shell vs Display (B5 Fix)

Wallpaper **mutations** require `shell = true`. Wallpaper **queries** require `display = true`.

```rust
// supervisor/src/desktop/commands.rs

pub fn handle_desktop_cmd(app: &AppState, line: &str, state: &mut SupervisorState) {
    let is_mutation = matches!(line.split(':').next().unwrap_or(""),
        "wallpaper_set" | "wallpaper_reload" | "wallpaper_reset" |
        "screensaver_set" | "screensaver_dismiss");

    if is_mutation && !app.capabilities.shell {
        log::warn!(app = &app.name, "wallpaper mutation requires shell capability");
        return;
    }
    if !is_mutation && !app.capabilities.display {
        log::warn!(app = &app.name, "wallpaper query requires display capability");
        return;
    }
    // dispatch ...
}
```

WIT `init_linker_for_app`:
- `set-wallpaper` / `reset-wallpaper` / `set-screensaver` → wired only if `shell = true`
- `get-wallpaper` → wired only if `display = true`

A settings app configuring wallpapers declares `shell = true`. No framebuffer surface
allocated until/unless it also declares `display = true`.

---

## 10. `VYOMA_DESKTOP:` Protocol

### 10.1 App → Supervisor (stdout)

Mutation commands (require `shell = true` in manifest):
```
VYOMA_DESKTOP:wallpaper_set:<space>,solid,<rgba>
VYOMA_DESKTOP:wallpaper_set:<space>,gradient,<from_rgba>,<to_rgba>,<angle>
VYOMA_DESKTOP:wallpaper_set:<space>,image,<path>,<fill|fit|stretch|center>
VYOMA_DESKTOP:wallpaper_reset:<space>
VYOMA_DESKTOP:wallpaper_reload
VYOMA_DESKTOP:screensaver_set:<idle_secs>
VYOMA_DESKTOP:screensaver_dismiss
```

Query commands (require `display = true` in manifest):
```
VYOMA_DESKTOP:wallpaper_get:<space>
```

### 10.2 Supervisor → App (stdin push)

```
VYOMA_DESKTOP:wallpaper_changed:<space>,<kind>   ← broadcast on any wallpaper change
VYOMA_DESKTOP:screensaver_will_start:<secs>       ← N seconds before screensaver
VYOMA_DESKTOP:screensaver_started
VYOMA_DESKTOP:screensaver_dismissed
```

---

## 11. Screen Saver (v1 — Blank Only)

`screensaver_idle_ticks: u32` incremented each vsync frame; reset to 0 on any keyboard
or mouse event. Default idle threshold: 600 seconds (≈36000 ticks at 60fps).

When threshold exceeded:
1. Set `screensaver_active = true`
2. `fill_wallpaper` replaced by `fb.fill_rows(0, physical_h, 0x00000000)` — blank black
3. `INPUT_LOCK_LEVEL` unchanged (keyboard wakes screensaver: first keypress dispatches
   `screensaver_dismiss` internally, second keypress routes normally)

Animated screensavers (pluggable WASM) deferred to future round.

---

## 12. Stage Manager Integration

`fill_wallpaper` fills the full framebuffer (including strip area). The `stage-strip`
WASM app (space-0, z=65532) composites on top with its own background panel. No
wallpaper blurring in v1 (deferred to blur filter support).

---

## 13. WIT Interface `vyoma:desktop@1.0.0`

```wit
package vyoma:desktop@1.0.0;

interface desktop-config {
    /// Set wallpaper for a space (requires shell capability).
    set-wallpaper: func(space: u8, kind: wallpaper-kind) -> result<_, string>;

    /// Reset space to default wallpaper (requires shell capability).
    reset-wallpaper: func(space: u8) -> result<_, string>;

    /// Get current wallpaper config (requires display capability).
    get-wallpaper: func(space: u8) -> result<wallpaper-kind, string>;

    /// Configure screensaver idle threshold in seconds (requires shell capability).
    set-screensaver-idle: func(secs: u32) -> result<_, string>;
}

variant wallpaper-kind {
    solid(u32),
    gradient(u32, u32, u16),
    image(string, scale-mode),
}

enum scale-mode { fill, fit, stretch, center }
```

---

## 14. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Wallpaper engine | yes | lock-screen only | no | no |
| Per-space wallpaper | yes | no (single) | no | no |
| Image wallpaper | yes | yes | no | no |
| Dynamic (time-variant) | yes | no | no | no |
| Screen saver | yes (blank) | no | no | no |
| Desktop icons | deferred to R41 | no | no | no |

---

## 15. File Layout

```
supervisor/src/
├── desktop/
│   ├── mod.rs         (re-exports)
│   ├── wallpaper.rs   (WallpaperConfig, WallpaperKind, ScaleMode, WallpaperCache)
│   ├── render.rs      (fill_wallpaper_locked, blit_wallpaper_image, fill_gradient)
│   ├── time.rs        (maybe_apply_time_variant, wallpaper_kinds_eq)
│   ├── commands.rs    (handle_desktop_cmd: capability gates B5, dispatch)
│   └── persist.rs     (load_wallpaper_config, save_wallpaper_config, on_data_mount_ready)
└── compositor.rs      (fill_wallpaper call in space-switch and blit pass)
```

---

## 16. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: WallpaperCache data race between IPC writer and compositor reader | `wallpaper_cache: Arc<RwLock<WallpaperCache>>`; compositor acquires `read()` inside `fill_wallpaper`; IPC acquires `write()` during set/load; never write inside `vsync_lock` section |
| B2: `load_wallpaper_config` before `/data` mount ready | `wallpaper_preload_complete: AtomicBool`; `fill_wallpaper` returns solid fallback while false; `on_data_mount_ready` called after mount confirmed, sets flag true |
| B3: `maybe_apply_time_variant` mutates wallpapers and cache with no lock spec | Called after `vsync_lock` drops; `SupervisorState` behind `Arc<Mutex>`; `wallpaper_cache.write()` only acquired outside any `vsync_lock` section |
| B4: `as u32` truncation in Fill mode leaves 1-pixel seam on HiDPI | `scaled_w = (src_w as f32 * scale).ceil() as u32` for Fill mode; `floor()` for Fit; `dst_w` directly for Stretch |
| B5: `VYOMA_DESKTOP:` mutations gated on `display = true` — over-provisions | Mutations gate on `shell = true`; queries gate on `display = true`; WIT `init_linker_for_app` wires accordingly |
