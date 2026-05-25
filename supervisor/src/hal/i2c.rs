// HAL I2C mock driver — T034
//
// Implements `I2cDriver` with an in-memory register map keyed by (bus, addr).
// Supports seeding read data for use in tests without real hardware.

use std::{
    collections::HashMap,
    sync::Mutex,
};

use super::I2cDriver;

// ── Device key ────────────────────────────────────────────────────────────────

/// Identifies a unique device on a specific I2C bus.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceKey {
    pub bus: u8,
    pub addr: u8,
}

// ── Device state ──────────────────────────────────────────────────────────────

/// Per-device in-memory state.
#[derive(Debug, Default, Clone)]
pub struct DeviceState {
    /// Bytes written to this device (appended on each write).
    pub written: Vec<u8>,
    /// Data to return on the next read (rotated: front element returned first).
    pub read_data: Vec<u8>,
}

// ── MockI2cDriver ─────────────────────────────────────────────────────────────

/// Thread-safe I2C mock.
pub struct MockI2cDriver {
    devices: Mutex<HashMap<DeviceKey, DeviceState>>,
}

impl MockI2cDriver {
    pub fn new() -> Self {
        Self { devices: Mutex::new(HashMap::new()) }
    }

    /// Seed bytes that will be returned by subsequent `read` / `write_read` calls
    /// to `(bus, addr)`.  Each call to `read` consumes from the front of the queue.
    pub fn seed_read(&self, bus: u8, addr: u8, data: Vec<u8>) {
        let mut devices = self.devices.lock().unwrap();
        devices.entry(DeviceKey { bus, addr }).or_default().read_data = data;
    }

    /// Return the bytes most recently written to `(bus, addr)`.
    pub fn written_to(&self, bus: u8, addr: u8) -> Vec<u8> {
        let devices = self.devices.lock().unwrap();
        devices
            .get(&DeviceKey { bus, addr })
            .map(|d| d.written.clone())
            .unwrap_or_default()
    }
}

impl Default for MockI2cDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cDriver for MockI2cDriver {
    fn write(&self, bus: u8, addr: u8, data: &[u8]) -> Result<(), String> {
        let mut devices = self.devices.lock().unwrap();
        devices
            .entry(DeviceKey { bus, addr })
            .or_default()
            .written
            .extend_from_slice(data);
        Ok(())
    }

    fn read(&self, bus: u8, addr: u8, len: usize) -> Result<Vec<u8>, String> {
        let mut devices = self.devices.lock().unwrap();
        let state = devices.entry(DeviceKey { bus, addr }).or_default();
        let available = state.read_data.len();
        if available < len {
            return Err(format!(
                "I2C mock read: requested {len} bytes from bus {bus} addr 0x{addr:02X} \
                 but only {available} seeded"
            ));
        }
        let chunk: Vec<u8> = state.read_data.drain(..len).collect();
        Ok(chunk)
    }

    fn write_read(
        &self,
        bus: u8,
        addr: u8,
        write: &[u8],
        read_len: usize,
    ) -> Result<Vec<u8>, String> {
        // Write phase.
        self.write(bus, addr, write)?;
        // Read phase (uses seeded data).
        self.read(bus, addr, read_len)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_stores_bytes() {
        let i2c = MockI2cDriver::new();
        i2c.write(0, 0x48, &[0x01, 0x02]).unwrap();
        assert_eq!(i2c.written_to(0, 0x48), vec![0x01, 0x02]);
    }

    #[test]
    fn read_returns_seeded_bytes() {
        let i2c = MockI2cDriver::new();
        i2c.seed_read(1, 0x48, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let data = i2c.read(1, 0x48, 2).unwrap();
        assert_eq!(data, vec![0xDE, 0xAD]);
    }

    #[test]
    fn read_insufficient_data_returns_err() {
        let i2c = MockI2cDriver::new();
        i2c.seed_read(0, 0x50, vec![0xFF]);
        let result = i2c.read(0, 0x50, 4);
        assert!(result.is_err(), "should fail when not enough data seeded");
    }

    #[test]
    fn write_read_combines_operations() {
        let i2c = MockI2cDriver::new();
        i2c.seed_read(0, 0x48, vec![0x12, 0x34]);
        let result = i2c.write_read(0, 0x48, &[0x00], 2).unwrap();
        assert_eq!(result, vec![0x12, 0x34]);
        // Write phase stored register byte.
        assert_eq!(i2c.written_to(0, 0x48), vec![0x00]);
    }

    #[test]
    fn different_devices_independent() {
        let i2c = MockI2cDriver::new();
        i2c.write(0, 0x10, &[0xAA]).unwrap();
        i2c.write(0, 0x20, &[0xBB]).unwrap();
        assert_eq!(i2c.written_to(0, 0x10), vec![0xAA]);
        assert_eq!(i2c.written_to(0, 0x20), vec![0xBB]);
    }
}
