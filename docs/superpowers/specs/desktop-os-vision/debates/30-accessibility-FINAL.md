# FINAL Spec: Accessibility Tree & AX API (Round 30)

**Subsystem**: Accessibility Tree & AX API  
**macOS Analogue**: `NSAccessibility` / `AXUIElement` / VoiceOver  
**Depends on**: R21 (apps-map lock), R22 (lifecycle), R28 (focus, FOCUSED_APP, pending_focus_notifications), R29 (capability gating pattern)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

AX is a hybrid push/pull system. Apps push their UI tree to the supervisor via
`VYOMA_AX:` line protocol (same pattern as `VYOMA_DRAW:`). The supervisor maintains an
`AXRegistry`. Registered AX client apps receive push events and pull tree data via WIT.

A reference screen-reader WASM app (`voiceover`, space=0, z=65531) provides caption-bar
accessibility in v1.

---

## 2. AX Node Model

```rust
// supervisor/src/accessibility/model.rs

#[derive(Clone, Debug)]
pub struct AXNode {
    pub id:       u32,
    pub role:     AXRole,
    pub label:    String,         // human-readable name
    pub value:    String,         // current value (input, progress, etc.)
    pub hint:     String,         // additional description
    pub bounds:   AXRect,         // app-local coords (origin = app window top-left)
    pub state:    AXStateFlags,
    pub actions:  AXAction,
    pub children: Vec<u32>,       // child node IDs
    pub parent:   Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct AXRect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

bitflags::bitflags! {
    pub struct AXStateFlags: u16 {
        const FOCUSED      = 0x0001;
        const ENABLED      = 0x0002;
        const SELECTED     = 0x0004;
        const CHECKED      = 0x0008;
        const EXPANDED     = 0x0010;
        const VISIBLE      = 0x0020;
        const REQUIRED     = 0x0040;
        const LIVE_REGION  = 0x0080;
        const SECURE_INPUT = 0x0100;   // suppress value in all AX output
    }
}

bitflags::bitflags! {
    pub struct AXAction: u8 {
        const PRESS      = 0x01;
        const FOCUS      = 0x02;
        const SCROLL     = 0x04;
        const SET_VALUE  = 0x08;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AXRole {
    Button, Checkbox, ComboBox, Dialog, Heading, Image, Label,
    Link, List, ListItem, Menu, MenuItem, ProgressBar, RadioButton,
    ScrollBar, Slider, Tab, TabGroup, Table, TableCell, TableRow,
    TextArea, TextField, Toolbar, Tree, TreeItem, Window, Other,
}

pub struct AppAXTree {
    pub revision: u32,                     // 0 = not yet published
    pub nodes:    HashMap<u32, AXNode>,
    pub root_id:  Option<u32>,
}
```

---

## 3. AXParseState — Thread-Local (B1 Fix)

`AXParseState` is a **local variable on the per-app output reader thread** — not stored
in `AppState` or `AXRegistry`.

```rust
// supervisor/src/accessibility/parser.rs

pub enum AXParseState {
    Idle,
    InTree { revision: u32, partial: AppAXTree },
}

pub fn run_app_output_reader(
    app_name: String,
    stdout: ChildStdout,
    ax_registry: Arc<RwLock<AXRegistry>>,
    ax_event_queue: AXEventQueue,
    // ...
) {
    let mut ax_state = AXParseState::Idle;   // thread-local; sole owner
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap_or_default();
        if line.starts_with("VYOMA_AX:") {
            handle_ax_line(&app_name, &line, &mut ax_state, &ax_registry, &ax_event_queue);
        } else {
            // VYOMA_DRAW:, @supervisor:, etc.
        }
    }
}
```

`ax_state` has no shared ownership. Each app has exactly one output reader thread, so
`AXParseState` has exactly one owner at all times. `ax_registry.write()` is acquired only
for the brief atomic swap on `end_tree` — not held across the multi-line parse block.

---

## 4. AX Event Delivery Queue (B3 Fix)

All AX events are serialized through a single-consumer queue rather than calling
`send_to_stdin` directly from output reader threads:

```rust
// supervisor/src/accessibility/events.rs

pub type AXEventQueue = Arc<Mutex<Vec<(String, String)>>>;
//                               ^client_app_name  ^event_line
```

`handle_ax_line` pushes `(client_name, event_line)` onto `AXEventQueue` (brief lock).
The compositor tick phase 3b (after blit pass) drains this queue and calls
`send_to_stdin` for each entry — single-threaded delivery, no concurrent pipe writes.

```rust
// supervisor/src/compositor.rs — vsync_tick() phase 3b

fn drain_ax_events(queue: &AXEventQueue, app_stdin_txs: &HashMap<String, Sender<String>>) {
    let mut q = queue.lock();
    for (client, line) in q.drain(..) {
        if let Some(tx) = app_stdin_txs.get(&client) {
            let _ = tx.send(line);
        }
    }
}
```

`AXEventQueue` lives in `SupervisorState`. Output reader threads are producers; the
compositor tick is the sole consumer.

---

## 5. AX Registry

```rust
// supervisor/src/accessibility/registry.rs

pub struct AXRegistry {
    pub trees:   HashMap<String, AppAXTree>,    // app_name → tree
    pub clients: Vec<String>,                    // registered AX client app names
}
```

Wrapped as `Arc<RwLock<AXRegistry>>` in `SupervisorState`. Lock ordering: apps-map lock
acquired before `ax_registry` lock (never the reverse).

### 5.1 App Registration

App registers by declaring `accessibility = true` in `vyoma.toml` (publishes a tree) OR
`accessibility_client = true` (consumes the registry). Only one capability needed per role.

### 5.2 AX Client Bootstrap

When an AX client app starts, the supervisor sends synthetic events for all registered apps:
```
VYOMA_AX_EVENT:app_appeared:<app_name>
```

**Semantics**: `app_appeared` means "registered in registry" — NOT "valid tree available"
(B5 fix). `revision == 0` = tree not yet published. AX clients must wait for `tree_ready`.

---

## 6. `VYOMA_AX:` App → Supervisor Protocol

### 6.1 Full-Tree Replace

```
VYOMA_AX:begin_tree:<revision>          ← integer revision, monotonically increasing
VYOMA_AX:node:<id>,<role>,<flags>,<actions>,<x>,<y>,<w>,<h>,<parent_id|0>,<label>|<value>|<hint>
VYOMA_AX:children:<id>,<child1>,<child2>,...
VYOMA_AX:end_tree:<root_id>
```

On `end_tree`: `ax_registry.write()` acquired; old tree replaced; new `AppAXTree` stored;
`tree_ready` event (first time) or `tree_changed` event pushed to `ax_event_queue`.

### 6.2 Incremental Updates

```
VYOMA_AX:update_node:<id>,<flags_hex>,<label>|<value>|<hint>
VYOMA_AX:remove_node:<id>
VYOMA_AX:focused_node:<id>
VYOMA_AX:announce:<politeness>,<message>   ← live region; politeness=polite|assertive
```

### 6.3 Parse Limit

Maximum 4096 nodes per tree. `begin_tree` with more nodes is rejected; supervisor sends
`VYOMA_AX:error:tree_too_large` to app stdin.

---

## 7. `VYOMA_AX_EVENT:` Supervisor → Client Protocol (B5 Fix)

```
VYOMA_AX_EVENT:app_appeared:<app_name>            ← app registered; tree may be empty
VYOMA_AX_EVENT:tree_ready:<app_name>:<revision>   ← B5: first valid tree published
VYOMA_AX_EVENT:tree_changed:<app_name>:<revision> ← subsequent full-tree updates
VYOMA_AX_EVENT:node_updated:<app_name>:<id>
VYOMA_AX_EVENT:node_removed:<app_name>:<id>
VYOMA_AX_EVENT:focus_changed:<app_name>:<node_id>
VYOMA_AX_EVENT:announce:<app_name>:<politeness>,<message>
VYOMA_AX_EVENT:app_exited:<app_name>
VYOMA_AX_EVENT:window_moved:<app_name>:<x>,<y>    ← B2: position change invalidation
```

**`tree_ready` vs `tree_changed`**: `tree_ready` fired on `end_tree` when `revision` goes
from 0 to ≥1. Subsequent full-tree replaces emit `tree_changed`. AX clients wait for
`tree_ready` before calling `get-tree()` for the first time.

---

## 8. Screen Coordinate Translation (B2 Fix)

App-local bounds in `AXNode` use app window top-left as origin. AX clients needing screen
coordinates use `get-window-bounds` WIT:

```wit
record window-bounds {
    screen-x: s32,
    screen-y: s32,
    width: u32,
    height: u32,
}
get-window-bounds: func(app-name: string) -> option<window-bounds>;
```

Screen coordinate = `(window.screen_x + node.bounds.x, window.screen_y + node.bounds.y)`.

`VYOMA_AX_EVENT:window_moved:<app_name>:<x>,<y>` is emitted by the compositor when a
window position changes. AX clients invalidate cached screen coordinates on receipt.

---

## 9. WIT Interface `vyoma:accessibility@1.0.0`

### 9.1 `accessibility-client` (requires `accessibility_client = true`)

```wit
interface accessibility-client {
    /// Get full tree for an app. Returns empty list if revision == 0.
    get-tree: func(app-name: string) -> list<ax-node>;

    /// Get a single node.
    get-node: func(app-name: string, node-id: u32) -> option<ax-node>;

    /// Get summary (root ID, revision) without full tree fetch.
    get-tree-summary: func(app-name: string) -> option<ax-tree-summary>;

    /// Get screen bounds for an app window (B2).
    get-window-bounds: func(app-name: string) -> option<window-bounds>;

    /// List all registered app names.
    list-apps: func() -> list<string>;

    /// Synthesize a key press into the focused app (requires shell = true also).
    synthesize-key: func(key: string, mods: u32) -> result<_, string>;
}
```

### 9.2 `accessibility-publish` (requires `accessibility = true`)

```wit
interface accessibility-publish {
    /// Announce a live-region message (alternative to VYOMA_AX:announce stdout).
    announce: func(msg: string, politeness: ax-politeness) -> result<_, string>;
}
```

---

## 10. VoiceOver Caption App (B4 Fix)

### 10.1 Space-0 Table Update (Normative)

| App | z |
|-----|---|
| voiceover | 65531 |
| stage-strip | 65532 |
| mission-control | 65533 |
| dock | 65534 |
| chrome | 65535 |

Space-0 assertion updated:
```rust
assert!(matches!(app.name.as_str(),
    "chrome"|"dock"|"mission-control"|"stage-strip"|"voiceover"),
    "only these apps may use space=0");
```

### 10.2 Caption Bar Position (B4 Fix)

Supervisor computes safe caption y on startup and sends to `voiceover`:
```
VYOMA_AX:config_set:caption_y,<y>
```
where `y = logical_h - DOCK_H_PTS - 32 - (if sm_enabled { 0 } else { 0 })`.

`voiceover` renders caption at `(0, caption_y, logical_w, 32)`.

### 10.3 Surface Lifecycle — Clear on Empty Label (B4 Fix)

On `VYOMA_AX_EVENT:focus_changed` with empty label or `SECURE_INPUT` node:
```
VYOMA_DRAW:fill_rect:0,<caption_y>,<logical_w>,32,0x00000000
VYOMA_DRAW:flush
```
Active surface clear required. `voiceover` must not skip this step even if it thinks no
caption was previously drawn.

### 10.4 `voiceover` Bootstrap — Ignores `revision=0` (B5 Fix)

```rust
// apps/voiceover/src/main.rs

fn handle_focus_changed(app_name: &str, node_id: u32, state: &mut VoiceoverState) {
    let summary = ax_client::get_tree_summary(app_name);
    if summary.map(|s| s.revision == 0).unwrap_or(true) {
        return;  // tree not yet published; silently skip
    }
    // ... read node, render caption
}
```

---

## 11. Focus Integration (R28 Extension)

`PendingFocusNotif` extended with `ax_node_id: Option<u32>`. When `focused_node` line
received (`VYOMA_AX:focused_node:<id>`), supervisor pushes:
```rust
ax_event_queue.lock().push((client, format!("VYOMA_AX_EVENT:focus_changed:{app_name}:{id}\n")));
```

AX focus events share the same delivery queue as other AX events — drained by compositor
tick phase 3b after apps-map lock released.

---

## 12. Capability Gates

| Capability | Effect |
|-----------|--------|
| `accessibility = true` | App may publish `VYOMA_AX:` tree; `accessibility-publish` WIT wired |
| `accessibility_client = true` | App receives `VYOMA_AX_EVENT:` push; `accessibility-client` WIT wired |

`init_linker_for_app` only wires AX interfaces if the corresponding capability is declared.
Apps with neither capability never receive AX push events (output reader thread skips
`VYOMA_AX:` lines silently).

---

## 13. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| AX tree publishing | yes | yes | no | no |
| AX client WIT | yes | yes | no | no |
| `voiceover` caption app | yes | yes (bottom) | no | no |
| True TTS | deferred v2 | deferred v2 | no | no |

---

## 14. File Layout

```
apps/voiceover/src/
├── main.rs     (event loop: AX events, caption render, config_set)
└── state.rs    (VoiceoverState: caption_y, focused_app, tree_summaries)

supervisor/src/
├── accessibility/
│   ├── mod.rs       (re-exports)
│   ├── model.rs     (AXNode, AXRole, AXStateFlags, AXAction, AXRect, AppAXTree)
│   ├── registry.rs  (AXRegistry: Arc<RwLock>; trees, clients)
│   ├── parser.rs    (run_app_output_reader; AXParseState thread-local; handle_ax_line)
│   ├── events.rs    (AXEventQueue: Arc<Mutex<Vec<...>>>; drain_ax_events)
│   └── wit.rs       (accessibility-client + accessibility-publish WIT registration)
└── compositor.rs    (phase 3b: drain_ax_events; window_moved emission)
```

---

## 15. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `AXParseState` storage undefined — ABBA deadlock if embedded in AppState | Declared as thread-local on per-app output reader thread; sole owner; `ax_registry.write()` held only for atomic swap on `end_tree` |
| B2: App-local bounds without screen coordinate translation mechanism | `get-window-bounds` WIT function added to `accessibility-client`; `VYOMA_AX_EVENT:window_moved` emitted by compositor on position change |
| B3: Output reader thread calls `send_to_stdin` directly — concurrent pipe writes | `AXEventQueue: Arc<Mutex<Vec<(String, String)>>>` in SupervisorState; output readers push events; compositor tick phase 3b is sole consumer/sender |
| B4: `voiceover` z=65531 not in normative table; stale caption not cleared | Added to space-0 normative table; `VYOMA_AX:config_set:caption_y` for adaptive position; explicit clear on empty-label or SECURE_INPUT focus |
| B5: `app_appeared` arrives before `revision>0` tree exists — client queries empty tree | `app_appeared` semantics clarified (registration, not tree availability); `tree_ready:<app>:<revision>` event added (fires when revision 0→1); `voiceover` skips focus_changed for `revision==0` apps |
