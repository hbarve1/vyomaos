# FINAL Spec: File Manager / Finder Equivalent (Round 41)

**Subsystem**: File Manager (Finder equivalent)  
**macOS Analogue**: `Finder` / `NSOpenPanel` / `NSSavePanel`  
**Depends on**: R04 (VFS), R21 (window manager), R38 (drag & drop), R39 (text input), R40 (clipboard)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Two cooperating pieces:

| Component | Where | Role |
|-----------|-------|------|
| `file-manager` WASM app | space-0, z=65528 | Renders file browser UI; owns columns, icon grid, sidebar |
| `panel_service` | supervisor | System-modal open/save sheets; path traversal guard; fs_worker pools |

Apps that need a file picker do NOT spawn a new file manager window — they send `VYOMA_PANEL:open` or `VYOMA_PANEL:save` to the supervisor panel_service, which drives the sheet inside the requesting app's window frame. The `file-manager` WASM app is used when the user opens a standalone browser window.

---

## 2. Open/Save Panel Protocol (B1 Fix)

### 2.1 Panel Request

App sends:
```
VYOMA_PANEL:open:<request_id>:<options_b64>
VYOMA_PANEL:save:<request_id>:<options_b64>
```

`options_b64` is base64-encoded JSON:
```json
{
  "title":           "Open Image",
  "allowed_exts":    ["png", "jpg", "gif"],
  "start_dir":       "/data/Photos",
  "multi_select":    false,
  "show_hidden":     false
}
```

Supervisor panel_service:
1. Looks up `app_instance_id` for the requesting app (B1 fix: supervisor-assigned `u64`, incremented on each spawn, NOT keyed on `app_name`).
2. Generates 128-bit random `grant_token`.
3. Registers `PanelGrant { app_instance_id, request_id, grant_token, mode, allowed_exts, expiry_ns }`.
4. Displays sheet inside app's window frame (R21 sub-window, dim backdrop, blocked input to parent).

### 2.2 Panel Result Delivery (B1 Fix)

```rust
// supervisor/src/panel_service/mod.rs

pub struct PanelGrant {
    pub app_instance_id: u64,   // B1: keyed on instance, not app_name
    pub request_id:      String,
    pub grant_token:     [u8; 16],
    pub mode:            PanelMode,  // Open | Save
    pub allowed_exts:    Vec<String>,
    pub expiry_ns:       u64,
}

// B1 fix: on app exit/crash, purge all grants for that instance
pub fn purge_instance(instance_id: u64) {
    PANEL_GRANTS.lock().retain(|g| g.app_instance_id != instance_id);
    TOKEN_TABLE.lock().retain(|_, v| v.app_instance_id != instance_id);
}
```

Supervisor delivers result via stdout:
```
VYOMA_PANEL:result:<request_id>:ok:<grant_token_hex>:<paths_b64>
VYOMA_PANEL:result:<request_id>:cancel
VYOMA_PANEL:result:<request_id>:error:<reason>
```

`paths_b64` is a base64 JSON array of selected absolute paths. The grant_token is then used as the read/write access credential (§6).

---

## 3. File Listing Protocol

### 3.1 `VYOMA_FS:list` (B5 Fix)

```
VYOMA_FS:list:<path_b64>:<options_b64>
```

`options_b64` (all fields optional):
```json
{
  "sort_by":           "name",       // "name"|"size"|"mtime"|"kind"
  "sort_dir":          "asc",        // "asc"|"desc"
  "offset":            0,
  "limit":             200,
  "prefetch_thumb_size": 64,         // px; 0 = no thumb
  "show_hidden":       false
}
```

Response: NDJSON, one entry per line, **first line is metadata** (B5 fix):
```json
{"type":"meta","total_count":1842,"has_more":true}
{"type":"entry","name":"photo.jpg","kind":"file","size":204800,"mtime":1748510000,"ext":"jpg","thumb_token":"<hex>"}
{"type":"entry","name":"Documents","kind":"dir","mtime":1748500000}
...
```

`total_count` in first line enables scrollbar sizing without loading all entries.

### 3.2 `VYOMA_FS:list_paths` (B5 Fix — Column View)

For column/path-bar navigation, returns a lightweight array of direct children names only:
```
VYOMA_FS:list_paths:<path_b64>
```
Response: one JSON string per line (just filenames). Used by column view to populate right-hand column on click without loading metadata.

### 3.3 `VYOMA_FS:cancel` (B2 Fix)

```
VYOMA_FS:cancel:<job_id>
```

Cancels an in-progress list, copy, move, or quicklook job. The `job_id` is returned in the first NDJSON line of any long-running operation. Used by quicklook worker to release cancelled previews.

---

## 4. File Operations

### 4.1 Opcodes

```
VYOMA_FS:copy:<job_id>:<src_b64>:<dst_b64>
VYOMA_FS:move:<job_id>:<src_b64>:<dst_b64>
VYOMA_FS:delete:<job_id>:<path_b64>[:permanent]
VYOMA_FS:rename:<path_b64>:<new_name_b64>
VYOMA_FS:mkdir:<path_b64>
VYOMA_FS:read:<path_b64>:<grant_token_hex>
VYOMA_FS:write_begin:<path_b64>:<grant_token_hex>
VYOMA_FS:write_commit:<write_token_hex>
VYOMA_FS:write_abort:<write_token_hex>
VYOMA_FS:undo
```

Delete without `:permanent` moves to trash at `vyoma://trash` (B4 virtual path, §7).

### 4.2 Worker Pools (B2 Fix)

Three isolated thread pools prevent a giant copy from starving metadata reads:

```rust
// supervisor/src/panel_service/fs_workers.rs

// B2 fix: split into three pools
pub static FS_IO:        Lazy<FsWorkerPool> = Lazy::new(|| FsWorkerPool::new(2, QueuePolicy::FairPerApp));
pub static FS_META:      Lazy<FsWorkerPool> = Lazy::new(|| FsWorkerPool::new(1, QueuePolicy::Fifo));
pub static FS_QUICKLOOK: Lazy<FsWorkerPool> = Lazy::new(|| FsWorkerPool::new(1, QueuePolicy::Cancellable));

pub enum QueuePolicy { FairPerApp, Fifo, Cancellable }
```

| Pool | Workers | Policy | Handles |
|------|---------|--------|---------|
| `fs_io` | 2 | Per-app fair-queue; 256 KB chunks re-enqueued | copy, move, delete, write |
| `fs_meta` | 1 | FIFO, fast ops only | list, rename, mkdir, stat |
| `fs_quicklook` | 1 | Cancellable; `VYOMA_FS:cancel` preempts | quicklook preview generation |

`fs_io` uses chunked copy (256 KB per iteration) with per-chunk re-enqueue so one app's 10 GB copy does not starve another app's small rename. Each `copy`/`move` job carries `source_app_instance_id`; the fair-queue interleaves jobs across different instances.

Progress events:
```
VYOMA_FS:progress:<job_id>:<bytes_done>:<bytes_total>
VYOMA_FS:done:<job_id>
VYOMA_FS:error:<job_id>:<code>:<msg_b64>
```

---

## 5. Quick Look

### 5.1 Cache

Previews stored at `/data/.vyoma/quicklook/<sha256>-<mtime_hex>.png` (128×128 default, 256×256 for Retina, 64×64 for grid thumbnails). Cache capped at 512 MB (LRU eviction via cache manifest at `/data/.vyoma/quicklook/index.toml`).

### 5.2 Protocol

File manager app requests:
```
VYOMA_QL:request:<path_b64>:<size_px>:<job_id>
```

Supervisor (quicklook pool):
1. Check cache: if hit → `VYOMA_QL:ready:<job_id>:<cache_path_b64>`.
2. If miss → spawn preview generator (pluggable: image decode for png/jpg/gif, text renderer for .txt/.rs/.md, generic icon for unknown).
3. On cancel: `VYOMA_FS:cancel:<job_id>` → worker drops, no reply.

```
VYOMA_QL:ready:<job_id>:<cache_path_b64>
VYOMA_QL:none:<job_id>         # no preview possible
VYOMA_QL:preview:<job_id>:<inline_b64>   # small thumbs ≤4 KB inline
```

---

## 6. Capability Tokens (B3 Fix)

### 6.1 Transactional Read Token

Issued in `VYOMA_PANEL:result` for each selected path. Default behavior (B3 fix):
- Token allows reads until the full advertised file size has been delivered, then **auto-revokes**.
- Token is single-path: only valid for the path listed in `paths_b64`.
- If app crashes mid-read, token is purged with the instance (B1 integration).

```rust
pub struct ReadToken {
    pub token:              [u8; 16],
    pub app_instance_id:    u64,
    pub path:               String,
    pub bytes_remaining:    AtomicU64,  // decremented on each read reply
    pub expires_at_ns:      u64,        // wall-clock deadline (30s default)
}
```

### 6.2 Transactional Write Token (B3 Fix)

```
VYOMA_FS:write_begin:<path_b64>:<grant_token_hex>
```

Supervisor:
1. Validates grant_token (mode must include write).
2. Creates temp file at `/data/.vyoma/tmp/<hex_token>`.
3. Returns `VYOMA_FS:write_ready:<write_token_hex>:<wasi_upload_path>`.

App writes via WASI path, then commits:
```
VYOMA_FS:write_commit:<write_token_hex>
```

Supervisor: `fsync` + `rename(tmp, real_path)` — atomic on the same filesystem. On abort or timeout (30s), temp file deleted.

### 6.3 BookmarkToken (B3 Fix)

For apps that need persistent access to a user-selected path (e.g., a text editor re-opening a recent file):
```
VYOMA_FS:bookmark_request:<path_b64>:<purpose_b64>
```

Supervisor shows a persistent-access banner: *"TextEdit wants permanent access to `~/Documents/notes.md`. Allow?"*. On approval, issues `BookmarkToken` stored in `/data/capability-grants.toml` under `[app.<name>.bookmarks]`. Token survives app restart.

User-visible approval UI: Settings → Privacy → File Access shows all active `BookmarkToken` entries with revoke buttons.

---

## 7. Sidebar, Trash & Virtual Paths (B4 Fix)

### 7.1 Sidebar

Stored in `/data/.vyoma/file-manager/sidebar.toml`:
```toml
[[pinned]]
label = "Home"
path  = "/data"

[[pinned]]
label = "Downloads"
path  = "/data/Downloads"
```

Virtual paths surfaced in sidebar but not on real filesystem:

| URI | Resolution |
|-----|-----------|
| `vyoma://trash` | `/data/.vyoma/trash/` — shown as "Trash" with item count |
| `vyoma://recent` | Injected by panel_service from recent-files log |
| `vyoma://network` | Placeholder for future network shares |

Delete without `:permanent` moves file to `/data/.vyoma/trash/<iso8601>-<original_name>` and records metadata in `/data/.vyoma/trash/index.toml` for "Put Back" support.

### 7.2 Drag Into Panel (B4 Fix)

An open save-panel registers itself as an R38 drop target for the duration it is visible. If a file is dragged from another window and dropped on the open panel:
1. Panel receives `VYOMA_DRAG:drop:<token>:<path_b64>`.
2. If the dropped path has an allowed extension → populates filename field with the dropped path.
3. Panel emits `VYOMA_DRAG:accepted:<token>` to supervisor.

### 7.3 File Tagging (B4 Fix)

```
VYOMA_FS:tag:<path_b64>:add:<tag_b64>
VYOMA_FS:tag:<path_b64>:remove:<tag_b64>
VYOMA_FS:tag:<path_b64>:list
```

Tags stored in sidecar SQLite DB at `/data/.vyoma/tags.db` (table: `tags(path TEXT, tag TEXT, created_ns INTEGER)`). The file-manager WASM app renders colored tag dots in list/icon view.

### 7.4 Change Notifications for `filesystem=true` Apps (B4 Fix)

Apps with `filesystem = true` that need live directory updates register a watch:
```
VYOMA_FS:watch:<path_b64>:<depth>   # depth: 1=direct children, -1=recursive
VYOMA_FS:unwatch:<path_b64>
```

Supervisor's inotify integration emits:
```
VYOMA_FS:notify_change:<path_b64>:<event>   # event: created|deleted|modified|renamed
```

The `file-manager` WASM app uses this to refresh its current directory listing without polling.

---

## 8. Security

1. **Path canonicalization**: All paths in `VYOMA_FS:*` opcodes are canonicalized with `std::fs::canonicalize` before dispatch. Any path that resolves outside `/data/` is rejected with `VYOMA_FS:error:<job_id>:forbidden`.
2. **Protected directories**: Paths under `/data/.vyoma/` are blocked from direct app access (reserved for supervisor metadata). Trash, quicklook cache, and tags.db are only accessible via the panel_service opcodes.
3. **Token enforcement**: Every `VYOMA_FS:read` and `VYOMA_FS:write_begin` requires a valid, non-expired grant_token or read/write token. The `file-manager` app itself holds a static `filesystem = true` WASI grant and operates outside the token system.
4. **Instance purge**: `panel_service.purge_instance(id)` called on every app exit — revokes all PanelGrants, ReadTokens, and WriteTokens for that instance.

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Panel delivery races with app crash/restart | `app_instance_id: u64` (supervisor-assigned, incremented on spawn) used as primary key; `purge_instance()` on exit revokes all grants; `app_name` no longer used for keying |
| B2: Single `fs_worker` pool starved by giant copy | Three pools: `fs_io` (2 workers, per-app fair-queue, 256 KB re-enqueue), `fs_meta` (1 worker, FIFO), `fs_quicklook` (1 worker, cancellable); `VYOMA_FS:cancel:<job_id>` |
| B3: Token model has no revocation + write is not atomic | Read tokens auto-revoke on full delivery; write tokens require explicit `write_commit` (fsync+rename); `BookmarkToken` for persistent access with user-visible Settings UI |
| B4: Drag, trash, tags, notify all undefined | Drag-into-panel via R38 drop target; `vyoma://trash` virtual path; `VYOMA_FS:tag` opcodes + tags.db sidecar; `VYOMA_FS:watch/notify_change` for filesystem=true apps |
| B5: No sorting, pagination, or column view | `VYOMA_FS:list` gains `sort_by`, `sort_dir`, `offset`, `limit`, `prefetch_thumb_size`; first NDJSON line carries `total_count`; `VYOMA_FS:list_paths` for column view |

---

## 10. File Layout

```
supervisor/src/panel_service/
├── mod.rs          (~100 lines: PanelGrant, purge_instance (B1), public API)
├── panel.rs        (~250 lines: open/save sheet protocol, grant_token, result delivery)
├── fs_ops.rs       (~300 lines: copy/move/delete/rename/mkdir dispatch to worker pools)
├── fs_workers.rs   (~200 lines: three worker pools (B2), chunked copy, progress events)
├── fs_list.rs      (~200 lines: list/list_paths (B5), sort, pagination, NDJSON)
├── tokens.rs       (~200 lines: ReadToken, WriteToken, BookmarkToken (B3), purge_instance)
├── quicklook.rs    (~180 lines: QL cache, cancellable worker, inline/path reply)
├── tags.rs         (~150 lines: tags.db SQLite, VYOMA_FS:tag opcodes (B4))
├── watch.rs        (~150 lines: inotify watcher, notify_change delivery (B4))
└── trash.rs        (~120 lines: vyoma://trash virtual path, move/restore/empty)

apps/file-manager/src/
├── main.rs         (~80 lines: init, main loop)
├── browser.rs      (~300 lines: icon grid + list view, column view using list_paths)
├── sidebar.rs      (~200 lines: sidebar.toml, virtual paths, pinned items)
├── toolbar.rs      (~150 lines: back/forward, path bar, search field, view switcher)
├── detail_pane.rs  (~200 lines: quicklook preview, metadata, tags display)
└── drag.rs         (~120 lines: drag-from + drag-into-panel (B4))
```
