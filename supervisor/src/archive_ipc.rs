// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P55: Archive IPC command handler.
//!
//! Wires `@supervisor: extract` and `@supervisor: archive-list` commands
//! to the pure archive logic in `supervisor::archive`.

use crate::{log_info, log_warn, send_reply, Inbox};
use supervisor::archive::{extract_tar_gz, list_archive};
use supervisor::logging::Subsystem;

/// Handle `@supervisor: extract <path> <dest>` and `@supervisor: archive-list <path>`.
///
/// Returns `true` if the command was handled.
pub fn handle_archive_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "extract" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (archive_path, dest_dir) = match rest.split_once(' ') {
                Some((a, d)) => (a.trim(), d.trim()),
                None => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: extract <archive_path> <dest_dir>",
                        inbox,
                    );
                    return true;
                }
            };
            if archive_path.is_empty() || dest_dir.is_empty() {
                send_reply(
                    sender,
                    "REPLY:error: usage: extract <archive_path> <dest_dir>",
                    inbox,
                );
                return true;
            }
            match extract_tar_gz(archive_path, dest_dir) {
                Ok(files) => {
                    log_info!(
                        Subsystem::Lifecycle, None,
                        "extracted {} files from {} to {}",
                        files.len(), archive_path, dest_dir
                    );
                    let summary = if files.len() <= 20 {
                        files.join("|")
                    } else {
                        let first: Vec<&str> =
                            files.iter().take(20).map(|s| s.as_str()).collect();
                        format!("{}|... ({} total)", first.join("|"), files.len())
                    };
                    send_reply(
                        sender,
                        &format!("REPLY:extracted {} files: {summary}", files.len()),
                        inbox,
                    );
                }
                Err(e) => {
                    log_warn!(Subsystem::Lifecycle, None, "extract failed: {e}");
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
            true
        }

        "archive-list" => {
            let path = match parts.get(1).map(|s| s.trim()) {
                Some(p) if !p.is_empty() => p,
                _ => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: archive-list <path>",
                        inbox,
                    );
                    return true;
                }
            };
            match list_archive(path) {
                Ok(entries) => {
                    if entries.is_empty() {
                        send_reply(sender, "REPLY:archive is empty", inbox);
                    } else {
                        let joined = entries.join("|");
                        send_reply(
                            sender,
                            &format!("REPLY:archive-list {joined}"),
                            inbox,
                        );
                    }
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
            true
        }

        _ => false,
    }
}
