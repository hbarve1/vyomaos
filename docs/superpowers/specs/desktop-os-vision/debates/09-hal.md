# Round 9 — Hardware Abstraction Layer (HAL)

**Role:** Architect
**Date:** 2026-05-29
**Status:** Proposal (awaiting Critic review)
**Scope:** Platform-portable hardware access for VyomaOS supervisor + WASM apps across 6 platform profiles (desktop-full, iot-edge, robotics-rt, mobile, mcu-minimal, server-headless)

---

## 0. Executive summary

VyomaOS's Hardware Abstraction Layer (HAL) is the thin, portable layer between the Linux kernel (or bare metal, on `mcu-minimal`) and every VyomaOS subsystem that touches a physical or virtual peripheral. It sits **below** the driver model defined in R6 (DeviceManager / udev / three-tier drivers) and **above** the raw kernel ABI (`/sys/class/gpio`, `/dev/i2c-*`, `/dev/spidev*`, ioctls, MMIO registers).

Where R6 answered *"how do drivers get loaded, sandboxed, and hot-plugged?"*, R9 answers *"once the driver is loaded, what does it look like to the rest of the supervisor and to a WASM app on a given platform?"*. The HAL gives every subsystem a single Rust trait per peripheral class — `GpioPin`, `I2cBus`, `SpiBus`, `UartPort`, `AdcChannel`, `PwmChannel`, `DisplayDevice`, `AudioDevice`, `InputDevice` — and a per-platform implementation of those traits that compiles into the supervisor binary based on Cargo feature flags matching `PLATFORM=` in the Makefile.

The macOS analogue is **IOKit** plus the platform's **Board Support Package (BSP)**: a uniform Objective-C/Swift API (`IOService`, `IOHIDManager`, `CoreAudio`) backed by family-specific drivers (`IOUSBFamily`, `IOGraphicsFamily`, `AppleSiliconPlatform`). VyomaOS's HAL takes the same shape — uniform Rust trait API, platform-specific implementations — but with a much smaller, capability-secure surface area.

Key constraints honoured:

- **Rust-first.** All HAL implementations are safe Rust (`unsafe` only at the MMIO and ioctl boundary, audited).
- **500-line file limit.** HAL is split into `traits.rs`, `registry.rs`, `error.rs`, and one file per platform under `platform/`.
- **No filtering layer.** Capabilities map 1:1 to HAL handles. If a manifest doesn't declare `gpio_pins = [4]`, the WASM app gets no `GpioPin` resource for pin 4.
- **Exclusive ownership.** One app owns one pin/bus/port at a time. The `PeripheralRegistry` from CLAUDE.md enforces this; the HAL is the storage for the actual handle.
- **BootPhase aware.** HAL is initialised in two phases (R8): `init_early` registers the trait objects with the registry; `init_late` opens kernel file descriptors and arms interrupts.

The end state: every subsystem that previously called `std::fs::OpenOptions::new().open("/sys/class/gpio/gpio4/value")` now calls `hal.gpio(4)?.write(true)?`, and switching from `desktop-full` to `iot-edge` only changes which file in `supervisor/src/hal/platform/` is compiled in — no caller code changes.

---

## 1. HAL philosophy

### 1.1 What the HAL is

The HAL is **the only place in the supervisor that knows how Linux talks to hardware**. Every other subsystem — the compositor, the audio mixer, the input router, the WASM peripheral hostcalls, the thermal governor (R7), the package manager — speaks to the HAL through trait objects (`Arc<dyn GpioPin>`, `Arc<dyn DisplayDevice>`, etc.).

This gives us three properties that we cannot get any other way:

1. **Platform portability.** The same compositor code runs on QEMU virtio-gpu, RPi BCM2711 DSI, and a robotics SoC's eDP panel, because all three return `Arc<dyn DisplayDevice>`.
2. **Test isolation.** A `MockGpioPin` implementing the same trait lets us run supervisor unit tests on a developer's MacBook (no `/dev/gpiochip0`) and still cover the GPIO routing logic.
3. **Capability enforcement choke point.** Every peripheral handle is minted in exactly one place — `HalRegistry::open_*` — and that function consults the `PeripheralRegistry` before handing out the `Arc`. There is no back door.

### 1.2 What the HAL is not

The HAL is **not** a device driver. It does not parse register maps, implement protocol state machines, or run timing-critical bit-banging. Those live in:

- **Kernel drivers** (Linux 5.10, e.g., `i2c-bcm2835`, `spi-bcm2835`) when the chip is supported upstream.
- **R6 in-supervisor Rust drivers** (e.g., a `Bme280` sensor driver) that *use* the HAL's `I2cBus` trait to talk to the chip.
- **R6 privileged WASM drivers** (e.g., a userspace-driver replacement for a sensor) that import the HAL's WIT interface.

The HAL is the *narrow waist*: kernel ↔ HAL ↔ driver/subsystem ↔ app.

### 1.3 What about non-peripheral hardware?

The HAL handles **everything mediated by the kernel as a device**: GPIO, I2C, SPI, UART, ADC, PWM, framebuffer/DRM, ALSA PCM, evdev, IIO, V4L2, IPMI, watchdog. It does **not** handle:

- **CPU and memory.** Those are kernel + R8 boot concerns.
- **Filesystems.** That's R4.
- **Network sockets.** WASI sockets passes them through; the HAL is bypassed.
- **Clocks/timers from the supervisor's perspective.** `std::time::Instant` is fine; we don't need a `ClockDevice` trait.

The line is: "if it has a `/dev/*` or `/sys/*` path or an MMIO register, it's HAL; otherwise it's not."

### 1.4 Layering picture

```
┌─────────────────────────────────────────────────────────────┐
│  WASM apps  (wasm32-wasip2)                                  │
│   import vyoma:hal/{gpio,i2c,spi,uart,adc,pwm}               │
└─────────────────────────────────────────────────────────────┘
                       │  WIT host functions
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  Supervisor hostcalls  (supervisor/src/hostcalls/hal.rs)     │
│   look up HalRegistry, check PeripheralRegistry, dispatch    │
└─────────────────────────────────────────────────────────────┘
                       │  Rust trait calls
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  HAL traits  (supervisor/src/hal/traits.rs)                  │
│   GpioPin, I2cBus, SpiBus, UartPort, AdcChannel, PwmChannel  │
│   DisplayDevice, AudioDevice, InputDevice, ThermalSensor     │
└─────────────────────────────────────────────────────────────┘
                       │  trait object dispatch
                       ▼
┌──────────────┬───────────────┬───────────────┬──────────────┐
│ desktop.rs   │ arm64_sysfs.rs│ mcu.rs        │ mock.rs      │
│ (stub + DRM  │ (libgpiod,    │ (MMIO regs,   │ (unit tests) │
│  + ALSA)     │  ALSA, evdev) │  no kernel)   │              │
└──────────────┴───────────────┴───────────────┴──────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  Linux kernel 5.10 sysfs/devtmpfs/DRM/ALSA/evdev/IIO/V4L2    │
│   (or bare metal for mcu-minimal)                            │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Core HAL traits

All trait definitions live in `supervisor/src/hal/traits.rs`. Each trait is `Send + Sync` so handles can cross threads (the supervisor is multi-threaded; one thread per app per R8). Each method returns `Result<T, HalError>` — never panics, never returns `String` (we replace the existing `String` error type with a structured enum).

### 2.1 Error type

```rust
// supervisor/src/hal/error.rs

use std::io;

/// Every HAL operation returns `Result<T, HalError>`. The variants are
/// designed so that the WIT host-call layer can losslessly map them to
/// `hal-error` enum cases for WASM apps.
#[derive(Debug, thiserror::Error)]
pub enum HalError {
    /// Peripheral does not exist on this platform.
    /// e.g., calling `hal.gpio(4)` on `desktop-full`.
    #[error("peripheral {kind} #{id} not present on this platform")]
    NotPresent { kind: PeripheralKind, id: u32 },

    /// Peripheral exists but is already owned by another app.
    #[error("peripheral {kind} #{id} busy (owned by app {owner})")]
    Busy {
        kind: PeripheralKind,
        id: u32,
        owner: String,
    },

    /// Caller has no capability to access this peripheral.
    /// (manifest didn't declare it; checked by PeripheralRegistry)
    #[error("no capability for {kind} #{id}")]
    PermissionDenied { kind: PeripheralKind, id: u32 },

    /// Bad parameter (out-of-range pin number, unsupported baud rate, …).
    #[error("invalid argument: {0}")]
    InvalidArg(String),

    /// I/O failure from the underlying kernel interface.
    #[error("I/O error on {kind} #{id}: {source}")]
    Io {
        kind: PeripheralKind,
        id: u32,
        #[source]
        source: io::Error,
    },

    /// Operation timed out (UART read, I2C arbitration, etc.).
    #[error("timeout on {kind} #{id} after {ms} ms")]
    Timeout {
        kind: PeripheralKind,
        id: u32,
        ms: u32,
    },

    /// Protocol error from the device (NACK on I2C, parity error on UART).
    #[error("protocol error on {kind} #{id}: {0}")]
    Protocol(PeripheralKind, u32, String),

    /// The HAL implementation is not yet wired up (developer placeholder).
    /// Should never be seen in production.
    #[error("HAL operation unimplemented: {0}")]
    Unimplemented(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeripheralKind {
    Gpio,
    I2c,
    Spi,
    Uart,
    Adc,
    Pwm,
    Display,
    Audio,
    Input,
    Thermal,
}

impl std::fmt::Display for PeripheralKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PeripheralKind::Gpio => "gpio",
            PeripheralKind::I2c => "i2c",
            PeripheralKind::Spi => "spi",
            PeripheralKind::Uart => "uart",
            PeripheralKind::Adc => "adc",
            PeripheralKind::Pwm => "pwm",
            PeripheralKind::Display => "display",
            PeripheralKind::Audio => "audio",
            PeripheralKind::Input => "input",
            PeripheralKind::Thermal => "thermal",
        };
        f.write_str(s)
    }
}
```

### 2.2 GPIO

```rust
// supervisor/src/hal/traits.rs (GPIO section)

use crate::hal::error::HalError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioDirection {
    Input,
    InputPullUp,
    InputPullDown,
    Output,
    OutputOpenDrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioEdge {
    Rising,
    Falling,
    Both,
}

/// One physical GPIO pin. The caller obtained this from
/// `HalRegistry::open_gpio(pin)` after capability check.
pub trait GpioPin: Send + Sync {
    /// Stable u8 pin identifier (BCM numbering on RPi, linear index on MCU).
    fn id(&self) -> u8;

    /// Configure direction. Idempotent.
    fn set_direction(&self, dir: GpioDirection) -> Result<(), HalError>;

    /// Read current logical level.
    fn read(&self) -> Result<bool, HalError>;

    /// Drive output level.
    fn write(&self, high: bool) -> Result<(), HalError>;

    /// Toggle output level (read-modify-write; not atomic across threads).
    fn toggle(&self) -> Result<(), HalError> {
        let cur = self.read()?;
        self.write(!cur)
    }

    /// Register an interrupt callback. `cb` is invoked from the HAL's
    /// dedicated interrupt thread (not the caller's thread).
    /// Replaces any previous callback. Pass `None` to disarm.
    fn set_interrupt(
        &self,
        edge: GpioEdge,
        cb: Option<Box<dyn Fn(bool) + Send + Sync>>,
    ) -> Result<(), HalError>;
}
```

### 2.3 I2C

```rust
// supervisor/src/hal/traits.rs (I2C section)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I2cSpeed {
    Standard100k,
    Fast400k,
    FastPlus1m,
    HighSpeed3m4,
}

pub trait I2cBus: Send + Sync {
    fn id(&self) -> u8;

    /// Set bus speed. Bus must be otherwise quiescent.
    fn set_speed(&self, speed: I2cSpeed) -> Result<(), HalError>;

    /// Write `data` to 7-bit slave `addr`.
    fn write(&self, addr: u8, data: &[u8]) -> Result<(), HalError>;

    /// Read `read.len()` bytes from slave `addr`.
    fn read(&self, addr: u8, read: &mut [u8]) -> Result<(), HalError>;

    /// Atomic write-then-read with repeated-start (register read pattern).
    fn write_read(
        &self,
        addr: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), HalError>;

    /// SMBus quick-command device probe. Returns `true` if a device ACKs.
    fn probe(&self, addr: u8) -> Result<bool, HalError>;
}
```

### 2.4 SPI

```rust
// supervisor/src/hal/traits.rs (SPI section)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiMode {
    Mode0, // CPOL=0, CPHA=0
    Mode1, // CPOL=0, CPHA=1
    Mode2, // CPOL=1, CPHA=0
    Mode3, // CPOL=1, CPHA=1
}

pub struct SpiConfig {
    pub mode: SpiMode,
    pub bits_per_word: u8,    // typically 8
    pub max_hz: u32,
    pub lsb_first: bool,
    pub cs_active_high: bool,
}

pub trait SpiBus: Send + Sync {
    fn id(&self) -> u8;

    /// Configure for a given chip select. Different CS lines may use
    /// different SpiConfig.
    fn configure(&self, cs: u8, cfg: &SpiConfig) -> Result<(), HalError>;

    /// Full-duplex transfer on `cs`. `tx` and `rx` must be equal length.
    fn transfer(&self, cs: u8, tx: &[u8], rx: &mut [u8]) -> Result<(), HalError>;

    /// Write-only transfer (rx ignored).
    fn write(&self, cs: u8, tx: &[u8]) -> Result<(), HalError>;
}
```

### 2.5 UART

```rust
// supervisor/src/hal/traits.rs (UART section)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UartParity {
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UartStopBits {
    One,
    Two,
}

pub struct UartConfig {
    pub baud: u32,
    pub data_bits: u8, // 5..=9
    pub parity: UartParity,
    pub stop: UartStopBits,
    pub flow_control_rts_cts: bool,
}

pub trait UartPort: Send + Sync {
    fn id(&self) -> u8;

    fn configure(&self, cfg: &UartConfig) -> Result<(), HalError>;

    /// Blocking write. Returns when all bytes are queued to the FIFO.
    fn write(&self, data: &[u8]) -> Result<usize, HalError>;

    /// Read up to `buf.len()` bytes, blocking up to `timeout_ms`.
    /// Returns 0 on timeout (not an error).
    fn read(&self, buf: &mut [u8], timeout_ms: u32) -> Result<usize, HalError>;

    /// Flush the transmit FIFO synchronously.
    fn flush(&self) -> Result<(), HalError>;
}
```

### 2.6 ADC / PWM

```rust
// supervisor/src/hal/traits.rs (ADC/PWM section)

pub trait AdcChannel: Send + Sync {
    fn id(&self) -> u8;
    fn resolution_bits(&self) -> u8;
    fn vref_mv(&self) -> u32;

    /// Raw sample. Range: 0..=(2^resolution_bits - 1).
    fn read_raw(&self) -> Result<u16, HalError>;

    /// Convenience: convert raw → millivolts.
    fn read_mv(&self) -> Result<u32, HalError> {
        let raw = self.read_raw()? as u32;
        let max = (1u32 << self.resolution_bits()) - 1;
        Ok((raw * self.vref_mv()) / max)
    }
}

pub trait PwmChannel: Send + Sync {
    fn id(&self) -> u8;

    /// Set period in nanoseconds.
    fn set_period_ns(&self, period_ns: u32) -> Result<(), HalError>;

    /// Set duty cycle in nanoseconds (must be ≤ period).
    fn set_duty_ns(&self, duty_ns: u32) -> Result<(), HalError>;

    fn enable(&self) -> Result<(), HalError>;
    fn disable(&self) -> Result<(), HalError>;
}
```

### 2.7 Higher-level peripherals (display, audio, input)

These are not in the existing `supervisor/src/hal/mod.rs` but the architecture requires them — the compositor and audio mixer need a HAL abstraction too, otherwise we still hard-code `/dev/fb0` paths.

```rust
// supervisor/src/hal/traits.rs (high-level peripherals)

pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    pub pixel_format: PixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Xrgb8888,
    Argb8888,
    Rgb565,
}

pub trait DisplayDevice: Send + Sync {
    fn id(&self) -> u8;
    fn list_modes(&self) -> Result<Vec<DisplayMode>, HalError>;
    fn set_mode(&self, mode: &DisplayMode) -> Result<(), HalError>;

    /// Acquire a writable backing buffer for the current mode.
    /// Returns `(ptr, stride, len)`. Backed by a DRM dumb buffer on
    /// Linux DRM, by a mapped framebuffer on fbdev, by an MMIO region
    /// on mcu-minimal.
    fn map_buffer(&self) -> Result<DisplayBuffer, HalError>;

    /// Page-flip / commit the buffer to the scanout engine.
    fn flush(&self, buf: &DisplayBuffer) -> Result<(), HalError>;
}

pub struct DisplayBuffer {
    pub ptr: *mut u8,
    pub stride: usize,
    pub len: usize,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

// DisplayBuffer is Send+Sync because the supervisor owns it and
// the pointer is to a kernel-mapped region that survives thread moves.
unsafe impl Send for DisplayBuffer {}
unsafe impl Sync for DisplayBuffer {}

pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub sample_format: SampleFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    S16Le,
    F32Le,
}

pub trait AudioDevice: Send + Sync {
    fn id(&self) -> u8;
    fn open_playback(&self, fmt: &AudioFormat) -> Result<Box<dyn AudioStream>, HalError>;
    fn open_capture(&self, fmt: &AudioFormat) -> Result<Box<dyn AudioStream>, HalError>;
}

pub trait AudioStream: Send + Sync {
    fn write(&self, samples: &[u8]) -> Result<usize, HalError>;
    fn read(&self, samples: &mut [u8]) -> Result<usize, HalError>;
    fn drain(&self) -> Result<(), HalError>;
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    Key { code: u16, pressed: bool },
    MouseMove { dx: i32, dy: i32 },
    MouseButton { button: u8, pressed: bool },
    Touch { id: u32, x: i32, y: i32, pressed: bool },
}

pub trait InputDevice: Send + Sync {
    fn id(&self) -> u8;
    fn name(&self) -> &str;
    /// Blocking next-event with timeout.
    fn poll(&self, timeout_ms: u32) -> Result<Option<InputEvent>, HalError>;
}

pub trait ThermalSensor: Send + Sync {
    fn id(&self) -> u8;
    fn name(&self) -> &str;
    /// Read temperature in millidegrees Celsius.
    fn read_mc(&self) -> Result<i32, HalError>;
}
```

The 500-line limit is the reason `traits.rs` is allowed to grow only to about 480 lines as shown; the high-level peripherals above split out to `traits_av.rs` if it gets close.

---

## 3. HAL Registry

`HalRegistry` is the single owner of every HAL handle. It is created once during R8's `init_early` boot phase, populated by the platform module, and shared `Arc<HalRegistry>` to every subsystem and to the WASM hostcall layer.

```rust
// supervisor/src/hal/registry.rs

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;

use crate::hal::error::{HalError, PeripheralKind};
use crate::hal::traits::*;
use crate::capability::PeripheralRegistry;

pub struct HalRegistry {
    // Low-level peripherals
    gpio: RwLock<HashMap<u8, Arc<dyn GpioPin>>>,
    i2c: RwLock<HashMap<u8, Arc<dyn I2cBus>>>,
    spi: RwLock<HashMap<u8, Arc<dyn SpiBus>>>,
    uart: RwLock<HashMap<u8, Arc<dyn UartPort>>>,
    adc: RwLock<HashMap<u8, Arc<dyn AdcChannel>>>,
    pwm: RwLock<HashMap<u8, Arc<dyn PwmChannel>>>,

    // High-level peripherals
    display: RwLock<HashMap<u8, Arc<dyn DisplayDevice>>>,
    audio: RwLock<HashMap<u8, Arc<dyn AudioDevice>>>,
    input: RwLock<HashMap<u8, Arc<dyn InputDevice>>>,
    thermal: RwLock<HashMap<u8, Arc<dyn ThermalSensor>>>,

    // Capability enforcement (R6/R8: PeripheralRegistry)
    caps: Arc<PeripheralRegistry>,
}

impl HalRegistry {
    pub fn new(caps: Arc<PeripheralRegistry>) -> Self {
        Self {
            gpio: RwLock::new(HashMap::new()),
            i2c: RwLock::new(HashMap::new()),
            spi: RwLock::new(HashMap::new()),
            uart: RwLock::new(HashMap::new()),
            adc: RwLock::new(HashMap::new()),
            pwm: RwLock::new(HashMap::new()),
            display: RwLock::new(HashMap::new()),
            audio: RwLock::new(HashMap::new()),
            input: RwLock::new(HashMap::new()),
            thermal: RwLock::new(HashMap::new()),
            caps,
        }
    }

    // ── Registration (called from platform init) ─────────────────────────
    pub fn register_gpio(&self, pin: u8, drv: Arc<dyn GpioPin>) {
        self.gpio.write().insert(pin, drv);
    }
    pub fn register_i2c(&self, bus: u8, drv: Arc<dyn I2cBus>) {
        self.i2c.write().insert(bus, drv);
    }
    pub fn register_spi(&self, bus: u8, drv: Arc<dyn SpiBus>) {
        self.spi.write().insert(bus, drv);
    }
    pub fn register_uart(&self, port: u8, drv: Arc<dyn UartPort>) {
        self.uart.write().insert(port, drv);
    }
    pub fn register_adc(&self, ch: u8, drv: Arc<dyn AdcChannel>) {
        self.adc.write().insert(ch, drv);
    }
    pub fn register_pwm(&self, ch: u8, drv: Arc<dyn PwmChannel>) {
        self.pwm.write().insert(ch, drv);
    }
    pub fn register_display(&self, id: u8, drv: Arc<dyn DisplayDevice>) {
        self.display.write().insert(id, drv);
    }
    pub fn register_audio(&self, id: u8, drv: Arc<dyn AudioDevice>) {
        self.audio.write().insert(id, drv);
    }
    pub fn register_input(&self, id: u8, drv: Arc<dyn InputDevice>) {
        self.input.write().insert(id, drv);
    }
    pub fn register_thermal(&self, id: u8, drv: Arc<dyn ThermalSensor>) {
        self.thermal.write().insert(id, drv);
    }

    // ── Capability-checked open ──────────────────────────────────────────
    // Every open_* call goes through PeripheralRegistry to:
    //  1. Verify the calling app has the capability in its manifest.
    //  2. Atomically claim exclusive ownership.
    // Subsystems that aren't apps (e.g., compositor) pass app_id="system".

    pub fn open_gpio(&self, app_id: &str, pin: u8) -> Result<Arc<dyn GpioPin>, HalError> {
        let map = self.gpio.read();
        let drv = map.get(&pin).cloned().ok_or(HalError::NotPresent {
            kind: PeripheralKind::Gpio,
            id: pin as u32,
        })?;
        self.caps.claim_gpio(app_id, pin).map_err(|e| match e {
            crate::capability::CapError::NoCapability => HalError::PermissionDenied {
                kind: PeripheralKind::Gpio,
                id: pin as u32,
            },
            crate::capability::CapError::Busy(owner) => HalError::Busy {
                kind: PeripheralKind::Gpio,
                id: pin as u32,
                owner,
            },
        })?;
        Ok(drv)
    }

    pub fn open_i2c(&self, app_id: &str, bus: u8) -> Result<Arc<dyn I2cBus>, HalError> {
        let map = self.i2c.read();
        let drv = map.get(&bus).cloned().ok_or(HalError::NotPresent {
            kind: PeripheralKind::I2c,
            id: bus as u32,
        })?;
        self.caps.claim_i2c(app_id, bus).map_err(|e| match e {
            crate::capability::CapError::NoCapability => HalError::PermissionDenied {
                kind: PeripheralKind::I2c,
                id: bus as u32,
            },
            crate::capability::CapError::Busy(owner) => HalError::Busy {
                kind: PeripheralKind::I2c,
                id: bus as u32,
                owner,
            },
        })?;
        Ok(drv)
    }

    // open_spi, open_uart, open_adc, open_pwm follow the same shape; elided.

    /// Open the primary display. Shared, not exclusive (compositor owns it,
    /// apps draw into per-window surfaces blitted by the compositor).
    pub fn open_display(&self, id: u8) -> Result<Arc<dyn DisplayDevice>, HalError> {
        self.display
            .read()
            .get(&id)
            .cloned()
            .ok_or(HalError::NotPresent {
                kind: PeripheralKind::Display,
                id: id as u32,
            })
    }

    pub fn open_input(&self, id: u8) -> Result<Arc<dyn InputDevice>, HalError> {
        self.input
            .read()
            .get(&id)
            .cloned()
            .ok_or(HalError::NotPresent {
                kind: PeripheralKind::Input,
                id: id as u32,
            })
    }

    /// On app death (R8 shutdown hook), release every claim held by app_id.
    /// The Arc<dyn GpioPin> handle inside the wasm instance is also dropped
    /// when the instance is torn down, so the underlying resource is freed.
    pub fn release_all(&self, app_id: &str) {
        self.caps.release_all(app_id);
    }
}
```

### 3.1 Initialisation flow (R8 BootPhase)

```rust
// supervisor/src/boot.rs (excerpt)

pub fn init_early(profile: &PlatformProfile, caps: Arc<PeripheralRegistry>)
    -> Result<Arc<HalRegistry>, BootError>
{
    let registry = Arc::new(HalRegistry::new(caps));
    // Dispatch to the per-platform module that's compiled in.
    crate::hal::platform::init_early(&registry, profile)?;
    Ok(registry)
}

pub fn init_late(registry: &Arc<HalRegistry>, profile: &PlatformProfile)
    -> Result<(), BootError>
{
    // Open kernel FDs, arm interrupts. Anything that can fail at runtime.
    crate::hal::platform::init_late(registry, profile)?;
    Ok(())
}
```

The split matters: `init_early` is infallible and registers stub/struct handles. `init_late` is where we actually `open("/dev/i2c-1", O_RDWR)`. If that fails, only one peripheral is degraded — the rest of the system still boots, and subsystems get `HalError::NotPresent` for the missing piece.

---

## 4. Platform implementations

Each platform compiles in one and only one platform module via Cargo features that match `PLATFORM=`:

```toml
# supervisor/Cargo.toml
[features]
default = ["platform-desktop"]
platform-desktop        = []
platform-iot-edge       = ["libgpiod-sys", "linux-embedded-hal"]
platform-robotics-rt    = ["libgpiod-sys", "linux-embedded-hal", "thread-priority"]
platform-mobile         = ["libgpiod-sys", "evdev", "v4l2"]
platform-mcu-minimal    = ["volatile-register", "cortex-m"]
platform-server-headless = []
```

The Makefile maps `PLATFORM=iot-edge` to `cargo build --no-default-features --features platform-iot-edge`.

### 4.1 `desktop-full` — `supervisor/src/hal/platform/desktop.rs`

Targets x86-64 QEMU. No real GPIO/I2C/SPI/UART/ADC/PWM. We register **nothing** for those — `HalRegistry::open_gpio` returns `NotPresent`. Display and audio are real.

```rust
#[cfg(feature = "platform-desktop")]
pub fn init_early(reg: &HalRegistry, profile: &PlatformProfile) -> Result<(), BootError> {
    // Display: DRM via /dev/dri/card0, fallback to /dev/fb0 (existing code).
    reg.register_display(0, Arc::new(DrmDisplay::new("/dev/dri/card0")?));
    // Audio: ALSA via /dev/snd/pcmC0D0p.
    reg.register_audio(0, Arc::new(AlsaAudio::new("default")?));
    // Input: evdev over /dev/input/event* (mouse, keyboard).
    for path in glob_inputs() {
        let id = path_id(&path);
        reg.register_input(id, Arc::new(EvdevInput::open(&path)?));
    }
    // Thermal: /sys/class/thermal/thermal_zone*/temp
    for (id, zone) in enumerate_thermal_zones() {
        reg.register_thermal(id, Arc::new(SysfsThermal::new(zone)));
    }
    Ok(())
}

pub fn init_late(_reg: &HalRegistry, _profile: &PlatformProfile)
    -> Result<(), BootError>
{
    Ok(()) // Display/audio fds already opened.
}
```

`DrmDisplay` is essentially the existing `supervisor/src/display.rs` refactored to implement `DisplayDevice`. The compositor pulls its buffer via `hal.open_display(0)?.map_buffer()?` instead of opening `/dev/fb0` directly.

### 4.2 `iot-edge` — `supervisor/src/hal/platform/arm64_sysfs.rs`

Targets ARM64 Raspberry Pi / similar SBCs. Real peripherals via libgpiod and devtmpfs.

```rust
#[cfg(feature = "platform-iot-edge")]
pub fn init_early(reg: &HalRegistry, profile: &PlatformProfile) -> Result<(), BootError> {
    // GPIO: libgpiod on /dev/gpiochip0 (BCM2711).
    let chip = Arc::new(GpiodChip::open("/dev/gpiochip0")?);
    for &pin in profile.gpio_pins() {
        reg.register_gpio(pin, Arc::new(GpiodPin::new(chip.clone(), pin)));
    }
    // I2C: /dev/i2c-{0,1}
    for bus in profile.i2c_buses() {
        reg.register_i2c(bus, Arc::new(LinuxI2c::open(bus)?));
    }
    // SPI: /dev/spidev{0,1}.{0,1}
    for bus in profile.spi_buses() {
        reg.register_spi(bus, Arc::new(LinuxSpi::open(bus)?));
    }
    // UART: /dev/ttyAMA{0,1}
    for port in profile.uart_ports() {
        reg.register_uart(port, Arc::new(LinuxUart::open(port)?));
    }
    // ADC via MCP3008 (SPI) — registered as software ADC channel
    // backed by SpiBus 0, CS 0.
    if let Some(adc) = profile.adc_via_spi() {
        for ch in 0..8 {
            reg.register_adc(ch, Arc::new(Mcp3008Channel::new(reg.clone(), ch)));
        }
    }
    // PWM: /sys/class/pwm/pwmchip0/pwm{0,1}
    for ch in profile.pwm_channels() {
        reg.register_pwm(ch, Arc::new(SysfsPwm::new(ch)));
    }
    // No display/audio on a headless IoT image by default.
    Ok(())
}
```

`GpiodPin` wraps a `gpiod::Line` request and implements `GpioPin`. Interrupts are delivered via a dedicated kernel-event thread that reads `gpiod::LineEventBuffer` and dispatches to registered callbacks.

```rust
struct GpiodPin {
    chip: Arc<GpiodChip>,
    pin: u8,
    request: Mutex<Option<gpiod::LineRequest>>,
    cb: Mutex<Option<Box<dyn Fn(bool) + Send + Sync>>>,
}

impl GpioPin for GpiodPin {
    fn id(&self) -> u8 { self.pin }

    fn set_direction(&self, dir: GpioDirection) -> Result<(), HalError> {
        let req = self.chip.request_line(self.pin, dir)?;
        *self.request.lock() = Some(req);
        Ok(())
    }

    fn read(&self) -> Result<bool, HalError> {
        let guard = self.request.lock();
        let req = guard.as_ref().ok_or_else(|| HalError::InvalidArg(
            "pin not configured".into()
        ))?;
        Ok(req.get_value()? != 0)
    }

    fn write(&self, high: bool) -> Result<(), HalError> {
        let guard = self.request.lock();
        let req = guard.as_ref().ok_or_else(|| HalError::InvalidArg(
            "pin not configured".into()
        ))?;
        req.set_value(if high { 1 } else { 0 })?;
        Ok(())
    }

    fn set_interrupt(&self, edge: GpioEdge, cb: Option<Box<dyn Fn(bool) + Send + Sync>>)
        -> Result<(), HalError>
    {
        *self.cb.lock() = cb;
        // Re-arm the kernel edge detector on the GpiodChip's IRQ thread.
        self.chip.arm_edge(self.pin, edge)?;
        Ok(())
    }
}
```

### 4.3 `robotics-rt` — same file, with priority boost

Reuses `arm64_sysfs.rs` but adds real-time scheduling for the GPIO interrupt thread and the audio capture thread.

```rust
#[cfg(feature = "platform-robotics-rt")]
pub fn init_late(reg: &HalRegistry, _profile: &PlatformProfile)
    -> Result<(), BootError>
{
    use thread_priority::*;
    set_current_thread_priority(ThreadPriority::Crossplatform(
        ThreadPriorityValue::try_from(50).unwrap()
    ))?;
    set_current_thread_scheduling_policy(
        thread_priority::RealtimeThreadSchedulePolicy::Fifo
    )?;
    Ok(())
}
```

The HAL also exposes a `cycle_time_ns` hint via `profile.timing_budget()` so robotics drivers can verify they meet their loop deadline.

### 4.4 `mobile` — `supervisor/src/hal/platform/mobile.rs`

ARM64 with touchscreen. Touch via evdev. IMU via IIO sysfs. Camera via V4L2. No GPIO/I2C/SPI in the manifest unless the platform profile explicitly exposes peripheral pins (some mobile SoCs do via expansion connectors).

```rust
#[cfg(feature = "platform-mobile")]
pub fn init_early(reg: &HalRegistry, profile: &PlatformProfile) -> Result<(), BootError> {
    // Touchscreen as InputDevice
    reg.register_input(0, Arc::new(EvdevInput::open("/dev/input/event0")?));
    // Accelerometer as a custom thermal-like sensor returning g-force.
    // (For brevity, IMU is a separate trait not shown.)
    // Display: DSI panel as DRM
    reg.register_display(0, Arc::new(DrmDisplay::new("/dev/dri/card0")?));
    reg.register_audio(0, Arc::new(AlsaAudio::new("hw:0,0")?));
    Ok(())
}
```

### 4.5 `mcu-minimal` — `supervisor/src/hal/platform/mcu.rs`

The special case. No Linux. The supervisor itself becomes a Cortex-M `no_std` binary running on bare metal, hosting the wasm3 interpreter. The HAL is direct MMIO via `volatile-register`.

```rust
#![cfg(feature = "platform-mcu-minimal")]
#![no_std]

use volatile_register::{RO, RW};

#[repr(C)]
struct GpioRegs {
    moder:  RW<u32>, // mode register
    otyper: RW<u32>, // output type
    idr:    RO<u32>, // input data
    odr:    RW<u32>, // output data
}

const GPIOA_BASE: usize = 0x4002_0000;

struct McuGpioPin {
    bank: *const GpioRegs,
    bit:  u8,
}

unsafe impl Send for McuGpioPin {}
unsafe impl Sync for McuGpioPin {}

impl GpioPin for McuGpioPin {
    fn id(&self) -> u8 { self.bit }

    fn set_direction(&self, dir: GpioDirection) -> Result<(), HalError> {
        unsafe {
            let regs = &*self.bank;
            let mut moder = regs.moder.read();
            let mask = 0b11 << (self.bit * 2);
            moder &= !mask;
            moder |= match dir {
                GpioDirection::Input | GpioDirection::InputPullUp
                    | GpioDirection::InputPullDown => 0b00,
                GpioDirection::Output => 0b01,
                GpioDirection::OutputOpenDrain => 0b01,
            } << (self.bit * 2);
            regs.moder.write(moder);
        }
        Ok(())
    }

    fn read(&self) -> Result<bool, HalError> {
        unsafe { Ok(((&*self.bank).idr.read() >> self.bit) & 1 != 0) }
    }

    fn write(&self, high: bool) -> Result<(), HalError> {
        unsafe {
            let regs = &*self.bank;
            let mut odr = regs.odr.read();
            if high { odr |=  1 << self.bit; }
            else    { odr &= !(1 << self.bit); }
            regs.odr.write(odr);
        }
        Ok(())
    }

    fn set_interrupt(&self, edge: GpioEdge, cb: Option<Box<dyn Fn(bool) + Send + Sync>>)
        -> Result<(), HalError>
    {
        // mcu-minimal: callbacks are stored in a static slot indexed by
        // (bank, bit); the EXTI ISR looks them up. No allocation in the ISR.
        mcu_exti::register(self.bank as usize, self.bit, edge, cb)
    }
}
```

Constraints particular to `mcu-minimal`:

- **No alloc in interrupts.** `Box<dyn Fn>` is created at registration time; the ISR just calls it.
- **No threads.** The supervisor is a cooperative event loop. `RankedMutex` from R8 degrades to a critical-section guard.
- **No `Arc`.** Replaced by `&'static dyn GpioPin` references; the registry stores `&'static` slots in a `heapless::FnvIndexMap`.
- **No WASI.** wasm3 hostcalls are wired directly to the HAL trait methods.

This means there's a second `HalRegistry` definition gated on `#[cfg(not(feature = "platform-mcu-minimal"))]` vs. an `mcu-minimal` variant using `heapless::FnvIndexMap<u8, &'static dyn GpioPin, 32>`. We accept this duplication; it stays under the 500-line limit per file.

### 4.6 `server-headless` — stubs

Same as `desktop-full` for I/O peripherals (all `NotPresent`), but adds:

- IPMI via `/dev/ipmi0` as a `Bmc` device (new trait, not shown in core).
- Watchdog via `/dev/watchdog` exposed as a `WatchdogDevice`.
- Multiple thermal zones (CPU sockets).

### 4.7 `mock` — `supervisor/src/hal/platform/mock.rs`

Used by `cargo test`. Every trait gets an in-memory implementation:

```rust
pub struct MockGpio {
    state: Mutex<bool>,
    direction: Mutex<GpioDirection>,
    cb: Mutex<Option<Box<dyn Fn(bool) + Send + Sync>>>,
}

impl GpioPin for MockGpio {
    fn id(&self) -> u8 { 0 }
    fn set_direction(&self, dir: GpioDirection) -> Result<(), HalError> {
        *self.direction.lock() = dir; Ok(())
    }
    fn read(&self) -> Result<bool, HalError> { Ok(*self.state.lock()) }
    fn write(&self, high: bool) -> Result<(), HalError> {
        *self.state.lock() = high;
        if let Some(cb) = &*self.cb.lock() { cb(high); }
        Ok(())
    }
    fn set_interrupt(&self, _edge: GpioEdge, cb: Option<Box<dyn Fn(bool)+Send+Sync>>)
        -> Result<(), HalError>
    { *self.cb.lock() = cb; Ok(()) }
}
```

A `mock_registry()` helper builds a `HalRegistry` pre-populated with mocks for use in unit tests.

---

## 5. Capability enforcement

R6 introduced `PeripheralRegistry`. The HAL is its consumer. The full enforcement chain:

```
1. App declares in vyoma.toml:
       [capabilities]
       gpio_pins = [4, 17]
       i2c_bus   = 1

2. Supervisor parses manifest into:
       struct PeripheralCaps {
           gpio_pins:    Vec<u8>,
           i2c_buses:    Vec<u8>,
           spi_buses:    Vec<u8>,
           uart_ports:   Vec<u8>,
           adc_channels: Vec<u8>,
           pwm_channels: Vec<u8>,
       }

3. On app spawn, supervisor calls:
       PeripheralRegistry::grant_caps(app_id, &PeripheralCaps);
   which validates against HalRegistry: every requested pin must be
   present and not already owned. Returns HalError::NotPresent or
   HalError::Busy on failure -> app spawn aborts with a clear error.

4. App calls hostcall `vyoma:hal/gpio.open-pin(4)`. Supervisor:
       let pin = self.hal.open_gpio(app_id, 4)?;
   open_gpio asks PeripheralRegistry.claim_gpio(app_id, 4):
       - if app_id not in caps for pin 4 -> PermissionDenied
       - if another app owns pin 4         -> Busy
       - else                              -> mark owned, return OK
   Then returns Arc<dyn GpioPin> wrapped in a WIT resource handle.

5. WIT resource handle is destroyed when:
       - WASM calls gpio-pin.drop (explicit release)
       - WASM instance exits (Wasmtime drops resource table)
       - Supervisor kills app (R8 shutdown hook releases via release_all)
   Drop releases the PeripheralRegistry claim.
```

`PeripheralRegistry` looks like:

```rust
// supervisor/src/capability/peripheral.rs

use std::collections::HashMap;
use parking_lot::Mutex;

pub enum CapError {
    NoCapability,
    Busy(String),
}

#[derive(Default)]
struct CapState {
    /// Per-app declared capabilities.
    caps: HashMap<String, PeripheralCaps>,
    /// Current owners: peripheral identifier -> app_id.
    owners: HashMap<PeripheralId, String>,
}

#[derive(Hash, PartialEq, Eq, Clone)]
enum PeripheralId {
    Gpio(u8),
    I2c(u8),
    Spi(u8),
    Uart(u8),
    Adc(u8),
    Pwm(u8),
}

pub struct PeripheralRegistry { state: Mutex<CapState> }

impl PeripheralRegistry {
    pub fn grant_caps(&self, app_id: &str, caps: PeripheralCaps)
        -> Result<(), CapError>
    {
        let mut st = self.state.lock();
        // Cross-app collision check: refuse spawn if any requested
        // peripheral is already in someone else's caps.
        // (Note: collision in caps != ownership; ownership is per-open.)
        st.caps.insert(app_id.to_string(), caps);
        Ok(())
    }

    pub fn claim_gpio(&self, app_id: &str, pin: u8) -> Result<(), CapError> {
        let mut st = self.state.lock();
        let caps = st.caps.get(app_id).ok_or(CapError::NoCapability)?;
        if !caps.gpio_pins.contains(&pin) {
            return Err(CapError::NoCapability);
        }
        match st.owners.get(&PeripheralId::Gpio(pin)) {
            Some(owner) if owner != app_id => Err(CapError::Busy(owner.clone())),
            _ => {
                st.owners.insert(PeripheralId::Gpio(pin), app_id.to_string());
                Ok(())
            }
        }
    }

    pub fn release_all(&self, app_id: &str) {
        let mut st = self.state.lock();
        st.owners.retain(|_, owner| owner != app_id);
        st.caps.remove(app_id);
    }
}
```

### 5.1 Crash safety

R8 mandates a shutdown hook per app: when wasmtime returns or the process is killed, the supervisor's app-supervision thread calls `hal_registry.release_all(app_id)`. This releases every claim, makes the peripherals available again, and drops the trait-object `Arc`s if their refcount hits zero.

For peripherals with persistent state (e.g., a PWM channel left at duty=50%), the HAL guarantees a safe-default on release: GPIO outputs go to input mode, PWM disables, I2C/SPI sends a stop condition, UART flushes. This is implemented in a trait-object `Drop` impl on each platform-specific concrete type, not on the trait itself (traits can't define destructors).

```rust
impl Drop for GpiodPin {
    fn drop(&mut self) {
        // Best-effort: revert to input, ignore errors.
        let _ = self.set_direction(GpioDirection::Input);
    }
}
```

---

## 6. WIT interface for WASM apps

```wit
// wit/vyoma-hal.wit
package vyoma:hal@0.1.0;

interface types {
    variant hal-error {
        not-present(string),
        busy(string),
        permission-denied(string),
        invalid-arg(string),
        io-error(string),
        timeout(u32),
        protocol(string),
        unimplemented(string),
    }
}

interface gpio {
    use types.{hal-error};

    enum direction {
        input,
        input-pull-up,
        input-pull-down,
        output,
        output-open-drain,
    }

    enum edge { rising, falling, both }

    resource gpio-pin {
        read: func() -> result<bool, hal-error>;
        write: func(high: bool) -> result<_, hal-error>;
        toggle: func() -> result<_, hal-error>;
        set-direction: func(dir: direction) -> result<_, hal-error>;
        // Interrupts: WASM cannot supply a callback directly. Instead, the
        // pin is added to a poll-set; the app reads events via poll-pin.
        watch: func(edge: edge) -> result<_, hal-error>;
        unwatch: func() -> result<_, hal-error>;
    }

    open-pin: func(pin: u8) -> result<gpio-pin, hal-error>;
    /// Block up to timeout-ms; return list of pin numbers that fired.
    poll-pins: func(timeout-ms: u32) -> result<list<u8>, hal-error>;
}

interface i2c {
    use types.{hal-error};

    resource i2c-bus {
        write: func(addr: u8, data: list<u8>) -> result<_, hal-error>;
        read:  func(addr: u8, len: u32) -> result<list<u8>, hal-error>;
        write-read: func(addr: u8, write: list<u8>, read-len: u32)
            -> result<list<u8>, hal-error>;
        probe: func(addr: u8) -> result<bool, hal-error>;
    }
    open-bus: func(bus: u8) -> result<i2c-bus, hal-error>;
}

// spi, uart, adc, pwm follow the same pattern; elided for brevity.

world hal-consumer {
    import gpio;
    import i2c;
    import spi;
    import uart;
    import adc;
    import pwm;
}
```

Notes on the WIT design:

- **No callbacks across the WIT boundary.** WIT 0.2 has no story for "Rust → WASM callback". Instead we expose a `watch` + `poll-pins` pair: the app says "I care about edge events on pin 4", the supervisor's GPIO interrupt thread accumulates events in a per-app queue, the app polls. This costs one extra round-trip per event but stays within WIT semantics.
- **Resource handles are exclusive.** A `gpio-pin` resource cannot be cloned. Dropping it (or instance exit) releases the underlying claim.
- **Errors are strings inside the variant**, not enums-of-enums. Keeps WIT compilation simple while preserving structured error info on the Rust side via the variant tag.

### 6.1 Hostcall implementation

```rust
// supervisor/src/hostcalls/hal.rs
use wasmtime::component::Resource;

pub struct GpioHost {
    hal: Arc<HalRegistry>,
    app_id: String,
    /// Resource table: WIT handle -> Arc<dyn GpioPin>
    pins: ResourceTable<Arc<dyn GpioPin>>,
}

impl vyoma::hal::gpio::Host for GpioHost {
    fn open_pin(&mut self, pin: u8)
        -> Result<Result<Resource<GpioPin>, HalError>, anyhow::Error>
    {
        match self.hal.open_gpio(&self.app_id, pin) {
            Ok(arc) => {
                let handle = self.pins.push(arc)?;
                Ok(Ok(handle))
            }
            Err(e) => Ok(Err(e)),
        }
    }
}
```

When the wasmtime instance drops, the resource table is destructed; each `Arc<dyn GpioPin>` Drop fires, and the per-platform Drop returns the peripheral to a safe state. The `PeripheralRegistry` claim is released via the explicit `release_all` call from the app-supervision thread (we don't tie it to Drop because R8 mandates that the supervisor — not the wasm engine — owns release ordering).

---

## 7. The `mcu-minimal` special case in detail

`mcu-minimal` is fundamentally a different beast. It's worth stating exactly what changes:

| Concern                | Default platforms          | mcu-minimal                            |
|------------------------|----------------------------|----------------------------------------|
| Allocator              | system (glibc/musl)        | `embedded-alloc` over a static heap    |
| Concurrency            | OS threads + `Send + Sync` | Single-threaded cooperative loop       |
| Locks                  | `parking_lot::RwLock`      | `cortex_m::interrupt::free` critical sections |
| Trait object storage   | `Arc<dyn Trait>`           | `&'static dyn Trait`                   |
| Collections            | `std::collections::HashMap`| `heapless::FnvIndexMap<K, V, N>`       |
| WASM runtime           | Wasmtime                   | wasm3                                  |
| Hostcall mechanism     | Wasmtime resource tables   | wasm3 native function pointers         |
| Interrupt callback     | `Box<dyn Fn> + Send + Sync`| `fn(bool)` in a static slot            |
| HalError variant       | `Io { source: io::Error }` | `Io { errno: i32 }`                    |

The HAL traits compile under both — they're `Send + Sync` everywhere but we set up a `cfg`-gated `unsafe impl Send + Sync` on the `&'static` references on mcu-minimal where we know the executor is single-threaded.

Critically, the WIT interface is **the same** on mcu-minimal as elsewhere. wasm3 accepts the same WIT-generated bindings; only the hostcall implementation differs. This is important because it means an LED-blink app written against `vyoma:hal/gpio` runs unmodified on a Cortex-M4 and on QEMU (with a mock GPIO that prints "pin 4 = HIGH"). Tier-1 portability.

The `mcu-minimal` HAL drops a few features:

- No interrupt callbacks via `Box`. Instead, a const-generic dispatch table indexed by IRQ vector.
- No display/audio/V4L2/IIO traits — those subsystems aren't built in for this profile.
- No `PeripheralRegistry` mutex. Single-threaded execution means we can use a `RefCell<CapState>` in a static.

---

## 8. Interaction with prior rounds

### 8.1 With R6 (drivers)

The R6 three-tier driver model says drivers can be (a) in-supervisor Rust, (b) privileged WASM, or (c) kernel modules with a WIT shim. **All three consume the HAL traits.**

- **Tier A (in-supervisor Rust):** a `bme280` driver crate calls `hal.open_i2c(...)` once at registration and stores the `Arc<dyn I2cBus>`. The driver is registered with the supervisor's `DriverRegistry`, not the WASM apps.
- **Tier B (privileged WASM):** the WASM driver imports `vyoma:hal/i2c` and calls `i2c.open-bus(1)`. The supervisor's hostcall implementation routes that to `hal.open_i2c("driver:bme280", 1)`. Driver capability is granted by the supervisor at driver-load time, not by `vyoma.toml`.
- **Tier C (kernel module + WIT shim):** the kernel driver claims the bus directly; the supervisor's HAL implementation for that bus returns `NotPresent` (the bus is no longer available at the user-space `/dev/i2c-1` level). Apps and Tier-A/B drivers can't access it.

The R6 `DeviceManager` udev consumer enumerates `/dev/gpiochip*`, `/dev/i2c-*`, etc. and registers them with the HAL — so hot-plug works for USB-attached HAL devices (e.g., a USB-to-I2C bridge appearing at runtime).

### 8.2 With R7 (power + thermal)

R7's `ThermalGovernor` reads `hal.thermal(0)?.read_mc()` instead of opening `/sys/class/thermal/thermal_zone0/temp` directly. R7's power-assertion subsystem doesn't touch the HAL — power state is a kernel concern (suspend-to-RAM via `/sys/power/state`).

If a thermal trip fires (e.g., CPU > 85 °C), R7 calls into the HAL to throttle: `hal.pwm(fan)?.set_duty_ns(...)` for active cooling, or the supervisor lowers a CPU governor knob (not HAL).

### 8.3 With R8 (boot + RankedMutex)

`HalRegistry` uses `RankedMutex` internally. The mutex rank for HAL maps is "rank 60" (above PeripheralRegistry at 50, below subsystem-specific locks at 70+). This eliminates ABBA deadlocks like the one fixed in compositor commit `55fd121`.

Boot order is encoded in `BootPhase`:
- Phase 1 `init_early`: register trait stubs.
- Phase 2 `init_late`: open kernel FDs.
- Phase 3: subsystems (compositor, IPC, manifest validator) connect.
- Phase 4: app launcher honours `PeripheralRegistry::grant_caps`.

### 8.4 With existing supervisor/src/hal/mod.rs

The current `mod.rs` (118 lines) becomes a re-export shim:

```rust
// supervisor/src/hal/mod.rs (replaced)

pub mod error;
pub mod traits;
pub mod registry;
pub mod platform;

#[cfg(test)]
mod tests;

pub use error::{HalError, PeripheralKind};
pub use traits::*;
pub use registry::HalRegistry;

/// Compatibility shim for code that still uses the old `HalProvider`
/// trait. Wraps a `HalRegistry` and routes through. Slated for removal
/// once T002 callers migrate.
pub struct HalProviderShim(pub Arc<HalRegistry>);
```

The old `String`-error trait methods are removed in this refactor; callers switch to `HalError`.

---

## 9. Threading model

The HAL has three thread categories:

1. **Caller threads.** Any supervisor subsystem thread or wasmtime store thread that invokes a HAL method. Calls are synchronous and may block on a mutex or a kernel ioctl.
2. **Interrupt dispatch thread (per chip).** One thread per `gpiod::Chip` polls `epoll(/dev/gpiochip0)` for line events, decodes them, and dispatches registered callbacks. Registration uses an `RwLock` over the callback map; reads are common, writes rare. On `robotics-rt`, this thread runs SCHED_FIFO with high priority.
3. **Audio capture/playback thread.** ALSA PCM is blocking; one thread per active stream. Owned by the `AlsaStream` type, joined on stream drop.

`mcu-minimal` has only category 1 (the cooperative event loop) plus interrupt vectors (no thread).

### 9.1 Lock ordering (R8 RankedMutex)

| Lock                         | Rank | Notes                          |
|------------------------------|------|--------------------------------|
| `PeripheralRegistry::state`  | 50   | Always acquired first          |
| `HalRegistry::{gpio,i2c,…}`  | 60   | After PeripheralRegistry       |
| Concrete per-handle locks    | 70   | `GpiodPin::request`, etc.      |
| Subsystem-specific locks     | 80+  | Compositor, IPC, etc.          |

The pattern `caps.lock(); hal.lock(); pin.lock();` is enforced by `RankedMutex` at debug-assertion time.

### 9.2 Blocking calls

I2C writes can block tens of milliseconds (clock stretching). UART reads block on FIFO drain. ADC reads block on conversion-complete. None of these are `async` — VyomaOS apps are sync wasm32-wasip2 with synchronous WASI; the HAL matches.

For non-blocking idioms, callers use the `poll-pins` pattern (GPIO) or a separate thread (UART read loop in a driver).

---

## 10. Testing strategy

### 10.1 Unit tests (cargo test, mock HAL)

```rust
// supervisor/src/hal/tests.rs
#[test]
fn gpio_capability_enforcement() {
    let caps = Arc::new(PeripheralRegistry::default());
    let hal  = Arc::new(HalRegistry::new(caps.clone()));
    hal.register_gpio(4, Arc::new(MockGpio::default()));

    // No manifest grant -> PermissionDenied.
    let r = hal.open_gpio("app-a", 4);
    assert!(matches!(r, Err(HalError::PermissionDenied { .. })));

    // Grant cap, open succeeds.
    caps.grant_caps("app-a", PeripheralCaps { gpio_pins: vec![4], ..Default::default() }).unwrap();
    let pin = hal.open_gpio("app-a", 4).unwrap();
    pin.set_direction(GpioDirection::Output).unwrap();
    pin.write(true).unwrap();
    assert_eq!(pin.read().unwrap(), true);

    // Second app: Busy.
    caps.grant_caps("app-b", PeripheralCaps { gpio_pins: vec![4], ..Default::default() }).unwrap();
    let r = hal.open_gpio("app-b", 4);
    assert!(matches!(r, Err(HalError::Busy { .. })));

    // Release app-a: app-b can claim.
    caps.release_all("app-a");
    let _ = hal.open_gpio("app-b", 4).unwrap();
}
```

### 10.2 Integration tests (QEMU)

The existing `make smoke` boots desktop-full in QEMU. A `make smoke-iot` variant boots iot-edge ARM64 with virtual GPIO via QEMU's `gpio-virtio` device. A test app blinks LED on pin 4 and checks the host-side QEMU monitor for level changes.

### 10.3 mcu-minimal tests

QEMU `mps2-an385` (Cortex-M3) boots the supervisor; a test app writes pin 4 high, the QEMU GPIO model exposes the value over a test socket.

### 10.4 Loom tests

`HalRegistry`'s lock ordering is verified under `loom::model`. The PeripheralRegistry claim/release sequence is checked for ABBA freedom against the HAL's per-handle locks.

---

## 11. Implementation files & line budgets

| File                                              | Approx. lines | Purpose                          |
|---------------------------------------------------|---------------|----------------------------------|
| `supervisor/src/hal/mod.rs`                       | 60            | Re-exports + compatibility shim  |
| `supervisor/src/hal/error.rs`                     | 110           | `HalError`, `PeripheralKind`     |
| `supervisor/src/hal/traits.rs`                    | 480           | GPIO/I2C/SPI/UART/ADC/PWM/display/audio/input/thermal traits + types |
| `supervisor/src/hal/registry.rs`                  | 380           | `HalRegistry` with capability enforcement |
| `supervisor/src/hal/platform/mod.rs`              | 40            | `cfg`-gated module selector      |
| `supervisor/src/hal/platform/desktop.rs`          | 240           | DRM display + ALSA + evdev + thermal stubs for non-applicable |
| `supervisor/src/hal/platform/arm64_sysfs.rs`      | 460           | libgpiod GPIO + Linux I2C/SPI/UART + sysfs PWM + MCP3008 ADC |
| `supervisor/src/hal/platform/mobile.rs`           | 280           | Touch evdev + DRM + IIO IMU      |
| `supervisor/src/hal/platform/mcu.rs`              | 470           | MMIO GPIO/I2C/SPI/UART/ADC/PWM for STM32-class MCU |
| `supervisor/src/hal/platform/server_headless.rs`  | 200           | IPMI + watchdog + thermal        |
| `supervisor/src/hal/platform/mock.rs`             | 360           | Mocks for every trait, for tests |
| `supervisor/src/hal/tests.rs`                     | 320           | Unit tests, loom checks          |
| `supervisor/src/hostcalls/hal.rs`                 | 420           | WIT host implementations (gpio, i2c, spi, uart, adc, pwm) |
| `wit/vyoma-hal.wit`                               | n/a (WIT)     | Public WIT interface             |
| `supervisor/src/capability/peripheral.rs`         | 260           | `PeripheralRegistry`, `CapError` |

All Rust files are under the 500-line cap with margin to grow. The two close-to-limit files (`traits.rs` at 480, `arm64_sysfs.rs` at 460, `mcu.rs` at 470) will be split as soon as one more peripheral class is added — `traits.rs` would split into `traits_low.rs` (GPIO/I2C/SPI/UART/ADC/PWM) and `traits_av.rs` (display/audio/input/thermal). Architectural pre-commitment recorded here.

---

## 12. Migration plan from current code

The supervisor today opens `/dev/fb0` directly in `display.rs`, reads `/dev/input/event*` in `mouse_input.rs`, and only has the trait skeleton in `hal/mod.rs`. The migration is six steps:

1. **Add error and registry** (`error.rs`, `registry.rs`, no callers yet).
2. **Add platform skeletons** with `init_early`/`init_late` returning Ok and registering nothing.
3. **Migrate display**: refactor `display.rs` into `platform/desktop.rs::DrmDisplay: DisplayDevice`. Callers in `compositor.rs` now go through `hal.open_display(0)`.
4. **Migrate input**: `mouse_input.rs` becomes `platform/desktop.rs::EvdevInput: InputDevice`. Input router consumes via `hal.open_input(0)`.
5. **Add peripheral capability path** (gpio/i2c/etc. on iot-edge, robotics-rt). New code; no existing callers.
6. **Add WIT bindings + hostcall** for WASM apps. Update an example app (e.g., a new `apps/gpio-blink`) to use them.

Steps 1–4 are platform-portable refactors that don't require new hardware. Step 5 requires an ARM64 build machine. Step 6 enables the first end-user-visible feature.

---

## 13. Worked example: LED blink across all platforms

The same WASM app:

```rust
// apps/gpio-blink/src/main.rs
use vyoma::hal::gpio::{self, Direction};

fn main() {
    let pin = gpio::open_pin(4).expect("pin 4");
    pin.set_direction(Direction::Output).unwrap();
    loop {
        pin.write(true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        pin.write(false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
```

Manifest:

```toml
# apps/gpio-blink/vyoma.toml
[app]
name    = "gpio-blink"
version = "0.1.0"
wasm    = "gpio-blink.wasm"

[capabilities]
stdio     = true
gpio_pins = [4]
```

On each platform:

- `desktop-full`: `hal.open_gpio("gpio-blink", 4)` returns `NotPresent` → app exits with manifest-validation error at spawn time (cap requested, peripheral absent).
- `iot-edge` RPi: pin 4 (BCM4) toggles. LED blinks.
- `robotics-rt`: pin 4 toggles with real-time guarantees on the supervisor's interrupt thread (doesn't matter much here; matters for input).
- `mobile`: same as `desktop-full` unless the mobile profile exposes GPIO pins.
- `mcu-minimal`: pin 4 of GPIOA toggles via MMIO; no Linux involved.
- `server-headless`: same as `desktop-full`.

Same WASM binary, byte-identical, on all six. That's the win.

---

## 14. Open questions for the Critic

1. **Should the HAL grow an async surface?** Today everything is blocking-sync, matching wasm32-wasip2 semantics. But the compositor, the audio mixer, and the R6 udev hot-plug watcher all want non-blocking I/O. Do we add a parallel `AsyncHalRegistry` over tokio, or keep using dedicated threads per blocking resource? If we add async, do WIT apps see it (`async` in WIT preview)?

2. **Mid-blink hot-unplug of a USB-HAL device.** A USB-to-I2C bridge is registered at `i2c-2`. An app claims it and is mid-`write_read` when the user unplugs the bridge. The kernel returns `-ENODEV`. We surface `HalError::Io`, but the `Arc<dyn I2cBus>` is still in the registry. Do we de-register on udev REMOVE (forcing the app's next call to be `NotPresent`), or leave the handle live as a "ghost" so the app sees consistent `Io` errors until it releases? Hot-plug semantics need a concrete rule.

3. **Multi-instance peripherals (e.g., two cameras).** Today `HalRegistry` keys peripherals by `u8` id. A USB camera at `/dev/video0` and a CSI camera at `/dev/video1` both register. But the manifest currently says `camera = true`, not `camera_id = 0`. Do we extend the manifest schema, or do we keep "ask for camera, get the first one with that capability"? The same question applies to multiple I2C buses on an SBC with a header expansion.

4. **Mock HAL and CI on macOS dev machines.** The current build does everything inside the Linux Docker container. Should `cargo test` work on a developer's macOS laptop *outside* the container (i.e., with `--target x86_64-apple-darwin` and the mock HAL active)? That would speed up iteration but require us to make sure no platform module accidentally references `nix::sys` for unit tests.

5. **DisplayBuffer ownership across the WASM boundary.** The compositor needs zero-copy access to the scanout buffer. A WASM app drawing into a window writes to its per-window `Surface` (already merged in commit `2490ad6`), and the compositor blits that into the `DisplayBuffer`. But what about a privileged graphics driver (R6 Tier B) that wants direct buffer access? WIT can't share raw pointers. Do we expose `DisplayDevice::map_buffer` only to in-supervisor Rust (Tier A drivers), forcing Tier B graphics drivers through a bounce buffer? Or do we add a `vyoma:hal/display` WIT with a shared-memory handle (and accept a complexity hit)?

6. **mcu-minimal trait object representation.** I proposed `&'static dyn GpioPin` on mcu-minimal where the default is `Arc<dyn GpioPin>`. That creates two different "shapes" of HAL registry behind the same `HalRegistry` name. Is the divergence acceptable, or should we unify with `Arc` everywhere (paying for atomic refcount on Cortex-M, which is fine on -M3+) and `embedded-alloc` for `Box<dyn>`?

7. **WIT `gpio.poll-pins` versus `wasi:io/poll`.** The R6 IPC design used `wasi:io/poll` for event aggregation. Should GPIO interrupts also flow through `wasi:io/poll` (so an app can wait on stdin + GPIO simultaneously), or do they get their own `vyoma:hal/gpio.poll-pins` API as I have it? The former is more elegant but couples HAL evolution to WASI Preview 2 evolution.

8. **Capability granularity for I2C.** Today we grant `i2c_bus = 1` (whole bus). But two apps might want to talk to two different slave addresses on the same bus without colliding (they can coordinate via the kernel's I2C arbitration). Do we relax to per-slave-address claims (`i2c_devices = [{bus = 1, addr = 0x76}, {bus = 1, addr = 0x77}]`)? That changes the exclusivity model and the `PeripheralRegistry` schema.

---

## 15. Summary

This proposal replaces the current sketch HAL with a complete, six-platform, capability-secure abstraction. The key shapes:

- One Rust trait per peripheral class. `Send + Sync` everywhere, structured `HalError`, `Result` returns.
- One `HalRegistry` owning all handles, mediated by `PeripheralRegistry` for capability + exclusivity.
- One per-platform implementation file behind a Cargo feature, swapped by `PLATFORM=` in the Makefile.
- One WIT interface (`vyoma:hal`) for WASM apps; one hostcall implementation that bridges WIT to `HalRegistry`.
- R8 BootPhase split: `init_early` (infallible, register stubs) + `init_late` (open FDs, arm IRQs).
- R6 driver tiers integrated: in-supervisor and privileged-WASM drivers consume the same HAL traits as the rest of the supervisor.
- mcu-minimal carved out as a `no_std` cousin: same traits, different storage (`&'static` not `Arc`), same WIT.

The HAL becomes the single Linux-touch point. Everything above it (compositor, IPC, package manager, R6 drivers, R7 governor, WASM apps) is platform-portable Rust. Everything below it is kernel ABI. The Makefile's `PLATFORM=` knob produces six binaries from one source tree.

Eight open questions await the Critic.
