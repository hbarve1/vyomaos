# FINAL Spec: Global Shortcuts & Hotkeys (Round 35)

**Subsystem**: Global Shortcuts & Hotkeys  
**macOS Analogue**: `NSEvent` global monitors / Carbon `RegisterEventHotKey` / `CGEventTap`  
**Depends on**: R24 (space-0), R28 (FOCUSED_APP), R31 (route_keyboard P3 slot, INPUT_LOCK_LEVEL, KeyEvent), R34 (dispatch_ime, ime_is_composing), R22 (app lifecycle, generation counter)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

R35 defines the global hotkey subsystem — key combinations that fire regardless of the
currently focused application. It sits at **P3** in `route_keyboard`, between the lock gate
(P1/P2) and IME intercept (P4).

Two classes:
1. **System-reserved shortcuts** — compiled into the supervisor binary; cannot be overridden.
2. **App-registered global hotkeys** — declared at runtime by apps with `hotkeys = true`.

Design invariants:
- At most one consumer per `(key_code, modifiers)` per event.
- Hot path: single `ArcSwap::load` + `HashMap` probe — no mutex on input thread.
- Input thread NEVER holds `ChildStdin` mutex — all delivery is via per-app SPSC channels (B3 fix).
- IME composition is cleanly cancelled before any system shortcut fires (B4 fix).

---

## 2. System-Reserved Shortcuts

Defined in `supervisor/src/shortcuts/system.rs` as `const SYSTEM_SHORTCUTS`:

```rust
pub struct SystemShortcut {
    pub key:           KeyCode,
    pub modifiers:     Modifiers,
    pub allowed_locks: LockLevelMask,  // B2 fix: bitfield replaces min/max pair
    pub action:        SystemAction,
}

// B2 fix: fires_at uses bitfield test
impl SystemShortcut {
    pub fn fires_at(&self, lock: LockLevel) -> bool {
        self.allowed_locks & (1 << lock as u8) != 0
    }
}

/// Mask bits: bit N = LockLevel N is allowed
pub type LockLevelMask = u8;
const LL_NONE: u8             = 1 << 0;  // LockLevel::None
const LL_MC:   u8             = 1 << 1;  // MissionControl
const LL_CC:   u8             = 1 << 2;  // ChromeConsent
const LL_SS:   u8             = 1 << 3;  // StageSwitch
const LL_FS:   u8             = 1 << 4;  // FsTransition
const LL_HW:   u8             = 0b11111; // hardware: always
const LL_ANY:  u8             = 0b00111; // None+MC+CC
```

| Combo | Action | `allowed_locks` |
|-------|--------|----------------|
| `Cmd+Space` | Spotlight | `LL_NONE` |
| `Cmd+Tab` | App switcher forward | `LL_NONE` |
| `Cmd+Shift+Tab` | App switcher reverse | `LL_NONE` |
| `Cmd+H` | Hide focused app | `LL_NONE` |
| `Cmd+M` | Minimize | `LL_NONE` |
| `Cmd+Q` | Quit focused app | `LL_NONE` |
| `Cmd+W` | Close window | `LL_NONE` |
| `F3` / `Ctrl+Up` | Mission Control | `LL_NONE` |
| `Ctrl+F2` | Focus menu bar | `LL_NONE` |
| `Cmd+Ctrl+F` | Toggle fullscreen | `LL_NONE` |
| `Cmd+Shift+3` | Screenshot full | `LL_NONE \| LL_MC` |
| `Cmd+Shift+4` | Screenshot region | `LL_NONE` |
| `Cmd+Shift+5` | Screenshot tool | `LL_NONE` |
| `Cmd+Option+Esc` | Force-quit dialog | `LL_NONE \| LL_MC \| LL_CC` |
| `Cmd+Ctrl+Q` | Lock screen | `LL_ANY` |
| `Fn+F1..F12` | HW brightness/volume | `LL_HW` |

```rust
pub enum SystemAction {
    Spotlight,
    AppSwitcher      { reverse: bool },
    MissionControl,
    LockScreen,
    Screenshot       { kind: ShotKind },
    HardwareKey      { kind: HwKey },
    HideApp, MinimizeApp, QuitApp, CloseWindow,
    ForceQuitDialog,
}
```

---

## 3. App-Registered Global Hotkeys

### 3.1 Capability

```toml
# apps/my-app/vyoma.toml
[capabilities]
hotkeys = true
```

`hotkey_register` IPC is rejected with `NotAuthorized` log and error response if `hotkeys` is absent.

### 3.2 Combo Validation (B1 Fix)

```rust
// supervisor/src/shortcuts/registry.rs
fn validate_combo(c: &HotkeyCombo) -> Result<(), RegisterError> {
    // B1: must contain at least one of Cmd, Ctrl, Alt, Fn (Shift alone is insufficient)
    let strong_mods = Modifiers::CMD | Modifiers::CTRL | Modifiers::ALT | Modifiers::FN;
    if !c.modifiers.intersects(strong_mods) {
        return Err(RegisterError::InvalidCombo); // bare letters/digits/Shift+X forbidden
    }
    // B1: bare modifier keys (LCmd, RShift, etc.) as the primary key are forbidden
    if c.key.is_modifier_key() {
        return Err(RegisterError::InvalidCombo);
    }
    // B1: Escape, Tab, Return, Backspace, Arrow* require Cmd|Ctrl|Alt (not just Fn+Shift)
    let nav_keys = &[KeyCode::Escape, KeyCode::Tab, KeyCode::Return, KeyCode::Backspace,
                     KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::ArrowLeft, KeyCode::ArrowRight];
    if nav_keys.contains(&c.key) && !c.modifiers.intersects(Modifiers::CMD | Modifiers::CTRL | Modifiers::ALT) {
        return Err(RegisterError::InvalidCombo);
    }
    Ok(())
}
```

### 3.3 Storage

```rust
// supervisor/src/shortcuts/registry.rs

pub struct GlobalHotkeyRegistry {
    by_combo: HashMap<HotkeyCombo, Vec<Registration>>,
    by_id:    HashMap<HotkeyId, HotkeyCombo>,
    next_id:  u64,
}

// B5 fix: owner identity includes generation counter
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AppHandle {
    pub name:       String,
    pub generation: u64,   // monotonically minted per spawn by process manager
}

pub struct Registration {
    pub id:               HotkeyId,
    pub owner:            AppHandle,          // B5: not Weak<ChildStdin>
    pub options:          HotkeyOptions,
    pub scope:            HotkeyScope,
    pub epoch_registered: u64,               // for tie-break
    pub manifest_priority: u8,
}

// ArcSwap for lock-free reads on input thread
pub static HOTKEY_REGISTRY: Lazy<ArcSwap<GlobalHotkeyRegistry>> =
    Lazy::new(|| ArcSwap::from_pointee(GlobalHotkeyRegistry::empty()));

// All mutations follow this exact sequence (invariant documented at top of registry.rs):
// 1. acquire REGISTRY_WRITE_LOCK
// 2. load_full()
// 3. clone-and-modify
// 4. store
// No exceptions.
pub static REGISTRY_WRITE_LOCK: Mutex<()> = Mutex::new(());
```

### 3.4 Conflict Policy

1. System-reserved combo → `RegisterError::ReservedBySystem` (checked against full `SYSTEM_SHORTCUTS` table regardless of caller's lock context).
2. Per-app quota: max `HOTKEY_QUOTA_PER_APP = 32`. Excess → `RegisterError::QuotaExceeded`.
3. Among multiple registrants for same combo: highest `manifest_priority` wins; tie-break by `epoch_registered` (earlier = wins).
4. Shadowed registrants are kept; promoted automatically on winner's exit/unregister via `on_app_exit`.
5. Shadow/activation events: `VYOMA_HOTKEY:shadowed:<id>` and `VYOMA_HOTKEY:activated:<id>` delivered via per-app channel (§5).

---

## 4. Dispatch at P3

### 4.1 `dispatch_global_shortcuts`

```rust
// supervisor/src/shortcuts/dispatch.rs

pub enum ShortcutOutcome { Consumed, PassThrough }

pub fn dispatch_global_shortcuts(
    ev:         &KeyEvent,
    lock_now:   LockLevel,   // B2: fresh re-read inside this fn
) -> ShortcutOutcome {
    if ev.from_virtual_kbd { return ShortcutOutcome::PassThrough; }
    if !ev.is_press && !ev.is_repeat { return ShortcutOutcome::PassThrough; }

    // B2 fix: re-read lock inside dispatch (not just from P1 snapshot)
    let lock = LockLevel::from_u8(INPUT_LOCK_LEVEL.load(Ordering::Acquire));
    let combo = HotkeyCombo { key: ev.key_code, modifiers: ev.modifiers };

    // 4.a System-reserved table
    if let Some(sys) = lookup_system(&combo) {
        if !sys.fires_at(lock) { return ShortcutOutcome::PassThrough; }
        if ev.is_repeat { return ShortcutOutcome::Consumed; } // swallow repeats
        // B4 fix: cancel active IME composition before firing system shortcut
        maybe_cancel_ime_composition_before_system_action();
        // Enqueue action on broker queue — never call broker synchronously (B3)
        SYSTEM_ACTION_QUEUE.send(sys.action).ok();
        return ShortcutOutcome::Consumed;
    }

    // 4.b App-registered — only at LockLevel::None
    if lock != LockLevel::None { return ShortcutOutcome::PassThrough; }

    let snap = HOTKEY_REGISTRY.load();
    let Some(list) = snap.by_combo.get(&combo) else {
        return ShortcutOutcome::PassThrough;
    };
    let Some(reg) = pick_winner(list) else {
        return ShortcutOutcome::PassThrough;
    };

    if ev.is_repeat && !reg.options.deliver_on_repeat {
        return ShortcutOutcome::Consumed; // consumed but not delivered
    }

    // B4 fix: by the time we reach app-registered hotkeys, modifier bypass (R34)
    // has already ensured modifier-bearing events skip IME. But allow_during_ime=false
    // means we still need to handle pure-modifier + functional-key combos.
    // If IME is composing and this combo is NOT allowed during IME, flush first.
    if ime_is_composing() && !reg.options.allow_during_ime {
        maybe_cancel_ime_composition_before_system_action();
        // After cancellation, fall through and deliver hotkey
    }

    deliver_hotkey(&reg.owner, ev, reg.id, reg.options.deliver_on_repeat);
    ShortcutOutcome::Consumed
}
```

### 4.2 IME Composition Cancel (B4 Fix)

```rust
// supervisor/src/shortcuts/dispatch.rs

fn maybe_cancel_ime_composition_before_system_action() {
    if !ime_is_composing() { return; }
    // Enqueue cancel to composing app's stdin writer channel (non-blocking)
    let target = COMPOSING_APP.load().clone(); // ArcSwap<Option<String>>
    if let Some(app) = target.as_ref() {
        enqueue_stdin_line(app, "VYOMA_IME:cancel\n");
    }
    // Signal IME to flush
    if let Some(ime) = active_ime().lock().unwrap().as_ref() {
        enqueue_stdin_line(ime, "VYOMA_IME_CONTEXT:composition_cancelled_by_shortcut\n");
    }
    // Do NOT wait for acknowledgment — proceed immediately.
    // The 50ms best-effort timeout from R34 §5 applies here too.
}
```

---

## 5. Delivery — Per-App SPSC Channel (B3 Fix)

The input thread MUST NEVER acquire `ChildStdin` mutex. All delivery uses
per-app non-blocking sender channels.

```rust
// supervisor/src/shortcuts/dispatch.rs

// Delivery channel registry: app handle → SPSC sender
// Maintained by the process manager alongside ChildStdin.
pub static HOTKEY_CHANNELS: Lazy<ArcSwap<HashMap<AppHandle, mpsc::SyncSender<HotkeyLine>>>> =
    Lazy::new(|| ArcSwap::from_pointee(HashMap::new()));

fn deliver_hotkey(owner: &AppHandle, ev: &KeyEvent, id: HotkeyId, deliver_on_repeat: bool) {
    if ev.is_repeat && !deliver_on_repeat { return; }
    let line = format!(
        "VYOMA_HOTKEY:fire:{}:{}:{:04x}:{}\n",
        id, ev.key_code as u32, ev.modifiers.bits(), ev.is_repeat as u8
    );
    let channels = HOTKEY_CHANNELS.load();
    if let Some(tx) = channels.get(owner) {
        if tx.try_send(HotkeyLine(line)).is_err() {
            log_warn!(Subsystem::Input, Some(&owner.name),
                "hotkey channel full — dropped fire for id={}", id);
        }
    }
    // If channel is not found: app exited between pick_winner and deliver; ignore.
}

fn enqueue_stdin_line(app: &str, line: &str) {
    // Looks up HOTKEY_CHANNELS by current-generation handle.
    // Used for non-hotkey notifications (cancel, shadow, activate).
    let gen = PROCESS_MANAGER.generation_of(app);
    let handle = AppHandle { name: app.to_string(), generation: gen };
    let channels = HOTKEY_CHANNELS.load();
    if let Some(tx) = channels.get(&handle) {
        let _ = tx.try_send(HotkeyLine(line.to_string()));
    }
}
```

Each app has a `stdin-writer-<name>` thread (spawned by the process manager) that drains
the channel and writes to `ChildStdin`. This thread is the sole writer to each app's stdin
for hotkey events.

**Lock-acquisition order rule** (documented in `shortcuts/mod.rs`):
- Input thread: `ArcSwap::load` → `HashMap::get` → `try_send` (never `ChildStdin::lock`)
- `stdin-writer` thread: `channel::recv` → `ChildStdin::lock` → `write`
- No cross-thread lock inversion possible.

---

## 6. App-Exit Cleanup (B5 Fix)

```rust
// supervisor/src/shortcuts/registry.rs

pub fn on_app_exit(handle: &AppHandle) {
    let _guard = REGISTRY_WRITE_LOCK.lock().unwrap(); // step 1
    let cur = HOTKEY_REGISTRY.load_full();            // step 2
    let mut next = (*cur).clone();                    // step 3
    let promoted = next.purge_handle(handle);         // removes dead entries, returns promoted ids
    HOTKEY_REGISTRY.store(Arc::new(next));            // step 4

    // Notify promoted registrants
    for id in promoted {
        let snap = HOTKEY_REGISTRY.load();
        if let Some(combo) = snap.by_id.get(&id) {
            if let Some(list) = snap.by_combo.get(combo) {
                if let Some(reg) = list.iter().find(|r| r.id == id) {
                    enqueue_stdin_line(&reg.owner.name,
                        &format!("VYOMA_HOTKEY:activated:{}\n", id));
                }
            }
        }
    }
}

// Called BEFORE re-spawn under restart=always (B5 fix: dead generation purged
// before new generation registers, so new instance can reclaim its combo).
// The process manager calls on_app_exit → restart_app (in that order).
```

Generation tracking:
```rust
// supervisor/src/process_manager.rs (extension)
pub fn next_generation_for(app: &str) -> u64 {
    // AtomicU64 per-app counter; incremented on each spawn
    APP_GENERATIONS.get_or_insert(app).fetch_add(1, Ordering::Relaxed)
}
```

---

## 7. IPC Handlers

```rust
// supervisor/src/shortcuts/ipc_handlers.rs

pub fn hotkey_register(sender: &str, args: &str, manifest: &Manifest, inbox: &Inbox) {
    if !manifest.capabilities.hotkeys {
        log_warn!(Subsystem::Input, Some(sender), "hotkey_register denied: no hotkeys capability");
        send_reply(inbox, sender, "VYOMA_HOTKEY:error:not_authorized\n");
        return;
    }

    let Ok(combo) = parse_combo(args) else {
        send_reply(inbox, sender, "VYOMA_HOTKEY:error:parse_error\n");
        return;
    };
    if let Err(e) = validate_combo(&combo) {         // B1 fix
        send_reply(inbox, sender, &format!("VYOMA_HOTKEY:error:{:?}\n", e));
        return;
    }
    if lookup_system(&combo).is_some() {              // system table sealed
        send_reply(inbox, sender, "VYOMA_HOTKEY:error:reserved_by_system\n");
        return;
    }

    let gen = PROCESS_MANAGER.generation_of(sender);
    let handle = AppHandle { name: sender.to_string(), generation: gen };

    let _guard = REGISTRY_WRITE_LOCK.lock().unwrap();
    let cur = HOTKEY_REGISTRY.load_full();
    let count = cur.by_combo.values()
        .flat_map(|v| v.iter())
        .filter(|r| r.owner.name == sender)
        .count();
    if count >= HOTKEY_QUOTA_PER_APP {
        send_reply(inbox, sender, "VYOMA_HOTKEY:error:quota_exceeded\n");
        return;
    }
    let mut next = (*cur).clone();
    let id = next.insert(handle, combo.clone(), parse_options(args));
    let shadowed = next.by_combo[&combo].len() > 1; // winner already at index 0
    HOTKEY_REGISTRY.store(Arc::new(next));
    if shadowed {
        send_reply(inbox, sender, &format!("VYOMA_HOTKEY:shadowed:{}\n", id));
    } else {
        send_reply(inbox, sender, &format!("VYOMA_HOTKEY:registered:{}\n", id));
    }
}

pub fn hotkey_unregister(sender: &str, id_str: &str) {
    let Ok(id) = id_str.parse::<HotkeyId>() else { return; };
    let _guard = REGISTRY_WRITE_LOCK.lock().unwrap();
    let cur = HOTKEY_REGISTRY.load_full();
    let mut next = (*cur).clone();
    let promoted = next.remove_by_id(id, sender); // no-op if sender != owner
    HOTKEY_REGISTRY.store(Arc::new(next));
    for promoted_id in promoted {
        // notify promoted registrant (same enqueue_stdin_line pattern)
        // ...
    }
}
```

---

## 8. WIT Interface `vyoma:hotkeys@1.0.0`

```wit
package vyoma:hotkeys@1.0.0;

interface registry {
    use vyoma:input/types.{key-code, modifiers};

    enum scope { global, app-only }

    record combo {
        key:       key-code,
        modifiers: modifiers,  // MUST contain cmd, ctrl, alt, or fn
    }

    record options {
        deliver-on-repeat:  bool,
        allow-during-ime:   bool,  // default false
    }

    variant register-error {
        not-authorized,          // hotkeys=true missing from manifest
        reserved-by-system,      // combo is in SYSTEM_SHORTCUTS
        quota-exceeded,          // >32 registrations
        invalid-combo,           // fails validate_combo (B1)
    }

    type hotkey-id = u64;

    register:   func(c: combo, s: scope, o: options) -> result<hotkey-id, register-error>;
    unregister: func(id: hotkey-id);
    list-mine:  func() -> list<hotkey-id>;
}

interface events {
    use registry.{hotkey-id};

    record fire-event {
        id:        hotkey-id,
        is-repeat: bool,
    }

    on-fire:       func(ev: fire-event);
    on-shadowed:   func(id: hotkey-id);
    on-activated:  func(id: hotkey-id);
}

world hotkeys-client {
    import registry;
    export events;
}
```

---

## 9. File Layout

```
supervisor/src/
├── input/
│   └── route.rs          ← route_keyboard P3 calls dispatch_global_shortcuts
└── shortcuts/
    ├── mod.rs            ← module root; lock-order rules documented here
    ├── system.rs         ← SYSTEM_SHORTCUTS, SystemShortcut, fires_at, LockLevelMask
    ├── registry.rs       ← GlobalHotkeyRegistry, ArcSwap, REGISTRY_WRITE_LOCK
    │                        AppHandle (name+generation), validate_combo (B1)
    ├── dispatch.rs       ← dispatch_global_shortcuts, deliver_hotkey (B3),
    │                        maybe_cancel_ime_composition_before_system_action (B4),
    │                        HOTKEY_CHANNELS ArcSwap
    ├── ipc_handlers.rs   ← hotkey_register (B1+B5), hotkey_unregister, on_app_exit (B5)
    └── wit_impl.rs       ← vyoma:hotkeys@1.0.0 glue
```

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Bare-key registration enables permanent input lockout | `validate_combo` enforces at least one of Cmd/Ctrl/Alt/Fn; rejects modifier-key primaries and bare nav keys without strong modifier; WIT `invalid-combo` error raised at register time |
| B2: Lock-level race — stale lock snapshot and undefined `fires_at` | `dispatch_global_shortcuts` re-reads `INPUT_LOCK_LEVEL` (Acquire) on entry; `SystemShortcut` uses `allowed_locks: LockLevelMask` bitfield with `fires_at` = bitmask test |
| B3: Stdin write contention deadlock (input thread holds mutex) | Input thread NEVER acquires `ChildStdin` mutex; delivery via per-app SPSC `mpsc::SyncSender`; `stdin-writer` thread serializes writes; system actions enqueued on broker queue (never called synchronously) |
| B4: IME composition left corrupted after system shortcut fires | `maybe_cancel_ime_composition_before_system_action` enqueues `VYOMA_IME:cancel` to composing app and `focus_will_change` to IME before system action executes; best-effort (no wait) matching R34 §5 |
| B5: `Weak<ChildStdin>` ghost registrations under app restart | `Registration` uses `AppHandle { name, generation }` instead of Weak; `on_app_exit` called before re-spawn so dead generation purged before live generation registers; delivery uses `HOTKEY_CHANNELS` ArcSwap keyed by AppHandle |
