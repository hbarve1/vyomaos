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
```

Kernel command line in Makefile `run-net` / `run-gui-net` targets:
```makefile
-append "... ip=dhcp"
```

This triggers `CONFIG_IP_PNP_DHCP` to self-configure `eth0=10.0.2.15/24` before PID 1 starts. No userspace `dhclient` needed (B1 fix).

Estimated kernel size increase: ~60 KB. `/dev/net/tun` created automatically by devtmpfs once `CONFIG_TUN=y` is added in R53.

---

## 2. Extended Network Capability Model

`Capabilities` struct additions in `manifest.rs`:

```rust
#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicy {
    #[default]
    None,
    Loopback,   // 127.0.0.1 only; app spawned in isolated netns
    Full,       // inherit-network — raw WASI socket access
}

// Added to Capabilities:
pub network_policy: NetworkPolicy,
pub network_ports:  Vec<u16>,      // empty = unrestricted (Full only)
pub network_hosts:  Vec<String>,   // empty = unrestricted (Full only; IPC-path enforcement)
pub metered_ok:     bool,          // if false, paused when net-status=metered
```

Backward compatibility: `network = true` maps to `network_policy = Full`. Both can coexist.

**Spawn-time enforcement**:
```rust
match caps.network_policy {
    NetworkPolicy::None    => { /* no -S flag */ }
    NetworkPolicy::Loopback => {
        // B3 fix: kernel-enforced isolation via new netns
        cmd.pre_exec(move || {
            libc::unshare(libc::CLONE_NEWNET);
            network::netns::bring_up_loopback();
            Ok(())
        });
        cmd.args(["-S", "inherit-network"]);
    }
    NetworkPolicy::Full => {
        cmd.args(["-S", "inherit-network"]);
    }
}
```

Port conflict detection (B5 fix): at spawn, check AppRegistry for any running app with the same `network_port`; reject with error if conflict found.

---

## 3. DNS Resolver (Async Thread — B2 Fix)

The synchronous `dns_resolve_a` in `net.rs` (TCP/53, blocks IPC handler up to 4s) is replaced by an async thread:

```rust
// supervisor/src/network/dns.rs  (~180 lines)
static DNS_TX: OnceLock<mpsc::Sender<DnsRequest>> = OnceLock::new();

pub fn start_dns_thread(inbox: Arc<Inbox>) {
    let (tx, rx) = mpsc::channel::<DnsRequest>();
    DNS_TX.set(tx).ok();
    thread::Builder::new().name("supervisor-dns".into())
        .spawn(move || dns_worker(rx, inbox))
        .expect("spawn dns");
}
```

IPC handler for `@supervisor: dns-resolve <hostname>` sends to `DNS_TX` and returns immediately (B2 fix: zero IPC handler blocking).

DNS worker uses **UDP/53 to `10.0.2.3`** (QEMU's forwarder), with 5s timeout. Falls back to `8.8.8.8` on QEMU gateway timeout. TTL from DNS A record response cached.

Reply format:
```
VYOMA_DNS:resolved:<hostname>:<ip>
VYOMA_DNS:nxdomain:<hostname>
VYOMA_DNS:error:<hostname>:<reason>
```

---

## 4. DNS Cache

```rust
// supervisor/src/network/dns_cache.rs  (~80 lines)
pub struct DnsEntry { pub ip: String, pub expires: Instant, pub negative: bool }
pub struct DnsCache { map: HashMap<String, DnsEntry>, order: VecDeque<String>, limit: usize }
```

Max 256 entries; LRU eviction via `VecDeque`. TTL: min 30s, max 300s from DNS response. Negative cache (NXDOMAIN/SERVFAIL): 30s. Zero new crate dependencies.

---

## 5. Network Status Detection (B4 Fix)

QEMU user-mode always reports `operstate=unknown` — cannot use this field alone:

```rust
// supervisor/src/network/monitor.rs  (~150 lines)
fn detect_net_status(iface: &str) -> NetStatus {
    let carrier = fs::read_to_string(format!("/sys/class/net/{iface}/carrier"))
        .map(|s| s.trim() == "1").unwrap_or(false);
    let has_ip = parse_fib_trie_ip(iface).is_some();  // /proc/net/fib_trie
    // B4 fix: carrier=1 AND IP assigned = unmetered (QEMU user-mode never sets operstate=up)
    if carrier && has_ip { NetStatus::Unmetered }
    else                  { NetStatus::Offline }
}
```

Broadcast on change:
```
VYOMA_SYSTEM:net-change:eth0:up:10.0.2.15
VYOMA_SYSTEM:net-change:eth0:down:
```

Monitor thread uses netlink `RTMGRP_LINK | RTMGRP_IPV4_IFADDR` events (requires `CONFIG_RTNETLINK=y`), with sysfs fallback polling every 5s. Never holds AppRegistry lock while sending broadcasts (ABBA-deadlock prevention: snapshot names under inbox lock, drop, then send).

Query: `@supervisor: net-status` → `VYOMA_SYSTEM:net-status:unmetered|metered|offline` (reads `NET_STATE: OnceLock<Mutex<NetStatus>>`, returns immediately).

---

## 6. Per-App Network Accounting

```rust
// supervisor/src/network/accounting.rs  (~120 lines)
pub struct AppNetStats {
    pub bytes_sent: u64, pub bytes_recv: u64,
    pub connections: u32, pub dns_queries: u32,
    pub session_start: Instant,
}
static NET_ACCOUNTING: OnceLock<Mutex<HashMap<String, AppNetStats>>> = OnceLock::new();
```

Hooks in `ipc_commands/tcp.rs`: increment `bytes_sent/recv` on IPC-proxied `tcp-send/tcp-recv`. Increment `dns_queries` in DNS worker on each `dns-resolve` request. Note: direct WASI socket calls bypass IPC accounting — documented limitation until eBPF `sock_ops` support.

Query: `@supervisor: net-stats <app_name>` → `REPLY:net-stats http-server sent=102400 recv=8192 conns=3 dns=5 uptime=45s`

---

## 7. Network Namespace Isolation (B3 Fix)

`inherit-network` + `network_policy: Loopback` without isolation is advisory-only — WASM app can connect to any host via WASI sockets bypassing supervisor checks.

Fix: spawn Loopback apps in a new network namespace (kernel-enforced):

```rust
// supervisor/src/network/netns.rs  (~60 lines)
pub unsafe fn bring_up_loopback() -> std::io::Result<()> {
    let sock = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
    let mut ifr: libc::ifreq = std::mem::zeroed();
    b"lo\0".iter().enumerate().for_each(|(i,&b)| ifr.ifr_name[i] = b as i8);
    libc::ioctl(sock, libc::SIOCGIFFLAGS, &mut ifr);
    ifr.ifr_ifru.ifru_flags |= (libc::IFF_UP | libc::IFF_LOOPBACK) as i16;
    libc::ioctl(sock, libc::SIOCSIFFLAGS, &mut ifr);
    libc::close(sock);
    Ok(())
}
```

`unshare(CLONE_NEWNET)` in `pre_exec` creates an isolated netns with only `lo`. Any `wasi:sockets` connect to non-loopback address returns `ENETUNREACH` from kernel — no IPC bypass possible.

---

## 8. File Layout

```
supervisor/src/network/
├── mod.rs          (~35 lines: statics, start_dns_thread, start_monitor_thread)
├── dns.rs          (~180 lines: DnsRequest, async resolver thread (B2), UDP query, cache update)
├── dns_cache.rs    (~80 lines: DnsCache, LRU, TTL, negative entries)
├── policy.rs       (~120 lines: NetworkPolicy enforcement, port conflict check (B5))
├── monitor.rs      (~150 lines: netlink watcher, sysfs fallback, QEMU operstate fix (B4))
├── accounting.rs   (~120 lines: AppNetStats, query handler)
└── netns.rs        (~60 lines: bring_up_loopback ioctl, unshare helper (B3))

base/kernel.config  (modified: 7 new CONFIG_ lines (B1))
Makefile            (modified: ip=dhcp in -append for run-net/run-gui-net (B1))
supervisor/src/manifest.rs  (modified: NetworkPolicy enum, network_ports, network_hosts)
supervisor/src/app_threads.rs (modified: switch on NetworkPolicy, pre_exec netns (B3))
supervisor/src/ipc_commands/mod.rs (modified: async dns-resolve, net-status, net-stats)
```

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `eth0` has no IP — `CONFIG_IP_PNP_DHCP` absent; all TCP connects fail | Add `CONFIG_IP_PNP=y` + `CONFIG_IP_PNP_DHCP=y` to kernel.config; add `ip=dhcp` to kernel cmdline |
| B2: Synchronous `dns_resolve_a` blocks IPC handler thread up to 130s | Async DNS worker thread via `mpsc`; IPC handler returns immediately |
| B3: `inherit-network` bypasses `network_hosts` allowlist via direct WASI sockets | `Loopback` policy uses `unshare(CLONE_NEWNET)` in `pre_exec`; kernel enforces isolation |
| B4: `operstate=unknown` on QEMU user-mode makes `net-status` permanently offline | Use carrier=1 AND IP-in-fib_trie as "up" heuristic; ignore operstate |
| B5: `network_port` field is advisory-only; two apps can clash on same port silently | Port conflict check against AppRegistry at spawn time; reject second app with log error |
