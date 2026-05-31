# Round 9 Critic: Hardware Abstraction Layer

**Date:** 2026-05-29
**Round:** 9 of 80
**Subsystem:** Hardware Abstraction Layer
**Critic verdict:** FUNDAMENTAL FLAWS

## Executive Summary

The proposed VyomaOS HAL aspires to a clean, capability-secure abstraction
across six radically different platforms (Cortex-M MCU through x86-64
desktops) with a single `vyoma:hal` WIT-typed interface. The ambition is
correct — a unified surface for GPIO/I2C/SPI/UART/ADC/PWM is the only sane
way to keep apps portable across our target matrix. But the current design
sketch crashes into six fundamental realities that together make the v1
design as drafted **non-shippable**.

In short:

1. **The Linux substrate is wrong.** The proposed kernel interface
   (`/sys/class/gpio`) has been formally deprecated since Linux 4.8 (2016)
   and is on the kernel's removal list. We must build on the gpiod
   character-device API (`/dev/gpiochip*` + GPIO_V2 ioctls) from day one;
   anything else means rewriting the HAL within two years.

2. **The supervisor will block on I/O.** A synchronous `write_read()` API
   served from the supervisor process — even with a worker thread —
   collapses under realistic per-app rates because no current draft
   describes a per-bus scheduler. Apps cannot saturate an I2C bus through a
   single broker without queuing, and the queueing primitive is not
   designed.

3. **mcu-minimal cannot speak WIT.** wasm3 has no Preview-2 component
   model, no WIT codegen, no resource types. Pretending mcu-minimal apps
   share a HAL surface with desktop-full apps is a category error. The two
   classes need a *protocol-compatible but bind­ing-disjoint* HAL: same
   semantics, two distinct ABIs.

4. **Exclusive-ownership ACLs over-fit to the easy case.** Single-owner
   GPIO works for an LED. It fails for shared interrupt lines, for I2C
   address arbitration, for SPI chip selects, and for any pin that legally
   has multiple readers. The model needs read/write/notify decomposition
   à la file-descriptor access modes.

5. **Crash recovery does not equal safe state.** Releasing a handle is a
   software event; an output pin driving +3V3 into a motor controller is a
   physical event. Closing the file descriptor leaves the pin where it is.
   Without a per-pin **safe-state declaration** evaluated on handle drop
   (and at supervisor death), the HAL is a hazard.

6. **The platform matrix is a combinatorial explosion the CI plan
   ignores.** Six platforms × five buses × adapter selection × kernel
   versions × runtimes = a build matrix in the dozens. The current
   `cfg(feature = …)` sketch assumes mutual exclusion, which prevents a
   universal binary and prevents CI from building one image to test all
   targets.

Each of these is fixable. None is fixed in the present draft. I rate this
**FUNDAMENTAL FLAWS** — the HAL needs a second draft before any code lands
on `main`. The R6 three-tier driver model was a strong foundation; the HAL
proposal squanders it.

The remainder of this document attacks each dimension in detail, calls out
secondary concerns the proposal silently sidesteps, names the parts that
*are* sound, and closes with a concrete synthesis roadmap for the FINAL.

---

## Critical Issues (blocking)

The five issues below MUST be resolved before merging the HAL design. Each
breaks either correctness, portability, safety, or the build itself.

### C1. sysfs GPIO is the wrong substrate

The proposal hand-waves the GPIO backend as "writes to `/sys/class/gpio`".
This is wrong for three independent reasons.

**Reason 1: it is deprecated.** Linux 4.8 (October 2016) introduced
`/dev/gpiochip*` and marked sysfs GPIO obsolete. The kernel documentation
(`Documentation/admin-guide/gpio/sysfs.rst`) opens with: *"This ABI is
deprecated, will be removed after 2020, and is replaced with the GPIO
character device."* The removal has slipped, but kernels compiled with
`CONFIG_GPIO_SYSFS=n` already exist in production embedded distributions
(Yocto's default flips off in mickledore, OpenWrt no longer enables it on
several targets). VyomaOS's iot-edge/robotics-rt platforms target exactly
those distros downstream. We will ship a HAL that does not work on the
boards we claim to support.

**Reason 2: it leaks process-global state.** sysfs GPIO is rooted in a
process-global namespace. If app A exports pin 17 and crashes, the pin
stays exported. A second app trying to claim it gets `EBUSY` with no
indication of who owns it. There is no way to atomically "claim, configure,
read" — every operation is a separate `open()/write()/close()` of a
different sysfs file. Concurrent supervisors stomp on each other.
character-device gpiod provides a single file descriptor whose lifetime
*is* the lease, and closing it (including via process death) returns the
pin to its default state. This is exactly what a capability-secure HAL
needs; sysfs is exactly what we are trying to escape from.

**Reason 3: it cannot represent modern features.** Linux GPIO_V2 ioctls
(`GPIO_V2_LINE_REQUEST_IOCTL`) carry: edge-trigger configuration
(`GPIO_V2_LINE_FLAG_EDGE_RISING|FALLING`), debounce (microseconds), bias
(pull-up / pull-down / disable), drive (open-drain, open-source,
push-pull), and *atomic* multi-line claim of up to 64 lines per request.
sysfs supports none of this. The robotics-rt platform's manifest already
declares `gpio.bias = "pullup"` — we have committed to a feature the
proposed substrate cannot deliver.

**What this implies:** the HAL Linux backend MUST be built on
`gpiod`-style direct ioctls (the `gpio-cdev` crate, or a hand-rolled
wrapper around `libc::ioctl` with the `linux/gpio.h` request structs).
Sysfs is unacceptable even as a fallback because its semantics differ
materially (no atomic multi-line, no bias).

**Cost of doing this right:** about 800 LOC of `unsafe` ioctl wrapping +
strong types around the line-request structs. Higher up-front than sysfs,
but every other layer of the HAL is simpler because the substrate finally
has the right shape.

---

### C2. The supervisor will become an I/O bottleneck

The proposal places the HAL ioctl/syscall calls in the supervisor process,
gated behind capability checks, and exposed to apps via a WIT interface.
Apps call `i2c.write_read(addr, write_buf, read_len)` and block until the
broker performs the underlying transaction.

This works for an LED blinker. It does not work for the workloads the
robotics-rt and iot-edge platforms actually claim to support.

**Numbers.** A standard-mode I2C transaction at 100 kHz takes roughly
1 ms per byte; a typical sensor read is 6–10 bytes of `write_read`, so
~10 ms wall-clock per call. Even at 400 kHz fast-mode that's 2.5 ms. A
LIDAR over SPI at 10 MHz with a 256-byte transfer is 200 µs of bus time
*plus* kernel ioctl overhead (~30 µs) *plus* IPC round-trips through the
supervisor (~50 µs each way) — call it 350 µs end-to-end.

Now imagine the robotics-rt configuration: 1 IMU at 1 kHz, 1 LIDAR at 50
Hz, 4 ESC controllers at 500 Hz, 8 servo PWM updates at 200 Hz, GPS over
UART at 10 Hz, plus a planner loop reading shared state. That is
~6000 HAL calls per second sustained, peaks of 10k. The supervisor's main
event loop today handles ~50k IPC messages/sec under benchmark; adding
HAL syscalls per message means the supervisor *is* the scheduler for the
entire robot's I/O fabric.

If even one HAL call performs a 100 ms blocking I2C transfer (an EEPROM
page write, a battery-fuel-gauge wake) on the main event loop, every other
IPC freezes for that duration. Apps doing real-time control will miss
their deadlines.

**The proposal's only mention of concurrency** is "HAL calls run on a
dedicated thread per bus." That is not enough.

- A single thread per I2C bus means transactions on bus 1 serialize, even
  when an app is happy to wait. Fine in principle, but the *queueing
  policy* is undefined. Is it FIFO? Priority? Bounded queue with what
  back-pressure behavior? What happens when 30 apps each post a request
  on bus 1 with a 5 ms slot each — the 30th app waits 145 ms.
- No mention of *cancellation*. If app B is waiting in the I2C queue and
  the supervisor decides to kill app B, the queued transaction must be
  cancelled before it executes; otherwise the supervisor performs an I/O
  on behalf of a dead process.
- No mention of *priorities*. A real-time control loop must preempt a
  background telemetry read. Without priority queues at the bus
  scheduler, the HAL cannot satisfy the robotics-rt platform's real-time
  guarantees.

**What this implies:** the HAL needs a per-bus *scheduler* component, not
just a worker thread. Concretely:

```
HalBusScheduler<Bus> {
    queue: PriorityQueue<Request>,   // priority from app manifest
    in_flight: Option<Request>,
    cancellation: HashMap<AppId, Vec<RequestId>>,
    metrics: BusMetrics,             // p50/p99 latency, queue depth
}
```

with documented bounds: max queue depth (configurable, default 32), max
single-transaction time (manifest-declared, supervisor enforces with
hard timeout), and a public per-bus latency budget exposed via the
observability subsystem. Without those, the HAL silently degrades under
load and we get an undebuggable "the robot stuttered" report.

**Cost:** another ~600 LOC plus a redesign of the supervisor's
`ipc_handlers` module to route HAL requests through the bus scheduler
rather than handling them inline. Non-trivial but unavoidable.

---

### C3. mcu-minimal has no WIT, so it has no `vyoma:hal`

The proposal is internally inconsistent on this point. It says the HAL is
defined as a WIT package (`vyoma:hal@0.1.0`), and it says mcu-minimal uses
wasm3. These two statements cannot both be true.

**What wasm3 actually is.** wasm3 is an interpreter for the WebAssembly
1.0 *core* spec. It implements: i32/i64/f32/f64 ops, linear memory, tables,
control flow, host function imports by string name. That is it.

**What it does not implement.** No component model. No interface types.
No WASI Preview 2 (no `wasi:io/streams`, no `wasi:cli`, none of it). No
resource handles. No async functions. No multi-memory. WIT *requires* the
component model. There is no path to compile `vyoma:hal` for wasm3.

**What this implies.** mcu-minimal apps must use a totally different
binding mechanism:

- Raw core WASM module (not a component).
- Host imports by string from a single namespace, e.g. `vyoma_hal`.
- Function signatures in core WASM types only: i32 for handles, i32 for
  pointers into linear memory, i32 for length, i32 for status code.
- Apps explicitly serialize their request/response into a memory region
  whose address they pass to the import.

The HAL must therefore offer TWO host-side implementations and TWO
client-side libraries:

| Tier | Runtime | Binding | App language |
|------|---------|---------|--------------|
| A | Wasmtime (desktop, mobile, server, iot-edge, robotics-rt) | WIT component | Rust with `wit-bindgen` |
| B | wasm3 (mcu-minimal) | Core import | Rust with `#[link(wasm_import_module = "vyoma_hal")]` |

The *semantics* must be identical: a `gpio_write(handle, 1)` from a Tier B
app must reach the same kernel call as `gpio.write(handle, true)` from a
Tier A app. But the surface, the codegen, and the calling convention are
disjoint. Hand-waving this away with "the HAL has one API" is wrong.

**Concrete remedy.** Define the HAL in a runtime-neutral specification
(say `docs/hal-protocol.md`) that lists every operation by name, arg
types in *abstract* terms (handle, address, byte slice, status), and
ABI mappings for both tiers. WIT is then a *projection* of the protocol
for component-capable runtimes; raw imports are another projection. The
single source of truth is the protocol document, not the WIT file.

This also clarifies what we tell users: an app written for desktop-full
can be ported to mcu-minimal *with a recompile* iff it sticks to a HAL
subset that the wasm3 binding supports (no resource handles for streams,
no async). Apps that rely on the full WIT surface (e.g. interrupt
subscriptions as streams) cannot run on mcu-minimal at all.

**Cost.** Documenting the protocol is a one-time write, perhaps a day. The
wasm3 binding is real work — another ~500 LOC of host glue plus a
client-side library. Plus CI to keep both bindings in lockstep when the
protocol evolves.

---

### C4. Exclusive ownership over-fits the simple case

The proposal's capability model is: each GPIO pin, each I2C bus, each SPI
bus is owned by exactly one app, declared in `vyoma.toml`, enforced by the
`PeripheralRegistry`. Conflicts are detected at boot and the offending app
fails to start.

This is the right model for an LED on GPIO 17. It is the wrong model for
several realistic cases that *will* appear in our target platforms.

**Case 4a: shared interrupt line.** A common board layout has an INT pin
from a sensor wired to a host GPIO; the host configures the GPIO as
input with edge interrupt and the sensor's I2C address handler reads the
status register on each interrupt. Now suppose two sensors share the
interrupt line (datasheet says "if INT, poll both"). Two apps need to
*read* the GPIO state. Neither should *write*. Exclusive ownership
rejects this configuration at boot.

**Case 4b: I2C bus with multiple devices.** This is the *normal* case for
I2C. A bus has up to 127 addresses; the bus has *one* SDA/SCL pair, but
many devices, often owned by different apps (temperature sensor → climate
app, fuel gauge → battery app, accelerometer → motion app). Exclusive
bus ownership forces all three apps into one mega-app, which defeats the
isolation argument that justifies WASM in the first place.

The correct primitive is *bus access with address arbitration*: many apps
share the bus, each holds a slice of addresses, the supervisor's bus
scheduler interleaves transactions on different addresses.

**Case 4c: SPI chip selects.** SPI has one bus (SCLK/MOSI/MISO) and N
chip-select GPIOs. Conventionally, each device is owned by a different
app, and the bus is shared. Exclusive bus ownership again fails this
common topology. The HAL must distinguish "bus capability" from "device
on bus capability."

**Case 4d: PWM channels on a single timer.** Many MCUs and SoCs have N
PWM channels driven by one timer; channels can be configured
independently *as long as the timer base frequency is consistent*. Two
apps configuring different timer bases on channels sharing the same
timer is the conflict; two apps using the same timer base on different
channels is fine. The HAL must reason about *shared resources beneath
the abstraction* (the timer), not just the visible handles (the channels).

**What this implies.** The capability vocabulary needs three modes,
modelled on POSIX file open flags:

| Mode | GPIO | I2C | SPI | PWM |
|------|------|-----|-----|-----|
| `exclusive` | output, exclusive read | bus master with no co-tenants | bus master | timer + channel |
| `shared-read` | input with no edge config conflict | address read-only | n/a | n/a |
| `address` | n/a | one address on shared bus | one chip-select on shared bus | one channel on shared timer |
| `notify` | input + edge subscribe (multiple subscribers OK) | n/a | n/a | n/a |

Manifest syntax:

```toml
[capabilities.gpio]
pins.17 = "exclusive-output"
pins.22 = "shared-read"
pins.23 = { mode = "notify", edge = "both", debounce_us = 500 }

[capabilities.i2c]
bus = 1
addresses = [0x48, 0x50]   # implicit "address" mode

[capabilities.spi]
bus = 0
cs_pins = [25]             # implicit "address" mode
```

The registry validates at boot: only one `exclusive-output` claim per pin
allowed; any number of `shared-read` on the same pin allowed; one
exclusive holder of an I2C bus precludes address-mode peers; etc.

**Cost.** Manifest grammar revision, registry rewrite (~400 LOC), test
matrix for every combination of mode conflicts. Worth it: the
exclusive-only model is dead on arrival for any board with more than two
sensors.

---

### C5. Handle release is not safe-state recovery

The proposal says: when an app crashes, the supervisor releases its HAL
handles. Implicit assumption: closing the handle returns the hardware to
a safe state. False, sometimes catastrophically.

**The physics.** A GPIO pin configured as output driving HIGH stays at
3.3V (or whatever Vcc) on the pin until something else drives it. Closing
the kernel handle (in gpiod terms, releasing the line request) sets the
pin direction back to input, which floats it — the actual voltage on the
pin is then determined by external pull resistors or downstream
capacitance. *Float is not safe.* For a motor controller MOSFET gate
listening to that pin, "float" is "intermediate voltage" is "MOSFET in
linear region" is "burns up in seconds."

**Other actuator examples.**
- A PWM channel commanding 50% duty to a servo: handle release leaves
  the timer running at 50%; servo holds position; if the position was
  unsafe (arm overextended), it stays unsafe until a watchdog kicks.
- A heater MOSFET commanded ON via GPIO: handle release → input float →
  effectively still ON if there's a pull-up. Heater continues to heat
  until physical thermal fuse trips.
- An I2C device commanded "valve open" via a register write: handle
  release does nothing to the device. Valve stays open.

**The proposal's blind spot.** None of this is mentioned. The capability
manifest does not declare a safe-state per pin/channel. The supervisor's
crash handler does not run any sequence to drive outputs to safe levels
before releasing the kernel handle.

**What this implies.** Manifests must declare per-resource safe-state,
and the HAL must execute that state transition *before* releasing the
kernel handle. Sketch:

```toml
[capabilities.gpio]
pins.17 = { mode = "exclusive-output", safe_state = "low" }
pins.22 = "shared-read"   # input pins have no safe state

[capabilities.pwm]
channel = 3
safe_state = { duty = 0.0 }   # explicit zero, not "float"

[capabilities.i2c]
bus = 1
addresses = [0x48]
on_crash = [
  { addr = 0x48, write = "0x00 0x00" },   # write VALVE_OFF register
]
```

The supervisor's crash handler then executes:

1. Drain in-flight HAL requests from the dead app's queues (cancellation).
2. For each held capability, run the declared safe-state sequence.
3. Only after safe-state writes complete, release the kernel handles.

Step 2 may itself fail (the bus may be wedged, the kernel may return
EIO). The supervisor needs a *fallback*: a hard pin-level safe state
configured at the platform-profile level (board-specific TOML) that says
"if app-declared safe state cannot be executed, drive pin X to level L
via a separate kernel handle held by the supervisor itself."

**Cost.** Manifest grammar; supervisor crash-handler rework (~300 LOC);
platform-profile additions; integration tests that kill apps mid-actuation
and verify pins go to declared safe levels. This is *safety-critical
plumbing* and must not be deferred.

---

### C6. Platform feature flags explode and prevent universal binaries

The proposal sketches platform selection via Cargo features:
`#[cfg(feature = "platform-iot-edge")]` etc. Six platforms means six
features that need to be *mutually exclusive*, because the HAL adapter
selection (`hal::backend`) needs exactly one implementation linked in.

**Failure mode 1: no universal binary.** If `platform-iot-edge` and
`platform-robotics-rt` cannot coexist as features in a single compile, we
cannot ship a single supervisor binary that runs on both. Every board
needs its own build. This contradicts the project goal of a unified
WASM-first OS.

**Failure mode 2: CI matrix explosion.** Six features × test suite per
platform × `cargo test` not actually exercising the platform-specific
backend (you'd be running the host's backend on the build runner, not the
target's). The current CI plan (one `make test` invocation) tests one
platform. We have zero coverage on the other five.

**Failure mode 3: feature interaction with other crates.** Many crates in
the supervisor's dep tree have features that interact with our platform
features (e.g. `tokio` features, `embedded-hal` versions). A feature
flag that means "include platform X's adapter" gets confused with feature
flags meaning "include this optional dep." The Cargo feature flag system
does not have namespacing strong enough to manage this in a 50-feature
tree.

**What this implies.** Platform selection should be *runtime*, not
compile-time, with all backends linked in.

Concretely:

```rust
pub trait GpioBackend: Send + Sync {
    fn open(&self, chip: &str, line: u32, cfg: LineConfig) -> Result<LineHandle>;
    // ...
}

pub struct HalRegistry {
    gpio: Box<dyn GpioBackend>,
    i2c:  Box<dyn I2cBackend>,
    // ...
}

impl HalRegistry {
    pub fn from_profile(profile: &PlatformProfile) -> Self {
        let gpio: Box<dyn GpioBackend> = match profile.gpio_backend.as_str() {
            "linux-gpiod" => Box::new(LinuxGpiodBackend::new()),
            "mock"        => Box::new(MockGpioBackend::new()),
            "wasm3-mcu"   => Box::new(McuGpioBackend::new()),
            other => panic!("unknown gpio backend: {other}"),
        };
        // ...
    }
}
```

A single supervisor binary contains all backends; the platform profile
TOML picks one at boot. Cost: a couple percent binary-size increase from
linking unused backends, in exchange for one binary, one CI test pass that
exercises all backends via the mock plus a thin integration test per real
backend, and no Cargo-feature interaction hell.

**Where compile-time selection is still appropriate.** mcu-minimal genuinely
cannot link Wasmtime (binary too large), so the *runtime* selection must
itself be a compile-time choice. The right rule is: two top-level targets
(`platform-class = "wasmtime"` vs `platform-class = "wasm3"`) at compile
time, all backends within a class at runtime. Two binaries, not six.

---

## Significant Issues (important)

Issues below should be addressed before merge but are not as dangerous as
C1–C6 if temporarily deferred.

### S1. ADC sampling rate is a per-platform mess

The proposal lists `ADC` as a HAL trait with a `read(channel) -> u16`
method. This works for an ADC connected via i2c (one-shot conversion) or
an MCU's built-in ADC (~1 µs conversion). It does not work for streaming
ADC use cases: audio sampling at 48 kHz, oscilloscope captures at 1 MHz,
sensor fusion at 10 kHz. Those need a *streaming* API with a ring
buffer, DMA-backed where the kernel supports it.

The HAL needs at minimum two ADC abstractions: `OneShotAdc` and
`StreamingAdc`. Apps that only need polling use the one-shot; apps that
need streaming declare a buffer-size capability. Without this, audio and
motor-feedback workloads can never use the HAL.

### S2. UART flow control is undefined

The proposal lists `Uart` with `read`/`write`. It does not mention RTS/CTS
hardware flow control, XON/XOFF software flow control, parity, stop bits,
or break-signal generation. All of these are real-world UART requirements
— a GPS at 9600 8N1 differs from an LTE modem at 115200 8N1 with hardware
flow control differs from an industrial RS-485 link at 38400 8E1.

Configuration must move into the manifest:

```toml
[capabilities.uart]
port = 1
baud = 115200
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "rtscts"
```

The HAL must propagate this into the underlying termios call (Linux) or
register configuration (MCU).

### S3. No story for memory-mapped peripherals

Many embedded peripherals are not behind GPIO/I2C/SPI but behind direct
memory-mapped registers (display controllers, CAN controllers, custom
FPGA fabrics on robotics-rt). The HAL has no `Mmio` trait. Apps wanting
to drive such peripherals must escape the HAL entirely, defeating its
purpose.

A `MmioRegion` capability with declared base address and length,
mapped into the app's WASM linear memory at boot, would close this gap.
The kernel-side enforcement uses `/dev/mem` (privileged) or the udmabuf
character device. Security implications need their own design pass.

### S4. Interrupt latency is unbounded

The proposal describes interrupts as WIT streams: app subscribes,
supervisor sends events on the stream when the GPIO edge fires. Sounds
clean. The hidden cost: each edge → kernel poll → supervisor wakes →
supervisor finds owning app → supervisor pushes to WIT stream → wasmtime
schedules app → app drains stream. Five context switches per interrupt.
Typical latency: 100 µs–1 ms. For robotics-rt's stated "real-time
guarantees," that is too slow.

Options:
- Document the latency floor honestly (~500 µs typical on Linux, worse
  on a loaded system) and tell users to do hard real-time in a separate
  Linux process not under wasmtime.
- Offer an "interrupt batching" mode: app declares "wake me when N edges
  have accumulated or T µs have passed," supervisor coalesces. Works for
  encoder ticks, not for emergency-stop signals.
- Push interrupt handling into a kernel module written by us, with a
  ring buffer the app reads via `read()`. Eliminates supervisor hop.
  Highest performance, highest engineering cost.

The proposal picks none of these. The HAL design must explicitly state
the latency contract.

### S5. PWM resolution and frequency constraints

The proposal lists `Pwm.set_duty(channel, fraction)` and
`set_frequency(channel, hz)`. It does not address that:

- PWM frequency is per-*timer*, not per-channel. Two apps configuring
  different frequencies on channels of the same timer conflict.
- PWM resolution decreases as frequency increases (`resolution_bits =
  log2(timer_clock_hz / pwm_freq_hz)`); at 20 kHz on a 100 MHz timer you
  get 12 bits of duty; at 200 kHz you get 9.
- Some platforms cannot do arbitrary frequencies, only those expressible
  as `timer_clock / prescaler / period`.

The HAL must surface these constraints, ideally as a `query_capabilities`
call that returns the achievable frequency range and resolution.

### S6. Manifest schema migration story

We are extending `vyoma.toml` with: GPIO modes (C4), safe states (C5),
UART parameters (S2), MMIO regions (S3). That is a significant schema
revision. The proposal does not describe:

- How existing manifests parse against the new schema (back-compat?).
- Versioning: do we bump the manifest version field? What does the
  supervisor do with an old-version manifest?
- Tooling: is there a `vyoma manifest migrate` command?

Without a migration plan, every existing app needs hand-editing on every
HAL revision.

### S7. Testing strategy for hardware without hardware

The proposal mentions `MockGpioBackend` and similar. Mocks let unit tests
run, but they do not catch:

- Real ioctl error codes the kernel returns on edge cases (EBUSY when
  another process holds the line, ENODEV when the chip is unplugged,
  EAGAIN on non-blocking reads with no data).
- Timing-sensitive bugs (the supervisor's bus scheduler under load).
- Hardware-specific quirks (the BCM2835 GPIO controller has a
  per-bank-of-32 register write that races with reads).

We need at minimum:
- A QEMU-based integration test that uses QEMU's gpiochip emulation for
  Linux backends.
- A small "HAL conformance suite" that runs on real hardware at release
  time, on each supported board, with results published.

Neither is described.

---

## Design Gaps

These are not bugs in the proposal so much as missing chapters.

### G1. No power-management interaction

R7 (power) established sleep states. The HAL says nothing about what
happens to a held GPIO across a `S3` suspend → resume. Linux gpiod will
re-apply the line configuration on resume (recent kernels), but apps
must not assume the pin state held during sleep. The HAL needs a
"suspend hook" callback so apps can save state before sleep and a
"resume hook" so they can restore.

### G2. No HAL telemetry

No mention of per-bus throughput counters, per-pin toggle counts, per-app
HAL-error rates exposed via the observability subsystem. Debugging a
"the GPIO isn't toggling fast enough" report without these metrics is
guesswork. Each backend should emit counters into the observability
heartbeat by default.

### G3. No HAL versioning for ABI evolution

`vyoma:hal@0.1.0` will become `vyoma:hal@0.2.0`. Apps compiled against
0.1.0 must keep running. WIT supports semver; the wasm3 raw-import
binding does not. We need either dual-version host shims (run 0.1.0 and
0.2.0 host implementations concurrently, route per-app) or a hard
"recompile every app on HAL upgrade" rule. Pick one and document it.

### G4. No security model for malicious manifests

An app's `vyoma.toml` declares "I need GPIO 17." Who validates that GPIO
17 is *safe* for that app to hold? A user installing a downloaded app
that claims GPIO 17 will not know that pin 17 drives the e-stop. The
manifest review needs to be a step in the package-manager install flow,
with platform-profile-declared "dangerous pins" requiring an explicit
operator confirmation. Currently undefined.

### G5. No clock-source policy

ADC sample rates, PWM frequencies, UART baud rates all depend on
underlying clock sources whose accuracy varies (±50 ppm crystal vs
±2% RC oscillator vs system clock derived from PLL). The HAL exposes
nominal values; real values may differ. For sensor-fusion code,
clock-source accuracy matters. The HAL should expose a `clock_accuracy_ppm`
query per peripheral.

### G6. No documentation of error semantics

What does `i2c.write_read(addr, ...)` return if no device ACKs? If the
bus is wedged? If the kernel returns EINTR? The HAL needs a documented
error taxonomy with stable error codes that apps can switch on. Without
it, every app has to handle "unknown error" generically and we lose
diagnostic information.

---

## Points of Strength

The proposal is wrong in important ways, but it gets several big things
right and those should be preserved through the FINAL.

**P1. Single HAL trait surface across desktop and embedded.** Aiming for
one API that covers GPIO/I2C/SPI/UART/ADC/PWM at every platform is the
right ambition. Even if mcu-minimal requires a separate binding
mechanism (C3), the *semantic* shared surface lets us write portable
sensor drivers once.

**P2. Capability-secure HAL fits the supervisor model.** Treating
hardware access as a capability listed in `vyoma.toml`, enforced by the
supervisor, lines up cleanly with the rest of VyomaOS's security model.
The capability vocabulary needs expansion (C4) but the underlying idea
is correct.

**P3. Decoupling the trait from the backend.** The HAL trait definitions
live in `supervisor/src/hal/`, separate from the Linux adapter
implementations. This is a sound layering — the same trait can be
implemented for Linux gpiod, MCU registers, and mocks for tests.

**P4. Per-platform profile TOMLs.** The profile loader is a clean place
to put board-specific differences (which GPIO chip corresponds to which
pin number, what's the default I2C bus). Worth keeping; needs extension
to include safe-state defaults (C5) and dangerous-pin lists (G4).

**P5. Three-tier driver model from R6.** R6 established kernel-side
drivers + supervisor-side capability mediators + app-side WASM drivers.
The HAL slots naturally into the supervisor tier, exposing kernel
drivers to WASM apps with capability gating. The architecture is right.

**P6. WIT for component-capable platforms.** WIT is the right ABI for
Wasmtime-targeting platforms — strong types, resource handles, semver
versioning, codegen across host languages. The mistake is assuming it
covers mcu-minimal; the use for the other five platforms stays correct.

These points should anchor the FINAL synthesis.

---

## Synthesis Recommendations

The HAL FINAL should reorganize the proposal around the following
principles and concrete steps.

### Step 1: Replace sysfs with gpiod (C1)

Adopt `gpio-cdev`-style ioctls on Linux. Spec the exact ioctl set we
will support (`GPIO_V2_LINE_REQUEST_IOCTL`, `GPIO_V2_LINE_GET_VALUES_IOCTL`,
`GPIO_V2_LINE_SET_VALUES_IOCTL`, `GPIO_V2_GET_LINEINFO_IOCTL`,
`GPIO_V2_GET_LINEINFO_WATCH_IOCTL`). Document the minimum kernel version
(4.16 for GPIO_V2). Write the wrapper as a small in-tree crate under
`supervisor/src/hal/linux/gpiod.rs`. Drop sysfs entirely; no fallback.

### Step 2: Design per-bus schedulers (C2)

Specify the bus scheduler component:

- Bounded priority queue per I2C/SPI bus.
- Manifest-declared priority per app (`hal_priority = "high|normal|low"`).
- Per-transaction timeout from app manifest.
- Cancellation hooks for crashed/killed apps.
- Metrics: queue depth, wait time p50/p99, in-flight time p50/p99,
  exported through observability heartbeats.

Document the latency budget per platform: e.g., desktop-full has no
real-time guarantee; robotics-rt promises p99 ≤ 200 µs for high-priority
SPI transactions of ≤ 64 bytes.

### Step 3: Two-tier HAL: WIT for Wasmtime, raw imports for wasm3 (C3)

Write `docs/hal-protocol.md` as the single source of truth listing every
operation in abstract terms. Generate WIT from it (or hand-write WIT to
match it, with a CI check that they're in sync). Hand-write a Rust
client library for wasm3 that emits the raw imports. Document the wasm3
subset: no resource handles, no streams; one-shot operations only.

### Step 4: Three-mode capability vocabulary (C4)

Extend the manifest grammar: `exclusive-output | shared-read | notify`
for GPIO; `exclusive-bus | address` for I2C/SPI. Rewrite the
`PeripheralRegistry` to validate the richer model. Add integration tests
for every conflict matrix entry.

### Step 5: Safe-state-on-release (C5)

Every output-capable capability requires a `safe_state` declaration in
the manifest. The HAL crash handler executes safe-state writes before
releasing kernel handles. Platform profiles provide board-level fallback
safe states for pins the app failed to declare. Add a section to
`docs/vyoma-manifest-schema.md` describing safe states.

### Step 6: Runtime backend selection, two compile-time tiers (C6)

Drop per-platform Cargo features. Link all Linux backends + mock backend
into one supervisor binary per runtime class. Compile-time split is
`runtime = "wasmtime"` vs `runtime = "wasm3"`, picked per Makefile
platform target. Within a class, the platform profile selects backends
at boot. CI builds two binaries; integration tests cover all backends
via mocks plus thin smoke tests on at least one real board per profile
class.

### Step 7: Plug the design gaps (S1–S7, G1–G6)

In rough priority order for the FINAL:

1. Manifest schema migration plan and version bump (S6).
2. Documented error taxonomy (G6).
3. UART configuration in manifest (S2).
4. PWM constraint query (S5).
5. Streaming ADC API (S1).
6. Interrupt latency contract (S4).
7. HAL telemetry counters (G2).
8. Power-management hooks (G1).
9. MMIO capability (S3).
10. Clock-accuracy query (G5).
11. Dangerous-pin operator confirmation flow (G4).
12. HAL version evolution policy (G3).

### Step 8: A conformance test suite

Define a HAL conformance test crate that runs against any backend
(mock, Linux gpiod, MCU). Each backend ships a CI job that runs the
suite. The conformance suite is the *executable specification* of the
HAL — anywhere the prose disagrees with the tests, the tests win.

### Step 9: Phased roll-out

Ship the HAL in three phases instead of one big bang:

- **Phase HAL-A** (week 1–2): gpiod backend + basic GPIO + manifest mode
  vocabulary + safe-state. Smallest useful surface.
- **Phase HAL-B** (week 3–4): I2C + SPI with bus schedulers + address-mode
  capabilities. Real sensor support.
- **Phase HAL-C** (week 5–6): UART + ADC streaming + PWM constraints +
  conformance suite + wasm3 binding. Completes the matrix.

Each phase has its own design review, its own tests, its own CI gate.

### Final verdict

The HAL proposal as drafted has the right ambition and the wrong details.
None of the criticisms here are insurmountable; each has a known
solution in the embedded-Linux and component-model communities. We
should not merge the current draft. We should produce an HAL-FINAL that
addresses C1 through C6 explicitly and incorporates at least S1, S2, S5,
S6, G6 from the significant-issues list. The remaining gaps can be
phased in but must be acknowledged in the design document so they are
not surprises later.

The R6 three-tier driver model gave us a solid foundation. The HAL is
where we connect that foundation to the messy real world of pin
multiplexers, bus arbiters, kernel API churn, and safety-critical
actuators. Spending one more design pass here is cheaper than re-doing
it after the first robotics-rt customer melts a MOSFET because we
released an output pin without driving it low first.
