# FINAL Spec: DNS & mDNS / Bonjour (Round 52)

**Subsystem**: DNS & mDNS / Bonjour  
**macOS Analogue**: `mDNSResponder` / `Bonjour`  
**Depends on**: R51 (networking stack, DNS thread)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Two components in `supervisor/src/dns/`:

| Component | Thread | Role |
|-----------|--------|------|
| Stub resolver | `supervisor-dns` (from R51) | UDP/53 queries, TTL cache, in-flight dedup |
| mDNS responder | `mdns-responder` | `.local` A/PTR/SRV/TXT responses, DNS-SD browse |

Boot: supervisor writes `/etc/resolv.conf` (`nameserver 10.0.2.3`) and `/etc/hostname` from `/data/.vyoma/hostname` (default: `vyomaos.local`) before spawning apps.

---

## 2. Stub Resolver with In-Flight Dedup (B1 + B3 Fix)

The DNS thread from R51 is extended with in-flight dedup to prevent duplicate UDP queries:

```rust
// supervisor/src/dns/resolver.rs  (~180 lines)
static DNS_INFLIGHT: OnceLock<Mutex<HashMap<String, Vec<mpsc::Sender<ResolveResult>>>>> =
    OnceLock::new();

pub fn resolve_async(hostname: String, reply_tx: mpsc::Sender<ResolveResult>) {
    // Single critical section: check + register in one lock acquisition (B3 fix)
    let should_query = {
        let mut inflight = DNS_INFLIGHT.get().unwrap().lock().unwrap();
        if let Some(waiters) = inflight.get_mut(&hostname) {
            waiters.push(reply_tx);
            false   // another thread drives this query; just wait
        } else {
            inflight.insert(hostname.clone(), vec![reply_tx]);
            true    // this thread sends the UDP query
        }
        // lock released before any I/O
    };
    if should_query {
        let result = send_udp_query(&hostname);  // UDP/53 to 10.0.2.3, 5s timeout
        // Deliver to ALL waiters, remove entry
        let mut inflight = DNS_INFLIGHT.get().unwrap().lock().unwrap();
        if let Some(waiters) = inflight.remove(&hostname) {
            for tx in waiters { let _ = tx.send(result.clone()); }
        }
        update_cache(&hostname, &result);
    }
}
```

The check + register is one critical section — no TOCTOU window (B3 fix). Dispatching to the reply_tx is outside the lock.

Protocol reply (from R51):
```
VYOMA_DNS:resolved:<hostname>:<ip>
VYOMA_DNS:nxdomain:<hostname>
VYOMA_DNS:error:<hostname>:<reason>
```

---

## 3. mDNS Responder (B2 + B4 Fix)

```rust
// supervisor/src/dns/mdns.rs  (~200 lines)
pub fn run_mdns_responder() {
    // Detect QEMU user-mode BEFORE binding (B2 fix)
    if is_qemu_usermode() {
        log_warn!("[mdns] QEMU user-mode detected — multicast unavailable; .local queries only");
        // Still bind to handle local queries; skip multicast join
    } else {
        sock.join_multicast_v4(&Ipv4Addr::new(224,0,0,251), &Ipv4Addr::UNSPECIFIED).ok();
    }
    loop {
        let mut buf = [0u8; 512];
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                // B4 fix: Option-returning parser, never panics
                if let Some(q) = packet::parse_mdns_query(&buf[..n]) {
                    handle_query(&q, &src, &sock);
                }
                // malformed packets silently dropped
            }
            Err(_) => {}
        }
    }
}
```

**QEMU detection (B2 fix)**:
```rust
fn is_qemu_usermode() -> bool {
    std::fs::read_to_string("/proc/net/route")
        .map(|s| s.contains("0202000A"))  // 10.0.2.2 in little-endian hex
        .unwrap_or(false)
}
```

When QEMU user-mode detected: mDNS browse falls back to ARP unicast probe (scan `/proc/net/arp`, send unicast PTR queries to each neighbor:5353).

**Panic-safe packet parser (B4 fix)**:
```rust
// supervisor/src/dns/packet.rs  (~120 lines)
pub fn parse_mdns_query(buf: &[u8]) -> Option<MdnsQuery> {
    // All indexing via get() + ? — never panics on malformed input
    let flags = u16::from_be_bytes([*buf.get(2)?, *buf.get(3)?]);
    if flags & 0x8000 != 0 { return None; }  // not a query
    // ... all label parsing uses dns_skip_name_safe with pointer-loop guard
    Some(MdnsQuery { name, qtype })
}

fn dns_skip_name_safe(buf: &[u8], mut pos: usize) -> Option<usize> {
    let mut visited = [0u64; 8];  // bitmask for 512 offsets
    for _ in 0..128 {
        if pos >= buf.len() { return None; }
        let idx = pos / 64;
        let bit = 1u64 << (pos % 64);
        if idx < 8 { if visited[idx] & bit != 0 { return None; } visited[idx] |= bit; }
        let b = buf[pos] as usize;
        if b == 0 { return Some(pos + 1); }
        if (b & 0xC0) == 0xC0 { return Some(pos + 2); }
        pos += b + 1;
    }
    None
}
```

Record types served: A (local hostname → interface IP), PTR (service type → service name), SRV (service name → host+port), TXT (service metadata).

---

## 4. DNS-SD Service Registry (B5 Fix)

```rust
// supervisor/src/dns/sd.rs  (~150 lines)
pub struct ServiceRecord {
    pub app_name:     String,
    pub service_type: String,    // "_http._tcp"
    pub port:         u16,
    pub txt:          Vec<u8>,
    pub hostname:     String,
}
static MDNS_SERVICES: OnceLock<Mutex<Vec<ServiceRecord>>> = OnceLock::new();
```

**Persistence (B5 fix)**: every `mdns-register`/`mdns-unregister` writes to `/data/.vyoma/mdns-services.toml` (R41 atomic-rename). On `dns::init()`, loaded back into `MDNS_SERVICES`. Survives supervisor OTA restart within a running VM session.

**RFC 6762 Goodbye packets on shutdown (B5 fix)**: SIGTERM handler broadcasts registered services with TTL=0 before exec'ing new supervisor binary, immediately invalidating remote caches.

IPC protocol:
```
@supervisor: mdns-register <service_type> <port> <txt_b64>
→ VYOMA_MDNS:registered:<service_type>:<port>

@supervisor: mdns-unregister <service_type> <port>
→ VYOMA_MDNS:unregistered:<service_type>:<port>

@supervisor: mdns-browse <service_type>
→ VYOMA_MDNS:found:<name>:<ip>:<port>   (stream, 3s window)
→ VYOMA_MDNS:browse-done:<service_type>
```

Browse runs in a background thread (non-blocking IPC handler). Under QEMU user-mode: ARP unicast fallback (reads `/proc/net/arp`, sends unicast PTR queries).

---

## 5. Hostname

`/data/.vyoma/hostname` (default: `vyomaos.local`). Supervisor writes `/etc/hostname` at boot. mDNS responder serves A record for `<hostname>.local.` → interface IP (read from `/sys/class/net/eth0/address` → parse via `/proc/net/fib_trie`).

---

## 6. File Layout

```
supervisor/src/dns/
├── mod.rs        (~80 lines: init, spawn threads, DNS_TX + MDNS_SERVICES statics)
├── cache.rs      (~120 lines: DnsCache LRU, TTL, negative caching)
├── resolver.rs   (~180 lines: UDP query, in-flight dedup (B3), cache update)
├── mdns.rs       (~200 lines: responder thread, QEMU detection (B2), record builder)
├── sd.rs         (~150 lines: DNS-SD registry, browse, persistence (B5), Goodbye (B5))
└── packet.rs     (~120 lines: wire-format encode/decode, parse_mdns_query (B4), dns_skip_name_safe)
```

---

## 7. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: DNS blocks IPC handler thread up to 130s | Async DNS worker from R51; IPC handler returns immediately; this spec adds in-flight dedup |
| B2: mDNS multicast silently dead on QEMU user-mode | Detect QEMU gateway (`10.0.2.2` in `/proc/net/route`); log warning; use ARP unicast fallback for browse |
| B3: Concurrent `dns-resolve` for same hostname causes duplicate UDP queries | Single critical section: check-and-insert in `DNS_INFLIGHT` under one lock acquisition |
| B4: Malformed mDNS packet causes `abort()` (kills PID 1 via panic in detached thread) | `parse_mdns_query` uses `Option`-propagation throughout; `dns_skip_name_safe` has 128-iteration cap + pointer-loop bitmask guard |
| B5: Service registrations lost on supervisor restart; no Goodbye packets on shutdown | Persist to `/data/.vyoma/mdns-services.toml` (R41 atomic); restore at `dns::init()`; SIGTERM sends TTL=0 Goodbye packets |
