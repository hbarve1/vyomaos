// Structured logging module — ISO timestamp + level + subsystem + optional app name.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy)]
pub enum Subsystem {
    Manifest,
    Capability,
    Lifecycle,
    Ipc,
    Display,
    Input,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Info  => "INFO ",
            Level::Warn  => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

impl Subsystem {
    fn as_str(self) -> &'static str {
        match self {
            Subsystem::Manifest   => "manifest",
            Subsystem::Capability => "capability",
            Subsystem::Lifecycle  => "lifecycle",
            Subsystem::Ipc        => "ipc",
            Subsystem::Display    => "display",
            Subsystem::Input      => "input",
        }
    }
}

/// Format a structured log line.
/// Output: `[<ISO timestamp>] [<LEVEL>] [<subsystem>] [app=<name>] <msg>`
pub fn format_log(level: Level, sub: Subsystem, app: Option<&str>, msg: &str) -> String {
    let ts = iso_timestamp();
    let app_field = match app {
        Some(name) => format!(" app={name}"),
        None       => String::new(),
    };
    format!("[{ts}] [{}] [{}]{} {msg}", level.as_str(), sub.as_str(), app_field)
}

fn iso_timestamp() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs  = dur.as_secs();
    let millis = dur.subsec_millis();

    // Manual ISO 8601 formatting without external crates
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;

    // Simplified Gregorian calendar (good for dates after 1970)
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut remaining = days;
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }
    let month_days: &[u64] = if is_leap(year) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &md in month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        month += 1;
    }
    (year, month, remaining + 1)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Log at INFO level to stderr.
#[macro_export]
macro_rules! log_info {
    ($sub:expr, $app:expr, $msg:expr) => {
        eprintln!("{}", supervisor::logging::format_log(
            supervisor::logging::Level::Info, $sub, $app, $msg
        ))
    };
}

/// Log at WARN level to stderr.
#[macro_export]
macro_rules! log_warn {
    ($sub:expr, $app:expr, $msg:expr) => {
        eprintln!("{}", supervisor::logging::format_log(
            supervisor::logging::Level::Warn, $sub, $app, $msg
        ))
    };
}

/// Log at ERROR level to stderr.
#[macro_export]
macro_rules! log_error {
    ($sub:expr, $app:expr, $msg:expr) => {
        eprintln!("{}", supervisor::logging::format_log(
            supervisor::logging::Level::Error, $sub, $app, $msg
        ))
    };
}
