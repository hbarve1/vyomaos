// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::fs::OpenOptions;
use std::io::Write;

pub const HISTORY_PATH: &str = "/data/shell_history";
const HISTORY_CAP: usize = 100;

/// Load up to HISTORY_CAP lines from /data/shell_history.
/// Returns an empty Vec on any I/O error (silent failure).
pub fn load_history() -> Vec<String> {
    let content = match std::fs::read_to_string(HISTORY_PATH) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let all: Vec<String> = content
        .lines()
        .map(|l| l.to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if all.len() > HISTORY_CAP {
        all[all.len() - HISTORY_CAP..].to_vec()
    } else {
        all
    }
}

/// Append a single command to /data/shell_history.
/// Silently ignores any I/O error.
pub fn append_history(cmd: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(HISTORY_PATH)
    {
        let _ = writeln!(f, "{cmd}");
    }
}
