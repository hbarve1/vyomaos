# FINAL Spec: File Coordination & Locking (Round 43)

**Subsystem**: File Coordination & Locking  
**macOS Analogue**: `NSFileCoordinator` / `NSFilePresenter`  
**Depends on**: R04 (VFS), R41 (transactional write tokens, file_access_grants, BookmarkToken), R42 (Spotlight watch events)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Problem Statement

R41's transactional write tokens guarantee single-app write atomicity but do not solve multi-app coordination:

- **Simultaneous writers**: Two apps each get a `WriteToken` for the same file; the second `rename(tmp, real)` silently wins.
- **Read/truncate interleave**: App A reads a 10 MB file while App B commits a 0-byte overwrite — mixed old/new data returned.
- **Cross-app rename**: App A renames `/data/notes.md`; App B holds a `ReadToken` with the old path — path becomes stale silently.
- **Multi-file atomic save**: Two sequential `write_commit` calls with no cross-file rollback on crash between them.

R43 adds a **supervisor-brokered intent/consent protocol**: before mutating a file, an app declares intent; the supervisor notifies all registered presenters, collects acknowledgements, then grants the write token.

---

## 2. Presenter Registration

**Protocol (app → supervisor)**:
```
VYOMA_COORD:present:<path_b64>:<intent>:<presenter_token_hex>
VYOMA_COORD:unpresent:<presenter_token_hex>
```

`intent` = comma-separated flags: `read`, `write`, `monitor`

| Flag | Meaning |
|------|---------|
| `read` | App has file open for reading; wants `will_change` before writes |
| `write` | App has file open for writing |
| `monitor` | Wants `did_change` notifications only; no exclusive hold |

`presenter_token_hex` = 128-bit random, app-generated; supervisor validates it against the registering `app_instance_id`.

```rust
// supervisor/src/coordinator/registry.rs

pub type InstanceId = u64;

#[derive(Debug, Clone)]
pub struct PresenterRecord {
    pub path:             String,       // canonicalized
    pub app_instance_id:  InstanceId,
    pub app_name:         String,
    pub intent:           Vec<PresenterIntent>,
    pub presenter_token:  [u8; 16],
    pub timeout_strikes:  u8,           // incremented on missed acks; ≥3 → downgrade to monitor
    pub registered_at_ns: u64,
}

pub static PRESENTER_REGISTRY: Lazy<Mutex<HashMap<[u8;16], PresenterRecord>>>
    = Lazy::new(|| Mutex::new(HashMap::new()));
```

**New manifest capability**:
```toml
[capabilities]
file_coordination = true   # enables VYOMA_COORD: protocol
```

Apps without `file_coordination = true` use R41 non-coordinated `write_begin` (backwards compatible; no will_change round-trip).

**Authorization check** before `present` is accepted:
1. Path canonicalized; rejected if outside `/data/`.
2. App must have `file_access_grants` entry (R41/R42) or `filesystem = true`.
3. `write` intent requires an explicit write grant (BookmarkToken or PanelGrant mode=Save).
4. Paths under `/data/.vyoma/` always rejected (`CoordError::Forbidden`).

---

## 3. Coordination Protocol

### 3.1 App A Requests Coordinated Access

```
VYOMA_COORD:begin:<coord_id>:<op>:<path_b64>[:<extra_b64>]
```

| `op` | Extra | Meaning |
|------|-------|---------|
| `write` | — | Overwrite/truncate |
| `rename` | new path b64 | Rename/move |
| `delete` | — | Remove file |
| `read` | — | Exclusive read (no concurrent writers) |

### 3.2 Will-Change Notification

Supervisor looks up all `PresenterRecord` entries for the path (both old and new path for `rename`). For each presenter with `read` or `write` intent:

```
VYOMA_COORD:will_change:<path_b64>:<op>:<notify_token_hex>:<deadline_ms>
```

- `notify_token_hex`: per-notification 128-bit supervisor-generated token, echoed back in ack.
- `deadline_ms`: default 2000ms; configurable via `[coordination] ack_timeout_ms` in manifest (max 5000ms).

```rust
// supervisor/src/coordinator/session.rs

pub struct CoordSession {
    pub coord_id:            [u8; 16],
    pub requester_instance:  InstanceId,
    pub op:                  CoordOp,
    pub path:                String,
    pub extra_path:          Option<String>,
    pub pending_acks:        HashSet<[u8; 16]>,
    pub deadline_ns:         u64,           // ack deadline
    pub grant_deadline_ns:   u64,           // token expiry — set at GRANT time (B3 fix)
    pub reentrant_depth:     u32,
    pub state:               CoordState,
}

pub enum CoordState { WaitingAcks, Granted, TimedOut, Aborted }
pub enum CoordOp    { Write, Rename { new_path: String }, Delete, Read }
```

### 3.3 Ack Fast-Path (B1 Fix)

`VYOMA_COORD:ack:<notify_token_hex>` is extracted in `router.rs` **before** any display processing — it is NOT gated on `has_display` and does NOT go through the draw-command queue:

```rust
// supervisor/src/router.rs — top of route_or_print()
if let Some(tok) = line.strip_prefix("VYOMA_COORD:ack:") {
    // Fast-path: forward directly to coord session channel, no display queue
    coordinator::notify::on_ack(tok.trim(), sender_instance_id);
    return;
}
```

`on_ack` sends to a per-session `mpsc::Sender<AckMsg>` — never blocks on display processing. This prevents a slow-rendering presenter from stalling ack delivery (B1 fix).

### 3.4 Grant

Once all `pending_acks` received or timed out:
```
VYOMA_COORD:granted:<coord_id>:<write_token_hex>
```

`write_token_hex` is an R41 `WriteToken` with expiry set **at this moment** (grant time) + 30s — not at coordination-begin time (B3 fix). App A proceeds with `VYOMA_FS:write_begin:<path>:<write_token_hex>`.

Write token extension (B3 fix):
```
VYOMA_COORD:extend_write:<write_token_hex>
```
One-time 30s extension; granted while app instance is alive and token is valid.

### 3.5 Did-Change Notification

After `VYOMA_FS:write_commit` or `write_abort`:
```
VYOMA_COORD:did_change:<path_b64>:<op>
```
Sent to all `monitor` and `read`/`write` presenters of the path.

### 3.6 Unresponsive / Crashed Presenters

- Timeout: presenter's `notify_token` marked timed-out → coordination proceeds. 3 consecutive timeouts → presenter downgraded to `monitor`-only (receives `did_change` but no longer blocks `will_change` round-trips).
- Crash: `purge_instance(id)` removes all presenter records and wakes any `CoordSession` waiting on acks from that instance.

**Re-present hint (B4 Fix)**: before clearing a dead instance's records, supervisor saves the registered paths in a per-`app_name` hint table. On next spawn of the same `app_name`, supervisor sends:
```
VYOMA_COORD:re_present_hint:<path_b64>:<intent>
```
as part of startup messages (alongside `VYOMA_SYSTEM:screen:<w>,<h>`). Well-behaved apps re-issue `VYOMA_COORD:present` immediately on receiving this hint, closing the window where the file is unprotected after restart.

---

## 4. Locking Model — Unified State (B2 + B5 Fix)

`LOCK_TABLE` and `WaitForGraph` merged into a single mutex-protected struct to eliminate the TOCTOU race between lock acquisition and deadlock graph update (B2 fix):

```rust
// supervisor/src/coordinator/lock_table.rs

pub struct CoordLockState {
    pub lock_table:     HashMap<String, PathLockState>,
    pub wait_for_graph: WaitForGraph,
}

pub static COORD_LOCK_STATE: Lazy<Mutex<CoordLockState>>
    = Lazy::new(|| Mutex::new(CoordLockState::default()));

pub struct PathLockState {
    pub holder:  Option<[u8; 16]>,    // coord_id of active writer
    pub waiters: VecDeque<[u8; 16]>,  // queued coord_ids
    pub readers: Vec<[u8; 16]>,       // active shared readers
}
```

**Read/write semantics**:

| Situation | New write request |
|-----------|------------------|
| No holder, no readers | Proceed immediately |
| Readers present | Queue in waiters |
| Write holder present | Queue in waiters |

Max queue depth: 32. Beyond that: `VYOMA_COORD:error:<coord_id>:queue_full`.

**Deadlock detection (B5 Fix)**:

```rust
// supervisor/src/coordinator/deadlock.rs

pub struct WaitForGraph {
    edges: HashMap<InstanceId, HashSet<InstanceId>>,
}

impl WaitForGraph {
    pub fn would_deadlock(&self, from: InstanceId, to_set: &[InstanceId]) -> bool {
        // DFS with depth cap of 16 (B5: conservatively reject deep chains)
        for &to in to_set {
            if self.dfs_reaches_bounded(to, from, 16) { return true; }
        }
        false
    }
}
```

DFS runs on a **snapshot** (cloned HashMap) taken inside the lock, then released before traversal — avoids O(V+E) work inside the critical section (B5 fix). False negatives (approving a request that would create a cycle in the live state) are bounded by the ack deadline + session timeout.

Lock acquisition + deadlock check happen atomically in one `COORD_LOCK_STATE.lock()` critical section:
1. Clone wait-for graph for snapshot.
2. Release lock.
3. Run DFS on snapshot.
4. Re-acquire lock; if would_deadlock → `error:deadlock_detected`.

Wrap DFS in `std::panic::catch_unwind` to prevent mutex poisoning (B5 fix).

---

## 5. Reentrance

Same app instance re-requesting a lock it already holds returns the existing grant immediately (reentrancy counter incremented, no new `will_change` round-trip):

```rust
fn maybe_grant_reentrant(requester: InstanceId, path: &str, state: &CoordLockState) -> Option<[u8;16]> {
    let lock = state.lock_table.get(path)?;
    let holder_id = lock.holder?;
    let sessions = COORD_SESSIONS.lock().unwrap();
    let session = sessions.get(&holder_id)?;
    if session.requester_instance == requester { Some(holder_id) } else { None }
}
```

---

## 6. Atomic Cross-File Operations (Group Coordination)

```
VYOMA_COORD:group_begin:<group_id>:<n_paths>
VYOMA_COORD:group_add:<group_id>:<path_b64>:<op>
...
VYOMA_COORD:group_commit:<group_id>
```

Supervisor acquires locks for all paths in **sorted lexicographic order** (prevents ABBA deadlocks across concurrent group operations), sends `will_change` to all presenters across all paths, waits for all acks, then replies:

```
VYOMA_COORD:group_granted:<group_id>:<write_token_1>:<write_token_2>:...
```

Group write commit:
```
VYOMA_COORD:group_write_commit:<group_id>
```

Supervisor: `fsync` all tmp files, then `rename` all in sequence. Individual renames are filesystem-atomic; cross-file ordering is crash-consistent via pre-fsync. Max group size: 16 paths.

---

## 7. Security

- **Presenter access check**: path canonicalized, must be under `/data/`, must have `file_access_grants` entry or `filesystem = true`. Write intent requires explicit write grant.
- **Requester access check**: same check applied to `VYOMA_COORD:begin` requester — prevents lock-hogging on files the app cannot write.
- **Token ownership**: `ack` calls verify `notify_token` belongs to the presenting `app_instance_id`. Tokens travel only via per-app stdin pipe; cannot be forged by other apps.
- **Protected paths**: `/data/.vyoma/` always rejected.

---

## 8. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Slow-rendering presenter delays ack delivery via shared stdout BufReader | `VYOMA_COORD:ack:` extracted at top of `route_or_print()` before display processing; forwarded via per-session `mpsc::Sender<AckMsg>` — never blocked by draw queue |
| B2: Separate LOCK_TABLE and WaitForGraph mutexes create TOCTOU race for ABBA deadlock | Merged into single `CoordLockState` mutex; lock acquisition + deadlock check happen in one atomic critical section |
| B3: WriteToken expiry starts at coord-begin, leaving little time after slow ack round-trip | Token expiry set at actual grant time (+30s); `VYOMA_COORD:extend_write` one-time 30s extension for large writes |
| B4: Presenter records purged on crash; restarted instance unprotected until it re-registers | `purge_instance` saves registered paths in per-app_name hint table; new instance receives `VYOMA_COORD:re_present_hint` at startup |
| B5: DFS deadlock detection holds global mutex for O(V+E) traversal; panic poisons mutex | DFS runs on snapshot cloned under lock then released; depth capped at 16; wrapped in `catch_unwind` to prevent mutex poisoning |

---

## 9. File Layout

```
supervisor/src/coordinator/
├── mod.rs          (~80 lines: public types, COORD_SESSIONS, handle_coord_command())
├── registry.rs     (~180 lines: PresenterRecord, PRESENTER_REGISTRY, register/unregister,
│                               purge_instance (B4 hint save), check_present_authorization)
├── session.rs      (~200 lines: CoordSession, CoordState/Op, begin_coordination,
│                               grant_session (B3 grant-time expiry), release_session, timeout sweep)
├── lock_table.rs   (~150 lines: CoordLockState (B2 merged), PathLockState, acquire/release,
│                               enqueue/dequeue waiters)
├── deadlock.rs     (~80 lines: WaitForGraph, would_deadlock (B5 snapshot DFS, depth cap,
│                               catch_unwind))
├── group.rs        (~180 lines: CoordGroup, group_begin/add/commit, sorted path acquisition,
│                               group_write_commit)
└── notify.rs       (~120 lines: will_change/did_change delivery, ack fast-path (B1),
                                timeout logic, downgrade-to-monitor after 3 strikes)
```
