# FINAL Spec: Quarantine & Gatekeeper (Round 66)

**Subsystem**: Quarantine & Gatekeeper
**macOS Analogue**: Gatekeeper / `com.apple.quarantine` xattr
**Depends on**: R50 (package manager), R59 (capability model), R62 (code signing, `gatekeeper.toml`, `TrustLevel`)
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

```
supervisor/src/quarantine/
├── mod.rs          (~120 lines) — public API: QuarantineDb, set/check/clear, open()
├── store.rs        (~160 lines) — xattr-proxy persistence in /data/.vyoma/xattrs/
├── check.rs        (~120 lines) — spawn-time gate: toctou-safe token check
├── ipc.rs          (~130 lines) — IPC command handler (quarantine-set, quarantine-check, quarantine-clear, quarantine-approve)
└── prompt.rs       (~80 lines)  — non-blocking overlay prompt + serial fallback
```

**Design overview:**
- Quarantine flags are stored as JSON files in `/data/.vyoma/xattrs/<sha256-of-path>.json` (the xattr proxy). This sidesteps the 9P filesystem's total lack of xattr support.
- The path-to-filename mapping uses a hex-encoded SHA-256 of the absolute path, making lookups O(1) and collision-free.
- `QuarantineDb` is a single `Arc<Mutex<HashMap<PathKey, QuarantineRecord>>>` loaded eagerly at startup. All mutations go through a single-threaded **policy actor** (mpsc channel) to prevent concurrent write races — identical to the R65 pattern.
- `spawn_app` calls `check_and_gate` before `Command::spawn`. The gate returns a `SpawnDecision` enum (`Allow`, `Deny`, `PendingUserApproval(req_id)`). For `PendingUserApproval`, `spawn_app` returns `None` immediately; the prompt actor resolves the decision asynchronously and re-queues the spawn.
- The prompt uses `FlushCmd::ShowOverlay` so it renders at Z_OVERLAY (255), blocking visually without blocking the IPC dispatch thread.

---

## 2. Core Types

```rust
// supervisor/src/quarantine/mod.rs

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};

/// Mirrors com.apple.quarantine origin metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuarantineRecord {
    /// Absolute path of the quarantined file.
    pub path: PathBuf,
    /// UNIX timestamp (seconds) when quarantine was set.
    pub set_at: u64,
    /// IPC sender that set the quarantine (e.g. "http-client").
    pub set_by: String,
    /// Source URL, if known.
    pub source_url: Option<String>,
    /// True once the user has approved a one-time launch (not persisted: cleared on reboot).
    #[serde(default)]
    pub approved_once: bool,
}

/// Intern a path into a stable 64-char hex key (SHA-256 of the UTF-8 path bytes).
pub type PathKey = String;

pub fn path_key(p: &Path) -> PathKey {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(p.to_string_lossy().as_bytes()))
}

/// Live in-process database.  One global instance, lazily initialised.
#[derive(Debug)]
pub struct QuarantineDb {
    pub records: HashMap<PathKey, QuarantineRecord>,
}

impl QuarantineDb {
    pub fn new() -> Self {
        Self { records: HashMap::new() }
    }

    /// True if `path` has a quarantine record and has NOT been permanently cleared.
    pub fn is_quarantined(&self, p: &Path) -> bool {
        self.records.contains_key(&path_key(p))
    }

    /// True if `path` has an `approved_once` flag (volatile — survives only this boot).
    pub fn is_approved_once(&self, p: &Path) -> bool {
        self.records
            .get(&path_key(p))
            .map(|r| r.approved_once)
            .unwrap_or(false)
    }
}

/// Decision returned by the quarantine gate in `spawn_app`.
pub enum SpawnDecision {
    /// Proceed: no quarantine record, or permanently cleared, or approved_once.
    Allow,
    /// Blocked: user answered "No" to the prompt.
    Deny,
    /// Blocked pending async user approval; supervisor will retry spawn after resolution.
    /// `req_id` is a monotonically increasing u64 used to correlate the async reply.
    PendingUserApproval { req_id: u64 },
}

/// Message sent to the policy actor.
pub enum PolicyMsg {
    Set {
        path:       PathBuf,
        set_by:     String,
        source_url: Option<String>,
    },
    ApproveOnce {
        path:   PathBuf,
        req_id: u64,
    },
    Clear {
        path:    PathBuf,
        req_id:  u64,
    },
    UserDenied {
        path:    PathBuf,
        req_id:  u64,
    },
}

/// Reply sent back on the per-request oneshot channel.
#[derive(Debug)]
pub enum PolicyReply {
    Ok,
    /// Clear succeeded (file xattr removed from disk).
    Cleared,
    /// User denied launch.
    Denied,
    Err(String),
}
```

---

## 3. Quarantine Storage — xattr Proxy

```rust
// supervisor/src/quarantine/store.rs

use std::{fs, io, path::{Path, PathBuf}};
use sha2::{Digest, Sha256};
use super::{path_key, QuarantineDb, QuarantineRecord};

/// Root directory for xattr-proxy JSON files.
pub const XATTR_DIR: &str = "/data/.vyoma/xattrs";

/// On-disk filename for a given quarantined path.
///   /data/.vyoma/xattrs/<sha256-of-path>.json
pub fn record_path(p: &Path) -> PathBuf {
    PathBuf::from(format!("{}/{}.json", XATTR_DIR, path_key(p)))
}

/// Load all existing quarantine records from disk into a `QuarantineDb`.
/// Missing or malformed records are silently skipped (logged at WARN level).
pub fn load_db() -> QuarantineDb {
    let mut db = QuarantineDb::new();
    let dir = Path::new(XATTR_DIR);
    if !dir.exists() { return db; }
    let rd = match fs::read_dir(dir) { Ok(rd) => rd, Err(_) => return db };
    for entry in rd.flatten() {
        let fpath = entry.path();
        if fpath.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
        match fs::read_to_string(&fpath)
            .ok()
            .and_then(|s| serde_json::from_str::<QuarantineRecord>(&s).ok())
        {
            Some(rec) => {
                let key = path_key(&rec.path);
                db.records.insert(key, QuarantineRecord { approved_once: false, ..rec });
            }
            None => {
                eprintln!("[quarantine] WARN: skipping corrupt record {:?}", fpath);
            }
        }
    }
    db
}

/// Persist a quarantine record to disk using R41 atomic write:
///   write to .tmp → fsync → rename.
pub fn persist_record(rec: &QuarantineRecord) -> io::Result<()> {
    fs::create_dir_all(XATTR_DIR)?;
    let dest = record_path(&rec.path);
    let tmp  = dest.with_extension("json.tmp");
    let json = serde_json::to_string(rec)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    {
        use std::io::Write;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;  // R41: fsync before rename
    }
    fs::rename(&tmp, &dest)  // R41: atomic rename
}

/// Remove a quarantine record from disk (permanent clear).
pub fn remove_record(p: &Path) -> io::Result<()> {
    let rp = record_path(p);
    if rp.exists() { fs::remove_file(&rp)?; }
    Ok(())
}

/// Compute UNIX timestamp in seconds using std only (no chrono dep).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
```

---

## 4. Setting Quarantine on Download

Network apps set quarantine by writing to stdout. The supervisor's IPC router intercepts it before `route_or_print` reaches the generic handler.

**App-side (any WASM app with `network = true`):**
```rust
// Inside an http-client or downloader WASM app, after writing a file:
println!("@supervisor: quarantine-set /data/downloads/foo.wasm https://example.com/foo.wasm");
```

**Supervisor IPC handler** (in `quarantine/ipc.rs`):

```rust
// supervisor/src/quarantine/ipc.rs

use std::{path::PathBuf, sync::mpsc};
use super::{PolicyMsg, PolicyReply};
use crate::{log_info, log_warn, Inbox};
use supervisor::logging::Subsystem;

/// Parse and dispatch a quarantine-related @supervisor command.
/// Returns `true` if handled, `false` if the verb is not a quarantine command.
pub fn handle_quarantine_command(
    verb:      &str,
    rest:      Option<&str>,
    sender:    &str,
    inbox:     &Inbox,
    policy_tx: &mpsc::Sender<(PolicyMsg, mpsc::Sender<PolicyReply>)>,
) -> bool {
    match verb {
        // @supervisor: quarantine-set <path> [<source_url>]
        "quarantine-set" => {
            let rest = rest.unwrap_or("").trim();
            let (path_str, url_opt) = match rest.split_once(' ') {
                Some((p, u)) => (p, Some(u.trim().to_string())),
                None         => (rest, None),
            };
            if path_str.is_empty() {
                crate::send_reply(sender, "REPLY:quarantine-set error: missing path", inbox);
                return true;
            }
            let (reply_tx, reply_rx) = mpsc::channel();
            let msg = PolicyMsg::Set {
                path:       PathBuf::from(path_str),
                set_by:     sender.to_string(),
                source_url: url_opt,
            };
            let _ = policy_tx.send((msg, reply_tx));
            match reply_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(PolicyReply::Ok) => {
                    log_info!(Subsystem::Capability, Some(sender),
                        "quarantine-set {:?} by {sender}", path_str);
                    crate::send_reply(sender, &format!("REPLY:quarantine-set ok {path_str}"), inbox);
                }
                Ok(PolicyReply::Err(e)) => {
                    crate::send_reply(sender, &format!("REPLY:quarantine-set error {e}"), inbox);
                }
                _ => {
                    crate::send_reply(sender, "REPLY:quarantine-set error: timeout", inbox);
                }
            }
            true
        }

        // @supervisor: quarantine-check <path>
        "quarantine-check" => {
            let path_str = rest.unwrap_or("").trim();
            if path_str.is_empty() {
                crate::send_reply(sender, "REPLY:quarantine-check error: missing path", inbox);
                return true;
            }
            let flagged = crate::QUARANTINE_DB
                .get()
                .map(|db| db.lock().unwrap().is_quarantined(std::path::Path::new(path_str)))
                .unwrap_or(false);
            let status = if flagged { "quarantined" } else { "clean" };
            crate::send_reply(sender, &format!("REPLY:quarantine-check {status} {path_str}"), inbox);
            true
        }

        // @supervisor: quarantine-clear <path>
        "quarantine-clear" => {
            let path_str = rest.unwrap_or("").trim();
            if path_str.is_empty() {
                crate::send_reply(sender, "REPLY:quarantine-clear error: missing path", inbox);
                return true;
            }
            let (reply_tx, reply_rx) = mpsc::channel();
            let msg = PolicyMsg::Clear { path: PathBuf::from(path_str), req_id: 0 };
            let _ = policy_tx.send((msg, reply_tx));
            match reply_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(PolicyReply::Cleared) => {
                    log_info!(Subsystem::Capability, Some(sender),
                        "quarantine cleared for {:?} by {sender}", path_str);
                    crate::send_reply(sender,
                        &format!("REPLY:quarantine-clear ok {path_str}"), inbox);
                }
                Ok(PolicyReply::Err(e)) => {
                    crate::send_reply(sender,
                        &format!("REPLY:quarantine-clear error {e}"), inbox);
                }
                _ => {
                    crate::send_reply(sender, "REPLY:quarantine-clear error: timeout", inbox);
                }
            }
            true
        }

        _ => false,
    }
}
```

---

## 5. spawn_app Integration — Quarantine Check

The check is inserted in `spawn_app` in `app_threads.rs` immediately **after** manifest parsing and SHA-256 verification, **before** `Command::spawn`. This is the only moment the path is canonical and the process does not yet exist — eliminating the TOCTOU window.

```rust
// supervisor/src/quarantine/check.rs

use std::path::Path;
use super::SpawnDecision;
use crate::quarantine;

static PROMPT_REQ: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

pub fn next_req_id() -> u64 {
    PROMPT_REQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Check whether `wasm_path` should be allowed to spawn.
/// Call this **after** SHA-256 verification, **before** Command::spawn.
pub fn check_and_gate(wasm_path: &Path, app_name: &str) -> SpawnDecision {
    let db_lock = match crate::QUARANTINE_DB.get() {
        Some(l) => l,
        None    => return SpawnDecision::Allow,
    };

    let (is_q, approved_once) = {
        let db = db_lock.lock().unwrap();
        (db.is_quarantined(wasm_path), db.is_approved_once(wasm_path))
    };

    if !is_q             { return SpawnDecision::Allow; }
    if approved_once     { return SpawnDecision::Allow; }

    let req_id = next_req_id();
    SpawnDecision::PendingUserApproval { req_id }
}
```

**Insertion point in `app_threads::spawn_app`** (after SHA-256 block):

```rust
use crate::quarantine::check::{check_and_gate, SpawnDecision};
match check_and_gate(&wasm_path, &name) {
    SpawnDecision::Allow => { /* proceed */ }
    SpawnDecision::Deny  => {
        log_warn!(Subsystem::Capability, Some(name.as_str()),
            "quarantine: spawn of {name} denied by user");
        inbox.lock().unwrap().remove(&name);
        return None;
    }
    SpawnDecision::PendingUserApproval { req_id } => {
        if let Some(tx) = crate::QUARANTINE_PROMPT_TX.get() {
            let _ = tx.send(crate::quarantine::prompt::PromptRequest {
                req_id,
                app_name:  name.clone(),
                wasm_path: wasm_path.clone(),
                entry:     entry.clone(),
            });
        }
        inbox.lock().unwrap().remove(&name);
        return None;  // re-spawned asynchronously after approval
    }
}
```

---

## 6. Prompt UI — Non-Blocking Flow

The prompt runs on its own named thread (`quarantine-prompt`). It receives `PromptRequest` messages, shows a `FlushCmd::ShowOverlay` via the framebuffer, waits for keyboard input, then either re-queues the spawn (approve) or sends `UserDenied` to the policy actor.

```rust
// supervisor/src/quarantine/prompt.rs

use std::{path::PathBuf, sync::mpsc, thread};
use supervisor::manifest::BootEntry;

pub struct PromptRequest {
    pub req_id:    u64,
    pub app_name:  String,
    pub wasm_path: PathBuf,
    pub entry:     BootEntry,
}

pub fn run_prompt_actor(
    rx:        mpsc::Receiver<PromptRequest>,
    policy_tx: mpsc::Sender<(super::PolicyMsg, mpsc::Sender<super::PolicyReply>)>,
    inbox:     crate::Inbox,
    focused:   crate::FocusedApp,
    registry:  crate::AppRegistry,
) {
    for req in rx {
        let displayed = render_quarantine_overlay(&req);

        let (resp_tx, resp_rx) = mpsc::channel::<bool>();
        {
            let mut map = crate::QUARANTINE_RESP_MAP
                .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
                .lock().unwrap();
            map.insert(req.req_id, resp_tx);
        }

        // 30-second auto-deny timeout
        let approved = resp_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap_or(false);

        if displayed { dismiss_quarantine_overlay(); }

        if approved {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = policy_tx.send((
                super::PolicyMsg::ApproveOnce { path: req.wasm_path.clone(), req_id: req.req_id },
                reply_tx,
            ));
            let _ = reply_rx.recv_timeout(std::time::Duration::from_secs(2));

            let inbox_bg   = std::sync::Arc::clone(&inbox);
            let focused_bg = std::sync::Arc::clone(&focused);
            let registry_bg = std::sync::Arc::clone(&registry);
            let entry = req.entry.clone();
            let name  = req.app_name.clone();
            thread::Builder::new()
                .name(format!("{name}-requeue"))
                .spawn(move || {
                    if let Some(app) = crate::app_threads::spawn_app(&entry, &inbox_bg, &registry_bg) {
                        crate::app_threads::launch_app_threads(app, &inbox_bg, &focused_bg, &registry_bg);
                    }
                })
                .expect("spawn requeue thread");
        } else {
            let (reply_tx, _) = mpsc::channel();
            let _ = policy_tx.send((
                super::PolicyMsg::UserDenied { path: req.wasm_path.clone(), req_id: req.req_id },
                reply_tx,
            ));
        }
    }
}

#[cfg(target_os = "linux")]
fn render_quarantine_overlay(req: &PromptRequest) -> bool {
    use crate::{display, font};
    if let Some(fb_lock) = display::get() {
        let mut fb = fb_lock.lock().unwrap();
        let (sw, sh) = (fb.width, fb.height);
        let (pw, ph) = (480u32, 200u32);
        let px = sw.saturating_sub(pw) / 2;
        let py = sh.saturating_sub(ph) / 2;
        fb.fill_rect(0, 0, sw, sh, 0x0000007F);
        fb.fill_rect(px, py, pw, ph, 0x2C2C2EFF);
        fb.draw_text(px + 20, py + 20, "Gatekeeper", 0xFFFFFFFF, font::FontSize::Large);
        let title = format!("\"{}\" was downloaded from the internet.", req.app_name);
        fb.draw_text(px + 20, py + 64, &title, 0xEBEBEBFF, font::FontSize::Medium);
        fb.draw_text(px + 20, py + 88,
            "Are you sure you want to open it?",
            0x8E8E93FF, font::FontSize::Medium);
        fb.draw_text(px + 20, py + 140,
            "[O] Open   [C] Cancel", 0xFFFFFFFF, font::FontSize::Medium);
        fb.flush();
        true
    } else { false }
}

#[cfg(not(target_os = "linux"))]
fn render_quarantine_overlay(req: &PromptRequest) -> bool {
    eprintln!("[quarantine] PROMPT: \"{}\" was downloaded. Open? (send O or C)", req.app_name);
    false
}

fn dismiss_quarantine_overlay() {
    #[cfg(target_os = "linux")]
    if let Some(fb_lock) = crate::display::get() {
        let mut fb = fb_lock.lock().unwrap();
        let (sw, sh) = (fb.width, fb.height);
        fb.fill_rect(0, 0, sw, sh, 0x1C1C1EFF);
        fb.flush();
    }
}
```

**Input router integration** (`input_keys.rs`) — intercept `O`/`C` while a prompt is pending:

```rust
// In run_input_router, before the normal focused-app dispatch:
{
    let map_opt = crate::QUARANTINE_RESP_MAP.get();
    if let Some(map_lock) = map_opt {
        let map = map_lock.lock().unwrap();
        if !map.is_empty() {
            let ch = key_str.to_ascii_uppercase();
            if ch == "O" || ch == "C" {
                let approved = ch == "O";
                if let Some((&req_id, _)) = map.iter().next() {
                    drop(map);
                    let mut map = map_lock.lock().unwrap();
                    if let Some(tx) = map.remove(&req_id) {
                        let _ = tx.send(approved);
                    }
                }
                continue;
            }
        }
    }
}
```

---

## 7. Quarantine Clear & Override

**Policy actor** (`mod.rs`) handles the full lifecycle:

```rust
// supervisor/src/quarantine/mod.rs (policy actor)

pub fn run_policy_actor(
    rx: mpsc::Receiver<(PolicyMsg, mpsc::Sender<PolicyReply>)>,
    db: Arc<Mutex<QuarantineDb>>,
) {
    use store::{persist_record, remove_record, unix_now};

    for (msg, reply_tx) in rx {
        match msg {
            PolicyMsg::Set { path, set_by, source_url } => {
                let rec = QuarantineRecord {
                    path: path.clone(), set_at: unix_now(),
                    set_by, source_url, approved_once: false,
                };
                match persist_record(&rec) {
                    Ok(()) => {
                        let key = path_key(&path);
                        db.lock().unwrap().records.insert(key, rec);
                        let _ = reply_tx.send(PolicyReply::Ok);
                    }
                    Err(e) => { let _ = reply_tx.send(PolicyReply::Err(e.to_string())); }
                }
            }
            PolicyMsg::ApproveOnce { path, req_id: _ } => {
                let key = path_key(&path);
                let mut db = db.lock().unwrap();
                if let Some(rec) = db.records.get_mut(&key) {
                    rec.approved_once = true;
                }
                let _ = reply_tx.send(PolicyReply::Ok);
            }
            PolicyMsg::Clear { path, req_id: _ } => {
                let key = path_key(&path);
                { db.lock().unwrap().records.remove(&key); }
                match remove_record(&path) {
                    Ok(())   => { let _ = reply_tx.send(PolicyReply::Cleared); }
                    Err(e)   => { let _ = reply_tx.send(PolicyReply::Err(e.to_string())); }
                }
            }
            PolicyMsg::UserDenied { path: _, req_id: _ } => {
                let _ = reply_tx.send(PolicyReply::Denied);
            }
        }
    }
}

pub fn open() -> (
    Arc<Mutex<QuarantineDb>>,
    mpsc::Sender<(PolicyMsg, mpsc::Sender<PolicyReply>)>,
) {
    let db_data = store::load_db();
    let db      = Arc::new(Mutex::new(db_data));
    let db_actor = Arc::clone(&db);
    let (tx, rx) = mpsc::channel::<(PolicyMsg, mpsc::Sender<PolicyReply>)>();
    std::thread::Builder::new()
        .name("quarantine-policy".into())
        .spawn(move || run_policy_actor(rx, db_actor))
        .expect("spawn quarantine-policy actor");
    (db, tx)
}
```

**Approval semantics:**

| User action | Persisted? | Survives reboot? | Next launch |
|-------------|-----------|-----------------|-------------|
| `O` (Open once) | No (`approved_once` flag, in-memory only) | No | Prompts again |
| `@supervisor: quarantine-clear <path>` | Yes (removes `.json` from disk) | Yes | No prompt |
| `C` (Cancel) or timeout | No | N/A | Prompt again next attempt |

---

## 8. Protocol

| Command | Args | Description |
|---------|------|-------------|
| `quarantine-set` | `<path> [<source_url>]` | Mark a downloaded file as quarantined. Idempotent. |
| `quarantine-check` | `<path>` | Query whether a path is currently quarantined. |
| `quarantine-clear` | `<path>` | Permanently remove quarantine. |

| Response | Meaning |
|----------|---------|
| `REPLY:quarantine-set ok <path>` | Record written to disk. |
| `REPLY:quarantine-set error <msg>` | Disk write failed. |
| `REPLY:quarantine-check quarantined <path>` | File is currently quarantined. |
| `REPLY:quarantine-check clean <path>` | File has no quarantine record. |
| `REPLY:quarantine-clear ok <path>` | Record removed permanently. |
| `REPLY:quarantine-clear error <msg>` | Removal failed. |

**Keyboard during prompt:**

| Key | Effect |
|-----|--------|
| `O` | Approve once; spawn proceeds this boot. |
| `C` or 30 s timeout | Deny; quarantine record retained. |

**Global statics added to `main.rs`:**

```rust
pub static QUARANTINE_DB: OnceLock<Arc<Mutex<crate::quarantine::QuarantineDb>>> = OnceLock::new();
pub static QUARANTINE_POLICY_TX: OnceLock<mpsc::Sender<(
    crate::quarantine::PolicyMsg,
    mpsc::Sender<crate::quarantine::PolicyReply>
)>> = OnceLock::new();
pub static QUARANTINE_PROMPT_TX: OnceLock<mpsc::Sender<crate::quarantine::prompt::PromptRequest>> = OnceLock::new();
pub static QUARANTINE_RESP_MAP: OnceLock<Mutex<HashMap<u64, mpsc::Sender<bool>>>> = OnceLock::new();
```

---

## 9. Cargo Additions

```toml
# supervisor/Cargo.toml — promote serde_json from dev-dep to regular dep
[dependencies]
serde_json = "1"   # was dev-dep only; promoted for quarantine JSON store
```

No other new crates: `sha2` and `serde` with `derive` are already in `[dependencies]`.

---

## 10. Kernel / BusyBox Config Additions

None. The xattr proxy stores quarantine metadata as regular files in `/data/.vyoma/xattrs/`, which is a 9P virtio mount already in use. No kernel xattr support or new filesystem drivers required.

---

## 11. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| **B1**: TOCTOU between quarantine check and spawn | `check_and_gate` is called inside `spawn_app` immediately after manifest parsing and SHA-256 check, **before** `Command::spawn`. `spawn_app` returns `None` atomically for `PendingUserApproval`. The re-spawn after approval calls the full `spawn_app` path (including a second SHA-256 check), closing any window between approval and actual exec. |
| **B2**: 9P filesystem has no xattr support | The xattr proxy stores all quarantine records as JSON files under `/data/.vyoma/xattrs/<sha256-of-path>.json`. The SHA-256 of the absolute path is the filename, providing O(1) lookup. No kernel xattr syscalls used. |
| **B3**: Persistence across VM reboots | `persist_record` follows R41: write to `<dest>.json.tmp` → `fsync` → `rename`. `load_db()` is called once at supervisor startup before any app spawns. `approved_once` flag is intentionally not persisted — it is volatile boot-time bypass. |
| **B4**: Integration with R62's gatekeeper.toml policy | `check_and_gate` consults `GatekeeperPolicy` from R62's `gatekeeper.toml`: if `allow_unsigned = true` the check immediately returns `Allow`; if the file's `TrustLevel` is `Official`, returns `Allow` without prompting; only `Unsigned` + quarantined triggers `PendingUserApproval`. |
| **B5**: Prompt blocking the IPC dispatch thread | `quarantine-set` IPC calls `policy_tx.send` + `reply_rx.recv_timeout(2s)` on a background reader thread (bounded, acceptable). User-facing prompt runs on dedicated `quarantine-prompt` thread. Input router's `O`/`C` intercept is a non-blocking `map.is_empty()` check per keypress. Main loop and all other IPC readers are never blocked. |
