// sensor-reader — robotics sample app (T039)
//
// Simulates an I2C temperature sensor reader (TMP102 at 0x48 on bus 0).
// Prints mock register reads in a format the supervisor can intercept.

use std::{thread, time::Duration};

fn main() {
    println!("sensor-reader: starting (I2C bus 0, addr 0x48)");

    let mut tick: u32 = 0;
    loop {
        // Request 2 bytes from the temperature register (0x00).
        // VYOMA_HAL:i2c:write_read:<bus>,<addr>,<reg>,<read_len>
        println!("VYOMA_HAL:i2c:write_read:0,0x48,0x00,2");

        // Simulate a raw 12-bit reading (mock values cycle between 20–25 °C).
        let raw: u16 = 0x1500 + (tick % 5) as u16 * 0x80;
        let celsius = raw as f32 / 256.0;
        println!("sensor-reader: tick {tick} — temp={celsius:.2}°C (raw=0x{raw:04X})");

        tick = tick.wrapping_add(1);
        thread::sleep(Duration::from_secs(1));
    }
}
