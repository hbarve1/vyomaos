// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

/// Maximum output lines kept in the buffer (derived from panel geometry).
pub const MAX_LINES: usize = 23;

/// Append to the output buffer, evicting the oldest line when full.
pub fn push_line(lines: &mut Vec<String>, s: String) {
    lines.push(s);
    while lines.len() > MAX_LINES {
        lines.remove(0);
    }
}

pub const COMPLETIONS: &[&str] = &[
    "ls", "ps", "kill", "log", "logf", "restart", "clear", "help", "exit",
    "status", "list", "reload", "logs", "run", "focus", "raise", "lower",
    "update", "notify", "resize", "wallpaper", "shutdown", "reboot",
    "session-save", "session-restore", "monitors", "dns", "tls-info",
    "download", "clip-set", "clip-get", "screenshot", "pkg",
];

pub const DATA_ENTRIES: &[&str] = &["shell_history", "boot_count.txt", "boot_log.txt"];

const ENV_PAIRS: &[(&str, &str)] = &[
    ("PATH", "/data/bin"),
    ("HOME", "/data"),
    ("SHELL", "vyomash"),
];

/// Split `text` into chunks of at most `cols` chars. Pure function.
pub fn wrap_line(text: &str, cols: usize) -> Vec<String> {
    if cols == 0 || text.is_empty() {
        return vec![text.to_string()];
    }
    text.chars()
        .collect::<Vec<_>>()
        .chunks(cols)
        .map(|c| c.iter().collect())
        .collect()
}

/// Returns the unique completion for `input` if exactly one command starts with it.
pub fn tab_complete(input: &str) -> Option<&'static str> {
    let matches: Vec<&str> = COMPLETIONS
        .iter()
        .copied()
        .filter(|c| c.starts_with(input))
        .collect();
    if matches.len() == 1 { Some(matches[0]) } else { None }
}

/// Return every entry whose name starts with `prefix`. Pure: no I/O, no global state.
pub fn path_completions<'a>(prefix: &str, entries: &'a [&'a str]) -> Vec<&'a str> {
    entries.iter().copied().filter(|e| e.starts_with(prefix)).collect()
}

pub fn is_clear_cmd(input: &str) -> bool {
    input.trim() == "clear"
}

/// Returns `Some(text)` when `input` is an `echo` command, `None` otherwise.
pub fn parse_echo_cmd(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed == "echo" {
        Some(String::new())
    } else if let Some(rest) = trimmed.strip_prefix("echo ") {
        Some(rest.to_string())
    } else {
        None
    }
}

pub fn is_pwd_cmd(input: &str) -> bool { input.trim() == "pwd" }

pub fn format_date_output(secs: u64) -> String { format!("Unix time: {secs}s") }

pub fn format_env_output(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n")
}

pub fn is_whoami_cmd(input: &str) -> bool { input.trim() == "whoami" }

pub fn handle_command(cmd: &str, lines: &mut Vec<String>) {
    if let Some(text) = parse_echo_cmd(cmd) {
        push_line(lines, text);
        return;
    }

    match cmd {
        "help" => {
            push_line(lines, "commands:".into());
            push_line(lines, "  help              — this text".into());
            push_line(lines, "  Ctrl+L            — clear screen".into());
            push_line(lines, "  Ctrl+A            — move cursor to line start".into());
            push_line(lines, "  Ctrl+E            — move cursor to line end".into());
            push_line(lines, "  echo [text]       — print text to output".into());
            push_line(lines, "  ps                — list all apps + status".into());
            push_line(lines, "  status            — running app count".into());
            push_line(lines, "  list              — list app names".into());
            push_line(lines, "  log <app>         — last 20 lines (memory)".into());
            push_line(lines, "  logf <app>        — last 30 lines from disk log".into());
            push_line(lines, "  logs              — list apps with log files".into());
            push_line(lines, "  kill <app>        — terminate an app".into());
            push_line(lines, "  restart <app>     — kill + relaunch an app".into());
            push_line(lines, "  run <app>         — launch an app by name".into());
            push_line(lines, "  reload            — re-read boot.toml".into());
            push_line(lines, "  pkg list          — list available packages".into());
            push_line(lines, "  pkg install <n>   — install a package".into());
            push_line(lines, "  pkg remove <n>    — remove a package".into());
            push_line(lines, "  pkg installed     — list installed packages".into());
            push_line(lines, "  focus <app>        — give keyboard focus to an app".into());
            push_line(lines, "  raise <app>        — bring window to front (Z-order)".into());
            push_line(lines, "  lower <app>        — send window to back (Z-order)".into());
            push_line(lines, "  update <app> <url> — OTA hot-swap wasm binary from url".into());
            push_line(lines, "  wallpaper <rgba>   — set desktop background color".into());
            push_line(lines, "  resize <app> <w> <h> — resize app window".into());
            push_line(lines, "  notify <title> <msg> — show toast notification".into());
            push_line(lines, "  shutdown           — power off the system".into());
            push_line(lines, "  reboot             — restart the system".into());
            push_line(lines, "  session-save       — save window layout to /data/session.toml".into());
            push_line(lines, "  session-restore    — restore window layout from /data/session.toml".into());
            push_line(lines, "  monitors           — count connected DRM displays".into());
            push_line(lines, "  dns <hostname>     — resolve hostname to IP via supervisor".into());
            push_line(lines, "  tls-info           — check TLS cert/key availability".into());
            push_line(lines, "  download <url> <dest> — download file from HTTP URL to /data path".into());
            push_line(lines, "  clip-set <text>    — copy text to supervisor clipboard".into());
            push_line(lines, "  clip-get           — paste text from supervisor clipboard".into());
            push_line(lines, "  screenshot [path]  — save framebuffer PPM to /data/screenshot.ppm".into());
            push_line(lines, "  clear             — clear shell output".into());
            push_line(lines, "  pwd               — print working directory".into());
            push_line(lines, "  date              — print current Unix timestamp".into());
            push_line(lines, "  env               — print environment variables".into());
            push_line(lines, "  whoami            — print current user name".into());
        }
        "ps" => { println!("@supervisor: ps"); }
        "status" => { println!("@supervisor: status"); }
        "list" => { println!("@supervisor: list"); }
        "reload" => {
            println!("@supervisor: reload");
            push_line(lines, "reloading boot.toml...".into());
        }
        "logs" => { println!("@supervisor: logs"); }
        other if other.starts_with("logf ") => {
            let app = other[5..].trim();
            if app.is_empty() {
                push_line(lines, "usage: logf <appname>".into());
            } else {
                println!("@supervisor: logf {app}");
            }
        }
        other if other.starts_with("log ") => {
            let app = other[4..].trim();
            if app.is_empty() {
                push_line(lines, "usage: log <appname>".into());
            } else {
                println!("@supervisor: log {app}");
            }
        }
        other if other.starts_with("kill ") => {
            let app = other[5..].trim();
            if app.is_empty() {
                push_line(lines, "usage: kill <appname>".into());
            } else {
                println!("@supervisor: kill {app}");
                push_line(lines,format!("killing {app}..."));
            }
        }
        other if other.starts_with("restart ") => {
            let app = other[8..].trim();
            if app.is_empty() {
                push_line(lines, "usage: restart <appname>".into());
            } else {
                println!("@supervisor: restart {app}");
                push_line(lines,format!("restarting {app}..."));
            }
        }
        other if other.starts_with("pkg") => {
            let sub = other.get(4..).map(|s| s.trim()).unwrap_or("");
            match sub {
                "list" => { println!("@supervisor: pkg-list"); }
                "installed" => { println!("@supervisor: pkg-installed"); }
                s if s.starts_with("install ") => {
                    let pkg = s[8..].trim();
                    if pkg.is_empty() {
                        push_line(lines, "usage: pkg install <name>".into());
                    } else {
                        println!("@supervisor: pkg-install {pkg}");
                        push_line(lines,format!("installing {pkg}..."));
                    }
                }
                s if s.starts_with("remove ") => {
                    let pkg = s[7..].trim();
                    if pkg.is_empty() {
                        push_line(lines, "usage: pkg remove <name>".into());
                    } else {
                        println!("@supervisor: pkg-remove {pkg}");
                        push_line(lines,format!("removing {pkg}..."));
                    }
                }
                _ => {
                    push_line(lines, "pkg: list | install <n> | remove <n> | installed".into());
                }
            }
        }
        other if other.starts_with("resize ") => {
            let args = other[7..].trim();
            if args.split_whitespace().count() < 3 {
                push_line(lines, "usage: resize <app> <w> <h>".into());
            } else {
                println!("@supervisor: resize {args}");
                push_line(lines,format!("resizing {args}..."));
            }
        }
        other if other.starts_with("wallpaper ") => {
            let color = other[10..].trim();
            if color.is_empty() {
                push_line(lines, "usage: wallpaper <rgba_hex>  e.g. wallpaper 0x1E1E2EFF".into());
            } else {
                println!("@supervisor: wallpaper {color}");
                push_line(lines,format!("setting wallpaper to {color}..."));
            }
        }
        other if other.starts_with("raise ") => {
            let app = other[6..].trim();
            if app.is_empty() {
                push_line(lines, "usage: raise <appname>".into());
            } else {
                println!("@supervisor: raise {app}");
                push_line(lines,format!("raising {app}..."));
            }
        }
        other if other.starts_with("lower ") => {
            let app = other[6..].trim();
            if app.is_empty() {
                push_line(lines, "usage: lower <appname>".into());
            } else {
                println!("@supervisor: lower {app}");
                push_line(lines,format!("lowering {app}..."));
            }
        }
        other if other.starts_with("update ") => {
            let args = other[7..].trim();
            if args.is_empty() {
                push_line(lines, "usage: update <app> <url>".into());
            } else {
                println!("@supervisor: update {args}");
                push_line(lines,format!("updating {args}…"));
            }
        }
        other if other.starts_with("notify ") => {
            let args = other[7..].trim();
            if args.is_empty() {
                push_line(lines, "usage: notify <title> <msg>".into());
            } else {
                println!("@supervisor: notify {args}");
                push_line(lines,format!("notification sent: {args}"));
            }
        }
        other if other.starts_with("focus ") => {
            let app = other[6..].trim();
            if app.is_empty() {
                push_line(lines, "usage: focus <appname>".into());
            } else {
                println!("@supervisor: focus {app}");
            }
        }
        other if other.starts_with("run ") => {
            let app = other[4..].trim();
            if app.is_empty() {
                push_line(lines, "usage: run <appname>".into());
            } else {
                println!("@supervisor: run /apps/{app}/vyoma.toml");
                println!("@supervisor: focus {app}");
                push_line(lines,format!("launching {app}..."));
            }
        }
        "shutdown" => {
            push_line(lines, "system shutting down...".into());
            println!("@supervisor: shutdown");
        }
        "reboot" => {
            push_line(lines, "system rebooting...".into());
            println!("@supervisor: reboot");
        }
        "session-save" => {
            println!("@supervisor: session-save");
            push_line(lines, "saving session...".into());
        }
        "session-restore" => {
            println!("@supervisor: session-restore");
            push_line(lines, "restoring session...".into());
        }
        "monitors" => { println!("@supervisor: monitors"); }
        "tls-info" => { println!("@supervisor: tls-info"); }
        other if other.starts_with("dns ") => {
            let host = other[4..].trim();
            if host.is_empty() {
                push_line(lines, "usage: dns <hostname>".into());
            } else {
                println!("@supervisor: dns-resolve {host}");
                push_line(lines,format!("resolving {host}..."));
            }
        }
        other if other.starts_with("download ") => {
            let args = other[9..].trim();
            match args.split_once(' ') {
                Some((url, dest)) if !url.is_empty() && !dest.is_empty() => {
                    println!("@supervisor: download {url} {dest}");
                    push_line(lines,format!("downloading {url} → {dest}"));
                }
                _ => push_line(lines, "usage: download <url> <dest>".into()),
            }
        }
        other if other.starts_with("clip-set ") => {
            let text = other[9..].trim();
            if text.is_empty() {
                push_line(lines, "usage: clip-set <text>".into());
            } else {
                println!("@supervisor: clipboard-set {text}");
                push_line(lines,format!("clipboard set ({} chars)", text.len()));
            }
        }
        "clip-get" => { println!("@supervisor: clipboard-get"); }
        other if other.starts_with("screenshot") => {
            let path = other[10..].trim();
            let dest = if path.is_empty() { "/data/screenshot.ppm" } else { path };
            println!("@supervisor: screenshot {dest}");
            push_line(lines,format!("saving screenshot to {dest}..."));
        }
        _ if is_pwd_cmd(cmd) => {
            push_line(lines, "/data".into());
        }
        "date" => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            push_line(lines,format_date_output(secs));
        }
        "env" => {
            let out = format_env_output(ENV_PAIRS);
            for line in out.lines() {
                push_line(lines,line.to_string());
            }
        }
        _ if is_whoami_cmd(cmd) => {
            push_line(lines, "vyoma".to_string());
        }
        other => {
            push_line(lines,format!("unknown: {other}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{wrap_line, path_completions, is_clear_cmd, parse_echo_cmd,
                is_pwd_cmd, format_date_output, format_env_output, is_whoami_cmd};

    #[test]
    fn wrap_line_short_fits_one_chunk() {
        assert_eq!(wrap_line("hello", 90), vec!["hello"]);
    }

    #[test]
    fn wrap_line_exact_boundary() {
        let s = "a".repeat(90);
        assert_eq!(wrap_line(&s, 90), vec![s]);
    }

    #[test]
    fn wrap_line_splits_long_line() {
        let s = "a".repeat(91);
        let result = wrap_line(&s, 90);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), 90);
        assert_eq!(result[1].len(), 1);
    }

    #[test]
    fn wrap_line_empty_returns_one() {
        assert_eq!(wrap_line("", 90), vec![""]);
    }

    #[test]
    fn wrap_line_zero_cols_no_panic() {
        let result = wrap_line("hello", 0);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn path_completions_exact_match() {
        let entries = &["shell_history", "boot_count.txt", "boot_log.txt"];
        assert_eq!(path_completions("shell", entries), vec!["shell_history"]);
    }

    #[test]
    fn path_completions_prefix_match_multiple() {
        let entries = &["boot_count.txt", "boot_log.txt", "shell_history"];
        assert_eq!(path_completions("boot", entries).len(), 2);
    }

    #[test]
    fn path_completions_no_match() {
        let entries = &["shell_history", "boot_count.txt"];
        assert!(path_completions("xyz", entries).is_empty());
    }

    #[test]
    fn path_completions_empty_prefix_returns_all() {
        let entries = &["a", "b", "c"];
        assert_eq!(path_completions("", entries).len(), 3);
    }

    #[test]
    fn is_clear_cmd_basic() { assert!(is_clear_cmd("clear")); }

    #[test]
    fn is_clear_cmd_with_space() { assert!(is_clear_cmd("  clear  ")); }

    #[test]
    fn is_clear_cmd_not_clear() { assert!(!is_clear_cmd("cls")); }

    #[test]
    fn is_clear_cmd_empty() { assert!(!is_clear_cmd("")); }

    #[test]
    fn is_clear_cmd_partial() { assert!(!is_clear_cmd("clear all")); }

    #[test]
    fn parse_echo_cmd_basic() {
        assert_eq!(parse_echo_cmd("echo hello"), Some("hello".to_string()));
    }

    #[test]
    fn parse_echo_cmd_no_args() {
        assert_eq!(parse_echo_cmd("echo"), Some(String::new()));
    }

    #[test]
    fn parse_echo_cmd_not_echo() {
        assert_eq!(parse_echo_cmd("ls"), None);
    }

    #[test]
    fn parse_echo_cmd_empty() {
        assert_eq!(parse_echo_cmd(""), None);
    }

    #[test]
    fn parse_echo_cmd_with_spaces() {
        assert_eq!(
            parse_echo_cmd("echo  hello world"),
            Some(" hello world".to_string())
        );
    }

    #[test]
    fn is_pwd_cmd_basic() { assert!(is_pwd_cmd("pwd")); }
    #[test]
    fn is_pwd_cmd_spaces() { assert!(is_pwd_cmd("  pwd  ")); }
    #[test]
    fn is_pwd_cmd_not_pwd() { assert!(!is_pwd_cmd("ls")); }
    #[test]
    fn is_pwd_cmd_empty() { assert!(!is_pwd_cmd("")); }
    #[test]
    fn is_pwd_cmd_partial() { assert!(!is_pwd_cmd("pwd /")); }

    #[test]
    fn format_date_output_zero() { assert_eq!(format_date_output(0), "Unix time: 0s"); }
    #[test]
    fn format_date_output_nonzero() { assert_eq!(format_date_output(1000), "Unix time: 1000s"); }
    #[test]
    fn format_date_output_prefix() { assert!(format_date_output(42).starts_with("Unix time:")); }

    #[test]
    fn format_env_output_empty() { assert_eq!(format_env_output(&[]), ""); }
    #[test]
    fn format_env_output_one() { assert_eq!(format_env_output(&[("K", "V")]), "K=V"); }
    #[test]
    fn format_env_output_multiple() {
        assert_eq!(format_env_output(&[("A", "1"), ("B", "2")]), "A=1\nB=2");
    }
    #[test]
    fn format_env_no_trailing_newline() {
        assert!(!format_env_output(&[("X", "y")]).ends_with('\n'));
    }

    #[test]
    fn is_whoami_cmd_basic() { assert!(is_whoami_cmd("whoami")); }
    #[test]
    fn is_whoami_cmd_spaces() { assert!(is_whoami_cmd("  whoami  ")); }
    #[test]
    fn is_whoami_cmd_not_who() { assert!(!is_whoami_cmd("who")); }
    #[test]
    fn is_whoami_cmd_empty() { assert!(!is_whoami_cmd("")); }
}
