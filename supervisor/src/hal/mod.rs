// Hardware Abstraction Layer — T002
//
// Trait definitions for GPIO, I2C, SPI, UART, and ADC.
// Platforms supply concrete implementations via `HalProvider`.
// The supervisor uses `HalProvider` to validate capability manifests and
// to route WASM host-function calls to the correct driver.

pub mod gpio;
pub mod i2c;
pub mod uart;

pub use gpio::MockGpioDriver;
pub use i2c::MockI2cDriver;
pub use uart::MockUartDriver;

// ── Supporting types ──────────────────────────────────────────────────────────

/// GPIO pin direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

/// UART parity setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

// ── GPIO ──────────────────────────────────────────────────────────────────────

/// GPIO driver interface.
pub trait GpioDriver: Send + Sync {
    /// Set the direction (input / output) of a pin.
    fn pin_mode(&self, pin: u8, direction: Direction) -> Result<(), String>;

    /// Write a digital value to an output pin.
    fn digital_write(&self, pin: u8, value: bool) -> Result<(), String>;

    /// Read the digital level of an input pin.
    fn digital_read(&self, pin: u8) -> Result<bool, String>;

    /// Read an analogue value from a pin (platform-dependent resolution).
    fn analog_read(&self, pin: u8) -> Result<u16, String>;
}

// ── I2C ───────────────────────────────────────────────────────────────────────

/// I2C bus driver interface.
pub trait I2cDriver: Send + Sync {
    /// Write bytes to a device on `bus` at `addr`.
    fn write(&self, bus: u8, addr: u8, data: &[u8]) -> Result<(), String>;

    /// Read `len` bytes from a device on `bus` at `addr`.
    fn read(&self, bus: u8, addr: u8, len: usize) -> Result<Vec<u8>, String>;

    /// Combined write-then-read (register read pattern).
    fn write_read(
        &self,
        bus: u8,
        addr: u8,
        write: &[u8],
        read_len: usize,
    ) -> Result<Vec<u8>, String>;
}

// ── SPI ───────────────────────────────────────────────────────────────────────

/// SPI bus driver interface.
pub trait SpiDriver: Send + Sync {
    /// Full-duplex transfer: send `data` and return received bytes.
    fn transfer(&self, bus: u8, cs: u8, data: &[u8]) -> Result<Vec<u8>, String>;

    /// Write-only transfer (ignore received bytes).
    fn write(&self, bus: u8, cs: u8, data: &[u8]) -> Result<(), String>;
}

// ── UART ──────────────────────────────────────────────────────────────────────

/// UART serial driver interface.
pub trait UartDriver: Send + Sync {
    /// Configure baud rate, word length, parity, and stop bits.
    fn configure(
        &self,
        port: u8,
        baud: u32,
        bits: u8,
        parity: Parity,
        stop: u8,
    ) -> Result<(), String>;

    /// Write bytes to the UART port; returns the number of bytes written.
    fn write(&self, port: u8, data: &[u8]) -> Result<usize, String>;

    /// Read up to `buf.len()` bytes with a millisecond timeout.
    fn read(&self, port: u8, buf: &mut [u8], timeout_ms: u32) -> Result<usize, String>;
}

// ── ADC ───────────────────────────────────────────────────────────────────────

/// ADC (analogue-to-digital converter) driver interface.
pub trait AdcDriver: Send + Sync {
    /// Read the raw ADC count for a channel.
    fn read_channel(&self, channel: u8) -> Result<u16, String>;

    /// Read the voltage in millivolts for a channel.
    fn read_voltage_mv(&self, channel: u8) -> Result<u32, String>;
}

// ── HalProvider ───────────────────────────────────────────────────────────────

/// A platform registers drivers via this trait.
/// Returns `None` for peripherals unavailable on that platform.
/// The supervisor uses the returned options to validate capability manifests:
/// if a module declares `gpio = true` but `gpio()` returns `None`, spawning fails.
pub trait HalProvider: Send + Sync {
    fn gpio(&self) -> Option<&dyn GpioDriver>;
    fn i2c(&self) -> Option<&dyn I2cDriver>;
    fn spi(&self) -> Option<&dyn SpiDriver>;
    fn uart(&self) -> Option<&dyn UartDriver>;
    fn adc(&self) -> Option<&dyn AdcDriver>;
}
