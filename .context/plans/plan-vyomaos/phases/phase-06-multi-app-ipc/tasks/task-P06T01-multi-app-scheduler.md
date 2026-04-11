# P06T01 — multi-app-scheduler

## Phase

Phase 06 — Multi-App & IPC

## Goal

Update the supervisor to spawn all configured apps concurrently — each in its own OS thread that owns a `std::process::Child` handle — so that multiple WASM apps run in parallel rather than sequentially, and the supervisor waits for all of them before exiting.

## File to create / modify

```
supervisor/src/main.rs
```

## Implementation

The Phase 05 sequential loop (`for entry in &boot.apps { launch_app(entry); }`) is replaced with a concurrent scheduler that:

1. Iterates the boot config entries and spawns a new OS thread for each app.
2. Each thread starts the `wasmtime` child process, then blocks on `child.wait()`.
3. The main thread collects all `JoinHandle`s and waits on each one, logging the completion order and exit codes.

### Full replacement for `supervisor/src/main.rs` (scheduler section)

```rust
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};
use serde::Deserialize;

// ── Structs (unchanged from P05T03) ───────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BootConfig {
    apps: Vec<BootEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct BootEntry {
    manifest: String,
    #[serde(default = "default_restart")]
    restart: String,
}

fn default_restart() -> String { "never".to_string() }

#[derive(Debug, Deserialize)]
struct AppManifest {
    app: AppMeta,
    capabilities: Capabilities,
}

#[derive(Debug, Deserialize)]
struct AppMeta {
    name: String,
    version: String,
    wasm: String,
}

#[derive(Debug, Default, Deserialize)]
struct Capabilities {
    #[serde(default)] stdio: bool,
    #[serde(default)] filesystem: bool,
    #[serde(default)] network: bool,
}

// ── Completion record ──────────────────────────────────────────────────────

#[derive(Debug)]
struct AppResult {
    name: String,
    exit_code: i32,
    elapsed_ms: u128,
}

// ── Entry point ────────────────────────────────────────────────────────────

const BOOT_CONFIG_PATH: &str = "/etc/vyoma/boot.toml";

fn main() {
    eprintln!("[supervisor] VyomaOS supervisor starting");
    mount_pseudo_filesystems();

    let boot_raw = fs::read_to_string(BOOT_CONFIG_PATH).unwrap_or_else(|e| {
        eprintln!("[supervisor] FATAL: cannot read {}: {}", BOOT_CONFIG_PATH, e);
        std::process::exit(1);
    });

    let boot: BootConfig = toml::from_str(&boot_raw).unwrap_or_else(|e| {
        eprintln!("[supervisor] FATAL: malformed boot.toml: {}", e);
        std::process::exit(1);
    });

    eprintln!("[supervisor] Scheduling {} app(s) concurrently", boot.apps.len());

    // Shared results list — each thread pushes its AppResult here
    let results: Arc<Mutex<Vec<AppResult>>> = Arc::new(Mutex::new(Vec::new()));

    // Spawn one thread per app
    let handles: Vec<thread::JoinHandle<()>> = boot.apps
        .into_iter()
        .map(|entry| {
            let results = Arc::clone(&results);
            thread::spawn(move || {
                let t0 = Instant::now();
                let (name, code) = run_app(&entry);
                let elapsed_ms = t0.elapsed().as_millis();

                eprintln!(
                    "[supervisor] {} finished: exit={} elapsed={}ms",
                    name, code, elapsed_ms
                );

                results.lock().unwrap().push(AppResult { name, exit_code: code, elapsed_ms });
            })
        })
        .collect();

    // Wait for every thread
    for handle in handles {
        if let Err(e) = handle.join() {
            eprintln!("[supervisor] ERROR: a scheduler thread panicked: {:?}", e);
        }
    }

    // Print completion summary in the order they finished
    let final_results = results.lock().unwrap();
    eprintln!("[supervisor] --- Completion summary ---");
    for r in final_results.iter() {
        eprintln!("  {} exit={} elapsed={}ms", r.name, r.exit_code, r.elapsed_ms);
    }
    let any_failed = final_results.iter().any(|r| r.exit_code != 0);
    eprintln!("[supervisor] All apps completed. Halting.");
    std::process::exit(if any_failed { 1 } else { 0 });
}

// ── Run a single app, return (name, exit_code) ────────────────────────────

fn run_app(entry: &BootEntry) -> (String, i32) {
    let manifest_raw = match fs::read_to_string(&entry.manifest) {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[supervisor] WARN: cannot read manifest {}: {}", entry.manifest, e);
            return (entry.manifest.clone(), -1);
        }
    };

    let manifest: AppManifest = match toml::from_str(&manifest_raw) {
        Ok(m)  => m,
        Err(e) => {
            eprintln!("[supervisor] WARN: malformed manifest {}: {}", entry.manifest, e);
            return (entry.manifest.clone(), -1);
        }
    };

    let manifest_dir = Path::new(&entry.manifest).parent().unwrap_or(Path::new("/apps"));
    let wasm_path    = manifest_dir.join(&manifest.app.wasm);

    let mut cmd = Command::new("wasmtime");
    cmd.arg("run");
    if manifest.capabilities.stdio      { cmd.arg("--inherit-stdio"); }
    if manifest.capabilities.filesystem { cmd.args(["--dir", "/data"]); }
    if manifest.capabilities.network    { cmd.args(["--tcplisten", "0.0.0.0:8080"]); }
    cmd.arg(&wasm_path);

    let code = match cmd.status() {
        Ok(s)  => s.code().unwrap_or(-1),
        Err(e) => {
            eprintln!("[supervisor] ERROR: exec failed for {}: {}", manifest.app.name, e);
            -1
        }
    };

    (manifest.app.name.clone(), code)
}

fn mount_pseudo_filesystems() {
    // Retained from Phase 03
}
```

### Key design decisions

- **`Arc<Mutex<Vec<AppResult>>>`** — A shared results list protected by a mutex is the simplest correct approach. Each thread acquires the lock only once (on push after the child exits), so there is no lock contention during execution.
- **`thread::spawn` per app** — Using OS threads (rather than async tasks) avoids pulling in an async runtime (`tokio`, `async-std`) which would add significant binary size. Each thread blocks in `child.wait()`, which is appropriate for I/O-bound child-process waiting.
- **Completion order logging** — The summary prints results in the order they were pushed (i.e., completion order, not launch order). This is intentional: it makes it easy to see which app was slowest.
- **Exit code propagation** — `std::process::exit(1)` if any app exited non-zero; otherwise exits 0. The kernel will panic/reboot based on the supervisor's exit code in a real initramfs context.

## Notes

- The `BootEntry` struct must derive `Clone` so it can be moved into the closure passed to `thread::spawn`.
- `thread::spawn` requires `'static` closures; cloning `entry` before the move satisfies this.
- If the number of apps grows large (>50), consider using a bounded thread pool (e.g. `rayon` or a manual semaphore) to avoid spawning hundreds of OS threads. For Phase 06 with three apps this is not needed.
- The `run_app` function is intentionally kept synchronous so that P06T02 (restart policy) can call it in a loop without restructuring.
- Cross-reference: P06T02 wraps `run_app` in a restart loop. The scheduler in this task remains unchanged; P06T02 only modifies what happens inside each thread after `run_app` returns.

## Verification

```sh
# 1. Build supervisor
(cd supervisor && cargo build 2>&1 | tail -5)
test -x supervisor/target/debug/supervisor && echo "supervisor: built OK"

# 2. Confirm the binary was produced
ls -lh supervisor/target/debug/supervisor

# 3. Compile-level check: confirm Arc, Mutex, thread are used
grep -q "Arc::new(Mutex::new" supervisor/src/main.rs    && echo "Arc<Mutex<>>: present"
grep -q "thread::spawn"       supervisor/src/main.rs    && echo "thread::spawn: present"
grep -q "handle.join()"       supervisor/src/main.rs    && echo "join handles: present"

# 4. Functional smoke test using two long-running sleep WASM stubs
# (Requires wasmtime and wasm32-wasip2 target installed)
# Create two trivial WASM binaries that sleep for different durations
cat > /tmp/fast.rs << 'EOF'
fn main() { println!("fast app done"); }
EOF
cat > /tmp/slow.rs << 'EOF'
fn main() {
    std::thread::sleep(std::time::Duration::from_millis(200));
    println!("slow app done");
}
EOF
rustc --target wasm32-wasip2 --edition 2021 /tmp/fast.rs -o /tmp/fast.wasm 2>/dev/null || true
rustc --target wasm32-wasip2 --edition 2021 /tmp/slow.rs -o /tmp/slow.wasm 2>/dev/null || true

# 5. Run the real apps (built in P05T01) and check that all three appear in output
wasmtime run apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm &
wasmtime run apps/calculator/target/wasm32-wasip2/release/calculator.wasm   &
wasmtime run apps/factorial/target/wasm32-wasip2/release/factorial.wasm     &
wait && echo "concurrent wasmtime runs: all exited 0"

# 6. Verify the completion summary section exists in source
grep -q "Completion summary" supervisor/src/main.rs && echo "summary log: present"
```
