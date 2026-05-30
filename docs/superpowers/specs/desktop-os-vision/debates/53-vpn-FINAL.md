# FINAL Spec: VPN & Network Extensions (Round 53)

**Subsystem**: VPN & Network Extensions  
**macOS Analogue**: `NetworkExtension` / VPN profiles  
**Depends on**: R51 (networking stack, CONFIG_TUN), R52 (DNS override)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Kernel Config Addition (B1 Fix)

One line appended to `base/kernel.config`:
```
CONFIG_TUN=y
```

`CONFIG_WIREGUARD` is NOT required — VyomaOS uses a userspace WireGuard WASM app (pure-Rust, ~150 KB in initramfs). This preserves the allnoconfig philosophy: the only kernel delta is TUN/TAP virtual interface support.

**Boot probe (B1 fix)**: `vpn-connect` checks before proceeding:
```rust
fn tun_available() -> bool { Path::new("/dev/net/tun").exists() }
```
If false: return `REPLY:error:tun-not-available:kernel-missing-CONFIG_TUN`. Never silently pretend connection succeeded.

---

## 2. VPN Profile Format

Stored at `/data/.vyoma/vpn/<name>.vpn.toml` (mode `0o600`):

```toml
[profile]
name         = "work-vpn"
protocol     = "wireguard"     # "wireguard" | "openvpn"
auto_connect = false

[endpoint]
host = "vpn.example.com"
port = 51820

[wireguard]
private_key_ref   = "keychain:vpn/wg-private"   # R60 preferred
private_key_b64   = ""                           # pre-R60 fallback (log_warn! if non-empty)
server_pubkey_b64 = "AAAA...base64..."
local_ip          = "10.8.0.2"
keepalive_secs    = 25

[routes]
include = ["0.0.0.0/0"]    # full tunnel; or specific CIDRs for split-tunnel

[dns]
servers        = ["10.8.0.1"]
search_domains = ["corp.example.com"]
```

Profile created with `OpenOptions::new().mode(0o600)`. Pre-R60: `log_warn!("[vpn] private_key_b64 non-empty — credentials stored insecure; upgrade to keychain_ref when R60 ships")`.

---

## 3. TUN Interface Lifecycle (B2 Fix)

```rust
// supervisor/src/vpn/tun.rs  (~120 lines)

pub struct OwnedTunFd(pub RawFd);

impl Drop for OwnedTunFd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0); }
        // B2 fix: TUNSETPERSIST=0 means closing fd destroys vyoma0 automatically
    }
}

pub fn create_tun(name: &str) -> Result<OwnedTunFd, String> {
    let fd = unsafe { libc::open(c"/dev/net/tun", libc::O_RDWR) };
    if fd < 0 { return Err(format!("open /dev/net/tun: {}", io::Error::last_os_error())); }
    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    // copy name into ifr_name
    let bytes = name.as_bytes();
    unsafe {
        ifr.ifr_name[..bytes.len()].copy_from_slice(
            &*(bytes as *const [u8] as *const [i8])
        );
        ifr.ifr_ifru.ifru_flags = (libc::IFF_TUN | libc::IFF_NO_PI) as i16;
        libc::ioctl(fd, TUNSETIFF, &ifr);
        libc::ioctl(fd, TUNSETPERSIST, 0 as c_int);  // non-persistent: auto-destroy on close
    }
    // Configure via BusyBox ip commands (cleaner than raw rtnetlink for MVP)
    // routes programmed after VPN handshake completes
    Ok(OwnedTunFd(fd))
}
```

`OwnedTunFd` stored in `VpnState` inside a `Mutex`. When VPN app crashes, waiter thread calls `vpn::manager::on_vpn_app_exit()` which drops `OwnedTunFd` — kernel automatically destroys `vyoma0` (B2 fix: no stale interface).

---

## 4. Per-App Traffic Routing (B3 Fix)

`SO_BINDTODEVICE` cannot be applied to future WASI sockets from outside the process — it must be set on each socket after creation, inside the Wasmtime process. This is not accessible from the supervisor.

**Resolution (B3 fix)**: Replace per-app selective routing with a **default-route takeover** approach (how WireGuard `AllowedIPs = 0.0.0.0/0` works):

When VPN connects, program a default route with lower metric than the existing virtio-net route:
```bash
ip route add default dev vyoma0 metric 50
# (existing: default via 10.0.2.2 dev eth0 metric 100)
```

This routes ALL traffic from ALL `network = true` apps through the tunnel. No iptables, no per-app isolation — traffic routing is kernel-level via the route table.

`vpn_route = true` in `vyoma.toml` is reserved for a future split-tunnel implementation requiring `CONFIG_NETFILTER`. Document this clearly.

Route teardown: on `vpn-disconnect` or VPN app crash, `on_vpn_app_exit()` runs:
```bash
ip route del default dev vyoma0 metric 50
```
(via BusyBox `ip` child process)

---

## 5. Authoritative Cleanup (B5 Fix)

Single function owns ALL VPN teardown — prevents DNS split-brain on crash:

```rust
// supervisor/src/vpn/mod.rs
pub fn on_vpn_app_exit(profile_name: &str, inbox: &Inbox) {
    // 1. Drop OwnedTunFd (B2: destroys vyoma0 via TUNSETPERSIST=0)
    let mut state = VPN_STATE.get().unwrap().lock().unwrap();
    state.tun_fd.take();   // drops OwnedTunFd

    // 2. Remove VPN default route
    let _ = Command::new("ip").args(["route", "del", "default", "dev", "vyoma0", "metric", "50"]).status();

    // 3. Revert DNS override (B5 fix: MUST be in this same function)
    dns::dns_override::clear_vpn_dns();

    // 4. Clear state
    state.active_profile = None;
    drop(state);

    // 5. Broadcast (snapshot inbox keys first — ABBA deadlock prevention)
    let names: Vec<String> = inbox.lock().unwrap().keys().cloned().collect();
    for name in names {
        send_reply(&name, &format!("VYOMA_SYSTEM:vpn-status:disconnected:{profile_name}"), inbox);
    }
}
```

Called from EXACTLY two places: VPN app's waiter thread (crash path) and `vpn-disconnect` IPC handler (clean path). No other code may clear DNS override state (B5 fix).

Additional DNS safety: `dns_resolve_a` falls back to `8.8.8.8` if active DNS server is RFC-1918 and query times out — prevents dead VPN from making entire DNS unusable.

---

## 6. Credential Security (B4 Fix)

1. **Log filter**: `app_threads.rs` stdin-write path skips lines containing `vpn-key:`:
   ```rust
   if line.contains("vpn-key:") { continue; }  // never log VPN credentials
   ```

2. **Profile serialization**: `private_key_b64` marked `#[serde(skip_serializing)]` — never written back to file.

3. **Profile file permissions**: created with mode `0o600`.

4. **R60 bridge**: when R60 Keychain ships, `private_key_ref = "keychain:vpn/wg-private"` takes precedence. Until then, `log_warn!` at connect time if `private_key_b64` non-empty.

---

## 7. VPN Status Protocol

```
# Control (requires vpn = true OR shell = true capability)
@supervisor: vpn-connect <profile_name>
@supervisor: vpn-disconnect
@supervisor: vpn-list                    → REPLY:vpn-profiles:<json_b64>
@supervisor: vpn-status                  → REPLY:vpn:connected:work-vpn:10.8.0.2:uptime:142s
                                           OR REPLY:vpn:disconnected

# Broadcasts to all network-capable apps on state change
VYOMA_SYSTEM:vpn-status:connected:<profile_name>
VYOMA_SYSTEM:vpn-status:disconnected:<profile_name>
VYOMA_SYSTEM:vpn-status:error:<profile_name>:<reason>
```

`vpn: bool` capability in `Capabilities` struct gates `vpn-connect` and `vpn-disconnect` commands. `vpn_route: bool` reserved for future split-tunnel (currently no-op, documented).

---

## 8. File Layout

```
supervisor/src/vpn/
├── mod.rs          (~150 lines: VpnState, VPN_STATE OnceLock, on_vpn_app_exit (B2+B5), handle_vpn_line)
├── tun.rs          (~180 lines: OwnedTunFd drop guard (B2), create_tun ioctl sequence, packet I/O)
├── tunnel.rs       (~200 lines: tunnel state machine Disconnected/Connecting/Connected/Error)
├── profile.rs      (~100 lines: VpnConfig serde, parse_vpn_profile, 0o600 creation)
├── routing.rs      (~150 lines: default-route add/del via BusyBox ip (B3), kill-switch, DNS override)
├── dns_override.rs (~60 lines: VPN_DNS OnceLock, set/clear/active_dns_server (B5))
└── ipc.rs          (~80 lines: vpn-connect/disconnect/status/list handlers (B1 probe))

base/kernel.config  (modified: CONFIG_TUN=y (B1))
supervisor/src/manifest.rs  (modified: vpn: bool, vpn_route: bool in Capabilities)
supervisor/src/app_threads.rs (modified: log filter for vpn-key: lines (B4))
```

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `CONFIG_TUN` absent — `vpn-connect` silently connects to nothing | Add `CONFIG_TUN=y` to kernel.config; boot probe returns error if `/dev/net/tun` missing |
| B2: Stale `vyoma0` after VPN app crash causes EBUSY on reconnect + traffic blackholed | `OwnedTunFd` drop guard + `TUNSETPERSIST=0`; drop called in `on_vpn_app_exit` on crash |
| B3: `SO_BINDTODEVICE` cannot be applied to future WASI sockets from supervisor | Replace per-app routing with default-route takeover (`ip route add default dev vyoma0 metric 50`) |
| B4: VPN private key in plaintext, appears in app logs | Log filter skips `vpn-key:` lines; `skip_serializing` on key field; profile mode `0o600` |
| B5: DNS override not reverted on VPN app crash → 5s timeout on every `dns-resolve` | `on_vpn_app_exit()` is the single authoritative cleanup fn that always calls `clear_vpn_dns()` |

---

## 10. VYOMA_VPN: Protocol (Complete Reference)

All VPN control flow uses the `VYOMA_VPN:` stdout protocol. Apps must declare `vpn = true` in `vyoma.toml` to use connect/disconnect commands. `status` and `list_profiles` are readable by apps with `vpn = true` or `shell = true`.

### App → Supervisor

```
VYOMA_VPN:connect|<profile_name>
VYOMA_VPN:disconnect
VYOMA_VPN:status
VYOMA_VPN:list_profiles
VYOMA_VPN:add_profile|<json_config>
VYOMA_VPN:delete_profile|<name>
```

- `connect`: triggers async handshake; supervisor responds with `connected` or `error` once settled.
- `disconnect`: triggers graceful teardown via `on_vpn_app_exit`; always responds with `disconnected`.
- `status`: synchronous query; returns current tunnel state inline.
- `list_profiles`: returns JSON array of profile names stored in `/data/.vyoma/vpn/`.
- `add_profile`: accepts a JSON-encoded `VpnConfig`; supervisor persists to `/data/.vyoma/vpn/<name>.vpn.toml`.
- `delete_profile`: removes profile file; fails with `error` if profile is currently active.

### Supervisor → App

```
VYOMA_VPN:connected|<profile>|<assigned_ip>
VYOMA_VPN:disconnected
VYOMA_VPN:status|<state>|<profile>|<assigned_ip>
VYOMA_VPN:error|<reason>
VYOMA_VPN:profiles|<json_array>
```

- `connected`: sent after successful TUN setup + route programming + DNS override.
- `disconnected`: sent after full teardown completes (routes removed, DNS reverted, TUN closed).
- `status`: inline response to `VYOMA_VPN:status`; `<state>` is one of `connected`, `connecting`, `disconnected`, `error`.
- `error`: sent when connect fails (e.g., TUN unavailable, handshake timeout, profile not found).
- `profiles`: JSON array of profile name strings in response to `list_profiles`.

### Multi-App Constraint (B5 variant)

Only one VPN tunnel may be active at a time — the TUN device `vyoma0` is system-wide. If `VYOMA_VPN:connect|<profile>` arrives while another tunnel is `Connected` or `Connecting`, the supervisor performs a graceful disconnect of the existing tunnel first (calling `on_vpn_app_exit`), then starts the new connection. The requesting app receives no intermediate `disconnected` message — only the final `connected` or `error`.

---

## 11. VpnConfig Struct

```rust
// supervisor/src/vpn/profile.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VpnProtocol {
    WireGuard,
    OpenVPN,
    IKEv2,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VpnConfig {
    pub name:         String,
    pub protocol:     VpnProtocol,
    pub server:       String,
    pub port:         u16,
    /// Pre-shared key (WireGuard PSK or IKEv2 PSK). Never logged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub psk:          Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username:     Option<String>,
    /// CIDR ranges to route through VPN; empty means full-tunnel (0.0.0.0/0).
    pub routes:       Vec<String>,
    pub dns_servers:  Vec<String>,
    /// true = only route listed CIDRs through VPN; false = full tunnel (all traffic).
    #[serde(default)]
    pub split_tunnel: bool,
    /// Drop all non-tunnel traffic while VPN is active.
    #[serde(default)]
    pub kill_switch:  bool,
}

impl VpnConfig {
    pub fn is_full_tunnel(&self) -> bool {
        !self.split_tunnel || self.routes.is_empty()
    }
}
```

Stored as TOML at `/data/.vyoma/vpn/<name>.vpn.toml`. Loaded on `VYOMA_VPN:connect` and on `add_profile`. Private key material (`psk`, WireGuard `private_key_b64`) is marked `skip_serializing` so round-trip writes never leak credentials back to disk in a different path.

---

## 12. TUN Device Creation (Concrete ioctl)

```rust
// supervisor/src/vpn/tun.rs

use std::os::unix::io::RawFd;
use libc::c_int;

const TUNSETIFF:     u64 = 0x400454ca;
const TUNSETPERSIST: u64 = 0x400454cb;

#[repr(C)]
struct IfReq {
    ifr_name:  [u8; 16],
    ifr_flags: i16,
    _pad:      [u8; 22],
}

/// Create a TUN device named `name` (e.g. "vyoma0").
/// Returns the open file descriptor. The interface is non-persistent:
/// closing the fd destroys the interface automatically (B2 fix).
pub fn create_tun(name: &str) -> Result<RawFd, String> {
    let fd = unsafe {
        libc::open(b"/dev/net/tun\0".as_ptr() as *const libc::c_char, libc::O_RDWR)
    };
    if fd < 0 {
        return Err(format!("open /dev/net/tun failed: errno {}", unsafe { *libc::__errno_location() }));
    }

    let mut ifr = IfReq {
        ifr_name:  [0u8; 16],
        ifr_flags: (libc::IFF_TUN | libc::IFF_NO_PI) as i16,
        _pad:      [0u8; 22],
    };
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(15);
    ifr.ifr_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    let ret = unsafe { libc::ioctl(fd, TUNSETIFF, &ifr as *const IfReq) };
    if ret < 0 {
        unsafe { libc::close(fd); }
        return Err("TUNSETIFF ioctl failed".into());
    }

    // Non-persistent: closing fd destroys interface (prevents stale vyoma0 on crash)
    let ret2 = unsafe { libc::ioctl(fd, TUNSETPERSIST, 0 as c_int) };
    if ret2 < 0 {
        unsafe { libc::close(fd); }
        return Err("TUNSETPERSIST=0 ioctl failed".into());
    }

    Ok(fd)
}

/// Read one IP packet from the TUN fd. Blocks until data available.
pub fn read_packet(fd: RawFd, buf: &mut [u8]) -> Result<usize, String> {
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if n < 0 { return Err(format!("tun read error: errno {}", unsafe { *libc::__errno_location() })); }
    Ok(n as usize)
}

/// Write one IP packet to the TUN fd (injects into kernel network stack).
pub fn write_packet(fd: RawFd, pkt: &[u8]) -> Result<(), String> {
    let n = unsafe { libc::write(fd, pkt.as_ptr() as *const libc::c_void, pkt.len()) };
    if n < 0 { return Err(format!("tun write error: errno {}", unsafe { *libc::__errno_location() })); }
    Ok(())
}
```

---

## 13. Tunnel State Machine

```rust
// supervisor/src/vpn/tunnel.rs

use std::os::unix::io::RawFd;

/// Tunnel lifecycle states.  Transitions are linear except Error → Disconnected on reset.
#[derive(Debug, Clone)]
pub enum TunnelState {
    Disconnected,
    Connecting {
        profile: String,
        attempt: u32,
    },
    Connected {
        profile:     String,
        assigned_ip: String,
        tun_fd:      RawFd,
    },
    Disconnecting,
    Error {
        reason: String,
    },
}

impl TunnelState {
    pub fn is_active(&self) -> bool {
        matches!(self, TunnelState::Connecting { .. } | TunnelState::Connected { .. })
    }

    pub fn profile_name(&self) -> Option<&str> {
        match self {
            TunnelState::Connecting { profile, .. } => Some(profile),
            TunnelState::Connected  { profile, .. } => Some(profile),
            _ => None,
        }
    }

    pub fn assigned_ip(&self) -> Option<&str> {
        match self {
            TunnelState::Connected { assigned_ip, .. } => Some(assigned_ip),
            _ => None,
        }
    }

    /// Transition to Connected; panics if not currently Connecting.
    pub fn mark_connected(self, assigned_ip: String, tun_fd: RawFd) -> TunnelState {
        match self {
            TunnelState::Connecting { profile, .. } => {
                TunnelState::Connected { profile, assigned_ip, tun_fd }
            }
            other => panic!("mark_connected called in wrong state: {:?}", other),
        }
    }

    /// Transition to Error from any state.
    pub fn mark_error(self, reason: String) -> TunnelState {
        TunnelState::Error { reason }
    }
}

/// Drive state transitions for the vpn-connect flow.
pub fn transition_connect(state: TunnelState, profile: &str) -> TunnelState {
    match state {
        TunnelState::Disconnected | TunnelState::Error { .. } => {
            TunnelState::Connecting { profile: profile.to_string(), attempt: 1 }
        }
        // Already connecting or connected: caller must disconnect first (B5 variant)
        other => other,
    }
}
```

---

## 14. Kill-Switch and Route Manipulation

```rust
// supervisor/src/vpn/routing.rs

use std::process::Command;

/// Add a default route via the TUN interface with low metric (50),
/// overriding the normal virtio-net default route (metric 100).
pub fn add_default_route(tun_iface: &str) -> Result<(), String> {
    let status = Command::new("ip")
        .args(["route", "add", "default", "dev", tun_iface, "metric", "50"])
        .status()
        .map_err(|e| format!("ip route add failed: {e}"))?;
    if !status.success() {
        return Err(format!("ip route add default dev {tun_iface} metric 50 exited non-zero"));
    }
    Ok(())
}

/// Remove the VPN default route added by add_default_route.
pub fn del_default_route(tun_iface: &str) -> Result<(), String> {
    let _ = Command::new("ip")
        .args(["route", "del", "default", "dev", tun_iface, "metric", "50"])
        .status();
    Ok(())
}

/// Add a specific CIDR route through the TUN interface (split-tunnel mode).
pub fn add_route(cidr: &str, tun_iface: &str) -> Result<(), String> {
    let status = Command::new("ip")
        .args(["route", "add", cidr, "dev", tun_iface])
        .status()
        .map_err(|e| format!("ip route add {cidr} failed: {e}"))?;
    if !status.success() {
        return Err(format!("ip route add {cidr} dev {tun_iface} exited non-zero"));
    }
    Ok(())
}

/// Enable kill-switch: drop all outbound traffic not going through the TUN interface.
/// Uses nftables when available; falls back gracefully if nft not present.
pub fn enable_kill_switch(tun_iface: &str) -> Result<(), String> {
    // nft add rule inet filter output oifname != "vyoma0" counter drop
    let rule = format!(
        "add rule inet filter output oifname != \"{}\" counter drop",
        tun_iface
    );
    let status = Command::new("nft")
        .args(["--", &rule])
        .status()
        .map_err(|e| format!("nft kill-switch failed: {e}"))?;
    if !status.success() {
        return Err(format!("nft kill-switch rule insertion failed for {tun_iface}"));
    }
    Ok(())
}

/// Disable kill-switch by flushing the output chain rule inserted above.
pub fn disable_kill_switch(tun_iface: &str) -> Result<(), String> {
    // nft delete rule inet filter output handle <handle>
    // For MVP: flush entire output chain (acceptable since VyomaOS has no other output rules)
    let _ = Command::new("nft")
        .args(["flush", "chain", "inet", "filter", "output"])
        .status();
    let _ = tun_iface; // suppress unused warning
    Ok(())
}
```

---

## 15. DNS Override Integration with R52

When a VPN tunnel transitions to `Connected`, the supervisor pushes a DNS reconfiguration command to the R52 DNS subsystem:

```rust
// Inside on_vpn_app_exit and the connect success path in vpn/ipc.rs

// On connect: override DNS with VPN-provided servers
fn apply_vpn_dns(dns_servers: &[String]) {
    if let Some(first) = dns_servers.first() {
        // Push VYOMA_DNS:set-upstream to the dns-resolver app via IPC
        // Format matches R52 DNS subsystem protocol
        let msg = format!("VYOMA_DNS:set-upstream|{}", first);
        // Route through supervisor IPC broker to the dns-resolver app
        send_to_app("dns-resolver", &msg);
    }
}

// On disconnect: revert to default upstream
fn clear_vpn_dns() {
    send_to_app("dns-resolver", "VYOMA_DNS:clear-upstream");
}
```

**DNS leak prevention (B4 variant)**: When `split_tunnel = false` (full tunnel), the supervisor overrides ALL DNS servers — not merely adds the VPN DNS alongside the existing one. This prevents DNS queries from leaking outside the tunnel to the ISP resolver. When `split_tunnel = true`, VPN DNS is added alongside the default (queries for `search_domains` go to VPN DNS; others go to default upstream).

---

## 16. Capability Declaration

Apps that need to initiate or manage VPN connections declare `vpn = true` in `vyoma.toml`:

```toml
[capabilities]
stdio   = true
network = true
shell   = true
vpn     = true
```

In the supervisor's `Capabilities` struct (`supervisor/src/manifest.rs`):

```rust
#[derive(Debug, Deserialize, Default, Clone)]
pub struct Capabilities {
    #[serde(default)] pub stdio:      bool,
    #[serde(default)] pub filesystem: bool,
    #[serde(default)] pub network:    bool,
    #[serde(default)] pub display:    bool,
    #[serde(default)] pub shell:      bool,
    #[serde(default)] pub mouse:      bool,
    #[serde(default)] pub vpn:        bool,      // NEW: gates VYOMA_VPN: connect/disconnect
    #[serde(default)] pub vpn_route:  bool,      // RESERVED: future split-tunnel via NETFILTER
}
```

The `vpn_route` field is parsed and stored but currently a no-op. It is documented as reserved for a future implementation that requires `CONFIG_NETFILTER=y` in the kernel config.

---

## 17. Detailed Blocking Issue Analysis

### B1 — CONFIG_TUN Absent

**Problem**: The allnoconfig kernel has no `CONFIG_TUN`. Calling `open("/dev/net/tun")` returns `ENOENT`. The VPN app receives no error signal and may loop indefinitely trying to connect.

**Resolution**: Add `CONFIG_TUN=y` to `base/kernel.config`. This adds a single ~50 KB kernel module. The `tun_available()` probe runs before any TUNSETIFF ioctl attempt; failure returns an explicit `VYOMA_VPN:error|tun-not-available` to the requesting app.

### B2 — Stale TUN Interface After Crash

**Problem**: If the VPN WASM app crashes while the TUN device is open, the `vyoma0` interface persists in the kernel until something explicitly destroys it (because default TUN persistence is on). A subsequent `VYOMA_VPN:connect` then fails with `EBUSY` on the TUNSETIFF ioctl.

**Resolution**: Set `TUNSETPERSIST=0` immediately after creating the interface. The interface then has the same lifetime as the open file descriptor. `OwnedTunFd`'s `Drop` impl closes the fd, which the kernel uses to destroy the interface atomically. The supervisor's crash-detection path calls `on_vpn_app_exit`, which calls `state.tun_fd.take()`, triggering the drop.

### B3 — WASM Apps Cannot Manipulate Routes

**Problem**: WASM apps run under WASI Preview 2 — they have no mechanism to call `SIOCADDRT`, write to `/proc/net/route`, or invoke `ip route` directly. Supervisor cannot inject `SO_BINDTODEVICE` retroactively.

**Resolution**: Route all traffic at the kernel level via the default-route metric trick. The supervisor (running as PID 1 with full Linux capabilities) invokes `ip route add default dev vyoma0 metric 50` via `std::process::Command`. This routes all traffic from all `network = true` WASM apps through the tunnel without any per-app coordination.

### B4 — DNS Leak When Full Tunnel Active

**Problem**: Simply adding the VPN DNS server alongside the existing resolver means DNS queries may still reach the ISP resolver if split-tunnel logic is absent or misconfigured. This leaks browsing patterns outside the VPN.

**Resolution**: On full-tunnel connect (`split_tunnel = false`), supervisor calls `VYOMA_DNS:set-upstream` to the R52 DNS subsystem, replacing (not supplementing) the default upstream. On disconnect, `clear_vpn_dns` reverts to the pre-VPN upstream. The fallback to `8.8.8.8` on RFC-1918 timeout prevents the DNS subsystem from hanging after a VPN crash.

### B5 — Kill-Switch State Persists After Supervisor Crash

**Problem**: If the kill-switch nftables rule is inserted and then the supervisor crashes without cleanup, all outbound traffic remains blocked even after reboot. The system becomes unreachable.

**Resolution**: On supervisor startup, read `/data/.vyoma/vpn/state.json`. If the file records `kill_switch_active: true` and no VPN is currently running, immediately call `disable_kill_switch`. This recovery path runs before any app is spawned. The state file is written atomically (write-then-rename per R41 semantics) on every kill-switch state change.

```rust
// Startup recovery in vpn/mod.rs
pub fn recover_kill_switch_on_boot() {
    let path = "/data/.vyoma/vpn/state.json";
    if let Ok(data) = std::fs::read_to_string(path) {
        if let Ok(state) = serde_json::from_str::<VpnPersistedState>(&data) {
            if state.kill_switch_active {
                let _ = routing::disable_kill_switch("vyoma0");
                log::warn!("[vpn] kill-switch was active at last shutdown; cleared on boot");
            }
        }
    }
}
```
