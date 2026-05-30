# FINAL Spec: File Tagging & Metadata (Round 47)

**Subsystem**: File Tagging & Metadata  
**macOS Analogue**: `Finder tags` / `xattrs` / `NSMetadataItem`  
**Depends on**: R04 (VFS), R41 (file_access_grants, VYOMA_FS:watch), R42 (Spotlight indexer)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Storage Architecture

Two SQLite databases under `/data/.vyoma/`:

**`tags.db`** (extended):
```sql
CREATE TABLE tag_names (
    id         INTEGER PRIMARY KEY,
    name       TEXT UNIQUE NOT NULL,
    color      TEXT,                 -- hex color "#FF6347" or NULL
    created_at INTEGER NOT NULL
);
CREATE TABLE file_tags (
    path_hash  TEXT NOT NULL,        -- sha256(canonical_path)
    tag_id     INTEGER NOT NULL REFERENCES tag_names(id) ON DELETE CASCADE,
    tagged_at  INTEGER NOT NULL,
    tagged_by  TEXT NOT NULL,        -- app_name
    PRIMARY KEY (path_hash, tag_id)
);
CREATE INDEX file_tags_tag_id ON file_tags(tag_id);
```

**`xattrs.db`** (new — extended attributes + app metadata):
```sql
CREATE TABLE xattrs (
    path_hash  TEXT NOT NULL,
    ns         TEXT NOT NULL,        -- "vyoma", "user", "com.<bundle>"
    key        TEXT NOT NULL,
    value      BLOB NOT NULL,
    set_at     INTEGER NOT NULL,
    set_by     TEXT NOT NULL,        -- app_instance_id
    PRIMARY KEY (path_hash, ns, key)
);
CREATE TABLE app_meta (
    path_hash  TEXT NOT NULL,
    bundle_id  TEXT NOT NULL,        -- "com.vyomaos.notes"
    key        TEXT NOT NULL,
    value      BLOB NOT NULL,
    PRIMARY KEY (path_hash, bundle_id, key)
);
```

Both databases opened in WAL mode. Writer connection: exclusive to writer thread. Reader connection: read-only, used by query threads (B2 fix).

**rusqlite musl compilation (B1 fix)**: `rusqlite` with `bundled` feature compiles SQLite C source. Requires:
```toml
# supervisor/Cargo.toml
rusqlite = { version = "0.31", features = ["bundled", "backup"] }
```
```toml
# supervisor/.cargo/config.toml
[target.x86_64-unknown-linux-musl]
linker = "musl-gcc"
[env]
CC_x86_64_unknown_linux_musl = "musl-gcc"
```
`Makefile` passes `CC_x86_64_unknown_linux_musl=musl-gcc` for Docker builds:
```makefile
supervisor:
    docker run ... -e CC_x86_64_unknown_linux_musl=musl-gcc cargo build ...
```

---

## 2. Namespace Model

| Namespace | Format | Writer | Reader |
|-----------|--------|--------|--------|
| `vyoma.*` | `vyoma.quarantine`, `vyoma.origin_url` | supervisor only | any filesystem app |
| `user.*` | `user.comment`, `user.rating` | any filesystem app | any filesystem app |
| `com.<bundle>.*` | `com.vyomaos.notes.folded` | only the matching bundle | only the matching bundle |

**`bundle_id: Option<String>` in AppMeta (B5 fix)**: `AppState` gains:
```rust
pub bundle_id: Option<String>,   // from vyoma.toml [app] bundle_id = "com.vendor.app"
```
Namespace enforcement in `ipc_handler`:
```rust
fn check_ns_write(ns: &str, sender_bundle: Option<&str>) -> bool {
    if ns == "vyoma" { return false; }           // supervisor-only
    if ns == "user"  { return true; }
    // com.<bundle>:
    if let Some(b) = ns.strip_prefix("com.") {
        return sender_bundle.map(|s| s == b).unwrap_or(false);
    }
    false
}
```
The check uses `bundle_id` (e.g. `"com.vyomaos.notes"`) — NOT the unqualified app `name` (B5 fix).

---

## 3. Protocol Lines

App stdout → supervisor (all require `filesystem = true`):
```
VYOMA_FS:tag/<path_b64>/add/<tag_name>
VYOMA_FS:tag/<path_b64>/remove/<tag_name>
VYOMA_FS:tag/<path_b64>/list
VYOMA_FS:tag/rename/<old_name>/<new_name>
VYOMA_FS:tag/delete/<tag_name>
VYOMA_FS:tag/list_all
VYOMA_FS:tag/search/<tag_name>                 # paths with this tag
VYOMA_FS:xattr/<path_b64>/set/<ns_key_b64>/<value_b64>
VYOMA_FS:xattr/<path_b64>/get/<ns_key_b64>
VYOMA_FS:xattr/<path_b64>/list
VYOMA_FS:xattr/<path_b64>/remove/<ns_key_b64>
VYOMA_FS:app_meta/<path_b64>/set/<key_b64>/<value_b64>
VYOMA_FS:app_meta/<path_b64>/get/<key_b64>
VYOMA_FS:app_meta/<path_b64>/list
```

Supervisor → app stdin:
```
VYOMA_FS:tag_reply:<path_b64>:ok
VYOMA_FS:tag_list_reply:<path_b64>:<json_b64>
VYOMA_FS:tag_all_reply:<json_b64>
VYOMA_FS:tag_search_reply:<tag>:<paths_json_b64>
VYOMA_FS:xattr_reply:<path_b64>:ok
VYOMA_FS:xattr_get_reply:<path_b64>:<ns_key_b64>:<value_b64>
VYOMA_FS:xattr_list_reply:<path_b64>:<json_b64>
VYOMA_FS:app_meta_reply:<path_b64>:ok
VYOMA_FS:error:<reason>
```

---

## 4. Router Dispatch (B4 Fix)

`router.rs` currently has NO `VYOMA_FS:` arm — all opcodes silently drop. Fix: add dispatch arm before the draw block:

```rust
// supervisor/src/router.rs — add BEFORE VYOMA_DRAW: check
if let Some(rest) = line.strip_prefix("VYOMA_FS:") {
    // Routing: watch/notify handled by cloud_sync/ipc_bridge already;
    // tag/xattr/app_meta handled by new meta_store handler
    if rest.starts_with("tag/") || rest.starts_with("xattr/") || rest.starts_with("app_meta/") {
        meta_store::ipc_handler::handle_meta_line(rest, sender, &META_STORE, inbox);
        return;
    }
    // write_begin / write_commit / cancel / watch handled in fs_ipc.rs
    fs_ipc::handle_fs_line(rest, sender, app_registry, inbox);
    return;
}
```

`META_STORE` global:
```rust
// supervisor/src/meta_store/mod.rs
pub static META_STORE: OnceLock<MetaStore> = OnceLock::new();
// initialized in main() after data disk confirmed available
```

---

## 5. WAL Concurrency (B2 Fix)

Single `Mutex<Connection>` serializes all reads behind writes. Fix: split into writer + readers:

```rust
// supervisor/src/meta_store/db.rs
pub struct MetaStore {
    writer: Mutex<Connection>,      // exclusive: INSERT/UPDATE/DELETE only
    reader: Connection,             // read_only=true, no mutex needed for parallel reads
}

impl MetaStore {
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let writer = Connection::open(path)?;
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let reader = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self { writer: Mutex::new(writer), reader })
    }
}
```

Tag lookups (`tag_list`, `xattr_get`) use `self.reader` directly (no contention). Tag mutations use `self.writer.lock()`.

---

## 6. Tag Rename (B3 Fix)

`BEGIN IMMEDIATE` prevents ghost `tag_names` rows when two apps rename simultaneously:

```rust
// supervisor/src/meta_store/tags.rs
pub fn rename_tag(store: &MetaStore, old: &str, new: &str) -> Result<(), String> {
    let conn = store.writer.lock().unwrap();
    conn.execute_batch("BEGIN IMMEDIATE")?;
    // Check new name doesn't already exist
    let exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM tag_names WHERE name = ?1",
        params![new],
        |r| r.get(0),
    )?;
    if exists {
        conn.execute_batch("ROLLBACK")?;
        return Err(format!("tag '{new}' already exists"));
    }
    conn.execute("UPDATE tag_names SET name = ?1 WHERE name = ?2", params![new, old])?;
    conn.execute_batch("COMMIT")?;
    Ok(())
}
```

Without `BEGIN IMMEDIATE`, two concurrent renames to the same target could each pass the existence check and then race to violate the `UNIQUE` constraint — leaving a half-updated `tag_names` row. `IMMEDIATE` acquires a write lock before either read, preventing the race (B3 fix).

---

## 7. Spotlight Integration

When tags change, `META_STORE` notifies the Spotlight indexer via the existing `IndexCmd` channel:

```rust
// after successful tag add/remove:
if let Some(idx) = SPOTLIGHT_INDEX.get() {
    let _ = idx.cmd_tx.send(IndexCmd::Reindex(canonical_path));
}
```

Tag data written to `xattrs.db` is also readable by the Spotlight indexer thread via the reader connection, enabling `tag:red` queries in Spotlight.

---

## 8. File Layout

```
supervisor/src/meta_store/
├── mod.rs           (~50 lines: MetaStore struct, OnceLock, init)
├── db.rs            (~80 lines: open, WAL setup, writer+reader split (B2))
├── tags.rs          (~150 lines: add/remove/list/rename (B3)/delete/search)
├── xattrs.rs        (~120 lines: set/get/list/remove, namespace check (B5))
├── app_meta.rs      (~80 lines: set/get/list, bundle_id scoping (B5))
└── ipc_handler.rs   (~160 lines: parse VYOMA_FS:tag|xattr|app_meta lines, route (B4))

supervisor/src/router.rs  (modified: add VYOMA_FS: dispatch arm (B4))
supervisor/Cargo.toml     (modified: rusqlite bundled feature (B1))
supervisor/.cargo/config.toml  (modified: CC_x86_64_unknown_linux_musl (B1))
Makefile                  (modified: pass CC env var to Docker builds (B1))
```

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: rusqlite bundled feature fails to compile for musl target | `CC_x86_64_unknown_linux_musl=musl-gcc` in Makefile + `.cargo/config.toml` |
| B2: Single `Mutex<Connection>` serializes all reads behind writes | Split into `writer: Mutex<Connection>` (mutations) + `reader: Connection` (read_only, no lock) |
| B3: Concurrent tag renames corrupt `tag_names` UNIQUE constraint | `BEGIN IMMEDIATE` transaction + pre-check for target name existence before UPDATE |
| B4: `VYOMA_FS:` has no dispatch arm in `router.rs` — all opcodes silently dropped | Add `VYOMA_FS:` arm in `router.rs` dispatching to `meta_store::ipc_handler`; `META_STORE: OnceLock` |
| B5: Namespace enforcement uses unqualified app `name` — insufficient for `com.<bundle>.*` | `bundle_id: Option<String>` in `AppState`; namespace check strips `com.` prefix and matches against `bundle_id` |
