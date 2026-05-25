// T022: Failing tests for peripheral capability denial.
//
// An app without a GPIO capability declaration cannot call GPIO host functions.
// The PeripheralEnforcer.check() method enforces this at the supervisor level.

use supervisor::capability::peripheral::{PeripheralCapability, PeripheralEnforcer};
use supervisor::hal::Direction;

// ── T022-1: app with no GPIO declaration is denied GPIO access ────────────────

#[test]
fn test_no_gpio_declaration_denies_gpio_access() {
    // No [gpio] section — completely empty capabilities.
    let enforcer = PeripheralEnforcer::from_toml("").expect("empty caps parses ok");

    let req = PeripheralCapability::Gpio { pin: 4, direction: Direction::Output };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "GPIO must be denied without declaration");

    let msg = result.unwrap_err();
    assert!(
        msg.to_lowercase().contains("denied") || msg.to_lowercase().contains("not declared"),
        "error message must indicate denial: {msg}"
    );
}

// ── T022-2: app with no I2C declaration is denied I2C access ─────────────────

#[test]
fn test_no_i2c_declaration_denies_i2c_access() {
    let enforcer = PeripheralEnforcer::from_toml("").expect("empty caps parses ok");

    let req = PeripheralCapability::I2c { bus: 0, addr: 0x40 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "I2C must be denied without declaration");
}

// ── T022-3: app with no SPI declaration is denied SPI access ─────────────────

#[test]
fn test_no_spi_declaration_denies_spi_access() {
    let enforcer = PeripheralEnforcer::from_toml("").expect("empty caps parses ok");

    let req = PeripheralCapability::Spi { bus: 0, cs: 0 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "SPI must be denied without declaration");
}

// ── T022-4: app with no UART declaration is denied UART access ───────────────

#[test]
fn test_no_uart_declaration_denies_uart_access() {
    let enforcer = PeripheralEnforcer::from_toml("").expect("empty caps parses ok");

    let req = PeripheralCapability::Uart { port: 0 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "UART must be denied without declaration");
}

// ── T022-5: app with no ADC declaration is denied ADC access ─────────────────

#[test]
fn test_no_adc_declaration_denies_adc_access() {
    let enforcer = PeripheralEnforcer::from_toml("").expect("empty caps parses ok");

    let req = PeripheralCapability::Adc { channel: 2 };
    let result = enforcer.check(&req);
    assert!(result.is_err(), "ADC must be denied without declaration");
}

// ── T022-6: app declaring only GPIO cannot access I2C ────────────────────────

#[test]
fn test_gpio_only_declaration_denies_i2c() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[gpio]
pins = [4, 17]
"#).expect("gpio-only toml parses ok");

    let gpio_req = PeripheralCapability::Gpio { pin: 4, direction: Direction::Output };
    assert!(enforcer.check(&gpio_req).is_ok(), "GPIO pin 4 should be allowed");

    let i2c_req = PeripheralCapability::I2c { bus: 1, addr: 0x48 };
    assert!(enforcer.check(&i2c_req).is_err(), "I2C must be denied when only GPIO declared");
}

// ── T022-7: app declaring GPIO with specific pins denies unlisted pin ─────────

#[test]
fn test_gpio_specific_pins_denies_undeclared_pin() {
    let enforcer = PeripheralEnforcer::from_toml(r#"
[gpio]
pins = [4, 17]
"#).expect("parses ok");

    let bad_pin = PeripheralCapability::Gpio { pin: 27, direction: Direction::Input };
    let result = enforcer.check(&bad_pin);
    assert!(result.is_err(), "pin 27 not in [4, 17] must be denied");
    assert!(
        result.unwrap_err().contains("27"),
        "error message must mention pin 27"
    );
}

// ── T022-8: error message includes the peripheral type ───────────────────────

#[test]
fn test_denial_error_mentions_peripheral_type() {
    let enforcer = PeripheralEnforcer::from_toml("").expect("empty caps");

    // GPIO denial should mention GPIO
    let gpio_err = enforcer
        .check(&PeripheralCapability::Gpio { pin: 1, direction: Direction::Input })
        .unwrap_err();
    assert!(
        gpio_err.to_uppercase().contains("GPIO"),
        "GPIO denial must mention GPIO: {gpio_err}"
    );

    // I2C denial should mention I2C
    let i2c_err = enforcer
        .check(&PeripheralCapability::I2c { bus: 0, addr: 0x20 })
        .unwrap_err();
    assert!(
        i2c_err.to_uppercase().contains("I2C"),
        "I2C denial must mention I2C: {i2c_err}"
    );
}
