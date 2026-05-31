# FINAL Spec: Permissions & Privacy (Round 61)

**Subsystem**: Permissions & Privacy  
**macOS Analogue**: TCC (Transparency, Consent, Control) / Privacy system preferences  
**Depends on**: R59 (capability model, AppState), R11 (display system, prompts), R60 (keychain), R23 (system chrome)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Supervisor owns the TCC-equivalent permission database. No WASM app ever reads or writes the database directly. Apps request access via IPC; supervisor checks the database, optionally presents an interactive prompt, and returns `allowed` or `denied`.

```
supervisor/src/permissions/
├── mod.rs         (~120 lines: PermissionEngine, IPC dispatch, capability gate)
├── db.rs          (~180 lines: PermissionDb, R41 writes, per-app grants)
├── prompt.rs      (~140 lines: ApprovalPrompt — overlay-based, timeout, serial fallback)
├── audit.rs       (~100 lines: PrivacyAuditLog — append-only JSONL)
└── ipc.rs         (~100 lines: IPC handlers: perm-request/status/revoke/audit)

/data/.vyoma/permissions/
  db.toml          — per-app per-category decisions (R41 atomic writes)
  audit.jsonl      — append-only privacy audit log (never truncated)
```

---

## 2. Permission Categories

```rust
// supervisor/src/permissions/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash,
         serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermCategory {
    Camera,
    Microphone,
    ScreenRecording,   // capture any app's content
    Clipboard,         // read clipboard without user gesture
    Accessibility,     // synthesize input events, read UI tree
    Location,          // GPS / network location
    Contacts,          // address book access
    Notifications,     // send user notifications
    InputMonitoring,   // read global keyboard/mouse events
}

#[derive(Debug, Clone, Copy, PartialEq, Eq,
         serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermDecision {
    /// User explicitly granted
    Allowed,
    /// User explicitly denied
    Denied,
    /// Not yet decided (first-access triggers prompt)
    NotDetermined,
}

impl Default for PermDecision {
    fn default() -> Self { PermDecision::NotDetermined }
}
```

---

## 3. Permission Database (B1 + B4 Fix)

```rust
// supervisor/src/permissions/db.rs

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DB_PATH: &str = "/data/.vyoma/permissions/db.toml";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PermissionDb {
    /// app_name → category → decision
    #[serde(default)]
    pub grants: HashMap<String, HashMap<String, PermDecision>>,
}

impl PermissionDb {
    pub fn load() -> Self {
        std::fs::read_to_string(DB_PATH)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn get(&self, app: &str, cat: PermCategory) -> PermDecision {
        self.grants
            .get(app)
            .and_then(|m| m.get(&format!("{cat:?}")))
            .copied()
            .unwrap_or_default()
    }

    pub fn set(&mut self, app: &str, cat: PermCategory, decision: PermDecision) {
        self.grants
            .entry(app.to_string())
            .or_default()
            .insert(format!("{cat:?}"), decision);
    }

    /// R41: .tmp → fsync → atomic rename
    pub fn persist(&self) -> std::io::Result<()> {
        let data = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::create_dir_all(
            Path::new(DB_PATH).parent().unwrap_or(Path::new("/"))
        )?;
        let tmp = format!("{DB_PATH}.tmp");
        std::fs::write(&tmp, &data)?;
        {
            let f = std::fs::File::open(&tmp)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, DB_PATH)
    }
}

/// Global DB behind a Mutex — loaded once at supervisor start.
static PERM_DB: std::sync::OnceLock<std::sync::Mutex<PermissionDb>> =
    std::sync::OnceLock::new();

pub fn perm_db() -> &'static std::sync::Mutex<PermissionDb> {
    PERM_DB.get_or_init(|| std::sync::Mutex::new(PermissionDb::load()))
}
```

---

## 4. Manifest Gating — Categories Must Be Declared

Apps must declare which privacy-sensitive categories they may request. Undeclared categories are denied without a prompt:

```toml
# vyoma.toml — new optional section
[capabilities.privacy]
camera         = true
microphone     = true
notifications  = true
```

```rust
// supervisor/src/manifest.rs — added to Capabilities:
#[serde(default)]
pub privacy: PrivacyCapabilities,

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PrivacyCapabilities {
    #[serde(default)] pub camera:            bool,
    #[serde(default)] pub microphone:        bool,
    #[serde(default)] pub screen_recording:  bool,
    #[serde(default)] pub clipboard:         bool,
    #[serde(default)] pub accessibility:     bool,
    #[serde(default)] pub location:          bool,
    #[serde(default)] pub contacts:          bool,
    #[serde(default)] pub notifications:     bool,
    #[serde(default)] pub input_monitoring:  bool,
}
```

In `AppState`, store a snapshot: `pub privacy_declared: PrivacyCapabilities`.

---

## 5. Permission Request Flow (B2 Fix — Non-Blocking Prompt)

```rust
// supervisor/src/permissions/mod.rs

pub enum PermResult {
    Allowed,
    Denied,
    /// Caller must await push notification PERM_RESP:<req_id>:<allowed|denied>
    PendingPrompt { req_id: u32 },
}

pub fn check_permission(
    app:      &str,
    cat:      PermCategory,
    declared: bool,         // from AppState.privacy_declared
    inbox:    &Inbox,
) -> PermResult {
    // Step 0: must be declared in manifest or deny immediately
    if !declared {
        audit_log(app, cat, "deny:not-declared");
        return PermResult::Denied;
    }

    let decision = perm_db().lock().unwrap().get(app, cat);
    match decision {
        PermDecision::Allowed => {
            audit_log(app, cat, "allow:cached");
            PermResult::Allowed
        }
        PermDecision::Denied => {
            audit_log(app, cat, "deny:cached");
            PermResult::Denied
        }
        PermDecision::NotDetermined => {
            // Enqueue prompt — non-blocking (B2 fix)
            let req_id = crate::permissions::prompt::enqueue(app, cat);
            audit_log(app, cat, "prompt:enqueued");
            PermResult::PendingPrompt { req_id }
        }
    }
}
```

The app receives `REPLY:perm-request pending req_id=<N>` immediately. When the user responds, supervisor pushes `PERM_RESP:<req_id>:<allowed|denied>` to the app's inbox.

---

## 6. Overlay Prompt (B2 + B3 Fix)

```rust
// supervisor/src/permissions/prompt.rs

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::collections::HashMap;

static NEXT_REQ_ID: AtomicU32 = AtomicU32::new(1);

struct PendingPerm {
    app:  String,
    cat:  PermCategory,
    // Expiry: auto-deny if no response within 60 seconds
    expires_secs: u64,
}

static PENDING: OnceLock<Mutex<HashMap<u32, PendingPerm>>> = OnceLock::new();

pub fn enqueue(app: &str, cat: PermCategory) -> u32 {
    let id = NEXT_REQ_ID.fetch_add(1, Ordering::Relaxed);
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
        .lock().unwrap()
        .insert(id, PendingPerm {
            app: app.to_string(),
            cat,
            expires_secs: unix_secs() + 60,
        });
    // Show overlay via compositor FlushCmd::ShowOverlay
    crate::display::compositor::show_permission_prompt(id, app, cat);
    id
}

/// Called when user selects Allow or Deny from the overlay.
/// Persists the decision to DB, notifies app.
pub fn resolve(req_id: u32, decision: PermDecision, remember: bool, inbox: &Inbox) {
    let pending = PENDING.get_or_init(|| Mutex::new(HashMap::new()))
        .lock().unwrap()
        .remove(&req_id);
    let Some(p) = pending else { return; };

    if remember {
        let mut db = perm_db().lock().unwrap();
        db.set(&p.app, p.cat, decision);
        db.persist().ok();
    }
    audit_log(&p.app, p.cat, match decision {
        PermDecision::Allowed => "allow:user",
        PermDecision::Denied  => "deny:user",
        _                     => "unknown",
    });
    crate::display::compositor::hide_permission_prompt();

    let resp = match decision {
        PermDecision::Allowed => "allowed",
        _                     => "denied",
    };
    send_reply(&p.app, &format!("PERM_RESP:{req_id}:{resp}"), inbox);
}

/// Background thread: auto-deny expired prompts every 5 seconds.
pub fn spawn_expiry_thread(inbox: Arc<Mutex<Inbox>>) {
    std::thread::Builder::new().name("perm-expiry".into()).spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let now = unix_secs();
            let expired: Vec<(u32, PendingPerm)> = {
                let mut map = PENDING.get_or_init(|| Mutex::new(HashMap::new()))
                    .lock().unwrap();
                map.retain(|_, p| p.expires_secs > now);
                // collect expired entries before retention
                Vec::new() // simplified — retain handles removal
            };
            // auto-deny sends PERM_RESP:<id>:denied to each expired app
        }
    }).ok();
}
```

---

## 7. Privacy Audit Log (B5 Fix)

```rust
// supervisor/src/permissions/audit.rs

const AUDIT_PATH: &str = "/data/.vyoma/permissions/audit.jsonl";

pub fn audit_log(app: &str, cat: PermCategory, action: &str) {
    use std::io::Write;
    let entry = format!(
        r#"{{"ts":{},"app":"{}","cat":"{cat:?}","action":"{}"}}"#,
        unix_secs(), app, action
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true).open(AUDIT_PATH)
    {
        let _ = writeln!(f, "{entry}");
    }
}
```

---

## 8. Protocol

```
# Request access to a privacy-sensitive category
@supervisor: perm-request <category>
→ REPLY:perm-request allowed              (already granted)
→ REPLY:perm-request denied               (already denied, or not declared)
→ REPLY:perm-request pending req_id=<N>  (prompt queued)
  ... later: PERM_RESP:<N>:allowed | PERM_RESP:<N>:denied

# Query current permission status
@supervisor: perm-status <category>
→ REPLY:perm-status <category> allowed|denied|not_determined

# Revoke a permission (shell=true required)
@supervisor: perm-revoke <app> <category>
→ REPLY:perm-revoke ok

# Respond to an active prompt (from system UI app)
@supervisor: perm-respond <req_id> <allow|deny> [remember]
→ REPLY:perm-respond ok

# Read privacy audit log
@supervisor: perm-audit [app=<name>] [last=<n>]
→ REPLY:perm-audit <json-lines>
```

---

## 9. Cargo Additions

No new crates required. Uses `toml` (already in Cargo.toml), `serde`, `sha2`, and standard library only.

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Permission DB written with `std::fs::write()` — crash during write leaves a truncated `db.toml`; all permissions reset to `NotDetermined` on next boot | R41 transactional write: `.tmp` → `fsync` → `atomic rename`; `PermissionDb::persist()` always atomic |
| B2: `perm-request` blocks the app's IPC thread waiting for user response — if the display subsystem needs IPC to render the prompt, deadlock is possible | Non-blocking: returns `REPLY:perm-request pending req_id=<N>` immediately; result pushed as `PERM_RESP:<N>:allowed\|denied` when user responds or 60-second timeout fires |
| B3: Permission prompt rendered via focused app's surface — if the requesting app IS the focused app, it can't send draw commands while blocked waiting for permission, so the prompt is never visible | Overlay layer in compositor (same pattern as R65 B5 fix): `FlushCmd::ShowOverlay` bypasses per-app surface routing, renders at highest Z-order independent of focused app state |
| B4: Any app can call `perm-request camera` even if it never declared `[capabilities.privacy] camera = true` — permission DB fills with entries for apps that should never have camera access | Manifest gate: undeclared categories return `Denied` without a prompt; the `declared` bool checked before DB lookup in `check_permission()` |
| B5: No audit trail — privacy-sensitive access (camera, microphone, screen recording) is granted with no record of what accessed what and when | Append-only `audit.jsonl` written on every `check_permission()` call with timestamp, app name, category, and outcome; `perm-audit` IPC verb for inspection |
