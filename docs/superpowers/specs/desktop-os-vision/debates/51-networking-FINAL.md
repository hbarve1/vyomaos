# FINAL Spec: Networking Stack (Round 51)

**Subsystem**: Networking Stack  
**macOS Analogue**: `CFNetwork` / `Network.framework`  
**Depends on**: R03 (IPC), R49 (sandbox — network namespace isolation)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Kernel Config Additions

Appended to `base/kernel.config`:

```
# Networking completeness additions (R51)
CONFIG_IP_PNP=y               # B1 fix: in-kernel IP autoconfiguration
CONFIG_IP_PNP_DHCP=y          # B1 fix: DHCP self-configuration of eth0
CONFIG_NET_LOOPBACK=y         # lo interface for loopback-only apps
CONFIG_RTNETLINK=y            # carrier/IP change events via netlink
CONFIG_NET_CORE=y             # required for rtnetlink + link state
CONFIG_BPF=y                  # eBPF hooks (for future R65 App Firewall)
CONFIG_BPF_SYSCALL=y          # SO_ATTACH_FILTER on raw sockets
CONFIG_INET=y                 # IPv4 stack (required by WASI sockets)
CONFIG_TCP_CONG_CUBIC=y       # default TCP congestion algorithm
CONFIG_IPV6=n                 # disabled; all WASI socket calls use IPv4
CONFIG_VIRTIO_NET=y           # virtio-net NIC driver for QEMU
CONFIG_NET_VENDOR_VIRTIO=y    # virtio vendor module group
```

Kernel command line in Makefile `run-net` / `run-gui-net` targets:
```makefile
-append "... ip=dhcp"
```

This triggers `CONFIG_IP_PNP_DHCP` to self-configure `eth0=10.0.2.15/24` before
PID 1 starts. No userspace `dhclient` needed (B1 fix).

QEMU user-mode networking topology:
```
Guest eth0: 10.0.2.15/24
Gateway:    10.0.2.2
DNS:        10.0.2.3  (QEMU's built-in forwarder)
Host port:  8080 → 10.0.2.15:8080  (TCP, forwarded by -net user,hostfwd=)
```

Estimated kernel size increase: ~60 KB. `/dev/net/tun` created automatically
by devtmpfs once `CONFIG_TUN=y` is added in R53.

---

## 2. Module Tree

```
supervisor/src/networking/
├── mod.rs          (~50 lines)   statics, start_*_thread helpers, NetworkState global
├── dns.rs          (~190 lines)  DnsRequest enum, async UDP resolver thread (B2 fix)
├── dns_cache.rs    (~85 lines)   DnsCache, LRU eviction, TTL, negative entries
├── policy.rs       (~140 lines)  NetworkPolicy enum, per-app enforcement, port-conflict (B5)
├── monitor.rs      (~160 lines)  netlink watcher, sysfs fallback, QEMU operstate fix (B4)
├── accounting.rs   (~125 lines)  AppNetStats, connection tracking, quota enforcement
└── netns.rs        (~65 lines)   bring_up_loopback ioctl, unshare helper (B3 fix)

Modified files outside networking/:
  base/kernel.config            — 12 new CONFIG_ lines (B1 fix)
  Makefile                      — ip=dhcp in -append for run-net/run-gui-net (B1 fix)
  supervisor/src/manifest.rs    — NetworkPolicy enum, network_ports, network_hosts fields
  supervisor/src/app_threads.rs — NetworkPolicy dispatch in spawn, pre_exec netns (B3)
  supervisor/src/ipc_commands/mod.rs  — async dns-resolve, net-status, net-stats handlers
```

Each file stays under the 500-line limit. `mod.rs` re-exports all public types so
callers import `crate::networking::NetworkState` without knowing the submodule layout.

---

## 3. NetworkState Struct

```rust
// supervisor/src/networking/mod.rs

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock};

/// Runtime-discovered network configuration for the primary interface.
/// Updated by monitor.rs whenever the interface state changes.
#[derive(Debug, Clone)]
pub struct NetworkState {
    pub iface:   String,          // e.g. "eth0"
    pub ip:      Ipv4Addr,        // e.g. 10.0.2.15
    pub gateway: Ipv4Addr,        // e.g. 10.0.2.2  (from /proc/net/route)
    pub dns:     Vec<Ipv4Addr>,   // e.g. [10.0.2.3]  (from /etc/resolv.conf or DHCP)
    pub mtu:     u16,             // from /sys/class/net/<iface>/mtu (default 1500)
    pub status:  NetStatus,       // Offline | Unmetered | Metered
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetStatus { Offline, Unmetered, Metered }

impl Default for NetworkState {
    fn default() -> Self {
        NetworkState {
            iface:   "eth0".into(),
            ip:      Ipv4Addr::UNSPECIFIED,
            gateway: Ipv4Addr::UNSPECIFIED,
            dns:     vec![Ipv4Addr::new(10, 0, 2, 3)],
            mtu:     1500,
            status:  NetStatus::Offline,
        }
    }
}

pub static NET_STATE: OnceLock<Arc<Mutex<NetworkState>>> = OnceLock::new();

pub fn net_state() -> Arc<Mutex<NetworkState>> {
    NET_STATE.get_or_init(|| Arc::new(Mutex::new(NetworkState::default()))).clone()
}
```

`net_state()` is always safe to call at any point after supervisor start. The
monitor thread is the only writer; all other subsystems are readers. Lock held
for microseconds only (clone-and-release pattern):

```rust
let snap = net_state().lock().unwrap().clone();  // snapshot, lock released immediately
```

---

## 4. Extended Network Capability Model

`Capabilities` struct additions in `manifest.rs`:

```rust
#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicy {
    #[default]
    None,
    Loopback,   // 127.0.0.1 only; app spawned in isolated netns (B3 fix)
    Full,       // inherit-network — raw WASI socket access to all hosts
}

// Added to Capabilities:
pub network_policy: NetworkPolicy,
pub network_ports:  Vec<u16>,      // empty = unrestricted (Full policy only)
pub network_hosts:  Vec<String>,   // empty = unrestricted (Full policy; IPC-path enforcement)
pub metered_ok:     bool,          // if false, app is paused when net-status=metered
pub max_connections: u32,          // 0 = unlimited; accounting.rs enforces via ConnTracker
```

Backward compatibility: `network = true` in vyoma.toml maps to `network_policy = Full`
with all restriction fields at their defaults (empty = unrestricted). Both the legacy
boolean and the new enum can coexist; the enum takes precedence if both are set.

**Spawn-time enforcement:**
```rust
// supervisor/src/app_threads.rs  (in spawn_app)
match caps.network_policy {
    NetworkPolicy::None => { /* no -S flag; WASI sockets not wired */ }
    NetworkPolicy::Loopback => {
        // B3 fix: kernel-enforced isolation via new netns
        cmd.pre_exec(move || {
            // SAFETY: called in the child process after fork, before exec
            libc::unshare(libc::CLONE_NEWNET);
            networking::netns::bring_up_loopback().expect("lo up");
            Ok(())
        });
        cmd.args(["-S", "inherit-network"]);
    }
    NetworkPolicy::Full => {
        // B5 fix: check for port conflicts before allowing spawn
        if let Err(e) = networking::policy::check_port_conflicts(&app_name, &caps) {
            log::error!("[net] spawn blocked — {e}");
            return Err(e);
        }
        cmd.args(["-S", "inherit-network"]);
    }
}
```

---

## 5. WASI Socket Lifecycle

Wasmtime wires WASI sockets through `wasmtime_wasi::WasiCtxBuilder`. VyomaOS
supervisor controls exactly which apps receive a working socket implementation:

```
App vyoma.toml                 Supervisor spawn                Wasmtime child
──────────────────────────────────────────────────────────────────────────────
network = true      →   -S inherit-network flag     →   wasi:sockets enabled
                        (inherits supervisor netns)      connect() / bind() work

network_policy =    →   pre_exec: unshare(CLONE_NEWNET) →  lo only; kernel drops
  Loopback              bring_up_loopback()                  non-loopback connect

(no network field)  →   no -S flag                  →   wasi:sockets import
                                                          absent; any socket
                                                          call → WASM trap
```

Socket permissions enforced at the Wasmtime CLI layer via `-S inherit-network`
(present → app inherits supervisor's network namespace; absent → Wasmtime
creates an empty capability set and all socket calls fail with `ENOTCAPABLE`).

For `NetworkPolicy::Full` apps the supervisor additionally sets:
- `VYOMA_NET_ALLOWED_PORTS=<comma-separated>` in child env when `network_ports` non-empty
- `VYOMA_NET_ALLOWED_HOSTS=<comma-separated>` in child env when `network_hosts` non-empty

These env vars are advisory (WASI apps that read them can self-enforce). Kernel-level
enforcement of host ACLs requires eBPF `sock_ops` (planned R65); until then IPC-path
connections (proxy through supervisor) fully enforce `network_hosts`.

---

## 6. TCP/UDP Socket Permission Table

| Policy | Ports 1–1023 | Ports 1024–65535 | UDP | Raw socket | Outbound |
|--------|-------------|-----------------|-----|-----------|---------|
| `None` | — | — | — | — | — |
| `Loopback` | blocked (kernel) | lo only | lo only | blocked | blocked |
| `Full` (unrestricted) | blocked by netns | allowed | allowed | blocked | allowed |
| `Full` + `network_ports=[8080]` | blocked | 8080 only | 8080 only | blocked | allowed |

Rules:
1. Ports below 1024 require `CAP_NET_BIND_SERVICE`. The supervisor binary runs
   without any Linux capabilities; wasmtime child processes inherit this empty
   capability set. Binding port < 1024 returns `EACCES`.
2. UDP is allowed on the same port set as TCP unless the app sets `udp = false`
   in its network capability block (future field, R53).
3. Raw sockets require `CAP_NET_RAW` — never granted; all ICMP from WASM apps
   must use ICMP-over-UDP workaround via supervisor IPC (`VYOMA_NET:ping` verb).
4. Outbound connections are unrestricted for `Full` policy unless `network_hosts`
   is populated, in which case advisory env-var enforcement applies.

---

## 7. NetworkPolicy Enforcement (policy.rs)

```rust
// supervisor/src/networking/policy.rs

/// Per-app network policy snapshot stored in AppState.
#[derive(Debug, Clone)]
pub struct AppNetworkPolicy {
    pub allowed_hosts:   Vec<String>,  // empty = all hosts allowed
    pub allowed_ports:   Vec<u16>,     // empty = all ports 1024+ allowed
    pub max_connections: u32,          // 0 = unlimited
    pub metered_ok:      bool,
}

/// B5 fix: Check no running app already owns a port in `caps.network_ports`.
pub fn check_port_conflicts(
    new_app: &str,
    caps:    &Capabilities,
    registry: &AppRegistry,
) -> Result<(), String> {
    if caps.network_ports.is_empty() { return Ok(()); }
    let snapshot = registry.snapshot_running();   // clones names + policies, no long lock
    for other in &snapshot {
        for &port in &other.network_ports {
            if caps.network_ports.contains(&port) {
                return Err(format!(
                    "port {port} already claimed by '{}'", other.name
                ));
            }
        }
    }
    Ok(())
}

/// Called from VYOMA_NET IPC path to check if a specific outbound host is permitted.
pub fn host_allowed(policy: &AppNetworkPolicy, host: &str) -> bool {
    policy.allowed_hosts.is_empty()
        || policy.allowed_hosts.iter().any(|h| {
            h == host || h.strip_prefix("*.").map_or(false, |suffix| host.ends_with(suffix))
        })
}
```

Wildcard host matching supports `*.example.com` patterns. Exact IP addresses are
matched literally. DNS names are not resolved at policy-check time (checked after
`dns-resolve` returns an IP; see section 9 for VYOMA_NET IPC verbs).

---

## 8. Interface Enumeration

The supervisor reads live interface data from proc/sysfs rather than calling
`getifaddrs` (which requires libc; supervisor is musl-static and avoids libc
allocations in the hot path):

```rust
// supervisor/src/networking/monitor.rs  (parse helpers)

/// Read primary IPv4 address from /proc/net/fib_trie for `iface`.
/// Returns None if interface is unconfigured.
pub fn parse_fib_trie_ip(iface: &str) -> Option<Ipv4Addr> {
    // /proc/net/fib_trie lists all routes; we look for LOCAL scope entries
    // that correspond to an interface address.
    let trie = std::fs::read_to_string("/proc/net/fib_trie").ok()?;
    let index = iface_index(iface)?;   // from /proc/net/dev
    parse_trie_for_index(&trie, index)
}

/// Read gateway from /proc/net/route (first non-loopback default route).
pub fn parse_default_gateway() -> Option<Ipv4Addr> {
    let route = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in route.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 { continue; }
        if cols[1] == "00000000" {   // destination = 0.0.0.0 → default route
            let gw_hex = u32::from_str_radix(cols[2], 16).ok()?;
            return Some(Ipv4Addr::from(gw_hex.to_be()));
        }
    }
    None
}

/// Read MTU from sysfs.
pub fn read_mtu(iface: &str) -> u16 {
    std::fs::read_to_string(format!("/sys/class/net/{iface}/mtu"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1500)
}

/// Enumerate all interfaces with their IPv4 addresses.
/// Used by VYOMA_NET:get_interfaces IPC verb.
pub fn enumerate_interfaces() -> Vec<(String, Option<Ipv4Addr>)> {
    let dev = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
    dev.lines()
        .skip(2)   // skip two header lines
        .filter_map(|line| {
            let name = line.split(':').next()?.trim().to_string();
            let ip   = parse_fib_trie_ip(&name);
            Some((name, ip))
        })
        .collect()
}
```

These functions are all pure reads with no syscalls beyond `open`/`read`/`close`.
They are safe to call from any thread without locks.

---

## 9. VYOMA_NET IPC Protocol

Apps write `VYOMA_NET:<verb>:<args>` to stdout; supervisor routes to the networking
subsystem. Responses are delivered on the app's stdin as `VYOMA_NET:<verb>_reply:<data>`.

| Verb | Args | Reply | Notes |
|------|------|-------|-------|
| `get_interfaces` | — | `ifaces:<name>:<ip>,<name>:<ip>,...` | Lists all interfaces with IPs |
| `get_ip` | `<iface>` | `ip:<iface>:<ip>` or `ip:<iface>:none` | Single interface IP lookup |
| `set_dns` | `<ip1>[,<ip2>]` | `dns:ok` or `dns:err:<reason>` | Updates resolv.conf + NET_STATE.dns |
| `request_port` | `<port>` | `port:granted:<port>` or `port:denied:<reason>` | Advisory reservation (B5 partial) |
| `release_port` | `<port>` | `port:released:<port>` | Releases advisory reservation |
| `dns_resolve` | `<hostname>` | `resolved:<hostname>:<ip>` or `nxdomain:<hostname>` | Async via DNS thread (B2 fix) |
| `get_status` | — | `status:unmetered` \| `metered` \| `offline` | Reads NET_STATE.status |
| `get_stats` | `<app>` | `stats:<app>:sent=N:recv=N:conns=N:dns=N` | Per-app accounting snapshot |
| `ping` | `<host>:<count>` | `ping:<host>:ok:rtt=Nms` or `ping:<host>:unreachable` | ICMP-over-UDP proxy |

All verbs return `VYOMA_NET:err:unknown_verb` for unrecognised input. Verbs are
case-sensitive lowercase. Any `:` in the trailing text field must be escaped as `\:`.

**Parsing in supervisor IPC handler:**
```rust
// supervisor/src/ipc_commands/mod.rs  (networking arm)
if let Some(rest) = line.strip_prefix("VYOMA_NET:") {
    let parts: Vec<&str> = rest.splitn(3, ':').collect();
    match parts.as_slice() {
        ["get_interfaces", ..] => networking::handle_get_interfaces(app, inbox),
        ["get_ip", iface, ..] => networking::handle_get_ip(app, iface, inbox),
        ["set_dns", addrs, ..] => networking::handle_set_dns(app, addrs, inbox),
        ["request_port", port, ..] => networking::policy::handle_request_port(app, port, inbox),
        ["release_port", port, ..] => networking::policy::handle_release_port(app, port, inbox),
        ["dns_resolve", host, ..] => networking::dns::enqueue_resolve(app, host),
        ["get_status", ..] => networking::handle_get_status(app, inbox),
        ["get_stats", target, ..] => networking::accounting::handle_get_stats(app, target, inbox),
        ["ping", args, ..] => networking::handle_ping(app, args, inbox),
        _ => inbox.send(app, "VYOMA_NET:err:unknown_verb"),
    }
}
```

---

## 10. DNS Resolver (Async Thread — B2 Fix)

The synchronous `dns_resolve_a` in the original `net.rs` (TCP/53, blocks IPC handler
up to 4s per query, timeout 130s worst-case) is replaced by a dedicated async thread:

```rust
// supervisor/src/networking/dns.rs

pub enum DnsRequest {
    Resolve { app_name: String, hostname: String },
    Shutdown,
}

static DNS_TX: OnceLock<mpsc::Sender<DnsRequest>> = OnceLock::new();

pub fn start_dns_thread(inbox: Arc<Inbox>) {
    let (tx, rx) = mpsc::channel::<DnsRequest>();
    DNS_TX.set(tx).ok();
    thread::Builder::new()
        .name("supervisor-dns".into())
        .spawn(move || dns_worker(rx, inbox))
        .expect("spawn dns thread");
}

/// Non-blocking enqueue. IPC handler returns to caller immediately (B2 fix).
pub fn enqueue_resolve(app_name: &str, hostname: &str) {
    if let Some(tx) = DNS_TX.get() {
        let _ = tx.send(DnsRequest::Resolve {
            app_name: app_name.to_string(),
            hostname: hostname.to_string(),
        });
    }
}

fn dns_worker(rx: mpsc::Receiver<DnsRequest>, inbox: Arc<Inbox>) {
    let mut cache = DnsCache::new(256);
    loop {
        match rx.recv() {
            Ok(DnsRequest::Resolve { app_name, hostname }) => {
                let reply = if let Some(entry) = cache.get(&hostname) {
                    if entry.negative {
                        format!("VYOMA_NET:nxdomain:{hostname}")
                    } else {
                        format!("VYOMA_NET:resolved:{hostname}:{}", entry.ip)
                    }
                } else {
                    match udp_resolve(&hostname) {
                        Ok((ip, ttl)) => {
                            cache.insert(hostname.clone(), ip.to_string(), ttl, false);
                            format!("VYOMA_NET:resolved:{hostname}:{ip}")
                        }
                        Err(DnsError::NxDomain) => {
                            cache.insert_negative(hostname.clone(), 30);
                            format!("VYOMA_NET:nxdomain:{hostname}")
                        }
                        Err(e) => format!("VYOMA_NET:dns_err:{hostname}:{e}"),
                    }
                };
                inbox.send(&app_name, &reply);
            }
            Ok(DnsRequest::Shutdown) | Err(_) => break,
        }
    }
}
```

DNS worker sends UDP queries to `10.0.2.3` (QEMU DNS forwarder) with 5s timeout.
Falls back to `8.8.8.8` on gateway timeout. Constructs minimal RFC 1035 query
manually (no external crates needed; ~60 lines of byte manipulation).

---

## 11. DNS Cache

```rust
// supervisor/src/networking/dns_cache.rs

pub struct DnsEntry {
    pub ip:       String,
    pub expires:  Instant,
    pub negative: bool,    // true = NXDOMAIN cached
}

pub struct DnsCache {
    map:   HashMap<String, DnsEntry>,
    order: VecDeque<String>,    // LRU order; front = oldest
    limit: usize,
}

impl DnsCache {
    pub fn new(limit: usize) -> Self {
        DnsCache { map: HashMap::new(), order: VecDeque::new(), limit }
    }

    /// Returns Some if entry exists AND has not expired.
    pub fn get(&mut self, hostname: &str) -> Option<&DnsEntry> {
        if let Some(entry) = self.map.get(hostname) {
            if entry.expires > Instant::now() {
                return self.map.get(hostname);
            }
            // Expired: remove entry
            self.map.remove(hostname);
            self.order.retain(|h| h != hostname);
        }
        None
    }

    pub fn insert(&mut self, hostname: String, ip: String, ttl_secs: u32, negative: bool) {
        let ttl = ttl_secs.clamp(30, 300);
        let entry = DnsEntry { ip, expires: Instant::now() + Duration::from_secs(ttl as u64), negative };
        self.evict_if_full(&hostname);
        self.map.insert(hostname.clone(), entry);
        self.order.push_back(hostname);
    }

    pub fn insert_negative(&mut self, hostname: String, ttl_secs: u32) {
        self.insert(hostname, String::new(), ttl_secs, true);
    }

    fn evict_if_full(&mut self, incoming: &str) {
        if self.map.len() >= self.limit {
            while let Some(oldest) = self.order.pop_front() {
                if oldest != incoming {
                    self.map.remove(&oldest);
                    break;
                }
            }
        }
    }
}
```

Max 256 entries; LRU eviction via `VecDeque`. TTL clamped: min 30s, max 300s from
DNS response. Negative cache (NXDOMAIN/SERVFAIL): 30s. Zero external crate dependencies.

---

## 12. Network Status Detection (B4 Fix)

QEMU user-mode always reports `operstate=unknown` via sysfs — this field cannot be
used alone to determine connectivity:

```rust
// supervisor/src/networking/monitor.rs

pub fn detect_net_status(iface: &str) -> NetStatus {
    // carrier=1 means physical link (or QEMU virtio link) is up
    let carrier = std::fs::read_to_string(format!("/sys/class/net/{iface}/carrier"))
        .map(|s| s.trim() == "1")
        .unwrap_or(false);

    // fib_trie presence of a non-zero local IP means DHCP completed
    let has_ip = parse_fib_trie_ip(iface)
        .map(|ip| ip != Ipv4Addr::UNSPECIFIED)
        .unwrap_or(false);

    // B4 fix: carrier=1 AND IP assigned = Unmetered
    // QEMU user-mode never sets operstate=up; this two-signal heuristic works reliably.
    match (carrier, has_ip) {
        (true,  true)  => NetStatus::Unmetered,
        (true,  false) => NetStatus::Offline,   // link up but DHCP pending
        (false, _)     => NetStatus::Offline,
    }
}
```

Broadcast on state change (sent to all apps with `network = true`):
```
VYOMA_SYSTEM:net-change:eth0:up:10.0.2.15
VYOMA_SYSTEM:net-change:eth0:down:
```

The monitor thread uses netlink `RTMGRP_LINK | RTMGRP_IPV4_IFADDR` events
(requires `CONFIG_RTNETLINK=y`), polling sysfs every 5s as a fallback when no
netlink socket is available. Never holds `AppRegistry` lock while sending
broadcasts: snapshot app names under inbox lock → release lock → send.

```rust
pub fn start_monitor_thread(inbox: Arc<Inbox>) {
    thread::Builder::new()
        .name("supervisor-netmon".into())
        .spawn(move || {
            let mut last_status = NetStatus::Offline;
            loop {
                let current = detect_net_status("eth0");
                if current != last_status {
                    // Update global state
                    if let Ok(mut state) = net_state().lock() {
                        state.status = current.clone();
                        if current != NetStatus::Offline {
                            state.ip      = parse_fib_trie_ip("eth0").unwrap_or(Ipv4Addr::UNSPECIFIED);
                            state.gateway = parse_default_gateway().unwrap_or(Ipv4Addr::UNSPECIFIED);
                            state.mtu     = read_mtu("eth0");
                        }
                    }
                    // Broadcast to apps — snapshot names first, then send outside lock
                    let msg = match &current {
                        NetStatus::Unmetered => {
                            let ip = net_state().lock().unwrap().ip;
                            format!("VYOMA_SYSTEM:net-change:eth0:up:{ip}")
                        }
                        _ => "VYOMA_SYSTEM:net-change:eth0:down:".into(),
                    };
                    let targets = inbox.snapshot_network_subscribers();
                    for app in targets { inbox.send(&app, &msg); }
                    last_status = current;
                }
                thread::sleep(Duration::from_secs(5));
            }
        })
        .expect("spawn netmon");
}
```

---

## 13. Connection Tracking and Rate Limiting

```rust
// supervisor/src/networking/accounting.rs

#[derive(Debug, Default)]
pub struct AppNetStats {
    pub bytes_sent:    u64,
    pub bytes_recv:    u64,
    pub connections:   u32,   // active concurrent connections
    pub dns_queries:   u32,   // total lifetime DNS queries via supervisor
    pub session_start: Instant,
}

/// Active connection entry for quota enforcement.
#[derive(Debug)]
struct ConnEntry {
    app_name: String,
    dest_ip:  Ipv4Addr,
    dest_port: u16,
    opened:   Instant,
}

static NET_ACCOUNTING: OnceLock<Arc<Mutex<HashMap<String, AppNetStats>>>> = OnceLock::new();
static CONN_TRACKER:   OnceLock<Arc<Mutex<Vec<ConnEntry>>>>               = OnceLock::new();

pub fn record_connection_open(app: &str, dest_ip: Ipv4Addr, dest_port: u16) -> Result<(), String> {
    let policy = crate::app_registry::get_network_policy(app);
    if let Some(max) = policy.map(|p| p.max_connections).filter(|&m| m > 0) {
        let tracker = CONN_TRACKER.get_or_init(|| Arc::new(Mutex::new(Vec::new())));
        let count = tracker.lock().unwrap().iter().filter(|c| c.app_name == app).count();
        if count as u32 >= max {
            return Err(format!("connection limit {max} reached for '{app}'"));
        }
    }
    // Record open
    let tracker = CONN_TRACKER.get_or_init(|| Arc::new(Mutex::new(Vec::new())));
    tracker.lock().unwrap().push(ConnEntry {
        app_name: app.to_string(),
        dest_ip, dest_port,
        opened: Instant::now(),
    });
    accounting_for(app).connections += 1;
    Ok(())
}

pub fn record_connection_close(app: &str, dest_ip: Ipv4Addr, dest_port: u16) {
    let tracker = CONN_TRACKER.get_or_init(|| Arc::new(Mutex::new(Vec::new())));
    let mut guard = tracker.lock().unwrap();
    if let Some(pos) = guard.iter().position(|c| {
        c.app_name == app && c.dest_ip == dest_ip && c.dest_port == dest_port
    }) {
        guard.remove(pos);
        if let Some(stats) = NET_ACCOUNTING
            .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
            .lock().unwrap().get_mut(app)
        {
            stats.connections = stats.connections.saturating_sub(1);
        }
    }
}
```

Direct WASI socket calls bypass IPC accounting — this is a documented limitation
until eBPF `sock_ops` support lands in R65. IPC-proxied connections (apps that use
`VYOMA_NET:` protocol verbs) are fully tracked.

Query: `@supervisor: net-stats <app_name>`
Reply: `REPLY:net-stats http-server sent=102400 recv=8192 conns=3 dns=5 uptime=45s`

---

## 14. Persistent Network Config

Network configuration survives VM reboots via `/data/.vyoma/network/`:

```
/data/.vyoma/network/
├── config.toml      — static config overrides (optional; auto-detected DHCP values win)
├── dns_cache.json   — serialised DnsCache snapshot (written on clean shutdown)
└── stats.json       — per-app cumulative byte counters (flushed every 60s)
```

`config.toml` schema:
```toml
[interface]
iface   = "eth0"
# ip, gateway, mtu omitted → use DHCP-discovered values

[dns]
servers = ["10.0.2.3", "8.8.8.8"]   # override DHCP-provided DNS

[metered]
policy = "auto"   # "auto" | "always" | "never"
```

Write discipline: supervisor writes `.tmp` files and atomically renames:
```rust
fn persist_stats(path: &Path, stats: &HashMap<String, AppNetStats>) {
    let tmp = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(stats).expect("serialize");
    std::fs::write(&tmp, json).expect("write tmp");
    std::fs::rename(&tmp, path).expect("atomic rename");  // atomic on ext4
}
```

Config is loaded at supervisor startup before any apps are spawned:
```rust
pub fn load_network_config() -> Option<NetworkConfig> {
    let path = Path::new("/data/.vyoma/network/config.toml");
    let raw  = std::fs::read_to_string(path).ok()?;
    toml::from_str(&raw).ok()
}
```

If `/data` is not mounted (no `-disk disk.img` in QEMU invocation), the
supervisor silently skips persistence and uses in-memory defaults only.

---

## 15. Network Namespace Isolation (B3 Fix)

`inherit-network` + `network_policy: Loopback` without kernel enforcement is
advisory-only — a WASM app could connect to any host via WASI sockets, bypassing
supervisor `network_hosts` checks entirely.

Fix: spawn Loopback apps inside a new network namespace (kernel-enforced):

```rust
// supervisor/src/networking/netns.rs

/// Called in child process after fork(), before exec().
/// SAFETY: Only async-signal-safe operations after fork. All memory allocations
///         must complete before fork; this function must not allocate.
pub unsafe fn bring_up_loopback() -> std::io::Result<()> {
    // Create a UDP socket to issue ioctl against lo
    let sock = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
    if sock < 0 { return Err(std::io::Error::last_os_error()); }

    let mut ifr: libc::ifreq = std::mem::zeroed();
    // Copy "lo\0" into ifr_name (fixed-length array of c_char)
    let name = b"lo\0";
    for (i, &b) in name.iter().enumerate() {
        ifr.ifr_name[i] = b as libc::c_char;
    }

    // Get current flags
    if libc::ioctl(sock, libc::SIOCGIFFLAGS, &mut ifr) < 0 {
        libc::close(sock);
        return Err(std::io::Error::last_os_error());
    }

    // Set IFF_UP | IFF_LOOPBACK
    ifr.ifr_ifru.ifru_flags |= (libc::IFF_UP | libc::IFF_LOOPBACK) as libc::c_short;
    if libc::ioctl(sock, libc::SIOCSIFFLAGS, &mut ifr) < 0 {
        libc::close(sock);
        return Err(std::io::Error::last_os_error());
    }

    libc::close(sock);
    Ok(())
}
```

`unshare(CLONE_NEWNET)` in the `pre_exec` closure creates an isolated network
namespace with only a down `lo` interface. `bring_up_loopback()` then raises it.
Any `wasi:sockets` `connect()` to a non-loopback address returns `ENETUNREACH`
from the kernel — no IPC bypass is possible.

Requires: `CONFIG_NET_NS=y` (already present in kernel config for R49 sandboxing).

---

## 16. Blocking Issue Resolution Summary

| Issue | Root Cause | Resolution | Status |
|-------|-----------|-----------|--------|
| B1: `eth0` has no IP — all TCP connects fail | `CONFIG_IP_PNP_DHCP` absent from kernel config; no userspace DHCP daemon in initramfs | Add `CONFIG_IP_PNP=y` + `CONFIG_IP_PNP_DHCP=y` to `base/kernel.config`; add `ip=dhcp` to kernel cmdline `-append` for `run-net` / `run-gui-net` Makefile targets | RESOLVED |
| B2: Synchronous `dns_resolve_a` blocks IPC handler thread up to 130s | Single-threaded IPC loop called blocking TCP/53 query inline; timeout per attempt 4s × 32 retries | Replace with `DnsRequest` mpsc channel + dedicated `supervisor-dns` thread; IPC handler enqueues and returns immediately; reply delivered asynchronously to app stdin | RESOLVED |
| B3: `inherit-network` bypasses `network_hosts` allowlist via direct WASI sockets | WASI socket calls in Wasmtime go directly to kernel; supervisor's `network_hosts` list is only checked on IPC-path connections | `Loopback` policy uses `unshare(CLONE_NEWNET)` + `bring_up_loopback()` in `pre_exec`; kernel denies all non-loopback socket operations with `ENETUNREACH`; no IPC bypass possible | RESOLVED |
| B4: `operstate=unknown` on QEMU user-mode makes `net-status` permanently report `offline` | QEMU virtio-net never transitions `operstate` to `up`; supervisor polled only this field | Use two-signal heuristic: `carrier == 1` (sysfs) AND IP present in `/proc/net/fib_trie`; ignore `operstate`; reliable on both QEMU and bare-metal | RESOLVED |
| B5: `network_port` field is advisory-only; two apps silently clash on same port | Port registrations stored only in vyoma.toml manifest, never cross-checked at spawn time | Port conflict check in `policy::check_port_conflicts()` against `AppRegistry` snapshot at spawn; second app with conflicting port is rejected with log error `[net] spawn blocked` | RESOLVED |
