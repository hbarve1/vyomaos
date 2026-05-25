// T032: Failing unit tests for exclusive peripheral conflict detection.
//
// Tests verify that two modules claiming the same GPIO pin as output are
// rejected — the second module's spawn is refused with a clear error.
// The `PeripheralRegistry` struct is implemented in T037.

use supervisor::capability::peripheral::{PeripheralCapability, PeripheralRegistry};
use supervisor::hal::Direction;

// ── helpers ───────────────────────────────────────────────────────────────────

fn gpio_output(pin: u8) -> PeripheralCapability {
    PeripheralCapability::Gpio { pin, direction: Direction::Output }
}

fn gpio_input(pin: u8) -> PeripheralCapability {
    PeripheralCapability::Gpio { pin, direction: Direction::Input }
}

// ── T032-1: first module to claim a GPIO output pin succeeds ─────────────────

#[test]
fn test_first_gpio_output_claim_succeeds() {
    let mut registry = PeripheralRegistry::new();
    let result = registry.claim("motor-driver", &gpio_output(5));
    assert!(result.is_ok(), "first claim should succeed: {:?}", result);
}

// ── T032-2: second module claiming same output pin is rejected ───────────────

#[test]
fn test_second_gpio_output_claim_rejected() {
    let mut registry = PeripheralRegistry::new();
    registry.claim("motor-driver", &gpio_output(5)).expect("first claim");
    let result = registry.claim("led-controller", &gpio_output(5));
    assert!(
        result.is_err(),
        "second exclusive claim on pin 5 output should be rejected"
    );
}

// ── T032-3: error message mentions the conflicting module ────────────────────

#[test]
fn test_conflict_error_mentions_owner() {
    let mut registry = PeripheralRegistry::new();
    registry.claim("motor-driver", &gpio_output(5)).expect("first claim");
    let err = registry.claim("led-controller", &gpio_output(5)).unwrap_err();
    assert!(
        err.contains("motor-driver"),
        "conflict error should mention owning module: {err}"
    );
}

// ── T032-4: two modules can claim different pins ──────────────────────────────

#[test]
fn test_different_pins_no_conflict() {
    let mut registry = PeripheralRegistry::new();
    registry.claim("motor-a", &gpio_output(5)).expect("pin 5 claim");
    let result = registry.claim("motor-b", &gpio_output(6));
    assert!(result.is_ok(), "different pins should not conflict: {:?}", result);
}

// ── T032-5: two modules reading the same input pin is allowed ────────────────

#[test]
fn test_shared_input_pin_allowed() {
    let mut registry = PeripheralRegistry::new();
    registry.claim("sensor-a", &gpio_input(3)).expect("first input claim");
    let result = registry.claim("sensor-b", &gpio_input(3));
    assert!(
        result.is_ok(),
        "two modules reading the same input pin should be allowed: {:?}", result
    );
}

// ── T032-6: releasing a claim allows re-claim ────────────────────────────────

#[test]
fn test_release_allows_reclaim() {
    let mut registry = PeripheralRegistry::new();
    registry.claim("motor-driver", &gpio_output(5)).expect("first claim");
    registry.release("motor-driver");
    let result = registry.claim("new-driver", &gpio_output(5));
    assert!(result.is_ok(), "after release, new claim should succeed: {:?}", result);
}

// ── T032-7: UART port exclusive — second module rejected ─────────────────────

#[test]
fn test_uart_exclusive_second_rejected() {
    let mut registry = PeripheralRegistry::new();
    let uart0 = PeripheralCapability::Uart { port: 0 };
    registry.claim("lidar-sim", &uart0).expect("first uart claim");
    let result = registry.claim("gps-reader", &uart0);
    assert!(
        result.is_err(),
        "second module claiming same UART port should be rejected"
    );
}

// ── T032-8: I2C bus + address exclusive — second module rejected ─────────────

#[test]
fn test_i2c_exclusive_same_addr_rejected() {
    let mut registry = PeripheralRegistry::new();
    let sensor1 = PeripheralCapability::I2c { bus: 0, addr: 0x48 };
    registry.claim("temp-sensor", &sensor1).expect("first i2c claim");
    let result = registry.claim("another-sensor", &sensor1);
    assert!(
        result.is_err(),
        "two modules on same I2C bus+addr should conflict"
    );
}

// ── T032-9: I2C same bus but different address — allowed ─────────────────────

#[test]
fn test_i2c_same_bus_diff_addr_allowed() {
    let mut registry = PeripheralRegistry::new();
    registry.claim("temp-sensor",  &PeripheralCapability::I2c { bus: 0, addr: 0x48 })
        .expect("first i2c claim");
    let result = registry.claim("pressure-sensor", &PeripheralCapability::I2c { bus: 0, addr: 0x77 });
    assert!(
        result.is_ok(),
        "different I2C addresses on same bus should not conflict: {:?}", result
    );
}
