# P04T03 — supervisor-runtime-integration

## Phase

Phase 04 — WASM Runtime

## Goal

Wire the supervisor's `run_app()` to invoke wasmtime with the correct WASI CLI arguments so that WASI stdio works end-to-end, pipe app stdout and stderr to the serial console, and verify the full pipeline: supervisor boots → discovers `/apps/hello-world.wasm` → runs it via wasmtime → `"Hello from VyomaOS WASM!"` appears on ttyS0.

## File to create / modify

```
supervisor/src/main.rs
```

## Implementation

Update `run_app()` (introduced in P03T04) with the correct wasmtime invocation for WASI Preview 2 binaries and explicit stdio inheritance.

```rust
use std::path::Path;

/// Run a single WASM application via wasmtime.
///
/// Wasmtime CLI for WASI Preview 2 (Component Model) binaries:
///   wasmtime run -- <path>
///
/// The `--` is required to separate wasmtime flags from the WASM file path.
/// WASI stdio is enabled by default in wasmtime 14+; no extra flags needed.
///
/// Stdout and stderr are inherited so that app output goes directly to
/// the serial console (ttyS0). In Phase 08 this will be piped for
/// structured log capture.
fn run_app(path: &Path) {
    let display = path.display();
    eprintln!("vyoma-supervisor: starting app: {display}");

    let status = std::process::Command::new("/usr/bin/wasmtime")
        .args([
            "run",
            "--",
            path.to_str().expect("WASM path must be valid UTF-8"),
        ])
        .stdin(std::process::Stdio::null())     // apps should not read from stdin
        .stdout(std::process::Stdio::inherit()) // app stdout -> ttyS0
        .stderr(std::process::Stdio::inherit()) // app stderr -> ttyS0
        .status();

    match status {
        Ok(exit_status) => {
            if exit_status.success() {
                eprintln!("vyoma-supervisor: app exited cleanly: {display}");
            } else {
                let code = exit_status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "killed-by-signal".to_string());
                eprintln!(
                    "vyoma-supervisor: app exited with code {code}: {display}"
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("vyoma-supervisor: ERROR: /usr/bin/wasmtime not found — \
                       rebuild rootfs with P04T01 applied");
        }
        Err(e) => {
            eprintln!(
                "vyoma-supervisor: failed to exec wasmtime for {display}: {e}"
            );
        }
    }
}
```

### Complete `main()` for Phase 04

```rust
fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();                       // P03T02
    eprintln!("vyoma-supervisor: filesystems ready");

    let apps = discover_apps();               // P03T03

    if apps.is_empty() {
        eprintln!("vyoma-supervisor: no apps found — idling");
        loop { std::thread::park(); }
    }

    // Sequential run loop: run each app to completion, then repeat.
    // Replaced by concurrent scheduler in Phase 06.
    loop {
        for app in &apps {
            run_app(app);
        }
        // Prevent tight-spin if all apps exit immediately.
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
```

### Rationale for the wasmtime flags

| Flag | Reason |
|------|--------|
| `run` | Wasmtime subcommand for executing a WASM file |
| `--` | Delimiter: everything after is the WASM path, not a wasmtime flag |
| `<path>` | Absolute path to the `.wasm` file inside the VM (`/apps/hello-world.wasm`) |

WASI stdio (stdin/stdout/stderr) is **on by default** in wasmtime 14+ for `wasm32-wasip2` components. The older `--wasi-modules=wasi:cli` flag is unnecessary with current wasmtime releases and has been removed from the wasmtime CLI. The `Stdio::inherit()` on the Command side is sufficient.

## Notes

- `Stdio::null()` for stdin is deliberate: WASM apps in this OS have no interactive terminal. Inheriting stdin would make the app block on `read()` from the console input, which is not desirable.
- The `ErrorKind::NotFound` branch gives an actionable error message rather than a generic "exec failed" — critical for diagnosing missing wasmtime in the rootfs.
- Do not pass `--mapdir` or `--dir` in this phase. App filesystem access is not sandboxed yet; Phase 07 adds explicit WASI preopened directory grants.
- Do not pass `--env` unless an app explicitly requires environment variables; an empty environment is the secure default.
- The `wasmtime run -- path` form works for both core WASM modules (WASI Preview 1) and components (WASI Preview 2). Wasmtime auto-detects the format from the binary's magic bytes / component layer marker.

## Verification

```sh
# 1. run_app uses "run" subcommand and "--" separator
grep -q '"run"' supervisor/src/main.rs
grep -q '"--"' supervisor/src/main.rs

# 2. stdin is set to null (not inherited)
grep -q 'Stdio::null()' supervisor/src/main.rs

# 3. stdout and stderr are inherited
grep -c 'Stdio::inherit()' supervisor/src/main.rs | grep -qE '^2$'

# 4. NotFound error is handled with actionable message
grep -q 'NotFound' supervisor/src/main.rs
grep -q 'rebuild rootfs\|not found' supervisor/src/main.rs

# 5. Compiles cleanly for musl target
cargo build --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl --release 2>&1 | grep -v "^warning"

# 6. End-to-end integration test
#    Build everything and run QEMU — expect hello-world output on stdout
make build
timeout 30 make run 2>&1 | grep -q "Hello from VyomaOS WASM"

# 7. Confirm supervisor lifecycle messages appear
timeout 30 make run 2>&1 | grep -q "vyoma-supervisor starting"
timeout 30 make run 2>&1 | grep -q "filesystems ready"
timeout 30 make run 2>&1 | grep -q "discovered app"
timeout 30 make run 2>&1 | grep -q "exited cleanly\|exited with code"
```
