# Architect Spec: Desktop & Wallpaper Engine (Round 29)

**Subsystem**: Desktop & Wallpaper Engine  
**macOS Analogue**: Finder desktop / wallpaper daemon (`legacyScreenSaver`, `com.apple.desktop`)  
**Depends on**: R21 (WM spaces, PerSpaceZ, PENDING_SPACE_SWITCH), R22 (app lifecycle), R24 (dock), R25 (INPUT_LOCK_LEVEL, last_flushed_snapshot), R26 (stage-strip, blit_clipped 7-step), R28 (focus_transfer, FOCUSED_APP)  
**Status**: DRAFT — pending critique round

---

## 1. Overview

The Desktop & Wallpaper Engine renders the lowest visual layer of each space. In macOS this
is `com.apple.desktop` writing through `legacyScreenSaver` and the dock-managed wallpaper
slot. In VyomaOS, the equivalent must be:

- The lowest visible z-order element in each space (below all app windows)
- Per-space configurable (switching spaces changes wallpaper)
- Able to render solid colors, linear gradients, and raw BGRA images in v1
- Persistent across reboots (config in `/data/wallpaper.toml`)
- Changeable at runtime via IPC command or `VYOMA_DESKTOP:` stdout protocol
- Platform-aware (full on desktop-full, different on mobile, absent on server-headless)

The desktop layer is **not** responsible for file icons (deferred to R41 — File Manager).

---

## 2. Architecture Choice: Supervisor-Rendered Background vs. Dedicated WASM App

Three options exist for where to render the wallpaper layer.

### Option A — Supervisor fills background inline

`fb.fill(BACKGROUND_COLOR)` already runs at the start of each space switch (R21, §5.3).
Extend this to draw a gradient or blit a pre-loaded raw BGRA buffer before the compositor
blit loop runs. No WASM app. No surface allocation overhead.

**Pros**:
- Zero additional WASM startup latency
- No inter-process sync between wallpaper update and first frame
- Background is guaranteed to be painted before any app window blit
- No z-order complexity — background is under everything by construction
- No `AppState` entry for the desktop; no space-0 slot consumed

**Cons**:
- Supervisor must bundle an image decoder (raw BGRA trivial; PNG requires `lodepng`)
- Dynamic wallpapers (time-of-day) require a timer callback in the compositor loop
- Supervisor `compositor.rs` grows in scope

### Option B — Space-N WASM app at z=1 (`desktop` app in each space)

A dedicated `desktop.wasm` app lives in each space at z=1. It renders wallpaper via
`VYOMA_DRAW:` protocol. Space switch naturally switches which `desktop` app's surface is
composited.

**Pros**:
- Wallpaper rendering logic encapsulated in a WASM app
- Future dynamic wallpapers (animated, interactive) compose naturally

**Cons**:
- Each space needs a separate `desktop` app instance (9 spaces = 9 WASM processes, each
  consuming a surface buffer of full-screen size — wasteful)
- A z=1 WASM app can race with other space-N apps; manual z=1 assignment creates a
  reserved slot that must be enforced globally
- `fb.fill(BACKGROUND_COLOR)` from R21 already clears before the blit pass; the desktop
  app is the first blit but there is still a race if the app has not yet flushed a frame
  (black flash on first display)
- Boot-order fragility: `desktop` must have flushed before any other space-N app paints

### Option C — Space-0 WASM app (`desktop` app, always-on-bottom)

A single `desktop` WASM app lives in space-0 at z=0 (lowest space-0 z-index). The R21
space-0 two-pass model blits space-0 apps AFTER space-N apps — so space-0 z=0 would be
rendered on top of everything, the opposite of what we want.

**Verdict**: Option C contradicts the Model A compositor invariant from R21/R26. Rejected.

### Decision: Option A — Supervisor-Rendered Background

**Rationale**:

1. `fb.fill(BACKGROUND_COLOR)` already exists in the compositor space-switch block
   (R21 §5.3). Extending it to `fill_wallpaper(fb, &wallpaper)` is a minimal diff that
   keeps all wallpaper state inside `SupervisorState`.

2. The compositor must paint the background before any surface blit. With a WASM app
   (Option B), supervisor must wait for the app's surface to be non-empty — adding
   startup synchronization. With supervisor-rendered background (Option A), there is no
   wait: the background is painted immediately in the compositor tick itself.

3. No space-N z=1 slot reserved. No z-order enforcement across spaces needed.

4. The 500-line file limit is respected: wallpaper rendering moves into
   `supervisor/src/wallpaper/mod.rs` (its own focused module).

5. Per-space wallpaper state lives in `SupervisorState.wallpapers: HashMap<u8, WallpaperConfig>`,
   loaded from `/data/wallpaper.toml` at boot and mutated via IPC.

The desktop app concept is retained for R41 (file icons on desktop), where a thin
`desktop.wasm` will need to exist as a space-N z=1 app per space to handle drop targets
and icon rendering. That design is explicitly deferred to R41.

---

## 3. Data Model

### 3.1 `WallpaperKind` Enum

```rust
// supervisor/src/wallpaper/config.rs

/// v1 supported wallpaper kinds.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WallpaperKind {
    /// Solid fill color (RGBA packed: R<<24 | G<<16 | B<<8 | A).
    SolidColor {
        rgba: u32,
    },

    /// Horizontal or vertical linear gradient between two RGBA colors.
    LinearGradient {
        from_rgba: u32,
        to_rgba: u32,
        angle_deg: u16,   // 0 = left-to-right, 90 = top-to-bottom; clamped to 0|90 in v1
    },

    /// Pre-decoded BGRA image stored at a path on /data.
    /// At load time supervisor reads the raw file and caches it in WallpaperCache.
    ImageBgra {
        path: String,     // absolute path inside VM, e.g. /data/wallpapers/space1.bgra
        width: u32,       // native image width in pixels (physical, not logical pts)
        height: u32,      // native image height
        scale_mode: ScaleMode,
    },

    /// PNG image (decoded on load via lodepng; result stored as BGRA in WallpaperCache).
    ImagePng {
        path: String,
        scale_mode: ScaleMode,
    },
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMode {
    #[default]
    Fill,     // scale to fill screen (crop if needed)
    Fit,      // scale to fit, letterbox with solid black
    Center,   // no scale; centered; background color fills margins
    Stretch,  // non-uniform scale to exact screen size
    Tile,     // repeat the image to fill the screen
}
```

### 3.2 `WallpaperConfig` Per Space

```rust
// supervisor/src/wallpaper/config.rs

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct WallpaperConfig {
    pub kind: WallpaperKind,

    /// Dynamic: time-of-day variant table. v1 may be empty.
    /// Each entry is an (hour_utc: u8, WallpaperKind) pair.
    #[serde(default)]
    pub time_variants: Vec<(u8, WallpaperKind)>,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        WallpaperConfig {
            kind: WallpaperKind::SolidColor { rgba: 0x1E1E2EFF },
            time_variants: vec![],
        }
    }
}
```

### 3.3 `WallpaperCache` — Decoded Images

```rust
// supervisor/src/wallpaper/cache.rs

/// Supervisor-side cache of decoded wallpaper images.
/// key = space number (1-9); value = decoded BGRA pixels at physical resolution.
pub struct WallpaperCache {
    /// key = absolute path string, value = (width_px, height_px, bgra_pixels)
    images: HashMap<String, (u32, u32, Vec<u8>)>,
}

impl WallpaperCache {
    pub fn get_or_load(&mut self, path: &str, is_png: bool) -> Option<&(u32, u32, Vec<u8>)> {
        if !self.images.contains_key(path) {
            let pixels = if is_png {
                Self::load_png(path)?
            } else {
                Self::load_bgra_raw(path)?
            };
            self.images.insert(path.to_string(), pixels);
        }
        self.images.get(path)
    }

    fn load_png(path: &str) -> Option<(u32, u32, Vec<u8>)> {
        // Use lodepng::decode32_file (RGBA output); convert to BGRA
        match lodepng::decode32_file(path) {
            Ok(image) => {
                let bgra: Vec<u8> = image.buffer.iter().flat_map(|px| {
                    [px.b, px.g, px.r, px.a]
                }).collect();
                Some((image.width as u32, image.height as u32, bgra))
            }
            Err(e) => {
                log::warn!("[wallpaper] PNG load failed for {path}: {e}");
                None
            }
        }
    }

    fn load_bgra_raw(path: &str) -> Option<(u32, u32, Vec<u8>)> {
        // Raw BGRA: file must start with 8-byte header: width_px(u32 LE) + height_px(u32 LE)
        let data = std::fs::read(path).ok()?;
        if data.len() < 8 { return None; }
        let w = u32::from_le_bytes(data[0..4].try_into().ok()?);
        let h = u32::from_le_bytes(data[4..8].try_into().ok()?);
        let expected = 8 + (w * h * 4) as usize;
        if data.len() != expected { return None; }
        Some((w, h, data[8..].to_vec()))
    }

    /// Evict all cached images (call on wallpaper_set to avoid stale pixels).
    pub fn evict(&mut self, path: &str) {
        self.images.remove(path);
    }

    pub fn clear(&mut self) {
        self.images.clear();
    }
}
```

### 3.4 `SupervisorState` Extensions

```rust
// supervisor/src/supervisor_state.rs (or wherever SupervisorState lives)

pub struct SupervisorState {
    // existing fields ...

    /// Per-space wallpaper configuration. Key = space number (1-9).
    /// Space 0 is not a user space; no wallpaper entry for space 0.
    pub wallpapers: HashMap<u8, WallpaperConfig>,

    /// Decoded image cache (shared across spaces).
    pub wallpaper_cache: WallpaperCache,

    /// Screen-saver: ticks since last input event (60Hz ticks).
    pub idle_ticks: u64,

    /// Screen-saver timeout in seconds. 0 = disabled. Default: 0 (v1).
    pub screensaver_timeout_secs: u64,

    /// true if screensaver (blank) is active.
    pub screensaver_active: bool,
}
```

---

## 4. Per-Space Wallpaper Rendering

### 4.1 Compositor Integration — `fill_wallpaper`

The wallpaper is rendered in the compositor tick **inside** the `vsync_lock.write()` block
that already calls `fb.fill(BACKGROUND_COLOR)` during space switches, and also **at the
start of every normal compositor pass** before any surface blit.

```rust
// supervisor/src/wallpaper/render.rs

/// Render the wallpaper for the given space onto the framebuffer.
/// Called under vsync_lock.read() in the compositor blit pass (before any app blit).
/// On space switch it is called under vsync_lock.write() as a replacement for
/// fb.fill(BACKGROUND_COLOR).
pub fn fill_wallpaper(
    fb: &mut FrameBuffer,
    config: &WallpaperConfig,
    display: &DisplayConfig,
    cache: &WallpaperCache,
) {
    match &config.kind {
        WallpaperKind::SolidColor { rgba } => {
            fb.fill(*rgba);
        }
        WallpaperKind::LinearGradient { from_rgba, to_rgba, angle_deg } => {
            fill_gradient(fb, *from_rgba, *to_rgba, *angle_deg, display);
        }
        WallpaperKind::ImageBgra { path, width, height, scale_mode } => {
            if let Some((w, h, pixels)) = cache.images.get(path) {
                blit_wallpaper_image(fb, pixels, *w, *h, *scale_mode, display);
            } else {
                // Image not yet loaded (or failed to load) — fall back to dark bg
                fb.fill(0x1E1E2EFF);
            }
        }
        WallpaperKind::ImagePng { path, scale_mode } => {
            if let Some((w, h, pixels)) = cache.images.get(path) {
                blit_wallpaper_image(fb, pixels, *w, *h, *scale_mode, display);
            } else {
                fb.fill(0x1E1E2EFF);
            }
        }
    }
}

fn fill_gradient(
    fb: &mut FrameBuffer,
    from: u32, to: u32,
    angle_deg: u16,
    display: &DisplayConfig,
) {
    let pw = display.physical_width;
    let ph = display.physical_height;
    let (fr, fg, fb_c, fa) = unpack_rgba(from);
    let (tr, tg, tb, ta) = unpack_rgba(to);

    for row in 0..ph {
        for col in 0..pw {
            let t: f32 = if angle_deg == 0 {
                col as f32 / pw.max(1) as f32
            } else {
                row as f32 / ph.max(1) as f32
            };
            let r = lerp_u8(fr, tr, t);
            let g = lerp_u8(fg, tg, t);
            let b = lerp_u8(fb_c, tb, t);
            let a = lerp_u8(fa, ta, t);
            fb.set_pixel(col, row, pack_bgra(b, g, r, a));
        }
    }
}

#[inline(always)]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

fn unpack_rgba(v: u32) -> (u8, u8, u8, u8) {
    ((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn pack_bgra(b: u8, g: u8, r: u8, a: u8) -> u32 {
    ((b as u32) << 24) | ((g as u32) << 16) | ((r as u32) << 8) | (a as u32)
}
```

### 4.2 Updated `vsync_tick` — Wallpaper Position

```rust
// supervisor/src/compositor.rs — updated vsync_tick()

fn vsync_tick(state: &mut SupervisorState, /* ... */) {
    // Step 1: Check pending space switch (R21 §5.3)
    let pending = PENDING_SPACE_SWITCH.swap(0, Ordering::AcquireRelease);
    if pending != 0 {
        let _write = vsync_lock.write();
        spaces.active = pending;
        // Replace fb.fill(BACKGROUND_COLOR) with wallpaper for new space
        let wc = state.wallpapers.get(&pending)
            .cloned()
            .unwrap_or_default();
        fill_wallpaper(&mut fb, &wc, &display_config, &state.wallpaper_cache);
        mark_new_space_dirty(&mut apps, pending);
        compact_z_order(&mut apps, pending);
        // Reload time-of-day variant if applicable
        maybe_apply_time_variant(&mut state.wallpapers, pending, current_hour_utc());
    }

    // Step 2: Drain pending resizes (R21 §6.3)
    {
        let _write = vsync_lock.write();
        drain_pending_resizes(&mut apps, &display_config);
    }

    // Step 3: Compositor blit pass (read-lock)
    {
        let _read = vsync_lock.read();

        // 3a: Paint wallpaper for active space FIRST (below all app surfaces)
        let active = spaces.active;
        let wc = state.wallpapers.get(&active).cloned().unwrap_or_default();
        fill_wallpaper(&mut fb, &wc, &display_config, &state.wallpaper_cache);

        // 3b: Space-N apps (ascending z)
        let mut space_n: Vec<_> = apps.values()
            .filter(|a| a.win_space == active && a.win_visible && !a.win_minimized)
            .sorted_by_key(|a| a.win_z.get(active))
            .collect();
        for app in &space_n {
            blit_clipped(&mut fb, app, &display_config, chrome_h_px, dock_clip_y_px, strip_clip_x_px);
        }

        // 3c: Space-0 apps (ascending z; always on top — R21 Model A)
        let mut space_0: Vec<_> = apps.values()
            .filter(|a| a.win_space == 0 && a.win_visible && !a.win_minimized)
            .sorted_by_key(|a| a.win_z.get(0))
            .collect();
        for app in &space_0 {
            blit_clipped(&mut fb, app, &display_config, 0, display_config.physical_height, 0);
        }
    }

    // Step 4: DRM blit (write-lock)
    {
        let _write = vsync_lock.write();
        fb.blit_to_device();
    }

    // Step 5: Screen-saver tick
    tick_screensaver(state);
}
```

**Critical note**: `fill_wallpaper` is called in Step 3a under `vsync_lock.read()`. This
requires that `fill_wallpaper` does NOT modify `WallpaperCache` (no new loads during the
read-locked path). Image loading happens in `wallpaper_set` (IPC handler) and at boot,
always outside the vsync read-lock. If an image is not yet in the cache during a read-lock
blit, the fallback solid color is used until the image is loaded.

---

## 5. Per-Space Wallpapers — Space Switch Behavior

### 5.1 Space Association

```rust
// supervisor/src/wallpaper/config.rs

impl SupervisorState {
    /// Returns the effective WallpaperConfig for the given space.
    /// Falls back to default (solid dark) if space has no config.
    pub fn wallpaper_for_space(&self, space: u8) -> WallpaperConfig {
        self.wallpapers.get(&space).cloned().unwrap_or_default()
    }
}
```

When `PENDING_SPACE_SWITCH` fires, the compositor applies the new space's wallpaper inside
the `vsync_lock.write()` block — the same block that previously called `fb.fill(BG)`.
The transition is instantaneous (no cross-fade in v1; deferred to R30 Transitions).

### 5.2 New Space Initialization

When `space_add` creates a new space (R21 §3.1), the wallpaper for the new space is
initialized to the system default:

```rust
// supervisor/src/wm/commands.rs — handle_space_add()

pub fn handle_space_add(state: &mut SupervisorState) {
    let new_space = /* allocate next space id */;
    // ... existing space registry logic ...
    // Initialize wallpaper for new space to system default
    state.wallpapers.entry(new_space).or_insert_with(WallpaperConfig::default);
}
```

### 5.3 Space Deletion

When `space_del <N>` runs, the wallpaper config for that space is removed:

```rust
// supervisor/src/wm/commands.rs — handle_space_del()

state.wallpapers.remove(&space_n);
// Evict any image cached exclusively for that space's path
// (cache eviction is path-based; if path is shared by another space, it stays)
```

---

## 6. Wallpaper Formats — v1 Scope

| Format | Support | Notes |
|--------|---------|-------|
| Solid color (RGBA u32) | v1 | Trivial; matches existing VYOMA_DRAW color encoding |
| Linear gradient (2-stop, 0° or 90°) | v1 | Rendered inline by supervisor; no external dep |
| Raw BGRA image | v1 | File starts with 8-byte header: width_u32_le + height_u32_le, then raw BGRA |
| PNG image | v1 | Decoded via `lodepng` (already pulled in by R46/US2); RGBA→BGRA conversion |
| JPEG | deferred | R35 (image codecs); not in v1 |
| Animated GIF / APNG / AVIF | deferred | R36 (video) |
| macOS `.heic` dynamic wallpaper | deferred | R37 |

### 6.1 Unsupported Format Fallback

If an image path fails to load (file not found, corrupt PNG, wrong BGRA header), supervisor
logs a structured warning and falls back to the default solid color:

```rust
log::warn!(
    event = "wallpaper_load_failed",
    space = space,
    path = path,
    reason = %e,
    "wallpaper image unavailable; using solid color fallback"
);
// WallpaperConfig.kind is NOT changed in state — the configured path is preserved
// so that if the file appears later (e.g. copied via IPC), it will load correctly.
```

The configured `kind` field remains unchanged. `WallpaperCache` will retry on the next
`wallpaper_set` or `wallpaper_reload` IPC command. Failed loads are not retried
automatically on every compositor tick (that would cause per-frame filesystem calls).

---

## 7. Desktop Icons — Boundary Decision

**Desktop icons are explicitly out of scope for R29.** The boundary is:

- R29 (this spec): Supervisor paints the wallpaper layer. No interactive content on the
  desktop. No file icons, no drop targets.
- R41 (File Manager): A thin `desktop.wasm` app will live in each space at z=1. It
  renders file icons fetched from the VyomaOS virtual filesystem. It handles click events
  (R32 mouse) for opening files. Its surface covers the full content area minus chrome,
  dock, and stage strip. The wallpaper supervisor module continues to paint the sub-layer;
  the `desktop.wasm` surface has a transparent background and is blitted over the
  wallpaper.

This two-layer model (wallpaper in supervisor + desktop icons in WASM app) mirrors macOS
where Finder's desktop window sits above the wallpaper layer with a transparent background.

---

## 8. Wallpaper Persistence — `/data/wallpaper.toml`

### 8.1 File Location and Format

```
/data/wallpaper.toml
```

The file lives on the persistent `/data` 9P mount. It survives VM reboots. Supervisor
reads it at boot during the manifest loading phase (before any app is spawned).

```toml
# /data/wallpaper.toml — per-space wallpaper configuration
# Spaces not listed here use the system default (solid #1E1E2E).

[spaces.1]
kind = "solid_color"
rgba = 3905462271         # 0xE8D5BFFF — warm sand

[spaces.2]
kind = "linear_gradient"
from_rgba = 2566914303    # 0x990000FF — deep red
to_rgba = 1717986815      # 0x66338FFF — purple
angle_deg = 90

[spaces.3]
kind = "image_png"
path = "/data/wallpapers/forest.png"
scale_mode = "fill"

[spaces.4]
kind = "image_bgra"
path = "/data/wallpapers/space4.bgra"
width = 1920
height = 1080
scale_mode = "fit"

# Dynamic wallpaper: space 5 changes at dawn/dusk
[spaces.5]
kind = "solid_color"
rgba = 520093695           # 0x1F0A3FFF — deep indigo (night)

[[spaces.5.time_variants]]
hour_utc = 6              # 06:00 UTC — switch to morning color
kind = "solid_color"
rgba = 4290822143         # 0xFFAA77FF — dawn orange

[[spaces.5.time_variants]]
hour_utc = 10             # 10:00 UTC — switch to day color
kind = "solid_color"
rgba = 1191182335         # 0x470041FF — teal

[[spaces.5.time_variants]]
hour_utc = 20             # 20:00 UTC — switch to evening color
kind = "solid_color"
rgba = 520093695           # 0x1F0A3FFF — night
```

### 8.2 Boot Loading

```rust
// supervisor/src/wallpaper/persistence.rs

pub fn load_wallpaper_config(state: &mut SupervisorState) {
    let path = "/data/wallpaper.toml";
    match std::fs::read_to_string(path) {
        Ok(content) => match toml::from_str::<WallpaperFile>(&content) {
            Ok(wf) => {
                state.wallpapers = wf.spaces;
                // Pre-load all image paths into cache
                for (space, wc) in &state.wallpapers.clone() {
                    preload_wallpaper_image(wc, &mut state.wallpaper_cache, *space);
                }
                log::info!(event = "wallpaper_config_loaded", path = path);
            }
            Err(e) => {
                log::warn!(event = "wallpaper_config_parse_error", path = path, reason = %e);
                // Use defaults for all spaces
            }
        },
        Err(_) => {
            // File absent — use defaults; will be written on first wallpaper_set command
            log::info!(event = "wallpaper_config_absent", path = path, "using defaults");
        }
    }
}

fn preload_wallpaper_image(wc: &WallpaperConfig, cache: &mut WallpaperCache, space: u8) {
    match &wc.kind {
        WallpaperKind::ImagePng { path, .. } => {
            cache.get_or_load(path, true);
        }
        WallpaperKind::ImageBgra { path, .. } => {
            cache.get_or_load(path, false);
        }
        _ => {}
    }
    // Also preload time variants
    for (_, variant) in &wc.time_variants {
        match variant {
            WallpaperKind::ImagePng { path, .. } => { cache.get_or_load(path, true); }
            WallpaperKind::ImageBgra { path, .. } => { cache.get_or_load(path, false); }
            _ => {}
        }
    }
}
```

### 8.3 Write-Through Persistence

Every `wallpaper_set` IPC command updates `state.wallpapers` and immediately writes
`/data/wallpaper.toml` via `persist_wallpaper_config`. The write is synchronous (small
file, <4 KB typical). No background thread for v1.

```rust
pub fn persist_wallpaper_config(state: &SupervisorState) -> std::io::Result<()> {
    let wf = WallpaperFile { spaces: state.wallpapers.clone() };
    let content = toml::to_string_pretty(&wf)?;
    std::fs::write("/data/wallpaper.toml", content)
}
```

---

## 9. Wallpaper Change API

### 9.1 IPC Commands (shell → supervisor)

Users and scripts change wallpaper via the supervisor IPC command interface:

```
wallpaper_set <space> solid <rgba_u32>
wallpaper_set <space> gradient <from_rgba> <to_rgba> <angle_deg>
wallpaper_set <space> image <path> <scale_mode>
wallpaper_reload <space>          -- reload image from disk (evicts cache entry)
wallpaper_get <space>             -- print current config as TOML
wallpaper_reset <space>           -- reset to system default
```

**Handler**:

```rust
// supervisor/src/wallpaper/commands.rs

pub fn handle_wallpaper_set(
    args: &[&str],
    state: &mut SupervisorState,
) {
    let space: u8 = args[0].parse().unwrap_or(1);
    let kind = match args[1] {
        "solid" => {
            let rgba: u32 = args[2].parse().unwrap_or(0x1E1E2EFF);
            WallpaperKind::SolidColor { rgba }
        }
        "gradient" => {
            let from = args[2].parse().unwrap_or(0x1E1E2EFF);
            let to   = args[3].parse().unwrap_or(0x4A4A8AFF);
            let ang: u16 = args[4].parse().unwrap_or(90);
            WallpaperKind::LinearGradient { from_rgba: from, to_rgba: to, angle_deg: ang }
        }
        "image" => {
            let path = args[2].to_string();
            let scale_mode = args.get(3)
                .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
                .unwrap_or_default();
            // Evict stale cache entry; force re-load next frame
            state.wallpaper_cache.evict(&path);
            // Pre-load eagerly in the IPC thread (not under vsync lock)
            state.wallpaper_cache.get_or_load(&path, path.ends_with(".png"));
            if path.ends_with(".png") {
                WallpaperKind::ImagePng { path, scale_mode }
            } else {
                // Assume raw BGRA; width/height read from file header by WallpaperCache
                let (w, h) = state.wallpaper_cache.images.get(&path)
                    .map(|(w, h, _)| (*w, *h))
                    .unwrap_or((0, 0));
                WallpaperKind::ImageBgra { path, width: w, height: h, scale_mode }
            }
        }
        _ => {
            log::warn!(event = "wallpaper_set_unknown_kind", kind = args[1]);
            return;
        }
    };

    let wc = state.wallpapers.entry(space).or_insert_with(WallpaperConfig::default);
    wc.kind = kind;
    persist_wallpaper_config(state).ok();
    log::info!(event = "wallpaper_set", space = space);
}
```

### 9.2 `VYOMA_DESKTOP:` Stdout Protocol

WASM apps with `display = true` may change the wallpaper via the `VYOMA_DESKTOP:` stdout
protocol. This is the standard mechanism for apps (e.g. a future settings app) to change
the wallpaper without shell access.

```
VYOMA_DESKTOP:wallpaper_set:<space>,solid,<rgba>
VYOMA_DESKTOP:wallpaper_set:<space>,gradient,<from_rgba>,<to_rgba>,<angle_deg>
VYOMA_DESKTOP:wallpaper_set:<space>,image,<path>,<scale_mode>
VYOMA_DESKTOP:wallpaper_reload:<space>
VYOMA_DESKTOP:wallpaper_get:<space>
```

**Supervisor response** (via app stdin):

```
VYOMA_DESKTOP_WALLPAPER:<space>,<kind_tag>,<serialized_fields>
```

Only apps with `display = true` in their manifest may issue `VYOMA_DESKTOP:` commands. The
capability check mirrors the existing `VYOMA_DRAW:` and `VYOMA_WM:` gating:

```rust
// supervisor/src/draw_cmd.rs (or a new desktop_cmd.rs)

if line.starts_with("VYOMA_DESKTOP:") {
    if !app.capabilities.display {
        log::warn!(event = "desktop_cmd_denied", app = &app.name, "app lacks display capability");
        continue;
    }
    handle_desktop_cmd(&line["VYOMA_DESKTOP:".len()..], app_name, state);
}
```

### 9.3 WIT Interface `vyoma:desktop@1.0.0`

```wit
package vyoma:desktop@1.0.0;

record wallpaper-solid {
    rgba: u32,
}

record wallpaper-gradient {
    from-rgba: u32,
    to-rgba: u32,
    angle-deg: u16,
}

record wallpaper-image {
    path: string,
    scale-mode: string,
}

variant wallpaper-kind {
    solid(wallpaper-solid),
    gradient(wallpaper-gradient),
    image(wallpaper-image),
}

interface desktop {
    set-wallpaper: func(space: u8, kind: wallpaper-kind) -> result<_, string>;
    get-wallpaper: func(space: u8) -> result<wallpaper-kind, string>;
    reload-wallpaper: func(space: u8) -> result<_, string>;
    reset-wallpaper: func(space: u8) -> result<_, string>;
}

world desktop-world {
    import desktop;
}
```

`init_linker_for_app` only wires the `vyoma:desktop` WIT interface for apps that declare
`display = true` (consistent with R18 conditional WIT gating pattern).

---

## 10. Dynamic Wallpapers — Time-of-Day (v1 Scope)

### 10.1 Design

Time-of-day wallpaper switching mirrors macOS Sonoma's dynamic wallpapers. In v1:
- Each `WallpaperConfig` carries an optional `time_variants: Vec<(u8, WallpaperKind)>`
  sorted by `hour_utc`.
- The compositor tick calls `maybe_apply_time_variant` once per minute (every 3600 ticks
  at 60Hz).

### 10.2 Timer Integration

```rust
// supervisor/src/wallpaper/dynamic.rs

/// Called every compositor tick. Only does real work once per minute.
pub fn maybe_apply_time_variant(
    wallpapers: &mut HashMap<u8, WallpaperConfig>,
    active_space: u8,
    tick: u64,
    cache: &mut WallpaperCache,
) {
    // Check at most once per minute (3600 ticks at 60 Hz)
    if tick % 3600 != 0 { return; }
    let hour = current_hour_utc();
    for (space, wc) in wallpapers.iter_mut() {
        if wc.time_variants.is_empty() { continue; }
        // Find the latest variant whose hour_utc <= current hour
        let best = wc.time_variants.iter()
            .filter(|(h, _)| *h <= hour)
            .max_by_key(|(h, _)| *h);
        if let Some((_, new_kind)) = best {
            if std::mem::discriminant(new_kind) != std::mem::discriminant(&wc.kind) {
                wc.kind = new_kind.clone();
                // Preload image if needed
                preload_wallpaper_image(wc, cache, *space);
            }
        }
    }
}

fn current_hour_utc() -> u8 {
    // Read from supervisor's wall clock (via std::time::SystemTime)
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ((secs / 3600) % 24) as u8
}
```

### 10.3 v1 Limitations

- Only `hour_utc` granularity (no minute precision).
- `time_variants` is a linear scan; max 24 entries makes this negligible cost.
- No animation between time variants (instantaneous switch). Cross-fade deferred to R30.
- Variants are checked on the compositor thread — they must not block (image pre-loading
  is done eagerly at load time, not during variant apply).

---

## 11. Stage Manager Interaction

### 11.1 Wallpaper Visibility Behind Stage Strip

The wallpaper fills the entire framebuffer (all physical pixels) before any surface blit.
The stage-strip WASM app (space=0, z=65532) renders over the wallpaper in its 140pt left
region. The wallpaper is therefore **visible behind transparent regions of the stage-strip
surface**.

The `stage-strip` app uses a translucent panel background (glassmorphism — blurred
wallpaper). In v1 the wallpaper is not blurred; the strip renders a semi-transparent panel
with `rgba = 0x1E1E2E99` (dark fill, alpha=0.6) over the wallpaper pixels. True Gaussian
blur of the underlying wallpaper pixels is deferred to R30 (Effects & Blur).

### 11.2 `strip_clip_x_px` Does Not Apply to Wallpaper

`fill_wallpaper` writes to the full framebuffer. The `blit_clipped` 7-step formula (R26
§13) with `strip_clip_x_px` applies only to per-app surface blits (space-N apps). The
wallpaper underlies everything including the strip area.

### 11.3 Main Area vs. Strip Area Wallpaper Distinction

In v1, the same wallpaper is visible both behind the stage strip and behind the main
content area. Stage strip app renders its own translucent overlay. No distinction required
at the wallpaper layer.

Future (R36+): blur radius could differ between strip region and main area, rendered as two
separate `fill_wallpaper` calls with different parameters. Deferred.

---

## 12. Screen Saver — Minimal Blank-Screen Fallback

### 12.1 v1 Scope

v1 ships a minimal blank-screen screen saver only:
- After `screensaver_timeout_secs` of no keyboard or mouse input, the compositor replaces
  `fill_wallpaper` with `fb.fill(0x00000000)` (full black).
- Any keyboard press or mouse event clears `screensaver_active` and restores wallpaper on
  the next tick.
- Default timeout = 0 (disabled). User sets via IPC `screensaver_timeout <secs>`.

True screen savers (animated, GPU-rendered) are deferred to R38.

### 12.2 Idle Tracking

```rust
// supervisor/src/wallpaper/screensaver.rs

pub fn tick_screensaver(state: &mut SupervisorState) {
    if state.screensaver_timeout_secs == 0 { return; }
    state.idle_ticks += 1;
    let timeout_ticks = state.screensaver_timeout_secs * 60; // 60 Hz ticks
    if state.idle_ticks >= timeout_ticks && !state.screensaver_active {
        state.screensaver_active = true;
        log::info!(event = "screensaver_activated");
    }
}

pub fn reset_idle_on_input(state: &mut SupervisorState) {
    state.idle_ticks = 0;
    if state.screensaver_active {
        state.screensaver_active = false;
        log::info!(event = "screensaver_dismissed");
    }
}
```

`reset_idle_on_input` is called from the keyboard input router (TTY input thread) and the
mouse input handler (R32) on every input event.

### 12.3 Compositor Integration for Screen Saver

```rust
// In vsync_tick, Step 3a:

if state.screensaver_active {
    fb.fill(0x00000000); // blank screen
    // Skip all app blits — nothing rendered over blank
    return; // early exit after DRM blit
} else {
    fill_wallpaper(&mut fb, &wc, &display_config, &state.wallpaper_cache);
}
```

**Lock note**: the blank path still acquires and releases `vsync_lock` normally.

---

## 13. `VYOMA_DESKTOP:` Protocol — Full Specification

### 13.1 App → Supervisor Commands (stdout)

```
VYOMA_DESKTOP:wallpaper_set:<space>,solid,<rgba_u32>
VYOMA_DESKTOP:wallpaper_set:<space>,gradient,<from_rgba>,<to_rgba>,<angle_deg>
VYOMA_DESKTOP:wallpaper_set:<space>,image,<path>,<scale_mode>
VYOMA_DESKTOP:wallpaper_reload:<space>
VYOMA_DESKTOP:wallpaper_get:<space>
VYOMA_DESKTOP:wallpaper_reset:<space>
VYOMA_DESKTOP:screensaver_set:<timeout_secs>
VYOMA_DESKTOP:screensaver_dismiss
```

### 13.2 Supervisor → App Responses (stdin push)

```
VYOMA_DESKTOP_WALLPAPER:<space>,solid,<rgba>
VYOMA_DESKTOP_WALLPAPER:<space>,gradient,<from_rgba>,<to_rgba>,<angle_deg>
VYOMA_DESKTOP_WALLPAPER:<space>,image,<path>,<scale_mode>
VYOMA_DESKTOP_WALLPAPER:<space>,error,<reason>
VYOMA_DESKTOP_SCREENSAVER:<timeout_secs>
```

Only the requesting app receives the response (not broadcast).

### 13.3 Capability Gate

`VYOMA_DESKTOP:` commands require `display = true` in the app's manifest. No separate
`desktop` capability. This is consistent with all other `VYOMA_*:` protocol gates.

---

## 14. Platform Matrix

| Feature | desktop-full | mobile | server-headless | iot-edge / robotics / mcu |
|---------|-------------|--------|----------------|--------------------------|
| Per-space wallpaper | yes (1-9 spaces) | yes (1 space, lock screen only) | no | no |
| Solid color | yes | yes | no | no |
| Linear gradient | yes | yes | no | no |
| Raw BGRA image | yes | yes | no | no |
| PNG image | yes | yes | no | no |
| Dynamic (time-of-day) | yes | no (v1) | no | no |
| Screen saver | yes (blank) | no | no | no |
| `VYOMA_DESKTOP:` protocol | yes | yes | no | no |
| WIT `vyoma:desktop@1.0.0` | yes | yes | no | no |
| `/data/wallpaper.toml` | yes | yes | no | no |

**Mobile note**: Mobile has one space. The wallpaper is painted for that space. The lock
screen wallpaper is a separate concept handled by R34 (Lock Screen). For v1 mobile, the
standard wallpaper fills the content area; the lock screen uses the same config.

**server-headless**: No display — wallpaper module is compiled in but `fill_wallpaper` is
a no-op when `display_config.physical_width == 0`.

**Platform guard in compositor**:

```rust
if display_config.physical_width == 0 || display_config.physical_height == 0 {
    return; // headless: skip all rendering including wallpaper
}
```

---

## 15. `blit_wallpaper_image` — Scale Modes

```rust
// supervisor/src/wallpaper/render.rs

pub fn blit_wallpaper_image(
    fb: &mut FrameBuffer,
    src: &[u8],        // BGRA pixels, row-major
    src_w: u32,
    src_h: u32,
    scale_mode: ScaleMode,
    display: &DisplayConfig,
) {
    let dst_w = display.physical_width;
    let dst_h = display.physical_height;

    match scale_mode {
        ScaleMode::Fill => {
            // Scale uniformly to cover dst; crop center
            let scale = (dst_w as f32 / src_w as f32).max(dst_h as f32 / src_h as f32);
            let scaled_w = (src_w as f32 * scale) as u32;
            let scaled_h = (src_h as f32 * scale) as u32;
            let off_x = (scaled_w.saturating_sub(dst_w)) / 2;
            let off_y = (scaled_h.saturating_sub(dst_h)) / 2;
            blit_scaled(fb, src, src_w, src_h, scaled_w, scaled_h, off_x, off_y, dst_w, dst_h);
        }
        ScaleMode::Fit => {
            // Scale uniformly to fit inside dst; letterbox
            fb.fill(0x000000FF); // black letterbox
            let scale = (dst_w as f32 / src_w as f32).min(dst_h as f32 / src_h as f32);
            let scaled_w = (src_w as f32 * scale) as u32;
            let scaled_h = (src_h as f32 * scale) as u32;
            let dest_x = (dst_w.saturating_sub(scaled_w)) / 2;
            let dest_y = (dst_h.saturating_sub(scaled_h)) / 2;
            blit_scaled_at(fb, src, src_w, src_h, scaled_w, scaled_h, dest_x, dest_y);
        }
        ScaleMode::Center => {
            fb.fill(0x000000FF); // background for margins
            let dest_x = (dst_w.saturating_sub(src_w)) / 2;
            let dest_y = (dst_h.saturating_sub(src_h)) / 2;
            blit_direct(fb, src, src_w, src_h, dest_x, dest_y, dst_w, dst_h);
        }
        ScaleMode::Stretch => {
            blit_scaled(fb, src, src_w, src_h, dst_w, dst_h, 0, 0, dst_w, dst_h);
        }
        ScaleMode::Tile => {
            blit_tiled(fb, src, src_w, src_h, dst_w, dst_h);
        }
    }
}
```

Scaling uses nearest-neighbor sampling in v1 (bilinear deferred to R30 for quality wallpaper rendering):

```rust
fn blit_scaled(fb: &mut FrameBuffer, src: &[u8], src_w: u32, src_h: u32,
               scaled_w: u32, scaled_h: u32, off_x: u32, off_y: u32,
               dst_w: u32, dst_h: u32)
{
    for dst_y in 0..dst_h {
        let src_y = ((dst_y + off_y) as f32 * src_h as f32 / scaled_h as f32) as u32;
        let src_y = src_y.min(src_h - 1);
        for dst_x in 0..dst_w {
            let src_x = ((dst_x + off_x) as f32 * src_w as f32 / scaled_w as f32) as u32;
            let src_x = src_x.min(src_w - 1);
            let src_idx = ((src_y * src_w + src_x) * 4) as usize;
            if src_idx + 3 < src.len() {
                let px = u32::from_le_bytes([src[src_idx], src[src_idx+1], src[src_idx+2], src[src_idx+3]]);
                fb.set_pixel(dst_x, dst_y, px);
            }
        }
    }
}
```

---

## 16. File Layout

```
supervisor/src/wallpaper/
├── mod.rs          (WallpaperKind, WallpaperConfig, WallpaperFile — pub re-exports)
├── config.rs       (serde types: WallpaperKind, WallpaperConfig, ScaleMode, WallpaperFile)
├── cache.rs        (WallpaperCache: get_or_load, load_png, load_bgra_raw, evict, clear)
├── render.rs       (fill_wallpaper, fill_gradient, blit_wallpaper_image, scale mode impls)
├── commands.rs     (handle_wallpaper_set, handle_wallpaper_reload, handle_wallpaper_get)
├── persistence.rs  (load_wallpaper_config, persist_wallpaper_config, preload_wallpaper_image)
├── dynamic.rs      (maybe_apply_time_variant, current_hour_utc)
└── screensaver.rs  (tick_screensaver, reset_idle_on_input)

supervisor/src/
├── compositor.rs   (updated vsync_tick: fill_wallpaper in Step 3a, screensaver gate)
├── supervisor_state.rs (wallpapers: HashMap<u8, WallpaperConfig>, wallpaper_cache,
│                        idle_ticks, screensaver_timeout_secs, screensaver_active)
└── desktop_cmd.rs  (VYOMA_DESKTOP: protocol parser; routes to wallpaper/commands.rs)

supervisor/tests/
├── wallpaper_render.rs   (fill_gradient, blit_wallpaper_image for each scale mode)
├── wallpaper_persistence.rs (load/persist roundtrip, missing file default)
├── wallpaper_dynamic.rs  (time variant selection, minute boundary)
└── screensaver.rs        (idle tick counting, activation, dismiss)

base/wallpapers/
└── default.png           (1920×1080 VyomaOS default wallpaper, shipped in initramfs)
```

---

## 17. Supervisor State Extensions Summary

```rust
// Added to SupervisorState:

pub wallpapers: HashMap<u8, WallpaperConfig>,          // per-space wallpaper config
pub wallpaper_cache: WallpaperCache,                    // decoded image cache
pub idle_ticks: u64,                                    // ticks since last input
pub screensaver_timeout_secs: u64,                      // 0 = disabled
pub screensaver_active: bool,                           // blanking active

// Added to AppState: (none — wallpaper is supervisor-owned, not per-app)
```

---

## 18. Blocking Issue Resolution — Pre-empting Known Risks

The following risks are identified and resolved in this design:

| Risk | Resolution |
|------|-----------|
| `fill_wallpaper` under `vsync_lock.read()` attempts image load (filesystem I/O blocks) | Image loading always happens in IPC handler or boot (outside vsync lock). Under read-lock, only already-loaded pixels in `WallpaperCache` are accessed. If not loaded, solid fallback used. |
| Space switch `fb.fill()` replaced by `fill_wallpaper()` — wallpaper not yet loaded | Same fallback: `fill_wallpaper` uses solid default if image not in cache. Boot pre-loads all configured images. |
| Dynamic variant check on every compositor tick (3600x/min overhead) | `if tick % 3600 != 0 { return; }` — 1-instruction modulo, negligible. |
| PNG decode adds lodepng dependency | Already pulled in by R46 US2 (image support). No new dependency. |
| Screen saver blank fires during active use (false positive) | `reset_idle_on_input` called from TTY input router and mouse handler on every event. Any activity resets `idle_ticks`. |
| wallpaper.toml write on every `wallpaper_set` call (I/O on IPC thread) | Synchronous write is < 4 KB, ~100 µs. Acceptable for a config-change event. Not on the hot path. |

---

## 19. Open Questions (Deferred)

1. **Blur behind stage strip (glassmorphism)**: Sampling the already-rendered wallpaper
   pixels and applying Gaussian blur for the stage-strip background. Deferred to R30
   (Effects & Blur).
2. **Cross-fade on space switch**: Fade between two wallpapers during space transition.
   Deferred to R30.
3. **Bilinear scaling for wallpaper images**: Replace nearest-neighbor in `blit_scaled`
   with bilinear sampling. Deferred to R30.
4. **JPEG support**: Requires a JPEG decoder (mozjpeg or libjpeg-turbo — both have C deps;
   or a pure-Rust option). Deferred to R35.
5. **Lock screen wallpaper (mobile)**: Mobile lock screen is R34. At that point
   `WallpaperConfig` gains a `lock_screen` variant.
6. **Dynamic wallpaper per-minute precision**: `current_hour_utc()` gives hour granularity.
   Fine-grained scheduling deferred to R38 (Screen Saver & Idle).
7. **Desktop icons (file icons on wallpaper layer)**: Explicitly deferred to R41 (File
   Manager). A `desktop.wasm` app will run as space-N z=1 with a transparent background
   surface.
