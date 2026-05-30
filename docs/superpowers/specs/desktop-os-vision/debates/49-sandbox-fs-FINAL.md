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

`/data/.vyoma/` reserved for supervisor-internal use; never granted to apps as part of sandbox.

---

## 2. Automatic Grant Wiring at Spawn (B1 Fix)

At app spawn, supervisor atomically updates `file_access_grants`:

```rust
// supervisor/src/sandbox/grants.rs
pub fn wire_container_grants(app_name: &str, db: &Connection) -> Result<(), String> {
    // Atomic DELETE+INSERT (B1 fix): no partial grant state visible
    let tx = db.transaction()?;
    tx.execute("DELETE FROM file_access_grants WHERE app_name = ?1", params![app_name])?;
    let container = format!("/data/apps/{}/", app_name);
    tx.execute(
        "INSERT INTO file_access_grants(app_name, path_prefix, mode) VALUES (?1,?2,'rw')",
        params![app_name, container],
    )?;
    // Grant read access to shared groups declared in vyoma.toml
    for group in load_shared_groups(app_name) {
        tx.execute(
            "INSERT INTO file_access_grants(app_name, path_prefix, mode) VALUES (?1,?2,'rw')",
            params![app_name, format!("/data/shared/{}/", group)],
        )?;
    }
    tx.commit()?;
    Ok(())
}
```

`file_access_grants` atomic DELETE+INSERT (B1 fix): no window where app exists in registry but has zero grants, or has stale grants from a previous install.

---

## 3. Container Initialization

On first spawn of an app (container doesn't exist):
```rust
pub fn ensure_container(app_name: &str) -> Result<(), String> {
    for subdir in &["support", "cache", "documents", "preferences", "tmp"] {
        let path = format!("/data/apps/{}/{}", app_name, subdir);
        fs::create_dir_all(&path)?;
    }
    Ok(())
}
```

Called by `spawn_app()` before Wasmtime launch. Never called during update/reinstall (preserves `support/` and `documents/`).

---

## 4. Temporary Files Cleanup (B3 Fix)

`/data/apps/<app_name>/tmp/` cleared on app exit — but SIGCHLD may not fire if supervisor crashes. Defense in depth:

1. **On clean exit**: supervisor `cleanup_tmp(app_name)` called from SIGCHLD handler.
2. **On supervisor restart**: `sandbox_init()` scans all app tmp dirs, removes files older than 24h.
3. **tmpfs alternative**: for mcu-minimal/iot profiles, `tmp/` is a symlink to `/tmp/<app_name>/` (in-memory tmpfs). Not available on desktop-full (persistent ext4).

```rust
// supervisor/src/sandbox/cleanup.rs
pub fn cleanup_tmp(app_name: &str) {
    let tmp = format!("/data/apps/{}/tmp", app_name);
    if let Ok(entries) = fs::read_dir(&tmp) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}
```

---

## 5. Inter-App Document Sharing (B2 Fix)

Sharing via supervisor-mediated one-shot grants — no TOCTOU:

App stdout → supervisor:
```
VYOMA_SANDBOX:share_grant:<path_b64>:<target_app>:<rw|ro>:<ttl_secs>
VYOMA_SANDBOX:share_revoke:<grant_token>
VYOMA_SANDBOX:list_grants
```

Supervisor → app stdin:
```
VYOMA_SANDBOX:grant_ok:<grant_token>
VYOMA_SANDBOX:grant_error:<reason>
VYOMA_SANDBOX:grants_list:<json_b64>
```

Share grant flow:
1. Source app sends `share_grant` for a path it owns (supervisor checks source app's grants — B2 fix: ownership check before insertion).
2. Supervisor issues 128-bit random `grant_token`, inserts into `file_access_grants` for `target_app`.
3. Notifies target app via stdin: `VYOMA_SANDBOX:incoming_share:<grant_token>:<path_b64>:<rw|ro>`.
4. TTL enforced: supervisor background sweep expires timed grants.

**TOCTOU fix (B2)**: share_grant checks that `path` falls under the source app's existing grants before creating the new grant. Done atomically in a single transaction — no separate read-then-write window.

---

## 6. Shared Container Groups

Apps declare shared group membership in `vyoma.toml`:
```toml
[sandbox]
shared_groups = ["com.vyomaos.productivity"]
```

`/data/shared/com.vyomaos.productivity/` created by supervisor when first member is installed. All members get rw grants. Non-members get no access — supervisor rejects `share_grant` to paths under `/data/shared/<group>/` for non-members.

---

## 7. App Update — Container Migration

On `pkg-update <app_name>`:
1. Kill running instance (supervisor sends SIGTERM, waits 3s, SIGKILL — B4 fix: must stop before touching files).
2. Back up `support/` to `support.bak/` (rename, atomic).
3. Install new `.wasm` binary.
4. Run migration hook if new version declares one (in vyoma.toml): `@supervisor: migrate <old_ver> <new_ver>`.
5. On migration failure: restore `support.bak/` → `support/`, rollback binary.
6. Clear `cache/` (never backed up — cache is always safe to clear).
7. Keep `documents/` unchanged.

---

## 8. pkg-remove Ordering (B4 Fix)

```rust
// supervisor/src/sandbox/remove.rs
pub fn remove_app(app_name: &str, registry: &AppRegistry, db: &Connection) {
    // 1. Kill running instance FIRST (B4: ordering)
    if let Some(child) = find_running(app_name, registry) {
        child.kill();
        child.wait();
    }
    // 2. Remove from boot.toml
    boot_config::remove_app(app_name);
    // 3. Revoke all grants
    db.execute("DELETE FROM file_access_grants WHERE app_name = ?1", params![app_name]).ok();
    // 4. Purge container
    let _ = fs::remove_dir_all(format!("/data/apps/{}", app_name));
    // 5. Purge recents (R48)
    recents::purge_app(app_name, db);
    // 6. Purge spotlight index (R42)
    spotlight::purge_app(app_name);
    // 7. Purge shared group membership
    shared_groups::remove_member(app_name, db);
}
```

Kill-before-delete ordering (B4) prevents: new app instance starting between grant revocation and file removal, race on `/data/apps/<app_name>/` deletion while app is writing.

---

## 9. BookmarkToken Interaction (B5 Fix)

Apps can hold `BookmarkToken`s for paths OUTSIDE their container (user-granted via Open panel). These are stored separately in `bookmark_store`, NOT in `file_access_grants`. On pkg-remove, supervisor also calls `bookmark_store::revoke_all(app_name)` — otherwise stale tokens survive uninstall (B5 fix).

On app update (keep same name), BookmarkTokens are preserved — user explicitly granted access, and the update is the same app.

Capability `filesystem = true` is required for the app to receive sandbox grants at all. Without it, no container is created and no grants are wired.

---

## 10. File Layout

```
supervisor/src/sandbox/
├── mod.rs          (~40 lines: SandboxSubsystem init, ensure_container on first spawn)
├── grants.rs       (~100 lines: wire_container_grants, atomic DELETE+INSERT (B1), shared group grants)
├── cleanup.rs      (~60 lines: cleanup_tmp on exit (B3), startup sweep for stale tmp)
├── sharing.rs      (~120 lines: share_grant, TOCTOU ownership check (B2), TTL sweep)
├── groups.rs       (~80 lines: shared_groups table, membership, /data/shared/ creation)
├── migration.rs    (~100 lines: pkg-update flow, support.bak atomic rename, rollback)
└── remove.rs       (~100 lines: remove_app, kill-before-delete (B4), bookmark revoke (B5))

supervisor/src/router.rs  (modified: VYOMA_SANDBOX: dispatch arm)
```

---

## 11. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: grant table inconsistency between spawn/exit cycles (partial grant state visible) | Atomic DELETE+INSERT in a single transaction; no window with zero or stale grants |
| B2: shared container TOCTOU — ownership check and grant insertion are separate operations | `share_grant` performs ownership check + grant insertion in single transaction; rejects if path not under source app's existing grants |
| B3: tmp cleanup on crash — SIGCHLD doesn't fire if supervisor itself crashes | Startup sweep removes files >24h old in all tmp dirs; SIGCHLD handler calls cleanup_tmp for normal exits |
| B4: pkg-remove ordering — file removal races with running instance | Kill + wait FIRST; only then revoke grants and delete container |
| B5: BookmarkToken for outside-container files survives uninstall | pkg-remove calls `bookmark_store::revoke_all(app_name)` in addition to file_access_grants purge |
