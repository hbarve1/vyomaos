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

## 2. Module Tree

```
supervisor/src/cloud_sync/
├── mod.rs          (~50 lines)   global OnceLock + subsystem init + re-exports
├── protocol.rs     (~160 lines)  VYOMA_SYNC: + VYOMA_KV: + VYOMA_CLOUD: parsers
├── kv_store.rs     (~130 lines)  per-app KV, 1 MB cap, is_sync_coordinator guard (B4)
├── creds.rs        (~95 lines)   CredentialStore trait, EncryptedFileCredStore
├── enrollment.rs   (~140 lines)  path enrollment registry, grant checking, sync_watch_paths
└── ipc_bridge.rs   (~85 lines)   notify_path_change lock-safe delivery (B1)

apps/cloud-sync/src/
├── main.rs         (~200 lines)
├── engine.rs       (~300 lines)  sync state machine, delta computation
├── queue.rs        (~190 lines)  priority queue, token bucket, bounded channel (B2)
├── conflict.rs     (~155 lines)  detection, KeepBoth fallback (B5), pending_conflicts.json
├── placeholder.rs  (~80 lines)   eviction stubs, .vyomacloud format
└── plugin_mgmt.rs  (~125 lines)  spawn/kill providers, credential token handoff

apps/cloud-sync-s3/src/main.rs    (~400 lines: S3 v4 signing + multipart)
apps/cloud-sync-webdav/src/main.rs (~350 lines: WebDAV PUT/PROPFIND)
```

**Global init** in `supervisor/src/cloud_sync/mod.rs`:
```rust
// supervisor/src/cloud_sync/mod.rs
use std::sync::{Arc, Mutex, OnceLock};

pub mod protocol;
pub mod kv_store;
pub mod creds;
pub mod enrollment;
pub mod ipc_bridge;

pub use kv_store::KvStore;
pub use enrollment::EnrollmentRegistry;
pub use creds::CredentialStore;

static KV_STORE: OnceLock<Arc<Mutex<KvStore>>> = OnceLock::new();
static ENROLLMENT: OnceLock<Arc<Mutex<EnrollmentRegistry>>> = OnceLock::new();

pub fn init(data_root: &str) {
    KV_STORE.get_or_init(|| {
        Arc::new(Mutex::new(KvStore::load(data_root)))
    });
    ENROLLMENT.get_or_init(|| {
        Arc::new(Mutex::new(EnrollmentRegistry::load(data_root)))
    });
}

pub fn kv_store() -> Arc<Mutex<KvStore>> {
    KV_STORE.get().expect("cloud_sync::init not called").clone()
}

pub fn enrollment() -> Arc<Mutex<EnrollmentRegistry>> {
    ENROLLMENT.get().expect("cloud_sync::init not called").clone()
}
```

---

## 3. SyncProvider Trait

The coordinator WASM app interacts with provider plugins entirely via IPC messages; the trait below is implemented by the provider-side runner in each `cloud-sync-<provider>` binary:

```rust
// apps/cloud-sync/src/engine.rs (trait def shared via crate)

use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub etag:       String,
    pub size:       u64,
    pub modified:   SystemTime,
    pub local_path: String,
}

pub type FileManifest = std::collections::HashMap<String, FileEntry>;

#[derive(Debug, Clone, PartialEq)]
pub enum SyncState {
    Idle,
    Syncing { job_id: u64, path: String, progress_pct: u8 },
    Conflict { path: String, provider: String },
    Error(String),
    Paused { reason: PauseReason },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PauseReason {
    UserRequested,
    MeteredNetwork,
    LowBattery,
    QuotaExceeded,
}

pub trait SyncProvider: Send + 'static {
    /// List all files stored remotely for this provider, returning a FileManifest.
    fn list_remote(&self, prefix: &str) -> Result<FileManifest, SyncError>;

    /// Upload a local file to the remote key. Returns the server-assigned ETag.
    fn upload_file(
        &self,
        local_path: &str,
        remote_key: &str,
        rate_limit_bps: Option<u64>,
    ) -> Result<String, SyncError>;

    /// Download a remote key to a local path. Uses atomic write (.tmp → rename).
    fn download_file(
        &self,
        remote_key: &str,
        local_path: &str,
        rate_limit_bps: Option<u64>,
    ) -> Result<FileEntry, SyncError>;

    /// Delete a remote key. No-op if key does not exist (idempotent).
    fn delete_remote(&self, remote_key: &str) -> Result<(), SyncError>;

    /// Fetch metadata (ETag, size, mtime) for a single remote key without downloading.
    fn get_metadata(&self, remote_key: &str) -> Result<Option<FileEntry>, SyncError>;
}

#[derive(Debug)]
pub enum SyncError {
    Network(String),
    Auth(String),
    NotFound,
    QuotaExceeded,
    Conflict,
    Io(std::io::Error),
}
```

Provider plugins (`cloud-sync-s3`, `cloud-sync-webdav`) implement this trait and expose it via the IPC chunk protocol. The coordinator never calls trait methods directly — instead it dispatches via `@cloud-sync-<provider>: <verb>` messages and awaits replies.

---

## 4. Sync Metadata Storage (B3 Fix)

**No SQLite in the coordinator WASM app.** Instead: flat directory of per-file JSON records using R41 write tokens:

```
/data/.vyoma/cloud/records/<sha256_of_path>.json
/data/.vyoma/cloud/manifests/<provider_id>.json
/data/.vyoma/cloud/conflict_log.jsonl
/data/.vyoma/cloud/kv/<app_name>.kv
/data/.vyoma/cloud/policy.toml
/data/.vyoma/cloud/pending_conflicts.json
```

Each record written via `VYOMA_FS:write_begin` → write → `VYOMA_FS:write_commit` (fsync + atomic rename). This eliminates WAL corruption risk from QEMU hard-kill mid-transaction. Each JSON record:

```json
{
  "path":         "/data/notes/todo.txt",
  "provider_id":  "s3",
  "remote_key":   "notes/todo.txt",
  "local_mtime":  1748563200,
  "local_size":   1024,
  "content_hash": "a3b4c5d6e7f8...",
  "remote_etag":  "\"d41d8cd98f00b204e9800998ecf8427e\"",
  "sync_state":   "synced",
  "conflict_mode":"keep-both",
  "enrolled_by":  "notes-app",
  "last_synced":  1748563201
}
```

`sync_state` ∈ `synced | pending_upload | pending_download | conflict | evicted`

**Atomic write helper** used throughout the coordinator:
```rust
// apps/cloud-sync/src/engine.rs
use std::fs;
use std::path::Path;

pub fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?; // atomic on Linux ext4 (single filesystem)
    Ok(())
}
```

---

## 5. Delta Sync: Local vs. Remote Manifest Comparison

The coordinator runs a delta pass before each sync cycle. It compares the local `FileManifest` (built from `/data/.vyoma/cloud/records/`) against the remote manifest (fetched via `list_remote`). Only changed files are synced.

```rust
// apps/cloud-sync/src/engine.rs

pub struct DeltaResult {
    pub upload:   Vec<String>,   // local paths newer than remote or absent remotely
    pub download: Vec<String>,   // remote keys newer than local or absent locally
    pub conflict: Vec<String>,   // both sides changed since last sync
    pub delete_remote: Vec<String>, // locally deleted but still on remote
}

pub fn compute_delta(
    local:  &FileManifest,
    remote: &FileManifest,
    records: &FileManifest,  // last-synced baseline
) -> DeltaResult {
    let mut result = DeltaResult {
        upload: vec![], download: vec![], conflict: vec![], delete_remote: vec![],
    };

    for (key, local_entry) in local {
        match (remote.get(key), records.get(key)) {
            (None, _) => result.upload.push(key.clone()),
            (Some(rem), Some(base)) => {
                let local_changed  = local_entry.etag != base.etag;
                let remote_changed = rem.etag != base.etag;
                match (local_changed, remote_changed) {
                    (true, false) => result.upload.push(key.clone()),
                    (false, true) => result.download.push(key.clone()),
                    (true, true)  => result.conflict.push(key.clone()),
                    (false, false) => { /* in sync */ }
                }
            }
            (Some(_), None) => result.download.push(key.clone()),
        }
    }

    // Locally deleted entries
    for key in records.keys() {
        if !local.contains_key(key) && remote.contains_key(key) {
            result.delete_remote.push(key.clone());
        }
    }

    result
}
```

---

## 6. Notify Path Change — Lock-Safe Delivery (B1 Fix)

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

## 7. Coordinator ↔ Plugin Backpressure (B2 Fix)

Plugin delivery channels use **bounded `mpsc::sync_channel(8)`** (not unbounded). Coordinator sends file chunks via `@cloud-sync-s3: chunk:<job_id>:<base64>` with an acknowledgement protocol:

- Plugin sends `SYNC_CHUNK_ACK:<job_id>:<seq>` after processing each chunk.
- Coordinator waits for ack before sending next chunk.

This prevents 100 MB files from buffering 1600 messages in memory. Backpressure blocks the coordinator's chunk-sender thread only (not supervisor routing).

```rust
// apps/cloud-sync/src/queue.rs

use std::sync::mpsc;

pub struct PluginChannel {
    pub tx: mpsc::SyncSender<String>,
    pub rx: mpsc::Receiver<String>,
}

impl PluginChannel {
    pub fn new() -> (mpsc::SyncSender<String>, mpsc::Receiver<String>) {
        mpsc::sync_channel(8) // bounded: blocks sender when plugin is slow
    }
}

pub fn send_chunk_and_wait(
    tx: &mpsc::SyncSender<String>,
    ack_rx: &mpsc::Receiver<String>,
    job_id: u64,
    seq: u32,
    chunk: &[u8],
) -> Result<(), SyncError> {
    let encoded = base64_encode(chunk);
    tx.send(format!("chunk:{job_id}:{seq}:{encoded}"))
        .map_err(|_| SyncError::Network("plugin channel closed".into()))?;
    // Block until ACK received
    let ack = ack_rx.recv()
        .map_err(|_| SyncError::Network("ack channel closed".into()))?;
    if ack != format!("SYNC_CHUNK_ACK:{job_id}:{seq}") {
        return Err(SyncError::Network(format!("unexpected ack: {ack}")));
    }
    Ok(())
}
```

---

## 8. VYOMA_CLOUD: IPC Protocol

The `VYOMA_CLOUD:` protocol is the unified interface apps use for all cloud-storage operations. It extends and supersedes `VYOMA_SYNC:` for new apps; `VYOMA_SYNC:` remains for backward compatibility.

**App → Supervisor commands** (written to stdout):

| Verb | Arguments | Effect |
|------|-----------|--------|
| `sync_now` | `<path_b64>:<provider_id>` | Immediately enqueue upload/download for path |
| `pause` | `<requester_id>` | Pause all background sync for this app |
| `resume` | `<requester_id>` | Resume paused sync |
| `get_status` | `<path_b64>` | Query current SyncState for a path |
| `resolve_conflict` | `<path_b64>:<keep-local\|keep-remote\|keep-both>` | Resolve pending conflict |
| `set_kv` | `<key>=<url_encoded_value>` | Set a KV key (stored under app namespace) |
| `get_kv` | `<key>` | Retrieve a KV value |
| `del_kv` | `<key>` | Delete a KV key |
| `list_kv` | _(none)_ | List all keys for this app's namespace |
| `subscribe_kv` | _(none)_ | Receive `VYOMA_CLOUD:kv_changed` push events |
| `enroll` | `<path_b64>:<provider_id>:<conflict_mode>` | Register path for background sync |
| `unenroll` | `<path_b64>:<provider_id>` | Deregister path |
| `cancel` | `<job_id>` | Cancel an in-progress sync job |

**Supervisor → App replies** (delivered to stdin):

| Verb | Arguments | Meaning |
|------|-----------|---------|
| `status_reply` | `<path_b64>:<state>:<iso_timestamp>` | Response to `get_status` |
| `conflict` | `<path_b64>:<provider_id>` | Conflict detected; app must resolve |
| `kv_reply` | `<key>:<url_encoded_value>` | Response to `get_kv` |
| `kv_keys_reply` | `<k1>\|<k2>\|...` | Response to `list_kv` |
| `kv_changed` | `<key>:<url_encoded_value>` | Push notification on remote KV update |
| `sync_complete` | `<path_b64>:<job_id>:<etag>` | Upload or download finished successfully |
| `sync_progress` | `<path_b64>:<job_id>:<pct>` | Progress update during large file sync |
| `error` | `<reason>` | General error response |
| `quota_exceeded` | `<provider_id>:<used_bytes>:<limit_bytes>` | Remote quota exceeded |

**Full protocol lines** (stdout format):
```
VYOMA_CLOUD:sync_now:<path_b64>:<provider_id>
VYOMA_CLOUD:pause:<requester_id>
VYOMA_CLOUD:resume:<requester_id>
VYOMA_CLOUD:get_status:<path_b64>
VYOMA_CLOUD:resolve_conflict:<path_b64>:keep-local
VYOMA_CLOUD:resolve_conflict:<path_b64>:keep-remote
VYOMA_CLOUD:resolve_conflict:<path_b64>:keep-both
VYOMA_CLOUD:set_kv:<key>=<url_encoded_value>
VYOMA_CLOUD:get_kv:<key>
VYOMA_CLOUD:del_kv:<key>
VYOMA_CLOUD:list_kv
VYOMA_CLOUD:subscribe_kv
VYOMA_CLOUD:enroll:<path_b64>:<provider_id>:<conflict_mode>
VYOMA_CLOUD:unenroll:<path_b64>:<provider_id>
VYOMA_CLOUD:cancel:<job_id>
```

---

## 9. App Integration Protocol

**Capability declaration**:
```toml
[capabilities]
filesystem  = true
cloud_sync  = true   # grants VYOMA_CLOUD: + VYOMA_SYNC: protocols
kv_store    = true   # opt-in to per-app KV sync (≤1 MB per app)
```

**Legacy VYOMA_SYNC: protocol lines** still parsed for backward compatibility:
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

**Path enrollment access check**: Supervisor validates `VYOMA_CLOUD:enroll` path against `file_access_grants` — apps cannot enroll paths they don't have grants for.

**Placeholder (evicted) files**: Zero-byte stub at `<path>.vyomacloud` with JSON content:
```json
{"remote_key":"notes/foo.md","provider":"s3","etag":"abc123","size":4096}
```
File manager renders these with a cloud icon. User open → app sends `VYOMA_CLOUD:sync_now`.

---

## 10. Conflict Resolution

Default strategy is **last-write-wins** based on the `modified` timestamp in `FileEntry`. When both sides have changed since the last baseline sync record, a conflict is declared. Conflict copies are named:

```
<name> (conflict <timestamp>).<ext>
```

For example: `report (conflict 2026-05-30T14:32:00Z).docx`

The conflict file is written locally and uploaded as a separate remote key. The original path keeps whichever version was written most recently (last-write-wins as tie-breaker after user notification timeout).

```rust
// apps/cloud-sync/src/conflict.rs

use std::time::{SystemTime, UNIX_EPOCH};
use std::path::Path;

pub enum ConflictResolution { KeepLocal, KeepRemote, KeepBoth }

pub fn make_conflict_copy_name(original: &str, timestamp: SystemTime) -> String {
    let secs = timestamp.duration_since(UNIX_EPOCH)
        .unwrap_or_default().as_secs();
    let iso = format_iso_utc(secs); // e.g. "2026-05-30T14:32:00Z"
    let p = Path::new(original);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext  = p.extension().and_then(|s| s.to_str());
    let parent = p.parent().and_then(|p| p.to_str()).unwrap_or("");
    match ext {
        Some(e) => format!("{parent}/{stem} (conflict {iso}).{e}"),
        None    => format!("{parent}/{stem} (conflict {iso})"),
    }
}

pub fn resolve(
    resolution: ConflictResolution,
    local_path: &str,
    remote_path: &str,
    timestamp: SystemTime,
) -> std::io::Result<()> {
    match resolution {
        ConflictResolution::KeepLocal  => { /* upload local, discard remote */ Ok(()) }
        ConflictResolution::KeepRemote => {
            std::fs::copy(remote_path, local_path)?;
            Ok(())
        }
        ConflictResolution::KeepBoth   => {
            let conflict_name = make_conflict_copy_name(local_path, timestamp);
            std::fs::copy(local_path, &conflict_name)?;
            std::fs::copy(remote_path, local_path)?;
            Ok(())
        }
    }
}
```

---

## 11. Key-Value Store (B4 Fix)

KV is supervisor-owned (not coordinator-owned) to prevent forgery. `VYOMA_CLOUD:set_kv` / `VYOMA_KV:` commands are handled entirely in `supervisor/src/cloud_sync/kv_store.rs`. The coordinator sends `VYOMA_KV:push_changed` to the supervisor which the supervisor verifies and delivers:

```rust
// supervisor/src/cloud_sync/kv_store.rs

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const KV_MAX_BYTES: usize = 1024 * 1024; // 1 MB per app

pub struct KvStore {
    data_root: PathBuf,
}

impl KvStore {
    pub fn load(data_root: &str) -> Self {
        let path = PathBuf::from(data_root).join(".vyoma/cloud/kv");
        fs::create_dir_all(&path).ok();
        KvStore { data_root: PathBuf::from(data_root) }
    }

    pub fn kv_path(&self, app_name: &str) -> PathBuf {
        self.data_root.join(format!(".vyoma/cloud/kv/{app_name}.kv"))
    }

    pub fn get(&self, app_name: &str, key: &str) -> Option<String> {
        let content = fs::read_to_string(self.kv_path(app_name)).ok()?;
        for line in content.lines() {
            if let Some((k, v)) = line.split_once('=') {
                if k == key { return Some(url_decode(v)); }
            }
        }
        None
    }

    pub fn set(&self, app_name: &str, key: &str, value: &str) -> Result<(), KvError> {
        let path = self.kv_path(app_name);
        let content = fs::read_to_string(&path).unwrap_or_default();
        let mut map: HashMap<String, String> = content.lines()
            .filter_map(|l| l.split_once('=').map(|(k,v)| (k.to_string(), v.to_string())))
            .collect();
        map.insert(key.to_string(), url_encode(value));
        let new_content: String = map.iter()
            .map(|(k, v)| format!("{k}={v}\n")).collect();
        if new_content.len() > KV_MAX_BYTES {
            return Err(KvError::QuotaExceeded);
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &new_content)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn del(&self, app_name: &str, key: &str) -> std::io::Result<()> {
        let path = self.kv_path(app_name);
        let content = fs::read_to_string(&path).unwrap_or_default();
        let new_content: String = content.lines()
            .filter(|l| !l.starts_with(&format!("{key}=")))
            .map(|l| format!("{l}\n")).collect();
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &new_content)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

pub fn handle_kv_command(
    cmd: &str,
    sender: &str,
    app_registry: &AppRegistry,
    inbox: &Inbox,
    kv: &KvStore,
) {
    // Only app with is_sync_coordinator=true may send push_changed
    let is_coordinator = {
        let reg = app_registry.lock().unwrap();
        reg.get(sender)
            .map(|st| st.lock().unwrap().is_sync_coordinator)
            .unwrap_or(false)
    };
    if cmd.starts_with("push_changed:") && !is_coordinator {
        send_reply(sender, "VYOMA_KV:error:unauthorized", inbox);
        return;
    }
    // dispatch set/get/del/keys as above...
}

#[derive(Debug)]
pub enum KvError {
    QuotaExceeded,
    Io(std::io::Error),
}

impl From<std::io::Error> for KvError {
    fn from(e: std::io::Error) -> Self { KvError::Io(e) }
}
```

`is_sync_coordinator: bool` set at spawn for the app with `cloud_sync_coordinator = true` manifest field, keyed on `app_instance_id` (never app_name).

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

## 12. Bandwidth Throttling

Configurable upload/download rate limits are enforced via a **token bucket** in the coordinator's queue module. The token bucket replenishes at the configured bytes/sec rate and blocks the transfer thread when depleted.

```rust
// apps/cloud-sync/src/queue.rs

pub struct TokenBucket {
    capacity:    u64,   // bytes
    tokens:      u64,
    refill_bps:  u64,   // bytes per second
    last_refill: std::time::Instant,
}

impl TokenBucket {
    pub fn new(rate_bps: u64) -> Self {
        TokenBucket {
            capacity:   rate_bps * 2,  // 2-second burst buffer
            tokens:     rate_bps * 2,
            refill_bps: rate_bps,
            last_refill: std::time::Instant::now(),
        }
    }

    /// Consume `bytes` tokens, blocking until available.
    pub fn consume(&mut self, bytes: u64) {
        loop {
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(self.last_refill).as_secs_f64();
            let refill = (elapsed * self.refill_bps as f64) as u64;
            if refill > 0 {
                self.tokens = (self.tokens + refill).min(self.capacity);
                self.last_refill = now;
            }
            if self.tokens >= bytes {
                self.tokens -= bytes;
                return;
            }
            // Sleep proportionally to avoid busy-spin
            let wait_ms = ((bytes - self.tokens) as f64 / self.refill_bps as f64 * 1000.0) as u64;
            std::thread::sleep(std::time::Duration::from_millis(wait_ms.max(1)));
        }
    }
}
```

Priority queue with separate token buckets per direction:

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

Policy file at `/data/.vyoma/cloud/policy.toml`:
```toml
[bandwidth]
upload_bps     = 512000    # 500 KB/s
download_bps   = 2097152   # 2 MB/s
pause_on_metered = true
pause_on_battery = false

[queue]
user_priority_slots = 4
background_slots    = 2
max_chunk_bytes     = 65536   # 64 KB per IPC chunk
```

Network status: coordinator calls `@supervisor: net-status` → `VYOMA_SYSTEM:net-status:metered|unmetered|offline`. Battery: subscribes to `VYOMA_SYSTEM:battery:<pct>:<charging|discharging>`.

---

## 13. Security

**Credentials**: R60 Keychain when available; interim: `EncryptedFileCredStore` (AES-256-GCM, key from device UUID + random salt) at `/data/.vyoma/cloud/creds.enc`. Credentials delivered to plugins as 128-bit random proxy tokens (in-memory only, never on disk). Plugins use token as bearer credential to cloud — supervisor never exposes raw secrets to WASM.

```rust
// supervisor/src/cloud_sync/creds.rs

pub trait CredentialStore: Send + Sync {
    fn store(&mut self, provider_id: &str, secret: &[u8]) -> Result<(), CredError>;
    fn retrieve(&self, provider_id: &str) -> Result<Vec<u8>, CredError>;
    fn delete(&mut self, provider_id: &str) -> Result<(), CredError>;
    fn issue_proxy_token(&self, provider_id: &str) -> Result<[u8; 16], CredError>;
}

pub struct EncryptedFileCredStore {
    path: std::path::PathBuf,
    key:  [u8; 32],  // AES-256-GCM key derived from device UUID + salt
}
```

**Plugin sandbox**: `network = true` (single declared endpoint only), NO `filesystem` capability. Plugin receives file content as stdin stream; returns `SYNC_RESULT:ok:<etag>` or `SYNC_RESULT:error:<msg>`.

---

## 14. Persistent Sync State Under /data/.vyoma/cloud/

All supervisor-side and coordinator-side state persists under this directory. The supervisor initializes it on first boot:

```
/data/.vyoma/cloud/
├── records/              per-file sync records (one JSON per enrolled path)
│   └── <sha256_of_path>.json
├── manifests/            cached remote manifests per provider
│   └── <provider_id>.json
├── kv/                   per-app KV namespaces
│   └── <app_name>.kv
├── conflict_log.jsonl    append-only log of all conflict events
├── pending_conflicts.json queue of undelivered conflict notifications
├── policy.toml           bandwidth and queue policy
└── creds.enc             encrypted credential store (AES-256-GCM)
```

`conflict_log.jsonl` entries (one JSON object per line, append-only):
```json
{"ts":1748563200,"path":"/data/notes/todo.txt","provider":"s3","resolution":"keep-both","conflict_copy":"/data/notes/todo (conflict 2026-05-30T14:32:00Z).txt","resolved_by":"notes-app"}
```

---

## 15. Pending Conflict Notifications (B5 Fix)

`PromptUser` conflict notification delivered to enrolling app's inbox. If app is not running:
1. Coordinator falls back to `KeepBoth` (no data loss).
2. Conflict queued in `/data/.vyoma/cloud/pending_conflicts.json`.
3. On app next spawn: supervisor delivers queued `VYOMA_CLOUD:conflict:<path>:<provider>` lines at startup alongside `VYOMA_SYSTEM:screen:<w>,<h>`.

```rust
// apps/cloud-sync/src/conflict.rs

use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct PendingConflict {
    pub path:      String,
    pub provider:  String,
    pub queued_at: u64,   // Unix timestamp
    pub app_name:  String,
}

pub fn queue_conflict(
    path: &str,
    provider: &str,
    app_name: &str,
    data_root: &str,
) -> std::io::Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let record = PendingConflict {
        path: path.to_string(),
        provider: provider.to_string(),
        queued_at: SystemTime::now()
            .duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        app_name: app_name.to_string(),
    };
    let file_path = format!("{data_root}/.vyoma/cloud/pending_conflicts.json");
    let mut existing: Vec<PendingConflict> = std::fs::read_to_string(&file_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    existing.push(record);
    let serialized = serde_json::to_vec_pretty(&existing)?;
    atomic_write(std::path::Path::new(&file_path), &serialized)
}
```

---

## 16. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `notify_change` dispatch ABBA deadlock (`AppRegistry` + `inbox`) | **RESOLVED** — Snapshot watcher list under `AppRegistry` lock then drop; deliver to `inbox` separately (§6) |
| B2: Unbounded plugin chunk delivery buffers 100 MB in-memory | **RESOLVED** — Bounded `sync_channel(8)` per plugin; `SYNC_CHUNK_ACK` before each next chunk (§7) |
| B3: Coordinator SQLite WAL corruption on hard-kill | **RESOLVED** — Flat JSON records per file via R41 `write_commit` (fsync + atomic rename); no SQLite in WASM (§4) |
| B4: KV changed notifications forgeable via IPC passthrough | **RESOLVED** — KV handled entirely in supervisor; `push_changed` gated on `is_sync_coordinator` (app_instance_id keyed) (§11) |
| B5: `PromptUser` conflict delivery to dead app silently lost | **RESOLVED** — Fallback to `KeepBoth` when inbox absent; persistent queue in `pending_conflicts.json`; delivered at next spawn (§15) |

---

## 17. File Layout (Complete)

```
supervisor/src/cloud_sync/
├── mod.rs          (~50 lines)   OnceLock<Arc<Mutex<T>>> globals, subsystem init
├── protocol.rs     (~160 lines)  VYOMA_SYNC: + VYOMA_KV: + VYOMA_CLOUD: parsers
├── kv_store.rs     (~130 lines)  per-app KV, 1 MB cap, is_sync_coordinator guard (B4)
├── creds.rs        (~95 lines)   CredentialStore trait, EncryptedFileCredStore
├── enrollment.rs   (~140 lines)  path enrollment registry, grant checking, sync_watch_paths
└── ipc_bridge.rs   (~85 lines)   notify_path_change lock-safe delivery (B1)

apps/cloud-sync/src/
├── main.rs         (~200 lines)  init, event loop, startup conflict delivery
├── engine.rs       (~300 lines)  SyncProvider trait, FileManifest, DeltaResult, atomic_write
├── queue.rs        (~190 lines)  TokenBucket, SyncJob, priority queue, bounded channel (B2)
├── conflict.rs     (~155 lines)  detection, KeepBoth fallback, pending_conflicts.json (B5)
├── placeholder.rs  (~80 lines)   eviction stubs, .vyomacloud format, force_pull trigger
└── plugin_mgmt.rs  (~125 lines)  spawn/kill providers, credential token handoff

apps/cloud-sync-s3/src/main.rs    (~400 lines: S3 v4 signing + multipart upload)
apps/cloud-sync-webdav/src/main.rs (~350 lines: WebDAV PUT/PROPFIND/LOCK)
```

**Size invariant**: every file stays under the 500-line repo limit. The largest file (`apps/cloud-sync-s3/src/main.rs` at ~400 lines) is the S3 provider with full v4 request signing and multipart support.
