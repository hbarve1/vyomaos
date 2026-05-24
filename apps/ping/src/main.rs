// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

/// VyomaOS IPC demo — ping side
///
/// Sends 3 messages to the pong app via the supervisor IPC broker,
/// then reads 3 replies back from stdin.
///
/// Message format understood by the supervisor: "@<app>: <text>"
fn main() {
    // Send 3 pings — supervisor routes these to pong's stdin
    for i in 1..=3 {
        println!("@pong: ping-{i}");
    }

    // Read 3 pong replies from our stdin (routed back by supervisor)
    use std::io::{self, BufRead};
    let stdin = io::stdin();
    let mut received = 0;
    for line in stdin.lock().lines() {
        match line {
            Ok(msg) => {
                eprintln!("ping: got reply: {msg}");
                received += 1;
                if received >= 3 {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    eprintln!("ping: done ({received}/3 replies received)");
}
