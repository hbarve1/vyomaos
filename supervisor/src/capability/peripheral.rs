// Peripheral capability enforcer — T006 + T014
//
// Parses `[capabilities.gpio]`, `[capabilities.i2c]` etc. from a vyoma.toml
// TOML value and enforces access at spawn time.

use serde::Deserialize;
use super::{
    AdcCapability, GpioCapability, I2cCapability, PeripheralCapabilitySet, SpiCapability,
    UartCapability,
};
use crate::hal::Direction;

// ── TOML deserialisation helpers ──────────────────────────────────────────────
// These mirror the TOML schema but are kept separate from the runtime structs.

#[derive(Debug, Default, Deserialize)]
struct RawGpio {
    #[serde(default)]
    pins: Vec<u8>,
    #[serde(default)]
    direction: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawI2c {
    #[serde(default)]
    bus: u8,
    #[serde(default)]
    address: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSpi {
    #[serde(default)]
    bus: u8,
    #[serde(default)]
    cs_pin: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
struct RawUart {
    #[serde(default)]
    port: u8,
    #[serde(default)]
    baud: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAdc {
    #[serde(default)]
    channels: Vec<u8>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPeripherals {
    gpio: Option<RawGpio>,
    i2c: Option<RawI2c>,
    spi: Option<RawSpi>,
    uart: Option<RawUart>,
    adc: Option<RawAdc>,
}

// ── PeripheralCapability ──────────────────────────────────────────────────────

/// A single checked peripheral access request from a module.
#[derive(Debug, Clone)]
pub enum PeripheralCapability {
    Gpio { pin: u8, direction: Direction },
    I2c { bus: u8, addr: u8 },
    Spi { bus: u8, cs: u8 },
    Uart { port: u8 },
    Adc { channel: u8 },
}

// ── PeripheralEnforcer ────────────────────────────────────────────────────────

/// Loaded from `vyoma.toml`; checked before any HAL call.
#[derive(Debug, Default, Clone)]
pub struct PeripheralEnforcer {
    pub caps: PeripheralCapabilitySet,
}

impl PeripheralEnforcer {
    /// Parse peripheral capabilities from the `[capabilities]` TOML table string.
    ///
    /// Accepts the raw TOML text of the whole `[capabilities]` section; only
    /// the peripheral sub-tables are examined here.
    pub fn from_toml(toml_text: &str) -> Result<Self, String> {
        // Wrap in a [root] so toml::from_str can parse it as a table.
        let wrapped = format!("[root]\n{toml_text}");

        #[derive(Deserialize)]
        struct Root {
            root: RawPeripherals,
        }

        let root: Root = toml::from_str(&wrapped)
            .map_err(|e| format!("peripheral capability parse error: {e}"))?;
        let raw = root.root;

        let gpio = raw.gpio.map(|g| {
            let direction = g.direction.as_deref().and_then(|d| match d {
                "input" => Some(Direction::Input),
                "output" => Some(Direction::Output),
                _ => None,
            });
            GpioCapability { pins: g.pins, direction }
        });

        let i2c = raw.i2c.map(|i| I2cCapability { bus: i.bus, address: i.address });
        let spi = raw.spi.map(|s| SpiCapability { bus: s.bus, cs_pin: s.cs_pin });
        let uart = raw.uart.map(|u| UartCapability { port: u.port, baud: u.baud });
        let adc = raw.adc.map(|a| AdcCapability { channels: a.channels });

        Ok(Self {
            caps: PeripheralCapabilitySet { gpio, i2c, spi, uart, adc },
        })
    }

    /// Check whether `request` is allowed by this enforcer.
    ///
    /// Returns `Ok(())` if allowed, `Err(String)` with a denial message otherwise.
    pub fn check(&self, request: &PeripheralCapability) -> Result<(), String> {
        match request {
            PeripheralCapability::Gpio { pin, direction } => {
                let gpio = self.caps.gpio.as_ref().ok_or_else(|| {
                    format!("GPIO access denied: capability not declared (pin {pin})")
                })?;
                if !gpio.pins.contains(pin) {
                    return Err(format!("GPIO access denied: pin {pin} not in allowed set"));
                }
                if let Some(allowed_dir) = gpio.direction {
                    if allowed_dir != *direction {
                        return Err(format!(
                            "GPIO access denied: pin {pin} direction mismatch"
                        ));
                    }
                }
                Ok(())
            }

            PeripheralCapability::I2c { bus, addr } => {
                let i2c = self.caps.i2c.as_ref().ok_or_else(|| {
                    format!("I2C access denied: capability not declared (bus {bus})")
                })?;
                if i2c.bus != *bus {
                    return Err(format!("I2C access denied: bus {bus} not allowed"));
                }
                if let Some(allowed_addr) = i2c.address {
                    if allowed_addr != *addr {
                        return Err(format!(
                            "I2C access denied: address 0x{addr:02X} not allowed on bus {bus}"
                        ));
                    }
                }
                Ok(())
            }

            PeripheralCapability::Spi { bus, cs } => {
                let spi = self.caps.spi.as_ref().ok_or_else(|| {
                    format!("SPI access denied: capability not declared (bus {bus})")
                })?;
                if spi.bus != *bus {
                    return Err(format!("SPI access denied: bus {bus} not allowed"));
                }
                if let Some(allowed_cs) = spi.cs_pin {
                    if allowed_cs != *cs {
                        return Err(format!(
                            "SPI access denied: CS pin {cs} not allowed on bus {bus}"
                        ));
                    }
                }
                Ok(())
            }

            PeripheralCapability::Uart { port } => {
                let uart = self.caps.uart.as_ref().ok_or_else(|| {
                    format!("UART access denied: capability not declared (port {port})")
                })?;
                if uart.port != *port {
                    return Err(format!("UART access denied: port {port} not allowed"));
                }
                Ok(())
            }

            PeripheralCapability::Adc { channel } => {
                let adc = self.caps.adc.as_ref().ok_or_else(|| {
                    format!("ADC access denied: capability not declared (channel {channel})")
                })?;
                if !adc.channels.contains(channel) {
                    return Err(format!(
                        "ADC access denied: channel {channel} not in allowed set"
                    ));
                }
                Ok(())
            }
        }
    }
}
