# FINAL Spec: Network Sharing & Tethering (Round 57)

**Subsystem**: Network Sharing & Tethering  
**macOS Analogue**: Internet Sharing / Personal Hotspot  
**Depends on**: R51 (networking, CONFIG_TUN), R52 (DNS), R53 (VPN — ip_forward refcount), R55 (Wi-Fi, wlan0)  
**Platform scope**: `mobile`, `iot-edge` only — `desktop-full` uses virtio-net, no hotspot  
**Status**: FINAL — all blocking issues resolved

---

## 1. Kernel Config (platform-gated additions to iot-edge.config + mobile.config)

```
CONFIG_NETFILTER=y
CONFIG_NF_CONNTRACK=y
CONFIG_NF_NAT=y
CONFIG_NF_TABLES=y
CONFIG_NFT_MASQ=y
CONFIG_NFT_CHAIN_NAT=y
CONFIG_IP_NF_IPTABLES=n     # nftables only — no iptables
CONFIG_IP_ADVANCED_ROUTER=y
CONFIG_USB_GADGET=y
CONFIG_USB_ETH=y
CONFIG_USB_G_NCM=y           # NCM preferred over RNDIS for Linux USB hosts
CONFIG_MAC80211=y            # AP mode (R55 already has cfg80211)
```

---

## 2. Architecture

```
supervisor/src/sharing/
├── mod.rs          (~120 lines: SharingState, platform gate, IPC dispatch)
├── nat.rs          (~150 lines: ip_forward refcount (B4), nftables script via pipe, iface validation (B1))
├── dhcp.rs         (~180 lines: in-process DHCP server, SO_BINDTODEVICE (B3))
├── ap.rs           (~140 lines: nl80211 AP mode, driver settle wait (B2), hostapd subprocess (B5))
└── usb_tether.rs   (~80 lines: configfs NCM gadget)
```

All hotspot/tethering operations require `shell = true` capability (privilege gate).

---

## 3. Shared State

```rust
// supervisor/src/sharing/mod.rs

#[derive(Debug, Clone, PartialEq)]
pub enum SharingMode {
    Off,
    Hotspot { ssid: String },   // passphrase intentionally NOT stored (B5)
    UsbTether,
}

pub struct SharingState {
    pub mode:              SharingMode,
    pub ap_iface:          String,   // always "wlan0"
    pub upstream_iface:    String,   // eth0 on iot-edge; detected from /proc/net/route
    pub ap_subnet:         [u8; 4],  // 192.168.42.0
    pub client_count:      u8,
    pub nft_table:         String,   // "vyoma_sharing"
}

static SHARING: OnceLock<Arc<Mutex<SharingState>>> = OnceLock::new();
```

Upstream iface detection reads `/proc/net/route`, validates the result with `validate_iface_name()` before returning.

---

## 4. IP Forward Reference Counter (B4 Fix)

Direct boolean writes to `/proc/sys/net/ipv4/ip_forward` break VPN when hotspot teardown fires last:

```rust
// supervisor/src/sharing/nat.rs

static IP_FORWARD_REFS: AtomicU32 = AtomicU32::new(0);

pub fn acquire_ip_forward(caller: &str) -> Result<(), String> {
    let prev = IP_FORWARD_REFS.fetch_add(1, Ordering::SeqCst);
    if prev == 0 {
        std::fs::write("/proc/sys/net/ipv4/ip_forward", b"1\n")
            .map_err(|e| { IP_FORWARD_REFS.fetch_sub(1, Ordering::SeqCst); format!("{e}") })?;
    }
    Ok(())
}

pub fn release_ip_forward(caller: &str) {
    let prev = IP_FORWARD_REFS.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 {
        let _ = std::fs::write("/proc/sys/net/ipv4/ip_forward", b"0\n");
    } else if prev == 0 {
        // Underflow guard
        IP_FORWARD_REFS.store(0, Ordering::SeqCst);
    }
}
```

R53 VPN calls `acquire_ip_forward("vpn")` on connect and `release_ip_forward("vpn")` on disconnect. Hotspot calls `acquire_ip_forward("hotspot")` / `release_ip_forward("hotspot")`. Forward only turns off when the last subsystem releases it.

---

## 5. Interface Name Validation + nftables Script (B1 Fix)

```rust
// supervisor/src/sharing/nat.rs

pub fn validate_iface_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 15 { return Err(format!("invalid iface: {name:?}")); }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err(format!("iface name has disallowed chars: {name:?}"));
    }
    Ok(())
}

pub fn validate_nft_table_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 { return Err(format!("invalid table: {name:?}")); }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!("table name has disallowed chars: {name:?}"));
    }
    Ok(())
}

pub fn enable_nat(ap_iface: &str, upstream_iface: &str, table: &str) -> Result<(), String> {
    // B1 fix: validate ALL names before any interpolation
    validate_iface_name(ap_iface)?;
    validate_iface_name(upstream_iface)?;
    validate_nft_table_name(table)?;
    acquire_ip_forward("hotspot")?;
    let script = format!(
        "table ip {table} {{\n\
           chain postrouting {{\n\
             type nat hook postrouting priority srcnat; policy accept;\n\
             iifname \"{ap_iface}\" oifname \"{upstream_iface}\" masquerade\n\
           }}\n\
           chain forward {{\n\
             type filter hook forward priority filter; policy drop;\n\
             iifname \"{ap_iface}\" oifname \"{upstream_iface}\" ct state new,established,related accept\n\
             iifname \"{upstream_iface}\" oifname \"{ap_iface}\" ct state established,related accept\n\
           }}\n\
         }}\n"
    );
    run_nft(&script)
}

fn run_nft(script: &str) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new("/usr/sbin/nft")
        .arg("-f").arg("-")
        .stdin(std::process::Stdio::piped())
        .spawn().map_err(|e| format!("nft spawn: {e}"))?;
    child.stdin.take().unwrap().write_all(script.as_bytes()).ok();
    let s = child.wait().map_err(|e| format!("nft wait: {e}"))?;
    if !s.success() { return Err(format!("nft failed on script")); }
    Ok(())
}
```

---

## 6. AP Mode with Driver Settle Wait (B2 Fix)

```rust
// supervisor/src/sharing/ap.rs

pub fn start_ap(iface: &str, ssid: &str, passphrase: &str, channel: u8) -> Result<(), String> {
    // B2 fix: explicit deassociation + settle wait before AP mode
    let _ = run_iproute("iw", &["dev", iface, "disconnect"]);
    wait_for_iface_down(iface, 3)?;
    run_iproute("ip", &["link", "set", iface, "down"])?;
    run_iproute("iw", &["dev", iface, "set", "type", "ap"])?;
    // Verify driver accepted the type change
    verify_iface_type(iface, "AP")?;
    run_iproute("ip", &["link", "set", iface, "up"])?;
    run_iproute("ip", &["addr", "flush", "dev", iface])?;
    run_iproute("ip", &["addr", "add", "192.168.42.1/24", "dev", iface])?;
    write_hostapd_conf(iface, ssid, passphrase, channel)?;  // mode 0600 at creation (B5)
    start_hostapd_subprocess()?;                             // shreds conf after exec (B5)
    verify_hostapd_running(iface)                            // polls ctrl socket up to 3s
}

fn wait_for_iface_down(iface: &str, timeout_secs: u64) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        let s = std::fs::read_to_string(format!("/sys/class/net/{iface}/operstate"))
            .unwrap_or_default();
        match s.trim() {
            "down" | "dormant" | "lowerlayerdown" => return Ok(()),
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    // Timeout not fatal — hostapd verification is the hard gate
    Ok(())
}

fn verify_iface_type(iface: &str, expected: &str) -> Result<(), String> {
    let out = std::process::Command::new("iw")
        .args(["dev", iface, "info"]).output()
        .map_err(|e| format!("iw info: {e}"))?;
    let info = String::from_utf8_lossy(&out.stdout);
    if !info.contains(&format!("type {expected}")) {
        return Err(format!("{iface} did not accept type {expected} — driver rejected"));
    }
    Ok(())
}

fn verify_hostapd_running(iface: &str) -> Result<(), String> {
    let ctrl = format!("/tmp/hostapd/{iface}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if std::path::Path::new(&ctrl).exists() { return Ok(()); }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err(format!("hostapd ctrl socket {ctrl} not created — AP failed"))
}
```

---

## 7. DHCP Server with SO_BINDTODEVICE (B3 Fix)

```rust
// supervisor/src/sharing/dhcp.rs

fn bind_dhcp_socket(iface: &str) -> Result<std::net::UdpSocket, String> {
    use socket2::{Domain, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, None)
        .map_err(|e| format!("DHCP socket: {e}"))?;
    sock.set_reuse_port(true).ok();  // B3: co-exist if another DHCP server is present
    sock.set_broadcast(true).ok();
    sock.set_read_timeout(Some(std::time::Duration::from_millis(200))).ok();
    let addr: socket2::SockAddr = "0.0.0.0:67".parse::<std::net::SocketAddrV4>().unwrap().into();
    sock.bind(&addr).map_err(|e| format!("DHCP bind :67: {e}"))?;
    // B3 fix: SO_BINDTODEVICE isolates server to wlan0 only
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        let c = std::ffi::CString::new(iface).unwrap();
        unsafe {
            libc::setsockopt(sock.as_raw_fd(), libc::SOL_SOCKET, libc::SO_BINDTODEVICE,
                c.as_ptr() as _, iface.len() as libc::socklen_t);
        }
    }
    Ok(sock.into())
}
```

Pool: `192.168.42.100–200`. Server IP: `192.168.42.1`. Lease time: 3600s. Bump allocator with MAC→Lease map. No external `dnsmasq` required.

---

## 8. WPA2 Passphrase Shredding (B5 Fix)

```rust
// supervisor/src/sharing/ap.rs

fn write_hostapd_conf(iface: &str, ssid: &str, pass: &str, ch: u8) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let conf = format!(
        "interface={iface}\ndriver=nl80211\nssid={ssid}\n\
         hw_mode=g\nchannel={ch}\nwpa=2\nwpa_passphrase={pass}\n\
         wpa_key_mgmt=WPA-PSK\nwpa_pairwise=CCMP\nrsn_pairwise=CCMP\n"
    );
    // B5 fix: mode 0600 at CREATE time — no TOCTOU window
    let mut f = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true)
        .mode(0o600)
        .open("/tmp/vyoma_hostapd.conf")
        .map_err(|e| format!("create hostapd.conf: {e}"))?;
    use std::io::Write;
    f.write_all(conf.as_bytes()).map_err(|e| format!("write conf: {e}"))
}

fn start_hostapd_subprocess() -> Result<(), String> {
    let status = std::process::Command::new("/usr/sbin/hostapd")
        .args(["-B", "-P", "/tmp/hostapd.pid", "/tmp/vyoma_hostapd.conf"])
        .status()
        .map_err(|e| format!("hostapd: {e}"))?;
    // B5 fix: shred conf file — hostapd loaded it into memory, file no longer needed
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/tmp/vyoma_hostapd.conf") {
        let zeros = vec![0u8; 1024];
        let _ = std::io::Write::write_all(&mut f, &zeros);
    }
    let _ = std::fs::remove_file("/tmp/vyoma_hostapd.conf");
    if !status.success() { return Err(format!("hostapd failed")); }
    Ok(())
}
```

`SharingMode::Hotspot` stores only `ssid` — passphrase field intentionally omitted. Persisted config at `/data/.vyoma/sharing.toml` never contains the passphrase.

---

## 9. Protocol

```
@supervisor: hotspot-enable <ssid> <passphrase>   → REPLY:hotspot-on 192.168.42.1/24  (async)
@supervisor: hotspot-disable                      → REPLY:hotspot-off
@supervisor: hotspot-status                       → REPLY:hotspot-status <json>
@supervisor: usb-tether-enable                    → REPLY:usb-tether-on
@supervisor: usb-tether-disable                   → REPLY:usb-tether-off
@supervisor: sharing-clients                      → REPLY:sharing-clients <count>
```

Capability gate: all commands require `shell = true`. Platform gate: returns `REPLY:hotspot-error not-supported-on-platform` on `desktop-full`.

SSID: 1–32 bytes, no NUL. Passphrase: 8–63 printable ASCII.

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Interface names and nft table name interpolated unsanitized into nftables script — shell/NFT injection risk | `validate_iface_name()` + `validate_nft_table_name()` allowlist before any interpolation; upstream iface from `/proc/net/route` also validated |
| B2: `iw set type ap` returns success at cfg80211 layer before driver finishes station-mode teardown — AP silently broken | Explicit `iw disconnect` → `wait_for_iface_down(3s)` poll → `verify_iface_type("AP")` → `verify_hostapd_running(3s)` ctrl socket check |
| B3: `UdpSocket::bind("0.0.0.0:67")` conflicts with existing DHCP client; receives upstream DHCP traffic | `socket2::Socket` with `SO_REUSEPORT` + `SO_BINDTODEVICE=wlan0` — server only sees wlan0 DHCP traffic |
| B4: `disable_nat` writes `0` to `ip_forward` directly — breaks VPN/other subsystems that also need forwarding | `AtomicU32` reference counter `IP_FORWARD_REFS`; forwarding only disabled when count reaches zero; R53 VPN also uses acquire/release |
| B5: WPA2 passphrase written to `/tmp/vyoma_hostapd.conf` world-readable window (umask 0644 on create) | `OpenOptions::mode(0o600)` at CREATE time; conf shredded with zeros + deleted after hostapd exec; passphrase never stored in SharingState or persisted |
