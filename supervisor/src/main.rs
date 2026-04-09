//! VyomaOS supervisor — PID 1
//!
//! Responsibilities:
//!   P03T01 — scaffold: print banner
//!   P03T02 — mount /proc, /sys, /dev
//!   P05T03 — read /etc/vyoma/boot.toml and launch apps via capability manifests

#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::{fs, path::Path};

use serde::Deserialize;

// ── Boot config structs ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BootConfig {
    apps: Vec<BootEntry>,
}

#[derive(Debug, Deserialize)]
struct BootEntry {
    manifest: String,
    #[serde(default = "default_restart")]
    restart: String,
}

fn default_restart() -> String {
    "never".to_string()
}

// ── App manifest structs ──────────────────────────────────────────────────────

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
    #[serde(default)]
    stdio: bool,
    #[serde(default)]
    filesystem: bool,
    #[serde(default)]
    network: bool,
}

// ── Main ──────────────────────────────────────────────────────────────────────

const BOOT_CONFIG_PATH: &str = "/etc/vyoma/boot.toml";

fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();
    eprintln!("vyoma-supervisor: filesystems mounted");

    let boot_raw = match fs::read_to_string(BOOT_CONFIG_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vyoma-supervisor: FATAL: cannot read {BOOT_CONFIG_PATH}: {e}");
            std::process::exit(1);
        }
    };

    let boot: BootConfig = match toml::from_str(&boot_raw) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("vyoma-supervisor: FATAL: malformed {BOOT_CONFIG_PATH}: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("vyoma-supervisor: {} app(s) in boot config", boot.apps.len());

    if boot.apps.is_empty() {
        eprintln!("vyoma-supervisor: no apps configured, idling");
        loop {
            std::thread::park();
        }
    }

    // Sequential run loop — run each app; respect restart policy.
    // Phase 06 replaces this with a concurrent scheduler.
    let mut completed = vec![false; boot.apps.len()];
    loop {
        let mut any_running = false;
        for (i, entry) in boot.apps.iter().enumerate() {
            if completed[i] {
                continue;
            }
            launch_app(entry);
            if entry.restart == "never" {
                completed[i] = true;
            } else {
                any_running = true;
            }
        }
        if !any_running && completed.iter().all(|&d| d) {
            eprintln!("vyoma-supervisor: all apps completed, idling");
            loop {
                std::thread::park();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

// ── App launch ────────────────────────────────────────────────────────────────

fn launch_app(entry: &BootEntry) {
    let manifest_raw = match fs::read_to_string(&entry.manifest) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vyoma-supervisor: WARN: cannot read manifest {}: {e}", entry.manifest);
            return;
        }
    };

    let manifest: AppManifest = match toml::from_str(&manifest_raw) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("vyoma-supervisor: WARN: malformed manifest {}: {e}", entry.manifest);
            return;
        }
    };

    // Resolve wasm path relative to the manifest directory.
    let manifest_dir = Path::new(&entry.manifest)
        .parent()
        .unwrap_or(Path::new("/apps"));
    let wasm_path = manifest_dir.join(&manifest.app.wasm);

    eprintln!(
        "vyoma-supervisor: launching {} v{} (restart={})",
        manifest.app.name, manifest.app.version, entry.restart
    );

    // Wasmtime 43: stdio is inherited by default; no --inherit-stdio flag.
    // WASI capabilities are passed via -S <option>=<value>.
    let mut cmd = std::process::Command::new("/usr/bin/wasmtime");
    cmd.arg("run");

    if manifest.capabilities.filesystem {
        cmd.args(["--dir", "/data::/data"]);
    }
    if manifest.capabilities.network {
        cmd.args(["-S", "tcplisten=0.0.0.0:8080"]);
    }

    cmd.arg("--");
    cmd.arg(&wasm_path);

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    match cmd.status() {
        Ok(s) if s.success() => {
            eprintln!("vyoma-supervisor: {} exited cleanly", manifest.app.name);
        }
        Ok(s) => {
            let code = s.code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string());
            eprintln!("vyoma-supervisor: {} exited with code {code}", manifest.app.name);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "vyoma-supervisor: wasmtime not found at /usr/bin/wasmtime — \
                 rebuild the initramfs with `make rootfs`"
            );
        }
        Err(e) => {
            eprintln!("vyoma-supervisor: failed to launch wasmtime for {}: {e}", manifest.app.name);
        }
    }
}

// ── P03T02: mount virtual filesystems ────────────────────────────────────────

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
