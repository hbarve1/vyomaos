// HAL GPIO mock driver — T033
//
// Implements `GpioDriver` with per-pin state tracking backed by a `Mutex<HashMap>`.
// Designed for use in tests and in-process simulation without real hardware.

use std::{
    collections::HashMap,
    sync::Mutex,
};

use super::{Direction, GpioDriver};

// ── Pin state ─────────────────────────────────────────────────────────────────

/// State maintained per GPIO pin.
#[derive(Debug, Clone)]
pub struct PinState {
    pub direction: Direction,
    /// Digital level (true = high, false = low).
    pub level: bool,
    /// Analogue value (0–1023 typical).
    pub analog: u16,
}

impl Default for PinState {
    fn default() -> Self {
        Self {
            direction: Direction::Input,
            level: false,
            analog: 0,
        }
    }
}

// ── MockGpioDriver ────────────────────────────────────────────────────────────

/// Thread-safe GPIO mock.  All 40 pins default to input-low.
pub struct MockGpioDriver {
    pins: Mutex<HashMap<u8, PinState>>,
}

impl MockGpioDriver {
    /// Create a new mock with no pre-configured pins (lazily initialised on first access).
    pub fn new() -> Self {
        Self { pins: Mutex::new(HashMap::new()) }
    }

    /// Pre-seed a pin with a specific analogue value (useful in tests for
    /// `analog_read` assertions).
    pub fn set_analog(&self, pin: u8, value: u16) {
        let mut pins = self.pins.lock().unwrap();
        pins.entry(pin).or_default().analog = value;
    }

    /// Pre-seed a pin with a specific digital level (useful for `digital_read` tests).
    pub fn set_level(&self, pin: u8, level: bool) {
        let mut pins = self.pins.lock().unwrap();
        pins.entry(pin).or_default().level = level;
    }

    /// Return a snapshot of all pin states (for assertion in tests).
    pub fn snapshot(&self) -> HashMap<u8, PinState> {
        self.pins.lock().unwrap().clone()
    }
}

impl Default for MockGpioDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl GpioDriver for MockGpioDriver {
    fn pin_mode(&self, pin: u8, direction: Direction) -> Result<(), String> {
        let mut pins = self.pins.lock().unwrap();
        pins.entry(pin).or_default().direction = direction;
        Ok(())
    }

    fn digital_write(&self, pin: u8, value: bool) -> Result<(), String> {
        let mut pins = self.pins.lock().unwrap();
        let state = pins.entry(pin).or_default();
        if state.direction != Direction::Output {
            return Err(format!(
                "digital_write: pin {pin} is not set as output \
                 (call pin_mode first)"
            ));
        }
        state.level = value;
        Ok(())
    }

    fn digital_read(&self, pin: u8) -> Result<bool, String> {
        let pins = self.pins.lock().unwrap();
        Ok(pins.get(&pin).map(|s| s.level).unwrap_or(false))
    }

    fn analog_read(&self, pin: u8) -> Result<u16, String> {
        let pins = self.pins.lock().unwrap();
        Ok(pins.get(&pin).map(|s| s.analog).unwrap_or(0))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_mode_sets_direction() {
        let gpio = MockGpioDriver::new();
        gpio.pin_mode(2, Direction::Output).unwrap();
        let snap = gpio.snapshot();
        assert_eq!(snap[&2].direction, Direction::Output);
    }

    #[test]
    fn digital_write_after_pin_mode_sets_level() {
        let gpio = MockGpioDriver::new();
        gpio.pin_mode(3, Direction::Output).unwrap();
        gpio.digital_write(3, true).unwrap();
        assert_eq!(gpio.digital_read(3).unwrap(), true);
    }

    #[test]
    fn digital_write_without_pin_mode_returns_err() {
        let gpio = MockGpioDriver::new();
        // Default direction is Input — write should fail.
        let result = gpio.digital_write(7, true);
        assert!(result.is_err(), "write to input pin should fail");
    }

    #[test]
    fn analog_read_returns_seeded_value() {
        let gpio = MockGpioDriver::new();
        gpio.set_analog(4, 512);
        assert_eq!(gpio.analog_read(4).unwrap(), 512);
    }

    #[test]
    fn uninitialized_pin_reads_false_zero() {
        let gpio = MockGpioDriver::new();
        assert_eq!(gpio.digital_read(99).unwrap(), false);
        assert_eq!(gpio.analog_read(99).unwrap(), 0);
    }
}
