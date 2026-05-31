# Round 3 Critic: Inter-Process Communication (IPC)

**Date:** 2026-05-29
**Round:** 3 of 80
**Subsystem:** IPC
**Critic verdict:** FUNDAMENTAL FLAWS

## Executive Summary

The proposed IPC subsystem — a single Rust supervisor process brokering every
message between every `wasm32-wasip2` app through `crossbeam::channel::bounded`
inboxes addressed by `@<bundle>:` — is **architecturally inadequate as a
general-purpose desktop OS IPC layer**. It is a perfectly serviceable design
for the *current* VyomaOS workload (≤16 demo apps, a few hundred messages per
second of shell/ping/pong/draw traffic) and it is consistent with the existing
stdout line protocol. But the spec's own roadmap states the long-term goal is
"a lightweight but fully capable general-purpose OS", and at that target the
Round 1 + Round 2 designs collapse under load, deadlock under realistic
workflows, leak shared memory on crash, and provide no auth model at all.

The five most serious problems are:

1. **Single-router serialization.** Every message in the system passes through
   one Rust actor (or one in-process channel hierarchy). At desktop-class
   workloads — audio, video, drag-and-drop, notifications, accessibility tree
   updates, clipboard, observer notifications — this saturates one CPU core
   long before the WASM apps themselves do.
2. **No deadlock detection on synchronous request-reply.** The spec introduces
   request-reply semantics but does not specify a wait-for graph, a per-app
   request budget, or a fairness scheduler. A three-app cycle (A→B→C→A) will
   permanently wedge three apps and the supervisor's reply-waiter table will
   leak until the deadlines fire — which is *not the same as deadlock
   detection*.
3. **`SharedBuffer` has no ownership model.** Round 2 introduces zero-copy
   shared memory but the spec does not state who owns the buffer, what
   happens on sender crash, what happens on receiver crash, whether the
   buffer is refcounted, how it interacts with WASM linear memory growth,
   or how the supervisor reclaims it. This is a textbook
   use-after-free / memory-leak surface.
4. **The stdout `@supervisor:` protocol has zero authentication and the
   migration path to a typed WIT protocol is undefined.** Any app with
   `stdio = true` can today write `@supervisor: kill shell` and the
   supervisor obeys. There is no per-app capability for
   "may-issue-supervisor-commands". The spec proposes WIT-typed commands
   for the future but does not say how the unauthenticated legacy path is
   deprecated, sandboxed, or shut off.
5. **The "no direct app-to-app channel" invariant is wrong for media-class
   workloads.** 60 FPS 1080p RGBA video is 500 MB/s. 48 kHz stereo audio in
   256-sample chunks is 187 messages/sec per stream and any jitter is
   audible. Forcing both through one Rust mutex-protected actor is a
   non-starter; the design needs an explicit *fast path* (shared ring buffer
   established by the supervisor, then operated peer-to-peer) and the spec
   does not describe one.

Less catastrophic but still important: the capability check is on a cold-shard
`RwLock` (Round 2's split-state assumption), broadcast fan-out has no flow
control, WIT version skew is not addressed, request timeouts cannot be
delivered while the recipient is mid-`on-ipc`, and the inbox-overflow policy
is left to "TBD".

The design needs another iteration before it can be called complete. The
Architect's instincts — broker-mediated routing, typed envelopes, capability
gating — are sound. The execution lacks the contention analysis, lifecycle
analysis, security analysis, and a fast-path carve-out that desktop IPC
requires.

---

## Critical Issues (blocking)

### C1. The supervisor IS the bottleneck — and the math is unforgiving

The Round 1 design centralizes all routing in a single `IpcRouter` actor (or
equivalent module owned by PID 1). Let's price this out at desktop-class
workloads, not the demo workload.

**Steady-state desktop traffic budget (per second):**

| Source                                  | Msgs/sec | Bytes/msg | MB/sec |
|-----------------------------------------|---------:|----------:|-------:|
| Compositor frame callbacks (60 Hz × 30 windows) | 1,800   |    64     |  0.11  |
| Mouse moves (125 Hz) routed to focused window   |   125   |    48     |  0.006 |
| Pointer hover → accessibility tree diffs        |   200   |   512     |  0.10  |
| Clipboard change broadcast                      |     5   | 64 KB     |  0.31  |
| Notification daemon ⇄ apps                      |    50   |   256     |  0.013 |
| Settings observer updates (per pref change)     |   200   |   128     |  0.025 |
| Audio: 4 streams × 48 kHz / 256 samples         |   750   | 1 KB      |  0.73  |
| Video preview at 30 fps × 1280×720 RGBA         |    30   | 3.7 MB    | 110.0  |
| File system watcher events                      |   500   |   128     |  0.06  |
| Drag-and-drop hit tests                         |   500   |    64     |  0.03  |
| Plugin host ⇄ extension RPC                     | 2,000   |   512     |  1.02  |
| **Total (no video)**                            | **~6,130** |          |  **~2.3** |
| **Total (with one video preview)**              | **~6,160** |          |  **~112** |

Even excluding video, **6 K messages/sec of capability-checked, type-checked,
deadline-tracked, addressed-and-routed envelopes** through one actor thread
is in the wrong neighborhood. At desktop loads we want microsecond p99
latency on cheap routes (mouse, kbd, compositor frames). The Round 1 design
gives us:

- one `crossbeam::channel::send` (lock-free for bounded MPSC, but still a
  cache-line write)
- one `RwLock` read on the cold-shard manifest (sub-200 ns *uncontended*; can
  spike to microseconds when the lifecycle thread is mutating)
- one capability-policy match (cheap)
- one `crossbeam::channel::recv_timeout` for reply-waiter setup if it's a
  request
- one `crossbeam::channel::send` to the destination inbox

That's roughly 1–3 µs per message on warm cache, and 10–30 µs when the cold
shard is contended. At 6 K msg/sec the router itself burns ~18 ms/sec of CPU
just doing routing bookkeeping — call it 2 % of one core, which sounds fine,
**but the system has a hard ordering bottleneck**: every other message has
to wait behind the in-flight one. Tail latency is what kills desktops, not
throughput. A 60 FPS scroll feels janky when frame-time stutters above
16 ms even once per second.

**What the spec must specify** (currently missing):
- A sharded router (e.g., per-bundle inbox owned by the receiving app's
  scheduler thread, with the router merely doing addressing lookups). The
  Round 1 design conflates "address resolution" with "queueing".
- A "fast path" for hot routes (compositor frames, input events) that
  bypasses the typed envelope path and uses a pre-resolved, capability-checked
  direct handle.
- A per-app outbound rate limit (and the corresponding behavior under
  backpressure — see C4).
- An explicit `IpcRouter` SLO: p50 latency, p99 latency, max sustained
  throughput on a fixed hardware target. The spec offers neither numbers nor
  a methodology.

**Verdict:** Blocking. The router as currently described cannot meet
desktop SLOs.

### C2. Cascading backpressure can starve unrelated apps

Round 1 specifies per-app **bounded** crossbeam inboxes. The bound is
unspecified. Whatever it is, this scenario is unavoidable:

1. App `slow-typer` has a 1024-slot inbox. It is in a 300 ms WASM
   computation.
2. App `mouse-flood` sends mouse-move events to `slow-typer` at 125 Hz.
3. After 8 seconds the inbox is full.
4. The next `send()` from the router blocks (or returns `Full`, depending
   on policy — the spec does not say).

If the router blocks: every other message stops. A flood at one slow
receiver halts the whole desktop. This is **head-of-line blocking** and it
is the classic failure mode of any centralized broker.

If the router returns `Full`: the message is dropped. What does the sender
see? Does it get a NACK? Does the `send` future resolve with an error? Does
it just disappear? The spec does not say.

The only safe answer is: **the router never blocks**. It must use
`try_send` and have a per-destination overflow policy:

- **Drop oldest** (suitable for liveness events: mouse moves, compositor
  ticks)
- **Drop newest** (suitable for idempotent state updates: "set volume to N")
- **Reject** (return error to sender immediately; suitable for requests
  that the sender can retry)
- **Coalesce** (merge with last unread message of same kind; required for
  high-frequency state replication)

The spec mentions none of these. Until it does, every app is one slow
neighbor away from a partially-frozen desktop.

**Verdict:** Blocking.

### C3. Request-reply has no cycle detection and no fairness scheduler

The Round 1 design introduces request-reply pairs (the standard pattern for
typed RPC). The spec describes the happy path: A sends, B replies, A wakes
up. It does not describe:

**Cycle detection.** A → B → C → A is a textbook IPC deadlock. The
supervisor knows the full wait-for graph (it brokers every request and
every reply). It MUST maintain a directed graph of `awaiter → awaitee` and
detect a cycle on every new request. The spec does not propose this.

**Reentrancy.** Can app A receive a request while it is awaiting a reply
to a request it sent? If yes, that's how cycles form, and we need cycle
detection. If no, that's a partial deadlock (A is blocked, can't service
incoming requests). The spec does not say.

**Reply-waiter table leak.** Each pending request occupies an entry in
the supervisor's reply-waiter table keyed on a correlation ID. With a
100-second timeout (a typical default) and 50 messages/sec of requests, the
table grows to ~5K entries before reaping kicks in. Memory is cheap, but
this is a denial-of-service surface: a malicious app can hold open
1 M pending requests pinned to long timeouts. The spec must specify a
per-app pending-request cap.

**Fairness.** Suppose A spams 10K requests to B at once. B's inbox has
1024 slots. The other 9K requests are…what? Buffered in the router?
Rejected? Backpressured to A's scheduler thread? The spec must specify.

**Reentrancy + cycle detection together.** Even with cycle detection, the
WASM apps are single-threaded (see C5). If `on-ipc` is non-reentrant, then
*any* synchronous request from A to B requires that B be currently idle
(not in `on-ipc`). The supervisor must either:

(a) queue the request and let B see it on its next tick (high latency,
    breaks "request-reply" semantics for fast paths), or
(b) suspend A in WASM and resume it when the reply comes (requires WASM
    continuation primitives or a callback model — the spec describes WIT
    `on-ipc` which is callback-based, so request-reply *must* be expressed
    as send-then-wait-for-future, which is itself a reentrancy concern).

The spec must pick a model and define it precisely.

**Verdict:** Blocking. Request-reply without cycle detection is a
production hazard.

### C4. `SharedBuffer` has no lifetime owner

Round 2 introduces zero-copy `SharedBuffer` handles and a `VYOMA_SHM`
protocol. This is the right idea for the bandwidth problems C1 and C12
describe. But the spec leaves the most important questions unanswered.

**Ownership model — none specified.** Possibilities:

- **Creator-owned.** Sender owns the buffer; receiver gets a read-only
  view. Sender's death deallocates the buffer. Receiver UAF risk.
- **Refcounted.** Buffer lives until last handle drops. Receiver and
  sender both contribute. Death of either decrements. Supervisor maintains
  the refcount. This is safe but requires *every WASM operation on the
  handle* to round-trip through the supervisor for refcount maintenance.
- **Supervisor-owned.** Buffer is allocated by the supervisor on
  behalf of the sender, lent to N receivers, reclaimed when all handles
  drop OR when an explicit `release` is called. Receivers cannot extend
  lifetime past the explicit release. This is the only sane choice.

The spec does not pick one. Therefore **C4 is a UAF / leak surface**.

**Crash semantics — none specified.** When app A crashes:

- All `SharedBuffer`s for which A is the sender: must be invalidated.
  Existing receiver handles must start returning `IpcError::SenderGone`
  on read. **The spec does not describe a handle-validation step**, which
  means in the worst case receivers read from freed memory.
- All `SharedBuffer`s for which A is a receiver: A's handle must be
  decremented from the supervisor's refcount table; this is bookkeeping
  the spec does not describe.
- Pending writes by A that crashed mid-write: the buffer may be in a
  half-written state. Receivers see garbage. **The spec needs a
  "publish/commit" boundary** (e.g., a sequence number written last,
  receivers ignore buffers where the sequence hasn't advanced).

**WASM memory model interaction — none specified.** WASM linear memory
can grow. `SharedBuffer` must be mapped *outside* the WASM instance's
linear memory or it gets mmove'd during `memory.grow`. The spec does not
say where the buffer lives. If it lives in Wasmtime's host-allocated
storage and is exposed via WIT `resource`, fine — but the spec must say so
and must specify the address translation (WASM offset → host pointer) for
both reads and writes, including bounds-checking on every access.

**No capability check on shm operations.** The Round 1 design checks
capability on every IPC envelope. Round 2 does not say whether reads and
writes through `SharedBuffer` are capability-checked. If they aren't, a
buffer received from a trusted source can be read by code that doesn't
have the corresponding capability — capability laundering.

**Verdict:** Blocking. Cannot ship `SharedBuffer` until ownership,
crash, growth, and capability semantics are pinned down.

### C5. WASM single-threading + `on-ipc` non-reentrancy creates a delivery cliff

WASI P2 (and the proposed component model concurrency proposal) describe a
single-threaded execution model per WASM component instance. The WIT
`on-ipc` callback executes on that single thread. While it executes, the
instance is busy; no other entry into the instance is possible.

The Round 1 design says messages "queue in the inbox" while the app is
busy. Fine — but **the inbox is bounded** (C2). And the spec does not
specify:

**What is the inbox depth, and why?** A guess: "1024 messages." Why 1024?
For a UI app receiving mouse moves at 125 Hz, 1024 buys 8 seconds of
unprocessed mouse moves. For a service receiving file-system events,
1024 buys however many file changes you can do in N seconds. The depth
needs to be **per-message-class** (high-rate events get larger queues with
coalescing; low-rate requests get smaller queues with reject-on-full).

**What happens when the inbox is full and the sender is mid-`on-ipc`?**
We have nested overflow:

- Receiver is mid-`on-ipc`, slow.
- Inbox fills.
- Router invokes overflow policy (drop / reject / coalesce).
- Sender gets back an error, raises that error inside its own
  `on-ipc` callback if applicable.
- Sender's `on-ipc` is now doing error handling, and *its* inbox might
  be filling.

The spec needs an explicit answer.

**How does the app pull messages off the inbox?** Two models:

(a) **Push.** Supervisor invokes `on-ipc` for each message. Requires
    the app to be currently NOT in `on-ipc` — otherwise it's a
    reentrancy violation. Supervisor must track "is app in `on-ipc`?"
    state and defer delivery if so. This serializes per-app.
(b) **Pull.** App calls `ipc.recv()` in a loop. App controls pace.
    Inbox depth becomes a backpressure signal the app can see. This is
    more flexible but requires apps to be polling-aware.

The WIT-based `on-ipc` design is push. The spec must therefore say what
"push while busy" means. The likely answer is "the supervisor sets a
'dirty' flag and re-invokes `on-ipc` when the current invocation
returns", which doubles the per-message overhead and makes batching
impossible.

**Verdict:** Blocking. Cannot ship without per-message-class queue
sizing, an explicit overflow contract, and a documented push/pull model.

### C6. Capability check on every message is on the wrong shard

Round 2 splits per-app state into hot/cold shards (per the virtual memory
design or analogous structure). The manifest is presumably "cold" data:
read-once at startup, immutable thereafter, only re-read on `reload`.

The IPC router checks capability on **every** message:

```rust
fn route(envelope: IpcEnvelope) -> Result<()> {
    let app_state = APPS.get(envelope.sender).ok_or(...)?;
    let cold = app_state.cold.read();   // RwLock read
    if !cold.manifest.can_send_to(envelope.target) {
        return Err(IpcError::PermissionDenied);
    }
    drop(cold);
    // ... actual routing ...
}
```

The problem: **the cold-shard `RwLock` is contended by `reload`, by the
package manager, and by every `ps` / introspection request.** Even with
parking_lot's fast path (no syscall on uncontended read), at 6 K msg/sec
the cache line for the lock state is being touched on every message —
multiplied across multiple sender threads (recall apps run on their own
scheduler threads), and the cache line ping-pongs between cores.

**The right design** is to extract capability state into a *third* shard:
"policy snapshot", read-mostly, replaced atomically on `reload` via
`ArcSwap<PolicyTable>`. The router does a single atomic load per message,
no lock. The cost drops from ~200 ns (uncontended `RwLock` read) to ~20 ns
(atomic load + arc clone, possibly avoidable with hazard pointers).

The spec does not describe a policy snapshot. It implicitly puts the
check on the cold shard, which is a 10× cost.

**Verdict:** Blocking for desktop-scale workloads.

### C7. Stdout `@supervisor:` protocol is unauthenticated and the migration is undefined

This is the single most security-critical flaw. The existing protocol is:

```
println!("@supervisor: kill shell");
```

The supervisor reads this from the WASM app's stdout pipe. There is **no
authentication, no capability check, and no per-app gating**. Any app with
`stdio = true` (which is approximately every app) can:

- `@supervisor: kill <any_app>` — DoS any other app
- `@supervisor: list` — enumerate all running apps (information disclosure)
- `@supervisor: focus <any_app>` — hijack focus
- `@supervisor: restart <any_app>` — force-restart any app
- `@supervisor: reload <any_app>` — force a manifest reload, possibly
  exploiting a manifest parser bug

The Round 1 design proposes typed WIT supervisor commands. Excellent.
The Round 1 design does **not** propose:

- A new capability `supervisor_commands = ["list", "focus"]` declaring the
  set of supervisor commands an app is allowed to issue.
- A migration path for the existing stdout protocol. Three options, all
  with consequences:
  - **Disable stdout protocol entirely.** Breaks every existing app
    (shell, calc, etc.). Requires recompiling apps against new WIT
    bindings. Hard but right.
  - **Keep stdout protocol but require the app to declare
    `legacy_supervisor_stdout = true`.** Backward compat, but every
    legacy app is forever insecure.
  - **Parse stdout commands but capability-check them as if they came
    via WIT.** Best compromise: the channel changes but the policy
    becomes the gating mechanism. Requires capability declarations on
    *every* existing app's manifest.

The spec must pick one. The current state is: the spec describes a
beautiful WIT API and silently leaves the unauthenticated stdout protocol
in place. **This is a security regression hidden behind a feature.**

**Verdict:** Blocking. Cannot ship a "capability-secure OS" with this
unaddressed.

---

## Significant Issues (important)

### S1. Broadcast fan-out has no flow control and ambiguous delivery guarantees

The `@broadcast:` mechanism (or its WIT equivalent, presumably a
"subscribe-publish" topic system) requires sending the same envelope to
every subscribed app. At 50 apps:

- 50 inbox `try_send` calls
- 50 possible overflow events
- 50 possible capability checks (each subscriber's `can_receive_topic(T)`)
- The total cost is 50× a single send

**Delivery semantics — undefined.** Is broadcast:

- **At-most-once** (drop on overflow, no retry): bounded cost,
  acceptable for notifications. Spec must say.
- **At-least-once** (retry on overflow): unbounded retries; needs an
  ACK protocol; the supervisor must track per-subscriber delivery
  state, which is a per-message ledger entry. Memory cost grows with
  topic backlog × subscriber count.
- **Exactly-once**: requires deduplication on the receiver side via
  sequence numbers, plus persistent storage of the dedup window.
  Probably overkill for desktop IPC but required for some kinds of
  state replication (e.g., settings observers must converge).

The spec gestures at "at-least-once" without defining the ACK,
deduplication, retry-budget, or persistence semantics. This is one of the
hardest parts of any pub-sub system; the spec treats it as a footnote.

**Fan-out under partial failure — undefined.** If 47 of 50 subscribers
receive the broadcast and 3 have full inboxes:

- Does the broadcast "succeed"? (47/50 delivered)
- Does the broadcast "fail"? (3 of 50 lost)
- Does the publisher get told which 3 failed?
- Does the supervisor queue retries for the 3?

Without an answer, broadcast is non-deterministic from the publisher's
perspective.

**Recommendation:** carve out three broadcast tiers:

- **Best-effort topic** (default): at-most-once, no ACKs, drop on
  overflow, publisher gets no feedback. Suitable for liveness events.
- **Reliable topic**: at-least-once, per-subscriber ACK, supervisor
  retries until ACK or subscriber crashes. Bounded by per-topic backlog
  (max N undelivered messages per subscriber; subscriber missing more
  is force-disconnected from the topic and must re-sync).
- **State-replication topic**: idempotent updates with sequence
  numbers; receiver applies highest-seq-seen; built on top of "reliable"
  with last-write-wins semantics.

### S2. WIT typing has no version negotiation

The spec proposes WIT-typed envelopes. WIT types are compile-time. At
runtime, the supervisor sees raw bytes flowing between two WASM
instances that may have been compiled against *different versions* of the
IPC WIT.

The relevant scenarios:

- App A is built against `ipc@0.1.0`. App B against `ipc@0.2.0`. The
  `0.2.0` version added an optional field to `IpcEnvelope`. Component
  model encoding rules say new optional fields are forward-compatible —
  but only if both ends agree on the wire format version. The spec does
  not describe a wire format version negotiation.

- App A sends a typed message of type `Foo` to App B. App B's bindings
  do not include `Foo` (compiled before `Foo` existed, or against a
  different interface). The supervisor must either:
  - drop the message (Loss of data, silently)
  - return a "type unknown" error to A (requires reply path even for
    fire-and-forget)
  - deliver the raw bytes to B with a `unknown_type` flag, let B handle
    it (requires every app to have an "unknown" fallback handler)

- The supervisor itself parses message contents for capability checks
  (e.g., "this message asks for the user's location"). If supervisor's
  WIT is at v0.3 and the app's is at v0.1, what does the supervisor parse?

**The spec must specify** a wire format that is independent of the
generated WIT bindings — likely a `(type_id: u64, payload: Vec<u8>)`
pair where `type_id` is a stable hash of the WIT type signature, with a
type registry maintained by the supervisor and updated at app load time.

### S3. Timeout enforcement breaks on suspended recipients

If app A sends a request to app B with a 100 ms deadline, and app B is in
`Stopping` state (per the lifecycle FSM), the reply will never arrive. The
timeout machinery should:

1. Detect the elapsed deadline.
2. Cancel the pending reply waiter in the supervisor's table.
3. Deliver an `IpcError::DeadlineExceeded` to A.

The question is **how** step 3 happens. If A's WASM instance is currently
in `on-ipc` (handling some other message), the deadline error gets queued
in A's own inbox and waits for the current `on-ipc` to return. That can
itself blow the budget.

If A's WASM is currently waiting on the reply (the WIT `request` call is
modeled as a suspendable future), the timeout completes the future with
an error and the instance resumes. WIT does not natively support
suspension in P2; it requires custom Wasmtime engine support. The spec
must specify this.

**Worse:** A may have been killed for unrelated reasons between sending
the request and the timeout firing. The supervisor must not deliver the
error to a non-existent A; it should silently reap the waiter entry.

**Even worse:** A may have been killed and restarted (restart_policy =
`always`). The new A is a fresh instance. Delivering A's old request's
timeout to the new A is incorrect — it must be discarded. Discard logic
requires comparing instance epoch numbers, which the spec does not
describe.

### S4. The "fast path" for media is missing

C1 establishes that the centralized router can't handle 60 FPS RGBA frames
or 48 kHz audio. The Round 2 `SharedBuffer` is intended to address this
but does not go far enough. Real solutions:

- **Pre-established channel.** App A asks supervisor for a "stream
  channel" to app B with declared rate/jitter requirements. Supervisor
  allocates a shared ring buffer (lock-free SPSC), returns handles to
  both. From that point on, A writes and B reads with **zero supervisor
  involvement per frame**. Supervisor only sees the setup, teardown, and
  health-check events.

- **Notification mechanism for ring buffer non-emptiness.** Could be a
  futex-like primitive: the supervisor wakes B's WASM thread when the
  ring buffer goes from empty to non-empty. This is one syscall per
  *wakeup*, not one per *frame*.

- **Backpressure via ring buffer fullness.** A's `write` returns the
  amount written; if the ring is full, A handles the partial. No
  supervisor mediation needed.

The spec describes `SharedBuffer` but does not describe:
- The signaling mechanism (how does the receiver know data is ready?
  Polling? Supervisor notification? Futex?)
- The capability scoping of the fast path (does B need an explicit
  `stream_from = ["A"]` declaration? Probably yes.)
- The teardown / crash semantics on the fast path (covered in C4 in
  general but the fast path has extra cases — half-full rings on
  crash, in-flight reads, etc.)

### S5. Inbox queue is FIFO; some messages need priority

A single FIFO inbox is fine for many workloads but not all. Examples
that need priority:

- **Input events** (mouse, keyboard) must preempt background work in a
  UI app. A user clicking the close button should not wait behind 10K
  queued file-system-watcher events.
- **Quit / SIGTERM analogs** must be deliverable even when the inbox
  is "full".
- **Heartbeats** for the watchdog should never be queued behind app
  data.

The spec needs at minimum a two-class priority scheme: "control" (input,
lifecycle, supervisor commands) vs "data" (everything else). Each class
has its own inbox. The control inbox is sized small (≤16) but is checked
first on every `on-ipc` invocation.

### S6. No introspection / debugging surface

For an OS aiming to be "lightweight but fully capable", the IPC subsystem
needs operator visibility:

- `ps --ipc <app>`: show inbox depth, pending requests, recent messages
- `iptop`: top-style live view of message rates per app, per route
- `iptrace <app>`: log all messages in and out of an app
- `ipdump <app>`: dump current inbox contents
- `ipdeadlock`: print the wait-for graph

The spec is silent on operator tooling. Without it, diagnosing C1, C2,
C3, S1 incidents in production is impossible.

### S7. No backpressure signaling to senders

When the router drops a message due to overflow, the sender's only
feedback is the return value of `ipc.send()`. The spec does not specify:

- Is `send()` synchronous or asynchronous (returns a future)?
- If asynchronous: does it `Pending` until inbox space is available
  (blocks the sender)? Or does it `Ready(Err(Full))` immediately?
- Does the supervisor offer a "wait until I can send to X" primitive
  for backpressure-aware senders (a credit-based flow control)?

Without credit-based flow control, well-behaved senders can't rate-limit
themselves. They flood-and-recover, which is worse than steady-state
flow control.

### S8. Multicast vs broadcast distinction missing

The spec talks about `@broadcast:` to all apps. Most messaging systems
also need:

- **Multicast** (publish to a topic, multiple subscribers chosen by
  subscription, not by name)
- **Anycast** (any one available instance of a role — e.g., "any
  notification daemon")

A topic-based pub/sub layered on top of unicast IPC is the standard
answer. The spec doesn't have one.

### S9. Reply correlation IDs are not specified

For request-reply, the supervisor maintains a correlation ID → waiter
map. The spec does not specify:

- Are correlation IDs unique per-sender, per-system, or per-(sender,
  receiver)? Affects table size and collision risk.
- Are they sequential or random? Sequential leaks information about
  message ordering across processes; random requires PRNG state.
- What's the type? `u64`? `u128`? `Uuid`? Size affects the per-message
  overhead.

### S10. No quota system for IPC resources

Apps can hold:
- N pending outbound requests
- M pending reply waiters
- K active subscriptions
- L `SharedBuffer` handles, each with size S

Without per-app quotas, a malicious or buggy app can exhaust any of
these. The spec must declare per-app caps with sensible defaults and
overridable via manifest:

```toml
[capabilities.ipc]
max_pending_requests = 64
max_active_subscriptions = 32
max_shared_buffers = 16
max_shared_buffer_bytes = 16_777_216  # 16 MiB
```

---

## Design Gaps

The following are not "blocking" because the design as proposed doesn't
*actively break* — they are areas where the spec is silent and a
production OS needs an answer.

### G1. Persistence

Does any IPC message survive a supervisor crash? A reboot? An app
restart? The spec is silent. For at-least-once broadcast (S1), the
answer probably needs to be "yes for some classes" and the spec needs
to specify which.

### G2. Cross-VM IPC

If VyomaOS ever spans VMs (e.g., a fleet of microservices each in its
own VM, addressed via `@bundle@host`), the broker design must extend.
Even within a single VM, if the supervisor ever splits across multiple
processes (e.g., a kernel half and a UI half), the IPC must support
inter-supervisor routing. The spec doesn't acknowledge this future.

### G3. Auditing

Compliance (SOC2, ISO, internal) requires audit trails. Every
capability decision (allowed / denied) should be logged with sender,
receiver, topic, decision, reason. The spec does not require this.

### G4. Tracing / observability

Modern distributed tracing (OpenTelemetry) is built on propagated
trace IDs in every message. The IPC envelope should reserve a field for
this. The spec does not.

### G5. Memory accounting

Inbox messages live in supervisor memory until delivered. They count
against the supervisor's RAM budget, not the sender's or receiver's.
A misbehaving sender can OOM the supervisor by flooding messages that
sit in a slow receiver's inbox. The spec must charge inbox occupancy
against either the sender or the receiver and enforce per-app limits.

### G6. Test infrastructure

For an OS, IPC bugs are some of the hardest to reproduce. The spec
should require:

- Deterministic IPC scheduling under a test seed
- Message-ordering chaos testing (random reorder within causal
  ordering)
- Backpressure simulation
- Crash injection (kill app mid-`on-ipc`)
- Wall-clock simulation for timeout testing

None of this appears in the spec.

### G7. WIT interface evolution policy

How does a new WIT type get added to the IPC interface? Who reviews?
What's the deprecation policy? What's the "remove this in 12 months"
list? Without a policy, the IPC surface grows monotonically and
becomes unmaintainable.

### G8. Migration story for the existing `@<app>:` stdout protocol

Beyond the security issue (C7), there's a feature parity issue. The
existing protocol supports:

- `@<app>: <message>` (data)
- `@supervisor: <command>` (control)
- `@broadcast: <message>` (broadcast)
- `@logger: <line>` (a writes to b's log? Or is this just convention?)

The new WIT protocol must cover all of these. The spec needs a
migration table:

| Old stdout form | New WIT form | Behavior change |
|---|---|---|
| `@app: data` | `ipc.send(target="app", payload=...)` | ... |
| `@supervisor: cmd` | `supervisor.invoke(cmd)` | + capability check |
| `@broadcast: ...` | `ipc.publish(topic="*", payload=...)` | + topic |
| `@logger: line` | `log.write(line)` | separate API |

None of this appears in the spec.

### G9. Apps don't know the addressing namespace

`@<bundle>:` addressing requires the sender to know the receiver's
bundle ID. How does an app discover that "the notification daemon" is
at `com.vyoma.notify`? Two options:

- **Static**: hard-coded bundle IDs. Brittle, requires every app to
  agree on names. Microsoft's COM model.
- **Dynamic**: a directory service. Apps register roles
  ("notification-daemon"); senders ask supervisor "who plays the
  notification-daemon role?". The supervisor returns a bundle ID.
  Requires a registry, requires role-arbitration policy (who plays
  the role if two apps register?).

The spec is silent. This is a feature gap, not a correctness gap, but
it's a feature gap that determines whether app developers will use the
IPC system at all.

### G10. App death notifications

When app B dies, every app that has an outstanding request to B needs
to be notified. Every app subscribed to topics published by B needs to
know that the producer is gone. The spec does not describe these
notifications.

---

## Points of Strength

To be fair, the design has real merits.

### P1. Broker-mediated routing is correct

For a capability-secure system, brokering every message through a
trusted PID 1 is the right architecture. Direct app-to-app channels
would require each app to enforce its own policy, multiplying the
attack surface. Centralization here is a feature, not a bug — provided
C1 is addressed by sharding the broker rather than abandoning it.

### P2. `@<bundle>:` addressing is sensible

Bundle-ID addressing is human-readable, stable across restarts, and
maps cleanly to manifest declarations. It's better than PID-style
addressing (which churns on restart) or capability-token addressing
(which requires bootstrap).

### P3. Bounded inboxes are the right primitive

`crossbeam::channel::bounded` is the right substrate for inboxes —
fast, lock-free for SPSC, well-tested. The problem is what surrounds
the inbox (policy, sizing, overflow), not the inbox itself.

### P4. WIT-typed envelopes are aspirationally correct

Pushing IPC into the WIT type system gives apps generated bindings,
compile-time checking, and a stable interface surface. The right
direction. The execution gaps (S2: versioning; C5: re-entrancy) are
fixable.

### P5. `SharedBuffer` for zero-copy is necessary

Acknowledging that some IPC needs zero-copy is correct. The Round 2
addition is conceptually right. C4 is about getting the lifecycle
right, not about whether the primitive should exist.

### P6. Per-app inbox isolation

One inbox per app means a slow neighbor can't corrupt a fast neighbor's
queue. This is sound. C2 is about *how to recover* when a neighbor is
slow, not about the choice of isolation.

---

## Synthesis Recommendations

This section is for the Architect to absorb into Round 4.

### R1. Promote the router from "actor" to "sharded routing fabric"

Replace "the IpcRouter actor" with a sharded design:

- **Address resolver** (read-mostly, lock-free, ArcSwap of bundle →
  inbox-handle table). Updated on app spawn/teardown.
- **Per-receiver delivery thread** (each app's scheduler thread also
  drains its inbox; the router only does enqueue).
- **Capability snapshot** (R3 below) consulted by the sender's
  scheduler thread before enqueue — so the router's only work per
  message is the enqueue.

Outcome: the per-message critical path is `(atomic load PolicyTable) →
(policy check) → (atomic load AddressTable) → (try_send)`. No locks.
~50 ns per message on warm cache.

### R2. Add a dedicated "fast path" for streams

Pre-established lock-free SPSC ring buffers between two apps,
allocated by the supervisor at stream creation. After setup, no
supervisor involvement per frame. Capability check is at setup, not at
each frame. Required for audio, video, drag-image previews.

WIT API sketch:

```wit
interface streams {
    record stream-config {
        capacity: u32,
        item-size: u32,
        wakeup: wakeup-mode,  // poll, signal, blocking-read
    }
    resource stream-tx { write: func(data: list<u8>) -> result<u32, stream-error>; }
    resource stream-rx { read: func() -> result<list<u8>, stream-error>; }
    open-stream: func(peer: string, config: stream-config) -> result<tuple<stream-tx, stream-rx>, ipc-error>;
}
```

### R3. Replace the cold-shard capability read with a policy snapshot

Maintain `ArcSwap<PolicyTable>` containing every app's send / receive
/ topic / subscribe / shm permissions. Updated on app spawn, teardown,
reload. Read by the router on every message via a single atomic load
+ Arc clone. No `RwLock`. Cost: ~20 ns / message.

### R4. Specify the overflow policy in WIT and in the manifest

```toml
[capabilities.ipc]
inbox_capacity = 1024
overflow_policy = "drop-oldest"  # | "drop-newest" | "reject" | "coalesce"
priority_classes = ["control", "data"]
```

Default policy: `reject` (sender sees `IpcError::Full`, can decide to
retry, drop, or escalate). Apps with rate-tolerant inputs (mouse,
compositor) can opt into `drop-oldest`.

### R5. Add cycle detection for request-reply

Maintain a wait-for graph in the supervisor: edge from A to B exists
whenever A is awaiting a reply from B. On each new request, walk the
graph: if adding `A → B` would create a cycle, fail the request
immediately with `IpcError::WouldDeadlock`. O(degree) on average,
trivial.

### R6. Define `SharedBuffer` lifecycle

- Supervisor-owned.
- Refcounted via per-handle drop in supervisor's resource table.
- Sender crash invalidates all handles; subsequent reads return
  `IpcError::SenderGone`.
- Receiver crash decrements refcount; supervisor reclaims when
  refcount → 0.
- Publish/commit boundary: writer must call `commit(seq)`; readers
  ignore buffers where `last_committed_seq < read_seq`.
- Capability check: receiver must have `shm_recv = true`; sender
  must have `shm_send = true`; declared in manifest.
- Bounded by manifest: `max_shared_buffer_bytes`.

### R7. Specify supervisor command capability

```toml
[capabilities.supervisor]
allowed_commands = ["list", "focus_self"]
```

Default: empty list. Apps must declare the supervisor commands they
intend to issue. Disable the unauthenticated stdout protocol via a
boot.toml flag: `legacy_stdout_protocol = false` (default).

### R8. WIT version negotiation

Wire format: `(type_id: u64, version: u16, payload: Vec<u8>)`. The
`type_id` is a stable hash of the WIT type's fully-qualified name. The
supervisor's type registry knows the layout. Receiver bindings declare
which type_ids and versions they accept; mismatched messages get
`IpcError::TypeUnknown` returned to the sender (or dropped silently
for fire-and-forget).

### R9. Timeout delivery with epoch numbers

Each app has an `epoch: u64` that increments on every restart. Pending
reply waiters are keyed by `(app_id, epoch, correlation_id)`. Timeouts
fire into the waiter's app only if the epoch matches. Otherwise the
waiter is reaped silently.

For in-flight delivery: model the WIT `request` as a future. The
supervisor wakes the future with `Err(DeadlineExceeded)` at the
deadline. The Wasmtime engine resumes the WASM continuation. This
requires Wasmtime support for WASM suspension (component model
async; in progress upstream). Until it lands, the spec must specify a
fallback: request-reply via callback (`on-reply(correlation_id,
result)`).

### R10. Introspection commands

```
ps --ipc <app>        # inbox depth, pending out, subscriptions, handles
iptop                 # live tops-style view
iptrace <app>         # log all messages in/out
ipdump <app>          # dump inbox contents
ipdeadlock            # print wait-for graph
```

Each requires a corresponding supervisor command and a capability
(`debug = true` in manifest, presumably reserved for dev/admin builds).

### R11. Per-app IPC quotas

```toml
[capabilities.ipc.quotas]
max_pending_requests = 64
max_active_subscriptions = 32
max_shared_buffers = 16
max_inbox_messages_queued = 1024
max_outbound_msg_per_sec = 10000
```

Exceeding quotas: send returns `IpcError::QuotaExceeded`; supervisor
logs and emits a heartbeat warning.

### R12. Tracing field in envelope

Add `trace_id: Option<u128>` and `span_id: Option<u64>` to
`IpcEnvelope`. Propagated by the supervisor. Apps that want to
participate in distributed tracing read these fields.

### R13. Test methodology

Spec must mandate, in test infrastructure:

- Deterministic scheduling under seed
- Message reorder chaos (within causal bounds)
- Backpressure chaos (random inbox shrinkage)
- Crash injection mid-`on-ipc`
- Wall-clock simulation
- Property tests for: no-deadlock, no-leak, ordering preservation

---

## Final verdict and what Round 4 must address

**Verdict:** FUNDAMENTAL FLAWS. The Architect's direction is right —
brokered IPC, typed envelopes, capability gating — but the spec as
described is missing:

1. A sharded router design with a fast path (C1, S4)
2. A non-blocking overflow contract with named policies (C2)
3. Cycle detection for request-reply (C3)
4. `SharedBuffer` lifecycle, crash, capability semantics (C4)
5. A push/pull / re-entrancy model for `on-ipc` (C5)
6. A policy snapshot to avoid cold-shard contention (C6)
7. A capability for supervisor commands and a stdout migration plan (C7)

These are not nits. They are the difference between a desktop IPC layer
that works under load and one that wedges after 30 seconds of real use.
Round 4 must address all seven Critical items and at least S1, S2, S3,
S4, S7 from Significant Issues. The Design Gaps can wait for Round 5
or later.

Until these are in the spec, the IPC subsystem is not buildable.
