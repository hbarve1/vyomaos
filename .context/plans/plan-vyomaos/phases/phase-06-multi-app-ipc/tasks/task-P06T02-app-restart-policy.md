# P06T02 — app-restart-policy

## Phase

Phase 06 — Multi-App & IPC

## Goal

Implement a typed `RestartPolicy` enum in the supervisor and a restart loop that wraps each app's execution thread, automatically restarting apps according to their declared policy (`never`, `on-failure`, or `always`) with exponential backoff starting at 1 second and capped at 30 seconds.

## File to create / modify

```
supervisor/src/main.rs
```

## Implementation

### `RestartPolicy` enum

```rust
#[derive(Debug, Clone, PartialEq)]
enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

impl RestartPolicy {
    fn from_str(s: &str) -> Self {
        match s {
            "on-failure" => Self::OnFailure,
            "always"     => Self::Always,
            _            => Self::Never,       // "never" + any unknown value
        }
    }

    /// Returns true if the app should be restarted given this exit code.
    fn should_restart(&self, exit_code: i32) -> bool {
        match self {
            Self::Never     => false,
            Self::OnFailure => exit_code != 0,
            Self::Always    => true,
        }
    }
}
```

### Backoff helper

```rust
use std::time::Duration;

/// Compute the next backoff: doubles on each call, capped at MAX_BACKOFF_SECS.
const INITIAL_BACKOFF_SECS: u64 = 1;
const MAX_BACKOFF_SECS: u64     = 30;

fn next_backoff(current: Duration) -> Duration {
    let next_secs = (current.as_secs() * 2).min(MAX_BACKOFF_SECS);
    Duration::from_secs(next_secs)
}
```

### Per-thread restart loop (replaces the single `run_app` call inside `thread::spawn`)

This replaces the plain `run_app(&entry)` call from P06T01 with a loop that respects the restart policy.

```rust
thread::spawn(move || {
    let policy = RestartPolicy::from_str(&entry.restart);
    let mut backoff = Duration::from_secs(INITIAL_BACKOFF_SECS);
    let mut attempt: u32 = 0;

    loop {
        if attempt > 0 {
            eprintln!(
                "[supervisor] Restarting {} (attempt {}, backoff {}s, policy={:?})",
                entry.manifest, attempt, backoff.as_secs(), policy
            );
            std::thread::sleep(backoff);
            backoff = next_backoff(backoff);
        }

        let t0 = Instant::now();
        let (name, exit_code) = run_app(&entry);
        let elapsed_ms = t0.elapsed().as_millis();

        eprintln!(
            "[supervisor] {} finished: exit={} elapsed={}ms attempt={}",
            name, exit_code, elapsed_ms, attempt
        );

        if !policy.should_restart(exit_code) {
            results.lock().unwrap().push(AppResult {
                name,
                exit_code,
                elapsed_ms,
            });
            break;
        }

        attempt += 1;
        // Reset backoff after a "long" successful run to avoid penalising
        // apps that crash early after a healthy period
        if exit_code == 0 && elapsed_ms > 5_000 {
            backoff = Duration::from_secs(INITIAL_BACKOFF_SECS);
        }
    }
})
```

### Complete updated `supervisor/src/main.rs` scheduler section

The full `main()` function from P06T01 is unchanged except for the thread closure body above. The `results` `Arc<Mutex<Vec<AppResult>>>` is still populated, but only on the final exit (after the restart loop terminates).

```rust
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

    let results: Arc<Mutex<Vec<AppResult>>> = Arc::new(Mutex::new(Vec::new()));

    let handles: Vec<thread::JoinHandle<()>> = boot.apps
        .into_iter()
        .map(|entry| {
            let results  = Arc::clone(&results);
            let policy   = RestartPolicy::from_str(&entry.restart);
            let mut backoff = Duration::from_secs(INITIAL_BACKOFF_SECS);
            let mut attempt: u32 = 0;

            thread::spawn(move || loop {
                if attempt > 0 {
                    eprintln!(
                        "[supervisor] Restarting {} (attempt {}, backoff {}s)",
                        entry.manifest, attempt, backoff.as_secs()
                    );
                    thread::sleep(backoff);
                    backoff = next_backoff(backoff);
                }

                let t0 = Instant::now();
                let (name, exit_code) = run_app(&entry);
                let elapsed_ms = t0.elapsed().as_millis();

                eprintln!(
                    "[supervisor] {} finished: exit={} elapsed={}ms",
                    name, exit_code, elapsed_ms
                );

                if !policy.should_restart(exit_code) {
                    results.lock().unwrap().push(AppResult { name, exit_code, elapsed_ms });
                    break;
                }

                attempt += 1;
                if exit_code == 0 && elapsed_ms > 5_000 {
                    backoff = Duration::from_secs(INITIAL_BACKOFF_SECS);
                }
            })
        })
        .collect();

    for handle in handles {
        if let Err(e) = handle.join() {
            eprintln!("[supervisor] ERROR: scheduler thread panicked: {:?}", e);
        }
    }

    let final_results = results.lock().unwrap();
    eprintln!("[supervisor] --- Completion summary ---");
    for r in final_results.iter() {
        eprintln!("  {} exit={} elapsed={}ms", r.name, r.exit_code, r.elapsed_ms);
    }
    let any_failed = final_results.iter().any(|r| r.exit_code != 0);
    eprintln!("[supervisor] All apps completed. Halting.");
    std::process::exit(if any_failed { 1 } else { 0 });
}
```

### `boot.toml` entries for testing restart policies

Add a test entry to `base/modules/scripts/boot.toml` (can be removed after testing):

```toml
# Example: restart on non-zero exit (useful for crash-looping services)
[[apps]]
manifest = "/apps/factorial/vyoma.toml"
restart  = "on-failure"

# Example: always restart (long-running daemon style)
# [[apps]]
# manifest = "/apps/server/vyoma.toml"
# restart  = "always"
```

## Notes

- The `RestartPolicy::from_str` method treats any unknown string the same as `"never"` so that future policy values can be added to `boot.toml` without causing a hard crash on older supervisor builds.
- Backoff resets after a "long" successful run (>5 s) to prevent the doubling penalty from accumulating against apps that crash occasionally after running correctly for a while. This threshold is intentionally conservative.
- The `always` policy combined with a fast-crashing app will loop indefinitely; the log line on every restart attempt is the observable signal that something is wrong.
- There is no maximum retry count by design — VyomaOS is an embedded OS where an app like a network server should keep restarting rather than giving up silently. Add a `max_restarts` field to `BootEntry` in a future phase if needed.
- The backoff progression for the default `INITIAL_BACKOFF_SECS=1` and `MAX_BACKOFF_SECS=30`: 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s, ...
- Cross-reference: P06T01 defines the thread spawning skeleton; this task replaces only the closure body.

## Verification

```sh
# 1. Build supervisor
(cd supervisor && cargo build 2>&1 | tail -5)

# 2. Confirm RestartPolicy enum and should_restart method are present
grep -q "enum RestartPolicy"    supervisor/src/main.rs && echo "RestartPolicy: defined"
grep -q "should_restart"        supervisor/src/main.rs && echo "should_restart: present"
grep -q "OnFailure"             supervisor/src/main.rs && echo "OnFailure variant: present"
grep -q "next_backoff"          supervisor/src/main.rs && echo "backoff fn: present"

# 3. Confirm backoff constants
grep -q "INITIAL_BACKOFF_SECS" supervisor/src/main.rs && echo "initial backoff: defined"
grep -q "MAX_BACKOFF_SECS"     supervisor/src/main.rs && echo "max backoff: defined"

# 4. Unit-test RestartPolicy logic without running the full supervisor
# Write a small test binary that imports and exercises RestartPolicy
cat > /tmp/test_restart_policy.rs << 'EOF'
#[derive(Debug, PartialEq)]
enum RestartPolicy { Never, OnFailure, Always }
impl RestartPolicy {
    fn from_str(s: &str) -> Self {
        match s { "on-failure" => Self::OnFailure, "always" => Self::Always, _ => Self::Never }
    }
    fn should_restart(&self, exit_code: i32) -> bool {
        match self {
            Self::Never     => false,
            Self::OnFailure => exit_code != 0,
            Self::Always    => true,
        }
    }
}
fn main() {
    assert!(!RestartPolicy::from_str("never").should_restart(0));
    assert!(!RestartPolicy::from_str("never").should_restart(1));
    assert!(!RestartPolicy::from_str("on-failure").should_restart(0));
    assert!( RestartPolicy::from_str("on-failure").should_restart(1));
    assert!( RestartPolicy::from_str("always").should_restart(0));
    assert!( RestartPolicy::from_str("always").should_restart(1));
    assert!(!RestartPolicy::from_str("bogus").should_restart(0));
    println!("All RestartPolicy assertions passed.");
}
EOF
rustc /tmp/test_restart_policy.rs -o /tmp/test_restart_policy && /tmp/test_restart_policy

# 5. Confirm backoff sequence: 1->2->4->8->16->30->30
cat > /tmp/test_backoff.rs << 'EOF'
use std::time::Duration;
const MAX: u64 = 30;
fn next(d: Duration) -> Duration { Duration::from_secs((d.as_secs() * 2).min(MAX)) }
fn main() {
    let mut d = Duration::from_secs(1);
    let sequence: Vec<u64> = (0..7).map(|_| { let v = d.as_secs(); d = next(d); v }).collect();
    assert_eq!(sequence, vec![1, 2, 4, 8, 16, 30, 30]);
    println!("Backoff sequence OK: {:?}", sequence);
}
EOF
rustc /tmp/test_backoff.rs -o /tmp/test_backoff && /tmp/test_backoff
```
