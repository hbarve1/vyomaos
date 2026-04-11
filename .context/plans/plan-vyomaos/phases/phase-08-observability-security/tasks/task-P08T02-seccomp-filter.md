# P08T02 — seccomp-filter

## Phase

Phase 08 — Observability & Security

## Goal

Apply a seccomp allowlist filter to each Wasmtime child process before it executes, restricting the child to a minimal set of Linux syscalls (read, write, mmap, mprotect, brk, exit, exit_group, clock_gettime, futex, sigaltstack, munmap) using the `seccompiler` Rust crate, so that any unexpected syscall terminates the process with `SIGSYS` instead of silently succeeding.

## File to create / modify

```
supervisor/src/seccomp.rs   (new)
supervisor/src/main.rs      (modify — call apply_seccomp_filter() in child before exec)
supervisor/Cargo.toml       (modify — add seccompiler dependency)
```

## Implementation

### `supervisor/Cargo.toml` addition

```toml
[dependencies]
# ... existing deps ...
seccompiler = "0.4"
```

`seccompiler` is a pure-Rust crate that compiles BPF seccomp rules and installs them via `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, ...)`. It requires Linux >= 3.5 and a kernel built with `CONFIG_SECCOMP_FILTER=y`.

---

### `supervisor/src/seccomp.rs`

```rust
//! seccomp.rs — apply a seccomp-BPF allowlist to the calling process.
//!
//! Call `apply_seccomp_filter()` in the child after `fork()` but before
//! `exec()`, i.e. inside the closure passed to `std::os::unix::process::
//! CommandExt::pre_exec`.
//!
//! Any syscall not in the allowlist causes the kernel to send SIGSYS to
//! the process, which terminates it immediately.

use seccompiler::{
    BpfProgram, SeccompAction, SeccompFilter, SeccompRule,
    SyscallRuleSet,
};
use std::collections::BTreeMap;

/// The minimal set of syscalls required for Wasmtime to run a WASM module.
///
/// Derived by running `strace -ff wasmtime run hello.wasm` on a clean
/// Linux environment and retaining only the syscalls actually called.
/// Extend this list if Wasmtime or the WASM module require additional calls.
const ALLOWED_SYSCALLS: &[i64] = &[
    libc::SYS_read,
    libc::SYS_write,
    libc::SYS_readv,
    libc::SYS_writev,
    libc::SYS_close,
    libc::SYS_fstat,
    libc::SYS_mmap,
    libc::SYS_mprotect,
    libc::SYS_munmap,
    libc::SYS_brk,
    libc::SYS_rt_sigaction,
    libc::SYS_rt_sigprocmask,
    libc::SYS_rt_sigreturn,
    libc::SYS_pread64,
    libc::SYS_pwrite64,
    libc::SYS_lseek,
    libc::SYS_exit,
    libc::SYS_exit_group,
    libc::SYS_futex,
    libc::SYS_clock_gettime,
    libc::SYS_clock_nanosleep,
    libc::SYS_nanosleep,
    libc::SYS_getpid,
    libc::SYS_gettid,
    libc::SYS_sigaltstack,
    libc::SYS_tgkill,
    libc::SYS_madvise,
    libc::SYS_prctl,
    libc::SYS_sched_yield,
    libc::SYS_sched_getaffinity,
    // Allow seccomp itself so a nested filter can be installed
    libc::SYS_seccomp,
];

/// Compile and install a seccomp-BPF allowlist filter for the current process.
///
/// This function is intended to be called from the pre_exec() hook of a child
/// process, immediately before execve() hands control to wasmtime.
///
/// # Safety
/// This function calls `libc` FFI. It must only be called from a single-
/// threaded context (i.e., directly after fork, before exec).
pub fn apply_seccomp_filter() -> Result<(), Box<dyn std::error::Error>> {
    // Build a map of syscall number → list of rules (empty list = always allow)
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    for &syscall in ALLOWED_SYSCALLS {
        rules.insert(syscall, vec![]);
    }

    // Default action for any syscall NOT in the allowlist: SIGSYS (kill)
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::KillProcess,   // default: kill if not in list
        SeccompAction::Allow,         // action for listed syscalls
        std::env::consts::ARCH.try_into()?,
    )?;

    // Compile to BPF bytecode and install via prctl()
    let program: BpfProgram = filter.try_into()?;
    seccompiler::apply_filter(&program)?;

    Ok(())
}
```

---

### `supervisor/src/main.rs` — using `pre_exec` to apply seccomp

Import the seccomp module and add it to the `run_app` function via `CommandExt::pre_exec`. `pre_exec` runs the given closure in the child process after `fork()` but before `exec()`, which is exactly the right moment to apply a seccomp filter.

```rust
mod seccomp;

use std::os::unix::process::CommandExt;

fn run_app(entry: &BootEntry) -> (String, i32) {
    // ... manifest parsing unchanged ...

    let mut cmd = Command::new("wasmtime");
    cmd.arg("run");
    if manifest.capabilities.stdio      { cmd.arg("--inherit-stdio"); }
    if manifest.capabilities.filesystem { cmd.args(["--dir", "/data::/data"]); }
    if manifest.capabilities.network    { cmd.args(["--tcplisten", "0.0.0.0:8080"]); }
    cmd.arg(&wasm_path);

    // Install the seccomp filter in the child process before execve()
    unsafe {
        cmd.pre_exec(|| {
            seccomp::apply_seccomp_filter()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        });
    }

    let code = match cmd.status() {
        Ok(s)  => s.code().unwrap_or(-1),
        Err(e) => {
            log_app_error!(&manifest.app.name, "exec failed: {}", e);
            -1
        }
    };

    (manifest.app.name.clone(), code)
}
```

The `unsafe` block is required by `pre_exec`'s safety contract: the closure runs in a single-threaded fork child, so it must not allocate, call async-signal-unsafe functions, or use mutexes. The `seccomp::apply_seccomp_filter` function only calls `seccompiler::apply_filter` (a `prctl` wrapper) and is safe in that context.

---

### Kernel configuration requirement

`CONFIG_SECCOMP=y` and `CONFIG_SECCOMP_FILTER=y` must be set in `base/kernel.config`:

```
CONFIG_SECCOMP=y
CONFIG_SECCOMP_FILTER=y
```

These are already enabled in most x86_64 `defconfig` kernels, but add them explicitly to avoid accidental stripping by the minimal config from Phase 02.

---

### Extending the allowlist

If Wasmtime gains new syscall requirements (e.g. after an upgrade), the child will exit with SIGSYS and the supervisor will log `exit=-1`. Run `strace -f wasmtime run <app>.wasm` to discover the missing syscall and add its `libc::SYS_*` constant to `ALLOWED_SYSCALLS`.

## Notes

- `pre_exec` is marked `unsafe` because the closure runs in the child after `fork()`, where the process is single-threaded and async-signal-safe rules apply. The seccomp installation only calls `prctl(2)`, which is async-signal-safe, so this is correct.
- `SeccompAction::KillProcess` (rather than `KillThread`) terminates the entire process on violation, not just the offending thread. This is stronger and avoids partial execution in a multi-threaded Wasmtime.
- The `libc` crate is already an indirect dependency of `nix` (used in P08T03); it does not need to be added separately.
- The allowlist includes `SYS_prctl` so that Wasmtime itself can install an inner seccomp filter (some versions do this). If that is undesirable, remove it and test carefully.
- `seccompiler = "0.4"` requires Rust 1.65+. The crate is maintained by the Firecracker VMM team and is production-hardened.
- Do NOT apply seccomp to the supervisor process itself — only to the child processes. The supervisor needs `fork`, `clone`, `wait4`, `mount`, and other syscalls that are not in the Wasmtime allowlist.
- Cross-reference: P08T03 adds PID and mount namespace isolation. Seccomp is applied after the namespace clone, so the two mechanisms layer cleanly.

## Verification

```sh
# 1. Confirm seccomp.rs was created
test -f supervisor/src/seccomp.rs && echo "seccomp.rs: present"

# 2. Confirm seccompiler is in Cargo.toml
grep -q "seccompiler" supervisor/Cargo.toml && echo "seccompiler dep: present"

# 3. Build supervisor with seccomp support
(cd supervisor && cargo build 2>&1 | tail -10)
test -x supervisor/target/debug/supervisor && echo "supervisor: built OK"

# 4. Confirm apply_seccomp_filter is referenced in main.rs
grep -q "apply_seccomp_filter" supervisor/src/main.rs && echo "seccomp applied in run_app: OK"
grep -q "pre_exec"             supervisor/src/main.rs && echo "pre_exec hook: present"

# 5. Confirm kernel config has seccomp options
grep -q "^CONFIG_SECCOMP=y"        base/kernel.config && echo "CONFIG_SECCOMP: set"
grep -q "^CONFIG_SECCOMP_FILTER=y" base/kernel.config && echo "CONFIG_SECCOMP_FILTER: set"

# 6. Unit test the ALLOWED_SYSCALLS list contains the required entries
grep -q "SYS_read"        supervisor/src/seccomp.rs && echo "SYS_read: present"
grep -q "SYS_write"       supervisor/src/seccomp.rs && echo "SYS_write: present"
grep -q "SYS_mmap"        supervisor/src/seccomp.rs && echo "SYS_mmap: present"
grep -q "SYS_exit_group"  supervisor/src/seccomp.rs && echo "SYS_exit_group: present"
grep -q "SYS_futex"       supervisor/src/seccomp.rs && echo "SYS_futex: present"
grep -q "SYS_sigaltstack" supervisor/src/seccomp.rs && echo "SYS_sigaltstack: present"

# 7. Functional test: run hello-world under the supervisor and verify it exits 0
# (requires a running Linux host or the VyomaOS QEMU VM)
# On Linux host (supervisor itself does not need QEMU):
test -f apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm && \
  supervisor/target/debug/supervisor 2>/tmp/sup_log.txt; echo "exit: $?"

# 8. Verify that a WASM app attempting a blocked syscall is killed
# (manual test — requires a custom WASM binary that calls SYS_fork)
# Expected: supervisor logs exit=-1 or similar for the offending app
```
