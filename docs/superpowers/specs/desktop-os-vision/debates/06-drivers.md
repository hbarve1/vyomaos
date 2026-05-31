# Round 6 Architect Proposal: Device Driver Model

**Status:** 🟡 Architect proposal — awaiting Critic review
**Date:** 2026-05-29
**Subsystem:** Device discovery, driver bundles, hot-plug, capability handles, sensor APIs
**macOS equivalent:** IOKit + DriverKit (user-space drivers) + HID Manager + USB
Family + Bluetooth Stack + CoreAudio HAL + CoreLocation/CoreMotion + SensorKit
**Integrates with:**
  - R1 — `AppIdentity`, `AppHandle`, WIT lifecycle callbacks, `BootPhase` ordering
  - R2 — `Arc<AppLimiter>`, `PSI` pressure, `SharedBuffer` for camera/audio buffers
  - R3 — `IpcEnvelope`, `IpcAddr::Driver(BundleId)`, sharded `IpcRouter`,
    `ArcSwap<PolicySnapshot>` for capability checks
  - R4 — `VfsBackend` (for USB storage), `WatcherBackend`, WASI P2 preopens
  - R5 — `QosClass`, `ActivityAssertion` (driver IO posts assertions), cgroup
    `cpu.max` per driver bundle, `SCHED_DEADLINE` for audio render thread

---

## 0. Executive Summary

VyomaOS does **not** ship its own kernel drivers. The Linux 5.10+ kernel beneath
the supervisor owns every contact with real silicon: USB transactions go through
`drivers/usb`, ALSA owns `/dev/snd/*`, evdev exposes `/dev/input/event*`, DRM/KMS
owns `/dev/dri/card0`. The supervisor's contribution is the **personality layer**:
it turns raw Linux device nodes into typed, revocable, capability-checked handles
that WASM apps can ergonomically consume — the macOS equivalent of the gap between
"the kernel knows how to talk to a USB printer" and "Pages can choose `Send to
Printer…` from a menu and the right toner-low banner pops up three months later
when the cartridge is empty."

This proposal specifies:

1. **DeviceManager** — a supervisor subsystem owning the udev netlink socket,
   parsing hotplug events into a typed `DeviceRegistry`, and emitting
   plug/unplug events through the R3 IPC fabric.
2. **DeviceClass taxonomy** — 21 distinct hardware classes with a unified
   `DeviceInfo` shape and per-class extension data.
3. **HID input pipeline** — raw `/dev/input/event*` → evdev parsing → HID report
   descriptor decoding → `InputDispatcher` (defined in R5/future input round) →
   focused WASM app's `on-input` callback.
4. **Driver bundle model** — a sandboxed `wasm32-wasip2` binary that may declare
   `driver = true` in its manifest, declares device matchers (vendor/product/class),
   and gets exclusive use of one device handle. Driver bundles publish parsed
   events back to the supervisor as ordinary `IpcEnvelope`s; the supervisor
   re-broadcasts them with a synthetic `IpcAddr::Driver(...)` source.
5. **Capability handles** — `DeviceHandle<T>` resources stored in Wasmtime
   resource tables, revocable from the supervisor side via `ArcSwap<PolicySnapshot>`,
   gated by the per-app `[capabilities.devices]` manifest section.
6. **Display, audio, USB, and sensor subsystem stubs** — surface area sketched
   with the exact crate, file path, and integration point. Detailed designs for
   the compositor, audio mixer, and camera pipeline arrive in separate rounds;
   this round defines the *interface* they all satisfy.
7. **WIT package `vyoma:device@0.1.0`** — the public WIT surface apps and driver
   bundles see. Six interfaces: `registry`, `events`, `input`, `audio`,
   `usb`, `sensor`.
8. **Hot-plug semantics** — graceful disconnect (driver gets a 250 ms drain
   window) versus surprise removal (driver killed immediately, app receives
   `DeviceLost` event).
9. **Driver crash recovery** — driver bundles are restarted with exponential
   backoff (R1 `RestartPolicy::AlwaysWithBackoff`); if a driver crashes three
   times in 60 s, the device is marked `Disabled` until the user explicitly
   re-enables it from System Preferences.

Roughly 9 new supervisor files (~3 600 LOC) + 1 WIT package + 1 new top-level
manifest section. All files stay under the 500-line ceiling. Per-event hot-path
cost (kernel event → focused app `on-input`): ~6 µs p50, ~20 µs p99 (estimate;
must be validated by R10 benchmarks).

---

## 1. Driver Model Philosophy

### 1.1 Layering

```
┌──────────────────────────────────────────────────────────────────┐
│  L4: WASM app (Pages, Mail, Safari)                              │
│      Receives parsed events via `on-input`/`on-device-event`     │
│      Holds `device-handle<T>` resources                          │
├──────────────────────────────────────────────────────────────────┤
│  L3: WASM driver bundle (optional, only for non-class devices)   │
│      Manifest declares `driver = true` + `[driver.matches]`      │
│      Receives raw byte buffers, publishes parsed events          │
│      Sandboxed: no filesystem, no network, only ONE device       │
├──────────────────────────────────────────────────────────────────┤
│  L2: Supervisor DeviceManager (Rust, supervisor/src/drivers/)    │
│      • udev netlink socket reader                                │
│      • DeviceRegistry (sharded, ArcSwap<RegistrySnapshot>)       │
│      • Built-in class drivers (HID/keyboard, HID/mouse, ALSA,    │
│        DRM, USB mass storage, BT HCI, evdev)                     │
│      • DeviceHandle<T> minting + revocation                      │
│      • Hotplug event broadcast via R3 IpcRouter                  │
├──────────────────────────────────────────────────────────────────┤
│  L1: Linux kernel (5.10+)                                        │
│      • Real device drivers (xhci_hcd, snd_hda_intel, i915, etc.) │
│      • Exposes nodes: /dev/input/event*, /dev/snd/*, /dev/fb0,   │
│        /dev/dri/card*, /dev/bus/usb/, /dev/hidraw*, /sys/class/  │
│      • udev netlink: NETLINK_KOBJECT_UEVENT broadcasts hotplug   │
└──────────────────────────────────────────────────────────────────┘
```

The contract is **strict**: the supervisor never `insmod`s, never opens
`/proc/sys/kernel/modules_disabled = 0`, never compiles a kernel module on the
fly. If a piece of hardware needs a Linux driver, that driver is built into the
kernel at image-build time (see `base/kernel.config`). This is non-negotiable for
the < 5 s boot target — module loading is one of the slowest parts of Linux init.

What VyomaOS adds, that Linux + a typical X11/Wayland stack cannot, is:

- **Typed capability handles** (not raw fds). An app holding a
  `DeviceHandle<Keyboard>` cannot use it to read `/dev/input/event5` as a
  generic event device — the WIT surface only exposes keyboard-shaped methods.
- **Per-bundle revocation**. The supervisor can drop the handle table entry for
  one app while keeping the same physical device available to another, without
  affecting Linux's view of the kernel device.
- **WASM driver bundles** for vendor peripherals (a USB-C dock with proprietary
  protocol, a MIDI controller with unusual HID descriptors, a gaming mouse with
  vendor-specific config). The bundle runs in the same Wasmtime + capability
  sandbox as user-space apps; a crash in the driver does not affect the kernel
  or other devices.
- **Coordinated power management**. The supervisor sees plug/unplug across all
  classes and can correlate "lid closed + external display gone" → suspend.
- **User-facing privacy gates**. Camera and microphone access require an
  explicit user consent flow (System Preferences > Privacy > Camera/Mic), which
  cannot exist in raw Linux because there is no concept of "this app may use the
  camera but only when it has focus and only for the next 5 minutes."

### 1.2 The "no kernel module loading" rule

The supervisor never calls `init_module(2)`, `finit_module(2)`, or
`delete_module(2)`. The kernel is built with `CONFIG_MODULES=n` for the
`desktop-full` profile. (Other profiles vary: `iot-edge` and `robotics-rt` may
allow modules for vendor drivers, configurable in
`supervisor/src/profile/profiles/<profile>.toml`.)

Implication: every device class the supervisor knows about must have a kernel
driver compiled-in by image build time. The list lives in
`base/kernel.config.fragment-desktop-full`:

```
CONFIG_USB_XHCI_HCD=y
CONFIG_USB_HID=y
CONFIG_HID_GENERIC=y
CONFIG_INPUT_EVDEV=y
CONFIG_DRM_I915=y
CONFIG_DRM_AMDGPU=y
CONFIG_SND_HDA_INTEL=y
CONFIG_SND_USB_AUDIO=y
CONFIG_BT=y
CONFIG_BT_HCIUSB=y
CONFIG_VIDEO_UVC=y
CONFIG_USB_STORAGE=y
…
```

When a user plugs in a device whose driver is *not* compiled in, the supervisor
emits a `DeviceUnsupported(class, vendor_id, product_id)` event and surfaces a
notification: *"This device is not supported by your VyomaOS install. Visit
vyomaos.dev/hardware to request support."* The supervisor never tries to
load a module on demand.

This is uncomfortable but correct. The alternative — a userspace driver loader —
is exactly the attack surface CVE-2023-XXX took advantage of in `udev` on
RHEL-clone distributions. We trade convenience for an auditable, minimal kernel.

### 1.3 The "driver bundle" escape valve

For hardware that *does* have a kernel driver but whose protocol on top of
USB-HID/USB-bulk requires vendor-specific decoding (e.g., a Stream Deck, a Wacom
tablet's pressure curve, a Razer mouse's RGB), VyomaOS offers WASM **driver
bundles**. A driver bundle is an ordinary `wasm32-wasip2` binary with a special
manifest:

```toml
# apps/streamdeck-driver/vyoma.toml
[app]
name    = "streamdeck-driver"
version = "1.0.0"
wasm    = "streamdeck-driver.wasm"

[driver]
enabled = true
matches = [
  { class = "usb", vendor_id = 0x0fd9, product_id = 0x0060 },  # Stream Deck Original
  { class = "usb", vendor_id = 0x0fd9, product_id = 0x006d },  # Stream Deck V2
]
exclusive_class = false  # see §4.4

[capabilities]
stdio = true            # for logging
device = "raw"          # see §8 — raw HID read/write to matched device
# Notably absent: filesystem, network, display, shell.
# A driver bundle has access to ONE device handle and nothing else.

[capabilities.ipc]
publish = ["streamdeck:button-press", "streamdeck:dial-rotate"]
# Other apps subscribe to these topics via the R3 broadcast facility.
```

When the supervisor's udev reader sees a hotplug event matching `vendor_id=0x0fd9
&& product_id=0x0060`, it consults its driver registry, finds
`streamdeck-driver`, and spawns it via the R1 `LifecycleActor` with a
`DeviceHandle<Usb>` already in its initial resource table. The bundle exits when
the device is unplugged (`on-device-removed` callback fires before the handle is
revoked).

This pattern means VyomaOS can ship in a slim base image but still get a thriving
ecosystem of community-written drivers, each one as auditable and sandboxed as
any other WASM app.

---

## 2. Device Registry & Discovery

### 2.1 Identity model

```rust
// supervisor/src/drivers/registry.rs

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::identity::BundleId;
use crate::ipc::IpcAddr;

/// Stable identity of a physical device, derived from the kernel's
/// notion of identity and persisted across reboots when possible.
///
/// The discriminant carries the stability guarantee:
///  - `Permanent` ids survive reboot AND device unplug+replug;
///    used for internal devices (built-in keyboard, internal display,
///    soldered audio codec).
///  - `Session` ids survive reboot but a re-plug allocates a new id;
///    used for USB peripherals identified by `(bus, port-chain,
///    vendor, product, serial?)`. If serial is present, the id is
///    stable across replug.
///  - `Ephemeral` ids are valid only for one Linux-kernel boot; used
///    for transient devices (Bluetooth LE peripherals during scan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdScope {
    Permanent,
    Session,
    Ephemeral,
}

impl DeviceId {
    pub fn scope(&self) -> IdScope {
        match self.0 >> 60 {
            0 => IdScope::Permanent,
            1 => IdScope::Session,
            _ => IdScope::Ephemeral,
        }
    }
}
```

The 64-bit id space is allocated as:

```
 63          60 59                                                   0
┌──────────────┬──────────────────────────────────────────────────────┐
│ scope (4b)   │ stable hash or monotonic counter (60b)               │
└──────────────┴──────────────────────────────────────────────────────┘
```

For `Permanent` and `Session` devices, the lower 60 bits are a BLAKE3 hash of
`(class_string, vendor_id, product_id, serial_or_port_chain)`. For
`Ephemeral` devices, it's a monotonic counter from the registry's atomic
`session_counter`.

### 2.2 `DeviceInfo`

```rust
// supervisor/src/drivers/registry.rs (continued)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub class: DeviceClass,
    pub name: String,                // human-readable, from sysfs `name`
    pub vendor_id: u16,
    pub product_id: u16,
    pub vendor_name: Option<String>, // from `/usr/share/hwdata/usb.ids` lookup
    pub product_name: Option<String>,
    pub serial: Option<String>,      // USB iSerialNumber, only if device exposes
    pub bus_path: String,            // e.g. "usb1-2.3" or "pci0000:00/0000:00:1f.3"
    pub sysfs_path: String,          // absolute path under /sys/devices/
    pub dev_nodes: Vec<DevNode>,     // populated by the per-class scanner
    pub capabilities: DeviceCaps,
    pub state: DeviceState,
    pub power: Option<PowerInfo>,
    pub claimed_by: Option<ClaimHolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevNode {
    pub path: std::path::PathBuf,    // e.g. /dev/input/event5
    pub kind: DevNodeKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DevNodeKind {
    Evdev,      // /dev/input/event*
    Hidraw,     // /dev/hidraw*
    AlsaPcm,    // /dev/snd/pcmC*D*
    AlsaCtl,    // /dev/snd/controlC*
    DrmCard,    // /dev/dri/card*
    DrmRender,  // /dev/dri/renderD*
    UsbDevice,  // /dev/bus/usb/<bus>/<dev>
    V4l2,       // /dev/video*
    Sysfs,      // /sys/.../...  (for sensors via sysfs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceState {
    Discovered,      // udev event seen but not yet usable
    Initialising,    // class-specific setup in progress
    Available,       // ready, may be claimed
    Claimed,         // exclusively held by an app or driver bundle
    InUse,           // shared-access in use (e.g., display)
    Disabled,        // user disabled, or driver crashed too many times
    Lost,            // surprise removal; pending unplug delivery
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerInfo {
    pub battery_percent: Option<u8>,     // 0..=100, if device is a power source
    pub charging: Option<bool>,
    pub wakeup_capable: bool,             // can wake the host from suspend
    pub autosuspend_enabled: bool,
    pub current_draw_ma: Option<u16>,     // USB current draw
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClaimHolder {
    App { bundle_id: BundleId, instance_id: u64 },
    Driver { bundle_id: BundleId, instance_id: u64 },
    Supervisor { subsystem: &'static str }, // e.g. "compositor", "alsa-mixer"
}
```

The `claimed_by` field is the single source of truth for exclusive access. Only
one `ClaimHolder` may exist at a time per device (for classes where exclusivity
makes sense — see §4.4). The `DeviceManager` enforces this on the `claim()`
path with a `compare_exchange` on an atomic field, after taking the appropriate
shard lock for serialization with the slow-path policy check.

### 2.3 `DeviceClass`

```rust
// supervisor/src/drivers/registry.rs (continued)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DeviceClass {
    // Input
    Keyboard      = 0x01,
    Mouse         = 0x02,
    Touchpad      = 0x03,
    Touchscreen   = 0x04,
    Stylus        = 0x05,
    GameController= 0x06,

    // Display
    Display       = 0x10,
    Projector     = 0x11,

    // Audio
    AudioOutput   = 0x20,
    AudioInput    = 0x21,
    Midi          = 0x22,

    // Imaging
    Camera        = 0x30,
    Scanner       = 0x31,
    Printer       = 0x32,

    // Storage
    UsbStorage    = 0x40,
    SdCard        = 0x41,
    OpticalDrive  = 0x42,

    // Connectivity
    UsbHub            = 0x50,
    BluetoothAdapter  = 0x51,
    NetworkAdapter    = 0x52,
    Modem             = 0x53,

    // Sensors
    Accelerometer = 0x60,
    Gyroscope     = 0x61,
    Magnetometer  = 0x62,
    AmbientLight  = 0x63,
    Proximity     = 0x64,
    Temperature   = 0x65,
    Gps           = 0x66,
    Barometer     = 0x67,

    // Catchall
    Generic       = 0xff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceCaps(pub u32);

impl DeviceCaps {
    pub const SUPPORTS_HOTPLUG:      Self = Self(1 << 0);
    pub const SUPPORTS_WAKEUP:       Self = Self(1 << 1);
    pub const SUPPORTS_AUTOSUSPEND:  Self = Self(1 << 2);
    pub const REQUIRES_USER_CONSENT: Self = Self(1 << 3);  // camera, mic
    pub const EXCLUSIVE_ACCESS_ONLY: Self = Self(1 << 4);  // input devices
    pub const SHAREABLE:             Self = Self(1 << 5);  // display
    pub const BATTERY_POWERED:       Self = Self(1 << 6);
    pub const HAS_PRESS_INDICATOR:   Self = Self(1 << 7);  // recording LED
}
```

### 2.4 `DeviceRegistry`

```rust
// supervisor/src/drivers/registry.rs (continued)

use arc_swap::ArcSwap;
use dashmap::DashMap;
use std::sync::Arc;

/// The authoritative, read-mostly device table.
///
/// Reads (capability checks, `list_devices`, enumeration) hit the
/// ArcSwap snapshot for ~20ns Arc clone. Writes (hotplug add/remove,
/// state transitions) take the per-shard write lock and rebuild the
/// snapshot under a single rebuild lock.
pub struct DeviceRegistry {
    /// Read-mostly snapshot, atomically swapped on every change.
    snapshot: ArcSwap<RegistrySnapshot>,
    /// Slow path for sysfs-derived metadata: sharded for write throughput.
    shards: [DashMap<DeviceId, Arc<DeviceInfo>>; 16],
    /// Monotonic counter for `Ephemeral` ids.
    session_counter: std::sync::atomic::AtomicU64,
    /// Rebuild lock — only one rebuild may run at a time.
    rebuild: std::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
pub struct RegistrySnapshot {
    /// Fast lookup by id.
    pub by_id: im::HashMap<DeviceId, Arc<DeviceInfo>>,
    /// Fast lookup by class.
    pub by_class: im::HashMap<DeviceClass, im::Vector<DeviceId>>,
    /// Fast lookup by sysfs path (for udev "remove" events).
    pub by_sysfs: im::HashMap<String, DeviceId>,
    pub revision: u64,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::new(Arc::new(RegistrySnapshot::default())),
            shards: std::array::from_fn(|_| DashMap::new()),
            session_counter: std::sync::atomic::AtomicU64::new(0),
            rebuild: std::sync::Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> arc_swap::Guard<Arc<RegistrySnapshot>> {
        self.snapshot.load()
    }

    pub fn upsert(&self, info: DeviceInfo) {
        let id = info.id;
        let shard = self.shard_for(id);
        self.shards[shard].insert(id, Arc::new(info));
        self.rebuild_snapshot();
    }

    pub fn remove(&self, id: DeviceId) -> Option<Arc<DeviceInfo>> {
        let shard = self.shard_for(id);
        let removed = self.shards[shard].remove(&id).map(|(_, v)| v);
        if removed.is_some() {
            self.rebuild_snapshot();
        }
        removed
    }

    fn shard_for(&self, id: DeviceId) -> usize {
        (id.0 as usize) & 0x0f
    }

    fn rebuild_snapshot(&self) {
        // Single-rebuild discipline: holding `rebuild` serializes rebuilds.
        let _guard = self.rebuild.lock().unwrap();
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
        let new = Arc::new(RegistrySnapshot {
            by_id, by_class, by_sysfs,
            revision: prev.revision + 1,
        });
        self.snapshot.store(new);
    }
}
```

Performance budget:

- `snapshot()` + lookup by id: ~25 ns (Arc clone + `im::HashMap` lookup).
- `upsert()`: ~50 µs (shard insert + snapshot rebuild over ~50 devices).
- `remove()`: ~50 µs.

The rebuild cost is dominated by the immutable `im::HashMap` reconstruction. On a
typical laptop, the steady-state device count is 15–40 (built-in plus a couple
of USB peripherals), so the per-rebuild cost stays in the tens of microseconds.
Bluetooth scan events that produce 200+ ephemeral devices need to be
*batched* — the udev reader collects events for 100 ms before triggering a
rebuild. This is implemented in `supervisor/src/drivers/udev.rs` §3.2.

### 2.5 `DeviceManager`

```rust
// supervisor/src/drivers/mod.rs

use std::sync::Arc;
use crate::ipc::{IpcRouter, IpcEnvelope, IpcAddr, IpcPriority};

pub struct DeviceManager {
    pub registry: Arc<DeviceRegistry>,
    pub udev_reader: Arc<udev::UdevReader>,
    pub bundles: Arc<bundle::BundleRegistry>,
    pub claims: Arc<ClaimTable>,
    pub policy: arc_swap::ArcSwap<policy::DevicePolicy>,
    pub ipc_router: Arc<IpcRouter>,
    pub limiter: Arc<crate::limits::AppLimiter>,
    /// Per-class subsystem handles for built-in drivers.
    pub hid: Arc<hid::HidSubsystem>,
    pub display: Arc<display_dev::DisplaySubsystem>,
    pub audio: Arc<audio_dev::AudioSubsystem>,
    pub usb: Arc<usb::UsbSubsystem>,
    pub sensor: Arc<sensor::SensorSubsystem>,
}

impl DeviceManager {
    pub fn boot(
        boot_phase: crate::lifecycle::BootPhase,
        ipc_router: Arc<IpcRouter>,
        limiter: Arc<crate::limits::AppLimiter>,
    ) -> Result<Arc<Self>, DeviceError> {
        // Boot ordering (Round 1 BootPhase):
        //  1. Wait for BootPhase::FsReady — needs /sys to be mounted.
        //  2. Wait for BootPhase::IpcReady — needs router for events.
        //  3. Initialise UdevReader; do NOT start the reader thread yet.
        //  4. Run a one-shot enumeration of /sys/class/* to populate
        //     the registry with already-present devices.
        //  5. Start subsystem threads (audio mixer, display compositor liaison,
        //     etc.). They register their initial claims here.
        //  6. Now start the UdevReader thread; new events flow.
        debug_assert!(boot_phase >= crate::lifecycle::BootPhase::IpcReady);

        let registry = Arc::new(DeviceRegistry::new());
        let claims = Arc::new(ClaimTable::new());

        let hid = hid::HidSubsystem::start(registry.clone(), ipc_router.clone())?;
        let display = display_dev::DisplaySubsystem::start(registry.clone())?;
        let audio = audio_dev::AudioSubsystem::start(registry.clone(), limiter.clone())?;
        let usb = usb::UsbSubsystem::start(registry.clone())?;
        let sensor = sensor::SensorSubsystem::start(registry.clone(), ipc_router.clone())?;

        let bundles = Arc::new(bundle::BundleRegistry::scan_installed()?);

        let policy = arc_swap::ArcSwap::new(Arc::new(policy::DevicePolicy::default()));

        let mgr = Arc::new(Self {
            registry: registry.clone(),
            udev_reader: udev::UdevReader::open()?,
            bundles, claims, policy,
            ipc_router: ipc_router.clone(),
            limiter,
            hid, display, audio, usb, sensor,
        });

        mgr.enumerate_existing()?;
        mgr.spawn_udev_thread();
        Ok(mgr)
    }

    fn enumerate_existing(&self) -> Result<(), DeviceError> {
        // Walk /sys/class/{input,sound,drm,...} and synthesise initial
        // DeviceInfo entries. This avoids the udev-replay dance that
        // breaks under different distro init sequences.
        udev::Enumerator::new().scan(&self.registry)
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

The boot-phase discipline is from R1: device manager starts only after IPC is
ready, so plug events have somewhere to go. The pre-enumeration via sysfs is a
deliberate choice over relying on udev's "trigger" mechanism, because udev
trigger in containers and minimal initramfs setups is unreliable.

---

## 3. udev Netlink Integration

### 3.1 Why netlink, not libudev

The supervisor links against `rust-udev` *for parsing only*; the netlink socket
is opened directly. Reasons:

- libudev's daemon (`systemd-udevd`) is the runtime owner on a stock Linux
  system. VyomaOS does not run systemd; we are PID 1. We must own the
  `NETLINK_KOBJECT_UEVENT` socket ourselves or events go nowhere.
- `udevadm trigger` after sysfs is mounted *re*-emits the kernel's KOBJECT_ADD
  events for all already-present devices, but this is wasteful. We scan
  `/sys/class/*` directly in `enumerate_existing()` and only use the netlink
  socket for *deltas*.

### 3.2 `UdevReader`

```rust
// supervisor/src/drivers/udev.rs

use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

const NETLINK_KOBJECT_UEVENT: i32 = 15;
const UDEV_MONITOR_GROUP_KERNEL: u32 = 1;

pub struct UdevReader {
    fd: RawFd,
    coalesce_window: Duration,
}

impl UdevReader {
    pub fn open() -> Result<Arc<Self>, std::io::Error> {
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
        addr.nl_pid = 0; // kernel assigns
        addr.nl_groups = UDEV_MONITOR_GROUP_KERNEL;

        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as u32,
            )
        };
        if rc < 0 {
            unsafe { libc::close(fd); }
            return Err(std::io::Error::last_os_error());
        }

        // Set 1 MB receive buffer — Bluetooth scan can flood.
        let bufsize: libc::c_int = 1 << 20;
        unsafe {
            libc::setsockopt(
                fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                &bufsize as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as u32,
            );
        }

        Ok(Arc::new(Self { fd, coalesce_window: Duration::from_millis(100) }))
    }

    pub fn run(self: Arc<Self>, mgr: Arc<DeviceManager>) {
        let mut buf = vec![0u8; 65536];
        let mut pending: Vec<UEvent> = Vec::with_capacity(64);
        let mut last_flush = Instant::now();

        loop {
            // Poll with the coalesce-window timeout.
            let mut pfd = libc::pollfd {
                fd: self.fd,
                events: libc::POLLIN,
                revents: 0,
            };
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
                        pending.push(ev);
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
    pub subsystem: String,            // "input", "sound", "drm", "usb", "bluetooth"
    pub devpath: String,              // "/devices/pci0000:00/0000:00:14.0/usb1/1-2"
    pub devnode: Option<String>,      // "/dev/input/event5"
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UAction { Add, Remove, Change, Online, Offline, Bind, Unbind }

impl UEvent {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        // Kernel uevent format: NUL-separated KEY=VALUE pairs.
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
        let vendor_id = props.get("ID_VENDOR_ID").and_then(|s| u16::from_str_radix(s, 16).ok());
        let product_id = props.get("ID_MODEL_ID").and_then(|s| u16::from_str_radix(s, 16).ok());

        Some(Self { action, subsystem, devpath, devnode, vendor_id, product_id, properties: props })
    }
}
```

### 3.3 Applying batched events

```rust
// supervisor/src/drivers/mod.rs (continued)

impl DeviceManager {
    pub fn apply_uevents(&self, events: Vec<UEvent>) {
        // Group by devpath so that an "add" immediately followed by a
        // matching "remove" within the coalesce window cancels out.
        let mut by_path: HashMap<String, Vec<UEvent>> = HashMap::new();
        for e in events {
            by_path.entry(e.devpath.clone()).or_default().push(e);
        }

        let mut to_announce: Vec<DeviceEvent> = Vec::new();
        for (path, evs) in by_path {
            let final_action = evs.iter().fold(None, |_, e| Some(e.action));
            match final_action {
                Some(UAction::Add) | Some(UAction::Bind) => {
                    if let Some(info) = self.synthesize_from_uevent(&evs) {
                        let dev_event = DeviceEvent::Added(info.clone());
                        self.registry.upsert(info);
                        to_announce.push(dev_event);
                    }
                }
                Some(UAction::Remove) | Some(UAction::Unbind) => {
                    if let Some(id) = self.registry.snapshot().by_sysfs.get(&path).copied() {
                        if let Some(info) = self.registry.remove(id) {
                            self.revoke_claims_for(id);
                            to_announce.push(DeviceEvent::Removed { id, last_info: info });
                        }
                    }
                }
                Some(UAction::Change) => {
                    // Re-read sysfs, upsert with updated state.
                    if let Some(info) = self.synthesize_from_uevent(&evs) {
                        let id = info.id;
                        let prev = self.registry.snapshot().by_id.get(&id).cloned();
                        self.registry.upsert(info.clone());
                        to_announce.push(DeviceEvent::Changed { id, prev, current: info });
                    }
                }
                _ => {}
            }
        }

        for ev in to_announce {
            self.broadcast(ev);
        }
    }

    fn broadcast(&self, ev: DeviceEvent) {
        // R3 IPC broadcast: every app holding a device-events subscription
        // receives this on its `on-ipc` callback.
        let envelope = IpcEnvelope::system(
            IpcAddr::System("device-manager"),
            IpcAddr::Broadcast { topic: "vyoma:device/events" },
            IpcPriority::Critical,
            serde_cbor::to_vec(&ev).unwrap(),
        );
        self.ipc_router.dispatch(envelope);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeviceEvent {
    Added(DeviceInfo),
    Removed { id: DeviceId, last_info: Arc<DeviceInfo> },
    Changed { id: DeviceId, prev: Option<Arc<DeviceInfo>>, current: DeviceInfo },
    Lost { id: DeviceId, reason: LostReason },   // surprise removal
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LostReason { SurpriseRemove, DriverCrash, PowerLoss, Timeout }
```

---

## 4. Input Device Abstraction (HID Manager)

### 4.1 The pipeline

```
/dev/input/event*  →  evdev read loop  →  classify(device)  →  parse(report)
                                                                   ↓
                              InputDispatcher.route_event(parsed_event)
                                                                   ↓
                                  focused_app.on_input(...)  (via R5 QoS lane)
```

For keyboards and mice, the kernel already gives us cooked input — `EV_KEY` and
`EV_REL` events with normalized codes from `<linux/input-event-codes.h>`. Our
job is to:

1. Route to the *focused* WASM app (focus tracking lives in the window-manager
   subsystem, future round).
2. For grabbed devices (a game using a controller with exclusive access), bypass
   the focused-app rule.
3. For HID devices with non-standard descriptors (Wacom pen, Stream Deck), route
   raw reports to a registered driver bundle.

### 4.2 `HidSubsystem`

```rust
// supervisor/src/drivers/hid.rs

use std::sync::Arc;
use std::os::unix::io::RawFd;

pub struct HidSubsystem {
    pub devices: Arc<dashmap::DashMap<DeviceId, OpenEvdev>>,
    pub epoll_fd: RawFd,
    pub registry: Arc<DeviceRegistry>,
    pub ipc_router: Arc<crate::ipc::IpcRouter>,
    pub input_dispatcher: Arc<InputDispatcher>,
}

pub struct OpenEvdev {
    pub fd: RawFd,
    pub device_id: DeviceId,
    pub class: DeviceClass,
    pub abs_info: Option<AbsInfo>,             // for touchscreens, tablets
    pub report_descriptor: Option<Vec<u8>>,    // from /dev/hidraw* HIDIOCGRDESC
    pub grabbed_by: Option<crate::identity::BundleId>,
}

#[derive(Debug, Clone)]
pub struct AbsInfo {
    pub x_min: i32, pub x_max: i32,
    pub y_min: i32, pub y_max: i32,
    pub pressure_max: i32,
    pub resolution_dpi: u32,
}

impl HidSubsystem {
    pub fn start(
        registry: Arc<DeviceRegistry>,
        ipc_router: Arc<crate::ipc::IpcRouter>,
    ) -> Result<Arc<Self>, DeviceError> {
        let epoll_fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if epoll_fd < 0 { return Err(DeviceError::EpollCreate(std::io::Error::last_os_error())); }

        let dispatcher = Arc::new(InputDispatcher::new());
        let me = Arc::new(Self {
            devices: Arc::new(dashmap::DashMap::new()),
            epoll_fd,
            registry,
            ipc_router,
            input_dispatcher: dispatcher,
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

        let path = std::ffi::CString::new(evdev_node.path.as_os_str().as_bytes())
            .map_err(|_| DeviceError::InvalidPath)?;
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 { return Err(DeviceError::EvdevOpen(std::io::Error::last_os_error())); }

        let mut ev = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLET) as u32,
            u64: info.id.0,
        };
        unsafe { libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut ev); }

        self.devices.insert(info.id, OpenEvdev {
            fd,
            device_id: info.id,
            class: info.class,
            abs_info: read_abs_info(fd, info.class),
            report_descriptor: try_read_hidraw_descriptor(info),
            grabbed_by: None,
        });
        Ok(())
    }

    fn run_event_loop(self: Arc<Self>) {
        let mut events: [libc::epoll_event; 64] = unsafe { std::mem::zeroed() };
        let mut rbuf = [0u8; std::mem::size_of::<libc::input_event>() * 32];

        loop {
            let n = unsafe { libc::epoll_wait(self.epoll_fd, events.as_mut_ptr(), 64, -1) };
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
            let n = unsafe { libc::read(dev.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 { break; }
            let count = n as usize / std::mem::size_of::<libc::input_event>();
            let raw: &[libc::input_event] = unsafe {
                std::slice::from_raw_parts(buf.as_ptr() as *const _, count)
            };
            for e in raw {
                if let Some(parsed) = self.parse_event(dev, e) {
                    self.input_dispatcher.dispatch(dev.device_id, dev.class, parsed);
                }
            }
        }
    }

    fn parse_event(&self, dev: &OpenEvdev, e: &libc::input_event) -> Option<ParsedInput> {
        match (e.type_, e.code) {
            (libc::EV_KEY, _)  => Some(ParsedInput::Key  { code: e.code, pressed: e.value != 0 }),
            (libc::EV_REL, _)  => Some(ParsedInput::Rel  { axis: e.code, delta: e.value }),
            (libc::EV_ABS, _)  => Some(ParsedInput::Abs  { axis: e.code, value: e.value,
                                                          info: dev.abs_info.clone() }),
            (libc::EV_SYN, _)  => Some(ParsedInput::Sync),
            _ => None,
        }
    }
}

fn read_abs_info(fd: RawFd, class: DeviceClass) -> Option<AbsInfo> {
    // EVIOCGABS(ABS_X) and ABS_Y are the typical reads for touchscreens / tablets.
    // ... actual ioctl boilerplate omitted for brevity. Returns None if not supported.
    None
}

fn try_read_hidraw_descriptor(info: &DeviceInfo) -> Option<Vec<u8>> {
    // HIDIOCGRDESCSIZE + HIDIOCGRDESC on a /dev/hidraw* node.
    None
}

#[derive(Debug, Clone)]
pub enum ParsedInput {
    Key { code: u16, pressed: bool },
    Rel { axis: u16, delta: i32 },
    Abs { axis: u16, value: i32, info: Option<AbsInfo> },
    Sync,
}
```

### 4.3 `InputDispatcher` interface

This rounds defines the *interface* the input dispatcher must satisfy. The full
implementation belongs to a future round on Window Management & Input Routing,
but the shape is fixed here:

```rust
// supervisor/src/drivers/hid.rs (continued)

pub struct InputDispatcher {
    // Implementation lives in supervisor/src/input_routing/ (future round).
    pub inner: Arc<dyn InputRouter>,
}

pub trait InputRouter: Send + Sync {
    fn dispatch(&self, source: DeviceId, class: DeviceClass, ev: ParsedInput);
    fn grab(&self, source: DeviceId, by: crate::identity::BundleId)
        -> Result<GrabHandle, DeviceError>;
}

pub struct GrabHandle {
    device_id: DeviceId,
    holder: crate::identity::BundleId,
    /// RAII: drops the grab on the dispatcher when the handle drops.
    _release: Box<dyn FnOnce() + Send>,
}
```

### 4.4 Class-based exclusivity rules

```rust
// supervisor/src/drivers/registry.rs (continued)

impl DeviceClass {
    pub fn default_exclusive(self) -> bool {
        use DeviceClass::*;
        match self {
            // Input goes to ONE focused app at a time (modulo broadcast for
            // accessibility apps holding a special capability).
            Keyboard | Mouse | Touchpad | Touchscreen | Stylus | GameController => true,

            // Display is shared via compositor — supervisor "claims" it, then
            // multiplexes among apps' surfaces.
            Display | Projector => false,

            // Audio: per-app streams mixed by supervisor.
            AudioOutput => false,
            AudioInput  => false,  // but consent gated

            // Camera: ONE app at a time, consent gated.
            Camera => true,

            // USB devices: ONE driver bundle at a time.
            UsbHub | UsbStorage | SdCard | OpticalDrive => true,

            // Sensors: shared.
            Accelerometer | Gyroscope | Magnetometer | AmbientLight
            | Proximity | Temperature | Gps | Barometer => false,

            // Network/Bluetooth: shared via subsystem services.
            BluetoothAdapter | NetworkAdapter | Modem => false,

            // Printers/scanners: per-job exclusivity at the spool level.
            Printer | Scanner => false,

            // MIDI: shared via subsystem.
            Midi => false,

            Generic => true,
        }
    }
}
```

---

## 5. WASM Driver Bundles

### 5.1 Manifest schema additions

A new top-level `[driver]` section in `vyoma.toml`:

```toml
[driver]
enabled = true
matches = [
  { class = "usb", vendor_id = 0x056a, product_id = 0x0357 },  # Wacom Intuos
  { class = "usb", vendor_id = 0x056a, product_id = 0x0358 },
]
exclusive_class = true     # this bundle wants exclusive use of the device
event_topic = "wacom:input"
priority = 10              # if multiple bundles match, higher wins
```

Schema:

```rust
// supervisor/src/manifest.rs (additions)

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DriverSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub matches: Vec<DeviceMatcher>,
    #[serde(default)]
    pub exclusive_class: bool,
    pub event_topic: Option<String>,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "class", rename_all = "lowercase")]
pub enum DeviceMatcher {
    Usb {
        vendor_id: u16,
        product_id: u16,
        #[serde(default)]
        interface_class: Option<u8>,
        #[serde(default)]
        interface_subclass: Option<u8>,
    },
    Bluetooth { service_uuid: String },
    Hid { vendor_id: u16, product_id: u16, usage_page: Option<u16>, usage: Option<u16> },
    Sysfs { driver_name: String, modalias_pattern: String },
}
```

Manifest validation (extending R1 `manifest::validate`):

```rust
pub fn validate_driver(d: &DriverSection, caps: &Capabilities) -> Result<(), ManifestError> {
    if !d.enabled { return Ok(()); }
    if d.matches.is_empty() {
        return Err(ManifestError::DriverNoMatchers);
    }
    // A driver bundle MUST NOT have filesystem / network / display / shell.
    if caps.filesystem.is_some() || caps.network.is_some()
       || caps.display || caps.shell {
        return Err(ManifestError::DriverWideCapabilities);
    }
    Ok(())
}
```

### 5.2 `BundleRegistry`

```rust
// supervisor/src/drivers/bundle.rs

use std::sync::Arc;
use crate::identity::BundleId;
use crate::manifest::DriverSection;

pub struct BundleRegistry {
    /// Drivers indexed by `(class, vendor_id, product_id)` for O(1) lookup.
    pub by_match: dashmap::DashMap<MatchKey, Vec<DriverEntry>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum MatchKey {
    Usb(u16, u16),
    Bluetooth(String),
    Hid(u16, u16),
}

#[derive(Debug, Clone)]
pub struct DriverEntry {
    pub bundle_id: BundleId,
    pub priority: i32,
    pub exclusive_class: bool,
    pub event_topic: Option<String>,
}

impl BundleRegistry {
    pub fn scan_installed() -> Result<Self, DeviceError> {
        let registry = Self { by_match: dashmap::DashMap::new() };
        // Walk /data/registry/*.toml and ingest any with [driver] section.
        for entry in std::fs::read_dir("/data/registry")? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") { continue; }
            let bytes = std::fs::read(&path)?;
            let parsed: crate::manifest::Manifest =
                toml::from_slice(&bytes).map_err(DeviceError::ManifestParse)?;
            if let Some(d) = parsed.driver.as_ref() {
                if d.enabled {
                    registry.ingest(parsed.app.bundle_id.clone(), d.clone());
                }
            }
        }
        Ok(registry)
    }

    fn ingest(&self, bundle: BundleId, section: DriverSection) {
        for m in &section.matches {
            let key = match m {
                crate::manifest::DeviceMatcher::Usb { vendor_id, product_id, .. } =>
                    MatchKey::Usb(*vendor_id, *product_id),
                crate::manifest::DeviceMatcher::Bluetooth { service_uuid } =>
                    MatchKey::Bluetooth(service_uuid.clone()),
                crate::manifest::DeviceMatcher::Hid { vendor_id, product_id, .. } =>
                    MatchKey::Hid(*vendor_id, *product_id),
                crate::manifest::DeviceMatcher::Sysfs { .. } => continue, // separate index
            };
            self.by_match.entry(key).or_default().push(DriverEntry {
                bundle_id: bundle.clone(),
                priority: section.priority,
                exclusive_class: section.exclusive_class,
                event_topic: section.event_topic.clone(),
            });
        }
    }

    pub fn lookup(&self, key: &MatchKey) -> Option<DriverEntry> {
        let entries = self.by_match.get(key)?;
        entries.iter().max_by_key(|e| e.priority).cloned()
    }
}
```

### 5.3 Driver bundle lifecycle

When the supervisor sees a new `DeviceEvent::Added(info)` and finds a matching
driver bundle, it goes through this dance:

```rust
// supervisor/src/drivers/bundle.rs (continued)

pub async fn maybe_spawn_driver(
    mgr: Arc<DeviceManager>,
    lifecycle: Arc<crate::lifecycle::LifecycleActor>,
    info: Arc<DeviceInfo>,
) -> Result<(), DeviceError> {
    let key = match info.class {
        DeviceClass::UsbStorage | DeviceClass::UsbHub | DeviceClass::Camera
        | DeviceClass::Printer | DeviceClass::Scanner => {
            MatchKey::Usb(info.vendor_id, info.product_id)
        }
        DeviceClass::Keyboard | DeviceClass::Mouse | DeviceClass::Touchpad
        | DeviceClass::Touchscreen | DeviceClass::Stylus | DeviceClass::GameController => {
            MatchKey::Hid(info.vendor_id, info.product_id)
        }
        _ => return Ok(()),  // no driver-bundle slot for this class
    };

    let Some(driver) = mgr.bundles.lookup(&key) else { return Ok(()); };

    // Spawn the bundle via R1 LifecycleActor with the device handle
    // pre-installed in the initial resource table.
    let handle = mgr.mint_handle(info.id, &driver.bundle_id)?;
    let init = crate::lifecycle::InitialResources {
        device_handles: vec![(info.class, handle)],
        ..Default::default()
    };
    let _iid = lifecycle.launch_with_init(driver.bundle_id.clone(), init).await?;

    // Mark the device as Claimed by this driver bundle.
    mgr.claims.set(info.id, ClaimHolder::Driver {
        bundle_id: driver.bundle_id.clone(),
        instance_id: 0, // filled when launch_with_init returns
    });
    Ok(())
}
```

When the device unplugs:

```rust
pub fn on_device_removed(
    mgr: &DeviceManager,
    lifecycle: &crate::lifecycle::LifecycleActor,
    id: DeviceId,
    holder: Option<ClaimHolder>,
) {
    if let Some(ClaimHolder::Driver { bundle_id, instance_id }) = holder {
        // 250 ms drain window: deliver the on-device-removed callback first,
        // then send SIGTERM to the supervisor's wasmtime-host thread (Round 1
        // graceful-shutdown).
        lifecycle.shutdown_instance(
            bundle_id, instance_id,
            crate::lifecycle::ShutdownReason::DeviceLost(id),
            std::time::Duration::from_millis(250),
        );
    }
}
```

### 5.4 Driver crash backoff

```rust
// supervisor/src/drivers/bundle.rs (continued)

pub struct CrashTracker {
    /// `(bundle_id, device_id) → window of crash timestamps`
    pub windows: dashmap::DashMap<(BundleId, DeviceId), Vec<std::time::Instant>>,
}

impl CrashTracker {
    pub fn record(&self, bundle: &BundleId, dev: DeviceId) -> CrashDecision {
        let mut window = self.windows.entry((bundle.clone(), dev)).or_default();
        let now = std::time::Instant::now();
        window.retain(|t| now.duration_since(*t) < std::time::Duration::from_secs(60));
        window.push(now);
        match window.len() {
            1 => CrashDecision::Retry { delay: std::time::Duration::from_millis(100) },
            2 => CrashDecision::Retry { delay: std::time::Duration::from_millis(1_000) },
            3 => CrashDecision::Retry { delay: std::time::Duration::from_millis(10_000) },
            _ => CrashDecision::DisableUntilUserRetry,
        }
    }
}

pub enum CrashDecision {
    Retry { delay: std::time::Duration },
    DisableUntilUserRetry,
}
```

---

## 6. Display Device

### 6.1 Scope

Round 6 only *enumerates* displays; the full compositor design lives in a
future round. What this round defines:

- How `DisplaySubsystem` discovers monitors from DRM.
- The `DisplayHandle` opaque type apps see.
- The plug/unplug event flow when an HDMI/DP cable is connected.
- The interaction with the existing `supervisor/src/display/` module (which
  currently owns `/dev/fb0` for the boot framebuffer).

### 6.2 `DisplaySubsystem`

```rust
// supervisor/src/drivers/display_dev.rs

use std::sync::Arc;
use std::os::unix::io::RawFd;

pub struct DisplaySubsystem {
    pub registry: Arc<DeviceRegistry>,
    pub drm_fd: RawFd,
    pub monitors: parking_lot::RwLock<Vec<MonitorInfo>>,
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub device_id: DeviceId,
    pub connector: ConnectorKind,
    pub connector_id: u32,         // DRM connector ID
    pub edid: Option<Vec<u8>>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub width_mm: u16,
    pub height_mm: u16,
    pub current_mode: Option<DisplayMode>,
    pub available_modes: Vec<DisplayMode>,
    pub state: ConnectorState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorKind { Hdmi, DisplayPort, Vga, Dvi, Edp, LvDs, Usb, Virtual }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorState { Connected, Disconnected, Unknown }

#[derive(Debug, Clone, Copy)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,           // millihertz, e.g. 59_940 = 59.94 Hz
    pub interlaced: bool,
}

impl DisplaySubsystem {
    pub fn start(registry: Arc<DeviceRegistry>) -> Result<Arc<Self>, DeviceError> {
        // Open /dev/dri/card0 read/write (we need DRM_IOCTL_MODE_GETRESOURCES).
        let path = std::ffi::CString::new("/dev/dri/card0").unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        // Fall back to /dev/fb0 (current display.rs behaviour) if DRM unavailable.
        if fd < 0 {
            return Self::start_fb_fallback(registry);
        }
        let me = Arc::new(Self {
            registry, drm_fd: fd,
            monitors: parking_lot::RwLock::new(Vec::new()),
        });
        me.scan_connectors()?;
        Ok(me)
    }

    fn start_fb_fallback(registry: Arc<DeviceRegistry>) -> Result<Arc<Self>, DeviceError> {
        // For the dev/QEMU path: /dev/fb0 is the single virtual monitor.
        Ok(Arc::new(Self {
            registry,
            drm_fd: -1,
            monitors: parking_lot::RwLock::new(vec![MonitorInfo::synthetic_fb0()]),
        }))
    }

    fn scan_connectors(&self) -> Result<(), DeviceError> {
        // DRM_IOCTL_MODE_GETRESOURCES → list of connector IDs
        // For each: DRM_IOCTL_MODE_GETCONNECTOR → state + EDID + modes
        // ... implementation in display_dev/drm_scan.rs (separate file)
        Ok(())
    }

    pub fn on_drm_hotplug(&self, _ev: &UEvent) {
        // udev "change" event with HOTPLUG=1: rescan all connectors.
        let _ = self.scan_connectors();
    }
}

impl MonitorInfo {
    pub fn synthetic_fb0() -> Self {
        Self {
            device_id: DeviceId(0x1000_0000_0000_0001),
            connector: ConnectorKind::Virtual,
            connector_id: 0,
            edid: None,
            manufacturer: Some("QEMU".into()),
            model: Some("Virtual Framebuffer".into()),
            width_mm: 0, height_mm: 0,
            current_mode: Some(DisplayMode {
                width: 960, height: 720, refresh_mhz: 60_000, interlaced: false,
            }),
            available_modes: vec![],
            state: ConnectorState::Connected,
        }
    }
}
```

The supervisor *always* holds the DRM master fd. Apps never touch DRM. The
existing `supervisor/src/display/` module (boot framebuffer) is wrapped by a
`MonitorBackend` trait, with `DrmKmsBackend` for real hardware and
`FbDevBackend` for QEMU/dev. Future round (compositor) elaborates.

### 6.3 `DisplayHandle` for apps

```rust
// supervisor/src/drivers/display_dev.rs (continued)

/// Capability handle for an app's window on a specific monitor.
/// Apps obtain this via `vyoma:device/registry.attach-display(device_id)`.
pub struct DisplayHandle {
    pub monitor_id: DeviceId,
    pub surface_id: u64,           // compositor-assigned surface id
    pub bundle_id: BundleId,
    pub revoked: std::sync::atomic::AtomicBool,
}
```

The `surface_id` ties into the existing `apps/*/AppState.surface` field added
in `feat(display): per-window Surface buffer + blit_surface compositor primitive`
(see recent git log). When the user disconnects the monitor that hosts a
surface, the supervisor sends `on-display-changed { new_monitor_id }` and
either migrates the surface to another monitor or hides the window with a
toast: *"<App> was on <Monitor>, which is now disconnected."*

---

## 7. Audio Device (HAL equivalent)

### 7.1 Goals

Mirror CoreAudio HAL semantics on top of ALSA:

- Enumerate output and input devices via `/proc/asound/cards` and
  `/dev/snd/pcmC*D*p` / `…c` patterns.
- Sample-rate / format negotiation per stream.
- Per-app submix; supervisor performs the master mix into the ALSA PCM device.
- Hot-plug for USB audio (handled by the udev pipeline; subsystem only needs to
  re-scan on `SUBSYSTEM=sound` change events).

### 7.2 `AudioSubsystem`

```rust
// supervisor/src/drivers/audio_dev.rs

use std::sync::Arc;

pub struct AudioSubsystem {
    pub registry: Arc<DeviceRegistry>,
    pub limiter: Arc<crate::limits::AppLimiter>,
    pub output_devices: parking_lot::RwLock<Vec<AudioDevice>>,
    pub input_devices: parking_lot::RwLock<Vec<AudioDevice>>,
    pub render_thread: parking_lot::Mutex<Option<AudioRenderThread>>,
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

pub struct AudioRenderThread {
    /// SCHED_DEADLINE thread from R5: (runtime=2ms, deadline=8ms, period=10ms).
    /// Calls each app's `on-audio-render(buffer, frames)` WIT callback.
    pub join: std::thread::JoinHandle<()>,
}

impl AudioSubsystem {
    pub fn start(
        registry: Arc<DeviceRegistry>,
        limiter: Arc<crate::limits::AppLimiter>,
    ) -> Result<Arc<Self>, DeviceError> {
        let me = Arc::new(Self {
            registry, limiter,
            output_devices: parking_lot::RwLock::new(Vec::new()),
            input_devices: parking_lot::RwLock::new(Vec::new()),
            render_thread: parking_lot::Mutex::new(None),
        });
        me.enumerate_alsa()?;
        me.start_render_thread()?;
        Ok(me)
    }

    fn enumerate_alsa(&self) -> Result<(), DeviceError> {
        // Read /proc/asound/cards, then for each card scan
        // /proc/asound/cardN/pcmMp/info for outputs and
        // /proc/asound/cardN/pcmMc/info for inputs.
        // Format negotiation is via SNDRV_PCM_IOCTL_HW_PARAMS later, at open.
        // Stub.
        Ok(())
    }

    fn start_render_thread(&self) -> Result<(), DeviceError> {
        // R5 §7 audio realtime lane: spawn a thread with SCHED_DEADLINE and
        // RLIMIT_RTTIME=5ms. The thread:
        //   loop {
        //     wait_for_next_period();
        //     for each app with an active audio stream:
        //         scope.spawn(on-audio-render(scratch));
        //     mix scratch buffers into master_out;
        //     write to /dev/snd/pcmC0D0p;
        //   }
        // Stub here; full impl in supervisor/src/audio/render.rs.
        Ok(())
    }
}
```

### 7.3 Per-app audio streams

```rust
// supervisor/src/drivers/audio_dev.rs (continued)

pub struct AudioStream {
    pub stream_id: u64,
    pub bundle_id: BundleId,
    pub device_id: DeviceId,
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub channels: u8,
    pub buffer_frames: u32,
    /// Ring buffer between the app's on-audio-render callback and the
    /// supervisor's master mixer. Size = 4× buffer_frames.
    pub ring: Arc<crate::ipc::shared_buffer::SharedBuffer>,
    pub muted: std::sync::atomic::AtomicBool,
    pub volume_q15: std::sync::atomic::AtomicU16,  // 0..32768 linear
}
```

Audio streams are negotiated at open time:
- App requests `(format=F32Le, rate=48000, channels=2, frames=512)`.
- Supervisor matches against device capabilities; if exact mismatch, an SRC
  shim runs on the supervisor side and the app sees its requested format.
- The `SharedBuffer` (R2/R3) is the zero-copy lane between app and mixer.

---

## 8. USB Subsystem

### 8.1 Why a dedicated USB subsystem

Linux's `/dev/bus/usb/` is the raw `usbfs` interface. Most apps shouldn't
touch it; they use class drivers (HID, audio, storage) above. But the USB
subsystem owns:

- Whole-device enumeration during boot (matching driver bundles to plugged
  peripherals before the user even logs in).
- Transfer of `DeviceHandle<Usb>` to driver bundles via WASI custom imports.
- Topology tracking for power budgeting and "this hub is overloaded" toasts.

### 8.2 `UsbSubsystem`

```rust
// supervisor/src/drivers/usb.rs

use std::sync::Arc;

pub struct UsbSubsystem {
    pub registry: Arc<DeviceRegistry>,
    pub topology: parking_lot::RwLock<UsbTopology>,
}

#[derive(Debug, Default)]
pub struct UsbTopology {
    pub root_hubs: Vec<UsbDeviceNode>,
}

#[derive(Debug, Clone)]
pub struct UsbDeviceNode {
    pub device_id: DeviceId,
    pub bus: u8,
    pub port_chain: Vec<u8>,
    pub speed: UsbSpeed,
    pub current_draw_ma: u16,
    pub interfaces: Vec<UsbInterface>,
    pub children: Vec<UsbDeviceNode>,
}

#[derive(Debug, Clone, Copy)]
pub enum UsbSpeed { Low, Full, High, Super, SuperPlus }

#[derive(Debug, Clone)]
pub struct UsbInterface {
    pub number: u8,
    pub alternate: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<UsbEndpoint>,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbEndpoint {
    pub address: u8,             // top bit = direction (1=in)
    pub max_packet: u16,
    pub interval: u8,
    pub kind: UsbEndpointKind,
}

#[derive(Debug, Clone, Copy)]
pub enum UsbEndpointKind { Control, Isochronous, Bulk, Interrupt }
```

### 8.3 USB-storage → VFS bridge

When a USB mass storage device shows up, the supervisor:
1. Waits for the kernel to bind `usb-storage`, which creates `/dev/sdX`.
2. Mounts it under `/data/mnt/usb-<vendor>-<product>-<serial>/` (read-write
   if FAT/exFAT; read-only for unknown filesystems pending user prompt).
3. Registers the mount as a `VfsBackend` (R4) so apps with the right capability
   can read from it.
4. Issues a `DeviceEvent::Added` of class `UsbStorage` *and* a
   `vyoma:fs/events: VolumeMounted { path }` IPC broadcast.

```rust
// supervisor/src/drivers/usb.rs (continued)

pub fn on_usb_storage_attached(
    mgr: &DeviceManager,
    vfs: &crate::vfs::VfsManager,
    info: &DeviceInfo,
) -> Result<(), DeviceError> {
    let dev_node = info.dev_nodes.iter()
        .find(|n| matches!(n.kind, DevNodeKind::UsbDevice))
        .ok_or(DeviceError::NoBlockNode)?;

    // Resolve the actual block device (e.g., /dev/sda1) by walking sysfs.
    let block_dev = resolve_block_device(&info.sysfs_path)?;
    let mount_point = format!(
        "/data/mnt/usb-{:04x}-{:04x}-{}",
        info.vendor_id, info.product_id,
        info.serial.as_deref().unwrap_or("nopath"),
    );

    std::fs::create_dir_all(&mount_point)?;
    let fs_type = probe_filesystem(&block_dev)?;
    mount(&block_dev, &mount_point, fs_type, MountFlags::read_only())?;

    vfs.register_backend(crate::vfs::ExternalVolumeBackend::new(
        info.id, mount_point.into(),
    ))?;
    Ok(())
}

fn resolve_block_device(_sysfs: &str) -> Result<std::path::PathBuf, DeviceError> {
    // /sys/class/block/sda → /dev/sda; pick first partition if present.
    todo!()
}

fn probe_filesystem(_dev: &std::path::Path) -> Result<&'static str, DeviceError> {
    // Run `blkid -o value -s TYPE <dev>`, or use libblkid via FFI.
    todo!()
}

struct MountFlags;
impl MountFlags { fn read_only() -> u64 { libc::MS_RDONLY as u64 } }

fn mount(_dev: &std::path::Path, _at: &str, _fs: &'static str, _flags: u64)
    -> Result<(), DeviceError> { todo!() }
```

### 8.4 Raw USB for driver bundles

For driver bundles that need raw transfers (not class-driver mediated), we
provide `vyoma:device/usb` interface backed by `usbfs` ioctls
(`USBDEVFS_BULK`, `USBDEVFS_CONTROL`, `USBDEVFS_SUBMITURB`). The host
implementation lives in `supervisor/src/drivers/usb.rs::raw_transfer()`.

---

## 9. Sensor APIs

### 9.1 Scope

Relevant on `mobile`, `iot-edge`, `robotics-rt` profiles. On `desktop-full`,
only thermal sensors and battery (when on a laptop) are typically present.

Reading from sensors:
- Thermal: `/sys/class/thermal/thermal_zone*/temp` (millidegrees C).
- Battery: `/sys/class/power_supply/BAT0/{capacity,status,voltage_now,...}`.
- IMUs on mobile/IoT: industrial I/O (`/sys/bus/iio/devices/`) or via the
  HAL traits from `supervisor/src/hal/` (I2C/SPI).

### 9.2 `SensorSubsystem`

```rust
// supervisor/src/drivers/sensor.rs

use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct SensorSubsystem {
    pub registry: Arc<DeviceRegistry>,
    pub ipc_router: Arc<crate::ipc::IpcRouter>,
    pub subscribers: dashmap::DashMap<(BundleId, DeviceId), SensorSub>,
    pub poll_thread: parking_lot::Mutex<Option<std::thread::JoinHandle<()>>>,
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
        });
        me.scan_sysfs_sensors()?;
        me.start_poll_thread()?;
        Ok(me)
    }

    fn scan_sysfs_sensors(&self) -> Result<(), DeviceError> {
        // Walk /sys/class/thermal, /sys/class/power_supply, /sys/bus/iio/devices
        // and synthesise DeviceInfo entries.
        Ok(())
    }

    fn start_poll_thread(&self) -> Result<(), DeviceError> {
        // Single low-priority thread, SCHED_BATCH, nice +15. Iterates known
        // sensors at their declared `min_interval` and emits SensorSample
        // events to subscribed apps via R3 IPC broadcast.
        Ok(())
    }

    pub fn subscribe(
        &self,
        bundle: BundleId,
        device: DeviceId,
        min_interval: Duration,
    ) -> Result<(), DeviceError> {
        let snapshot = self.registry.snapshot();
        let info = snapshot.by_id.get(&device).ok_or(DeviceError::NoSuchDevice)?;
        // Floor at 50 ms for normal apps to prevent battery drain. Apps with
        // `[capabilities.sensor] high_rate = true` are allowed down to 1 ms.
        let floor = if self.high_rate_allowed(&bundle) {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(50)
        };
        let interval = min_interval.max(floor);
        self.subscribers.insert((bundle, device), SensorSub {
            min_interval: interval,
            last_emit: Instant::now(),
            class: info.class,
        });
        Ok(())
    }

    fn high_rate_allowed(&self, _b: &BundleId) -> bool { false }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SensorSample {
    pub device_id: DeviceId,
    pub timestamp_ns: u64,
    pub data: SensorData,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SensorData {
    Accelerometer { x: f32, y: f32, z: f32 },         // m/s^2
    Gyroscope { x: f32, y: f32, z: f32 },             // rad/s
    Magnetometer { x: f32, y: f32, z: f32 },          // microtesla
    AmbientLight { lux: f32 },
    Proximity { distance_mm: f32 },
    Temperature { celsius: f32 },
    Barometer { kpa: f32 },
    Gps { lat: f64, lon: f64, alt_m: f32, accuracy_m: f32 },
    Battery { percent: u8, charging: bool, time_to_full_s: Option<u32> },
}
```

### 9.3 Privacy gating

Location (`Gps`) is consent-gated identically to camera/microphone:
- `[capabilities.devices] location = "ask"` in manifest.
- On first `subscribe(Gps)`, supervisor surfaces a consent prompt.
- The grant is per-bundle and persisted in
  `/data/registry/<bundle>.consents.toml` with revocation timestamp.

---

## 10. Capability Granting to Apps

### 10.1 Manifest schema

```toml
# Existing capabilities continue (stdio, filesystem, network, display, shell)
# New section: [capabilities.devices]
[capabilities.devices]
keyboard      = "focused"   # "focused" | "always" | false
mouse         = "focused"
touchscreen   = "focused"
microphone    = "ask"       # "ask" prompts user; "allow" requires prior consent
camera        = "ask"
usb_storage   = "allow"     # may read/write any mounted USB volume
bluetooth     = false
location      = "ask"
sensors       = ["accelerometer", "gyroscope"]
```

```rust
// supervisr/src/manifest.rs (additions)

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct DeviceCapabilities {
    #[serde(default = "default_focused")]
    pub keyboard: AccessLevel,
    #[serde(default = "default_focused")]
    pub mouse: AccessLevel,
    #[serde(default)]
    pub touchscreen: AccessLevel,
    #[serde(default)]
    pub microphone: AccessLevel,
    #[serde(default)]
    pub camera: AccessLevel,
    #[serde(default)]
    pub usb_storage: AccessLevel,
    #[serde(default)]
    pub bluetooth: AccessLevel,
    #[serde(default)]
    pub location: AccessLevel,
    #[serde(default)]
    pub sensors: Vec<String>,    // ["accelerometer", "gyroscope", ...]
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AccessLevel {
    #[default]
    None,
    Focused,    // only while app is focused
    Allow,      // always allowed (pre-granted)
    Ask,        // prompt user on first use
}

fn default_focused() -> AccessLevel { AccessLevel::Focused }
```

### 10.2 Policy snapshot integration

```rust
// supervisor/src/drivers/policy.rs

#[derive(Debug, Clone)]
pub struct DevicePolicy {
    /// Decision-cached map: (bundle_id, device_class) → decision.
    /// Rebuilt on manifest reload + consent grant/revoke.
    pub decisions: im::HashMap<(crate::identity::BundleId, DeviceClass), Decision>,
}

#[derive(Debug, Clone, Copy)]
pub enum Decision {
    Allow,
    DenyByManifest,
    DenyNoConsent,
    AllowWhenFocused,
    PromptUser,
}

impl Default for DevicePolicy {
    fn default() -> Self { Self { decisions: im::HashMap::new() } }
}
```

The `DevicePolicy` snapshot is stored in `ArcSwap<DevicePolicy>` and consulted
on the *send* side of an IPC envelope tagged
`IpcAddr::Device(class, device_id) → IpcAddr::App(bundle_id)`. Cost: ~20 ns Arc
clone + `im::HashMap` lookup. Identical pattern to R3 `PolicySnapshot` for IPC
capability checks.

### 10.3 Handle revocation

When the user clicks "Disable camera for FaceTime" in System Preferences:
1. UI app sends `vyoma:device/registry.revoke(bundle_id, DeviceClass::Camera)`
   to the supervisor.
2. Supervisor rebuilds `DevicePolicy`; new snapshot has
   `Decision::DenyByManifest` for that pair.
3. Supervisor walks the resource table of every running instance of that
   bundle and marks any `DeviceHandle<Camera>` as `revoked = true`.
4. Next time the app calls a method on the handle, it returns
   `DeviceError::Revoked`.
5. Supervisor optionally sends `on-device-revoked { class, reason }` for the
   app to gracefully tear down its UI.

---

## 11. Implementation File Layout

| File | LOC budget | Purpose |
|------|------------|---------|
| `supervisor/src/drivers/mod.rs` | ~280 | `DeviceManager`, boot, broadcast |
| `supervisor/src/drivers/registry.rs` | ~430 | `DeviceId`, `DeviceInfo`, `DeviceRegistry`, snapshot |
| `supervisor/src/drivers/udev.rs` | ~410 | Netlink socket, `UEvent` parsing, coalescing |
| `supervisor/src/drivers/hid.rs` | ~480 | evdev pipeline, `HidSubsystem`, `InputDispatcher` trait |
| `supervisor/src/drivers/bundle.rs` | ~450 | `BundleRegistry`, driver lifecycle, crash backoff |
| `supervisor/src/drivers/display_dev.rs` | ~390 | DRM enumeration, `MonitorInfo`, `DisplayHandle` |
| `supervisor/src/drivers/audio_dev.rs` | ~470 | ALSA scan, per-app streams, render-thread hook |
| `supervisor/src/drivers/usb.rs` | ~440 | usbfs, topology, USB-storage→VFS bridge |
| `supervisor/src/drivers/sensor.rs` | ~340 | sysfs/IIO scan, sample emission, consent gating |
| `supervisor/src/drivers/policy.rs` | ~210 | `DevicePolicy`, decision cache, revocation |
| `supervisor/src/drivers/claim.rs` | ~180 | `ClaimTable` atomic exclusivity check |
| `wit/vyoma-device.wit` | ~520 lines WIT | Six WIT interfaces |

Total: **~4 600 LOC Rust + ~520 lines WIT** across 11 Rust files and 1 WIT
package. Every file under the 500-line ceiling.

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

    enum device-state { discovered, initialising, available, claimed, in-use, disabled, lost }

    record device-info {
        id: device-id,
        class: device-class,
        name: string,
        vendor-id: u16,
        product-id: u16,
        serial: option<string>,
        state: device-state,
        power-battery-percent: option<u8>,
    }

    variant device-event {
        added(device-info),
        removed(tuple<device-id, string>),    // (id, last-name)
        changed(device-info),
        lost(tuple<device-id, lost-reason>),
    }

    enum lost-reason { surprise-remove, driver-crash, power-loss, timeout }

    enum device-error {
        no-such-device,
        denied,
        revoked,
        busy,
        io-error,
        unsupported,
    }
}

interface registry {
    use types.{device-id, device-class, device-info, device-error};

    /// List devices matching `class`. Pass `none` to list all.
    list-devices: func(class: option<device-class>) -> list<device-info>;

    /// Resolve a device by stable id. Returns error if device went away.
    get-device: func(id: device-id) -> result<device-info, device-error>;
}

interface events {
    use types.{device-event};

    /// Subscribe to plug/unplug events. Events arrive via `on-device-event`
    /// in the importer's lifecycle export.
    subscribe: func() -> result<_, string>;

    /// Unsubscribe; idempotent.
    unsubscribe: func();
}

interface input {
    use types.{device-id, device-error};

    record key-event { code: u16, pressed: bool, modifiers: u8 }
    record mouse-event { dx: s32, dy: s32, buttons: u8, scroll: s8 }
    record touch-event { id: u16, x: u32, y: u32, pressure: u16, phase: touch-phase }
    enum touch-phase { began, moved, ended, cancelled }

    /// Request exclusive grab on `device-id`. Useful for games + controllers.
    /// Released when the returned handle is dropped.
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

    /// Open a playback stream. Sample buffers are delivered via the
    /// `on-audio-render` lifecycle export.
    open-output: func(cfg: stream-config) -> result<stream, device-error>;

    /// Open a capture stream. Mic consent is enforced here.
    open-input: func(cfg: stream-config) -> result<stream, device-error>;

    resource stream {
        set-volume: func(linear: f32);
        mute: func();
        unmute: func();
        close: func();
    }
}

interface usb {
    use types.{device-id, device-error};

    record interface-info { number: u8, alternate: u8, class: u8, subclass: u8 }
    record endpoint-info { address: u8, max-packet: u16, kind: endpoint-kind }
    enum endpoint-kind { control, isochronous, bulk, interrupt }

    /// Claim a specific interface of a USB device. Must have driver-bundle
    /// privilege OR `[capabilities.devices] usb_raw = true`.
    claim-interface: func(device-id, interface-number: u8) -> result<usb-handle, device-error>;

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

    /// Subscribe with a minimum interval. Real interval may be slower if
    /// platform throttles. Floor for normal apps is 50ms; `high_rate` cap is 1ms.
    subscribe: func(device-id, min-interval-ms: u32) -> result<_, device-error>;
    unsubscribe: func(device-id);
}

// Lifecycle export expected from any importer.
interface device-lifecycle {
    use types.{device-event, device-id, device-class};
    on-device-event: func(ev: device-event);
    on-device-revoked: func(device-id, class: device-class);
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

world app {
    import registry;
    import events;
    import input;
    import audio;
    import sensor;
    // USB import is NOT in the default `app` world.
    export device-lifecycle;
}
```

---

## 13. Hot-Plug Semantics

### 13.1 Graceful disconnect (cooperative)

For software-initiated disconnect (e.g., user clicks "Eject" on a USB stick):
1. UI app calls `vyoma:device/registry.disconnect(device_id)`.
2. Supervisor marks device as `Lost`.
3. Subscribed apps receive `on-device-event(Lost { id, reason: Eject })`.
4. Apps have **2 seconds** to release their handles.
5. After timeout, supervisor force-revokes any remaining handles.
6. Supervisor unmounts (if applicable), then signals the kernel via
   `/sys/bus/usb/drivers/usb/unbind`.
7. Physical device is now safe to remove.

### 13.2 Surprise removal

For unplug without warning:
1. udev `remove` event fires.
2. Supervisor reads `Lost { reason: SurpriseRemove }` to all subscribers.
3. All handles for this device are immediately marked `revoked = true`.
4. Subsequent calls to those handles return `DeviceError::Revoked`.
5. Driver bundle (if any) gets a 250 ms drain window to call `on-shutdown`,
   then is force-killed.
6. Mount points (if any) are lazily unmounted (`umount2(MNT_DETACH)`); files
   currently open in apps return EIO on next read.

### 13.3 Hot-replug semantics

If the *same* device (same vendor/product/serial) is replugged within 5
seconds of removal, the supervisor reuses the prior `DeviceId`. This means:
- Apps that haven't received `on-device-event` yet may never observe the
  transient absence.
- Apps that did receive the event get a fresh `Added` and must re-acquire any
  capability handles.

If serial is absent or the device is different, a new `DeviceId` is minted.

---

## 14. Performance Budget

Measured against the R5 boot target (< 5 s to first app) and the steady-state
60 FPS UI cadence:

| Path | p50 budget | p99 budget | Notes |
|------|------------|------------|-------|
| `udev` event → registry rebuild | 50 µs | 200 µs | After 100 ms coalesce |
| `udev` event → app `on-device-event` callback | 6 µs | 20 µs | R3 routing dominates |
| evdev read → focused-app `on-input` | 6 µs | 25 µs | Same path as R5 dispatch |
| `vyoma:device/registry.list-devices` (40 devs) | 8 µs | 20 µs | `im::Vector` clone |
| `vyoma:device/registry.get-device` | 25 ns | 60 ns | Atomic load + HashMap |
| Boot: enumerate sysfs + initial scan | 80 ms | 200 ms | Fits in < 5 s budget |
| Driver bundle spawn on hotplug | 35 ms | 110 ms | One Wasmtime instantiate |
| Audio sample render (per 10 ms period) | 1.2 ms | 1.8 ms | Inside SCHED_DEADLINE 2 ms |
| Sensor `subscribe` → first sample | 50 ms | 250 ms | Depends on hardware interval |

The hottest path is `evdev → app`, identical to R5's input dispatch numbers.
Per-event overhead added by this round is ~1 µs for the class-based exclusivity
check and the `ArcSwap<DevicePolicy>` lookup.

---

## 15. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| udev socket EOF | `recv()` returns 0 | Reopen socket; log; preserve registry |
| /sys unmounted (boot bug) | `enumerate_existing` returns empty | Panic; supervisor cannot serve devices |
| ALSA card disappeared mid-stream | `snd_pcm_writei` returns -ENODEV | Drain stream, fire DeviceLost; reroute to next default output |
| Driver bundle WASM trap | R1 `CrashKind::Trap` | Crash backoff (§5.4) |
| Driver bundle holds device for >5s without I/O | Watchdog | Revoke handle; mark device Available |
| User revokes consent during active use | Policy snapshot swap | Active handle calls return Revoked; broadcast on-device-revoked |
| DRM master fd lost (lid close while VT-switch) | DRM event poll EAGAIN | Re-acquire on next focus; meanwhile show black |
| USB hub overcurrent | sysfs `port/status` shows `over-current` | Toast user; disable port until physical replug |
| Bluetooth scan flood (>200 devices/sec) | Coalesce window exceeded | Drop oldest ephemeral entries; emit `BluetoothScanOverflow` warning |
| Sensor sample arrives during App Nap | R5 `ActivityAssertion` absent | Drop sample; do not wake app |

---

## 16. Integration Points with Existing Code

The current `supervisor/src/` already contains stubs we must integrate with,
not replace:

- `supervisor/src/hal/mod.rs` — HAL traits for GPIO/I2C/SPI/UART/ADC. The new
  `SensorSubsystem` uses these on `iot-edge` and `robotics-rt` profiles.
  Specifically: `SensorSubsystem::probe_iio_via_hal` consults
  `HalProvider::i2c()` for IIO sensors on I2C.
- `supervisor/src/display/` — existing framebuffer/compositor code becomes the
  `MonitorBackend::FbDev` implementation. The new `DisplaySubsystem::start`
  detects whether `/dev/dri/card0` is available and either uses it or falls
  back to the existing path.
- `supervisor/src/mouse_input.rs` and `supervisor/src/input_keys.rs` — these
  currently route TTY/serial input. They become *sources* feeding the new
  `InputDispatcher` trait (§4.3). The dispatcher implementation lives in a
  future round but consumes their events identically.
- `supervisor/src/ipc.rs` — `IpcAddr` gains a `Device(DeviceId)` variant for
  driver-bundle event publishing; R3's router already supports the address
  enum extension via the policy snapshot.
- `supervisor/src/lifecycle.rs` — R1 `LifecycleActor` gets a
  `launch_with_init(InitialResources)` overload that pre-installs device
  handles in the WIT resource table.
- `supervisor/src/manifest.rs` — adds `DriverSection`, `DeviceCapabilities`,
  validates that driver bundles do not request filesystem/network/display.
- `supervisor/src/profile/` — per-platform profile TOMLs gain a
  `[devices]` section listing which classes are *expected* on this platform
  (used for boot-time "missing required device" alerts).

---

## 17. Driver-Bundle Sandbox Specifics

A driver bundle is more constrained than an ordinary app:

- **No filesystem access.** Manifest validation rejects any
  `[capabilities.filesystem]` block. The bundle's WASI preopens list is empty.
- **No network.** `[capabilities.network]` rejected.
- **No display.** `[capabilities.display]` rejected.
- **No shell.** `[capabilities.shell]` rejected.
- **One device handle.** `InitialResources::device_handles` has exactly one
  entry; subsequent attempts to claim other devices return `DeviceError::Denied`.
- **Smaller memory limit.** Default `[limits] max_memory_mb = 32` (vs 256 for
  ordinary apps). Override via manifest.
- **Tighter CPU cap.** R5 `QosClass::Utility` default (cgroup cpu.max 50%).
- **No stdout `@supervisor:` shell channel.** Bundle can only emit events on
  its declared `event_topic`.
- **Crash budget.** Three crashes in 60 s → device disabled, requiring user
  re-enable in System Preferences.

This sandbox mirrors macOS DriverKit's much-stricter entitlement set: a
DriverKit extension can do far less than an ordinary app, and the user
explicitly approves it via the "Allow system extension" prompt at install.

---

## 18. Boot Sequence Detail

R1's `BootPhase` enum is extended:

```rust
// supervisor/src/lifecycle.rs (extension)

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BootPhase {
    KernelHandoff,
    FsReady,           // /data mounted
    IpcReady,          // R3 router up
    DevicesReady,      // NEW — DeviceManager has done first enumeration
    PolicyLoaded,
    AppsLaunching,
    AppsLaunched,
}
```

The full boot order:

1. PID-1 starts; sets up signal handlers, opens log.
2. `mount.rs` mounts `/proc`, `/sys`, `/dev`, `/data`. `BootPhase::FsReady`.
3. `ipc.rs` starts the sharded router. `BootPhase::IpcReady`.
4. `drivers::DeviceManager::boot()`:
   a. Open udev netlink socket (but do not start reader).
   b. Subsystem starts: `HidSubsystem`, `DisplaySubsystem`, `AudioSubsystem`,
      `UsbSubsystem`, `SensorSubsystem`.
   c. `enumerate_existing()` walks sysfs and populates the registry.
   d. Start udev reader thread.
   `BootPhase::DevicesReady`.
5. `policy.rs` loads DevicePolicy from `/data/registry/policy.toml`.
   `BootPhase::PolicyLoaded`.
6. Driver bundles in registry are matched to already-present devices and
   spawned. (This is *before* user apps so that input drivers are alive when
   apps need keyboard.)
7. User apps launch. `BootPhase::AppsLaunching` → `AppsLaunched`.

Timing budget (must fit in < 5 s overall boot):
- Steps 1–3: ~80 ms.
- Step 4: ~150 ms (sysfs scan dominates).
- Step 5: ~5 ms.
- Step 6: ~50 ms per driver bundle (parallel up to 4 at once).
- Step 7: existing.

Driver enumeration and policy load consume ~200 ms of the boot budget;
acceptable.

---

## 19. Testing Strategy

### 19.1 Unit tests

- `udev::UEvent::parse` round-trip against captured kernel uevent buffers
  (kept in `supervisor/tests/fixtures/uevents/*.bin`).
- `DeviceRegistry` upsert/remove invariants under concurrent inserts.
- `DeviceClass::default_exclusive` matrix.
- `CrashTracker` window sliding under simulated time.
- `DevicePolicy` decision cache rebuild on consent grant.

### 19.2 Integration tests (under QEMU)

- Boot with `-usb -device usb-mouse,vendorid=0x046d,productid=0xc52b` and
  verify a `DeviceEvent::Added` of class `Mouse` arrives at a test app.
- Hot-plug: send QEMU monitor commands `device_add` and `device_del` mid-test
  and verify add/remove events.
- USB-storage: attach a virtual block device, verify `/data/mnt/usb-…`
  appears and a test app can read from it.
- Audio: open a stream, push a 440 Hz sine for 100 ms, verify it lands in
  `/dev/snd/pcmC0D0p` via a virtual ALSA loopback.
- Driver bundle: install a fake `streamdeck-driver` matching a virtual USB
  device, verify it gets the handle and emits the right IPC topic.

### 19.3 Fault injection

- Kill driver bundle 3 times; assert device transitions to `Disabled`.
- Drop udev netlink socket; assert recovery.
- Revoke camera consent during active capture; assert `Revoked` errors.
- Surprise-remove USB-storage with files open; assert EIO on subsequent reads.

### 19.4 Profile parity

The `mcu-minimal` and `iot-edge` profiles must successfully boot with
`DeviceManager` disabled (the WIT package exports stub implementations that
return `Unsupported`). `make smoke PLATFORM=mcu-minimal` validates.

---

## 20. Security Model

### 20.1 Threat surface

The driver subsystem is a *high-value target*: it bridges the kernel and
WASM apps. Attack paths the design must resist:

| Threat | Mitigation |
|--------|------------|
| Malicious driver bundle reads other devices | `InitialResources` pre-installs *one* handle; `claim_interface` consults `ClaimTable` |
| App spoofs a `@supervisor:` device-revoke command | R3 §17: stdout-shell channel kills `legacy_stdout_supervisor` flag in v1 |
| Forged `DeviceEvent` published by malicious app | R3 broadcast topics are publisher-restricted: only `IpcAddr::System("device-manager")` can publish to `vyoma:device/events` |
| Camera app keeps capturing after window blur | Manifest `camera = "focused"` ⇒ R5 focus-state change re-evaluates policy snapshot, revokes |
| Driver bundle uses raw USB to reflash device firmware | Manifest declaration requires `usb_raw = true`; surfaced as a prompt at install |
| User extension exfiltrates EDID/serial | All `DeviceInfo` fields are guarded by `device` capability; bundles see only the device they're attached to |
| Bluetooth scan reveals nearby devices to background app | `bluetooth = "focused"` floor; scan disabled when unfocused |
| Sensor data continuously streams during App Nap | R5 `nap_eligible = true` apps don't receive sensor samples while parked |

### 20.2 Consent UX

Consent prompts are *blocking* — the calling app's IPC stream pauses (R3
inbox high-water mark) until the user clicks Allow/Deny. The supervisor surfaces
the prompt via the System UI app (a privileged app like Finder). The
"remember this decision" checkbox persists the grant to disk; future bundle
launches consult the persisted grant without re-prompting.

Consent revocation is immediate: System Preferences' "Privacy" pane writes a
new `DevicePolicy` and swaps the ArcSwap. All running instances see the new
policy on their next IPC send.

---

## 21. Open Questions for the Critic

These are the six-to-eight specific weaknesses where I expect (and welcome)
the Critic to attack:

### Q1. Synthesising stable `DeviceId`s from sysfs is brittle

The hash inputs `(class_string, vendor_id, product_id, serial_or_port_chain)`
are *plausible* but not specified by the kernel as stable. A device firmware
update can change a USB descriptor; a port-chain changes if a USB hub is
introduced upstream. What's the right fallback when the hash collides or
shifts unexpectedly across reboots? Should we maintain a learned alias table
in `/data/registry/device-aliases.toml`?

### Q2. The 100 ms udev coalesce window may add real latency to first input

If the user plugs in a USB keyboard at the login screen, they expect to start
typing immediately. 100 ms is imperceptible to most users, but a 100 ms
*after* the kernel finishes enumeration *plus* the cost of synthesising a
DeviceInfo *plus* spawning a driver bundle *plus* the initial scan cycle —
that's easily 500 ms total. Is the coalesce window too aggressive for
input-class devices? Should `Keyboard`/`Mouse`/`Touchscreen` events bypass
the coalesce buffer entirely?

### Q3. Driver bundle exclusivity vs class-driver coexistence

When a USB device matches *both* a kernel class driver (e.g., USB HID generic)
*and* a registered VyomaOS driver bundle (e.g., a vendor-specific Stream Deck
driver), what wins? The kernel has already bound `usbhid` and created
`/dev/input/eventN`. Do we `unbind` it from the kernel and rebind to our
driver bundle? That's the `/sys/bus/usb/drivers/usbhid/unbind` dance, which
in some hub configurations causes kernel oopses. Is there a cleaner separation
where the kernel keeps generic HID alive but our bundle claims only the
vendor-specific interface number?

### Q4. The `Generic` device class is a footgun

Anything we don't classify falls into `Generic`. If a driver bundle declares
`matches = [{ class = "generic", vendor_id = ... }]`, it can claim
*any* unclassified device — which on a wide range of hardware is most of
them at first plug. This is a real privilege-escalation path. Should we
forbid `Generic` as a matcher target and require the bundle to declare a
specific class? Or require user consent for any Generic claim?

### Q5. Audio is sketched but not designed

The `AudioSubsystem` defers all the hard parts (master mix, format
negotiation, resampling, render-thread plumbing) to a future round and
just says "Round 5 SCHED_DEADLINE handles it." That's a hand-wave. Concretely:
how does a per-app `SharedBuffer` ring buffer connect to the render thread
without a per-app lock contention point? Is the audio render thread one
thread per output device, or one global? On macOS, CoreAudio HAL is
per-device for a reason — should we be?

### Q6. The `DisplayHandle` and the existing `surface` field interact unsafely

The recent compositor work (`feat(display): per-window Surface buffer +
blit_surface compositor primitive`) added `AppState.surface`. The new
`DisplayHandle` is parallel state. What happens when:
- Display unplugged but app has an active surface mid-frame?
- App tries to attach to a `DeviceId` that's already `Lost`?
- Two apps both try to make their surface fullscreen on the same monitor?

Section 6.3 hand-waves with "supervisor migrates the surface to another
monitor" but the actual ABBA between the compositor lock and the
`ClaimTable` lock is unspecified. Lock order discipline owed.

### Q7. The 250 ms graceful-shutdown for driver bundles assumes cooperation

If a driver bundle is in an infinite WASM loop when the device unplugs, the
250 ms drain window expires and we send `SIGKILL` (well, drop the Wasmtime
Store). But what about the kernel side? Some USB transfers can be
"queued" by the time `usb-storage` is told to release — these may take
much longer than 250 ms to abort cleanly. Should the driver-bundle drain
window be *device-class-specific* (5 s for storage, 250 ms for HID)?

### Q8. The threat model for driver bundles is largely unspecified

Section 20.1 lists threats but does not specify what auditing the supervisor
performs on a driver bundle at install time. Driver bundles are uniquely
dangerous: they get raw access to a device class. Does VyomaOS require code
signing for them (R1 §10)? Does it require a manual review like Apple's
notarisation? Or do we just shrug and say "the manifest restricts them,
the user accepted the install"? Without an enforcement policy, "anyone can
publish a Stream Deck driver" becomes "anyone can publish a keyboard
keystroke logger."

---

## 22. Summary of Differences vs Linux Default

If you handed VyomaOS to a Linux veteran, the differences would be:

| Aspect | Linux + X11/Wayland default | VyomaOS |
|--------|------------------------------|---------|
| Kernel module loading | `modprobe` on demand | Disabled; everything compiled-in |
| udev daemon | systemd-udevd (or eudev) | Supervisor owns NETLINK directly |
| Input access | `xinput`/`libinput` reads `/dev/input/event*` directly | Apps receive parsed events via WIT callback; no raw `evdev` access |
| Camera access | `/dev/video*` open by any process in `video` group | Per-bundle, consent-gated, focus-respecting |
| Audio | PulseAudio / PipeWire userspace daemon | Supervisor-internal mixer; `SCHED_DEADLINE` render thread |
| USB drivers | kernel only; libusb for userspace | Kernel for class drivers; WASM bundles for vendor-specific |
| Display | Wayland compositor + DRM | Supervisor-internal compositor + DRM |
| Hotplug events to apps | dbus/sd-bus signals | R3 IPC broadcast on `vyoma:device/events` |
| Privacy controls | None at OS level | Per-bundle, per-class, consent-driven |
| Driver crashes | kernel oops / userspace daemon restart | WASM bundle restart with exponential backoff |

The honest cost of these choices:
- We exclude the long tail of obscure USB devices that need on-demand kernel
  modules.
- We force every audio app to render via WIT callback rather than mmap'd
  ALSA buffer (small but nonzero overhead).
- We add a userspace event-routing hop between kernel input and app input
  (~6 µs vs ~1 µs for raw evdev access).

The benefit is a fully capability-secure, auditable device subsystem with
per-app revocable handles and consent gating — the actual desktop personality
macOS users expect.

---

**End of Architect proposal.**
Awaiting Critic review at
`/Users/hbarve1/codes/github/hbarve1/vyomaos/docs/superpowers/specs/desktop-os-vision/debates/06-drivers-critique.md`.
