# Critique: Keyboard & Input Methods (Round 31)

**Critiquing**: `31-keyboard-input.md`  
**Critic role**: blocking issues only — correctness, threading safety, integration coherence

---

## CRITIQUE — 5 Blocking Issues

---

### B1: `route_keyboard` Called from Two Paths — Section 16 Claims Single-Threaded Dispatch but `kbd-evdev` Thread Can Race with `input-router`

**Problem**:

Section 16 states the key invariant: "Only `input-router` calls `route_keyboard`" and
"kbd-evdev thread never calls `route_keyboard`." This is correct in intent, but the
mechanism described — the input-router "drains `KeyEventQueue` at top of each byte-read
loop iteration" — creates a subtle race window.

The `input-router` thread reads from `/dev/tty0` with a blocking `read` call
(`VMIN=1, VTIME=0`). The loop structure implied by Section 16 is:

```rust
loop {
    // Phase A: drain KeyEventQueue (from kbd-evdev)
    for ev in key_queue.lock().unwrap().drain(..) {
        route_keyboard(&ev, ...);
    }
    // Phase B: blocking read from /dev/tty0
    tty.read(&mut buf);  // BLOCKS HERE until a byte arrives
    // Phase C: process tty byte → route_keyboard
    route_keyboard(&tty_ev, ...);
}
```

During Phase B (blocking read), the `kbd-evdev` thread pushes events into `KeyEventQueue`.
These events sit unprocessed until the next TTY byte arrives and Phase A runs again. On a
server-headless platform or a system where no characters are typed for an extended period
(e.g., only modifier keys like Caps Lock are pressed), evdev events may be delayed by
seconds or indefinitely.

More critically: on `desktop-full` with a physical keyboard, the user may hold a modifier
key (e.g., Shift, Ctrl) and simultaneously type a character. The evdev modifier-press event
and the TTY character event are produced at approximately the same time. The race is:

1. `kbd-evdev` pushes `KeyEvent { key_code: LeftShift, is_press: true }` → `KeyEventQueue`
2. `input-router` is in Phase B blocking read, receives the 'a' byte from TTY
3. `input-router` enters Phase C, constructs `KeyEvent { character: Some('a') }` — but
   has not yet drained the Shift press from `KeyEventQueue`
4. `route_keyboard` is called for 'a' with `ModStateRef` snapshot that includes Shift
   (because `ModStateRef` was updated by `kbd-evdev` before the TTY read completed)

In this specific scenario the modifier snapshot from `ModStateRef` is correct (Shift is
already set), so the delivered character is 'A'. However, the evdev `KeyEvent` for
LeftShift is still in `KeyEventQueue` and will be drained next loop iteration, causing a
**duplicate modifier-down event** to be delivered to the app: the app first receives 'A'
with Shift=1, then receives a `VYOMA_KEY:left_shift,0,0,1` event, making it appear that
Shift was pressed after the character.

For apps using `VYOMA_KEY:` structured events to track modifier state independently, this
out-of-order delivery breaks their internal modifier model. For example, a drawing app that
treats Shift+drag differently from drag will receive key events in the wrong order.

**Proposed fix**:

Change the drain strategy: instead of draining at the top of the byte-read loop, use a
**non-blocking poll** before each `route_keyboard` call to flush all pending evdev events
first, then process the TTY event. This is implemented by setting `VTIME > 0` (e.g., 5ms)
on the TTY fd so the blocking read has a short timeout, and polling `KeyEventQueue` on
each timeout expiry. Concretely:

```rust
// Set VTIME=1 (100ms timeout) so the read call returns periodically even with no input
t.c_cc[libc::VTIME as usize] = 1;   // 100ms (tenths of seconds)
t.c_cc[libc::VMIN  as usize] = 0;   // return immediately if no byte within VTIME
```

With `VMIN=0, VTIME=1`, `tty.read()` returns 0 bytes after 100ms if no key was pressed,
allowing the loop to drain `KeyEventQueue` regularly. Evdev modifier events are always
drained **before** any TTY character event in the same loop tick by checking the queue
first. The 100ms polling overhead is negligible on desktop.

Add a section to the spec titled "**TTY Read Timeout and Evdev Drain Ordering**" that
specifies `VMIN=0, VTIME=1` as the required termios configuration when `kbd-evdev` is
active, and states the drain-before-process invariant explicitly.

---

### B2: `ime_commit` and `ime_passthrough` Handlers Allow Any App to Commit Text to Any Target — No Authorization Check

**Problem**:

Section 6.3 defines the IME commit protocol:

```
@supervisor: ime_commit <target_app> <utf8_text>
@supervisor: ime_passthrough <target_app> <vyoma_key_line>
```

The `target_app` field is a free-form string. The `ipc_handlers::handle_supervisor_command`
function is called with `sender` (the name of the app that wrote the IPC command) and `msg`
(the command body). The spec does not state that the supervisor verifies `sender == ACTIVE_IME`.

An app without `ime = true` in its capabilities can send:
```
@supervisor: ime_commit shell SIGKILL
```
and if `ime_commit` blindly calls `send_reply("shell", "SIGKILL", inbox)`, the shell app
receives a spurious line in its stdin. A malicious app could use this to inject arbitrary
text into any other app's stdin — bypassing the IPC `@<app>: <msg>` routing which the
app registry mediates.

The IME commit path is particularly dangerous because it bypasses the normal `@target: msg`
IPC routing that records `LAST_SENDER`. An app receiving an `ime_commit`-injected line
cannot distinguish it from legitimate IME output.

This is not a theoretical attack: any app with `stdio = true` can write `@supervisor:`
commands. If `ime_commit` is not gated by `sender == active_ime`, it is an unrestricted
stdin injection primitive available to all apps.

**Proposed fix**:

In the `ime_commit` and `ime_passthrough` handlers, enforce that `sender` matches the
currently registered IME name:

```rust
// supervisor/src/ipc_handlers.rs

"ime_commit" => {
    let active = crate::input_keys::active_ime().lock().unwrap().clone();
    if active.as_deref() != Some(sender) {
        log_warn!(Subsystem::Input, Some(sender),
            "ime_commit rejected: sender {sender} is not active IME (active: {:?})", active);
        return;
    }
    // parse target and text, then deliver
}

"ime_passthrough" => {
    let active = crate::input_keys::active_ime().lock().unwrap().clone();
    if active.as_deref() != Some(sender) {
        log_warn!(Subsystem::Input, Some(sender), "ime_passthrough rejected: not active IME");
        return;
    }
    // decode and re-dispatch
}
```

Add a section "**IME Security: Commit Authorization**" to Section 6 stating this invariant
explicitly. Also add a test: `ime_commit_from_non_ime_app_is_rejected`.

---

### B3: `ComposeState` Is Described as Thread-Local but `resolve_character` Signature Takes `&mut ComposeState` — No Ownership Path Defined for the Two-Source Model

**Problem**:

Section 8.1 correctly states that `ComposeState` is "thread-local to the input-router
thread." Section 7.3 shows `resolve_character` taking `compose_st: &mut ComposeState`.
This is consistent: the input-router thread owns the single `ComposeState` on its stack.

However, the spec adds a second input source: the `kbd-evdev` thread pushes `KeyEvent`
values into `KeyEventQueue`. The input-router drains these and calls `route_keyboard`.
`route_keyboard` calls `dispatch_ime` or `send_key_to_app`, which call `send_key_to_app`
with a pre-constructed `KeyEvent` that already has `character: None` (set by the evdev
thread in Section 2.3: "character: None  // set by TTY path for printable keys").

The `character` field for evdev-sourced printable keys is **never set to a resolved
character** in the flow described. The evdev thread constructs `KeyEvent` with
`character: None` and pushes it. The input-router drains it and calls `route_keyboard`
with `ev.character = None`. The `resolve_character` function (which takes `&mut
ComposeState`) is never called for evdev-sourced events in the dispatch path.

This means:
1. Dead key state managed in `ComposeState` is never advanced by evdev events.
2. If a user is composing a character using dead keys, the compose state will be split
   between the TTY character path (which calls `resolve_character`) and the evdev
   structural path (which does not). On a system where both sources are active, the TTY
   path may receive a raw 'e' byte while the compose state has a pending dead-acute from
   an evdev scan code event — or vice versa.
3. The `VYOMA_KEY:` structured event for an evdev keypress will always have `character=0`
   for any key, including printable ones, because the evdev thread sets `character: None`
   and nothing resolves it.

This breaks both the compose/dead-key feature and the `VYOMA_KEY:` protocol's character
field for structured-event consumers.

**Proposed fix**:

The `character` field of a `KeyEvent` must be resolved **in the input-router thread**,
after draining from `KeyEventQueue`, and before calling `route_keyboard`. Add a
`resolve_character_for_ev` step in the drain loop:

```rust
// In input-router drain loop:
let mut key_queue_drain: Vec<KeyEvent> = {
    let mut q = key_queue.lock().unwrap();
    q.drain(..).collect()
};
for mut ev in key_queue_drain {
    // Resolve character using active keymap + compose state
    if ev.is_press || ev.is_repeat {
        ev.character = resolve_character(
            ev.scan_code,
            ev.modifiers,  // already has current ModState snapshot
            &active_keymap(),
            &mut compose_state,
        );
    }
    route_keyboard(&ev, &focused_app, &inbox, &app_registry);
}
```

Add a section "**Character Resolution in the Drain Loop**" to Section 7.3 specifying that
`resolve_character` is called exclusively in the `input-router` thread — never in
`kbd-evdev` — so that `ComposeState` ownership is never shared.

---

### B4: Layout TOML Maps Scan Codes to Unicode — But `resolve_character` Is Called with `scan_code` While `KeyCode` Enum Is Layout-Independent: Wrong Abstraction Layer Crossing

**Problem**:

Section 3 defines `KeyCode` as a "layout-independent logical key code." Section 7.2
defines the keymap TOML as mapping `scan_code` (Linux evdev integer) to Unicode code
points. Section 7.3 shows `resolve_character` taking `scan_code: u16` as input and calling
`keymap.map.get(&(scan_code, shift, altgr))`.

Section 3 also shows `linux_scancode_to_keycode(code)` producing `KeyCode::KeyA` from
scan code 30. So the pipeline is:

```
evdev scan_code 30  →  linux_scancode_to_keycode  →  KeyCode::KeyA
evdev scan_code 30  →  keymap.map.get((30, ...))  →  'a'/'A'
```

Both mappings start from the same Linux scan code. This means the keymap is a
**scan-code-to-character** mapping, not a **KeyCode-to-character** mapping. This is
intentional for Dvorak/AZERTY support: on Dvorak, key 30 (which is physically the key
labeled 'A' on QWERTY hardware) produces 'a', and `linux_scancode_to_keycode` still
returns `KeyCode::KeyA` (layout-independent). The character 'a' is the same in both
cases — no problem here.

The problem arises for the **TTY path**. In Section 5.1, the TTY router receives byte
`0x61` ('a') from `/dev/tty0`. It does not have access to `scan_code`. The TTY path
cannot call `resolve_character(scan_code, ...)` because there is no scan code for TTY
input. The TTY path constructs a `KeyEvent` with `scan_code: 0` (Section 2.3: "0 if
derived from TTY-only path without evdev").

When `route_keyboard` is called for a TTY-derived event with `scan_code=0` and
`character=Some('a')`, the character is already resolved. But when this event reaches
`dispatch_ime` (Section 6.2), the IME receives:
```
VYOMA_IME_INPUT:shell:key_unknown,0x0000,97,1
```
because `ev.key_code = KeyCode::Unknown(0)` (scan code 0 maps to Unknown).

An IME that wants to perform context-sensitive composition based on `KeyCode` (e.g., a
Japanese IME that handles Latin romanization like 'k' + 'a' → 'か') receives
`key_unknown` instead of `key_k` + `key_a`. The `KeyCode` field is essential for
romanization-based IMEs — receiving `key_unknown` breaks the IME entirely for TTY-sourced
events when both sources are active.

**Proposed fix**:

Implement **character-to-KeyCode inference** for the TTY path. When `scan_code=0` and
`character=Some(ch)`, derive the `key_code` from the character value in the active keymap
(reverse lookup):

```rust
// supervisor/src/keymap.rs

impl Keymap {
    /// Reverse lookup: given a character, return the KeyCode that most likely produces it.
    /// Used to infer KeyCode for TTY-derived events lacking scan codes.
    pub fn char_to_keycode(&self, ch: char) -> KeyCode {
        // Search normal and shift maps for a matching character
        for (&(scan, _shift, _altgr), &mapped_ch) in &self.map {
            if mapped_ch == ch {
                return linux_scancode_to_keycode(scan);
            }
        }
        KeyCode::Unknown(0)
    }
}
```

In `ev_to_raw_string` (or wherever TTY `KeyEvent`s are constructed), set:
```rust
if ev.scan_code == 0 {
    if let Some(ch) = ev.character {
        ev.key_code = active_keymap().lock().unwrap().char_to_keycode(ch);
    }
}
```

Add a section "**TTY KeyCode Inference**" to Section 5.1 documenting this reverse-lookup
step and its limitation (ambiguity when the same character appears under multiple scan
codes in the active layout — take the first match in normal-then-shift order).

---

### B5: `kbd_inject` Bypasses Lock-Level Gates — Virtual Keyboard Can Inject Keys During `FsTransition` (Level 4), Corrupting Animation State

**Problem**:

Section 13 defines the virtual keyboard injection path:

> "The supervisor `kbd_inject` handler (requires `shell = true`) calls `route_keyboard`
> directly at Priority 5 (bypassing lock gates and IME — the virtual keyboard is itself
> an IME-equivalent input source)."

Bypassing Priority 1 (the `INPUT_LOCK_LEVEL` gate) is the explicitly stated behavior.
The rationale given is that the virtual keyboard is "an IME-equivalent input source."
However, this rationale only holds for Priority 4 (IME bypass), not Priority 1 (lock gate).

The `FsTransition` lock level (4) is set during window animations for entering or exiting
full-screen mode. During this animation, all input is suppressed to prevent:
1. The user accidentally typing into an app mid-transition (could corrupt app state)
2. A second full-screen transition being triggered during the first (could corrupt animation state)

If the virtual keyboard calls `kbd_inject` during `FsTransition` and lock gates are
bypassed, characters are delivered to the focused app while the full-screen animation is
playing. On `mobile`, the full-screen keyboard-reveal animation fires when a text field
is tapped — the exact moment when the virtual keyboard is becoming visible and the user
may start typing quickly. The first keystrokes arrive before the animation completes,
potentially in a partial framebuffer state.

Similarly, `ChromeConsent` level (2) allows only Enter/Escape. The virtual keyboard
displaying a full QWERTY layout while a consent dialog is shown could confuse users by
showing keys that do not work, and could corrupt the consent flow if unexpected characters
bypass the gate.

The spec's blanket "bypassing lock gates" statement is too broad. It is only correct for
bypassing the IME layer (Priority 4) to prevent circular re-entry. The lock-level gates
(Priority 1) must still apply to virtual keyboard injection.

**Proposed fix**:

Change the `kbd_inject` description in Section 13 to explicitly re-enter `route_keyboard`
at **Priority 1** (not Priority 5), setting only a `from_virtual_kbd = true` flag to skip
the IME intercept layer (Priority 4) while still respecting all lock levels:

```rust
// supervisor/src/ipc_handlers.rs

"kbd_inject" => {
    // Requires shell = true (already checked by capability gate)
    // Parse VYOMA_KEY line back into KeyEvent
    let Ok(mut ev) = parse_vyoma_key(parts.get(1).unwrap_or(&"")) else { return };
    ev.from_virtual_kbd = true;  // skip IME re-intercept (Priority 4)
    // Do NOT bypass INPUT_LOCK_LEVEL — inject through full priority pipeline
    let focused = focused.lock().unwrap().clone().unwrap_or_default();
    crate::input_keys::route_keyboard(&ev, &focused, inbox, app_registry);
}
```

In `route_keyboard`, change the IME dispatch to check `ev.from_virtual_kbd`:

```rust
// Priority 4: IME intercept — skip if event came from virtual keyboard
if !ev.from_virtual_kbd {
    if dispatch_ime(ev, focused_app, inbox, app_registry) { return; }
}
```

Add a boolean field `from_virtual_kbd: bool` to `KeyEvent`. Update Section 13 to state
that `kbd_inject` calls `route_keyboard` at the **full priority pipeline** — not Priority
5 — with only the IME layer bypassed, and add a test
`kbd_inject_during_fs_transition_is_suppressed`.
