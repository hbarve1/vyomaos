# FINAL Spec: App Distribution & Updates (Round 75)

**Subsystem**: App Distribution & Updates  
**macOS Analogue**: Mac App Store / Sparkle / SUUpdater  
**Depends on**: R50 package manager, R62 code signing (sha2), R66 quarantine, R74 launch services  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

```
supervisor/src/update/
├── mod.rs          — UpdateManager, UpdateCmd enum, handle_update_line, run_check_loop
├── appcast.rs      — HTTP fetch + JSON parse of appcast feed
├── version.rs      — SemVer struct, parse, Ord impl, is_update_available
├── verifier.rs     — Ed25519 verify via ed25519-dalek, SHA-256 via sha2
├── delta.rs        — VYDIFF01 binary patch format, apply_patch
├── installer.rs    — atomic_install (tmp→verify→rename→backup), restore_backup
├── rollback.rs     — crash-within-60s watchdog, register_update_time, check_and_rollback
└── store_index.rs  — /data/.vyoma/store/index.json TTL cache, search
```

Background check thread wakes every 86400s (24h). `VYOMA_UPDATE:` lines dispatched from router.rs.

## 2. Update Check Protocol

Appcast feed JSON format:
```json
{
  "app": "calculator", "version": "1.3.0",
  "url": "http://store.vyomaos.dev/apps/calculator-1.3.0.wasm",
  "sha256": "<64 hex chars>", "sig": "<128 hex chars>",
  "min_os_ver": "0.17.0",
  "delta_url": "http://store.vyomaos.dev/patches/calc-1.2.0-to-1.3.0.patch",
  "delta_sha256": "<64 hex chars>", "delta_sig": "<128 hex chars>"
}
```

vyoma.toml extension: `appcast = "http://store.vyomaos.dev/feeds/calculator.json"` under `[app]`

## 3. Delta Update & Full Replace

```rust
// version.rs
pub struct SemVer { pub major: u32, pub minor: u32, pub patch: u32 }
impl SemVer {
    pub fn parse(s: &str) -> Option<Self> { /* split on '.', parse 3 u32s */ }
}
impl Ord for SemVer { /* compare major, then minor, then patch */ }
pub fn is_update_available(installed: &str, remote: &str) -> bool {
    matches!((SemVer::parse(installed), SemVer::parse(remote)), (Some(a), Some(b)) if b > a)
}
```

Delta patch format (VYDIFF01): 40-byte header (magic + new_size + ctrl_len + diff_len + extra_len), then three raw blocks. apply_patch: ADD phase (old+diff bytes), COPY phase (verbatim extra), SEEK phase (advance old_pos by i32 skip).

Use delta when: `patch.len() < full_estimated_size * 30 / 100`.

## 4. VYOMA_UPDATE Protocol

```
VYOMA_UPDATE:check:<app_name>         → VYOMA_NOTIFY:post:update-check-result:<app>:up-to-date|update-available:<ver>|error:<msg>
VYOMA_UPDATE:install:<app_name>:<ver> → VYOMA_NOTIFY:post:install-result:<app>:ok|error:<msg>
VYOMA_UPDATE:rollback:<app_name>      → VYOMA_NOTIFY:post:rollback-result:<app>:ok|error:<msg>
VYOMA_UPDATE:list_updates:            → VYOMA_NOTIFY:post:update-list:[{"app":"...","from":"...","to":"..."}]
VYOMA_UPDATE:search:<query>           → VYOMA_NOTIFY:post:search-result:[{"name":"...","version":"..."}]
```

All commands dispatched to background threads; never block router thread.

## 5. Rollback Manager

```rust
static UPDATE_TIMES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

pub fn register_update_time(app_name: &str) {
    UPDATE_TIMES.get_or_init(Default::default).lock().unwrap()
        .insert(app_name.to_string(), Instant::now());
}

pub fn check_and_rollback(app_name: &str, exit_code: i32, registry: &AppRegistry, inbox: &Inbox) {
    if exit_code == 0 { return; }
    let updated_recently = UPDATE_TIMES.get_or_init(Default::default).lock().unwrap()
        .get(app_name).map(|t| t.elapsed().as_secs() < 60).unwrap_or(false);
    if !updated_recently { return; }
    if restore_backup(app_name).is_ok() {
        UPDATE_TIMES.get_or_init(Default::default).lock().unwrap().remove(app_name);
        restart_app_by_name(app_name, registry, inbox);
        // post VYOMA_NOTIFY:post:update-rolled-back:<app>
    }
}
```

Called from app_threads.rs waiter thread on non-zero exit.

## 6. App Store Index

`/data/.vyoma/store/index.json` — TTL-based cache (default 86400s). Format:
```json
{ "fetched_at_secs": 1748649600, "ttl_secs": 86400, "apps": [{"name":"...","description":"...","version":"...","size_bytes":14320,"appcast":"..."}] }
```

`load_or_refresh()`: read cache, check TTL, fetch from `http://store.vyomaos.dev/index.json` if stale, write atomically.

## 7. Code Signing Integration

SHA-256 (sha2 crate, already in tree) + Ed25519 (ed25519-dalek 2.1, new dep):

```rust
// verifier.rs
const STORE_PUBLIC_KEY: &[u8; 32] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/keys/store_pubkey.ed25519"));

pub fn verify_ed25519(data: &[u8], sig_hex: &str) -> Result<(), String> {
    let sig_bytes = hex_decode_64(sig_hex).ok_or("invalid sig hex")?;
    let key = VerifyingKey::from_bytes(STORE_PUBLIC_KEY).map_err(|e| e.to_string())?;
    key.verify(data, &Signature::from_bytes(&sig_bytes)).map_err(|_| "signature invalid".into())
}
```

Atomic install sequence:
1. Download to `/data/apps/<name>.wasm.tmp`
2. Verify SHA-256
3. Verify Ed25519 sig
4. `rename(.wasm → .wasm.bak)`
5. `rename(.tmp → .wasm)` (POSIX atomic, same fs)
6. Write `/data/.vyoma/quarantine/<name>` = "verified" (clears R66 quarantine)
7. Re-register via R74 launch services

## 8. Blocking Issues (B1–B5)

**B1 — `restart_app_by_name` doesn't exist as standalone function**  
Extract from ipc_handlers.rs: kill existing child, call spawn_app + launch_app_threads. Make `pub fn restart_app_by_name(app_name, registry, inbox)`. Used by rollback.rs and installer.rs post-install.

**B2 — ed25519-dalek 2.x on musl: must use `alloc`-only features (no `getrandom`)**  
```toml
ed25519-dalek = { version = "2.1", default-features = false, features = ["alloc"] }
```
verify is pure computation — no RNG needed.

**B3 — serde_json must be moved from dev-dependencies to dependencies**  
`store_index.rs` and `appcast.rs` use `serde_json::from_str` in production code. Move `serde_json = "1"` to `[dependencies]`.

**B4 — `VYOMA_UPDATE:` lines not yet parsed by router.rs**  
Add branch in router.rs before IPC broker fallthrough:
```rust
if line.starts_with("VYOMA_UPDATE:") {
    if let Some(mgr) = crate::UPDATE_MANAGER.get() {
        crate::update::handle_update_line(line, app_name, registry, inbox, mgr);
        continue;
    }
}
```
Add `UPDATE_MANAGER: OnceLock<Arc<Mutex<UpdateManager>>>` to main.rs.

**B5 — `fs::rename` returns EXDEV if tmp and dst on different mounts**  
Guard in atomic_install: verify `tmp_path.parent() == dst_path.parent()` before rename. All paths under `/data/apps/` are same mount. Document invariant with assertion comment.
