# P03T04 — process-lifecycle

## Phase

Phase 03 — Rust Supervisor

## Goal

Implement `run_app(path: &Path)` in `supervisor/src/main.rs` that uses `std::process::Command` to exec `/usr/bin/wasmtime` with the WASM path as its argument, inherits stdio so app output reaches the console, waits for the child to exit, and logs the exit code — completing the sequential single-app execution loop.

## File to create / modify

```
supervisor/src/main.rs
```

## Implementation

### New function: `run_app`

```rust
use std::path::Path;

/// Run a single WASM application via wasmtime and wait for it to exit.
///
/// Wasmtime is expected at /usr/bin/wasmtime (installed in P04T01).
/// Stdio is inherited so app stdout/stderr reach ttyS0 directly.
///
/// This is a *sequential* runner: the function blocks until the app exits.
/// Concurrent execution is added in Phase 06.
fn run_app(path: &Path) {
    let display = path.display();
    eprintln!("vyoma-supervisor: starting app: {display}");

    let status = std::process::Command::new("/usr/bin/wasmtime")
        .args([
            "run",
            "--",
            path.to_str().expect("app path must be valid UTF-8"),
        ])
        // Inherit the supervisor's stdin/stdout/stderr so WASM app output
        // appears directly on the serial console (ttyS0).
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    match status {
        Ok(exit_status) => {
            if exit_status.success() {
                eprintln!("vyoma-supervisor: app exited cleanly: {display}");
            } else {
                let code = exit_status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                eprintln!(
                    "vyoma-supervisor: app exited with code {code}: {display}"
                );
            }
        }
        Err(e) => {
            eprintln!(
                "vyoma-supervisor: failed to launch wasmtime for {display}: {e}"
            );
        }
    }
}
```

### Updated `main()` — sequential run loop

Replace the placeholder `loop` from P03T03 with:

```rust
fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();                       // P03T02
    eprintln!("vyoma-supervisor: filesystems mounted");

    let apps = discover_apps();               // P03T03

    if apps.is_empty() {
        eprintln!("vyoma-supervisor: no apps to run, idling");
        loop { std::thread::park(); }
    }

    // Sequential run loop: run each app in filename order, then repeat.
    // Phase 06 replaces this with a concurrent multi-app scheduler.
    loop {
        for app in &apps {
            run_app(app);
        }
        // Brief pause between full app cycles to avoid a tight spin
        // if all apps exit immediately (e.g. during development).
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
```

## Notes

- `std::process::Command` on Linux internally calls `fork(2)` + `execve(2)` (via glibc/musl). The parent (supervisor) calls `wait(2)` inside `.status()` before returning — this is the sequential waitpid behaviour specified in the goal.
- Using `.status()` rather than `.spawn()` + `.wait()` is intentional for the sequential case: it is a single blocking call with no intermediate state to manage. Phase 06 switches to `.spawn()` so the supervisor can hold handles to multiple running children.
- `Stdio::inherit()` is used for all three streams. This means the WASM app's stdout and stderr go directly to ttyS0. In Phase 08, this will be replaced with piped I/O for structured log capture.
- The `"--"` argument before the WASM path is the POSIX convention to signal end-of-options. Without it, a WASM file starting with `-` would be misinterpreted as a wasmtime flag.
- Wasmtime requires `run` before `--` because the `wasmtime` CLI dispatches via subcommands. The full invocation is: `wasmtime run -- /apps/hello-world.wasm`.
- If `/usr/bin/wasmtime` is not present, `Command::new` returns `Err(ENOENT)` and `run_app` logs a warning but does not panic — the supervisor continues running (important for PID-1 stability).
- The outer `loop { for app in &apps ... }` means apps are restarted automatically after they exit. This is intentional for Phase 03. Phase 06 (P06T02) adds a proper restart policy with back-off.

## Verification

```sh
# 1. run_app function is defined
grep -q 'fn run_app' supervisor/src/main.rs

# 2. Uses std::process::Command with wasmtime
grep -q '/usr/bin/wasmtime' supervisor/src/main.rs

# 3. Stdio is inherited
grep -q 'Stdio::inherit' supervisor/src/main.rs

# 4. Exit code is logged
grep -q 'exited with code\|exited cleanly' supervisor/src/main.rs

# 5. Compiles for musl target without errors
cargo build --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl --release 2>&1 | grep -v "^warning"

# 6. Release binary is statically linked
file supervisor/target/x86_64-unknown-linux-musl/release/supervisor \
  | grep -q "statically linked"

# 7. Integration test: full boot with hello-world.wasm
#    (requires P04T01 wasmtime in rootfs and P04T02 hello-world.wasm)
# make build
# make run 2>&1 | grep -q "Hello from VyomaOS WASM"

# 8. Supervisor continues running after app exits (restart loop test)
# make run 2>&1 | grep -c "starting app" | awk '{if($1>=2) exit 0; else exit 1}'
```
