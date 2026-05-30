# FINAL Spec: Document Model & Recent Files (Round 48)

**Subsystem**: Document Model & Recent Files  
**macOS Analogue**: `NSDocument` / `NSDocumentController`  
**Depends on**: R04 (VFS), R41 (write tokens, atomic rename), R43 (file coordination)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Three supervisor-side components:

| Component | Location | Role |
|-----------|----------|------|
| Document registry | `supervisor/src/document/registry.rs` | Live open-document tracking; dirty state; autosave scheduling |
| Recents store | `supervisor/src/document/recents.rs` | SQLite `recents.db`; per-app cap 10, system cap 100 |
| Version store | `supervisor/src/document/versions.rs` | Gzip-compressed snapshots under `/data/.vyoma/versions/` |

Platform gating (B2 fix): `document-model` Cargo feature controls compilation. Disabled for `mcu-minimal` profile via `.cargo/config.toml` conditional:
```toml
# supervisor/.cargo/config.toml
[profile.mcu-minimal]
features = ["mcu-minimal"]  # excludes document-model
```

---

## 2. Document Lifecycle Protocol

### App → Supervisor (stdout)
```
VYOMA_DOC:open:<path_b64>:<display_name_b64>
VYOMA_DOC:close:<doc_id>
VYOMA_DOC:dirty:<doc_id>
VYOMA_DOC:clean:<doc_id>
VYOMA_DOC:autosave_ack:<doc_id>
VYOMA_DOC:should_close_response:<doc_id>:<save|discard|cancel>
```

### Supervisor → App (stdin)
```
VYOMA_DOC:opened:<doc_id>
VYOMA_DOC:autosave_request:<doc_id>
VYOMA_DOC:should_close:<doc_id>           # supervisor asks app before force-close
VYOMA_DOC:version_list:<doc_id>:<json_b64>
VYOMA_DOC:error:<reason>
```

### IPC Commands (shell-style, `@supervisor: ...`)
```
@supervisor: doc_open <path> <display_name>    → REPLY:doc_id:<id>
@supervisor: doc_close <doc_id>
@supervisor: doc_list                          → REPLY:doc_list:<json_b64>
@supervisor: doc_versions <doc_id>             → VYOMA_DOC:version_list
@supervisor: doc_restore <doc_id> <version_n>  # requires filesystem+BookmarkToken (B4)
@supervisor: recent_files                      → REPLY:recent_files:<json_b64>
@supervisor: recent_files_app <app_name>       → REPLY:recent_files:<json_b64>
```

`VYOMA_DOC:` lines routed in `router.rs` before draw block:
```rust
if let Some(cmd) = line.strip_prefix("VYOMA_DOC:") {
    document::ipc_handler::handle_doc_line(cmd, sender, &DOC_REGISTRY, &RECENTS_DB, inbox);
    return;
}
```

---

## 3. Document Registry

```rust
// supervisor/src/document/registry.rs  (~160 lines)
pub struct DocEntry {
    pub doc_id:      u64,
    pub path:        String,
    pub display_name: String,
    pub owner_app:   String,
    pub is_dirty:    bool,
    pub opened_at:   u64,
    pub last_dirty:  Option<u64>,
    pub doc_signal:  mpsc::Sender<DocSignal>,   // B3 fix
}

pub enum DocSignal {
    ShouldClose { reply_tx: oneshot::Sender<CloseDecision> },
    Autosave,
}

pub static DOC_REGISTRY: OnceLock<Mutex<HashMap<u64, DocEntry>>> = OnceLock::new();
```

**`should_close` via channel (B3 fix)**: When supervisor needs to force-close (e.g. `pkg-remove`, system shutdown), it sends `DocSignal::ShouldClose` via the per-doc channel — NOT by blocking the IPC handler thread. The signal handler thread waits for the `oneshot` reply with a 5-second timeout, then forces close regardless:

```rust
// document/close.rs
pub fn request_close(entry: &DocEntry, inbox: &Inbox) -> CloseDecision {
    let (reply_tx, reply_rx) = oneshot::channel();
    let _ = entry.doc_signal.send(DocSignal::ShouldClose { reply_tx });
    // deliver VYOMA_DOC:should_close to app via inbox (B3: non-blocking)
    send_to_app(&entry.owner_app, &format!("VYOMA_DOC:should_close:{}", entry.doc_id), inbox);
    // wait up to 5s
    reply_rx.recv_timeout(Duration::from_secs(5))
        .unwrap_or(CloseDecision::Discard)
}
```

**Stale entry sweep (B1 fix)**: On app exit, `sweep_stale_docs(app_name, registry)` removes all `DocEntry` records owned by that app and writes any dirty state to the autosave shadow:

```rust
// called from lifecycle.rs on SIGCHLD / app exit
pub fn sweep_stale_docs(app_name: &str, registry: &Mutex<HashMap<u64, DocEntry>>) {
    let stale: Vec<DocEntry> = {
        let mut reg = registry.lock().unwrap();
        reg.drain().filter(|(_, e)| e.owner_app == app_name)
                   .map(|(_, e)| e)
                   .collect()
    };
    for entry in stale {
        if entry.is_dirty {
            autosave::flush_shadow(&entry);  // B5: atomic-rename shadow write
        }
    }
}
```

---

## 4. Recent Files Store

SQLite at `/data/.vyoma/recents.db` (WAL mode; `PRAGMA wal_autocheckpoint=10` for non-desktop profiles — B2 fix):

```sql
CREATE TABLE recents (
    id         INTEGER PRIMARY KEY,
    path       TEXT NOT NULL,
    app_name   TEXT NOT NULL,
    opened_at  INTEGER NOT NULL,
    display_name TEXT
);
CREATE INDEX recents_app ON recents(app_name, opened_at DESC);
CREATE INDEX recents_time ON recents(opened_at DESC);
```

Retention: per-app cap = 10 rows (delete oldest beyond 10). System cap = 100 rows total.

Capability gate: `recent_files: bool` in `Capabilities` grants access to **all** apps' recents via `@supervisor: recent_files`. Without it, apps can only see their own recents via `@supervisor: recent_files_app <own_name>`.

---

## 5. Version Snapshots

Directory layout:
```
/data/.vyoma/versions/<hex16_of_sha256(canonical_path)>/
    v0001_<unix_ts>.snap      # gzip-compressed file content
    v0002_<unix_ts>.snap
    ...
    v0020_<unix_ts>.snap      # max 20 per path; oldest pruned on overflow
```

Snapshot written on every `VYOMA_DOC:clean` event (i.e. after successful save), NOT on autosave:
```rust
pub fn save_version(path: &str, content: &[u8]) -> Result<(), String> {
    let dir = version_dir(path);
    fs::create_dir_all(&dir)?;
    prune_old_versions(&dir, 19)?;      // keep space for new one
    let n    = next_version_number(&dir)?;
    let ts   = unix_now();
    let snap = format!("{dir}/v{n:04}_{ts}.snap");
    let tmp  = format!("{snap}.tmp");
    let mut enc = GzEncoder::new(File::create(&tmp)?, Compression::fast());
    enc.write_all(content)?;
    enc.finish()?;
    fs::rename(&tmp, &snap)?;           // atomic
    Ok(())
}
```

**Version restore gating (B4 fix)**: `@supervisor: doc_restore` requires both `filesystem = true` AND an active `BookmarkToken` for the file path. Additionally, the path must have an existing `DocEntry` history in the registry (prevents restoring arbitrary files the app has never opened):
```rust
fn handle_doc_restore(doc_id: u64, version_n: u32, sender: &str, ...) {
    let has_bookmark = bookmark_store.check(sender, &entry.path);
    let has_history  = version_dir_exists(&entry.path);
    if !has_bookmark || !has_history {
        return send_reply(sender, "VYOMA_DOC:error:unauthorized", inbox);
    }
    versions::restore(&entry.path, version_n)?;
}
```

---

## 6. Autosave & Shadow Files (B5 Fix)

Autosave shadow at `/data/.vyoma/autosave/<hex16>.shadow`. Written via R41 atomic-rename:

```rust
// supervisor/src/document/autosave.rs  (~80 lines)
pub fn flush_shadow(entry: &DocEntry) {
    let shadow = shadow_path(&entry.path);
    let tmp    = format!("{shadow}.tmp");
    // R41 pattern: write to .tmp, fsync, rename
    let mut f = File::create(&tmp).expect("autosave tmp");
    f.write_all(&entry.last_content).expect("autosave write");
    f.sync_all().expect("autosave fsync");
    fs::rename(&tmp, &shadow).expect("autosave rename");
}
```

`/data/.vyoma/autosave/` created with permissions `0o777` (world-writable) so any app can notify the supervisor of content to shadow — supervisor does the actual write via the R41 path. On startup, supervisor scans for orphaned `.shadow` files and offers recovery to the relevant app when it next opens the file (B5 fix).

---

## 7. File Layout

```
supervisor/src/document/
├── mod.rs          (~40 lines: DOC_REGISTRY + RECENTS_DB statics, init)
├── registry.rs     (~160 lines: DocEntry, DocSignal, sweep_stale_docs (B1))
├── recents.rs      (~120 lines: recents.db, per-app cap 10, system cap 100, wal_autocheckpoint (B2))
├── versions.rs     (~140 lines: save_version, restore, prune, gzip, restore gating (B4))
├── autosave.rs     (~80 lines: flush_shadow, R41 atomic-rename, startup scan (B5))
├── close.rs        (~80 lines: request_close, doc_signal channel, 5s timeout (B3))
└── ipc_handler.rs  (~180 lines: VYOMA_DOC: + @supervisor:doc_* dispatch)

supervisor/src/router.rs  (modified: VYOMA_DOC: dispatch arm)
```

---

## 8. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: App exit leaves stale DocEntry records; dirty state lost | `sweep_stale_docs(app_name)` on SIGCHLD: drain all entries for that app, flush dirty ones to autosave shadow |
| B2: SQLite WAL accumulates unbounded on non-desktop profiles; mcu-minimal has no document model | `PRAGMA wal_autocheckpoint=10` for non-desktop; `document-model` Cargo feature excluded for mcu-minimal |
| B3: `should_close` blocks IPC handler thread waiting for app reply | Per-doc `doc_signal: mpsc::Sender<DocSignal>` channel; signal thread waits on oneshot with 5s timeout; IPC handler returns immediately |
| B4: Version restore allows arbitrary file access without grants | Restore requires `filesystem = true` + `BookmarkToken` for the path + existing DocEntry history for that path |
| B5: Autosave shadow writes not atomic; `/data/.vyoma/autosave/` permission model unclear | R41 pattern: write `.tmp`, `fsync`, atomic `rename`; `/data/.vyoma/autosave/` mode `0o777`; supervisor does actual write |
