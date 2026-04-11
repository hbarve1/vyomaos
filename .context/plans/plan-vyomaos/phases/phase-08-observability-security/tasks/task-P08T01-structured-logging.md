# P08T01 — structured-logging

## Phase

Phase 08 — Observability & Security

## Goal

Add a `log!` macro to the supervisor that writes JSON-Lines records to `stderr` in the format `{"ts": <unix_ms>, "level": "INFO"|"WARN"|"ERROR", "app": <name_or_null>, "msg": <string>}`, and replace every existing `eprintln!` call in `supervisor/src/main.rs` with the new macro.

## File to create / modify

```
supervisor/src/log.rs    (new)
supervisor/src/main.rs   (modify — add mod log; replace all eprintln! calls)
```

## Implementation

### `supervisor/src/log.rs`

```rust
//! log.rs — structured JSON-Lines logging for the VyomaOS supervisor.
//!
//! All log output goes to stderr as one JSON object per line (JSON-Lines format).
//! Each line is self-contained and can be piped directly into `jq` or a log
//! aggregator without any framing.
//!
//! Format:
//!   {"ts":1712600000123,"level":"INFO","app":null,"msg":"supervisor starting"}
//!   {"ts":1712600000456,"level":"WARN","app":"server","msg":"exited with code 1"}

use std::time::{SystemTime, UNIX_EPOCH};

/// Valid log severity levels.
#[derive(Debug, Clone, Copy)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Info  => "INFO",
            Level::Warn  => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// Return the current Unix timestamp in milliseconds.
/// Falls back to 0 on platforms where SystemTime is unavailable.
pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Escape a string for JSON: replace `\` → `\\`, `"` → `\"`,
/// and control characters → `\uXXXX`.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"'  => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Write one JSON-Lines log record to stderr.
///
/// Do NOT call this directly — use the `log!` / `log_info!` / `log_warn!` /
/// `log_error!` macros instead.
pub fn write_log(level: Level, app: Option<&str>, msg: &str) {
    let ts       = now_ms();
    let level_s  = level.as_str();
    let msg_esc  = json_escape(msg);

    let line = match app {
        Some(a) => {
            let app_esc = json_escape(a);
            format!(
                "{{\"ts\":{},\"level\":\"{}\",\"app\":\"{}\",\"msg\":\"{}\"}}",
                ts, level_s, app_esc, msg_esc
            )
        }
        None => {
            format!(
                "{{\"ts\":{},\"level\":\"{}\",\"app\":null,\"msg\":\"{}\"}}",
                ts, level_s, msg_esc
            )
        }
    };

    // Write atomically to stderr; ignore write errors (can't log a log failure)
    eprintln!("{}", line);
}

// ── Public macros ──────────────────────────────────────────────────────────

/// Log at INFO level with no app context.
/// Usage: `log_info!("supervisor starting")`
#[macro_export]
macro_rules! log_info {
    ($msg:expr) => {
        $crate::log::write_log($crate::log::Level::Info, None, &format!($msg))
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::log::write_log($crate::log::Level::Info, None, &format!($fmt, $($arg)*))
    };
}

/// Log at WARN level with no app context.
#[macro_export]
macro_rules! log_warn {
    ($msg:expr) => {
        $crate::log::write_log($crate::log::Level::Warn, None, &format!($msg))
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::log::write_log($crate::log::Level::Warn, None, &format!($fmt, $($arg)*))
    };
}

/// Log at ERROR level with no app context.
#[macro_export]
macro_rules! log_error {
    ($msg:expr) => {
        $crate::log::write_log($crate::log::Level::Error, None, &format!($msg))
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::log::write_log($crate::log::Level::Error, None, &format!($fmt, $($arg)*))
    };
}

/// Log at INFO level with an app name.
/// Usage: `log_app_info!("hello-world", "exited normally")`
#[macro_export]
macro_rules! log_app_info {
    ($app:expr, $msg:expr) => {
        $crate::log::write_log($crate::log::Level::Info, Some($app), &format!($msg))
    };
    ($app:expr, $fmt:expr, $($arg:tt)*) => {
        $crate::log::write_log($crate::log::Level::Info, Some($app), &format!($fmt, $($arg)*))
    };
}

/// Log at WARN level with an app name.
#[macro_export]
macro_rules! log_app_warn {
    ($app:expr, $msg:expr) => {
        $crate::log::write_log($crate::log::Level::Warn, Some($app), &format!($msg))
    };
    ($app:expr, $fmt:expr, $($arg:tt)*) => {
        $crate::log::write_log($crate::log::Level::Warn, Some($app), &format!($fmt, $($arg)*))
    };
}

/// Log at ERROR level with an app name.
#[macro_export]
macro_rules! log_app_error {
    ($app:expr, $msg:expr) => {
        $crate::log::write_log($crate::log::Level::Error, Some($app), &format!($msg))
    };
    ($app:expr, $fmt:expr, $($arg:tt)*) => {
        $crate::log::write_log($crate::log::Level::Error, Some($app), &format!($fmt, $($arg)*))
    };
}
```

---

### `supervisor/src/main.rs` — migration of `eprintln!` calls

Add `mod log;` at the top of the file, then replace every `eprintln!("[supervisor] ...")` with the appropriate macro. Key substitutions:

| Old `eprintln!` pattern                          | New macro                             |
|--------------------------------------------------|---------------------------------------|
| `eprintln!("[supervisor] FATAL: ...")`           | `log_error!("...")`                   |
| `eprintln!("[supervisor] WARN: ...")`            | `log_warn!("...")`                    |
| `eprintln!("[supervisor] ...")`                  | `log_info!("...")`                    |
| `eprintln!("[supervisor] {} finished: ...", name)` | `log_app_info!(name, "exited exit={}", code)` |
| `eprintln!("[supervisor] ERROR: ... {}", name)` | `log_app_error!(name, "...")`         |

Example of what a migrated startup sequence looks like:

```rust
mod log;

fn main() {
    log_info!("VyomaOS supervisor starting");
    mount_pseudo_filesystems();

    let boot_raw = fs::read_to_string(BOOT_CONFIG_PATH).unwrap_or_else(|e| {
        log_error!("cannot read {}: {}", BOOT_CONFIG_PATH, e);
        std::process::exit(1);
    });

    let boot: BootConfig = toml::from_str(&boot_raw).unwrap_or_else(|e| {
        log_error!("malformed boot.toml: {}", e);
        std::process::exit(1);
    });

    log_info!("scheduling {} app(s) concurrently", boot.apps.len());

    // ... (thread spawn loop)

    // In each thread, after run_app returns:
    log_app_info!(&name, "finished: exit={} elapsed={}ms", exit_code, elapsed_ms);

    // In run_app on exec error:
    log_app_error!(&manifest.app.name, "exec failed: {}", e);

    // Completion summary:
    log_info!("--- completion summary ---");
    for r in final_results.iter() {
        log_app_info!(&r.name, "exit={} elapsed={}ms", r.exit_code, r.elapsed_ms);
    }
    log_info!("all apps completed, halting");
}
```

---

### Sample JSON output

```
{"ts":1712600001000,"level":"INFO","app":null,"msg":"VyomaOS supervisor starting"}
{"ts":1712600001005,"level":"INFO","app":null,"msg":"scheduling 3 app(s) concurrently"}
{"ts":1712600001100,"level":"INFO","app":"hello-world","msg":"finished: exit=0 elapsed=95ms"}
{"ts":1712600001200,"level":"INFO","app":"calculator","msg":"finished: exit=0 elapsed=195ms"}
{"ts":1712600001300,"level":"INFO","app":"factorial","msg":"finished: exit=0 elapsed=295ms"}
{"ts":1712600001301,"level":"INFO","app":null,"msg":"all apps completed, halting"}
```

## Notes

- JSON-Lines (one JSON object per newline) is chosen over multi-line JSON because it is trivially parseable by `jq -c '.'`, `grep`, and structured log aggregators (Loki, Fluentd, CloudWatch).
- Writing to `stderr` (not `stdout`) preserves `stdout` for WASM app output piped through `--inherit-stdio`. Operators can redirect stderr separately: `./vyomaos.sh run 2>supervisor.log`.
- The `json_escape` function handles the most common unsafe characters. It does not validate UTF-8 (Rust strings are always valid UTF-8, so this is safe).
- No external crate (e.g. `serde_json`, `log`, `tracing`) is used to keep the dependency count low. The serialisation is intentionally simple. Add `serde_json` in a later phase if the log format becomes more complex.
- The `ts` field is Unix time in milliseconds — sufficient resolution for event ordering without requiring nanosecond precision.
- The `app` field is `null` for supervisor-level events and a quoted string for app-scoped events. Consumers can filter with `jq 'select(.app != null)'`.
- Cross-reference: P08T02 and P08T03 add `seccomp` and namespace isolation; their log messages should also use these macros.

## Verification

```sh
# 1. Confirm log.rs was created
test -f supervisor/src/log.rs && echo "log.rs: present"

# 2. Confirm the module is declared in main.rs
grep -q "^mod log;" supervisor/src/main.rs && echo "mod log: declared"

# 3. Confirm no bare eprintln! with [supervisor] prefix remain
! grep -q 'eprintln!.*\[supervisor\]' supervisor/src/main.rs \
  && echo "no bare eprintln![supervisor]: OK" \
  || echo "WARN: bare eprintln! calls remain"

# 4. Confirm macro definitions are present
grep -q "macro_rules! log_info"      supervisor/src/log.rs && echo "log_info! macro: present"
grep -q "macro_rules! log_warn"      supervisor/src/log.rs && echo "log_warn! macro: present"
grep -q "macro_rules! log_error"     supervisor/src/log.rs && echo "log_error! macro: present"
grep -q "macro_rules! log_app_info"  supervisor/src/log.rs && echo "log_app_info! macro: present"

# 5. Build the supervisor
(cd supervisor && cargo build 2>&1 | tail -5)
test -x supervisor/target/debug/supervisor && echo "supervisor: built OK"

# 6. Run supervisor (with dummy boot.toml) and confirm JSON-Lines output on stderr
echo '[[apps]]
manifest = "/apps/hello-world/vyoma.toml"
restart = "never"' > /tmp/test_boot.toml

# Patch: if supervisor honours BOOT_CONFIG_PATH env, override it; otherwise run directly
supervisor/target/debug/supervisor 2>/tmp/sup_stderr.log || true
head -5 /tmp/sup_stderr.log

# 7. Validate output lines are valid JSON
python3 -c "
import json, sys
lines = open('/tmp/sup_stderr.log').readlines()
for i, line in enumerate(lines, 1):
    line = line.strip()
    if not line: continue
    obj = json.loads(line)  # raises on invalid JSON
    assert 'ts' in obj and 'level' in obj and 'msg' in obj, f'missing keys in line {i}'
    assert obj['level'] in ('INFO','WARN','ERROR'), f'invalid level in line {i}'
    print(f'line {i}: OK ({obj[\"level\"]})')
print('All log lines are valid JSON-Lines.')
"

# 8. Confirm json_escape handles special characters
cat > /tmp/test_escape.rs << 'EOF'
fn json_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"'  => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
fn main() {
    assert_eq!(json_escape("hello"),        "hello");
    assert_eq!(json_escape("say \"hi\""),   r#"say \"hi\""#);
    assert_eq!(json_escape("line\nnewline"), r"line\nnewline");
    assert_eq!(json_escape("tab\there"),    r"tab\there");
    println!("json_escape: all assertions passed.");
}
EOF
rustc /tmp/test_escape.rs -o /tmp/test_escape && /tmp/test_escape
```
