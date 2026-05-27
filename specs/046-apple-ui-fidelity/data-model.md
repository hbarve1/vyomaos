# Data Model: Apple-Platform UI Fidelity

## Entities

### `FontCache`
Loaded once at supervisor startup; shared across all apps.

| Field | Type | Notes |
|-------|------|-------|
| `ui_font` | `fontdue::Font` | Inter Regular |
| `ui_font_bold` | `fontdue::Font` | Inter Bold |
| `mono_font` | `fontdue::Font` | IBM Plex Mono Regular |
| `glyph_cache` | `HashMap<(char, u32), GlyphBitmap>` | keyed by (char, pt_size×10) |

**Validation**: All four fonts must load at startup or supervisor aborts with a structured error log.

### `GlyphBitmap`
Cached rasterized glyph.

| Field | Type | Notes |
|-------|------|-------|
| `coverage` | `Vec<u8>` | Alpha mask, width×height bytes |
| `width` | `u32` | Bitmap pixel width |
| `height` | `u32` | Bitmap pixel height |
| `advance_x` | `f32` | Horizontal advance in pixels |
| `bearing_y` | `i32` | Vertical offset from baseline |

### `ImageCache`
In-memory decoded PNG images keyed by file path.

| Field | Type | Notes |
|-------|------|-------|
| `path` | `String` | Absolute path in initramfs |
| `rgba` | `Vec<u8>` | Raw RGBA bytes, 4 bytes per pixel |
| `width` | `u32` | |
| `height` | `u32` | |

**State transitions**: `Unloaded → Loading → Loaded | Error`

### `Layer`
One compositing surface per window or system chrome element.

| Field | Type | Notes |
|-------|------|-------|
| `z` | `u32` | Z-index (existing `win_z` field) |
| `x`, `y` | `i32` | Screen position |
| `width`, `height` | `u32` | Surface dimensions |
| `alpha` | `u8` | Surface-level alpha (255 = opaque) |
| `shadow` | `Option<Shadow>` | Drop shadow parameters |
| `corner_radius` | `u32` | Rounded corner radius in px (0 = square) |
| `pixels` | `Vec<u32>` | RGBA pixel buffer |

### `Shadow`
Drop shadow descriptor attached to a `Layer`.

| Field | Type | Notes |
|-------|------|-------|
| `offset_x` | `i32` | Horizontal offset (default 0) |
| `offset_y` | `i32` | Vertical offset (default 4) |
| `blur_radius` | `u32` | Box blur radius (default 4) |
| `alpha` | `u8` | Shadow opacity (default 120 ≈ 47%) |
| `color` | `u32` | Shadow color RGBA (default 0x00000078) |

### `Animation`
Wall-clock bounded transition between two visual states.

| Field | Type | Notes |
|-------|------|-------|
| `kind` | `AnimKind` | Open / Close / Minimize / Custom |
| `target` | `String` | App name being animated |
| `start_ms` | `u64` | Epoch ms when animation began |
| `duration_ms` | `u32` | Total duration (Open=300, Close=250, Minimize=300) |
| `from_scale` | `f32` | Start scale factor (Open=0.85, Close=1.0) |
| `to_scale` | `f32` | End scale factor (Open=1.0, Close=0.0) |
| `from_alpha` | `u8` | Start opacity (Open=0, Close=255) |
| `to_alpha` | `u8` | End opacity (Open=255, Close=0) |

**State transitions**: `Queued → Running → Complete`
Frame-skip rule: if wall-clock advance exceeds `duration_ms`, snap to final state immediately.

### `DisplayProfile`
Active form-factor layout configuration (read from profile TOML at boot).

| Field | Type | Notes |
|-------|------|-------|
| `kind` | `ProfileKind` | Desktop / Phone / Tablet / Watch / TV / Vision |
| `show_dock` | `bool` | |
| `show_menu_bar` | `bool` | |
| `windowed_mode` | `bool` | false = full-screen single app |
| `focus_ring` | `bool` | TV / Vision: keyboard focus highlight |
| `spatial_layout` | `bool` | Vision: floating 3D panel grid |
| `home_gesture` | `bool` | Phone/Tablet: swipe-up home |
| `crown_scroll` | `bool` | Watch: scroll input maps to content scroll |

### `MenuItem`
Declared by an app in `vyoma.toml`; rendered in menu bar dropdown.

| Field | Type | Notes |
|-------|------|-------|
| `label` | `String` | Display text |
| `shortcut` | `Option<String>` | e.g., "Cmd+N" (display only) |
| `action` | `String` | IPC message sent to app on activation |
| `enabled` | `bool` | Greyed out if false |

### `NotificationBanner`
Queued system notification.

| Field | Type | Notes |
|-------|------|-------|
| `app_name` | `String` | Source app |
| `icon_path` | `Option<String>` | App icon PNG path |
| `title` | `String` | Bold header text |
| `body` | `String` | Body text (max 120 chars) |
| `created_ms` | `u64` | Epoch ms when queued |
| `dismiss_after_ms` | `u32` | Default 4000ms |

**State transitions**: `Queued → Showing → Dismissed`

---

## Key Relationships

```
DisplayProfile
  └── determines which shell chrome elements are active

FontCache
  └── GlyphBitmap[] (cached per char+size)

AppState (existing)
  ├── win_region (existing)
  ├── win_z (existing)
  └── Layer           ← new: per-window compositing surface
        └── Shadow    ← new: optional drop shadow

Supervisor compositor
  ├── Layer[] sorted by z-index
  ├── FontCache (singleton)
  ├── ImageCache (path → decoded PNG)
  ├── Animation[] (active transitions)
  └── NotificationBanner[] (queue)

App manifest (vyoma.toml)
  ├── capabilities.display = true (existing)
  ├── window.icon = "icon.png"    ← new
  └── menu_items[]                ← new
```
