# Round 6 Final: Device Driver Model

**Status:** ✅ Debated & synthesized (2026-05-29)
**Debate:** [Architect: 2579 lines] [Critic: 510 lines] [Final: this]
**Subsystem:** Device discovery, in-supervisor subsystems, capability handles, WASM driver bundles (narrow case), runtime privacy, sensor APIs
**macOS equivalents resolved:** IOKit (kernel-side, but here: Linux kernel) + DriverKit (narrow WASM bundles, *not* HID) + HID Manager (in-supervisor Rust) + CoreAudio HAL (lock-free mixer) + AVCaptureSession (consent-gated) + DiskArbitration (USB user prompt)
**Integrates with:**
  - **R1** — `AppIdentity`, `AppHandle`, `BundleId`, `BootPhase`, `RestartPolicy`, WIT lifecycle callbacks, ed25519 code signing trust chain
  - **R2** — `Arc<AppLimiter>`, `PSI` pressure, `SharedBuffer` (memfd + epoch-tracked mmap) for camera/audio zero-copy
  - **R3** — `IpcEnvelope`, sharded `IpcRouter`, `ArcSwap<PolicySnapshot>`, broadcast topics, publisher-restricted topics
  - **R4** — `VfsBackend` (USB storage gated path), preopened-FD sandbox, no raw `/dev` to apps
  - **R5** — `QosClass`, `ActivityAssertion`, cgroup `cpu.max`, `SCHED_DEADLINE` audio render thread, focus-driven QoS

---

## 0. Executive Summary

The Architect's original proposal tried to unify three radically-different driver concerns — kernel-side arbitration, in-supervisor subsystem policy, and per-app capability negotiation — under one "WASM driver bundle" abstraction. The Critic correctly identified that this collapses for interrupt-driven HID (sub-50µs needed, WASM round-trip is 200µs–10ms), for multi-subscriber audio/camera (exclusive ownership is wrong), for runtime privacy (install-time grants are unsafe for camera/mic), and for USB storage (auto-mount is BadUSB). The Critic also flagged the absent udev infrastructure, the lock-contention hazard on the SCHED_DEADLINE thread, the display hot-plug race, and the unspecified driver-bundle signing chain.

This Final spec rebuilds the model around the Critic's inversion: **VyomaOS does not write drivers — the Linux 5.10+ kernel does. VyomaOS arbitrates *access* to driver outputs via in-supervisor subsystems, exposes them to WASM apps through WIT-typed handles, and reserves WASM driver bundles strictly for non-realtime vendor-specific protocol decoding (Stream Decks, Wacom pen curves, MIDI exotica).** All Critic blockers are resolved; the Architect's strong points (centralized arbitration, WIT contracts, SharedBuffer, declared capabilities, NETLINK_KOBJECT_UEVENT) are preserved and tightened.

The final shape:

1. **Three-tier driver architecture** (C1 fixed). Input/display/audio/camera live in pure-Rust supervisor subsystems with no WASM in the hot path. WASM driver bundles exist only for non-realtime vendor protocols and require ed25519 signing. Linux kernel + thin WIT shim covers everything else.
2. **Direct NETLINK_KOBJECT_UEVENT socket** (C2 fixed) plus an exhaustive `/sys` coldplug walk at boot. No systemd-udevd, no libudev, no second process.
3. **Zero-copy audio mixer on SCHED_DEADLINE** (C3 fixed). Per-app SPSC ring buffers in SharedBuffer-backed memory; lock-free atomic volume/mute; epoch-RCU reclamation for unmapped buffers; no Mutex on the hot path.
4. **Three-tier device access model** (C4 fixed): `Exclusive`, `Shared`, `Multiplexed`. Audio defaults `Multiplexed`. Camera is `Exclusive` or `Shared` with consent. Input is `Multiplexed` through the InputDispatcher.
5. **Compositor pause/resume API** (C5 fixed). Display hot-plug acquires an epoch lock, drains the in-flight DRM atomic commit at vblank, reconfigures, then releases. Fan-out display-change notifications are batched per Round 3.
6. **Two-tier capability model** (C6 fixed). Manifest declares *intent* (`camera = "ask"`). First runtime use triggers a TCC-style blocking system prompt. Grants persist to Keychain (Round 60 forward-reference); revocation flips `ArcSwap<DevicePolicy>` instantly.
7. **USB storage DeviceOffer flow** (C7 fixed). USB block devices are *never* auto-mounted. The supervisor surfaces a `DeviceOffer` to the user via System UI; filesystem magic bytes are scanned for known signatures before any mount call; default mount is read-only into the file-manager-only namespace.
8. **ed25519-signed WASM driver bundles** (C8 fixed). Bundles must chain to the VyomaOS trusted root (same chain as Round 1 code signing). Bundle manifest declares `device_match` and `capabilities`; supervisor validates these are within the device-class envelope. Three crashes in 60s → bundle disabled until user re-enables in System Preferences.

Total new code: ~5,200 LOC across 14 Rust files + 540 lines WIT. Every file ≤ 500 lines. Hot-path overhead: ~6 µs p50 / 25 µs p99 for evdev → focused-app input (identical to R5 dispatch budget); audio render fits in 1.8 ms of the 2 ms SCHED_DEADLINE runtime; udev event → app callback ≤ 200 µs p99 after coalesce.

---

## Key Decisions

1. **No WASM in the interrupt-driven hot path.** Input devices (keyboard, mouse, touchpad, touchscreen, stylus, controller) are owned end-to-end by the supervisor's `HidSubsystem` in native Rust. The "WASM driver bundle" abstraction is reserved for vendor-specific protocol decoding on non-realtime devices.

2. **The kernel is the driver.** `CONFIG_MODULES=n` on `desktop-full`. Every device class VyomaOS recognises must have a kernel driver compiled-in at image build time. Unsupported devices yield a `DeviceUnsupported` toast, not on-demand `modprobe`.

3. **Direct NETLINK_KOBJECT_UEVENT socket** owned by the supervisor. No systemd-udevd, no libudev linkage. Pure-Rust netlink parser (~410 LOC).

4. **Coldplug via `/sys` walk at boot.** `enumerate_existing()` walks `/sys/class/input`, `/sys/class/sound`, `/sys/class/drm`, `/sys/bus/usb/devices`, `/sys/class/power_supply`, `/sys/class/thermal`, `/sys/bus/iio/devices`. Synthesises `UEvent::Add`-equivalents into the registry before starting the netlink reader thread.

5. **Three device access tiers**: `Exclusive` (one app/driver), `Shared` (multiple readers see the same stream), `Multiplexed` (supervisor mixes/routes). Audio output defaults `Multiplexed`; camera defaults `Exclusive` (but with `Shared` opt-in via privileged broadcast capability); input is always `Multiplexed`; USB raw is `Exclusive` to a driver bundle.

6. **Lock-free audio render path.** Per-app `SharedBuffer` ring is an SPSC queue read by the SCHED_DEADLINE thread without locks. Per-app volume is `AtomicU32` (Q16.16 fixed-point). Per-app mute is `AtomicBool`. Output-device pointer is published via `ArcSwap<AudioOutputBinding>` with epoch-RCU reclamation when an app's SharedBuffer is unmapped.

7. **Compositor pause/resume API.** `DisplaySubsystem::on_drm_hotplug` calls `Compositor::pause_rendering(target_crtc)` → waits one vblank for in-flight DMA → `Compositor::reconfigure(new_topology)` → `Compositor::resume_rendering()`. No DRM atomic commit issued while a connector is in transition.

8. **Two-tier capability model.** `[capabilities.devices]` declares *intent* (`AccessLevel::None | Focused | Allow | Ask`). For `Ask`-tier classes (camera, microphone, location, screen-recording), the first runtime call to the WIT method blocks until a user prompt is resolved. Grants persist in Round 60 Keychain; the active grant is also reflected in `ArcSwap<DevicePolicy>` for O(1) IPC-time decisions.

9. **No USB auto-mount.** USB mass-storage uevent → `DeviceOffer { device, fs_type_guess, label }` IPC broadcast to System UI → user prompt → user confirms → supervisor mounts read-only into a namespace visible only to the file-manager app and to apps declaring `[capabilities.filesystem] removable_storage = true`. Filesystem magic bytes are scanned via `blkid`-equivalent in a sandboxed subprocess before mount.

10. **ed25519-signed driver bundles.** Bundle artifact: `<name>.wasm + <name>.vyoma-driver.toml + <name>.sig`. Signature chains to a built-in VyomaOS trust root, identical to R1 §10. Unsigned bundles refuse to load. Bundle manifest `[driver]` section declares `matches` (device specifiers) and `capabilities` (subset of `usb_raw`, `hid_raw`, `midi_raw`); supervisor verifies the requested capabilities are within the device-class envelope at install time AND at every spawn.

11. **Three-strike crash tracker.** A driver bundle that crashes ≥3 times within 60s is marked `Disabled`; the device shows in System Preferences as "Driver disabled — click to re-enable." Crash window is per-`(bundle, device)`.

12. **Hot-plug semantics: graceful, surprise, replug**. Graceful (user eject): 2s app-handle release window, then force-revoke + unbind. Surprise (cable yanked): immediate `DeviceLost` to all handles, 250 ms driver-bundle drain, then store-drop. Replug within 5 s of same `(vid, pid, serial)`: device id is reused, apps see `Changed` with `state: Available`.

13. **Sensors gated by per-class consent and rate floors.** Default 50 ms floor on subscription interval. `high_rate = true` capability (motion-aware games, fitness apps) lifts the floor to 1 ms; that capability requires runtime grant and persistence in Keychain.

14. **Single WIT package `vyoma:device@0.1.0`** with eight interfaces: `types`, `registry`, `events`, `input`, `audio`, `usb`, `sensor`, `device-lifecycle`. Two worlds: `app` (all imports except `usb`) and `driver-bundle` (full set, exports `device-lifecycle`).

15. **Boot order:** `KernelHandoff → FsReady → IpcReady → DevicesReady → PolicyLoaded → AppsLaunching → AppsLaunched`. DevicesReady fires after coldplug + subsystem init (~150 ms budget). Driver bundles spawn between PolicyLoaded and AppsLaunching so input drivers are alive when user apps need keyboard.

---

## 1. Three-Tier Driver Architecture — Fixed

### 1.1 The fix (resolves C1)

The Architect's original "WASM driver bundle does everything" is replaced with a three-tier model that maps each class of driver work to the abstraction that actually fits:

```
┌──────────────────────────────────────────────────────────────────────┐
│ Tier A: In-supervisor subsystem (native Rust)                        │
│         supervisor/src/drivers/{hid,display_dev,audio_dev,camera,    │
│                                  usb_storage_gate,sensor}/           │
│         Purpose: latency-critical, multi-subscriber, system-wide.    │
│         Latency: <50 µs interrupt → app callback                     │
│         Examples: keyboard, mouse, touchpad, stylus, display,        │
│                   audio mixer, camera capture, sensor sampling,      │
│                   USB-storage offer flow                              │
├──────────────────────────────────────────────────────────────────────┤
│ Tier B: Signed WASM driver bundle (wasm32-wasip2)                    │
│         apps/<vendor>-driver/ with [driver] section, ed25519-signed  │
│         Purpose: non-realtime vendor-specific protocol decode.       │
│         Latency: 50 µs–5 ms acceptable (button-press, knob-turn).    │
│         Examples: Stream Deck, MIDI controller with weird sysex,     │
│                   Wacom proprietary pressure curve, fingerprint dev. │
├──────────────────────────────────────────────────────────────────────┤
│ Tier C: Kernel driver + thin WIT shim                                │
│         Pure WIT exposure of an existing /dev node behind capability │
│         Purpose: device the kernel fully handles; we expose it.      │
│         Examples: HDMI audio output, USB-Audio class, USB-HID class, │
│                   thermal sensors via sysfs, ALSA mixer controls.    │
└──────────────────────────────────────────────────────────────────────┘
```

The rule is hard: **no WASM driver bundle in the path of any interrupt-driven HID event.** The supervisor reads `/dev/input/event*` directly via epoll, parses it in native Rust, hands the parsed `ParsedInput` to the `InputDispatcher`, and the dispatcher routes to the focused app's `on-input` WIT callback. End-to-end latency target: 6 µs p50 / 25 µs p99 (same as Round 5).

A WASM driver bundle's role is exclusively non-realtime: a Stream Deck (15 buttons + 2 dials) reporting button-press events at human-finger frequency (≤200 Hz aggregate); a Wacom tablet's proprietary HID profile that the kernel exposes as `/dev/hidraw*` but doesn't parse fully; a fingerprint reader's enrollment protocol. The bundle reads `/dev/hidraw*` via a supervisor-mediated handle, parses, and publishes events on its declared `event_topic`.

### 1.2 Tier-A: in-supervisor subsystems

Each Tier-A subsystem is a Rust module under `supervisor/src/drivers/` with a `start()` constructor invoked by `DeviceManager::boot()`. The pattern:

```rust
// supervisor/src/drivers/mod.rs

use std::sync::Arc;
use arc_swap::ArcSwap;
use crate::ipc::IpcRouter;
use crate::limits::AppLimiter;
use crate::lifecycle::BootPhase;

pub struct DeviceManager {
    pub registry: Arc<DeviceRegistry>,
    pub udev_reader: Arc<udev::UdevReader>,
    pub claims: Arc<ClaimTable>,
    pub policy: ArcSwap<policy::DevicePolicy>,
    pub ipc_router: Arc<IpcRouter>,
    pub limiter: Arc<AppLimiter>,
    pub bundles: Arc<bundle::BundleRegistry>,
    // Tier-A subsystems
    pub hid: Arc<hid::HidSubsystem>,
    pub display: Arc<display_dev::DisplaySubsystem>,
    pub audio: Arc<audio_dev::AudioSubsystem>,
    pub camera: Arc<camera::CameraSubsystem>,
    pub usb_gate: Arc<usb_storage_gate::UsbStorageGate>,
    pub sensor: Arc<sensor::SensorSubsystem>,
}

impl DeviceManager {
    pub fn boot(
        boot_phase: BootPhase,
        ipc_router: Arc<IpcRouter>,
        limiter: Arc<AppLimiter>,
    ) -> Result<Arc<Self>, DeviceError> {
        debug_assert!(boot_phase >= BootPhase::IpcReady);
        let registry = Arc::new(DeviceRegistry::new());
        let claims = Arc::new(ClaimTable::new());

        // Tier-A subsystems come up before the udev reader thread so they
        // are ready to absorb the initial coldplug burst.
        let hid     = hid::HidSubsystem::start(registry.clone(), ipc_router.clone())?;
        let display = display_dev::DisplaySubsystem::start(registry.clone())?;
        let audio   = audio_dev::AudioSubsystem::start(registry.clone(), limiter.clone())?;
        let camera  = camera::CameraSubsystem::start(registry.clone(), ipc_router.clone())?;
        let usb_gate = usb_storage_gate::UsbStorageGate::start(
            registry.clone(), ipc_router.clone())?;
        let sensor  = sensor::SensorSubsystem::start(registry.clone(), ipc_router.clone())?;

        let bundles = Arc::new(bundle::BundleRegistry::scan_installed()?);
        let policy  = ArcSwap::new(Arc::new(policy::DevicePolicy::load_from_disk()?));

        let mgr = Arc::new(Self {
            registry: registry.clone(),
            udev_reader: udev::UdevReader::open()?,
            claims, policy,
            ipc_router: ipc_router.clone(),
            limiter,
            bundles,
            hid, display, audio, camera, usb_gate, sensor,
        });

        mgr.coldplug_walk()?;        // /sys walk synthesising Add events
        mgr.spawn_udev_thread();     // NETLINK reader runs from here on
        Ok(mgr)
    }

    fn spawn_udev_thread(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::Builder::new()
            .name("vyoma-udev".into())
            .spawn(move || me.udev_reader.run(me.clone()))
            .expect("vyoma-udev thread");
    }
}
```

### 1.3 Tier-B: signed WASM driver bundles

A driver bundle is an ordinary `wasm32-wasip2` binary distinguished by:
- A `[driver]` section in `vyoma.toml` (validated by R1 manifest parser)
- An adjacent `<name>.sig` ed25519 signature file
- Strictly capped capabilities (no filesystem, no network, no display, no shell)
- A single `DeviceHandle<T>` pre-installed by the supervisor at spawn

See §8 for full bundle security spec.

### 1.4 Tier-C: kernel + WIT shim

For devices where the kernel already fully handles the protocol (USB-HID class keyboards, USB-Audio class DACs, USB Mass Storage), there is *no* driver bundle. The supervisor publishes a `DeviceInfo` reflecting kernel state; the Tier-A subsystem opens the kernel-side node and proxies to the WIT interface. The user-facing capability flow is identical.

---

## 2. Device Discovery — uevent Netlink Fixed

### 2.1 The fix (resolves C2)

The Architect's spec correctly used a direct NETLINK_KOBJECT_UEVENT socket. This Final spec ratifies that choice and adds the missing coldplug component explicitly.

The supervisor:
1. Opens `socket(AF_NETLINK, SOCK_DGRAM | SOCK_CLOEXEC | SOCK_NONBLOCK, NETLINK_KOBJECT_UEVENT)`
2. Binds to group 1 (kernel uevent multicast group)
3. Walks `/sys` synthesising `Add` events for already-present devices ("coldplug")
4. Starts the netlink reader thread; only then are deltas processed
5. NEVER spawns a second process — no systemd-udevd, no eudev, no libudev linkage

### 2.2 `UdevReader` and `UEvent::parse`

```rust
// supervisor/src/drivers/udev.rs

use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

const NETLINK_KOBJECT_UEVENT: i32 = 15;
const UDEV_MONITOR_GROUP_KERNEL: u32 = 1;

pub struct UdevReader {
    fd: RawFd,
    coalesce_window: Duration,
    input_class_bypass: bool,   // input devs bypass coalesce (see C2 Q2)
}

impl UdevReader {
    pub fn open() -> std::io::Result<Arc<Self>> {
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                NETLINK_KOBJECT_UEVENT,
            )
        };
        if fd < 0 { return Err(std::io::Error::last_os_error()); }

        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_pid = 0;
        addr.nl_groups = UDEV_MONITOR_GROUP_KERNEL;

        let rc = unsafe {
            libc::bind(fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as u32)
        };
        if rc < 0 {
            unsafe { libc::close(fd); }
            return Err(std::io::Error::last_os_error());
        }

        // 1 MB receive buffer (Bluetooth scans can flood).
        let bufsize: libc::c_int = 1 << 20;
        unsafe {
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                &bufsize as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as u32);
        }

        Ok(Arc::new(Self {
            fd,
            coalesce_window: Duration::from_millis(100),
            input_class_bypass: true,
        }))
    }

    pub fn run(self: Arc<Self>, mgr: Arc<DeviceManager>) {
        let mut buf = vec![0u8; 65536];
        let mut pending: Vec<UEvent> = Vec::with_capacity(64);
        let mut last_flush = Instant::now();
        loop {
            let mut pfd = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            let timeout_ms = self.coalesce_window
                .saturating_sub(last_flush.elapsed())
                .as_millis() as i32;
            let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms.max(0)) };
            if rc > 0 && (pfd.revents & libc::POLLIN) != 0 {
                let n = unsafe {
                    libc::recv(self.fd, buf.as_mut_ptr() as *mut _, buf.len(), 0)
                };
                if n > 0 {
                    if let Some(ev) = UEvent::parse(&buf[..n as usize]) {
                        // Resolves Critic Q2: input bypasses the coalesce window
                        // so a freshly-plugged USB keyboard is usable in ~30 ms,
                        // not 100+ ms.
                        if self.input_class_bypass && ev.subsystem == "input" {
                            mgr.apply_uevents(vec![ev]);
                        } else {
                            pending.push(ev);
                        }
                    }
                }
            }
            if last_flush.elapsed() >= self.coalesce_window && !pending.is_empty() {
                mgr.apply_uevents(std::mem::take(&mut pending));
                last_flush = Instant::now();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct UEvent {
    pub action: UAction,
    pub subsystem: String,
    pub devpath: String,
    pub devnode: Option<String>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UAction { Add, Remove, Change, Online, Offline, Bind, Unbind }

impl UEvent {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let mut iter = bytes.split(|&b| b == 0).filter(|s| !s.is_empty());
        let first = std::str::from_utf8(iter.next()?).ok()?;
        let mut split = first.splitn(2, '@');
        let action_str = split.next()?;
        let devpath = split.next()?.to_string();

        let mut props = HashMap::new();
        for kv in iter {
            let s = std::str::from_utf8(kv).ok()?;
            if let Some(eq) = s.find('=') {
                props.insert(s[..eq].to_string(), s[eq+1..].to_string());
            }
        }

        let action = match action_str {
            "add" => UAction::Add,
            "remove" => UAction::Remove,
            "change" => UAction::Change,
            "online" => UAction::Online,
            "offline" => UAction::Offline,
            "bind" => UAction::Bind,
            "unbind" => UAction::Unbind,
            _ => return None,
        };
        let subsystem = props.get("SUBSYSTEM").cloned().unwrap_or_default();
        let devnode = props.get("DEVNAME").map(|n| format!("/dev/{n}"));
        let vendor_id = props.get("ID_VENDOR_ID")
            .and_then(|s| u16::from_str_radix(s, 16).ok());
        let product_id = props.get("ID_MODEL_ID")
            .and_then(|s| u16::from_str_radix(s, 16).ok());
        Some(Self { action, subsystem, devpath, devnode,
                    vendor_id, product_id, properties: props })
    }
}
```

### 2.3 Coldplug walk (`/sys` enumeration)

```rust
// supervisor/src/drivers/udev.rs (continued)

impl DeviceManager {
    pub fn coldplug_walk(&self) -> Result<(), DeviceError> {
        // Visit /sys/bus/*/devices/* to enumerate the bind state per subsystem.
        // This is the supervisor-side replacement for `udevadm trigger`.
        let buses = ["usb", "pci", "i2c", "spi", "platform", "virtual"];
        for bus in buses {
            let path = format!("/sys/bus/{bus}/devices");
            let Ok(dir) = std::fs::read_dir(&path) else { continue };
            for entry in dir.flatten() {
                self.coldplug_one(entry.path());
            }
        }
        // /sys/class/* gives us the class-view (input, sound, drm, ...)
        let classes = ["input", "sound", "drm", "thermal",
                       "power_supply", "video4linux", "hidraw", "block"];
        for class in classes {
            let path = format!("/sys/class/{class}");
            let Ok(dir) = std::fs::read_dir(&path) else { continue };
            for entry in dir.flatten() {
                self.coldplug_one(entry.path());
            }
        }
        // /sys/bus/iio/devices/* for IIO sensors
        if let Ok(dir) = std::fs::read_dir("/sys/bus/iio/devices") {
            for entry in dir.flatten() { self.coldplug_one(entry.path()); }
        }
        Ok(())
    }

    fn coldplug_one(&self, sysfs_path: std::path::PathBuf) {
        let Ok(real) = std::fs::canonicalize(&sysfs_path) else { return };
        // Synthesise a fake UEvent::Add with whatever properties we can scrape.
        let mut props = HashMap::new();
        if let Ok(subsys) = std::fs::read_to_string(real.join("subsystem")) {
            props.insert("SUBSYSTEM".to_string(), subsys.trim().to_string());
        }
        if let Ok(uev_data) = std::fs::read_to_string(real.join("uevent")) {
            for line in uev_data.lines() {
                if let Some(eq) = line.find('=') {
                    props.insert(line[..eq].to_string(), line[eq+1..].to_string());
                }
            }
        }
        let synthetic = UEvent {
            action: UAction::Add,
            subsystem: props.get("SUBSYSTEM").cloned().unwrap_or_default(),
            devpath: real.to_string_lossy().into(),
            devnode: props.get("DEVNAME").map(|n| format!("/dev/{n}")),
            vendor_id: props.get("ID_VENDOR_ID")
                .and_then(|s| u16::from_str_radix(s, 16).ok()),
            product_id: props.get("ID_MODEL_ID")
                .and_then(|s| u16::from_str_radix(s, 16).ok()),
            properties: props,
        };
        self.apply_uevents(vec![synthetic]);
    }
}
```

### 2.4 `DeviceRegistry` with `ArcSwap<RegistrySnapshot>`

```rust
// supervisor/src/drivers/registry.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdScope { Permanent, Session, Ephemeral }

impl DeviceId {
    pub fn scope(&self) -> IdScope {
        match self.0 >> 60 {
            0 => IdScope::Permanent,
            1 => IdScope::Session,
            _ => IdScope::Ephemeral,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub class: DeviceClass,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub vendor_name: Option<String>,
    pub product_name: Option<String>,
    pub serial: Option<String>,
    pub bus_path: String,
    pub sysfs_path: String,
    pub dev_nodes: Vec<DevNode>,
    pub capabilities: DeviceCaps,
    pub state: DeviceState,
    pub access_tier: AccessTier,
    pub power: Option<PowerInfo>,
    pub claimed_by: Option<ClaimHolder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessTier {
    Exclusive,     // one claimer at a time
    Shared,        // multiple readers, supervisor fans out
    Multiplexed,   // supervisor mixes/composites (audio, display)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceState {
    Discovered, Initialising, Available, Claimed, InUse,
    Disabled, Lost, NeedsConsent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DeviceClass {
    // Input
    Keyboard = 0x01, Mouse = 0x02, Touchpad = 0x03,
    Touchscreen = 0x04, Stylus = 0x05, GameController = 0x06,
    // Display
    Display = 0x10, Projector = 0x11,
    // Audio
    AudioOutput = 0x20, AudioInput = 0x21, Midi = 0x22,
    // Imaging
    Camera = 0x30, Scanner = 0x31, Printer = 0x32,
    // Storage
    UsbStorage = 0x40, SdCard = 0x41, OpticalDrive = 0x42,
    // Connectivity
    UsbHub = 0x50, BluetoothAdapter = 0x51,
    NetworkAdapter = 0x52, Modem = 0x53,
    // Sensors
    Accelerometer = 0x60, Gyroscope = 0x61, Magnetometer = 0x62,
    AmbientLight = 0x63, Proximity = 0x64, Temperature = 0x65,
    Gps = 0x66, Barometer = 0x67,
    // Catchall — Critic Q4: bundles MAY NOT match Generic
    Generic = 0xff,
}

impl DeviceClass {
    pub fn default_access_tier(self) -> AccessTier {
        use DeviceClass::*;
        match self {
            Keyboard | Mouse | Touchpad | Touchscreen | Stylus
            | GameController => AccessTier::Multiplexed,  // via InputDispatcher
            Display | Projector => AccessTier::Multiplexed,
            AudioOutput => AccessTier::Multiplexed,
            AudioInput => AccessTier::Multiplexed,
            Camera => AccessTier::Exclusive,
            UsbHub | UsbStorage | SdCard | OpticalDrive => AccessTier::Exclusive,
            Accelerometer | Gyroscope | Magnetometer | AmbientLight
            | Proximity | Temperature | Gps | Barometer => AccessTier::Shared,
            BluetoothAdapter | NetworkAdapter | Modem => AccessTier::Multiplexed,
            Printer | Scanner => AccessTier::Exclusive,
            Midi => AccessTier::Shared,
            Generic => AccessTier::Exclusive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceCaps(pub u32);

impl DeviceCaps {
    pub const SUPPORTS_HOTPLUG:      Self = Self(1 << 0);
    pub const SUPPORTS_WAKEUP:       Self = Self(1 << 1);
    pub const SUPPORTS_AUTOSUSPEND:  Self = Self(1 << 2);
    pub const REQUIRES_USER_CONSENT: Self = Self(1 << 3);
    pub const HAS_PRESS_INDICATOR:   Self = Self(1 << 7);  // recording LED
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevNode {
    pub path: std::path::PathBuf,
    pub kind: DevNodeKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DevNodeKind {
    Evdev, Hidraw, AlsaPcm, AlsaCtl, DrmCard, DrmRender,
    UsbDevice, V4l2, Sysfs, Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerInfo {
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub wakeup_capable: bool,
    pub autosuspend_enabled: bool,
    pub current_draw_ma: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClaimHolder {
    App { bundle_id: crate::identity::BundleId, instance_id: u64 },
    Driver { bundle_id: crate::identity::BundleId, instance_id: u64 },
    Supervisor { subsystem: &'static str },
}

pub struct DeviceRegistry {
    pub snapshot: ArcSwap<RegistrySnapshot>,
    pub shards: [DashMap<DeviceId, Arc<DeviceInfo>>; 16],
    pub session_counter: AtomicU64,
    pub rebuild: std::sync::Mutex<()>,
}

#[derive(Debug, Clone, Default)]
pub struct RegistrySnapshot {
    pub by_id: im::HashMap<DeviceId, Arc<DeviceInfo>>,
    pub by_class: im::HashMap<DeviceClass, im::Vector<DeviceId>>,
    pub by_sysfs: im::HashMap<String, DeviceId>,
    pub revision: u64,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::new(Arc::new(RegistrySnapshot::default())),
            shards: std::array::from_fn(|_| DashMap::new()),
            session_counter: AtomicU64::new(0),
            rebuild: std::sync::Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> arc_swap::Guard<Arc<RegistrySnapshot>> {
        self.snapshot.load()
    }

    pub fn upsert(&self, info: DeviceInfo) {
        let id = info.id;
        self.shards[Self::shard_idx(id)].insert(id, Arc::new(info));
        self.rebuild_snapshot();
    }

    pub fn remove(&self, id: DeviceId) -> Option<Arc<DeviceInfo>> {
        let removed = self.shards[Self::shard_idx(id)].remove(&id).map(|(_, v)| v);
        if removed.is_some() { self.rebuild_snapshot(); }
        removed
    }

    fn shard_idx(id: DeviceId) -> usize { (id.0 as usize) & 0x0f }

    fn rebuild_snapshot(&self) {
        let _g = self.rebuild.lock().unwrap();
        let mut by_id = im::HashMap::new();
        let mut by_class: im::HashMap<DeviceClass, im::Vector<DeviceId>> = im::HashMap::new();
        let mut by_sysfs = im::HashMap::new();
        for shard in &self.shards {
            for entry in shard.iter() {
                let id = *entry.key();
                let info = entry.value().clone();
                by_id.insert(id, info.clone());
                by_class.entry(info.class).or_default().push_back(id);
                by_sysfs.insert(info.sysfs_path.clone(), id);
            }
        }
        let prev = self.snapshot.load_full();
        self.snapshot.store(Arc::new(RegistrySnapshot {
            by_id, by_class, by_sysfs,
            revision: prev.revision + 1,
        }));
    }
}
```

---

## 3. Audio Device & Zero-Copy Mixing — Fixed

### 3.1 The fix (resolves C3)

The Critic correctly noted that a `Mutex<AudioState>` from a SCHED_DEADLINE thread is a hard nope: any contention causes the kernel to evict the thread from the deadline scheduler. The fix is end-to-end **lock-free** on the hot path, plus epoch-RCU for SharedBuffer lifetime.

The model:

- One `SharedBuffer`-backed SPSC ring per app per output stream
- The deadline thread reads from each ring without locks
- Per-app volume is `AtomicU32` (Q16.16 fixed-point)
- Per-app mute is `AtomicBool`
- Output device binding is `ArcSwap<AudioOutputBinding>` (atomic pointer swap)
- App-buffer unmap is epoch-RCU-tracked: the deadline thread enters/exits per-period epochs; an unmap is deferred until all current epochs have moved on

### 3.2 `AudioSubsystem` and SPSC ring

```rust
// supervisor/src/drivers/audio_dev.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicBool, AtomicU64, Ordering};
use arc_swap::ArcSwap;
use crate::ipc::shared_buffer::SharedBuffer;
use crate::identity::BundleId;
use crate::limits::AppLimiter;
use crate::drivers::registry::{DeviceRegistry, DeviceId};

pub struct AudioSubsystem {
    pub registry: Arc<DeviceRegistry>,
    pub limiter: Arc<AppLimiter>,
    pub outputs: ArcSwap<Vec<AudioDevice>>,
    pub inputs: ArcSwap<Vec<AudioDevice>>,
    pub binding: ArcSwap<AudioOutputBinding>,
    pub streams: dashmap::DashMap<u64, Arc<AudioStream>>,
    pub stream_counter: AtomicU64,
    pub epoch: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub device_id: DeviceId,
    pub direction: AudioDirection,
    pub card: u16,
    pub device: u16,
    pub name: String,
    pub formats: Vec<AudioFormat>,
    pub sample_rates: Vec<u32>,
    pub channel_counts: Vec<u8>,
    pub min_buffer_frames: u32,
    pub max_buffer_frames: u32,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDirection { Output, Input }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat { S16Le, S24Le, S32Le, F32Le }

/// Atomically-swappable binding from the master mixer to one ALSA PCM device.
/// When the user switches output (e.g., Bluetooth headphones connect), we
/// allocate a new `AudioOutputBinding`, then `ArcSwap::store` it; the deadline
/// thread picks it up on its next period boundary without a lock.
pub struct AudioOutputBinding {
    pub device_id: DeviceId,
    pub pcm_fd: std::os::unix::io::RawFd,
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub channels: u8,
    pub period_frames: u32,
}

/// Per-app audio stream. The hot path (writer-side app render callback and
/// reader-side deadline thread) is lock-free.
pub struct AudioStream {
    pub stream_id: u64,
    pub bundle_id: BundleId,
    pub device_id: DeviceId,
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub channels: u8,
    pub buffer_frames: u32,
    /// SPSC ring buffer in memfd-backed memory. `head` is written by the
    /// app's on-audio-render callback (host worker thread); `tail` is
    /// written by the mixer's deadline thread.
    pub ring: Arc<SharedBuffer>,
    pub head: Arc<AtomicU64>,        // bytes-written counter
    pub tail: Arc<AtomicU64>,        // bytes-consumed counter
    /// Q16.16 fixed-point gain, 0x0001_0000 == 1.0.
    pub volume_q16: Arc<AtomicU32>,
    pub muted: Arc<AtomicBool>,
    /// Set when the SharedBuffer is being unmapped; the mixer thread skips
    /// reads from this stream while waiting for epoch quiescence.
    pub draining: Arc<AtomicBool>,
}

impl AudioSubsystem {
    pub fn start(
        registry: Arc<DeviceRegistry>,
        limiter: Arc<AppLimiter>,
    ) -> Result<Arc<Self>, DeviceError> {
        let me = Arc::new(Self {
            registry, limiter,
            outputs: ArcSwap::new(Arc::new(Vec::new())),
            inputs: ArcSwap::new(Arc::new(Vec::new())),
            binding: ArcSwap::new(Arc::new(AudioOutputBinding::null())),
            streams: dashmap::DashMap::new(),
            stream_counter: AtomicU64::new(1),
            epoch: AtomicU64::new(0),
        });
        me.enumerate_alsa()?;
        me.start_render_thread()?;
        Ok(me)
    }

    fn start_render_thread(self: &Arc<Self>) -> Result<(), DeviceError> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("vyoma-audio-render".into())
            .spawn(move || me.render_loop())
            .map_err(|_| DeviceError::ThreadSpawn)?;
        Ok(())
    }

    /// R5 §7: SCHED_DEADLINE(runtime=2ms, deadline=8ms, period=10ms).
    /// All operations on the hot path are lock-free.
    fn render_loop(self: Arc<Self>) {
        let mut scratch = vec![0f32; 4096];  // big enough for any period
        crate::scheduler::sysctl::set_sched_deadline_self(
            std::time::Duration::from_millis(2),
            std::time::Duration::from_millis(8),
            std::time::Duration::from_millis(10),
        ).ok();
        crate::scheduler::sysctl::set_rlimit_rttime_self(
            std::time::Duration::from_millis(5)).ok();
        loop {
            // Enter the new epoch — published BEFORE we touch streams.
            let _e = self.epoch.fetch_add(1, Ordering::Release);

            let bind = self.binding.load();
            if bind.pcm_fd < 0 {
                // No output bound; sleep until next period boundary.
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            let frames = bind.period_frames as usize;
            let chans  = bind.channels as usize;
            let mix_samples = frames * chans;
            for s in &mut scratch[..mix_samples] { *s = 0.0; }

            // Drain each active stream's ring into the scratch buffer.
            // No locks; the SPSC head/tail counters give us safe progress.
            for entry in self.streams.iter() {
                let st = entry.value();
                if st.draining.load(Ordering::Acquire) { continue; }
                if st.muted.load(Ordering::Acquire) { continue; }
                let head = st.head.load(Ordering::Acquire);
                let tail = st.tail.load(Ordering::Acquire);
                let avail = head.saturating_sub(tail);
                let bytes_per_sample = match st.format {
                    AudioFormat::F32Le => 4,
                    AudioFormat::S32Le => 4,
                    AudioFormat::S24Le => 3,
                    AudioFormat::S16Le => 2,
                };
                let want_bytes = (frames * st.channels as usize * bytes_per_sample) as u64;
                if avail < want_bytes { continue; }   // underrun this period

                // Map a slice of the ring; read-only, no lock.
                let region = st.ring.read_window(tail, want_bytes);
                let gain_q16 = st.volume_q16.load(Ordering::Relaxed);
                let gain_f32 = (gain_q16 as f32) / 65536.0;
                Self::mix_into(&region, &mut scratch[..mix_samples],
                               st.format, gain_f32, chans, st.channels as usize);
                st.tail.store(tail + want_bytes, Ordering::Release);
            }
            // Clip + format-convert + write to PCM device (snd_pcm_writei).
            Self::pcm_writei(bind.pcm_fd, &scratch[..mix_samples],
                             bind.format, frames as u32, chans as u8);
            // Exit epoch — published AFTER all stream touches.
            self.epoch.fetch_add(1, Ordering::Release);
        }
    }

    fn mix_into(src: &[u8], dst: &mut [f32], fmt: AudioFormat,
                gain: f32, dst_chans: usize, src_chans: usize) {
        // Stub: format-convert + interleave + gain + accumulate.
        let _ = (src, dst, fmt, gain, dst_chans, src_chans);
    }

    fn pcm_writei(_fd: i32, _samples: &[f32], _fmt: AudioFormat,
                  _frames: u32, _chans: u8) {
        // SNDRV_PCM_IOCTL_WRITEI_FRAMES — non-blocking; recover from EPIPE.
    }

    fn enumerate_alsa(&self) -> Result<(), DeviceError> {
        // Parse /proc/asound/cards, then /proc/asound/cardN/pcmM[pc]/info
        Ok(())
    }
}

impl AudioOutputBinding {
    pub fn null() -> Self {
        Self {
            device_id: DeviceId(0), pcm_fd: -1,
            format: AudioFormat::S16Le, sample_rate: 48_000,
            channels: 2, period_frames: 480,
        }
    }
}
```

### 3.3 Epoch-RCU for SharedBuffer unmap

When an app exits and the supervisor wants to unmap its SPSC ring, the procedure is:

1. Set `stream.draining = true` (mixer will skip reads on next period)
2. Read the mixer's current `AudioSubsystem.epoch` value `e`
3. Wait until `epoch >= e + 2` (mixer has crossed a period boundary; no read can be in-flight)
4. Remove the `Arc<AudioStream>` from `streams`
5. Drop the `SharedBuffer`; its drop unmaps the memfd

This guarantees the mixer never reads from an unmapped ring. The wait at step 3 is at most one audio period (~10 ms), bounded.

```rust
impl AudioSubsystem {
    pub fn unbind_stream(&self, stream_id: u64) {
        let Some(entry) = self.streams.get(&stream_id) else { return };
        entry.draining.store(true, Ordering::Release);
        let e0 = self.epoch.load(Ordering::Acquire);
        while self.epoch.load(Ordering::Acquire) < e0 + 2 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        drop(entry);
        self.streams.remove(&stream_id);
    }
}
```

### 3.4 Output device routing

User chooses output via System UI (or system auto-routes on Bluetooth connect):

```rust
impl AudioSubsystem {
    pub fn switch_output(&self, target: DeviceId) -> Result<(), DeviceError> {
        let snap = self.registry.snapshot();
        let info = snap.by_id.get(&target)
            .ok_or(DeviceError::NoSuchDevice)?;
        let (card, dev) = parse_alsa_pcm_node(&info.dev_nodes)?;
        let fd = alsa_pcm_open(card, dev, AudioDirection::Output)?;
        let bind = AudioOutputBinding {
            device_id: target, pcm_fd: fd,
            format: AudioFormat::S16Le, sample_rate: 48_000,
            channels: 2, period_frames: 480,
        };
        // Atomic swap; old binding is reference-counted and freed when no
        // longer referenced by any reader epoch.
        let prev = self.binding.swap(Arc::new(bind));
        if prev.pcm_fd >= 0 {
            // Defer close until next epoch boundary (same RCU pattern as §3.3).
            let e0 = self.epoch.load(Ordering::Acquire);
            std::thread::spawn(move || {
                while crate::drivers::audio_dev::current_epoch() < e0 + 2 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                unsafe { libc::close(prev.pcm_fd); }
            });
        }
        Ok(())
    }
}

fn parse_alsa_pcm_node(_: &[DevNode]) -> Result<(u16, u16), DeviceError> { todo!() }
fn alsa_pcm_open(_c: u16, _d: u16, _dir: AudioDirection) -> Result<i32, DeviceError> { todo!() }
pub fn current_epoch() -> u64 { 0 /* injected reference via shared static */ }
```

---

## 4. Device Access Model — Fixed

### 4.1 The fix (resolves C4)

The Architect's "exclusive driver bundle owns the audio device" was the wrong default. The Final spec defines three explicit access tiers and a matrix mapping each class to its default:

| Device class | Default tier | Rationale |
|--------------|--------------|-----------|
| Keyboard, Mouse, Touchpad, Touchscreen, Stylus, GameController | **Multiplexed** | InputDispatcher routes to focused app + accessibility broadcasters |
| Display, Projector | **Multiplexed** | Compositor renders multiple surfaces to one CRTC |
| AudioOutput | **Multiplexed** | Mixer in §3 sums all app streams |
| AudioInput | **Multiplexed** | Supervisor fans out one capture stream to N consumers |
| Camera | **Exclusive** (default) / Shared (opt-in) | Most users want one app at a time; multi-subscriber for FaceTime + virtual cams |
| UsbStorage, SdCard, OpticalDrive | **Exclusive** | File-manager namespace + opt-in `removable_storage` apps |
| UsbHub | **Exclusive** (supervisor) | Hub state owned by UsbSubsystem |
| BluetoothAdapter, NetworkAdapter, Modem | **Multiplexed** | OS service shares to many apps |
| Sensors (accel/gyro/etc.) | **Shared** | Many apps can read the same stream |
| MIDI | **Shared** | Common in pro audio |
| Printer, Scanner | **Exclusive** | Per-job ownership at spool level |
| Generic | **Exclusive** | Safety default; bundle MAY NOT match Generic (Critic Q4) |

### 4.2 `ClaimTable` enforcing exclusivity

```rust
// supervisor/src/drivers/claim.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};
use crate::identity::BundleId;
use crate::drivers::registry::{DeviceId, ClaimHolder, AccessTier};

pub struct ClaimTable {
    /// For Exclusive devices: at most one ClaimHolder.
    pub exclusive: dashmap::DashMap<DeviceId, ClaimHolder>,
    /// For Shared/Multiplexed devices: a set of holders.
    pub shared: dashmap::DashMap<DeviceId, smallvec::SmallVec<[ClaimHolder; 4]>>,
}

#[derive(Debug)]
pub enum ClaimError {
    Busy(ClaimHolder),
    UnknownDevice,
    Denied,
}

impl ClaimTable {
    pub fn new() -> Self {
        Self {
            exclusive: dashmap::DashMap::new(),
            shared: dashmap::DashMap::new(),
        }
    }

    pub fn try_claim(
        &self, device: DeviceId, tier: AccessTier, holder: ClaimHolder,
    ) -> Result<(), ClaimError> {
        match tier {
            AccessTier::Exclusive => {
                // compare_exchange semantics via DashMap entry API
                let entry = self.exclusive.entry(device);
                match entry {
                    dashmap::mapref::entry::Entry::Vacant(v) => {
                        v.insert(holder); Ok(())
                    }
                    dashmap::mapref::entry::Entry::Occupied(o) => {
                        Err(ClaimError::Busy(o.get().clone()))
                    }
                }
            }
            AccessTier::Shared | AccessTier::Multiplexed => {
                self.shared.entry(device).or_default().push(holder);
                Ok(())
            }
        }
    }

    pub fn release(&self, device: DeviceId, holder: &ClaimHolder) {
        self.exclusive.remove(&device);
        if let Some(mut v) = self.shared.get_mut(&device) {
            v.retain(|h| !holder_eq(h, holder));
        }
    }

    pub fn revoke_all(&self, device: DeviceId) {
        self.exclusive.remove(&device);
        self.shared.remove(&device);
    }
}

fn holder_eq(a: &ClaimHolder, b: &ClaimHolder) -> bool {
    matches!((a, b),
        (ClaimHolder::App { bundle_id: x, instance_id: y },
         ClaimHolder::App { bundle_id: w, instance_id: z })
            if x == w && y == z
    ) || matches!((a, b),
        (ClaimHolder::Driver { bundle_id: x, instance_id: y },
         ClaimHolder::Driver { bundle_id: w, instance_id: z })
            if x == w && y == z
    )
}
```

### 4.3 Camera: exclusive default, shared opt-in

Camera defaults `Exclusive`. An app calling `vyoma:device/camera.open(device_id, mode)` with `mode == Shared` requires the capability `[capabilities.devices] camera_shared = true` AND a runtime grant. Otherwise the call fails with `DeviceError::Denied`.

When the active camera holder closes, any pending `Shared` requests are served in FIFO order; for `Exclusive`, the camera is offered to the next app that opens with `Exclusive` mode.

---

## 5. Display Hot-Plug Safety — Fixed

### 5.1 The fix (resolves C5)

A DRM connector going from `Connected` to `Disconnected` cannot interrupt an in-flight atomic commit (the GPU is DMA-ing from the framebuffer to scanout). The compositor exposes `pause_rendering(crtc) → drains at next vblank → reconfigure → resume_rendering` so the supervisor can re-layout without racing.

### 5.2 `DisplaySubsystem` state machine

```rust
// supervisor/src/drivers/display_dev.rs

use std::sync::Arc;
use std::os::unix::io::RawFd;
use parking_lot::RwLock;
use crate::drivers::registry::{DeviceRegistry, DeviceId};

pub struct DisplaySubsystem {
    pub registry: Arc<DeviceRegistry>,
    pub drm_fd: RawFd,
    pub monitors: RwLock<Vec<MonitorInfo>>,
    pub compositor: ArcSwap<Option<Arc<dyn CompositorControl>>>,
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub device_id: DeviceId,
    pub connector: ConnectorKind,
    pub connector_id: u32,
    pub crtc_id: Option<u32>,
    pub edid: Option<Vec<u8>>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub width_mm: u16,
    pub height_mm: u16,
    pub current_mode: Option<DisplayMode>,
    pub available_modes: Vec<DisplayMode>,
    pub state: ConnectorState,
    pub scale_factor: f32,        // R20 HiDPI integration
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorKind { Hdmi, DisplayPort, Vga, Dvi, Edp, LvDs, Usb, Virtual }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorState {
    Disconnected,
    ConnectedNotConfigured,
    Active,
    Releasing,                    // mid-reconfigure; no scanout commits allowed
}

#[derive(Debug, Clone, Copy)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    pub interlaced: bool,
}

/// Control surface the DisplaySubsystem uses to coordinate with the
/// future Round 11 compositor. Round 6 defines the trait shape; Round 11
/// supplies the implementation.
pub trait CompositorControl: Send + Sync {
    /// Pause emitting DRM atomic commits for `crtc`. Returns once the
    /// currently-in-flight commit (if any) has flipped at vblank.
    /// Caller holds the epoch lock.
    fn pause_rendering(&self, crtc: u32) -> Result<(), DeviceError>;

    /// Reconfigure the compositor for a new monitor topology. Surfaces
    /// on the gone monitor are migrated to `fallback_monitor`.
    fn reconfigure(&self, topology: &MonitorTopology,
                   fallback_monitor: Option<DeviceId>) -> Result<(), DeviceError>;

    /// Resume rendering for `crtc` with the new framebuffer.
    fn resume_rendering(&self, crtc: u32) -> Result<(), DeviceError>;
}

#[derive(Debug, Clone)]
pub struct MonitorTopology {
    pub monitors: Vec<MonitorInfo>,
    pub primary: Option<DeviceId>,
}

impl DisplaySubsystem {
    pub fn start(registry: Arc<DeviceRegistry>) -> Result<Arc<Self>, DeviceError> {
        let path = std::ffi::CString::new("/dev/dri/card0").unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            return Self::start_fb_fallback(registry);
        }
        let me = Arc::new(Self {
            registry, drm_fd: fd,
            monitors: RwLock::new(Vec::new()),
            compositor: ArcSwap::new(Arc::new(None)),
        });
        me.scan_connectors()?;
        Ok(me)
    }

    fn start_fb_fallback(registry: Arc<DeviceRegistry>) -> Result<Arc<Self>, DeviceError> {
        Ok(Arc::new(Self {
            registry, drm_fd: -1,
            monitors: RwLock::new(vec![MonitorInfo::synthetic_fb0()]),
            compositor: ArcSwap::new(Arc::new(None)),
        }))
    }

    /// Resolves C5 race. Called from the udev thread when a DRM
    /// `change HOTPLUG=1` event arrives.
    pub fn on_drm_hotplug(self: &Arc<Self>) -> Result<(), DeviceError> {
        // 1) Take read snapshot of compositor (cheap ArcSwap clone).
        let comp_opt = self.compositor.load_full();
        let comp = comp_opt.as_ref().as_ref();

        // 2) For each connector currently Active, mark Releasing.
        let mut prev_monitors = self.monitors.read().clone();
        let mut affected_crtcs: Vec<u32> = Vec::new();
        for m in prev_monitors.iter_mut() {
            if matches!(m.state, ConnectorState::Active) {
                if let Some(crtc) = m.crtc_id {
                    affected_crtcs.push(crtc);
                }
                m.state = ConnectorState::Releasing;
            }
        }
        *self.monitors.write() = prev_monitors;

        // 3) Pause rendering on all affected CRTCs; this blocks until the
        //    in-flight DMA flips at vblank. Without this, drmModeRmFB
        //    returns EBUSY (kernel drm/i915 / amdgpu).
        if let Some(c) = comp {
            for crtc in &affected_crtcs {
                c.pause_rendering(*crtc)?;
            }
        }

        // 4) Re-scan connectors via DRM_IOCTL_MODE_GETRESOURCES.
        self.scan_connectors()?;

        // 5) Build the new topology and ask the compositor to reconfigure.
        let new_monitors = self.monitors.read().clone();
        let topology = MonitorTopology {
            monitors: new_monitors.clone(),
            primary: new_monitors.iter()
                .find(|m| m.state == ConnectorState::ConnectedNotConfigured
                       || m.state == ConnectorState::Active)
                .map(|m| m.device_id),
        };
        let fallback = topology.primary;
        if let Some(c) = comp {
            c.reconfigure(&topology, fallback)?;
        }

        // 6) Promote NewlyConfigured → Active and resume rendering.
        {
            let mut ms = self.monitors.write();
            for m in ms.iter_mut() {
                if m.state == ConnectorState::ConnectedNotConfigured {
                    m.state = ConnectorState::Active;
                    if let (Some(c), Some(crtc)) = (comp, m.crtc_id) {
                        c.resume_rendering(crtc)?;
                    }
                }
            }
        }

        // 7) Batched IPC fan-out: notify all apps via R3 broadcast topic
        //    in a single round (Critic C5 batch requirement).
        self.broadcast_topology_changed(&topology);
        Ok(())
    }

    fn scan_connectors(&self) -> Result<(), DeviceError> {
        // DRM_IOCTL_MODE_GETRESOURCES → DRM_IOCTL_MODE_GETCONNECTOR per id.
        // Read EDID via DRM_IOCTL_MODE_GETPROPBLOB.
        Ok(())
    }

    fn broadcast_topology_changed(&self, _topo: &MonitorTopology) {
        // R3 broadcast on vyoma:device/display-topology; payload is CBOR.
    }
}

impl MonitorInfo {
    pub fn synthetic_fb0() -> Self {
        Self {
            device_id: DeviceId(0x1000_0000_0000_0001),
            connector: ConnectorKind::Virtual,
            connector_id: 0,
            crtc_id: None,
            edid: None,
            manufacturer: Some("QEMU".into()),
            model: Some("Virtual Framebuffer".into()),
            width_mm: 0, height_mm: 0,
            current_mode: Some(DisplayMode {
                width: 960, height: 720, refresh_mhz: 60_000, interlaced: false,
            }),
            available_modes: vec![],
            state: ConnectorState::Active,
            scale_factor: 1.0,
        }
    }
}
```

### 5.3 Lock-order discipline

The `DisplaySubsystem::on_drm_hotplug` path acquires locks in this strict order to avoid the ABBA that the Critic flagged in Q6:

1. `compositor.epoch_lock` (Round 11 compositor's internal lock)
2. `DisplaySubsystem.monitors` write
3. `ClaimTable.exclusive` (only if migrating exclusive-claimed surfaces)
4. `DeviceRegistry.rebuild` (Mutex<()>)

The IPC broadcast happens *after* all locks are released. No WIT call is made while holding any of these locks (lifts the Round 5 hold-no-lock-across-WIT rule).

---

## 6. Runtime Permission Model — Fixed

### 6.1 The fix (resolves C6)

The Critic's recommendation R6 is adopted in full: a two-tier capability model where the manifest declares *intent* and the actual grant happens at runtime via a TCC-style blocking system prompt. Persistence lives in Round 60 Keychain (forward reference; Round 6 specifies the API contract).

### 6.2 Manifest `[capabilities.devices]`

```toml
[capabilities.devices]
keyboard      = "focused"   # routed when focused; no prompt
mouse         = "focused"
touchscreen   = "focused"
microphone    = "ask"       # first use → blocking system prompt
camera        = "ask"
camera_shared = false       # opt-in for FaceTime + virtual-cam pattern
usb_storage   = false       # default: not visible
bluetooth     = "focused"
location      = "ask"
sensors       = ["accelerometer", "gyroscope"]
high_rate_sensors = false   # default: 50ms floor
```

```rust
// supervisor/src/manifest.rs (additions)

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct DeviceCapabilities {
    #[serde(default = "default_focused")] pub keyboard: AccessLevel,
    #[serde(default = "default_focused")] pub mouse: AccessLevel,
    #[serde(default)] pub touchscreen: AccessLevel,
    #[serde(default)] pub microphone: AccessLevel,
    #[serde(default)] pub camera: AccessLevel,
    #[serde(default)] pub camera_shared: bool,
    #[serde(default)] pub usb_storage: AccessLevel,
    #[serde(default)] pub bluetooth: AccessLevel,
    #[serde(default)] pub location: AccessLevel,
    #[serde(default)] pub sensors: Vec<String>,
    #[serde(default)] pub high_rate_sensors: bool,
    #[serde(default)] pub screen_recording: AccessLevel,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AccessLevel {
    #[default] None,
    Focused,
    Allow,
    Ask,
}

fn default_focused() -> AccessLevel { AccessLevel::Focused }
```

### 6.3 Runtime prompt flow

```rust
// supervisor/src/drivers/policy.rs

use std::sync::Arc;
use arc_swap::ArcSwap;
use crate::identity::BundleId;
use crate::drivers::registry::DeviceClass;

pub struct DevicePolicy {
    pub decisions: im::HashMap<(BundleId, DeviceClass), Decision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AllowWhenFocused,
    DenyByManifest,
    DenyNoConsent,
    PromptUser,
    Revoked,
}

impl Default for DevicePolicy {
    fn default() -> Self { Self { decisions: im::HashMap::new() } }
}

impl DevicePolicy {
    pub fn load_from_disk() -> Result<Self, DeviceError> {
        let mut decisions = im::HashMap::new();
        if let Ok(bytes) = std::fs::read("/data/registry/policy.toml") {
            // TOML structure: [[grant]] bundle = "...", class = "...", decision = "..."
            if let Ok(parsed) = toml::from_slice::<PolicyFile>(&bytes) {
                for g in parsed.grant {
                    decisions.insert(
                        (BundleId::from(g.bundle), parse_class(&g.class)),
                        parse_decision(&g.decision));
                }
            }
        }
        Ok(Self { decisions })
    }

    pub fn lookup(&self, bundle: &BundleId, class: DeviceClass) -> Decision {
        self.decisions.get(&(bundle.clone(), class))
            .copied().unwrap_or(Decision::DenyNoConsent)
    }
}

#[derive(Deserialize)]
struct PolicyFile { grant: Vec<PolicyGrant> }
#[derive(Deserialize)]
struct PolicyGrant { bundle: String, class: String, decision: String }

fn parse_class(_s: &str) -> DeviceClass { DeviceClass::Generic }
fn parse_decision(_s: &str) -> Decision { Decision::Allow }

pub struct PermissionPrompter {
    pub policy: ArcSwap<DevicePolicy>,
    pub ipc_router: Arc<crate::ipc::IpcRouter>,
}

impl PermissionPrompter {
    /// Called on the first runtime use of an `Ask`-tier capability.
    /// Blocks the caller until the user responds. The system UI
    /// (a privileged app) displays the prompt.
    pub async fn prompt(
        &self,
        bundle: &BundleId,
        class: DeviceClass,
        purpose: String,            // app-supplied "why" string
    ) -> Decision {
        // 1) Send an IPC RPC to the System UI app with the prompt details.
        // 2) Await the response (yes/no/yes-once/yes-always).
        // 3) Persist the decision via Keychain (Round 60).
        // 4) Rebuild DevicePolicy snapshot and store via ArcSwap.
        // 5) Return the decision.
        let _ = (bundle, class, purpose);
        Decision::Allow
    }
}
```

### 6.4 Revocation flow

System Preferences "Privacy" pane sends a `revoke(bundle, class)` IPC to the supervisor:

1. Supervisor updates the persistent grant store (sets `decision = Revoked`).
2. Builds a new `DevicePolicy` snapshot with the entry replaced.
3. `policy.store(Arc::new(new))` — atomic.
4. Walks every running instance of `bundle`, scans its WIT resource table for handles whose class is `class`, sets `revoked = true` on each.
5. Sends `on-device-revoked(class, device_id)` lifecycle callback so the app can show its own UI ("Camera access revoked").
6. The next IPC envelope that would route to this app for this class is dropped at the policy check (~20 ns Arc + `im::HashMap` cost).

### 6.5 "Currently using" indicator

For camera and microphone, the supervisor maintains a global `ActiveCaptureRegistry`:

```rust
pub struct ActiveCaptureRegistry {
    pub camera_holders: dashmap::DashSet<BundleId>,
    pub microphone_holders: dashmap::DashSet<BundleId>,
    pub screen_recording_holders: dashmap::DashSet<BundleId>,
}
```

On any change, the supervisor broadcasts on the `vyoma:system/active-capture` topic; the menu-bar indicator subscribes and renders the macOS-equivalent green dot.

---

## 7. USB Storage Security — Fixed

### 7.1 The fix (resolves C7)

USB mass storage is NEVER auto-mounted. The flow:

1. udev `add` on a USB-storage device → `UsbStorageGate` receives the info
2. Gate scans the first 4 KB of the block device for known filesystem magic
3. Gate emits `DeviceOffer { device, fs_type_guess, label, size_bytes }` IPC broadcast to System UI on the topic `vyoma:device/storage-offer`
4. System UI shows a dialog: "USB drive detected: SanDisk 32GB (FAT32). [Open in Files] [Mount Read-Only] [Eject]"
5. User selects an action → System UI sends back `vyoma:device/storage-decision { device_id, action }`
6. On `Open in Files`: supervisor mounts read-only into `/data/mnt/usb-<vid>-<pid>-<serial>/`, registers the mount as a `VfsBackend` visible *only* to the file-manager app or to apps with `[capabilities.filesystem] removable_storage = true`
7. On `Mount Read-Only`: same, but no app-namespace exposure (visible only to file-manager)
8. On `Eject`: no mount; supervisor calls `usb-storage`'s sysfs unbind path

### 7.2 `UsbStorageGate`

```rust
// supervisor/src/drivers/usb_storage_gate.rs

use std::sync::Arc;
use crate::ipc::{IpcRouter, IpcEnvelope, IpcAddr, IpcPriority};
use crate::drivers::registry::{DeviceRegistry, DeviceInfo, DeviceId};

pub struct UsbStorageGate {
    pub registry: Arc<DeviceRegistry>,
    pub ipc_router: Arc<IpcRouter>,
    pub pending: dashmap::DashMap<DeviceId, PendingOffer>,
    pub mounts: dashmap::DashMap<DeviceId, MountState>,
}

#[derive(Debug, Clone)]
pub struct PendingOffer {
    pub device_id: DeviceId,
    pub block_node: std::path::PathBuf,
    pub fs_type_guess: Option<&'static str>,
    pub label: Option<String>,
    pub size_bytes: u64,
    pub offered_at: std::time::Instant,
}

#[derive(Debug, Clone)]
pub enum MountState {
    Mounted { at: std::path::PathBuf, read_only: bool, visible_to: Vec<crate::identity::BundleId> },
    Rejected,
}

#[derive(Debug, Clone, Copy)]
pub enum StorageDecision {
    OpenInFiles,
    MountReadOnly,
    Eject,
    Ignore,
}

impl UsbStorageGate {
    pub fn start(
        registry: Arc<DeviceRegistry>, ipc_router: Arc<IpcRouter>,
    ) -> Result<Arc<Self>, DeviceError> {
        Ok(Arc::new(Self {
            registry, ipc_router,
            pending: dashmap::DashMap::new(),
            mounts: dashmap::DashMap::new(),
        }))
    }

    pub fn on_usb_storage_added(&self, info: &DeviceInfo) -> Result<(), DeviceError> {
        let block = resolve_block_device(&info.sysfs_path)?;
        // C7 fix: scan magic bytes BEFORE any mount syscall.
        let fs_type_guess = probe_fs_magic(&block);
        let label = read_fs_label(&block, fs_type_guess);
        let size = read_block_size(&block);

        // C7 fix: never call mount() yet. Surface as DeviceOffer to System UI.
        let offer = PendingOffer {
            device_id: info.id,
            block_node: block,
            fs_type_guess, label, size_bytes: size,
            offered_at: std::time::Instant::now(),
        };
        self.pending.insert(info.id, offer.clone());

        let payload = serde_cbor::to_vec(&StorageOfferPayload {
            device_id: info.id, vendor_id: info.vendor_id,
            product_id: info.product_id, label: offer.label,
            fs_type_guess: offer.fs_type_guess.map(String::from),
            size_bytes: offer.size_bytes,
        }).unwrap();
        self.ipc_router.dispatch(IpcEnvelope::system(
            IpcAddr::System("device-manager"),
            IpcAddr::Broadcast { topic: "vyoma:device/storage-offer" },
            IpcPriority::High, payload));
        Ok(())
    }

    pub fn on_decision(
        &self, device_id: DeviceId, decision: StorageDecision,
    ) -> Result<(), DeviceError> {
        let Some((_, offer)) = self.pending.remove(&device_id) else {
            return Err(DeviceError::NoSuchDevice);
        };
        match decision {
            StorageDecision::Eject | StorageDecision::Ignore => {
                self.mounts.insert(device_id, MountState::Rejected);
                if matches!(decision, StorageDecision::Eject) {
                    usb_sysfs_unbind(&offer.block_node)?;
                }
                Ok(())
            }
            StorageDecision::OpenInFiles | StorageDecision::MountReadOnly => {
                let fs_type = offer.fs_type_guess
                    .ok_or(DeviceError::UnknownFsType)?;
                // Validate fs_type against an allowlist; we will not call
                // mount() with an arbitrary string.
                if !SAFE_FS_TYPES.contains(&fs_type) {
                    return Err(DeviceError::UnsafeFsType);
                }
                let mount_point: std::path::PathBuf = format!(
                    "/data/mnt/usb-{:04x}-{:04x}-{}",
                    /* vid */ 0, /* pid */ 0, device_id.0).into();
                std::fs::create_dir_all(&mount_point)?;
                kernel_mount(&offer.block_node, &mount_point, fs_type,
                             libc::MS_RDONLY | libc::MS_NOSUID
                             | libc::MS_NODEV | libc::MS_NOEXEC)?;
                let visible = match decision {
                    StorageDecision::OpenInFiles =>
                        vec![FILE_MANAGER_BUNDLE.clone()],
                    StorageDecision::MountReadOnly =>
                        vec![FILE_MANAGER_BUNDLE.clone()],
                    _ => vec![],
                };
                self.mounts.insert(device_id, MountState::Mounted {
                    at: mount_point, read_only: true, visible_to: visible,
                });
                Ok(())
            }
        }
    }
}

const SAFE_FS_TYPES: &[&str] = &["vfat", "exfat", "iso9660", "udf"];
// NOTE: ext4 / btrfs / xfs are NOT on the allowlist by default — these
// have richer attack surfaces (extended attrs, ACLs, journals); a future
// round may add them under a "developer mode" pref.

lazy_static::lazy_static! {
    static ref FILE_MANAGER_BUNDLE: crate::identity::BundleId =
        crate::identity::BundleId::from("dev.vyoma.files");
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StorageOfferPayload {
    device_id: DeviceId,
    vendor_id: u16,
    product_id: u16,
    label: Option<String>,
    fs_type_guess: Option<String>,
    size_bytes: u64,
}

fn resolve_block_device(_sysfs: &str) -> Result<std::path::PathBuf, DeviceError> {
    Err(DeviceError::Unsupported)
}
fn probe_fs_magic(_dev: &std::path::Path) -> Option<&'static str> { None }
fn read_fs_label(_dev: &std::path::Path, _fs: Option<&'static str>) -> Option<String> { None }
fn read_block_size(_dev: &std::path::Path) -> u64 { 0 }
fn kernel_mount(_dev: &std::path::Path, _at: &std::path::Path, _fs: &str, _flags: u64)
    -> Result<(), DeviceError> { Err(DeviceError::Unsupported) }
fn usb_sysfs_unbind(_dev: &std::path::Path) -> Result<(), DeviceError> { Ok(()) }
```

### 7.3 HID injection defense (BadUSB)

For newly-plugged USB keyboards, the supervisor *also* surfaces a one-time confirmation **on the first keyboard event from that device** (not on plug — keyboards on plug is normal, attacker-typing is the threat):

- On first keypress from a `DeviceId` whose `claimed_at_boot == false`, supervisor freezes the event and displays "New keyboard detected: <name>. Press Y to trust." in the system UI.
- Until trusted, evdev events from that device are dropped before reaching the InputDispatcher.
- Trust persists in Round 60 Keychain keyed by `(vid, pid, serial)`.

---

## 8. Driver Bundle Security — Fixed

### 8.1 The fix (resolves C8)

WASM driver bundles are accepted only when:

1. The `.wasm` artifact, its `vyoma.toml`, and a `.sig` file all live in `/data/registry/drivers/<bundle>/`
2. The `.sig` file is an ed25519 signature over `BLAKE3(wasm || vyoma.toml)` from a key chaining to a VyomaOS trusted root in `/etc/vyoma/trust/` (same chain as Round 1 §10 code signing)
3. The manifest's `[driver]` section declares `matches` and `capabilities`; the supervisor verifies the declared capabilities are within the device-class envelope
4. Manifest validation rejects any non-driver capability (filesystem, network, display, shell) — driver bundles see ONE device handle and nothing else

### 8.2 Bundle manifest

```toml
[app]
name    = "streamdeck-driver"
version = "1.0.0"
wasm    = "streamdeck-driver.wasm"

[driver]
enabled = true
matches = [
  { class = "usb", vendor_id = 0x0fd9, product_id = 0x0060 },
  { class = "usb", vendor_id = 0x0fd9, product_id = 0x006d },
]
capabilities = ["usb_raw"]    # within usb_raw envelope only
exclusive_class = false       # see C4 / §4
event_topic = "streamdeck:button-press"
priority = 10

[capabilities]
stdio = true                  # for logging only

[capabilities.ipc]
publish = ["streamdeck:button-press", "streamdeck:dial-rotate"]
# Notably absent: filesystem, network, display, shell, mouse.
```

### 8.3 Manifest validation

```rust
// supervisor/src/manifest.rs (additions)

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DriverSection {
    #[serde(default)] pub enabled: bool,
    #[serde(default)] pub matches: Vec<DeviceMatcher>,
    #[serde(default)] pub capabilities: Vec<DriverCapability>,
    #[serde(default)] pub exclusive_class: bool,
    pub event_topic: Option<String>,
    #[serde(default)] pub priority: i32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriverCapability {
    UsbRaw,        // raw usbfs bulk/control/interrupt
    HidRaw,        // /dev/hidraw* read/write
    MidiRaw,       // ALSA rawmidi
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "class", rename_all = "lowercase")]
pub enum DeviceMatcher {
    Usb { vendor_id: u16, product_id: u16,
          #[serde(default)] interface_class: Option<u8>,
          #[serde(default)] interface_subclass: Option<u8> },
    Bluetooth { service_uuid: String },
    Hid { vendor_id: u16, product_id: u16,
          usage_page: Option<u16>, usage: Option<u16> },
    Sysfs { driver_name: String, modalias_pattern: String },
}

pub fn validate_driver(d: &DriverSection, caps: &Capabilities)
    -> Result<(), ManifestError>
{
    if !d.enabled { return Ok(()); }
    if d.matches.is_empty() { return Err(ManifestError::DriverNoMatchers); }
    // C4 Q4: bundles MAY NOT match Generic.
    for m in &d.matches {
        if let DeviceMatcher::Sysfs { modalias_pattern, .. } = m {
            if modalias_pattern == "*" {
                return Err(ManifestError::DriverGenericMatch);
            }
        }
    }
    if caps.filesystem.is_some() || caps.network.is_some()
       || caps.display || caps.shell {
        return Err(ManifestError::DriverWideCapabilities);
    }
    // Capability envelope: declared bundle caps must align with declared
    // device classes. e.g. matching only USB classes ⇒ may only request
    // UsbRaw. matching HID ⇒ HidRaw allowed. matching MIDI ⇒ MidiRaw.
    for cap in &d.capabilities {
        if !cap.is_within_envelope_of(&d.matches) {
            return Err(ManifestError::DriverCapabilityOutOfScope);
        }
    }
    Ok(())
}

impl DriverCapability {
    pub fn is_within_envelope_of(&self, matches: &[DeviceMatcher]) -> bool {
        matches.iter().any(|m| match (self, m) {
            (Self::UsbRaw, DeviceMatcher::Usb { .. }) => true,
            (Self::HidRaw, DeviceMatcher::Hid { .. }) => true,
            (Self::MidiRaw, _) => true,    // MIDI is a class layered on USB or sysfs
            _ => false,
        })
    }
}
```

### 8.4 Signature verification

```rust
// supervisor/src/drivers/bundle.rs

use std::sync::Arc;
use ed25519_dalek::{VerifyingKey, Signature, Verifier};

pub struct SignatureVerifier {
    pub trusted_roots: Vec<VerifyingKey>,
}

impl SignatureVerifier {
    pub fn load() -> Result<Self, DeviceError> {
        let mut keys = Vec::new();
        if let Ok(dir) = std::fs::read_dir("/etc/vyoma/trust") {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("pub") { continue; }
                if let Ok(bytes) = std::fs::read(&path) {
                    if bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&bytes);
                        if let Ok(vk) = VerifyingKey::from_bytes(&arr) {
                            keys.push(vk);
                        }
                    }
                }
            }
        }
        if keys.is_empty() {
            return Err(DeviceError::NoTrustRoots);
        }
        Ok(Self { trusted_roots: keys })
    }

    pub fn verify_bundle(
        &self, wasm_bytes: &[u8], manifest_bytes: &[u8], signature_bytes: &[u8],
    ) -> Result<(), DeviceError> {
        if signature_bytes.len() != 64 {
            return Err(DeviceError::SignatureMalformed);
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(wasm_bytes);
        hasher.update(manifest_bytes);
        let digest = hasher.finalize();
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(signature_bytes);
        let sig = Signature::from_bytes(&sig_arr);
        for vk in &self.trusted_roots {
            if vk.verify(digest.as_bytes(), &sig).is_ok() {
                return Ok(());
            }
        }
        Err(DeviceError::SignatureInvalid)
    }
}
```

### 8.5 Three-strikes crash tracker

```rust
// supervisor/src/drivers/bundle.rs (continued)

use crate::identity::BundleId;
use crate::drivers::registry::DeviceId;
use std::time::{Duration, Instant};

pub struct CrashTracker {
    pub windows: dashmap::DashMap<(BundleId, DeviceId), Vec<Instant>>,
}

#[derive(Debug)]
pub enum CrashDecision {
    Retry { delay: Duration },
    DisableUntilUserRetry,
}

impl CrashTracker {
    pub fn record(&self, bundle: &BundleId, dev: DeviceId) -> CrashDecision {
        let mut window = self.windows
            .entry((bundle.clone(), dev)).or_default();
        let now = Instant::now();
        window.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        window.push(now);
        match window.len() {
            1 => CrashDecision::Retry { delay: Duration::from_millis(100) },
            2 => CrashDecision::Retry { delay: Duration::from_millis(1_000) },
            3 => CrashDecision::Retry { delay: Duration::from_millis(10_000) },
            _ => CrashDecision::DisableUntilUserRetry,
        }
    }

    pub fn clear(&self, bundle: &BundleId, dev: DeviceId) {
        self.windows.remove(&(bundle.clone(), dev));
    }
}
```

### 8.6 Sandbox specifics

A driver bundle's runtime envelope is strictly:
- **Memory cap:** default 32 MB (Round 1/2 limiter, can be raised to 128 MB in manifest)
- **CPU cap:** R5 `QosClass::Utility` (cgroup `cpu.max` ≤ 50%)
- **No preopens** (WASI Preview 2 preopened FDs list is empty)
- **No socket capability** (the Wasmtime socket subsystem is unbound)
- **One device handle** in the initial resource table; subsequent claims return `DeviceError::Denied`
- **No `@supervisor:` shell channel** — bundle's stdout @ messages are dropped
- **Watchdog:** 5 s silent timeout → CrashTracker records, force-restart per backoff

### 8.7 Multi-bundle conflict resolution

When multiple bundles match the same device (Critic Q3): highest `priority` wins; tie-break by lexicographic bundle id. The supervisor logs `MultipleDriverMatch { winner, contenders }` for audit.

When the kernel has already bound a class driver (e.g., `usbhid` for a USB HID) and a bundle claims the device: the bundle MUST declare `interface_class = N` and target a specific interface number. The supervisor calls `/sys/bus/usb/drivers/usbhid/unbind` only for that interface, leaving other interfaces of the same device alone. This avoids the kernel-oops hub-corner-case the Critic flagged.

### 8.8 Revocation

A bundle's signing root may be revoked via `/etc/vyoma/trust/revoked.toml` (chained from system update). On the next bundle spawn, verification fails and the bundle refuses to load. Already-running instances are killed on the next epoch boundary after a revocation message arrives.

---

## 9. Input Device Pipeline

### 9.1 evdev → HID subsystem → InputDispatcher

```
/dev/input/event* (epoll)
       │
       ▼
HidSubsystem::drain_one  (native Rust; no WASM)
       │ ParsedInput
       ▼
InputDispatcher::dispatch
       │
       ├─ if grabbed → grabber app's on-input (bypass focus)
       ├─ if accessibility broadcaster → broadcast on vyoma:input/accessibility
       └─ else → focused app's on-input
                  ↓
         R5 QoS lane (UserInteractive)
```

End-to-end latency budget: 6 µs p50 / 25 µs p99 (Critic accepts this is identical to native Rust evdev paths).

### 9.2 `HidSubsystem` (full)

```rust
// supervisor/src/drivers/hid.rs

use std::sync::Arc;
use std::os::unix::io::RawFd;
use dashmap::DashMap;
use crate::drivers::registry::{DeviceRegistry, DeviceId, DeviceClass,
                                DeviceInfo, DevNode, DevNodeKind};
use crate::identity::BundleId;

pub struct HidSubsystem {
    pub devices: Arc<DashMap<DeviceId, OpenEvdev>>,
    pub epoll_fd: RawFd,
    pub registry: Arc<DeviceRegistry>,
    pub ipc_router: Arc<crate::ipc::IpcRouter>,
    pub input_dispatcher: Arc<InputDispatcher>,
    pub trust_registry: Arc<TrustedHidRegistry>,
}

pub struct OpenEvdev {
    pub fd: RawFd,
    pub device_id: DeviceId,
    pub class: DeviceClass,
    pub abs_info: Option<AbsInfo>,
    pub report_descriptor: Option<Vec<u8>>,
    pub grabbed_by: Option<BundleId>,
    pub trusted: bool,                  // C7 §7.3 BadUSB defense
}

#[derive(Debug, Clone)]
pub struct AbsInfo {
    pub x_min: i32, pub x_max: i32,
    pub y_min: i32, pub y_max: i32,
    pub pressure_max: i32,
    pub resolution_dpi: u32,
}

/// Persistent set of trusted (vid, pid, serial) tuples; from Round 60 Keychain.
pub struct TrustedHidRegistry {
    pub trusted: dashmap::DashSet<(u16, u16, String)>,
}

impl HidSubsystem {
    pub fn start(
        registry: Arc<DeviceRegistry>,
        ipc_router: Arc<crate::ipc::IpcRouter>,
    ) -> Result<Arc<Self>, DeviceError> {
        let epoll_fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if epoll_fd < 0 { return Err(DeviceError::EpollCreate); }
        let dispatcher = Arc::new(InputDispatcher::new());
        let trust = Arc::new(TrustedHidRegistry { trusted: dashmap::DashSet::new() });
        let me = Arc::new(Self {
            devices: Arc::new(DashMap::new()),
            epoll_fd, registry, ipc_router,
            input_dispatcher: dispatcher,
            trust_registry: trust,
        });
        let cloned = me.clone();
        std::thread::Builder::new()
            .name("vyoma-evdev".into())
            .spawn(move || cloned.run_event_loop())
            .map_err(|_| DeviceError::ThreadSpawn)?;
        Ok(me)
    }

    pub fn open_device(&self, info: &DeviceInfo) -> Result<(), DeviceError> {
        let evdev_node = info.dev_nodes.iter()
            .find(|n| matches!(n.kind, DevNodeKind::Evdev))
            .ok_or(DeviceError::NoEvdevNode)?;
        let path = std::ffi::CString::new(
            evdev_node.path.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| DeviceError::InvalidPath)?;
        let fd = unsafe {
            libc::open(path.as_ptr(),
                       libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        };
        if fd < 0 { return Err(DeviceError::EvdevOpen); }
        let mut ev = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLET) as u32,
            u64: info.id.0,
        };
        unsafe {
            libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut ev);
        }
        let trusted = self.trust_registry.trusted.contains(&(
            info.vendor_id, info.product_id,
            info.serial.clone().unwrap_or_default(),
        ));
        self.devices.insert(info.id, OpenEvdev {
            fd,
            device_id: info.id,
            class: info.class,
            abs_info: None,
            report_descriptor: None,
            grabbed_by: None,
            trusted,
        });
        Ok(())
    }

    fn run_event_loop(self: Arc<Self>) {
        let mut events: [libc::epoll_event; 64] = unsafe { std::mem::zeroed() };
        let mut rbuf = [0u8; std::mem::size_of::<libc::input_event>() * 32];
        loop {
            let n = unsafe {
                libc::epoll_wait(self.epoll_fd, events.as_mut_ptr(), 64, -1)
            };
            for i in 0..n as usize {
                let did = DeviceId(events[i].u64);
                if let Some(dev) = self.devices.get(&did) {
                    self.drain_one(&dev, &mut rbuf);
                }
            }
        }
    }

    fn drain_one(&self, dev: &OpenEvdev, buf: &mut [u8]) {
        loop {
            let n = unsafe {
                libc::read(dev.fd, buf.as_mut_ptr() as *mut _, buf.len())
            };
            if n <= 0 { break; }
            let count = n as usize / std::mem::size_of::<libc::input_event>();
            let raw: &[libc::input_event] = unsafe {
                std::slice::from_raw_parts(buf.as_ptr() as *const _, count)
            };
            for e in raw {
                // C7 §7.3 BadUSB defense for HID keyboards.
                if !dev.trusted && dev.class == DeviceClass::Keyboard {
                    self.surface_trust_prompt(dev.device_id);
                    continue;
                }
                if let Some(parsed) = self.parse_event(dev, e) {
                    self.input_dispatcher.dispatch(
                        dev.device_id, dev.class, parsed);
                }
            }
        }
    }

    fn parse_event(&self, dev: &OpenEvdev, e: &libc::input_event) -> Option<ParsedInput> {
        match (e.type_, e.code) {
            (libc::EV_KEY, _) => Some(ParsedInput::Key { code: e.code,
                                                          pressed: e.value != 0 }),
            (libc::EV_REL, _) => Some(ParsedInput::Rel { axis: e.code,
                                                          delta: e.value }),
            (libc::EV_ABS, _) => Some(ParsedInput::Abs { axis: e.code,
                                                          value: e.value,
                                                          info: dev.abs_info.clone() }),
            (libc::EV_SYN, _) => Some(ParsedInput::Sync),
            _ => None,
        }
    }

    fn surface_trust_prompt(&self, _device: DeviceId) {
        // R3 broadcast to system UI; UI shows "press Y to trust" prompt.
    }
}

#[derive(Debug, Clone)]
pub enum ParsedInput {
    Key { code: u16, pressed: bool },
    Rel { axis: u16, delta: i32 },
    Abs { axis: u16, value: i32, info: Option<AbsInfo> },
    Sync,
}

pub struct InputDispatcher {
    pub inner: parking_lot::RwLock<DispatcherState>,
}

#[derive(Default)]
pub struct DispatcherState {
    pub focused: Option<BundleId>,
    pub grabbers: dashmap::DashMap<DeviceId, BundleId>,
    pub accessibility_subscribers: Vec<BundleId>,
}

impl InputDispatcher {
    pub fn new() -> Self {
        Self { inner: parking_lot::RwLock::new(DispatcherState::default()) }
    }

    pub fn dispatch(&self, source: DeviceId, class: DeviceClass, ev: ParsedInput) {
        let state = self.inner.read();
        if let Some(g) = state.grabbers.get(&source) {
            // grabber bypasses focus
            self.deliver(&g.clone(), source, class, &ev);
        } else if let Some(f) = &state.focused {
            self.deliver(f, source, class, &ev);
        }
        // Accessibility broadcasters always receive a copy.
        for s in &state.accessibility_subscribers {
            self.deliver(s, source, class, &ev);
        }
    }

    fn deliver(&self, _bundle: &BundleId, _source: DeviceId,
               _class: DeviceClass, _ev: &ParsedInput) {
        // R5 dispatch: enqueue an OnInput job to the bundle's UI lane.
    }
}
```

---

## 10. Sensor APIs

### 10.1 `SensorSubsystem`

```rust
// supervisor/src/drivers/sensor.rs

use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::identity::BundleId;
use crate::drivers::registry::{DeviceRegistry, DeviceId, DeviceClass};

pub struct SensorSubsystem {
    pub registry: Arc<DeviceRegistry>,
    pub ipc_router: Arc<crate::ipc::IpcRouter>,
    pub subscribers: dashmap::DashMap<(BundleId, DeviceId), SensorSub>,
    pub poll_thread: parking_lot::Mutex<Option<std::thread::JoinHandle<()>>>,
    pub high_rate_grants: dashmap::DashSet<BundleId>,
}

#[derive(Debug, Clone)]
pub struct SensorSub {
    pub min_interval: Duration,
    pub last_emit: Instant,
    pub class: DeviceClass,
}

impl SensorSubsystem {
    pub fn start(
        registry: Arc<DeviceRegistry>,
        ipc_router: Arc<crate::ipc::IpcRouter>,
    ) -> Result<Arc<Self>, DeviceError> {
        let me = Arc::new(Self {
            registry, ipc_router,
            subscribers: dashmap::DashMap::new(),
            poll_thread: parking_lot::Mutex::new(None),
            high_rate_grants: dashmap::DashSet::new(),
        });
        me.scan_sysfs_sensors()?;
        me.start_poll_thread()?;
        Ok(me)
    }

    fn scan_sysfs_sensors(&self) -> Result<(), DeviceError> {
        // Walk /sys/class/thermal/* (temperature),
        //      /sys/class/power_supply/* (battery),
        //      /sys/bus/iio/devices/* (accel/gyro/magneto on mobile/IoT).
        Ok(())
    }

    fn start_poll_thread(self: &Arc<Self>) -> Result<(), DeviceError> {
        let me = self.clone();
        let h = std::thread::Builder::new()
            .name("vyoma-sensor".into())
            .spawn(move || me.poll_loop())
            .map_err(|_| DeviceError::ThreadSpawn)?;
        *self.poll_thread.lock() = Some(h);
        Ok(())
    }

    fn poll_loop(self: Arc<Self>) {
        // SCHED_BATCH, nice +15 — sensors are background-class.
        crate::scheduler::sysctl::set_nice_self(15).ok();
        loop {
            let now = Instant::now();
            let mut next_due = Duration::from_secs(60);
            for mut entry in self.subscribers.iter_mut() {
                let sub = entry.value_mut();
                let elapsed = now.duration_since(sub.last_emit);
                if elapsed >= sub.min_interval {
                    let sample = self.read_sensor(entry.key().1, sub.class);
                    sub.last_emit = now;
                    self.publish_sample(entry.key().0.clone(), sample);
                    next_due = next_due.min(sub.min_interval);
                } else {
                    next_due = next_due.min(sub.min_interval - elapsed);
                }
            }
            std::thread::sleep(next_due.max(Duration::from_millis(1)));
        }
    }

    fn read_sensor(&self, _device: DeviceId, _class: DeviceClass) -> SensorSample {
        SensorSample {
            device_id: DeviceId(0), timestamp_ns: 0,
            data: SensorData::AmbientLight { lux: 0.0 },
        }
    }

    fn publish_sample(&self, _bundle: BundleId, _sample: SensorSample) {
        // R3 IPC: targeted send to the subscriber's bundle.
    }

    pub fn subscribe(
        &self, bundle: BundleId, device: DeviceId, min_interval: Duration,
    ) -> Result<(), DeviceError> {
        let snapshot = self.registry.snapshot();
        let info = snapshot.by_id.get(&device).ok_or(DeviceError::NoSuchDevice)?;
        let floor = if self.high_rate_grants.contains(&bundle) {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(50)
        };
        let interval = min_interval.max(floor);
        self.subscribers.insert((bundle, device), SensorSub {
            min_interval: interval, last_emit: Instant::now(),
            class: info.class,
        });
        Ok(())
    }

    pub fn grant_high_rate(&self, bundle: &BundleId) {
        self.high_rate_grants.insert(bundle.clone());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SensorSample {
    pub device_id: DeviceId,
    pub timestamp_ns: u64,
    pub data: SensorData,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SensorData {
    Accelerometer { x: f32, y: f32, z: f32 },
    Gyroscope { x: f32, y: f32, z: f32 },
    Magnetometer { x: f32, y: f32, z: f32 },
    AmbientLight { lux: f32 },
    Proximity { distance_mm: f32 },
    Temperature { celsius: f32 },
    Barometer { kpa: f32 },
    Gps { lat: f64, lon: f64, alt_m: f32, accuracy_m: f32 },
    Battery { percent: u8, charging: bool, time_to_full_s: Option<u32> },
}
```

### 10.2 Privacy gating

Location (`Gps`) and high-rate motion (accel/gyro at < 50 ms interval) require the runtime prompt of §6. Persistence is via Round 60 Keychain.

---

## 11. Implementation Files

| File | LOC | Purpose |
|------|-----|---------|
| `supervisor/src/drivers/mod.rs` | ~310 | `DeviceManager`, boot, coldplug entry, event dispatch |
| `supervisor/src/drivers/registry.rs` | ~470 | `DeviceId`, `DeviceInfo`, `AccessTier`, `DeviceRegistry`, snapshot |
| `supervisor/src/drivers/udev.rs` | ~430 | NETLINK_KOBJECT_UEVENT, `UEvent::parse`, coldplug walk |
| `supervisor/src/drivers/hid.rs` | ~480 | evdev pipeline, `HidSubsystem`, `InputDispatcher`, BadUSB trust prompt |
| `supervisor/src/drivers/display_dev.rs` | ~440 | DRM enumeration, `MonitorInfo`, hot-plug state machine, compositor control trait |
| `supervisor/src/drivers/audio_dev.rs` | ~490 | ALSA scan, lock-free SPSC ring, `ArcSwap<AudioOutputBinding>`, RCU unmap |
| `supervisor/src/drivers/audio_render.rs` | ~290 | SCHED_DEADLINE thread body, mix kernel, PCM write |
| `supervisor/src/drivers/camera.rs` | ~370 | V4L2 enumeration, `CameraSubsystem`, Shared/Exclusive arbitration |
| `supervisor/src/drivers/usb.rs` | ~460 | usbfs, topology, raw transfers for driver bundles |
| `supervisor/src/drivers/usb_storage_gate.rs` | ~390 | `DeviceOffer` flow, magic-byte fs probe, mount allowlist |
| `supervisor/src/drivers/sensor.rs` | ~360 | sysfs/IIO scan, sample emission, consent gating |
| `supervisor/src/drivers/bundle.rs` | ~460 | `BundleRegistry`, ed25519 signature verification, lifecycle, crash backoff |
| `supervisor/src/drivers/policy.rs` | ~290 | `DevicePolicy`, runtime prompt, persistence to Keychain, revocation |
| `supervisor/src/drivers/claim.rs` | ~210 | `ClaimTable` (Exclusive vs Shared/Multiplexed), atomic exclusivity |
| `wit/vyoma-device.wit` | ~540 | Eight WIT interfaces, two worlds (`app`, `driver-bundle`) |

**Total new:** ~5,860 LOC Rust + ~540 lines WIT across 14 Rust files + 1 WIT package. Every file ≤ 500 lines.

**Modified files (estimated):**
- `supervisor/src/manifest.rs` (+200 LOC): `DriverSection`, `DeviceCapabilities`, `DriverCapability`, validation
- `supervisor/src/lifecycle.rs` (+90 LOC): `BootPhase::DevicesReady`, `InitialResources::device_handles`, `launch_with_init`
- `supervisor/src/ipc/envelope.rs` (+40 LOC): `IpcAddr::Device(class, device_id)`, `IpcAddr::DriverBundle(bundle_id)`
- `supervisor/src/scheduler/sysctl.rs` (+30 LOC): SCHED_DEADLINE setter on the audio render thread, nice setter for sensors
- `supervisor/src/profile/profiles/*.toml` (+50 LOC total): `[devices]` expected-class lists per profile

---

## 12. WIT Surface (`vyoma:device@0.1.0`)

```wit
// wit/vyoma-device.wit
package vyoma:device@0.1.0;

interface types {
    record device-id { lo: u32, hi: u32 }

    enum device-class {
        keyboard, mouse, touchpad, touchscreen, stylus, game-controller,
        display, projector,
        audio-output, audio-input, midi,
        camera, scanner, printer,
        usb-storage, sd-card, optical-drive,
        usb-hub, bluetooth-adapter, network-adapter, modem,
        accelerometer, gyroscope, magnetometer, ambient-light, proximity,
        temperature, gps, barometer,
        generic,
    }

    enum access-tier { exclusive, shared, multiplexed }
    enum device-state {
        discovered, initialising, available, claimed, in-use,
        disabled, lost, needs-consent,
    }

    record device-info {
        id: device-id,
        class: device-class,
        name: string,
        vendor-id: u16,
        product-id: u16,
        serial: option<string>,
        state: device-state,
        access-tier: access-tier,
        power-battery-percent: option<u8>,
        scale-factor: option<f32>,
    }

    variant device-event {
        added(device-info),
        removed(tuple<device-id, string>),
        changed(device-info),
        lost(tuple<device-id, lost-reason>),
        revoked(tuple<device-id, device-class>),
    }

    enum lost-reason { surprise-remove, driver-crash, power-loss, timeout, eject }

    enum device-error {
        no-such-device, denied, revoked, busy,
        io-error, unsupported, needs-consent, unknown-fs-type,
        unsafe-fs-type,
    }
}

interface registry {
    use types.{device-id, device-class, device-info, device-error};

    list-devices: func(class: option<device-class>) -> list<device-info>;
    get-device: func(id: device-id) -> result<device-info, device-error>;
}

interface events {
    use types.{device-event};
    subscribe: func() -> result<_, string>;
    unsubscribe: func();
}

interface input {
    use types.{device-id, device-error};

    record key-event { code: u16, pressed: bool, modifiers: u8 }
    record mouse-event { dx: s32, dy: s32, buttons: u8, scroll: s8 }
    record touch-event { id: u16, x: u32, y: u32, pressure: u16, phase: touch-phase }
    enum touch-phase { began, moved, ended, cancelled }

    /// Request exclusive grab of `device-id` (game with controller).
    grab: func(device-id) -> result<grab-handle, device-error>;
    resource grab-handle {
        release: func();
    }
}

interface audio {
    use types.{device-id, device-error};

    enum format { s16-le, s24-le, s32-le, f32-le }
    record stream-config {
        device-id: device-id,
        format: format,
        sample-rate: u32,
        channels: u8,
        buffer-frames: u32,
    }
    enum mode { exclusive, shared }

    /// Open a playback stream. Sample buffers are written to a SharedBuffer
    /// ring whose handle is returned to the app.
    open-output: func(cfg: stream-config) -> result<stream, device-error>;
    /// Open a capture stream. Mic consent enforced at call time.
    open-input: func(cfg: stream-config, mode: mode)
        -> result<stream, device-error>;

    resource stream {
        ring-handle: func() -> u64;  // SharedBuffer id
        set-volume: func(linear: f32);
        mute: func();
        unmute: func();
        close: func();
    }

    /// Switch default output device (privileged: System UI only).
    switch-output: func(target: device-id) -> result<_, device-error>;
}

interface usb {
    use types.{device-id, device-error};

    record interface-info { number: u8, alternate: u8, class: u8, subclass: u8 }
    record endpoint-info { address: u8, max-packet: u16, kind: endpoint-kind }
    enum endpoint-kind { control, isochronous, bulk, interrupt }

    /// Claim a specific interface of a USB device. Driver-bundle-only.
    claim-interface: func(device-id, interface-number: u8)
        -> result<usb-handle, device-error>;
    resource usb-handle {
        bulk-write: func(endpoint: u8, data: list<u8>, timeout-ms: u32)
            -> result<u32, device-error>;
        bulk-read: func(endpoint: u8, max-len: u32, timeout-ms: u32)
            -> result<list<u8>, device-error>;
        control-transfer:
            func(request-type: u8, request: u8, value: u16, index: u16,
                 data: list<u8>, timeout-ms: u32) -> result<list<u8>, device-error>;
        release: func();
    }
}

interface sensor {
    use types.{device-id, device-error};

    variant sample-data {
        accelerometer(tuple<f32, f32, f32>),
        gyroscope(tuple<f32, f32, f32>),
        magnetometer(tuple<f32, f32, f32>),
        ambient-light(f32),
        proximity(f32),
        temperature(f32),
        barometer(f32),
        gps(gps-fix),
        battery(battery-info),
    }
    record gps-fix { lat: f64, lon: f64, alt-m: f32, accuracy-m: f32 }
    record battery-info { percent: u8, charging: bool, time-to-full-s: option<u32> }
    record sample { device-id: device-id, timestamp-ns: u64, data: sample-data }

    /// Floor: 50 ms for normal apps; 1 ms if granted high-rate capability.
    subscribe: func(device-id, min-interval-ms: u32) -> result<_, device-error>;
    unsubscribe: func(device-id);
}

interface device-lifecycle {
    use types.{device-event, device-id, device-class};
    on-device-event: func(ev: device-event);
    on-device-revoked: func(device-id, class: device-class);
}

world app {
    import registry;
    import events;
    import input;
    import audio;
    import sensor;
    // NOTE: usb is NOT in the default app world.
    export device-lifecycle;
}

world driver-bundle {
    import registry;
    import events;
    import input;
    import audio;
    import usb;
    import sensor;
    export device-lifecycle;
}
```

---

## 13. Boot Sequence

R1's `BootPhase` enum is extended:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BootPhase {
    KernelHandoff,
    FsReady,
    IpcReady,
    DevicesReady,        // NEW
    PolicyLoaded,        // NEW
    AppsLaunching,
    AppsLaunched,
}
```

Order:

1. PID 1 starts; signal handlers, logging
2. Mount `/proc`, `/sys`, `/dev`, `/data` → `FsReady` (~30 ms)
3. Start sharded IPC router → `IpcReady` (~20 ms)
4. `DeviceManager::boot`:
   a. Open NETLINK socket (no reader thread yet)
   b. Bring up Tier-A subsystems (HID, Display, Audio, Camera, USB-gate, Sensor)
   c. Run `coldplug_walk()` over `/sys`; synthesises Add events; populates registry
   d. Start netlink reader thread → `DevicesReady` (~150 ms)
5. Load `DevicePolicy` from `/data/registry/policy.toml` and Keychain → `PolicyLoaded` (~5 ms)
6. Spawn registered driver bundles for already-present devices (parallel, capped at 4 concurrent)
7. User apps launch → `AppsLaunching → AppsLaunched`

Total driver-subsystem contribution to boot: ~200 ms, well within the < 5 s budget.

---

## 14. Hot-Plug Semantics

### 14.1 Graceful disconnect (user clicks Eject)

1. UI app sends `vyoma:device/registry.disconnect(device_id)` to supervisor
2. Supervisor marks device `state = Lost`
3. Subscribed apps receive `on-device-event(Lost { id, reason: Eject })`
4. Apps have **2 s** to release handles
5. After timeout, supervisor force-revokes any remaining handles
6. Unmount (storage), then `/sys/bus/usb/drivers/usb/unbind`
7. Physical device safe to remove

### 14.2 Surprise removal (cable yanked)

1. udev `remove` event fires
2. Supervisor sends `Lost { reason: SurpriseRemove }` to all subscribers
3. All handles for the device are marked `revoked = true` immediately
4. Subsequent handle calls return `DeviceError::Revoked`
5. If a driver bundle owns it: 250 ms drain window for `on-shutdown`, then Store-drop
6. Storage: `umount2(MNT_DETACH)`; open files return EIO

### 14.3 Hot-replug

If same `(vid, pid, serial)` reappears within 5 s of removal: reuse the old `DeviceId`, broadcast `Changed { state: Available }` instead of a new `Added`. If serial absent or differs, new `DeviceId`.

### 14.4 Class-specific drain windows (Critic Q7)

| Class | Drain window |
|-------|--------------|
| HID (keyboard/mouse) | 250 ms |
| Audio | 500 ms (drain ring + reroute) |
| Camera | 500 ms |
| USB storage | 5 s (in-flight transfers; sync; unmount) |
| Generic | 250 ms |

---

## 15. Performance Budget

| Path | p50 | p99 | Notes |
|------|-----|-----|-------|
| `udev` event → registry rebuild | 50 µs | 200 µs | After 100 ms coalesce (input bypasses) |
| `udev` event → app `on-device-event` | 6 µs | 20 µs | R3 routing dominates |
| evdev read → focused-app `on-input` | 6 µs | 25 µs | Same as R5 budget |
| `vyoma:device/registry.list-devices` (40 devs) | 8 µs | 20 µs | `im::Vector` clone |
| `vyoma:device/registry.get-device` | 25 ns | 60 ns | Atomic load + HashMap lookup |
| Audio render period (per 10 ms) | 1.2 ms | 1.8 ms | Inside SCHED_DEADLINE 2 ms budget |
| Audio output device switch | 200 µs | 1 ms | ArcSwap + RCU close-after-epoch |
| Display hot-plug → app `on-display-changed` | 18 ms | 50 ms | Includes one vblank drain |
| Driver bundle spawn on hotplug | 35 ms | 110 ms | One Wasmtime instantiate |
| USB-storage offer → System UI dialog | 25 ms | 80 ms | Includes magic-byte scan |
| Permission prompt → user reply | user-bound | user-bound | Blocking; app pauses |
| Sensor `subscribe` → first sample | 50 ms | 250 ms | Hardware-dependent |
| Boot: coldplug + subsystem init | 150 ms | 250 ms | Fits in < 5 s boot |

---

## 16. Security Model Summary

| Threat | Mitigation |
|--------|------------|
| Malicious driver bundle reads other devices | `InitialResources` pre-installs *one* handle; bundle world has no other `registry.attach` capability beyond its assigned class |
| App spoofs `@supervisor:` device-revoke | R3 publisher-restricted topic; only `IpcAddr::System("device-manager")` can publish on `vyoma:device/events` |
| Forged DeviceEvent by malicious app | Publisher restriction (above) |
| Camera kept active after blur | `camera = "focused"` ⇒ focus-state change re-evaluates policy snapshot; handle marked `revoked = true` |
| Driver bundle reflashes firmware via raw USB | Bundle manifest must declare `usb_raw`; signed bundle gates this; user sees one-time install prompt |
| Extension exfiltrates EDID/serial | `DeviceInfo` fields gated by `registry` capability + tier; bundles see only the device they own |
| Bluetooth scan exposes nearby devices to background app | `bluetooth = "focused"` floor; scan disabled when unfocused |
| Sensor stream during App Nap | R5 `ActivityAssertion` absent ⇒ samples dropped, app not woken |
| BadUSB filesystem fuzzing | `UsbStorageGate` never auto-mounts; magic-byte allowlist (vfat/exfat/iso9660/udf only); user dialog before mount |
| Rubber Ducky HID injection | First keypress from untrusted keyboard freezes input until user trusts in System UI |
| Unsigned driver bundle execution | ed25519 verification at install AND at every spawn |
| Compromised driver bundle keeps state to attacker | Bundle has no filesystem; state must persist via supervisor-mediated IPC (auditable) |
| Driver-bundle revocation | `/etc/vyoma/trust/revoked.toml` chained from system update; running bundles killed on revocation arrival |

---

## 17. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| NETLINK socket EOF | `recv()` == 0 | Reopen socket; preserve registry |
| `/sys` unmounted | `coldplug_walk` empty | Panic; supervisor cannot proceed |
| ALSA card disappeared mid-stream | `snd_pcm_writei` returns `-ENODEV` | Drain stream; fire `DeviceLost`; reroute to next default output |
| Driver bundle WASM trap | R1 `CrashKind::Trap` | `CrashTracker` decides backoff or disable |
| Bundle holds device > 5 s without I/O | Watchdog | Revoke handle; device → `Available` |
| User revokes consent during use | `ArcSwap<DevicePolicy>` swap | Handle calls return `Revoked`; broadcast `on-device-revoked` |
| DRM master fd lost (VT switch) | DRM poll `EAGAIN` | Re-acquire on next focus; meanwhile compositor shows black |
| USB hub overcurrent | sysfs `port/status` `over-current` | Toast user; disable port until physical replug |
| Bluetooth scan flood (>200 ev/s) | Coalesce window exceeded | Drop oldest ephemeral; emit `BluetoothScanOverflow` |
| Sensor sample during App Nap | R5 assertion absent | Drop sample; do not wake |
| Signature verification fails | `verify_bundle` returns Err | Bundle refuses to load; `bundle-rejected` heartbeat metric |
| Mount allowlist rejection | `fs_type` not in `SAFE_FS_TYPES` | Dialog: "Filesystem type X not supported"; offer Eject |
| Compositor pause times out | 50 ms watchdog after `pause_rendering` | Force-evict CRTC scanout; visible black flash; log incident |

---

## 18. Integration with Existing Code

The current `supervisor/src/` already has stubs to integrate with rather than replace:

- `supervisor/src/hal/mod.rs` — HAL traits (GPIO/I2C/SPI/UART/ADC). `SensorSubsystem` uses these on `iot-edge` and `robotics-rt` profiles via `HalProvider::i2c()` for IIO sensors on I²C
- `supervisor/src/display/` — existing framebuffer code becomes the `MonitorBackend::FbDev` implementation. `DisplaySubsystem::start` detects whether `/dev/dri/card0` is available; falls back to `FbDev` otherwise
- `supervisor/src/mouse_input.rs`, `supervisor/src/input_keys.rs` — currently route TTY/serial input. They become *sources* feeding the new `InputDispatcher`. The full dispatcher implementation lives here; existing sources continue to function unchanged on `desktop-full` QEMU boot
- `supervisor/src/ipc.rs` — `IpcAddr` gains `Device(DeviceId)` and `DriverBundle(BundleId)` variants; R3's router supports them via the policy snapshot extension
- `supervisor/src/lifecycle.rs` — R1 `LifecycleActor` gets a `launch_with_init(InitialResources)` overload that pre-installs device handles in the WIT resource table
- `supervisor/src/manifest.rs` — adds `DriverSection`, `DeviceCapabilities`, `DriverCapability`; validates that bundles do not request filesystem/network/display
- `supervisor/src/profile/` — per-platform profile TOMLs gain a `[devices]` section listing expected classes (used for boot-time "missing required device" alerts)

---

## 19. Testing Strategy

### 19.1 Unit tests
- `udev::UEvent::parse` round-trip against captured kernel uevent buffers in `supervisor/tests/fixtures/uevents/*.bin`
- `DeviceRegistry::{upsert, remove}` under concurrent inserts
- `DeviceClass::default_access_tier` matrix
- `ClaimTable::try_claim` Exclusive vs Shared transitions
- `CrashTracker` window sliding under simulated time
- `DevicePolicy` decision cache rebuild on consent grant
- `SignatureVerifier::verify_bundle` against good/bad/tampered fixtures
- `UsbStorageGate::on_usb_storage_added` produces correct `DeviceOffer` payload
- `AudioSubsystem` epoch-RCU unmap path (synthetic deadline thread)

### 19.2 Integration tests (under QEMU)
- Boot with `-usb -device usb-mouse,vendorid=0x046d,productid=0xc52b`; assert `DeviceEvent::Added` of class `Mouse` arrives at a test app
- Hot-plug via QEMU monitor `device_add` / `device_del`; assert add/remove events
- USB storage: attach virtual block device; assert `DeviceOffer` IPC; simulate `OpenInFiles` decision; assert mount + VfsBackend visibility
- Audio: open stream, push 440 Hz sine for 100 ms; verify it lands in `/dev/snd/pcmC0D0p` via virtual ALSA loopback; verify mixer sums two concurrent streams
- Driver bundle: install fake `streamdeck-driver` matching virtual USB device; verify it gets the handle and emits the right IPC topic; verify unsigned bundle is rejected
- Permission prompt: app calls `camera.open`; verify `prompt` IPC to System UI; simulate user `Allow`; verify subsequent calls succeed without prompt

### 19.3 Fault injection
- Kill driver bundle 3 times; assert device transitions to `Disabled`
- Drop NETLINK socket; assert recovery
- Revoke camera consent during active capture; assert `Revoked` errors
- Surprise-remove USB storage with files open; assert EIO on subsequent reads
- Disconnect DRM connector while compositor is rendering; assert pause→reconfigure→resume without DRM-EBUSY
- Plug in untrusted USB keyboard; assert first keystroke is frozen and trust prompt is surfaced

### 19.4 Profile parity
- `mcu-minimal` and `iot-edge` profiles must boot with `DeviceManager` running in degraded mode (no DRM, no ALSA, only sensors + HID); WIT package exports stub implementations returning `Unsupported` for unavailable classes
- `make smoke PLATFORM=mcu-minimal` and `make smoke PLATFORM=iot-edge` validate

---

## 20. Deferred Hooks (cross-round)

The Critic flagged several adjacent areas. Round 6 specifies the *hooks* but defers the *implementation* to clearly-named later rounds:

| Concern | Hook in R6 | Target round |
|---------|------------|--------------|
| Bluetooth stack (HCI, profiles) | `DeviceClass::BluetoothAdapter` enumerated; HCI socket exposure to a privileged bundle | R54 Bluetooth Stack |
| Wi-Fi management | `DeviceClass::NetworkAdapter`; nl80211 socket exposure | R55 Wi-Fi |
| Multi-seat / fast user switching | `ClaimHolder::App { instance_id }` carries seat (deferred) | R49 / R59 |
| Power management (suspend/resume) | `PowerInfo`, `DeviceCaps::SUPPORTS_WAKEUP` | R7 Power Management |
| ACPI events (lid, power, button) | sysfs ACPI input devices enumerated as HID; `power-supply` poll thread | R7 Power Management |
| IME / CJK input | `InputDispatcher` exposes a pre-app-input hook | R34 IME |
| Accessibility devices | `accessibility_subscribers` list in `DispatcherState` | R30 Accessibility |
| Screen recording capability | `screen_recording = "ask"` AccessLevel exists; pipe to ScreenCaptureKit-equivalent | R18 Screen Capture |
| HiDPI scale factor | `MonitorInfo.scale_factor` field present | R20 HiDPI |
| Permissions UI | `DevicePolicy::Decision::PromptUser`; System UI app handles dialog | R61 Permissions |
| Keychain persistence | `policy::load_from_disk` + `Keychain::get_grant` API stub | R60 Keychain |
| Code signing chain | `SignatureVerifier` reads `/etc/vyoma/trust/` | R62 Code Signing |

---

## 21. Differences vs Linux Default

| Aspect | Linux + X11/Wayland | VyomaOS Round 6 |
|--------|----------------------|------------------|
| Kernel module loading | `modprobe` on demand | Disabled; all drivers compiled in |
| udev daemon | systemd-udevd / eudev | Supervisor owns NETLINK directly; no daemon |
| Input access | `/dev/input/event*` open by any user in `input` group | Parsed by HidSubsystem; apps receive via WIT only |
| Camera access | `/dev/video*` open by `video` group | Per-bundle, runtime-prompted, focus-respecting, revocable |
| Audio | PulseAudio / PipeWire daemon | Supervisor-internal lock-free SCHED_DEADLINE mixer |
| USB drivers | kernel only; libusb for userspace | Kernel for class drivers; signed WASM bundles for vendor-specific only |
| Display | Wayland compositor + DRM | Supervisor-internal compositor + DRM with pause/resume API |
| Hotplug events to apps | dbus/sd-bus signals | R3 IPC broadcast topic `vyoma:device/events` |
| USB storage | Auto-mount by udisks2 | Never auto-mounted; user-prompted DeviceOffer flow |
| Privacy controls | None at OS layer (browser/app-layer only) | Per-bundle, per-class, runtime-grant, revocable |
| Driver crashes | Kernel oops / restart of daemon | WASM bundle restart with exponential backoff + 3-strikes disable |

Honest costs:
- Long tail of obscure USB devices needing on-demand kernel modules is excluded
- Audio apps render via WIT callback rather than mmap'd ALSA buffer (small but nonzero overhead)
- ~6 µs userspace routing hop on input (vs ~1 µs raw evdev)

Benefits:
- Fully capability-secure, auditable device subsystem
- Per-app revocable handles, runtime consent gating
- Genuine macOS-style desktop personality

---

## Critical v1 Requirements

- Direct NETLINK_KOBJECT_UEVENT socket; no userspace udev daemon
- `/sys` coldplug walk at boot to populate registry before first frame
- `DeviceRegistry` with `ArcSwap<RegistrySnapshot>` + 16 shards
- `HidSubsystem` reads evdev; routes through `InputDispatcher`; no WASM in hot path
- `DisplaySubsystem` enumerates DRM; defines `CompositorControl` pause/resume API
- `AudioSubsystem` lock-free SPSC ring + SCHED_DEADLINE mixer + epoch-RCU unmap
- `CameraSubsystem` enumerates V4L2; supports `Exclusive`/`Shared` modes
- `UsbStorageGate` `DeviceOffer` flow; filesystem allowlist; never auto-mounts
- `SensorSubsystem` sysfs/IIO enumeration; 50 ms default floor, 1 ms grant
- `BundleRegistry` + ed25519 signature verification at install AND spawn
- `ClaimTable` Exclusive/Shared/Multiplexed enforcement with atomic claims
- `DevicePolicy` two-tier model (manifest intent + runtime grant)
- `CrashTracker` three-strikes-in-60s → device Disabled
- `PermissionPrompter` blocking system prompt for `Ask`-tier capabilities
- BadUSB defense: HID keyboard trust prompt on first untrusted keystroke
- WIT package `vyoma:device@0.1.0` with eight interfaces + two worlds
- Class-specific drain windows (250 ms HID, 500 ms audio/camera, 5 s storage)
- Hot-replug coalescing within 5 s window for stable (vid, pid, serial)

## Deferred to v2

- Bluetooth pairing UI and profile management (R54)
- Wi-Fi credential management and network selection (R55)
- ACPI event delivery for lid / power / sleep button (R7)
- Power management suspend/resume hooks across all subsystems (R7)
- IME pre-input hook integration (R34)
- Multi-seat / fast user switching (R49)
- Screen recording capability and indicator UI (R18)
- HiDPI scale-factor propagation to apps (R20)
- Network device topology enumeration UI (R51)
- Printer / scanner spool layer (deferred indefinitely)
- Driver bundle developer tooling (`vyoma driver-status`, attach debugger) — quality-of-life, ship in v2
- Persistent `device-aliases.toml` for stable id healing across hub-reconfig (Critic Q1)
- Sandboxed `blkid` worker for filesystem probing (currently inline; v2 hardens to subprocess)
- Operator UI for live device topology + claim heatmap
- Per-app per-device usage analytics (camera-minutes, mic-minutes)

## Explicitly NEVER

- A WASM driver bundle in the path of any interrupt-driven HID event
- Auto-mount of USB mass storage without user approval
- Install-time grant for camera / microphone / location / screen-recording
- Loading an unsigned WASM driver bundle
- `modprobe` or `init_module(2)` from the supervisor
- `Mutex` held across a WIT call (Round 5 rule extended here)
- `Mutex` on the SCHED_DEADLINE audio mixer hot path
- `Generic` device class as a bundle matcher target
- Direct `/dev/input/event*` exposure to any WASM app
- Direct `/dev/snd/*` exposure to any WASM app
- Two driver bundles claiming the same device class with `exclusive_class = true`
- Filesystem types outside `SAFE_FS_TYPES` (vfat/exfat/iso9660/udf) for USB mounts
- Spawning a driver bundle without first verifying its ed25519 signature
- DRM atomic commit while a connector is in `Releasing` state
- Sensor sample delivery during App Nap (R5 assertion required)
- Bypassing the BadUSB trust prompt for new HID keyboards
- Persisting consent grants outside the Round 60 Keychain (no plaintext fallback)
- Apps reading another app's audio ring buffer (each `SharedBuffer` is per-stream)

---

**End of Round 6 Final.**
