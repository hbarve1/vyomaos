// path-planner — robotics sample app (T039)
//
// Emits a sequence of (x, y) waypoints every second using stdio only.
// No hardware capabilities needed — pure computation.

use std::{thread, time::Duration};

const WAYPOINTS: &[(i32, i32)] = &[
    (0, 0), (10, 0), (10, 10), (0, 10), (0, 0),
];

fn main() {
    println!("path-planner: starting ({} waypoints)", WAYPOINTS.len());

    let mut idx: usize = 0;
    loop {
        let (x, y) = WAYPOINTS[idx % WAYPOINTS.len()];
        println!("path-planner: waypoint[{}] → ({x}, {y})", idx % WAYPOINTS.len());
        idx = idx.wrapping_add(1);
        thread::sleep(Duration::from_secs(1));
    }
}
