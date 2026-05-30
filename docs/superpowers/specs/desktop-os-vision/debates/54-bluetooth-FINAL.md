# FINAL Spec: Bluetooth Stack (Round 54)

**Subsystem**: Bluetooth Stack  
**macOS Analogue**: `IOBluetooth` / `CoreBluetooth`  
**Depends on**: R03 (IPC), R51 (kernel base)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Kernel Config (B1 Fix)

```
# Bluetooth HCI + LE (R54)
CONFIG_BT=y
CONFIG_BT_BREDR=y
CONFIG_BT_LE=y
CONFIG_BT_HCIUART=y
CONFIG_BT_HCIUART_H4=y
# CONFIG_BT_HCIUSB is NOT included in R54 — it pulls in CONFIG_USB_SUPPORT
# which adds 200-400 KB. USB BT passthrough deferred to R54.1.
```

**B1 fix**: Drop `CONFIG_BT_HCIUSB` from initial scope to avoid hidden USB dependency chain. HCI UART passthrough covers QEMU development; USB BT added in R54.1 with explicit `CONFIG_USB=y` chain.

**QEMU support**: No `virtio-bluetooth-pci` device exists in any released QEMU (B5 fix: the spec was proposed in 2021 but never merged). Document only two working paths:
```makefile
# make run-bt-uart: HCI UART via virtio-serial
-device virtio-serial-pci -device virtserialport,chardev=bt0,name=hci0
# make run-bt-usb: USB BT adapter passthrough (requires host adapter)
-device usb-host,vendorid=0x0a5c,productid=0x21e8
```

---

## 2. Architecture

Supervisor-owned `AF_BLUETOOTH / BTPROTO_HCI` raw socket (apps cannot open `AF_BLUETOOTH` via WASI). Three dedicated threads:

```
bt_reader_thread   — reads HCI events; posts to BT_EVENT_TX mpsc
bt_dispatch_thread — pulls events; snapshot BT_SUBSCRIPTIONS under lock, drop, send_reply
bt_worker_thread   — executes bt-connect / bt-scan blocking ops from BT_CMD_TX channel
```

IPC handler returns immediately by sending to `BT_CMD_TX`; never blocks.

**Global statics:**
```rust
static BT_STATE:         OnceLock<Mutex<BtState>> = OnceLock::new();
static BT_EVENT_TX:      OnceLock<mpsc::Sender<BtEvent>> = OnceLock::new();
static BT_SUBSCRIPTIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static BT_CMD_TX:        OnceLock<mpsc::Sender<BtCmd>> = OnceLock::new();
```

When no HCI device found: `BtState::available = false`; all `bt-*` IPC returns `REPLY:bt-error:no-adapter`. Boot not blocked.

---

## 3. Protocol

App → supervisor (`@supervisor: bt-*`):
```
@supervisor: bt-enable
@supervisor: bt-disable
@supervisor: bt-status              → REPLY:bt-status:<enabled|disabled>:<adapter_addr>
@supervisor: bt-scan-start/<ms>     → async VYOMA_BT:device_found events
@supervisor: bt-scan-stop
@supervisor: bt-connect/<addr>
@supervisor: bt-disconnect/<handle>
@supervisor: bt-pair/<addr>
@supervisor: bt-pair-confirm/<addr>/<pin>
@supervisor: bt-peers               → REPLY:bt-peers:<addr1>|<addr2>|...
@supervisor: bt-gatt-read/<handle>/<char_uuid>
@supervisor: bt-gatt-write/<handle>/<char_uuid>/<hex_bytes>
@supervisor: bt-gatt-notify-on/<handle>/<char_uuid>/<min_ms>   # min_ms = rate limit (B3)
@supervisor: bt-gatt-notify-off/<handle>/<char_uuid>
@supervisor: bt-subscribe           # register for async VYOMA_BT: events
@supervisor: bt-unsubscribe
```

Supervisor → app stdin (async, only to subscribed apps):
```
VYOMA_BT:device_found:<addr>:<name>:<rssi>
VYOMA_BT:scan_done
VYOMA_BT:connected:<handle>:<addr>
VYOMA_BT:connect_failed:<addr>:<reason>
VYOMA_BT:disconnected:<handle>
VYOMA_BT:pair_pin:<addr>:<pin>
VYOMA_BT:paired:<addr>
VYOMA_BT:pair_failed:<addr>:<reason>
VYOMA_BT:gatt_notify:<handle>:<char_uuid>:<hex_bytes>
VYOMA_BT:error:<code>:<message>
```

Capability gate: `bluetooth: bool` in `Capabilities` struct. All `bt-*` commands rejected without it.

---

## 4. bt_reader_thread Shutdown (B2 Fix)

`read()` on HCI socket blocks indefinitely if adapter removed or QEMU suspends. Fix: `poll()` with 500ms timeout + `AtomicBool` shutdown flag:

```rust
// supervisor/src/bluetooth/hci.rs
pub fn bt_reader_thread(fd: RawFd, tx: mpsc::Sender<BtEvent>, shutdown: Arc<AtomicBool>) {
    let mut pollfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    loop {
        if shutdown.load(Ordering::Relaxed) { break; }
        let ret = unsafe { libc::poll(&mut pollfd, 1, 500) };
        if ret > 0 {
            let event = hci_read_event(fd);
            let _ = tx.send(event);
        }
    }
}
```

`bluetooth::shutdown()` sets the flag; thread exits within 500ms. Clean join possible (B2 fix).

---

## 5. GATT Notification Rate Limiting (B3 Fix)

100 Hz BLE characteristic → 100 unbounded mpsc messages/sec → OOM. Fix: per-subscription rate limiter + max queue depth tracking:

```rust
// supervisor/src/bluetooth/events.rs
struct GattSub { char_uuid: [u8;16], min_interval_ms: u64, last_sent_ms: u64 }
// In bt_dispatch_thread, before sending gatt_notify:
let now_ms = now_unix_ms();
if now_ms - sub.last_sent_ms >= sub.min_interval_ms {
    send_reply(app_name, &msg, inbox);
    sub.last_sent_ms = now_ms;
}
// else: drop this notification
```

`min_ms` parameter in `bt-gatt-notify-on/<handle>/<uuid>/<min_ms>`. Default 0 (no rate limit). Apps monitoring high-frequency sensors set it explicitly (e.g. `50` for 20 Hz max delivery).

---

## 6. Paired Device Persistence (B4 Fix)

`/data/.vyoma/bt-peers.toml` written atomically. Crash recovery when both `.toml` and `.tmp` exist:

```rust
// supervisor/src/bluetooth/pairing.rs
pub fn load_peers() -> Vec<BtPeer> {
    let toml_path = "/data/.vyoma/bt-peers.toml";
    let tmp_path  = "/data/.vyoma/bt-peers.toml.tmp";
    // B4 fix: both exist → use newer file
    let use_tmp = if Path::new(toml_path).exists() && Path::new(tmp_path).exists() {
        let tm = metadata(toml_path).and_then(|m| m.modified()).ok();
        let tt = metadata(tmp_path).and_then(|m| m.modified()).ok();
        tt > tm
    } else { Path::new(tmp_path).exists() };
    let path = if use_tmp { tmp_path } else { toml_path };
    toml::from_str(&fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
}
```

Write: write to `.tmp`, fsync, rename to `.toml`. The mtime comparison ensures recovery always picks the most recent file regardless of which rename succeeded.

---

## 7. File Layout

```
supervisor/src/bluetooth/
├── mod.rs       (~120 lines: BtState, statics, init(), handle_bt_command())
├── hci.rs       (~180 lines: HCI socket, poll loop (B2), LE scan/connect HCI commands)
├── gatt.rs      (~200 lines: ATT protocol, read/write/notify, per-connection handle table)
├── pairing.rs   (~150 lines: SSP state machine, pairing flow, bt-peers.toml (B4))
├── events.rs    (~80 lines: BtEvent enum, dispatch thread, rate limiter (B3))
└── ipc.rs       (~120 lines: command parser, capability check, forward to BT_CMD_TX)
```

---

## 8. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `CONFIG_BT_HCIUSB` pulls in 200-400 KB USB dependency chain | Drop HCIUSB from R54; use HCI UART only; USB BT in R54.1 with explicit USB chain |
| B2: `bt_reader_thread` blocks indefinitely on `read()` if adapter removed | `poll()` with 500ms timeout + `AtomicBool` shutdown flag; clean join on supervisor shutdown |
| B3: GATT 100 Hz characteristic floods app inbox → OOM | Per-subscription `min_interval_ms` rate limiter in dispatch thread; `bt-gatt-notify-on` takes min_ms param |
| B4: Both `.toml` and `.tmp` exist after crash — wrong file used for recovery | mtime comparison: always use newer file; drop stale one |
| B5: `virtio-bluetooth-pci` QEMU device doesn't exist — silent failure | Document only HCI UART + USB passthrough; remove virtio-bt from all docs; add CI device-check |
