# Round 3 — Inter-Process Communication (IPC)

**Role:** Architect
**Date:** 2026-05-29
**Status:** Proposal (awaiting Critic round)
**Subsystem:** Supervisor-brokered IPC for `wasm32-wasip2` apps under Wasmtime PID-1

---

## 0. Executive Summary

VyomaOS IPC is the spinal cord of the user-facing OS: every clipboard, drag,
notification, window event, XPC-style RPC, broadcast, system command, and
zero-copy frame buffer hand-off flows through it. It must subsume the roles
that on macOS are split across Mach ports, XPC, NSDistributedNotification,
Apple Events, Pasteboard, and `launchctl`, while running on a stack that has
**none of those primitives** available — only:

- A Linux 5.10 kernel with virtio + 9P + DRM (no Mach, no Binder, no D-Bus).
- A single PID-1 Rust supervisor (`supervisor`) that embeds Wasmtime as a
  library and instantiates every WASM app **in-process** as a `Store`.
- WIT-typed component model boundaries (`wasm32-wasip2`).
- 500-line file limit on every `.rs` file (constitutional rule).

The design below resolves these constraints by making the **supervisor the
sole broker**: there is no app-to-app socket, no shared kernel object between
apps, and no addressable endpoint outside the supervisor's process space. All
addressing is *logical* (bundle ID, instance ID, focused, broadcast,
supervisor). All physical delivery is `crossbeam::channel::Sender<IpcEnvelope>`
to a per-app inbox, drained by that app's `CallbackDispatcher` (defined in
Round 1, §4) which invokes `on-ipc` on the WASM instance.

The supervisor-brokered star topology turns out to be the *only* correct
choice for a WASM-capability OS: capability-secure WASM forbids ambient
authority, but a peer-to-peer socket *is* ambient authority (any process with
the FD can speak). Forcing every message through the broker means every
delivery is gated by an in-memory capability check the broker performs
against the sender's manifest. This is the same architectural invariant that
makes seL4 endpoint capabilities safe, expressed in user space.

Round 1 nailed down: `AppIdentity`, `InstanceId`, sharded `AppTable`, WIT
`on-ipc` callback, `IpcEnvelope`, `IpcTarget`, `IpcClass`, per-class bounded
channel caps. Round 2 nailed down: `SharedBuffer`, `SharedBufferKind::Surface`
aliasing, `VYOMA_SHM` protocol, `vyoma:memory@0.1.0`, suspension grant.
Round 3 (this document) builds the **routing, request-reply, broadcast,
supervisor-command, security, and WIT-typed-message** layers that turn those
primitives into a full XPC + Apple Events + NSDistributedNotification
replacement.

---

## 1. IPC Topology

### 1.1 Why supervisor-brokered, not peer-to-peer

Three constraints fully determine the topology:

**Constraint A — WASM capability security.** A `wasm32-wasip2` instance has
*no* ambient authority. The only things it can do are the imports its
`world` declares. If we hand it a raw FD or a pointer to a peer's channel,
we have just granted ambient authority — that FD/pointer can be passed,
forged in some host-bug case, or kept after the supposed revocation. The
*only* way to keep capability semantics intact is to make every send go
through a host function (a WIT import) whose host implementation
re-validates the sender's right to talk to the recipient on every call.

**Constraint B — Single-process supervisor.** Apps are not subprocesses;
they are Wasmtime `Store`s in the supervisor's address space. There *is no
kernel object* between them. The only thing that exists between two apps is
Rust code in the supervisor. A "peer-to-peer" channel between two WASM
instances would still be a `crossbeam::channel::Sender` held by the
supervisor on behalf of both ends, plus a routing decision made by the
supervisor at every send. That *is* brokered IPC, just hidden.

**Constraint C — Lifecycle authority.** Suspension, resume, kill, restart,
and watchdog escalation all live in the supervisor. An app's inbox can be
swapped out from under it on resume from snapshot; a "peer" channel
established before suspension would be dangling on the wakeup side.
Brokered IPC means the address (`InstanceId`) is resolved at *delivery*
time, not at *connection* time, so post-resume continuity is automatic.

Conclusion: **all IPC is brokered; there are no app-held channel endpoints
pointing at other apps**. The broker (the `IpcRouter` actor of §3) holds
every `Sender<IpcEnvelope>` and resolves every address.

### 1.2 Logical topology diagram

```
                      ┌────────────────────────────────┐
                      │   IpcRouter (single actor)     │
                      │   - routing table              │
                      │   - reply correlator           │
                      │   - subscription table         │
                      │   - capability matrix          │
                      └──┬──────────┬──────────┬───────┘
                         │          │          │
              .. tx.send(env) .. tx.send(env) .. tx.send(env) ..
                         │          │          │
              ┌──────────▼─┐  ┌─────▼─────┐  ┌─▼─────────┐
              │ App A inbox│  │App B inbox│  │App C inbox│
              │  (bounded) │  │  (bounded)│  │  (bounded)│
              └──────────┬─┘  └─────┬─────┘  └─┬─────────┘
                         │          │          │
              CallbackDispatcher  CallbackDispatcher  CallbackDispatcher
                  ↓ call_on_ipc       ↓                  ↓
              [Wasmtime Store A]  [Wasmtime Store B]  [Wasmtime Store C]
```

No arrow ever skips the central node. The single hop is enforced by the WIT
import: `vyoma:ipc/client.send(addr, msg)` has only the router as its host
implementation.

### 1.3 Connection lifecycle

Because there is no "connection", lifecycle is per-message. But for
**request-reply** (§4) and **subscriptions** (§5) we do hold per-message
soft state. The three lifecycle events the broker observes:

| Event | What happens in the router |
|---|---|
| `open` (implicit) | First send registers the sender's outbox latency credit; first subscribe registers the topic FD; supervisor performs capability check once and caches the decision keyed by `(sender_bundle, recipient_bundle, channel_class)` in a 1 MiB LRU `dashmap::DashMap`. |
| `message` | Capability cache lookup, route, enqueue. If `reply_to` is set, register `PendingReply` with deadline. |
| `close` | Explicit `disconnect(addr)` or instance death. On instance death the `AppTable::remove` hook fires `IpcRouter::on_instance_gone(id)` which (a) drops `Sender<IpcEnvelope>` for that inbox, (b) reaps every `PendingReply` whose target was this instance with `Err(RecipientGone)`, (c) cancels every subscription this instance held, (d) sends a `vyoma.lifecycle.died` broadcast on the system bus, (e) invalidates capability cache entries naming this bundle. |

### 1.4 Namespaces

Three address namespaces share the same `@`-prefixed grammar but are routed
through different policy paths:

| Namespace | Prefix | Policy |
|---|---|---|
| **User IPC** | `@<bundle_id>:` or `@<bundle_id>#<iid>:` | Subject to manifest `[capabilities.ipc.allow]` allowlist (§9). Per-class bounded channel. |
| **System IPC (events / topics)** | `@@vyoma.<dotted.topic>` | Subscriber model (§5). Capability `[capabilities.events.subscribe] = ["vyoma.focus.*"]` required. |
| **Supervisor commands** | `@supervisor:` | RPC into the supervisor itself (§6). Requires elevated entitlement `[entitlements.supervisor]`. |
| **Focused proxy** | `@@focused:` | Resolves to the current focused instance via `InputDispatcher::focused()`. Sender still needs `[capabilities.ipc.allow] = ["*"]` or membership in the focused app's `accepts_from` set. |
| **Broadcast** | `@@broadcast.<topic>:` | Topic publish (§5). |

These four prefix forms (`@`, `@@`, `@@`, `@supervisor`) exhaust the
grammar; anything else is a parse error.

---

## 2. Message Types & Envelope

### 2.1 Envelope (Round-1 type, extended)

Round 1 defined `IpcEnvelope` minimally. Round 3 extends it with the fields
needed for request-reply, deadlines, flags, and structured payloads, while
keeping it `Clone`-cheap (all heavy fields are `bytes::Bytes` or `Arc`):

```rust
//! supervisor/src/ipc/envelope.rs   (~280 lines)

use std::num::NonZeroU64;
use bytes::Bytes;
use crate::ipc::addr::IpcAddr;

#[derive(Debug, Clone)]
pub struct IpcEnvelope {
    /// Monotonic, supervisor-assigned. 0 reserved for "not assigned yet";
    /// any non-zero id is unique across one supervisor uptime.
    pub id:        u64,
    /// Set by the router from the authenticated sender; apps cannot lie (§9).
    pub sender:    IpcAddr,
    /// As provided by the sender; resolved logically at delivery time.
    pub recipient: IpcAddr,
    /// What's in the envelope. Tagged union over four physical forms.
    pub payload:   IpcPayload,
    /// If this message is a reply, the `id` of the original request.
    /// If this message expects a reply, `None` here and `flags.expects_reply`.
    pub reply_to:  Option<u64>,
    /// CLOCK_MONOTONIC ms since boot at which the message expires.
    /// `None` → no deadline (default for fire-and-forget).
    pub deadline_ms: Option<u64>,
    /// Per-message lane / priority / hint bits.
    pub flags:     IpcFlags,
    /// CPU billing class (Round 1 §3.2 overflow policy).
    pub class:     IpcClass,
}

#[derive(Debug, Clone)]
pub enum IpcPayload {
    /// Legacy stdout line protocol (`println!("@pong: hello")`).
    /// Preserved verbatim for printf demos and shell scripts.
    Raw(Bytes),

    /// WIT-typed structured message (the modern path).
    Typed(TypedMessage),

    /// Zero-copy handle to a Round-2 `SharedBuffer`. The router does NOT
    /// dereference; it merely passes the handle. Reader-attach (§7) is the
    /// SHM registry's job.
    SharedMem(SharedMemRef),

    /// Capability passing: the sender hands a host-side resource (file,
    /// socket, surface, sub-window) to the recipient. The router converts
    /// this to a WIT `resource` handle in the recipient's store on
    /// delivery. The sender's handle is invalidated by `flags.move`.
    Capability(VyomaResource),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IpcFlags {
    /// Sender wants a reply. Router will set up a `PendingReply` row.
    pub expects_reply: bool,
    /// If recipient inbox full, NACK rather than queue/drop.
    pub no_queue: bool,
    /// Move semantics: sender loses the embedded capability/handle.
    pub r#move: bool,
    /// System-priority lane; only supervisor and entitled apps may set.
    pub priority: Priority,
    /// Reserved bits — must be zero in v1.
    pub _reserved: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Priority {
    #[default] Normal,
    /// Input lane: keystrokes, mouse — preempts Normal.
    Input,
    /// System lane: lifecycle events, focus, OOM warnings.
    System,
    /// Supervisor-only.
    Critical,
}

/// Already in Round 1; restated here for completeness.
#[derive(Debug, Clone, Copy)]
pub enum IpcClass { Input, Ipc, Render, Control }
```

#### Wire-size budget

| Field | Bytes |
|---|---|
| `id`, `reply_to`, `deadline_ms` | 24 |
| `sender`, `recipient` (`IpcAddr` is `(u8 tag, NonZeroU32, [u8; 24])`) | 64 |
| `flags`, `class` | 8 |
| `payload` (Bytes pointer pair + tag) | 32 |
| **Total** | **128** |

128 bytes per envelope, all `Clone` ops are pointer bumps. A 1 MiB envelope
buffer holds ~8 000 outstanding messages — comfortably above the per-app
inbox depth of 256.

### 2.2 `TypedMessage` — the XPC-equivalent

```rust
//! Part of supervisor/src/ipc/envelope.rs

#[derive(Debug, Clone)]
pub struct TypedMessage {
    /// Reverse-DNS dotted name identifying the schema, e.g.
    /// "os.vyoma.notes.create-note" or "os.vyoma.intent.open-url".
    pub schema_id: SchemaId,
    /// CBOR-encoded body matching the schema. CBOR chosen over JSON for
    /// (a) compact size, (b) preserves byte strings, (c) wit-bindgen-cbor
    /// integration. Capped at 256 KiB (system) / 64 KiB (user) per send;
    /// larger blobs MUST go via `IpcPayload::SharedMem`.
    pub body: Bytes,
}

/// Interned, hash-prefixed schema name. We keep the full string for debug
/// and the 8-byte hash for fast comparison and capability matching.
#[derive(Debug, Clone)]
pub struct SchemaId {
    pub name: Arc<str>,           // e.g. "os.vyoma.intent.open-url"
    pub hash: [u8; 8],            // SipHash-2-4 of `name`
}
```

**WIT typing maps to runtime dispatch** like so: each `vyoma:*` package can
declare an `interface ipc-message` whose `record`/`variant` cases are the
schemas. `wit-bindgen` codegens a Rust serde adapter; the sender calls
`my_app::send("os.vyoma.intent.open-url", IntentOpenUrl { url })` which the
host binds to `IpcClient::send_typed`. The host serializes via
`ciborium::ser::into_writer` on the way out and the recipient's
`on_ipc(envelope)` callback receives `Vec<u8>` plus the schema_id string;
their generated code does `ciborium::de::from_reader::<IntentOpenUrl>` and
dispatches to a typed handler. The host does *not* validate the body
shape — it cannot, the schema is private to the apps — but it *does*
validate that the recipient declares `[capabilities.ipc.schemas]` accepting
this `schema_id` (§9, allowlist by schema regex).

### 2.3 `SharedMemRef` and `VyomaResource`

```rust
//! Part of supervisor/src/ipc/envelope.rs

use crate::shm::{SharedBufferId, SharedBufferKind};

#[derive(Debug, Clone, Copy)]
pub struct SharedMemRef {
    pub id:      SharedBufferId,
    pub kind:    SharedBufferKind,
    /// What the recipient may do.
    pub access:  SharedAccess,
    /// Optional sub-range [offset, offset+len) within the buffer.
    pub offset:  u32,
    pub len:     u32,
}

#[derive(Debug, Clone, Copy)]
pub enum SharedAccess { Read, Write, ReadWrite }

#[derive(Debug, Clone)]
pub enum VyomaResource {
    /// Already-opened file handle in supervisor's VFS layer.
    File   { fd: VyomaFd,        access: FileAccess },
    /// A window/sub-window. Lets recipient draw INTO sender's surface.
    Surface{ surface_id: u64,    access: SurfaceAccess },
    /// An IPC port name reservation (for handing off a service endpoint).
    Port   { schema: SchemaId,   exclusive: bool },
    /// A notification channel subscription.
    Sub    { topic: TopicId },
}

#[derive(Debug, Clone, Copy)]
pub struct VyomaFd(pub NonZeroU64);

#[derive(Debug, Clone, Copy)]
pub enum FileAccess    { Read, Write, ReadWrite }

#[derive(Debug, Clone, Copy)]
pub enum SurfaceAccess { ReadOnly, WriteOnly }
```

`VyomaFd` is a supervisor-side opaque handle. The supervisor maintains a
`fd_table: DashMap<VyomaFd, FileBacking>` mapping the abstract handle to
the actual `std::fs::File` (or 9P channel). Sending a `VyomaResource::File`
in an envelope causes the router to:

1. Look up `FileBacking`.
2. Verify the sender currently holds the FD.
3. If `flags.move = true`, remove the FD from the sender's per-instance
   resource set; otherwise clone (dup) it.
4. Insert into the recipient's per-instance resource set.
5. Generate a fresh `VyomaFd` for the recipient (FDs are not portable across
   instances; each instance sees its own opaque ID).
6. Rewrite the envelope's `Capability` payload to reference the new FD,
   then deliver.

The recipient's WIT bindings translate the FD into a `vyoma:vfs/file`
resource handle. This is the WASM-native equivalent of macOS `xpc_fd`.

---

## 3. `IpcRouter` Actor

### 3.1 Actor layout

The router is a single Rust task. It owns the routing table and the reply
correlator; it does **not** own the per-instance inboxes (each `AppHandle`
in the `AppTable` already owns its inbox `Sender`). The router *clones*
that `Sender` once at instance birth and stores it in `routes`.

```rust
//! supervisor/src/ipc/router.rs   (~480 lines)

use std::sync::Arc;
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

use crossbeam::channel::{bounded, Receiver, Sender, TrySendError};
use dashmap::DashMap;
use parking_lot::Mutex;

use crate::ipc::addr::{IpcAddr, ResolveError};
use crate::ipc::envelope::{IpcEnvelope, IpcPayload, IpcClass, Priority};
use crate::ipc::reply::{PendingReply, ReplyMailbox};
use crate::ipc::broadcast::{TopicTable, TopicId};
use crate::ipc::supervisor_cmd::SupervisorCmdHandler;
use crate::instance::{AppTable, InstanceId, BundleId, AppHandle};

pub struct IpcRouter {
    /// Inbound: every WIT `send()` call enqueues here.
    inbound:    Receiver<RouterCmd>,
    /// One sender per supervisor subsystem that emits IPC.
    inbound_tx: Sender<RouterCmd>,

    /// Resolved (InstanceId → Sender<IpcEnvelope>) routing table.
    routes:     DashMap<InstanceId, RouteEntry>,

    /// Bundle → SmallVec<InstanceId> lookup, populated from AppTable on
    /// instance start; kept warm on resume.
    by_bundle:  DashMap<BundleId, smallvec::SmallVec<[InstanceId; 2]>>,

    /// Pending request-reply correlation: id → ReplyMailbox.
    pending:    DashMap<u64, PendingReply>,
    /// Monotonic envelope-id generator.
    next_id:    std::sync::atomic::AtomicU64,

    /// Topic table for broadcast / subscriptions.
    topics:     TopicTable,

    /// `(sender_bundle, recipient_bundle, class) → CapDecision` LRU.
    cap_cache:  parking_lot::Mutex<lru::LruCache<CapKey, CapDecision>>,

    /// AppTable handle for membership lookups.
    app_table:  Arc<AppTable>,

    /// Supervisor command sub-actor.
    supcmd:     SupervisorCmdHandler,
}

#[derive(Debug)]
pub struct RouteEntry {
    pub inbox:        Sender<IpcEnvelope>,
    pub bundle:       BundleId,
    pub manifest_rev: u32,         // bump on hot-reload; invalidates cache
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct CapKey {
    sender_bundle:    BundleId,
    recipient_bundle: BundleId,
    class:            IpcClass,
}

#[derive(Clone, Copy)]
enum CapDecision { Allow, Deny }

/// Commands the router accepts from anywhere in the supervisor.
pub enum RouterCmd {
    /// WIT `send()` from an app. Already capability-checked at the import
    /// boundary against `sender`'s manifest? NO — that check happens here
    /// so the manifest revision can be observed atomically.
    Send(IpcEnvelope),

    /// AppTable lifecycle hook: an instance was created.
    InstanceOnline {
        id:       InstanceId,
        bundle:   BundleId,
        inbox:    Sender<IpcEnvelope>,
        rev:      u32,
    },

    /// AppTable lifecycle hook: an instance died.
    InstanceGone { id: InstanceId },

    /// Manifest hot-reload (bumps `manifest_rev`, invalidates cache).
    ManifestReloaded { bundle: BundleId, rev: u32 },

    /// Periodic 100 ms tick — expires PendingReply entries and processes
    /// the deadline wheel.
    Tick,
}
```

### 3.2 Main loop

The router runs on a dedicated OS thread (NOT a `tokio` task — supervisor
is sync-first by Round-1 decree). It is the only writer to `pending` and
the only writer to `routes`. Readers (other supervisor subsystems doing
introspection) use `DashMap`'s lock-free reads.

```rust
impl IpcRouter {
    pub fn run(mut self) {
        let ticker = crossbeam::channel::tick(Duration::from_millis(100));
        loop {
            crossbeam::select! {
                recv(self.inbound) -> cmd => match cmd {
                    Ok(c) => self.handle_cmd(c),
                    Err(_) => return,  // supervisor shutdown
                },
                recv(ticker) -> _ => self.handle_tick(),
            }
        }
    }

    fn handle_cmd(&mut self, cmd: RouterCmd) {
        match cmd {
            RouterCmd::Send(env) => {
                if let Err(e) = self.route(env) {
                    tracing::warn!(error = ?e, "ipc.route.fail");
                }
            }
            RouterCmd::InstanceOnline { id, bundle, inbox, rev } => {
                self.routes.insert(id, RouteEntry { inbox, bundle: bundle.clone(), manifest_rev: rev });
                self.by_bundle.entry(bundle).or_default().push(id);
            }
            RouterCmd::InstanceGone { id } => self.on_instance_gone(id),
            RouterCmd::ManifestReloaded { bundle, rev } => {
                if let Some(mut e) = self.routes.iter_mut().find(|e| e.bundle == bundle) {
                    e.manifest_rev = rev;
                }
                self.cap_cache.lock().clear();   // simplest correct invalidation
            }
            RouterCmd::Tick => self.handle_tick(),
        }
    }

    fn route(&mut self, mut env: IpcEnvelope) -> Result<(), RouteError> {
        env.id = self.alloc_id();

        // (1) Capability check — manifest_rev-stamped cache.
        let sender_bundle = match env.sender.bundle(&self.app_table) {
            Some(b) => b,
            None    => return Err(RouteError::UnknownSender),
        };
        let recipients = self.resolve(&env.recipient)?;
        for rid in &recipients {
            let rb = self.routes.get(rid).map(|e| e.bundle.clone());
            let Some(recipient_bundle) = rb else { continue };
            self.check_cap(&sender_bundle, &recipient_bundle, env.class)?;
        }

        // (2) Register pending reply if requested.
        if env.flags.expects_reply {
            let mailbox = ReplyMailbox::new();
            self.pending.insert(env.id, PendingReply {
                id:        env.id,
                sender:    env.sender.clone(),
                recipient: env.recipient.clone(),
                deadline:  env.deadline_ms.map(Instant::from_boot_ms).unwrap_or_else(|| Instant::now() + Duration::from_secs(30)),
                mailbox,
            });
        }

        // (3) If it's a reply, satisfy the matching PendingReply and DROP
        // (do not redeliver) — the original sender is awaiting it.
        if let Some(orig) = env.reply_to {
            if let Some((_, pending)) = self.pending.remove(&orig) {
                pending.mailbox.fulfill(env);
                return Ok(());
            }
            return Err(RouteError::OrphanReply);
        }

        // (4) Physical delivery.
        for rid in recipients {
            self.deliver(rid, env.clone())?;
        }
        Ok(())
    }

    fn deliver(&self, rid: InstanceId, env: IpcEnvelope) -> Result<(), RouteError> {
        let Some(route) = self.routes.get(&rid) else {
            return Err(RouteError::RecipientGone);
        };
        match route.inbox.try_send(env.clone()) {
            Ok(())                            => Ok(()),
            Err(TrySendError::Full(env))      => self.on_full(rid, env),
            Err(TrySendError::Disconnected(_))=> Err(RouteError::RecipientGone),
        }
    }
}
```

### 3.3 Back-pressure: per-class policy

Round 1 §3.2 already specified the four IPC classes and their overflow
policies. Round 3 turns that table into code:

```rust
impl IpcRouter {
    fn on_full(&self, rid: InstanceId, env: IpcEnvelope) -> Result<(), RouteError> {
        match env.class {
            // Input: drop-oldest. Pop the oldest message off the inbox by
            // temporarily downgrading to mpsc — done by holding a per-inbox
            // mutex that lets us `try_recv` one element before re-sending.
            IpcClass::Input => {
                let Some(route) = self.routes.get(&rid) else { return Err(RouteError::RecipientGone) };
                // Inbox is crossbeam-bounded; no atomic pop. We rely on the
                // CallbackDispatcher to drain quickly; if it's truly stuck,
                // drop the new event (NOT the old — preserves typed sequence).
                tracing::warn!(?rid, "input.drop");
                Ok(())
            }
            // Ipc: NACK to sender as a reply with `IpcError::QueueFull`.
            IpcClass::Ipc => {
                self.send_nack(&env, IpcError::QueueFull);
                Ok(())
            }
            // Render: coalesced upstream by the compositor — by the time
            // a Render message reaches the router, it's already been folded.
            // If it still arrives full, log and drop.
            IpcClass::Render => {
                tracing::warn!(?rid, "render.full.drop");
                Ok(())
            }
            // Control: block up to 50 ms, then error.
            IpcClass::Control => {
                let Some(route) = self.routes.get(&rid) else { return Err(RouteError::RecipientGone) };
                match route.inbox.send_timeout(env, Duration::from_millis(50)) {
                    Ok(()) => Ok(()),
                    Err(_) => Err(RouteError::ControlBlocked),
                }
            }
        }
    }
}
```

### 3.4 Priority lanes

`IpcFlags::priority` is a *hint* — the router has only one inbox per
instance, not four. The lane mechanism is the *scheduling order in the
`CallbackDispatcher`*: when the dispatcher drains the inbox, it pulls all
`Critical`/`System` envelopes first, then `Input`, then `Normal`. To make
this efficient without scanning the channel, each app actually has **two
inboxes**: `inbox_hi` (cap 64, System+Critical) and `inbox_lo` (cap 256,
Normal+Input). The dispatcher's `select!` strictly favors `inbox_hi`:

```rust
fn drain(&self) {
    loop {
        crossbeam::select_biased! {
            recv(self.inbox_hi) -> env => self.dispatch(env),
            recv(self.inbox_lo) -> env => self.dispatch(env),
            default(Duration::from_millis(50)) => return,
        }
    }
}
```

The router's `deliver` picks the lane from `env.flags.priority`. This adds
exactly one branch per send; channel count doubles but bounded depth on
`inbox_hi` is small (64 × 128 bytes = 8 KiB per app).

### 3.5 `on_instance_gone` semantics

```rust
fn on_instance_gone(&mut self, id: InstanceId) {
    // 1. Drop route.
    let removed = self.routes.remove(&id);
    if let Some((_, RouteEntry { bundle, .. })) = removed {
        if let Some(mut v) = self.by_bundle.get_mut(&bundle) {
            v.retain(|x| *x != id);
        }
    }
    // 2. Reap pending replies whose recipient was us.
    let dead: Vec<u64> = self.pending.iter()
        .filter(|p| p.recipient.matches_instance(id))
        .map(|p| p.id).collect();
    for d in dead {
        if let Some((_, p)) = self.pending.remove(&d) {
            p.mailbox.fail(IpcError::RecipientGone);
        }
    }
    // 3. Unsubscribe from all topics.
    self.topics.drop_subscriber(id);
    // 4. Emit system event.
    let _ = self.inbound_tx.try_send(RouterCmd::Send(IpcEnvelope::system_event(
        "vyoma.lifecycle.died",
        id,
    )));
    // 5. Cap-cache invalidation handled lazily by manifest_rev mismatch.
}
```

---

## 4. Request-Reply (XPC Equivalent)

### 4.1 The hard problem

A WASM app calls `vyoma:ipc/client.call(addr, msg) -> result<reply, error>`.
The host implementation must:

1. **NOT block the Wasmtime thread.** Wasmtime instances are single-threaded
   by design; blocking the host call function pauses the entire instance.
2. **Make the call look synchronous to WASM source code** (developer
   ergonomics; matches `xpc_connection_send_message_with_reply_sync`).
3. **Honor a deadline** — at most 30 s by default, configurable.
4. **Survive sender suspension** — if the sender is `Suspending` /
   `Suspended` mid-call, the reply must be re-routed on resume.

### 4.2 The solution: epoch-yielded async with reply mailbox

We use **Wasmtime's epoch interruption** combined with **asynchronous
fuel exhaustion**: the host call function is generated as `async` by
`wit-bindgen`, registered with `wasmtime::Linker::func_wrap_async`, and
returns a `Pin<Box<dyn Future>>`. While the future is pending, Wasmtime
yields back to the embedder, freeing the worker thread to service other
instances. When the future resolves, the dispatcher resumes the instance.

This is the *only* place in VyomaOS where we use `wasmtime`'s async
instance support. The rest of the supervisor stays sync.

```rust
//! supervisor/src/ipc/reply.rs   (~340 lines)

use std::sync::Arc;
use std::time::Instant;
use crossbeam::channel::{bounded, Sender, Receiver};
use crate::ipc::envelope::{IpcEnvelope, IpcPayload};
use crate::ipc::addr::IpcAddr;

pub struct PendingReply {
    pub id:        u64,
    pub sender:    IpcAddr,     // who called
    pub recipient: IpcAddr,     // who was called
    pub deadline:  Instant,
    pub mailbox:   ReplyMailbox,
}

#[derive(Clone)]
pub struct ReplyMailbox(Arc<ReplyMailboxInner>);

struct ReplyMailboxInner {
    /// Single-shot fulfillment.
    tx: Sender<Result<IpcEnvelope, IpcError>>,
    rx: Receiver<Result<IpcEnvelope, IpcError>>,
}

impl ReplyMailbox {
    pub fn new() -> Self {
        let (tx, rx) = bounded(1);
        Self(Arc::new(ReplyMailboxInner { tx, rx }))
    }
    pub fn fulfill(&self, env: IpcEnvelope)        { let _ = self.0.tx.try_send(Ok(env)); }
    pub fn fail(&self, e: IpcError)                { let _ = self.0.tx.try_send(Err(e)); }
    pub fn waiter(&self) -> ReplyWaiter            { ReplyWaiter { rx: self.0.rx.clone() } }
}

/// Future-ish handle the host call returns to wasmtime.
pub struct ReplyWaiter {
    rx: Receiver<Result<IpcEnvelope, IpcError>>,
}

impl std::future::Future for ReplyWaiter {
    type Output = Result<IpcEnvelope, IpcError>;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        match self.rx.try_recv() {
            Ok(r) => std::task::Poll::Ready(r),
            Err(crossbeam::channel::TryRecvError::Empty) => {
                // Bridge crossbeam to async by parking the waker on a
                // companion `tokio::sync::Notify` — see §4.3.
                std::task::Poll::Pending
            }
            Err(crossbeam::channel::TryRecvError::Disconnected) => {
                std::task::Poll::Ready(Err(IpcError::ChannelDisconnected))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum IpcError {
    NoSuchRecipient,
    NoCapability,
    QueueFull,
    ReplyTimeout,
    RecipientGone,
    ChannelDisconnected,
    SchemaUnsupported,
    PayloadTooLarge,
    Forbidden,
}
```

### 4.3 Bridging crossbeam to async

`crossbeam::channel` is sync-only; `wasmtime::Linker::func_wrap_async`
demands a `Future`. We bridge with a per-mailbox `tokio::sync::Notify`:

```rust
struct ReplyMailboxInner {
    tx:     Sender<Result<IpcEnvelope, IpcError>>,
    rx:     Receiver<Result<IpcEnvelope, IpcError>>,
    notify: tokio::sync::Notify,
}

impl ReplyMailbox {
    pub fn fulfill(&self, env: IpcEnvelope) {
        let _ = self.0.tx.try_send(Ok(env));
        self.0.notify.notify_one();
    }
}

impl std::future::Future for ReplyWaiter {
    type Output = Result<IpcEnvelope, IpcError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Ok(r) = self.rx.try_recv() {
            return Poll::Ready(r);
        }
        // Subscribe to the notify and then re-check to avoid the race.
        let fut = self.notify.notified();
        tokio::pin!(fut);
        match fut.as_mut().poll(cx) {
            Poll::Ready(()) => {
                match self.rx.try_recv() {
                    Ok(r) => Poll::Ready(r),
                    Err(_) => Poll::Pending,
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
```

Wasmtime's async support uses a `tokio` runtime under the hood (we'll
spawn a single-threaded `tokio::runtime::Runtime` with one worker per CPU
core, dedicated to IPC await). The cost per outstanding `call` is
~256 bytes (waker + notify slot); we cap concurrent outstanding calls per
instance at 32 (manifest field `[capabilities.ipc.max_inflight]`,
default 8). Apps that need more must use fire-and-forget `send` plus
explicit `on-ipc` correlation.

### 4.4 Reply timeout enforcement

`IpcRouter::handle_tick` (every 100 ms) walks the `pending` map and fails
any entry whose `deadline` has passed:

```rust
fn handle_tick(&mut self) {
    let now = Instant::now();
    let expired: Vec<u64> = self.pending.iter()
        .filter(|p| p.deadline <= now)
        .map(|p| p.id).collect();
    for id in expired {
        if let Some((_, p)) = self.pending.remove(&id) {
            p.mailbox.fail(IpcError::ReplyTimeout);
        }
    }
}
```

For < 1 ms latency targets (input lane), the 100 ms tick is fine — the
mailbox fulfills *directly* from `route()`, and the tick is only a
safety net for misbehaving recipients. Apps must not rely on tick
granularity for correctness; they must specify their own deadline.

### 4.5 Survive sender suspension

If the sender enters `Suspending` while a reply is in flight, the
lifecycle actor calls `IpcRouter::pause_pending_for(sender)` which marks
each `PendingReply` whose `sender == this_instance` as
`PendingState::ParkedForResume`. Replies that arrive during parking are
queued on the mailbox but the `notify` is *not* signaled. On `Resume`,
the lifecycle actor calls `IpcRouter::resume_pending_for(sender)` which
clears the park bit and notifies waiters; the WIT call effectively
"continues" across suspend with the reply value.

### 4.6 WIT-side ergonomics

```rust
// inside a WASM app, codegen'd by wit-bindgen
let reply: NoteCreateReply = vyoma::ipc::client::call(
    "@os.vyoma.notes:create-note",
    NoteCreateRequest { title: "Hi".into() },
)?;
```

The codegen wraps the WIT call in CBOR encoding and decoding; from the
app developer's perspective it looks like a sync RPC.

---

## 5. Broadcast & Notifications

### 5.1 Topic model

A *topic* is an interned reverse-DNS string. The system reserves
`vyoma.*` (e.g., `vyoma.focus.changed`, `vyoma.display.rotated`,
`vyoma.memory.warning`). Apps may publish `<bundle_id>.*` topics
(e.g., `os.vyoma.notes.note-created`).

```rust
//! supervisor/src/ipc/broadcast.rs   (~250 lines)

use dashmap::DashMap;
use smallvec::SmallVec;
use crate::ipc::envelope::{IpcEnvelope, IpcPayload};
use crate::instance::InstanceId;

pub struct TopicTable {
    /// Topic → exact subscribers.
    exact:    DashMap<TopicId, SmallVec<[InstanceId; 8]>>,
    /// Wildcard subscribers: ("vyoma.focus.*", [inst]).
    /// Walked linearly; max 256 wildcard subs system-wide.
    wildcard: parking_lot::RwLock<Vec<(TopicGlob, InstanceId)>>,
    /// Reverse map for fast un-subscribe on instance death.
    by_inst:  DashMap<InstanceId, SmallVec<[TopicId; 4]>>,
    /// SipHash-interned topic strings.
    intern:   DashMap<Arc<str>, TopicId>,
}

#[derive(Hash, Eq, PartialEq, Clone, Copy)]
pub struct TopicId(pub u64);     // SipHash-2-4 of the dotted name

#[derive(Clone)]
pub struct TopicGlob {
    pub head:    Arc<str>,        // "vyoma.focus."
    pub tail:    Option<Arc<str>>,// ".changed" or None
}

impl TopicTable {
    pub fn subscribe(&self, who: InstanceId, topic: &str) -> Result<TopicId, BroadcastError> {
        let tid = self.intern_topic(topic);
        self.exact.entry(tid).or_default().push(who);
        self.by_inst.entry(who).or_default().push(tid);
        Ok(tid)
    }
    pub fn subscribe_glob(&self, who: InstanceId, glob: &str) -> Result<(), BroadcastError> {
        let g = TopicGlob::parse(glob)?;
        self.wildcard.write().push((g, who));
        Ok(())
    }
    pub fn publish(&self, topic: &str, env_template: &IpcEnvelope, deliver: impl Fn(InstanceId, IpcEnvelope)) {
        let tid = self.intern_topic(topic);
        if let Some(v) = self.exact.get(&tid) {
            for id in v.iter() {
                let mut e = env_template.clone();
                e.recipient = IpcAddr::Instance(*id);
                deliver(*id, e);
            }
        }
        let wc = self.wildcard.read();
        for (g, id) in wc.iter() {
            if g.matches(topic) {
                let mut e = env_template.clone();
                e.recipient = IpcAddr::Instance(*id);
                deliver(*id, e);
            }
        }
    }
    pub fn drop_subscriber(&self, id: InstanceId) {
        if let Some((_, topics)) = self.by_inst.remove(&id) {
            for tid in topics {
                if let Some(mut v) = self.exact.get_mut(&tid) {
                    v.retain(|x| *x != id);
                }
            }
        }
        self.wildcard.write().retain(|(_, x)| *x != id);
    }
}
```

### 5.2 Delivery guarantees

| Topic class | Guarantee | Rationale |
|---|---|---|
| User app topic | **At-most-once.** Inbox full → drop with WARN log. | Notifications must not block publisher. |
| System `vyoma.*` topic | **At-least-once for subscribed, best-effort for wildcard.** Inbox full → block 5 ms then drop with ERROR. | System needs reliable focus / lifecycle events. |
| `vyoma.lifecycle.*` | **Special.** Persisted to a 32-entry ring per subscriber for replay across suspend/resume. | Apps must not miss a `died` event. |

There is no at-least-once for general broadcasts — that would require
acknowledgments and per-recipient retries, which would let one slow
subscriber back-pressure the publisher. Round 4 may revisit if pub-sub
becomes a bottleneck.

### 5.3 System events

| Topic | Payload | When |
|---|---|---|
| `vyoma.focus.changed`        | `FocusChanged { from: InstanceId, to: InstanceId }` | InputDispatcher swaps focus |
| `vyoma.display.rotated`      | `DisplayRotated { angle_deg: u16 }`                  | Display subsystem rotates |
| `vyoma.display.resolution`   | `Resolution { w: u32, h: u32, scale: u16 }`          | Multi-resolution change |
| `vyoma.memory.warning`       | `MemoryWarning { level: PressureLevel }`             | Round-2 memory governor |
| `vyoma.lifecycle.spawned`    | `Spawned { id, bundle }`                             | Instance added to AppTable |
| `vyoma.lifecycle.died`       | `Died { id, bundle, reason }`                        | Instance removed |
| `vyoma.lifecycle.suspended`  | `Suspended { id, bundle }`                           | App entered Suspended |
| `vyoma.lifecycle.resumed`    | `Resumed { id, bundle }`                             | App returned to Foreground |
| `vyoma.input.idle`           | `Idle { secs: u32 }`                                 | No input for N s |
| `vyoma.clipboard.changed`    | `ClipChanged { schemas: Vec<SchemaId> }`             | Pasteboard updated |
| `vyoma.network.status`       | `NetStatus { online: bool }`                         | virtio-net up/down |
| `vyoma.shutdown.requested`   | `Shutdown { grace_ms: u32 }`                         | Sysctl reboot/halt |

These are sent by various supervisor subsystems via the router's normal
`Send(IpcEnvelope)` path with `recipient = IpcAddr::Topic("vyoma.foo")`
and `sender = IpcAddr::Supervisor`. The router treats `IpcAddr::Topic`
specially: it routes through the `TopicTable::publish` fan-out.

---

## 6. Supervisor Command Channel

### 6.1 Grammar

`@supervisor: <verb> [<args>]`

Verbs (v1):

| Verb | Args | Effect |
|---|---|---|
| `list` | (none) | Reply with `Vec<(InstanceId, BundleId, LifecycleState)>` |
| `kill <bundle>` | bundle | Send `KillSignal::Term` to all instances of that bundle |
| `kill <bundle>#<iid>` | bundle, iid | Kill one instance |
| `focus <bundle>` | bundle | InputDispatcher swaps focus to oldest instance |
| `spawn <bundle> [args...]` | | Allocate new InstanceId, enqueue Launch |
| `restart <bundle>` | | Kill then re-spawn (single transaction) |
| `suspend <bundle>` | | Force into Suspending |
| `resume <bundle>` | | Force out of Suspended |
| `topic <op> <name>` | op ∈ pub,sub,unsub,list | Manage topic subscriptions |
| `manifest reload <bundle>` | | Re-read TOML, bump `manifest_rev` |
| `quota <bundle> <kv>` | e.g. `cpu_pct=20` | Hot-patch ResourceQuota |
| `log tail <bundle> [n]` | | Stream last N log lines |
| `crash list [n]` | | List recent crash reports |

### 6.2 Implementation

```rust
//! supervisor/src/ipc/supervisor_cmd.rs   (~360 lines)

use crate::ipc::envelope::{IpcEnvelope, IpcPayload, IpcError, TypedMessage};
use crate::ipc::addr::IpcAddr;
use crate::instance::{BundleId, InstanceId, AppTable};
use crate::lifecycle::LifecycleCmd;

pub struct SupervisorCmdHandler {
    app_table:        Arc<AppTable>,
    lifecycle_tx:     crossbeam::channel::Sender<LifecycleCmd>,
    auth:             EntitlementChecker,
}

#[derive(Debug)]
pub enum SupCmd {
    List,
    Kill        { bundle: BundleId, iid: Option<InstanceId> },
    Focus       { bundle: BundleId },
    Spawn       { bundle: BundleId, args: LaunchArgs },
    Restart     { bundle: BundleId },
    Suspend     { bundle: BundleId },
    Resume      { bundle: BundleId },
    ManifestReload { bundle: BundleId },
    QuotaSet    { bundle: BundleId, key: String, value: String },
    LogTail     { bundle: BundleId, n: u32 },
    CrashList   { n: u32 },
    TopicOp     { op: TopicOp, name: String },
}

#[derive(Debug)]
pub enum SupCmdReply {
    Ok(serde_json::Value),
    Err(String),
}

impl SupervisorCmdHandler {
    pub fn dispatch(&self, env: IpcEnvelope) -> Result<IpcEnvelope, IpcError> {
        let cmd = self.parse(&env)?;
        let sender_bundle = env.sender.bundle(&self.app_table).ok_or(IpcError::NoSuchRecipient)?;
        self.auth.check(&sender_bundle, &cmd)?;
        let result = self.execute(cmd)?;
        Ok(self.build_reply(&env, result))
    }

    fn parse(&self, env: &IpcEnvelope) -> Result<SupCmd, IpcError> {
        match &env.payload {
            IpcPayload::Raw(b)         => Self::parse_raw(b),                // legacy line
            IpcPayload::Typed(t)       => Self::parse_typed(t),              // CBOR record
            _                          => Err(IpcError::SchemaUnsupported),
        }
    }
    // ...
}
```

### 6.3 Auth & entitlements

Sending to `@supervisor:` requires `[entitlements.supervisor]` in the
sender's manifest. Subset entitlements:

| Entitlement | Allows |
|---|---|
| `supervisor.read`  | `list`, `log tail`, `crash list`, `topic list` |
| `supervisor.focus` | `focus`                                        |
| `supervisor.spawn` | `spawn` for bundles in `[entitlements.supervisor.spawn.allow]` |
| `supervisor.kill`  | `kill`, `suspend`, `resume` for bundles in allow list |
| `supervisor.admin` | All of the above + `manifest reload`, `quota`  |

`supervisor.admin` is **only** granted to:
1. The shell (`os.vyoma.shell`) when launched by the supervisor itself.
2. Apps installed by the `Trusted` install origin (signed by trust root).
3. Interactive elevation prompt (§6.4).

```rust
pub struct EntitlementChecker {
    table: DashMap<BundleId, Entitlements>,
}

#[derive(Debug, Clone, Default)]
pub struct Entitlements {
    pub read: bool,
    pub focus: bool,
    pub spawn_allow: Vec<BundleId>,
    pub kill_allow: Vec<BundleId>,
    pub admin: bool,
}

impl EntitlementChecker {
    pub fn check(&self, sender: &BundleId, cmd: &SupCmd) -> Result<(), IpcError> {
        let ent = self.table.get(sender).map(|e| e.clone()).unwrap_or_default();
        match cmd {
            SupCmd::List | SupCmd::LogTail { .. } | SupCmd::CrashList { .. } | SupCmd::TopicOp { .. }
                if ent.read || ent.admin => Ok(()),
            SupCmd::Focus { .. } if ent.focus || ent.admin => Ok(()),
            SupCmd::Spawn { bundle, .. }
                if ent.admin || ent.spawn_allow.contains(bundle) => Ok(()),
            SupCmd::Kill { bundle, .. } | SupCmd::Restart { bundle } |
            SupCmd::Suspend { bundle } | SupCmd::Resume { bundle }
                if ent.admin || ent.kill_allow.contains(bundle) => Ok(()),
            SupCmd::ManifestReload { .. } | SupCmd::QuotaSet { .. } if ent.admin => Ok(()),
            _ => Err(IpcError::Forbidden),
        }
    }
}
```

### 6.4 Privilege escalation prompt (sudo / UAC)

When a non-admin app needs an admin op, it calls
`vyoma:ipc/client.elevate(reason, cmd)`. The router routes this to a
**SecurityAgent** app (`os.vyoma.security-agent`, a special bundle
installed at build time, focus-priority MAX) which renders a modal dialog
asking the user to confirm. The user types their password (or biometric
on supported platforms), the agent verifies it against
`/data/secrets/auth.enc`, then sends `@supervisor: grant-once <bundle>
<cmd-hash>` which the supervisor records in an in-memory one-shot
allowlist (TTL 30 s). The original app retries the call within the
window and succeeds.

This entire flow is implemented in pure IPC; no special host privileges
required. The trust root is the SecurityAgent's bundle being signed by
the same trust root as the supervisor itself.

---

## 7. Shared Memory IPC

### 7.1 When to use SharedMem vs copy

```
Payload size       Recommended path
---------------------------------------------------
< 4 KiB            Typed CBOR in envelope (zero ceremony)
4 KiB – 256 KiB    Typed CBOR (CBOR caps at 256 KiB anyway)
> 256 KiB          SharedBuffer (mandatory; envelope rejected if not)
Video frames       SharedBufferKind::Surface (aliased)
Audio frames       SharedBufferKind::AudioBuffer
Image data         SharedBufferKind::ImageData (e.g., camera frames)
```

The router enforces this with `validate_payload_size` before queueing.

### 7.2 Lifecycle

Round 2 defined `SharedBufferRegistry`. Round 3 hooks it into IPC:

```rust
// Sender side (in app):
let buf = vyoma::memory::create_shared(SharedBufferKind::ImageData, 1_048_576)?;
buf.write(0, &pixels)?;
vyoma::ipc::client::send_with_shm(
    "@os.vyoma.image-viewer:open-image",
    OpenImage { width: 1024, height: 1024 },
    buf.handle(),  // attaches as IpcPayload::SharedMem
)?;
// Sender keeps buf alive until reply (or drops to relinquish).

// Recipient side (on-ipc):
fn on_ipc(env: Envelope) {
    if let Some(shm) = env.shared_mem() {
        let reader = vyoma::memory::attach_reader(shm.id)?;
        reader.read(shm.offset, shm.len, |bytes| {
            // process pixels
        });
        reader.detach();
    }
}
```

The router on delivery:
1. Calls `SharedBufferRegistry::attach_reader(buf_id, recipient_iid)`.
2. Increments refcount on the shared buffer.
3. If `flags.r#move = true`, detaches the sender's writer side and the
   buffer becomes recipient-owned.

When the recipient drops the reader (or dies), `on_instance_gone` cleans
up via `SharedBufferRegistry::detach_all(iid)`.

### 7.3 Concurrent access semantics

The Round-2 `SharedBufferStorage` is either `Owned(RwLock<Vec<u8>>)` or
`SurfaceAlias { surface, .. }`:

- **Owned**: multi-reader / single-writer via the `parking_lot::RwLock`.
  IPC envelope's `SharedAccess::Read` grants `read()` only; `Write` grants
  `write()` only; `ReadWrite` grants both. The router enforces these by
  embedding them in the WIT resource handle the recipient receives.
- **SurfaceAlias**: single-writer only (the surface owner). Readers (the
  compositor) read the back buffer directly under the surface's own
  double-buffer protocol; no IPC-layer locking needed.

### 7.4 Detach on instance death

```rust
impl IpcRouter {
    fn on_instance_gone(&mut self, id: InstanceId) {
        // ... (existing steps)
        self.shm_registry.detach_all(id);
        // SharedBufferRegistry decrements refcounts; when zero, frees.
    }
}
```

---

## 8. WIT Interface

### 8.1 Full `wit/vyoma-ipc.wit`

```wit
package vyoma:ipc@0.1.0;

/// Errors returned by the IPC layer.
variant ipc-error {
    no-such-recipient,
    no-capability,
    queue-full,
    reply-timeout,
    recipient-gone,
    channel-disconnected,
    schema-unsupported,
    payload-too-large(u32),
    forbidden,
    parse-error(string),
}

/// One end of a SharedBuffer attached to this app.
resource shm-ref {
    id:    func() -> u64;
    kind:  func() -> shm-kind;
    size:  func() -> u32;
    read:  func(offset: u32, len: u32) -> result<list<u8>, ipc-error>;
    write: func(offset: u32, data: list<u8>) -> result<_, ipc-error>;
    drop:  func();
}

enum shm-kind { surface, audio-buffer, image-data, custom }

/// Capability handles that may travel inside an envelope.
variant resource-handle {
    file(u64),         // opaque vyoma-fd
    surface(u64),
    port(string),
    sub(u64),
}

/// What this app sees when an envelope arrives.
record envelope {
    id:        u64,
    sender:    string,             // pretty form, e.g. "os.vyoma.notes#3"
    recipient: string,
    reply-to:  option<u64>,
    deadline-ms: option<u64>,
    schema:    option<string>,     // SchemaId.name; absent for Raw
    body:      list<u8>,           // CBOR (Typed) or raw bytes (Raw)
    shm:       option<shm-ref>,    // None unless payload was SharedMem
    capability: option<resource-handle>,
    priority:  priority-level,
}

enum priority-level { normal, input, system, critical }

/// Outbound API.
interface ipc-client {
    use ipc-error;
    use envelope;
    use resource-handle;
    use shm-ref;

    /// Fire-and-forget. Returns once the envelope is queued.
    send: func(recipient: string, schema: string, body: list<u8>) -> result<_, ipc-error>;

    /// Send with a shared-memory attachment.
    send-with-shm: func(
        recipient: string,
        schema: string,
        body: list<u8>,
        shm: shm-ref,
        access: shm-access,
    ) -> result<_, ipc-error>;

    /// Send a capability handle (file, surface, sub-window, port).
    send-with-cap: func(
        recipient: string,
        schema: string,
        body: list<u8>,
        cap: resource-handle,
        move-semantics: bool,
    ) -> result<_, ipc-error>;

    /// Request-reply. Returns when reply arrives or deadline elapses.
    call: func(
        recipient: string,
        schema: string,
        body: list<u8>,
        deadline-ms: option<u32>,
    ) -> result<envelope, ipc-error>;

    /// Broadcast publish.
    publish: func(topic: string, schema: string, body: list<u8>) -> result<_, ipc-error>;

    /// Subscribe to a topic (exact or glob ending in `*`).
    subscribe: func(topic: string) -> result<u64 /*sub-id*/, ipc-error>;
    unsubscribe: func(sub-id: u64) -> result<_, ipc-error>;

    /// Privilege escalation prompt — user-visible dialog.
    elevate: func(
        reason: string,
        verb: string,
        args: list<u8>,
    ) -> result<_, ipc-error>;

    /// Reply to a previously received envelope.
    reply: func(
        to: u64,                   // envelope.id of the request
        schema: string,
        body: list<u8>,
    ) -> result<_, ipc-error>;

    /// Reply with an error to a request.
    reply-error: func(
        to: u64,
        e: ipc-error,
    ) -> result<_, ipc-error>;
}

enum shm-access { read, write, read-write }

/// Inbound — the app exports these.
interface ipc-server {
    use envelope;

    /// Called by host once per envelope. Inbox drained in FIFO order
    /// (with high-priority lane preempting).
    on-ipc: func(env: envelope);

    /// Notification that a reply timed out before the recipient sent it.
    on-reply-timeout: func(req-id: u64);

    /// Topic subscription delivered.
    on-broadcast: func(topic: string, env: envelope);
}

/// The single world an app may target.
world vyoma-app {
    import ipc-client;
    export ipc-server;
    import vyoma:memory/storage@0.1.0;
    import vyoma:lifecycle/callbacks;
}
```

### 8.2 Why CBOR over Protobuf or MessagePack

| Property | CBOR | Protobuf | MessagePack |
|---|---|---|---|
| Schema-optional | yes | NO (.proto required at runtime) | yes |
| Byte-string preserving | yes | yes | yes |
| `wit-bindgen` support | yes (ciborium) | requires .proto codegen | partial |
| Compact for primitives | yes | yes | yes |
| Required reserved bytes | low | medium | low |
| WASM binary size cost | ~25 KiB | ~120 KiB | ~30 KiB |

CBOR wins on schema flexibility (we don't ship a global registry of every
app's IPC schemas) and WASM footprint.

---

## 9. Security

### 9.1 The capability matrix

A sender X may send to recipient Y if and only if Y's manifest grants:

```toml
[capabilities.ipc.allow]
# Glob patterns over bundle IDs.
from = [
  "os.vyoma.notes",      # exact bundle
  "os.vyoma.studio.*",   # any matching glob
  "*",                   # anyone (rare; loud at install time)
]
# Optional schema allowlist; if absent, any schema is accepted.
schemas = [
  "os.vyoma.intent.open-url",
  "os.vyoma.notes.*",
]
# Max envelope rate (per sender) — protects against abuse.
rate_limit_per_sec = 100
```

Sender X's manifest must also declare:

```toml
[capabilities.ipc]
declare = true                 # opt in to having an inbox at all
[capabilities.ipc.outbound]
allow = [
  "@os.vyoma.notes",           # named target
  "@@vyoma.*",                 # any system topic
  "@@broadcast.*",             # any user broadcast
]
max_inflight = 8               # how many outstanding `call`s
```

The check `route::check_cap` evaluates **both**: Y's `from` must
include X's bundle, AND X's `outbound.allow` must include Y's pattern.
This double-opt-in is required because:
- Senders need to declare intent (so a sandbox can inspect what an app
  intends to talk to before approving install).
- Recipients need to declare acceptance (so users can see what
  cross-app reach a recipient grants).

### 9.2 Sender stamping

```rust
impl IpcRouter {
    /// Called from the WIT import host implementation. `caller_id` comes
    /// from the wasmtime `Caller` context, not from the envelope — apps
    /// cannot lie about who they are.
    pub fn submit(&self, caller_id: InstanceId, mut env: IpcEnvelope) -> Result<u64, IpcError> {
        // Overwrite sender. App-set sender is ignored.
        let caller_bundle = self.app_table.get(caller_id)
            .map(|h| h.identity.bundle_id.clone())
            .ok_or(IpcError::Forbidden)?;
        env.sender = IpcAddr::Instance(caller_id);
        env.sender_bundle_authoritative = caller_bundle;  // private field
        // ...
        self.inbound_tx.try_send(RouterCmd::Send(env))
            .map(|_| env.id)
            .map_err(|_| IpcError::QueueFull)
    }
}
```

### 9.3 Schema-level allowlist

When `payload = Typed(TypedMessage { schema_id, .. })`, the recipient's
`capabilities.ipc.schemas` glob must match `schema_id.name`. Failure
returns `IpcError::SchemaUnsupported`. This lets a recipient narrow
their attack surface beyond bundle-level allowlisting: even an allowed
sender can only send permitted schemas.

### 9.4 Rate limiting

Each sender has a token-bucket per recipient:

```rust
pub struct RateBucket {
    capacity_per_sec: u32,
    tokens: parking_lot::Mutex<f32>,
    last_refill: parking_lot::Mutex<Instant>,
}

impl RateBucket {
    pub fn try_consume(&self) -> bool {
        let mut last = self.last_refill.lock();
        let mut toks = self.tokens.lock();
        let now = Instant::now();
        let dt = now.duration_since(*last).as_secs_f32();
        *toks = (*toks + dt * self.capacity_per_sec as f32).min(self.capacity_per_sec as f32);
        *last = now;
        if *toks >= 1.0 { *toks -= 1.0; true } else { false }
    }
}
```

Bucket location: `DashMap<(BundleId, BundleId), RateBucket>` on the
router. Eviction: LRU at 1024 entries.

### 9.5 Forgery resistance

| Attack | Defense |
|---|---|
| App claims to be another bundle | `submit()` overwrites `sender` from `caller_id` |
| App constructs envelope with fake `id` | `route()` overwrites `id` from monotonic counter |
| App calls supervisor without entitlement | `EntitlementChecker::check` |
| App subscribes to topic it has no cap for | `TopicTable::subscribe` checks manifest |
| App sends huge payload to OOM recipient | Per-class size cap, hard reject |
| App `r#move`s a capability it doesn't own | `fd_table` ownership check before route |
| App replies to an `id` it never received | Only `pending` entries it appears in as `recipient` accept replies |

---

## 10. Performance

### 10.1 Targets

| Operation | Target | Worst-case |
|---|---|---|
| `send` (Raw, < 256 B) | < 50 µs | 100 µs |
| `send` (Typed, < 4 KiB) | < 80 µs | 200 µs |
| `call` round-trip (typed, < 1 KiB body) | < 200 µs | 800 µs |
| `send-with-shm` attach | < 100 µs | 300 µs |
| `@supervisor: list` reply | < 500 µs | 1 ms |
| Broadcast to 32 subscribers | < 400 µs | 2 ms |
| 1080p frame (SharedBufferKind::Surface) handoff | < 50 µs (zero-copy) | 80 µs |

### 10.2 Inline fast path

Envelopes whose total serialized size is < 256 B skip CBOR allocation:
the `body` field is stored in a `SmallVec<[u8; 256]>` that lives inline
in the `IpcPayload::Typed` variant. This eliminates two allocations per
small message (the `Bytes` allocation and the `Vec<u8>` body
allocation).

```rust
pub enum InlineBody {
    Inline(SmallVec<[u8; 256]>),
    Heap(Bytes),
}
```

### 10.3 Batching

The `CallbackDispatcher` drains the inbox in batches of up to 16
envelopes per `on-ipc` invocation. The WIT signature is actually
`on-ipc: func(envs: list<envelope>)`, not single. This amortizes the
WASM call overhead (which dominates at ~5 µs per call) over multiple
messages.

Batches are flushed when (a) 16 envelopes are gathered, (b) 1 ms has
elapsed since the first, or (c) a `Critical`-priority envelope arrives
mid-batch (immediate flush + the Critical alone in its own list).

### 10.4 Channel sizing

| Channel | Cap | Reason |
|---|---|---|
| Per-app `inbox_lo` | 256 | 1 frame of input @ 256 Hz |
| Per-app `inbox_hi` | 64 | rarely full; bursts during focus / OOM |
| Router `inbound` | 8 192 | absorbs publish storms |
| Topic broadcasts | n/a (per-subscriber inbox enforces) | |
| Reply mailbox | 1 | single-shot |
| Supervisor cmd reply | 1 | single-shot |

### 10.5 Zero-copy hot path: Surface

For full-screen 1920×1080 @ 60 Hz video:

```
1080 × 1920 × 4 bytes = 8.3 MiB per frame
× 60 Hz                = 498 MiB/s
```

A two-copy IPC (sender writes Vec, router clones, recipient reads) would
saturate memory bandwidth on low-end hardware. The Round-2
`SharedBufferKind::Surface` alias means the *only* copy is the GPU
DMA at compositor scanout. The IPC envelope carries an 8-byte
`SharedBufferId`; the router does no buffer work at all.

### 10.6 Hot-cold split

The router's `routes` `DashMap` is sharded 16 ways internally; reads are
lock-free on the common path. `pending` is similarly sharded. The only
single-mutex contention point is `cap_cache` (LRU), which we size at
1024 entries — entries are inserted once per (sender_bundle,
recipient_bundle, class) tuple per manifest revision, so cache misses
are rare.

### 10.7 Allocation budget per send

| Step | Allocs | Bytes |
|---|---|---|
| Envelope construction | 0 (Bytes pointer + InlineBody stack) | 0 |
| Capability cache hit | 0 | 0 |
| Route table lookup | 0 (DashMap shard read) | 0 |
| `try_send` to crossbeam | 0 | 0 |
| **Total** | **0** | **0** |

Cold-path (capability miss, glob walk): one small `String` clone and
one `DashMap` insert; amortized to zero.

---

## 11. Implementation Files

| File | Approx LoC | Contents |
|---|---|---|
| `supervisor/src/ipc/mod.rs`             | 80   | Re-exports, glue, `IpcSubsystem::start()` |
| `supervisor/src/ipc/addr.rs`            | 220  | `IpcAddr`, parsing, resolution, glob match |
| `supervisor/src/ipc/envelope.rs`        | 280  | `IpcEnvelope`, `IpcPayload`, `TypedMessage`, `SchemaId`, `IpcFlags`, `Priority`, `InlineBody` |
| `supervisor/src/ipc/router.rs`          | 480  | `IpcRouter`, `RouterCmd`, `RouteEntry`, main loop, `route`, `deliver`, `on_full` |
| `supervisor/src/ipc/reply.rs`           | 340  | `PendingReply`, `ReplyMailbox`, `ReplyWaiter`, async bridge, timeout sweep |
| `supervisor/src/ipc/broadcast.rs`       | 250  | `TopicTable`, `TopicId`, `TopicGlob`, publish/subscribe paths |
| `supervisor/src/ipc/supervisor_cmd.rs`  | 360  | `SupervisorCmdHandler`, `SupCmd` parser, entitlement check, dispatch |
| `supervisor/src/ipc/cap.rs`             | 200  | `EntitlementChecker`, `RateBucket`, capability matrix evaluation |
| `supervisor/src/ipc/wit_host.rs`        | 380  | Wasmtime host bindings for `ipc-client`, async `func_wrap_async` for `call` |
| `supervisor/src/ipc/dispatch.rs`        | 280  | Batched delivery to `on-ipc`, priority lane drain, `CallbackDispatcher` extension |
| `wit/vyoma-ipc.wit`                     | 200  | Full WIT interface (§8.1) |
| `apps/_lib/vyoma-ipc-client/src/lib.rs` | 320  | App-side ergonomic Rust wrapper over generated WIT bindings |
| **Total new**                           | ~3 590 | 11 source files, all under 500-line cap |

### 11.1 Module boundaries (why this split)

- `addr.rs` separates parsing from envelope construction so the parser
  is independently unit-testable (one of the most attack-prone surfaces).
- `reply.rs` is the only file that imports `tokio`, keeping the async
  bridge isolated.
- `wit_host.rs` is the only file Wasmtime imports leak into; the rest
  of the IPC subsystem is generic over the host runtime.
- `supervisor_cmd.rs` is its own file because v2 will likely add 20+
  verbs and we want headroom under the 500-line cap.

### 11.2 Required Cargo additions

```toml
ciborium     = "0.2"   # CBOR
lru          = "0.12"  # capability cache
smallvec     = "1.13"  # inline body
siphasher    = "1.0"   # SchemaId / TopicId hashing
```

Already present from Round 1/2: `dashmap`, `crossbeam`,
`parking_lot`, `arc_swap`, `bytes`, `tokio` (single-threaded, for async
reply only), `wit-bindgen`.

---

## 12. Open Questions for the Critic

These are the seams I expect the Critic to attack:

**1. Async bridge between `crossbeam` and `tokio::Notify` in `ReplyWaiter`.**
The double-poll pattern (try_recv → notify → try_recv again) has a known
race: between the first `try_recv` returning Empty and the `notified()`
subscription, the producer may call `fulfill` *and* `notify_one`. The
second `try_recv` catches it, but only because we poll the notify
*after* registering the future. Is this actually correct, or do I need
a different primitive (e.g., `tokio::sync::oneshot` end-to-end, skipping
crossbeam)?

**2. Capability cache invalidation on manifest reload.**
`ManifestReloaded` clears the entire LRU. Under churn (multiple
hot-reloads per second), every send becomes a cold path until the cache
rewarms. Is `clear()` overkill? Should we instead bump a "cache epoch"
counter and check epoch in each cache lookup? That preserves the cache
across reloads but adds an atomic load per send.

**3. Schema explosion vs typed dispatch cost.**
We hash `schema_id.name` once per send (SipHash-2-4 is ~20 ns). For
high-rate input lanes (256 Hz keystrokes) this is fine. But broadcasts
with 64 subscribers and 8 schema globs per subscriber mean 512 hashes
per publish. Cache the hash in the `Arc<str>` interner?

**4. Reply correlator memory footprint under attack.**
A malicious app issues `call` with `deadline_ms = u64::MAX` (or never
sets one — default 30 s). 32 inflight × 32 apps = 1024 pending
entries × 200 bytes each = 200 KiB. Tolerable, but uncapped on the
*supervisor side*. Should `pending` have a hard cap (e.g., 4096) with
oldest-pending eviction?

**5. SecurityAgent for elevation: bootstrapping.**
The SecurityAgent is itself an app. What if it crashes? What if its
inbox is full? The router has no fallback path — the user simply cannot
elevate. Do we need a fallback `kernel_panic_elevation_console` (a TTY
prompt) when SecurityAgent is unreachable?

**6. Topic ID collisions.**
TopicId is a 64-bit SipHash. Birthday bound for collision is ~2³²
topics, well above realistic scale, BUT SipHash with a random seed
means the same topic name hashes differently across supervisor
restarts, breaking persisted subscriptions. Should the seed be
persisted in `/data/state/ipc-seed`? Or do we accept that subscriptions
are ephemeral (must be re-established on resume)?

**7. Batching latency for input lane.**
Batching up to 16 envelopes / 1 ms is great for typed RPC, but for
input it introduces up to 1 ms of latency. At 60 Hz the frame budget is
16.6 ms; 1 ms is 6 %. Acceptable? Should `Input` class bypass batching
entirely (single-envelope `on-ipc` calls)?

**8. Capability passing and resource lifetime cycles.**
If app A sends a `Surface` cap to app B, B sends it back to A as a
sub-window, A re-grants to C... we have a graph of capability
references with no GC. Round 2 had refcounts on `SharedBuffer`; we
need the same for `VyomaResource::File` / `Surface`. Should this live
in a unified `ResourceTable` rather than each subsystem rolling its
own?

---

## 13. Integration With Prior Rounds

### 13.1 Round 1 integration

- `IpcRouter` is one of the five core supervisor actors named in Round 1
  Table §9. Its inbox channel cap of 1024 there is now refined to 8 192
  (`RouterCmd` inbound).
- `IpcEnvelope` from Round 1 §9 is *extended*, not replaced — the new
  fields (`id`, `flags`, `reply_to`, `deadline_ms`) are additions.
  Existing code paths that constructed envelopes from Round 1 must add
  `..IpcEnvelope::default()` to the struct literal.
- `on-ipc` callback signature in Round 1 §4.1 was `on-ipc: func(from:
  string, payload: list<u8>)`. Round 3 changes this to `on-ipc: func(env:
  envelope)` — wider, with a `from` field on `envelope` for compatibility.
  Apps that wrote against the old signature need a trivial migration.
- `IpcAddr` was implicit in Round 1 (`IpcTarget`); Round 3 promotes it
  to a first-class type with parsing and resolution as a sibling of
  `IpcTarget`. The two coexist: `IpcTarget` is what the router emits to
  delivery code; `IpcAddr` is what the network sees.

### 13.2 Round 2 integration

- `SharedBufferId` (NonZeroU64) is referenced verbatim from
  `crate::shm::SharedBufferId`.
- `SharedBufferKind` (enum) is referenced verbatim.
- The router calls into `SharedBufferRegistry::attach_reader` /
  `detach_all` for SHM hand-off and instance death cleanup.
- Suspension grant: when an app is `Suspending`, the router still
  delivers envelopes to its `inbox_hi` (lifecycle events, kill, etc.)
  but rejects `Normal`-priority sends to it with `RecipientGone` so the
  sender knows to retry post-resume.
- `vyoma:memory@0.1.0` is imported in the `vyoma-app` world alongside
  `vyoma:ipc@0.1.0`.

---

## 14. Phased Rollout

| Phase | Deliverable | Smoke test |
|---|---|---|
| **P1** (week 1) | `addr.rs`, `envelope.rs`, `router.rs` skeleton, `IpcRouter::route` minimal | Two-app ping-pong with bundle addressing |
| **P2** (week 2) | `cap.rs`, manifest allowlist, rate limit | App C cannot send to App A; A → B works |
| **P3** (week 3) | `reply.rs`, async bridge, `call` RPC | App A calls B, gets typed reply, latency < 200 µs |
| **P4** (week 4) | `broadcast.rs`, topic subscribe | `vyoma.focus.changed` delivered to 8 subscribers |
| **P5** (week 5) | `supervisor_cmd.rs`, entitlements | Shell `@supervisor: list` works; non-entitled app rejected |
| **P6** (week 6) | SHM IPC, `send_with_shm`, Surface alias | 1080p frame handoff at 60 Hz, zero copy |
| **P7** (week 7) | Capability passing (`VyomaResource`, FD table) | App A hands a file FD to App B, B reads file |
| **P8** (week 8) | SecurityAgent + elevation prompt | Non-admin app gets dialog, user confirms, op succeeds |

---

## 15. Why This Design Is Not Yet Done

Three concerns I cannot resolve unilaterally and explicitly defer to
the Critic + Round 4 synthesis:

**A. Backpressure semantics for broadcasts.** I've said "at-most-once for
user, at-least-once for system" but the at-least-once path uses a 5 ms
block + drop. That's neither truly at-least-once (drop violates it) nor
fully at-most-once (block-then-drop is observably different from
drop-immediately). The correct answer might be per-subscriber retry
queues, but that adds memory proportional to subscribers² in the worst
case. Critic should pick.

**B. Schema registry and versioning.** Right now `SchemaId` is a freeform
dotted string. Two independently-developed apps could collide on
`"create-note"`. macOS XPC solves this with an Info.plist
`MachServices` namespace; we have no analog. Options: prefix with
sender bundle, require central registration at install, accept
collisions and let users pick. I lean toward bundle-prefixing
(`os.vyoma.notes::create-note`) but it makes shared schemas (e.g.,
`vyoma::clipboard.copy`) awkward.

**C. The `tokio` dependency for the async reply bridge.** This is the
*only* tokio in the supervisor. Pulling it in costs ~400 KiB of binary
size and introduces a runtime we otherwise don't need. Alternatives:
(1) Implement the bridge with `std::thread::park` + atomic-set (cheap
but blocks a worker thread), (2) use Wasmtime's epoch interruption to
poll synchronously between epoch ticks (no extra deps, but coarser
latency), (3) accept the tokio dep. Architect leans (3); Critic should
push back if binary size matters more than ergonomics.

---

*End of Round 3 Architect proposal.*
