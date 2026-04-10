//! VyomaOS supervisor — PID 1
//!
//! Responsibilities:
//!   P03T01 — scaffold: print banner
//!   P03T02 — mount /proc, /sys, /dev
//!   P05T03 — read /etc/vyoma/boot.toml and launch apps via capability manifests
//!   P06T01 — concurrent scheduler: one thread per app, per-app restart policy
//!   P07T01 — IPC broker: route @<app>: <msg> lines between app stdio pipes
//!   P08T01 — security: seccomp BPF denylist applied to every wasmtime child
//!   P08T02 — security: capability audit log + manifest unknown-field rejection
//!   P09T01 — display: open /dev/fb0, mmap framebuffer; dispatch VYOMA_DRAW: commands
//!   P12T01 — input thread: route /dev/tty0 keypresses to focused app stdin
//!   P12T02 — focus manager: shell capability, focused_app state
//!   P12T03 — @supervisor: IPC command handler (list, status, focus, run)

#[cfg(target_os = "linux")]
mod display;
mod font8x16;

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

// deny_unknown_fields ensures manifests cannot declare undocumented capabilities.
// Any unknown key is a hard parse error — the app is rejected at boot.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    #[serde(default)]
    stdio: bool,
    #[serde(default)]
    filesystem: bool,
    #[serde(default)]
    network: bool,
    /// TCP port to expose when network = true. Defaults to 8080.
    #[serde(default)]
    network_port: Option<u16>,
    /// Gates VYOMA_DRAW: display protocol — supervisor routes commands to /dev/fb0.
    #[serde(default)]
    display: bool,
    /// Receives keyboard input from /dev/tty0 by default at boot.
    #[serde(default)]
    shell: bool,
}

// ── P08T01: seccomp BPF denylist ──────────────────────────────────────────────
//
// Applied to each wasmtime child process via Command::pre_exec (runs in the
// forked child after fork, before execve).
//
// Strategy: denylist — block the small set of syscalls that should never be
// callable from sandboxed WASM runtimes.  Allowlists are stronger but fragile;
// wasmtime's syscall surface is wide and version-dependent.  The denylist
// provides a meaningful second-layer defence without risking false kills.
//
// If CONFIG_SECCOMP / CONFIG_SECCOMP_FILTER are absent from the running kernel
// the prctl call returns EINVAL; we silently skip and continue booting.
#[cfg(target_os = "linux")]
mod seccomp {
    // Classic BPF filter instruction (same layout as struct sock_filter)
    #[repr(C)]
    pub struct SockFilter {
        pub code: u16,
        pub jt: u8,
        pub jf: u8,
        pub k: u32,
    }

    // BPF program descriptor passed to prctl
    #[repr(C)]
    pub struct SockFprog {
        pub len: u16,
        pub filter: *const SockFilter,
    }

    // BPF instruction codes
    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;  // 32-bit word load
    const BPF_ABS: u16 = 0x20; // absolute offset into seccomp_data
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10; // jump if equal
    const BPF_K: u16 = 0x00;   // immediate constant
    const BPF_RET: u16 = 0x06;

    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

    // Byte offsets inside struct seccomp_data
    const OFF_NR: u32 = 0;   // int nr (syscall number)
    const OFF_ARCH: u32 = 4; // __u32 arch

    // x86_64 architecture token (AUDIT_ARCH_X86_64)
    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

    // Syscalls denied in wasmtime children (x86_64 numbers)
    const DENIED: &[u32] = &[
        101, // ptrace        — inspect/modify other processes
        169, // reboot        — only the supervisor may shut down
        246, // kexec_load    — replace the running kernel
        248, // add_key       — add entries to the kernel keyring
        249, // request_key   — search the kernel keyring
        250, // keyctl        — kernel key-management operations
        272, // unshare       — namespace creation (privilege escalation path)
        317, // seccomp       — prevent sandbox from weakening its own filter
    ];

    macro_rules! stmt {
        ($code:expr, $k:expr) => {
            SockFilter { code: $code, jt: 0, jf: 0, k: $k }
        };
    }
    macro_rules! jump {
        ($code:expr, $k:expr, $jt:expr, $jf:expr) => {
            SockFilter { code: $code, jt: $jt, jf: $jf, k: $k }
        };
    }

    /// Build the BPF filter program.  Returned Vec is heap-allocated in the
    /// parent before fork and captured by the pre_exec closure, so its pointer
    /// is valid in the child's copied address space.
    pub fn build() -> Vec<SockFilter> {
        let mut f = vec![
            // ── Arch guard: kill immediately if not x86_64 ───────────────
            stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_ARCH),
            jump!(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_X86_64, 1, 0),
            stmt!(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
            // ── Load syscall number ───────────────────────────────────────
            stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_NR),
        ];

        // ── Denylist entries ──────────────────────────────────────────────
        // Pattern: if nr == DENIED[i], fall through to KILL; else skip KILL.
        for &nr in DENIED {
            f.push(jump!(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1));
            f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));
        }

        // ── Default: allow ────────────────────────────────────────────────
        f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
        f
    }

    /// Install the BPF denylist on the calling process.
    ///
    /// Called inside Command::pre_exec (after fork, before exec).
    /// Returns Ok(()) if seccomp was applied or if the kernel lacks support
    /// (EINVAL) — the child simply continues without filtering in that case.
    ///
    /// # Safety
    /// Must be called in an async-signal-safe context.
    pub unsafe fn apply(filter: &[SockFilter]) -> std::io::Result<()> {
        // PR_SET_NO_NEW_PRIVS: prevent privilege re-escalation via exec.
        // Required to install a seccomp filter without CAP_SYS_ADMIN.
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EINVAL) {
                return Err(e);
            }
        }

        let prog = SockFprog {
            len: filter.len() as u16,
            filter: filter.as_ptr(),
        };

        let ret = libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER as libc::c_ulong,
            &prog as *const SockFprog as *const libc::c_void,
            0,
            0,
        );

        if ret < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINVAL) {
                // Kernel built without CONFIG_SECCOMP_FILTER — skip silently.
                return Ok(());
            }
            return Err(e);
        }

        Ok(())
    }
}

// ── IPC inbox map + focus state ───────────────────────────────────────────────

type Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
type FocusedApp = Arc<Mutex<Option<String>>>;

// ── Spawned app descriptor ────────────────────────────────────────────────────

struct SpawnedApp {
    entry: BootEntry,
    name: String,
    child: Child,
    msg_rx: mpsc::Receiver<String>,
    child_stdin: ChildStdin,
    child_stdout: ChildStdout,
    has_display: bool,
    is_shell: bool,
}

// ── Main ──────────────────────────────────────────────────────────────────────

const BOOT_CONFIG_PATH: &str = "/etc/vyoma/boot.toml";

fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();
    eprintln!("vyoma-supervisor: filesystems mounted");

    // Try to open /dev/fb0 (non-fatal — headless boots proceed without GUI).
    #[cfg(target_os = "linux")]
    if display::init() {
        eprintln!("vyoma-supervisor: display ready");
    }

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
        loop { thread::park(); }
    }

    let inbox: Inbox = Arc::new(Mutex::new(HashMap::new()));
    let focused: FocusedApp = Arc::new(Mutex::new(None));

    // ── Pass 1: spawn all processes and register every inbox entry ────────────
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

    // ── Set default keyboard focus to the first app with shell = true ─────────
    {
        let shell_name = spawned.iter().find(|a| a.is_shell).map(|a| a.name.clone());
        if let Some(ref name) = shell_name {
            eprintln!("vyoma-supervisor: keyboard focus → {name}");
        }
        *focused.lock().unwrap() = shell_name;
    }

    // ── P12T01: input-router thread — reads /dev/tty0, forwards to focused app ─
    #[cfg(target_os = "linux")]
    {
        let inbox_input = Arc::clone(&inbox);
        let focused_input = Arc::clone(&focused);
        thread::Builder::new()
            .name("input-router".into())
            .spawn(move || {
                let tty = match std::fs::File::open("/dev/tty0") {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("vyoma-supervisor: cannot open /dev/tty0: {e}");
                        return;
                    }
                };
                for line in BufReader::new(tty).lines() {
                    let line = match line { Ok(l) => l, Err(_) => break };
                    let target = focused_input.lock().unwrap().clone();
                    if let Some(name) = target {
                        let map = inbox_input.lock().unwrap();
                        if let Some(tx) = map.get(&name) {
                            let _ = tx.send(line);
                        }
                    }
                }
            })
            .expect("spawn input-router");
    }

    // ── Pass 2: start IO threads for each spawned app ─────────────────────────
    let mut waiter_handles = vec![];
    for app in spawned {
        let wh = launch_app_threads(app, &inbox, &focused);
        waiter_handles.push(wh);
    }

    // Drop main thread's inbox reference so writer threads can clean up.
    drop(inbox);
    drop(focused);

    for handle in waiter_handles {
        let _ = handle.join();
    }

    eprintln!("vyoma-supervisor: all apps completed, idling");
    loop { thread::park(); }
}

// ── Launch writer/reader/waiter threads for one app ──────────────────────────

fn launch_app_threads(
    app: SpawnedApp,
    inbox: &Inbox,
    focused: &FocusedApp,
) -> thread::JoinHandle<()> {
    let SpawnedApp { entry, name, child, msg_rx, child_stdin, child_stdout, has_display, is_shell: _ } = app;

    // Writer thread: msg_rx → child stdin
    thread::Builder::new()
        .name(format!("{name}-writer"))
        .spawn(move || {
            let mut stdin = child_stdin;
            while let Ok(msg) = msg_rx.recv() {
                if writeln!(stdin, "{msg}").is_err() {
                    break;
                }
            }
        })
        .expect("spawn writer thread");

    // Reader thread: child stdout → route_or_print
    let inbox_r  = Arc::clone(inbox);
    let focused_r = Arc::clone(focused);
    let name_r   = name.clone();
    thread::Builder::new()
        .name(format!("{name}-reader"))
        .spawn(move || {
            for line in BufReader::new(child_stdout).lines() {
                let line = match line { Ok(l) => l, Err(_) => break };
                route_or_print(&line, &name_r, &inbox_r, has_display, &focused_r);
            }
        })
        .expect("spawn reader thread");

    // Waiter thread: wait for child exit, respect restart policy
    thread::Builder::new()
        .name(format!("{name}-waiter"))
        .spawn(move || wait_app(entry, name, child))
        .expect("spawn waiter thread")
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
            // deny_unknown_fields makes unknown capability keys a hard error here.
            eprintln!("vyoma-supervisor: WARN: rejected manifest {}: {e}", entry.manifest);
            return None;
        }
    };

    let name = manifest.app.name.clone();
    let caps = &manifest.capabilities;

    // ── P08T02: capability audit log ─────────────────────────────────────────
    let net_port = caps.network_port.unwrap_or(8080);
    eprintln!(
        "vyoma-supervisor: [security] {name} capabilities — \
         stdio:{} fs:{} net:{} display:{} shell:{} seccomp:denylist",
        if caps.stdio { "yes" } else { "no" },
        if caps.filesystem { "yes" } else { "no" },
        if caps.network { format!("yes(port={net_port})") } else { "no".to_string() },
        if caps.display { "yes" } else { "no" },
        if caps.shell { "yes" } else { "no" },
    );

    // Register inbox before spawning so routing is ready immediately.
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

    let mut cmd = std::process::Command::new("/usr/bin/wasmtime");
    cmd.arg("run");
    if caps.filesystem {
        cmd.args(["--dir", "/data::/data"]);
    }
    if caps.network {
        cmd.args(["-S", &format!("tcplisten=0.0.0.0:{net_port}")]);
    }
    cmd.arg("--").arg(&wasm_path);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    // ── P08T01: apply seccomp denylist in child before exec ───────────────────
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let filter = seccomp::build();
        unsafe {
            cmd.pre_exec(move || seccomp::apply(&filter));
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("vyoma-supervisor: failed to spawn wasmtime for {name}: {e}");
            inbox.lock().unwrap().remove(&name);
            return None;
        }
    };

    let child_stdin  = child.stdin.take().expect("stdin pipe");
    let child_stdout = child.stdout.take().expect("stdout pipe");

    Some(SpawnedApp {
        entry: entry.clone(),
        name,
        child,
        msg_rx,
        child_stdin,
        child_stdout,
        has_display: caps.display,
        is_shell: caps.shell,
    })
}

// ── IPC router + display dispatcher ──────────────────────────────────────────

fn route_or_print(line: &str, sender: &str, inbox: &Inbox, has_display: bool, focused: &FocusedApp) {
    // VYOMA_DRAW: display protocol — only honoured for display-capable apps.
    #[cfg(target_os = "linux")]
    if has_display {
        if let Some(cmd) = line.strip_prefix("VYOMA_DRAW:") {
            handle_draw_command(cmd, sender);
            return;
        }
    }
    let _ = has_display;

    // IPC routing: @<target>: <message>
    if let Some(rest) = line.strip_prefix('@') {
        if let Some((target, msg)) = rest.split_once(": ") {
            // ── P12T03: @supervisor: commands ────────────────────────────
            if target == "supervisor" {
                handle_supervisor_command(msg, sender, inbox, focused);
                return;
            }
            let map = inbox.lock().unwrap();
            if let Some(tx) = map.get(target) {
                if tx.send(msg.to_string()).is_ok() {
                    return;
                }
            }
        }
    }
    println!("[{sender}] {line}");
}

// ── P12T03: @supervisor: command handler ─────────────────────────────────────

fn handle_supervisor_command(cmd: &str, sender: &str, inbox: &Inbox, focused: &FocusedApp) {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    match parts[0] {
        "list" => {
            let names = inbox.lock().unwrap().keys().cloned().collect::<Vec<_>>().join("|");
            let reply = format!("REPLY:{names}");
            if let Some(tx) = inbox.lock().unwrap().get(sender) {
                let _ = tx.send(reply);
            }
        }
        "status" => {
            let count = inbox.lock().unwrap().len();
            let reply = format!("REPLY:{{\"running\":{count}}}");
            if let Some(tx) = inbox.lock().unwrap().get(sender) {
                let _ = tx.send(reply);
            }
        }
        "focus" => {
            if let Some(name) = parts.get(1).map(|s| s.trim()) {
                *focused.lock().unwrap() = Some(name.to_string());
                eprintln!("vyoma-supervisor: focus → {name}");
            }
        }
        "run" => {
            let path = match parts.get(1).map(|s| s.trim()) {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: run <manifest_path>", inbox);
                    return;
                }
            };
            eprintln!("vyoma-supervisor: @supervisor: run {path}");
            let entry = BootEntry { manifest: path.clone(), restart: "never".to_string() };
            match spawn_app(&entry, inbox) {
                Some(app) => {
                    let app_name = app.name.clone();
                    launch_app_threads(app, inbox, focused);
                    send_reply(sender, &format!("REPLY:launched {app_name}"), inbox);
                }
                None => {
                    send_reply(sender, &format!("REPLY:error: could not spawn {path}"), inbox);
                }
            }
        }
        "kill" => {
            // Deferred: requires tracking child PIDs separately
            send_reply(sender, "REPLY:error: kill not yet implemented", inbox);
        }
        other => {
            eprintln!("vyoma-supervisor: unknown @supervisor command from {sender}: {other}");
        }
    }
}

fn send_reply(target: &str, msg: &str, inbox: &Inbox) {
    if let Some(tx) = inbox.lock().unwrap().get(target) {
        let _ = tx.send(msg.to_string());
    }
}

// ── VYOMA_DRAW command dispatcher ─────────────────────────────────────────────

/// Parse and execute one VYOMA_DRAW command string (everything after the prefix).
#[cfg(target_os = "linux")]
fn handle_draw_command(cmd: &str, sender: &str) {
    let Some(fb_lock) = display::get() else { return };

    if cmd == "flush" {
        fb_lock.lock().unwrap().flush();
        return;
    }

    if let Some(args) = cmd.strip_prefix("fill_rect:") {
        let v: Vec<u32> = args.split(',').filter_map(|s| s.parse().ok()).collect();
        if let [x, y, w, h, rgba] = v.as_slice() {
            fb_lock.lock().unwrap().fill_rect(*x, *y, *w, *h, *rgba);
        } else {
            eprintln!("vyoma-display: [{sender}] bad fill_rect args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text:") {
        // format: x,y,rgba,text  (text may contain commas — use splitn 4)
        let parts: Vec<&str> = args.splitn(4, ',').collect();
        if let [xs, ys, cs, text] = parts.as_slice() {
            if let (Ok(x), Ok(y), Ok(rgba)) =
                (xs.parse::<u32>(), ys.parse::<u32>(), cs.parse::<u32>())
            {
                fb_lock.lock().unwrap().draw_text(x, y, text, rgba);
            } else {
                eprintln!("vyoma-display: [{sender}] bad draw_text numeric args: {args}");
            }
        } else {
            eprintln!("vyoma-display: [{sender}] bad draw_text args: {args}");
        }
        return;
    }

    eprintln!("vyoma-display: [{sender}] unknown command: {cmd}");
}

// ── App waiter ────────────────────────────────────────────────────────────────

fn wait_app(entry: BootEntry, name: String, mut child: Child) {
    loop {
        let status = match child.wait() {
            Ok(s) if s.success() => {
                eprintln!("vyoma-supervisor: {name} exited cleanly");
                Some(s)
            }
            Ok(s) => {
                let code = s.code()
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

        match entry.restart.as_str() {
            "always" | "on-failure" => {
                // Restart with IPC requires new pipes + inbox re-registration.
                // Not implemented — treat as never.
                eprintln!(
                    "vyoma-supervisor: [WARN] restart={} not supported for IPC apps ({name}), \
                     treating as never",
                    entry.restart
                );
                let _ = status;
                break;
            }
            _ => break,
        }
    }
}

// ── P03T02: mount virtual filesystems ────────────────────────────────────────

fn mount_filesystems() {
    mount_fs("proc",     "/proc", "proc",     0);
    mount_fs("sysfs",    "/sys",  "sysfs",    0);
    mount_fs("devtmpfs", "/dev",  "devtmpfs", 0);
    // Persistent /data via virtio-9P host share (mount_tag=vyoma-data).
    // Falls back to tmpfs for diskless/development boots without -virtfs.
    #[cfg(target_os = "linux")]
    {
        let opts = c"trans=virtio,version=9p2000.L";
        let ret = mount_fs_with_data("vyoma-data", "/data", "9p", 0, opts.as_ptr());
        if !ret {
            eprintln!("vyoma-supervisor: 9P share not available — /data will be empty tmpfs");
            mount_fs("tmpfs", "/data", "tmpfs", 0);
        }
    }
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

/// Like mount_fs but passes a data string to the filesystem (e.g. 9P options).
/// Returns true on success, false if the mount fails (e.g. no 9P share attached).
#[cfg(target_os = "linux")]
fn mount_fs_with_data(
    source: &str,
    target: &str,
    fstype: &str,
    flags: libc::c_ulong,
    data: *const libc::c_char,
) -> bool {
    let c_source = CString::new(source).expect("source NUL");
    let c_target = CString::new(target).expect("target NUL");
    let c_fstype = CString::new(fstype).expect("fstype NUL");

    let ret = unsafe {
        libc::mount(
            c_source.as_ptr(),
            c_target.as_ptr(),
            c_fstype.as_ptr(),
            flags,
            data as *const libc::c_void,
        )
    };

    if ret == 0 {
        eprintln!("vyoma-supervisor: mounted {target} ({fstype})");
        true
    } else {
        let err = std::io::Error::last_os_error();
        eprintln!("vyoma-supervisor: mount({source} -> {target}, {fstype}) failed: {err}");
        false
    }
}
