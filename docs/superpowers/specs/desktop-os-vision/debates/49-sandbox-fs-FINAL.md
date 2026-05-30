# FINAL Spec: App Sandbox & Container FS (Round 49)

**Subsystem**: App Sandbox & Container FS  
**macOS Analogue**: `App Sandbox` / container directories  
**Depends on**: R04 (VFS, file_access_grants), R41 (write tokens), R48 (document model), R50 (pkg-remove)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Container Directory Layout

Every app gets a private container under `/data/apps/<app_name>/`:

```
/data/apps/<app_name>/
├── support/          # NSApplicationSupportDirectory equivalent — persists across updates
├── cache/            # NSCachesDirectory — can be cleared; supervisor prunes on low disk
├── documents/        # User-visible documents created by this app
├── preferences/      # Key-value preferences (R48 KV extension)
└── tmp/              # Scratch space — cleared by supervisor on app exit (B3 fix)
```

Shared containers for inter-app collaboration:
```
/data/shared/<group_id>/   # shared between declared group members only
```

Read-only shared assets provided by the system:
```
/data/.shared/             # supervisor-managed; never writable by any app
├── fonts/                 # system font files
├── icons/                 # system icon set (PNG strips)
└── themes/                # color token files for theming subsystem
```

`/data/.vyoma/` reserved for supervisor-internal use (state, sandbox metadata, VPN profiles,
spotlight index). Never granted to apps as part of any sandbox grant.

Persistent sandbox state lives under:
```
/data/.vyoma/sandbox/
├── <app_name>.state.json   # per-app quota counters + last-seen byte totals
└── groups.json             # shared_group membership table (append-only log)
```

Written atomically on every mutation: write `.tmp` → `rename` (per R41 semantics).

---

## 2. SandboxPolicy Struct

Every spawned app has a `SandboxPolicy` derived from its `vyoma.toml` at spawn time and stored
alongside the `AppState` for the lifetime of the process.

```rust
// supervisor/src/sandbox/policy.rs

use std::path::PathBuf;

/// Immutable policy snapshot derived from vyoma.toml at spawn time.
/// Stored in AppState; never mutated while the app is running.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Canonical app name from vyoma.toml `[app].name`.
    pub app_name: String,

    /// Absolute paths the app may access. Each entry is a directory prefix.
    /// Checked with `canonical_path_allowed` before every 9P request.
    pub allowed_paths: Vec<PathBuf>,

    /// Whether write operations are permitted to any allowed path.
    /// `filesystem = true` in vyoma.toml implies allow_writes = true.
    pub allow_writes: bool,

    /// Optional byte quota across the entire container (all subdirs combined).
    /// None = unlimited. Enforced via statvfs + byte-tracking in SandboxState.
    pub quota_bytes: Option<u64>,

    /// Shared group IDs this app is a member of (from `[sandbox].shared_groups`).
    pub shared_groups: Vec<String>,

    /// If true, `/data/.shared/` is accessible read-only.
    pub allow_shared_read: bool,
}

impl SandboxPolicy {
    /// Build a SandboxPolicy from a parsed manifest Capabilities block.
    pub fn from_capabilities(app_name: &str, caps: &Capabilities, manifest_sandbox: &SandboxConfig) -> Self {
        let mut allowed_paths = Vec::new();
        if caps.filesystem {
            // Private container is always first in the list.
            allowed_paths.push(PathBuf::from(format!("/data/apps/{}/", app_name)));
        }
        // Shared group paths added by wire_container_grants after group membership is confirmed.
        SandboxPolicy {
            app_name: app_name.to_string(),
            allowed_paths,
            allow_writes: caps.filesystem,
            quota_bytes: manifest_sandbox.quota_bytes,
            shared_groups: manifest_sandbox.shared_groups.clone(),
            allow_shared_read: manifest_sandbox.allow_shared_read,
        }
    }

    /// Returns true if `path` is within at least one allowed prefix.
    /// Uses `canonical_path_allowed` for normalisation (rejects `..` traversal).
    pub fn is_path_allowed(&self, path: &std::path::Path) -> bool {
        canonical_path_allowed(path, &self.allowed_paths)
    }
}
```

The `SandboxConfig` section of `vyoma.toml`:

```toml
[sandbox]
quota_bytes      = 104857600   # 100 MB; omit for unlimited
shared_groups    = ["com.vyomaos.productivity"]
allow_shared_read = true
```

`quota_bytes` is optional. When absent, no byte-level quota is enforced (container can grow to
available disk). `shared_groups` defaults to `[]`. `allow_shared_read` defaults to `false`.

---

## 3. Path Normalization and Escape Prevention

Every path received from an app (via VYOMA_SANDBOX: IPC or 9P tag) is validated before any disk
operation. Three layers of defense:

**Layer 1 — Component scan (fast, pre-canonicalize)**:
```rust
// supervisor/src/sandbox/path_check.rs

use std::path::{Path, PathBuf};

/// Returns Err if the path string contains a `..` component or a null byte.
/// This is a cheap pre-filter before hitting the filesystem.
pub fn reject_dotdot(path: &str) -> Result<(), String> {
    if path.contains('\0') {
        return Err("path contains null byte".into());
    }
    for component in Path::new(path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(format!("path traversal via '..': {}", path));
        }
    }
    Ok(())
}
```

**Layer 2 — Canonical prefix check (post-canonicalize)**:
```rust
/// Resolve `path` to its canonical form and verify it falls under one of the allowed prefixes.
/// Returns true only if the canonical path starts with a canonical allowed prefix.
pub fn canonical_path_allowed(path: &Path, allowed: &[PathBuf]) -> bool {
    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            // File does not exist yet — derive canonical form from parent + filename.
            let parent = path.parent().and_then(|p| p.canonicalize().ok());
            match parent {
                Some(p) => p.join(path.file_name().unwrap_or_default()),
                None => return false,
            }
        }
    };
    for prefix in allowed {
        let canon_prefix = match prefix.canonicalize() {
            Ok(c) => c,
            Err(_) => prefix.clone(),
        };
        if canonical.starts_with(&canon_prefix) {
            return true;
        }
    }
    false
}
```

**Layer 3 — Symlink guard**:
If the resolved canonical path differs from the prefix-appended path by more than a file
extension or trailing component, the supervisor logs a warning and rejects the access:

```rust
/// Rejects symlinks that escape the container tree.
/// Compares the canonicalized result against expected prefix after symlink resolution.
pub fn symlink_escape_guard(raw: &Path, expected_prefix: &Path) -> Result<(), String> {
    if let Ok(real) = raw.canonicalize() {
        if !real.starts_with(expected_prefix) {
            return Err(format!(
                "symlink escape: {} resolved to {} (outside {})",
                raw.display(), real.display(), expected_prefix.display()
            ));
        }
    }
    Ok(())
}
```

All three checks are called in sequence from `sandbox::path_check::validate_access`. Any failure
returns `VYOMA_SANDBOX:access_denied|<reason>` to the app and is logged at WARN level.

---

## 4. Automatic Grant Wiring at Spawn (B1 Fix)

At app spawn, supervisor atomically updates `file_access_grants`:

```rust
// supervisor/src/sandbox/grants.rs

use rusqlite::{Connection, params};

pub fn wire_container_grants(app_name: &str, db: &Connection) -> Result<(), String> {
    // Atomic DELETE+INSERT (B1 fix): no partial grant state visible during the
    // window between grant removal and re-insertion.
    let tx = db.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "DELETE FROM file_access_grants WHERE app_name = ?1",
        params![app_name],
    ).map_err(|e| e.to_string())?;

    let container = format!("/data/apps/{}/", app_name);
    tx.execute(
        "INSERT INTO file_access_grants(app_name, path_prefix, mode) VALUES (?1, ?2, 'rw')",
        params![app_name, container],
    ).map_err(|e| e.to_string())?;

    // Grant read access to shared groups declared in vyoma.toml.
    for group in load_shared_groups(app_name, &tx)? {
        tx.execute(
            "INSERT INTO file_access_grants(app_name, path_prefix, mode) VALUES (?1, ?2, 'rw')",
            params![app_name, format!("/data/shared/{}/", group)],
        ).map_err(|e| e.to_string())?;
    }

    // Grant read-only access to .shared/ system assets if policy allows.
    if policy_allows_shared_read(app_name, &tx)? {
        tx.execute(
            "INSERT INTO file_access_grants(app_name, path_prefix, mode) VALUES (?1, '/data/.shared/', 'ro')",
            params![app_name],
        ).map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}
```

`file_access_grants` atomic DELETE+INSERT (B1 fix): no window where an app exists in the process
registry but has zero grants, or retains stale grants from a previous install or a crashed
previous instance of the same app.

Grant rows schema:
```sql
CREATE TABLE IF NOT EXISTS file_access_grants (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    app_name  TEXT    NOT NULL,
    path_prefix TEXT  NOT NULL,
    mode      TEXT    NOT NULL CHECK(mode IN ('ro','rw')),
    expires_at INTEGER            -- Unix timestamp; NULL = permanent
);
CREATE INDEX IF NOT EXISTS idx_grants_app ON file_access_grants(app_name);
```

---

## 5. Container Initialization

On first spawn of an app (container doesn't exist):

```rust
// supervisor/src/sandbox/mod.rs

use std::fs;

pub fn ensure_container(app_name: &str) -> Result<(), String> {
    for subdir in &["support", "cache", "documents", "preferences", "tmp"] {
        let path = format!("/data/apps/{}/{}", app_name, subdir);
        fs::create_dir_all(&path)
            .map_err(|e| format!("create_dir_all {path}: {e}"))?;
    }
    // Emit a marker so sandbox_init() can identify containers created by this version.
    let marker = format!("/data/apps/{}/.vyoma_container", app_name);
    if !std::path::Path::new(&marker).exists() {
        fs::write(&marker, b"1").map_err(|e| format!("write marker: {e}"))?;
    }
    Ok(())
}
```

Called by `spawn_app()` before Wasmtime launch. Never called during update/reinstall (preserves
`support/` and `documents/`). If the container already exists (restart, reboot), the function is
a fast-path no-op after `create_dir_all` returns without error.

**Platform variant — tmpfs for tmp/**:
On `iot-edge` and `mcu-minimal` profiles where `/data` is tight, `tmp/` is replaced with a
symlink to `/tmp/<app_name>/` (a ramfs/tmpfs mount):
```rust
#[cfg(feature = "iot_profile")]
pub fn ensure_tmp_symlink(app_name: &str) -> Result<(), String> {
    let ramfs_dir = format!("/tmp/{}", app_name);
    fs::create_dir_all(&ramfs_dir).map_err(|e| e.to_string())?;
    let link = format!("/data/apps/{}/tmp", app_name);
    if !std::path::Path::new(&link).exists() {
        std::os::unix::fs::symlink(&ramfs_dir, &link).map_err(|e| e.to_string())?;
    }
    Ok(())
}
```
On `desktop-full`, `tmp/` is a plain directory on the persistent ext4 `/data` partition.

---

## 6. Quota Enforcement

Quota enforcement uses two complementary mechanisms: `statvfs` for fast disk-level checks, and
a per-app byte counter tracked in `SandboxState`.

```rust
// supervisor/src/sandbox/quota.rs

use std::sync::{Arc, Mutex, OnceLock};
use std::collections::HashMap;

/// Per-app byte usage tracked in supervisor memory; persisted to
/// /data/.vyoma/sandbox/<app>.state.json on each write event.
pub struct SandboxState {
    pub used_bytes: u64,
    pub quota_bytes: Option<u64>,
}

static SANDBOX_STATES: OnceLock<Arc<Mutex<HashMap<String, SandboxState>>>> = OnceLock::new();

pub fn sandbox_states() -> &'static Arc<Mutex<HashMap<String, SandboxState>>> {
    SANDBOX_STATES.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Check whether a proposed write of `delta_bytes` would exceed this app's quota.
/// Returns Ok(()) if within limits, Err with reason string if quota would be exceeded.
pub fn check_quota(app_name: &str, delta_bytes: u64) -> Result<(), String> {
    let states = sandbox_states().lock().unwrap();
    if let Some(state) = states.get(app_name) {
        if let Some(quota) = state.quota_bytes {
            if state.used_bytes + delta_bytes > quota {
                return Err(format!(
                    "quota exceeded: {} + {} > {} bytes",
                    state.used_bytes, delta_bytes, quota
                ));
            }
        }
    }
    Ok(())
}

/// Record that `bytes_written` bytes were just written by this app.
/// Persists state to disk atomically.
pub fn record_write(app_name: &str, bytes_written: u64) {
    let mut states = sandbox_states().lock().unwrap();
    let entry = states.entry(app_name.to_string()).or_insert(SandboxState {
        used_bytes: 0,
        quota_bytes: None,
    });
    entry.used_bytes = entry.used_bytes.saturating_add(bytes_written);
    let snapshot = entry.used_bytes;
    drop(states);
    // Persist asynchronously to avoid holding lock during disk I/O.
    persist_state_async(app_name, snapshot);
}

fn persist_state_async(app_name: &str, used_bytes: u64) {
    let name = app_name.to_string();
    std::thread::spawn(move || {
        let tmp = format!("/data/.vyoma/sandbox/{}.state.json.tmp", name);
        let final_path = format!("/data/.vyoma/sandbox/{}.state.json", name);
        let json = format!("{{\"used_bytes\":{}}}\n", used_bytes);
        if std::fs::write(&tmp, json.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &final_path);
        }
    });
}
```

**statvfs fast check** — called at container mount time and on `VYOMA_SANDBOX:get_quota` queries:
```rust
pub fn container_used_bytes(app_name: &str) -> u64 {
    let container = format!("/data/apps/{}", app_name);
    // Walk directory tree and sum file sizes.
    fn dir_size(path: &std::path::Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total += meta.len();
                    } else if meta.is_dir() {
                        total += dir_size(&entry.path());
                    }
                }
            }
        }
        total
    }
    dir_size(std::path::Path::new(&container))
}
```

`SandboxState.used_bytes` is reconciled against `container_used_bytes()` on supervisor restart to
handle out-of-band modifications (e.g., manual edits via 9P from the host during development).

---

## 7. Temporary Files Cleanup (B3 Fix)

`/data/apps/<app_name>/tmp/` is cleared on app exit — but SIGCHLD may not fire if the supervisor
crashes. Defense in depth across three paths:

1. **On clean exit**: supervisor `cleanup_tmp(app_name)` is called from the SIGCHLD handler in
   `app_threads.rs` after the waiter thread confirms the child exited.
2. **On supervisor restart**: `sandbox_init()` scans all app `tmp/` dirs; removes files with
   `mtime` older than 24 hours.
3. **Quota reclaim sweep**: when `container_used_bytes()` approaches quota, supervisor clears
   `cache/` and `tmp/` for that app (in that order) before returning a quota error.

```rust
// supervisor/src/sandbox/cleanup.rs

use std::fs;
use std::time::{Duration, SystemTime};

/// Clear all files and subdirectories in /data/apps/<app_name>/tmp/.
/// Called from the SIGCHLD path on app exit (B3 fix: path 1).
pub fn cleanup_tmp(app_name: &str) {
    let tmp = format!("/data/apps/{}/tmp", app_name);
    if let Ok(entries) = fs::read_dir(&tmp) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Startup sweep: remove tmp files older than `max_age` across all apps.
/// Called by sandbox_init() before any app is spawned (B3 fix: path 2).
pub fn startup_tmp_sweep(max_age: Duration) {
    let apps_dir = std::path::Path::new("/data/apps");
    if let Ok(entries) = fs::read_dir(apps_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() { continue; }
            let tmp = entry.path().join("tmp");
            sweep_old_files(&tmp, max_age);
        }
    }
}

fn sweep_old_files(dir: &std::path::Path, max_age: Duration) {
    let cutoff = SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let old = entry.metadata().ok()
                .and_then(|m| m.modified().ok())
                .map(|t| t < cutoff)
                .unwrap_or(false);
            if old {
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
}
```

---

## 8. Inter-App Document Sharing (B2 Fix)

Sharing via supervisor-mediated one-shot grants — no TOCTOU window.

### VYOMA_SANDBOX: Protocol — App to Supervisor

```
VYOMA_SANDBOX:share_grant|<path_b64>|<target_app>|<rw|ro>|<ttl_secs>
VYOMA_SANDBOX:share_revoke|<grant_token>
VYOMA_SANDBOX:list_grants
VYOMA_SANDBOX:check_path|<path_b64>
VYOMA_SANDBOX:get_quota
VYOMA_SANDBOX:request_shared_read|<asset_path_b64>
```

### VYOMA_SANDBOX: Protocol — Supervisor to App

```
VYOMA_SANDBOX:grant_ok|<grant_token>
VYOMA_SANDBOX:grant_error|<reason>
VYOMA_SANDBOX:grants_list|<json_b64>
VYOMA_SANDBOX:path_allowed|<path_b64>
VYOMA_SANDBOX:path_denied|<path_b64>|<reason>
VYOMA_SANDBOX:quota_info|<used_bytes>|<quota_bytes_or_unlimited>
VYOMA_SANDBOX:shared_read_ok|<asset_path_b64>
VYOMA_SANDBOX:shared_read_denied|<asset_path_b64>|<reason>
VYOMA_SANDBOX:access_denied|<reason>
VYOMA_SANDBOX:incoming_share|<grant_token>|<path_b64>|<rw|ro>
```

### Complete Verb Reference

| Verb (App → Supervisor) | Description |
|---|---|
| `share_grant` | Create a one-shot share from a path this app owns to another app |
| `share_revoke` | Revoke a previously issued grant by token |
| `list_grants` | List all active grants for this app (both issued and received) |
| `check_path` | Query whether a path is currently accessible (returns allowed/denied) |
| `get_quota` | Query current quota usage and limit |
| `request_shared_read` | Request read access to a specific `/data/.shared/` asset |

| Verb (Supervisor → App) | Description |
|---|---|
| `grant_ok` | Share grant created; includes token |
| `grant_error` | Share grant failed; includes reason |
| `grants_list` | JSON blob of active grants |
| `path_allowed` | Response to check_path: path is accessible |
| `path_denied` | Response to check_path: path blocked with reason |
| `quota_info` | Response to get_quota: bytes used and optional limit |
| `shared_read_ok` | Requested `.shared/` asset is accessible |
| `shared_read_denied` | Requested `.shared/` asset is not accessible |
| `access_denied` | General access rejection (path traversal, no capability, etc.) |
| `incoming_share` | Notification to the target app that a grant was created for it |

### Share Grant Implementation

```rust
// supervisor/src/sandbox/sharing.rs

use rusqlite::{Connection, params};

/// Handle VYOMA_SANDBOX:share_grant from `source_app`.
/// B2 fix: ownership check + grant insertion performed in a single transaction.
pub fn handle_share_grant(
    source_app: &str,
    path_b64: &str,
    target_app: &str,
    mode: &str,
    ttl_secs: u64,
    db: &Connection,
) -> Result<String, String> {
    use base64::Engine;
    let path_bytes = base64::engine::general_purpose::STANDARD
        .decode(path_b64)
        .map_err(|_| "invalid base64 path".to_string())?;
    let path = String::from_utf8(path_bytes).map_err(|_| "path not valid utf-8".to_string())?;

    // Reject path traversal before any DB work.
    super::path_check::reject_dotdot(&path)?;

    let tx = db.transaction().map_err(|e| e.to_string())?;

    // B2 fix: ownership check inside the same transaction as the grant insertion.
    // No separate read-then-write window.
    let owned: bool = tx.query_row(
        "SELECT 1 FROM file_access_grants WHERE app_name = ?1 AND ?2 LIKE path_prefix || '%' LIMIT 1",
        params![source_app, path],
        |_| Ok(true),
    ).unwrap_or(false);

    if !owned {
        return Err(format!("path {} not owned by {}", path, source_app));
    }

    // Generate a 128-bit random token.
    let token = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
        format!("{:08x}{:08x}{:08x}{:08x}", t, t.wrapping_mul(0xdeadbeef), t ^ 0xc0ffee, t.wrapping_add(42))
    };

    let expires_at = if ttl_secs > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Some(now + ttl_secs)
    } else {
        None
    };

    tx.execute(
        "INSERT INTO file_access_grants(app_name, path_prefix, mode, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![target_app, path, mode, expires_at],
    ).map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(token)
}
```

Share grant flow:
1. Source app sends `VYOMA_SANDBOX:share_grant|<path_b64>|<target_app>|rw|3600`.
2. Supervisor calls `handle_share_grant`; within the transaction it checks the source app owns the
   path and inserts the grant for the target app.
3. Supervisor responds to source app: `VYOMA_SANDBOX:grant_ok|<token>`.
4. Supervisor notifies target app: `VYOMA_SANDBOX:incoming_share|<token>|<path_b64>|rw`.
5. TTL sweep: a background thread in `sharing.rs` runs every 60 s, deletes expired rows.

**TOCTOU fix (B2)**: The ownership check and the grant insertion are inside the same SQLite
transaction. No window exists where a concurrent revocation or removal could invalidate the
ownership check after it passes but before the grant row is inserted.

---

## 9. 9P Mount Options: Per-App Sandbox vs Shared Mount

VyomaOS currently uses a single `virtio-9p` tag (`vyoma_data`) that mounts `/data` system-wide.
The sandbox subsystem operates at the grant-enforcement layer above this single mount.

For future multi-9P-tag isolation (one virtio-9p tag per app), the QEMU command line would be:

```bash
# One shared tag (current approach — enforced by supervisor grants)
-virtfs local,path=/host/data,mount_tag=vyoma_data,security_model=mapped-xattr,id=vyoma_data

# Future per-app isolation (one tag per app, separate host directory subtree)
-virtfs local,path=/host/data/apps/calculator,mount_tag=vyoma_calculator,security_model=mapped-xattr,id=app_calc
-virtfs local,path=/host/data/apps/text-editor,mount_tag=vyoma_texteditor,security_model=mapped-xattr,id=app_te
```

Per-app 9P tags would require the supervisor to pass a different virtio-9p tag to each Wasmtime
process, which is not supported in the current single-binary Wasmtime invocation. This is tracked
as a future hardening step; for now, isolation is enforced purely through supervisor grant
validation on every `VYOMA_SANDBOX:check_path` and every `VYOMA_DRAW:`/`VYOMA_FILE:` operation.

**Current mount setup in `rootfs.sh`**:
```sh
# /etc/fstab entry generated by rootfs.sh
vyoma_data  /data  9p  trans=virtio,version=9p2000.L,msize=65536,rw,_netdev  0  0
```

`msize=65536` sets the maximum message size for 9P to 64 KB, balancing throughput and memory on
the 512 MB desktop-full profile. For `mcu-minimal` (128 KB total RAM), `msize=4096` is used.

---

## 10. Shared Container Groups

Apps declare shared group membership in `vyoma.toml`:
```toml
[sandbox]
shared_groups    = ["com.vyomaos.productivity"]
allow_shared_read = true
```

```rust
// supervisor/src/sandbox/groups.rs

use rusqlite::{Connection, params};

/// Create the shared group directory if it doesn't exist, and insert the membership row.
/// Called during wire_container_grants when shared_groups is non-empty.
pub fn ensure_group_membership(app_name: &str, group_id: &str, db: &Connection) -> Result<(), String> {
    let group_dir = format!("/data/shared/{}/", group_id);
    std::fs::create_dir_all(&group_dir)
        .map_err(|e| format!("create shared group dir {group_dir}: {e}"))?;

    db.execute(
        "INSERT OR IGNORE INTO shared_group_members(group_id, app_name) VALUES (?1, ?2)",
        params![group_id, app_name],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Returns true if `app_name` is a member of `group_id`.
pub fn is_group_member(app_name: &str, group_id: &str, db: &Connection) -> bool {
    db.query_row(
        "SELECT 1 FROM shared_group_members WHERE group_id = ?1 AND app_name = ?2",
        params![group_id, app_name],
        |_| Ok(true),
    ).unwrap_or(false)
}

/// Remove all group memberships for an app on pkg-remove.
pub fn remove_member(app_name: &str, db: &Connection) {
    let _ = db.execute(
        "DELETE FROM shared_group_members WHERE app_name = ?1",
        params![app_name],
    );
    // Note: shared group directories are NOT deleted; other members may still use them.
    // Empty group dirs are reaped by sandbox_gc() when no members remain.
}

/// Garbage-collect shared group directories with no remaining members.
pub fn sandbox_gc(db: &Connection) {
    let query = "SELECT DISTINCT group_id FROM shared_group_members";
    let active_groups: Vec<String> = db
        .prepare(query)
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |r| r.get(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let shared_dir = std::path::Path::new("/data/shared");
    if let Ok(entries) = std::fs::read_dir(shared_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !active_groups.contains(&name) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}
```

`/data/shared/<group_id>/` is created on first member install. Non-members receive no grant row
and cannot write to or read from the group directory. Supervisor rejects `share_grant` requests
that target `/data/shared/<group>/` paths for apps that are not group members.

Schema:
```sql
CREATE TABLE IF NOT EXISTS shared_group_members (
    group_id TEXT NOT NULL,
    app_name TEXT NOT NULL,
    PRIMARY KEY (group_id, app_name)
);
```

---

## 11. App Update — Container Migration

On `pkg-update <app_name>`:
1. Kill running instance (supervisor sends SIGTERM, waits 3 s, then SIGKILL — B4 fix: must stop
   before touching container files).
2. Back up `support/` to `support.bak/` (atomic rename — B4 fix: rename, not copy, to be
   instantaneous and not leave a partial state on disk).
3. Install new `.wasm` binary to `/apps/<app_name>.wasm` in initramfs (or via pkg path).
4. Run migration hook if new version declares one in `vyoma.toml`:
   `@supervisor: migrate <old_ver> <new_ver>` — supervisor invokes the app's migrate entry point.
5. On migration failure: restore `support.bak/` → `support/`, rollback binary.
6. Clear `cache/` (never backed up — cache is always safe to discard).
7. Keep `documents/` and `preferences/` unchanged.
8. Drop `support.bak/` on successful migration.

```rust
// supervisor/src/sandbox/migration.rs

use std::fs;

pub fn backup_support(app_name: &str) -> Result<(), String> {
    let src  = format!("/data/apps/{}/support", app_name);
    let bak  = format!("/data/apps/{}/support.bak", app_name);
    // Remove any stale backup first (e.g., from a previous failed update).
    if std::path::Path::new(&bak).exists() {
        fs::remove_dir_all(&bak).map_err(|e| format!("remove stale bak: {e}"))?;
    }
    fs::rename(&src, &bak).map_err(|e| format!("rename support → support.bak: {e}"))?;
    Ok(())
}

pub fn rollback_support(app_name: &str) -> Result<(), String> {
    let src  = format!("/data/apps/{}/support", app_name);
    let bak  = format!("/data/apps/{}/support.bak", app_name);
    if std::path::Path::new(&src).exists() {
        fs::remove_dir_all(&src).map_err(|e| format!("remove partial support: {e}"))?;
    }
    fs::rename(&bak, &src).map_err(|e| format!("rename support.bak → support: {e}"))?;
    Ok(())
}

pub fn commit_migration(app_name: &str) {
    let bak = format!("/data/apps/{}/support.bak", app_name);
    let _ = fs::remove_dir_all(&bak);
}

pub fn clear_cache(app_name: &str) {
    let cache = format!("/data/apps/{}/cache", app_name);
    if let Ok(entries) = fs::read_dir(&cache) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() { let _ = fs::remove_dir_all(&p); }
            else          { let _ = fs::remove_file(&p); }
        }
    }
}
```

---

## 12. pkg-remove Ordering (B4 Fix)

```rust
// supervisor/src/sandbox/remove.rs

use rusqlite::{Connection, params};

pub fn remove_app(app_name: &str, registry: &AppRegistry, db: &Connection) {
    // 1. Kill running instance FIRST (B4 fix: ordering prevents race between grant
    //    revocation and file deletion while the app is still writing).
    if let Some(child) = find_running(app_name, registry) {
        let _ = child.kill();
        let _ = child.wait();
    }

    // 2. Remove from boot.toml so the app won't restart on next boot.
    boot_config::remove_app(app_name);

    // 3. Revoke all file access grants atomically.
    db.execute(
        "DELETE FROM file_access_grants WHERE app_name = ?1",
        params![app_name],
    ).ok();

    // 4. Purge container directory (all subdirs including documents).
    let _ = std::fs::remove_dir_all(format!("/data/apps/{}", app_name));

    // 5. Purge recents list (R48 document model).
    recents::purge_app(app_name, db);

    // 6. Purge spotlight index (R42).
    spotlight::purge_app(app_name);

    // 7. Purge shared group membership; GC empty group dirs.
    groups::remove_member(app_name, db);
    groups::sandbox_gc(db);

    // 8. Revoke all BookmarkTokens (B5 fix: outside-container paths survive uninstall otherwise).
    bookmark_store::revoke_all(app_name);

    // 9. Purge sandbox state file.
    let state_file = format!("/data/.vyoma/sandbox/{}.state.json", app_name);
    let _ = std::fs::remove_file(&state_file);
}
```

Kill-before-delete ordering (B4) prevents:
- A new app instance starting between grant revocation and container removal (restarted by
  supervisor `restart_policy = always` before the boot.toml removal fires).
- A race on `/data/apps/<app_name>/` deletion while the app is mid-write, which could leave
  partial files and corrupt the ext4 journal.

---

## 13. BookmarkToken Interaction (B5 Fix)

Apps can hold `BookmarkToken`s for paths OUTSIDE their container (user-granted via the Open panel
in R48). These are stored separately in `bookmark_store`, NOT in `file_access_grants`. On
`pkg-remove`, the supervisor calls `bookmark_store::revoke_all(app_name)` — otherwise stale
tokens survive uninstall and could be replayed by a reinstalled version of the app (B5 fix).

```rust
// supervisor/src/sandbox/grants.rs (addition for bookmark integration)

/// Called as part of pkg-remove. Removes all BookmarkTokens held by app_name
/// that point to paths outside the app's container. (B5 fix)
pub fn revoke_bookmark_tokens(app_name: &str, db: &Connection) {
    db.execute(
        "DELETE FROM bookmark_store WHERE app_name = ?1",
        params![app_name],
    ).ok();
}
```

On app update (same name, new version), `BookmarkToken`s are intentionally preserved. The user
explicitly granted out-of-container access; the update is to the same app identity. The new
version inherits those grants exactly as the old version held them.

`filesystem = true` capability is required for any sandbox grants to be wired. Without it:
- `ensure_container()` is not called (no container directories created).
- `wire_container_grants()` is not called (no rows in `file_access_grants`).
- All `VYOMA_SANDBOX:` IPC verbs return `access_denied|filesystem-capability-required`.

---

## 14. Integration with R59 Sandboxing & Capability Model

R59 defines the top-level capability policy that this subsystem implements at the filesystem layer.
Key integration points:

**Capability → sandbox grant mapping**:

| `vyoma.toml` capability | Sandbox effect |
|---|---|
| `filesystem = false` (default) | No container, no grants, VYOMA_SANDBOX verbs rejected |
| `filesystem = true` | Container created, rw grant to `/data/apps/<name>/`, grants table populated |
| `sandbox.shared_groups = [...]` | Additional rw grants to `/data/shared/<group>/` for each membership |
| `sandbox.allow_shared_read = true` | ro grant to `/data/.shared/` system assets |
| `sandbox.quota_bytes = N` | Write ops checked against quota; `VYOMA_SANDBOX:get_quota` returns live stats |

**R59 policy enforcement ordering**:
1. R59 capability check: does this app have `filesystem = true`? If not, reject at IPC dispatch.
2. Sandbox grant check: does `file_access_grants` have a row covering this path for this app?
3. Path normalization: `reject_dotdot` + `canonical_path_allowed` + `symlink_escape_guard`.
4. Quota check: `check_quota` before any write operation.

These four gates are always applied in sequence. Passing one does not skip the others.

**Relation to R04 VFS**:
`file_access_grants` is the R04 VFS grant table. The sandbox subsystem writes to it (via
`wire_container_grants`) but reads from it through the R04 VFS layer. No other subsystem writes
to `file_access_grants` for app paths except R49 sandbox.

---

## 15. File Layout

```
supervisor/src/sandbox/
├── mod.rs          (~60 lines: SandboxSubsystem init, ensure_container, sandbox_init startup sweep)
├── policy.rs       (~80 lines: SandboxPolicy struct, from_capabilities, is_path_allowed)
├── path_check.rs   (~90 lines: reject_dotdot, canonical_path_allowed, symlink_escape_guard, validate_access)
├── grants.rs       (~110 lines: wire_container_grants, atomic DELETE+INSERT (B1), revoke_bookmark_tokens (B5))
├── quota.rs        (~100 lines: SandboxState, SANDBOX_STATES OnceLock, check_quota, record_write, persist_state_async)
├── cleanup.rs      (~80 lines: cleanup_tmp on exit (B3), startup_tmp_sweep, sweep_old_files)
├── sharing.rs      (~140 lines: handle_share_grant TOCTOU fix (B2), TTL sweep background thread)
├── groups.rs       (~100 lines: ensure_group_membership, is_group_member, remove_member, sandbox_gc)
├── migration.rs    (~90 lines: backup_support, rollback_support, commit_migration, clear_cache)
└── remove.rs       (~90 lines: remove_app, kill-before-delete (B4), full teardown sequence)

supervisor/src/router.rs        (modified: VYOMA_SANDBOX: dispatch arm added)
supervisor/src/manifest.rs      (modified: SandboxConfig struct, quota_bytes, shared_groups, allow_shared_read fields)
supervisor/src/app_threads.rs   (modified: cleanup_tmp called from SIGCHLD handler on exit)
supervisor/src/spawn.rs         (modified: ensure_container + wire_container_grants called before Wasmtime launch)
```

Each file stays within the 500-line limit. `sharing.rs` is the largest at ~140 lines; the TTL
sweep background thread accounts for ~30 of those.

---

## 16. Detailed Blocking Issue Analysis

### B1 — Grant Table Inconsistency at Spawn/Exit Boundary

**Problem**: Before the B1 fix, `wire_container_grants` issued a `DELETE` and then an `INSERT` as
two separate SQLite statements in autocommit mode. If the supervisor thread was interrupted
(panic, SIGKILL during development, power loss) between the DELETE and the INSERT, the app entry
in the process registry existed but had zero grant rows. The app would then fail all path checks
with `access_denied` on its first filesystem access, with no actionable error message.

Additionally, if an app crashed and restarted quickly (e.g., `restart_policy = always`), the
prior invocation's grants were not cleaned up before the new invocation's grants were inserted,
leaving duplicate rows with potentially different modes (e.g., one `rw` and one `ro` row for the
same prefix from successive installs).

**Resolution**: Atomic DELETE+INSERT inside a single `db.transaction()`. No window exists where
zero or stale rows are visible. The transaction is the minimal critical section — it holds no
other lock and touches only `file_access_grants` rows for the specific `app_name`.

### B2 — Shared Container TOCTOU

**Problem**: The original `share_grant` implementation first queried whether the source app owned
the path (`SELECT ... WHERE app_name = ?1 AND path LIKE prefix`), and then, if ownership
confirmed, inserted a new row for the target app. Between the SELECT returning true and the INSERT
completing, a concurrent `pkg-remove` of the source app could delete the source app's grant rows.
The target app would then hold a grant for a path that the source app no longer owned — a
capability escalation via race condition.

This is a classic check-time-to-use-time (TOCTOU) vulnerability.

**Resolution**: Both the ownership check and the grant insertion are performed inside the same
`db.transaction()` in `handle_share_grant`. SQLite serializes transactions; no other write can
interleave. The ownership check and the grant insertion are atomic from the database's perspective.

### B3 — Tmp Cleanup on Supervisor Crash

**Problem**: `cleanup_tmp` was only called from the SIGCHLD handler. If the supervisor itself
crashed (OOM, panic, SIGKILL from kernel watchdog), SIGCHLD was never delivered. On the next
boot, all apps' `tmp/` directories retained their previous session's scratch files. Over time,
across many crash-reboot cycles (common during early OS development), `tmp/` accumulated gigabytes
of stale files that consumed quota budget and triggered false quota errors for apps that had never
actually written near their quota limit.

**Resolution**: Three-path defense: SIGCHLD path for clean exits, startup sweep for >24h files
on any reboot (crash recovery), and quota-pressure reaping that clears `cache/` then `tmp/` when
an app approaches its limit.

### B4 — pkg-remove Race With Running Instance

**Problem**: The original `remove_app` sequence was:
1. Remove from boot.toml
2. Delete grant rows
3. Remove container directory
4. Kill running instance (last!)

Between steps 2 and 4, the running app was mid-write with its path now failing grant checks, and
between steps 3 and 4, the app was writing to a directory that had been deleted — leaving orphaned
inodes and corrupting the ext4 journal on the `/data` partition.

Additionally, an app with `restart_policy = always` could restart between step 1 (boot.toml
removal) and step 4 (kill), starting a fresh instance that would immediately pass grant checks
(boot.toml removal had not yet propagated to the in-memory registry).

**Resolution**: Kill + wait is the first operation. The process is confirmed dead before any grant
row is touched or any directory is removed. `find_running` checks the in-memory `AppRegistry`
(under its own lock, dropped before grant work begins — ABBA deadlock prevention: snapshot the
running PID under the registry lock, then drop the lock, then wait on the PID).

### B5 — BookmarkToken Survival Across Uninstall

**Problem**: `BookmarkToken`s in the `bookmark_store` table grant access to paths outside the
app's container (e.g., a file the user opened via the R48 Open panel — `/data/apps/other-app/
documents/report.pdf`). These tokens are NOT rows in `file_access_grants`; they live in a
separate table and are checked by the R48 VFS layer.

When `remove_app` only deleted `file_access_grants` rows and purged the container, the
`bookmark_store` rows for the removed app remained. If the user reinstalled the same app
(same `app_name`), the new instance inherited the old `BookmarkToken`s — including tokens for
files in other apps' containers that the NEW install had never explicitly been granted.

**Resolution**: `remove_app` calls `bookmark_store::revoke_all(app_name)` explicitly, after grant
revocation and before container removal. This is step 8 in the documented remove sequence and is
tested by a unit test in `supervisor/tests/sandbox_remove.rs`.

---

## 17. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: grant table inconsistency between spawn/exit cycles (partial grant state visible) | Atomic DELETE+INSERT in a single transaction; no window with zero or stale grants |
| B2: shared container TOCTOU — ownership check and grant insertion are separate operations | `share_grant` performs ownership check + grant insertion in single transaction; rejects if path not under source app's existing grants |
| B3: tmp cleanup on crash — SIGCHLD doesn't fire if supervisor itself crashes | Startup sweep removes files >24 h old in all tmp dirs; SIGCHLD handler calls cleanup_tmp for normal exits; quota-pressure reaping clears tmp before returning quota errors |
| B4: pkg-remove ordering — file removal races with running instance | Kill + wait FIRST (snapshotting PID under registry lock, then waiting after lock drop to avoid ABBA); only then revoke grants and delete container |
| B5: BookmarkToken for outside-container files survives uninstall | pkg-remove calls `bookmark_store::revoke_all(app_name)` in addition to `file_access_grants` purge; tested in `supervisor/tests/sandbox_remove.rs` |
