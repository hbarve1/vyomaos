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
├── mod.rs          (~80 lines: VpnState, VPN_STATE OnceLock, on_vpn_app_exit (B2+B5))
├── tun.rs          (~120 lines: OwnedTunFd drop guard (B2), create_tun ioctl sequence)
├── profile.rs      (~100 lines: VpnProfile serde, parse_vpn_profile, 0o600 creation)
├── routing.rs      (~90 lines: default-route add/del via BusyBox ip (B3))
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
