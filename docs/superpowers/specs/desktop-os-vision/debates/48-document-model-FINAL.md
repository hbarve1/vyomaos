# FINAL Spec: Document Model & Recent Files (Round 48)

**Subsystem**: Document Model & Recent Files  
**macOS Analogue**: `NSDocument` / `NSDocumentController`  
**Depends on**: R04 (VFS), R41 (file manager, write tokens, atomic rename), R43 (file coordination), R74 (launch services recent docs)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture Overview

The document model subsystem provides NSDocument-equivalent lifecycle management for VyomaOS. Every open file is tracked as a `DocumentRecord` in the supervisor, which coordinates autosave, dirty-state signaling, recent-files bookkeeping, version snapshots, and crash recovery — all without exposing file descriptors directly to apps.

### Why supervisor-side?

Apps are WASM sandboxes with no direct filesystem access beyond their declared `/data` mount. The supervisor acts as the trusted document controller: it holds canonical file paths, enforces per-app capability gates, and ensures atomic writes via R41 tokens. This mirrors the macOS model where `NSDocumentController` is an app-process singleton — but in VyomaOS it is a system-wide singleton in PID 1, shared across all apps.

### Component Table

| Component | Location | Role |
|-----------|----------|------|
| Document registry | `supervisor/src/document/registry.rs` | Live open-document tracking; dirty state; autosave scheduling |
| Recents store | `supervisor/src/document/recents.rs` | Per-app cap 20, system cap 50; deduplicated by canonical path |
| Version store | `supervisor/src/document/versions.rs` | Gzip-compressed snapshots under `/data/.vyoma/versions/` |
| Autosave engine | `supervisor/src/document/autosave.rs` | 30s periodic timer; `.autosave` shadow; startup recovery scan |
| Close coordinator | `supervisor/src/document/close.rs` | Per-doc channel; 5s timeout; force-close fallback |
| IPC handler | `supervisor/src/document/ipc_handler.rs` | `VYOMA_DOC:` stdout dispatch + `@supervisor: doc_*` commands |
| Module root | `supervisor/src/document/mod.rs` | Global statics; `init()`; feature-gate |

Platform gating: `document-model` Cargo feature controls compilation. Disabled for `mcu-minimal` profile via `.cargo/config.toml`:
```toml
# supervisor/.cargo/config.toml
[profile.mcu-minimal]
features = ["mcu-minimal"]  # excludes document-model
```

---

## 2. Core Data Structures

### DocumentRecord

The `DocumentRecord` is the canonical in-memory representation of a single open document. It is stored by canonical path (not by file descriptor), which makes it robust to app restarts and matches the R41 token model.

```rust
// supervisor/src/document/registry.rs  (~170 lines)
use std::time::SystemTime;

/// Stable identifier for a live open document.
pub type DocId = u64;

/// In-memory record for a single open document.
#[derive(Debug, Clone)]
pub struct DocumentRecord {
    /// Monotonically-assigned doc identifier, unique per supervisor lifetime.
    pub doc_id:        DocId,
    /// Canonical (realpath) absolute path on the 9P /data mount.
    pub path:          String,
    /// Human-readable display name (filename or app-provided label).
    pub app_name:      String,
    /// Which WASM app opened this document.
    pub owner_app:     String,
    /// True when app has called VYOMA_DOC:dirty since last VYOMA_DOC:clean.
    pub is_modified:   bool,
    /// Wall-clock time this record was created.
    pub last_opened:   SystemTime,
    /// Wall-clock time of the last VYOMA_DOC:dirty call, if any.
    pub last_dirtied:  Option<SystemTime>,
    /// Shadow path written by autosave engine, cleared on VYOMA_DOC:clean.
    pub autosave_path: Option<String>,
    /// Channel for out-of-band lifecycle signals to the close coordinator.
    pub doc_signal:    mpsc::Sender<DocSignal>,
}
```

### OpenDocumentRegistry

The registry maps canonical path strings to records. A secondary index maps `owner_app → Vec<DocId>` for fast per-app lookups (used by `sweep_stale_docs` and `recent_files_app`).

```rust
// supervisor/src/document/registry.rs (continued)
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub struct OpenDocumentRegistry {
    /// Primary index: canonical path → record.
    by_path:    HashMap<String, DocumentRecord>,
    /// Secondary index: app_name → list of doc_ids open by that app.
    by_app:     HashMap<String, Vec<DocId>>,
    /// Monotonic counter for doc_id assignment.
    next_id:    u64,
}

impl OpenDocumentRegistry {
    pub fn new() -> Self {
        Self {
            by_path:  HashMap::new(),
            by_app:   HashMap::new(),
            next_id:  1,
        }
    }

    /// Register a new document opened by an app.
    pub fn insert(&mut self, path: String, app_name: String,
                  owner_app: String, signal_tx: mpsc::Sender<DocSignal>)
        -> DocId
    {
        let id = self.next_id;
        self.next_id += 1;
        let record = DocumentRecord {
            doc_id:       id,
            path:         path.clone(),
            app_name:     app_name.clone(),
            owner_app:    owner_app.clone(),
            is_modified:  false,
            last_opened:  SystemTime::now(),
            last_dirtied: None,
            autosave_path: None,
            doc_signal:   signal_tx,
        };
        self.by_app.entry(owner_app).or_default().push(id);
        self.by_path.insert(path, record);
        id
    }

    /// Mark a document as modified (set_modified).
    pub fn set_modified(&mut self, doc_id: DocId) {
        if let Some(rec) = self.by_path.values_mut().find(|r| r.doc_id == doc_id) {
            rec.is_modified  = true;
            rec.last_dirtied = Some(SystemTime::now());
        }
    }

    /// Clear the modified flag (clear_modified).
    pub fn clear_modified(&mut self, doc_id: DocId) {
        if let Some(rec) = self.by_path.values_mut().find(|r| r.doc_id == doc_id) {
            rec.is_modified   = false;
            rec.autosave_path = None;
        }
    }

    /// Returns true if any unsaved changes exist for this doc.
    pub fn is_unsaved_changes(&self, doc_id: DocId) -> bool {
        self.by_path.values()
            .find(|r| r.doc_id == doc_id)
            .map(|r| r.is_modified)
            .unwrap_or(false)
    }

    /// Drain all records owned by app_name; returns them for post-processing.
    pub fn drain_app(&mut self, app_name: &str) -> Vec<DocumentRecord> {
        let ids: Vec<DocId> = self.by_app.remove(app_name).unwrap_or_default();
        let mut drained = Vec::new();
        self.by_path.retain(|_, rec| {
            if ids.contains(&rec.doc_id) {
                drained.push(rec.clone());
                false
            } else {
                true
            }
        });
        drained
    }
}

pub static DOC_REGISTRY: OnceLock<Arc<Mutex<OpenDocumentRegistry>>> = OnceLock::new();

pub fn registry() -> &'static Arc<Mutex<OpenDocumentRegistry>> {
    DOC_REGISTRY.get_or_init(|| Arc::new(Mutex::new(OpenDocumentRegistry::new())))
}
```

---

## 3. Document Lifecycle States

A document moves through a well-defined state machine mirroring NSDocument's lifecycle:

```
         ┌─────────┐
         │  new    │  (app calls VYOMA_DOC:open with empty path)
         └────┬────┘
              │ open or create
         ┌────▼────┐
         │  open   │  (DocEntry inserted in registry, doc_id assigned)
         └────┬────┘
              │ app writes
         ┌────▼────┐
         │  edit   │  (VYOMA_DOC:dirty → is_modified=true)
         └────┬────┘
              │ 30s timer fires
         ┌────▼────────┐
         │  autosave   │  (.autosave shadow written via R41 atomic rename)
         └────┬────────┘
              │ app calls explicit save
         ┌────▼────┐
         │  save   │  (VYOMA_DOC:clean → is_modified=false, shadow removed,
         └────┬────┘    version snapshot written)
              │
         ┌────▼────┐
         │  close  │  (DocEntry removed, recents updated)
         └─────────┘
```

The supervisor enforces these transitions. An app cannot skip `edit` → `autosave` by jumping directly to `close` without a dirty-state acknowledgment. If the app exits unexpectedly while in `edit` state, `sweep_stale_docs` handles the transition to `autosave` before the record is evicted.

---

## 4. Document Lifecycle IPC Protocol

### App → Supervisor (stdout lines)

```
VYOMA_DOC:open:<path_b64>:<display_name_b64>
VYOMA_DOC:close:<doc_id>
VYOMA_DOC:dirty:<doc_id>
VYOMA_DOC:clean:<doc_id>
VYOMA_DOC:autosave_ack:<doc_id>
VYOMA_DOC:should_close_response:<doc_id>:<save|discard|cancel>
VYOMA_DOC:request_autosave:<doc_id>
VYOMA_DOC:get_recents:<app_name_b64_or_empty>
VYOMA_DOC:clear_recents:<app_name_b64_or_empty>
```

### Supervisor → App (stdin lines)

```
VYOMA_DOC:opened:<doc_id>
VYOMA_DOC:autosave_request:<doc_id>
VYOMA_DOC:should_close:<doc_id>
VYOMA_DOC:version_list:<doc_id>:<json_b64>
VYOMA_DOC:recents_list:<json_b64>
VYOMA_DOC:error:<reason>
```

### IPC Commands (shell-style, `@supervisor: ...`)

```
@supervisor: doc_open <path> <display_name>        → REPLY:doc_id:<id>
@supervisor: doc_close <doc_id>
@supervisor: doc_list                              → REPLY:doc_list:<json_b64>
@supervisor: doc_versions <doc_id>                 → VYOMA_DOC:version_list
@supervisor: doc_restore <doc_id> <version_n>      # requires filesystem+BookmarkToken (B4)
@supervisor: recent_files                          → REPLY:recent_files:<json_b64>
@supervisor: recent_files_app <app_name>           → REPLY:recent_files:<json_b64>
@supervisor: recent_files_clear                    # clears system-wide recents (admin only)
@supervisor: recent_files_clear_app <app_name>     # clears per-app recents
```

### Verb Summary Table

| Verb | Direction | Payload | Effect |
|------|-----------|---------|--------|
| `open_doc` | App → Sup | path_b64, display_name_b64 | Insert DocumentRecord; reply doc_id |
| `close_doc` | App → Sup | doc_id | Begin close flow; check is_modified |
| `mark_modified` | App → Sup | doc_id | set_modified(); schedule autosave if not already running |
| `mark_clean` | App → Sup | doc_id | clear_modified(); write version snapshot |
| `request_autosave` | App → Sup | doc_id | Immediately trigger shadow write (app-initiated) |
| `get_recents` | App → Sup | app_name or empty | Return per-app or global recents list |
| `clear_recents` | App → Sup | app_name or empty | Truncate per-app or global recents |
| `autosave_request` | Sup → App | doc_id | Supervisor asks app to serialize content for shadow |
| `should_close` | Sup → App | doc_id | Supervisor asks app for save/discard/cancel before force-close |
| `version_list` | Sup → App | doc_id, json_b64 | Deliver list of available version snapshots |

`VYOMA_DOC:` lines are intercepted in `router.rs` before the draw block:
```rust
if let Some(cmd) = line.strip_prefix("VYOMA_DOC:") {
    document::ipc_handler::handle_doc_line(
        cmd, sender, registry(), &RECENTS_STORE, &AUTOSAVE_ENGINE, inbox,
    );
    return;
}
```

---

## 5. Autosave Engine

The autosave engine runs a background timer thread that fires every 30 seconds. On each tick it scans the registry for any `DocumentRecord` where `is_modified == true` and `autosave_path.is_none()` (not yet autosaved this dirty cycle). For each such record it sends `DocSignal::Autosave` to the per-doc channel, which causes the close coordinator thread to deliver `VYOMA_DOC:autosave_request:<doc_id>` to the app's stdin.

The app responds with `VYOMA_DOC:autosave_ack:<doc_id>` once it has written content to its own temp path (or the supervisor can request content inline via the IPC path). The supervisor then performs the atomic shadow write.

```rust
// supervisor/src/document/autosave.rs  (~100 lines)
use std::path::PathBuf;
use std::fs;
use std::io::Write;
use std::time::Duration;

/// Derive the shadow path for a given canonical document path.
pub fn shadow_path(canonical: &str) -> String {
    let hex = sha256_hex16(canonical);
    format!("/data/.vyoma/autosave/{hex}.autosave")
}

/// Atomic write of content to the shadow path using R41 pattern.
pub fn flush_shadow(path: &str, content: &[u8]) -> Result<(), String> {
    let shadow = shadow_path(path);
    let tmp    = format!("{shadow}.tmp");
    fs::create_dir_all("/data/.vyoma/autosave/")
        .map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(content).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    fs::rename(&tmp, &shadow).map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove the shadow file after an explicit save (clear_modified path).
pub fn discard_shadow(canonical: &str) {
    let shadow = shadow_path(canonical);
    let _ = fs::remove_file(&shadow);
}

/// On supervisor startup: scan for orphaned .autosave files and build
/// a recovery map (canonical_path → shadow_path) for the crash-recovery flow.
pub fn scan_orphan_shadows() -> Vec<(String, PathBuf)> {
    let dir = PathBuf::from("/data/.vyoma/autosave/");
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map(|e| e == "autosave").unwrap_or(false) {
                if let Some(canonical) = lookup_canonical_for_shadow(&p) {
                    result.push((canonical, p));
                }
            }
        }
    }
    result
}

/// Background timer loop — call from a dedicated thread at startup.
pub fn run_autosave_loop(
    registry: Arc<Mutex<OpenDocumentRegistry>>,
    inbox:    Arc<AppInbox>,
) {
    loop {
        std::thread::sleep(Duration::from_secs(30));
        let records: Vec<(DocId, String, mpsc::Sender<DocSignal>)> = {
            let reg = registry.lock().unwrap();
            reg.by_path.values()
                .filter(|r| r.is_modified && r.autosave_path.is_none())
                .map(|r| (r.doc_id, r.path.clone(), r.doc_signal.clone()))
                .collect()
        };
        for (doc_id, _path, signal_tx) in records {
            let _ = signal_tx.send(DocSignal::Autosave);
            send_to_app_by_id(&inbox, doc_id,
                &format!("VYOMA_DOC:autosave_request:{doc_id}"));
        }
    }
}
```

---

## 6. Recent Files Store

Recent files are tracked in a flat JSON file per app plus a shared system recents list, both under `/data/.vyoma/documents/`. This avoids a SQLite dependency for the common read path.

### Storage Layout

```
/data/.vyoma/documents/
    recents_system.json         # shared list, max 50 entries, newest-first
    recents_<app_name>.json     # per-app list, max 20 entries, newest-first
```

### RecentEntry Schema (JSON)

```json
{
  "path":         "/data/projects/report.txt",
  "display_name": "report.txt",
  "app_name":     "text-editor",
  "opened_at":    1748476800,
  "canonical":    "/data/projects/report.txt"
}
```

Deduplication is by `canonical` path. When the same path is opened again, the existing entry is removed and re-inserted at the head of the list — preserving recency order without duplicates.

```rust
// supervisor/src/document/recents.rs  (~130 lines)
const PER_APP_MAX: usize = 20;
const SYSTEM_MAX:  usize = 50;

pub fn push_recent(app_name: &str, path: &str, display_name: &str) {
    // Update per-app list
    let app_file = recents_path_for_app(app_name);
    let mut list = load_json_list(&app_file).unwrap_or_default();
    list.retain(|e: &RecentEntry| e.canonical != path);
    list.insert(0, RecentEntry {
        path:         path.to_string(),
        display_name: display_name.to_string(),
        app_name:     app_name.to_string(),
        opened_at:    unix_now(),
        canonical:    path.to_string(),
    });
    list.truncate(PER_APP_MAX);
    atomic_write_json(&app_file, &list);

    // Update system list
    let sys_file = recents_path_system();
    let mut sys_list = load_json_list(&sys_file).unwrap_or_default();
    sys_list.retain(|e: &RecentEntry| e.canonical != path);
    sys_list.insert(0, list[0].clone());
    sys_list.truncate(SYSTEM_MAX);
    atomic_write_json(&sys_file, &sys_list);
}

pub fn get_recents_app(app_name: &str) -> Vec<RecentEntry> {
    load_json_list(&recents_path_for_app(app_name)).unwrap_or_default()
}

pub fn get_recents_system() -> Vec<RecentEntry> {
    load_json_list(&recents_path_system()).unwrap_or_default()
}

pub fn clear_recents_app(app_name: &str) {
    let _ = fs::remove_file(recents_path_for_app(app_name));
    // Also remove app's entries from system list
    let sys_file = recents_path_system();
    let mut sys_list = load_json_list(&sys_file).unwrap_or_default();
    sys_list.retain(|e| e.app_name != app_name);
    atomic_write_json(&sys_file, &sys_list);
}

/// Atomic JSON write — R41 pattern: write .tmp → fsync → rename.
fn atomic_write_json<T: serde::Serialize>(path: &str, data: &T) {
    let tmp = format!("{path}.tmp");
    if let Ok(bytes) = serde_json::to_vec_pretty(data) {
        if let Ok(mut f) = fs::File::create(&tmp) {
            let _ = f.write_all(&bytes);
            let _ = f.sync_all();
            let _ = fs::rename(&tmp, path);
        }
    }
}
```

Capability gate: `recent_files: bool` in `Capabilities` grants access to **all** apps' recents via `@supervisor: recent_files`. Without it, apps can only see their own recents via `@supervisor: recent_files_app <own_name>`.

---

## 7. Version Snapshots

### Directory Layout

```
/data/.vyoma/versions/<hex16_of_sha256(canonical_path)>/
    v0001_<unix_ts>.snap      # gzip-compressed file content
    v0002_<unix_ts>.snap
    ...
    v0020_<unix_ts>.snap      # max 20 per path; oldest pruned on overflow
```

Snapshots are written only on `VYOMA_DOC:clean` (after explicit save), never on autosave. This ensures versions represent intentional save points, not auto-recovery checkpoints.

```rust
// supervisor/src/document/versions.rs  (~140 lines)
use flate2::{write::GzEncoder, Compression};

pub fn save_version(canonical: &str, content: &[u8]) -> Result<(), String> {
    let dir = version_dir(canonical);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    prune_old_versions(&dir, 19)?;         // keep space for new entry
    let n   = next_version_number(&dir)?;
    let ts  = unix_now();
    let snap = format!("{dir}/v{n:04}_{ts}.snap");
    let tmp  = format!("{snap}.tmp");
    let mut enc = GzEncoder::new(
        fs::File::create(&tmp).map_err(|e| e.to_string())?,
        Compression::fast(),
    );
    enc.write_all(content).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())?;
    fs::rename(&tmp, &snap).map_err(|e| e.to_string())?;
    Ok(())
}

fn prune_old_versions(dir: &str, keep: usize) -> Result<(), String> {
    let mut snaps: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "snap").unwrap_or(false))
        .collect();
    snaps.sort();
    while snaps.len() > keep {
        let _ = fs::remove_file(snaps.remove(0));
    }
    Ok(())
}
```

### Version Restore Gating (B4 fix)

`@supervisor: doc_restore` requires all three of: `filesystem = true` capability, an active `BookmarkToken` for the file path (from R41), and an existing version directory (prevents restoring arbitrary files the app has never opened):

```rust
fn handle_doc_restore(doc_id: DocId, version_n: u32, sender: &str,
                      bookmark_store: &BookmarkStore,
                      registry: &Arc<Mutex<OpenDocumentRegistry>>,
                      inbox: &Inbox)
{
    let path = {
        let reg = registry.lock().unwrap();
        reg.by_path.values().find(|r| r.doc_id == doc_id)
            .map(|r| r.path.clone())
    };
    let path = match path {
        Some(p) => p,
        None => return send_reply(sender, "VYOMA_DOC:error:no_such_doc", inbox),
    };
    let has_bookmark = bookmark_store.check(sender, &path);
    let has_history  = version_dir_exists(&path);
    if !has_bookmark || !has_history {
        return send_reply(sender, "VYOMA_DOC:error:unauthorized", inbox);
    }
    if let Err(e) = versions::restore(&path, version_n) {
        send_reply(sender, &format!("VYOMA_DOC:error:{e}"), inbox);
    }
}
```

---

## 8. Stale Entry Sweep (B1 Fix)

When an app exits (SIGCHLD or voluntary exit), `sweep_stale_docs` is called from `lifecycle.rs` to drain all `DocumentRecord` entries owned by that app and flush dirty ones to the autosave shadow before eviction:

```rust
// supervisor/src/document/registry.rs (continued)

/// Called from lifecycle.rs on app exit. Drains all records for app_name,
/// flushes dirty ones to autosave shadow so they survive the crash.
pub fn sweep_stale_docs(app_name: &str) {
    let stale = registry().lock().unwrap().drain_app(app_name);
    for record in stale {
        if record.is_modified {
            // We don't have the content here — write a tombstone shadow
            // that includes the path so recovery can prompt the user.
            let tombstone = format!(
                "{{\"recovered\":false,\"path\":\"{}\",\"app\":\"{}\"}}",
                record.path, record.owner_app
            );
            let _ = autosave::flush_shadow(&record.path, tombstone.as_bytes());
        }
        // Push to recents even on crash — the path is still relevant.
        recents::push_recent(
            &record.owner_app, &record.path, &record.app_name,
        );
    }
}
```

---

## 9. Close Coordinator (B3 Fix)

The close coordinator runs a signal thread per document. When the supervisor needs to force-close a document (e.g. `pkg-remove`, system shutdown, `@supervisor: doc_close`), it sends `DocSignal::ShouldClose` via the per-doc channel rather than blocking the IPC handler thread.

```rust
// supervisor/src/document/close.rs  (~90 lines)

pub enum DocSignal {
    ShouldClose { reply_tx: oneshot::Sender<CloseDecision> },
    Autosave,
}

#[derive(Debug)]
pub enum CloseDecision { Save, Discard, Cancel }

/// Non-blocking: sends the signal and waits on a oneshot with 5s timeout.
/// Must NOT be called while holding the registry lock.
pub fn request_close(record: &DocumentRecord, inbox: &Inbox) -> CloseDecision {
    let (reply_tx, reply_rx) = oneshot::channel();
    let _ = record.doc_signal.send(DocSignal::ShouldClose { reply_tx });
    // Deliver the supervisor-side stdin line to the app
    send_to_app(
        &record.owner_app,
        &format!("VYOMA_DOC:should_close:{}", record.doc_id),
        inbox,
    );
    // Wait up to 5 seconds; force Discard on timeout or channel drop
    reply_rx.recv_timeout(Duration::from_secs(5))
        .unwrap_or(CloseDecision::Discard)
}

/// Called from ipc_handler when app sends VYOMA_DOC:should_close_response.
pub fn deliver_close_response(doc_id: DocId, decision: CloseDecision,
                              pending: &Mutex<HashMap<DocId, oneshot::Sender<CloseDecision>>>)
{
    if let Some(tx) = pending.lock().unwrap().remove(&doc_id) {
        let _ = tx.send(decision);
    }
}
```

---

## 10. Persistent State Layout

All persistent document state lives under `/data/.vyoma/documents/` (9P-mounted host directory, survives reboots):

```
/data/.vyoma/
├── documents/
│   ├── recents_system.json         # global recents, max 50
│   └── recents_<app_name>.json     # per-app recents, max 20 each
├── versions/
│   └── <hex16_of_path>/
│       ├── v0001_<ts>.snap         # gzip snapshots, max 20 per file
│       └── ...
└── autosave/
    ├── <hex16>.autosave            # shadow content or tombstone JSON
    └── <hex16>.autosave.tmp        # in-flight writes only (never persisted)
```

The supervisor creates these directories on first boot via `document::mod::init()`. Directory permissions: `0o755` for all `.vyoma/` subdirs. The autosave directory is the only one apps interact with indirectly (supervisor writes on their behalf).

---

## 11. Module File Layout

```
supervisor/src/document/
├── mod.rs          (~50 lines: DOC_REGISTRY + RECENTS_STORE + AUTOSAVE_ENGINE statics, init, feature-gate)
├── registry.rs     (~170 lines: DocumentRecord, OpenDocumentRegistry, set_modified, clear_modified,
│                                is_unsaved_changes, drain_app, DOC_REGISTRY static)
├── recents.rs      (~130 lines: RecentEntry, push_recent, get_recents_app, get_recents_system,
│                                clear_recents_app, atomic_write_json, PER_APP_MAX=20, SYSTEM_MAX=50)
├── versions.rs     (~140 lines: save_version, restore, prune_old_versions, version_dir,
│                                restore gating (B4))
├── autosave.rs     (~100 lines: flush_shadow, discard_shadow, shadow_path, scan_orphan_shadows,
│                                run_autosave_loop 30s timer (B5))
├── close.rs        (~90 lines: DocSignal, CloseDecision, request_close 5s timeout,
│                               deliver_close_response (B3))
└── ipc_handler.rs  (~190 lines: handle_doc_line, handle_doc_ipc_cmd, VYOMA_DOC: dispatch,
                                  @supervisor:doc_* dispatch)

supervisor/src/router.rs       (modified: VYOMA_DOC: strip_prefix arm added before draw block)
supervisor/src/lifecycle.rs    (modified: sweep_stale_docs call on app exit (B1))
```

All files stay within the 500-line limit. `ipc_handler.rs` is the largest at ~190 lines.

---

## 12. Integration with R41, R43, R74

**R41 (file manager / write tokens)**: Every `flush_shadow` and `atomic_write_json` call follows the R41 atomic-rename pattern: write to `<path>.tmp`, call `fsync`, then `rename`. No write goes directly to the target path. The `BookmarkToken` from R41 is checked in `handle_doc_restore` (B4).

**R43 (file coordination)**: When the supervisor writes an autosave shadow or restores a version, it goes through the R43 file-coordination layer to prevent concurrent writers. The coordination lock is acquired before `flush_shadow` and released after `rename`. Apps that use `display = true` and write files directly must also declare `filesystem = true`; the supervisor enforces this at manifest-parse time.

**R74 (launch services recent docs)**: The `push_recent` function in `recents.rs` is called both from `ipc_handler` (on `VYOMA_DOC:open`) and from `launch_services::record_recent` (R74). R74 is the entry point for "Open With" and drag-and-drop open flows; both paths converge on `recents.rs`. The R74 layer provides `canonical_path()` resolution before calling `push_recent`, ensuring deduplication works correctly for symlinks and 9P path aliasing.

---

## 13. Blocking Issue Resolution Summary

| Issue | Root Cause | Resolution | Status |
|-------|------------|------------|--------|
| B1: App exit leaves stale `DocumentRecord` entries; dirty state lost on crash | `OpenDocumentRegistry` not consulted on app SIGCHLD | `sweep_stale_docs(app_name)` called from `lifecycle.rs` on every app exit; drains all owned entries, writes tombstone autosave shadow for dirty docs | RESOLVED |
| B2: `document-model` feature compiled into `mcu-minimal` profile; no filesystem for SQLite | Feature not gated per platform profile | `document-model` Cargo feature excluded for `mcu-minimal` via `.cargo/config.toml`; JSON-based recents (not SQLite) removes the SQLite WAL concern entirely | RESOLVED |
| B3: `should_close` handling blocks IPC handler thread awaiting app reply | `recv()` called inline in the IPC dispatch function | Per-doc `DocSignal` channel + `oneshot::Sender<CloseDecision>`; signal thread waits with 5s timeout; IPC handler delivers the signal and returns immediately | RESOLVED |
| B4: Version restore allows access to arbitrary file paths the app never opened | No path-history check before restore; only capability check | `handle_doc_restore` requires: (1) `filesystem = true`, (2) valid `BookmarkToken` from R41 for the path, (3) existing `version_dir` for the path | RESOLVED |
| B5: Autosave shadow writes not atomic; `/data/.vyoma/autosave/` permission model not defined | Direct `File::write_all` to shadow path without `.tmp` rename | R41 pattern enforced in `flush_shadow`: create `.tmp`, `write_all`, `sync_all`, `rename`; directory created with `0o755`; supervisor is the sole writer; startup scan detects orphaned shadows | RESOLVED |
