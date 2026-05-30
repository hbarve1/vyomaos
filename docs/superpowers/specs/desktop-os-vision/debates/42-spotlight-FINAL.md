# FINAL Spec: Spotlight & Metadata Search (Round 42)

**Subsystem**: Spotlight & Metadata Search  
**macOS Analogue**: `Spotlight` / `MDQuery` / `NSMetadataQuery`  
**Depends on**: R04 (VFS), R21 (window manager), R35 (Cmd+Space hotkey), R41 (FS watch + notify_change, BookmarkToken, tags.db)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Two cooperating pieces:

| Component | Where | Role |
|-----------|-------|------|
| `supervisor/src/spotlight/` | Supervisor (native thread) | Index maintenance, query serving, access control |
| `apps/spotlight/` | WASM app (space-0, z=150) | Search UI, keyboard navigation, result launch |

The indexer runs as a **native thread inside the supervisor** (not a WASM app): it needs direct SQLite access, reacts to internal R41 `WatchEvent` state, and must not hold permanent `filesystem = true` open to a privileged WASM process.

---

## 2. Index Architecture

**Storage**: `/data/.vyoma/spotlight/index.db` (SQLite WAL mode)

**What is indexed**:

| Category | Fields | Source |
|----------|--------|--------|
| Files in `/data/` | path, kind, mtime_unix, size_bytes, content_snippet (≤512 UTF-8 bytes for text extensions) | R41 `VYOMA_FS:watch` + inotify walk |
| Installed WASM apps | name, display_name, version, icon_path, manifest_path | boot.toml + vyoma.toml manifests |
| App-provided metadata | app_instance_id, key, value JSON (≤4 KB) | `VYOMA_SPOTLIGHT:meta_push` |
| Tags | path → tag array | Joined from `/data/.vyoma/tags.db` via `ATTACH DATABASE` |

**Schema**:

```sql
CREATE TABLE IF NOT EXISTS entries (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL UNIQUE,
    kind         TEXT    NOT NULL,   -- 'file' | 'dir' | 'app'
    display_name TEXT    NOT NULL,
    mtime_unix   INTEGER NOT NULL DEFAULT 0,
    size_bytes   INTEGER NOT NULL DEFAULT 0,
    content_snippet TEXT,            -- NULL for non-text files (B4 fix)
    icon_path    TEXT,
    score_boost  REAL    NOT NULL DEFAULT 1.0,
    updated_at   INTEGER NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts
    USING fts5(display_name, content_snippet, path UNINDEXED,
               content='entries', content_rowid='id');

CREATE TABLE IF NOT EXISTS app_meta (
    app_instance_id INTEGER NOT NULL,
    entry_id        INTEGER REFERENCES entries(id) ON DELETE CASCADE,
    key             TEXT NOT NULL,
    value_json      TEXT NOT NULL,
    PRIMARY KEY (app_instance_id, key)
);

CREATE TABLE IF NOT EXISTS launch_freq (
    path        TEXT PRIMARY KEY,
    count       INTEGER NOT NULL DEFAULT 0,
    last_launch INTEGER NOT NULL DEFAULT 0
);

-- B3 fix: keyed on app_name, not app_instance_id
CREATE TABLE IF NOT EXISTS file_access_grants (
    app_name    TEXT NOT NULL,
    path_prefix TEXT NOT NULL,
    PRIMARY KEY (app_name, path_prefix)
);
```

---

## 3. Indexer Pipeline

### 3.1 Dual-Connection Model (B1 Fix)

Two SQLite connections prevent boot-walk write lock from starving query reads:

```rust
// supervisor/src/spotlight/indexer.rs

pub struct Indexer {
    db_path:      String,
    state:        IndexerState,
    walk_queue:   VecDeque<PathBuf>,
    walk_depth:   HashMap<PathBuf, u32>,
    rate_limiter: RateLimiter,   // 20 file-stat/sec (B4 fix: separate budget)
    snip_limiter: RateLimiter,   // 5 snippet-read/sec (B4 fix)
    write_conn:   rusqlite::Connection,  // B1: exclusive to indexer thread
    cmd_rx:       mpsc::Receiver<IndexCmd>,
}

// B1 fix: query threads open their own read-only connection
pub fn open_read_conn(db_path: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_with_flags(
        db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    ).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    conn
}
```

Boot walk commits in **batches of 100 rows** (`BEGIN; INSERT 100; COMMIT;`), releasing the write lock every ~5 seconds. Query threads (2-thread pool) are unblocked between commits.

### 3.2 Boot Walk

On supervisor startup, if `index.db` doesn't exist or is >24h stale:
1. Push `/data/` to `walk_queue`.
2. Each iteration: pop one path, call `fs::metadata`, upsert `entries` row. Max depth 6 to prevent pathological trees.
3. Rate-limited via token bucket (`rate_limiter`: 20 file-stat/sec; `snip_limiter`: 5 snippet-read/sec — see B4 fix).
4. After walk completes: `INSERT INTO entries_fts(entries_fts) VALUES('rebuild')` for bulk FTS rebuild (faster than incremental inserts).
5. Log `[spotlight] boot-index done: N entries in T ms`.

### 3.3 Live Updates via R41 Watch Events

The R41 filesystem-watch subsystem emits `WatchEvent { path, kind: Created|Modified|Deleted }`. The indexer drains these every 100ms:
- `Created`/`Modified`: upsert `entries` row; FTS5 content-table triggers auto-update.
- `Deleted`: `DELETE FROM entries WHERE path = ?` (FTS5 cascades via trigger).

### 3.4 App Manifest Indexing

On `spawn_app`: send `IndexCmd::AppSpawned { name, display_name, ... }` → indexer upserts `kind='app'` row.  
On `pkg-remove`: send `IndexCmd::AppRemoved { name }` — **B5 fix** (see §8).

```rust
pub enum IndexCmd {
    FileEvent    { path: PathBuf, event: WatchEventKind },
    AppSpawned   { name: String, display_name: String, icon_path: Option<String> },
    AppRemoved   { name: String },               // B5 fix
    MetaPush     { app_instance_id: u64, key: String, value_json: String },
    LaunchRecord { path: String },
    Shutdown,
}
```

**Global sender** in supervisor: `static SPOTLIGHT_TX: OnceLock<mpsc::Sender<IndexCmd>> = OnceLock::new();`

---

## 4. Query Protocol

### 4.1 Protocol Lines

App → Supervisor (stdout):
```
VYOMA_SPOTLIGHT:query:<rid>:<query_b64>
VYOMA_SPOTLIGHT:query_page:<rid>:<offset>:<limit>
VYOMA_SPOTLIGHT:cancel:<rid>
VYOMA_SPOTLIGHT:meta_push:<key_b64>:<value_json_b64>
VYOMA_SPOTLIGHT:launch_record:<path_b64>
```

Supervisor → App (stdin):
```
VYOMA_SPOTLIGHT:result:<rid>:<total>:<json_b64>
VYOMA_SPOTLIGHT:result_end:<rid>
VYOMA_SPOTLIGHT:error:<rid>:<reason>
```

`<query_b64>` and `<key_b64>` are base64url-encoded UTF-8 (prevents protocol-breaking spaces/colons). `<limit>` max = 50. `<total>` = total matching entries count for scrollbar sizing.

### 4.2 Router Placement (B2 Fix)

The `VYOMA_SPOTLIGHT:` prefix check is **unconditional** in `router.rs` — NOT gated on `has_display`:

```rust
pub fn route_or_print(line: &str, sender: &str, ...) {
    // 1. Display commands (gated on has_display)
    if has_display {
        if let Some(cmd) = line.strip_prefix("VYOMA_DRAW:") { ... return; }
    }

    // 2. Spotlight — NOT gated on has_display (B2 fix)
    if let Some(cmd) = line.strip_prefix("VYOMA_SPOTLIGHT:") {
        crate::spotlight::handle_spotlight_command(cmd, sender, inbox, app_registry);
        return;
    }

    // 3. IPC routing
    if let Some(rest) = line.strip_prefix('@') { ... }
    println!("[{sender}] {line}");
}
```

Capability violations send explicit error reply (B2 fix):
```
VYOMA_SPOTLIGHT:error:<rid>:no-spotlight-capability
```

### 4.3 Query Execution

```rust
// supervisor/src/spotlight/query.rs
pub fn execute_query(
    conn: &rusqlite::Connection,
    query: &str,
    app_name: &str,
    offset: usize,
    limit: usize,
) -> Result<(Vec<SearchResult>, usize), rusqlite::Error>
```

FTS5 query with launch-frequency boost:
```sql
ATTACH DATABASE '/data/.vyoma/tags.db' AS tags;

SELECT e.path, e.kind, e.display_name, e.icon_path,
       snippet(entries_fts, 1, '<b>', '</b>', '…', 20),
       bm25(entries_fts) * e.score_boost AS relevance,
       e.mtime_unix, e.size_bytes
FROM entries_fts
JOIN entries e ON entries_fts.rowid = e.id
WHERE entries_fts MATCH ?
  AND (
    e.kind = 'app'
    OR EXISTS (
        SELECT 1 FROM file_access_grants
        WHERE app_name = ? AND e.path LIKE path_prefix || '%'
    )
  )
ORDER BY relevance DESC
LIMIT ? OFFSET ?
```

**Launch frequency boost**: On `VYOMA_SPOTLIGHT:launch_record:<path_b64>`, increment `launch_freq.count`; recalculate `entries.score_boost = MIN(2.5, 1.0 + ln(1 + count) / 10.0)`.

**Cancellation**: `HashSet<u64>` of cancelled `rid`s; checked before each page send. SQLite statement not interrupted (fast on <10K rows); results dropped client-side.

**Pagination**: Stateless re-execution per `query_page` call. FTS5 on <10K entries: <1ms per query.

### 4.4 SearchResult Schema

```json
{
  "path":         "/data/notes/todo.txt",
  "kind":         "file",
  "display_name": "todo.txt",
  "icon_path":    null,
  "snippet":      "Buy milk, call dentist…",
  "score":        0.87,
  "mtime_unix":   1748563200,
  "size_bytes":   1024
}
```

---

## 5. Content Snippet Extraction (B4 Fix)

**Hard read limit**: Always `File::open` + `Read::take(512)` — never `fs::read`. Bounded to one 512-byte read regardless of file size:

```rust
fn extract_snippet(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 512];
    let n = f.read(&mut buf).ok()?;
    std::str::from_utf8(&buf[..n]).ok().map(|s| s.to_owned())
}
```

**Extension allowlist** (compile-time): `&[".txt", ".md", ".toml", ".json", ".rs", ".log"]`. Binary files (`.wasm`, `.png`, `.db`) get `content_snippet = NULL`.

**Separate snippet budget**: `snip_limiter` token bucket at 5 reads/sec (independent of `rate_limiter`'s 20 stat/sec). If snippet tokens exhausted, index file without snippet and queue for snippet-fill pass after boot walk completes.

---

## 6. App-Provided Metadata

New capability in `vyoma.toml`:
```toml
[capabilities]
spotlight = true   # allows VYOMA_SPOTLIGHT:meta_push
```

Added to `Capabilities` struct in `manifest.rs`. `AppState` gains two new fields:
```rust
pub has_spotlight_cap: bool,
pub app_instance_id:   u64,  // monotonic counter, set in spawn_app
```

**Protocol**:
```
VYOMA_SPOTLIGHT:meta_push:<key_b64>:<value_json_b64>
```

Payload cap: 4 KB. Supervisor rejects oversized pushes with:
```
VYOMA_SPOTLIGHT:error:0:meta-too-large
```

`app_meta` row keyed on `(app_instance_id, key)` — restarted app overwrites stale metadata from previous instance.

---

## 7. Security — Access Control (B3 Fix)

**Access grants keyed on `app_name` (not `app_instance_id`)** to survive app restart:

On `spawn_app`: `DELETE FROM file_access_grants WHERE app_name = ?` then re-insert from manifest. This is a single write transaction — atomic refresh.

- Apps with `filesystem = true`: automatic grant for `/data/<app_name>/` (or manifest-declared `data_prefix`).
- Apps with `filesystem = false`: no file grants; can only see `kind = 'app'` entries.
- Spotlight WASM app itself (`filesystem = false`): sees apps catalog only; file results only visible via `shell = true` + `@supervisor: spotlight-grant <path_prefix>`.

**Collision detection** (B3 fix): On `spawn_app`, supervisor checks if any existing `file_access_grants` row has the new app's `path_prefix` under a different `app_name`. On collision, manifest rejected:
```
[capability] ERROR: /data/password-manager/ already claimed by 'password-manager'
```

---

## 8. App Removal (B5 Fix)

`pkg-remove` handler sends `IndexCmd::AppRemoved { name }` to indexer, which executes:

```sql
DELETE FROM entries WHERE path LIKE '/apps/' || ? || '/%' AND kind = 'app';
DELETE FROM file_access_grants WHERE app_name = ?;
DELETE FROM app_meta WHERE app_instance_id IN (
    SELECT DISTINCT app_instance_id FROM app_meta
    JOIN entries ON app_meta.entry_id = entries.id
    WHERE display_name = ?
);
```

Prevents stale `kind = 'app'` entries and stale grants from persisting after uninstall.

---

## 9. Spotlight UI (WASM App)

**Invocation**: R35 Cmd+Space → supervisor opens `apps/spotlight/` WASM app. If already running → sends ESC to stdin (toggle). Previous focused app stored in `SPOTLIGHT_PREV_FOCUS: Mutex<Option<String>>`.

**Window**: space-0, z=150, 600×400px, centered. R16 open animation: scale 0.92→1.0, alpha 0→255, 200ms.

**Layout**:
```
┌──────────────────────────────────────────────┐
│  [🔍 20×20]  [query text cursor___]          │  search bar (BOX_W=600, r=12)
├──────────────────────────────────────────────┤
│  ● icon  Name                           ▸    │  result rows (ROW_H=56)
│    /path/snippet…                            │
├──────────────────────────────────────────────┤
│  … up to 8 visible rows …                    │
├──────────────────────────────────────────────┤
│  ↑↓ navigate   ↵ open   ⎋ dismiss           │  footer hint
└──────────────────────────────────────────────┘
```

**Keyboard**:
- Printable characters: append to query → re-query with new `rid`.
- Backspace: pop char → re-query.
- Up/Down: move cursor through results (wrap).
- Enter: open selected result.
- Tab: cycle categories (Apps → Files → Documents).
- Escape: clear query if non-empty; dismiss if empty.

**Opening a result**:
```rust
fn open_result(result: &SearchResult) {
    match result.kind.as_str() {
        "app"  => println!("@supervisor: run /apps/{}/vyoma.toml", result.path),
        "file" => println!("@supervisor: spotlight-open {}", base64url(&result.path)),
        _ => {}
    }
    println!("VYOMA_SPOTLIGHT:launch_record:{}", base64url(&result.path));
    std::process::exit(0);
}
```

On exit: Spotlight sends `@supervisor: focus <prev>` to restore pre-spotlight focus.

**Supervisor `spotlight-open` handler** (in `ipc_commands/mod.rs`): looks up handler app for the file's extension, grants a R41 `BookmarkToken` (128-bit random) to that handler, launches it with `VYOMA_FS:open_bookmark:<token>` on its stdin.

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Boot-walk write lock starves query reads | Two connections: write-conn exclusive to indexer thread; query threads open `SQLITE_OPEN_READ_ONLY`; boot walk commits in batches of 100 rows (max ~5s lock-hold per batch) |
| B2: `VYOMA_SPOTLIGHT:query` silently dropped for non-display apps | `VYOMA_SPOTLIGHT:` check in `router.rs` is unconditional — NOT inside `if has_display` guard; capability violations send explicit `error:<rid>:no-spotlight-capability` |
| B3: `app_instance_id`-keyed grants go stale/leak on app restart | `file_access_grants` keyed on `(app_name, path_prefix)`; atomic `DELETE + INSERT` on every `spawn_app`; collision detection rejects duplicate path_prefix claims |
| B4: Snippet extraction blocks indexer on large files | Hard cap: `Read::take(512)` regardless of file size; extension allowlist skips binaries; separate `snip_limiter` (5/sec) independent of stat budget; snippet-fill deferred pass after boot walk |
| B5: Uninstalled app entries + grants persist in index | `pkg-remove` sends `IndexCmd::AppRemoved { name }` → indexer deletes `entries`, `file_access_grants`, and `app_meta` rows for that app_name in one write transaction |

---

## 11. File Layout

```
supervisor/src/spotlight/
├── mod.rs          (~80 lines: handle_spotlight_command, SPOTLIGHT_TX, module init)
├── indexer.rs      (~420 lines: Indexer struct, boot walk (B1 batch commits), rate limiters (B4),
│                               IndexCmd, AppRemoved handler (B5))
├── query.rs        (~200 lines: execute_query, open_read_conn (B1), access-control SQL (B3),
│                               SearchResult struct, launch_record boost)
├── schema.rs       (~60 lines: DDL strings, ensure_schema(conn), file_access_grants (B3))
└── handler.rs      (~140 lines: meta_push (capability check, 4KB cap), launch_record,
│                               cancel, spotlight-open handler, B2 unconditional routing note)

apps/spotlight/src/
├── main.rs         (~280 lines: event loop, keyboard nav, query lifecycle, focus restore)
├── query.rs        (~120 lines: protocol encode/decode, base64url, SearchResult deserialize)
├── render.rs       (~180 lines: VYOMA_DRAW calls, layout constants, result row rendering)
└── search.rs       (~55 lines: static fallback app list for offline/pre-index mode)
```
