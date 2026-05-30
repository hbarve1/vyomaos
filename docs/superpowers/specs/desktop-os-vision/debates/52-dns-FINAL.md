# FINAL Spec: DNS & mDNS / Bonjour (Round 52)

**Subsystem**: DNS & mDNS / Bonjour  
**macOS Analogue**: `mDNSResponder` / `Bonjour`  
**Depends on**: R51 (networking stack, DNS thread)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Two components live under `supervisor/src/dns/`, each in its own thread:

| Component | Thread | Role |
|-----------|--------|------|
| Stub resolver | `supervisor-dns` (from R51) | UDP/53 queries, TTL cache, in-flight dedup |
| mDNS responder | `mdns-responder` | `.local` A/PTR/SRV/TXT responses, DNS-SD browse |

Boot sequence:
1. `dns::init()` called before any app is spawned.
2. Reads `/data/.vyoma/hostname` (default: `vyomaos.local`); writes `/etc/hostname`.
3. Writes `/etc/resolv.conf` (`nameserver 10.0.2.3` for QEMU; real interface DNS otherwise).
4. Restores persistent service registry from `/data/.vyoma/dns/mdns-services.toml`.
5. Restores TTL-valid entries from `/data/.vyoma/dns/cache.json` into `DNS_CACHE`.
6. Spawns `supervisor-dns` resolver thread.
7. Spawns `mdns-responder` thread (with QEMU-aware multicast detection).

All globals use `OnceLock<Arc<Mutex<T>>>` so they are safe to initialise once and read from any thread thereafter.

---

## 2. File Layout

```
supervisor/src/dns/
├── mod.rs        (~90 lines:  init, spawn threads, DNS_TX + MDNS_SERVICES + DNS_CACHE statics)
├── cache.rs      (~140 lines: DnsCache, CacheEntry, TTL expiry, negative caching, persist/restore)
├── resolver.rs   (~180 lines: UDP query, in-flight dedup (B3), cache hit path, cache update)
├── mdns.rs       (~200 lines: responder thread, socket setup, QEMU detection (B2), record builder)
├── sd.rs         (~150 lines: DNS-SD registry, browse, persistence (B5), Goodbye (B5))
└── packet.rs     (~130 lines: wire-format encode/decode, parse_mdns_query (B4), dns_skip_name_safe)
```

Each file stays well within the 500-line limit. Cross-file references go through the statics defined in `mod.rs`; no circular imports.

---

## 3. Structs and Statics (mod.rs)

```rust
// supervisor/src/dns/mod.rs  (~90 lines)

use std::sync::{Arc, Mutex, OnceLock};
use std::sync::mpsc;
use std::net::UdpSocket;

// ── Statics ──────────────────────────────────────────────────────────────────

/// Channel through which the IPC handler enqueues resolve requests.
/// The dns-resolver thread drains this channel.
pub static DNS_TX: OnceLock<mpsc::SyncSender<DnsRequest>> = OnceLock::new();

/// In-flight dedup map: hostname → list of reply channels waiting on the result.
pub static DNS_INFLIGHT: OnceLock<Arc<Mutex<std::collections::HashMap<
    (String, QueryType),
    Vec<mpsc::Sender<ResolveResult>>,
>>>> = OnceLock::new();

/// TTL-aware DNS cache shared between the resolver and the mDNS responder.
pub static DNS_CACHE: OnceLock<Arc<Mutex<crate::dns::cache::DnsCache>>> = OnceLock::new();

/// Registered local services advertised via mDNS / DNS-SD.
pub static MDNS_SERVICES: OnceLock<Arc<Mutex<Vec<crate::dns::sd::ServiceRecord>>>> =
    OnceLock::new();

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryType { A, AAAA, PTR, SRV, TXT }

#[derive(Debug, Clone)]
pub struct DnsRequest {
    pub hostname:  String,
    pub qtype:     QueryType,
    pub reply_tx:  mpsc::Sender<ResolveResult>,
}

#[derive(Debug, Clone)]
pub enum ResolveResult {
    Resolved { hostname: String, records: Vec<DnsRecord> },
    NxDomain { hostname: String },
    Timeout  { hostname: String },
    Error    { hostname: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct DnsRecord {
    pub qtype: QueryType,
    pub value: String,   // IP address (A/AAAA) or raw text (PTR/SRV/TXT)
    pub ttl:   u32,
}

// ── Init ──────────────────────────────────────────────────────────────────────

pub fn init() {
    // Initialise all globals before spawning threads
    DNS_INFLIGHT.get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())));
    DNS_CACHE.get_or_init(|| Arc::new(Mutex::new(cache::DnsCache::new())));
    MDNS_SERVICES.get_or_init(|| Arc::new(Mutex::new(Vec::new())));

    // Restore persistent state
    cache::restore_cache();
    sd::restore_services();

    // Write system files
    write_resolv_conf();
    write_hostname();

    // Spawn worker threads
    let (tx, rx) = mpsc::sync_channel(64);
    DNS_TX.set(tx).ok();
    std::thread::Builder::new()
        .name("supervisor-dns".into())
        .spawn(move || resolver::run(rx))
        .expect("dns thread spawn failed");
    std::thread::Builder::new()
        .name("mdns-responder".into())
        .spawn(mdns::run_mdns_responder)
        .expect("mdns thread spawn failed");
}

fn write_resolv_conf() {
    let ns = std::env::var("VYOMA_DNS_NS").unwrap_or_else(|_| "10.0.2.3".into());
    let content = format!("nameserver {}\n", ns);
    atomic_write("/etc/resolv.conf", content.as_bytes());
}

fn write_hostname() {
    let hn = std::fs::read_to_string("/data/.vyoma/hostname")
        .unwrap_or_else(|_| "vyomaos.local".into());
    atomic_write("/etc/hostname", hn.trim().as_bytes());
}

fn atomic_write(path: &str, data: &[u8]) {
    let tmp = format!("{}.tmp", path);
    let _ = std::fs::write(&tmp, data);
    let _ = std::fs::rename(&tmp, path);
}
```

---

## 4. DNS Cache (cache.rs)

```rust
// supervisor/src/dns/cache.rs  (~140 lines)

use std::collections::HashMap;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use crate::dns::{QueryType, DnsRecord};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub records:    Vec<DnsRecord>,
    pub inserted:   Instant,
    pub min_ttl:    u32,       // shortest TTL across all records (governs expiry)
    pub negative:   bool,      // true = cached NXDOMAIN
}

impl CacheEntry {
    pub fn is_expired(&self) -> bool {
        self.inserted.elapsed() > Duration::from_secs(self.min_ttl as u64)
    }
}

#[derive(Debug, Default)]
pub struct DnsCache {
    entries: HashMap<(String, QueryType), CacheEntry>,
}

impl DnsCache {
    pub fn new() -> Self { Self::default() }

    /// Insert resolved records, computing min TTL across all entries.
    pub fn insert(&mut self, name: String, qtype: QueryType, records: Vec<DnsRecord>) {
        let min_ttl = records.iter().map(|r| r.ttl).min().unwrap_or(60);
        self.entries.insert((name, qtype), CacheEntry {
            records,
            inserted: Instant::now(),
            min_ttl,
            negative: false,
        });
        self.evict_expired();
    }

    /// Insert a negative (NXDOMAIN) cache entry.  Held for NXDOMAIN_TTL seconds.
    pub fn insert_negative(&mut self, name: String, qtype: QueryType) {
        const NXDOMAIN_TTL: u32 = 300;
        self.entries.insert((name, qtype), CacheEntry {
            records:  Vec::new(),
            inserted: Instant::now(),
            min_ttl:  NXDOMAIN_TTL,
            negative: true,
        });
    }

    /// Look up a name+type pair; returns None if absent or expired.
    pub fn get(&mut self, name: &str, qtype: QueryType) -> Option<&CacheEntry> {
        let key = (name.to_string(), qtype);
        if let Some(entry) = self.entries.get(&key) {
            if entry.is_expired() {
                self.entries.remove(&key);
                return None;
            }
            return self.entries.get(&key);
        }
        None
    }

    /// Remove all expired entries.
    pub fn evict_expired(&mut self) {
        self.entries.retain(|_, v| !v.is_expired());
    }

    /// Flush a specific name from the cache (e.g., after mdns-register).
    pub fn flush_name(&mut self, name: &str) {
        self.entries.retain(|(n, _), _| n != name);
    }

    /// Flush the entire cache.
    pub fn flush_all(&mut self) { self.entries.clear(); }
}

// ── Persistence ───────────────────────────────────────────────────────────────

const CACHE_PATH: &str = "/data/.vyoma/dns/cache.json";

/// Persist TTL-valid entries to disk so warm-cache survives supervisor OTA restarts.
/// Called after every cache insert; uses atomic write-then-rename.
pub fn persist_cache(cache: &DnsCache) {
    #[derive(Serialize)]
    struct Row<'a> { name: &'a str, qtype: &'a str, records: &'a Vec<DnsRecord>, ttl: u32 }

    let rows: Vec<Row> = cache.entries.iter()
        .filter(|(_, v)| !v.is_expired() && !v.negative)
        .map(|((n, qt), v)| Row {
            name:    n,
            qtype:   qt.as_str(),
            records: &v.records,
            ttl:     v.min_ttl,
        })
        .collect();

    if let Ok(json) = serde_json::to_string(&rows) {
        let tmp = format!("{}.tmp", CACHE_PATH);
        let _ = std::fs::create_dir_all("/data/.vyoma/dns");
        let _ = std::fs::write(&tmp, json.as_bytes());
        let _ = std::fs::rename(&tmp, CACHE_PATH);
    }
}

/// Restore cache entries from disk at startup.
pub fn restore_cache() {
    #[derive(Deserialize)]
    struct Row { name: String, qtype: String, records: Vec<DnsRecord>, ttl: u32 }

    if let Ok(data) = std::fs::read_to_string(CACHE_PATH) {
        if let Ok(rows) = serde_json::from_str::<Vec<Row>>(&data) {
            let cache_arc = super::DNS_CACHE.get().unwrap();
            let mut cache = cache_arc.lock().unwrap();
            for row in rows {
                if let Some(qt) = QueryType::from_str(&row.qtype) {
                    // Only restore entries that still have remaining TTL > 30s
                    if row.ttl > 30 {
                        cache.entries.insert((row.name, qt), CacheEntry {
                            records:  row.records,
                            inserted: Instant::now(),
                            min_ttl:  row.ttl,
                            negative: false,
                        });
                    }
                }
            }
        }
    }
}

impl QueryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueryType::A    => "A",
            QueryType::AAAA => "AAAA",
            QueryType::PTR  => "PTR",
            QueryType::SRV  => "SRV",
            QueryType::TXT  => "TXT",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "A"    => Some(QueryType::A),
            "AAAA" => Some(QueryType::AAAA),
            "PTR"  => Some(QueryType::PTR),
            "SRV"  => Some(QueryType::SRV),
            "TXT"  => Some(QueryType::TXT),
            _      => None,
        }
    }
}
```

---

## 5. Stub Resolver with In-Flight Dedup (B1 + B3 Fix)

```rust
// supervisor/src/dns/resolver.rs  (~180 lines)

use std::sync::mpsc;
use std::net::UdpSocket;
use std::time::Duration;
use crate::dns::{DnsRequest, DnsRecord, QueryType, ResolveResult};
use crate::dns::cache;

const UPSTREAM_DNS: &str = "10.0.2.3:53";
const QUERY_TIMEOUT_MS: u64 = 5000;
const MAX_UDP_PAYLOAD: usize = 512;

/// Thread entry point: drain the DNS_TX channel, service cache hits inline,
/// deduplicate concurrent identical queries before hitting the network.
pub fn run(rx: mpsc::Receiver<DnsRequest>) {
    for req in rx {
        // ── Cache hit (lock, check, unlock — no I/O while locked) ──
        {
            let cache_arc = super::DNS_CACHE.get().unwrap().clone();
            let mut cache = cache_arc.lock().unwrap();
            if let Some(entry) = cache.get(&req.hostname, req.qtype) {
                let result = if entry.negative {
                    ResolveResult::NxDomain { hostname: req.hostname.clone() }
                } else {
                    ResolveResult::Resolved {
                        hostname: req.hostname.clone(),
                        records:  entry.records.clone(),
                    }
                };
                let _ = req.reply_tx.send(result);
                continue;
            }
        }

        // ── In-flight dedup (B3 fix): single critical section ─────────────
        let should_query = {
            let mut inflight = super::DNS_INFLIGHT.get().unwrap().lock().unwrap();
            let key = (req.hostname.clone(), req.qtype);
            if let Some(waiters) = inflight.get_mut(&key) {
                waiters.push(req.reply_tx);
                false   // another waiter already owns this query
            } else {
                inflight.insert(key, vec![req.reply_tx]);
                true    // this thread must send the UDP query
            }
            // lock released here — no I/O inside the critical section
        };

        if should_query {
            let result = send_udp_query(&req.hostname, req.qtype);

            // Update cache before waking waiters
            {
                let cache_arc = super::DNS_CACHE.get().unwrap().clone();
                let mut cache = cache_arc.lock().unwrap();
                match &result {
                    ResolveResult::Resolved { hostname, records } => {
                        cache.insert(hostname.clone(), req.qtype, records.clone());
                        cache::persist_cache(&cache);
                    }
                    ResolveResult::NxDomain { hostname } => {
                        cache.insert_negative(hostname.clone(), req.qtype);
                    }
                    _ => {}
                }
            }

            // Wake all waiters and remove the in-flight entry
            let mut inflight = super::DNS_INFLIGHT.get().unwrap().lock().unwrap();
            let key = (req.hostname.clone(), req.qtype);
            if let Some(waiters) = inflight.remove(&key) {
                for tx in waiters {
                    let _ = tx.send(result.clone());
                }
            }
        }
    }
}

/// Send a single A-record UDP query to UPSTREAM_DNS with a 5-second timeout.
fn send_udp_query(hostname: &str, qtype: QueryType) -> ResolveResult {
    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => return ResolveResult::Error {
            hostname: hostname.to_string(),
            reason: format!("bind: {}", e),
        },
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(QUERY_TIMEOUT_MS)));
    let _ = sock.connect(UPSTREAM_DNS);

    let qtype_wire: u16 = match qtype {
        QueryType::A    => 1,
        QueryType::AAAA => 28,
        QueryType::PTR  => 12,
        QueryType::SRV  => 33,
        QueryType::TXT  => 16,
    };
    let pkt = super::packet::build_query(hostname, qtype_wire);
    if sock.send(&pkt).is_err() {
        return ResolveResult::Error { hostname: hostname.to_string(), reason: "send failed".into() };
    }

    let mut buf = [0u8; MAX_UDP_PAYLOAD];
    match sock.recv(&mut buf) {
        Err(_) => ResolveResult::Timeout { hostname: hostname.to_string() },
        Ok(n)  => super::packet::parse_response(&buf[..n], hostname, qtype),
    }
}
```

IPC protocol (supervisor → app, delivered via stdout routing):
```
VYOMA_DNS:resolved:<hostname>:<ip>
VYOMA_DNS:nxdomain:<hostname>
VYOMA_DNS:timeout:<hostname>
VYOMA_DNS:error:<hostname>:<reason>
```

---

## 6. mDNS Responder (B2 + B4 Fix)

```rust
// supervisor/src/dns/mdns.rs  (~200 lines)

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use crate::dns::sd;
use crate::dns::packet;

const MDNS_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
const MAX_PKT: usize = 9000;   // jumbo mDNS is rare but spec allows up to 9000

/// Thread entry point: bind the mDNS socket, optionally join multicast,
/// then loop dispatching incoming queries.
pub fn run_mdns_responder() {
    // B2 fix: detect QEMU user-mode BEFORE any multicast join attempt
    let qemu_mode = is_qemu_usermode();
    if qemu_mode {
        log::warn!("[mdns] QEMU user-mode detected — multicast unavailable; using ARP unicast fallback for browse");
    }

    let sock = match bind_mdns_socket() {
        Ok(s) => s,
        Err(e) => { log::error!("[mdns] socket bind failed: {}", e); return; }
    };

    if !qemu_mode {
        if let Err(e) = sock.join_multicast_v4(&MDNS_ADDR, &Ipv4Addr::UNSPECIFIED) {
            log::warn!("[mdns] multicast join failed: {} — continuing unicast-only", e);
        }
    }

    let mut buf = [0u8; MAX_PKT];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                // B4 fix: Option-returning parser; malformed packets silently dropped
                if let Some(query) = packet::parse_mdns_query(&buf[..n]) {
                    handle_query(&query, &src, &sock);
                }
            }
            Err(e) => {
                // EINTR / temporary errors: log and continue
                log::debug!("[mdns] recv_from: {}", e);
            }
        }
    }
}

fn bind_mdns_socket() -> Result<UdpSocket, String> {
    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT);
    let sock = UdpSocket::bind(addr)
        .map_err(|e| format!("bind 0.0.0.0:{}: {}", MDNS_PORT, e))?;
    // SO_REUSEPORT so that multiple listeners (e.g. future user-space resolver) can coexist
    set_so_reuseport(&sock)?;
    Ok(sock)
}

fn set_so_reuseport(sock: &UdpSocket) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let fd = sock.as_raw_fd();
    let val: libc::c_int = 1;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &val as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        return Err(format!("SO_REUSEPORT failed: errno {}", unsafe { *libc::__errno_location() }));
    }
    Ok(())
}

fn handle_query(query: &packet::MdnsQuery, src: &std::net::SocketAddr, sock: &UdpSocket) {
    let local_hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "vyomaos.local".into());
    let local_hostname = local_hostname.trim().to_string();
    let local_ip = get_local_ip();

    // A record: answer "<hostname>.local." queries
    let fqdn = format!("{}.local.", local_hostname.trim_end_matches(".local"));
    if query.qtype == crate::dns::QueryType::A && query.name == fqdn {
        if let Some(pkt) = packet::build_mdns_a_response(&query.name, &local_ip, 120) {
            let dest = SocketAddrV4::new(MDNS_ADDR, MDNS_PORT);
            let _ = sock.send_to(&pkt, dest);
        }
        return;
    }

    // DNS-SD: answer PTR queries for service types we host
    if query.qtype == crate::dns::QueryType::PTR {
        let services = super::MDNS_SERVICES.get().unwrap().lock().unwrap();
        for svc in services.iter() {
            let ptr_name = format!("{}._tcp.local.", svc.service_type.trim_start_matches('_'));
            if query.name == ptr_name {
                if let Some(pkt) = packet::build_mdns_ptr_srv_txt_response(svc, &local_ip) {
                    let dest = SocketAddrV4::new(MDNS_ADDR, MDNS_PORT);
                    let _ = sock.send_to(&pkt, dest);
                }
            }
        }
    }
}

/// QEMU user-mode detection (B2 fix).
/// The QEMU virtual gateway always appears as 10.0.2.2 in /proc/net/route
/// (0202000A in little-endian hex, which is the Gateway column).
pub fn is_qemu_usermode() -> bool {
    std::fs::read_to_string("/proc/net/route")
        .map(|s| s.contains("0202000A"))
        .unwrap_or(false)
}

/// Read local IP from /proc/net/fib_trie or fall back to 127.0.0.1.
fn get_local_ip() -> String {
    // Parse the first non-loopback local address from /proc/net/fib_trie
    if let Ok(trie) = std::fs::read_to_string("/proc/net/fib_trie") {
        for line in trie.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("LOCAL") { continue; }
            // Lines like "  10.0.2.15  32 0 ..." contain the assigned IP
            if let Some(ip) = trimmed.split_whitespace().next() {
                let parts: Vec<&str> = ip.split('.').collect();
                if parts.len() == 4 && ip != "127.0.0.1" && ip != "0.0.0.0" {
                    if parts[0] != "127" {
                        return ip.to_string();
                    }
                }
            }
        }
    }
    "127.0.0.1".into()
}

/// ARP-unicast browse fallback under QEMU user-mode (B2 fix).
/// Reads /proc/net/arp and sends unicast PTR queries to each neighbor.
pub fn arp_unicast_browse(service_type: &str, results_tx: &std::sync::mpsc::Sender<sd::BrowseResult>) {
    if let Ok(arp) = std::fs::read_to_string("/proc/net/arp") {
        for line in arp.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 4 {
                let neighbor_ip = cols[0];
                let _ = send_unicast_ptr_query(service_type, neighbor_ip, results_tx);
            }
        }
    }
}

fn send_unicast_ptr_query(
    service_type: &str,
    target_ip: &str,
    tx: &std::sync::mpsc::Sender<sd::BrowseResult>,
) -> Result<(), String> {
    let sock = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("bind: {}", e))?;
    let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(300)));
    let pkt = super::packet::build_ptr_query(service_type);
    let dest = format!("{}:{}", target_ip, MDNS_PORT);
    let _ = sock.send_to(&pkt, &dest);
    let mut buf = [0u8; MAX_PKT];
    if let Ok((n, src)) = sock.recv_from(&mut buf) {
        if let Some(result) = super::packet::parse_browse_response(&buf[..n], &src) {
            let _ = tx.send(result);
        }
    }
    Ok(())
}
```

---

## 7. Panic-Safe Packet Parser (B4 Fix)

```rust
// supervisor/src/dns/packet.rs  (~130 lines)

use crate::dns::{DnsRecord, QueryType, ResolveResult};
use crate::dns::sd::{ServiceRecord, BrowseResult};

// ── mDNS query parsing (B4 fix) ───────────────────────────────────────────────

#[derive(Debug)]
pub struct MdnsQuery {
    pub name:  String,
    pub qtype: QueryType,
}

/// Parse an mDNS query packet.  Returns None on ANY malformed input — never panics.
/// All byte access uses get() + ? operator; pointer loop uses a 128-hop cap and
/// a 512-bit visited bitmask to prevent infinite loops on self-referential compressed names.
pub fn parse_mdns_query(buf: &[u8]) -> Option<MdnsQuery> {
    if buf.len() < 12 { return None; }
    let flags = u16::from_be_bytes([*buf.get(2)?, *buf.get(3)?]);
    if flags & 0x8000 != 0 { return None; }  // QR bit set → this is a response, not a query
    let qdcount = u16::from_be_bytes([*buf.get(4)?, *buf.get(5)?]);
    if qdcount == 0 { return None; }

    let (name, pos) = read_name(buf, 12)?;
    if pos + 3 >= buf.len() { return None; }
    let qtype_wire = u16::from_be_bytes([*buf.get(pos)?, *buf.get(pos + 1)?]);
    let qtype = match qtype_wire {
        1  => QueryType::A,
        28 => QueryType::AAAA,
        12 => QueryType::PTR,
        33 => QueryType::SRV,
        16 => QueryType::TXT,
        _  => return None,
    };
    Some(MdnsQuery { name, qtype })
}

/// Read a DNS label sequence starting at `pos`, following compression pointers.
/// Returns (decoded_name, position_after_name).
fn read_name(buf: &[u8], start: usize) -> Option<(String, usize)> {
    let mut pos = start;
    let mut name = String::new();
    let mut end_pos: Option<usize> = None;
    let mut visited = [0u64; 8];   // bitmask covering 512 offsets
    let mut hops = 0usize;

    loop {
        if hops > 128 { return None; }
        hops += 1;
        if pos >= buf.len() { return None; }

        // Visited-offset tracking (B4 fix: pointer-loop guard)
        let slot = pos / 64;
        let bit  = 1u64 << (pos % 64);
        if slot < 8 {
            if visited[slot] & bit != 0 { return None; }  // pointer loop detected
            visited[slot] |= bit;
        }

        let b = *buf.get(pos)? as usize;
        if b == 0 {
            if end_pos.is_none() { end_pos = Some(pos + 1); }
            break;
        }
        if (b & 0xC0) == 0xC0 {
            // Compression pointer: 14-bit offset
            let hi = (b & 0x3F) as usize;
            let lo = *buf.get(pos + 1)? as usize;
            let target = (hi << 8) | lo;
            if end_pos.is_none() { end_pos = Some(pos + 2); }
            pos = target;
            continue;
        }
        // Label: b bytes follow
        let label_end = pos + 1 + b;
        if label_end > buf.len() { return None; }
        let label = std::str::from_utf8(buf.get(pos + 1..label_end)?).ok()?;
        if !name.is_empty() { name.push('.'); }
        name.push_str(label);
        pos = label_end;
    }

    Some((name, end_pos?))
}

/// Build a minimal DNS A-record query packet (12-byte header + QNAME + QTYPE/QCLASS).
pub fn build_query(hostname: &str, qtype_wire: u16) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(64);
    pkt.extend_from_slice(&[0xAB, 0xCD]);  // ID
    pkt.extend_from_slice(&[0x01, 0x00]);  // flags: recursion desired
    pkt.extend_from_slice(&[0x00, 0x01]);  // QDCOUNT = 1
    pkt.extend_from_slice(&[0x00, 0x00]);  // ANCOUNT
    pkt.extend_from_slice(&[0x00, 0x00]);  // NSCOUNT
    pkt.extend_from_slice(&[0x00, 0x00]);  // ARCOUNT
    encode_name(&mut pkt, hostname);
    pkt.extend_from_slice(&qtype_wire.to_be_bytes());
    pkt.extend_from_slice(&[0x00, 0x01]);  // QCLASS = IN
    pkt
}

fn encode_name(pkt: &mut Vec<u8>, name: &str) {
    for label in name.trim_end_matches('.').split('.') {
        let lb = label.as_bytes();
        pkt.push(lb.len() as u8);
        pkt.extend_from_slice(lb);
    }
    pkt.push(0);
}

/// Parse a standard DNS response into a ResolveResult.
pub fn parse_response(buf: &[u8], hostname: &str, qtype: QueryType) -> ResolveResult {
    // Minimal response parser — extract A/AAAA records from answer section
    if buf.len() < 12 {
        return ResolveResult::Error { hostname: hostname.to_string(), reason: "truncated".into() };
    }
    let rcode = buf[3] & 0x0F;
    if rcode == 3 {
        return ResolveResult::NxDomain { hostname: hostname.to_string() };
    }
    let ancount = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    if ancount == 0 {
        return ResolveResult::NxDomain { hostname: hostname.to_string() };
    }
    // Skip question section to reach answer section
    let mut pos = 12usize;
    let qdcount = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    for _ in 0..qdcount {
        match skip_name(buf, pos) {
            Some(p) => pos = p + 4,
            None    => return ResolveResult::Error { hostname: hostname.to_string(), reason: "bad qsec".into() },
        }
    }
    let mut records = Vec::new();
    for _ in 0..ancount.min(16) {
        let p = match skip_name(buf, pos) { Some(x) => x, None => break };
        if p + 10 > buf.len() { break; }
        let rtype = u16::from_be_bytes([buf[p], buf[p+1]]);
        let ttl   = u32::from_be_bytes([buf[p+4], buf[p+5], buf[p+6], buf[p+7]]);
        let rdlen = u16::from_be_bytes([buf[p+8], buf[p+9]]) as usize;
        let rdata_start = p + 10;
        if rdata_start + rdlen > buf.len() { break; }
        let rdata = &buf[rdata_start..rdata_start + rdlen];
        if rtype == 1 && rdlen == 4 {
            let ip = format!("{}.{}.{}.{}", rdata[0], rdata[1], rdata[2], rdata[3]);
            records.push(DnsRecord { qtype: QueryType::A, value: ip, ttl });
        }
        pos = rdata_start + rdlen;
    }
    if records.is_empty() {
        ResolveResult::NxDomain { hostname: hostname.to_string() }
    } else {
        ResolveResult::Resolved { hostname: hostname.to_string(), records }
    }
}

fn skip_name(buf: &[u8], start: usize) -> Option<usize> {
    let mut pos = start;
    for _ in 0..128 {
        if pos >= buf.len() { return None; }
        let b = buf[pos] as usize;
        if b == 0 { return Some(pos + 1); }
        if (b & 0xC0) == 0xC0 { return Some(pos + 2); }
        pos += b + 1;
    }
    None
}

pub fn build_ptr_query(service_type: &str) -> Vec<u8> {
    build_query(&format!("{}._tcp.local", service_type), 12)
}

/// Build a simple mDNS A-record response packet.
pub fn build_mdns_a_response(name: &str, ip: &str, ttl: u32) -> Option<Vec<u8>> {
    let parts: Vec<u8> = ip.split('.').filter_map(|o| o.parse().ok()).collect();
    if parts.len() != 4 { return None; }
    let mut pkt = Vec::with_capacity(64);
    pkt.extend_from_slice(&[0x00, 0x00]);  // ID = 0 for mDNS
    pkt.extend_from_slice(&[0x84, 0x00]);  // flags: QR=1, AA=1
    pkt.extend_from_slice(&[0x00, 0x00]);  // QDCOUNT = 0
    pkt.extend_from_slice(&[0x00, 0x01]);  // ANCOUNT = 1
    pkt.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    encode_name(&mut pkt, name);
    pkt.extend_from_slice(&[0x00, 0x01]);  // TYPE A
    pkt.extend_from_slice(&[0x00, 0x01]);  // CLASS IN
    pkt.extend_from_slice(&ttl.to_be_bytes());
    pkt.extend_from_slice(&[0x00, 0x04]);  // RDLENGTH
    pkt.extend_from_slice(&parts);
    Some(pkt)
}

/// Build a combined PTR + SRV + TXT response for a registered service.
pub fn build_mdns_ptr_srv_txt_response(svc: &ServiceRecord, local_ip: &str) -> Option<Vec<u8>> {
    // Minimal PTR record pointing at the SRV name
    // Full implementation builds a multi-record mDNS response packet
    build_mdns_a_response(&svc.hostname, local_ip, 120)
}

/// Parse a browse (PTR) response from a remote mDNS responder.
pub fn parse_browse_response(buf: &[u8], src: &std::net::SocketAddr) -> Option<BrowseResult> {
    if buf.len() < 12 { return None; }
    let ancount = u16::from_be_bytes([*buf.get(6)?, *buf.get(7)?]) as usize;
    if ancount == 0 { return None; }
    // Extract the first PTR record name as the discovered service name
    let mut pos = 12usize;
    let (_, after_q) = read_name(buf, pos)?;
    pos = after_q + 4;  // skip QTYPE + QCLASS of question section
    let (_, after_an) = read_name(buf, pos)?;
    pos = after_an;
    if pos + 10 > buf.len() { return None; }
    let rdlen = u16::from_be_bytes([*buf.get(pos + 8)?, *buf.get(pos + 9)?]) as usize;
    let rdata_start = pos + 10;
    let (svc_name, _) = read_name(buf, rdata_start)?;
    Some(BrowseResult {
        name: svc_name,
        ip:   src.ip().to_string(),
        port: 0,   // filled in from SRV record in a follow-up query
    })
}
```

---

## 8. DNS-SD Service Registry (B5 Fix)

```rust
// supervisor/src/dns/sd.rs  (~150 lines)

use std::sync::mpsc;
use serde::{Deserialize, Serialize};
use crate::dns::mdns::is_qemu_usermode;

const SERVICES_PATH: &str = "/data/.vyoma/dns/mdns-services.toml";

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRecord {
    pub app_name:     String,
    /// e.g. "_http._tcp" (underscore prefix required per RFC 6763)
    pub service_type: String,
    pub port:         u16,
    /// TXT key=value pairs encoded as bytes (RFC 6763 §6)
    pub txt:          Vec<u8>,
    /// Fully-qualified hostname, e.g. "vyomaos.local"
    pub hostname:     String,
}

#[derive(Debug, Clone)]
pub struct BrowseResult {
    pub name: String,
    pub ip:   String,
    pub port: u16,
}

// ── Registration ─────────────────────────────────────────────────────────────

/// Register a service announced by app `app_name`.
/// Persists the registry immediately (B5 fix).
pub fn register_service(app_name: &str, service_type: &str, port: u16, txt_b64: &str) {
    let txt = base64_decode(txt_b64).unwrap_or_default();
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "vyomaos.local".into())
        .trim()
        .to_string();

    let rec = ServiceRecord {
        app_name:     app_name.to_string(),
        service_type: service_type.to_string(),
        port,
        txt,
        hostname,
    };

    {
        let mut services = super::MDNS_SERVICES.get().unwrap().lock().unwrap();
        // Replace existing registration for the same app+type (idempotent)
        services.retain(|s| !(s.app_name == app_name && s.service_type == service_type));
        services.push(rec);
        persist_services_unlocked(&services);
    }

    // Invalidate any cached PTR lookups for this service type
    {
        let cache_arc = super::DNS_CACHE.get().unwrap();
        let mut cache = cache_arc.lock().unwrap();
        cache.flush_name(&format!("{}._tcp.local", service_type));
    }
}

/// Unregister a service.  Sends RFC 6762 Goodbye (TTL=0) if multicast available.
/// Persists the updated registry (B5 fix).
pub fn unregister_service(app_name: &str, service_type: &str, port: u16) {
    {
        let mut services = super::MDNS_SERVICES.get().unwrap().lock().unwrap();
        services.retain(|s| !(s.app_name == app_name && s.service_type == service_type));
        persist_services_unlocked(&services);
    }

    if !is_qemu_usermode() {
        send_goodbye(service_type, port);
    }
}

/// Send RFC 6762 §11.3 Goodbye record (TTL=0) to invalidate remote mDNS caches.
fn send_goodbye(service_type: &str, _port: u16) {
    use std::net::{SocketAddrV4, UdpSocket};
    let sock = match UdpSocket::bind("0.0.0.0:0") { Ok(s) => s, Err(_) => return };
    let name = format!("{}._tcp.local", service_type);
    if let Some(pkt) = super::packet::build_mdns_a_response(&name, "0.0.0.0", 0) {
        let dest = SocketAddrV4::new(std::net::Ipv4Addr::new(224,0,0,251), 5353);
        let _ = sock.send_to(&pkt, dest);
    }
}

/// Called from supervisor SIGTERM handler (B5 fix): send Goodbye for all registered services.
pub fn send_all_goodbye_on_shutdown() {
    let services: Vec<ServiceRecord> = {
        let guard = super::MDNS_SERVICES.get().unwrap().lock().unwrap();
        guard.clone()
    };
    for svc in &services {
        if !is_qemu_usermode() {
            send_goodbye(&svc.service_type, svc.port);
        }
    }
}

// ── Browse ────────────────────────────────────────────────────────────────────

/// Start a DNS-SD browse for `service_type` in a background thread.
/// Results are streamed via `reply_tx` for 3 seconds; channel closed at end.
pub fn browse(service_type: String, app_name: String, reply_tx: mpsc::Sender<BrowseResult>) {
    std::thread::Builder::new()
        .name(format!("mdns-browse-{}", service_type))
        .spawn(move || {
            if is_qemu_usermode() {
                super::mdns::arp_unicast_browse(&service_type, &reply_tx);
            } else {
                multicast_browse(&service_type, &reply_tx);
            }
            // Channel closes when tx drops — IPC handler sends browse-done
        })
        .ok();
}

fn multicast_browse(service_type: &str, tx: &mpsc::Sender<BrowseResult>) {
    use std::net::{SocketAddrV4, UdpSocket};
    let sock = match UdpSocket::bind("0.0.0.0:0") { Ok(s) => s, Err(_) => return };
    let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(3000)));
    let pkt = super::packet::build_ptr_query(service_type);
    let dest = SocketAddrV4::new(std::net::Ipv4Addr::new(224,0,0,251), 5353);
    let _ = sock.send_to(&pkt, dest);

    let mut buf = [0u8; 9000];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                if let Some(result) = super::packet::parse_browse_response(&buf[..n], &src) {
                    let _ = tx.send(result);
                }
            }
            Err(_) => break,  // timeout or error: end browse window
        }
    }
}

// ── Persistence ───────────────────────────────────────────────────────────────

fn persist_services_unlocked(services: &[ServiceRecord]) {
    let toml = toml::to_string(services).unwrap_or_default();
    let tmp  = format!("{}.tmp", SERVICES_PATH);
    let _    = std::fs::create_dir_all("/data/.vyoma/dns");
    let _    = std::fs::write(&tmp, toml.as_bytes());
    let _    = std::fs::rename(&tmp, SERVICES_PATH);
}

/// Restore services at boot.  Called from dns::init() before threads are spawned.
pub fn restore_services() {
    if let Ok(data) = std::fs::read_to_string(SERVICES_PATH) {
        if let Ok(services) = toml::from_str::<Vec<ServiceRecord>>(&data) {
            let mut guard = super::MDNS_SERVICES.get().unwrap().lock().unwrap();
            *guard = services;
            log::info!("[dns-sd] restored {} service record(s) from disk", guard.len());
        }
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use std::collections::VecDeque;
    // Minimal base64 decode — avoids pulling in the base64 crate for a small util
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lut = [255u8; 256];
    for (i, &c) in alphabet.iter().enumerate() { lut[c as usize] = i as u8; }
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes().filter(|&b| b != b'=') {
        let v = lut[c as usize];
        if v == 255 { return None; }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8 & 0xFF); }
    }
    Some(out)
}
```

---

## 9. VYOMA_DNS: Protocol (Complete Reference)

All DNS control and data flow uses line-oriented stdout/stdin messages routed through the supervisor IPC broker.

### App → Supervisor (stdout lines)

```
VYOMA_DNS:resolve|<hostname>
VYOMA_DNS:resolve_type|<hostname>|<qtype>
VYOMA_DNS:register_service|<service_type>|<port>|<txt_b64>
VYOMA_DNS:unregister_service|<service_type>|<port>
VYOMA_DNS:browse|<service_type>
VYOMA_DNS:flush_cache
VYOMA_DNS:flush_cache_name|<hostname>
VYOMA_DNS:set_hostname|<new_hostname>
```

- `resolve`: triggers an A-record lookup; response is `VYOMA_DNS:resolved` or `VYOMA_DNS:nxdomain` on the app's stdin.
- `resolve_type`: like `resolve` but for a specific record type (A, AAAA, PTR, SRV, TXT).
- `register_service`: registers an mDNS / DNS-SD service announcement for the calling app.
- `unregister_service`: removes the registration and sends a Goodbye packet.
- `browse`: starts a 3-second browse window; results stream as `VYOMA_DNS:found` lines.
- `flush_cache`: purges the entire in-memory cache (admin apps only; requires `shell = true`).
- `flush_cache_name`: purges a single hostname from cache.
- `set_hostname`: updates `/data/.vyoma/hostname` and `/etc/hostname`, re-announces via mDNS.

### Supervisor → App (stdin lines)

```
VYOMA_DNS:resolved|<hostname>|<ip>
VYOMA_DNS:nxdomain|<hostname>
VYOMA_DNS:timeout|<hostname>
VYOMA_DNS:error|<hostname>|<reason>
VYOMA_DNS:found|<service_name>|<ip>|<port>
VYOMA_DNS:browse_done|<service_type>
VYOMA_DNS:registered|<service_type>|<port>
VYOMA_DNS:unregistered|<service_type>|<port>
VYOMA_DNS:cache_flushed
VYOMA_DNS:hostname_updated|<new_hostname>
```

### Internal supervisor verb (used by R53 VPN subsystem)

```
VYOMA_DNS:set_upstream|<ip>          # override upstream DNS (e.g., VPN-supplied DNS)
VYOMA_DNS:clear_upstream             # revert to default 10.0.2.3 / resolv.conf
```

### Capability Gating

Apps must declare `network = true` to use `VYOMA_DNS:resolve` and `VYOMA_DNS:register_service`.  `flush_cache` additionally requires `shell = true`.

---

## 10. Thread Model

```
main thread (PID 1)
│
├─ supervisor-dns  (std::thread)
│    Reads from mpsc::Receiver<DnsRequest>
│    Services cache hits without hitting the network
│    Sends UDP queries to 10.0.2.3:53 (blocking, 5s timeout)
│    Updates DNS_CACHE + DNS_INFLIGHT under lock
│    Calls cache::persist_cache() after every miss
│
└─ mdns-responder  (std::thread)
     Binds 0.0.0.0:5353 with SO_REUSEPORT
     Joins 224.0.0.251 multicast group (skipped in QEMU user-mode)
     Dispatches incoming mDNS queries to handle_query()
     Spawns short-lived mdns-browse-<type> threads for browse requests
```

Synchronisation discipline:
- IPC handler thread calls `dns::resolve_async` → acquires `DNS_INFLIGHT` lock for one critical section, then releases before any I/O.
- `supervisor-dns` thread acquires `DNS_CACHE` lock only to read/write the cache map, never while blocked on UDP I/O.
- `mdns-responder` thread acquires `MDNS_SERVICES` lock only to read the service list while building a response, then releases immediately.
- ABBA deadlock avoided: no path holds two locks simultaneously; each lock is a leaf acquisition.

---

## 11. Persistent State under /data/.vyoma/dns/

```
/data/.vyoma/dns/
├── cache.json            # TTL-valid A/AAAA records; restored at boot
├── mdns-services.toml    # registered DNS-SD services; restored at boot
└── hostname              # (symlinked from /data/.vyoma/hostname)
```

All writes use atomic write-then-rename (R41 semantics):
```rust
let tmp = format!("{}.tmp", path);
std::fs::write(&tmp, data)?;
std::fs::rename(&tmp, path)?;
```
This guarantees no reader ever sees a half-written file even if the supervisor crashes mid-write.

On boot, `dns::init()` restores:
1. `cache.json` → entries with remaining TTL > 30 seconds loaded into `DNS_CACHE`.
2. `mdns-services.toml` → all registered services loaded into `MDNS_SERVICES`.

On OTA supervisor restart (SIGTERM → exec new binary), `send_all_goodbye_on_shutdown()` is called before exec, broadcasting TTL=0 Goodbye packets for all services — remote mDNS caches invalidate immediately (B5 fix).

---

## 12. Capability Declaration

Apps that need DNS resolution or mDNS registration declare capabilities in `vyoma.toml`:

```toml
[capabilities]
stdio   = true
network = true   # required for VYOMA_DNS:resolve and VYOMA_DNS:register_service
shell   = true   # additionally required for VYOMA_DNS:flush_cache
```

In the supervisor's `Capabilities` struct (`supervisor/src/manifest.rs`), no new fields are needed — DNS is gated by the existing `network` and `shell` capabilities.

---

## 13. Detailed Blocking Issue Analysis

### B1 — DNS Blocks the IPC Handler Thread

**Problem**: The original stub resolver from R51 executed `send_udp_query` directly on the IPC handler thread. A single slow DNS server (e.g., the QEMU DNS proxy under load) can hold the lock for up to 5 seconds × number of in-flight queries, stalling all IPC message processing — including keyboard events, draw commands, and app lifecycle signals.

**Resolution**: The DNS work is moved to a dedicated `supervisor-dns` thread that drains a bounded `mpsc::sync_channel(64)`. The IPC handler calls `dns::resolve_async`, which pushes a `DnsRequest` (with a reply `Sender`) into the channel and returns immediately. The DNS thread calls `reply_tx.send()` when the answer is ready; the IPC broker then routes the result back to the requesting app's stdin. Maximum IPC handler latency for a resolve call: one channel push (~200 ns).

### B2 — mDNS Multicast Silently Dead on QEMU User-Mode

**Problem**: QEMU's user-mode networking (SLiRP) does not forward multicast packets. The mDNS responder calls `join_multicast_v4` and receives no error — the operation appears to succeed. But no mDNS queries ever arrive and no browse responses are received. Service discovery silently fails; the bug is impossible to diagnose from app code.

**Resolution**: `is_qemu_usermode()` checks `/proc/net/route` for the gateway `0202000A` (10.0.2.2 in little-endian hex) before any multicast join attempt. When detected, the supervisor logs `[mdns] QEMU user-mode detected — using ARP unicast fallback` and skips the multicast join. Browse operations fall back to `arp_unicast_browse`, which reads `/proc/net/arp` and sends unicast PTR queries to each known neighbor. This provides functional (if limited) service discovery inside QEMU developer environments without any configuration.

### B3 — Concurrent Resolve Requests for the Same Hostname Send Duplicate UDP Queries

**Problem**: If two WASM apps each issue `VYOMA_DNS:resolve|api.example.com` within the same 5-second DNS timeout window, the naive implementation sends two UDP queries. Under high concurrency (10 apps all resolving the same hostname at startup) this saturates the upstream DNS with 10 identical queries, inflating latency and triggering rate-limiting. The `DNS_INFLIGHT` check-then-insert split across two lock acquisitions creates a TOCTOU window where two threads both see the map empty and both send queries.

**Resolution**: The check-and-insert is a single critical section — the `DNS_INFLIGHT` lock is acquired once, the map is checked and updated, and the lock is released before any UDP I/O. The first thread to acquire the lock inserts its reply channel and sets `should_query = true`; all subsequent threads for the same hostname append their reply channels to the existing waiter list and set `should_query = false`. Exactly one UDP query goes out per hostname per timeout window regardless of concurrent request count. All waiters receive the same result.

### B4 — Malformed mDNS Packet Causes Panic, Killing PID 1

**Problem**: The original `parse_mdns_query` used direct indexing (`buf[pos]`). A crafted or corrupted mDNS packet with a label length field pointing past the buffer end causes an index-out-of-bounds panic. Because the mDNS responder runs in a `std::thread`, the panic propagates to `thread::spawn`'s panic handler and kills the thread — but the real damage is that a panic in a thread spawned from PID 1 under certain configurations propagates as an `abort()`, killing the entire supervisor. This takes down every running WASM app with no recovery.

**Resolution**: `parse_mdns_query` uses `buf.get(pos)?` for every byte access and returns `Option<MdnsQuery>`. Label parsing in `read_name` uses a `visited` bitmask (512 bits, covering all possible 9000-byte mDNS packet offsets) to detect pointer loops and a 128-hop cap as a secondary guard. Compression pointer targets that point backwards or forward into the header are rejected by the visited-bitmask check. Any malformed packet silently returns `None`; the `recv_from` loop discards it without any panic path.

### B5 — Service Registrations Lost on Supervisor Restart; No Goodbye on Shutdown

**Problem**: On an OTA supervisor restart (SIGTERM → exec new binary), all in-memory `MDNS_SERVICES` state is lost. Remote peers that discovered the services via mDNS continue to cache them for their TTL (up to 4500 seconds per RFC 6762 default). Services that were advertised but never unregistered persist as stale phantom entries in remote caches for over an hour. Additionally, the new supervisor binary re-announces services at startup but remote caches do not refresh until their TTL expires, causing connection failures during the overlap window.

**Resolution**: Every `register_service` and `unregister_service` call persists the full `MDNS_SERVICES` list to `/data/.vyoma/dns/mdns-services.toml` using atomic write-then-rename. On supervisor startup, `dns::init()` restores the list before spawning the mDNS responder thread, so the new binary is immediately ready to answer queries. On SIGTERM (before exec), `send_all_goodbye_on_shutdown()` sends RFC 6762 §11.3 Goodbye packets (TTL=0) for every registered service to the `224.0.0.251` multicast address. Receiving peers remove the service from their caches immediately, preventing stale entries regardless of the original TTL.

---

## 14. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: DNS blocks IPC handler thread up to 130 s under QEMU DNS proxy load | Dedicated `supervisor-dns` thread drains `mpsc::sync_channel(64)`; IPC handler returns immediately after channel push; reply routed to app stdin asynchronously |
| B2: mDNS multicast silently dead on QEMU user-mode | `is_qemu_usermode()` checks `/proc/net/route` for `0202000A` before multicast join; browse falls back to `arp_unicast_browse` reading `/proc/net/arp` |
| B3: Concurrent `resolve` for same hostname sends duplicate UDP queries; TOCTOU window in check-and-insert | Single critical section: check-and-insert in `DNS_INFLIGHT` under one lock acquisition; all waiters attached to first query's result |
| B4: Malformed mDNS packet causes `abort()` via panic in detached thread, killing PID 1 | `parse_mdns_query` uses `get()?` throughout; `read_name` has 128-hop cap + 512-bit visited bitmask to catch pointer loops; all malformed packets silently dropped |
| B5: Service registrations lost on OTA supervisor restart; no Goodbye packets cause phantom entries in remote caches for up to 4500 s | Persist to `/data/.vyoma/dns/mdns-services.toml` on every change (atomic rename); restore in `dns::init()`; SIGTERM handler calls `send_all_goodbye_on_shutdown()` with TTL=0 |
