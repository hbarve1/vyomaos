# FINAL Spec: App Firewall & Network Policy (Round 65)

**Subsystem**: App Firewall & Network Policy  
**macOS Analogue**: ALF / pfctl / NetworkExtension content filter  
**Depends on**: R51 (networking), R53 (VPN — ip_forward), R57 (hotspot — ap0 NAT), R59 (capability model), R64 (cgroups for kernel enforcement)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

The firewall operates at two layers:

1. **IPC intercept layer** — supervisor intercepts `tcp-connect`/`tcp-connect-host` IPC commands before they hit `ipc_commands/tcp.rs`, enforces per-app policy, logs decisions, and optionally presents an approval prompt.
2. **nftables enforcement layer** — belt-and-suspenders kernel enforcement using cgroup-based matching (per-app Wasmtime child cgroup placement).

```
supervisor/src/firewall/
├── mod.rs          (~120 lines: FirewallEngine, FirewallDecision, init)
├── policy.rs       (~200 lines: PolicyStore, AppPolicy, Rule, PolicyMutation actor)
├── audit.rs        (~80 lines:  AuditLog, append-only JSONL at /data/.vyoma/firewall/)
├── prompt.rs       (~120 lines: PromptQueue, PendingPrompt — non-blocking, deferred reply)
├── nft.rs          (~100 lines: cgroup-based nftables rules via /dev/mapper/control)
├── dns_filter.rs   (~100 lines: DnsBlocklist, wildcard domain matching)
├── rate_limiter.rs (~80 lines:  TokenBucket per (app, direction))
└── ipc.rs          (~120 lines: firewall-status/allow/deny/rule-add/remove/audit/prompt-respond)

/data/.vyoma/firewall/
  policy.toml      — per-app rule sets (R41 transactional writes via policy actor)
  audit.log        — append-only connection audit
  dns_block.txt    — one domain per line; *.prefix supported
```

---

## 2. Core Types

```rust
// supervisor/src/firewall/policy.rs

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct PolicyStore {
    pub apps:           std::collections::HashMap<String, AppPolicy>,
    pub global_default: RuleAction,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppPolicy {
    pub app_name:          String,
    pub rules:             Vec<Rule>,       // ordered: first match wins
    pub default_action:    RuleAction,
    pub max_conn_per_sec:  u32,             // 0 = unlimited
    pub prompt_on_new_host: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Rule {
    pub id:    u32,
    pub host:  Option<String>,
    pub ip:    Option<std::net::IpAddr>,
    pub port:  Option<u16>,
    pub proto: String,    // "tcp" | "udp" | "*"
    pub action: RuleAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq,
         serde::Serialize, serde::Deserialize)]
pub enum RuleAction {
    Allow,
    Deny,
    Prompt,  // pause; await user decision
}
impl Default for RuleAction { fn default() -> Self { RuleAction::Prompt } }
```

---

## 3. FirewallEngine

```rust
// supervisor/src/firewall/mod.rs

pub struct FirewallEngine {
    pub policy:  Arc<RwLock<PolicyStore>>,
    pub audit:   Arc<AuditLog>,
    pub prompts: Arc<PromptQueue>,
    pub rates:   Arc<RateLimiterMap>,
    pub nft:     Arc<NftManager>,
}

pub enum FirewallDecision {
    Allow,
    Deny { reason: String },
    PendingPrompt { prompt_id: u32 },
}

impl FirewallEngine {
    pub fn check_connect(
        &self, app: &str, host: &str, resolved_ip: IpAddr, port: u16, proto: &str,
    ) -> FirewallDecision {
        // 1. DNS blocklist — fast path before any resolution
        if self.dns_filter.is_blocked_for_app(app, host) {
            self.audit.log(app, host, port, "deny:dns-blocklist");
            return FirewallDecision::Deny { reason: format!("dns-blocked:{host}") };
        }
        // 2. Rate limit
        if !self.rates.try_acquire(app) {
            self.audit.log(app, host, port, "deny:rate-limit");
            return FirewallDecision::Deny { reason: "rate-limited".into() };
        }
        // 3. Policy rule match
        let store = self.policy.read().unwrap();
        let action = store.apps.get(app)
            .map(|p| p.evaluate(host, resolved_ip, port, proto))
            .unwrap_or(store.global_default);
        match action {
            RuleAction::Allow  => { self.audit.log(app, host, port, "allow"); FirewallDecision::Allow }
            RuleAction::Deny   => { self.audit.log(app, host, port, "deny:policy"); FirewallDecision::Deny { reason: "policy".into() } }
            RuleAction::Prompt => {
                let id = self.prompts.enqueue(app, host, port);
                FirewallDecision::PendingPrompt { prompt_id: id }
            }
        }
    }
}
```

---

## 4. Cgroup Placement at Spawn (B1 Fix)

The nftables `meta cgroup` enforcement requires each Wasmtime child in its own cgroup. This is added to `lifecycle.rs` after `child.id()` is known:

```rust
// supervisor/src/lifecycle.rs

fn place_in_cgroup(app_name: &str, pid: u32) -> std::io::Result<()> {
    let dir = format!("/sys/fs/cgroup/vyoma/{app_name}");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(format!("{dir}/cgroup.procs"), format!("{pid}\n"))?;
    Ok(())
}

// In spawn_app, after child.id() is known:
if let Some(pid) = child.id() {
    if let Err(e) = place_in_cgroup(&app.name, pid) {
        log_warn!(Subsystem::Firewall, Some(&app.name),
            "cgroup placement failed: {e} — nft firewall inert for this app");
        // Non-fatal: IPC-layer firewall still enforces
    }
}
```

Kernel config additions (beyond existing allnoconfig set):

```
CONFIG_CGROUPS=y
CONFIG_CGROUP_NET_CLASSID=y
CONFIG_NET_CLS_CGROUP=y
```

If cgroupv2 `meta cgroup` is unavailable, fall back to `net_cls` classid: assign classid `0x00010001 + app_index` per app.

---

## 5. Non-Blocking Prompt Path (B2 Fix)

Blocking the app's IPC dispatch thread 30 seconds waiting for user response freezes all IPC for that app. The deferred-reply pattern avoids this:

```rust
// In ipc_commands/tcp.rs intercept — tcp-connect-host verb:

FirewallDecision::PendingPrompt { prompt_id } => {
    // Stash connection intent; return immediately with "pending" reply.
    let conn_id = pending_conns.insert(PendingConn {
        prompt_id,
        sender: sender.to_string(),
        host: host_str.clone(),
        port,
    });
    send_reply(sender,
        &format!("REPLY:tcp-connect pending prompt_id={prompt_id} conn_id={conn_id}"),
        inbox);
    return true;  // IPC thread is free immediately
}
```

When `firewall-prompt-respond <prompt_id> allow` arrives, supervisor completes the dial on a separate connection worker thread and pushes:

```rust
send_reply(&pending.sender,
    &format!("NOTIFY:tcp-connect {conn_id} ok id={stream_id}"), inbox);
```

---

## 6. Policy Mutation Actor (B3 Fix)

Concurrent `firewall-rule-add` calls from multiple apps can race on the in-memory `PolicyStore` write + file persistence. A single-threaded actor serializes all mutations:

```rust
// supervisor/src/firewall/policy.rs

pub enum PolicyMutation {
    AddRule    { app: String, rule: Rule,  reply_tx: oneshot::Sender<u32>   },
    RemoveRule { app: String, id:   u32,   reply_tx: oneshot::Sender<bool>  },
    SetDefault { app: String, action: RuleAction                            },
}

pub fn spawn_policy_actor(
    store: Arc<RwLock<PolicyStore>>,
    policy_path: std::path::PathBuf,
) -> std::sync::mpsc::SyncSender<PolicyMutation> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<PolicyMutation>(32);
    std::thread::Builder::new().name("firewall-policy".into()).spawn(move || {
        for msg in rx {
            let mut s = store.write().unwrap();
            match msg {
                PolicyMutation::AddRule { app, rule, reply_tx } => {
                    let id = s.next_rule_id();
                    s.apps.entry(app).or_insert_with(AppPolicy::default)
                        .rules.push(Rule { id, ..rule });
                    persist_r41(&*s, &policy_path).ok();
                    reply_tx.send(id).ok();
                }
                PolicyMutation::RemoveRule { app, id, reply_tx } => {
                    let found = s.apps.get_mut(&app)
                        .map(|p| { let n = p.rules.len(); p.rules.retain(|r| r.id != id); p.rules.len() < n })
                        .unwrap_or(false);
                    if found { persist_r41(&*s, &policy_path).ok(); }
                    reply_tx.send(found).ok();
                }
                PolicyMutation::SetDefault { app, action } => {
                    s.apps.entry(app).or_insert_with(AppPolicy::default)
                        .default_action = action;
                    persist_r41(&*s, &policy_path).ok();
                }
            }
        }
    }).ok();
    tx
}

fn persist_r41(store: &PolicyStore, path: &std::path::Path) -> std::io::Result<()> {
    let data = toml::to_string(store)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &data)?;
    { let f = std::fs::File::open(&tmp)?; f.sync_all()?; }
    std::fs::rename(&tmp, path)
}
```

All IPC handlers send `PolicyMutation` to the actor and await the oneshot reply. No caller writes `store.write()` directly.

---

## 7. Hostname-First IPC Verb (B4 Fix)

Existing `tcp-connect` receives only an already-resolved `ip:port`. Host-based rules are useless without the original hostname. A new `tcp-connect-host` verb carries the hostname separately:

```rust
// New IPC verb: @supervisor: tcp-connect-host <hostname> <port>

"tcp-connect-host" => {
    let hostname = parts.get(0).copied().unwrap_or("");
    let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);

    // Step 1: DNS filter — no resolution yet; blocked hostnames never resolved
    if firewall.dns_filter.is_blocked_for_app(sender, hostname) {
        send_reply(sender, "REPLY:tcp-connect error firewall:dns-blocked", inbox);
        return true;
    }

    // Step 2: Supervisor-side DNS resolution (not app-visible)
    let resolved: IpAddr = match resolve_host(hostname) {
        Ok(ip) => ip,
        Err(e) => {
            send_reply(sender, &format!("REPLY:tcp-connect error dns:{e}"), inbox);
            return true;
        }
    };

    // Step 3: Firewall check with both hostname and IP available
    match firewall.check_connect(sender, hostname, resolved, port, "tcp") {
        FirewallDecision::Allow => {
            // Dial with already-resolved IP — no second lookup
            match TcpStream::connect((resolved, port)) { /* ... */ }
        }
        FirewallDecision::Deny { reason } => {
            send_reply(sender, &format!("REPLY:tcp-connect error firewall:{reason}"), inbox);
        }
        FirewallDecision::PendingPrompt { prompt_id } => {
            // Non-blocking deferred reply (B2 fix)
            /* ... */
        }
    }
}
```

The DNS filter check happens before resolution — a blocked hostname is never looked up.

---

## 8. Compositor Overlay for Prompt UI (B5 Fix)

If the requesting app is the focused app, it cannot send draw commands while its IPC thread is handling the prompt. The compositor's overlay layer (added in R65, same mechanism as R61 Permissions) bypasses per-app surface routing:

```rust
// supervisor/src/display/compositor.rs (additions)

pub fn show_firewall_prompt(prompt_id: u32, app: &str, host: &str, port: u16) {
    let lines = [
        format!("\"{}\" wants to connect to", app),
        format!("{}:{}", host, port),
    ];
    // FlushCmd::ShowOverlay renders at highest Z-order, independent of focused app
    FLUSH_TX.get().and_then(|tx| {
        tx.send(FlushCmd::ShowOverlay {
            lines: lines.iter().map(String::from).collect(),
            hint:  format!("prompt_id={prompt_id} — type: fw allow/deny"),
        }).ok()
    });
}

pub fn hide_firewall_prompt() {
    FLUSH_TX.get().and_then(|tx| tx.send(FlushCmd::HideOverlay).ok());
}
```

Additionally, a `NOTIFY:firewall-prompt <prompt_id> <app> <host> <port>` message is sent to the system shell app as a keyboard-only fallback.

---

## 9. Rate Limiter

```rust
// supervisor/src/firewall/rate_limiter.rs

pub struct TokenBucket {
    tokens:      f64,
    capacity:    f64,      // = max_conn_per_sec
    refill_rate: f64,      // tokens per second
    last_refill: std::time::Instant,
}

impl TokenBucket {
    pub fn try_acquire(&mut self) -> bool {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 { self.tokens -= 1.0; true } else { false }
    }
}
```

---

## 10. DNS Filtering

```rust
// supervisor/src/firewall/dns_filter.rs

pub struct DnsBlocklist { entries: Vec<DnsEntry> }
enum DnsEntry { Exact(String), Wildcard(String) }

impl DnsBlocklist {
    pub fn load() -> Self {
        let entries = std::fs::read_to_string("/data/.vyoma/firewall/dns_block.txt")
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(|l| {
                if let Some(suffix) = l.strip_prefix("*.") {
                    DnsEntry::Wildcard(format!(".{suffix}"))
                } else {
                    DnsEntry::Exact(l.to_string())
                }
            })
            .collect();
        Self { entries }
    }

    pub fn is_blocked(&self, host: &str) -> bool {
        self.entries.iter().any(|e| match e {
            DnsEntry::Exact(h)    => h == host,
            DnsEntry::Wildcard(s) => host.ends_with(s.as_str()),
        })
    }
}
```

---

## 11. Protocol

```
@supervisor: firewall-status <app>
→ REPLY:firewall-status <app> rules=<n> default=<allow|deny|prompt>

@supervisor: firewall-allow <app> <host_or_*> <port_or_*>
→ REPLY:firewall-allow ok rule_id=<n>

@supervisor: firewall-deny <app> <host_or_*> <port_or_*>
→ REPLY:firewall-deny ok rule_id=<n>

@supervisor: firewall-rule-add <app> <host> <port> <proto> <allow|deny|prompt>
→ REPLY:firewall-rule-add ok rule_id=<n>

@supervisor: firewall-rule-remove <app> <rule_id>
→ REPLY:firewall-rule-remove ok | error not-found

@supervisor: firewall-audit <app> [last=<n>]
→ REPLY:firewall-audit <json-lines>

@supervisor: firewall-prompt-respond <prompt_id> <allow|deny> [remember]
→ REPLY:firewall-prompt-respond ok

@supervisor: tcp-connect-host <hostname> <port>
→ REPLY:tcp-connect ok id=<stream_id>  (existing tcp-connect response format)
→ REPLY:tcp-connect pending prompt_id=<N> conn_id=<C>
→ REPLY:tcp-connect error firewall:<reason>
   ... later: NOTIFY:tcp-connect <conn_id> ok id=<stream_id> | denied
```

---

## 12. Cargo Additions

No new crates required. Uses `libc`, `serde`, `toml`, and standard library only.

---

## 13. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: nftables `meta cgroup` matching requires each Wasmtime child in its own cgroup — current `lifecycle.rs` never places children in cgroups; enforcement layer is entirely inert | `place_in_cgroup(app_name, pid)` called in `spawn_app` after `child.id()`; kernel config: `CONFIG_CGROUPS=y`, `CONFIG_CGROUP_NET_CLASSID=y`; non-fatal (IPC layer still enforces) |
| B2: Prompt approval blocks app's IPC dispatch thread up to 30 seconds — all IPC frozen; display system deadlock possible if prompt rendering also needs IPC | Non-blocking deferred reply: `REPLY:tcp-connect pending prompt_id=<N> conn_id=<C>` returned immediately; dial completed on connection worker thread; result pushed as `NOTIFY:tcp-connect <conn_id>` |
| B3: Concurrent `firewall-rule-add` from multiple apps races on `RwLock<PolicyStore>` write + file persistence — second write silently discards first | Single-threaded policy actor via `mpsc::sync_channel`; all mutations serialized through actor; no caller writes `store.write()` directly; R41 persistence inside actor |
| B4: `tcp-connect` only receives already-resolved `ip:port` — hostname-based rules never match because hostname is discarded before the firewall sees it | New `tcp-connect-host <hostname> <port>` IPC verb: DNS filter checked before resolution (blocked hosts never resolved); firewall receives both hostname and resolved IP for rule matching |
| B5: Approval prompt must render on screen, but if the requesting app is the focused app it cannot send draw commands while its IPC thread handles the prompt | Compositor overlay layer via `FlushCmd::ShowOverlay`: renders at highest Z-order bypassing per-app surface routing; same mechanism as R61 permissions overlay; serial shell fallback via `NOTIFY:firewall-prompt` |
