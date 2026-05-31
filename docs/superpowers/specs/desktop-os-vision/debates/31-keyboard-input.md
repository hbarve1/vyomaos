# Architect Spec: Keyboard & Input Methods (Round 31)

**Subsystem**: Keyboard & Input Methods  
**macOS Analogue**: `IOHIDFamily` / `NSTextInputClient` / `NSEvent` keyboard handling  
**Depends on**: R25 (Mission Control — F3 intercept), R26 (Stage Manager — Escape/focus), R27 (Split View — Tab navigation), R28 (focus/FOCUSED_APP), R30 (AX/VoiceOver — Tab in AX tree), R33 (virtual keyboard), R34 (IME), R35 (global shortcuts)  
**Status**: DRAFT — Round 31

---

## 1. Overview

The keyboard subsystem converts raw TTY bytes and evdev key events into structured `KeyEvent`
values, applies a layered dispatch pipeline, and delivers events to the correct consumer:
a focused WASM app via its stdin IPC inbox, the IME interceptor, a global shortcut handler,
or a space-0 chrome consumer (MC, Stage Manager, Dock).

The design has four concerns:

1. **Physical event acquisition** — raw TTY escape sequences from `/dev/tty0` (current) plus
   evdev `EV_KEY` events from `/dev/input/eventN` (new, needed for full modifier tracking).
2. **Structured `KeyEvent`** — typed Rust struct replacing raw `&[u8]` pattern matching.
3. **Dispatch pipeline** — layered priority: global shortcuts > lock-level gates >
   IME intercept > focused app.
4. **Ancillary subsystems** — keymap/layout, compose/dead keys, repeat, VYOMA_KBD protocol,
   WIT interface, platform matrix.

---

## 2. Physical Event Acquisition

### 2.1 Current State

`input_keys::run_input_router` opens `/dev/tty0`, sets raw termios mode (no ICANON, no
ECHO, no ISIG), and reads bytes one or three at a time. The parser handles:

- `[0x1B, b]` → two-byte Alt sequence
- `[0x1B, 0x5B, b]` → three-byte CSI sequence (arrow keys)
- Single printable bytes `0x20..=0x7E`, CR/LF, DEL, ETX

This is adequate for interactive shell use but has three limitations:
1. No modifier state tracking (Shift, Ctrl, Caps Lock are inferred from byte values, not
   tracked as independent state).
2. No key release events — only key press (character delivery on press).
3. No scan codes — no way to distinguish e.g. left Shift from right Shift.

### 2.2 Dual-Source Model

For full keyboard fidelity, the supervisor reads from **two sources**:

| Source | Purpose | Thread |
|--------|---------|--------|
| `/dev/tty0` (termios raw) | Character delivery for printable keys, Ctrl combinations | `input-router` (existing) |
| `/dev/input/eventN` (evdev `EV_KEY`) | Modifier tracking, key release, scan codes, function keys | `kbd-evdev` (new) |

The two sources feed a shared `KeyEventQueue` (same `Arc<Mutex<Vec<KeyEvent>>>` pattern as
`AXEventQueue` from R30). The dispatch thread drains this queue and runs the priority
pipeline.

**Why not evdev-only?** The TTY path handles character encoding correctly for UTF-8
multibyte sequences (e.g., non-ASCII input from compose). The evdev path provides raw scan
codes but not composed characters. Both are needed.

**Why not TTY-only?** TTY does not deliver key-release events, does not distinguish left/
right modifier keys, and does not expose scan codes needed for layout remapping.

### 2.3 evdev Thread (`kbd-evdev`)

```rust
// supervisor/src/input_keys.rs  (new section, within 500-line limit)

/// Body of the kbd-evdev thread — /dev/input/eventN keyboard events → KeyEventQueue.
/// Tracks modifier state independently of TTY.
#[cfg(target_os = "linux")]
pub fn run_kbd_evdev(key_queue: KeyEventQueue, mod_state: ModStateRef) {
    let Some(mut dev) = open_kbd_device() else {
        log_info!(Subsystem::Input, None, "kbd-evdev: no keyboard evdev, skipping");
        return;
    };
    // EV_KEY = 1; value 1=press, 2=repeat, 0=release
    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() { break; }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        if ev_type != EV_KEY { continue; }
        let pressed = value == 1 || value == 2; // press or autorepeat
        let is_repeat = value == 2;
        // Update modifier state
        {
            let mut ms = mod_state.lock().unwrap();
            ms.update(code, pressed);
        }
        // Enqueue KeyEvent
        let ev = KeyEvent {
            scan_code:  code,
            key_code:   linux_scancode_to_keycode(code),
            modifiers:  mod_state.lock().unwrap().snapshot(),
            character:  None,   // set by TTY path for printable keys
            is_press:   pressed && !is_repeat,
            is_repeat,
            is_release: value == 0,
        };
        key_queue.lock().unwrap().push(ev);
    }
}
```

`open_kbd_device` scans `/dev/input/event0..15`, selects the first device with EV_KEY
capability bit set **and** at least one of KEY_A..KEY_Z in its key bitmap (to exclude
mice that also report EV_KEY for buttons).

---

## 3. KeyEvent Struct

```rust
// supervisor/src/input_keys.rs

/// A fully decoded keyboard event.
#[derive(Clone, Debug)]
pub struct KeyEvent {
    /// Linux evdev key code (e.g. KEY_A=30, KEY_LEFTSHIFT=42).
    /// 0 if derived from TTY-only path without evdev.
    pub scan_code:  u16,

    /// Logical key code (layout-independent; see KeyCode enum).
    pub key_code:   KeyCode,

    /// Active modifier set at the time of this event.
    pub modifiers:  Modifiers,

    /// Resolved Unicode character (after layout mapping + compose).
    /// None for non-character keys (F-keys, arrows, modifiers themselves).
    pub character:  Option<char>,

    /// True on initial key-down (not repeat).
    pub is_press:   bool,

    /// True on OS/supervisor-generated key repeat (held key).
    pub is_repeat:  bool,

    /// True on key-up.
    pub is_release: bool,
}

/// Layout-independent logical key codes (subset; extend as needed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    // Alphanumeric — layout-independent identity
    KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ,
    KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT,
    KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    Digit0, Digit1, Digit2, Digit3, Digit4,
    Digit5, Digit6, Digit7, Digit8, Digit9,
    // Control
    Enter, Escape, Backspace, Tab, Space, Delete,
    // Navigation
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Home, End, PageUp, PageDown,
    // Function
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    // Modifiers (for key-down/up tracking)
    LeftShift, RightShift, LeftCtrl, RightCtrl,
    LeftAlt, RightAlt, LeftMeta, RightMeta,
    CapsLock, NumLock, ScrollLock,
    // Punctuation (layout-dependent; character field carries actual glyph)
    Minus, Equal, BracketLeft, BracketRight, Backslash,
    Semicolon, Quote, Grave, Comma, Period, Slash,
    // Numpad
    Numpad0, Numpad1, Numpad2, Numpad3, Numpad4,
    Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
    NumpadEnter, NumpadPlus, NumpadMinus, NumpadStar, NumpadSlash,
    // Unknown
    Unknown(u16),
}
```

### 3.1 Modifiers Struct

```rust
// supervisor/src/input_keys.rs

bitflags::bitflags! {
    /// Active keyboard modifier flags.
    pub struct Modifiers: u16 {
        const SHIFT      = 0x0001;  // left or right shift
        const LEFT_SHIFT = 0x0002;
        const RIGHT_SHIFT= 0x0004;
        const CTRL       = 0x0010;  // left or right ctrl
        const LEFT_CTRL  = 0x0020;
        const RIGHT_CTRL = 0x0040;
        const ALT        = 0x0100;  // left alt (Option on macOS)
        const RIGHT_ALT  = 0x0200;  // AltGr (right alt) — separate from ALT
        const META       = 0x0400;  // left or right super/command
        const LEFT_META  = 0x0800;
        const RIGHT_META = 0x1000;
        const CAPS_LOCK  = 0x2000;  // toggle state, not held state
        const NUM_LOCK   = 0x4000;
    }
}

/// Mutable modifier tracking state; updated by evdev EV_KEY events.
pub struct ModifierState {
    raw: Modifiers,
    caps_lock_on: bool,
    num_lock_on:  bool,
}

impl ModifierState {
    pub fn update(&mut self, scan_code: u16, pressed: bool) {
        // Map scan_code to modifier flag; toggle CapsLock on press only
        match scan_code {
            KEY_LEFTSHIFT  => self.raw.set(Modifiers::LEFT_SHIFT  | Modifiers::SHIFT, pressed),
            KEY_RIGHTSHIFT => self.raw.set(Modifiers::RIGHT_SHIFT | Modifiers::SHIFT, pressed),
            KEY_LEFTCTRL   => self.raw.set(Modifiers::LEFT_CTRL   | Modifiers::CTRL,  pressed),
            KEY_RIGHTCTRL  => self.raw.set(Modifiers::RIGHT_CTRL  | Modifiers::CTRL,  pressed),
            KEY_LEFTALT    => self.raw.set(Modifiers::ALT,  pressed),
            KEY_RIGHTALT   => self.raw.set(Modifiers::RIGHT_ALT, pressed),
            KEY_LEFTMETA   => self.raw.set(Modifiers::LEFT_META  | Modifiers::META, pressed),
            KEY_RIGHTMETA  => self.raw.set(Modifiers::RIGHT_META | Modifiers::META, pressed),
            KEY_CAPSLOCK   => { if pressed { self.caps_lock_on = !self.caps_lock_on; } }
            KEY_NUMLOCK    => { if pressed { self.num_lock_on  = !self.num_lock_on;  } }
            _ => {}
        }
    }

    pub fn snapshot(&self) -> Modifiers {
        let mut m = self.raw;
        m.set(Modifiers::CAPS_LOCK, self.caps_lock_on);
        m.set(Modifiers::NUM_LOCK,  self.num_lock_on);
        m
    }
}

pub type ModStateRef    = Arc<Mutex<ModifierState>>;
pub type KeyEventQueue  = Arc<Mutex<Vec<KeyEvent>>>;
```

---

## 4. Key Event Dispatch Pipeline

### 4.1 Priority Order (normative)

```
Priority 1 (highest): INPUT_LOCK_LEVEL gate
Priority 2:           Global shortcuts (R35) — Cmd+Tab, Cmd+Space, etc.
Priority 3:           Space-0 chrome interceptors — F3=MC, Escape=FS exit, Cmd+M=minimize
Priority 4:           IME intercept (R34) — if IME app is registered and active
Priority 5 (lowest):  Focused app stdin delivery
```

Only one priority level may consume an event; once consumed the event is not passed further.

### 4.2 INPUT_LOCK_LEVEL

```rust
// supervisor/src/input_keys.rs

use std::sync::atomic::{AtomicU8, Ordering};

/// Controls which app receives key events.
/// Values:
///   0 = None        — normal dispatch to focused app
///   1 = MC          — Mission Control active; keys go to MC app (R25)
///   2 = ChromeConsent — modal consent dialog has focus; only Enter/Escape pass
///   3 = StageSwitch — Stage Manager switching; Tab/Shift+Tab cycle stages
///   4 = FsTransition — full-screen transition; all keys suppressed during animation
pub static INPUT_LOCK_LEVEL: AtomicU8 = AtomicU8::new(0);
```

When `INPUT_LOCK_LEVEL != 0`, `route_keyboard` checks the level before all other dispatch:

| Level | Keys that pass | Keys suppressed |
|-------|---------------|-----------------|
| 0 (None) | All | — |
| 1 (MC) | All keys → MC app inbox | No delivery to focused app |
| 2 (ChromeConsent) | Enter, Escape only → chrome handler | All others suppressed |
| 3 (StageSwitch) | Tab, Shift+Tab → stage-strip | All others suppressed |
| 4 (FsTransition) | — | All keys suppressed |

### 4.3 `route_keyboard` Entry Point

```rust
// supervisor/src/input_keys.rs

/// Central keyboard dispatch. Called by both the TTY input thread (for character events)
/// and the kbd-evdev thread (for non-character key events, via the KeyEventQueue drain loop).
pub fn route_keyboard(
    ev:           &KeyEvent,
    focused_app:  &str,
    inbox:        &Inbox,
    app_registry: &AppRegistry,
) {
    let lock_level = INPUT_LOCK_LEVEL.load(Ordering::Relaxed);

    // ── Priority 1: INPUT_LOCK_LEVEL gate ────────────────────────────────────
    match lock_level {
        4 => return,   // FsTransition: suppress all
        3 => {
            // StageSwitch: only Tab/Shift+Tab
            if ev.key_code == KeyCode::Tab {
                dispatch_stage_switch(ev, inbox);
            }
            return;
        }
        2 => {
            // ChromeConsent: only Enter/Escape
            if matches!(ev.key_code, KeyCode::Enter | KeyCode::Escape) {
                dispatch_chrome_consent(ev, inbox);
            }
            return;
        }
        1 => {
            // MC: all keys to MC app
            send_key_to_app("mission-control", ev, inbox);
            return;
        }
        _ => {}   // 0 = normal
    }

    // ── Priority 2: Global shortcuts (R35) ───────────────────────────────────
    if dispatch_global_shortcut(ev, inbox, app_registry) { return; }

    // ── Priority 3: Chrome interceptors ──────────────────────────────────────
    if dispatch_chrome_key(ev, inbox, app_registry) { return; }

    // ── Priority 4: IME intercept ─────────────────────────────────────────────
    if dispatch_ime(ev, focused_app, inbox, app_registry) { return; }

    // ── Priority 5: Focused app ───────────────────────────────────────────────
    send_key_to_app(focused_app, ev, inbox);
}
```

### 4.4 Chrome Interceptors (Priority 3)

These match the existing `classify_input_sequence` shortcuts and extend them:

| Key combination | Action | Handler |
|----------------|--------|---------|
| F3 | Open Mission Control | Set `INPUT_LOCK_LEVEL=1`; send focus to `mission-control` |
| Escape (in FS mode) | Exit full-screen | `INPUT_LOCK_LEVEL=0`; restore tiled layout |
| Alt+Tab | Cycle focus forward | `cycle_focus_forward` (existing) |
| Alt+Shift+Tab | Cycle focus backward | `cycle_focus_backward` (existing) |
| Alt+W | Close focused window | `SIGKILL` focused app (existing) |
| Alt+F | Snap/maximize | `compute_snap_layout` (existing) |
| Alt+? | Shortcut overlay | `show_shortcut_overlay` (existing) |
| Cmd+M (Meta+M) | Minimize focused window | `do_minimize` |
| Cmd+H (Meta+H) | Hide focused window | `do_hide` (future) |

`dispatch_chrome_key` returns `true` if it consumed the event, `false` otherwise.

---

## 5. Key Event Delivery to Apps

### 5.1 Current Delivery (PassThrough)

Currently, `run_input_router` sends raw strings to app stdin via `mpsc::Sender<String>`:

- Printable ASCII → `String::from(ch)`
- Enter/CR → `String::new()` (empty line)
- DEL/BS → `"\x7f"`
- Ctrl-C → `"\x03"`
- Arrow keys → `"\x1b[A"`, `"\x1b[B"` etc.

Apps receive these as raw stdin lines — the same format a terminal would deliver.

### 5.2 Structured VYOMA_KEY Protocol (new)

In addition to the raw PassThrough delivery, the supervisor also delivers structured
`VYOMA_KEY:` events to apps that opt in via `keyboard_events = true` in `vyoma.toml`.

```
VYOMA_KEY:<key_code>,<modifiers>,<char>,<flags>
```

Where:
- `<key_code>` — `KeyCode` variant name as lowercase string (e.g. `key_a`, `f3`, `arrow_up`)
- `<modifiers>` — hex bitmask matching `Modifiers` bitflags value
- `<char>` — Unicode code point as decimal, or `0` if no character
- `<flags>` — bitmask: bit0=press, bit1=repeat, bit2=release

Examples:
```
VYOMA_KEY:key_a,0x0001,97,1       # 'A' (Shift held), key press
VYOMA_KEY:f3,0x0000,0,1           # F3 press, no character
VYOMA_KEY:arrow_up,0x0010,0,1     # Ctrl+ArrowUp press
VYOMA_KEY:key_a,0x0000,97,4       # 'a' key release
```

This is an **additive** protocol: apps that ignore `VYOMA_KEY:` lines continue to work with
raw PassThrough delivery. Apps that want full modifier/release events declare
`keyboard_events = true`.

### 5.3 Manifest Opt-In

```toml
# apps/my-app/vyoma.toml
[capabilities]
stdio            = true
keyboard_events  = true   # receive VYOMA_KEY: structured events in addition to raw bytes
```

### 5.4 Delivery Implementation

```rust
// supervisor/src/input_keys.rs

fn send_key_to_app(app_name: &str, ev: &KeyEvent, inbox: &Inbox) {
    // Always deliver raw PassThrough bytes (existing behavior)
    if let Some(raw) = ev_to_raw_string(ev) {
        let inb = inbox.lock().unwrap();
        if let Some(tx) = inb.get(app_name) {
            let _ = tx.send(raw);
        }
    }
    // Additionally deliver VYOMA_KEY: line if app opts in
    // (registry lookup to check `keyboard_events` flag happens here)
    if app_has_keyboard_events(app_name) {
        let structured = format_vyoma_key(ev);
        let inb = inbox.lock().unwrap();
        if let Some(tx) = inb.get(app_name) {
            let _ = tx.send(structured);
        }
    }
}
```

---

## 6. Input Method Editor (IME) Hook Point

### 6.1 IME Registration

An IME is a WASM app with `ime = true` in its capabilities manifest. At most one IME may
be active at a time. The active IME name is stored in:

```rust
// supervisor/src/input_keys.rs
static ACTIVE_IME: OnceLock<Mutex<Option<String>>> = OnceLock::new();
fn active_ime() -> &'static Mutex<Option<String>> {
    ACTIVE_IME.get_or_init(|| Mutex::new(None))
}
```

An IME app registers by writing `@supervisor: ime_register` to stdout. The supervisor
sets `ACTIVE_IME` to the sender's app name and acknowledges with
`VYOMA_SYSTEM:ime_active:1`.

An IME deregisters by writing `@supervisor: ime_unregister`.

### 6.2 IME Interception

`dispatch_ime` at Priority 4 in `route_keyboard`:

```rust
fn dispatch_ime(
    ev:          &KeyEvent,
    focused_app: &str,
    inbox:       &Inbox,
    _registry:   &AppRegistry,
) -> bool {
    let ime_name = active_ime().lock().unwrap().clone();
    let Some(ime) = ime_name else { return false; };

    // Non-character keys (arrows, function keys, Escape) bypass IME
    if ev.character.is_none()
        && !matches!(ev.key_code, KeyCode::Backspace | KeyCode::Enter | KeyCode::Space) {
        return false;
    }

    // Forward key to IME app as VYOMA_KEY: structured event
    // IME returns composed text (or passes through) via @supervisor: ime_commit <text>
    // or @supervisor: ime_passthrough (to send the original event to focused app)
    let line = format_vyoma_key(ev);
    let inb = inbox.lock().unwrap();
    if let Some(tx) = inb.get(&ime) {
        // Piggyback the target app name so IME knows who to commit to
        let _ = tx.send(format!("VYOMA_IME_INPUT:{focused_app}:{line}"));
        return true; // event consumed by IME; IME decides final delivery
    }
    false
}
```

### 6.3 IME Commit Protocol

When the IME has composed a character sequence, it commits via IPC:

```
# IME commits composed text to the currently focused app
@supervisor: ime_commit <target_app> <utf8_text>

# IME passes the original key through unchanged
@supervisor: ime_passthrough <target_app> <vyoma_key_line>
```

The supervisor handler for `ime_commit` calls `send_reply(target_app, text, inbox)`.
The supervisor handler for `ime_passthrough` decodes the `vyoma_key_line` back into a
`KeyEvent` and delivers it at Priority 5 (focused app), bypassing the IME intercept layer
(to avoid re-entry).

### 6.4 IME State Isolation

The IME maintains its own compose buffer as internal WASM state. The supervisor does not
store IME composition state — this keeps the supervisor stateless with respect to IME,
matching the pattern used for AX state (R30 B1 fix).

---

## 7. Keyboard Layout and Keymap

### 7.1 Keymap Architecture

Keyboard layouts are defined as TOML files stored at `/data/keymaps/<name>.toml` (runtime,
user-installed) or baked into the initramfs at `/keymaps/<name>.toml` (built-in). The
supervisor loads the active layout at startup:

```rust
// supervisor/src/input_keys.rs

pub struct Keymap {
    pub name:    String,
    /// Maps (scan_code, shift, altgr) → Unicode code point.
    /// Key: (scan_code: u16, shift: bool, altgr: bool)
    pub map:     HashMap<(u16, bool, bool), char>,
    /// Dead key table: maps (dead_char, base_char) → composed_char.
    pub dead:    HashMap<(char, char), char>,
    /// Compose sequences: maps Vec<char> → char.
    pub compose: HashMap<Vec<char>, char>,
}
```

Built-in layouts shipped in initramfs:
- `qwerty-us` — QWERTY United States (default)
- `qwerty-gb` — QWERTY United Kingdom
- `azerty-fr` — AZERTY French
- `dvorak-us` — Dvorak
- `colemak`   — Colemak

### 7.2 Layout TOML Format

```toml
# /keymaps/qwerty-us.toml
name = "qwerty-us"

# Normal (no modifier): scan_code → unicode codepoint
[normal]
30 = 97   # a
31 = 115  # s
# ...

# Shifted
[shift]
30 = 65   # A
31 = 83   # S
# ...

# AltGr (right-alt)
[altgr]
18 = 8364  # AltGr+e → € on some layouts

# Dead key compositions: "dead_char,base_char" → composed
[dead]
"´,e" = 233   # dead acute + e → é
"´,a" = 225   # dead acute + a → á
"^,e" = 234   # dead circumflex + e → ê
"`,e" = 232   # dead grave + e → è

# Compose sequences: "seq" → composed
[compose]
"ae"  = 230   # a + e → æ
"oe"  = 248   # o + e → ø
```

### 7.3 Character Resolution

```rust
// supervisor/src/input_keys.rs

fn resolve_character(
    scan_code:  u16,
    modifiers:  Modifiers,
    keymap:     &Keymap,
    compose_st: &mut ComposeState,
) -> Option<char> {
    let shift  = modifiers.contains(Modifiers::SHIFT)
              ^ modifiers.contains(Modifiers::CAPS_LOCK);
    let altgr  = modifiers.contains(Modifiers::RIGHT_ALT);

    let raw_ch = keymap.map.get(&(scan_code, shift, altgr)).copied()?;

    // Check if this is a dead key (no character output yet; enters compose state)
    if keymap.dead.contains_key(&(raw_ch, '\0')) {
        compose_st.push_dead(raw_ch);
        return None;   // no character yet
    }

    // Apply dead key composition if one is pending
    if let Some(dead) = compose_st.pending_dead() {
        if let Some(&composed) = keymap.dead.get(&(dead, raw_ch)) {
            compose_st.clear();
            return Some(composed);
        } else {
            // No composition match: emit dead char, then current char
            compose_st.clear();
            return Some(raw_ch);  // caller handles the deferred dead char
        }
    }

    // Check multi-key compose sequences
    compose_st.push(raw_ch);
    if let Some(&composed) = keymap.compose.get(&compose_st.current_seq()) {
        compose_st.clear();
        return Some(composed);
    }
    if compose_st.is_prefix_of_any(&keymap.compose) {
        return None;   // partial compose sequence; wait for more keys
    }

    compose_st.clear();
    Some(raw_ch)
}
```

### 7.4 Layout Switching

Apps request a layout switch via IPC:

```
@supervisor: kbd_layout <name>
```

The supervisor loads `/keymaps/<name>.toml` (or `/data/keymaps/<name>.toml`),
replaces the active `Keymap` (stored in `static ACTIVE_KEYMAP: OnceLock<Mutex<Keymap>>`),
and broadcasts to all running apps:

```
VYOMA_KBD:layout_changed:<name>
```

This allows apps to update their on-screen keyboard labels or input hints.

---

## 8. Compose Key and Dead Keys

### 8.1 ComposeState

Compose state is **thread-local to the input-router thread** — identical rationale to the
AX parse state decision in R30 B1. It is not stored in `AppState` or any shared struct.

```rust
// supervisor/src/input_keys.rs

struct ComposeState {
    pending_dead: Option<char>,   // most recent dead key character (if any)
    seq:          Vec<char>,      // partial multi-key compose sequence
}

impl ComposeState {
    fn push_dead(&mut self, dead: char) {
        self.pending_dead = Some(dead);
    }
    fn pending_dead(&self) -> Option<char> { self.pending_dead }
    fn push(&mut self, ch: char) { self.seq.push(ch); }
    fn current_seq(&self) -> &[char] { &self.seq }
    fn is_prefix_of_any(&self, table: &HashMap<Vec<char>, char>) -> bool {
        table.keys().any(|k| k.starts_with(&self.seq) && k.len() > self.seq.len())
    }
    fn clear(&mut self) { self.pending_dead = None; self.seq.clear(); }
}
```

### 8.2 Compose Key Designation

The compose key is configured via `@supervisor: kbd_compose <key>` where `<key>` is a
`KeyCode` name (e.g. `right_alt`, `scroll_lock`, `right_ctrl`). Default: none.

When the user presses the designated compose key, `compose_state.start_compose()` is
called. Subsequent key presses accumulate in `compose_state.seq` and are checked against
the compose table. The compose key itself emits no character.

### 8.3 Dead Key Visual Indicator

When `compose_state.pending_dead.is_some()`, the supervisor sends:

```
VYOMA_KBD:compose_pending:<dead_char>
```

to the focused app, allowing it to show a visual indicator (e.g. display the dead
character with an underline in a text field) while the user types the base character.

When composition is resolved (success or cancelled), the supervisor sends:

```
VYOMA_KBD:compose_done
```

---

## 9. Key Repeat Handling

### 9.1 TTY-Level Repeat vs Supervisor-Level Repeat

On Linux, the kernel TTY layer generates key repeat events at the rate set by
`/sys/class/input/input*/delay` and `/sys/class/input/input*/rate` (default: 250ms delay,
33 Hz repeat rate). When the supervisor reads raw bytes from `/dev/tty0`, repeated
characters arrive at TTY-determined rate — no supervisor involvement needed for printable
character repeat.

For non-character keys (arrows, F-keys) and for apps that want `is_repeat=true` in their
`VYOMA_KEY:` events, the evdev source provides repeat events (`value=2` in `EV_KEY`).

### 9.2 Supervisor Repeat Configuration

The supervisor exposes repeat parameters as IPC commands (requires `shell = true`):

```
@supervisor: kbd_repeat_delay <ms>   # initial delay before repeat starts (default: 250)
@supervisor: kbd_repeat_rate  <hz>   # repeats per second (default: 33)
```

Implementation: write to `/sys/class/input/input<N>/delay` and `/sys/class/input/input<N>/rate`
via `std::fs::write`. Only the keyboard evdev device index `N` is modified (not the mouse).

### 9.3 Repeat in VYOMA_KEY Protocol

For apps using `VYOMA_KEY:` structured events, `is_repeat=true` (bit1 in flags) is set on
repeat events from the evdev source. Apps can use this to:
- Suppress repeat for toggle-style keyboard shortcuts (e.g., don't toggle bold 10 times
  when Cmd+B is held)
- Implement their own higher-level repeat logic

---

## 10. Special Key Filtering and Priority

### 10.1 Tab Key Priority

| Context | Tab behavior |
|---------|-------------|
| AX navigation active (R30) | Tab → AX focus advance (consumed at Priority 3) |
| IME active, composing | Tab → IME app (consumed at Priority 4) |
| Normal | Tab → focused app (Priority 5, raw byte `\x09`) |

AX navigation mode is entered when the user presses a designated AX navigation toggle
(e.g., Cmd+F5, following macOS VoiceOver). When active, `INPUT_LOCK_LEVEL` is set to a
future level 5 (AX) and Tab is intercepted at Priority 3 to advance AX focus in the
`voiceover` app.

### 10.2 Escape Key Priority

| Context | Escape behavior |
|---------|----------------|
| Full-screen active | Exit full-screen (Priority 3) |
| Dropdown/modal open | Dismiss dropdown (Priority 3) |
| IME composing | Cancel compose sequence (Priority 4) |
| Normal | Escape → focused app as `\x1b` (Priority 5) |

### 10.3 F3 Key Priority

F3 is always intercepted at Priority 3 to open/close Mission Control, regardless of
focused app. This mirrors macOS where F3 is a hardware key with fixed binding.

Apps wishing to use F3 for their own purposes must note this in their documentation.
No override mechanism is provided at this tier — R35 (global shortcuts) will define
the user-configurable shortcut override system.

---

## 11. VYOMA_KBD Protocol

`VYOMA_KBD:` events are pushed to all running apps with `keyboard_events = true`. They
are informational events (not key deliveries) about the keyboard subsystem state.

### 11.1 Events

| Event | Format | Meaning |
|-------|--------|---------|
| `layout_changed` | `VYOMA_KBD:layout_changed:<name>` | Active layout was switched |
| `compose_pending` | `VYOMA_KBD:compose_pending:<dead_char_codepoint>` | Dead key entered; awaiting base |
| `compose_done` | `VYOMA_KBD:compose_done` | Compose sequence resolved or cancelled |
| `repeat_rate` | `VYOMA_KBD:repeat_rate:<delay_ms>,<rate_hz>` | Repeat parameters changed |
| `caps_lock` | `VYOMA_KBD:caps_lock:<0|1>` | Caps Lock toggled |
| `num_lock` | `VYOMA_KBD:num_lock:<0|1>` | Num Lock toggled |

### 11.2 Global Key Monitor (R35 preview)

Apps with `global_key_monitor = true` capability (defined in R35) receive `VYOMA_KEY:`
events for **every** key press system-wide, not just when focused. This is how a
global hotkey daemon (e.g., a Spotlight-like launcher) can respond to Cmd+Space without
being the focused app. Delivery at Priority 2 (global shortcut path) happens before
focused-app delivery but after lock-level gates.

---

## 12. WIT Interface

```wit
// supervisor/src/wit/keyboard.wit
// Package: vyoma:keyboard@1.0.0

package vyoma:keyboard@1.0.0;

interface keyboard-query {
    /// Return the name of the currently active keyboard layout.
    get-active-layout: func() -> string;

    /// List all available keyboard layout names (built-in + user-installed).
    list-layouts: func() -> list<string>;

    /// Return current modifier state as a bitmask.
    get-modifiers: func() -> u16;

    /// Simulate a key press for testing (requires shell capability).
    /// Returns true if the event was dispatched (app running, inbox available).
    simulate-key: func(key-code: string, modifiers: u16, character: u32) -> bool;

    /// Request a layout switch (requires shell capability).
    set-layout: func(name: string) -> result<_, string>;

    /// Return the Unicode character that the given scan code + modifier combination
    /// maps to in the active layout. Returns 0 if non-character key.
    resolve-scancode: func(scan-code: u16, shift: bool, altgr: bool) -> u32;
}

world keyboard {
    import keyboard-query;
}
```

`init_linker_for_app` conditionally wires these WIT imports only for apps with
`keyboard_events = true` or `shell = true`:

```rust
// supervisor/src/runtime/wasmtime.rs (conditional WIT wiring)
if manifest.capabilities.keyboard_events {
    keyboard_query::add_to_linker(&mut linker, |s| s)?;
}
```

---

## 13. Virtual Keyboard (R33 Preview Hook Point)

On `mobile` and `tablet` platform profiles, no physical keyboard may be present.
The virtual keyboard is a WASM app in space-0 (z=65530, below voiceover at 65531).

The virtual keyboard app:
1. Renders its keys using `VYOMA_DRAW:` (covering the bottom portion of the screen)
2. Receives touch events via `VYOMA_INPUT:touch:`
3. Translates taps to `KeyEvent`s and submits them via IPC:
   ```
   @supervisor: kbd_inject <vyoma_key_line>
   ```

The supervisor `kbd_inject` handler (requires `shell = true`) calls `route_keyboard`
directly at Priority 5 (bypassing lock gates and IME — the virtual keyboard is itself
an IME-equivalent input source). This injection path must set `is_virtual = true` in
the `KeyEvent` to prevent infinite re-entry through the IME layer.

When a text field (AX node with role `TextField` or `TextArea`) receives focus, the
focused app sends:
```
@supervisor: kbd_request_virtual
```

The supervisor broadcasts to all `display = true` apps:
```
VYOMA_SYSTEM:virtual_kbd:show
```

The virtual keyboard app renders itself. On `desktop-full`, this event is a no-op
(no virtual keyboard app registered on that profile).

---

## 14. Platform Matrix

| Platform | Input Source | Key Repeat | IME | Layout Switch | Notes |
|----------|-------------|-----------|-----|--------------|-------|
| `desktop-full` | `/dev/tty0` + evdev | Kernel TTY | Optional WASM IME | Via IPC | Full feature set |
| `mobile` | Virtual keyboard app (R33) | Software (vkbd app) | Virtual keyboard IS the IME | Per-language vkbd layout | No physical kbd |
| `iot-edge` | `/dev/tty0` raw only | Kernel TTY | None | QWERTY-US fixed | Headless or serial console |
| `robotics-rt` | Serial terminal only | Kernel TTY | None | QWERTY-US fixed | SSH or serial |
| `server-headless` | `/dev/tty0` via SSH stdin or absent | Kernel TTY | None | QWERTY-US fixed | No GUI; keyboard is SSH session stdin |
| `mcu-minimal` | UART serial only | None | None | Fixed (ASCII only) | Extremely constrained; only printable ASCII |

### 14.1 Server-Headless: No TTY

On `server-headless`, `/dev/tty0` may not be accessible (no VGA console). The
`run_input_router` thread catches the `open` error, logs it, and exits gracefully. The
supervisor continues without keyboard input. Management is via the `mgmt-server` (port 9090)
or SSH.

### 14.2 Mobile: Virtual Keyboard Replaces Physical

On `mobile`, `run_input_router` and `run_kbd_evdev` still start (they will exit gracefully
if no device found). The virtual keyboard app is listed in boot.toml with `restart = always`
and `capabilities.display = true`, `capabilities.touch = true`, `capabilities.shell = true`.

---

## 15. File Layout

### 15.1 Modified Files

```
supervisor/src/input_keys.rs          (modified, stays ≤500 lines)
  - KeyEvent struct
  - Modifiers bitflags
  - ModifierState + ModStateRef + KeyEventQueue
  - route_keyboard (replaces classify_input_sequence dispatch)
  - dispatch_global_shortcut, dispatch_chrome_key, dispatch_ime, send_key_to_app
  - open_kbd_device, run_kbd_evdev
  - Keymap struct, ComposeState
  - active_ime static, INPUT_LOCK_LEVEL atomic
  - format_vyoma_key, ev_to_raw_string

supervisor/src/main.rs                (modified)
  - Spawn kbd-evdev thread
  - Initialize ModStateRef, KeyEventQueue, ACTIVE_KEYMAP

supervisor/src/manifest.rs            (modified)
  - Add keyboard_events: bool, global_key_monitor: bool to capabilities
  - Add ime: bool to capabilities

supervisor/src/ipc_handlers.rs        (modified)
  - kbd_layout, kbd_repeat_delay, kbd_repeat_rate, kbd_compose handlers
  - kbd_inject handler
  - ime_register, ime_unregister, ime_commit, ime_passthrough handlers
```

### 15.2 New Files

```
supervisor/src/keymap.rs              (new, ≤500 lines)
  - Keymap struct definition and TOML loader
  - linux_scancode_to_keycode mapping table
  - ACTIVE_KEYMAP static

supervisor/src/wit/keyboard.wit       (new)
  - keyboard-query WIT interface definition

base/keymaps/qwerty-us.toml          (new)
base/keymaps/qwerty-gb.toml          (new)
base/keymaps/azerty-fr.toml          (new)
base/keymaps/dvorak-us.toml          (new)
base/keymaps/colemak.toml            (new)

base/rootfs.sh                        (modified)
  - Copy base/keymaps/*.toml → /keymaps/ in initramfs

supervisor/tests/keyboard.rs          (new)
  - ModifierState update tests
  - Keymap character resolution tests
  - ComposeState dead-key tests
  - route_keyboard priority ordering tests
  - VYOMA_KEY format/parse round-trip tests
```

---

## 16. Threading Model

```
Thread: input-router  (existing)
  Reads /dev/tty0 raw bytes
  Acquires inbox lock briefly for send
  Maintains ComposeState as local var (no shared state)
  Calls route_keyboard for each KeyEvent constructed from TTY bytes

Thread: kbd-evdev  (new)
  Reads /dev/input/eventN evdev events
  Acquires ModStateRef lock to update modifier state
  Pushes KeyEvent into KeyEventQueue

Thread: input-router  (also drains KeyEventQueue)
  Drains KeyEventQueue at top of each byte-read loop iteration
  Calls route_keyboard for non-character evdev events

  Note: Only input-router calls route_keyboard → single-threaded dispatch
        No concurrent keyboard event processing → no stdin write races
```

The key invariant: **`route_keyboard` is called only from the `input-router` thread**.
The kbd-evdev thread only writes to `ModStateRef` and `KeyEventQueue`; it never calls
`route_keyboard` or `send_key_to_app`. This serializes all keyboard dispatch.

---

## 17. Lock-Safety Analysis

| Lock | Acquired by | Held during |
|------|------------|-------------|
| `INPUT_LOCK_LEVEL` | Any thread (atomic) | Single load/store, no hold |
| `ModStateRef` | kbd-evdev thread (write), input-router (read snapshot) | Update of one field; brief |
| `KeyEventQueue` | kbd-evdev (push), input-router (drain) | Single push or full drain; brief |
| `ACTIVE_IME` | input-router (read), ipc_handlers (write) | One clone; brief |
| `ACTIVE_KEYMAP` | input-router (read), ipc_handlers (write) | Keymap lookup; brief |
| `Inbox` | input-router (send), ipc_handlers (send) | One HashMap lookup + send; brief |

No lock is held while calling another lock-acquiring function. No ABBA deadlock risk.

---

## 18. Logging

All keyboard events log at `Subsystem::Input`:

```rust
log_info!(Subsystem::Input, Some(app_name), "key → {app_name}: {:?} mods={:?}", ev.key_code, ev.modifiers);
log_info!(Subsystem::Input, None, "IME registered: {ime_name}");
log_info!(Subsystem::Input, None, "layout switched: {name}");
log_warn!(Subsystem::Input, None, "compose sequence abandoned: {:?}", seq);
```

Key events at Priority 2–3 (global shortcuts, chrome interceptors) log at `Subsystem::Input`
with `Some(app_name)` = the consuming handler's logical name (e.g., `"chrome"`, `"mission-control"`).

---

## 19. Capability Summary

| Capability field | Type | Grants |
|-----------------|------|--------|
| `keyboard_events` | bool | Receive `VYOMA_KEY:` structured events + `VYOMA_KBD:` notifications |
| `global_key_monitor` | bool | Receive `VYOMA_KEY:` for every key press (R35; requires supervisor review) |
| `ime` | bool | Register as active IME; receive `VYOMA_IME_INPUT:` forwarded keys |

`shell = true` is required for `kbd_inject`, `kbd_layout`, `kbd_repeat_delay/rate`, `kbd_compose`.
