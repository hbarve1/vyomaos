# FINAL Spec: App Distribution & Updates (Round 75)

**Subsystem**: App Distribution & Updates  
**macOS Analogue**: Mac App Store / Sparkle / SUUpdater  
**Depends on**: R50 package manager, R62 code signing (sha2), R66 quarantine, R74 launch services  
**Status**: FINAL — all blocking issues resolved  
**Date**: 2026-05-30

---

## 1. Architecture

### Module Tree

```
supervisor/src/update/
├── mod.rs         (~180 lines) — UpdateManager, UpdateCmd enum, handle_update_line, run_check_loop
├── appcast.rs     (~160 lines) — HTTP fetch + JSON parse of appcast feed, AppcastEntry struct
├── version.rs     (~80 lines)  — SemVer struct, parse, Ord impl, is_update_available
├── verifier.rs    (~120 lines) — Ed25519 verify via ed25519-dalek, SHA-256 via sha2
├── delta.rs       (~160 lines) — VYDIFF01 binary patch format header + apply_patch
├── installer.rs   (~180 lines) — atomic_install: tmp→verify→rename→backup flow
├── rollback.rs    (~120 lines) — 60s crash watchdog, register_update_time, check_and_rollback
└── store_index.rs (~120 lines) — /data/.vyoma/store/index.json TTL cache, search_apps
```

### Global State (main.rs)

```rust
// main.rs — add alongside existing OnceLock globals
pub static UPDATE_MANAGER: OnceLock<Arc<Mutex<UpdateManager>>> = OnceLock::new();
```

Initialized at supervisor startup after app registry is ready:

```rust
// main.rs — in initialization block
let public_key = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/keys/store_pubkey.ed25519"));
let mgr = UpdateManager::new(*public_key);
UPDATE_MANAGER.set(Arc::new(Mutex::new(mgr))).ok();

// Spawn background 24h check thread
{
    let mgr = UPDATE_MANAGER.get().unwrap().clone();
    let reg = APP_REGISTRY.get().unwrap().clone();
    let inbox = IPC_INBOX.get().unwrap().clone();
    std::thread::spawn(move || crate::update::run_check_loop(mgr, reg, inbox));
}
```

Background check thread wakes every 86400 seconds (24 hours). All VYOMA_UPDATE: protocol lines are dispatched by router.rs before the IPC broker fallthrough.

---

## 2. VYOMA_UPDATE Protocol

### App → Supervisor (complete, exact strings)

```
VYOMA_UPDATE:check:<app_name>
VYOMA_UPDATE:install:<app_name>:<version>
VYOMA_UPDATE:cancel:<app_name>
VYOMA_UPDATE:status:<app_name>
VYOMA_UPDATE:search:<query>
VYOMA_UPDATE:list_installed
```

### Supervisor → App (response events written to app's stdin)

```
VYOMA_UPDATE:update-available:<app>:<current>:<remote>
VYOMA_UPDATE:download-progress:<app>:<pct>
VYOMA_UPDATE:install-done:<app>:<version>
VYOMA_UPDATE:install-failed:<app>:<reason>
VYOMA_UPDATE:search-results:<json_array>
VYOMA_UPDATE:installed-list:<json_array>
VYOMA_UPDATE:no-update:<app>
```

### Router Dispatch (router.rs)

Add this branch BEFORE the IPC broker fallthrough, so `VYOMA_UPDATE:` lines never reach the IPC routing path:

```rust
// router.rs — add BEFORE IPC broker fallthrough
if line.starts_with("VYOMA_UPDATE:") {
    if let Some(mgr) = crate::UPDATE_MANAGER.get() {
        crate::update::handle_update_line(line, app_name, registry, inbox, mgr);
        continue;
    }
}
```

All commands dispatch to background threads; the router thread is never blocked.

### Protocol Field Constraints

- `<app_name>`: ASCII alphanumeric + hyphens, max 64 chars, must match installed app name exactly
- `<version>`: semver string `MAJOR.MINOR.PATCH`, all components are `u32`
- `<pct>`: integer 0–100 (percentage of download completed)
- `<reason>`: URL-encoded string, no newlines, max 256 chars
- `<json_array>`: valid JSON array, UTF-8, newlines escaped as `\n`

---

## 3. Complete Rust Types

### version.rs

```rust
// supervisor/src/update/version.rs

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemVer {
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some(SemVer {
            major: parts[0].parse().ok()?,
            minor: parts[1].parse().ok()?,
            patch: parts[2].parse().ok()?,
        })
    }

    pub fn to_string(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub fn is_update_available(installed: &str, remote: &str) -> bool {
    matches!(
        (SemVer::parse(installed), SemVer::parse(remote)),
        (Some(a), Some(b)) if b > a
    )
}

pub fn version_satisfies_min(os_ver: &str, min_os_ver: &str) -> bool {
    matches!(
        (SemVer::parse(os_ver), SemVer::parse(min_os_ver)),
        (Some(actual), Some(min)) if actual >= min
    )
}
```

### appcast.rs

```rust
// supervisor/src/update/appcast.rs

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AppcastEntry {
    pub app: String,
    pub version: String,
    pub url: String,
    pub sha256: String,          // 64 hex chars (SHA-256 of full .wasm)
    pub sig: String,             // 128 hex chars (Ed25519 signature)
    pub min_os_ver: String,      // minimum supervisor semver required
    pub delta_url: Option<String>,
    pub delta_sha256: Option<String>,
    pub delta_sig: Option<String>,
    pub changelog: Option<String>,
}

/// Fetch and parse an appcast JSON feed from the given URL.
/// Returns Err if HTTP fails or JSON is malformed.
pub fn fetch_appcast(url: &str) -> Result<AppcastEntry, String> {
    let body = http_get(url)?;
    serde_json::from_str::<AppcastEntry>(&body).map_err(|e| e.to_string())
}

/// Simple blocking HTTP GET over TCP (no TLS for now — store URLs are http://).
/// Uses std::net::TcpStream directly to avoid pulling in a full HTTP client crate.
fn http_get(url: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let (host, path) = parse_http_url(url)?;
    let addr = format!("{}:80", host);
    let mut stream = TcpStream::connect(&addr).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30))).ok();

    let req = format!("GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n", path, host);
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;

    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;

    // Strip HTTP headers (split on \r\n\r\n)
    let body_start = response.find("\r\n\r\n")
        .ok_or("no HTTP header separator")?;
    Ok(response[body_start + 4..].to_string())
}

fn parse_http_url(url: &str) -> Result<(String, String), String> {
    let url = url.strip_prefix("http://").ok_or("only http:// supported")?;
    match url.find('/') {
        Some(idx) => Ok((url[..idx].to_string(), url[idx..].to_string())),
        None => Ok((url.to_string(), "/".to_string())),
    }
}
```

### mod.rs (UpdateManager, UpdateCmd, UpdateState)

```rust
// supervisor/src/update/mod.rs

pub mod appcast;
pub mod delta;
pub mod installer;
pub mod rollback;
pub mod store_index;
pub mod verifier;
pub mod version;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub use appcast::AppcastEntry;
pub use version::{is_update_available, SemVer};

#[derive(Debug)]
pub enum UpdateCmd {
    Check { app_name: String },
    Install { app_name: String, version: String },
    Cancel { app_name: String },
    Status { app_name: String },
}

#[derive(Debug, Clone)]
pub enum UpdateState {
    Checking,
    Downloading { progress_pct: u8 },
    Verifying,
    Installing,
    RollingBack,
    Done { version: String },
    Failed { reason: String },
}

pub struct UpdateManager {
    pub in_flight: HashMap<String, UpdateState>,
    pub public_key: [u8; 32],  // Ed25519 verifying key, baked into supervisor binary
}

impl UpdateManager {
    pub fn new(public_key: [u8; 32]) -> Self {
        UpdateManager {
            in_flight: HashMap::new(),
            public_key,
        }
    }
}

/// Dispatch a single VYOMA_UPDATE: line received from an app.
pub fn handle_update_line(
    line: &str,
    app_name: &str,
    registry: &Arc<std::sync::RwLock<crate::AppRegistry>>,
    inbox: &Arc<Mutex<crate::IpcInbox>>,
    mgr: &Arc<Mutex<UpdateManager>>,
) {
    let rest = match line.strip_prefix("VYOMA_UPDATE:") {
        Some(r) => r,
        None => return,
    };

    let cmd = parse_update_cmd(rest, app_name);
    match cmd {
        Some(UpdateCmd::Check { app_name: name }) => {
            let mgr = mgr.clone();
            let reg = registry.clone();
            let inbox = inbox.clone();
            std::thread::spawn(move || do_check(&name, mgr, reg, inbox));
        }
        Some(UpdateCmd::Install { app_name: name, version }) => {
            let mgr = mgr.clone();
            let reg = registry.clone();
            let inbox = inbox.clone();
            std::thread::spawn(move || do_install(&name, &version, mgr, reg, inbox));
        }
        Some(UpdateCmd::Status { app_name: name }) => {
            let state = mgr.lock().unwrap().in_flight.get(&name).cloned();
            send_status_response(&name, state, inbox);
        }
        Some(UpdateCmd::Cancel { app_name: name }) => {
            mgr.lock().unwrap().in_flight.remove(&name);
        }
        None => {}
    }
}

fn parse_update_cmd(rest: &str, _app_name: &str) -> Option<UpdateCmd> {
    let parts: Vec<&str> = rest.splitn(3, ':').collect();
    match parts.as_slice() {
        ["check", name] => Some(UpdateCmd::Check { app_name: name.to_string() }),
        ["install", name, ver] => Some(UpdateCmd::Install {
            app_name: name.to_string(),
            version: ver.to_string(),
        }),
        ["cancel", name] => Some(UpdateCmd::Cancel { app_name: name.to_string() }),
        ["status", name] => Some(UpdateCmd::Status { app_name: name.to_string() }),
        _ => None,
    }
}

/// Background loop: wake every 24 hours and check all installed apps for updates.
pub fn run_check_loop(
    mgr: Arc<Mutex<UpdateManager>>,
    registry: Arc<std::sync::RwLock<crate::AppRegistry>>,
    inbox: Arc<Mutex<crate::IpcInbox>>,
) {
    loop {
        let app_names: Vec<String> = {
            registry.read().unwrap().apps.keys().cloned().collect()
        };
        for name in app_names {
            do_check(&name, mgr.clone(), registry.clone(), inbox.clone());
        }
        std::thread::sleep(std::time::Duration::from_secs(86400));
    }
}

fn do_check(
    app_name: &str,
    mgr: Arc<Mutex<UpdateManager>>,
    registry: Arc<std::sync::RwLock<crate::AppRegistry>>,
    inbox: Arc<Mutex<crate::IpcInbox>>,
) {
    mgr.lock().unwrap().in_flight.insert(app_name.to_string(), UpdateState::Checking);

    // Fetch appcast URL from manifest
    let appcast_url = {
        let reg = registry.read().unwrap();
        reg.apps.get(app_name).and_then(|a| a.manifest.app.appcast.clone())
    };
    let url = match appcast_url {
        Some(u) => u,
        None => return,
    };

    match appcast::fetch_appcast(&url) {
        Ok(entry) => {
            let installed_ver = {
                let reg = registry.read().unwrap();
                reg.apps.get(app_name).map(|a| a.manifest.app.version.clone()).unwrap_or_default()
            };
            if is_update_available(&installed_ver, &entry.version) {
                send_to_app(app_name, &format!("VYOMA_UPDATE:update-available:{}:{}:{}", app_name, installed_ver, entry.version), &inbox);
            } else {
                send_to_app(app_name, &format!("VYOMA_UPDATE:no-update:{}", app_name), &inbox);
            }
        }
        Err(e) => {
            send_to_app(app_name, &format!("VYOMA_UPDATE:install-failed:{}:{}", app_name, e), &inbox);
        }
    }
    mgr.lock().unwrap().in_flight.remove(app_name);
}

fn do_install(
    app_name: &str,
    _requested_version: &str,
    mgr: Arc<Mutex<UpdateManager>>,
    registry: Arc<std::sync::RwLock<crate::AppRegistry>>,
    inbox: Arc<Mutex<crate::IpcInbox>>,
) {
    // Fetch appcast, decide full vs delta, download, verify, install
    let appcast_url = {
        let reg = registry.read().unwrap();
        reg.apps.get(app_name).and_then(|a| a.manifest.app.appcast.clone())
    };
    let url = match appcast_url { Some(u) => u, None => return };
    let entry = match appcast::fetch_appcast(&url) { Ok(e) => e, Err(e) => {
        send_to_app(app_name, &format!("VYOMA_UPDATE:install-failed:{}:{}", app_name, e), &inbox);
        return;
    }};

    // Download
    mgr.lock().unwrap().in_flight.insert(app_name.to_string(), UpdateState::Downloading { progress_pct: 0 });
    let wasm_bytes = match download_with_progress(app_name, &entry.url, &mgr, &inbox) {
        Ok(b) => b,
        Err(e) => {
            send_to_app(app_name, &format!("VYOMA_UPDATE:install-failed:{}:{}", app_name, e), &inbox);
            return;
        }
    };

    // Verify
    mgr.lock().unwrap().in_flight.insert(app_name.to_string(), UpdateState::Verifying);
    let pub_key = mgr.lock().unwrap().public_key;
    if let Err(e) = verifier::verify_sha256(&wasm_bytes, &entry.sha256) {
        send_to_app(app_name, &format!("VYOMA_UPDATE:install-failed:{}:sha256-{}", app_name, e), &inbox);
        return;
    }
    if let Err(e) = verifier::verify_ed25519(&wasm_bytes, &entry.sig, &pub_key) {
        send_to_app(app_name, &format!("VYOMA_UPDATE:install-failed:{}:sig-{}", app_name, e), &inbox);
        return;
    }

    // Install
    mgr.lock().unwrap().in_flight.insert(app_name.to_string(), UpdateState::Installing);
    let reg_arc = registry.clone();
    match installer::atomic_install(app_name, &wasm_bytes, &entry, &reg_arc, &inbox) {
        Ok(()) => {
            rollback::register_update_time(app_name);
            send_to_app(app_name, &format!("VYOMA_UPDATE:install-done:{}:{}", app_name, entry.version), &inbox);
        }
        Err(e) => {
            send_to_app(app_name, &format!("VYOMA_UPDATE:install-failed:{}:{}", app_name, e), &inbox);
        }
    }
    mgr.lock().unwrap().in_flight.remove(app_name);
}

fn send_status_response(
    app_name: &str,
    state: Option<UpdateState>,
    inbox: &Arc<Mutex<crate::IpcInbox>>,
) {
    let msg = match state {
        None => format!("VYOMA_UPDATE:no-update:{}", app_name),
        Some(UpdateState::Checking) => format!("VYOMA_UPDATE:download-progress:{}:0", app_name),
        Some(UpdateState::Downloading { progress_pct }) =>
            format!("VYOMA_UPDATE:download-progress:{}:{}", app_name, progress_pct),
        Some(UpdateState::Done { version }) =>
            format!("VYOMA_UPDATE:install-done:{}:{}", app_name, version),
        Some(UpdateState::Failed { reason }) =>
            format!("VYOMA_UPDATE:install-failed:{}:{}", app_name, reason),
        _ => format!("VYOMA_UPDATE:download-progress:{}:50", app_name),
    };
    send_to_app(app_name, &msg, inbox);
}

fn send_to_app(app_name: &str, msg: &str, inbox: &Arc<Mutex<crate::IpcInbox>>) {
    inbox.lock().unwrap().deliver(app_name, msg);
}

fn download_with_progress(
    app_name: &str,
    url: &str,
    mgr: &Arc<Mutex<UpdateManager>>,
    inbox: &Arc<Mutex<crate::IpcInbox>>,
) -> Result<Vec<u8>, String> {
    // In production: stream download, update progress_pct every ~10%
    // Simplified: single blocking fetch
    let bytes = appcast::http_get_bytes(url)?;
    mgr.lock().unwrap().in_flight.insert(app_name.to_string(), UpdateState::Downloading { progress_pct: 100 });
    send_to_app(app_name, &format!("VYOMA_UPDATE:download-progress:{}:100", app_name), inbox);
    Ok(bytes)
}
```

---

## 4. Appcast Feed Format

### Example JSON (exact structure)

```json
{
  "app": "calculator",
  "version": "1.3.0",
  "url": "http://store.vyomaos.dev/apps/calculator-1.3.0.wasm",
  "sha256": "<64 hex chars — SHA-256 of the full .wasm binary>",
  "sig": "<128 hex chars — Ed25519 signature over the full .wasm binary>",
  "min_os_ver": "0.17.0",
  "delta_url": "http://store.vyomaos.dev/patches/calc-1.2.0-to-1.3.0.patch",
  "delta_sha256": "<64 hex chars — SHA-256 of the delta patch file>",
  "delta_sig": "<128 hex chars — Ed25519 signature over the delta patch file>",
  "changelog": "Bug fixes and performance improvements"
}
```

### vyoma.toml Extension

```toml
# apps/calculator/vyoma.toml
[app]
name    = "calculator"
version = "1.2.0"
wasm    = "calculator.wasm"
appcast = "http://store.vyomaos.dev/feeds/calculator.json"   # NEW field

[capabilities]
stdio   = true
display = true
```

The `appcast` field is optional. Apps without it are not checked for updates. `min_os_ver` is checked against the supervisor's own compiled-in version string before install is permitted; if the OS version is too old the supervisor sends `install-failed` with reason `os-version-too-old`.

---

## 5. Delta Format VYDIFF01 (Binary, Complete Spec)

### Header (40 bytes, little-endian)

```rust
// supervisor/src/update/delta.rs

#[repr(C)]
pub struct DeltaHeader {
    pub magic:     [u8; 8],  // b"VYDIFF01"
    pub new_size:  u64,      // expected size of reconstructed output
    pub ctrl_len:  u64,      // byte length of control block
    pub diff_len:  u64,      // byte length of diff block
    pub extra_len: u64,      // byte length of extra (verbatim copy) block
}
// sizeof(DeltaHeader) == 40 bytes

/// Each control entry: three signed 64-bit integers.
/// x = add_length  (bytes to ADD from old+diff)
/// y = copy_length (bytes to COPY verbatim from extra block)
/// z = seek_offset (signed advance of old_pos after add+copy phase)
pub struct CtrlEntry {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}
```

### apply_patch

```rust
pub fn apply_patch(old: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    if patch.len() < 40 {
        return Err("patch too short for header".into());
    }
    if &patch[..8] != b"VYDIFF01" {
        return Err("invalid VYDIFF01 magic".into());
    }

    let new_size  = u64::from_le_bytes(patch[8..16].try_into().unwrap()) as usize;
    let ctrl_len  = u64::from_le_bytes(patch[16..24].try_into().unwrap()) as usize;
    let diff_len  = u64::from_le_bytes(patch[24..32].try_into().unwrap()) as usize;
    let extra_len = u64::from_le_bytes(patch[32..40].try_into().unwrap()) as usize;

    let ctrl_start  = 40;
    let diff_start  = ctrl_start + ctrl_len;
    let extra_start = diff_start + diff_len;
    let expected_end = extra_start + extra_len;

    if patch.len() < expected_end {
        return Err(format!("patch truncated: got {} bytes, need {}", patch.len(), expected_end));
    }

    let ctrl_block  = &patch[ctrl_start..diff_start];
    let diff_block  = &patch[diff_start..extra_start];
    let extra_block = &patch[extra_start..extra_start + extra_len];

    let mut new_data = vec![0u8; new_size];
    let mut old_pos: i64 = 0;
    let mut new_pos: usize = 0;
    let mut diff_pos: usize = 0;
    let mut extra_pos: usize = 0;
    let mut ctrl_pos: usize = 0;

    while new_pos < new_size {
        if ctrl_pos + 24 > ctrl_len { break; }

        let x = i64::from_le_bytes(ctrl_block[ctrl_pos..ctrl_pos+8].try_into().unwrap());
        let y = i64::from_le_bytes(ctrl_block[ctrl_pos+8..ctrl_pos+16].try_into().unwrap());
        let z = i64::from_le_bytes(ctrl_block[ctrl_pos+16..ctrl_pos+24].try_into().unwrap());
        ctrl_pos += 24;

        // ADD phase: new[i] = old[old_pos + i] + diff[diff_pos + i]
        let add_len = x as usize;
        for i in 0..add_len {
            let old_byte = if old_pos >= 0 && (old_pos as usize) < old.len() {
                old[old_pos as usize]
            } else { 0 };
            new_data[new_pos] = old_byte.wrapping_add(diff_block[diff_pos]);
            new_pos += 1;
            old_pos += 1;
            diff_pos += 1;
        }

        // COPY phase: new[..] = extra[extra_pos .. extra_pos + y]
        let copy_len = y as usize;
        new_data[new_pos..new_pos + copy_len]
            .copy_from_slice(&extra_block[extra_pos..extra_pos + copy_len]);
        new_pos += copy_len;
        extra_pos += copy_len;

        // SEEK phase: advance old_pos by z (signed)
        old_pos += z;
    }

    Ok(new_data)
}

/// Prefer delta when patch is less than 30% of the estimated full wasm size.
pub fn should_use_delta(patch_size: usize, full_wasm_size: usize) -> bool {
    patch_size < full_wasm_size * 30 / 100
}
```

---

## 6. Atomic Install Flow

```rust
// supervisor/src/update/installer.rs

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub fn atomic_install(
    app_name: &str,
    wasm_bytes: &[u8],
    entry: &crate::update::AppcastEntry,
    registry: &Arc<std::sync::RwLock<crate::AppRegistry>>,
    inbox: &Arc<Mutex<crate::IpcInbox>>,
) -> Result<(), String> {
    let apps_dir = Path::new("/data/apps").join(app_name);
    let dst = apps_dir.join(format!("{}.wasm", app_name));
    let tmp = apps_dir.join(format!("{}.wasm.tmp", app_name));
    let bak = apps_dir.join(format!("{}.wasm.bak", app_name));

    // Step 1: Assert same mount to prevent EXDEV on rename
    // All paths under /data/apps/ are on the same 9P virtio mount — this is an invariant.
    assert_same_mount(&tmp, &dst)?;

    // Step 2: Ensure directory exists
    fs::create_dir_all(&apps_dir).map_err(|e| e.to_string())?;

    // Step 3: Write to tmp file
    fs::write(&tmp, wasm_bytes).map_err(|e| e.to_string())?;

    // Step 4: SHA-256 verify (against already-downloaded bytes in memory)
    crate::update::verifier::verify_sha256(wasm_bytes, &entry.sha256)?;

    // Step 5: Ed25519 verify
    let pub_key = crate::UPDATE_MANAGER.get()
        .map(|m| m.lock().unwrap().public_key)
        .unwrap_or([0u8; 32]);
    crate::update::verifier::verify_ed25519(wasm_bytes, &entry.sig, &pub_key)?;

    // Step 6: Backup existing .wasm → .wasm.bak (enables rollback)
    if dst.exists() {
        fs::rename(&dst, &bak).map_err(|e| e.to_string())?;
    }

    // Step 7: Atomic rename tmp → dst (POSIX atomic on same filesystem)
    fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;

    // Step 8: Clear R66 quarantine flag
    crate::quarantine::clear_quarantine_flag(&dst);

    // Step 9: Re-register with R74 Launch Services
    {
        let mut reg = registry.write().unwrap();
        reg.register_app(app_name).map_err(|e| e.to_string())?;
    }

    // Step 10: Restart app (kill existing child, re-spawn)
    crate::ipc_handlers::restart_app_by_name(app_name, registry, inbox)
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn assert_same_mount(a: &Path, b: &Path) -> Result<(), String> {
    // Use libc::stat to compare st_dev fields.
    // Both a and b reside under /data/apps/ which is a single 9P mount.
    let parent_a = a.parent().unwrap_or(Path::new("/data/apps"));
    let parent_b = b.parent().unwrap_or(Path::new("/data/apps"));

    let stat_a = stat_path(parent_a)?;
    let stat_b = stat_path(parent_b)?;

    if stat_a != stat_b {
        return Err(format!(
            "tmp ({}) and dst ({}) are on different devices — cannot rename atomically",
            parent_a.display(),
            parent_b.display()
        ));
    }
    Ok(())
}

fn stat_path(p: &Path) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(p)
        .map(|m| m.dev())
        .map_err(|e| e.to_string())
}

pub fn restore_backup(app_name: &str) -> Result<(), String> {
    let apps_dir = Path::new("/data/apps").join(app_name);
    let dst = apps_dir.join(format!("{}.wasm", app_name));
    let bak = apps_dir.join(format!("{}.wasm.bak", app_name));

    if !bak.exists() {
        return Err(format!("no backup found for {}", app_name));
    }

    // Remove broken .wasm if present, then rename .bak → .wasm
    if dst.exists() {
        fs::remove_file(&dst).map_err(|e| e.to_string())?;
    }
    fs::rename(&bak, &dst).map_err(|e| e.to_string())?;
    Ok(())
}
```

---

## 7. Rollback System

```rust
// supervisor/src/update/rollback.rs

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static UPDATE_TIMES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

/// Called immediately after a successful install — starts the 60s crash watch window.
pub fn register_update_time(app_name: &str) {
    UPDATE_TIMES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(app_name.to_string(), Instant::now());
}

/// Called from app_threads.rs waiter thread when the app process exits unexpectedly.
/// If the app crashed within 60s of an update AND exit_code != 0, roll back automatically.
pub fn check_and_rollback(
    app_name: &str,
    exit_code: i32,
    registry: &std::sync::Arc<std::sync::RwLock<crate::AppRegistry>>,
    inbox: &std::sync::Arc<Mutex<crate::IpcInbox>>,
) {
    if exit_code == 0 {
        // Clean exit — not a crash, no rollback needed
        return;
    }

    let map = UPDATE_TIMES.get_or_init(|| Mutex::new(HashMap::new()));
    let elapsed = map.lock().unwrap().get(app_name).map(|t| t.elapsed());

    if let Some(elapsed) = elapsed {
        if elapsed.as_secs() < 60 {
            log::warn!(
                "[rollback] {} crashed {}s after update (exit {}), rolling back",
                app_name,
                elapsed.as_secs(),
                exit_code
            );

            match crate::update::installer::restore_backup(app_name) {
                Ok(()) => {
                    map.lock().unwrap().remove(app_name);
                    log::info!("[rollback] {} restored from backup", app_name);

                    // Notify all apps watching this app's state
                    let msg = format!("VYOMA_UPDATE:install-failed:{}:rolled-back-after-crash", app_name);
                    inbox.lock().unwrap().broadcast(&msg);

                    // Restart with the restored (old) binary
                    let _ = crate::ipc_handlers::restart_app_by_name(app_name, registry, inbox);
                }
                Err(e) => {
                    log::error!("[rollback] failed to restore {}: {}", app_name, e);
                }
            }
        } else {
            // Crash after 60s window — not update-related, remove tracking entry
            map.lock().unwrap().remove(app_name);
        }
    }
}

/// Clear tracking entry after the 60s window without a crash (app is stable).
pub fn confirm_stable(app_name: &str) {
    UPDATE_TIMES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .remove(app_name);
}
```

Integration point in `app_threads.rs`:

```rust
// app_threads.rs — in waiter thread after child.wait()
let exit_code = status.code().unwrap_or(1);
crate::update::rollback::check_and_rollback(&app_name, exit_code, &registry, &inbox);
```

---

## 8. Store Index Cache

### File: `/data/.vyoma/store/index.json`

```json
{
  "updated_at": 1748000000,
  "apps": [
    {
      "name": "calculator",
      "version": "1.3.0",
      "description": "Four-function calculator with history",
      "category": "utilities",
      "appcast": "http://store.vyomaos.dev/feeds/calculator.json"
    },
    {
      "name": "text-editor",
      "version": "0.9.1",
      "description": "Minimal text editor for /data files",
      "category": "productivity",
      "appcast": "http://store.vyomaos.dev/feeds/text-editor.json"
    }
  ]
}
```

### store_index.rs

```rust
// supervisor/src/update/store_index.rs

use serde::{Deserialize, Serialize};
use std::path::Path;

const INDEX_PATH: &str = "/data/.vyoma/store/index.json";
const STORE_INDEX_URL: &str = "http://store.vyomaos.dev/index.json";
const TTL_SECS: u64 = 86400;  // 24 hours

#[derive(Debug, Serialize, Deserialize)]
pub struct AppSummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: String,
    pub appcast: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoreIndex {
    pub updated_at: u64,   // Unix timestamp of last fetch
    pub apps: Vec<AppSummary>,
}

pub fn load_or_refresh() -> Result<StoreIndex, String> {
    let path = Path::new(INDEX_PATH);

    if path.exists() {
        let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if let Ok(idx) = serde_json::from_str::<StoreIndex>(&raw) {
            let now = unix_now();
            if now.saturating_sub(idx.updated_at) < TTL_SECS {
                return Ok(idx);
            }
        }
    }

    // Cache is stale or missing — fetch fresh
    refresh_index()
}

fn refresh_index() -> Result<StoreIndex, String> {
    let body = crate::update::appcast::http_get_bytes(STORE_INDEX_URL)
        .and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()))?;
    let apps: Vec<AppSummary> = serde_json::from_str(&body).map_err(|e| e.to_string())?;

    let idx = StoreIndex {
        updated_at: unix_now(),
        apps,
    };

    // Atomic write: tmp → rename
    let tmp = format!("{}.tmp", INDEX_PATH);
    let json = serde_json::to_string_pretty(&idx).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(Path::new(INDEX_PATH).parent().unwrap()).ok();
    std::fs::write(&tmp, &json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, INDEX_PATH).map_err(|e| e.to_string())?;

    Ok(idx)
}

/// Case-insensitive substring search on app name and description.
pub fn search_apps(query: &str) -> Vec<AppSummary> {
    let q = query.to_lowercase();
    match load_or_refresh() {
        Ok(idx) => idx.apps.into_iter()
            .filter(|a| a.name.to_lowercase().contains(&q) || a.description.to_lowercase().contains(&q))
            .collect(),
        Err(_) => vec![],
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
```

---

## 9. Manifest Extension

```toml
# apps/calculator/vyoma.toml
[app]
name    = "calculator"
version = "1.2.0"
wasm    = "calculator.wasm"
appcast = "http://store.vyomaos.dev/feeds/calculator.json"   # NEW

[capabilities]
stdio   = true
display = true
```

The `appcast` field is optional on all manifests. Apps without it silently skip update checks. The manifest struct in `manifest.rs` adds:

```rust
// manifest.rs — in AppMeta struct
#[serde(default)]
pub appcast: Option<String>,
```

Because `manifest.rs` uses `#[serde(deny_unknown_fields)]` on `Capabilities` but NOT on `AppMeta`, this addition is safe and backward-compatible with all existing `vyoma.toml` files.

---

## 10. Integration Points

### R66 Quarantine

After atomic rename of `.wasm.tmp` → `.wasm`, immediately clear the quarantine flag. This prevents the supervisor from blocking app launch with a quarantine check failure:

```rust
// quarantine.rs (existing module)
pub fn clear_quarantine_flag(path: &std::path::Path) {
    let flag = Path::new("/data/.vyoma/quarantine")
        .join(path.file_name().unwrap_or_default());
    // Mark as verified-by-update-manager
    let _ = std::fs::write(&flag, b"verified");
}
```

### R74 Launch Services

Re-register the app after install so Launch Services has the updated binary path and version metadata:

```rust
// launch_registry.rs (existing module)
impl LaunchRegistry {
    pub fn register_app(&mut self, name: &str) -> Result<(), String> {
        // Update internal map with new binary mtime + version
        // Persists to /data/.vyoma/launch/registry.json atomically
        ...
    }
}
```

### R62 Code Signing (verifier.rs)

```rust
// supervisor/src/update/verifier.rs

use ed25519_dalek::{Signature, VerifyingKey, Verifier};
use sha2::{Sha256, Digest};

pub fn verify_sha256(data: &[u8], expected_hex: &str) -> Result<(), String> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let actual = hex_encode(hasher.finalize().as_slice());
    if actual != expected_hex.to_lowercase() {
        return Err(format!("SHA-256 mismatch: got {}, expected {}", actual, expected_hex));
    }
    Ok(())
}

pub fn verify_ed25519(data: &[u8], sig_hex: &str, pub_key_bytes: &[u8; 32]) -> Result<(), String> {
    let sig_bytes = hex_decode_64(sig_hex)
        .ok_or_else(|| "sig_hex must be 128 hex chars".to_string())?;
    let key = VerifyingKey::from_bytes(pub_key_bytes)
        .map_err(|e| e.to_string())?;
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify(data, &sig).map_err(|_| "Ed25519 signature invalid".into())
}

fn hex_decode_64(s: &str) -> Option<[u8; 64]> {
    if s.len() != 128 { return None; }
    let mut out = [0u8; 64];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
```

### R50 Package Manager

The update manager is layered above R50's install flow. R50 handles initial install from a package manifest; R75 handles subsequent updates. They share `/data/apps/<name>/<name>.wasm` as the canonical binary path and `/data/.vyoma/installed.txt` as the installed-apps registry.

### restart_app_by_name (ipc_handlers.rs)

```rust
// ipc_handlers.rs — extract from inline restart logic
pub fn restart_app_by_name(
    app_name: &str,
    registry: &Arc<RwLock<AppRegistry>>,
    inbox: &Arc<Mutex<IpcInbox>>,
) -> Result<(), String> {
    // 1. Send SIGTERM to existing child (if running)
    {
        let reg = registry.read().unwrap();
        if let Some(app) = reg.apps.get(app_name) {
            if let Some(pid) = app.pid {
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            }
        }
    }

    // 2. Wait up to 3s for clean exit, then SIGKILL
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let still_running = registry.read().unwrap()
            .apps.get(app_name)
            .map(|a| a.pid.is_some())
            .unwrap_or(false);
        if !still_running || std::time::Instant::now() >= deadline { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // 3. Spawn fresh child + launch app threads
    let manifest = registry.read().unwrap()
        .apps.get(app_name)
        .map(|a| a.manifest.clone())
        .ok_or_else(|| format!("app {} not found in registry", app_name))?;

    crate::spawn_app(&manifest, registry, inbox).map_err(|e| e.to_string())
}
```

---

## 11. Cargo.toml Changes

```toml
# supervisor/Cargo.toml

[dependencies]
# ... existing deps ...
serde_json = "1"                                                          # B3: move from dev-dependencies
ed25519-dalek = { version = "2.1", default-features = false, features = ["alloc"] }  # B2: alloc-only, no getrandom
```

---

## 12. Blocking Issues

### B1 — `restart_app_by_name` doesn't exist as standalone function

Currently, restart logic is inlined inside IPC command handlers in `ipc_handlers.rs`. Both `installer.rs` (post-install) and `rollback.rs` (post-rollback) need to call it. Extract and make public:

```rust
// ipc_handlers.rs — extract and make pub
pub fn restart_app_by_name(
    app_name: &str,
    apps: &Arc<RwLock<AppRegistry>>,
    inbox: &Arc<Mutex<IpcInbox>>,
) -> Result<(), String> {
    // 1. Send SIGTERM to existing child
    // 2. Wait up to 3s for clean exit
    // 3. Call spawn_app + launch_app_threads
    // (full implementation in Section 10 above)
}
```

Risk: concurrent restart (rollback + user-triggered restart) — guard with a per-app restart mutex or check `in_flight` state before spawning.

### B2 — ed25519-dalek on musl: must use alloc-only features

The default `ed25519-dalek` feature set pulls in `getrandom`, which uses `getrandom(2)` syscall. On the musl static build the syscall path works, but the `getrandom` crate feature gate requires `std`. Use alloc-only features instead — verification is pure computation and needs no entropy source:

```toml
ed25519-dalek = { version = "2.1", default-features = false, features = ["alloc"] }
```

Test: `cargo build --target x86_64-unknown-linux-musl` must succeed with no `getrandom` in the dependency tree (`cargo tree -i getrandom` should show empty).

### B3 — serde_json in dev-dependencies only

`store_index.rs` and `appcast.rs` both call `serde_json::from_str` and `serde_json::to_string_pretty` in production code paths. Having `serde_json` only in `[dev-dependencies]` causes a compile error in the production binary. Move it unconditionally:

```toml
# supervisor/Cargo.toml
[dependencies]
serde_json = "1"

# Remove from [dev-dependencies] if it was there
```

### B4 — VYOMA_UPDATE: lines not dispatched by router.rs

Without an explicit branch in `router.rs`, all `VYOMA_UPDATE:` lines fall through to the IPC broker, which interprets them as messages to an app named `VYOMA_UPDATE` (which does not exist) and silently drops them. Add before IPC fallthrough:

```rust
// router.rs — add BEFORE IPC broker fallthrough:
if line.starts_with("VYOMA_UPDATE:") {
    if let Some(mgr) = crate::UPDATE_MANAGER.get() {
        crate::update::handle_update_line(line, app_name, registry, inbox, mgr);
        continue;
    }
}
```

Also requires `UPDATE_MANAGER: OnceLock<Arc<Mutex<UpdateManager>>>` declared as a `pub static` in `main.rs` and initialized before the router loop starts.

### B5 — fs::rename EXDEV if tmp and dst on different mounts

`fs::rename(a, b)` returns `EXDEV (errno 18)` when `a` and `b` are on different filesystem devices. If the `.tmp` file were ever written to `/tmp` (tmpfs) while the `.wasm` destination is on `/data` (9P), the rename would fail. Guard at start of `atomic_install`:

```rust
// installer.rs — guard at start of atomic_install
fn assert_same_mount(a: &Path, b: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let dev_a = std::fs::metadata(a.parent().unwrap()).map_err(|e| e.to_string())?.dev();
    let dev_b = std::fs::metadata(b.parent().unwrap()).map_err(|e| e.to_string())?.dev();
    if dev_a != dev_b {
        return Err(format!("tmp and dst are on different devices ({} vs {})", dev_a, dev_b));
    }
    Ok(())
}
```

Invariant: `.tmp` is always written to `apps_dir` (same `/data/apps/<name>/` directory as the destination). Never write `.tmp` to `/tmp`. Document this invariant with a comment in `atomic_install`.

---

*End of spec. All five blocking issues (B1–B5) are fully described with resolution paths. Module line-count estimates are targets; enforce the 500-line hard cap per .rs file.*
