// T013: Failing unit tests for peripheral capability enforcement.
//
// Tests cover per-pin GPIO, per-bus I2C, and denying undeclared peripherals.
// Tests are written against PeripheralEnforcer (implemented in T014).

use supervisor::capability::peripheral::{PeripheralCapability, PeripheralEnforcer};
use supervisor::hal::Direction;

// ── Helper ────────────────────────────────────────────────────────────────────

fn enforcer_with_gpio(pins: &[u8], direction: Option<&str>) -> PeripheralEnforcer {
    let dir_str = match direction {
        Some(d) => format!(r#"direction = "{}""#, d),
        None => String::new(),
    };
    let pins_str = format!(
        "[{}]",
        pins.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
    );
    let toml = format!(
        r#"
[gpio]
pins = {pins_str}
{dir_str}
"#
    );
    PeripheralEnforcer::from_toml(&toml).expect("valid gpio toml")
}

// ── T013-1: GPIO pin in allowed set is permitted ──────────────────────────────

#[test]
fn test_gpio_allowed_pin_is_permitted() {
    let enforcer = enforcer_with_gpio(&[2, 4], None);
    let req = PeripheralCapability::Gpio { pin: 2, direction: Direction::Output };
    assert!(enforcer.check(&req).is_ok(), "pin 2 should be allowed");
}

// ── T013-2: GPIO pin NOT in allowed set is denied ─────────────────────────────

#[test]
fn test_gpio_denied_pin_returns_err() {
    let enforcer = enforcer_with_gpio(&[2, 4], None);
    let req = PeripheralCapability::Gpio { pin: 7, direction: Direction::Output };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "pin 7 should be denied");
    let msg = result.unwrap_err();
    assert!(msg.contains("7"), "error should mention pin 7: {msg}");
}

// ── T013-3: GPIO direction mismatch is denied ─────────────────────────────────

#[test]
fn test_gpio_direction_mismatch_denied() {
    let enforcer = enforcer_with_gpio(&[5], Some("output"));
    let req = PeripheralCapability::Gpio { pin: 5, direction: Direction::Input };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "wrong direction should be denied");
}

// ── T013-4: GPIO direction match is permitted ─────────────────────────────────

#[test]
fn test_gpio_direction_match_permitted() {
    let enforcer = enforcer_with_gpio(&[5], Some("output"));
    let req = PeripheralCapability::Gpio { pin: 5, direction: Direction::Output };
    assert!(enforcer.check(&req).is_ok(), "matching direction should be allowed");
}

// ── T013-5: I2C access on allowed bus is permitted ───────────────────────────

#[test]
fn test_i2c_allowed_bus_is_permitted() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[i2c]
bus = 1
"#).expect("valid i2c toml");
    let req = PeripheralCapability::I2c { bus: 1, addr: 0x48 };
    assert!(enforcer.check(&req).is_ok(), "bus 1 should be allowed");
}

// ── T013-6: I2C access on wrong bus is denied ────────────────────────────────

#[test]
fn test_i2c_wrong_bus_denied() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[i2c]
bus = 1
"#).expect("valid i2c toml");
    let req = PeripheralCapability::I2c { bus: 2, addr: 0x48 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "bus 2 should be denied");
}

// ── T013-7: I2C access on specific address (declared) is permitted ───────────

#[test]
fn test_i2c_specific_address_permitted() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[i2c]
bus = 0
address = 0x48
"#).expect("valid i2c toml");
    let req = PeripheralCapability::I2c { bus: 0, addr: 0x48 };
    assert!(enforcer.check(&req).is_ok(), "address 0x48 should be allowed");
}

// ── T013-8: I2C access on wrong address is denied ────────────────────────────

#[test]
fn test_i2c_wrong_address_denied() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[i2c]
bus = 0
address = 0x48
"#).expect("valid i2c toml");
    let req = PeripheralCapability::I2c { bus: 0, addr: 0x50 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "address 0x50 should be denied when 0x48 declared");
}

// ── T013-9: Undeclared GPIO access is denied (no [gpio] section) ─────────────

#[test]
fn test_undeclared_gpio_denied() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[i2c]
bus = 0
"#).expect("valid toml (no gpio)");
    let req = PeripheralCapability::Gpio { pin: 2, direction: Direction::Output };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "GPIO should be denied when not declared");
}

// ── T013-10: Undeclared UART access is denied ────────────────────────────────

#[test]
fn test_undeclared_uart_denied() {
    let enforcer = PeripheralEnforcer::from_toml("").expect("empty caps");
    let req = PeripheralCapability::Uart { port: 0 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "UART should be denied when not declared");
}

// ── T013-11: ADC allowed channel is permitted ─────────────────────────────────

#[test]
fn test_adc_allowed_channel_permitted() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[adc]
channels = [0, 1, 2]
"#).expect("valid adc toml");
    let req = PeripheralCapability::Adc { channel: 1 };
    assert!(enforcer.check(&req).is_ok(), "channel 1 should be allowed");
}

// ── T013-12: ADC denied channel returns Err ──────────────────────────────────

#[test]
fn test_adc_denied_channel_returns_err() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[adc]
channels = [0, 1]
"#).expect("valid adc toml");
    let req = PeripheralCapability::Adc { channel: 5 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "channel 5 should be denied");
}

// ── T013-13: SPI bus allowed is permitted ────────────────────────────────────

#[test]
fn test_spi_allowed_bus_permitted() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[spi]
bus = 0
"#).expect("valid spi toml");
    let req = PeripheralCapability::Spi { bus: 0, cs: 0 };
    assert!(enforcer.check(&req).is_ok(), "SPI bus 0 should be allowed");
}

// ── T013-14: SPI wrong bus is denied ─────────────────────────────────────────

#[test]
fn test_spi_wrong_bus_denied() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[spi]
bus = 0
"#).expect("valid spi toml");
    let req = PeripheralCapability::Spi { bus: 1, cs: 0 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "SPI bus 1 should be denied when only bus 0 declared");
}
