// Peripheral capability enforcer — T006 + T014 + T037
//
// Parses `[capabilities.gpio]`, `[capabilities.i2c]` etc. from a vyoma.toml
// TOML value and enforces access at spawn time.
//
// T037: `PeripheralRegistry` — tracks which module owns each peripheral resource
// and rejects conflicting exclusive-access claims at spawn time.

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
        // Parse the TOML text directly as a RawPeripherals struct.
        // If the text is empty, use defaults.
        let raw: RawPeripherals = if toml_text.trim().is_empty() {
            RawPeripherals::default()
        } else {
            toml::from_str(toml_text)
                .map_err(|e| format!("peripheral capability parse error: {e}"))?
        };

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

// ── PeripheralRegistry — T037 ─────────────────────────────────────────────────

/// Identifies a unique peripheral resource that can be exclusively owned.
///
/// For GPIO Output pins and UART ports, the resource is exclusive (one owner).
/// For GPIO Input pins and I2C devices, multiple readers are allowed on the
/// same resource.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ResourceKey {
    /// GPIO output pin — exclusive.
    GpioOutput(u8),
    /// UART port — exclusive.
    UartPort(u8),
    /// I2C device (bus + address) — exclusive.
    I2cDevice(u8, u8),
    /// SPI device (bus + cs) — exclusive.
    SpiDevice(u8, u8),
}

/// System-wide peripheral ownership registry.
///
/// Tracks which module has claimed each exclusive peripheral resource.
/// Must be consulted before spawning any module that declares peripheral
/// capabilities — if a conflict is detected the spawn is rejected.
pub struct PeripheralRegistry {
    /// Maps exclusive resource → owning module name.
    owners: std::collections::HashMap<ResourceKey, String>,
}

impl PeripheralRegistry {
    pub fn new() -> Self {
        Self { owners: std::collections::HashMap::new() }
    }

    /// Attempt to claim a peripheral resource on behalf of `module_name`.
    ///
    /// Returns `Ok(())` if the resource is available (or is non-exclusive).
    /// Returns `Err(String)` with a descriptive conflict message otherwise.
    pub fn claim(
        &mut self,
        module_name: &str,
        capability: &PeripheralCapability,
    ) -> Result<(), String> {
        let key = match capability {
            PeripheralCapability::Gpio { pin, direction } => {
                // Only output pins are exclusive; input pins can be shared.
                if *direction == Direction::Output {
                    Some(ResourceKey::GpioOutput(*pin))
                } else {
                    None // Input — shared, no conflict.
                }
            }
            PeripheralCapability::Uart { port } => {
                Some(ResourceKey::UartPort(*port))
            }
            PeripheralCapability::I2c { bus, addr } => {
                Some(ResourceKey::I2cDevice(*bus, *addr))
            }
            PeripheralCapability::Spi { bus, cs } => {
                Some(ResourceKey::SpiDevice(*bus, *cs))
            }
            PeripheralCapability::Adc { .. } => {
                None // ADC channels are read-only; sharing is safe.
            }
        };

        if let Some(key) = key {
            if let Some(owner) = self.owners.get(&key) {
                return Err(format!(
                    "peripheral conflict: {key:?} already owned by '{owner}'; \
                     '{module_name}' cannot claim exclusive access"
                ));
            }
            self.owners.insert(key, module_name.to_string());
        }

        Ok(())
    }

    /// Release all peripheral claims held by `module_name`.
    ///
    /// Called when a module exits or is killed.
    pub fn release(&mut self, module_name: &str) {
        self.owners.retain(|_, owner| owner != module_name);
    }

    /// Return the name of the module that owns a resource, if any.
    #[cfg(test)]
    pub fn owner_of(&self, capability: &PeripheralCapability) -> Option<&str> {
        let key = match capability {
            PeripheralCapability::Gpio { pin, direction } => {
                if *direction == Direction::Output {
                    Some(ResourceKey::GpioOutput(*pin))
                } else {
                    None
                }
            }
            PeripheralCapability::Uart { port } => Some(ResourceKey::UartPort(*port)),
            PeripheralCapability::I2c { bus, addr } => Some(ResourceKey::I2cDevice(*bus, *addr)),
            PeripheralCapability::Spi { bus, cs } => Some(ResourceKey::SpiDevice(*bus, *cs)),
            PeripheralCapability::Adc { .. } => None,
        };
        key.and_then(|k| self.owners.get(&k).map(|s| s.as_str()))
    }
}

impl Default for PeripheralRegistry {
    fn default() -> Self {
        Self::new()
    }
}
