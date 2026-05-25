// lidar-sim — robotics sample app (T039)
//
// Simulates a LIDAR sensor that reads distance measurements via UART.
// Prints mock distance data in NMEA-style format once per second.

use std::{thread, time::Duration};

fn main() {
    println!("lidar-sim: starting (UART port 0, 115200 baud)");

    // Announce UART configuration to the supervisor.
    println!("VYOMA_HAL:uart:configure:0,115200,8,none,1");

    let mut tick: u32 = 0;
    loop {
        // Mock distance readings — triangular wave between 50–230 cm.
        // angle_deg rotates 10° per tick; dist uses a triangle wave.
        let angle_deg = (tick * 10) % 360;
        let dist_cm: u32 = if angle_deg <= 180 {
            50 + angle_deg as u32
        } else {
            50 + (360 - angle_deg) as u32
        };

        // LIDAR frame format: $LIDAR,<angle>,<distance_cm>*<checksum>
        let frame = format!("$LIDAR,{angle_deg:03},{dist_cm:04}");
        let checksum: u8 = frame.bytes().fold(0u8, |acc, b| acc ^ b);
        println!("VYOMA_HAL:uart:write:0,{frame}*{checksum:02X}");
        println!("lidar-sim: tick {tick} — {angle_deg}° → {dist_cm} cm");

        tick = tick.wrapping_add(1);
        thread::sleep(Duration::from_secs(1));
    }
}
