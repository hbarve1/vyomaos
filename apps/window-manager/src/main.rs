// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS window manager
//!
//! Arranges running app windows by Z-order using @supervisor: IPC.
//! On startup: queries the running app list, raises apps in sorted order
//! (alphabetically first = topmost), focuses the top app.
//! Re-tiles when it receives a re-tile request via stdin.

use std::io::{BufRead, Write};

fn main() {
    // Request the running app list from the supervisor
    println!("@supervisor: list");
    let _ = std::io::stdout().flush();

    let stdin = std::io::stdin();
    let mut arranged = false;

    for raw in stdin.lock().lines() {
        let raw = match raw { Ok(l) => l, Err(_) => break };

        // First REPLY is the response to @supervisor: list
        if !arranged {
            if let Some(reply) = raw.strip_prefix("REPLY:") {
                arrange(reply);
                arranged = true;
            }
            continue;
        }

        // After initial arrangement, accept "retile" to redo the layout
        match raw.trim() {
            "retile" => {
                arranged = false;
                println!("@supervisor: list");
                let _ = std::io::stdout().flush();
            }
            _ => {}
        }
    }
}

/// Issue raise/focus commands to arrange windows in a tiled order.
/// Apps are sorted alphabetically; the first becomes the topmost window.
/// "window-manager" and "shell" are excluded from layout management.
fn arrange(reply: &str) {
    const SKIP: &[&str] = &["window-manager", "shell"];

    let mut apps: Vec<&str> = reply
        .split('|')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && !SKIP.contains(s))
        .collect();

    if apps.is_empty() {
        return;
    }

    apps.sort_unstable();

    // Raise apps in reverse sorted order so apps[0] lands on top
    for name in apps.iter().rev() {
        println!("@supervisor: raise {name}");
    }

    // Focus the topmost app
    println!("@supervisor: focus {}", apps[0]);
    let _ = std::io::stdout().flush();
}
