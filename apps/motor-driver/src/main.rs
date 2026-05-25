// motor-driver — robotics sample app (T039)
//
// Simulates a motor controller that toggles GPIO output pins every second.
// In production the supervisor would intercept VYOMA_HAL:gpio:write: lines
// and route them to the HAL driver.  This stub prints the commands so they
// can be verified in integration tests.

use std::{thread, time::Duration};

fn main() {
    println!("motor-driver: starting (gpio pins 5, 6)");

    let mut tick: u32 = 0;
    loop {
        let level = (tick % 2) == 0;
        // HAL command format: VYOMA_HAL:gpio:write:<pin>,<level>
        println!("VYOMA_HAL:gpio:write:5,{}", if level { 1 } else { 0 });
        println!("VYOMA_HAL:gpio:write:6,{}", if level { 0 } else { 1 });
        println!("motor-driver: tick {tick} — pin5={} pin6={}", level as u8, (!level) as u8);
        tick = tick.wrapping_add(1);
        thread::sleep(Duration::from_secs(1));
    }
}
