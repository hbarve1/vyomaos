# Spec: Accessibility Tree & AX API (Round 30)

**Subsystem**: Accessibility Tree & AX API
**macOS Analogue**: `NSAccessibility` / `AXUIElement` / VoiceOver
**Depends on**: R23 (chrome), R24 (compositor, Model A), R28 (focus/z-order), R29 (wallpaper cache pattern)
**Status**: DRAFT — under critique

---

## 1. Architecture Decision: Hybrid Supervisor + App-Side Model

### 1.1 Problem Statement

WASM apps have no shared memory. Each app has its own isolated address space inside
Wasmtime. An accessibility client (a screen-reader WASM app) cannot directly introspect
another app's heap. Therefore, accessibility data must transit through the supervisor.

Three candidate architectures:

**Option A — Pure supervisor aggregation**: Supervisor polls each app's AX tree via a
dedicated IPC command (`@supervisor: ax_tree`), aggregates the responses, and exposes the
combined tree to AX clients. Apps have no WIT interface; they respond only to line-based
IPC queries.

**Option B — Pure WIT interface**: Each app exposes a `vyoma:accessibility@1.0.0` WIT
world. The screen-reader app binds to other apps via a WIT bridge. Requires supervisor to
act as a WIT-to-WIT proxy, which adds significant complexity and doesn't fit the existing
line-protocol architecture.

**Option C — Hybrid push/pull**: Apps push incremental AX tree updates to supervisor via
`VYOMA_AX:` stdout commands (analogous to `VYOMA_DRAW:`). Supervisor maintains an AX
registry. AX clients pull the registry via WIT query OR receive push events via stdin.

**Decision: Option C** — hybrid push/pull hybrid.

Rationale:
- Consistent with existing `VYOMA_DRAW:` line protocol — no new IPC primitives needed.
- Push-on-change is more efficient than supervisor polling (no unnecessary round-trips when
  UI is static).
- Supervisor can serve AX queries from its registry without round-tripping to apps (the
  registry is always coherent).
- WIT query interface for AX clients fits the established `init_linker_for_app` pattern.
- The line protocol is human-readable and debuggable via `log <name>` at the console.

---

## 2. AX Node Data Model

### 2.1 `AXNode` Struct

```rust
// supervisor/src/accessibility/tree.rs

#[derive(Clone, Debug)]
pub struct AXNode {
    pub id: u32,                        // node ID, unique within one app's tree
    pub role: AXRole,
    pub label: String,                  // accessible name (aria-label equivalent)
    pub description: Option<String>,    // secondary description
    pub value: Option<String>,          // current value for inputs, sliders, etc.
    pub bounds: AXBounds,               // bounding box in app-local coordinates
    pub state: AXStateFlags,
    pub children: Vec<u32>,             // child node IDs (IDs only; nodes are flat-stored)
    pub parent: Option<u32>,            // parent node ID; None = root
    pub actions: Vec<AXAction>,         // supported interactions
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AXBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct AXStateFlags: u16 {
        const FOCUSED     = 0x0001;
        const ENABLED     = 0x0002;
        const VISIBLE     = 0x0004;
        const EXPANDED    = 0x0008;
        const CHECKED     = 0x0010;
        const SELECTED    = 0x0020;
        const READONLY    = 0x0040;
        const BUSY        = 0x0080;
        const MODAL       = 0x0100;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AXRole {
    Window,
    Button,
    CheckBox,
    RadioButton,
    TextInput,
    TextArea,
    Label,
    StaticText,
    Image,
    Link,
    List,
    ListItem,
    Menu,
    MenuItem,
    MenuBar,
    Toolbar,
    TabGroup,
    Tab,
    Slider,
    ProgressIndicator,
    ScrollArea,
    ScrollBar,
    Table,
    Row,
    Cell,
    Heading,
    Group,
    Dialog,
    Alert,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AXAction {
    Press,
    Focus,
    SetValue,
    ScrollUp,
    ScrollDown,
    ShowMenu,
    Expand,
    Collapse,
}
```

### 2.2 App AX Tree

```rust
// supervisor/src/accessibility/tree.rs

#[derive(Clone, Debug, Default)]
pub struct AppAXTree {
    pub app_name: String,
    pub nodes: HashMap<u32, AXNode>,    // id → node, flat storage
    pub root_id: Option<u32>,
    pub revision: u64,                  // monotonically increasing; 0 = never published
    pub focused_node_id: Option<u32>,   // which node within this app is focused
}

impl AppAXTree {
    pub fn node(&self, id: u32) -> Option<&AXNode> {
        self.nodes.get(&id)
    }

    pub fn root(&self) -> Option<&AXNode> {
        self.root_id.and_then(|id| self.nodes.get(&id))
    }

    /// Flat DFS traversal from root, yielding nodes in focus-tab order.
    pub fn focusable_nodes(&self) -> Vec<u32> {
        let mut out = Vec::new();
        if let Some(root_id) = self.root_id {
            self.collect_focusable(root_id, &mut out);
        }
        out
    }

    fn collect_focusable(&self, id: u32, out: &mut Vec<u32>) {
        if let Some(node) = self.nodes.get(&id) {
            if node.state.contains(AXStateFlags::ENABLED)
                && node.state.contains(AXStateFlags::VISIBLE)
                && node.actions.contains(&AXAction::Focus)
            {
                out.push(id);
            }
            for &child_id in &node.children {
                self.collect_focusable(child_id, out);
            }
        }
    }
}
```

---

## 3. `VYOMA_AX:` Line Protocol (App → Supervisor)

Apps write `VYOMA_AX:` commands to stdout. Supervisor intercepts lines beginning with
`VYOMA_AX:` before routing output elsewhere, identical to the `VYOMA_DRAW:` interception
in `display.rs`.

### 3.1 Commands

```
VYOMA_AX:begin_tree:<revision>
VYOMA_AX:node:<id>,<parent_id|none>,<role>,<state_bits>,<x>,<y>,<w>,<h>,<actions_bits>,<label>
VYOMA_AX:node_value:<id>,<value>
VYOMA_AX:node_desc:<id>,<description>
VYOMA_AX:end_tree
VYOMA_AX:update_node:<id>,<field>,<value>
VYOMA_AX:focused_node:<id>
VYOMA_AX:remove_node:<id>
```

**Full tree publish** (revision bump): `begin_tree` → N × `node` lines → `end_tree`.
The supervisor replaces the entire stored `AppAXTree` atomically on `end_tree`.

**Incremental update** (no revision bump required for partial): `update_node` patches a
single field on an existing node without requiring full tree retransmission.

**Encoding rules**:
- `parent_id` = decimal integer or the literal `none` for root.
- `role` = lowercase enum name (e.g. `button`, `text_input`).
- `state_bits` = decimal `u16` bitmask matching `AXStateFlags`.
- `actions_bits` = decimal `u8` bitmask; bit0=Press, bit1=Focus, bit2=SetValue, etc.
- `label` = UTF-8 text; pipe `|` character escaped as `\|`; newline forbidden.
- `revision` = decimal `u64`.

**Example** — a simple dialog:
```
VYOMA_AX:begin_tree:7
VYOMA_AX:node:1,none,dialog,6,0,0,960,700,0,Save Document
VYOMA_AX:node:2,1,label,6,20,20,200,20,0,Filename
VYOMA_AX:node:3,1,text_input,3,20,44,300,28,6,
VYOMA_AX:node_value:3,Untitled.txt
VYOMA_AX:node:4,1,button,3,340,44,80,28,3,Save
VYOMA_AX:node:5,1,button,3,430,44,80,28,3,Cancel
VYOMA_AX:end_tree
```

### 3.2 Incremental Updates

After the initial full publish, apps should prefer incremental patches:

```
VYOMA_AX:update_node:3,value,MyDocument.txt
VYOMA_AX:update_node:3,state,7
VYOMA_AX:focused_node:4
```

`update_node` field names: `label`, `value`, `description`, `state`, `bounds`
(`x,y,w,h` comma-separated), `actions`.

### 3.3 Why Line Protocol over JSON or Binary

- **Line protocol**: Zero-copy interception in `handle_app_output`; no serde dependency in
  apps; consistent with existing `VYOMA_DRAW:` and `VYOMA_INPUT:` patterns. Parse errors
  are logged and skipped, not fatal.
- **JSON**: Would require a JSON parser in WASM apps and supervisor. Apps are kept minimal.
- **Binary**: Harder to debug; `log <name>` would show gibberish; inconsistent with the
  rest of the protocol.

---

## 4. Supervisor AX Registry

### 4.1 Registry Struct

```rust
// supervisor/src/accessibility/registry.rs

pub struct AXRegistry {
    /// Per-app AX trees, indexed by app name.
    pub trees: HashMap<String, AppAXTree>,

    /// Revision counter incremented each time any tree changes.
    /// AX clients use this to detect staleness without polling.
    pub global_revision: u64,

    /// Registered AX client app names (apps with `accessibility = true`).
    /// These receive push notifications via stdin.
    pub clients: Vec<String>,
}
```

### 4.2 Thread Safety

`AXRegistry` is stored inside `SupervisorState` wrapped in `Arc<RwLock<AXRegistry>>`:

```rust
// In SupervisorState:
pub ax_registry: Arc<RwLock<AXRegistry>>,
```

**Locking discipline**:

| Operation | Lock acquired |
|-----------|--------------|
| App publishes `VYOMA_AX:end_tree` | `ax_registry.write()` |
| App sends `VYOMA_AX:update_node` | `ax_registry.write()` |
| AX client queries via WIT | `ax_registry.read()` |
| Focus change notification dispatch | `ax_registry.read()` (read clients list) |
| Register AX client at app spawn | `ax_registry.write()` |

**Critical invariant**: `ax_registry.write()` is NEVER acquired while holding
`apps_map.write()`. The ordering is always `apps_map` → `ax_registry` when both are
needed simultaneously (e.g., app termination cleanup). This prevents ABBA deadlock.

**`apps_map` is not held during AX push notifications**: After draining focus
notifications (R28 pattern with `pending_focus_notifications`), AX events are dispatched
to clients after releasing `apps_map`. Client stdin delivery uses the existing
`send_to_stdin(app_name, msg)` path which acquires `apps_map.read()` only.

### 4.3 App Termination Cleanup

When an app terminates, its AX tree is removed:

```rust
fn on_app_terminated(state: &mut SupervisorState, app_name: &str) {
    // apps_map write lock is held here (from lifecycle handler)
    // Do NOT acquire ax_registry write while apps_map write is held.
    // Instead, queue removal:
    state.pending_ax_removals.push(app_name.to_string());
    // Drain pending_ax_removals after apps_map lock is released.
}
```

`pending_ax_removals: Vec<String>` — drained in the compositor tick, mirroring the
`pending_focus_notifications` pattern from R28.

---

## 5. AX Client Protocol (Supervisor → AX Client)

### 5.1 Push Events via Stdin

Apps registered as AX clients (capability `accessibility = true`) receive push events
delivered to their stdin:

```
VYOMA_AX_EVENT:focus_changed:<app_name>:<node_id>
VYOMA_AX_EVENT:element_changed:<app_name>:<node_id>:<field>
VYOMA_AX_EVENT:tree_changed:<app_name>:<revision>
VYOMA_AX_EVENT:app_appeared:<app_name>
VYOMA_AX_EVENT:app_disappeared:<app_name>
VYOMA_AX_EVENT:announcement:<app_name>:<text>
```

`VYOMA_AX_EVENT:announcement` is how an app requests the screen reader announce
arbitrary text (live region equivalent). The app writes:
```
VYOMA_AX:announce:<priority>,<text>
```
Supervisor relays this as a `VYOMA_AX_EVENT:announcement` to all registered AX clients.
Priority: `polite` (0) or `assertive` (1).

### 5.2 Pull Query via WIT Interface

AX clients may also query the registry synchronously via a WIT interface:

```wit
// wit/world.wit — fragment for accessibility capability

interface accessibility-client {
    record ax-node-summary {
        id: u32,
        role: string,
        label: string,
        value: option<string>,
        state-bits: u16,
        x: s32,
        y: s32,
        width: u32,
        height: u32,
        children: list<u32>,
        actions-bits: u8,
    }

    record ax-tree-summary {
        app-name: string,
        revision: u64,
        root-id: option<u32>,
        focused-node-id: option<u32>,
    }

    /// List all apps currently registered in AX registry.
    list-apps: func() -> list<ax-tree-summary>;

    /// Get all nodes for one app's tree.
    get-tree: func(app-name: string) -> option<list<ax-node-summary>>;

    /// Get a single node.
    get-node: func(app-name: string, node-id: u32) -> option<ax-node-summary>;

    /// Get the globally focused node (app + node).
    get-focused: func() -> option<tuple<string, u32>>;

    /// Get the global AX registry revision.
    get-revision: func() -> u64;
}

world accessibility-world {
    use accessibility-client.{...};
    import accessibility-client;
}
```

The `init_linker_for_app` function conditionally adds `accessibility-client` imports only
when `capabilities.accessibility == true`:

```rust
// supervisor/src/accessibility/wit_host.rs

pub fn init_linker_for_ax_client(
    linker: &mut Linker<AppCtx>,
    registry: Arc<RwLock<AXRegistry>>,
) -> Result<()> {
    linker.func_wrap("vyoma:accessibility/accessibility-client", "list-apps",
        move |_caller: Caller<AppCtx>| {
            let reg = registry.read().unwrap();
            // serialize list<ax-tree-summary> ...
        })?;
    // ... additional bindings ...
    Ok(())
}
```

### 5.3 Client Registration

When an app with `accessibility = true` is spawned, the supervisor:
1. Acquires `ax_registry.write()`.
2. Pushes `app_name` onto `registry.clients`.
3. Delivers a synthetic `VYOMA_AX_EVENT:app_appeared:<name>` for every app already in the
   registry, so the AX client can bootstrap its internal state.

---

## 6. Focus Integration (R28 Coupling)

### 6.1 Focus Change → AX Event

When `FOCUSED_APP` (R28 `ArcSwap<String>`) changes, the focus-change path in
`handle_focus_change` appends an AX notification to `pending_focus_notifications`:

```rust
pub struct PendingFocusNotif {
    pub kind: FocusNotifKind,
    pub app_name: String,
    pub node_id: Option<u32>,   // None = app-level focus
}

pub enum FocusNotifKind {
    AppFocused,          // emits VYOMA_AX_EVENT:focus_changed:<app> (no node)
    NodeFocused,         // emits VYOMA_AX_EVENT:focus_changed:<app>:<node>
}
```

After `apps_map` lock is released (following R28 discipline), the compositor tick drains
`pending_focus_notifications` and sends `VYOMA_AX_EVENT:focus_changed:...` to all
`registry.clients`.

### 6.2 Per-Node Focus within App

`VYOMA_AX:focused_node:<id>` updates `AppAXTree.focused_node_id` and also triggers a
`PendingFocusNotif { kind: NodeFocused, app_name, node_id: Some(id) }`.

The globally focused node is:
1. The app identified by `FOCUSED_APP`.
2. Within that app, the node identified by `AppAXTree.focused_node_id` (if set).

`get-focused()` WIT call returns this pair.

---

## 7. Keyboard Navigation (Tab Traversal)

### 7.1 Design Decision: App-Owned Tab Logic

Tab/shift-tab are **delivered to the focused app as normal keystrokes**, not intercepted by
the supervisor. Rationale:
- Apps know their own focus order semantics (logical tab order may differ from spatial
  layout).
- Supervisor interception would require the supervisor to understand per-app layout, which
  breaks the capability model.
- Apps that implement tab navigation update `VYOMA_AX:focused_node:<id>` when their
  internal focus changes, keeping the AX tree current.

### 7.2 AX-Driven Tab Traversal (Screen Reader Mode)

When a screen reader is active and has consumed input focus (e.g., VoiceOver-cursor mode),
the screen reader WASM app may send an IPC command to the supervisor to synthesize a tab
event to the currently focused app:

```
@supervisor: ax_synthesize_key:<app_name>,tab
@supervisor: ax_synthesize_key:<app_name>,shift+tab
```

This requires `shell = true` capability on the screen reader app (it is using shell IPC).
Supervisor injects the key event into the target app's stdin as if it arrived from the TTY.

### 7.3 Supervisor Tab Intercept for AX Traversal (v2, out of scope for v1)

In v1, the supervisor does not intercept Tab. In v2, an `INPUT_LOCK_LEVEL` of
`AXNavigation=5` (above `FsTransition=4`) could be defined to allow the supervisor to own
tab routing when a screen reader is active. This is deferred.

---

## 8. VoiceOver Equivalent (`voiceover` App)

### 8.1 Scope for v1

A `voiceover` WASM app is in scope as a framework stub, not a full TTS implementation.

The stub:
- Has capabilities `{ accessibility = true, stdio = true, display = true }`.
- Receives `VYOMA_AX_EVENT:focus_changed:...` via stdin.
- Queries `get-node()` via WIT to read the focused node's label.
- Writes the label text to a `VYOMA_DRAW:draw_text` overlay at a fixed position (e.g.,
  bottom of screen) — a visual "caption bar" approach.

### 8.2 Text-to-Speech Hook

True TTS is out of scope for v1. The `voiceover` app writes captions visually.

For v2, a `vyoma:tts@1.0.0` WIT interface would expose:
```wit
interface tts {
    speak: func(text: string, priority: u8);
    stop: func();
}
```
This would require an audio subsystem (out of scope for the current roadmap through R30+).

### 8.3 Visual Caption Overlay

The `voiceover` app runs in space-0 at z=65531 (below stage-strip at 65532, above normal
app windows). It draws a translucent caption bar:

```
VYOMA_DRAW:fill_rect:0,670,960,30,0x000000CC
VYOMA_DRAW:draw_text:8,675,0xFFFFFFFF,m,<focused label>
VYOMA_DRAW:flush
```

It registers as a space-0 compositor participant (same pattern as chrome, dock) by
declaring `display = true` and placing itself in space 0 via manifest:

```toml
[app]
name = "voiceover"
space = 0
z_order = 65531

[capabilities]
stdio       = true
display     = true
accessibility = true
```

---

## 9. Security — `accessibility = true` Capability Gate

### 9.1 Why Gate Access

An AX client can read the full label, value, and structure of every visible UI element
across all apps. This could leak:
- Passwords typed into a text input (if the app does not redact its AX tree).
- Confidential document content rendered in a text area.

### 9.2 Capability Enforcement

The supervisor enforces `accessibility = true` at two points:

**At app spawn**: `init_linker_for_app` only adds `accessibility-client` WIT imports when
`manifest.capabilities.accessibility == true`.

**At AX client registration**: `register_ax_client()` checks the manifest before pushing
the app onto `registry.clients`. An app without the capability that writes
`VYOMA_AX_EVENT:` to stdout is ignored; events are never delivered.

Apps **publishing** their AX tree via `VYOMA_AX:` commands do NOT need `accessibility =
true` — they are publishing their own tree, which is their own data. Any app with
`display = true` may optionally publish an AX tree.

### 9.3 Password Field Redaction

Apps are responsible for redacting their AX tree for sensitive fields. A password input
should publish:

```
VYOMA_AX:update_node:3,value,
VYOMA_AX:update_node:3,state,67
```

(empty value; state bit `READONLY` indicates "secure input", but a dedicated
`SECURE_INPUT` state flag bit 0x0200 is added to `AXStateFlags`.)

The supervisor does not automatically redact — this matches macOS behavior where
`NSSecureTextField` returns an empty string for `AXValue`.

---

## 10. `VYOMA_AX:` Protocol Summary Table

| Command | Direction | Meaning |
|---------|-----------|---------|
| `VYOMA_AX:begin_tree:<rev>` | App → Supervisor | Start full tree replace |
| `VYOMA_AX:node:<fields>` | App → Supervisor | Declare one node within `begin/end_tree` |
| `VYOMA_AX:node_value:<id>,<v>` | App → Supervisor | Node value (may be long) |
| `VYOMA_AX:node_desc:<id>,<d>` | App → Supervisor | Node description |
| `VYOMA_AX:end_tree` | App → Supervisor | Commit tree replace |
| `VYOMA_AX:update_node:<id>,<field>,<v>` | App → Supervisor | Patch single node field |
| `VYOMA_AX:focused_node:<id>` | App → Supervisor | Report intra-app focused node |
| `VYOMA_AX:remove_node:<id>` | App → Supervisor | Remove a node incrementally |
| `VYOMA_AX:announce:<priority>,<text>` | App → Supervisor | Request live-region announcement |
| `VYOMA_AX_EVENT:focus_changed:...` | Supervisor → AX client | Focus moved |
| `VYOMA_AX_EVENT:element_changed:...` | Supervisor → AX client | Node attribute changed |
| `VYOMA_AX_EVENT:tree_changed:...` | Supervisor → AX client | Full tree replaced |
| `VYOMA_AX_EVENT:app_appeared:<name>` | Supervisor → AX client | App joined registry |
| `VYOMA_AX_EVENT:app_disappeared:<name>` | Supervisor → AX client | App left registry |
| `VYOMA_AX_EVENT:announcement:<app>,<text>` | Supervisor → AX client | Live-region text |

---

## 11. Platform Matrix

| Platform | AX Registry | AX Client | VoiceOver stub | Tab nav |
|----------|-------------|-----------|----------------|---------|
| `desktop-full` | enabled | enabled | optional | app-owned |
| `mobile` | enabled | enabled | optional | app-owned |
| `server-headless` | disabled | disabled | n/a | n/a |
| `iot-edge` | disabled | disabled | n/a | n/a |
| `robotics-rt` | disabled | disabled | n/a | n/a |
| `mcu-minimal` | disabled | disabled | n/a | n/a |

On headless/embedded platforms, `VYOMA_AX:` lines from apps are silently discarded by
supervisor (no registry allocated). `accessibility = true` capability is invalid on those
profiles (manifest validator emits a warning, capability is stripped).

---

## 12. File Layout

```
supervisor/src/
├── accessibility/
│   ├── mod.rs           (re-exports; platform gate: #[cfg(feature = "gui")])
│   ├── tree.rs          (AXNode, AXRole, AXStateFlags, AXAction, AppAXTree — ≤ 300 lines)
│   ├── registry.rs      (AXRegistry, pending_ax_removals drain — ≤ 250 lines)
│   ├── parser.rs        (VYOMA_AX: line parser, begin/end_tree state machine — ≤ 200 lines)
│   ├── wit_host.rs      (init_linker_for_ax_client, WIT bindings — ≤ 250 lines)
│   └── events.rs        (dispatch VYOMA_AX_EVENT: to clients — ≤ 150 lines)

apps/voiceover/
├── Cargo.toml
├── vyoma.toml           (space=0, z=65531, display+stdio+accessibility)
└── src/
    └── main.rs          (stdin event loop, WIT get-node query, VYOMA_DRAW caption — ≤ 300 lines)

wit/
└── world.wit            (accessibility-client interface addition)
```

All files respect the 500-line limit. `tree.rs` is the largest at ~300 lines (node struct,
state flags, focusable traversal).

---

## 13. `SupervisorState` Additions

```rust
// supervisor/src/main.rs (or state.rs)

pub ax_registry: Arc<RwLock<AXRegistry>>,
pub pending_ax_removals: Vec<String>,     // drained in compositor tick, post apps-map release
```

`AXRegistry` is initialized empty at boot. No disk persistence — apps re-publish their AX
trees each time they start.

---

## 14. Compositor Tick Integration

In the compositor tick (after draining `pending_focus_notifications` per R28):

```rust
// Phase 3 of compositor tick: drain AX removals and dispatch AX events

// 3a. Drain pending_ax_removals
{
    let removals = std::mem::take(&mut state.pending_ax_removals);
    if !removals.is_empty() {
        let mut reg = state.ax_registry.write().unwrap();
        for name in &removals {
            reg.trees.remove(name);
            reg.global_revision += 1;
        }
        // Deliver VYOMA_AX_EVENT:app_disappeared to clients (outside write lock)
        let clients = reg.clients.clone();
        drop(reg);
        for name in &removals {
            for client in &clients {
                send_ax_event(state, client,
                    &format!("VYOMA_AX_EVENT:app_disappeared:{name}"));
            }
        }
    }
}

// 3b. Drain pending_ax_events (focus notifications already converted to AX events
//     in handle_focus_change)
{
    let events = std::mem::take(&mut state.pending_ax_events);
    let reg = state.ax_registry.read().unwrap();
    let clients = reg.clients.clone();
    drop(reg);
    for (client, msg) in events {
        send_to_stdin(state, &client, &msg);
    }
}
```

`pending_ax_events: Vec<(String, String)>` — (client_name, event_line) — accumulated
during the tick and drained at phase 3b. This avoids acquiring `ax_registry.read()` inside
the `apps_map` write region.

---

## 15. AX Tree Parse State Machine

The parser must handle multi-line `begin_tree`/`end_tree` blocks. Each app context carries
parse state:

```rust
// supervisor/src/accessibility/parser.rs

pub enum AXParseState {
    Idle,
    InTree { revision: u64, partial: AppAXTree },
}
```

`handle_app_output` for a line starting with `VYOMA_AX:`:
1. If `begin_tree` → transition to `InTree`.
2. If `node`/`node_value`/`node_desc` while `InTree` → accumulate into `partial`.
3. If `end_tree` while `InTree` → write-lock registry, swap tree, bump `global_revision`,
   enqueue `tree_changed` event, transition to `Idle`.
4. If `update_node`/`focused_node`/`remove_node` while `Idle` → write-lock registry,
   apply patch in-place, bump `global_revision`, enqueue `element_changed` or
   `focus_changed` event.
5. Malformed lines: log warning (`[ax] parse error in <app>: <line>`), skip, no state
   change.

The `partial` tree in `InTree` state is private to the parse state and not visible to AX
clients until `end_tree` commits it. This ensures clients never see a half-built tree.

---

## 16. Wire Format Encoding Details

### 16.1 Role Encoding

Roles are transmitted as lowercase snake_case strings matching enum variant names:
`window`, `button`, `check_box`, `radio_button`, `text_input`, `text_area`, `label`,
`static_text`, `image`, `link`, `list`, `list_item`, `menu`, `menu_item`, `menu_bar`,
`toolbar`, `tab_group`, `tab`, `slider`, `progress_indicator`, `scroll_area`,
`scroll_bar`, `table`, `row`, `cell`, `heading`, `group`, `dialog`, `alert`, `unknown`.

Unknown role strings are parsed as `AXRole::Unknown` (no parse error).

### 16.2 Actions Encoding

Actions bitmask (u8):
- bit 0: Press
- bit 1: Focus
- bit 2: SetValue
- bit 3: ScrollUp
- bit 4: ScrollDown
- bit 5: ShowMenu
- bit 6: Expand
- bit 7: Collapse

### 16.3 Bounds

App-local pixel coordinates (origin at app window top-left). The supervisor does NOT
translate to screen coordinates in the registry — AX clients that need screen coordinates
add the app window origin (available from the compositor's `AppState.position` field).

This keeps the AX tree stable across window moves without requiring a full tree republish.

---

## 17. Error Handling and Robustness

**Apps that never publish an AX tree**: The supervisor creates an empty `AppAXTree` entry
on app spawn (with `revision = 0`). AX clients can still query the app (get empty list).

**Apps that crash mid-tree publish**: If an app crashes while `InTree` state is active, the
partial tree is discarded. The `on_app_terminated` path calls
`clear_ax_parse_state(app_name)` before queuing the removal.

**AX client app crash**: Removed from `registry.clients` via `on_app_terminated` +
`pending_ax_removals`. No further events dispatched to it.

**Large trees (> 500 nodes)**: No hard limit imposed by supervisor. The line protocol is
streaming. A 500-node tree at ~100 bytes/node = ~50 KB total — acceptable for a single
begin/end_tree block. Apps are responsible for keeping trees small; degenerate cases are a
quality-of-implementation concern for the app, not a supervisor invariant.

---

## 18. Risk Table

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `ax_registry.write()` contention during tree publish blocking IPC thread | Low | write() only held for HashMap update (~microseconds); no I/O inside lock |
| ABBA deadlock: apps_map + ax_registry | Medium | strict ordering enforced; pending_ax_removals pattern |
| AX client leaking password values via `get-node` | Medium | `SECURE_INPUT` state flag + app responsibility; capability gate on client |
| Apps publishing stale trees after suspend | Low | `AppAXTree.revision` allows client to detect staleness; client re-syncs on `app_appeared` |
| VoiceOver caption bar obscuring content | Low | z=65531 < dock(65534); user can disable voiceover via `@supervisor: kill voiceover` |
