//! VyomaOS supervisor — PID 1
//!
//! Responsibilities:
//!   P03T01 — scaffold: print banner
//!   P03T02 — mount /proc, /sys, /dev
//!   P03T03 — discover WASM apps under /apps/
//!   P03T04 — exec each app via wasmtime, sequential run loop

#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::path::{Path, PathBuf};

fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();
    eprintln!("vyoma-supervisor: filesystems mounted");

    let apps = discover_apps();

    if apps.is_empty() {
        eprintln!("vyoma-supervisor: no apps to run, idling");
        loop {
            std::thread::park();
        }
    }

    // Sequential run loop: run each app in filename order, then repeat.
    // Phase 06 replaces this with a concurrent multi-app scheduler.
    loop {
        for app in &apps {
            run_app(app);
        }
        // Brief pause between cycles to avoid tight spin when all apps
        // exit immediately (e.g. during development / stub apps).
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

// ── P03T02: mount virtual filesystems ────────────────────────────────────────

/// Mount /proc, /sys, and /dev before any userspace work proceeds.
fn mount_filesystems() {
    mount_fs("proc",     "/proc", "proc",     0);
    mount_fs("sysfs",    "/sys",  "sysfs",    0);
    mount_fs("devtmpfs", "/dev",  "devtmpfs", 0);
}

/// Call libc::mount and handle EBUSY gracefully.
///
/// EBUSY means the filesystem is already mounted — this happens for /dev
/// when CONFIG_DEVTMPFS_MOUNT=y causes the kernel to auto-mount devtmpfs
/// before PID 1 starts. Treat it as success.
fn mount_fs(_source: &str, target: &str, fstype: &str, _flags: libc::c_ulong) {
    #[cfg(target_os = "linux")]
    {
        let source = _source;
        let c_source = CString::new(source).expect("mount source contains NUL");
        let c_target = CString::new(target).expect("mount target contains NUL");
        let c_fstype = CString::new(fstype).expect("mount fstype contains NUL");

        // SAFETY: all pointers are valid CStrings for the duration of the call.
        let ret = unsafe {
            libc::mount(
                c_source.as_ptr(),
                c_target.as_ptr(),
                c_fstype.as_ptr(),
                _flags,
                std::ptr::null(),
            )
        };

        if ret != 0 {
            let err = std::io::Error::last_os_error();
            let errno = err.raw_os_error().unwrap_or(0);
            if errno == libc::EBUSY {
                eprintln!("vyoma-supervisor: {target} already mounted, skipping");
                return;
            }
            panic!("mount({source} -> {target}, {fstype}) failed: {err}");
        }

        eprintln!("vyoma-supervisor: mounted {target} ({fstype})");
    }

    #[cfg(not(target_os = "linux"))]
    eprintln!("vyoma-supervisor: [dev build] skipping mount {target} ({fstype})");
}

// ── P03T03: app discovery ─────────────────────────────────────────────────────

/// Scan /apps/ for .wasm modules and return sorted paths.
///
/// Returns an empty Vec (not an error) when /apps/ is missing or empty.
/// Sorting by filename gives deterministic boot order across restarts.
fn discover_apps() -> Vec<PathBuf> {
    const APPS_DIR: &str = "/apps";

    let dir = match std::fs::read_dir(APPS_DIR) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("vyoma-supervisor: warning: cannot read {APPS_DIR}: {e}");
            return Vec::new();
        }
    };

    let mut apps: Vec<PathBuf> = dir
        .filter_map(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("vyoma-supervisor: warning: read_dir entry error: {e}");
                    return None;
                }
            };
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "wasm") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    // Sort by filename for stable ordering.
    apps.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    for app in &apps {
        eprintln!("vyoma-supervisor: discovered app: {}", app.display());
    }

    if apps.is_empty() {
        eprintln!("vyoma-supervisor: no .wasm apps found in {APPS_DIR}");
    } else {
        eprintln!("vyoma-supervisor: {} app(s) discovered", apps.len());
    }

    apps
}

// ── P03T04: process lifecycle ─────────────────────────────────────────────────

/// Run a single WASM app via wasmtime and block until it exits.
///
/// Stdio is inherited so app output appears on ttyS0 directly.
/// If wasmtime is not present, logs a warning and returns (no panic —
/// the supervisor must stay alive as PID 1).
fn run_app(path: &Path) {
    let display = path.display();
    eprintln!("vyoma-supervisor: starting app: {display}");

    let status = std::process::Command::new("/usr/bin/wasmtime")
        .args([
            "run",
            "--",
            path.to_str().expect("app path must be valid UTF-8"),
        ])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    match status {
        Ok(s) if s.success() => {
            eprintln!("vyoma-supervisor: app exited cleanly: {display}");
        }
        Ok(s) => {
            let code = s.code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string());
            eprintln!("vyoma-supervisor: app exited with code {code}: {display}");
        }
        Err(e) => {
            eprintln!("vyoma-supervisor: failed to launch wasmtime for {display}: {e}");
        }
    }
}
