//! VyomaOS supervisor — PID 1
//!
//! Responsibilities:
//!   P03T01 — scaffold: print banner
//!   P03T02 — mount /proc, /sys, /dev
//!   P05T03 — read /etc/vyoma/boot.toml and launch apps via capability manifests
//!   P06T01 — concurrent scheduler: one thread per app, per-app restart policy
//!   P07T01 — IPC broker: route @<app>: <msg> lines between app stdio pipes

#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
};

use serde::Deserialize;

// ── Boot config structs ───────────────────────────────────────────────────────

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
    #[allow(dead_code)] // declared in manifests for documentation; supervisor always pipes stdio
    stdio: bool,
    #[serde(default)]
    filesystem: bool,
    #[serde(default)]
    network: bool,
}

// ── IPC inbox map ─────────────────────────────────────────────────────────────
//
// Maps app name → sender half of its message channel.
// Reader threads use this to route "@<name>: <msg>" lines.
// Dropping the Arc (after all reader threads exit) closes all channels,
// which in turn terminates the writer threads cleanly.

type Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;

// ── Spawned app descriptor ────────────────────────────────────────────────────

struct SpawnedApp {
    entry: BootEntry,
    name: String,
    child: Child,
    msg_rx: mpsc::Receiver<String>,
    child_stdin: ChildStdin,
    child_stdout: ChildStdout,
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
            thread::park();
        }
    }

    let inbox: Inbox = Arc::new(Mutex::new(HashMap::new()));

    // ── Pass 1: spawn all processes and register every inbox entry ────────────
    // All registrations happen before any IO threads start, so no @-routed
    // message can arrive before the target's sender is in the map.
    let mut spawned: Vec<SpawnedApp> = Vec::new();
    for entry in boot.apps {
        match spawn_app(&entry, &inbox) {
            Some(app) => spawned.push(app),
            None => eprintln!(
                "vyoma-supervisor: WARN: failed to spawn {}, skipping",
                entry.manifest
            ),
        }
    }

    // ── Pass 2: start IO threads and waiter threads ───────────────────────────
    let mut waiter_handles = vec![];

    for app in spawned {
        let SpawnedApp {
            entry,
            name,
            child,
            msg_rx,
            child_stdin,
            child_stdout,
        } = app;

        // Writer thread — drains the message channel into the app's stdin pipe.
        thread::Builder::new()
            .name(format!("{name}-writer"))
            .spawn(move || {
                let mut stdin = child_stdin;
                while let Ok(msg) = msg_rx.recv() {
                    if writeln!(stdin, "{msg}").is_err() {
                        break; // broken pipe: app exited
                    }
                }
            })
            .expect("spawn writer thread");

        // Reader thread — reads app stdout; routes @<target>: lines, prints rest.
        let inbox_r = Arc::clone(&inbox);
        let name_r = name.clone();
        thread::Builder::new()
            .name(format!("{name}-reader"))
            .spawn(move || {
                for line in BufReader::new(child_stdout).lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    route_or_print(&line, &name_r, &inbox_r);
                }
            })
            .expect("spawn reader thread");

        // Waiter thread — waits for the child process; respawns per policy.
        waiter_handles.push(
            thread::Builder::new()
                .name(format!("{name}-waiter"))
                .spawn(move || wait_app(entry, name, child))
                .expect("spawn waiter thread"),
        );
    }

    // Drop the main thread's reference to the inbox.
    // When all reader threads also exit (apps' stdout pipes close),
    // the inbox is deallocated, all tx values drop, and writer threads
    // terminate via RecvError on their msg_rx channels.
    drop(inbox);

    for handle in waiter_handles {
        let _ = handle.join();
    }

    eprintln!("vyoma-supervisor: all apps completed, idling");
    loop {
        thread::park();
    }
}

// ── Spawn one app ─────────────────────────────────────────────────────────────

fn spawn_app(entry: &BootEntry, inbox: &Inbox) -> Option<SpawnedApp> {
    let manifest_raw = match fs::read_to_string(&entry.manifest) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vyoma-supervisor: WARN: cannot read manifest {}: {e}", entry.manifest);
            return None;
        }
    };
    let manifest: AppManifest = match toml::from_str(&manifest_raw) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("vyoma-supervisor: WARN: malformed manifest {}: {e}", entry.manifest);
            return None;
        }
    };

    let name = manifest.app.name.clone();

    // Register inbox entry before spawning so other apps can route to this one
    // as soon as their reader threads start.
    let (tx, msg_rx) = mpsc::channel::<String>();
    inbox.lock().unwrap().insert(name.clone(), tx);

    let wasm_path = Path::new(&entry.manifest)
        .parent()
        .unwrap_or(Path::new("/apps"))
        .join(&manifest.app.wasm);

    eprintln!(
        "vyoma-supervisor: spawning {} v{} (restart={})",
        name, manifest.app.version, entry.restart
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
    cmd.arg("--").arg(&wasm_path);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("vyoma-supervisor: failed to spawn wasmtime for {name}: {e}");
            inbox.lock().unwrap().remove(&name);
            return None;
        }
    };

    let child_stdin = child.stdin.take().expect("stdin pipe");
    let child_stdout = child.stdout.take().expect("stdout pipe");

    Some(SpawnedApp {
        entry: entry.clone(),
        name,
        child,
        msg_rx,
        child_stdin,
        child_stdout,
    })
}

// ── IPC router ────────────────────────────────────────────────────────────────

fn route_or_print(line: &str, sender: &str, inbox: &Inbox) {
    // Format: @<target>: <message>
    if let Some(rest) = line.strip_prefix('@') {
        if let Some((target, msg)) = rest.split_once(": ") {
            let map = inbox.lock().unwrap();
            if let Some(tx) = map.get(target) {
                if tx.send(msg.to_string()).is_ok() {
                    return; // routed successfully — don't print
                }
            }
            // Unknown target or channel closed — fall through and print
        }
    }
    println!("[{sender}] {line}");
}

// ── App waiter: handles restart policy ───────────────────────────────────────

fn wait_app(entry: BootEntry, name: String, mut child: Child) {
    loop {
        let status = match child.wait() {
            Ok(s) if s.success() => {
                eprintln!("vyoma-supervisor: {name} exited cleanly");
                Some(s)
            }
            Ok(s) => {
                let code = s
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                eprintln!("vyoma-supervisor: {name} exited with code {code}");
                Some(s)
            }
            Err(e) => {
                eprintln!("vyoma-supervisor: wait failed for {name}: {e}");
                None
            }
        };

        // Note: restart=always/on-failure with IPC apps is not supported in P07
        // because respawning requires new stdio pipes and inbox re-registration.
        // All IPC apps should use restart=never.
        match entry.restart.as_str() {
            "always" => {
                eprintln!("vyoma-supervisor: [WARN] restart=always unsupported for IPC apps, treating as never");
                break;
            }
            "on-failure" => {
                let failed = status.map(|s| !s.success()).unwrap_or(true);
                if failed {
                    eprintln!("vyoma-supervisor: [WARN] restart=on-failure unsupported for IPC apps, treating as never");
                }
                break;
            }
            _ => break, // "never"
        }
    }
}

// ── P03T02: mount virtual filesystems ────────────────────────────────────────

fn mount_filesystems() {
    mount_fs("proc",     "/proc", "proc",     0);
    mount_fs("sysfs",    "/sys",  "sysfs",    0);
    mount_fs("devtmpfs", "/dev",  "devtmpfs", 0);
}

fn mount_fs(_source: &str, target: &str, fstype: &str, _flags: libc::c_ulong) {
    #[cfg(target_os = "linux")]
    {
        let source = _source;
        let c_source = CString::new(source).expect("mount source contains NUL");
        let c_target = CString::new(target).expect("mount target contains NUL");
        let c_fstype = CString::new(fstype).expect("mount fstype contains NUL");

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
