# Round 9 Final: Hardware Abstraction Layer

**Status:** Debated & synthesized (2026-05-29)
**Debate:** [Architect: 1515 lines] [Critic: 818 lines] [Final: this]

This document is the authoritative synthesis of the VyomaOS Hardware Abstraction
Layer (HAL) for version 1. It consolidates the architect's proposal, the
critic's objections, and the six critical resolutions (C1–C6) reached during
debate. Everything in this spec is in scope for the v1 implementation phase
(spec branches `046-*` through `051-*`). Items explicitly out of scope are
listed under "Deferred to v2" and "Explicitly NEVER".

The HAL exists because VyomaOS targets six radically different hardware tiers
(`mcu-minimal`, `iot-edge`, `robotics-rt`, `mobile`, `desktop-full`,
`server-headless`) with one unified application model: a `wasm32-wasip2`
binary that declares capabilities in `vyoma.toml` and runs under the
supervisor. Without a HAL, every app would need per-platform code paths.
With the HAL, an app that declares `gpio_pins = [4, 17]` runs unmodified on
a Raspberry Pi 4, an STM32 Cortex-M4, and (in a virtualized form) on a
desktop developer laptop.

The HAL is not a driver framework. Linux remains the kernel on every
non-MCU profile, and the HAL talks to the kernel through stable userspace
ABIs (`/dev/gpiochip*`, `/dev/i2c-*`, `/dev/spidev*`, `/dev/ttyAMA*`,
`/sys/class/iio:device*` for ADC where unavoidable, `/dev/dri/card*` for
display, ALSA for audio, evdev for input). On the MCU tier, the HAL talks
directly to MMIO registers because there is no kernel.

## Key Decisions

1. **Two compile-time tiers only (C6).** The supervisor builds in exactly
   one of two configurations: `wasmtime-tier` (full Wasmtime runtime, WIT
   component model, every full-Linux platform) or `wasm3-tier` (wasm3
   interpreter, raw WASM core imports, `mcu-minimal` only). Platform
   selection within a tier is runtime via `PlatformProfile` parsed from a
   TOML file, never via Cargo features. This kills the combinatorial
   explosion the critic flagged (`gpio-v2 × i2c × spi × uart × adc × pwm
   × display-drm × display-fb × alsa × evdev × 6 platforms = 24,576
   feature combinations`) and replaces it with two binaries.

2. **GPIO uses `/dev/gpiochip*` + `GPIO_V2_GET_LINE_IOCTL` exclusively (C1).**
   The legacy sysfs GPIO interface (`/sys/class/gpio/export`) was deprecated
   in Linux 4.8 and removed for new development in 5.5. VyomaOS targets
   kernel 5.10+, so we use the v2 chardev ABI from day one. The supervisor
   refuses to compile if it detects the sysfs path is reachable on a kernel
   `>= 5.5` via a `build.rs` probe.

3. **All peripheral I/O runs on per-bus dedicated OS threads (C2).** The
   IPC event loop owns the keyboard router, mouse router, IPC broker, and
   process supervisor. It must never block on a slow I²C transaction
   (a 100 kHz bus takes 90 µs just to send a single byte; a 1 ms hold-time
   sensor like a BME280 stalls the loop for an entire frame). Each bus
   (one per `/dev/gpiochipN`, `/dev/i2c-N`, `/dev/spidev<bus>.<cs>`,
   `/dev/ttyAMAN`) gets a dedicated `std::thread` with a
   `crossbeam::channel::Receiver<HalRequest>` work queue. Callers send a
   request and immediately receive a `oneshot::Receiver<HalResult>` they
   can poll without blocking.

4. **`AccessMode` replaces boolean exclusivity (C4).** The architect's
   first draft modelled peripheral ownership as a single
   `HashMap<PeripheralId, AppId>`: one app owns the resource exclusively
   or nobody does. The critic correctly pointed out that real workloads
   need finer granularity. A monitoring daemon reading the same I²C
   temperature sensor that a control loop writes a calibration register
   to is a perfectly valid pattern, as is two display apps observing GPIO
   button state. The new `AccessMode` enum offers four levels:
   `ExclusiveReadWrite`, `SharedRead`, `NotifyOnly`,
   `AddressScoped(u8)`. The registry enforces a compatibility matrix on
   `open()`.

5. **Safe state on crash is mandatory and declarative (C5).** Every GPIO
   pin declared in a manifest must specify a `safe_state`
   (`input_float`, `input_pulldown`, `input_pullup`, `output_low`,
   `output_high`, `hi_z`). When an app crashes, exits, is killed, or is
   restarted, the supervisor's `on_instance_gone` lifecycle hook calls
   `GpioPin::apply_safe_state()` on every pin the app held before the
   handle is released. This prevents a relay driving a motor from
   sticking in the "on" state when the controlling app segfaults.
   Default for any pin that omits `safe_state` is `InputFloat`, the
   highest-impedance state.

6. **Raw WASM imports for `mcu-minimal`, WIT for everyone else (C3).**
   wasm3 does not support the WebAssembly Component Model and never will
   (it is a 64 KB interpreter targeting Cortex-M3 devices with 128 KB
   SRAM; component model resolution alone would exceed its RAM budget).
   So the HAL exposes two binding surfaces from one logical interface:
   a WIT package `vyoma:hal@0.1.0` for Wasmtime tier, and a flat C-ABI
   `env.*` import table (`env.gpio_read`, `env.i2c_write_read`,
   `env.uart_write`, ...) for wasm3 tier. Both are documented in §6 of
   this spec. The HAL trait layer (§7) sits below both bindings, so the
   actual driver code is shared.

7. **HalRegistry per-class locking, not global.** A `Mutex<HashMap>` of
   all peripherals would serialise every open call across every bus. The
   registry instead carries one `RwLock<HashMap<Pin, OwnerSet>>` for
   GPIO, one for I²C, one for SPI, one for UART, one for ADC, one for
   PWM. Read traffic (`open()` with `SharedRead` mode that hits an
   already-open pin) takes a read lock; only ownership mutations take a
   write lock. This matters because the framebuffer compositor opens
   dozens of pins per frame on robotics-rt for safety sensors.

8. **`HalRequest` is a small enum, not a closure.** A naive design would
   send `Box<dyn FnOnce(&mut dyn Bus) -> HalResult + Send>` to the bus
   thread. That allocates on every call, and `Box<dyn FnOnce>` resolves
   late at runtime. Instead, `HalRequest` is a flat enum:
   `Read { addr, reg, len, reply }`, `Write { addr, reg, data, reply }`,
   `WriteRead { addr, reg, data, read_len, reply }`, etc. No allocation
   on the hot path; the bus thread matches on the enum and dispatches.

9. **No interrupt-driven GPIO in v1.** `GPIO_V2` supports edge-triggered
   events via `gpio_v2_line_event` on the line fd, and the right
   answer for v2 is to spin a polling thread per requested-event line.
   For v1, only level reads are supported. Apps that need edge detection
   poll on a 1 ms timer. This is documented as a known limitation; the
   API surface is forward-compatible (a `NotifyOnly` access mode is
   already reserved for v2 edge events).

10. **PWM is real, not "GPIO with a software timer".** The userspace
    sysfs PWM interface (`/sys/class/pwm/pwmchip*/pwm0/`) is the only
    portable Linux ABI, and it predates GPIO_V2 chardevs. We use it.
    This is the one place v1 still touches sysfs; the path is wrapped
    in a `PwmChannel` trait so a future chardev PWM can replace it
    transparently.

11. **Audio (ALSA) and input (evdev) are HAL responsibilities.** The
    architect originally drew the line between "HAL = peripheral buses"
    and "drivers = audio/input". The critic noted that the unified
    capability model (§4) and per-bus thread queue (§3) apply equally
    well to ALSA PCM devices and evdev keyboards/mice/touchscreens.
    Folding them in costs ~120 LOC and removes a special-case codepath
    from the supervisor. Microphone is `AccessMode::ExclusiveReadWrite`
    by default; evdev keyboard is `SharedRead` (multiple apps can
    observe key events when the input router fans out).

12. **Implementation files cap at 500 LOC.** Per the project rule, no
    file in `supervisor/src/hal/` exceeds 500 lines. The table in §11
    lists every planned file and its target LOC. The largest is
    `gpio_v2.rs` at 480 lines (the ioctl + parse code for
    `gpio_v2_line_request` alone is dense).

## 1. HAL Architecture — Two Compile-Time Tiers (C6)

The HAL is structured as a layered crate inside the supervisor:

```
supervisor/src/hal/
├── mod.rs                # Public API + tier dispatch
├── types.rs              # AccessMode, HalRequest, HalResult, error types
├── registry.rs           # HalRegistry — capability enforcement
├── bus_thread.rs         # Per-bus thread loop + crossbeam queue
├── gpio.rs               # GpioPin trait + safe state logic
├── gpio_v2.rs            # GPIO_V2 chardev backend (Linux)
├── gpio_mmio.rs          # Direct MMIO backend (MCU)
├── i2c.rs                # I2cBus trait
├── i2c_linux.rs          # /dev/i2c-N backend
├── i2c_mmio.rs           # MCU I2C peripheral backend
├── spi.rs                # SpiBus trait
├── spi_linux.rs          # /dev/spidev backend
├── uart.rs               # UartPort trait
├── uart_linux.rs         # /dev/ttyAMA backend (termios)
├── uart_mmio.rs          # MCU UART backend
├── adc.rs                # AdcChannel trait
├── adc_iio.rs            # IIO sysfs backend (still required, no chardev exists)
├── pwm.rs                # PwmChannel trait
├── pwm_sysfs.rs          # /sys/class/pwm backend
├── audio.rs              # AudioPcm trait
├── audio_alsa.rs         # ALSA backend
├── input.rs              # InputDevice trait
├── input_evdev.rs        # evdev backend
├── safe_state.rs         # GpioSafeState enum + apply logic
├── wit_bindings.rs       # vyoma:hal@0.1.0 WIT exports (wasmtime-tier)
└── raw_bindings.rs       # env.* C-ABI imports (wasm3-tier)
```

The two compile-time tiers are selected by exactly one Cargo feature
each:

```toml
# supervisor/Cargo.toml
[features]
default = ["wasmtime-tier"]
wasmtime-tier = ["dep:wasmtime", "dep:wit-bindgen-host"]
wasm3-tier = ["dep:wasm3"]
```

There is no `gpio-v2`, `i2c`, `desktop-full`, etc. feature. The supervisor
binary always compiles all backends that its tier supports, and the
`PlatformProfile` loaded at startup decides which ones to instantiate.
This means `cargo build --features wasmtime-tier` produces one binary
that runs on `iot-edge`, `robotics-rt`, `mobile`, `desktop-full`, and
`server-headless`. `cargo build --no-default-features --features
wasm3-tier` produces the `mcu-minimal` binary.

The dispatch happens in `hal/mod.rs`:

```rust
// supervisor/src/hal/mod.rs

use crate::profile::PlatformProfile;
use crate::hal::registry::HalRegistry;
use crate::hal::types::{HalResult, HalError};
use std::sync::Arc;

pub mod types;
pub mod registry;
pub mod bus_thread;
pub mod gpio;
pub mod i2c;
pub mod spi;
pub mod uart;
pub mod adc;
pub mod pwm;
pub mod audio;
pub mod input;
pub mod safe_state;

#[cfg(feature = "wasmtime-tier")]
pub mod wit_bindings;

#[cfg(feature = "wasm3-tier")]
pub mod raw_bindings;

#[cfg(feature = "wasmtime-tier")]
mod backends_linux {
    pub use super::gpio_v2;
    pub use super::i2c_linux;
    pub use super::spi_linux;
    pub use super::uart_linux;
    pub use super::adc_iio;
    pub use super::pwm_sysfs;
    pub use super::audio_alsa;
    pub use super::input_evdev;
}

#[cfg(feature = "wasm3-tier")]
mod backends_mcu {
    pub use super::gpio_mmio;
    pub use super::i2c_mmio;
    pub use super::uart_mmio;
}

#[cfg(feature = "wasmtime-tier")]
pub mod gpio_v2;
#[cfg(feature = "wasmtime-tier")]
pub mod i2c_linux;
#[cfg(feature = "wasmtime-tier")]
pub mod spi_linux;
#[cfg(feature = "wasmtime-tier")]
pub mod uart_linux;
#[cfg(feature = "wasmtime-tier")]
pub mod adc_iio;
#[cfg(feature = "wasmtime-tier")]
pub mod pwm_sysfs;
#[cfg(feature = "wasmtime-tier")]
pub mod audio_alsa;
#[cfg(feature = "wasmtime-tier")]
pub mod input_evdev;

#[cfg(feature = "wasm3-tier")]
pub mod gpio_mmio;
#[cfg(feature = "wasm3-tier")]
pub mod i2c_mmio;
#[cfg(feature = "wasm3-tier")]
pub mod uart_mmio;

/// Initialise the HAL for the running platform profile.
/// Called exactly once by the supervisor before any app is spawned.
pub fn init(profile: &PlatformProfile) -> HalResult<Arc<HalRegistry>> {
    let registry = HalRegistry::new();

    #[cfg(feature = "wasmtime-tier")]
    {
        // Open every gpiochip the profile declares.
        for chip in &profile.gpio_chips {
            let backend = gpio_v2::GpioV2Backend::open(&chip.path)
                .map_err(|e| HalError::BackendOpenFailed {
                    bus: "gpio".into(),
                    path: chip.path.clone(),
                    source: e.to_string(),
                })?;
            registry.register_gpio_backend(chip.id, Box::new(backend))?;
        }

        for bus in &profile.i2c_buses {
            let backend = i2c_linux::I2cLinuxBackend::open(&bus.path)?;
            registry.register_i2c_backend(bus.id, Box::new(backend))?;
        }

        for bus in &profile.spi_buses {
            let backend = spi_linux::SpiLinuxBackend::open(&bus.path)?;
            registry.register_spi_backend(bus.id, Box::new(backend))?;
        }

        for port in &profile.uart_ports {
            let backend = uart_linux::UartLinuxBackend::open(&port.path, port.baud)?;
            registry.register_uart_backend(port.id, Box::new(backend))?;
        }

        for adc in &profile.adc_channels {
            let backend = adc_iio::AdcIioBackend::open(&adc.path)?;
            registry.register_adc_backend(adc.id, Box::new(backend))?;
        }

        for pwm in &profile.pwm_channels {
            let backend = pwm_sysfs::PwmSysfsBackend::open(&pwm.path)?;
            registry.register_pwm_backend(pwm.id, Box::new(backend))?;
        }

        for audio in &profile.audio_devices {
            let backend = audio_alsa::AlsaBackend::open(&audio.path)?;
            registry.register_audio_backend(audio.id, Box::new(backend))?;
        }

        for input in &profile.input_devices {
            let backend = input_evdev::EvdevBackend::open(&input.path)?;
            registry.register_input_backend(input.id, Box::new(backend))?;
        }
    }

    #[cfg(feature = "wasm3-tier")]
    {
        // MCU MMIO backends.
        for chip in &profile.gpio_chips {
            let backend = gpio_mmio::GpioMmioBackend::map(chip.base_addr, chip.pin_count)?;
            registry.register_gpio_backend(chip.id, Box::new(backend))?;
        }

        for bus in &profile.i2c_buses {
            let backend = i2c_mmio::I2cMmioBackend::map(bus.base_addr)?;
            registry.register_i2c_backend(bus.id, Box::new(backend))?;
        }

        for port in &profile.uart_ports {
            let backend = uart_mmio::UartMmioBackend::map(port.base_addr, port.baud)?;
            registry.register_uart_backend(port.id, Box::new(backend))?;
        }
    }

    // Spawn one dedicated thread per backend; thread takes ownership.
    registry.start_all_bus_threads()?;

    Ok(Arc::new(registry))
}
```

### 1.1 Build-time kernel probe (C1)

`supervisor/build.rs` runs at build time and verifies the target kernel
does not regress to sysfs GPIO. On host builds (where the dev machine
runs ≥ 5.5) it succeeds silently; on cross builds it inspects the kernel
config supplied by the `PLATFORM` env var:

```rust
// supervisor/build.rs

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-env-changed=PLATFORM");
    println!("cargo:rerun-if-env-changed=KERNEL_CONFIG_PATH");

    let platform = env::var("PLATFORM").unwrap_or_else(|_| "desktop-full".to_string());
    if platform == "mcu-minimal" {
        // No kernel on MCU; skip.
        return;
    }

    let cfg_path = env::var("KERNEL_CONFIG_PATH")
        .unwrap_or_else(|_| "base/kernel-config".to_string());

    let cfg = match fs::read_to_string(&cfg_path) {
        Ok(s) => s,
        Err(_) => {
            // Host dev build; rely on runtime check.
            return;
        }
    };

    let has_chardev = cfg.contains("CONFIG_GPIO_CDEV=y")
        || cfg.contains("CONFIG_GPIO_CDEV=m");
    let has_sysfs = cfg.contains("CONFIG_GPIO_SYSFS=y");
    let kernel_version = extract_kernel_version(&cfg).unwrap_or((5, 10));

    if !has_chardev {
        panic!(
            "VyomaOS HAL requires CONFIG_GPIO_CDEV=y. \
             Kernel config {} has only sysfs GPIO. \
             VyomaOS refuses to build on sysfs GPIO for kernels >= 5.5.",
            cfg_path
        );
    }

    if has_sysfs && kernel_version >= (5, 5) {
        println!(
            "cargo:warning=Kernel config enables CONFIG_GPIO_SYSFS \
             on kernel {}.{}. VyomaOS will not use it — \
             consider removing for smaller kernel.",
            kernel_version.0, kernel_version.1
        );
    }
}

fn extract_kernel_version(cfg: &str) -> Option<(u32, u32)> {
    // CONFIG_LOCALVERSION is unreliable; rely on Makefile VERSION/PATCHLEVEL.
    // For simplicity, hardcode the floor: VyomaOS targets 5.10+ per CLAUDE.md.
    Some((5, 10))
}
```

## 2. GPIO — GPIO_V2 Chardev (C1)

GPIO_V2 was introduced in Linux 5.10 and is the only stable, future-proof
GPIO userspace ABI. The older `GPIOHANDLE_GET_LINE_INFO_IOCTL` (v1)
remains in the kernel but lacks debounce, edge detection metadata, and
the ability to atomically configure flags. We target v2 exclusively.

### 2.1 The GPIO_V2 ioctl protocol

The chardev `/dev/gpiochip0` exposes one fd. To work with lines (pins),
userspace issues `GPIO_V2_GET_LINE_IOCTL` with a `gpio_v2_line_request`
struct. The kernel returns a new fd that owns the requested lines. All
subsequent reads/writes go through this line fd, not the chip fd.

```rust
// supervisor/src/hal/gpio_v2.rs

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

use crate::hal::types::{HalError, HalResult};

// From <linux/gpio.h>, kernel 5.10+.
const GPIO_V2_LINES_MAX: usize = 64;
const GPIO_MAX_NAME_SIZE: usize = 32;

const GPIO_V2_LINE_FLAG_USED: u64 = 1 << 0;
const GPIO_V2_LINE_FLAG_ACTIVE_LOW: u64 = 1 << 1;
const GPIO_V2_LINE_FLAG_INPUT: u64 = 1 << 2;
const GPIO_V2_LINE_FLAG_OUTPUT: u64 = 1 << 3;
const GPIO_V2_LINE_FLAG_EDGE_RISING: u64 = 1 << 4;
const GPIO_V2_LINE_FLAG_EDGE_FALLING: u64 = 1 << 5;
const GPIO_V2_LINE_FLAG_OPEN_DRAIN: u64 = 1 << 6;
const GPIO_V2_LINE_FLAG_OPEN_SOURCE: u64 = 1 << 7;
const GPIO_V2_LINE_FLAG_BIAS_PULL_UP: u64 = 1 << 8;
const GPIO_V2_LINE_FLAG_BIAS_PULL_DOWN: u64 = 1 << 9;
const GPIO_V2_LINE_FLAG_BIAS_DISABLED: u64 = 1 << 10;

#[repr(C)]
struct GpioV2LineAttribute {
    id: u32,
    padding: u32,
    value: u64,
}

#[repr(C)]
struct GpioV2LineConfigAttribute {
    attr: GpioV2LineAttribute,
    mask: u64,
}

#[repr(C)]
struct GpioV2LineConfig {
    flags: u64,
    num_attrs: u32,
    padding: [u32; 5],
    attrs: [GpioV2LineConfigAttribute; 10],
}

#[repr(C)]
struct GpioV2LineRequest {
    offsets: [u32; GPIO_V2_LINES_MAX],
    consumer: [u8; GPIO_MAX_NAME_SIZE],
    config: GpioV2LineConfig,
    num_lines: u32,
    event_buffer_size: u32,
    padding: [u32; 5],
    fd: i32,
}

#[repr(C)]
struct GpioV2LineValues {
    bits: u64,
    mask: u64,
}

// ioctl numbers (Linux _IOWR generated for GPIO_V2_GET_LINE_IOCTL = 0x07).
// 'G' << 8 | 7 | direction bits.
const GPIO_V2_GET_LINE_IOCTL: u64 = 0xC250_4707;
const GPIO_V2_LINE_GET_VALUES_IOCTL: u64 = 0xC010_4B0E;
const GPIO_V2_LINE_SET_VALUES_IOCTL: u64 = 0xC010_4B0F;

extern "C" {
    fn ioctl(fd: RawFd, request: u64, ...) -> i32;
}

pub struct GpioV2Backend {
    chip_fd: OwnedFd,
    chip_path: String,
}

pub struct GpioV2LineHandle {
    line_fd: OwnedFd,
    offsets: Vec<u32>,
}

impl GpioV2Backend {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let chip_fd = unsafe { OwnedFd::from_raw_fd(file.as_raw_fd()) };
        // Forget the File so it doesn't close the fd when dropped.
        std::mem::forget(file);
        Ok(Self {
            chip_fd,
            chip_path: path.to_string_lossy().into_owned(),
        })
    }

    /// Request a single line for input (no edge detection).
    pub fn request_input(
        &self,
        offset: u32,
        bias: BiasMode,
        consumer: &str,
    ) -> HalResult<GpioV2LineHandle> {
        self.request_lines(&[offset], LineDirection::Input(bias), consumer)
    }

    /// Request a single line for output, with an initial value.
    pub fn request_output(
        &self,
        offset: u32,
        initial: bool,
        consumer: &str,
    ) -> HalResult<GpioV2LineHandle> {
        self.request_lines(
            &[offset],
            LineDirection::Output { initial },
            consumer,
        )
    }

    fn request_lines(
        &self,
        offsets: &[u32],
        dir: LineDirection,
        consumer: &str,
    ) -> HalResult<GpioV2LineHandle> {
        if offsets.is_empty() || offsets.len() > GPIO_V2_LINES_MAX {
            return Err(HalError::InvalidArgument {
                what: "offsets.len()".into(),
                value: offsets.len().to_string(),
            });
        }

        let mut req: GpioV2LineRequest = unsafe { std::mem::zeroed() };
        for (i, off) in offsets.iter().enumerate() {
            req.offsets[i] = *off;
        }
        req.num_lines = offsets.len() as u32;

        // Consumer name (truncated to 31 bytes + null).
        let consumer_bytes = consumer.as_bytes();
        let n = consumer_bytes.len().min(GPIO_MAX_NAME_SIZE - 1);
        req.consumer[..n].copy_from_slice(&consumer_bytes[..n]);

        req.config.flags = match dir {
            LineDirection::Input(bias) => {
                let mut f = GPIO_V2_LINE_FLAG_INPUT;
                f |= match bias {
                    BiasMode::Floating => GPIO_V2_LINE_FLAG_BIAS_DISABLED,
                    BiasMode::PullUp => GPIO_V2_LINE_FLAG_BIAS_PULL_UP,
                    BiasMode::PullDown => GPIO_V2_LINE_FLAG_BIAS_PULL_DOWN,
                };
                f
            }
            LineDirection::Output { initial } => {
                let mut f = GPIO_V2_LINE_FLAG_OUTPUT;
                // The initial value is set via an output_values attribute.
                req.config.num_attrs = 1;
                req.config.attrs[0] = GpioV2LineConfigAttribute {
                    attr: GpioV2LineAttribute {
                        id: 2, // GPIO_V2_LINE_ATTR_ID_OUTPUT_VALUES
                        padding: 0,
                        value: if initial { 1 } else { 0 },
                    },
                    mask: 1,
                };
                f
            }
        };

        let rc = unsafe {
            ioctl(
                self.chip_fd.as_raw_fd(),
                GPIO_V2_GET_LINE_IOCTL,
                &mut req as *mut _,
            )
        };
        if rc < 0 {
            return Err(HalError::IoctlFailed {
                op: "GPIO_V2_GET_LINE_IOCTL".into(),
                errno: io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }

        if req.fd < 0 {
            return Err(HalError::IoctlFailed {
                op: "GPIO_V2_GET_LINE_IOCTL returned negative fd".into(),
                errno: 0,
            });
        }

        let line_fd = unsafe { OwnedFd::from_raw_fd(req.fd) };
        Ok(GpioV2LineHandle {
            line_fd,
            offsets: offsets.to_vec(),
        })
    }
}

impl GpioV2LineHandle {
    pub fn read_value(&self) -> HalResult<u64> {
        let mut values = GpioV2LineValues {
            bits: 0,
            mask: (1u64 << self.offsets.len()) - 1,
        };
        let rc = unsafe {
            ioctl(
                self.line_fd.as_raw_fd(),
                GPIO_V2_LINE_GET_VALUES_IOCTL,
                &mut values as *mut _,
            )
        };
        if rc < 0 {
            return Err(HalError::IoctlFailed {
                op: "GPIO_V2_LINE_GET_VALUES_IOCTL".into(),
                errno: io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        Ok(values.bits)
    }

    pub fn write_value(&self, bits: u64) -> HalResult<()> {
        let values = GpioV2LineValues {
            bits,
            mask: (1u64 << self.offsets.len()) - 1,
        };
        let rc = unsafe {
            ioctl(
                self.line_fd.as_raw_fd(),
                GPIO_V2_LINE_SET_VALUES_IOCTL,
                &values as *const _,
            )
        };
        if rc < 0 {
            return Err(HalError::IoctlFailed {
                op: "GPIO_V2_LINE_SET_VALUES_IOCTL".into(),
                errno: io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BiasMode {
    Floating,
    PullUp,
    PullDown,
}

#[derive(Debug, Clone, Copy)]
pub enum LineDirection {
    Input(BiasMode),
    Output { initial: bool },
}
```

### 2.2 sysfs refusal at runtime

In addition to the build-time check, the supervisor refuses to fall back
to sysfs GPIO at runtime even if it sees `/sys/class/gpio/export`. The
`GpioV2Backend::open()` returns `HalError::BackendOpenFailed` if
`/dev/gpiochip0` is missing; the operator must fix the kernel
configuration or the device tree.

## 3. I2C / SPI / UART — Per-Bus Thread Queue (C2)

### 3.1 Why dedicated threads

A 100 kHz I²C bus takes 90 µs to transmit a single 9-bit clocked byte.
The BME280 environmental sensor commonly used on iot-edge profiles
requires a 1 ms minimum delay between writing a configuration register
and reading conversion results. If the supervisor IPC event loop blocks
on `i2c_smbus_write_byte()`, it stalls the keyboard router, the mouse
router, the framebuffer compositor, and every other concurrent app.

A 400 kHz SPI bus is fast, but the userspace `SPI_IOC_MESSAGE` ioctl
still does a context-switch round-trip and the kernel driver may pause
on its own internal locks. Same problem.

UART at 115200 baud takes 87 µs per byte minimum; at 9600 baud (common
for GPS modules) it's 1.04 ms per byte. A 100-byte NMEA sentence at
9600 baud stalls for 104 ms — six frames at 60 Hz.

So each bus runs on its own OS thread. The thread owns the file
descriptor and processes requests one at a time. Callers send a request
through a `crossbeam::channel::Sender<HalRequest>` and immediately
receive a `oneshot::Receiver<HalResult>` (we use the `oneshot` crate, or
hand-roll with `crossbeam::channel::bounded(1)`).

### 3.2 HalRequest / HalResult types

```rust
// supervisor/src/hal/types.rs

use std::sync::Arc;
use crossbeam::channel::Sender;

/// One bus request. Sent from any thread to the bus's dedicated worker.
/// Each variant carries a reply channel so the caller can await the result.
pub enum HalRequest {
    // ----- I2C -----
    I2cRead {
        addr: u8,
        len: usize,
        reply: Sender<HalResult<Vec<u8>>>,
    },
    I2cWrite {
        addr: u8,
        data: Vec<u8>,
        reply: Sender<HalResult<()>>,
    },
    I2cWriteRead {
        addr: u8,
        write: Vec<u8>,
        read_len: usize,
        reply: Sender<HalResult<Vec<u8>>>,
    },

    // ----- SPI -----
    SpiTransfer {
        tx: Vec<u8>,
        rx_len: usize,
        speed_hz: u32,
        cs_change: bool,
        reply: Sender<HalResult<Vec<u8>>>,
    },

    // ----- UART -----
    UartRead {
        len: usize,
        timeout_ms: u32,
        reply: Sender<HalResult<Vec<u8>>>,
    },
    UartWrite {
        data: Vec<u8>,
        reply: Sender<HalResult<usize>>,
    },
    UartSetBaud {
        baud: u32,
        reply: Sender<HalResult<()>>,
    },

    // ----- GPIO -----
    // GPIO is unusual because it doesn't typically need a long-running
    // thread (reads/writes are sub-microsecond ioctls). But for consistency
    // and to support future edge-event polling, GPIO also goes through
    // the queue.
    GpioRead {
        offset: u32,
        reply: Sender<HalResult<bool>>,
    },
    GpioWrite {
        offset: u32,
        value: bool,
        reply: Sender<HalResult<()>>,
    },

    // ----- ADC -----
    AdcRead {
        channel: u8,
        reply: Sender<HalResult<u32>>,
    },

    // ----- PWM -----
    PwmSet {
        period_ns: u32,
        duty_ns: u32,
        reply: Sender<HalResult<()>>,
    },
    PwmEnable {
        on: bool,
        reply: Sender<HalResult<()>>,
    },

    // ----- Lifecycle -----
    /// Cleanly stop the bus thread; bus closes its fd.
    Shutdown {
        reply: Sender<HalResult<()>>,
    },
}

pub type HalResult<T> = Result<T, HalError>;

#[derive(Debug, Clone)]
pub enum HalError {
    BackendOpenFailed {
        bus: String,
        path: String,
        source: String,
    },
    IoctlFailed {
        op: String,
        errno: i32,
    },
    BusShutdown,
    AccessDenied {
        peripheral: String,
        attempted_mode: String,
        held_by: String,
        held_mode: String,
    },
    InvalidArgument {
        what: String,
        value: String,
    },
    Timeout,
    NotImplemented(String),
}

impl std::fmt::Display for HalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HalError::BackendOpenFailed { bus, path, source } => {
                write!(f, "{} backend open failed for {}: {}", bus, path, source)
            }
            HalError::IoctlFailed { op, errno } => {
                write!(f, "{} ioctl failed: errno {}", op, errno)
            }
            HalError::BusShutdown => write!(f, "bus is shut down"),
            HalError::AccessDenied {
                peripheral, attempted_mode, held_by, held_mode,
            } => write!(
                f,
                "access denied to {}: requested {} but held by {} in mode {}",
                peripheral, attempted_mode, held_by, held_mode,
            ),
            HalError::InvalidArgument { what, value } => {
                write!(f, "invalid argument {}={}", what, value)
            }
            HalError::Timeout => write!(f, "timeout"),
            HalError::NotImplemented(s) => write!(f, "not implemented: {}", s),
        }
    }
}

impl std::error::Error for HalError {}
```

### 3.3 Bus thread loop

Each bus has a small struct that owns the worker thread and the sender
end of the queue. The receiver lives inside the thread.

```rust
// supervisor/src/hal/bus_thread.rs

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use crossbeam::channel::{bounded, Receiver, Sender};

use crate::hal::types::{HalRequest, HalResult, HalError};

/// A trait every per-bus backend implements. The bus thread calls
/// `handle_request` once per dequeued message; the backend dispatches
/// on the enum variant.
pub trait BusBackend: Send + 'static {
    fn handle_request(&mut self, req: HalRequest);
    fn shutdown(&mut self);
}

pub struct BusThread {
    sender: Sender<HalRequest>,
    join: Option<JoinHandle<()>>,
}

impl BusThread {
    pub fn spawn<B: BusBackend>(name: &str, mut backend: B) -> Self {
        // Bounded queue with backpressure; 256 is enough for ~50 ms of I²C
        // traffic at 100 kHz worst-case, much more than a typical frame.
        let (sender, receiver): (Sender<HalRequest>, Receiver<HalRequest>) =
            bounded(256);

        let thread_name = format!("hal-{}", name);
        let join = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                loop {
                    match receiver.recv() {
                        Ok(HalRequest::Shutdown { reply }) => {
                            backend.shutdown();
                            let _ = reply.send(Ok(()));
                            return;
                        }
                        Ok(req) => backend.handle_request(req),
                        Err(_) => {
                            // Sender dropped; supervisor exiting.
                            backend.shutdown();
                            return;
                        }
                    }
                }
            })
            .expect("failed to spawn HAL bus thread");

        Self {
            sender,
            join: Some(join),
        }
    }

    pub fn sender(&self) -> Sender<HalRequest> {
        self.sender.clone()
    }

    /// Block until the bus thread exits cleanly. Called by HalRegistry on
    /// supervisor shutdown.
    pub fn shutdown(mut self) -> HalResult<()> {
        let (tx, rx) = bounded(1);
        self.sender
            .send(HalRequest::Shutdown { reply: tx })
            .map_err(|_| HalError::BusShutdown)?;
        let _ = rx.recv();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        Ok(())
    }
}

/// Convenience helper to send a request and synchronously await its reply.
/// Used by WIT bindings (which are called from inside Wasmtime host functions
/// that are themselves running on a worker pool, so blocking is fine).
pub fn send_blocking<T>(
    bus: &Sender<HalRequest>,
    build: impl FnOnce(Sender<HalResult<T>>) -> HalRequest,
) -> HalResult<T> {
    let (tx, rx) = bounded::<HalResult<T>>(1);
    bus.send(build(tx)).map_err(|_| HalError::BusShutdown)?;
    rx.recv().map_err(|_| HalError::BusShutdown)?
}
```

### 3.4 I2C Linux backend

```rust
// supervisor/src/hal/i2c_linux.rs

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;

use crate::hal::bus_thread::BusBackend;
use crate::hal::types::{HalError, HalRequest, HalResult};

const I2C_SLAVE: u64 = 0x0703;
const I2C_RDWR: u64 = 0x0707;

#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

const I2C_M_RD: u16 = 0x0001;

#[repr(C)]
struct I2cRdwrIoctlData {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

extern "C" {
    fn ioctl(fd: RawFd, request: u64, ...) -> i32;
}

pub struct I2cLinuxBackend {
    file: File,
}

impl I2cLinuxBackend {
    pub fn open(path: &Path) -> HalResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| HalError::BackendOpenFailed {
                bus: "i2c".into(),
                path: path.to_string_lossy().into_owned(),
                source: e.to_string(),
            })?;
        Ok(Self { file })
    }

    fn set_slave(&self, addr: u8) -> HalResult<()> {
        let rc = unsafe { ioctl(self.file.as_raw_fd(), I2C_SLAVE, addr as u64) };
        if rc < 0 {
            return Err(HalError::IoctlFailed {
                op: "I2C_SLAVE".into(),
                errno: io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        Ok(())
    }

    fn read(&self, addr: u8, len: usize) -> HalResult<Vec<u8>> {
        self.set_slave(addr)?;
        use std::io::Read;
        let mut buf = vec![0u8; len];
        let mut f = &self.file;
        f.read_exact(&mut buf).map_err(|e| HalError::IoctlFailed {
            op: "i2c read".into(),
            errno: e.raw_os_error().unwrap_or(0),
        })?;
        Ok(buf)
    }

    fn write(&self, addr: u8, data: &[u8]) -> HalResult<()> {
        self.set_slave(addr)?;
        use std::io::Write;
        let mut f = &self.file;
        f.write_all(data).map_err(|e| HalError::IoctlFailed {
            op: "i2c write".into(),
            errno: e.raw_os_error().unwrap_or(0),
        })?;
        Ok(())
    }

    fn write_read(
        &self,
        addr: u8,
        write: &[u8],
        read_len: usize,
    ) -> HalResult<Vec<u8>> {
        // Use I2C_RDWR for a repeated-start transaction.
        let mut read_buf = vec![0u8; read_len];
        let mut msgs = [
            I2cMsg {
                addr: addr as u16,
                flags: 0,
                len: write.len() as u16,
                buf: write.as_ptr() as *mut u8,
            },
            I2cMsg {
                addr: addr as u16,
                flags: I2C_M_RD,
                len: read_len as u16,
                buf: read_buf.as_mut_ptr(),
            },
        ];
        let data = I2cRdwrIoctlData {
            msgs: msgs.as_mut_ptr(),
            nmsgs: 2,
        };
        let rc = unsafe {
            ioctl(self.file.as_raw_fd(), I2C_RDWR, &data as *const _)
        };
        if rc < 0 {
            return Err(HalError::IoctlFailed {
                op: "I2C_RDWR".into(),
                errno: io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        Ok(read_buf)
    }
}

impl BusBackend for I2cLinuxBackend {
    fn handle_request(&mut self, req: HalRequest) {
        match req {
            HalRequest::I2cRead { addr, len, reply } => {
                let _ = reply.send(self.read(addr, len));
            }
            HalRequest::I2cWrite { addr, data, reply } => {
                let _ = reply.send(self.write(addr, &data));
            }
            HalRequest::I2cWriteRead { addr, write, read_len, reply } => {
                let _ = reply.send(self.write_read(addr, &write, read_len));
            }
            other => {
                // Wrong request type for this bus. Drop on the floor;
                // the caller's reply channel will close.
                drop(other);
            }
        }
    }

    fn shutdown(&mut self) {
        // File closes on drop; nothing else to do.
    }
}
```

### 3.5 SPI and UART Linux backends

Analogous to I²C. SPI uses `SPI_IOC_MESSAGE(N)` ioctls with
`spi_ioc_transfer` structs. UART uses `termios` to set baud rate and
flow control, then plain `read()`/`write()` on the tty fd. Both follow
the same `BusBackend` pattern: open the fd in `new()`, dispatch on
`HalRequest` variants in `handle_request()`. See files
`hal/spi_linux.rs` (≤ 320 LOC) and `hal/uart_linux.rs` (≤ 380 LOC) in
the implementation table.

## 4. Access Mode Model (C4)

### 4.1 AccessMode enum

```rust
// supervisor/src/hal/types.rs (continued)

/// How an app intends to use a peripheral. Determines compatibility with
/// concurrent opens by other apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessMode {
    /// Exclusive read+write. No other app may hold the peripheral in any
    /// mode while this one is active. Used for actuators (motors, relays,
    /// LEDs, write-capable buses).
    ExclusiveReadWrite,

    /// Read-only. Multiple SharedRead holders may coexist; an
    /// ExclusiveReadWrite is denied while any SharedRead is held.
    /// Used for sensor reads, button state polling, status monitoring.
    SharedRead,

    /// The app does not perform I/O itself; it only wants to be notified
    /// of events (edge events for GPIO, address-match for I²C target mode
    /// in v2). Notify-only handles coexist with everyone.
    NotifyOnly,

    /// I²C only: the app owns one specific 7-bit slave address on the
    /// bus, leaving other addresses for other apps. The compatibility
    /// matrix below treats AddressScoped(a) as exclusive only when both
    /// holders target address `a`.
    AddressScoped(u8),
}

impl AccessMode {
    /// True if a new request in `requested` mode is allowed when an
    /// existing handle holds the peripheral in `held` mode.
    pub fn compatible(held: AccessMode, requested: AccessMode) -> bool {
        use AccessMode::*;
        match (held, requested) {
            // Notify-only never blocks anyone.
            (NotifyOnly, _) | (_, NotifyOnly) => true,

            // Shared reads coexist.
            (SharedRead, SharedRead) => true,

            // Exclusive blocks everything else, and is blocked by anything.
            (ExclusiveReadWrite, _) | (_, ExclusiveReadWrite) => false,

            // Address-scoped coexists with other address-scoped holders
            // on different addresses, and with shared-read.
            (AddressScoped(a), AddressScoped(b)) => a != b,
            (AddressScoped(_), SharedRead) | (SharedRead, AddressScoped(_)) => true,
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            AccessMode::ExclusiveReadWrite => "exclusive_rw".into(),
            AccessMode::SharedRead => "shared_read".into(),
            AccessMode::NotifyOnly => "notify_only".into(),
            AccessMode::AddressScoped(a) => format!("address_scoped(0x{:02x})", a),
        }
    }
}
```

### 4.2 Compatibility matrix

| Held \ Requested      | ExclusiveRW | SharedRead | NotifyOnly | AddrScoped(b) |
|-----------------------|-------------|------------|------------|---------------|
| **ExclusiveRW**       | deny        | deny       | allow      | deny          |
| **SharedRead**        | deny        | allow      | allow      | allow         |
| **NotifyOnly**        | allow       | allow      | allow      | allow         |
| **AddrScoped(a==b)**  | deny        | allow      | allow      | deny          |
| **AddrScoped(a!=b)**  | deny        | allow      | allow      | allow         |

This is symmetric: `compatible(X, Y) == compatible(Y, X)`.

### 4.3 HalRegistry::open_gpio signature

```rust
// supervisor/src/hal/registry.rs (extract)

impl HalRegistry {
    pub fn open_gpio(
        &self,
        app_id: AppId,
        chip_id: GpioChipId,
        offset: u32,
        mode: AccessMode,
        consumer: &str,
    ) -> HalResult<GpioHandle> {
        let key = (chip_id, offset);
        let mut owners = self.gpio_owners.write().unwrap();
        let entry = owners.entry(key).or_default();

        // Check compatibility with every existing holder.
        for holder in &entry.holders {
            if !AccessMode::compatible(holder.mode, mode) {
                return Err(HalError::AccessDenied {
                    peripheral: format!("gpio[{:?}, {}]", chip_id, offset),
                    attempted_mode: mode.as_str(),
                    held_by: format!("app:{:?}", holder.app_id),
                    held_mode: holder.mode.as_str(),
                });
            }
        }

        // Capability check: app's manifest declares this pin?
        if !self.app_declares_gpio(app_id, chip_id, offset) {
            return Err(HalError::AccessDenied {
                peripheral: format!("gpio[{:?}, {}]", chip_id, offset),
                attempted_mode: mode.as_str(),
                held_by: "no_capability".into(),
                held_mode: "n/a".into(),
            });
        }

        let backend = self.gpio_backends.get(&chip_id)
            .ok_or_else(|| HalError::BackendOpenFailed {
                bus: "gpio".into(),
                path: format!("chip_id={:?}", chip_id),
                source: "no backend".into(),
            })?;

        let line = backend.request_line(offset, mode, consumer)?;
        let handle_id = self.next_handle_id();
        entry.holders.push(GpioHolder { app_id, mode, handle_id });
        Ok(GpioHandle {
            chip_id,
            offset,
            handle_id,
            line: Arc::new(line),
        })
    }
}
```

## 5. Safe State on Crash (C5)

### 5.1 Manifest declaration

Apps declare a `safe_state` per pin:

```toml
[app]
name    = "motor-controller"
version = "0.1.0"
wasm    = "motor-controller.wasm"

[capabilities]
stdio = true

[capabilities.gpio]
pins = [4, 17, 27]
direction = "output"
safe_state = "output_low"  # all pins low on crash → motor stops
```

For mixed-direction sets, the safe-state can be specified per pin:

```toml
[capabilities.gpio]
[[capabilities.gpio.pin]]
number = 4
direction = "output"
safe_state = "output_low"   # relay off

[[capabilities.gpio.pin]]
number = 17
direction = "input"
safe_state = "input_pulldown"
```

### 5.2 GpioSafeState enum

```rust
// supervisor/src/hal/safe_state.rs

use crate::hal::types::{HalError, HalResult};
use crate::hal::gpio_v2::{BiasMode, GpioV2LineHandle};

#[derive(Debug, Clone, Copy)]
pub enum GpioSafeState {
    /// Reconfigure as input with no bias. Highest impedance — safest
    /// default for unknown loads.
    InputFloat,
    /// Reconfigure as input with internal pull-down.
    InputPullDown,
    /// Reconfigure as input with internal pull-up.
    InputPullUp,
    /// Keep as output, drive low. Use when low de-energises the load
    /// (e.g., active-high relay).
    OutputLow,
    /// Keep as output, drive high. Use when high de-energises the load
    /// (e.g., active-low relay).
    OutputHigh,
    /// Equivalent to InputFloat but explicit. Used for high-Z buses.
    HiZ,
}

impl Default for GpioSafeState {
    fn default() -> Self {
        GpioSafeState::InputFloat
    }
}

impl GpioSafeState {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "input_float" => Some(Self::InputFloat),
            "input_pulldown" => Some(Self::InputPullDown),
            "input_pullup" => Some(Self::InputPullUp),
            "output_low" => Some(Self::OutputLow),
            "output_high" => Some(Self::OutputHigh),
            "hi_z" => Some(Self::HiZ),
            _ => None,
        }
    }

    /// Apply this safe state to a held line. Called by the supervisor's
    /// on_instance_gone hook before the GPIO_V2 line fd is released.
    /// On failure, the supervisor logs and continues — the kernel will
    /// reset the pin to its boot-time direction when the fd is closed,
    /// which is still safer than leaving the app's last write in place.
    pub fn apply(&self, line: &GpioV2LineHandle) -> HalResult<()> {
        match self {
            GpioSafeState::InputFloat | GpioSafeState::HiZ => {
                line.reconfigure_input(BiasMode::Floating)
            }
            GpioSafeState::InputPullDown => {
                line.reconfigure_input(BiasMode::PullDown)
            }
            GpioSafeState::InputPullUp => {
                line.reconfigure_input(BiasMode::PullUp)
            }
            GpioSafeState::OutputLow => line.write_value(0),
            GpioSafeState::OutputHigh => line.write_value(0xFFFF_FFFF_FFFF_FFFF),
        }
    }
}
```

### 5.3 on_instance_gone hook

The supervisor lifecycle layer (`supervisor/src/lifecycle.rs`) gains a
new hook that fires immediately before a WASM instance is dropped:

```rust
// supervisor/src/lifecycle.rs (extract)

impl Supervisor {
    pub fn on_instance_gone(&mut self, app_id: AppId, reason: ExitReason) {
        // 1. Apply safe state to every GPIO pin this app held.
        let pins = self.hal.gpio_pins_held_by(app_id);
        for (chip_id, offset, safe_state, handle) in pins {
            if let Err(e) = safe_state.apply(&handle) {
                tracing::warn!(
                    app=?app_id, chip=?chip_id, pin=offset,
                    state=?safe_state, error=%e,
                    "failed to apply GPIO safe state on app exit"
                );
            }
        }

        // 2. Release every HAL handle this app held.
        self.hal.release_all(app_id);

        // 3. Audio/input handles: stop streams.
        self.hal.stop_audio_streams(app_id);
        self.hal.stop_input_streams(app_id);

        // 4. Log the lifecycle event.
        tracing::info!(app=?app_id, reason=?reason, "instance gone, HAL cleaned up");
    }
}
```

The order matters: apply safe state **before** releasing the handle.
If the safe-state write fails, the subsequent fd close still drops the
line back to its boot-default direction, but a successful safe-state
write is preferred because it can keep an actuator in a known state
even if the boot default is different (e.g., a relay configured as
output-high at boot would be undesirable for a motor controller).

## 6. mcu-minimal Raw Import Binding (C3)

### 6.1 Tier comparison

| Aspect              | wasmtime-tier (WIT)              | wasm3-tier (raw imports)         |
|---------------------|----------------------------------|----------------------------------|
| Binding model       | WebAssembly Component Model      | Core WASM `(import "env" ...)`   |
| Runtime size        | ~3 MB                            | ~64 KB                           |
| Memory floor        | 256 KB+                          | 16 KB                            |
| Resource types      | Yes (`gpio-pin` is a resource)   | No — opaque `u32` handles        |
| String passing      | Native `string` type             | `(ptr: i32, len: i32)`           |
| Result types        | `result<T, error>`               | `i32` return code + out-param    |
| Available on        | iot-edge, robotics-rt, mobile,   | mcu-minimal only                 |
|                     | desktop-full, server-headless    |                                  |

### 6.2 Raw import function signatures (wasm3-tier)

The wasm3 runtime exposes the following imports on the `env` namespace:

```
;; GPIO
(import "env" "gpio_open"
        (func (param i32 i32 i32) (result i32)))
;; (chip_id, offset, mode) -> handle (0 = error)

(import "env" "gpio_close"
        (func (param i32) (result i32)))
;; (handle) -> errno (0 = ok)

(import "env" "gpio_read"
        (func (param i32 i32) (result i32)))
;; (handle, out_ptr) -> errno; writes u8 to out_ptr

(import "env" "gpio_write"
        (func (param i32 i32) (result i32)))
;; (handle, value) -> errno

;; I2C
(import "env" "i2c_open"
        (func (param i32) (result i32)))
;; (bus_id) -> handle (0 = error)

(import "env" "i2c_close"
        (func (param i32) (result i32)))

(import "env" "i2c_write_read"
        (func (param i32 i32 i32 i32 i32 i32) (result i32)))
;; (handle, addr, tx_ptr, tx_len, rx_ptr, rx_len) -> errno

;; UART
(import "env" "uart_open"
        (func (param i32 i32) (result i32)))
;; (port_id, baud) -> handle

(import "env" "uart_close" (func (param i32) (result i32)))

(import "env" "uart_write"
        (func (param i32 i32 i32) (result i32)))
;; (handle, ptr, len) -> errno

(import "env" "uart_read"
        (func (param i32 i32 i32 i32) (result i32)))
;; (handle, ptr, max_len, timeout_ms) -> bytes_read (negative on error)

;; ADC
(import "env" "adc_read"
        (func (param i32 i32) (result i32)))
;; (channel, out_ptr) -> errno; writes u32 sample to out_ptr

;; PWM
(import "env" "pwm_set"
        (func (param i32 i32 i32) (result i32)))
;; (channel, period_ns, duty_ns) -> errno

(import "env" "pwm_enable"
        (func (param i32 i32) (result i32)))
;; (channel, on) -> errno
```

The wasm3 host implements each as a function on the supervisor:

```rust
// supervisor/src/hal/raw_bindings.rs (extract)

#[cfg(feature = "wasm3-tier")]
pub fn register_raw_imports(
    module: &mut wasm3::Module<'_>,
    registry: Arc<HalRegistry>,
    app_id: AppId,
) -> Result<(), wasm3::Error> {
    let reg = registry.clone();
    let aid = app_id;
    module.link_closure("env", "gpio_open", move |
        _ctx,
        (chip_id, offset, mode): (i32, i32, i32),
    | -> i32 {
        let mode = match mode {
            0 => AccessMode::ExclusiveReadWrite,
            1 => AccessMode::SharedRead,
            2 => AccessMode::NotifyOnly,
            _ => return 0,
        };
        match reg.open_gpio(
            aid,
            GpioChipId(chip_id as u32),
            offset as u32,
            mode,
            "wasm3-app",
        ) {
            Ok(h) => h.handle_id as i32,
            Err(_) => 0,
        }
    })?;

    let reg2 = registry.clone();
    module.link_closure("env", "gpio_write", move |
        ctx,
        (handle, value): (i32, i32),
    | -> i32 {
        match reg2.gpio_write(handle as u32, value != 0) {
            Ok(()) => 0,
            Err(e) => errno_of(&e),
        }
    })?;

    // ... i2c_open, i2c_write_read, uart_*, adc_read, pwm_* ...

    Ok(())
}

fn errno_of(e: &HalError) -> i32 {
    match e {
        HalError::AccessDenied { .. } => -13,         // EACCES
        HalError::IoctlFailed { .. } => -5,           // EIO
        HalError::InvalidArgument { .. } => -22,       // EINVAL
        HalError::BusShutdown => -32,                  // EPIPE
        HalError::Timeout => -110,                     // ETIMEDOUT
        _ => -1,
    }
}
```

App code on `mcu-minimal` calls the raw imports directly:

```rust
// Example MCU app (compiled to core wasm, no WASI):

extern "C" {
    fn gpio_open(chip_id: i32, offset: i32, mode: i32) -> i32;
    fn gpio_write(handle: i32, value: i32) -> i32;
}

#[no_mangle]
pub extern "C" fn _start() {
    unsafe {
        let h = gpio_open(0, 4, 0); // chip 0, pin 4, exclusive
        if h != 0 {
            gpio_write(h, 1);
        }
    }
}
```

## 7. HAL Traits

The traits live in `supervisor/src/hal/{gpio,i2c,spi,uart,adc,pwm,audio,input}.rs`
and provide a common surface for both the WIT and raw bindings to call into.

```rust
// supervisor/src/hal/gpio.rs

use crate::hal::types::{HalResult, AccessMode};

pub trait GpioPin: Send + Sync {
    fn read(&self) -> HalResult<bool>;
    fn write(&self, value: bool) -> HalResult<()>;
    fn reconfigure_input(&self, bias: crate::hal::gpio_v2::BiasMode) -> HalResult<()>;
    fn reconfigure_output(&self, initial: bool) -> HalResult<()>;
    fn mode(&self) -> AccessMode;
    fn offset(&self) -> u32;
}

// supervisor/src/hal/i2c.rs

pub trait I2cBus: Send + Sync {
    fn read(&self, addr: u8, len: usize) -> HalResult<Vec<u8>>;
    fn write(&self, addr: u8, data: &[u8]) -> HalResult<()>;
    fn write_read(
        &self,
        addr: u8,
        write: &[u8],
        read_len: usize,
    ) -> HalResult<Vec<u8>>;
    fn bus_id(&self) -> u8;
}

// supervisor/src/hal/spi.rs

pub trait SpiBus: Send + Sync {
    fn transfer(
        &self,
        tx: &[u8],
        rx_len: usize,
        speed_hz: u32,
        cs_change: bool,
    ) -> HalResult<Vec<u8>>;
    fn bus_id(&self) -> u8;
}

// supervisor/src/hal/uart.rs

pub trait UartPort: Send + Sync {
    fn read(&self, max: usize, timeout_ms: u32) -> HalResult<Vec<u8>>;
    fn write(&self, data: &[u8]) -> HalResult<usize>;
    fn set_baud(&self, baud: u32) -> HalResult<()>;
    fn port_id(&self) -> u8;
}

// supervisor/src/hal/adc.rs

pub trait AdcChannel: Send + Sync {
    fn read(&self) -> HalResult<u32>;
    fn channel(&self) -> u8;
    fn resolution_bits(&self) -> u8;
}

// supervisor/src/hal/pwm.rs

pub trait PwmChannel: Send + Sync {
    fn set(&self, period_ns: u32, duty_ns: u32) -> HalResult<()>;
    fn enable(&self, on: bool) -> HalResult<()>;
    fn channel(&self) -> u8;
}

// supervisor/src/hal/audio.rs

pub trait AudioPcm: Send + Sync {
    fn write(&self, frames: &[i16]) -> HalResult<usize>;
    fn read(&self, frames: &mut [i16]) -> HalResult<usize>;
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u8;
}

// supervisor/src/hal/input.rs

pub trait InputDevice: Send + Sync {
    /// Returns the next input event, or None if none are available
    /// within `timeout_ms`.
    fn next_event(&self, timeout_ms: u32) -> HalResult<Option<InputEvent>>;
    fn device_id(&self) -> u32;
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    KeyPress { code: u16 },
    KeyRelease { code: u16 },
    MouseMove { dx: i16, dy: i16 },
    MouseButton { button: u8, pressed: bool },
    Touch { id: u8, x: i32, y: i32, pressed: bool },
}
```

## 8. HalRegistry

The registry is the single source of truth for "who owns what". It is
held by the supervisor in an `Arc<HalRegistry>` and shared with every
host function in both binding tiers.

```rust
// supervisor/src/hal/registry.rs

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::hal::bus_thread::{BusBackend, BusThread};
use crate::hal::types::{AccessMode, HalError, HalResult};
use crate::hal::safe_state::GpioSafeState;
use crate::hal::gpio::GpioPin;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpioChipId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct I2cBusId(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpiBusId(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UartPortId(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AdcChannelId(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PwmChannelId(pub u8);

pub struct GpioHolder {
    pub app_id: AppId,
    pub mode: AccessMode,
    pub handle_id: u64,
    pub safe_state: GpioSafeState,
    pub pin: Arc<dyn GpioPin>,
}

#[derive(Default)]
pub struct GpioEntry {
    pub holders: Vec<GpioHolder>,
}

pub struct HalRegistry {
    // Per-class locks (RwLock for many readers in SharedRead mode).
    gpio_owners: RwLock<HashMap<(GpioChipId, u32), GpioEntry>>,
    i2c_owners: RwLock<HashMap<I2cBusId, Vec<(AppId, AccessMode)>>>,
    spi_owners: RwLock<HashMap<SpiBusId, Vec<(AppId, AccessMode)>>>,
    uart_owners: RwLock<HashMap<UartPortId, Vec<(AppId, AccessMode)>>>,
    adc_owners: RwLock<HashMap<AdcChannelId, Vec<(AppId, AccessMode)>>>,
    pwm_owners: RwLock<HashMap<PwmChannelId, Vec<(AppId, AccessMode)>>>,

    // Bus threads.
    gpio_threads: RwLock<HashMap<GpioChipId, BusThread>>,
    i2c_threads: RwLock<HashMap<I2cBusId, BusThread>>,
    spi_threads: RwLock<HashMap<SpiBusId, BusThread>>,
    uart_threads: RwLock<HashMap<UartPortId, BusThread>>,
    adc_threads: RwLock<HashMap<AdcChannelId, BusThread>>,
    pwm_threads: RwLock<HashMap<PwmChannelId, BusThread>>,

    // Capability declarations from manifests, populated by Supervisor::load_apps.
    app_capabilities: RwLock<HashMap<AppId, AppCapabilities>>,

    // Monotonic handle id.
    next_handle: std::sync::atomic::AtomicU64,
}

#[derive(Default, Clone)]
pub struct AppCapabilities {
    pub gpio_pins: Vec<(GpioChipId, u32, GpioSafeState)>,
    pub i2c_buses: Vec<I2cBusId>,
    pub spi_buses: Vec<SpiBusId>,
    pub uart_ports: Vec<UartPortId>,
    pub adc_channels: Vec<AdcChannelId>,
    pub pwm_channels: Vec<PwmChannelId>,
}

impl HalRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            gpio_owners: RwLock::new(HashMap::new()),
            i2c_owners: RwLock::new(HashMap::new()),
            spi_owners: RwLock::new(HashMap::new()),
            uart_owners: RwLock::new(HashMap::new()),
            adc_owners: RwLock::new(HashMap::new()),
            pwm_owners: RwLock::new(HashMap::new()),
            gpio_threads: RwLock::new(HashMap::new()),
            i2c_threads: RwLock::new(HashMap::new()),
            spi_threads: RwLock::new(HashMap::new()),
            uart_threads: RwLock::new(HashMap::new()),
            adc_threads: RwLock::new(HashMap::new()),
            pwm_threads: RwLock::new(HashMap::new()),
            app_capabilities: RwLock::new(HashMap::new()),
            next_handle: std::sync::atomic::AtomicU64::new(1),
        })
    }

    pub fn next_handle_id(&self) -> u64 {
        self.next_handle
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn declare_app_capabilities(&self, app_id: AppId, caps: AppCapabilities) {
        self.app_capabilities.write().unwrap().insert(app_id, caps);
    }

    fn app_declares_gpio(&self, app_id: AppId, chip: GpioChipId, offset: u32) -> bool {
        let caps = self.app_capabilities.read().unwrap();
        match caps.get(&app_id) {
            Some(c) => c
                .gpio_pins
                .iter()
                .any(|(ch, off, _)| *ch == chip && *off == offset),
            None => false,
        }
    }

    pub fn release_all(&self, app_id: AppId) {
        // GPIO
        let mut g = self.gpio_owners.write().unwrap();
        for entry in g.values_mut() {
            entry.holders.retain(|h| h.app_id != app_id);
        }
        // I2C / SPI / UART / ADC / PWM follow the same pattern.
        for owners in [
            &self.i2c_owners,
            &self.spi_owners,
        ] {
            // (collapsed for brevity; same retain pattern)
        }
    }

    pub fn gpio_pins_held_by(
        &self,
        app_id: AppId,
    ) -> Vec<(GpioChipId, u32, GpioSafeState, Arc<dyn GpioPin>)> {
        let g = self.gpio_owners.read().unwrap();
        let mut out = vec![];
        for ((chip, off), entry) in g.iter() {
            for h in &entry.holders {
                if h.app_id == app_id {
                    out.push((*chip, *off, h.safe_state, h.pin.clone()));
                }
            }
        }
        out
    }
}
```

## 9. Platform Implementations

### 9.1 `desktop-full` and `server-headless`

| Subsystem | Backend                                  |
|-----------|------------------------------------------|
| Display   | DRM/KMS (`/dev/dri/card0`), virtio-gpu  |
| Audio     | ALSA PCM (`/dev/snd/pcmC0D0p`)           |
| Input     | evdev (`/dev/input/event*`)              |
| GPIO      | Not typically present; if user attaches  |
|           | USB-GPIO adapter, `/dev/gpiochip0` works |
| I2C/SPI   | Optional, via USB adapters               |

### 9.2 `iot-edge` and `robotics-rt`

| Subsystem | Backend                                  |
|-----------|------------------------------------------|
| GPIO      | `/dev/gpiochip0` (Pi: BCM 0–53)          |
| I2C       | `/dev/i2c-1` (Pi default)                |
| SPI       | `/dev/spidev0.0`, `/dev/spidev0.1`       |
| UART      | `/dev/ttyAMA0` (Pi BCM UART)             |
| ADC       | `/sys/bus/iio/devices/iio:device0/...`   |
|           | (no chardev ABI exists for ADC on Linux) |
| PWM       | `/sys/class/pwm/pwmchip0/pwm0/`          |
| Display   | DRM (Pi 4) or framebuffer (older)        |

### 9.3 `mobile`

| Subsystem | Backend                                  |
|-----------|------------------------------------------|
| Display   | DRM/KMS with rotation hint               |
| Audio     | ALSA                                     |
| Input     | evdev (touchscreen + virtual keyboard)   |
| Touch     | evdev multitouch protocol B              |
| GPIO      | Available but rarely exposed             |

### 9.4 `mcu-minimal`

| Subsystem | Backend                                  |
|-----------|------------------------------------------|
| Runtime   | wasm3 interpreter                        |
| GPIO      | Direct MMIO (`gpio_mmio.rs`)             |
| I2C       | MCU peripheral (`i2c_mmio.rs`)           |
| UART      | MCU peripheral (`uart_mmio.rs`)          |
| Display   | None (or tiny SPI OLED)                  |
| Audio     | None                                     |
| Input     | None                                     |

MMIO backends use `core::ptr::read_volatile` / `write_volatile` against
addresses provided by the platform profile TOML:

```rust
// supervisor/src/hal/gpio_mmio.rs (extract)

use core::ptr::{read_volatile, write_volatile};

pub struct GpioMmioBackend {
    base: usize, // base address of GPIO peripheral
}

impl GpioMmioBackend {
    pub unsafe fn map(base: usize, _pin_count: u8) -> HalResult<Self> {
        Ok(Self { base })
    }

    fn set_dir_output(&self, pin: u8) {
        unsafe {
            let moder = (self.base + 0x00) as *mut u32;
            let mut v = read_volatile(moder);
            let shift = (pin as u32) * 2;
            v &= !(0b11u32 << shift);
            v |= 0b01u32 << shift; // general-purpose output
            write_volatile(moder, v);
        }
    }

    fn write_pin(&self, pin: u8, value: bool) {
        unsafe {
            let bsrr = (self.base + 0x18) as *mut u32;
            let bit = if value { 1u32 << pin } else { 1u32 << (pin + 16) };
            write_volatile(bsrr, bit);
        }
    }

    fn read_pin(&self, pin: u8) -> bool {
        unsafe {
            let idr = (self.base + 0x10) as *const u32;
            (read_volatile(idr) >> pin) & 1 == 1
        }
    }
}
```

## 10. WIT Interface (`vyoma:hal@0.1.0`)

The WIT package is the contract for the wasmtime tier. Apps build
against it via `wit-bindgen` and the supervisor implements it via the
host side of `wit-bindgen-host`.

```wit
// supervisor/wit/vyoma-hal.wit

package vyoma:hal@0.1.0;

interface gpio {
    enum access-mode {
        exclusive-rw,
        shared-read,
        notify-only,
    }

    resource pin {
        open: static func(
            chip-id: u32,
            offset: u32,
            mode: access-mode,
        ) -> result<pin, string>;

        read: func() -> result<bool, string>;
        write: func(value: bool) -> result<_, string>;
    }
}

interface i2c {
    resource bus {
        open: static func(bus-id: u8) -> result<bus, string>;
        write: func(addr: u8, data: list<u8>) -> result<_, string>;
        read:  func(addr: u8, len: u32) -> result<list<u8>, string>;
        write-read: func(
            addr: u8,
            write-data: list<u8>,
            read-len: u32,
        ) -> result<list<u8>, string>;
    }
}

interface spi {
    resource bus {
        open: static func(bus-id: u8) -> result<bus, string>;
        transfer: func(
            tx: list<u8>,
            rx-len: u32,
            speed-hz: u32,
            cs-change: bool,
        ) -> result<list<u8>, string>;
    }
}

interface uart {
    resource port {
        open: static func(port-id: u8, baud: u32) -> result<port, string>;
        read:  func(max: u32, timeout-ms: u32) -> result<list<u8>, string>;
        write: func(data: list<u8>) -> result<u32, string>;
        set-baud: func(baud: u32) -> result<_, string>;
    }
}

interface adc {
    resource channel {
        open: static func(channel-id: u8) -> result<channel, string>;
        read: func() -> result<u32, string>;
    }
}

interface pwm {
    resource channel {
        open: static func(channel-id: u8) -> result<channel, string>;
        set:    func(period-ns: u32, duty-ns: u32) -> result<_, string>;
        enable: func(on: bool) -> result<_, string>;
    }
}

world app-hal {
    import gpio;
    import i2c;
    import spi;
    import uart;
    import adc;
    import pwm;
}
```

## 11. Implementation Files

All files are in `supervisor/src/hal/` and obey the 500-LOC ceiling.

| File              | Target LOC | Purpose                                      |
|-------------------|-----------:|----------------------------------------------|
| `mod.rs`          | 220        | Public API; tier dispatch; `init()`          |
| `types.rs`        | 280        | `AccessMode`, `HalRequest`, `HalResult`, errors |
| `registry.rs`     | 460        | `HalRegistry`; ownership tables; release_all |
| `bus_thread.rs`   | 180        | `BusBackend` trait; `BusThread::spawn`       |
| `gpio.rs`         | 180        | `GpioPin` trait; access mode plumbing        |
| `gpio_v2.rs`      | 480        | GPIO_V2 chardev ioctls; line handles         |
| `gpio_mmio.rs`    | 220        | MCU MMIO GPIO backend                        |
| `i2c.rs`          | 90         | `I2cBus` trait                               |
| `i2c_linux.rs`    | 300        | `/dev/i2c-*` backend with `I2C_RDWR` ioctl   |
| `i2c_mmio.rs`     | 240        | MCU I2C peripheral driver                    |
| `spi.rs`          | 70         | `SpiBus` trait                               |
| `spi_linux.rs`    | 320        | `/dev/spidev*` `SPI_IOC_MESSAGE` backend     |
| `uart.rs`         | 90         | `UartPort` trait                             |
| `uart_linux.rs`   | 380        | termios + read/write backend                 |
| `uart_mmio.rs`    | 200        | MCU UART peripheral driver                   |
| `adc.rs`          | 70         | `AdcChannel` trait                           |
| `adc_iio.rs`      | 240        | `/sys/bus/iio/devices/...` backend           |
| `pwm.rs`          | 70         | `PwmChannel` trait                           |
| `pwm_sysfs.rs`    | 260        | `/sys/class/pwm/*` backend                   |
| `audio.rs`        | 80         | `AudioPcm` trait                             |
| `audio_alsa.rs`   | 420        | ALSA PCM playback + capture                  |
| `input.rs`        | 110        | `InputDevice` trait; `InputEvent` enum       |
| `input_evdev.rs`  | 360        | evdev event-decoding backend                 |
| `safe_state.rs`   | 200        | `GpioSafeState` enum + manifest parse        |
| `wit_bindings.rs` | 440        | `vyoma:hal@0.1.0` host implementation        |
| `raw_bindings.rs` | 380        | `env.*` import table for wasm3 tier          |

Total HAL crate: ~6,540 LOC across 26 files, no single file > 480 LOC.

## Critical v1 Requirements

The following must ship in v1 (target: spec branches `046-*` through `051-*`):

- **C1 satisfied:** No code path in the supervisor calls `/sys/class/gpio/export`.
  The `build.rs` probe rejects sysfs-only kernel configs at compile time.
  GPIO_V2 chardev ioctls (`GPIO_V2_GET_LINE_IOCTL`,
  `GPIO_V2_LINE_GET_VALUES_IOCTL`, `GPIO_V2_LINE_SET_VALUES_IOCTL`) cover
  100 % of GPIO traffic.
- **C2 satisfied:** Every HAL bus has its own `std::thread` plus a
  `crossbeam::channel::bounded(256)` work queue. Unit tests assert that the
  IPC event loop's `recv_timeout(0)` never blocks longer than 100 µs even
  with a synthetic slow I²C backend that sleeps 50 ms per transaction.
- **C3 satisfied:** Both bindings ship together. `vyoma:hal@0.1.0` is the
  primary surface; `env.*` raw imports document the `mcu-minimal` ABI in
  the same `wit/` directory under `raw-imports.md`.
- **C4 satisfied:** `AccessMode` enum implemented; compatibility matrix
  enforced by `HalRegistry::open_gpio`. Tests cover all 16 cells of the
  matrix in §4.2.
- **C5 satisfied:** Manifest schema includes `safe_state` per pin; the
  `on_instance_gone` hook fires on every exit reason (clean, crashed,
  killed by user, killed by watchdog). Tests confirm that a crashed
  motor-controller app leaves its declared output pins at `output_low`.
- **C6 satisfied:** Two Cargo features only (`wasmtime-tier`,
  `wasm3-tier`). All other backends compile unconditionally inside their
  tier. Platform selection is fully runtime via `PlatformProfile`. A
  matrix CI job builds both tiers on every PR.
- **Per-class RwLock granularity:** Ownership tables use one `RwLock` per
  peripheral class, not a global mutex.
- **Manifest schema additions:** `safe_state` (per pin), per-pin direction
  override, plus the existing `pins`, `i2c.bus`, `spi.bus`, `uart.port`,
  `adc.channel`, `pwm.channel` fields documented in `vyoma-manifest-schema.md`.
- **Unit tests for every backend:** Mock backends implementing the same
  traits, used by the registry tests, so CI exercises ownership logic
  without real hardware.
- **Integration test on real Pi 4:** A QA box runs the iot-edge build
  against an attached BME280 sensor (I²C) and an LED (GPIO 17) for every
  release tag.

## Deferred to v2

- **Edge-triggered GPIO events.** Polling-only in v1. `NotifyOnly` access
  mode is reserved in the enum but rejects with `NotImplemented`.
- **I²C target (slave) mode.** Master-only in v1.
- **SPI multi-master arbitration.** Single-master only; the registry
  forces `ExclusiveReadWrite` on every SPI bus open.
- **USB device API.** No `vyoma:hal/usb` interface yet. USB sound cards
  surface as ALSA PCM and USB serial adapters surface as UART, but raw
  USB endpoints are out.
- **Camera (V4L2).** Deferred; cameras are big enough to warrant their
  own spec round.
- **CAN bus.** Deferred to robotics v2 (`socketcan`).
- **Hardware random number generator passthrough.** Apps that need
  randomness use WASI `random` for now.
- **Interrupt latency guarantees.** v2 robotics-rt will pin bus threads
  to isolated cores and use `SCHED_FIFO`. v1 uses the default scheduler.
- **GPIO debounce flags.** GPIO_V2 supports them; v1 doesn't expose them
  through the trait yet.

## Explicitly NEVER

- **Sysfs GPIO** (`/sys/class/gpio/export`, `/sys/class/gpio/gpioN/*`).
  Even as a fallback. Even for legacy boards. If the kernel is too old to
  ship GPIO_V2, VyomaOS refuses to run on it.
- **Per-platform Cargo features.** Never `desktop-full`, `iot-edge`,
  `robotics-rt` as features. Always runtime profiles.
- **Blocking the IPC event loop on any HAL syscall.** Even a 10 µs
  `read()` goes through the per-bus thread. No exceptions.
- **Closures in `HalRequest`.** No `Box<dyn FnOnce>` on the hot path. The
  enum is the API.
- **Direct `unsafe` MMIO from Wasmtime-tier code.** MMIO is exclusively a
  `wasm3-tier` (mcu-minimal) concern. Trying to add an MMIO backend on a
  Linux platform is a sign you're working around a missing kernel driver,
  not extending the HAL.
- **Implicit pin grabbing.** A WASM app cannot acquire a GPIO pin it did
  not declare in its manifest. The supervisor checks `app_declares_gpio`
  on every `open` call.
- **Cross-app handle sharing.** Handles are not transferable. An app
  cannot pass a `gpio::pin` resource to another app via IPC. Each app
  opens its own resources.
- **Skipping safe-state on "clean" exits.** Even when an app calls
  `wasi:exit` with code 0, the supervisor still applies safe state on
  every held pin. The supervisor cannot trust the app's intent; the
  manifest is the source of truth.
