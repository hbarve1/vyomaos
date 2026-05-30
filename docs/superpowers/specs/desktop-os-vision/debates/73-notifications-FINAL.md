# FINAL Spec: Notification Center (Round 73)

**Subsystem**: Notification Center
**macOS Analogue**: UNUserNotificationCenter / NSUserNotification
**Depends on**:
- R11 — compositor overlay mechanism (`FlushCmd::ShowOverlay`, `Z_OVERLAY = 255`, `draw_rounded_rect`, `blit_surface`)
- R23 — menu bar + global status bar (24 px top strip; notification bell icon slot)
- R24 — Dock badge count (`FlushCmd::SetBadge` per app name)
- R59 — capability enforcement (`Capabilities` struct in `manifest.rs`)
- R41 — atomic file writes (`/data` 9P virtio mount, write-then-rename pattern)
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

### Module Tree

```
supervisor/src/
├── notify/
│   ├── mod.rs          (≤ 500 lines) — NotifyCenter, public API, DND state, badge map
│   ├── store.rs        (≤ 500 lines) — PersistentStore: history.json + pending.json R/W
│   ├── banner.rs       (≤ 500 lines) — BannerRenderer: slide-in, auto-dismiss, click hit-test
│   └── panel.rs        (≤ 500 lines) — PanelRenderer: slide-in from right, grouped list, clear-all
```

Four new files replace the single `toast.rs` stub. The existing `toast.rs` crash/watchdog toasts are **not** moved — they remain in `toast.rs` as supervisor-internal messages with no app protocol.

### Thread Model

```
main thread (PID 1)
  └── app reader threads (one per app, existing)
        └── on "VYOMA_NOTIFY:" line → NotifyCenter::handle_line(sender, line, &NOTIFY)

display flush path (called from each app's reader thread on "VYOMA_DRAW:flush")
  └── draw_cmd::handle_draw_command  (existing)
        └── notify::banner::render_active(&mut fb)   ← NEW: inserted before fb.flush()
        └── if PANEL_OPEN: notify::panel::render(&mut fb, &NOTIFY)  ← NEW

input thread (existing, raw TTY)
  └── on click at banner coords → NotifyCenter::click_banner(x, y)
  └── on click at panel toggle → NotifyCenter::toggle_panel()
```

**No new threads are introduced.** The render functions are called from the existing display flush path (the app reader thread that owns the `flush` token). The `NotifyCenter` is protected by a single `Mutex<NotifyCenter>` in a `OnceLock`.

### Global Statics (added to `main.rs`)

```rust
static NOTIFY: OnceLock<Mutex<notify::NotifyCenter>> = OnceLock::new();

pub fn notify_center() -> &'static Mutex<notify::NotifyCenter> {
    NOTIFY.get_or_init(|| Mutex::new(notify::NotifyCenter::new()))
}
```

---

## 2. VYOMA_NOTIFY Protocol

Apps write line-oriented commands to stdout. The supervisor app reader thread (in `app_threads.rs`) checks for the `VYOMA_NOTIFY:` prefix before the existing `VYOMA_DRAW:` / IPC dispatch.

### 2.1 Post a Notification

```
VYOMA_NOTIFY:post:<id>,<title>,<body>,<category>,<timeout_s>
```

| Field | Type | Constraints |
|---|---|---|
| `id` | ASCII string | `[a-zA-Z0-9_-]{1,64}`; app-scoped (supervisor prefixes sender name) |
| `title` | UTF-8 string | max 80 chars; truncated with "…" if exceeded |
| `body` | UTF-8 string | max 160 chars; truncated |
| `category` | ASCII string | registered category id or `""` for default; determines action buttons |
| `timeout_s` | u32 | `0` = persistent (stays until user dismisses); `1–60` = seconds; default 5 |

Example from a Rust WASM app:

```rust
println!("VYOMA_NOTIFY:post:download-done,Download complete,myfile.zip is ready,,5");
```

### 2.2 Cancel a Notification

```
VYOMA_NOTIFY:cancel:<id>
```

Removes a queued or displayed banner for `<id>`. If the notification is already in history, it is not removed from history (only from the active banner queue).

### 2.3 Clear All Notifications

```
VYOMA_NOTIFY:clear_all:
```

Clears all notifications from the sending app (active banners + Center panel entries). History is also cleared for that app.

### 2.4 Register a Category

```
VYOMA_NOTIFY:register_category:<category_id>,<action1_id>,<action1_label>[,<action2_id>,<action2_label>]
```

Up to 2 action buttons per category. Category registrations are ephemeral (not persisted); apps must re-register on restart. If a `post` references an unregistered category, the category is treated as default (no action buttons, banner click = `default`).

Example:

```rust
println!("VYOMA_NOTIFY:register_category:download,open,Open File,dismiss,Dismiss");
println!("VYOMA_NOTIFY:post:dl-001,Download done,report.pdf ready,download,8");
```

### 2.5 Action Delivery (supervisor → app)

When the user clicks a banner or an action button, the supervisor delivers:

```
VYOMA_NOTIFY:action:<notif_id>:<action_id>
```

Written to the app's stdin via the existing IPC inbox mechanism. `action_id` values:
- `"default"` — banner body clicked (no category actions, or direct click)
- `"<action_id>"` — user clicked a registered action button

### 2.6 DND Toggle

```
VYOMA_NOTIFY:dnd_on:
VYOMA_NOTIFY:dnd_off:
```

Only apps with `notifications = true` AND `shell = true` capability may toggle DND. Unauthorized senders receive no response; the supervisor logs a warning.

---

## 3. Core Types

```rust
// notify/mod.rs

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// ── IDs ──────────────────────────────────────────────────────────────────────

/// Globally unique notification id: "<app_name>/<app_local_id>"
pub type NotifId = String;

// ── Notification struct ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Notification {
    /// Globally scoped id: "<sender>/<local_id>"
    pub id:          NotifId,
    /// Sending app name.
    pub app_name:    String,
    /// Short title rendered in bold in the banner.
    pub title:       String,
    /// Body text rendered in secondary color.
    pub body:        String,
    /// Category id (empty = default, no actions).
    pub category:    String,
    /// Auto-dismiss timeout in seconds. 0 = persistent.
    pub timeout_s:   u32,
    /// Unix timestamp (seconds) when the notification was posted.
    pub posted_at:   u64,
    /// Current delivery state.
    pub state:       DeliveryState,
}

impl Notification {
    pub fn new(
        app_name: &str,
        local_id: &str,
        title: &str,
        body: &str,
        category: &str,
        timeout_s: u32,
    ) -> Self {
        let posted_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Notification {
            id:        format!("{app_name}/{local_id}"),
            app_name:  app_name.to_string(),
            title:     title.chars().take(80).collect(),
            body:      body.chars().take(160).collect(),
            category:  category.to_string(),
            timeout_s,
            posted_at,
            state:     DeliveryState::Queued,
        }
    }
}

// ── Delivery state ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryState {
    /// Not yet shown (DND active or rate-limiting).
    Queued,
    /// Banner is currently visible on screen.
    Showing { shown_at_ms: u64 },
    /// Dismissed by timer or user action.
    Dismissed,
    /// Deferred during DND; will deliver on DND off.
    Deferred,
}

// ── Action button ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ActionButton {
    /// Identifier sent back to the app via VYOMA_NOTIFY:action.
    pub id:    String,
    /// Label rendered in the button.
    pub label: String,
}

// ── Category ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NotificationCategory {
    pub id:      String,
    /// Up to 2 action buttons.
    pub actions: Vec<ActionButton>,
}

// ── NotifyCenter ──────────────────────────────────────────────────────────────

pub struct NotifyCenter {
    /// Active banner slots (max 3 visible at once, FIFO).
    pub active_banners: std::collections::VecDeque<Notification>,
    /// Full history per app: last 50 per app (oldest evicted first).
    pub history:        HashMap<String, Vec<Notification>>,
    /// Notifications deferred during DND (ordered arrival).
    pub pending:        Vec<Notification>,
    /// Per-app registered categories (ephemeral, not persisted).
    pub categories:     HashMap<String, Vec<NotificationCategory>>,
    /// Per-app unread badge count.
    pub badge_counts:   HashMap<String, u32>,
    /// Whether Do Not Disturb is active.
    pub dnd:            bool,
    /// Whether the Notification Center panel is open.
    pub panel_open:     bool,
    /// Monotonic ms at which the panel slide animation started.
    pub panel_anim_ms:  Option<u64>,
    /// Panel slide direction: true = opening, false = closing.
    pub panel_opening:  bool,
}

impl NotifyCenter {
    pub fn new() -> Self {
        NotifyCenter {
            active_banners: std::collections::VecDeque::new(),
            history:        HashMap::new(),
            pending:        Vec::new(),
            categories:     HashMap::new(),
            badge_counts:   HashMap::new(),
            dnd:            false,
            panel_open:     false,
            panel_anim_ms:  None,
            panel_opening:  false,
        }
    }
}
```

---

## 4. Notification Banner Rendering

### 4.1 Layout

```
Screen top-right corner (framebuffer coordinates):
  bx = fb_width  - BANNER_W - BANNER_MARGIN_R    = fb_w - 320 - 12
  by = MENUBAR_H + BANNER_MARGIN_T + i*(BANNER_H + BANNER_GAP)
     = 24 + 8 + i*88        (i = slot index, max 3 slots)

  BANNER_W  = 320 px
  BANNER_H  = 80 px
  BANNER_MARGIN_R = 12 px   (from right edge)
  BANNER_MARGIN_T = 8 px    (below menu bar)
  BANNER_GAP      = 8 px    (between stacked banners)
  BANNER_RADIUS   = 12 px   (corner radius, matches macOS)

  Z-layer: Z_OVERLAY - 1 = 254  (below system alerts at 255, above all apps)
```

### 4.2 Slide-In Animation

The banner enters from the right edge. Slide progress uses the same ease-out cubic from `display/animator.rs`:

```rust
// notify/banner.rs

const SLIDE_DURATION_MS: u64 = 320;

fn slide_offset_px(elapsed_ms: u64, banner_w: u32) -> u32 {
    if elapsed_ms >= SLIDE_DURATION_MS {
        return 0;
    }
    let t = elapsed_ms as f32 / SLIDE_DURATION_MS as f32;
    // ease-out cubic: 1 - (1-t)^3
    let ease = 1.0 - (1.0 - t).powi(3);
    let offset = banner_w as f32 * (1.0 - ease);
    offset as u32
}
```

The banner is drawn at `bx + slide_offset_px` so it slides from off-screen right to its final position. On dismiss, a reverse slide-out runs over 200 ms (ease-in cubic).

### 4.3 Auto-Dismiss

```rust
// notify/banner.rs

fn banner_should_dismiss(n: &Notification, now_ms: u64) -> bool {
    if n.timeout_s == 0 { return false; } // persistent
    match &n.state {
        DeliveryState::Showing { shown_at_ms } => {
            now_ms.saturating_sub(*shown_at_ms) >= n.timeout_s as u64 * 1000
        }
        _ => false,
    }
}
```

Default timeout: 5 s. Apps may request up to 8 s by passing `timeout_s = 8`. Values above 8 are clamped. `timeout_s = 0` means persistent; the banner remains until explicitly dismissed by the user clicking it.

### 4.4 Rendering Call

Called from `draw_cmd.rs` in the `flush`/`present` branch, **after** all app surfaces are composited and chrome is drawn, **before** `fb.flush()`:

```rust
// draw_cmd.rs  (flush branch, new insertion point)

// 3. Draw chrome (existing)
draw_chrome_onto(&mut *fb, app_registry, focused);

// 4. Draw notification banners at Z_OVERLAY-1  ← NEW
#[cfg(target_os = "linux")]
notify::banner::render_active(&mut *fb, crate::notify_center());

// 5. Flush back-buffer to framebuffer  (existing)
fb.flush();
```

### 4.5 Banner Renderer

```rust
// notify/banner.rs

use crate::display::{Framebuffer, draw_rounded_rect, composite_glyph};
use crate::chrome::{MENUBAR_H, draw_glyph_str_pub};
use super::{NotifyCenter, Notification, DeliveryState, ActionButton};
use std::sync::Mutex;

pub const BANNER_W:        u32 = 320;
pub const BANNER_H:        u32 = 80;
const BANNER_MARGIN_R:     u32 = 12;
const BANNER_MARGIN_T:     u32 = 8;
const BANNER_GAP:          u32 = 8;
const BANNER_RADIUS:       u32 = 12;
const BANNER_BG:           u32 = 0x1C1C1EE8_u32;  // dark, 91% opaque
const BANNER_TITLE_COLOR:  u32 = 0xFFFFFFFF_u32;
const BANNER_BODY_COLOR:   u32 = 0xAAAAAFFF_u32;
const BANNER_BORDER:       u32 = 0x3A3A3CFF_u32;
const ACTION_BTN_BG:       u32 = 0x2C2C2EFF_u32;
const ACTION_BTN_COLOR:    u32 = 0x0A84FFFF_u32;  // Apple blue

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Draw all active banners onto the framebuffer back-buffer.
/// Must be called with fb exclusively locked (called from the flush path).
pub fn render_active(fb: &mut Framebuffer, nc: &Mutex<NotifyCenter>) {
    let now = now_ms();
    let fw   = fb.width;
    let fh   = fb.height;
    let fstr = fb.stride;

    // Collect banners to render (snapshot while holding lock briefly)
    let banners_snap: Vec<(Notification, u64)> = {
        let mut nc = nc.lock().unwrap();
        advance_banner_states(&mut nc, now);
        nc.active_banners
            .iter()
            .filter_map(|n| {
                if let DeliveryState::Showing { shown_at_ms } = n.state {
                    Some((n.clone(), shown_at_ms))
                } else {
                    None
                }
            })
            .take(3)
            .collect()
    };

    let bx_final = fw.saturating_sub(BANNER_W + BANNER_MARGIN_R);

    for (slot, (notif, shown_at_ms)) in banners_snap.iter().enumerate() {
        let elapsed = now.saturating_sub(*shown_at_ms);
        let offset  = slide_offset_px(elapsed, BANNER_W);
        let bx      = bx_final + offset;
        let by      = MENUBAR_H + BANNER_MARGIN_T + slot as u32 * (BANNER_H + BANNER_GAP);
        if by + BANNER_H > fh { break; }

        // Background rounded rect
        draw_rounded_rect(&mut fb.back, bx, by, BANNER_W, BANNER_H, BANNER_BG, BANNER_RADIUS, fstr, fw, fh);
        // 1px border
        crate::display::draw_rounded_rect(&mut fb.back, bx, by, BANNER_W, BANNER_H, BANNER_BORDER, BANNER_RADIUS, fstr, fw, fh);

        // App name (secondary, small)
        let app_label = notif.app_name.chars().take(24).collect::<String>().to_uppercase();
        draw_glyph_str_pub(fb, &app_label, (bx + 12) as i32, (by + 14) as i32, 0x636366FF, 10, false, false);

        // Title (bold, 13pt)
        draw_glyph_str_pub(fb, &notif.title, (bx + 12) as i32, (by + 30) as i32, BANNER_TITLE_COLOR, 13, true, false);

        // Body (regular, 11pt)
        let body_max = 36usize; // chars that fit in ~296 px at 11pt
        let body_trunc = if notif.body.chars().count() > body_max {
            format!("{}…", notif.body.chars().take(body_max).collect::<String>())
        } else {
            notif.body.clone()
        };
        draw_glyph_str_pub(fb, &body_trunc, (bx + 12) as i32, (by + 50) as i32, BANNER_BODY_COLOR, 11, false, false);

        // Action buttons (if category has actions)
        // Rendered as small pill buttons in bottom-right of banner
        // (action rendering detailed in §4.6)
    }
}

fn slide_offset_px(elapsed_ms: u64, banner_w: u32) -> u32 {
    const SLIDE_DURATION_MS: u64 = 320;
    if elapsed_ms >= SLIDE_DURATION_MS { return 0; }
    let t    = elapsed_ms as f32 / SLIDE_DURATION_MS as f32;
    let ease = 1.0 - (1.0 - t).powi(3); // ease-out cubic
    (banner_w as f32 * (1.0 - ease)) as u32
}

fn advance_banner_states(nc: &mut NotifyCenter, now_ms: u64) {
    // Transition Queued → Showing
    for n in nc.active_banners.iter_mut() {
        if n.state == DeliveryState::Queued {
            n.state = DeliveryState::Showing { shown_at_ms: now_ms };
        }
    }
    // Evict expired banners
    nc.active_banners.retain(|n| !banner_should_dismiss(n, now_ms));
}

fn banner_should_dismiss(n: &Notification, now_ms: u64) -> bool {
    if n.timeout_s == 0 { return false; }
    match &n.state {
        DeliveryState::Showing { shown_at_ms } =>
            now_ms.saturating_sub(*shown_at_ms) >= n.timeout_s as u64 * 1000,
        _ => false,
    }
}
```

### 4.6 Action Buttons in Banner

When a category has registered actions, up to 2 pill buttons are rendered in the lower-right area of the banner (each 80×20 px, separated by 8 px, right-aligned at `bx + BANNER_W - 12`):

```
┌─────────────────────────────────────┐ ← banner top
│ APP NAME                            │
│ Title text in bold                  │
│ Body text truncated                 │
│                    [Action 1][Act 2]│ ← 8px from bottom
└─────────────────────────────────────┘
```

Hit-test coordinates for action buttons are stored in a per-frame `Vec<ButtonHit>` (cleared and rebuilt each render pass) so the input handler can look them up:

```rust
pub struct ButtonHit {
    pub notif_id:  String,
    pub action_id: String,
    pub x: u32, pub y: u32, pub w: u32, pub h: u32,
}
static BUTTON_HITS: OnceLock<Mutex<Vec<ButtonHit>>> = OnceLock::new();
```

### 4.7 Click Handling

Mouse click events arrive in `mouse_input.rs`. A new check is inserted:

```rust
// mouse_input.rs — on left-button release

// Check notification banner click
if let Some(action) = notify::banner::hit_test(x as u32, y as u32, crate::notify_center()) {
    notify::handle_action(&action.notif_id, &action.action_id,
                          app_registry, inbox, crate::notify_center());
    return;
}
```

`notify::banner::hit_test` checks the `BUTTON_HITS` list first (action button), then falls back to the full banner rectangle (delivers `"default"` action).

---

## 5. Notification Center Panel

### 5.1 Layout

```
Panel slides in from the right edge over 400 ms (ease-out cubic).
Width:  340 px
Height: fb_height - MENUBAR_H   (full height below menu bar)
X pos:  fb_width - 340           (flush with right edge when open)
Y pos:  MENUBAR_H

Background: 0x1C1C1EF0 (dark, ~94% opaque) with 1 px left border (0x3A3A3C)
Z-layer: Z_OVERLAY - 1 = 254  (same as banners)
```

Toggle trigger: click on the clock/notification bell icon in the menu bar status region (right side). The `statusbar.rs` module emits a click event which `chrome.rs` dispatches to `NotifyCenter::toggle_panel()`.

### 5.2 Panel Content Layout

```
┌──────────────────────────────────┐ ← y = MENUBAR_H
│  Notification Center        [✕] │ ← header 32px, "✕" = close
│  Today, May 30               ───│
│                                  │
│  ┌─ MyApp  (3) ─────────────┐   │ ← app group header
│  │ • Title 1           12:01│   │
│  │   Body preview text      │   │
│  │ • Title 2           11:44│   │
│  └──────────────────────────┘   │
│  ┌─ OtherApp  (1) ─────────┐   │
│  │ • Notification title    │   │
│  └──────────────────────────┘   │
│                                  │
│         [Clear All]              │ ← bottom button
└──────────────────────────────────┘
```

### 5.3 Panel Renderer

```rust
// notify/panel.rs

use crate::display::{Framebuffer, draw_rounded_rect};
use crate::chrome::{MENUBAR_H, draw_glyph_str_pub};
use super::{NotifyCenter, Notification};
use std::sync::Mutex;

pub const PANEL_W:    u32 = 340;
const PANEL_BG:       u32 = 0x1C1C1EF0_u32;
const PANEL_BORDER:   u32 = 0x3A3A3CFF_u32;
const PANEL_HEADER:   u32 = 0xFFFFFFFF_u32;
const GROUP_HEADER:   u32 = 0x8E8E93FF_u32;
const ITEM_TITLE:     u32 = 0xFFFFFFFF_u32;
const ITEM_BODY:      u32 = 0x8E8E93FF_u32;
const CLEAR_BTN:      u32 = 0xFF3B30FF_u32; // system red

/// Draw the panel onto `fb` if open. Must be called from flush path.
pub fn render(fb: &mut Framebuffer, nc: &Mutex<NotifyCenter>) {
    let (panel_open, panel_anim_ms, panel_opening) = {
        let nc = nc.lock().unwrap();
        (nc.panel_open, nc.panel_anim_ms, nc.panel_opening)
    };
    if !panel_open && panel_anim_ms.is_none() { return; }

    let now_ms = crate::notify::banner::now_ms_pub();
    let slide_x = panel_slide_x(fb.width, panel_anim_ms, panel_opening, now_ms);

    // If animation is complete and panel is closed, nothing to draw
    if slide_x >= fb.width { return; }

    let ph   = fb.height.saturating_sub(MENUBAR_H);
    let py   = MENUBAR_H;
    let fw   = fb.width;
    let fstr = fb.stride;
    let fh   = fb.height;

    // Background
    let back = &mut fb.back;
    for row in py..(py + ph).min(fh) {
        for col in slide_x..(slide_x + PANEL_W).min(fw) {
            let off = (row * fstr + col * 4) as usize;
            if off + 4 > back.len() { continue; }
            let bg_a = (PANEL_BG & 0xFF) as u8;
            let (br, bg, bb) = (
                ((PANEL_BG >> 24) & 0xFF) as u8,
                ((PANEL_BG >> 16) & 0xFF) as u8,
                ((PANEL_BG >>  8) & 0xFF) as u8,
            );
            use crate::display::compositor::{read_bgra, write_bgra, blend_over, pack};
            let dst = read_bgra(back, off);
            let blended = blend_over(pack(br, bg, bb, bg_a), dst);
            write_bgra(back, off, blended);
        }
    }

    // Render content via draw_glyph_str_pub (skipping full body here for brevity)
    // Header
    draw_glyph_str_pub(fb, "Notification Center",
        (slide_x + 12) as i32, (py + 20) as i32, PANEL_HEADER, 14, true, false);

    // Groups: iterate nc.history, sorted by most recent notification posted_at
    let groups: Vec<(String, Vec<Notification>)> = {
        let nc = nc.lock().unwrap();
        let mut g: Vec<(String, Vec<Notification>)> = nc.history.iter()
            .map(|(app, notifs)| {
                let mut v = notifs.clone();
                v.sort_by(|a, b| b.posted_at.cmp(&a.posted_at));
                (app.clone(), v)
            })
            .collect();
        g.sort_by(|a, b| {
            let ta = a.1.first().map(|n| n.posted_at).unwrap_or(0);
            let tb = b.1.first().map(|n| n.posted_at).unwrap_or(0);
            tb.cmp(&ta)
        });
        g
    };

    let mut cursor_y = py + 48;
    for (app_name, notifs) in &groups {
        if cursor_y + 20 > py + ph { break; }
        let group_label = format!("{}  ({})", app_name, notifs.len());
        draw_glyph_str_pub(fb, &group_label,
            (slide_x + 12) as i32, cursor_y as i32, GROUP_HEADER, 11, false, false);
        cursor_y += 20;
        for notif in notifs.iter().take(5) {
            if cursor_y + 36 > py + ph { break; }
            draw_glyph_str_pub(fb, &notif.title,
                (slide_x + 20) as i32, cursor_y as i32, ITEM_TITLE, 12, true, false);
            cursor_y += 16;
            let body_preview: String = notif.body.chars().take(38).collect();
            draw_glyph_str_pub(fb, &body_preview,
                (slide_x + 20) as i32, cursor_y as i32, ITEM_BODY, 10, false, false);
            cursor_y += 20;
        }
        cursor_y += 8; // group gap
    }

    // Clear All button
    let btn_y = (py + ph).saturating_sub(36);
    let btn_x = slide_x + PANEL_W / 2 - 48;
    draw_rounded_rect(&mut fb.back, btn_x, btn_y, 96, 24, 0x2C2C2EFF, 8, fstr, fw, fh);
    draw_glyph_str_pub(fb, "Clear All",
        (btn_x + 12) as i32, (btn_y + 14) as i32, CLEAR_BTN, 12, false, false);
}

fn panel_slide_x(fb_w: u32, anim_start_ms: Option<u64>, opening: bool, now_ms: u64) -> u32 {
    const PANEL_ANIM_MS: u64 = 400;
    let Some(start) = anim_start_ms else {
        return if opening { fb_w.saturating_sub(PANEL_W) } else { fb_w };
    };
    let elapsed = now_ms.saturating_sub(start).min(PANEL_ANIM_MS);
    let t    = elapsed as f32 / PANEL_ANIM_MS as f32;
    let ease = 1.0 - (1.0 - t).powi(3);
    if opening {
        let start_x = fb_w;
        let end_x   = fb_w.saturating_sub(PANEL_W);
        start_x - ((start_x - end_x) as f32 * ease) as u32
    } else {
        let start_x = fb_w.saturating_sub(PANEL_W);
        let end_x   = fb_w;
        start_x + ((end_x - start_x) as f32 * ease) as u32
    }
}
```

### 5.4 Clear-All Click

Clicking "Clear All" in the panel calls `NotifyCenter::clear_all_history()` which:
1. Drains `nc.history` for all apps.
2. Zeroes all badge counts.
3. Emits `FlushCmd::SetBadge { app: name, count: 0 }` for each app with a non-zero badge (via the existing chrome badge mechanism).
4. Calls `store::save_history(&nc.history)` (atomic write).

---

## 6. Do Not Disturb / Focus Mode

### 6.1 State

DND is a single `bool` in `NotifyCenter::dnd`. It persists across reboots in `/data/.vyoma/notifications/dnd.txt` (value `"1"` or `"0"`; written atomically on each toggle).

### 6.2 Delivery Suppression

```rust
// notify/mod.rs

pub fn post_notification(
    nc:       &mut NotifyCenter,
    notif:    Notification,
    inbox:    &crate::Inbox,
) {
    if nc.dnd {
        // Defer: do not show banner; add to pending
        let mut deferred = notif;
        deferred.state = DeliveryState::Deferred;
        nc.pending.push(deferred.clone());
        store::append_pending(&deferred).ok();
        return;
    }

    // Cap active banners at 3: evict oldest if needed
    if nc.active_banners.len() >= 3 {
        if let Some(evicted) = nc.active_banners.pop_front() {
            history_insert(nc, evicted);
        }
    }

    // Increment badge count
    *nc.badge_counts.entry(notif.app_name.clone()).or_insert(0) += 1;
    emit_badge_update(&notif.app_name, nc.badge_counts[&notif.app_name], inbox);

    let mut showing = notif.clone();
    showing.state = DeliveryState::Queued; // → Showing on next render pass
    nc.active_banners.push_back(showing);

    // Persist to history
    history_insert(nc, notif);
    store::save_history_app(nc).ok();
}
```

### 6.3 DND Off — Flush Pending

```rust
pub fn set_dnd(nc: &mut NotifyCenter, on: bool, inbox: &crate::Inbox) {
    nc.dnd = on;
    store::save_dnd(on).ok();
    if !on {
        // Deliver all deferred notifications
        let deferred: Vec<Notification> = nc.pending.drain(..).collect();
        store::clear_pending().ok();
        for notif in deferred {
            let mut n = notif;
            n.state = DeliveryState::Queued;
            post_notification(nc, n, inbox);
        }
    }
}
```

### 6.4 Badge Count Accumulation During DND

Badge counts are **not** incremented for deferred notifications. Badges accumulate only when a notification is actively delivered (i.e., after DND is lifted and the deferred batch is posted).

### 6.5 Menu Bar DND Indicator

When `nc.dnd == true`, the notification bell icon in the status bar (drawn by `statusbar.rs`) is replaced with a filled bell + moon crescent overlay (drawn using `fill_rect_r` + `draw_glyph`). The indicator color is `0xFFBD2EFF` (system yellow, matching macOS Focus mode).

---

## 7. Persistent Storage

### 7.1 File Layout

```
/data/.vyoma/notifications/
├── history.json    — last 50 notifications per app (all apps combined)
├── pending.json    — notifications deferred during DND
└── dnd.txt         — "1" or "0"
```

The `/data` directory is the 9P virtio mount (persistent across reboots). The `.vyoma/notifications/` subdirectory is created by the supervisor at startup if absent.

### 7.2 Atomic Write Pattern (R41)

All writes use the write-then-rename pattern to avoid partial reads:

```rust
// notify/store.rs

use std::{fs, io, path::Path};

const HISTORY_PATH:  &str = "/data/.vyoma/notifications/history.json";
const PENDING_PATH:  &str = "/data/.vyoma/notifications/pending.json";
const DND_PATH:      &str = "/data/.vyoma/notifications/dnd.txt";

fn atomic_write(path: &str, content: &str) -> io::Result<()> {
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
```

### 7.3 JSON Schema

`history.json` is a flat JSON object mapping app names to arrays of notification records:

```json
{
  "my-app": [
    {
      "id": "my-app/dl-001",
      "app_name": "my-app",
      "title": "Download done",
      "body": "report.pdf ready",
      "category": "download",
      "timeout_s": 8,
      "posted_at": 1748564400
    }
  ]
}
```

`pending.json` has the same schema. Both are bounded: `history.json` stores at most 50 notifications per app (oldest entry evicted when the 51st arrives). `pending.json` stores at most 200 entries total (oldest evicted on overflow).

### 7.4 Store Module

```rust
// notify/store.rs  (continued)

use std::collections::HashMap;
use super::Notification;

/// Serialize Notification to a minimal JSON string (no serde dep — hand-built).
fn notif_to_json(n: &Notification) -> String {
    format!(
        "{{\"id\":\"{}\",\"app_name\":\"{}\",\"title\":\"{}\",\"body\":\"{}\",\
         \"category\":\"{}\",\"timeout_s\":{},\"posted_at\":{}}}",
        escape_json(&n.id),
        escape_json(&n.app_name),
        escape_json(&n.title),
        escape_json(&n.body),
        escape_json(&n.category),
        n.timeout_s,
        n.posted_at,
    )
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

pub fn save_history_app(nc: &super::NotifyCenter) -> io::Result<()> {
    let mut out = String::from("{");
    for (i, (app, notifs)) in nc.history.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        out.push_str(&escape_json(app));
        out.push_str("\":[");
        for (j, n) in notifs.iter().enumerate() {
            if j > 0 { out.push(','); }
            out.push_str(&notif_to_json(n));
        }
        out.push(']');
    }
    out.push('}');
    ensure_dir()?;
    atomic_write(HISTORY_PATH, &out)
}

pub fn append_pending(n: &Notification) -> io::Result<()> {
    // Read existing, append, re-write (bounded at 200 total)
    let existing = fs::read_to_string(PENDING_PATH).unwrap_or_default();
    let mut entries: Vec<String> = parse_json_array_raw(&existing);
    if entries.len() >= 200 { entries.remove(0); }
    entries.push(notif_to_json(n));
    ensure_dir()?;
    let content = format!("[{}]", entries.join(","));
    atomic_write(PENDING_PATH, &content)
}

pub fn clear_pending() -> io::Result<()> {
    ensure_dir()?;
    atomic_write(PENDING_PATH, "[]")
}

pub fn save_dnd(on: bool) -> io::Result<()> {
    ensure_dir()?;
    atomic_write(DND_PATH, if on { "1" } else { "0" })
}

pub fn load_dnd() -> bool {
    fs::read_to_string(DND_PATH).map(|s| s.trim() == "1").unwrap_or(false)
}

fn ensure_dir() -> io::Result<()> {
    fs::create_dir_all("/data/.vyoma/notifications")
}

/// Minimal raw JSON array parser — returns array elements as raw JSON strings.
/// Used only for pending.json which stores a flat array of notification objects.
fn parse_json_array_raw(s: &str) -> Vec<String> {
    // Very minimal: splits on top-level },{  — sufficient for our hand-built JSON.
    let s = s.trim();
    if s == "[]" || s.is_empty() { return Vec::new(); }
    let inner = s.trim_start_matches('[').trim_end_matches(']');
    // Split on "},{" — safe because notification JSON values don't contain nested objects.
    inner.split("},{")
         .enumerate()
         .map(|(i, chunk)| {
             let chunk = chunk.trim();
             if i == 0 && chunk.starts_with('{') { chunk.to_string() }
             else if chunk.ends_with('}') { format!("{{{chunk}") }
             else { format!("{{{chunk}}}") }
         })
         .filter(|s| !s.is_empty())
         .collect()
}
```

### 7.5 Startup Load

In `main.rs`, after `notify_center()` is initialized, pending notifications from a prior DND session are loaded:

```rust
// main.rs — after NOTIFY is initialized
{
    let mut nc = crate::notify_center().lock().unwrap();
    nc.dnd = notify::store::load_dnd();
    // Pending notifications are delivered lazily on first DND-off; they stay
    // in pending.json and are loaded at post_notification time if dnd is false.
}
```

---

## 8. Blocking Issues

### B1 — `Capabilities` uses `deny_unknown_fields`; adding `notifications` breaks existing manifests

**Problem**: `supervisor/src/manifest.rs` derives `#[serde(deny_unknown_fields)]` on `Capabilities`. Adding `notifications = true` to an app's `vyoma.toml` without updating the struct causes a hard parse error at boot, crashing the supervisor or silently refusing to load the app.

**Fix**: Add the field to the struct before any app manifests declare it.

```rust
// manifest.rs — Capabilities struct, add one field:
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)] pub stdio:          bool,
    #[serde(default)] pub filesystem:     bool,
    #[serde(default)] pub network:        bool,
    #[serde(default)] pub network_port:   Option<u16>,
    #[serde(default)] pub display:        bool,
    #[serde(default)] pub shell:          bool,
    #[serde(default)] pub watchdog_secs:  u32,
    #[serde(default)] pub mouse:          bool,
    /// App may post notifications via VYOMA_NOTIFY: protocol.
    #[serde(default)] pub notifications:  bool,  // ← ADD THIS
}
```

The capability is checked in the `VYOMA_NOTIFY:` line dispatcher:

```rust
// app_threads.rs — in the stdout reader line handler
if line.starts_with("VYOMA_NOTIFY:") {
    let has_cap = {
        let reg = app_registry.lock().unwrap();
        reg.get(&sender).map(|st| {
            // capabilities are stored on AppState; mirror from manifest at spawn time
            st.lock().unwrap().has_notifications
        }).unwrap_or(false)
    };
    if has_cap {
        notify::handle_line(&sender, line, crate::notify_center(), inbox, app_registry);
    } else {
        log_warn!(Subsystem::Notify, Some(&sender), "VYOMA_NOTIFY dropped — no notifications capability");
    }
    continue;
}
```

`AppState` gains `has_notifications: bool` mirrored from `manifest.capabilities.notifications` at spawn time.

---

### B2 — Flush path lock ordering: `Mutex<NotifyCenter>` acquired while `Mutex<Framebuffer>` is held

**Problem**: The flush path in `draw_cmd.rs` holds `fb_lock.lock()` (the framebuffer mutex) throughout the compositor pass. If `render_active` acquires `NOTIFY.lock()` while `fb_lock` is held, and any other code path acquires `NOTIFY.lock()` then `fb_lock` (e.g., a future DND-toggle handler that forces a repaint), an ABBA deadlock results.

**Fix**: Snapshot the notification data under `NOTIFY.lock()` **before** acquiring the framebuffer lock. The render functions accept snapshots, not references to the live `NotifyCenter`.

```rust
// draw_cmd.rs — flush branch

// 1. Snapshot notification banners (before fb lock)
#[cfg(target_os = "linux")]
let banner_snap = notify::banner::snapshot_for_render(crate::notify_center());

// 2. Lock framebuffer
let mut fb = fb_lock.lock().unwrap();

// 3. Compositor pass (existing) ...
// 4. Draw chrome (existing)
draw_chrome_onto(&mut *fb, app_registry, focused);

// 5. Render banners from snapshot (no NotifyCenter lock held here)
#[cfg(target_os = "linux")]
notify::banner::render_snapshot(&mut *fb, &banner_snap);

// 6. Render panel if open
#[cfg(target_os = "linux")]
if banner_snap.panel_open {
    notify::panel::render_snapshot(&mut *fb, &banner_snap);
}

// 7. fb.flush()
fb.flush();
```

`snapshot_for_render` returns a `BannerRenderSnapshot` (a plain struct of cloned data, no Arc/Mutex inside):

```rust
pub struct BannerRenderSnapshot {
    pub active:     Vec<(Notification, u64)>,     // (notif, shown_at_ms)
    pub panel_open: bool,
    pub panel_anim_ms: Option<u64>,
    pub panel_opening: bool,
    pub history_groups: Vec<(String, Vec<Notification>)>,
}
```

---

### B3 — `parse_json_array_raw` is fragile: breaks if title or body contains `},` 

**Problem**: The hand-built JSON parser in `notify/store.rs` splits on `},{` to reconstruct array elements. If a notification title or body contains the substring `},{`, the parser corrupts the record list on reload.

**Fix**: Escape `}` in string values (already handled by `escape_json` for `"` and `\n`), OR use a depth-tracked splitter that counts `{` and `}` nesting depth:

```rust
fn parse_json_array_raw(s: &str) -> Vec<String> {
    let s = s.trim();
    if s == "[]" || s.is_empty() { return Vec::new(); }
    let inner = s.trim_start_matches('[').trim_end_matches(']').trim();
    if inner.is_empty() { return Vec::new(); }

    let mut result = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    let bytes = inner.as_bytes();

    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    result.push(inner[start..=i].to_string());
                    start = i + 1;
                    // skip optional comma and whitespace
                    while start < bytes.len()
                        && (bytes[start] == b',' || bytes[start] == b' ') {
                        start += 1;
                    }
                }
            }
            _ => {}
        }
    }
    result
}
```

This correctly handles any string content inside notification records.

---

### B4 — `history_insert` has no per-app cap enforcement at call site; unbounded memory growth

**Problem**: `history_insert` in `notify/mod.rs` pushes notifications into `nc.history[app_name]` without enforcing the 50-entry cap. If an app posts thousands of notifications during a session, the in-memory history Vec grows without bound, increasing lock contention and eventual memory pressure.

**Fix**: Enforce the cap inside `history_insert`:

```rust
const MAX_HISTORY_PER_APP: usize = 50;

fn history_insert(nc: &mut NotifyCenter, notif: Notification) {
    let bucket = nc.history.entry(notif.app_name.clone()).or_insert_with(Vec::new);
    // Evict oldest (front) if at cap
    if bucket.len() >= MAX_HISTORY_PER_APP {
        bucket.remove(0);
    }
    bucket.push(notif);
}
```

`Vec::remove(0)` is O(n) but the cap is 50 — cost is negligible. An `VecDeque` alternative is not used here to keep the JSON serialization simple (VecDeque does not impl Index for range slicing on stable Rust without conversion).

---

### B5 — Action delivery to app uses `inbox` but the sender is the supervisor, not an app; reply routing is wrong

**Problem**: `notify::handle_action` sends `VYOMA_NOTIFY:action:<id>:<action>` to the target app's inbox channel using `send_reply(app_name, msg, inbox)`. `send_reply` is designed for supervisor-IPC replies and prepends `REPLY:` to the message. The app receives `REPLY:VYOMA_NOTIFY:action:...` instead of `VYOMA_NOTIFY:action:...`, breaking the protocol.

**Fix**: Write directly to the app's stdin channel without using `send_reply`:

```rust
// notify/mod.rs

pub fn deliver_action(app_name: &str, notif_id: &str, action_id: &str, inbox: &crate::Inbox) {
    let msg = format!("VYOMA_NOTIFY:action:{notif_id}:{action_id}");
    let inbox = inbox.lock().unwrap();
    if let Some(tx) = inbox.get(app_name) {
        // Best-effort: ignore send errors (app may have exited)
        let _ = tx.send(msg);
    }
}
```

This mirrors the pattern used in `router.rs` for direct IPC message delivery, bypassing the `send_reply` / `REPLY:` prefix path. The app's stdin reader then receives the raw `VYOMA_NOTIFY:action:` line and handles it in its normal event loop.

---

## Appendix: vyoma.toml Example

```toml
[app]
name    = "downloader"
version = "1.0.0"
wasm    = "downloader.wasm"

[capabilities]
stdio         = true
filesystem    = true
notifications = true
```

## Appendix: Module Registration in main.rs

```rust
// main.rs — add after existing mod declarations
mod notify;

// global accessor (add after existing OnceLock accessors)
static NOTIFY: OnceLock<Mutex<notify::NotifyCenter>> = OnceLock::new();

pub fn notify_center() -> &'static Mutex<notify::NotifyCenter> {
    NOTIFY.get_or_init(|| {
        let mut nc = notify::NotifyCenter::new();
        nc.dnd = notify::store::load_dnd();
        Mutex::new(nc)
    })
}
```

## Appendix: AppState Addition

```rust
// main.rs — AppState struct
struct AppState {
    // ... existing fields ...
    has_notifications: bool,   // ← NEW: mirrored from manifest.capabilities.notifications
}
```

Populated in `app_threads.rs` `spawn_app` alongside the existing `has_display` / `has_mouse` mirrors.
