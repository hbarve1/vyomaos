# FINAL Spec: Input Method Editor / CJK (Round 34)

**Subsystem**: Input Method Editor (IME / CJK)  
**macOS Analogue**: `InputMethodKit` / `NSTextInputClient` / CJK input sources  
**Depends on**: R24 (space-0 z-order), R28 (FOCUSED_APP), R30 (AX cursor rect), R31 (ime_register, ACTIVE_IME, dispatch_ime Priority 4, ime_commit/ime_passthrough, VYOMA_KEY:, from_virtual_kbd), R33 (virtual keyboard for mobile CJK)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

An IME is a `wasm32-wasip2` app with `ime = true` in its capabilities manifest. It is NOT
part of the supervisor — it is a first-class WASM process with its own lifecycle. The
supervisor mediates key forwarding and commit delivery; the IME handles all composition
logic internally.

The subsystem has five concerns:
1. **IME lifecycle** — registration, activation, deregistration, multi-IME priority
2. **Key forwarding** — which keys reach the IME; which bypass it (B5 fix)
3. **Composition display** — candidate window surface (B1 fix); inline composition text (B2 fix)
4. **Cursor rect protocol** — IME positions candidate window relative to text cursor
5. **Focus transitions** — abort in-flight composition on focus change (B3 fix)

---

## 2. IME Registration and Activation

### 2.1 Registration

An IME app writes `@supervisor: ime_register` to stdout on startup. The supervisor
handler in `ipc_handlers.rs`:

```rust
"ime_register" => {
    let mut active = active_ime().lock().unwrap();
    if active.is_some() {
        // B4 fix: reject second registration — first registrant wins
        log_warn!(Subsystem::Input, Some(sender),
            "ime_register rejected: another IME ({:?}) already active", active);
        let inb = inbox.lock().unwrap();
        if let Some(tx) = inb.get(sender) {
            let _ = tx.send("VYOMA_SYSTEM:ime_register_rejected:already_active\n".into());
        }
        return;
    }
    *active = Some(sender.to_string());
    let inb = inbox.lock().unwrap();
    if let Some(tx) = inb.get(sender) {
        let _ = tx.send("VYOMA_SYSTEM:ime_active:1\n".into());
    }
}
```

**Only one IME may be active at a time.** First to register wins. A second `ime = true`
app that attempts to register receives `VYOMA_SYSTEM:ime_register_rejected:already_active`.

### 2.2 Deregistration

```
@supervisor: ime_unregister
```

The supervisor clears `ACTIVE_IME`. The next `ime_register` from any IME app will succeed.
An IME that exits without deregistering is detected by the supervisor's process lifecycle
handler (R22): on app exit, if `ACTIVE_IME == Some(exiting_app)`, `ACTIVE_IME` is cleared.

### 2.3 IME Activation by User (IME Selection)

The user selects an IME via:
```
@supervisor: ime_activate <app_name>
```
(requires `shell = true`). The supervisor:
1. If an IME is currently active: sends `VYOMA_SYSTEM:ime_deactivated` to the current IME.
2. Clears `ACTIVE_IME`.
3. Sends `VYOMA_SYSTEM:ime_activate_request` to the named app.
4. The named app calls `@supervisor: ime_register` to claim the slot.

This indirect handoff ensures the old IME can flush any in-progress composition before the
new IME takes over (B3 pattern applied to IME switching).

### 2.4 IME App Capabilities

```toml
# apps/japanese-ime/vyoma.toml
[capabilities]
stdio   = true
display = true   # to render candidate window surface
ime     = true   # to register as active IME and receive VYOMA_IME_INPUT:
```

---

## 3. Key Forwarding — Bypass Rules (B5 Fix)

R31 §6.2 `dispatch_ime` forwards to the IME when `ev.character.is_some()` or when the
key is `Backspace`, `Enter`, or `Space`. However, keyboard shortcuts that should reach the
focused app directly must NOT be intercepted by the IME:

**Keys that bypass IME even when active** (enforced in `dispatch_ime`):

```rust
fn dispatch_ime(ev: &KeyEvent, focused_app: &str, inbox: &Inbox, _: &AppRegistry) -> bool {
    // from_virtual_kbd events bypass IME (R31 B5)
    // Also bypass when modifier key is held (Ctrl/Meta/Alt + char = shortcut, not composition)
    if ev.modifiers.intersects(Modifiers::CTRL | Modifiers::META | Modifiers::ALT) {
        return false;  // e.g., Ctrl+C, Cmd+V go directly to focused app
    }

    let ime_name = active_ime().lock().unwrap().clone();
    let Some(ime) = ime_name else { return false; };

    if ev.character.is_none()
        && !matches!(ev.key_code, KeyCode::Backspace | KeyCode::Enter | KeyCode::Space) {
        return false;
    }

    // ... forward to IME with VYOMA_IME_INPUT ...
}
```

**Rule**: Any key with `Ctrl`, `Meta`, or `Alt` modifier bypasses the IME entirely and
goes to the focused app. Pure character input and composition-control keys (Backspace,
Enter, Space) are forwarded. This matches macOS InputMethodKit behavior where composition
mode only intercepts unmodified character keys.

---

## 4. Composition Display

### 4.1 Candidate Window (B1 Fix)

The IME app renders its candidate window using `VYOMA_DRAW:` in its own surface. The
candidate window must appear **above the focused app but below space-0 chrome**.

**Z-order assignment**: The IME app is assigned to space-N (same space as the focused app)
with a high z-value just below space-0 chrome. The supervisor assigns z=65530 to the IME
app's surface (below voiceover at 65531, but this renders in space-N not space-0).

Wait — space-0 apps are rendered in pass 2 of the compositor. The IME candidate window
must appear above focused app windows in space-N pass 1, but below chrome in pass 2. The
correct mechanism: **the IME app is a space-0 app with z=65529** (below voiceover 65531),
so it composites in pass 2 above all app content.

```
Space-0 z-order (normative, updated):
  chrome      z=65535
  dock        z=65534
  mc          z=65533
  stage-strip z=65532
  voiceover   z=65531
  ime-window  z=65529   ← new; IME candidate window
```

The IME app renders transparent pixels everywhere except the candidate list area, so it
does not obstruct the rest of the screen.

The IME app is launched at boot with `space = 0` in boot.toml (same as dock, chrome).
If no IME app is installed, the slot is empty and compositing is unaffected.

### 4.2 Cursor Rect Protocol — App Reports Text Cursor Position

The IME candidate window must appear near the text insertion point. Apps that host text
fields report the cursor position via:

```
VYOMA_IME:cursor_rect:<x>,<y>,<w>,<h>
```

Where `x, y` are screen-absolute coordinates of the text cursor and `w, h` are the cursor
dimensions (typically 1×line_height). The supervisor forwards this to the active IME:

```rust
"VYOMA_IME:" => {
    // Forwarded from focused app stdout to active IME stdin
    if line.starts_with("VYOMA_IME:cursor_rect:") {
        if let Some(ime) = active_ime().lock().unwrap().as_ref() {
            let inb = inbox.lock().unwrap();
            if let Some(tx) = inb.get(ime.as_str()) {
                let _ = tx.send(format!("VYOMA_IME_CONTEXT:cursor_rect:{}\n", &line[22..]));
            }
        }
    }
}
```

The IME app positions its candidate window surface below the reported cursor rect (or above
if near the bottom of the screen).

Apps that do not report cursor rects will have the IME candidate window appear at a default
position (bottom-center of the screen).

### 4.3 Inline Composition Text (B2 Fix)

During composition (e.g., typing 'k' → 'a' in Japanese before committing 'か'), the IME
sends the current composition string to the focused app for inline display:

```
@supervisor: ime_composition_update <target_app> <utf8_text>
@supervisor: ime_composition_end <target_app>
```

The supervisor delivers this to the focused app as:
```
VYOMA_IME:composition_update:<utf8_text>
VYOMA_IME:composition_end
```

**B2 fix — no duplicate delivery**: The IME app renders its candidate window separately
using `VYOMA_DRAW:`. The `ime_composition_update` is delivered only to the **target app**
(for inline underline display), **not** used for rendering in the IME's own surface. The
IME's own surface shows the candidate list, not a duplicate of the composition string.
These are two separate responsibilities:
- Focused app: receives `VYOMA_IME:composition_update:` → renders inline underlined text
- IME app's surface: renders candidate list (separate VYOMA_DRAW: calls)

**`ime_composition_update` authorization**: same `sender == active_ime()` check as
`ime_commit` (R31 B2). Rejected if sender is not the active IME.

---

## 5. Focus Transition Handling (B3 Fix)

When focus changes from app A to app B while the IME has a pending composition, the
in-flight composition must be aborted cleanly.

The supervisor's focus-change path (R28 `pending_focus_notifications`) is extended:

```rust
// supervisor/src/ipc_handlers.rs — focus change notification path

fn on_focus_changed(new_focused: &str, inbox: &Inbox) {
    // If IME has pending composition, abort it before switching focus
    if let Some(ref ime) = *active_ime().lock().unwrap() {
        let inb = inbox.lock().unwrap();
        if let Some(tx) = inb.get(ime.as_str()) {
            // Signal IME to flush/abort composition
            let _ = tx.send("VYOMA_IME_CONTEXT:focus_will_change\n".into());
        }
    }
    // Also notify focused app that composition was cancelled
    // (app clears any inline composition text it was displaying)
    {
        let inb = inbox.lock().unwrap();
        if let Some(tx) = inb.get(/* previous focused app */) {
            let _ = tx.send("VYOMA_IME:composition_end\n".into());
        }
    }
    // Then proceed with normal focus change
}
```

The IME receives `VYOMA_IME_CONTEXT:focus_will_change` and must immediately either:
1. Commit the current composition (calls `@supervisor: ime_commit <prev_target> <text>`), or
2. Discard it (calls `@supervisor: ime_composition_end <prev_target>`)

The supervisor does not wait for the IME response — focus changes proceed immediately. The
composition flush is best-effort. If the IME fails to respond within 50ms, the supervisor
sends `VYOMA_IME:composition_end` to the previous focused app on its own.

---

## 6. Input Context Signals

The supervisor informs the active IME of text field focus state:

```
# Sent to active IME when focused app or AX focused node changes:
VYOMA_IME_CONTEXT:text_input_began:<app_name>     # text field gained focus (AX TextField/TextArea)
VYOMA_IME_CONTEXT:text_input_ended:<app_name>     # text field lost focus
VYOMA_IME_CONTEXT:cursor_rect:<x>,<y>,<w>,<h>     # forwarded from app's VYOMA_IME:cursor_rect:
VYOMA_IME_CONTEXT:focus_will_change               # focus changing; flush composition (B3)
```

These are delivered by the supervisor IPC handler whenever:
- `FOCUSED_APP` changes (R28)
- AX `focused_node` changes to/from a text input role (R30)
- App sends `VYOMA_IME:cursor_rect:` (§4.2)

---

## 7. CJK End-to-End Example — Japanese Hiragana

User types 'k', 'a', Space, Enter to produce 'か' (hiragana KA):

1. User presses 'k' (no modifier). `route_keyboard` Priority 4: `dispatch_ime` forwards
   `VYOMA_IME_INPUT:text-editor:VYOMA_KEY:key_k,0x0000,107,1` to `japanese-ime`.
2. `japanese-ime` receives 'k', updates internal romanization buffer: `pending="k"`.
   Calls `@supervisor: ime_composition_update text-editor k`.
3. Supervisor forwards `VYOMA_IME:composition_update:k` to `text-editor` stdin.
   `text-editor` shows underlined 'k'.
4. User presses 'a'. `japanese-ime` receives 'a': `pending="ka"` → resolves to 'か'.
   Calls `@supervisor: ime_composition_update text-editor か`.
5. User presses Space (candidate selection). `japanese-ime` shows candidate list in its
   space-0 surface. User presses Enter to confirm.
6. `japanese-ime` calls `@supervisor: ime_commit text-editor か`.
7. Supervisor delivers `か` to `text-editor` stdin. `text-editor` inserts 'か'.
8. `japanese-ime` calls `@supervisor: ime_composition_end text-editor`.
9. Supervisor forwards `VYOMA_IME:composition_end` to `text-editor`. Underline cleared.

---

## 8. WIT Interface `vyoma:ime@1.0.0`

```wit
package vyoma:ime@1.0.0;

interface ime-host {
    /// Called by IME to commit composed text to the currently focused app.
    commit-text:         func(target: string, text: string) -> result<_, string>;
    /// Send in-progress composition string for inline display.
    update-composition:  func(target: string, text: string) -> result<_, string>;
    /// Signal composition ended without committing (e.g., user pressed Escape).
    end-composition:     func(target: string) -> result<_, string>;
    /// Pass a key through to the focused app unchanged (skip IME re-intercept).
    passthrough-key:     func(target: string, vyoma_key_line: string) -> result<_, string>;
    /// Get current cursor rect for positioning candidate window.
    get-cursor-rect:     func() -> option<tuple<s32, s32, u32, u32>>;
}

world ime { import ime-host; }
```

Wired by `init_linker_for_app` for apps with `ime = true`.

---

## 9. Security (B4 Fix — IME Registration Priority)

With R31's `sender == active_ime()` check on `ime_commit`/`ime_passthrough`, a malicious
second `ime = true` app cannot inject text. However, the spec must also prevent:

1. **Deregistration attack**: app B calls `@supervisor: ime_unregister` — but `ime_unregister`
   also checks `sender == active_ime()`. Only the currently registered IME can unregister.

2. **Registration race**: if the current IME exits (process crash), `ACTIVE_IME` is cleared
   by the lifecycle handler (§2.2). A malicious `ime = true` app then calls `ime_register`
   and succeeds. This is acceptable — the legitimate IME should also call `ime_register` on
   restart. The first caller wins.

3. **`ime_activate` privilege**: `@supervisor: ime_activate <name>` requires `shell = true`.
   Without `shell = true`, no app can forcibly switch the active IME.

**Invariants** (normative):
- `ime_unregister` only accepted from `sender == ACTIVE_IME`
- `ime_register` rejected if `ACTIVE_IME.is_some()` (first wins)
- `ime_commit`/`ime_passthrough`/`ime_composition_update` only accepted from `sender == ACTIVE_IME`
- `ime_activate` requires `shell = true`

---

## 10. Platform Matrix

| Platform | IME | Notes |
|----------|-----|-------|
| `desktop-full` | Optional WASM IME app | Physical keyboard; IME is a background WASM process |
| `mobile` | Virtual keyboard app IS the IME | The virtual keyboard handles CJK romanization internally; `ime_commit` path same |
| `server-headless` | No | No display, no IME |
| `iot-edge` | No | ASCII-only input |
| `robotics-rt` | No | No GUI |
| `mcu-minimal` | No | No input subsystem |

On `mobile`, the virtual keyboard app (R31/R33) has both `ime = true` and `display = true`.
It is the only app that can register as the active IME on this platform. The candidate list
is rendered directly in the virtual keyboard surface (no separate IME candidate window needed).

---

## 11. File Layout

```
apps/japanese-ime/src/main.rs  — example IME: romanization table, candidate list, composition buffer
apps/japanese-ime/vyoma.toml   — capabilities: stdio=true, display=true, ime=true

supervisor/src/ipc_handlers.rs — ime_register (B4: first-wins rejection), ime_unregister,
                                  ime_activate (shell=true gate), ime_commit (B2: sender check),
                                  ime_passthrough, ime_composition_update (sender check),
                                  ime_composition_end, VYOMA_IME: cursor_rect forwarding

supervisor/src/input_keys.rs   — dispatch_ime: B5 modifier bypass rule (Ctrl/Meta/Alt chars skip IME)
                                  focus_will_change notification (B3)

supervisor/src/wit/ime.wit      — vyoma:ime@1.0.0
```

Updated space-0 z-order table (normative):
```rust
assert!(matches!(app.name.as_str(),
    "chrome"|"dock"|"mission-control"|"stage-strip"|"voiceover"|<ime_app_name>),
    "only these apps may use space=0");
// ime z=65529; name determined by boot.toml ime_app entry
```

---

## 12. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: IME candidate window Z-order — must appear above app content but below chrome | IME app runs in space-0 at z=65529 (below voiceover 65531); composited in pass 2 above all space-N app content; renders transparent except candidate list area |
| B2: Composition text delivery duplicate — IME renders candidate window AND sends composition to focused app | Two separate channels: `ime_composition_update` goes to focused app stdin (inline underline); IME's own `VYOMA_DRAW:` surface shows candidate list; no duplicate data delivered to either recipient |
| B3: Focus transition race mid-composition — composition string orphaned in supervisor | `on_focus_changed` sends `VYOMA_IME_CONTEXT:focus_will_change` to active IME before focus switches; also sends `VYOMA_IME:composition_end` to previous focused app unconditionally; IME has 50ms to flush or discard |
| B4: Second ime=true app deregisters first — `ime_unregister` could be called by any app | `ime_unregister` enforces `sender == ACTIVE_IME` (same pattern as `ime_commit`); `ime_register` rejects with `ime_register_rejected:already_active` if slot taken; `ime_activate` requires `shell = true` |
| B5: VYOMA_IME_INPUT forwarding intercepts Ctrl+C etc — keyboard shortcuts during composition | `dispatch_ime` bypasses IME when `ev.modifiers` intersects `Ctrl | Meta | Alt`; only unmodified character keys and composition-control keys (Backspace, Enter, Space) are forwarded to IME |
