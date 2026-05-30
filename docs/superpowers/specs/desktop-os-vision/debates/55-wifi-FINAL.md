# FINAL Spec: Wi-Fi Management (Round 55)

**Subsystem**: Wi-Fi Management  
**macOS Analogue**: `CoreWLAN` / `WiFiKit`  
**Depends on**: R51 (kernel net base), R52 (DNS resolver for wlan0)  
**Platform scope**: `iot-edge`, `mobile`, `robotics-rt` only — `desktop-full` uses virtio-net  
**Status**: FINAL — all blocking issues resolved

---

## 1. Kernel Config (Platform-Specific)

Added to `base/kernel-configs/iot-edge.config` and `mobile.config` only:

```
CONFIG_CFG80211=y
CONFIG_MAC80211=y
CONFIG_MAC80211_MESH=n
CONFIG_BRCMFMAC=y          # Raspberry Pi 3/4/5 onboard Wi-Fi
CONFIG_BRCMUTIL=y
CONFIG_ATH9K=y             # Qualcomm AR9xxx
CONFIG_ATH9K_USB=y
CONFIG_RTW88=y             # B2 fix: Realtek RTW88 replaces RTL8192CU
CONFIG_RTW88_USB=y
CONFIG_CRYPTO_AES=y
CONFIG_CRYPTO_SHA256=y
```

**B2 fix**: `CONFIG_RTL8192CU` dropped — poor `NL80211_CMD_CONNECT` support. Replaced with `CONFIG_RTW88_USB=y` (Realtek's in-tree driver with proper cfg80211). `ATH9K` requires runtime verification of `NL80211_CMD_CONNECT` path; connect timeout returns `connect_failed:driver_unsupported` if no response within 15s.

**QEMU fallback**: `wifi/detect.rs` checks for `/sys/class/net/*/phy80211` symlink. If absent, `WifiManager` enters `NoHardware` state. All IPC returns `VYOMA_WIFI:status:no-hardware`. Zero overhead on desktop-full.

---

## 2. nl80211 Crates (B1 Fix)

Inline nlattr encoding silently corrupts nested attributes (`NL80211_ATTR_BSS`, cipher suites) — kernel drops malformed messages with no error return, causing scans to hang forever. Fix: add three pure-Rust crates:

```toml
# supervisor/Cargo.toml (platform-gated)
[target.'cfg(feature = "wifi")'.dependencies]
netlink-sys           = "0.8"
netlink-packet-core   = "0.7"
netlink-packet-generic = "0.3"
```

Total ~40 KB compressed. No C dependencies. These provide correct nested nlattr serialization and nl80211 family ID resolution.

---

## 3. WPA2 — Kernel-Delegated 4-Way Handshake

`NL80211_CMD_CONNECT` with PMK injection — kernel mac80211/FullMAC runs EAPOL internally:

```rust
// supervisor/src/wifi/connect.rs  (~150 lines)
// 1. Derive PMK (B3: stored in memory only, never on disk)
let pmk = crypto::pbkdf2_hmac_sha1(passphrase, ssid, 4096, 32);

// 2. Issue NL80211_CMD_CONNECT
nl80211_send_connect(nl_fd, ifindex, ssid, bssid, &pmk)?;

// 3. Listen on multicast "mlme" group for NL80211_CMD_CONNECT result
// (in wifi worker thread — not IPC handler thread)
```

PMK derivation: pure-Rust PBKDF2-HMAC-SHA1 in `wifi/crypto.rs` (~120 lines, no new crates — SHA-1 ~60 lines, HMAC/PBKDF2 ~60 lines).

---

## 4. Async Scan — Dedicated Worker Thread (B4 Fix)

nl80211 scan takes 2–5 seconds on real hardware. Must NOT block IPC handler thread:

```rust
// IPC handler:  immediate reply + return
"wifi-scan" => {
    send_reply(sender, "VYOMA_WIFI:scan_start", inbox);
    let _ = WIFI_CMD_TX.get().unwrap().send(WifiCmd::Scan { requester: sender.to_string() });
}

// wifi_worker_thread: runs NL80211_CMD_TRIGGER_SCAN → poll → GET_SCAN dump
// delivers VYOMA_WIFI:ap_found:... lines then VYOMA_WIFI:scan_done:<count>
```

`WIFI_CMD_TX: OnceLock<mpsc::Sender<WifiCmd>>` — one persistent worker thread spawned at init on Wi-Fi platforms. Snapshot `Inbox` Arc at dispatch; never holds AppRegistry during send.

---

## 5. Post-Association DHCP — Userspace Client (B5 Fix)

Kernel `ip=dhcp` only runs at boot on the boot interface. After `NL80211_CMD_CONNECT` success, wlan0 needs a userspace DHCP flow:

```rust
// supervisor/src/wifi/dhcp.rs  (~200 lines)
pub fn dhcp_acquire(iface: &str) -> Result<Dhcp4Config, String> {
    // 1. Poll /sys/class/net/<iface>/operstate until "up" (max 3s, 50ms intervals)
    wait_iface_up(iface, Duration::from_secs(3))?;
    // 2. DISCOVER → OFFER → REQUEST → ACK via AF_PACKET raw socket
    let sock = socket(AF_PACKET, SOCK_DGRAM, ETH_P_IP)?;
    let ack = dhcp_exchange(sock, iface)?;
    // 3. Configure interface: ioctl SIOCSIFADDR + SIOCSIFNETMASK + SIOCADDRT
    configure_iface(iface, &ack)?;
    Ok(ack.into())
}
```

On DHCP timeout (10s): emit `VYOMA_WIFI:connect_failed:<ssid>:dhcp_failed`. `dhcp_timeout_secs` configurable in platform profile Wi-Fi section.

---

## 6. PMK Security (B3 Fix)

PMK is cryptographically equivalent to passphrase for network impersonation. Never persisted to disk until R60 Keychain:

```rust
// supervisor/src/wifi/profiles.rs
pub struct WifiProfile {
    pub ssid:         String,
    pub security:     WifiSecurity,
    pub auto_connect: bool,
    pub priority:     i32,
    // keychain_ref replaces pmk_hex when R60 ships
    pub keychain_ref: Option<String>,  // "wifi/<ssid>"
    // Pre-R60 fallback: passphrase stored only in memory, never serialized
    #[serde(skip)]
    pub pmk_mem: Option<[u8; 32]>,
}
```

`wifi-add-profile <ssid> <passphrase>` command: supervisor derives PMK in memory, stores only in `WifiManager.pmk_cache: HashMap<String, [u8;32]>`. Profile TOML contains no secret material. On supervisor restart, app must re-supply passphrase. Platform profile sets `wifi.require_keychain = true` (mobile) to block `wifi-add-profile` until R60 Keychain available.

---

## 7. Protocol

```
@supervisor: wifi-scan              → VYOMA_WIFI:scan_start (async ap_found events)
@supervisor: wifi-connect <ssid>    → VYOMA_WIFI:connecting:<ssid> then connected/failed
@supervisor: wifi-disconnect        → VYOMA_WIFI:disconnected:<ssid>
@supervisor: wifi-status            → VYOMA_WIFI:status:<state>
@supervisor: wifi-add-profile <ssid> <passphrase>
@supervisor: wifi-forget <ssid>

VYOMA_WIFI:ap_found:<ssid>:<bssid>:<rssi_dbm>:<open|wpa2-psk|wpa3-sae>
VYOMA_WIFI:scan_done:<count>
VYOMA_WIFI:connected:<ssid>:<bssid>:<ip4>
VYOMA_WIFI:connect_failed:<ssid>:<wrong_psk|no_ap|dhcp_failed|driver_unsupported>
VYOMA_WIFI:disconnected:<ssid>
VYOMA_WIFI:status:no-hardware
VYOMA_WIFI:status:connected:<ssid>:<ip4>:<rssi_dbm>
```

Capability gate: `wifi: bool` in `Capabilities`. Desktop-full apps receive `VYOMA_WIFI:status:no-hardware`.

---

## 8. File Layout

```
supervisor/src/wifi/
├── mod.rs       (~150 lines: WifiManager, WifiCmd, statics, init(), handle_wifi_cmd())
├── nl80211.rs   (~200 lines: netlink socket, family ID resolution, send/recv helpers)
├── scan.rs      (~180 lines: TRIGGER_SCAN + GET_SCAN dump + BSS IE parser)
├── connect.rs   (~150 lines: NL80211_CMD_CONNECT + result listener + async worker)
├── crypto.rs    (~120 lines: PBKDF2-HMAC-SHA1 PMK derivation, pure Rust)
├── profiles.rs  (~130 lines: load/save wifi-profiles.toml, PMK memory cache (B3))
├── detect.rs    (~40 lines: /sys/class/net hardware detection)
└── dhcp.rs      (~200 lines: AF_PACKET DHCP client (B5), wait_iface_up)
```

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Inline nlattr encoding corrupts nested attributes, kernel silently drops messages | Add `netlink-sys`, `netlink-packet-core`, `netlink-packet-generic` (pure Rust, ~40 KB) |
| B2: RTL8192CU has poor NL80211_CMD_CONNECT support; ath9k needs runtime test | Replace RTL8192CU with RTW88; ath9k times out gracefully with `driver_unsupported` |
| B3: PMK storage in world-readable TOML is a security regression | PMK stored in-memory only (`WifiManager.pmk_cache`); disk only via R60 Keychain |
| B4: nl80211 scan blocks IPC thread 2–5 seconds | Dedicated `wifi_worker_thread`; IPC handler sends to `WIFI_CMD_TX` and returns immediately |
| B5: No userspace DHCP client; kernel `ip=dhcp` only runs at boot on boot interface | `wifi/dhcp.rs`: AF_PACKET DHCP exchange; wait_iface_up before DISCOVER; ioctl IP config |
