// HAL UART mock driver — T035
//
// Implements `UartDriver` with per-port in-memory byte queues.
// Supports seeding data for reads and inspecting writes without real hardware.

use std::{
    collections::HashMap,
    sync::Mutex,
};

use super::{Parity, UartDriver};

// ── Port configuration ────────────────────────────────────────────────────────

/// Configuration applied to a UART port.
#[derive(Debug, Clone)]
pub struct PortConfig {
    pub baud: u32,
    pub bits: u8,
    pub parity: Parity,
    pub stop: u8,
}

impl Default for PortConfig {
    fn default() -> Self {
        Self { baud: 9600, bits: 8, parity: Parity::None, stop: 1 }
    }
}

// ── Port state ────────────────────────────────────────────────────────────────

/// Per-port in-memory state.
#[derive(Debug, Default)]
struct PortState {
    config: Option<PortConfig>,
    /// Bytes written to this port.
    tx_buf: Vec<u8>,
    /// Bytes pre-seeded to be returned by `read`.
    rx_buf: Vec<u8>,
}

// ── MockUartDriver ────────────────────────────────────────────────────────────

/// Thread-safe UART mock.
pub struct MockUartDriver {
    ports: Mutex<HashMap<u8, PortState>>,
}

impl MockUartDriver {
    pub fn new() -> Self {
        Self { ports: Mutex::new(HashMap::new()) }
    }

    /// Seed bytes that will be returned by subsequent `read` calls on `port`.
    pub fn seed_rx(&self, port: u8, data: Vec<u8>) {
        let mut ports = self.ports.lock().unwrap();
        ports.entry(port).or_default().rx_buf = data;
    }

    /// Return all bytes written to `port` so far.
    pub fn tx_bytes(&self, port: u8) -> Vec<u8> {
        let ports = self.ports.lock().unwrap();
        ports.get(&port).map(|p| p.tx_buf.clone()).unwrap_or_default()
    }

    /// Return the active port configuration, if `configure` was called.
    pub fn port_config(&self, port: u8) -> Option<PortConfig> {
        let ports = self.ports.lock().unwrap();
        ports.get(&port).and_then(|p| p.config.clone())
    }
}

impl Default for MockUartDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl UartDriver for MockUartDriver {
    fn configure(
        &self,
        port: u8,
        baud: u32,
        bits: u8,
        parity: Parity,
        stop: u8,
    ) -> Result<(), String> {
        let mut ports = self.ports.lock().unwrap();
        ports.entry(port).or_default().config =
            Some(PortConfig { baud, bits, parity, stop });
        Ok(())
    }

    fn write(&self, port: u8, data: &[u8]) -> Result<usize, String> {
        let mut ports = self.ports.lock().unwrap();
        let state = ports.entry(port).or_default();
        state.tx_buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn read(&self, port: u8, buf: &mut [u8], _timeout_ms: u32) -> Result<usize, String> {
        let mut ports = self.ports.lock().unwrap();
        let state = ports.entry(port).or_default();
        let n = buf.len().min(state.rx_buf.len());
        buf[..n].copy_from_slice(&state.rx_buf[..n]);
        state.rx_buf.drain(..n);
        Ok(n)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_stores_baud_rate() {
        let uart = MockUartDriver::new();
        uart.configure(0, 115200, 8, Parity::None, 1).unwrap();
        let cfg = uart.port_config(0).unwrap();
        assert_eq!(cfg.baud, 115200);
    }

    #[test]
    fn write_stores_bytes() {
        let uart = MockUartDriver::new();
        uart.write(0, b"hello").unwrap();
        assert_eq!(uart.tx_bytes(0), b"hello");
    }

    #[test]
    fn write_returns_byte_count() {
        let uart = MockUartDriver::new();
        let n = uart.write(1, &[0x01, 0x02, 0x03]).unwrap();
        assert_eq!(n, 3);
    }

    #[test]
    fn read_returns_seeded_data() {
        let uart = MockUartDriver::new();
        uart.seed_rx(0, vec![0xAA, 0xBB, 0xCC]);
        let mut buf = [0u8; 3];
        let n = uart.read(0, &mut buf, 100).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn read_partial_consumes_and_leaves_remainder() {
        let uart = MockUartDriver::new();
        uart.seed_rx(0, vec![0x01, 0x02, 0x03, 0x04]);
        let mut buf = [0u8; 2];
        uart.read(0, &mut buf, 10).unwrap();
        // Remaining 2 bytes should still be available.
        let mut buf2 = [0u8; 4];
        let n = uart.read(0, &mut buf2, 10).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&buf2[..2], &[0x03, 0x04]);
    }

    #[test]
    fn read_empty_port_returns_zero() {
        let uart = MockUartDriver::new();
        let mut buf = [0u8; 8];
        let n = uart.read(5, &mut buf, 50).unwrap();
        assert_eq!(n, 0);
    }
}
