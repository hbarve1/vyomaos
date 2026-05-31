# FINAL Spec: Package Manager & App Store (Round 50)

**Subsystem**: Package Manager & App Store  
**macOS Analogue**: `Mac App Store` / Homebrew equivalent  
**Depends on**: R22 (App Lifecycle), R42 (Spotlight — purge on remove), R48 (recents — purge on remove), R49 (container FS — container migration/pruning)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Package Format — `.vyomapkg`

Tar+zst archive (fast decompression, no external tool beyond `zstd` + `tar` in initramfs):

```
my-app-1.2.0.vyomapkg
├── META-INF/
│   ├── manifest.json      # package metadata (see below)
│   └── signature.ed25519  # ed25519 signature over SHA-256 of archive minus this file
├── app/
│   ├── vyoma.toml         # app manifest (capabilities declaration)
│   └── my-app.wasm        # wasm32-wasip2 binary
└── assets/
    ├── icon.png            # 128×128 PNG app icon
    └── ...                 # other read-only assets
```

`META-INF/manifest.json`:
```json
{
  "bundle_id": "com.vendor.my-app",
  "display_name": "My App",
  "version": "1.2.0",
  "min_vyoma_version": "0.49",
  "sha256_wasm": "abc123...",
  "signature_key_id": "vyoma-official-2026",
  "required_capabilities": ["stdio", "display", "filesystem"]
}
```

---

## 2. Package Registry (`packages.db`)

SQLite at `/data/.vyoma/packages.db` (WAL mode; replaces `/data/installed.txt`):

```sql
CREATE TABLE packages (
    bundle_id      TEXT PRIMARY KEY,
    app_name       TEXT NOT NULL UNIQUE,   -- matches vyoma.toml [app] name
    display_name   TEXT NOT NULL,
    version        TEXT NOT NULL,
    install_date   INTEGER NOT NULL,       -- unix timestamp
    install_source TEXT NOT NULL,          -- 'store' | 'sideload' | 'builtin'
    wasm_path      TEXT NOT NULL,          -- absolute path to .wasm in /data/apps/<name>/
    icon_path      TEXT,
    capabilities   TEXT NOT NULL           -- JSON array of declared capabilities
);
CREATE TABLE install_history (
    bundle_id      TEXT NOT NULL,
    version        TEXT NOT NULL,
    action         TEXT NOT NULL,          -- 'install' | 'update' | 'remove'
    timestamp      INTEGER NOT NULL
);
```

Migration from `/data/installed.txt`: on first boot after R50, `packages_init()` reads the old txt file and populates `packages.db`, then renames `installed.txt` to `installed.txt.migrated`.

---

## 3. Install Flow (B1 Fix — Atomic Install)

```rust
// supervisor/src/package/install.rs  (~200 lines)
pub fn install_package(pkg_path: &str, source: &str) -> Result<(), String> {
    // 1. Extract to staging area first (B1: never touch live app dir during extract)
    let staging = format!("/data/.vyoma/staging/{}", random_hex(8));
    fs::create_dir_all(&staging)?;
    extract_vyomapkg(pkg_path, &staging)?;  // zstd+tar extract

    // 2. Verify signature (B4 fix: ed25519 via ring crate, no openssl)
    verify_signature(&staging)?;

    // 3. Verify wasm SHA-256 matches manifest.json sha256_wasm
    verify_wasm_hash(&staging)?;

    // 4. Read app name from staging/app/vyoma.toml
    let app_name = read_app_name(&staging)?;

    // 5. Check capability review (present capability diff to user if significant)
    let caps = read_capabilities(&staging)?;

    // 6. Atomic rename: staging → /data/apps/<name>/ (B1: rename is atomic)
    let dest = format!("/data/apps/{}", app_name);
    if Path::new(&dest).exists() {
        // Update path: backup old binary only
        let backup = format!("{}.prev", dest);
        fs::rename(&dest, &backup)?;     // atomic rename of old
        match fs::rename(&staging, &dest) {
            Ok(_) => { let _ = fs::remove_dir_all(&backup); }
            Err(e) => {
                // Rollback (B5 fix)
                fs::rename(&backup, &dest)?;
                return Err(format!("install failed, rolled back: {e}"));
            }
        }
    } else {
        fs::rename(&staging, &dest)?;
    }

    // 7. Register in packages.db (after rename — if this crashes, staging was cleaned up)
    packages_db::register(&app_name, &caps, source)?;

    // 8. Add to boot.toml (B2 fix: serialized through boot_config mutex)
    boot_config::add_app(&app_name)?;

    // 9. Set quarantine xattr (R47: vyoma.quarantine = "downloaded")
    meta_store::set_xattr(&dest, "vyoma", "quarantine", b"downloaded")?;

    Ok(())
}
```

**Install atomicity (B1 fix)**: all extraction goes to `/data/.vyoma/staging/` first. The `fs::rename` of the staging dir to `/data/apps/<name>/` is the single atomic commit point. If anything before the rename fails (signature check, hash check, disk full), the staging dir is cleaned up and no live app is touched.

---

## 4. Update Flow with Rollback (B5 Fix)

```rust
// supervisor/src/package/update.rs  (~160 lines)
pub fn update_package(app_name: &str, new_pkg: &str) -> Result<(), String> {
    // 1. Kill running instance (B3 fix: stop before any file mutation)
    lifecycle::stop_app_sync(app_name, Duration::from_secs(5))?;

    // 2. Create rollback snapshot: hard-link old wasm into /data/.vyoma/rollback/<name>/
    let rollback_dir = format!("/data/.vyoma/rollback/{}", app_name);
    snapshot_for_rollback(app_name, &rollback_dir)?;

    // 3. Install new version (atomic rename path in install_package)
    install_package(new_pkg, "update")?;

    // 4. Restart app; monitor for crash within first 30s (B5 fix: health check)
    lifecycle::start_app(app_name)?;
    match health_check(app_name, Duration::from_secs(30)) {
        Ok(_) => {
            // Success: clean up rollback snapshot
            let _ = fs::remove_dir_all(&rollback_dir);
        }
        Err(e) => {
            // New version crashed on first launch — rollback (B5 fix)
            log_warn!("update-rollback: {} crashed within 30s: {}", app_name, e);
            lifecycle::stop_app_sync(app_name, Duration::from_secs(3)).ok();
            restore_rollback(app_name, &rollback_dir)?;
            packages_db::revert_version(app_name)?;
            lifecycle::start_app(app_name)?;
        }
    }
    Ok(())
}
```

Health check: app is considered healthy if it produces at least one stdout line (including a `VYOMA_DRAW:flush` or any IPC message) within 30 seconds of spawn. If it exits or is silent for 30s, rollback triggers.

---

## 5. Uninstall Flow — pkg-remove Ordering (B3 Fix)

```rust
// supervisor/src/package/remove.rs  (~120 lines)
pub fn remove_package(app_name: &str) -> Result<(), String> {
    // 1. Kill running instance FIRST — must complete before ANY file changes (B3 fix)
    lifecycle::stop_app_sync(app_name, Duration::from_secs(5))?;

    // 2. Remove from boot.toml (B2 fix: serialized through mutex)
    boot_config::remove_app(app_name)?;

    // 3. Revoke all file grants
    grants::revoke_all(app_name)?;

    // 4. Purge container (R49: preserves user data option)
    sandbox::container_prune(app_name, /*keep_user_data=*/false)?;

    // 5. Remove wasm + assets
    let _ = fs::remove_dir_all(format!("/data/apps/{}", app_name));

    // 6. Purge recents (R48)
    recents::purge_app(app_name)?;

    // 7. Purge spotlight index (R42)
    spotlight::purge_app(app_name)?;

    // 8. Remove from packages.db
    packages_db::deregister(app_name)?;

    // 9. Log install_history entry
    packages_db::log_action(app_name, "remove")?;

    Ok(())
}
```

**Kill-before-file-mutation (B3 fix)**: `stop_app_sync` sends SIGTERM, waits up to 5s, then SIGKILL. Only after the child is confirmed dead (waiter thread signals via `oneshot`) does any file mutation proceed.

---

## 6. boot.toml Concurrency (B2 Fix)

`boot.toml` is a plain TOML file read at supervisor startup. Concurrent install/remove operations both rewrite it. Fix: serialize all `boot_config` operations through a `Mutex<()>`:

```rust
// supervisor/src/package/boot_config.rs  (~60 lines)
static BOOT_CONFIG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn add_app(app_name: &str) -> Result<(), String> {
    let _guard = BOOT_CONFIG_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let mut config = read_boot_toml()?;
    if !config.apps.contains(app_name) {
        config.apps.push(app_name.to_string());
    }
    write_boot_toml_atomic(&config)   // write to .tmp, fsync, rename
}
```

Write via R41 atomic-rename (write to `/data/boot.toml.tmp`, fsync, rename to `/data/boot.toml`). Never hold the lock across I/O — take it, read, modify in memory, drop to write atomically.

---

## 7. Signature Verification (B4 Fix)

Ed25519 via `ring` crate (pure Rust, no OpenSSL dependency):

```rust
// supervisor/src/package/verify.rs  (~80 lines)
use ring::signature::{ED25519, UnparsedPublicKey};

pub fn verify_signature(staging_dir: &str) -> Result<(), String> {
    let sig_bytes = fs::read(format!("{}/META-INF/signature.ed25519", staging_dir))?;
    let manifest  = fs::read(format!("{}/META-INF/manifest.json", staging_dir))?;
    let pubkey_pem = load_trusted_pubkey(&manifest_json.signature_key_id)?;
    let pubkey = UnparsedPublicKey::new(&ED25519, &pubkey_pem);
    // Signature covers SHA-256 of the archive tarball bytes (minus the signature file itself)
    let archive_hash = sha256_of_archive(staging_dir)?;
    pubkey.verify(&archive_hash, &sig_bytes)
        .map_err(|_| "signature verification failed".to_string())
}
```

Trusted public keys stored at `/etc/vyoma/trusted_keys/` in initramfs (read-only; apps cannot write to initramfs). Sideloaded packages (from `sources.list` or local file) can use any key registered in `/data/.vyoma/trusted_keys/` (writable, user-managed). `vyoma-official-*` keys are initramfs-only and cannot be overridden.

---

## 8. App Store WASM App

```toml
# apps/store/vyoma.toml
[capabilities]
stdio = true; display = true; mouse = true; network = true
store_manage = true  # gates @supervisor: pkg-install, pkg-update, pkg-remove
```

`store_manage: bool` in `Capabilities` struct. Supervisor rejects `@supervisor: pkg-*` commands from apps without this capability.

Three-panel UI:
- Left sidebar: Installed apps (from packages.db) + Featured (from repository)
- Center: App detail (description, screenshots, version, capabilities required)
- Action bar: Install / Update / Remove / Open

Repository sources from `/data/.vyoma/sources.list` (one URL per line). App polls `@supervisor: pkg-list-available` → supervisor fetches index JSON from each source URL.

```
apps/store/src/
├── main.rs      (~160 lines)
├── ui.rs        (~280 lines: three-panel layout, VYOMA_DRAW calls)
├── catalog.rs   (~180 lines: parse repository index, installed list from packages.db)
└── actions.rs   (~120 lines: install/update/remove IPC flows)
```

---

## 9. File Layout

```
supervisor/src/package/
├── mod.rs          (~40 lines: statics, PackageSubsystem init)
├── install.rs      (~200 lines: extract, verify, atomic rename (B1), register)
├── update.rs       (~160 lines: rollback snapshot, health check (B5), restore)
├── remove.rs       (~120 lines: kill-first (B3), ordered cleanup)
├── boot_config.rs  (~60 lines: mutex-serialized boot.toml R/W (B2))
├── verify.rs       (~80 lines: ed25519 via ring (B4), wasm SHA-256)
└── packages_db.rs  (~120 lines: packages.db wrapper, register/deregister/query)
```

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: partial extract on crash leaves corrupt app dir | All extraction goes to `/data/.vyoma/staging/`; single atomic `rename` to `/data/apps/<name>/` is the commit point |
| B2: concurrent install/remove rewrite boot.toml, losing entries | `BOOT_CONFIG_LOCK: OnceLock<Mutex<()>>` serializes all boot.toml reads+writes; R41 atomic-rename for the write |
| B3: pkg-remove with running instance races on file deletion | `stop_app_sync()` (SIGTERM→SIGKILL, wait for child exit confirmation) runs before ANY file mutation |
| B4: ed25519 signature verification needs OpenSSL on musl | `ring` crate (pure Rust ed25519); trusted keys in initramfs read-only for official packages |
| B5: update rollback when new version crashes on first launch | `snapshot_for_rollback` hard-links old binary; health check monitors first 30s; auto-restore on crash |
