# FINAL Spec: Sandboxing & Capability Model (Round 59)

**Subsystem**: Sandboxing & Capability Model  
**macOS Analogue**: App Sandbox / entitlements / TCC  
**Depends on**: R03 (IPC), R49 (sandbox FS), R50 (package manager), R51-R55 (networking capabilities)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

WASM memory isolation is the primary sandbox. The entitlement system adds three enforcement layers:

1. **Manifest-declared entitlements** — validated and stored on `AppState` at spawn
2. **IPC token authentication** — prevents confused-deputy relay attacks (B5)
3. **Seccomp allowlist** — capability-aware syscall whitelist replacing existing denylist (B4)

```
supervisor/src/capability/
├── entitlement.rs  (~120 lines: EntitlementSet, Entitlement enum, ShellCommand enum)
├── grant.rs        (~80 lines: GrantKind, temporal grant store at /data/caps/)
├── ratelimit.rs    (~60 lines: token-bucket rate limiter 200 msg/s per sender (B1))
├── audit.rs        (~80 lines: AuditEvent, cap_audit_log to /data/logs/capability-audit.log)
└── token.rs        (~60 lines: per-spawn IPC token, constant_time_eq (B5))
```

---

## 2. Entitlement Set

```rust
// supervisor/src/capability/entitlement.rs

#[derive(Debug, Clone)]
pub enum Entitlement {
    Stdio,
    Filesystem { paths: Vec<String>, readonly: bool },
    Network    { ports: Vec<u16>, egress_only: bool },
    Display,
    Shell      { commands: Vec<ShellCommand> },
    Mouse,
    IpcSend    { targets: Vec<String> },   // explicit allowlist of @<app> targets
    Peripheral(crate::hal::PeripheralCapability),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellCommand {
    Ps, Log, Logf, Logs, Focus, Notify,
    Restart(AppNamePattern),
    Kill(AppNamePattern),
    Shutdown,       // requires explicit grant
    PkgInstall,
    Run,            // spawn new apps — requires System trust (B2 fix)
    Any,            // unrestricted — only for TrustLevel::System
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TrustLevel {
    #[default]
    User,    // user-installed
    System,  // boot.toml entry only
}

#[derive(Debug, Default, Clone)]
pub struct EntitlementSet {
    pub entries: Vec<Entitlement>,
    pub trust:   TrustLevel,
}

impl EntitlementSet {
    pub fn from_manifest(manifest: &AppManifest, trust: TrustLevel) -> Self {
        let caps = &manifest.capabilities;
        let mut entries = Vec::new();
        if caps.stdio      { entries.push(Entitlement::Stdio); }
        if caps.filesystem { entries.push(Entitlement::Filesystem {
            paths: vec!["/data".into()], readonly: false }); }
        if caps.network {
            let port = caps.network_port.unwrap_or(8080);
            entries.push(Entitlement::Network { ports: vec![port], egress_only: false });
        }
        if caps.display  { entries.push(Entitlement::Display); }
        if caps.shell {
            let cmds = match trust {
                TrustLevel::System => vec![ShellCommand::Any],
                TrustLevel::User   => vec![
                    ShellCommand::Ps, ShellCommand::Log, ShellCommand::Logf,
                    ShellCommand::Logs, ShellCommand::Focus, ShellCommand::Notify,
                ],
            };
            entries.push(Entitlement::Shell { commands: cmds });
        }
        if caps.mouse { entries.push(Entitlement::Mouse); }
        // Default IPC: only to supervisor (gated) and replies
        entries.push(Entitlement::IpcSend { targets: vec!["supervisor".into()] });
        EntitlementSet { entries, trust }
    }

    pub fn allows_shell_cmd(&self, cmd: &str) -> bool {
        for e in &self.entries {
            if let Entitlement::Shell { commands } = e {
                if commands.iter().any(|c| matches!(c, ShellCommand::Any) || shell_cmd_matches(c, cmd)) {
                    return true;
                }
            }
        }
        false
    }

    pub fn allows_ipc_to(&self, target: &str) -> bool {
        for e in &self.entries {
            if let Entitlement::IpcSend { targets } = e {
                if targets.iter().any(|t| t == target || t == "*") { return true; }
            }
        }
        false
    }
}
```

`AppState` gains:
```rust
pub entitlements:  EntitlementSet,
pub ipc_token:     String,   // 32-char hex; set at spawn (B5)
```

---

## 3. IPC Broadcast Rate Limit (B1 Fix)

Any `stdio`-only app can flood all other apps at unbounded rate via `@broadcast:`:

```rust
// supervisor/src/capability/ratelimit.rs

static IPC_BUCKETS: OnceLock<Mutex<HashMap<String, BucketState>>> = OnceLock::new();
const MAX_TOKENS: u32 = 200;
const REFILL_HZ:  u32 = 200;

struct BucketState { tokens: u32, last_refill: Instant }

pub fn ipc_allowed(sender: &str) -> bool {
    let buckets = IPC_BUCKETS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = buckets.lock().unwrap();
    let now = Instant::now();
    let b = map.entry(sender.to_string()).or_insert(BucketState {
        tokens: MAX_TOKENS, last_refill: now,
    });
    let new = (b.last_refill.elapsed().as_secs_f32() * REFILL_HZ as f32) as u32;
    if new > 0 { b.tokens = (b.tokens + new).min(MAX_TOKENS); b.last_refill = now; }
    if b.tokens == 0 { return false; }
    b.tokens -= 1;
    true
}
```

In `router.rs`, before routing any `@` message:
```rust
if !crate::capability::ratelimit::ipc_allowed(sender) {
    log_warn!(Subsystem::Ipc, Some(sender), "IPC rate-limit exceeded — dropped");
    return;
}
```

Broadcast requires explicit `IpcSend { targets: ["broadcast"] }` entitlement — not granted by default.

---

## 4. Shell Privilege Escalation Prevention (B2 Fix)

`run` command restricted to System trust + approved path prefixes + `wasm_sha256` required:

```rust
// In handle_supervisor_command "run" branch
"run" => {
    // B2 fix: must have explicit Run permission (System trust only by default)
    let permitted = {
        let reg = app_registry.lock().unwrap();
        reg.get(sender).map(|st| st.lock().unwrap().entitlements.allows_shell_cmd("run"))
            .unwrap_or(false)
    };
    if !permitted {
        cap_audit_log(&AuditEvent::ShellDenied { app: sender.into(), cmd: "run".into() });
        send_reply(sender, "REPLY:error: permission denied: run", inbox);
        return;
    }
    const ALLOWED_PREFIXES: &[&str] = &["/apps/", "/data/apps/", "/etc/vyoma/"];
    let path = parts.get(1).map(|s| s.trim()).unwrap_or("");
    if !ALLOWED_PREFIXES.iter().any(|p| path.starts_with(p)) {
        send_reply(sender, "REPLY:error: run path not in approved location", inbox);
        return;
    }
    let manifest = parse_manifest(Path::new(path))?;
    if manifest.app.wasm_sha256.is_none() {
        send_reply(sender, "REPLY:error: wasm_sha256 required for dynamic run", inbox);
        return;
    }
    // Sub-apps always User trust — no inheritance
    spawn_with_trust(manifest, TrustLevel::User, inbox, app_registry);
}
```

`BootEntry::is_system_entry: bool` added to `manifest.rs` — `true` only for entries from `/etc/vyoma/boot.toml`. All runtime-spawned apps receive `TrustLevel::User`.

---

## 5. Temporal Grant Survival on Restart (B3 Fix)

Temporal grants (from `cap-request` flow) are lost on every crash/restart since `AppState` is rebuilt from scratch:

```rust
// supervisor/src/capability/grant.rs

pub fn persist_temporal_grant(app: &str, cap: &str, expires_secs: Option<u64>) {
    let _ = std::fs::create_dir_all("/data/caps");
    let line = match expires_secs {
        Some(e) => format!("{cap} {e}\n"),
        None    => format!("{cap}\n"),
    };
    let _ = std::fs::OpenOptions::new().create(true).append(true)
        .open(format!("/data/caps/{app}.grants"))
        .map(|mut f| use std::io::Write; f.write_all(line.as_bytes()));
}

pub fn load_temporal_grants(app: &str) -> Vec<Entitlement> {
    let path = format!("/data/caps/{app}.grants");
    let Ok(raw) = std::fs::read_to_string(&path) else { return vec![]; };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    raw.lines().filter_map(|line| {
        let mut p = line.trim().splitn(2, ' ');
        let cap = p.next()?;
        let expires: Option<u64> = p.next().and_then(|s| s.parse().ok());
        if let Some(e) = expires { if e < now { return None; } }
        parse_capability_name(cap).map(|c| c.into_entitlement())
    }).collect()
}
```

In `spawn_app`, after building baseline entitlements:
```rust
for grant in capability::grant::load_temporal_grants(&name) {
    if !entitlements.entries.iter().any(|e| e.same_type(&grant)) {
        entitlements.entries.push(grant);
    }
}
// Notify app its caps were restored
send_reply(&name, "VYOMA_SYSTEM:caps_restored", inbox);
```

---

## 6. Seccomp Allowlist (B4 Fix)

Current denylist is fragile — new syscalls implicitly allowed. Replace with default-deny allowlist:

```rust
// supervisor/src/seccomp.rs — additions to WASMTIME_BASE
// Full list includes sigaltstack (131), futex (202), memfd_create (319),
// set_robust_list (273), getcpu (309), sigaltstack (131) for wasmtime-fiber,
// and audit_mode toggle for development bring-up.

pub fn build_allowlist(profile: &SeccompProfile, audit_mode: bool) -> Vec<SockFilter> {
    let mut allowed: HashSet<u32> = WASMTIME_BASE.iter().copied().collect();
    if profile.network    { allowed.extend(NETWORK_SYSCALLS); }
    if profile.filesystem { allowed.extend(FILESYSTEM_EXTRA); }

    let mut f = vec![
        stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_ARCH),
        jump!(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_X86_64, 1, 0),
        stmt!(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
        stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_NR),
    ];
    // clone3 → ENOSYS (Wasmtime handles this gracefully)
    f.push(jump!(BPF_JMP | BPF_JEQ | BPF_K, 435, 0, 1));
    f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_ERRNO_ENOSYS));

    let mut sorted: Vec<u32> = allowed.into_iter().collect();
    sorted.sort_unstable();
    for nr in sorted {
        f.push(jump!(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1));
        f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
    }
    let default_action = if audit_mode { SECCOMP_RET_LOG } else { SECCOMP_RET_KILL_PROCESS };
    f.push(stmt!(BPF_RET | BPF_K, default_action));
    f
}
```

Enable audit mode via `VYOMA_SECCOMP_AUDIT=1` env var during development. Called in `spawn_app`:
```rust
let profile = SeccompProfile { network: caps.network, filesystem: caps.filesystem };
let audit = std::env::var("VYOMA_SECCOMP_AUDIT").map(|v| v == "1").unwrap_or(false);
let filter = crate::seccomp::build_allowlist(&profile, audit);
unsafe { cmd.pre_exec(move || crate::seccomp::apply(&filter)); }
```

---

## 7. IPC Token — Confused-Deputy Prevention (B5 Fix)

Without tokens, `shell-app` can relay arbitrary commands from `app-A`:

```rust
// supervisor/src/capability/token.rs

pub fn generate_token() -> String {
    let mut b = [0u8; 16];
    let _ = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| { use std::io::Read; f.read_exact(&mut b) });
    b.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
```

In `spawn_app`:
```rust
let token = generate_token();
state.lock().unwrap().ipc_token = token.clone();
// Deliver token on app's stdin before first messages
send_reply(&name, &format!("VYOMA_SYSTEM:token:{token}"), inbox);
```

In `handle_supervisor_command`:
```rust
// Commands must start with "TOKEN:<hex> <actual-command>"
let (token, cmd) = match raw_cmd.split_once(' ') {
    Some((t, c)) if t.starts_with("TOKEN:") => (&t[6..], c),
    _ => {
        // Non-System apps without token are rejected
        let is_system = { /* check TrustLevel */ };
        if !is_system {
            cap_audit_log(&AuditEvent::ShellDenied { app: sender.into(), cmd: raw_cmd.into() });
            send_reply(sender, "REPLY:error: missing IPC token", inbox);
            return;
        }
        ("", raw_cmd)
    }
};
if !token.is_empty() {
    let expected = {
        let reg = app_registry.lock().unwrap();
        reg.get(sender).map(|st| st.lock().unwrap().ipc_token.clone()).unwrap_or_default()
    }; // lock dropped before comparison
    if !constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        cap_audit_log(&AuditEvent::ShellDenied { app: sender.into(), cmd: cmd.into() });
        send_reply(sender, "REPLY:error: invalid IPC token", inbox);
        return;
    }
}
```

Relayed commands carry no (or wrong) token — confused-deputy attack fails at the token check.

---

## 8. Capability Audit Log

```rust
// supervisor/src/capability/audit.rs
pub enum AuditEvent {
    Granted   { app: String, cap: String, kind: GrantKind },
    Denied    { app: String, cap: String, reason: String },
    IpcDenied { sender: String, target: String },
    ShellDenied { app: String, cmd: String },
}
// Written to /data/logs/capability-audit.log (append, created at init)
// Also emitted to supervisor stderr via log_warn!
```

---

## 9. Integration

- **R49 filesystem grants**: `EntitlementSet::allows_path()` checks declared `Filesystem` entries first, then falls through to `check_file_access_grant(app, path, write)` from R49 SQLite table.
- **R61 Permissions/Privacy**: `cap-request` flow currently auto-denies for User-trust apps; R61 replaces this with the interactive TCC prompt.
- **R60 Keychain**: `keychain-*` commands gated by `has_keychain: bool` added to `AppState` (separate from `shell`).

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `@broadcast:` with no capability gate — any `stdio`-only app can flood all app inboxes at unbounded rate | Token-bucket rate limiter 200 msg/s per sender in `router.rs`; broadcast requires explicit `IpcSend { targets: ["broadcast"] }` entitlement |
| B2: `@supervisor: run <path>` with `shell = true` allows user-trust app to spawn arbitrary code with any capabilities | `run` requires `ShellCommand::Run` (System trust default); path must match approved prefixes; `wasm_sha256` required; sub-apps always `TrustLevel::User` |
| B3: Temporal grants from `cap-request` silently lost on crash/restart — app gets no notification that permissions changed | Temporal grants persisted to `/data/caps/<app>.grants` with expiry timestamps; reloaded at spawn; `VYOMA_SYSTEM:caps_restored` sent on load |
| B4: Seccomp denylist silently allows all unlisted syscalls including future ones; missing `sigaltstack`/`futex`/`memfd_create` kills Wasmtime-fiber silently | Replace denylist with default-deny allowlist; add `sigaltstack (131)`, `futex (202)`, `memfd_create (319)`, `set_robust_list (273)`; audit mode via `VYOMA_SECCOMP_AUDIT=1` |
| B5: IPC sender identity unverifiable — any `shell`-capable app can relay `@supervisor:` commands from non-privileged apps (confused-deputy) | Per-spawn 128-bit IPC token delivered on stdin; all `@supervisor:` commands must include `TOKEN:<hex>` prefix; constant-time comparison; relay fails because wrong/missing token |
