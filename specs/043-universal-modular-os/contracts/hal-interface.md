# Contract: Hardware Abstraction Layer (HAL)

The HAL provides platform-specific hardware access to WASM modules through supervisor-mediated host functions. Each hardware interface is a separate trait that platforms implement.

## Trait: `GpioDriver`

```
fn pin_mode(pin: u8, direction: Direction) → Result<()>
fn digital_write(pin: u8, value: bool) → Result<()>
fn digital_read(pin: u8) → Result<bool>
fn analog_read(pin: u8) → Result<u16>
```

## Trait: `I2cDriver`

```
fn write(bus: u8, addr: u8, data: &[u8]) → Result<()>
fn read(bus: u8, addr: u8, len: usize) → Result<Vec<u8>>
fn write_read(bus: u8, addr: u8, write: &[u8], read_len: usize) → Result<Vec<u8>>
```

## Trait: `SpiDriver`

```
fn transfer(bus: u8, cs: u8, data: &[u8]) → Result<Vec<u8>>
fn write(bus: u8, cs: u8, data: &[u8]) → Result<()>
```

## Trait: `UartDriver`

```
fn configure(port: u8, baud: u32, bits: u8, parity: Parity, stop: u8) → Result<()>
fn write(port: u8, data: &[u8]) → Result<usize>
fn read(port: u8, buf: &mut [u8], timeout_ms: u32) → Result<usize>
```

## Trait: `AdcDriver`

```
fn read_channel(channel: u8) → Result<u16>
fn read_voltage_mv(channel: u8) → Result<u32>
```

## Capability Enforcement

Before any HAL call reaches the driver implementation, the supervisor checks:

1. The calling module's vyoma.toml declares the relevant peripheral capability
2. The specific pin/bus/port/channel is listed in the module's allowed set
3. The direction (input/output) matches the declaration

If any check fails, the call returns an error and the supervisor logs a capability denial event.

## Platform Registration

Each platform provides a `HalProvider` that returns concrete implementations:

```
fn gpio() → Option<Box<dyn GpioDriver>>
fn i2c() → Option<Box<dyn I2cDriver>>
fn spi() → Option<Box<dyn SpiDriver>>
fn uart() → Option<Box<dyn UartDriver>>
fn adc() → Option<Box<dyn AdcDriver>>
```

Returns `None` for peripherals not available on the platform. The supervisor uses this to validate capability manifests against available hardware.
