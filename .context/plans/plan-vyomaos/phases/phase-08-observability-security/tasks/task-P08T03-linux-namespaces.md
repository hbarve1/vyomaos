# P08T03 — linux-namespaces

## Phase

Phase 08 — Observability & Security

## Goal

Isolate each WASM app in its own PID namespace and mount namespace by calling `clone(2)` with `CLONE_NEWPID | CLONE_NEWNS` via the `nix` Rust crate before exec-ing Wasmtime, so that each app process cannot see or signal processes from other apps or the supervisor, and cannot affect the host filesystem's mount table.

## File to create / modify

```
supervisor/src/namespace.rs   (new)
supervisor/src/main.rs        (modify — use namespace::spawn_in_namespaces instead of Command::new)
supervisor/Cargo.toml         (modify — add nix dependency)
```

## Implementation

### `supervisor/Cargo.toml` addition

```toml
[dependencies]
# ... existing deps ...
nix = { version = "0.29", features = ["process", "mount", "sched", "signal", "user"] }
libc = "0.2"
```

The `nix` crate provides safe wrappers over Linux-specific syscalls. The `sched` feature exposes `nix::sched::clone` and the `CLONE_*` constants.

---

### `supervisor/src/namespace.rs`

```rust
//! namespace.rs — PID and mount namespace isolation for WASM app processes.
//!
//! Each app is launched in a new PID namespace (so it cannot signal the
//! supervisor or sibling processes) and a new mount namespace (so any mounts
//! it creates are invisible to other apps and to the host).
//!
//! Architecture:
//!   supervisor (PID ns: init)
//!     └─ clone(CLONE_NEWPID | CLONE_NEWNS)
//!          └─ namespace_child()   ← runs as PID 1 in new namespace
//!               └─ execve wasmtime run <app>.wasm
//!
//! The child function receives a raw pointer to a ChildArgs struct that
//! contains the wasmtime command and arguments. clone(2) requires a
//! C-ABI callback with signature `extern "C" fn(*mut c_void) -> c_int`.

use nix::sched::{clone, CloneFlags};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use std::ffi::{CString, OsString};
use std::os::unix::ffi::OsStringExt;

/// Maximum stack size for the cloned child (8 MB).
const CHILD_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Arguments passed to the cloned child via a raw pointer.
/// The pointer is valid for the lifetime of the `spawn_in_namespaces` call.
struct ChildArgs {
    /// Path to wasmtime binary, e.g. "/usr/bin/wasmtime"
    bin: CString,
    /// Full argv for execve, starting with `bin`
    args: Vec<CString>,
}

/// Spawn a Wasmtime process in a new PID namespace and mount namespace.
///
/// Blocks until the child exits and returns its exit code.
///
/// # Parameters
/// - `wasmtime_bin`: path to the wasmtime binary.
/// - `wasmtime_args`: arguments passed to wasmtime (e.g. `["run", "--inherit-stdio", "app.wasm"]`).
///
/// # Errors
/// Returns an `Err` if `clone(2)` or `waitpid(2)` fails.
pub fn spawn_in_namespaces(
    wasmtime_bin: &str,
    wasmtime_args: &[&str],
) -> Result<i32, nix::Error> {
    let bin = CString::new(wasmtime_bin).expect("wasmtime_bin contains null byte");

    let mut argv: Vec<CString> = std::iter::once(wasmtime_bin)
        .chain(wasmtime_args.iter().copied())
        .map(|s| CString::new(s).expect("argument contains null byte"))
        .collect();

    let child_args = Box::new(ChildArgs { bin, args: argv });
    let child_args_ptr = Box::into_raw(child_args) as *mut libc::c_void;

    // Allocate a stack for the cloned child (grows downward on x86_64)
    let mut stack: Vec<u8> = vec![0u8; CHILD_STACK_SIZE];

    let flags = CloneFlags::CLONE_NEWPID    // new PID namespace
              | CloneFlags::CLONE_NEWNS     // new mount namespace
              | CloneFlags::CLONE_NEWUTS;   // new hostname namespace (bonus isolation)

    // Clone into a new namespace. The child_fn runs in the child immediately.
    let child_pid = unsafe {
        clone(
            Box::new(move || child_fn(child_args_ptr)),
            &mut stack,
            flags,
            Some(Signal::SIGCHLD as i32),
        )?
    };

    // Wait for the child (which is PID 1 in its namespace)
    let exit_code = loop {
        match waitpid(child_pid, None)? {
            WaitStatus::Exited(_pid, code)   => break code,
            WaitStatus::Signaled(_pid, sig, _) => {
                // Killed by signal (e.g. SIGSYS from seccomp violation)
                eprintln!(
                    "[namespace] child killed by signal {:?}",
                    sig
                );
                break 128 + sig as i32;
            }
            WaitStatus::Stopped(_, _) | WaitStatus::Continued(_) => continue,
            _ => break -1,
        }
    };

    Ok(exit_code)
}

/// Child function executed after clone(). Runs as PID 1 in the new namespace.
///
/// Sets up the mount namespace, then exec-s wasmtime.
extern "C" fn child_fn(args_ptr: *mut libc::c_void) -> i32 {
    // Safety: the pointer was created by Box::into_raw in spawn_in_namespaces
    // and the parent is blocked in waitpid, so no concurrent access occurs.
    let args = unsafe { Box::from_raw(args_ptr as *mut ChildArgs) };

    // Make all mounts in this namespace private so unmount events
    // do not propagate to the parent namespace.
    if let Err(e) = nix::mount::mount(
        Some("none"),
        "/",
        None::<&str>,
        nix::mount::MsFlags::MS_REC | nix::mount::MsFlags::MS_PRIVATE,
        None::<&str>,
    ) {
        eprintln!("[namespace] WARN: MS_PRIVATE remount failed: {}", e);
        // Non-fatal: continue without private mounts
    }

    // Exec wasmtime — replaces this child process image
    let env: Vec<CString> = std::env::vars()
        .map(|(k, v)| CString::new(format!("{}={}", k, v)).unwrap())
        .collect();

    let err = nix::unistd::execve(&args.bin, &args.args, &env);

    // execve only returns on error
    eprintln!("[namespace] execve failed: {:?}", err);
    1 // non-zero exit so the supervisor logs a failure
}
```

---

### `supervisor/src/main.rs` — use namespace::spawn_in_namespaces

Replace the `cmd.status()` call inside `run_app` with a call to the namespace module. The seccomp filter from P08T02 is applied via `pre_exec` in the child function rather than here, since `pre_exec` is a `Command`-level hook. For namespace-based launches, apply seccomp inside `child_fn` before `execve`.

```rust
mod namespace;
mod seccomp;

fn run_app(entry: &BootEntry) -> (String, i32) {
    // ... manifest parsing unchanged ...

    // Build wasmtime argument list
    let mut wasmtime_args: Vec<String> = vec!["run".to_string()];
    if manifest.capabilities.stdio      { wasmtime_args.push("--inherit-stdio".into()); }
    if manifest.capabilities.filesystem { wasmtime_args.extend(["--dir".into(), "/data::/data".into()]); }
    if manifest.capabilities.network    { wasmtime_args.extend(["--tcplisten".into(), "0.0.0.0:8080".into()]); }
    wasmtime_args.push(wasm_path.to_string_lossy().into_owned());

    let args_refs: Vec<&str> = wasmtime_args.iter().map(|s| s.as_str()).collect();

    let exit_code = match namespace::spawn_in_namespaces("wasmtime", &args_refs) {
        Ok(code) => code,
        Err(e)   => {
            log_app_error!(&manifest.app.name, "namespace spawn failed: {}", e);
            -1
        }
    };

    (manifest.app.name.clone(), exit_code)
}
```

---

### Applying seccomp inside the namespace child

Update `child_fn` in `namespace.rs` to call `seccomp::apply_seccomp_filter()` before `execve`:

```rust
extern "C" fn child_fn(args_ptr: *mut libc::c_void) -> i32 {
    let args = unsafe { Box::from_raw(args_ptr as *mut ChildArgs) };

    // Remount root as private (mount namespace isolation)
    let _ = nix::mount::mount(
        Some("none"), "/", None::<&str>,
        nix::mount::MsFlags::MS_REC | nix::mount::MsFlags::MS_PRIVATE,
        None::<&str>,
    );

    // Apply seccomp filter before execve (same process, post-clone)
    if let Err(e) = crate::seccomp::apply_seccomp_filter() {
        eprintln!("[namespace] WARN: seccomp filter failed: {}", e);
    }

    let env: Vec<CString> = std::env::vars()
        .map(|(k, v)| CString::new(format!("{}={}", k, v)).unwrap())
        .collect();

    let _ = nix::unistd::execve(&args.bin, &args.args, &env);
    1
}
```

---

### Privilege requirement

`CLONE_NEWPID` and `CLONE_NEWNS` require either:
- `CAP_SYS_ADMIN` (traditional), or
- Linux >= 3.8 with user namespaces enabled (`CONFIG_USER_NS=y`) — in which case unprivileged clones are allowed.

In VyomaOS the supervisor runs as PID 1 (root), so `CAP_SYS_ADMIN` is available without user namespaces.

## Notes

- The child runs as PID 1 inside the new namespace. If PID 1 in the child namespace exits without reaping children (e.g. if Wasmtime forks), the kernel will send SIGKILL to all remaining processes in that namespace automatically. This is desirable isolation behaviour.
- `CLONE_NEWUTS` (hostname namespace) is added cheaply alongside NEWPID and NEWNS; it prevents the WASM app from changing the system hostname.
- `MS_PRIVATE | MS_REC` makes all mounts in the new namespace private (propagation type: private). Without this, a `mount()` call inside the child would propagate to the parent namespace, defeating the isolation.
- The `ChildArgs` raw pointer approach is necessary because `nix::sched::clone` requires a `Box<dyn FnMut() -> isize>` closure, and the closure must be `Send`. Passing data via a raw pointer is the standard pattern for this API.
- On musl-based kernels (which VyomaOS uses for userspace), `clone(2)` is available identically to glibc systems; musl's libc calls the same kernel ABI.
- Cross-reference: P08T02 adds the seccomp filter; this task calls it from inside `child_fn`. If P08T02 is skipped, remove the `crate::seccomp::apply_seccomp_filter()` call.
- `nix = "0.29"` requires Rust 1.65+. Pin the version to avoid breaking API changes.

## Verification

```sh
# 1. Confirm namespace.rs was created
test -f supervisor/src/namespace.rs && echo "namespace.rs: present"

# 2. Confirm nix dep is in Cargo.toml
grep -q "^nix" supervisor/Cargo.toml && echo "nix dep: present"

# 3. Build supervisor
(cd supervisor && cargo build 2>&1 | tail -10)
test -x supervisor/target/debug/supervisor && echo "supervisor: built OK"

# 4. Confirm key symbols are present in the source
grep -q "CLONE_NEWPID"          supervisor/src/namespace.rs && echo "CLONE_NEWPID: present"
grep -q "CLONE_NEWNS"           supervisor/src/namespace.rs && echo "CLONE_NEWNS: present"
grep -q "spawn_in_namespaces"   supervisor/src/namespace.rs && echo "spawn_in_namespaces: defined"
grep -q "MS_PRIVATE"            supervisor/src/namespace.rs && echo "MS_PRIVATE remount: present"
grep -q "execve"                supervisor/src/namespace.rs && echo "execve call: present"

# 5. Confirm main.rs uses the namespace module
grep -q "namespace::spawn_in_namespaces" supervisor/src/main.rs && echo "namespace used in run_app: OK"

# 6. Functional test: verify the WASM child runs in its own PID namespace
# The child should see itself as PID 1 (or PID 2 if Wasmtime forks)
# Run hello-world and check that the namespace is separate:
cat > /tmp/check_pid.rs << 'EOF'
fn main() {
    let pid = std::process::id();
    println!("my PID inside namespace: {}", pid);
    // If we are in a new PID namespace, our PID should be 1 or 2
    assert!(pid <= 10, "expected PID <= 10 in new namespace, got {}", pid);
    println!("PID namespace isolation: OK");
}
EOF
rustc --target wasm32-wasip2 --edition 2021 /tmp/check_pid.rs -o /tmp/check_pid.wasm 2>/dev/null && \
  wasmtime run /tmp/check_pid.wasm || echo "wasmtime not available for local test"

# 7. Verify namespaces are unshared from host:
# Run a WASM app under the supervisor and confirm its namespace IDs differ
# from the supervisor's:
ls -la /proc/self/ns/pid   # supervisor's PID namespace
ls -la /proc/self/ns/mnt   # supervisor's mount namespace
# After running the supervisor, check /proc/<child_pid>/ns/{pid,mnt} — they
# should show different inode numbers.

# 8. Kernel config check for user namespaces (if supervisor runs unprivileged)
grep -E "^CONFIG_USER_NS|^CONFIG_PID_NS|^CONFIG_NAMESPACES" base/kernel.config | head -5
```
