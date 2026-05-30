# FINAL Spec: Cloud Storage Sync (Round 44)

**Subsystem**: Cloud Storage Sync  
**macOS Analogue**: `iCloud Drive` / `NSUbiquitousKeyValueStore`  
**Depends on**: R41 (transactional write tokens, file_access_grants, VYOMA_FS:watch), R43 (file coordination), R60 (Keychain, placeholder until built)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture Split

| Component | Where | Role |
|-----------|-------|------|
| `cloud-sync` WASM app | Coordinator, `restart = "always"` | Sync engine: queue, policy, conflict resolution, plugin lifecycle |
| `cloud-sync-<provider>` WASM apps | Per-provider plugin | HTTP upload/download to a single declared endpoint; NO filesystem access |
| `supervisor/src/cloud_sync/` | Supervisor | Protocol routing, KV store (B4 fix), path enrollment validation, credential handoff |

The supervisor holds only: sync metadata routing, KV file ownership, enrollment registry, and credential store interface. No HTTP, no provider logic, no sync state enters PID 1.

**Provider plugin registration** via `vyoma.toml`:
```toml
[sync_provider]
id       = "s3"
display  = "Amazon S3"
endpoint = "s3.amazonaws.com"   # only host this plugin may contact
protocol = "s3-v4"
```

Coordinator discovers providers via `@supervisor: apps` filtered by `sync_provider` manifest key, then sends `SYNC_INIT:<provider_id>:<cred_token>` (128-bit random, in-memory-only credential proxy token — see §6).

---

## 2. Sync Metadata Storage (B3 Fix)

**No SQLite in the coordinator WASM app.** Instead: flat directory of per-file JSON records using R41 write tokens:

```
/data/.vyoma/sync/records/<sha256_of_path>.json
```

Each record written via `VYOMA_FS:write_begin` → write → `VYOMA_FS:write_commit` (fsync + atomic rename). This eliminates WAL corruption risk from QEMU hard-kill mid-transaction. Each JSON record:

```json
{
  "path":         "/data/notes/todo.txt",
  "provider_id":  "s3",
  "remote_key":   "notes/todo.txt",
  "local_mtime":  1748563200,
  "local_size":   1024,
  "content_hash": "a3b4...",
  "remote_etag":  "\"d41d8\"",
  "sync_state":   "synced",
  "conflict_mode":"keep-both",
  "enrolled_by":  "notes-app"
}
```

`sync_state` ∈ `synced | pending_upload | pending_download | conflict | evicted`

---

## 3. Notify Path Change — Lock-Safe Delivery (B1 Fix)

The supervisor's `VYOMA_FS:notify_change` dispatch to the coordinator must never hold `AppRegistry` when acquiring `inbox` (ABBA deadlock prevention, same pattern as compositor fix):

```rust
// supervisor/src/cloud_sync/ipc_bridge.rs

pub fn notify_path_change(path: &str, kind: &str, app_registry: &AppRegistry, inbox: &Inbox) {
    // Step 1: snapshot watcher list under AppRegistry lock, then drop
    let watchers: Vec<String> = {
        let reg = app_registry.lock().unwrap();
        reg.iter()
            .filter_map(|(name, st)| {
                let st = st.lock().unwrap();
                if st.sync_watch_paths.iter().any(|p| path.starts_with(p.as_str())) {
                    Some(name.clone())
                } else { None }
            })
            .collect()
    };  // AppRegistry lock dropped here

    // Step 2: deliver — only inbox lock held
    let inb = inbox.lock().unwrap();
    for name in &watchers {
        if let Some(tx) = inb.get(name.as_str()) {
            let _ = tx.send(format!("VYOMA_FS:notify_change:{path}:{kind}"));
        }
    }
}
```

`AppState` gains one field: `pub sync_watch_paths: Vec<String>` (set when coordinator calls `VYOMA_SYNC:enroll`).

---

## 4. Coordinator ↔ Plugin Backpressure (B2 Fix)

Plugin delivery channels use **bounded `mpsc::sync_channel(8)`** (not unbounded). Coordinator sends file chunks via `@cloud-sync-s3: chunk:<job_id>:<base64>` with an acknowledgement protocol:

- Plugin sends `SYNC_CHUNK_ACK:<job_id>:<seq>` after processing each chunk.
- Coordinator waits for ack before sending next chunk.

This prevents 100 MB files from buffering 1600 messages in memory. Backpressure blocks the coordinator's chunk-sender thread only (not supervisor routing).

---

## 5. App Integration Protocol

**Capability declaration**:
```toml
[capabilities]
filesystem  = true
cloud_sync  = true   # grants VYOMA_SYNC: protocol
kv_store    = true   # opt-in to per-app KV sync (≤64 KB)
```

**VYOMA_SYNC: protocol lines** (app stdout → supervisor):
```
VYOMA_SYNC:enroll:<path_b64>:<provider_id>:<conflict_mode>
VYOMA_SYNC:unenroll:<path_b64>:<provider_id>
VYOMA_SYNC:status:<path_b64>
VYOMA_SYNC:pause:<requester_id>
VYOMA_SYNC:resume:<requester_id>
VYOMA_SYNC:force_push:<path_b64>:<provider_id>
VYOMA_SYNC:force_pull:<path_b64>:<provider_id>
VYOMA_SYNC:cancel:<job_id>
```

Replies (supervisor → app stdin):
```
VYOMA_SYNC:status_reply:<path_b64>:<state>:<iso_timestamp>
VYOMA_SYNC:conflict:<path_b64>:<provider_id>
VYOMA_SYNC:error:<reason>
```

Conflict resolution (app → coordinator via IPC):
```
@cloud-sync: resolve:<path_b64>:keep-local
@cloud-sync: resolve:<path_b64>:keep-remote
@cloud-sync: resolve:<path_b64>:keep-both
```

**Path enrollment access check**: Supervisor validates `VYOMA_SYNC:enroll` path against `file_access_grants` — apps cannot enroll paths they don't have grants for.

**Placeholder (evicted) files**: Zero-byte stub at `<path>.vyomacloud` with JSON content:
```json
{"remote_key":"notes/foo.md","provider":"s3","etag":"abc123","size":4096}
```
File manager renders these with a cloud icon. User open → app sends `VYOMA_SYNC:force_pull`.

---

## 6. Key-Value Store (B4 Fix)

KV is supervisor-owned (not coordinator-owned) to prevent forgery. `VYOMA_KV:` commands are handled entirely in `supervisor/src/cloud_sync/kv_store.rs` — the coordinator sends `VYOMA_KV:push_changed` to the supervisor which the supervisor verifies and delivers:

```rust
pub fn handle_kv_command(cmd: &str, sender: &str, app_registry: &AppRegistry, inbox: &Inbox) {
    // Only app with is_sync_coordinator=true (keyed on app_instance_id) may push_changed
    let is_coordinator = app_registry.lock().unwrap()
        .get(sender).map(|st| st.lock().unwrap().is_sync_coordinator).unwrap_or(false);
    if cmd.starts_with("push_changed:") && !is_coordinator {
        send_reply(sender, "VYOMA_KV:error:unauthorized", inbox);
        return;
    }
    // ...
}
```

`is_sync_coordinator: bool` set at spawn for the app with `cloud_sync_coordinator = true` manifest field, keyed on `app_instance_id` (never app_name).

Storage: `/data/.vyoma/kv/<app_name>.kv` — line-oriented `<key>=<url_encoded_value>`, max 64 KB enforced before write.

**Protocol**:
```
VYOMA_KV:set:<key>=<url_encoded_value>
VYOMA_KV:get:<key>
VYOMA_KV:del:<key>
VYOMA_KV:keys
VYOMA_KV:subscribe
```

Replies:
```
VYOMA_KV:get_reply:<key>:<url_encoded_value>
VYOMA_KV:keys_reply:<k1>|<k2>|...
VYOMA_KV:changed:<key>:<url_encoded_value>   # pushed on remote sync update
VYOMA_KV:error:<reason>
```

Conflict model: last-write-wins per key (compare `mtime_of_change`); same content on both sides = no conflict.

---

## 7. Bandwidth & Battery

Priority queue:
```rust
pub enum JobPriority { UserInitiated = 0, AppEnrolled = 1, Background = 2 }

pub struct SyncJob {
    pub job_id:    u64,
    pub path:      String,
    pub direction: Direction,
    pub provider:  String,
    pub priority:  JobPriority,
    pub cancel_tx: Option<mpsc::Sender<()>>,
}
```

Policy file at `/data/.vyoma/sync/policy.toml`:
```toml
[bandwidth]
upload_kbps    = 500
download_kbps  = 2048
pause_on_metered = true
pause_on_battery = false

[queue]
user_priority_slots = 4
background_slots    = 2
```

Network status: coordinator calls `@supervisor: net-status` → `VYOMA_SYSTEM:net-status:metered|unmetered|offline`. Battery: subscribes to `VYOMA_SYSTEM:battery:<pct>:<charging|discharging>`.

---

## 8. Security

**Credentials**: R60 Keychain when available; interim: `EncryptedFileCredStore` (AES-256-GCM, key from device UUID + random salt) at `/data/.vyoma/sync/creds.enc`. Credentials delivered to plugins as 128-bit random proxy tokens (in-memory only, never on disk). Plugins use token as bearer credential to cloud — supervisor never exposes raw secrets to WASM.

**Plugin sandbox**: `network = true` (single declared endpoint only), NO `filesystem` capability. Plugin receives file content as stdin stream; returns `SYNC_RESULT:ok:<etag>` or `SYNC_RESULT:error:<msg>`.

---

## 9. Pending Conflict Notifications (B5 Fix)

`PromptUser` conflict notification delivered to enrolling app's inbox. If app is not running:
1. Coordinator falls back to `KeepBoth` (no data loss).
2. Conflict queued in `/data/.vyoma/sync/pending_conflicts.json`.
3. On app next spawn: supervisor delivers queued `VYOMA_SYNC:conflict:<path>:<provider>` lines at startup alongside `VYOMA_SYSTEM:screen:<w>,<h>`.

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: notify_change dispatch ABBA deadlock (AppRegistry + inbox) | Snapshot watcher list under AppRegistry lock then drop; deliver to inbox separately |
| B2: Unbounded plugin chunk delivery buffers 100 MB in-memory | Bounded `sync_channel(8)` per plugin; `SYNC_CHUNK_ACK` before each next chunk |
| B3: Coordinator SQLite WAL corruption on hard-kill | Flat JSON records per file via R41 write_commit (fsync + atomic rename); no SQLite in WASM |
| B4: KV changed notifications forgeable via IPC passthrough | KV handled entirely in supervisor; `push_changed` gated on `is_sync_coordinator` (app_instance_id keyed) |
| B5: PromptUser conflict delivery to dead app silently lost | Fallback to KeepBoth when inbox absent; persistent queue in `pending_conflicts.json`; delivered at next spawn |

---

## 11. File Layout

```
supervisor/src/cloud_sync/
├── mod.rs          (~40 lines)
├── protocol.rs     (~150 lines: VYOMA_SYNC: + VYOMA_KV: parsers)
├── kv_store.rs     (~120 lines: per-app KV, 64KB cap, is_sync_coordinator check (B4))
├── creds.rs        (~90 lines: CredentialStore trait, EncryptedFileCredStore)
├── enrollment.rs   (~130 lines: path enrollment registry, grant checking, sync_watch_paths)
└── ipc_bridge.rs   (~80 lines: notify_path_change lock-safe delivery (B1))

apps/cloud-sync/src/
├── main.rs         (~200 lines)
├── engine.rs       (~300 lines: sync state machine)
├── queue.rs        (~180 lines: priority queue, token bucket, bounded channel (B2))
├── conflict.rs     (~150 lines: detection, KeepBoth fallback (B5), pending_conflicts.json)
├── placeholder.rs  (~80 lines: eviction stubs)
└── plugin_mgmt.rs  (~120 lines: spawn/kill providers, credential token handoff)

apps/cloud-sync-s3/src/main.rs    (~400 lines: S3 v4 signing + multipart)
apps/cloud-sync-webdav/src/main.rs (~350 lines: WebDAV PUT/PROPFIND)
```
