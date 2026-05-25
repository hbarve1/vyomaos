// Extended capability system — T006
//
// `PeripheralCapability` models the per-peripheral hardware capabilities that
// can be declared in a `vyoma.toml` manifest.  The enforcer logic lives in
// peripheral.rs and is invoked at spawn time.

pub mod peripheral;

pub use peripheral::{PeripheralCapability, PeripheralEnforcer, PeripheralRegistry};

// ── PeripheralCapabilitySet ───────────────────────────────────────────────────

/// The full set of peripheral capabilities declared by one module.
///
/// Each field maps to a `[capabilities.X]` section in `vyoma.toml`.
/// `None` means the section was absent — access to that peripheral is denied.
#[derive(Debug, Default, Clone)]
pub struct PeripheralCapabilitySet {
    pub gpio: Option<GpioCapability>,
    pub i2c: Option<I2cCapability>,
    pub spi: Option<SpiCapability>,
    pub uart: Option<UartCapability>,
    pub adc: Option<AdcCapability>,
}

// ── Individual peripheral capability structs ──────────────────────────────────

/// GPIO capability declaration.
#[derive(Debug, Clone, Default)]
pub struct GpioCapability {
    /// Pin numbers allowed (empty = all pins denied despite capability being present).
    pub pins: Vec<u8>,
    /// Allowed direction.  `None` = both input and output allowed.
    pub direction: Option<crate::hal::Direction>,
}

/// I2C bus capability declaration.
#[derive(Debug, Clone, Default)]
pub struct I2cCapability {
    /// Bus index.
    pub bus: u8,
    /// Device address (0x00–0x7F).  `None` = any address allowed on the bus.
    pub address: Option<u8>,
}

/// SPI bus capability declaration.
#[derive(Debug, Clone, Default)]
pub struct SpiCapability {
    pub bus: u8,
    /// Chip-select pin.  `None` = any CS pin allowed.
    pub cs_pin: Option<u8>,
}

/// UART capability declaration.
#[derive(Debug, Clone, Default)]
pub struct UartCapability {
    pub port: u8,
    pub baud: Option<u32>,
}

/// ADC capability declaration.
#[derive(Debug, Clone, Default)]
pub struct AdcCapability {
    /// ADC channel indices allowed.
    pub channels: Vec<u8>,
}
