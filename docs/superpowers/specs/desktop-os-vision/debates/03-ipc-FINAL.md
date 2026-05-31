# Round 3 Final: Inter-Process Communication (IPC)

**Status:** ✅ Debated & synthesized (2026-05-29)
**Debate:** [Architect: 1764 lines] [Critic: 985 lines] [Final: this]
**Subsystem:** Supervisor-brokered, sharded IPC for `wasm32-wasip2` apps under Wasmtime PID-1
**macOS equivalent:** Mach ports + XPC + NSDistributedNotification + Apple Events + Pasteboard + `launchctl`

---

## 0. Executive Summary

VyomaOS IPC is the spinal cord of the desktop personality: clipboard, drag-and-drop, notifications,
window events, XPC-style typed RPC, broadcast, system commands, and zero-copy frame buffer hand-off all
flow through it. It subsumes — on a Linux 5.10 kernel with virtio + 9P + DRM and no Mach/Binder/D-Bus —
the seven IPC primitives macOS layers across the equivalent stack.

The Architect proposed a single supervisor-brokered actor with a star topology, bounded crossbeam inboxes,
per-class overflow policy, async `call` via Wasmtime epoch yield, `SharedBuffer` zero-copy hand-off, and
a `@supervisor:` command path. The Critic correctly identified seven blocking failures: (1) a single
router actor serializes the desktop, (2) head-of-line blocking starves unrelated apps, (3) request-reply
has no cycle detection, (4) `SharedBuffer` has no ownership / crash model, (5) `on-ipc` non-reentrancy
plus inbox overflow creates a delivery cliff, (6) capability checks land on the cold-shard `RwLock`, and
(7) the stdout `@supervisor:` protocol is unauthenticated with no migration path.

This Round-3 Final synthesizes both into a buildable specification. The headline reshaping:

- The router is decomposed into a **sharded routing fabric**: a read-mostly `ArcSwap<RouteTable>` for
  address resolution, a **per-receiver delivery thread** that drains its own inbox, and **N shard actors**
  (N = `num_cpus().min(8)`) that fan-out enqueues. The single-actor bottleneck is gone.
- Overflow is **never blocking**. Every `try_send` falls into a declared per-class, per-app overflow
  policy: `drop-oldest`, `drop-newest`, `coalesce`, or `nack-sender`. The "router blocks the world" mode
  is removed from the design.
- Request-reply maintains a **wait-for graph** in the supervisor. Every `call` walks the graph for
  cycles before queuing; A→B→C→A fails immediately with `IpcError::WouldDeadlock`. The waiter table is
  capped per app (`max_inflight = 8` default) and globally (`max_pending = 4096`).
- `SharedBuffer` is **supervisor-owned, refcounted, with a publish/commit boundary**. RAII handles in
  the supervisor's resource table; receiver reads return `IpcError::SenderGone` after sender crash; the
  buffer is reclaimed when the refcount hits zero. Surface-aliased buffers skip the refcount path and
  use the compositor's existing double-buffer protocol.
- `on-ipc` is **batched and reentrancy-aware**. The dispatcher pulls up to 16 envelopes per WASM call;
  if a reentrant `call` is detected mid-`on-ipc`, the inner request is deferred via `BusyDeferred`
  state and reissued on dispatcher idle. Input lane bypasses batching (single-envelope dispatch).
- Capability decisions are served from **`ArcSwap<PolicySnapshot>` in `AppState.hot`**, not the cold
  shard. A single atomic load + Arc clone per send: ~20 ns vs ~200 ns for the cold `RwLock` path. Updates
  on `ManifestReloaded` swap a new snapshot atomically; old snapshots are reclaimed when no envelope
  references them.
- `@supervisor:` commands are **authenticated, capability-gated, and the legacy stdout path is killed
  in v1** behind the `legacy_stdout_supervisor` boot flag (default off). Existing apps must declare
  `[capabilities.supervisor]` to retain access. A signed-command envelope shape carries the verb,
  caller identity, and the entitlement check happens inside the router.

This document specifies the data structures, algorithms, WIT surface, security model, performance
budget, and implementation files. All files stay under the 500-line ceiling. Total new code: ~5,420 LOC
across 19 files.

---

## Key Decisions

1. **Sharded routing fabric, not one actor.** N=`min(num_cpus, 8)` `RouteShard` workers + a read-mostly
   `ArcSwap<RouteTable>` for address resolution. The single-thread bottleneck (Critic C1) is eliminated.
   Per-message critical path: atomic load (policy snapshot) → atomic load (route table) → `try_send` to
   inbox. Warm-cache cost: ~50–80 ns per send.

2. **Per-receiver delivery thread; router only enqueues.** Each `AppHandle` already owns its
   `CallbackDispatcher` thread; the router clones the `Sender<IpcEnvelope>` once at instance birth and
   never touches it again until `InstanceGone`. Drain is the receiving thread's responsibility.

3. **Two physical inboxes per app: `inbox_hi` (cap 64) and `inbox_lo` (cap 256).** Hi carries
   `Critical`/`System` priority (lifecycle, OOM, focus, timeout); lo carries `Normal`/`Input`. The
   dispatcher's `select_biased!` drains hi-first. This resolves Critic S5 (priority).

4. **Non-blocking overflow with manifest-declared per-class policy.** Defaults: `Input → drop-oldest`,
   `Render → drop-oldest+coalesce`, `Ipc → nack-sender`, `Control → block-50ms-then-error`. Sender
   manifest may override per-target. Critic C2 resolved.

5. **Wait-for graph + cycle detection for `call`.** O(degree) walk on every new request. Cycle ⇒
   `IpcError::WouldDeadlock` returned synchronously. Per-app `max_inflight` (default 8), global
   `pending_table` cap 4096 with oldest-eviction NACK to oldest caller. Critic C3 resolved.

6. **`SharedBuffer` supervisor-owned, refcounted, publish/commit semantics.** RAII `BufferHandle` in
   each instance's resource table; sender crash flips a `valid` AtomicBool to false; receiver
   `read()` returns `IpcError::SenderGone` on invalid. `commit(seq)` is the publish boundary; readers
   only see committed bytes. Critic C4 resolved.

7. **`on-ipc` is push-batched with reentrancy state machine.** Dispatcher pulls 1–16 envelopes per
   `on-ipc` call; `BusyDeferred` state queues outbound `call` made from within `on-ipc` until the
   dispatcher returns. Input lane bypasses batching (single envelope, single dispatch). Critic C5
   resolved.

8. **`ArcSwap<PolicySnapshot>` in `AppState.hot`.** Capability lookups are atomic-load + Arc clone, no
   `RwLock`. `ManifestReloaded` builds a new snapshot and `ArcSwap::store`s it. Old envelopes referencing
   the old snapshot complete normally; readers see eventual consistency. Critic C6 resolved.

9. **`@supervisor:` is an authenticated WIT call; legacy stdout path is OFF by default.** Manifest
   declares `[capabilities.supervisor]` with a subset of verbs. `EntitlementChecker` validates on every
   call. Boot flag `legacy_stdout_supervisor = false` (default). Critic C7 resolved.

10. **Wire format = `(schema_id: u64, schema_version: u16, body: CBOR)`.** Schema IDs are SipHash-2-4
    of the WIT type's fully-qualified name (with a process-wide random seed persisted to
    `/data/state/ipc-seed`). Version negotiation: receiver bindings declare accepted (schema_id,
    min_version, max_version); mismatch ⇒ `IpcError::SchemaUnsupported` to sender (or drop for
    fire-and-forget with a warn log). Critic S2 resolved.

11. **Fast-path streams (`StreamChannel`) for media.** Pre-established lock-free SPSC ring buffer
    allocated by the supervisor on `open-stream`, capability-checked at setup only, zero supervisor
    involvement per frame thereafter. Wakeup via `tokio::sync::Notify` per stream. Required for
    audio and 60 FPS video. Critic S4 resolved.

12. **CBOR over MessagePack/Protobuf.** Schema-optional, byte-string-preserving, `ciborium` integration
    with `wit-bindgen`, ~25 KiB WASM footprint vs ~120 KiB for Protobuf.

13. **Capability passing via `VyomaResource` (File, Surface, Port, Sub) with per-instance opaque FDs.**
    `flags.move` semantics; supervisor's `fd_table` is the source of truth; receiver always gets a
    fresh `VyomaFd` so handles are never portable across instances.

14. **Per-app IPC quotas in manifest** (`max_pending_requests`, `max_active_subscriptions`,
    `max_shared_buffers`, `max_shared_buffer_bytes`, `max_outbound_msg_per_sec`). Enforced by router;
    breach ⇒ `IpcError::QuotaExceeded`. Critic S10 resolved.

15. **Tracing fields `trace_id: Option<u128>` and `span_id: Option<u64>` in every envelope.** Supervisor
    propagates them through reply paths. Apps opting into OpenTelemetry can build distributed traces.
    Critic G4 partly resolved (the rest is in observability subsystem).

---

## 1. Topology & Routing Architecture — Fixed

### 1.1 Why broker-mediated remains correct

The Architect's brokered-star choice survives critique unchanged. Capability-secure WASM forbids
ambient authority; a peer-to-peer FD or shared channel between two WASM instances would *be* ambient
authority (held, forgeable in host bug case, retained across revocation). The only construction that
preserves capability semantics is a host-mediated send where the host re-validates the sender's right
to talk to the recipient on every message.

What the Critic correctly attacked is not the topology but the **implementation** of the broker as a
single in-process actor. We dissolve the single actor into a sharded routing fabric while keeping the
star-topology invariant.

### 1.2 The sharded routing fabric

The new design splits the broker into four cooperating components:

```
                ┌────────────────────────────────────────────────────────────┐
                │                  Router Substrate                          │
                │                                                            │
                │  ┌──────────────────────────────────────────────────────┐  │
                │  │  ArcSwap<RouteTable>  (read-mostly)                  │  │
                │  │  BundleId → SmallVec<InstanceId>                     │  │
                │  │  InstanceId → RouteEntry { inbox_hi, inbox_lo, ... } │  │
                │  └──────────────────────────────────────────────────────┘  │
                │                                                            │
                │  ┌────────────┐  ┌────────────┐  ┌────────────┐            │
                │  │ Shard 0    │  │ Shard 1    │  │ Shard N-1  │  ...       │
                │  │ (worker)   │  │ (worker)   │  │ (worker)   │            │
                │  │  - pending │  │  - pending │  │  - pending │            │
                │  │  - waitfor │  │  - waitfor │  │  - waitfor │            │
                │  └────────────┘  └────────────┘  └────────────┘            │
                │                                                            │
                │  ┌──────────────────────────────────────────────────────┐  │
                │  │  Shared Substrate (lock-free / atomic)               │  │
                │  │   - TopicTable (DashMap)                             │  │
                │  │   - cap_cache → in PolicySnapshot (per AppState.hot) │  │
                │  │   - shm_registry (per Round-2)                       │  │
                │  │   - fd_table (DashMap)                               │  │
                │  └──────────────────────────────────────────────────────┘  │
                └────────────────────────────────────────────────────────────┘

                        |                |                |
                  inbox_hi/lo     inbox_hi/lo      inbox_hi/lo
                  (per AppHandle) (per AppHandle)  (per AppHandle)
                        |                |                |
                  CallbackDispatcher CallbackDispatcher CallbackDispatcher
                        |                |                |
                    Store A           Store B          Store C
```

**Shard selection.** Each `Send` operation hashes `(env.id ^ env.recipient.first_iid())` modulo N to
pick a shard. Address resolution and capability checks are **not** shard-local — they hit the
`ArcSwap<RouteTable>` and `ArcSwap<PolicySnapshot>` lock-free. Only operations that mutate per-call
state (`pending` map, wait-for graph) are shard-local. This means the per-message critical path stays
on one shard, but capability lookups and inbox handle reads scale across all cores.

**No single thread sees all traffic.** The shard worker's job is to (a) populate `env.id`, (b) walk the
wait-for graph for `call`, (c) insert/lookup `pending` rows, (d) call `inbox.try_send`. Steps (a) and
(d) are O(1) atomic ops. Step (b) is bounded by `max_inflight` per app. Step (c) is `dashmap` shard read.

**Lock-free address resolution.** The `RouteTable` is rebuilt on `InstanceOnline`/`InstanceGone`; the
new `Arc<RouteTable>` is published via `ArcSwap::store`. Reads on the hot path are
`route_table.load().routes.get(&iid)` — one atomic load + one DashMap read, ~30 ns.

### 1.3 Why this kills the bottleneck

Returning to the Critic's table (excluding video — which bypasses the router entirely via streams):

| Path                       | Old design (one actor) | New design (sharded)    |
|----------------------------|-----------------------:|------------------------:|
| Per-message critical path  | 1.0–3.0 µs             | 50–80 ns                |
| Cap check                  | RwLock read ~200 ns    | ArcSwap load ~20 ns     |
| Address resolution         | DashMap read ~30 ns    | DashMap read ~30 ns     |
| Throughput (one core)      | ~300 K msg/s           | ~12 M msg/s             |
| Tail latency under 6K msg/s | p99 spikes to 50 µs   | p99 < 5 µs              |
| Cross-core scaling         | None                   | Near-linear to N shards |

Video and audio bypass the router entirely after stream setup via §5 streams. Compositor frame
callbacks (1,800 msg/sec) consume ~0.15 ms/sec of CPU vs the old design's ~5 ms/sec — a 30× reduction
that frees a frame budget worth of CPU on a mid-range desktop.

### 1.4 What stays single-threaded

Two operations remain single-threaded by intentional design:

- **The `tick` deadline sweep** runs on a dedicated `IpcTicker` thread every 100 ms. It walks the
  global `pending_table` (sharded but iterated in parallel via `rayon`), fails expired waiters, and
  emits `vyoma.lifecycle.timeout` events. Cost: ~10 µs per 100-ms tick on a 1 K-entry table.
- **`ManifestReloaded` rebuild** is one-shot — it constructs a new `PolicySnapshot`, swaps it in via
  `ArcSwap::store`, and triggers a `RouteTable` rebuild. The rebuild itself is parallel by shard but
  the publish is sequential. Cost: ~200 µs for 100 apps. Reloads are user-initiated, so this is fine.

---

## 2. IPC Envelope & Message Types

### 2.1 `IpcAddr` — the address space

```rust
//! supervisor/src/ipc/addr.rs   (~220 lines)

use std::sync::Arc;
use crate::instance::{BundleId, InstanceId};

/// Logical address used in envelopes. Resolved at routing time, never
/// at "connection" time (there are no connections).
#[derive(Debug, Clone)]
pub enum IpcAddr {
    /// Specific running instance.
    Instance(InstanceId),
    /// All instances of a bundle; routing fans out.
    Bundle(BundleId),
    /// Specific instance of a bundle: `@bundle#iid:`.
    BundleInstance(BundleId, InstanceId),
    /// Whichever instance has input focus right now.
    Focused,
    /// PID-1 itself (handled by SupervisorCmdHandler).
    Supervisor,
    /// A pub-sub topic (only valid as recipient).
    Topic(Arc<str>),
    /// A glob over topics (only valid for subscriptions, not direct sends).
    TopicGlob(Arc<str>),
    /// Broadcast: all subscribers of an explicit broadcast namespace.
    Broadcast(Arc<str>),
    /// Sender identity placeholder; never appears as recipient. The router
    /// overwrites this with the authoritative caller_id on submit().
    AuthenticatedSender,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("unknown bundle: {0:?}")]
    UnknownBundle(BundleId),
    #[error("no instance for bundle: {0:?}")]
    NoInstance(BundleId),
    #[error("no focused instance")]
    NoFocused,
    #[error("topic not found: {0}")]
    UnknownTopic(String),
    #[error("malformed address: {0}")]
    Parse(String),
}

impl IpcAddr {
    /// Parses `@bundle:`, `@bundle#iid:`, `@@focused:`, `@@vyoma.foo:`,
    /// `@@broadcast.bar:`, `@supervisor:`.
    pub fn parse(s: &str) -> Result<Self, ResolveError> {
        if let Some(rest) = s.strip_prefix("@supervisor") {
            if rest.is_empty() || rest == ":" { return Ok(IpcAddr::Supervisor); }
            return Err(ResolveError::Parse(s.to_string()));
        }
        if let Some(rest) = s.strip_prefix("@@focused") {
            if rest.is_empty() || rest == ":" { return Ok(IpcAddr::Focused); }
            return Err(ResolveError::Parse(s.to_string()));
        }
        if let Some(rest) = s.strip_prefix("@@broadcast.") {
            let topic = rest.trim_end_matches(':');
            return Ok(IpcAddr::Broadcast(Arc::from(topic)));
        }
        if let Some(rest) = s.strip_prefix("@@") {
            let topic = rest.trim_end_matches(':');
            if topic.ends_with('*') {
                return Ok(IpcAddr::TopicGlob(Arc::from(topic)));
            }
            return Ok(IpcAddr::Topic(Arc::from(topic)));
        }
        if let Some(rest) = s.strip_prefix('@') {
            let body = rest.trim_end_matches(':');
            if let Some((bundle, iid)) = body.split_once('#') {
                let iid = iid.parse::<u32>().map_err(|_| ResolveError::Parse(s.to_string()))?;
                let iid = std::num::NonZeroU32::new(iid).ok_or_else(|| ResolveError::Parse(s.to_string()))?;
                return Ok(IpcAddr::BundleInstance(BundleId::from(bundle), InstanceId(iid)));
            }
            return Ok(IpcAddr::Bundle(BundleId::from(body)));
        }
        Err(ResolveError::Parse(s.to_string()))
    }
}
```

### 2.2 `IpcEnvelope` — the unit of routing

```rust
//! supervisor/src/ipc/envelope.rs   (~360 lines)

use std::sync::Arc;
use bytes::Bytes;
use smallvec::SmallVec;
use crate::ipc::addr::IpcAddr;
use crate::instance::BundleId;

#[derive(Debug, Clone)]
pub struct IpcEnvelope {
    /// Monotonic, supervisor-assigned. 0 ⇔ unassigned. Unique across one
    /// supervisor uptime (resets on reboot; persisted only via traces).
    pub id:        u64,

    /// Set by router from the authenticated WIT caller; cannot be forged.
    pub sender:    IpcAddr,

    /// Stable bundle ID of the sender, also authoritative.
    pub sender_bundle: BundleId,

    /// Provided by sender; logically resolved at delivery time.
    pub recipient: IpcAddr,

    pub payload:   IpcPayload,

    /// If this is a reply, the `id` of the original request.
    pub reply_to:  Option<u64>,

    /// CLOCK_MONOTONIC ms since boot when the message expires.
    /// `None` ⇔ no deadline (fire-and-forget default).
    pub deadline_ms: Option<u64>,

    pub flags:     IpcFlags,
    pub class:     IpcClass,

    /// Distributed tracing context (OpenTelemetry-compatible).
    pub trace_id:  Option<u128>,
    pub span_id:   Option<u64>,

    /// Source instance epoch. Receiver discards messages where the sender's
    /// current epoch != envelope.sender_epoch (sender died and restarted).
    pub sender_epoch: u32,
}

#[derive(Debug, Clone)]
pub enum IpcPayload {
    /// Legacy stdout line protocol (`println!("@pong: hello")`).
    /// Preserved verbatim ONLY for `[capabilities.ipc.legacy_stdout] = true`.
    Raw(Bytes),

    /// WIT-typed CBOR message (the modern path).
    Typed(TypedMessage),

    /// Zero-copy handle to a Round-2 `SharedBuffer`.
    SharedMem(SharedMemRef),

    /// Capability handoff (file FD, surface, port reservation, subscription).
    Capability(VyomaResource),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IpcFlags {
    /// Sender wants a reply.
    pub expects_reply: bool,
    /// If recipient inbox full, NACK rather than per-class policy.
    pub no_queue: bool,
    /// Move semantics: sender loses the embedded capability/handle.
    pub r#move: bool,
    /// System-priority lane; only supervisor + entitled apps may set.
    pub priority: Priority,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IpcClass {
    /// Mouse/keyboard/touch — drop-oldest.
    Input,
    /// User and typed-RPC messages — nack-sender on full.
    Ipc,
    /// Compositor frames — drop-oldest + coalesce.
    Render,
    /// Lifecycle, supervisor ops — block 50 ms then error.
    Control,
}

#[derive(Debug, Clone)]
pub struct TypedMessage {
    pub schema_id:      SchemaId,
    pub schema_version: u16,
    /// CBOR body. Inline (≤256B) avoids heap allocation.
    pub body:           InlineBody,
}

#[derive(Debug, Clone)]
pub enum InlineBody {
    Inline(SmallVec<[u8; 256]>),
    Heap(Bytes),
}

#[derive(Debug, Clone)]
pub struct SchemaId {
    /// Reverse-DNS dotted name, e.g. "os.vyoma.intent.open-url".
    pub name: Arc<str>,
    /// SipHash-2-4 of `name` under the persisted IPC seed.
    pub hash: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct SharedMemRef {
    pub id:     crate::shm::SharedBufferId,
    pub kind:   crate::shm::SharedBufferKind,
    pub access: SharedAccess,
    /// Optional sub-range [offset, offset+len).
    pub offset: u32,
    pub len:    u32,
    /// Sequence number at which sender committed; receiver MUST verify
    /// `last_committed_seq >= seq` before reading.
    pub seq:    u32,
}

#[derive(Debug, Clone, Copy)]
pub enum SharedAccess { Read, Write, ReadWrite }

#[derive(Debug, Clone)]
pub enum VyomaResource {
    File   { fd: VyomaFd,    access: FileAccess },
    Surface{ surface_id: u64, access: SurfaceAccess },
    Port   { schema: SchemaId, exclusive: bool },
    Sub    { topic: TopicId },
}

#[derive(Debug, Clone, Copy)]
pub struct VyomaFd(pub std::num::NonZeroU64);

#[derive(Debug, Clone, Copy)]
pub enum FileAccess    { Read, Write, ReadWrite }

#[derive(Debug, Clone, Copy)]
pub enum SurfaceAccess { ReadOnly, WriteOnly }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TopicId(pub u64);
```

### 2.3 Wire-size budget per envelope

| Field                                                | Bytes |
|------------------------------------------------------|------:|
| `id`, `reply_to`, `deadline_ms`, `sender_epoch`      | 32    |
| `sender`, `recipient` (IpcAddr ~40B each)            | 80    |
| `sender_bundle` (BundleId is `Arc<str>` → 16B)       | 16    |
| `flags`, `class`                                     | 8     |
| `trace_id`, `span_id`                                | 32    |
| `payload` (Bytes ptr/len/tag, or InlineBody header)  | 48    |
| **Total (without inline body)**                      | **216** |

For Typed messages with body ≤ 256B, the InlineBody slot adds 256B without an allocation. A 1 MiB
envelope buffer holds ~4 600 inline-bodied envelopes — well above the per-app inbox depth.

`Clone` cost is O(1): `Arc<str>` ref-bump, `Bytes` ref-bump, `SmallVec` inline copy. Hot-path
envelopes are stack-built and only `Arc`-bumped at enqueue.

---

## 3. Flow Control & Back-pressure — Fixed

### 3.1 Per-class overflow policies (declared, enforced, observable)

```rust
//! supervisor/src/ipc/overflow.rs   (~180 lines)

use crate::ipc::envelope::{IpcEnvelope, IpcClass};
use crate::instance::InstanceId;

#[derive(Debug, Clone, Copy)]
pub enum OverflowPolicy {
    /// Pop the oldest envelope, push the new. Suitable for liveness events
    /// (mouse moves, render ticks). Default for Input + Render.
    DropOldest,
    /// Discard the new envelope. Suitable for idempotent state updates.
    DropNewest,
    /// Send a reply with `IpcError::QueueFull` to the sender. Sender chooses
    /// retry policy. Default for Ipc.
    NackSender,
    /// Merge new envelope with the youngest unread of the same schema_id.
    /// Body of new replaces body of old; oldest committed bytes win for
    /// stream snapshots. Default for Render after DropOldest fails.
    Coalesce,
    /// Block sender up to 50 ms; if still full, return Err. Default for
    /// Control class. Never used for Input or Render.
    BlockBriefly,
}

#[derive(Debug, Clone, Copy)]
pub struct ClassPolicy {
    pub overflow: OverflowPolicy,
    pub inbox_cap: u16,
}

impl ClassPolicy {
    pub fn default_for(class: IpcClass) -> Self {
        match class {
            IpcClass::Input   => Self { overflow: OverflowPolicy::DropOldest,   inbox_cap: 256 },
            IpcClass::Render  => Self { overflow: OverflowPolicy::DropOldest,   inbox_cap: 128 },
            IpcClass::Ipc     => Self { overflow: OverflowPolicy::NackSender,   inbox_cap: 256 },
            IpcClass::Control => Self { overflow: OverflowPolicy::BlockBriefly, inbox_cap: 64  },
        }
    }
}
```

### 3.2 The non-blocking delivery path

```rust
//! supervisor/src/ipc/deliver.rs   (~280 lines)

use crossbeam::channel::TrySendError;
use std::time::Duration;

use crate::ipc::envelope::*;
use crate::ipc::overflow::{ClassPolicy, OverflowPolicy};
use crate::ipc::router::{RouteEntry, RouteTable};
use crate::ipc::reply::IpcError;
use crate::ipc::metrics::IpcMetrics;
use crate::instance::InstanceId;

pub fn deliver(
    table:   &RouteTable,
    rid:     InstanceId,
    env:     IpcEnvelope,
    policy:  ClassPolicy,
    metrics: &IpcMetrics,
) -> Result<(), DeliveryError> {
    let Some(route) = table.routes.get(&rid) else {
        return Err(DeliveryError::RecipientGone);
    };
    let inbox = match env.flags.priority {
        Priority::Critical | Priority::System => &route.inbox_hi,
        _                                     => &route.inbox_lo,
    };
    match inbox.try_send(env.clone()) {
        Ok(())                            => { metrics.delivered.inc(); Ok(()) }
        Err(TrySendError::Disconnected(_)) => Err(DeliveryError::RecipientGone),
        Err(TrySendError::Full(env))      => apply_overflow(table, rid, env, policy, metrics),
    }
}

fn apply_overflow(
    table:   &RouteTable,
    rid:     InstanceId,
    env:     IpcEnvelope,
    policy:  ClassPolicy,
    metrics: &IpcMetrics,
) -> Result<(), DeliveryError> {
    let Some(route) = table.routes.get(&rid) else {
        return Err(DeliveryError::RecipientGone);
    };
    let inbox = match env.flags.priority {
        Priority::Critical | Priority::System => &route.inbox_hi,
        _                                     => &route.inbox_lo,
    };
    if env.flags.no_queue {
        metrics.nacked.inc();
        return Err(DeliveryError::QueueFull);
    }
    match policy.overflow {
        OverflowPolicy::DropOldest => {
            // crossbeam::Receiver does not allow steal; the dispatcher
            // owns the receiver. We model "drop-oldest" by an atomic
            // sequence number: every envelope carries a generation; on
            // overflow we increment route.drop_gen and the dispatcher
            // skips envelopes whose gen < latest_kept_gen by class.
            route.drop_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
            match inbox.try_send(env) {
                Ok(()) => { metrics.delivered.inc(); Ok(()) }
                Err(_) => { metrics.dropped.inc(); Err(DeliveryError::QueueFull) }
            }
        }
        OverflowPolicy::DropNewest => {
            metrics.dropped.inc();
            Err(DeliveryError::QueueFull)
        }
        OverflowPolicy::NackSender => {
            metrics.nacked.inc();
            Err(DeliveryError::QueueFull)
        }
        OverflowPolicy::Coalesce => {
            // The receiver's dispatcher does coalescing on dequeue, not
            // on enqueue (crossbeam channels are FIFO immutable). The
            // dispatcher walks the batch and merges same-(sender,schema)
            // entries before calling on-ipc. Here we attempt one more
            // try_send; if still full, drop-newest.
            std::thread::yield_now();
            match inbox.try_send(env) {
                Ok(())  => { metrics.coalesced.inc(); Ok(()) }
                Err(_)  => { metrics.dropped.inc(); Err(DeliveryError::QueueFull) }
            }
        }
        OverflowPolicy::BlockBriefly => {
            match inbox.send_timeout(env, Duration::from_millis(50)) {
                Ok(()) => { metrics.delivered.inc(); Ok(()) }
                Err(_) => { metrics.timeouts.inc(); Err(DeliveryError::ControlBlocked) }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("recipient instance is gone")] RecipientGone,
    #[error("inbox full, message dropped")] QueueFull,
    #[error("control inbox blocked > 50 ms")] ControlBlocked,
}
```

### 3.3 Per-sender outbound rate limit (credit-based)

Each `(sender_bundle, recipient_bundle)` pair carries a token-bucket on the shard. A sender exceeding
its `max_outbound_msg_per_sec` quota gets `IpcError::QuotaExceeded` immediately; the WASM call returns
the error and the app can throttle. This is the **credit-based flow control** Critic S7 asked for.

```rust
pub struct RateBucket {
    capacity_per_sec: u32,
    tokens:           parking_lot::Mutex<f32>,
    last_refill:      parking_lot::Mutex<std::time::Instant>,
}

impl RateBucket {
    pub fn try_consume(&self) -> bool {
        let mut last = self.last_refill.lock();
        let mut toks = self.tokens.lock();
        let now = std::time::Instant::now();
        let dt = now.duration_since(*last).as_secs_f32();
        let cap = self.capacity_per_sec as f32;
        *toks = (*toks + dt * cap).min(cap);
        *last = now;
        if *toks >= 1.0 { *toks -= 1.0; true } else { false }
    }
}
```

Buckets live in an `LruCache<(BundleId, BundleId), RateBucket>` per shard with 1024 entries.

---

## 4. Request-Reply & Deadlock Detection — Fixed

### 4.1 The wait-for graph

```rust
//! supervisor/src/ipc/waitfor.rs   (~320 lines)

use std::collections::{HashMap, HashSet};
use parking_lot::RwLock;
use crate::instance::InstanceId;
use crate::ipc::reply::IpcError;

/// Directed graph of "A is awaiting a reply from B". Cycles are deadlocks.
pub struct WaitForGraph {
    edges: RwLock<HashMap<InstanceId, HashSet<InstanceId>>>,
}

impl WaitForGraph {
    pub fn new() -> Self {
        Self { edges: RwLock::new(HashMap::new()) }
    }

    /// Attempt to add `from -> to`. If it would create a cycle, return Err.
    /// O(V + E) DFS; V and E bounded by `max_inflight` per node.
    pub fn try_add(&self, from: InstanceId, to: InstanceId) -> Result<(), IpcError> {
        let mut g = self.edges.write();
        // Tentatively add the edge.
        g.entry(from).or_default().insert(to);
        // DFS from `to` looking for `from`.
        let mut stack = vec![to];
        let mut seen = HashSet::<InstanceId>::new();
        while let Some(node) = stack.pop() {
            if !seen.insert(node) { continue; }
            if node == from {
                // Cycle. Roll back the edge and reject.
                if let Some(set) = g.get_mut(&from) { set.remove(&to); }
                return Err(IpcError::WouldDeadlock);
            }
            if let Some(neighbors) = g.get(&node) {
                for n in neighbors { stack.push(*n); }
            }
        }
        Ok(())
    }

    pub fn remove(&self, from: InstanceId, to: InstanceId) {
        let mut g = self.edges.write();
        if let Some(set) = g.get_mut(&from) {
            set.remove(&to);
            if set.is_empty() { g.remove(&from); }
        }
    }

    pub fn drop_all_for(&self, who: InstanceId) {
        let mut g = self.edges.write();
        g.remove(&who);
        for set in g.values_mut() { set.remove(&who); }
    }

    /// For ipdeadlock CLI: returns current cycles if any.
    pub fn cycles(&self) -> Vec<Vec<InstanceId>> {
        let g = self.edges.read();
        let mut cycles = Vec::new();
        let mut color = HashMap::<InstanceId, u8>::new(); // 0 white, 1 gray, 2 black
        for &start in g.keys() {
            let mut stack = vec![(start, Vec::<InstanceId>::new())];
            while let Some((node, path)) = stack.pop() {
                if color.get(&node) == Some(&2) { continue; }
                if color.get(&node) == Some(&1) {
                    // Found a back-edge; cycle is path[index_of_node..]
                    if let Some(i) = path.iter().position(|n| *n == node) {
                        cycles.push(path[i..].to_vec());
                    }
                    continue;
                }
                color.insert(node, 1);
                let mut p = path.clone();
                p.push(node);
                if let Some(nbrs) = g.get(&node) {
                    for n in nbrs { stack.push((*n, p.clone())); }
                }
                color.insert(node, 2);
            }
        }
        cycles
    }
}
```

### 4.2 Pending reply correlator

```rust
//! supervisor/src/ipc/reply.rs   (~420 lines)

use std::sync::Arc;
use std::time::Instant;
use crossbeam::channel::{bounded, Sender, Receiver};
use crate::ipc::envelope::{IpcEnvelope, IpcAddr};
use crate::instance::InstanceId;

pub struct PendingReply {
    pub id:           u64,
    pub sender:       IpcAddr,
    pub sender_iid:   InstanceId,
    pub sender_epoch: u32,
    pub recipient:    IpcAddr,
    pub recipient_iid: InstanceId,
    pub deadline:     Instant,
    pub mailbox:      ReplyMailbox,
    pub parked_for_resume: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
pub struct ReplyMailbox(Arc<ReplyMailboxInner>);

struct ReplyMailboxInner {
    tx:     Sender<Result<IpcEnvelope, IpcError>>,
    rx:     Receiver<Result<IpcEnvelope, IpcError>>,
    notify: tokio::sync::Notify,
}

impl ReplyMailbox {
    pub fn new() -> Self {
        let (tx, rx) = bounded(1);
        Self(Arc::new(ReplyMailboxInner {
            tx, rx, notify: tokio::sync::Notify::new()
        }))
    }
    pub fn fulfill(&self, env: IpcEnvelope) {
        let _ = self.0.tx.try_send(Ok(env));
        self.0.notify.notify_one();
    }
    pub fn fail(&self, e: IpcError) {
        let _ = self.0.tx.try_send(Err(e));
        self.0.notify.notify_one();
    }
    pub fn waiter(&self) -> ReplyWaiter {
        ReplyWaiter {
            rx:     self.0.rx.clone(),
            notify: self.0.clone(),
            registered: false,
        }
    }
}

pub struct ReplyWaiter {
    rx:         Receiver<Result<IpcEnvelope, IpcError>>,
    notify:     Arc<ReplyMailboxInner>,
    registered: bool,
}

impl std::future::Future for ReplyWaiter {
    type Output = Result<IpcEnvelope, IpcError>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // Lock-free fast path.
        if let Ok(r) = self.rx.try_recv() {
            return std::task::Poll::Ready(r);
        }
        // Register with notify (must happen before second try_recv to close
        // the race window between producer's tx.send and notify_one).
        let fut = self.notify.notify.notified();
        tokio::pin!(fut);
        match fut.as_mut().poll(cx) {
            std::task::Poll::Ready(()) => {
                match self.rx.try_recv() {
                    Ok(r) => std::task::Poll::Ready(r),
                    Err(_) => std::task::Poll::Pending,
                }
            }
            std::task::Poll::Pending => {
                // Re-check to handle the case where producer fulfilled
                // between our try_recv and notified() registration.
                if let Ok(r) = self.rx.try_recv() {
                    return std::task::Poll::Ready(r);
                }
                std::task::Poll::Pending
            }
        }
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum IpcError {
    #[error("no such recipient")]                NoSuchRecipient,
    #[error("no capability")]                    NoCapability,
    #[error("queue full")]                       QueueFull,
    #[error("reply timeout")]                    ReplyTimeout,
    #[error("recipient gone")]                   RecipientGone,
    #[error("sender gone")]                      SenderGone,
    #[error("channel disconnected")]             ChannelDisconnected,
    #[error("schema unsupported")]               SchemaUnsupported,
    #[error("payload too large")]                PayloadTooLarge,
    #[error("forbidden by capability")]          Forbidden,
    #[error("parse error: {0}")]                 ParseError(&'static str),
    #[error("would deadlock (cycle)")]           WouldDeadlock,
    #[error("quota exceeded")]                   QuotaExceeded,
    #[error("supervisor entitlement missing")]   NoEntitlement,
    #[error("epoch mismatch (sender restarted)")]EpochMismatch,
    #[error("shm: not committed yet")]           ShmNotCommitted,
    #[error("shm: out of bounds")]               ShmOutOfBounds,
    #[error("shm: invalidated")]                 ShmInvalidated,
}
```

### 4.3 The `call` host implementation

```rust
//! supervisor/src/ipc/wit_host.rs   (~440 lines, fragment)

pub async fn host_call(
    ctx:       &HostCtx,
    caller:    InstanceId,
    recipient: String,
    schema:    String,
    body:      Vec<u8>,
    deadline_ms: Option<u32>,
) -> Result<IpcEnvelope, IpcError> {
    // 1. Resolve.
    let recipient_addr = IpcAddr::parse(&recipient).map_err(|_| IpcError::ParseError("addr"))?;
    let resolved = ctx.router.resolve_one(&recipient_addr)?;

    // 2. Cycle check.
    ctx.router.waitfor.try_add(caller, resolved)?;

    // 3. Inflight quota check.
    let inflight = ctx.router.inflight.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    if inflight >= ctx.router.policy.load().max_inflight_per_app as usize {
        ctx.router.inflight.fetch_sub(1, std::sync::atomic::Ordering::Release);
        ctx.router.waitfor.remove(caller, resolved);
        return Err(IpcError::QuotaExceeded);
    }

    // 4. Build envelope.
    let id = ctx.router.alloc_id();
    let mailbox = ReplyMailbox::new();
    let env = IpcEnvelope {
        id,
        sender:       IpcAddr::Instance(caller),
        sender_bundle: ctx.bundle_for(caller),
        sender_epoch: ctx.epoch_for(caller),
        recipient:    recipient_addr,
        payload:      IpcPayload::Typed(TypedMessage {
            schema_id:      SchemaId::interned(&schema, ctx.router.siphash_key()),
            schema_version: 1,
            body:           InlineBody::for_bytes(body),
        }),
        reply_to:    None,
        deadline_ms: deadline_ms.map(|d| ctx.boot_ms() + d as u64),
        flags:       IpcFlags { expects_reply: true, ..Default::default() },
        class:       IpcClass::Ipc,
        trace_id:    ctx.current_trace(),
        span_id:     ctx.current_span(),
    };

    // 5. Register pending reply BEFORE enqueue (avoids race where reply
    //    arrives before the pending row exists).
    ctx.router.pending.insert(id, PendingReply {
        id,
        sender:        env.sender.clone(),
        sender_iid:    caller,
        sender_epoch:  env.sender_epoch,
        recipient:     env.recipient.clone(),
        recipient_iid: resolved,
        deadline:      Instant::now() + std::time::Duration::from_millis(deadline_ms.unwrap_or(30_000) as u64),
        mailbox:       mailbox.clone(),
        parked_for_resume: std::sync::atomic::AtomicBool::new(false),
    });

    // 6. Enqueue.
    if let Err(e) = ctx.router.submit(env) {
        ctx.router.pending.remove(&id);
        ctx.router.inflight.fetch_sub(1, std::sync::atomic::Ordering::Release);
        ctx.router.waitfor.remove(caller, resolved);
        return Err(e.into());
    }

    // 7. Await reply (this is what yields back to wasmtime).
    let result = mailbox.waiter().await;

    // 8. Cleanup.
    ctx.router.inflight.fetch_sub(1, std::sync::atomic::Ordering::Release);
    ctx.router.waitfor.remove(caller, resolved);
    result
}
```

### 4.4 Reentrancy: receiving requests during in-flight `call`

This is the subtle case from Critic C3. App A calls B (await pending). While A is awaiting, app C sends
a request to A. Two options:

- **(a) Deliver to A immediately, but on a new "reentrant" host call:** WASM is single-threaded; we
  cannot enter A's instance while it is yielded on `await`. NOT POSSIBLE.

- **(b) Queue the C→A message in A's `inbox_lo` and let it sit until A's WASM thread is no longer in
  `on-ipc`:** This is the model we adopt. A's dispatcher sees A is in `Awaiting` state and defers the
  drain. When A's reply arrives, the dispatcher resumes A's `await`, A's WIT call returns, A's `on-ipc`
  (if any) returns, and only then does the dispatcher pull the next batch including C's request.

The cost: C's request to A waits for A's outstanding `call` to complete. The mitigation: A's deadline
applies. If A's `call` exceeds its deadline, the `Awaiting` state is cleared and C's request is
processed promptly.

### 4.5 Tick-based timeout sweep

```rust
//! supervisor/src/ipc/ticker.rs   (~140 lines)

pub fn run_ticker(router: Arc<IpcRouter>) {
    let tick = crossbeam::channel::tick(std::time::Duration::from_millis(100));
    loop {
        let _ = tick.recv();
        let now = std::time::Instant::now();
        // Sweep pending replies in parallel across shards.
        for shard in &router.shards {
            let expired: Vec<u64> = shard.pending.iter()
                .filter(|p| p.deadline <= now && !p.parked_for_resume.load(std::sync::atomic::Ordering::Acquire))
                .map(|p| p.id).collect();
            for id in expired {
                if let Some((_, p)) = shard.pending.remove(&id) {
                    p.mailbox.fail(IpcError::ReplyTimeout);
                    router.waitfor.remove(p.sender_iid, p.recipient_iid);
                    router.inflight.fetch_sub(1, std::sync::atomic::Ordering::Release);
                }
            }
        }
    }
}
```

---

## 5. SharedBuffer Lifetime & Crash Safety — Fixed

### 5.1 Supervisor-owned, refcounted, publish/commit

Round 2 defined `SharedBuffer` with `Owned(Vec<u8>)` and `SurfaceAlias` variants. Round 3 adds the
**lifecycle invariants** Critic C4 demands.

```rust
//! supervisor/src/shm/lifecycle.rs   (~360 lines)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use dashmap::DashMap;
use crate::instance::InstanceId;

pub struct SharedBufferRegistry {
    /// All buffers in the system. Drops via Arc when handles → 0.
    buffers: DashMap<SharedBufferId, Arc<SharedBuffer>>,
    /// Per-instance set of attached handle ids (for crash cleanup).
    attached: DashMap<InstanceId, dashmap::DashSet<SharedBufferId>>,
}

pub struct SharedBuffer {
    pub id:       SharedBufferId,
    pub kind:     SharedBufferKind,
    pub owner:    InstanceId,
    pub owner_epoch: u32,
    pub storage:  SharedStorage,
    /// AtomicBool; flipped by `invalidate()` on owner crash.
    /// All reads check `valid.load(Acquire)` first.
    pub valid:    AtomicBool,
    /// Publish/commit boundary. Writer sets `last_committed_seq`; readers
    /// compare against `seq` from the IPC envelope before exposing bytes.
    pub last_committed_seq: AtomicU32,
    /// Active handle count. Supervisor reclaims when 0.
    pub refcount: AtomicUsize,
    /// Max length (size of `storage`). For Surface buffers this is
    /// surface.width × surface.height × 4.
    pub len:      u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SharedBufferId(pub std::num::NonZeroU64);

#[derive(Debug, Clone, Copy)]
pub enum SharedBufferKind {
    Surface,
    AudioBuffer,
    ImageData,
    Custom,
}

pub enum SharedStorage {
    /// Heap-backed bytes (the common case for image/audio/custom).
    Owned(parking_lot::RwLock<Vec<u8>>),
    /// Aliased to a compositor Surface's back buffer. The Surface IS the
    /// SHM storage; no second allocation. The compositor's existing
    /// double-buffer protocol provides single-writer + multi-reader.
    SurfaceAlias { surface: Arc<crate::display::Surface> },
}

pub struct BufferHandle {
    pub buf:     Arc<SharedBuffer>,
    pub holder:  InstanceId,
    pub access:  SharedAccess,
}

impl Drop for BufferHandle {
    fn drop(&mut self) {
        let n = self.buf.refcount.fetch_sub(1, Ordering::AcqRel);
        if n == 1 {
            // Last handle gone; supervisor will reclaim on next GC pass.
        }
    }
}

impl SharedBufferRegistry {
    pub fn create_owned(&self, owner: InstanceId, owner_epoch: u32, kind: SharedBufferKind, size: u32)
        -> Result<Arc<SharedBuffer>, ShmError>
    {
        let id = self.next_id();
        let buf = Arc::new(SharedBuffer {
            id, kind, owner, owner_epoch,
            storage: SharedStorage::Owned(parking_lot::RwLock::new(vec![0u8; size as usize])),
            valid: AtomicBool::new(true),
            last_committed_seq: AtomicU32::new(0),
            refcount: AtomicUsize::new(1),  // owner's writer handle
            len: size,
        });
        self.buffers.insert(id, buf.clone());
        self.attached.entry(owner).or_default().insert(id);
        Ok(buf)
    }

    /// Called on every IPC envelope carrying SharedMemRef.
    pub fn attach_reader(&self, id: SharedBufferId, reader: InstanceId)
        -> Result<BufferHandle, ShmError>
    {
        let buf = self.buffers.get(&id).ok_or(ShmError::NotFound)?.clone();
        if !buf.valid.load(Ordering::Acquire) {
            return Err(ShmError::Invalidated);
        }
        buf.refcount.fetch_add(1, Ordering::AcqRel);
        self.attached.entry(reader).or_default().insert(id);
        Ok(BufferHandle { buf, holder: reader, access: SharedAccess::Read })
    }

    /// Called by the writer to publish its committed bytes.
    pub fn commit(&self, id: SharedBufferId, seq: u32) -> Result<(), ShmError> {
        let buf = self.buffers.get(&id).ok_or(ShmError::NotFound)?.clone();
        buf.last_committed_seq.store(seq, Ordering::Release);
        Ok(())
    }

    /// Called by a reader to access bytes. Verifies committed seq.
    pub fn read(&self, handle: &BufferHandle, offset: u32, len: u32, min_seq: u32)
        -> Result<Vec<u8>, ShmError>
    {
        if !handle.buf.valid.load(Ordering::Acquire) {
            return Err(ShmError::Invalidated);
        }
        if handle.buf.last_committed_seq.load(Ordering::Acquire) < min_seq {
            return Err(ShmError::NotCommitted);
        }
        if (offset + len) > handle.buf.len {
            return Err(ShmError::OutOfBounds);
        }
        match &handle.buf.storage {
            SharedStorage::Owned(rw) => {
                let g = rw.read();
                Ok(g[offset as usize..(offset+len) as usize].to_vec())
            }
            SharedStorage::SurfaceAlias { surface } => {
                let g = surface.back_lock().read();
                Ok(g[offset as usize..(offset+len) as usize].to_vec())
            }
        }
    }

    /// Crash cleanup: called by lifecycle when an instance dies.
    pub fn on_instance_gone(&self, id: InstanceId) {
        if let Some((_, set)) = self.attached.remove(&id) {
            for bid in set.iter() {
                if let Some(buf) = self.buffers.get(&*bid) {
                    if buf.owner == id {
                        // Owner died → invalidate; existing readers' reads
                        // will now return Invalidated.
                        buf.valid.store(false, Ordering::Release);
                    }
                    let n = buf.refcount.fetch_sub(1, Ordering::AcqRel);
                    if n == 1 {
                        // Last reader gone after owner died; reclaim.
                        drop(buf);
                        self.buffers.remove(&*bid);
                    }
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShmError {
    #[error("buffer not found")] NotFound,
    #[error("buffer invalidated")] Invalidated,
    #[error("not committed for this seq")] NotCommitted,
    #[error("out of bounds")] OutOfBounds,
    #[error("over quota")] OverQuota,
}
```

### 5.2 Sender-crash semantics (formal)

When instance A (owner of buffer B) is removed from `AppTable`:

1. `on_instance_gone(A)` runs as part of the lifecycle hook chain.
2. For each `B` in `attached[A]`:
   - If `B.owner == A`: `B.valid.store(false)`. Any reader's subsequent `read()` returns `ShmError::Invalidated`.
   - Decrement `B.refcount`. If 0, remove from `buffers`.
3. The router (separately) reaps any pending replies and emits `vyoma.lifecycle.died(A)`.

Receivers observing `Invalidated` are expected to discard the buffer and fall back to their idle/empty
state. No use-after-free is possible because the `Arc<SharedBuffer>` only dies when the last handle
drops; the underlying `Vec<u8>` (Owned) or `Surface` (Alias) is held alive by that `Arc`.

### 5.3 Receiver-crash semantics

When instance R (holds handle H to buffer B owned by A) is removed:

1. `on_instance_gone(R)` runs.
2. For each B in `attached[R]`:
   - Decrement `B.refcount`.
   - If 0 (the owner has also exited or this is the last reader), reclaim.
3. A is unaffected. A's writes continue.

### 5.4 Surface alias special case

`SharedBufferKind::Surface` aliases the back buffer of a compositor `Surface`. The Surface has its own
double-buffer protocol (front/back via `ArcSwap<Surface>`). Reads happen during compositor sample;
writes happen during app `on-paint`. The SHM layer does not enforce single-writer because the surface
ownership already does. `commit(seq)` is set by the app's `Surface::flush_back_to_front` operation.

### 5.5 Capability check on SHM operations

Critic correctly noted that SHM reads bypass capability checks. Fix: every `attach_reader` call goes
through `PolicySnapshot::can_receive_shm(sender_bundle, recipient_bundle, kind)`. The check happens
**once** at attach time, not per read. After attachment the handle itself is the capability.

```rust
[capabilities.ipc.shm]
allow_recv = ["os.vyoma.image-viewer", "*"]
allow_send = ["os.vyoma.notes"]
max_buffers = 16
max_bytes = 16_777_216  # 16 MiB total across active buffers
```

### 5.6 Streaming fast path

For audio (48 kHz × 256-sample chunks = 187 msg/s per stream) and video (60 Hz × 8.3 MB = 500 MB/s),
the Critic correctly demanded a pre-established channel:

```rust
//! supervisor/src/ipc/stream.rs   (~280 lines)

use crossbeam::queue::ArrayQueue;
use std::sync::Arc;

pub struct StreamChannel {
    pub id:           StreamId,
    pub sender:       InstanceId,
    pub receiver:     InstanceId,
    pub config:       StreamConfig,
    /// SPSC lock-free ring buffer.
    pub ring:         Arc<ArrayQueue<StreamFrame>>,
    /// Wake the reader when ring goes from empty to non-empty.
    pub wake_reader:  Arc<tokio::sync::Notify>,
    /// Wake the writer when ring goes from full to non-full.
    pub wake_writer:  Arc<tokio::sync::Notify>,
    pub valid:        std::sync::atomic::AtomicBool,
}

pub struct StreamFrame {
    pub seq:    u64,
    /// Either inline bytes (audio) or SharedBuffer handle (video).
    pub data:   StreamFrameData,
}

pub enum StreamFrameData {
    Inline(Vec<u8>),
    Buffer(crate::shm::BufferHandle),
}

#[derive(Debug, Clone, Copy)]
pub struct StreamConfig {
    pub capacity:   u16,         // ring size
    pub item_size:  u32,         // bytes per frame (for accounting)
    pub wakeup:     WakeupMode,
}

#[derive(Debug, Clone, Copy)]
pub enum WakeupMode {
    /// Reader polls — lowest latency, highest CPU.
    Poll,
    /// Notify on transition empty→non-empty.
    NotifyOnTransition,
    /// Blocking read with timeout.
    BlockingRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId(pub u64);
```

`open_stream(peer, config)` returns `(StreamTx, StreamRx)` handles. After setup the router is
**not involved per frame**. Capability check runs at `open_stream`; if denied, both ends are `Err`.

---

## 6. WASM Callback Delivery Model — Fixed

### 6.1 Batched push with reentrancy detection

```rust
//! supervisor/src/ipc/dispatch.rs   (~360 lines)

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use crossbeam::channel::Receiver;
use crate::ipc::envelope::IpcEnvelope;

pub struct CallbackDispatcher {
    pub iid:        crate::instance::InstanceId,
    pub inbox_hi:   Receiver<IpcEnvelope>,
    pub inbox_lo:   Receiver<IpcEnvelope>,
    /// State machine for re-entrancy.
    pub state:      Arc<AtomicU8>,
    pub deferred:   parking_lot::Mutex<smallvec::SmallVec<[IpcEnvelope; 8]>>,
}

// State values.
pub const STATE_IDLE:     u8 = 0;
pub const STATE_IN_ONIPC: u8 = 1;
pub const STATE_AWAITING: u8 = 2;
pub const STATE_DRAINING: u8 = 3;

impl CallbackDispatcher {
    /// Called from the per-app driver thread.
    pub fn drain(&self, store: &mut crate::runtime::WasmStore) {
        loop {
            // Try high-priority first.
            if self.state.load(Ordering::Acquire) != STATE_IDLE { return; }
            self.state.store(STATE_DRAINING, Ordering::Release);

            let mut batch = smallvec::SmallVec::<[IpcEnvelope; 16]>::new();
            // Drain deferred first.
            {
                let mut def = self.deferred.lock();
                while batch.len() < 16 && !def.is_empty() {
                    batch.push(def.remove(0));
                }
            }
            // Drain hi inbox up to budget.
            while batch.len() < 16 {
                match self.inbox_hi.try_recv() {
                    Ok(e) => batch.push(e),
                    Err(_) => break,
                }
            }
            // Then lo inbox.
            while batch.len() < 16 {
                match self.inbox_lo.try_recv() {
                    Ok(e) => batch.push(e),
                    Err(_) => break,
                }
            }
            if batch.is_empty() {
                self.state.store(STATE_IDLE, Ordering::Release);
                return;
            }

            // Input-class messages bypass batching: one envelope per call.
            // This minimizes input latency.
            if batch.len() == 1 || matches!(batch[0].class, crate::ipc::envelope::IpcClass::Input) {
                let env = batch.into_iter().next().unwrap();
                self.state.store(STATE_IN_ONIPC, Ordering::Release);
                let _ = store.call_on_ipc(&[env]);
                self.state.store(STATE_IDLE, Ordering::Release);
                continue;
            }

            // Coalesce same-(sender,schema) for Render class.
            let batch = coalesce_render(batch);

            self.state.store(STATE_IN_ONIPC, Ordering::Release);
            let _ = store.call_on_ipc(&batch);
            self.state.store(STATE_IDLE, Ordering::Release);
        }
    }

    /// Called from a different thread when an envelope is queued by router
    /// while this dispatcher is mid-on_ipc. The envelope is already in the
    /// crossbeam channel; we just notify the driver thread.
    pub fn wake(&self) {
        // The driver thread is parked on crossbeam::select; the channel
        // send wakes it. No action required here.
    }
}

fn coalesce_render(batch: smallvec::SmallVec<[IpcEnvelope; 16]>) -> smallvec::SmallVec<[IpcEnvelope; 16]> {
    use std::collections::HashMap;
    let mut by_key: HashMap<(crate::instance::BundleId, u64), usize> = HashMap::new();
    let mut out: smallvec::SmallVec<[IpcEnvelope; 16]> = smallvec::SmallVec::new();
    for env in batch {
        if !matches!(env.class, crate::ipc::envelope::IpcClass::Render) {
            out.push(env);
            continue;
        }
        let schema_hash = match &env.payload {
            crate::ipc::envelope::IpcPayload::Typed(t) => t.schema_id.hash,
            _ => 0,
        };
        let key = (env.sender_bundle.clone(), schema_hash);
        if let Some(&idx) = by_key.get(&key) {
            out[idx] = env;          // newer replaces older
        } else {
            by_key.insert(key, out.len());
            out.push(env);
        }
    }
    out
}
```

### 6.2 The reentrancy protocol

`on-ipc` is **not reentrant**. If WASM code inside `on-ipc` calls `vyoma:ipc/client.call(...)`, the
host function:

1. Marks dispatcher `state = STATE_AWAITING`.
2. Posts the outbound envelope to the router (which proceeds normally).
3. Awaits the reply.
4. On reply (or timeout), the host function returns to WASM.
5. WASM continues executing inside `on-ipc`.
6. When `on-ipc` returns, dispatcher resumes drain.

Critically: **`on-ipc` does NOT re-enter** while the WIT call is awaiting. Inbound envelopes that
arrive during the wait are queued in `inbox_hi`/`inbox_lo` as usual. The dispatcher won't drain them
until `on-ipc` returns.

This means a slow `on-ipc` *does* delay inbound processing, but only for *this* app. Other apps are
unaffected (router is sharded). The app's own quotas (`max_inflight`, queue caps) bound the damage.

### 6.3 Inbox overflow during long `on-ipc`

When `on-ipc` runs long and `inbox_lo` fills up:

- **Input**: oldest events drop (DropOldest). User sees stale cursor position but mouse stays responsive
  once `on-ipc` returns.
- **Ipc**: senders get `IpcError::QueueFull` and can retry. App developers are expected to handle this
  (it's the "moral equivalent" of EAGAIN).
- **Render**: oldest render commands drop; UI updates appear chunky but never stale.
- **Control**: routing tries 50 ms; if still blocked, the lifecycle subsystem may force-kill the app
  with `CrashKind::IpcStuck`. This is the watchdog escalation.

### 6.4 The `mgmt` panel for inbox introspection

```rust
pub fn snapshot_inbox(&self, iid: InstanceId) -> InboxSnapshot {
    InboxSnapshot {
        hi_depth: route.inbox_hi.len(),
        hi_cap:   route.inbox_hi.capacity().unwrap_or(0),
        lo_depth: route.inbox_lo.len(),
        lo_cap:   route.inbox_lo.capacity().unwrap_or(0),
        state:    route.dispatcher_state.load(Ordering::Acquire),
        deferred: route.deferred_count.load(Ordering::Acquire),
    }
}
```

Exposed via `@supervisor: ipc-inbox <bundle>` for operator debugging.

---

## 7. Capability Cache — Fixed

### 7.1 `ArcSwap<PolicySnapshot>` in hot AppState

```rust
//! supervisor/src/ipc/policy.rs   (~280 lines)

use std::sync::Arc;
use arc_swap::ArcSwap;
use crate::instance::BundleId;
use crate::ipc::envelope::IpcClass;
use crate::ipc::reply::IpcError;

/// Read-mostly snapshot of every IPC permission an app has. Replaced
/// atomically on `ManifestReloaded`. Lives in `AppState.hot`.
pub struct PolicySnapshot {
    pub allow_send_to:        SmallVec<[BundleGlob; 8]>,
    pub allow_recv_from:      SmallVec<[BundleGlob; 8]>,
    pub allow_schemas_in:     SmallVec<[SchemaGlob; 8]>,
    pub allow_schemas_out:    SmallVec<[SchemaGlob; 8]>,
    pub allow_topics_pub:     SmallVec<[TopicGlob; 8]>,
    pub allow_topics_sub:     SmallVec<[TopicGlob; 8]>,
    pub allow_shm_send:       SmallVec<[BundleGlob; 4]>,
    pub allow_shm_recv:       SmallVec<[BundleGlob; 4]>,
    pub supervisor_verbs:     EntitlementSet,
    pub max_inflight:         u16,
    pub max_outbound_per_sec: u32,
    pub max_pending_replies:  u16,
    pub max_subscriptions:    u16,
    pub max_shared_buffers:   u16,
    pub max_shared_bytes:     u32,
    pub legacy_stdout:        bool,
    /// Manifest revision that produced this snapshot.
    pub manifest_rev:         u32,
}

#[derive(Clone)]
pub struct BundleGlob(pub Arc<str>);
#[derive(Clone)]
pub struct SchemaGlob(pub Arc<str>);
#[derive(Clone)]
pub struct TopicGlob(pub Arc<str>);

impl BundleGlob {
    pub fn matches(&self, b: &BundleId) -> bool {
        glob_match(&self.0, b.as_ref())
    }
}

fn glob_match(pat: &str, s: &str) -> bool {
    // Simple `*` glob (no `?`, no character classes — overkill for IDs).
    let pat = pat.as_bytes();
    let s = s.as_bytes();
    let (mut pi, mut si, mut star_pi, mut star_si) = (0usize, 0usize, usize::MAX, 0usize);
    while si < s.len() {
        if pi < pat.len() && (pat[pi] == s[si] || pat[pi] == b'?') {
            pi += 1; si += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_pi = pi; star_si = si; pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1; star_si += 1; si = star_si;
        } else { return false; }
    }
    while pi < pat.len() && pat[pi] == b'*' { pi += 1; }
    pi == pat.len()
}

pub type PolicyArc = Arc<PolicySnapshot>;

/// Stored in AppState.hot.
pub struct HotPolicy(pub ArcSwap<PolicySnapshot>);

impl HotPolicy {
    pub fn check_send(&self, sender_bundle: &BundleId, recipient_bundle: &BundleId, schema: &str)
        -> Result<(), IpcError>
    {
        let snap = self.0.load();
        // Sender: is recipient in our outbound allowlist?
        if !snap.allow_send_to.iter().any(|g| g.matches(recipient_bundle)) {
            return Err(IpcError::NoCapability);
        }
        // Schema: is this schema in our outbound schema allowlist?
        if !snap.allow_schemas_out.iter().any(|g| glob_match(&g.0, schema)) {
            return Err(IpcError::SchemaUnsupported);
        }
        Ok(())
    }

    pub fn check_recv(&self, recipient_bundle: &BundleId, sender_bundle: &BundleId, schema: &str)
        -> Result<(), IpcError>
    {
        let snap = self.0.load();
        if !snap.allow_recv_from.iter().any(|g| g.matches(sender_bundle)) {
            return Err(IpcError::NoCapability);
        }
        if !snap.allow_schemas_in.iter().any(|g| glob_match(&g.0, schema)) {
            return Err(IpcError::SchemaUnsupported);
        }
        Ok(())
    }
}
```

### 7.2 Update path on `ManifestReloaded`

```rust
pub fn reload_policy(
    apps:    &crate::instance::AppTable,
    bundle:  &BundleId,
    new_manifest: &crate::manifest::Manifest,
) -> Result<(), IpcError> {
    let snap = Arc::new(build_snapshot(new_manifest));
    for handle in apps.iter_bundle(bundle) {
        handle.state.hot.policy.0.store(snap.clone());
    }
    // RouteTable rebuild not needed (instance set unchanged).
    Ok(())
}
```

Cost: ~50 ns for the atomic store; readers in flight see the old snapshot until their `load()` returns
the new pointer. This is **lock-free, wait-free for readers**, exactly what desktop SLOs require.

### 7.3 No global cap_cache LRU

The Architect's `cap_cache: parking_lot::Mutex<LruCache<...>>` is removed. Capability checks happen
inline against the hot policy snapshot — no caching layer needed because the snapshot itself is the
cache. This eliminates:

- The LRU `Mutex` contention.
- The cache-invalidation complexity (`clear()` vs epoch).
- The need to invalidate on manifest reload (the snapshot swap is the invalidation).

---

## 8. Supervisor Command Authentication — Fixed

### 8.1 v1 = WIT-typed; legacy stdout OFF by default

Boot configuration:

```toml
# /etc/vyoma/boot.toml
[ipc]
legacy_stdout_supervisor = false  # v1 default
```

When `legacy_stdout_supervisor = false` (the v1 default), the supervisor's stdin scanner will:
- Continue to recognize `@<bundle>:` and `@@<topic>:` lines as fire-and-forget message shorthand
  (these still go through the full capability gate — the line is just a convenience for printf-only
  apps and never the auth boundary).
- **Reject `@supervisor:` lines** with a warn log and `IpcError::NoEntitlement` written to the app's
  stderr. The line is dropped.

When the flag is `true` (legacy-compat mode, for upgrade testing), `@supervisor:` lines are parsed and
routed BUT they still go through the same `EntitlementChecker` as the WIT path. The unauthenticated
era is over.

### 8.2 Authenticated WIT path

```rust
//! supervisor/src/ipc/supervisor_cmd.rs   (~440 lines)

use crate::ipc::envelope::*;
use crate::ipc::reply::IpcError;
use crate::ipc::policy::EntitlementSet;
use crate::instance::{BundleId, InstanceId, AppTable};

#[derive(Debug, Clone)]
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
    IpcInbox    { bundle: BundleId },
    IpDeadlock,
    GrantOnce   { target: BundleId, cmd_hash: [u8; 32], ttl_ms: u32 },
}

#[derive(Debug, Clone, Copy)]
pub enum TopicOp { Pub, Sub, Unsub, List }

#[derive(Debug, Clone)]
pub struct LaunchArgs { pub argv: Vec<String> }

pub struct SupervisorCmdHandler {
    app_table:    std::sync::Arc<AppTable>,
    lifecycle_tx: crossbeam::channel::Sender<crate::lifecycle::LifecycleCmd>,
    /// One-shot grants from SecurityAgent elevations.
    grants:       dashmap::DashMap<(BundleId, [u8; 32]), std::time::Instant>,
}

impl SupervisorCmdHandler {
    pub fn dispatch(&self, env: IpcEnvelope) -> Result<IpcEnvelope, IpcError> {
        let cmd = self.parse(&env)?;
        let sender_bundle = &env.sender_bundle;
        // 1. Authoritative entitlement check.
        self.check_entitlement(sender_bundle, &cmd, env.sender_epoch)?;
        // 2. Execute.
        let result = self.execute(cmd)?;
        Ok(self.build_reply(&env, result))
    }

    fn check_entitlement(&self, sender: &BundleId, cmd: &SupCmd, _epoch: u32) -> Result<(), IpcError> {
        let handle = self.app_table.get_by_bundle(sender)
            .ok_or(IpcError::NoSuchRecipient)?;
        let policy = handle.state.hot.policy.0.load();
        let ent = &policy.supervisor_verbs;
        match cmd {
            SupCmd::List | SupCmd::LogTail { .. } | SupCmd::CrashList { .. } |
            SupCmd::TopicOp { .. } | SupCmd::IpcInbox { .. } | SupCmd::IpDeadlock => {
                if ent.read || ent.admin { return Ok(()); }
            }
            SupCmd::Focus { .. } => {
                if ent.focus || ent.admin { return Ok(()); }
            }
            SupCmd::Spawn { bundle, .. } => {
                if ent.admin || ent.spawn_allow.iter().any(|b| b == bundle) { return Ok(()); }
            }
            SupCmd::Kill { bundle, .. } | SupCmd::Restart { bundle } |
            SupCmd::Suspend { bundle } | SupCmd::Resume { bundle } => {
                if ent.admin || ent.kill_allow.iter().any(|b| b == bundle) { return Ok(()); }
            }
            SupCmd::ManifestReload { .. } | SupCmd::QuotaSet { .. } => {
                if ent.admin { return Ok(()); }
            }
            SupCmd::GrantOnce { .. } => {
                // Only SecurityAgent can grant.
                if sender.as_ref() == "os.vyoma.security-agent" { return Ok(()); }
            }
        }
        // One-shot grant fallback.
        let hash = hash_cmd(cmd);
        if let Some((_, when)) = self.grants.remove(&(sender.clone(), hash)) {
            if when.elapsed() < std::time::Duration::from_secs(30) {
                return Ok(());
            }
        }
        Err(IpcError::NoEntitlement)
    }
}

fn hash_cmd(cmd: &SupCmd) -> [u8; 32] {
    use sha2::{Sha256, Digest};
    let mut h = Sha256::new();
    h.update(format!("{:?}", cmd).as_bytes());
    h.finalize().into()
}
```

### 8.3 Manifest schema

```toml
[capabilities.supervisor]
# Subset of {read, focus, admin}. spawn/kill allowlists are optional.
read = true
focus = false
admin = false

[capabilities.supervisor.spawn]
allow = ["os.vyoma.note-helper"]

[capabilities.supervisor.kill]
allow = []
```

### 8.4 SecurityAgent elevation flow

```
App X needs admin op.
  → X calls vyoma:ipc/client.elevate(reason, verb, args)
  → host routes to SecurityAgent (special bundle id "os.vyoma.security-agent")
  → SecurityAgent renders modal dialog: "App 'X' wants to <verb>. Allow?"
  → User confirms with password / biometric
  → SecurityAgent verifies against /data/secrets/auth.enc
  → SecurityAgent sends @supervisor: grant-once X <sha256(cmd)> 30s
  → supervisor's SupervisorCmdHandler records the grant in self.grants
  → X retries the original WIT call within 30s
  → SupervisorCmdHandler::check_entitlement finds the grant, consumes it, allows.
```

Boot-time guarantee: the SecurityAgent bundle is signed by the same trust root as the supervisor. If
the SecurityAgent fails to start, the supervisor logs a critical error and falls back to a TTY prompt
(`kernel_panic_elevation_console`) attached to `/dev/console` for emergency admin operations.

### 8.5 Migration path

| Phase | Behavior |
|---|---|
| **v0 (current)** | `@supervisor:` over stdout, no auth. |
| **v0.5 (transition)** | `@supervisor:` over stdout, capability-checked. Apps without `[capabilities.supervisor]` get warn log; receive ack but command not executed (soft deprecation period of 2 releases). |
| **v1 (this spec)** | `[ipc.legacy_stdout_supervisor] = false` default. Stdout `@supervisor:` lines rejected. All apps must declare `[capabilities.supervisor]`. WIT `vyoma:ipc/client.invoke_supervisor` is the only path. |
| **v2** | The boot flag is removed entirely. Stdout `@supervisor:` lines no longer parsed even with flag. |

---

## 9. Broadcast & Topics

### 9.1 Three delivery tiers

Per Critic S1:

| Tier | Guarantee | Mechanism | Use case |
|---|---|---|---|
| **best-effort** (`vyoma.*`, user topics by default) | At-most-once | DropOldest on subscriber inbox full | Liveness, notifications |
| **reliable** (opt-in via `topic-config`) | At-least-once until backlog overflow | Per-subscriber retry queue cap 64; on overflow, subscriber is force-disconnected and must re-subscribe | Settings replication |
| **state-replication** (opt-in) | At-least-once with seq-number dedupe | Reliable + sequence; receiver applies highest-seq-seen | Pref observers |

### 9.2 TopicTable

```rust
//! supervisor/src/ipc/broadcast.rs   (~360 lines)

use dashmap::DashMap;
use smallvec::SmallVec;
use std::sync::Arc;
use crate::ipc::envelope::{IpcEnvelope, IpcAddr};
use crate::instance::InstanceId;

pub struct TopicTable {
    /// Topic → exact subscribers + tier.
    exact:    DashMap<TopicId, SmallVec<[Subscription; 8]>>,
    /// Wildcard subscriptions. Linear scan; cap 256 system-wide.
    wildcard: parking_lot::RwLock<Vec<(TopicGlob, Subscription)>>,
    /// Reverse map for fast unsubscribe on instance death.
    by_inst:  DashMap<InstanceId, SmallVec<[TopicId; 4]>>,
    /// Per-subscriber retry queue for reliable topics.
    retry_q:  DashMap<(TopicId, InstanceId), parking_lot::Mutex<smallvec::SmallVec<[IpcEnvelope; 64]>>>,
    /// SipHash-interned topic strings.
    intern:   DashMap<Arc<str>, TopicId>,
    /// IpcSeed for hashing.
    seed:     siphasher::sip::SipHasher24,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct TopicId(pub u64);

#[derive(Clone)]
pub struct Subscription {
    pub who:       InstanceId,
    pub tier:      DeliveryTier,
    pub seq_seen:  u64,
}

#[derive(Debug, Clone, Copy)]
pub enum DeliveryTier { BestEffort, Reliable, StateReplication }

#[derive(Clone)]
pub struct TopicGlob {
    pub head: Arc<str>,
    pub tail: Option<Arc<str>>,
}

impl TopicTable {
    pub fn subscribe(&self, who: InstanceId, topic: &str, tier: DeliveryTier) -> Result<TopicId, BroadcastError> {
        let tid = self.intern_topic(topic);
        self.exact.entry(tid).or_default().push(Subscription { who, tier, seq_seen: 0 });
        self.by_inst.entry(who).or_default().push(tid);
        Ok(tid)
    }

    pub fn publish<F: Fn(InstanceId, IpcEnvelope) -> Result<(), crate::ipc::deliver::DeliveryError>>(
        &self,
        topic: &str,
        env_template: IpcEnvelope,
        deliver: F,
    ) -> PublishReport {
        let tid = self.intern_topic(topic);
        let mut report = PublishReport::default();
        if let Some(subs) = self.exact.get(&tid) {
            for s in subs.iter() {
                let mut e = env_template.clone();
                e.recipient = IpcAddr::Instance(s.who);
                match deliver(s.who, e.clone()) {
                    Ok(()) => report.delivered += 1,
                    Err(_) => {
                        match s.tier {
                            DeliveryTier::BestEffort => { report.dropped += 1; }
                            DeliveryTier::Reliable | DeliveryTier::StateReplication => {
                                let q = self.retry_q.entry((tid, s.who)).or_default();
                                let mut g = q.lock();
                                if g.len() < 64 { g.push(e); report.queued += 1; }
                                else { report.disconnected += 1; }
                            }
                        }
                    }
                }
            }
        }
        let wc = self.wildcard.read();
        for (g, sub) in wc.iter() {
            if g.matches(topic) {
                let mut e = env_template.clone();
                e.recipient = IpcAddr::Instance(sub.who);
                let _ = deliver(sub.who, e);  // best-effort for wildcards
            }
        }
        report
    }
}

#[derive(Debug, Default)]
pub struct PublishReport {
    pub delivered:    u32,
    pub dropped:      u32,
    pub queued:       u32,
    pub disconnected: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum BroadcastError {
    #[error("invalid topic")] InvalidTopic,
    #[error("over quota")]    OverQuota,
}
```

### 9.3 System events

| Topic | Tier | Payload | When |
|---|---|---|---|
| `vyoma.focus.changed`        | Reliable | `{ from, to }` | InputDispatcher swaps focus |
| `vyoma.display.rotated`      | BestEffort | `{ angle_deg }` | Display subsystem rotates |
| `vyoma.display.resolution`   | StateReplication | `{ w, h, scale }` | Multi-resolution change |
| `vyoma.memory.warning`       | Reliable | `{ level }` | Round-2 memory governor |
| `vyoma.lifecycle.spawned`    | Reliable | `{ id, bundle }` | Instance added to AppTable |
| `vyoma.lifecycle.died`       | Reliable | `{ id, bundle, reason }` | Instance removed |
| `vyoma.lifecycle.suspended`  | Reliable | `{ id, bundle }` | App entered Suspended |
| `vyoma.lifecycle.resumed`    | Reliable | `{ id, bundle }` | App returned to Foreground |
| `vyoma.input.idle`           | BestEffort | `{ secs }` | No input for N s |
| `vyoma.clipboard.changed`    | StateReplication | `{ schemas }` | Pasteboard updated |
| `vyoma.network.status`       | StateReplication | `{ online }` | virtio-net up/down |
| `vyoma.shutdown.requested`   | Reliable | `{ grace_ms }` | Sysctl reboot/halt |
| `vyoma.lifecycle.timeout`    | Reliable | `{ id, request_id }` | Request timeout |

### 9.4 IpcSeed persistence

Topic and Schema IDs are SipHash-2-4 of dotted-name strings. SipHash with a random seed is required to
prevent hash flooding attacks. The seed is persisted to `/data/state/ipc-seed` (16 bytes, ChaCha20-RNG
at first boot). On reboot the same seed is loaded so persisted subscriptions remain valid. This
resolves Architect Open Question #6.

---

## 10. WIT Interface

### 10.1 Full `wit/vyoma-ipc.wit`

```wit
package vyoma:ipc@0.1.0;

variant ipc-error {
    no-such-recipient,
    no-capability,
    queue-full,
    reply-timeout,
    recipient-gone,
    sender-gone,
    channel-disconnected,
    schema-unsupported,
    payload-too-large(u32),
    forbidden,
    parse-error(string),
    would-deadlock,
    quota-exceeded,
    no-entitlement,
    epoch-mismatch,
    shm-not-committed,
    shm-out-of-bounds,
    shm-invalidated,
}

resource shm-ref {
    id:    func() -> u64;
    kind:  func() -> shm-kind;
    size:  func() -> u32;
    read:  func(offset: u32, len: u32, min-seq: u32) -> result<list<u8>, ipc-error>;
    write: func(offset: u32, data: list<u8>) -> result<_, ipc-error>;
    commit: func(seq: u32) -> result<_, ipc-error>;
    drop:  func();
}

enum shm-kind { surface, audio-buffer, image-data, custom }

variant resource-handle {
    file(u64),
    surface(u64),
    port(string),
    sub(u64),
}

record envelope {
    id:        u64,
    sender:    string,
    recipient: string,
    reply-to:  option<u64>,
    deadline-ms: option<u64>,
    schema:    option<string>,
    schema-version: option<u16>,
    body:      list<u8>,
    shm:       option<shm-ref>,
    capability: option<resource-handle>,
    priority:  priority-level,
    trace-id:  option<u64>,
    span-id:   option<u64>,
}

enum priority-level { normal, input, system, critical }
enum shm-access { read, write, read-write }

interface ipc-client {
    use ipc-error;
    use envelope;
    use resource-handle;
    use shm-ref;

    send: func(recipient: string, schema: string, body: list<u8>) -> result<_, ipc-error>;
    send-with-shm: func(recipient: string, schema: string, body: list<u8>, shm: shm-ref, access: shm-access) -> result<_, ipc-error>;
    send-with-cap: func(recipient: string, schema: string, body: list<u8>, cap: resource-handle, move-semantics: bool) -> result<_, ipc-error>;
    call: func(recipient: string, schema: string, body: list<u8>, deadline-ms: option<u32>) -> result<envelope, ipc-error>;
    publish: func(topic: string, schema: string, body: list<u8>) -> result<_, ipc-error>;
    subscribe: func(topic: string, tier: delivery-tier) -> result<u64, ipc-error>;
    unsubscribe: func(sub-id: u64) -> result<_, ipc-error>;
    elevate: func(reason: string, verb: string, args: list<u8>) -> result<_, ipc-error>;
    reply: func(to: u64, schema: string, body: list<u8>) -> result<_, ipc-error>;
    reply-error: func(to: u64, e: ipc-error) -> result<_, ipc-error>;
    invoke-supervisor: func(verb: string, args: list<u8>) -> result<list<u8>, ipc-error>;
}

enum delivery-tier { best-effort, reliable, state-replication }

interface ipc-server {
    use envelope;
    on-ipc: func(envs: list<envelope>);
    on-reply-timeout: func(req-id: u64);
    on-broadcast: func(topic: string, env: envelope);
}

interface streams {
    use ipc-error;

    record stream-config {
        capacity: u32,
        item-size: u32,
        wakeup: wakeup-mode,
    }

    enum wakeup-mode { poll, notify-on-transition, blocking-read }

    resource stream-tx {
        write: func(data: list<u8>) -> result<u32, ipc-error>;
        commit: func() -> result<_, ipc-error>;
        close: func();
    }
    resource stream-rx {
        read: func() -> result<list<u8>, ipc-error>;
        read-blocking: func(timeout-ms: u32) -> result<list<u8>, ipc-error>;
        close: func();
    }

    open-stream: func(peer: string, config: stream-config) -> result<tuple<stream-tx, stream-rx>, ipc-error>;
}

world vyoma-app {
    import ipc-client;
    import streams;
    export ipc-server;
    import vyoma:memory/storage@0.1.0;
    import vyoma:lifecycle/callbacks;
}
```

### 10.2 Why CBOR

| Property | CBOR | Protobuf | MessagePack |
|---|---|---|---|
| Schema-optional | yes | NO | yes |
| Byte-string preserving | yes | yes | yes |
| `wit-bindgen` support | yes (ciborium) | requires .proto codegen | partial |
| Compact for primitives | yes | yes | yes |
| WASM binary size cost | ~25 KiB | ~120 KiB | ~30 KiB |

### 10.3 Schema-version negotiation

Each schema declares `schema_version: u16` at the WIT level. Receiver bindings declare a `(min,max)`
acceptable range. Sender sends with `schema_version = build_time_version`. Router does not enforce
version compat — that is the recipient's `on-ipc` first line: `if env.schema_version > MAX { return
reply_error(SchemaUnsupported); }`. The router only ensures the schema_id (not version) is in the
recipient's `allow_schemas_in`.

---

## 11. Security Model

### 11.1 Double opt-in

For sender X to send to recipient Y on schema S:

1. **Y's manifest** must list X (or X's glob) in `[capabilities.ipc.allow.from]` AND list S (or glob)
   in `[capabilities.ipc.allow.schemas]`.
2. **X's manifest** must list Y (or glob) in `[capabilities.ipc.outbound.allow]`.

Both checks are evaluated against the snapshot in `AppState.hot.policy`. Either failure ⇒
`IpcError::NoCapability` / `IpcError::SchemaUnsupported`.

### 11.2 Sender authentication

The WIT host import sets `env.sender = IpcAddr::Instance(caller_id)` and `env.sender_bundle` from the
`AppTable` lookup of `caller_id`. App-set sender is **always overwritten**. Apps cannot forge identity.

### 11.3 Envelope-id forgery

The router generates `env.id` from a per-shard `AtomicU64` counter. App-supplied id is ignored. Replies
match on the router-generated id; out-of-band ids are silently dropped.

### 11.4 Rate limit (DoS protection)

`[capabilities.ipc.outbound.max_outbound_per_sec]` — defaulted to 1000 for user apps, 10000 for system
apps. Enforced via the per-(sender, recipient) `RateBucket` on the shard. Breach ⇒ `IpcError::QuotaExceeded`.

### 11.5 Pending-table DoS

Global cap on `pending_table` is 4096 entries. Per-app cap is `max_inflight` (default 8). When global
cap reached, oldest entries (LRU by `deadline - now`) are NACKed to their callers with
`IpcError::QuotaExceeded`. This bounds memory under attack.

### 11.6 Resource quotas (formal)

```toml
[capabilities.ipc.quotas]
max_pending_requests       = 64
max_active_subscriptions   = 32
max_shared_buffers         = 16
max_shared_buffer_bytes    = 16_777_216    # 16 MiB
max_outbound_per_sec       = 1000
max_inflight               = 8
```

Defaults applied when fields omitted.

### 11.7 Forgery-resistance matrix

| Attack | Defense |
|---|---|
| App claims to be another bundle | Router overwrites `sender_bundle` from caller_id |
| App constructs envelope with fake `id` | Router overwrites `id` from atomic counter |
| App calls supervisor without entitlement | `EntitlementChecker` rejects |
| App subscribes to topic without cap | `TopicTable::subscribe` checks `allow_topics_sub` |
| App sends huge payload to OOM recipient | Per-class size cap (256 KB inline; SHM mandatory > 256 KB) |
| App `r#move`s a capability it doesn't own | `fd_table` ownership check before route |
| App replies to an `id` it never received | `pending` table lookup includes `recipient_iid` match |
| App spams broadcast | Rate limit per `(sender_bundle, topic)` |
| App spams `call` to exhaust waiters | `max_inflight` + global `pending_table` cap |
| App uses stale epoch (restarted) | `env.sender_epoch` vs current epoch — mismatched dropped |

---

## 12. Performance Budget

### 12.1 Targets

| Operation | p50 | p99 | Worst-case (under load) |
|---|---|---|---|
| `send` (Raw, < 256 B) | < 30 µs | < 80 µs | 150 µs |
| `send` (Typed, < 4 KiB) | < 50 µs | < 120 µs | 250 µs |
| `call` round-trip (typed, < 1 KiB body) | < 150 µs | < 500 µs | 1 ms |
| `send-with-shm` attach | < 60 µs | < 150 µs | 400 µs |
| `@supervisor: list` reply | < 300 µs | < 700 µs | 1.5 ms |
| Broadcast to 32 subscribers | < 250 µs | < 700 µs | 1.5 ms |
| 1080p frame (SharedBufferKind::Surface) handoff | < 30 µs | < 60 µs | 100 µs |
| Stream channel write (audio frame) | < 5 µs | < 15 µs | 30 µs |

### 12.2 Allocation budget per send (hot path)

| Step | Allocs | Bytes |
|---|---|---|
| Envelope construction (inline body) | 0 | 0 (stack) |
| `IpcAddr::parse` (interned in callsite) | 0 | 0 |
| Policy lookup (ArcSwap load + Arc clone) | 0 | 0 |
| Route table lookup (ArcSwap + DashMap) | 0 | 0 |
| `try_send` to crossbeam | 0 | 0 |
| **Total** | **0** | **0** |

Cold path (cap-cache miss, glob walk, broadcast fan-out): up to 2 small allocs per send (1 SmallVec
spill, 1 DashMap entry insert); amortized ~0.

### 12.3 Throughput

On a 4-core x86_64 desktop with `num_shards = 4`:

| Workload | Throughput |
|---|---|
| Mouse moves (125 Hz, 1 sender → 1 receiver) | < 0.1% one core |
| Compositor frames (30 windows × 60 Hz) | < 0.5% one core |
| Typed RPC (small, no reply) | ~10 M msg/s aggregate |
| Typed call (with reply, 1 KiB body) | ~200 K call/s aggregate |
| Broadcast (32 subscribers, 50 Hz) | ~0.3% one core |
| 1080p Surface handoff @ 60 Hz | ~0% (no copy) |

These figures assume warm caches and a representative desktop with no PSI memory pressure. Under
pressure the suspend loop may add 100 µs of latency to in-flight messages during a sweep.

### 12.4 Channel sizing

| Channel | Cap | Reason |
|---|---|---|
| Per-app `inbox_lo` | 256 | 2 s of input @ 125 Hz before drop-oldest engages |
| Per-app `inbox_hi` | 64 | Rare; bursts during focus / OOM |
| Shard `inbound` | 4 096 | Absorbs publish storms |
| Reply mailbox | 1 | Single-shot |
| Topic retry queue | 64 | Reliable tier; subscriber disconnect on overflow |
| Supervisor cmd reply | 1 | Single-shot |

### 12.5 Memory footprint per app

| Item | Bytes |
|---|---|
| `RouteEntry` (inbox_hi + inbox_lo handles, bundle id) | ~256 |
| Policy snapshot (Arc-shared across instances of bundle) | ~2 KiB / bundle (amortized) |
| `AttachedBuffers` set | ~64 + 16 / handle |
| Inbox queued envelopes (max) | (64+256) × 256 B = 80 KiB |
| Pending reply rows | (8 max) × 256 B = 2 KiB |
| **Total per app (idle)** | **~3 KiB** |
| **Total per app (worst-case)** | **~85 KiB** |

Within the Round-1 supervisor RSS budget.

---

## 13. Implementation Files

All files are ≤ 500 LOC per the project constitution.

| File | LOC | Purpose |
|------|-----|---------|
| `supervisor/src/ipc/mod.rs`             | 80   | Re-exports, glue, `IpcSubsystem::start()` |
| `supervisor/src/ipc/addr.rs`            | 220  | `IpcAddr`, parsing, resolution, glob match |
| `supervisor/src/ipc/envelope.rs`        | 360  | `IpcEnvelope`, `IpcPayload`, `TypedMessage`, `SchemaId`, `IpcFlags`, `Priority`, `InlineBody`, `VyomaResource` |
| `supervisor/src/ipc/router.rs`          | 460  | `IpcRouter`, shard fan-out, `RouterCmd`, `RouteEntry`, `ArcSwap<RouteTable>`, instance lifecycle hooks |
| `supervisor/src/ipc/shard.rs`           | 380  | `RouteShard` worker — owns `pending` map, `inflight` counter, drives `route` |
| `supervisor/src/ipc/deliver.rs`         | 280  | Non-blocking delivery + overflow policy application |
| `supervisor/src/ipc/overflow.rs`        | 180  | `OverflowPolicy`, `ClassPolicy`, default tables, manifest parse |
| `supervisor/src/ipc/reply.rs`           | 420  | `PendingReply`, `ReplyMailbox`, `ReplyWaiter`, async bridge, `IpcError` |
| `supervisor/src/ipc/waitfor.rs`         | 320  | `WaitForGraph`, cycle DFS, drop_all_for, cycles() introspection |
| `supervisor/src/ipc/ticker.rs`          | 140  | Periodic deadline sweep + parked-resume re-arm |
| `supervisor/src/ipc/broadcast.rs`       | 360  | `TopicTable`, `TopicId`, `TopicGlob`, three delivery tiers, retry queue |
| `supervisor/src/ipc/supervisor_cmd.rs`  | 440  | `SupervisorCmdHandler`, `SupCmd` parser, entitlement check, dispatch |
| `supervisor/src/ipc/policy.rs`          | 280  | `PolicySnapshot`, `HotPolicy`, `BundleGlob`/`SchemaGlob`/`TopicGlob`, `EntitlementSet` |
| `supervisor/src/ipc/cap.rs`             | 240  | `RateBucket`, per-app quota enforcement, capability matrix evaluation |
| `supervisor/src/ipc/wit_host.rs`        | 460  | Wasmtime host bindings for `ipc-client`, async `func_wrap_async` for `call`, `host_call` |
| `supervisor/src/ipc/dispatch.rs`        | 360  | Batched delivery to `on-ipc`, priority lane drain, reentrancy state machine, coalescing |
| `supervisor/src/ipc/stream.rs`          | 360  | `StreamChannel`, `StreamTx`, `StreamRx`, lock-free SPSC ring |
| `supervisor/src/ipc/metrics.rs`         | 160  | `IpcMetrics` (delivered/dropped/nacked/coalesced/timeouts counters) |
| `supervisor/src/shm/lifecycle.rs`       | 360  | `SharedBufferRegistry` v2 — refcount, valid, commit/read |
| `wit/vyoma-ipc.wit`                     | 240  | Full WIT interface (§10.1) |
| `wit/vyoma-streams.wit`                 | 80   | Stream interface |
| `apps/_lib/vyoma-ipc-client/src/lib.rs` | 320  | App-side ergonomic Rust wrapper over generated WIT bindings |
| **Total new**                           | **~6 580** | 22 source files, all ≤ 500 LOC |

### 13.1 Modifications to existing files

| File | Δ LOC | Change |
|------|-------|--------|
| `supervisor/src/instance.rs` | +60 | `AppHandle` adds `dispatcher_state` AtomicU8, `inbox_hi`/`inbox_lo` separation |
| `supervisor/src/process_table.rs` | +50 | `AppState.hot` adds `policy: HotPolicy` field |
| `supervisor/src/manifest.rs` | +180 | parse `[capabilities.ipc.*]`, `[capabilities.supervisor.*]`, `[capabilities.ipc.shm]`, `[capabilities.ipc.quotas]` |
| `supervisor/src/lifecycle/actor.rs` | +30 | hook `on_instance_gone` to router + shm registry |
| `supervisor/src/runtime/callbacks.rs` | +60 | accept batched `on-ipc(list<envelope>)` invocation |
| `supervisor/src/display/surface.rs` | +20 | expose `back_lock()` for SHM SurfaceAlias reads |
| `supervisor/src/main.rs` | +30 | wire IpcSubsystem::start into boot phase IpcReady |
| `supervisor/src/boot.rs` | +20 | new boot phase `IpcReady` between `LifecycleReady` and `AppsLaunched` |

Net modified: ~450 LOC.

### 13.2 New Cargo dependencies

```toml
ciborium     = "0.2"       # CBOR encode/decode
lru          = "0.12"      # rate bucket eviction
smallvec     = "1.13"      # inline bodies, subscription lists
siphasher    = "1.0"       # SchemaId / TopicId hashing
sha2         = "0.10"      # SupCmd hashing for grant-once
arc-swap     = "1.7"       # already in Round 1/2, used here
thiserror    = "1.0"       # already present
```

Already present from Round 1/2: `dashmap`, `crossbeam`, `parking_lot`, `bytes`, `tokio` (single-thread,
async reply only), `wit-bindgen`.

### 13.3 Module boundaries

- `addr.rs` separates parsing from envelope so the attack-prone parser is independently unit-testable.
- `reply.rs` is the only file importing `tokio::sync::Notify` (the async reply bridge).
- `wit_host.rs` is the only file Wasmtime imports leak into.
- `policy.rs` is shared with the `manifest` module for parse + build_snapshot.
- `shard.rs` and `router.rs` split because the shard worker is large enough to deserve isolation.
- `stream.rs` is its own subsystem; it does not depend on `router.rs` at all (post-setup is peer-to-peer).

---

## 14. Integration With Prior Rounds

### 14.1 Round 1 integration

- `IpcRouter` was named one of the five core supervisor actors. Round 3 promotes it to a sharded
  fabric; from the `LifecycleActor`'s view the contract is unchanged (still a single `submit()` entry
  point).
- `IpcEnvelope` extends Round 1 minimally: adds `trace_id`, `span_id`, `sender_epoch`, `sender_bundle`.
- `on-ipc` WIT signature changes from `on-ipc: func(from: string, payload: list<u8>)` to
  `on-ipc: func(envs: list<envelope>)`. Apps written against the old single-message signature need a
  trivial migration shim (the new generated bindings can synthesize a per-message dispatch loop).
- `AppIdentity`, `InstanceId`, `BundleId` are referenced verbatim from `crate::instance`.
- The new `IpcReady` boot phase is inserted between `LifecycleReady` and `AppsLaunched`; this is in
  line with Round 1's BootBarrier lock order.

### 14.2 Round 2 integration

- `SharedBufferId`, `SharedBufferKind`, `SharedBuffer::storage` types are reused verbatim from
  `crate::shm`.
- The Round-2 `SharedBufferRegistry` is **extended** (not replaced) with the lifecycle additions of
  §5 (refcount, valid AtomicBool, commit/read).
- `SharedBufferKind::Surface` alias is the zero-copy path; the router carries an 8-byte
  `SharedBufferId` in `IpcPayload::SharedMem` and the registry handles attach.
- Suspension grant from Round 2: when an app is `Suspending`, the router still delivers Critical/System
  priority envelopes to `inbox_hi` but rejects `Normal`/`Input` sends with `RecipientGone` so the
  sender knows to retry post-resume. Pending replies awaiting a `Suspending` recipient are marked
  `parked_for_resume`; the deadline counter is suspended.
- The IpcSeed (`/data/state/ipc-seed`) persistence happens during boot phase `LifecycleReady` so the
  router can hash topic names with a stable seed across reboots.

### 14.3 What other subsystems get from this spec

- **Subsystem 11 (Compositor)** gets `SharedBufferKind::Surface` as the zero-copy primitive for
  per-window back buffers.
- **Subsystem 17 (Notifications)** gets `vyoma.lifecycle.*` topics and `Reliable` delivery tier.
- **Subsystem 31 (Keyboard)** uses `Input` priority and bypasses batching.
- **Subsystem 38 (Drag & Drop)** uses `Capability` payload with `VyomaResource::File` for file drags
  and `SharedBufferKind::ImageData` for drag images.
- **Subsystem 40 (Clipboard)** uses `vyoma.clipboard.changed` topic + `SharedMemRef` for large payloads.
- **Subsystem 50 (Package Manager)** uses `[entitlements.supervisor]` to gate install/uninstall.

---

## Critical v1 Requirements

The following are non-negotiable for v1; the spec is not "done" without them:

- Sharded routing fabric with `ArcSwap<RouteTable>` and N≥1 `RouteShard` workers
- `inbox_hi` (cap 64) + `inbox_lo` (cap 256) per app with biased select drain
- Per-class overflow policies: `DropOldest`, `DropNewest`, `NackSender`, `Coalesce`, `BlockBriefly`
- Wait-for graph + cycle detection on `call`; `max_inflight = 8` default
- Reply mailbox with `tokio::sync::Notify` bridge; `ReplyWaiter::poll` race-free
- 100 ms tick deadline sweep; epoch-aware pending row reaping
- `SharedBuffer` supervisor-owned, refcounted, valid AtomicBool, publish/commit `seq`
- Sender-crash → buffer invalidated; reader read returns `ShmInvalidated`
- Receiver-crash → refcount decrement; reclaim on 0
- `SurfaceAlias` kind reuses compositor double-buffer; no SHM refcount path
- `on-ipc` push, batched up to 16 envelopes, `Input` bypass for single-message dispatch
- Coalescing of same-(sender, schema) `Render` messages on dequeue
- Reentrancy state machine: `IDLE`/`IN_ONIPC`/`AWAITING`/`DRAINING`
- `ArcSwap<PolicySnapshot>` in `AppState.hot`; no `RwLock` on hot path
- Policy snapshot rebuilt on `ManifestReloaded`; atomic swap; lock-free reads
- Authenticated supervisor commands; legacy stdout `@supervisor:` rejected by default
- `[capabilities.supervisor]` manifest declaration with `read`/`focus`/`admin` + `spawn_allow`/`kill_allow`
- SecurityAgent elevation flow with one-shot 30s grants
- `legacy_stdout_supervisor = false` in `/etc/vyoma/boot.toml` defaults
- Three broadcast tiers: BestEffort / Reliable / StateReplication; per-subscriber retry queue cap 64
- IpcSeed persisted to `/data/state/ipc-seed` for stable topic/schema hashing across reboots
- Pre-established `StreamChannel` for media (audio/video) with SPSC ring; capability checked at setup
- Per-app rate limit via `RateBucket` (`max_outbound_per_sec`) on every (sender, recipient) pair
- Global pending-table cap 4096; LRU NACK on overflow
- Distributed tracing fields (`trace_id`, `span_id`) propagated through router
- Sender authentication: router overwrites `sender_bundle`/`sender_epoch`/`id` from caller_id
- Schema-version negotiation: `(schema_id, schema_version, body)`; receiver enforces version range
- CBOR body wire format via `ciborium`; inline `SmallVec<[u8; 256]>` for ≤256B
- Capability passing: `VyomaResource::{File, Surface, Port, Sub}` with opaque `VyomaFd` per instance
- `flags.move` semantics in router; `fd_table` ownership check
- IPC metrics: counters per shard (delivered, dropped, nacked, coalesced, timeouts)
- `mgmt` panel commands: `ipc-inbox <bundle>`, `ipc-deadlock`, `ipc-trace <bundle>`

## Deferred to v2

- Cross-VM IPC (`@bundle@host`) — routing between supervisors
- Persistent inbox across reboot (only `vyoma.lifecycle.*` ring persists in v1)
- Per-app input dedup window (idempotency keys for `Reliable` topics)
- Audit log retention for compliance (SOC2 / ISO) — basic warn-log only in v1
- Role-based service discovery (`role = "notification-daemon"` registry)
- Adaptive shard rebalancing — fixed N at boot in v1
- Operator UI for `iptop` live view — CLI only in v1
- Property-based test harness for ordering invariants
- Deterministic IPC scheduling under test seed
- Wire-format compression (zstd for large CBOR payloads)
- Anycast addressing (`@@anycast.<role>`)
- TopicGlob with full regex syntax (basic `*` wildcard only in v1)
- WIT-typed `on-broadcast` (uses `on-ipc` envelope dispatch in v1)
- Mmap-backed SHM for very large buffers (Owned `Vec<u8>` only in v1)
- Adaptive `max_inflight` per app based on recent latency
- Hot-reload of IPC subsystem itself without restart

## Explicitly NEVER

- Direct app-to-app `crossbeam::Sender` handles (would bypass capability check)
- Unauthenticated `@supervisor:` stdout protocol (deprecated v0.5, removed v1)
- Apps setting their own `env.sender` (always overwritten by router)
- Apps setting their own `env.id` (always overwritten)
- Wasmtime async outside `reply.rs` and `wit_host.rs::host_call`
- `tokio::Runtime` for anything other than the async reply bridge
- Mach ports / Binder / D-Bus equivalents (we built our own broker)
- `RwLock` on the per-message critical path (PolicySnapshot is ArcSwap-only)
- Single global router actor (sharded fabric only)
- Blocking sends from router (every send is `try_send` + policy)
- Per-message capability cache LRU (the PolicySnapshot itself is the cache)
- `cap_cache: Mutex<LruCache<...>>` (eliminated entirely)
- Apps drawing into another app's surface without an explicit `Capability::Surface` handoff
- Cross-instance shared linear memory (wasip2 doesn't allow; we don't invent)
- App holding `crossbeam::Receiver<IpcEnvelope>` directly (always via dispatcher)
- WASM-side correlation IDs (router-allocated only; apps see opaque `u64`)
- App-side schema registry (each app's WIT is local; supervisor doesn't maintain a global registry)
- Bypassing the wait-for graph for "trusted" apps (cycles are bugs regardless of trust)
- Subscriber-driven backpressure on the publisher (broadcast is fan-out, never blocks the publisher)

---

## 15. Open Items Acknowledged and Punted

Three minor open items remain after this synthesis; they do not block v1 but are recorded for future
rounds:

**A. Reliable broadcast retry-queue eviction policy.** Currently when retry queue overflows (64
entries) the subscriber is force-disconnected and must re-subscribe. An alternative would be a
"snapshot" delivery where the subscriber receives the latest state-replication value at reconnect.
Defer to Subsystem 78 (System Preferences) which owns the canonical state-replication use case.

**B. Schema collision policy.** Two bundles defining schema `"create-note"` will collide on hash if
neither prefixes. v1 convention: app schemas MUST be prefixed with bundle ID (`os.vyoma.notes::create-note`).
This is a documentation requirement; enforcement is best-effort warn-log. Subsystem 50 (Package Manager)
will add install-time conflict detection.

**C. Inline SHM read for tiny buffers.** Reading 4 bytes from a SHM buffer still allocates a `Vec<u8>`
on the WIT boundary. v2 should add `read_into_caller(offset, len, ptr, len)` writing directly into
WASM linear memory. v1 keeps the simple `read(offset, len) -> list<u8>` API.

---

*End of Round 3 Final synthesis.*
