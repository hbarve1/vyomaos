# FINAL Spec: WASM App Framework (Round 76)

**macOS Analogue**: AppKit / UIKit / SwiftUI
**Status**: FINAL — all blocking issues resolved
**Subsystem**: R76 — `vyoma-ui` client-side widget library
**Location**: `libs/vyoma-ui/` (linked into WASM app binaries)
**Last Updated**: 2026-05-30

---

## Overview

`vyoma-ui` is a Rust crate that WASM apps link against to build reactive, widget-based UIs on
VyomaOS. It is entirely client-side — it runs inside the WASM binary, not in the supervisor.
It translates widget state into `VYOMA_DRAW:` stdout protocol lines and parses `VYOMA_INPUT:`
stdin lines into typed events. There are no supervisor changes required.

**Analogue**: AppKit + UIKit + SwiftUI combined — `App` trait mirrors `NSApplicationDelegate`,
`Widget` trait mirrors `NSView`, `DrawCtx` mirrors `NSGraphicsContext`.

---

## 1. Architecture

### Crate Location

```
libs/
  vyoma-ui/
    Cargo.toml
    src/
      lib.rs            ≤500 lines  App trait, run(), event loop
      event.rs          ≤500 lines  Event enum, stdin parser
      draw.rs           ≤500 lines  DrawCtx, clipping, rounded rects
      theme.rs          ≤200 lines  ColorTheme, dark/light presets
      layout/
        mod.rs          ≤500 lines  BoxLayout, FlexLayout, Constraint, two-pass layout
      widget/
        mod.rs          ≤300 lines  Widget trait, WidgetId, FocusManager, hit-test
        button.rs       ≤250 lines  Button
        label.rs        ≤150 lines  Label
        text_input.rs   ≤350 lines  TextInput with cursor + blink state
        container.rs    ≤300 lines  Container (VStack / HStack)
        scroll.rs       ≤300 lines  ScrollView with offset
```

### Thread Model

WASM is single-threaded. The event loop is a `loop { read_line → parse → on_event → render →
flush }` on the main thread. There are no background threads, no `Arc<Mutex<_>>`, no channels.
All mutable state lives in the `App` implementor. Cursor blink is tracked by frame count
(every N frames toggle), not by wall-clock timer.

### No External Dependencies

`vyoma-ui` uses only `std`. No `serde`, no `fontdue`, no allocator overrides. Binary size
impact on WASM apps is minimal (~15 KB uncompressed).

---

## 2. Cargo Configuration

```toml
# libs/vyoma-ui/Cargo.toml
[package]
name    = "vyoma-ui"
version = "0.1.0"
edition = "2021"

# No [dependencies] — stdlib only
```

WASM apps depend on it via a path dependency:

```toml
# apps/my-app/Cargo.toml
[package]
name    = "my-app"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "my-app"
path = "src/main.rs"

[dependencies]
vyoma-ui = { path = "../../libs/vyoma-ui" }
```

Workspace membership in `libs/vyoma-ui/` does NOT need to be added to the root `Cargo.toml`
(WASM apps each have their own workspace root). If a future monorepo workspace is desired, add
`"libs/vyoma-ui"` to the root workspace members list.

---

## 3. App Trait & Event Loop

### `lib.rs`

```rust
// libs/vyoma-ui/src/lib.rs

pub mod draw;
pub mod event;
pub mod layout;
pub mod theme;
pub mod widget;

pub use draw::DrawCtx;
pub use event::{Event, KeyCode};
pub use layout::{BoxLayout, Constraint, FlexLayout};
pub use theme::{dark_theme, light_theme, ColorTheme};
pub use widget::{Widget, WidgetId};

use std::io::{self, BufRead, Write};

/// Implement this trait for your application root.
pub trait App {
    /// Called for every parsed event before render.
    fn on_event(&mut self, event: Event, ctx: &mut DrawCtx);
    /// Called after every on_event to redraw the full frame.
    fn render(&self, ctx: &mut DrawCtx);
}

/// Start the event loop. Never returns.
///
/// Invariants:
/// - Reads stdin one line at a time (non-blocking via try_read).
/// - Always calls render() + flush() once per iteration even with no event
///   (handles the initial paint and cursor blink).
/// - Calls render() immediately before the first blocking read so the window
///   is not blank on startup.
pub fn run(mut app: impl App) -> ! {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut ctx = DrawCtx::new();

    // Initial paint before waiting for input.
    app.render(&mut ctx);
    ctx.flush();
    let _ = stdout.lock().flush();

    let mut reader = stdin.lock();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // EOF — supervisor closed stdin; exit cleanly.
                std::process::exit(0);
            }
            Ok(_) => {
                let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                if let Some(ev) = event::parse_line(trimmed) {
                    app.on_event(ev, &mut ctx);
                }
                // Always re-render after any input (including unrecognised lines).
                app.render(&mut ctx);
                ctx.flush();
                let _ = io::stdout().lock().flush();
            }
            Err(_) => {
                // I/O error — exit.
                std::process::exit(1);
            }
        }
    }
}
```

**Why render on every line**: WASM stdin `read_line` blocks until a newline arrives. This is
correct because the supervisor sends one event line per user action. The app re-renders every
event, keeping state consistent without needing dirty tracking.

---

## 4. Event Model

### `event.rs`

```rust
// libs/vyoma-ui/src/event.rs

/// All events an App can receive.
#[derive(Debug, Clone)]
pub enum Event {
    /// A physical key was pressed.
    KeyDown { code: KeyCode },
    /// A physical key was released.
    KeyUp { code: KeyCode },
    /// A Unicode character from the keyboard (after IME composition).
    TextInput { ch: char },
    /// Mouse button pressed.
    MouseDown { x: i32, y: i32, button: MouseButton },
    /// Mouse button released.
    MouseUp   { x: i32, y: i32, button: MouseButton },
    /// Mouse moved (no button required).
    MouseMove { x: i32, y: i32 },
    /// Window resized.
    Resize { w: u32, h: u32 },
    /// Window exposed / needs repaint (supervisor sends this on focus restore).
    Paint,
    /// Supervisor-defined tick for cursor blink or animation (optional).
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton { Left, Middle, Right, Other(u8) }

/// Physical key codes — subset covering typical keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Left, Right, Up, Down,
    Home, End,
    PageUp, PageDown,
    F(u8),
    Unknown,
}

/// Parse a single stdin line into an Event, if recognised.
/// Returns None for unrecognised lines (e.g. IPC messages).
pub fn parse_line(line: &str) -> Option<Event> {
    if let Some(rest) = line.strip_prefix("VYOMA_INPUT:key:") {
        return Some(Event::KeyDown { code: parse_keycode(rest) });
    }
    if let Some(rest) = line.strip_prefix("VYOMA_INPUT:keyup:") {
        return Some(Event::KeyUp { code: parse_keycode(rest) });
    }
    if let Some(rest) = line.strip_prefix("VYOMA_INPUT:char:") {
        let ch = rest.chars().next()?;
        return Some(Event::TextInput { ch });
    }
    if let Some(rest) = line.strip_prefix("VYOMA_INPUT:mouse:") {
        return parse_mouse(rest);
    }
    if let Some(rest) = line.strip_prefix("VYOMA_DRAW:resize:") {
        let mut parts = rest.splitn(2, ',');
        let w: u32 = parts.next()?.parse().ok()?;
        let h: u32 = parts.next()?.parse().ok()?;
        return Some(Event::Resize { w, h });
    }
    if line == "VYOMA_INPUT:paint" {
        return Some(Event::Paint);
    }
    if line == "VYOMA_INPUT:tick" {
        return Some(Event::Tick);
    }
    None
}

fn parse_mouse(s: &str) -> Option<Event> {
    // Format: <x>,<y>,<btn>,<state>
    let mut p = s.splitn(4, ',');
    let x: i32 = p.next()?.parse().ok()?;
    let y: i32 = p.next()?.parse().ok()?;
    let btn_raw: u8 = p.next()?.parse().ok()?;
    let state = p.next()?.trim();
    let button = match btn_raw {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        n => MouseButton::Other(n),
    };
    match state {
        "down" => Some(Event::MouseDown { x, y, button }),
        "up"   => Some(Event::MouseUp   { x, y, button }),
        "move" => Some(Event::MouseMove { x, y }),
        _      => None,
    }
}

fn parse_keycode(s: &str) -> KeyCode {
    match s {
        "enter"     | "Return"    => KeyCode::Enter,
        "backspace" | "BackSpace" => KeyCode::Backspace,
        "delete"    | "Delete"    => KeyCode::Delete,
        "tab"       | "Tab"       => KeyCode::Tab,
        "escape"    | "Escape"    => KeyCode::Escape,
        "left"      | "Left"      => KeyCode::Left,
        "right"     | "Right"     => KeyCode::Right,
        "up"        | "Up"        => KeyCode::Up,
        "down"      | "Down"      => KeyCode::Down,
        "home"      | "Home"      => KeyCode::Home,
        "end"       | "End"       => KeyCode::End,
        "pageup"    | "Prior"     => KeyCode::PageUp,
        "pagedown"  | "Next"      => KeyCode::PageDown,
        s if s.starts_with('F') => {
            s[1..].parse::<u8>().map(KeyCode::F).unwrap_or(KeyCode::Unknown)
        }
        s if s.chars().count() == 1 => {
            KeyCode::Char(s.chars().next().unwrap())
        }
        _ => KeyCode::Unknown,
    }
}
```

---

## 5. DrawCtx

### `draw.rs`

```rust
// libs/vyoma-ui/src/draw.rs

/// A rectangle in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self { Self { x, y, w, h } }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y
            && px < self.x + self.w as i32
            && py < self.y + self.h as i32
    }
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.w as i32).min(other.x + other.w as i32);
        let y2 = (self.y + self.h as i32).min(other.y + other.h as i32);
        if x2 > x1 && y2 > y1 {
            Some(Rect::new(x1, y1, (x2 - x1) as u32, (y2 - y1) as u32))
        } else {
            None
        }
    }
}

/// Font size token matching VYOMA_DRAW size letters.
#[derive(Debug, Clone, Copy)]
pub enum FontSize { Small, Medium, Large }

impl FontSize {
    pub fn as_char(self) -> char {
        match self { Self::Small => 's', Self::Medium => 'm', Self::Large => 'l' }
    }
    /// Character cell width in pixels.
    pub fn cell_w(self) -> u32 {
        match self { Self::Small => 4, Self::Medium => 8, Self::Large => 16 }
    }
    /// Character cell height in pixels.
    pub fn cell_h(self) -> u32 {
        match self { Self::Small => 8, Self::Medium => 16, Self::Large => 32 }
    }
}

/// Wraps VYOMA_DRAW stdout commands. Accumulates commands in a String
/// buffer and emits them all at once on flush().
///
/// Clipping is enforced in software: draw calls are intersected with the
/// current clip stack before being emitted. A clipped-out draw emits nothing.
pub struct DrawCtx {
    buf: String,
    clip_stack: Vec<Rect>,
    /// Canvas dimensions (updated on Resize events).
    pub width:  u32,
    pub height: u32,
}

impl DrawCtx {
    pub fn new() -> Self {
        Self {
            buf: String::with_capacity(4096),
            clip_stack: Vec::new(),
            width:  960,
            height: 720,
        }
    }

    /// Update canvas size — call from App::on_event on Event::Resize.
    pub fn set_size(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
    }

    // ── Clipping ──────────────────────────────────────────────────────────

    pub fn push_clip(&mut self, r: Rect) {
        let effective = if let Some(top) = self.clip_stack.last() {
            top.intersect(&r).unwrap_or(Rect::new(0, 0, 0, 0))
        } else {
            r
        };
        self.clip_stack.push(effective);
    }

    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    fn current_clip(&self) -> Option<Rect> {
        self.clip_stack.last().copied()
    }

    /// Clip a rect against the current clip region. Returns None if fully clipped.
    fn clip_rect(&self, r: Rect) -> Option<Rect> {
        match self.current_clip() {
            Some(clip) => clip.intersect(&r),
            None       => Some(r),
        }
    }

    // ── Draw Primitives ───────────────────────────────────────────────────

    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        if w == 0 || h == 0 { return; }
        if let Some(r) = self.clip_rect(Rect::new(x, y, w, h)) {
            self.buf.push_str(&format!(
                "VYOMA_DRAW:fill_rect:{},{},{},{},{}\n",
                r.x, r.y, r.w, r.h, color
            ));
        }
    }

    pub fn draw_text(&mut self, x: i32, y: i32, color: u32, size: FontSize, text: &str) {
        if text.is_empty() { return; }
        // Text clipping: only emit if the text origin is within clip bounds.
        let clip_ok = match self.current_clip() {
            Some(clip) => x < clip.x + clip.w as i32
                && y < clip.y + clip.h as i32
                && x + (text.chars().count() as i32 * size.cell_w() as i32) > clip.x
                && y + size.cell_h() as i32 > clip.y,
            None => true,
        };
        if clip_ok {
            self.buf.push_str(&format!(
                "VYOMA_DRAW:draw_text:{},{},{},{},{}\n",
                x, y, color, size.as_char(), text
            ));
        }
    }

    /// Software-rasterised rounded rectangle.
    /// Draws: centre fill, four edge fills, four corner arcs via point sampling.
    /// Radius is clamped to min(w,h)/2.
    pub fn draw_rounded_rect(&mut self, x: i32, y: i32, w: u32, h: u32, radius: u32, color: u32) {
        if w == 0 || h == 0 { return; }
        let r = radius.min(w / 2).min(h / 2) as i32;
        // Centre fill (full width, inner height)
        self.fill_rect(x, y + r, w, h - 2 * r as u32, color);
        // Top and bottom strips (inset by radius on left/right)
        self.fill_rect(x + r, y, w - 2 * r as u32, r as u32, color);
        self.fill_rect(x + r, y + h as i32 - r, w - 2 * r as u32, r as u32, color);
        // Four corner arcs (1×1 pixel rows that approximate quarter-circles)
        let r_f = r as f32;
        for dy in 0..r {
            // How far from centre of the arc circle
            let dist = ((r - 1 - dy) as f32 + 0.5) / r_f;
            let arc_w = (r_f * (1.0 - (dist * dist).sqrt()).max(0.0)) as u32;
            // Top-left corner
            self.fill_rect(x + r - arc_w as i32, y + dy, arc_w, 1, color);
            // Top-right corner
            self.fill_rect(x + w as i32 - r, y + dy, arc_w, 1, color);
            // Bottom-left corner
            self.fill_rect(x + r - arc_w as i32, y + h as i32 - r + dy, arc_w, 1, color);
            // Bottom-right corner
            self.fill_rect(x + w as i32 - r, y + h as i32 - r + dy, arc_w, 1, color);
        }
    }

    /// Emit `VYOMA_DRAW:flush` and flush stdout. Must be called once per frame.
    pub fn flush(&mut self) {
        use std::io::Write;
        self.buf.push_str("VYOMA_DRAW:flush\n");
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let _ = out.write_all(self.buf.as_bytes());
        let _ = out.flush();
        self.buf.clear();
    }
}
```

---

## 6. Theme System

### `theme.rs`

```rust
// libs/vyoma-ui/src/theme.rs

/// All colors are packed RGBA u32: (R<<24)|(G<<16)|(B<<8)|A
#[derive(Debug, Clone, Copy)]
pub struct ColorTheme {
    pub background:   u32,  // window / root background
    pub surface:      u32,  // card, panel background
    pub surface_high: u32,  // elevated surface (tooltip, popover)
    pub text:         u32,  // primary text
    pub text_dim:     u32,  // secondary / hint text
    pub accent:       u32,  // interactive highlight (focus ring, selection)
    pub border:       u32,  // separator, widget border
    pub button_bg:    u32,  // default button fill
    pub button_hover: u32,  // button hover fill
    pub button_press: u32,  // button active/press fill
    pub danger:       u32,  // destructive actions (red)
    pub success:      u32,  // confirmation (green)
}

/// Catppuccin Mocha dark palette.
pub fn dark_theme() -> ColorTheme {
    ColorTheme {
        background:   0x1E1E2EFF,
        surface:      0x313244FF,
        surface_high: 0x45475AFF,
        text:         0xCDD6F4FF,
        text_dim:     0xBAC2DEFF,
        accent:       0x89B4FAFF,
        border:       0x45475AFF,
        button_bg:    0x313244FF,
        button_hover: 0x585B70FF,
        button_press: 0x6C7086FF,
        danger:       0xF38BA8FF,
        success:      0xA6E3A1FF,
    }
}

/// Light theme (Catppuccin Latte).
pub fn light_theme() -> ColorTheme {
    ColorTheme {
        background:   0xEFF1F5FF,
        surface:      0xE6E9EFFF,
        surface_high: 0xDCE0E8FF,
        text:         0x4C4F69FF,
        text_dim:     0x6C6F85FF,
        accent:       0x1E66F5FF,
        border:       0xCCD0DAFF,
        button_bg:    0xE6E9EFFF,
        button_hover: 0xDCE0E8FF,
        button_press: 0xCCD0DAFF,
        danger:       0xD20F39FF,
        success:      0x40A02BFF,
    }
}
```

---

## 7. Widget System

### `widget/mod.rs`

```rust
// libs/vyoma-ui/src/widget/mod.rs

pub mod button;
pub mod container;
pub mod label;
pub mod scroll;
pub mod text_input;

pub use button::Button;
pub use container::{Container, Direction};
pub use label::Label;
pub use scroll::ScrollView;
pub use text_input::TextInput;

use crate::draw::{DrawCtx, Rect};
use crate::event::Event;

/// Opaque widget identifier for focus tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetId(pub u32);

/// Core widget trait.
pub trait Widget {
    fn id(&self) -> WidgetId;

    /// Handle an event given the widget's bounding rect.
    /// Returns Some(event) to bubble unhandled events upward, None to consume.
    fn handle_event(&mut self, event: &Event, bounds: Rect) -> Option<Event>;

    /// Draw the widget into ctx within bounds.
    fn render(&self, ctx: &mut DrawCtx, bounds: Rect);

    /// Natural size hint for layout (width, height). 0 means "flexible".
    fn size_hint(&self) -> (u32, u32) { (0, 0) }
}

/// Tracks keyboard focus across widgets.
pub struct FocusManager {
    focused: Option<WidgetId>,
}

impl FocusManager {
    pub fn new() -> Self { Self { focused: None } }

    pub fn focused(&self) -> Option<WidgetId> { self.focused }

    pub fn set_focus(&mut self, id: WidgetId) { self.focused = Some(id); }

    pub fn clear(&mut self) { self.focused = None; }

    pub fn has_focus(&self, id: WidgetId) -> bool {
        self.focused == Some(id)
    }
}
```

### `widget/button.rs`

```rust
// libs/vyoma-ui/src/widget/button.rs

use crate::draw::{DrawCtx, FontSize, Rect};
use crate::event::{Event, MouseButton};
use crate::theme::ColorTheme;
use super::{Widget, WidgetId};

pub struct Button {
    id:      WidgetId,
    label:   String,
    theme:   ColorTheme,
    hovered: bool,
    pressed: bool,
    /// Fires when button is clicked.
    on_click: Option<Box<dyn FnMut()>>,
}

impl Button {
    pub fn new(id: WidgetId, label: impl Into<String>, theme: ColorTheme) -> Self {
        Self {
            id,
            label: label.into(),
            theme,
            hovered: false,
            pressed: false,
            on_click: None,
        }
    }

    pub fn on_click(mut self, cb: impl FnMut() + 'static) -> Self {
        self.on_click = Some(Box::new(cb));
        self
    }
}

impl Widget for Button {
    fn id(&self) -> WidgetId { self.id }

    fn size_hint(&self) -> (u32, u32) {
        let w = self.label.len() as u32 * 8 + 32; // 16px horizontal padding each side
        (w.max(80), 36)
    }

    fn handle_event(&mut self, event: &Event, bounds: Rect) -> Option<Event> {
        match event {
            Event::MouseMove { x, y } => {
                self.hovered = bounds.contains(*x, *y);
                None
            }
            Event::MouseDown { x, y, button: MouseButton::Left } => {
                if bounds.contains(*x, *y) {
                    self.pressed = true;
                    None // consumed
                } else {
                    Some(event.clone())
                }
            }
            Event::MouseUp { x, y, button: MouseButton::Left } => {
                if self.pressed && bounds.contains(*x, *y) {
                    self.pressed = false;
                    if let Some(cb) = &mut self.on_click { cb(); }
                    None
                } else {
                    self.pressed = false;
                    Some(event.clone())
                }
            }
            _ => Some(event.clone()),
        }
    }

    fn render(&self, ctx: &mut DrawCtx, bounds: Rect) {
        let bg = if self.pressed {
            self.theme.button_press
        } else if self.hovered {
            self.theme.button_hover
        } else {
            self.theme.button_bg
        };
        ctx.draw_rounded_rect(bounds.x, bounds.y, bounds.w, bounds.h, 6, bg);
        // Border
        ctx.draw_rounded_rect(bounds.x, bounds.y, bounds.w, bounds.h, 6, self.theme.border);
        // Centred label
        let text_w = self.label.len() as i32 * 8;
        let tx = bounds.x + (bounds.w as i32 - text_w) / 2;
        let ty = bounds.y + (bounds.h as i32 - 16) / 2;
        ctx.draw_text(tx, ty, self.theme.text, FontSize::Medium, &self.label);
    }
}
```

### `widget/label.rs`

```rust
// libs/vyoma-ui/src/widget/label.rs

use crate::draw::{DrawCtx, FontSize, Rect};
use crate::event::Event;
use crate::theme::ColorTheme;
use super::{Widget, WidgetId};

pub struct Label {
    id:    WidgetId,
    text:  String,
    color: u32,
    size:  FontSize,
}

impl Label {
    pub fn new(id: WidgetId, text: impl Into<String>, theme: &ColorTheme) -> Self {
        Self { id, text: text.into(), color: theme.text, size: FontSize::Medium }
    }
    pub fn with_size(mut self, size: FontSize) -> Self { self.size = size; self }
    pub fn with_color(mut self, color: u32) -> Self { self.color = color; self }
    pub fn set_text(&mut self, text: impl Into<String>) { self.text = text.into(); }
}

impl Widget for Label {
    fn id(&self) -> WidgetId { self.id }

    fn size_hint(&self) -> (u32, u32) {
        let w = self.text.len() as u32 * self.size.cell_w();
        (w, self.size.cell_h())
    }

    fn handle_event(&mut self, event: &Event, _bounds: Rect) -> Option<Event> {
        Some(event.clone()) // labels don't consume events
    }

    fn render(&self, ctx: &mut DrawCtx, bounds: Rect) {
        ctx.draw_text(bounds.x, bounds.y, self.color, self.size, &self.text);
    }
}
```

### `widget/text_input.rs`

```rust
// libs/vyoma-ui/src/widget/text_input.rs

use crate::draw::{DrawCtx, FontSize, Rect};
use crate::event::{Event, KeyCode, MouseButton};
use crate::theme::ColorTheme;
use super::{Widget, WidgetId};

pub struct TextInput {
    id:        WidgetId,
    buf:       String,
    cursor:    usize,   // byte offset in buf
    focused:   bool,
    blink_ctr: u32,     // incremented each render(); cursor visible when ctr%BLINK_FRAMES < BLINK_ON
    theme:     ColorTheme,
}

const BLINK_FRAMES: u32 = 60;
const BLINK_ON: u32     = 30;

impl TextInput {
    pub fn new(id: WidgetId, theme: ColorTheme) -> Self {
        Self {
            id,
            buf: String::new(),
            cursor: 0,
            focused: false,
            blink_ctr: 0,
            theme,
        }
    }

    pub fn value(&self) -> &str { &self.buf }

    pub fn set_focused(&mut self, f: bool) { self.focused = f; if !f { self.blink_ctr = 0; } }

    fn insert_char(&mut self, ch: char) {
        self.buf.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    fn delete_back(&mut self) {
        if self.cursor == 0 { return; }
        // Step back one char boundary
        let mut pos = self.cursor;
        while pos > 0 && !self.buf.is_char_boundary(pos - 1) { pos -= 1; }
        pos -= 1;
        while pos > 0 && !self.buf.is_char_boundary(pos) { pos -= 1; }
        let removed = self.cursor - pos;
        self.buf.drain(pos..self.cursor);
        self.cursor = pos;
        let _ = removed;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor == 0 { return; }
        let mut pos = self.cursor - 1;
        while pos > 0 && !self.buf.is_char_boundary(pos) { pos -= 1; }
        self.cursor = pos;
    }

    fn move_cursor_right(&mut self) {
        if self.cursor >= self.buf.len() { return; }
        let mut pos = self.cursor + 1;
        while pos < self.buf.len() && !self.buf.is_char_boundary(pos) { pos += 1; }
        self.cursor = pos;
    }
}

impl Widget for TextInput {
    fn id(&self) -> WidgetId { self.id }

    fn size_hint(&self) -> (u32, u32) { (200, 36) }

    fn handle_event(&mut self, event: &Event, bounds: Rect) -> Option<Event> {
        match event {
            Event::MouseDown { x, y, button: MouseButton::Left } => {
                self.focused = bounds.contains(*x, *y);
                if self.focused { None } else { Some(event.clone()) }
            }
            Event::TextInput { ch } if self.focused => {
                self.insert_char(*ch);
                None
            }
            Event::KeyDown { code } if self.focused => {
                match code {
                    KeyCode::Backspace => self.delete_back(),
                    KeyCode::Left      => self.move_cursor_left(),
                    KeyCode::Right     => self.move_cursor_right(),
                    KeyCode::Home      => self.cursor = 0,
                    KeyCode::End       => self.cursor = self.buf.len(),
                    _                  => return Some(event.clone()),
                }
                None
            }
            _ => Some(event.clone()),
        }
    }

    fn render(&self, ctx: &mut DrawCtx, bounds: Rect) {
        // Background
        ctx.draw_rounded_rect(bounds.x, bounds.y, bounds.w, bounds.h, 4, self.theme.surface);
        // Focus ring
        if self.focused {
            ctx.draw_rounded_rect(bounds.x - 1, bounds.y - 1, bounds.w + 2, bounds.h + 2, 5, self.theme.accent);
        }
        let text_x = bounds.x + 8;
        let text_y = bounds.y + (bounds.h as i32 - 16) / 2;
        ctx.draw_text(text_x, text_y, self.theme.text, FontSize::Medium, &self.buf);
        // Cursor blink — mutating blink_ctr in render() violates &self, so we use a Cell trick.
        // TextInput stores blink_ctr as regular u32 and render takes &self, so the App must
        // call tick() each frame to increment; see note in Blocking Issues.
        let show_cursor = self.focused && (self.blink_ctr % BLINK_FRAMES < BLINK_ON);
        if show_cursor {
            let char_count = self.buf[..self.cursor].chars().count();
            let cx = text_x + char_count as i32 * 8;
            ctx.fill_rect(cx, text_y, 2, 16, self.theme.accent);
        }
    }
}

impl TextInput {
    /// Call once per render pass to advance the blink animation.
    pub fn tick(&mut self) { self.blink_ctr = self.blink_ctr.wrapping_add(1); }
}
```

---

## 8. Layout Engine

### `layout/mod.rs`

```rust
// libs/vyoma-ui/src/layout/mod.rs

use crate::draw::Rect;

/// Axis-independent constraint for a layout pass.
#[derive(Debug, Clone, Copy)]
pub struct Constraint {
    pub min_w: u32,
    pub max_w: u32,
    pub min_h: u32,
    pub max_h: u32,
}

impl Constraint {
    pub fn tight(w: u32, h: u32) -> Self {
        Self { min_w: w, max_w: w, min_h: h, max_h: h }
    }
    pub fn loose(max_w: u32, max_h: u32) -> Self {
        Self { min_w: 0, max_w, min_h: 0, max_h }
    }
}

/// Axis for BoxLayout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis { Vertical, Horizontal }

/// Simple stacking layout — children placed sequentially along an axis
/// with uniform spacing.
pub struct BoxLayout {
    pub axis:    Axis,
    pub spacing: u32,
    pub padding: u32,
}

impl BoxLayout {
    pub fn vertical(spacing: u32)   -> Self { Self { axis: Axis::Vertical,   spacing, padding: 0 } }
    pub fn horizontal(spacing: u32) -> Self { Self { axis: Axis::Horizontal, spacing, padding: 0 } }
    pub fn with_padding(mut self, padding: u32) -> Self { self.padding = padding; self }

    /// Compute child Rects given parent bounds and a slice of natural (w, h) size hints.
    /// Children with size_hint 0 on the main axis get equal shares of remaining space.
    pub fn place(&self, parent: Rect, hints: &[(u32, u32)]) -> Vec<Rect> {
        let n = hints.len();
        if n == 0 { return vec![]; }
        let p = self.padding as i32;
        let sp = self.spacing as i32;
        let avail_w = (parent.w as i32 - 2 * p).max(0) as u32;
        let avail_h = (parent.h as i32 - 2 * p).max(0) as u32;

        let mut rects = Vec::with_capacity(n);
        match self.axis {
            Axis::Vertical => {
                let fixed: u32 = hints.iter().map(|h| h.1).filter(|&&h| h > 0).sum();
                let flex_count = hints.iter().filter(|h| h.1 == 0).count() as u32;
                let total_spacing = sp as u32 * (n as u32).saturating_sub(1);
                let flex_each = if flex_count > 0 {
                    avail_h.saturating_sub(fixed + total_spacing) / flex_count
                } else { 0 };
                let mut y = parent.y + p;
                for &(_, hint_h) in hints {
                    let h = if hint_h == 0 { flex_each } else { hint_h }.min(avail_h);
                    rects.push(Rect::new(parent.x + p, y, avail_w, h));
                    y += h as i32 + sp;
                }
            }
            Axis::Horizontal => {
                let fixed: u32 = hints.iter().map(|h| h.0).filter(|&&w| w > 0).sum();
                let flex_count = hints.iter().filter(|h| h.0 == 0).count() as u32;
                let total_spacing = sp as u32 * (n as u32).saturating_sub(1);
                let flex_each = if flex_count > 0 {
                    avail_w.saturating_sub(fixed + total_spacing) / flex_count
                } else { 0 };
                let mut x = parent.x + p;
                for &(hint_w, _) in hints {
                    let w = if hint_w == 0 { flex_each } else { hint_w }.min(avail_w);
                    rects.push(Rect::new(x, parent.y + p, w, avail_h));
                    x += w as i32 + sp;
                }
            }
        }
        rects
    }
}

/// Flex child descriptor.
#[derive(Debug, Clone, Copy)]
pub struct FlexChild {
    /// 0 = use natural size; >0 = grow proportionally.
    pub flex_grow: u32,
    /// Minimum size in the main axis (always respected).
    pub min_size:  u32,
}

/// Proportional layout along a single axis (analogous to CSS flexbox, 1D).
pub struct FlexLayout {
    pub axis:    Axis,
    pub spacing: u32,
    pub padding: u32,
}

impl FlexLayout {
    pub fn new(axis: Axis, spacing: u32) -> Self { Self { axis, spacing, padding: 0 } }

    pub fn place(&self, parent: Rect, children: &[FlexChild]) -> Vec<Rect> {
        let n = children.len();
        if n == 0 { return vec![]; }
        let p = self.padding as i32;
        let sp = self.spacing as i32;
        let avail_w = (parent.w as i32 - 2 * p).max(0) as u32;
        let avail_h = (parent.h as i32 - 2 * p).max(0) as u32;
        let total_spacing = sp as u32 * n.saturating_sub(1) as u32;
        let total_flex: u32 = children.iter().map(|c| c.flex_grow).sum();
        let fixed_sum: u32  = children.iter().filter(|c| c.flex_grow == 0).map(|c| c.min_size).sum();

        let main_avail = match self.axis {
            Axis::Vertical   => avail_h,
            Axis::Horizontal => avail_w,
        }.saturating_sub(total_spacing + fixed_sum);

        let unit = if total_flex > 0 { main_avail / total_flex } else { 0 };

        let mut rects = Vec::with_capacity(n);
        let (mut cx, mut cy) = (parent.x + p, parent.y + p);
        for child in children {
            let main = if child.flex_grow == 0 {
                child.min_size
            } else {
                unit * child.flex_grow
            };
            let r = match self.axis {
                Axis::Vertical => {
                    let r = Rect::new(cx, cy, avail_w, main);
                    cy += main as i32 + sp;
                    r
                }
                Axis::Horizontal => {
                    let r = Rect::new(cx, cy, main, avail_h);
                    cx += main as i32 + sp;
                    r
                }
            };
            rects.push(r);
        }
        rects
    }
}
```

---

## 9. Example: Counter App

```rust
// apps/counter/src/main.rs
use vyoma_ui::{
    draw::{DrawCtx, FontSize, Rect},
    event::Event,
    layout::BoxLayout,
    theme::dark_theme,
    widget::{Button, Label, Widget, WidgetId},
    App,
};

struct CounterApp {
    count:    i32,
    label:    Label,
    inc_btn:  Button,
    dec_btn:  Button,
    theme:    vyoma_ui::theme::ColorTheme,
}

impl CounterApp {
    fn new() -> Self {
        let theme = dark_theme();
        let mut label = Label::new(WidgetId(1), "Count: 0", &theme);
        Self {
            count: 0,
            label,
            inc_btn: Button::new(WidgetId(2), "  +  ", theme),
            dec_btn: Button::new(WidgetId(3), "  -  ", theme),
            theme,
        }
    }
}

impl App for CounterApp {
    fn on_event(&mut self, event: Event, _ctx: &mut DrawCtx) {
        // Distribute events to widgets using a temporary layout.
        let root = Rect::new(0, 0, 960, 720);
        let layout = BoxLayout::vertical(16).with_padding(40);
        let hints = [
            self.label.size_hint(),
            self.inc_btn.size_hint(),
            self.dec_btn.size_hint(),
        ];
        let rects = layout.place(root, &hints);

        // Snapshot whether buttons were clicked before mutating count.
        let mut inc = false;
        let mut dec = false;

        // We need to track clicks via a flag since on_click closures can't
        // mutate self directly. Use mutable state in on_event instead.
        if let Event::MouseDown { x, y, button: _ } = &event {
            if rects[1].contains(*x, *y) { inc = true; }
            if rects[2].contains(*x, *y) { dec = true; }
        }
        if inc { self.count += 1; }
        if dec { self.count -= 1; }
        if inc || dec {
            self.label.set_text(format!("Count: {}", self.count));
        }
    }

    fn render(&self, ctx: &mut DrawCtx) {
        // Clear background
        ctx.fill_rect(0, 0, 960, 720, self.theme.background);

        let root = Rect::new(0, 0, 960, 720);
        let layout = BoxLayout::vertical(16).with_padding(40);
        let hints = [
            self.label.size_hint(),
            self.inc_btn.size_hint(),
            self.dec_btn.size_hint(),
        ];
        let rects = layout.place(root, &hints);
        self.label.render(ctx, rects[0]);
        self.inc_btn.render(ctx, rects[1]);
        self.dec_btn.render(ctx, rects[2]);
    }
}

fn main() {
    vyoma_ui::run(CounterApp::new());
}
```

---

## 10. Blocking Issues

### B1 — WASM stdout is fully buffered, not line-buffered

**Problem**: In WASM/WASI, `println!` writes to a buffer that may not be flushed until the
process exits, causing the display to remain black until shutdown.

**Fix**: `DrawCtx::flush()` explicitly calls `stdout.lock().flush()` after writing all
accumulated draw commands:
```rust
// libs/vyoma-ui/src/draw.rs  (in flush())
let stdout = std::io::stdout();
let mut out = stdout.lock();
let _ = out.write_all(self.buf.as_bytes());
let _ = out.flush();   // CRITICAL — forces WASI fd_write syscall
self.buf.clear();
```
Without this, frames are batched in libc's buffer and only appear on process exit.

### B2 — WASM stdin `read_line` blocks forever on no input

**Problem**: `BufRead::read_line` issues a blocking WASI `fd_read` syscall. If the supervisor
never sends a line (e.g., idle app, no user input), the event loop stalls and the window never
repaints (no cursor blink, no animation).

**Fix**: Supervisor must send a periodic `VYOMA_INPUT:tick\n` line (e.g., every 16 ms / 60 Hz)
to wake the event loop. This is a protocol contract, not a library fix. The spec for the
supervisor's input router must include this tick emission. The `Event::Tick` variant already
handles this. For apps that need blink without tick support, `TextInput::tick()` is called in
`App::render()` instead (accepting coarser blink granularity tied to user input rate).

### B3 — Cursor blink requires mutable state during `render(&self)`

**Problem**: `Widget::render` takes `&self`, but advancing `blink_ctr` requires `&mut self`.
Using `Cell<u32>` breaks `no_std` compatibility assumptions; `RefCell` adds panic risk.

**Fix**: `TextInput` exposes an explicit `tick(&mut self)` method. The `App` implementor calls
`text_input.tick()` inside `on_event` (or at the start of `render` via a `mut self` overload).
The `Widget::render` contract remains `&self`, so blink state is only advanced externally:
```rust
// In App::on_event — advance blink on every event (including Tick)
self.text_input.tick();
```
This is simpler than interior mutability and aligns with the "render is pure" convention.

### B4 — draw command ordering: border drawn before fill erases border

**Problem**: `draw_rounded_rect` emits multiple `fill_rect` commands. If border and fill are
separate calls in the wrong order, the fill overwrites the border. Additionally, widgets that
draw a focus ring after the fill will paint over child widgets if z-order is not respected.

**Fix**: Enforce draw order inside each widget's `render()`:
1. Background fill (lowest)
2. Border / focus ring (on top of fill, below text)
3. Text / icon (topmost)

The supervisor compositor already renders draw commands in the order received. `DrawCtx` makes
no attempt to reorder, so widget `render()` implementations must issue calls in the correct
z-order. The Button and TextInput implementations above follow this order explicitly.

### B5 — Layout Rects recomputed independently in `on_event` and `render`

**Problem**: `on_event` computes layout to determine which widget was hit. `render` computes
layout again to position widgets for drawing. If the window size changes between these two
calls (impossible in single-threaded WASM but architecturally fragile), or if layout
computation is expensive, this double-computation is wasteful and can produce inconsistent
hit-test vs. render positions.

**Fix**: Cache the last layout result in the `App` struct. Recompute only on `Event::Resize`
or when widget count changes. Provide a `LayoutCache` helper:
```rust
pub struct LayoutCache {
    pub rects: Vec<Rect>,
    dirty: bool,
}
impl LayoutCache {
    pub fn new() -> Self { Self { rects: Vec::new(), dirty: true } }
    pub fn invalidate(&mut self) { self.dirty = true; }
    pub fn get_or_compute(
        &mut self,
        layout: &BoxLayout,
        parent: Rect,
        hints: &[(u32, u32)],
    ) -> &[Rect] {
        if self.dirty {
            self.rects = layout.place(parent, hints);
            self.dirty = false;
        }
        &self.rects
    }
}
```
Apps store `LayoutCache` as a field and call `cache.invalidate()` on `Event::Resize`.

---

## Summary

| Section | Deliverable |
|---------|-------------|
| Crate   | `libs/vyoma-ui/` — stdlib only, no external deps |
| Entry   | `vyoma_ui::run(app)` — blocking event loop |
| Widgets | Button, Label, TextInput, Container, ScrollView |
| Layout  | BoxLayout (V/H stack), FlexLayout (proportional) |
| Events  | Key, MouseDown/Up/Move, TextInput, Resize, Paint, Tick |
| Draw    | DrawCtx with clip stack, rounded rects, explicit flush |
| Theme   | Dark (Catppuccin Mocha) + Light (Catppuccin Latte) |
| Example | CounterApp — 60 lines, button increments label |
| B-Issues| 5 resolved: stdout flush, stdin block, blink, z-order, layout cache |
