# FINAL Spec: Keyboard & Input Methods (Round 31)

**Subsystem**: Keyboard & Input Methods  
**macOS Analogue**: `IOHIDFamily` / `NSTextInputClient` / `NSEvent` keyboard handling  
**Depends on**: R25 (Mission Control — F3 intercept), R26 (Stage Manager — Escape/focus), R27 (Split View — Tab navigation), R28 (focus/FOCUSED_APP), R30 (AX/VoiceOver — Tab in AX tree), R33 (virtual keyboard), R34 (IME), R35 (global shortcuts)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

The keyboard subsystem converts raw TTY bytes and evdev key events into structured `KeyEvent`
values, applies a layered dispatch pipeline, and delivers events to the correct consumer:
a focused WASM app via its stdin IPC inbox, the IME interceptor, a global shortcut handler,
or a space-0 chrome consumer (MC, Stage Manager, Dock).

Four concerns:

1. **Physical event acquisition** — raw TTY escape sequences from `/dev/tty0` plus evdev
   `EV_KEY` events from `/dev/input/eventN`.
2. **Structured `KeyEvent`** — typed Rust struct replacing raw `&[u8]` pattern matching.
3. **Dispatch pipeline** — layered priority: lock-level gates → global shortcuts → chrome
   interceptors → IME intercept → focused app.
4. **Ancillary subsystems** — keymap/layout, compose/dead keys, repeat, VYOMA_KEY protocol,
   WIT interface, platform matrix.

---

## 2. Physical Event Acquisition

### 2.1 Dual-Source Model

| Source | Purpose | Thread |
|--------|---------|--------|
| `/dev/tty0` (termios raw) | Character delivery for printable keys, Ctrl combinations | `input-router` (existing) |
| `/dev/input/eventN` (evdev `EV_KEY`) | Modifier tracking, key release, scan codes, function keys | `kbd-evdev` (new) |

The two sources feed a shared `KeyEventQueue`:
```rust
pub type KeyEventQueue = Arc<Mutex<Vec<KeyEvent>>>;
pub type ModStateRef   = Arc<Mutex<ModifierState>>;
```

The `input-router` thread is the sole caller of `route_keyboard`. The `kbd-evdev` thread
only writes to `ModStateRef` and pushes into `KeyEventQueue` — it never calls `route_keyboard`.

### 2.2 evdev Thread (`kbd-evdev`)

```rust
// supervisor/src/input_keys.rs

#[cfg(target_os = "linux")]
pub fn run_kbd_evdev(key_queue: KeyEventQueue, mod_state: ModStateRef) {
    let Some(mut dev) = open_kbd_device() else {
        log_info!(Subsystem::Input, None, "kbd-evdev: no keyboard evdev, skipping");
        return;
    };
    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() { break; }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        if ev_type != EV_KEY { continue; }
        let is_repeat  = value == 2;
        let pressed    = value == 1 || is_repeat;
        {
            let mut ms = mod_state.lock().unwrap();
            ms.update(code, pressed);
        }
        let ev = KeyEvent {
            scan_code:       code,
            key_code:        linux_scancode_to_keycode(code),
            modifiers:       mod_state.lock().unwrap().snapshot(),
            character:       None,   // resolved in input-router drain loop (B3)
            is_press:        pressed && !is_repeat,
            is_repeat,
            is_release:      value == 0,
            from_virtual_kbd: false,
        };
        key_queue.lock().unwrap().push(ev);
    }
}
```

`open_kbd_device` scans `/dev/input/event0..15`, selects the first device with EV_KEY
capability **and** at least one of KEY_A..KEY_Z (to exclude mice with button EV_KEY).

### 2.3 TTY Read Timeout and Evdev Drain Ordering (B1 Fix)

The `input-router` thread must **not** use `VMIN=1, VTIME=0` (blocking read) when
`kbd-evdev` is active, because a blocking read on `/dev/tty0` prevents draining
`KeyEventQueue` until the next TTY byte arrives — causing evdev modifier events to be
processed after the character they modify, producing out-of-order delivery.

**Required termios configuration when `kbd-evdev` is active:**

```rust
// supervisor/src/input_keys.rs — run_input_router setup

let mut t = termios::Termios::from_fd(tty_fd).unwrap();
termios::cfmakeraw(&mut t);
// VMIN=0, VTIME=1: return immediately or after 100ms (10th-of-second units)
t.c_cc[libc::VTIME as usize] = 1;   // 100ms timeout
t.c_cc[libc::VMIN  as usize] = 0;   // do not block waiting for minimum bytes
termios::tcsetattr(tty_fd, termios::TCSANOW, &t).unwrap();
```

With `VMIN=0, VTIME=1`, `tty.read()` returns 0 bytes after 100ms if no key was pressed,
allowing the loop to drain `KeyEventQueue` on every timeout tick.

**Drain-before-process invariant**: The drain loop **always** runs before processing the TTY
byte — even when a TTY byte arrives in the same loop iteration. This ensures evdev modifier
events (e.g., Shift press) are processed **before** the character they modify.

```rust
// supervisor/src/input_keys.rs — run_input_router main loop

loop {
    // Phase A: drain KeyEventQueue FIRST — resolves characters, then dispatches
    let evdev_events: Vec<KeyEvent> = {
        let mut q = key_queue.lock().unwrap();
        q.drain(..).collect()
    };
    for mut ev in evdev_events {
        // B3: Resolve character in input-router thread (sole owner of compose_state)
        if ev.is_press || ev.is_repeat {
            ev.character = resolve_character(
                ev.scan_code,
                ev.modifiers,
                &active_keymap().lock().unwrap(),
                &mut compose_state,
            );
        }
        route_keyboard(&ev, &focused_app(), inbox, app_registry);
    }

    // Phase B: blocking read from /dev/tty0 (returns after VTIME=100ms if no input)
    let mut buf = [0u8; 4];
    let n = unsafe { libc::read(tty_fd, buf.as_mut_ptr() as _, buf.len()) };
    if n <= 0 { continue; }  // timeout or error; loop back to drain

    // Phase C: construct KeyEvent from TTY bytes; character already known
    let mut tty_ev = parse_tty_bytes(&buf[..n as usize], &mod_state.lock().unwrap().snapshot());

    // B4: Infer KeyCode for TTY events that lack scan_code
    if tty_ev.scan_code == 0 {
        if let Some(ch) = tty_ev.character {
            tty_ev.key_code = active_keymap().lock().unwrap().char_to_keycode(ch);
        }
    }

    route_keyboard(&tty_ev, &focused_app(), inbox, app_registry);
}
```

The 100ms polling overhead is negligible on desktop and mobile. On `server-headless` where
`kbd-evdev` is absent, `VMIN=1, VTIME=0` (blocking) may be used instead to avoid
unnecessary wakeups.

---

## 3. KeyEvent Struct

```rust
// supervisor/src/input_keys.rs

#[derive(Clone, Debug)]
pub struct KeyEvent {
    /// Linux evdev key code (e.g. KEY_A=30, KEY_LEFTSHIFT=42).
    /// 0 if derived from TTY-only path without evdev.
    pub scan_code:        u16,

    /// Logical key code (layout-independent; see KeyCode enum).
    pub key_code:         KeyCode,

    /// Active modifier set at the time of this event.
    pub modifiers:        Modifiers,

    /// Resolved Unicode character (after layout mapping + compose).
    /// None for non-character keys (F-keys, arrows, modifiers themselves).
    pub character:        Option<char>,

    /// True on initial key-down (not repeat).
    pub is_press:         bool,

    /// True on OS/supervisor-generated key repeat (held key).
    pub is_repeat:        bool,

    /// True on key-up.
    pub is_release:       bool,

    /// True for events injected by the virtual keyboard via kbd_inject.
    /// Causes route_keyboard to skip the IME intercept layer (Priority 4)
    /// while still respecting all INPUT_LOCK_LEVEL gates (Priority 1).
    pub from_virtual_kbd: bool,   // B5
}
```

### 3.1 KeyCode Enum

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ,
    KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT,
    KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    Digit0, Digit1, Digit2, Digit3, Digit4,
    Digit5, Digit6, Digit7, Digit8, Digit9,
    Enter, Escape, Backspace, Tab, Space, Delete,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Home, End, PageUp, PageDown,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    LeftShift, RightShift, LeftCtrl, RightCtrl,
    LeftAlt, RightAlt, LeftMeta, RightMeta,
    CapsLock, NumLock, ScrollLock,
    Minus, Equal, BracketLeft, BracketRight, Backslash,
    Semicolon, Quote, Grave, Comma, Period, Slash,
    Numpad0, Numpad1, Numpad2, Numpad3, Numpad4,
    Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
    NumpadEnter, NumpadPlus, NumpadMinus, NumpadStar, NumpadSlash,
    Unknown(u16),
}
```

### 3.2 Modifiers Struct

```rust
bitflags::bitflags! {
    pub struct Modifiers: u16 {
        const SHIFT       = 0x0001;
        const LEFT_SHIFT  = 0x0002;
        const RIGHT_SHIFT = 0x0004;
        const CTRL        = 0x0010;
        const LEFT_CTRL   = 0x0020;
        const RIGHT_CTRL  = 0x0040;
        const ALT         = 0x0100;
        const RIGHT_ALT   = 0x0200;  // AltGr — distinct from ALT
        const META        = 0x0400;
        const LEFT_META   = 0x0800;
        const RIGHT_META  = 0x1000;
        const CAPS_LOCK   = 0x2000;
        const NUM_LOCK    = 0x4000;
    }
}

pub struct ModifierState {
    raw:          Modifiers,
    caps_lock_on: bool,
    num_lock_on:  bool,
}

impl ModifierState {
    pub fn update(&mut self, scan_code: u16, pressed: bool) {
        match scan_code {
            KEY_LEFTSHIFT  => self.raw.set(Modifiers::LEFT_SHIFT  | Modifiers::SHIFT, pressed),
            KEY_RIGHTSHIFT => self.raw.set(Modifiers::RIGHT_SHIFT | Modifiers::SHIFT, pressed),
            KEY_LEFTCTRL   => self.raw.set(Modifiers::LEFT_CTRL   | Modifiers::CTRL,  pressed),
            KEY_RIGHTCTRL  => self.raw.set(Modifiers::RIGHT_CTRL  | Modifiers::CTRL,  pressed),
            KEY_LEFTALT    => self.raw.set(Modifiers::ALT,  pressed),
            KEY_RIGHTALT   => self.raw.set(Modifiers::RIGHT_ALT,  pressed),
            KEY_LEFTMETA   => self.raw.set(Modifiers::LEFT_META   | Modifiers::META, pressed),
            KEY_RIGHTMETA  => self.raw.set(Modifiers::RIGHT_META  | Modifiers::META, pressed),
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
```

---

## 4. Key Event Dispatch Pipeline

### 4.1 Priority Order (normative)

```
Priority 1 (highest): INPUT_LOCK_LEVEL gate
Priority 2:           Global shortcuts (R35) — Cmd+Tab, Cmd+Space, etc.
Priority 3:           Space-0 chrome interceptors — F3=MC, Escape=FS exit, Cmd+M=minimize
Priority 4:           IME intercept (R34) — skipped for from_virtual_kbd events (B5)
Priority 5 (lowest):  Focused app stdin delivery
```

Only one priority level may consume an event; once consumed the event is not passed further.

### 4.2 INPUT_LOCK_LEVEL

```rust
/// 0=None, 1=MissionControl, 2=ChromeConsent, 3=StageSwitch, 4=FsTransition
pub static INPUT_LOCK_LEVEL: AtomicU8 = AtomicU8::new(0);
```

| Level | Keys that pass | Keys suppressed |
|-------|---------------|-----------------|
| 0 (None) | All | — |
| 1 (MC) | All keys → MC app inbox | No delivery to focused app |
| 2 (ChromeConsent) | Enter, Escape only → chrome handler | All others suppressed |
| 3 (StageSwitch) | Tab, Shift+Tab → stage-strip | All others suppressed |
| 4 (FsTransition) | — | All keys suppressed |

**B5 clarification**: `INPUT_LOCK_LEVEL` gates apply to **all** keyboard events including
those from the virtual keyboard (`from_virtual_kbd = true`). Virtual keyboard events are
IME-equivalent only — they bypass the IME intercept (Priority 4), not the lock gates.

### 4.3 `route_keyboard` Entry Point

```rust
// supervisor/src/input_keys.rs

pub fn route_keyboard(
    ev:           &KeyEvent,
    focused_app:  &str,
    inbox:        &Inbox,
    app_registry: &AppRegistry,
) {
    let lock_level = INPUT_LOCK_LEVEL.load(Ordering::Relaxed);

    // ── Priority 1: INPUT_LOCK_LEVEL gate — applies to ALL events including virtual kbd ──
    match lock_level {
        4 => return,   // FsTransition: suppress all
        3 => {
            if ev.key_code == KeyCode::Tab {
                dispatch_stage_switch(ev, inbox);
            }
            return;
        }
        2 => {
            if matches!(ev.key_code, KeyCode::Enter | KeyCode::Escape) {
                dispatch_chrome_consent(ev, inbox);
            }
            return;
        }
        1 => {
            send_key_to_app("mission-control", ev, inbox);
            return;
        }
        _ => {}
    }

    // ── Priority 2: Global shortcuts (R35) ───────────────────────────────────
    if dispatch_global_shortcut(ev, inbox, app_registry) { return; }

    // ── Priority 3: Chrome interceptors ──────────────────────────────────────
    if dispatch_chrome_key(ev, inbox, app_registry) { return; }

    // ── Priority 4: IME intercept — skipped for virtual keyboard events (B5) ─
    if !ev.from_virtual_kbd {
        if dispatch_ime(ev, focused_app, inbox, app_registry) { return; }
    }

    // ── Priority 5: Focused app ───────────────────────────────────────────────
    send_key_to_app(focused_app, ev, inbox);
}
```

### 4.4 Chrome Interceptors (Priority 3)

| Key combination | Action | Handler |
|----------------|--------|---------|
| F3 | Open Mission Control | Set `INPUT_LOCK_LEVEL=1`; send focus to `mission-control` |
| Escape (in FS mode) | Exit full-screen | `INPUT_LOCK_LEVEL=0`; restore tiled layout |
| Alt+Tab | Cycle focus forward | `cycle_focus_forward` |
| Alt+Shift+Tab | Cycle focus backward | `cycle_focus_backward` |
| Alt+W | Close focused window | `SIGKILL` focused app |
| Alt+F | Snap/maximize | `compute_snap_layout` |
| Cmd+M (Meta+M) | Minimize focused window | `do_minimize` |

---

## 5. Key Event Delivery to Apps

### 5.1 Raw PassThrough (existing)

`ev_to_raw_string(ev)` converts `KeyEvent` to the raw byte string an app receives on stdin:
printable ASCII as `String::from(ch)`, Enter as `""`, DEL as `"\x7f"`, etc. TTY-path events
(`scan_code=0`, `character=Some(ch)`) are the primary source of raw PassThrough.

### 5.2 TTY KeyCode Inference (B4 Fix)

TTY-derived events lack a scan code (`scan_code=0`), so `linux_scancode_to_keycode(0)`
returns `KeyCode::Unknown(0)`. This breaks romanization IMEs (e.g., Japanese) that dispatch
on `key_code` (e.g., `KeyCode::KeyK` + `KeyCode::KeyA` → 'か').

**Fix**: After constructing the TTY `KeyEvent`, infer `key_code` from `character` via
reverse keymap lookup:

```rust
// supervisor/src/keymap.rs

impl Keymap {
    /// Reverse lookup: given a character, return the KeyCode most likely to produce it.
    /// Used to infer KeyCode for TTY-derived events lacking scan codes.
    /// Limitation: when the same char appears under multiple scan codes (e.g. on AltGr
    /// and Shift maps), the first match in normal-then-shift order is returned.
    pub fn char_to_keycode(&self, ch: char) -> KeyCode {
        // Search normal (shift=false, altgr=false) first
        for (&(scan, shift, altgr), &mapped) in &self.map {
            if mapped == ch && !shift && !altgr {
                return linux_scancode_to_keycode(scan);
            }
        }
        // Then shifted
        for (&(scan, shift, altgr), &mapped) in &self.map {
            if mapped == ch && shift && !altgr {
                return linux_scancode_to_keycode(scan);
            }
        }
        KeyCode::Unknown(0)
    }
}
```

Called in the input-router drain loop for TTY events:

```rust
// supervisor/src/input_keys.rs — in run_input_router Phase C

if tty_ev.scan_code == 0 {
    if let Some(ch) = tty_ev.character {
        tty_ev.key_code = active_keymap().lock().unwrap().char_to_keycode(ch);
    }
}
```

Add to the spec: the reverse lookup is best-effort (first match wins). IMEs that require
exact scan codes should use `keyboard_events = true` and a physical keyboard; TTY inference
is a fallback for headless/serial contexts.

### 5.3 Structured VYOMA_KEY Protocol

Apps opting in with `keyboard_events = true` in `vyoma.toml` receive **both** raw
PassThrough delivery and structured `VYOMA_KEY:` events:

```
VYOMA_KEY:<key_code>,<modifiers_hex>,<char_codepoint>,<flags>
```

Where:
- `<key_code>` — `KeyCode` variant as lowercase string (`key_a`, `f3`, `arrow_up`)
- `<modifiers_hex>` — hex bitmask matching `Modifiers` bitflags
- `<char_codepoint>` — Unicode code point as decimal, or `0` if no character
- `<flags>` — bitmask: bit0=press, bit1=repeat, bit2=release

Examples:
```
VYOMA_KEY:key_a,0x0001,65,1       # 'A' (Shift held), key press
VYOMA_KEY:f3,0x0000,0,1           # F3 press, no character
VYOMA_KEY:arrow_up,0x0010,0,1     # Ctrl+ArrowUp press
VYOMA_KEY:key_a,0x0000,97,4       # 'a' key release
```

### 5.4 Delivery Implementation

```rust
fn send_key_to_app(app_name: &str, ev: &KeyEvent, inbox: &Inbox) {
    if let Some(raw) = ev_to_raw_string(ev) {
        let inb = inbox.lock().unwrap();
        if let Some(tx) = inb.get(app_name) { let _ = tx.send(raw); }
    }
    if app_has_keyboard_events(app_name) {
        let structured = format_vyoma_key(ev);
        let inb = inbox.lock().unwrap();
        if let Some(tx) = inb.get(app_name) { let _ = tx.send(structured); }
    }
}
```

---

## 6. Input Method Editor (IME) Hook Point

### 6.1 IME Registration

At most one IME may be active. The active IME name is stored in:

```rust
static ACTIVE_IME: OnceLock<Mutex<Option<String>>> = OnceLock::new();
pub fn active_ime() -> &'static Mutex<Option<String>> {
    ACTIVE_IME.get_or_init(|| Mutex::new(None))
}
```

An IME app (capability `ime = true`) registers via `@supervisor: ime_register`. The
supervisor sets `ACTIVE_IME` to the sender's app name and acknowledges with
`VYOMA_SYSTEM:ime_active:1`.

### 6.2 IME Interception

```rust
fn dispatch_ime(
    ev:          &KeyEvent,
    focused_app: &str,
    inbox:       &Inbox,
    _registry:   &AppRegistry,
) -> bool {
    // from_virtual_kbd events must not enter this function (checked in route_keyboard)
    let ime_name = active_ime().lock().unwrap().clone();
    let Some(ime) = ime_name else { return false; };

    // Non-character keys (arrows, function keys, Escape) bypass IME
    if ev.character.is_none()
        && !matches!(ev.key_code, KeyCode::Backspace | KeyCode::Enter | KeyCode::Space) {
        return false;
    }

    let line = format_vyoma_key(ev);
    let inb = inbox.lock().unwrap();
    if let Some(tx) = inb.get(&ime) {
        let _ = tx.send(format!("VYOMA_IME_INPUT:{focused_app}:{line}"));
        return true;
    }
    false
}
```

### 6.3 IME Commit Protocol

```
@supervisor: ime_commit <target_app> <utf8_text>
@supervisor: ime_passthrough <target_app> <vyoma_key_line>
```

### 6.4 IME Security: Commit Authorization (B2 Fix)

Any app with `stdio = true` can write `@supervisor:` commands, including `ime_commit`.
Without authorization, a non-IME app could inject arbitrary text into any app's stdin.

**The `ime_commit` and `ime_passthrough` handlers must verify the sender is the active IME:**

```rust
// supervisor/src/ipc_handlers.rs

"ime_commit" => {
    let active = crate::input_keys::active_ime().lock().unwrap().clone();
    if active.as_deref() != Some(sender) {
        log_warn!(Subsystem::Input, Some(sender),
            "ime_commit rejected: sender {sender:?} is not active IME (active: {active:?})");
        return;
    }
    // parse target_app and text, then deliver
    let parts: Vec<&str> = rest.splitn(2, ' ').collect();
    let (target_app, text) = match parts[..] {
        [t, txt] => (t, txt),
        _ => return,
    };
    let inb = inbox.lock().unwrap();
    if let Some(tx) = inb.get(target_app) { let _ = tx.send(text.to_string()); }
}

"ime_passthrough" => {
    let active = crate::input_keys::active_ime().lock().unwrap().clone();
    if active.as_deref() != Some(sender) {
        log_warn!(Subsystem::Input, Some(sender), "ime_passthrough rejected: not active IME");
        return;
    }
    // decode vyoma_key_line back into KeyEvent and re-dispatch at Priority 5
    let Ok(mut ev) = parse_vyoma_key(rest) else { return };
    ev.from_virtual_kbd = true;  // prevents IME re-intercept on passthrough
    let focused = focused_app();
    route_keyboard(&ev, &focused, inbox, app_registry);
}
```

Test: `ime_commit_from_non_ime_app_is_rejected` — verifies that an app without `ime = true`
cannot deliver text to another app via `ime_commit`.

---

## 7. Keyboard Layout and Keymap

### 7.1 Keymap Architecture

```rust
// supervisor/src/keymap.rs

pub struct Keymap {
    pub name:    String,
    /// (scan_code, shift, altgr) → Unicode char
    pub map:     HashMap<(u16, bool, bool), char>,
    /// (dead_char, base_char) → composed_char
    pub dead:    HashMap<(char, char), char>,
    /// compose sequences: key sequence → composed char
    pub compose: HashMap<Vec<char>, char>,
}

pub static ACTIVE_KEYMAP: OnceLock<Mutex<Keymap>> = OnceLock::new();
pub fn active_keymap() -> &'static Mutex<Keymap> {
    ACTIVE_KEYMAP.get_or_init(|| Mutex::new(Keymap::load("qwerty-us")))
}
```

Built-in layouts in initramfs at `/keymaps/`:
`qwerty-us`, `qwerty-gb`, `azerty-fr`, `dvorak-us`, `colemak`

### 7.2 Layout TOML Format

```toml
name = "qwerty-us"

[normal]
30 = 97   # a
31 = 115  # s

[shift]
30 = 65   # A
31 = 83   # S

[altgr]
18 = 8364  # AltGr+e → €

[dead]
"´,e" = 233   # é
"´,a" = 225   # á
"^,e" = 234   # ê
"`,e" = 232   # è

[compose]
"ae"  = 230   # æ
"oe"  = 248   # ø
```

### 7.3 Character Resolution in the Drain Loop (B3 Fix)

`ComposeState` is thread-local to the `input-router` thread (same rationale as `AXParseState`
in R30 B1). It must never be stored in `AppState` or any shared struct.

`resolve_character` is called **exclusively in the `input-router` thread's drain loop** for
evdev events, and for TTY events before `route_keyboard`. It is **never** called in the
`kbd-evdev` thread.

```rust
// supervisor/src/input_keys.rs

fn resolve_character(
    scan_code:  u16,
    modifiers:  Modifiers,
    keymap:     &Keymap,
    compose_st: &mut ComposeState,
) -> Option<char> {
    let shift = modifiers.contains(Modifiers::SHIFT)
             ^ modifiers.contains(Modifiers::CAPS_LOCK);
    let altgr = modifiers.contains(Modifiers::RIGHT_ALT);

    let raw_ch = keymap.map.get(&(scan_code, shift, altgr)).copied()?;

    if keymap.dead.contains_key(&(raw_ch, '\0')) {
        compose_st.push_dead(raw_ch);
        return None;
    }
    if let Some(dead) = compose_st.pending_dead() {
        compose_st.clear();
        return Some(keymap.dead.get(&(dead, raw_ch)).copied().unwrap_or(raw_ch));
    }

    compose_st.push(raw_ch);
    if let Some(&composed) = keymap.compose.get(&compose_st.current_seq().to_vec()) {
        compose_st.clear();
        return Some(composed);
    }
    if compose_st.is_prefix_of_any(&keymap.compose) {
        return None;
    }
    compose_st.clear();
    Some(raw_ch)
}
```

**Drain loop** (from Section 2.3 / Phase A):

```rust
for mut ev in evdev_events {
    if ev.is_press || ev.is_repeat {
        ev.character = resolve_character(
            ev.scan_code,
            ev.modifiers,
            &active_keymap().lock().unwrap(),
            &mut compose_state,  // thread-local to input-router
        );
    }
    route_keyboard(&ev, &focused_app(), inbox, app_registry);
}
```

This ensures:
1. Dead key state in `ComposeState` is advanced by evdev events.
2. The `character` field of evdev-sourced `VYOMA_KEY:` events is always correctly resolved.
3. `ComposeState` has exactly one owner (the `input-router` thread's stack frame) — no shared
   ownership, no mutex needed.

---

## 8. Compose Key and Dead Keys

### 8.1 ComposeState

```rust
// supervisor/src/input_keys.rs

struct ComposeState {
    pending_dead: Option<char>,
    seq:          Vec<char>,
}

impl ComposeState {
    fn push_dead(&mut self, dead: char) { self.pending_dead = Some(dead); }
    fn pending_dead(&self) -> Option<char> { self.pending_dead }
    fn push(&mut self, ch: char) { self.seq.push(ch); }
    fn current_seq(&self) -> &[char] { &self.seq }
    fn is_prefix_of_any(&self, table: &HashMap<Vec<char>, char>) -> bool {
        table.keys().any(|k| k.starts_with(&self.seq) && k.len() > self.seq.len())
    }
    fn clear(&mut self) { self.pending_dead = None; self.seq.clear(); }
}
```

Declared as `let mut compose_state = ComposeState { ... };` at the top of
`run_input_router`. Never stored in a shared struct. Never locked.

### 8.2 Compose Key Designation

Configured via `@supervisor: kbd_compose <key_code_name>`. Default: none. When pressed,
`compose_state` accumulates subsequent characters and resolves against `keymap.compose`.

### 8.3 Dead Key Visual Indicator

When `compose_state.pending_dead.is_some()`:
```
VYOMA_KBD:compose_pending:<dead_char_codepoint>
```
When resolved:
```
VYOMA_KBD:compose_done
```

---

## 9. Key Repeat Handling

Linux kernel TTY layer generates repeats at the rate set by `input*/delay` and `input*/rate`
(default 250ms, 33 Hz). For printable keys, repeats arrive naturally from TTY.

For apps using `VYOMA_KEY:` structured events, `is_repeat=true` (bit1 in flags field) is
set on repeat events from the evdev source (`value=2` in `EV_KEY`).

Supervisor exposes repeat parameters (requires `shell = true`):
```
@supervisor: kbd_repeat_delay <ms>
@supervisor: kbd_repeat_rate  <hz>
```

---

## 10. Virtual Keyboard and kbd_inject (B5 Fix)

### 10.1 Virtual Keyboard App

On `mobile`, a WASM virtual keyboard app (space-0, z=65530) translates touch events to
`KeyEvent`s and submits via:

```
@supervisor: kbd_inject <vyoma_key_line>
```

Requires `shell = true`.

### 10.2 kbd_inject Implementation (B5 Fix)

The virtual keyboard is IME-equivalent — it must bypass the IME intercept to avoid circular
re-entry. However, it must **not** bypass `INPUT_LOCK_LEVEL` gates: injecting during
`FsTransition` (level 4) would deliver characters into an app mid-animation. Injecting during
`ChromeConsent` (level 2) with non-Enter/Escape keys would bypass the consent gate.

```rust
// supervisor/src/ipc_handlers.rs

"kbd_inject" => {
    // Requires shell = true (checked by capability gate before this call)
    let Ok(mut ev) = parse_vyoma_key(rest) else { return };
    ev.from_virtual_kbd = true;   // skip IME re-intercept (Priority 4) only
    // route_keyboard enforces all INPUT_LOCK_LEVEL gates at Priority 1
    let focused = focused_app();
    crate::input_keys::route_keyboard(&ev, &focused, inbox, app_registry);
}
```

In `route_keyboard`, the `from_virtual_kbd` flag is checked only at Priority 4:

```rust
// Priority 4: IME intercept — skipped for virtual keyboard events
if !ev.from_virtual_kbd {
    if dispatch_ime(ev, focused_app, inbox, app_registry) { return; }
}
```

Priority 1 (`INPUT_LOCK_LEVEL`) runs unconditionally — `from_virtual_kbd` does not affect it.

Test: `kbd_inject_during_fs_transition_is_suppressed` — sets `INPUT_LOCK_LEVEL=4`, calls
`route_keyboard` with `from_virtual_kbd=true`, verifies the focused app receives nothing.

### 10.3 kbd_request_virtual

When a text field receives focus, the focused app may send:
```
@supervisor: kbd_request_virtual
```

The supervisor broadcasts to all `display = true` apps:
```
VYOMA_SYSTEM:virtual_kbd:show
```

On `desktop-full`, no virtual keyboard app is registered — the broadcast is a no-op.

---

## 11. VYOMA_KBD Protocol

Pushed to all apps with `keyboard_events = true`:

| Event | Format | Meaning |
|-------|--------|---------|
| `layout_changed` | `VYOMA_KBD:layout_changed:<name>` | Active layout was switched |
| `compose_pending` | `VYOMA_KBD:compose_pending:<codepoint>` | Dead key entered |
| `compose_done` | `VYOMA_KBD:compose_done` | Compose resolved or cancelled |
| `repeat_rate` | `VYOMA_KBD:repeat_rate:<delay_ms>,<rate_hz>` | Repeat parameters changed |
| `caps_lock` | `VYOMA_KBD:caps_lock:<0\|1>` | Caps Lock toggled |
| `num_lock` | `VYOMA_KBD:num_lock:<0\|1>` | Num Lock toggled |

---

## 12. WIT Interface `vyoma:keyboard@1.0.0`

```wit
package vyoma:keyboard@1.0.0;

interface keyboard-query {
    get-active-layout:  func() -> string;
    list-layouts:       func() -> list<string>;
    get-modifiers:      func() -> u16;
    simulate-key:       func(key-code: string, modifiers: u16, character: u32) -> bool;
    set-layout:         func(name: string) -> result<_, string>;
    resolve-scancode:   func(scan-code: u16, shift: bool, altgr: bool) -> u32;
}

world keyboard {
    import keyboard-query;
}
```

Wired by `init_linker_for_app` for apps with `keyboard_events = true` or `shell = true`.

---

## 13. Platform Matrix

| Platform | Input Source | Key Repeat | IME | Notes |
|----------|-------------|-----------|-----|-------|
| `desktop-full` | `/dev/tty0` + evdev | Kernel TTY | Optional WASM IME | Full feature set; VMIN=0,VTIME=1 |
| `mobile` | Virtual keyboard app | Software (vkbd) | vkbd IS the IME | No physical kbd |
| `iot-edge` | `/dev/tty0` raw only | Kernel TTY | None | VMIN=1,VTIME=0 (no evdev needed) |
| `robotics-rt` | Serial terminal only | Kernel TTY | None | QWERTY-US fixed |
| `server-headless` | `/dev/tty0` if available | Kernel TTY | None | VMIN=1,VTIME=0 (no evdev) |
| `mcu-minimal` | UART serial only | None | None | ASCII only |

On `server-headless`, `run_input_router` catches the `/dev/tty0` open error and exits
gracefully. On `mobile`, `run_kbd_evdev` starts but exits gracefully if no evdev device found.

**`VMIN=0, VTIME=1`** is required only when `kbd-evdev` is active (i.e., `run_kbd_evdev`
found a device and is running). When `kbd-evdev` is absent (no evdev device, or
`server-headless`, `iot-edge`), `VMIN=1, VTIME=0` is used to avoid polling overhead.

---

## 14. Threading Model

```
Thread: input-router
  - Opens /dev/tty0 with VMIN=0, VTIME=1 (when kbd-evdev active)
  - Loop:
      Phase A: drain KeyEventQueue (evdev events)
               → resolve_character for each (compose_state is thread-local here)
               → route_keyboard for each
      Phase B: blocking read /dev/tty0 (returns in ≤100ms)
      Phase C: construct TTY KeyEvent
               → char_to_keycode inference (B4)
               → route_keyboard
  - Maintains ComposeState as local var — sole owner, no shared state

Thread: kbd-evdev
  - Reads /dev/input/eventN EV_KEY events
  - Acquires ModStateRef briefly to update modifier state
  - Pushes KeyEvent (character=None) into KeyEventQueue
  - NEVER calls route_keyboard, resolve_character, or send_key_to_app
```

**Key invariant**: `route_keyboard` and `resolve_character` are called only from the
`input-router` thread. Single-threaded keyboard dispatch. No concurrent stdin write races.

---

## 15. Lock-Safety Analysis

| Lock | Acquired by | Held during |
|------|------------|-------------|
| `INPUT_LOCK_LEVEL` | Any thread (atomic) | Single load/store; no hold |
| `ModStateRef` | kbd-evdev (write), input-router (read snapshot) | One field update; brief |
| `KeyEventQueue` | kbd-evdev (push), input-router (drain) | Single push or full drain; brief |
| `ACTIVE_IME` | input-router (read), ipc_handlers (write on register/unregister) | One clone; brief |
| `ACTIVE_KEYMAP` | input-router (read), ipc_handlers (write on layout switch) | Keymap lookup; brief |
| `Inbox` | input-router (send), ipc_handlers (send) | One HashMap lookup + send; brief |

No lock is held while calling another lock-acquiring function. No ABBA deadlock risk.

`ComposeState` is not a lock — it is a local variable on the input-router thread stack.

---

## 16. Capability Summary

| Capability field | Type | Grants |
|-----------------|------|--------|
| `keyboard_events` | bool | Receive `VYOMA_KEY:` structured events + `VYOMA_KBD:` notifications |
| `global_key_monitor` | bool | Receive `VYOMA_KEY:` for every key press (R35; requires supervisor review) |
| `ime` | bool | Register as active IME; receive `VYOMA_IME_INPUT:` forwarded keys |

`shell = true` required for: `kbd_inject`, `kbd_layout`, `kbd_repeat_delay/rate`,
`kbd_compose`, `kbd_request_virtual`.

---

## 17. File Layout

### 17.1 Modified Files

```
supervisor/src/input_keys.rs    — KeyEvent (+from_virtual_kbd), Modifiers, ModifierState,
                                   ModStateRef, KeyEventQueue, route_keyboard, dispatch_*,
                                   open_kbd_device, run_kbd_evdev, ComposeState (thread-local),
                                   active_ime, INPUT_LOCK_LEVEL, format_vyoma_key,
                                   ev_to_raw_string, run_input_router (VMIN=0,VTIME=1)

supervisor/src/main.rs          — spawn kbd-evdev thread; initialize ModStateRef, KeyEventQueue

supervisor/src/manifest.rs      — add keyboard_events: bool, global_key_monitor: bool, ime: bool

supervisor/src/ipc_handlers.rs  — kbd_layout, kbd_repeat_delay/rate, kbd_compose handlers;
                                   kbd_inject (B5: from_virtual_kbd, full lock pipeline);
                                   ime_register, ime_unregister;
                                   ime_commit (B2: sender==ACTIVE_IME check);
                                   ime_passthrough (B2: sender==ACTIVE_IME check)
```

### 17.2 New Files

```
supervisor/src/keymap.rs            — Keymap struct + TOML loader + char_to_keycode (B4) +
                                       linux_scancode_to_keycode + ACTIVE_KEYMAP static

supervisor/src/wit/keyboard.wit     — keyboard-query WIT interface

base/keymaps/qwerty-us.toml
base/keymaps/qwerty-gb.toml
base/keymaps/azerty-fr.toml
base/keymaps/dvorak-us.toml
base/keymaps/colemak.toml

base/rootfs.sh                      — copy base/keymaps/*.toml → /keymaps/ in initramfs

supervisor/tests/keyboard.rs        — ModifierState tests; Keymap resolution tests;
                                       ComposeState dead-key tests; route_keyboard priority
                                       tests; VYOMA_KEY round-trip tests;
                                       ime_commit_from_non_ime_app_is_rejected (B2);
                                       kbd_inject_during_fs_transition_is_suppressed (B5)
```

---

## 18. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `kbd-evdev` drain races with blocking TTY read — evdev modifier events arrive out-of-order relative to TTY character events | `VMIN=0, VTIME=1` on `/dev/tty0` termios; drain loop (Phase A) always runs before TTY byte processing (Phase C); invariant: evdev events processed before character in same loop tick |
| B2: `ime_commit`/`ime_passthrough` allow any app to inject text into any target — unrestricted stdin injection primitive | `ime_commit` and `ime_passthrough` handlers verify `sender == active_ime()` before processing; rejected with `log_warn!`; test `ime_commit_from_non_ime_app_is_rejected` |
| B3: `resolve_character` (and `ComposeState`) never called for evdev-sourced events — dead keys and compose broken; `VYOMA_KEY:` character field always 0 for evdev events | `resolve_character` called in input-router drain loop for each evdev event (before `route_keyboard`); `ComposeState` thread-local on input-router stack; `kbd-evdev` thread sets `character=None`, input-router resolves it |
| B4: TTY path sets `scan_code=0` → `KeyCode::Unknown(0)` — romanization IMEs (CJK) receive `key_unknown` for all TTY-sourced keys, breaking composition | `char_to_keycode` reverse lookup in `Keymap`: searches normal-then-shift entries for matching character, returns corresponding `KeyCode`; called for TTY events with `scan_code=0` in input-router Phase C |
| B5: `kbd_inject` bypasses all `INPUT_LOCK_LEVEL` gates — virtual keyboard can inject keys during `FsTransition` (level 4), corrupting animation state | `from_virtual_kbd: bool` field added to `KeyEvent`; `kbd_inject` sets this flag and calls `route_keyboard` at Priority 1 (full lock pipeline); only Priority 4 (IME intercept) is skipped for `from_virtual_kbd` events; test `kbd_inject_during_fs_transition_is_suppressed` |
