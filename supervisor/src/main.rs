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
//!   P17T01 — input thread: raw tty mode, per-keypress routing to focused app
//!   P12T02 — focus manager: shell capability, focused_app state
//!   P12T03 — @supervisor: IPC command handler (list, status, focus, run)
//!   P13T01 — process management: ps, kill, restart, reload, log

#[cfg(target_os = "linux")]
mod display;
mod font;

#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Stdio},
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
    time::Instant,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

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

#[derive(Debug, Default, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct WindowRegion {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Debug, Deserialize)]
struct AppManifest {
    app: AppMeta,
    capabilities: Capabilities,
    #[serde(default)]
    window: Option<WindowRegion>,  // optional [window] section in vyoma.toml
}

#[derive(Debug, Deserialize)]
struct AppMeta {
    name: String,
    version: String,
    wasm: String,
    /// P28: optional SHA-256 hex digest of the .wasm binary.
    /// If present, supervisor verifies before spawning; rejects on mismatch.
    #[serde(default)]
    wasm_sha256: Option<String>,
}

// deny_unknown_fields ensures manifests cannot declare undocumented capabilities.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    #[serde(default)]
    stdio: bool,
    #[serde(default)]
    filesystem: bool,
    #[serde(default)]
    network: bool,
    #[serde(default)]
    network_port: Option<u16>,
    #[serde(default)]
    display: bool,
    #[serde(default)]
    shell: bool,
    #[serde(default)]
    watchdog_secs: u32,  // 0 = disabled; >0 = kill app if silent for this many seconds
    #[serde(default)]
    mouse: bool,   // receives VYOMA_INPUT:mouse: events when cursor is in window
}

// ── P08T01: seccomp BPF denylist ──────────────────────────────────────────────
#[cfg(target_os = "linux")]
mod seccomp {
    #[repr(C)]
    pub struct SockFilter {
        pub code: u16,
        pub jt: u8,
        pub jf: u8,
        pub k: u32,
    }

    #[repr(C)]
    pub struct SockFprog {
        pub len: u16,
        pub filter: *const SockFilter,
    }

    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10;
    const BPF_K: u16 = 0x00;
    const BPF_RET: u16 = 0x06;

    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

    const OFF_NR: u32 = 0;
    const OFF_ARCH: u32 = 4;

    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

    const DENIED: &[u32] = &[
        101, // ptrace
        169, // reboot
        246, // kexec_load
        248, // add_key
        249, // request_key
        250, // keyctl
        272, // unshare
        317, // seccomp
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

    pub fn build() -> Vec<SockFilter> {
        let mut f = vec![
            stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_ARCH),
            jump!(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_X86_64, 1, 0),
            stmt!(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
            stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_NR),
        ];
        for &nr in DENIED {
            f.push(jump!(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1));
            f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));
        }
        f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
        f
    }

    pub unsafe fn apply(filter: &[SockFilter]) -> std::io::Result<()> {
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
                return Ok(());
            }
            return Err(e);
        }
        Ok(())
    }
}

// ── P13T01: per-app runtime state ─────────────────────────────────────────────

const LOG_BUF_SIZE: usize = 20;

#[derive(Clone, Debug)]
enum AppStatus {
    Running,
    Stopped(i32),
}

struct AppState {
    entry:            BootEntry,
    status:           AppStatus,
    start_time:       Instant,
    restart_count:    u32,
    log_buf:          VecDeque<String>,
    child_pid:        Option<u32>,
    // P19: watchdog fields
    watchdog_secs:    u32,
    last_output:      Arc<Mutex<Instant>>,
    watchdog_backoff: Arc<Mutex<u64>>,   // seconds until next restart allowed
    has_mouse:   bool,
    has_display: bool,
    win_region:  Option<(u32, u32, u32, u32)>,  // P22: (x,y,w,h) screen coords for mouse dispatch
}

type AppRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<AppState>>>>>;

// ── IPC inbox map + focus state ───────────────────────────────────────────────

type Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
type FocusedApp = Arc<Mutex<Option<String>>>;

// ── P33: Global Z-order stack (index 0 = topmost / frontmost window) ─────────

static Z_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn z_order_push_front(name: &str) {
    if let Some(m) = Z_ORDER.get() {
        let mut v = m.lock().unwrap();
        v.retain(|n| n != name);
        v.insert(0, name.to_string());
    }
}

fn z_order_push_back(name: &str) {
    if let Some(m) = Z_ORDER.get() {
        let mut v = m.lock().unwrap();
        v.retain(|n| n != name);
        v.push(name.to_string());
    }
}

// ── Spawned app descriptor ────────────────────────────────────────────────────

struct SpawnedApp {
    entry:        BootEntry,
    name:         String,
    child:        Child,
    msg_rx:       mpsc::Receiver<String>,
    child_stdin:  ChildStdin,
    child_stdout: ChildStdout,
    has_display:  bool,
    is_shell:     bool,
    win_region:   Option<(u32, u32, u32, u32)>,  // P21: (x, y, w, h) in screen coords
}

// ── Main ──────────────────────────────────────────────────────────────────────

const BOOT_CONFIG_PATH:  &str = "/etc/vyoma/boot.toml";
const USER_BOOT_PATH:    &str = "/data/installed.txt";
const DATA_APPS_DIR:     &str = "/data/apps";
const LOG_DIR:           &str = "/data/logs";
const LOG_TAIL_LINES:    usize = 30;

/// Built-in package catalog — apps shipped in the initramfs, available to install.
/// Tuple: (name, version, description)
const PACKAGES: &[(&str, &str, &str)] = &[
    ("calculator",   "0.1.0", "Basic arithmetic calculator"),
    ("factorial",    "0.1.0", "Recursive factorial computation"),
    ("gui-demo",     "0.1.0", "Framebuffer dashboard demo"),
    ("hello-world",  "0.1.0", "Hello World demo app"),
    ("http-server",  "0.1.0", "HTTP status server on :8080"),
    ("ping",         "0.1.0", "IPC demo — send side (pair with pong)"),
    ("pong",         "0.1.0", "IPC demo — recv side (pair with ping)"),
    ("storage-demo", "0.1.0", "Persistent storage demo"),
];

// ── P19: watchdog backoff helper ─────────────────────────────────────────────

fn watchdog_next_backoff(watchdog_secs: u32, restarts: u32) -> u64 {
    let base = watchdog_secs as u64;
    let factor = 1u64 << restarts.min(8);
    (base * factor).min(300)
}

fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();
    eprintln!("vyoma-supervisor: filesystems mounted");

    #[cfg(target_os = "linux")]
    if display::init() {
        eprintln!("vyoma-supervisor: display ready");
        // P35: paint default desktop background before any app draws
        if let Some(fb_lock) = display::get() {
            let mut fb = fb_lock.lock().unwrap();
            let (w, h) = (fb.width, fb.height);
            fb.fill_rect(0, 0, w, h, 0x0D1117FF);
            fb.flush();
        }
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

    // ── Merge user-installed apps from /data/installed.txt ───────────────────
    let mut all_entries = boot.apps;
    if let Ok(raw) = fs::read_to_string(USER_BOOT_PATH) {
        let mut user_count = 0u32;
        for name in raw.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let manifest = format!("{DATA_APPS_DIR}/{name}/vyoma.toml");
            if Path::new(&manifest).exists()
                && !all_entries.iter().any(|e| e.manifest == manifest)
            {
                all_entries.push(BootEntry { manifest, restart: "never".to_string() });
                user_count += 1;
            } else if !Path::new(&manifest).exists() {
                eprintln!("vyoma-supervisor: installed app {name} missing manifest, skipping");
            }
        }
        if user_count > 0 {
            eprintln!("vyoma-supervisor: {user_count} user-installed app(s) added");
        }
    }

    eprintln!("vyoma-supervisor: {} app(s) total", all_entries.len());

    if all_entries.is_empty() {
        eprintln!("vyoma-supervisor: no apps configured, idling");
        loop { thread::park(); }
    }

    let _ = Z_ORDER.set(Mutex::new(Vec::new()));

    let inbox:        Inbox       = Arc::new(Mutex::new(HashMap::new()));
    let focused:      FocusedApp  = Arc::new(Mutex::new(None));
    let app_registry: AppRegistry = Arc::new(Mutex::new(HashMap::new()));

    // ── Pass 1: spawn all processes and register inbox entries ────────────────
    let mut spawned: Vec<SpawnedApp> = Vec::new();
    for entry in all_entries {
        match spawn_app(&entry, &inbox, &app_registry) {
            Some(app) => spawned.push(app),
            None => eprintln!(
                "vyoma-supervisor: WARN: failed to spawn {}, skipping",
                entry.manifest
            ),
        }
    }

    // ── Set default keyboard focus to the first shell app ────────────────────
    {
        let shell_name = spawned.iter().find(|a| a.is_shell).map(|a| a.name.clone());
        if let Some(ref name) = shell_name {
            eprintln!("vyoma-supervisor: keyboard focus → {name}");
        }
        *focused.lock().unwrap() = shell_name;
    }

    // ── P17T01: input-router thread — /dev/tty0 → focused app (raw mode) ─────
    // Raw mode: each keystroke is forwarded immediately as a single-char message
    // instead of waiting for a full line.  The shell accumulates characters into
    // a live input buffer and redraws on every keystroke.
    #[cfg(target_os = "linux")]
    {
        let inbox_input   = Arc::clone(&inbox);
        let focused_input = Arc::clone(&focused);
        thread::Builder::new()
            .name("input-router".into())
            .spawn(move || {
                use std::io::Read;
                use std::os::unix::io::AsRawFd;

                let mut tty = match std::fs::File::open("/dev/tty0") {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("vyoma-supervisor: cannot open /dev/tty0: {e}");
                        return;
                    }
                };

                // Switch tty0 to raw mode so every keypress arrives immediately
                // without waiting for the Enter key (no line-discipline buffering).
                let fd = tty.as_raw_fd();
                let raw_ok = unsafe {
                    let mut t: libc::termios = std::mem::zeroed();
                    if libc::tcgetattr(fd, &mut t) == 0 {
                        // Disable canonical mode, echo, and signal keys (^C / ^Z)
                        t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHOE
                                     | libc::ECHOK  | libc::ECHONL | libc::ISIG
                                     | libc::IEXTEN);
                        // Disable special input translations (CR→NL, XON/XOFF, …)
                        t.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT
                                     | libc::INPCK | libc::ISTRIP);
                        // 8-bit characters; block until 1 byte arrives, no timeout
                        t.c_cflag |= libc::CS8;
                        t.c_cc[libc::VMIN  as usize] = 1;
                        t.c_cc[libc::VTIME as usize] = 0;
                        libc::tcsetattr(fd, libc::TCSANOW, &t) == 0
                    } else {
                        false
                    }
                };
                if raw_ok {
                    eprintln!("vyoma-supervisor: input-router: raw tty mode active");
                } else {
                    eprintln!("vyoma-supervisor: input-router: raw mode unavailable, using line mode");
                }

                let mut buf = [0u8; 1];
                loop {
                    match tty.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let msg: Option<String> = match buf[0] {
                                0x0D | 0x0A => Some(String::new()),              // Enter → execute
                                0x7F | 0x08 => Some("\x7f".to_string()),         // Backspace / DEL
                                0x03        => Some("\x03".to_string()),          // Ctrl+C
                                0x1B        => {
                                    // ANSI escape sequence — read next 2 bytes, forward arrows, discard rest.
                                    let mut esc = [0u8; 2];
                                    let _ = tty.read(&mut esc);
                                    match esc {
                                        [0x5B, 0x41] => Some("\x1b[A".to_string()), // ↑ up arrow
                                        [0x5B, 0x42] => Some("\x1b[B".to_string()), // ↓ down arrow
                                        _            => None,
                                    }
                                }
                                0x20..=0x7E => Some(String::from(buf[0] as char)), // printable ASCII
                                _           => None,
                            };
                            if let Some(msg) = msg {
                                let target = focused_input.lock().unwrap().clone();
                                if let Some(name) = target {
                                    let map = inbox_input.lock().unwrap();
                                    if let Some(tx) = map.get(&name) {
                                        let _ = tx.send(msg);
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .expect("spawn input-router");
    }

    // ── P22: mouse-input thread — /dev/input/eventN → mouse-capable apps ──────
    // Reads evdev input_event structs (24 bytes each on 64-bit Linux), tracks
    // global cursor position, dispatches VYOMA_INPUT:mouse:<lx>,<ly>,<btn>
    // to the first app with mouse=true whose window contains the cursor.
    // Exits silently if no pointer device is found (headless boot).
    #[cfg(target_os = "linux")]
    {
        let inbox_m    = Arc::clone(&inbox);
        let registry_m = Arc::clone(&app_registry);
        let focused_m  = Arc::clone(&focused);
        thread::Builder::new()
            .name("mouse-input".into())
            .spawn(move || {
                use std::io::Read;

                let Some(mut dev) = open_mouse_device() else {
                    eprintln!("vyoma-supervisor: mouse-input: no pointer device found, disabling");
                    return;
                };

                const EV_SYN: u16   = 0;
                const EV_KEY: u16   = 1;
                const EV_REL: u16   = 2;
                const EV_ABS: u16   = 3;
                const REL_X: u16    = 0;
                const REL_Y: u16    = 1;
                const ABS_X: u16    = 0;
                const ABS_Y: u16    = 1;
                const BTN_LEFT: u16 = 0x110;

                const SCREEN_W: i32 = 1440;
                const SCREEN_H: i32 = 900;
                // virtio-mouse-pci reports ABS coords in range 0..=32767
                const ABS_MAX: i64  = 32768;

                let mut cx: i32 = SCREEN_W / 2;
                let mut cy: i32 = SCREEN_H / 2;
                let mut btn: u8 = 0;
                let mut pending_abs_x: Option<i32> = None;
                let mut pending_abs_y: Option<i32> = None;
                let mut pending_dx:    i32 = 0;
                let mut pending_dy:    i32 = 0;

                // Linux input_event on 64-bit:
                //   i64 tv_sec + i64 tv_usec + u16 type + u16 code + i32 value = 24 bytes
                let mut buf = [0u8; 24];
                loop {
                    if dev.read_exact(&mut buf).is_err() {
                        eprintln!("vyoma-supervisor: mouse-input: device read error, exiting");
                        break;
                    }
                    let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
                    let code    = u16::from_ne_bytes([buf[18], buf[19]]);
                    let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);

                    match ev_type {
                        EV_REL => match code {
                            REL_X => pending_dx += value,
                            REL_Y => pending_dy += value,
                            _     => {}
                        },
                        EV_ABS => match code {
                            ABS_X => pending_abs_x = Some(value),
                            ABS_Y => pending_abs_y = Some(value),
                            _     => {}
                        },
                        EV_KEY => {
                            if code == BTN_LEFT {
                                btn = if value > 0 { 1 } else { 0 };
                            }
                        }
                        EV_SYN => {
                            // Apply REL movement (regular mouse)
                            if pending_dx != 0 || pending_dy != 0 {
                                cx = (cx + pending_dx).clamp(0, SCREEN_W - 1);
                                cy = (cy + pending_dy).clamp(0, SCREEN_H - 1);
                                pending_dx = 0;
                                pending_dy = 0;
                            }
                            // Apply ABS position (virtio-mouse-pci, scaled 0..32767 → screen)
                            if let Some(ax) = pending_abs_x.take() {
                                cx = (ax as i64 * SCREEN_W as i64 / ABS_MAX) as i32;
                            }
                            if let Some(ay) = pending_abs_y.take() {
                                cy = (ay as i64 * SCREEN_H as i64 / ABS_MAX) as i32;
                            }
                            dispatch_mouse(cx, cy, btn, &inbox_m, &registry_m, &focused_m);
                        }
                        _ => {}
                    }
                }
            })
            .expect("spawn mouse-input thread");
    }

    // ── Pass 2: start IO threads for each spawned app ─────────────────────────
    let mut waiter_handles = vec![];
    for app in spawned {
        let wh = launch_app_threads(app, &inbox, &focused, &app_registry);
        waiter_handles.push(wh);
    }

    // ── P19: watchdog thread — kills apps silent longer than watchdog_secs ───
    {
        use std::time::Duration;
        let registry_wd = Arc::clone(&app_registry);
        thread::Builder::new()
            .name("watchdog".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(1));
                    let reg = registry_wd.lock().unwrap();
                    for (name, state_arc) in reg.iter() {
                        let st = state_arc.lock().unwrap();
                        let wsecs = st.watchdog_secs;
                        if wsecs == 0 { continue; }

                        // Check backoff countdown
                        {
                            let mut backoff = st.watchdog_backoff.lock().unwrap();
                            if *backoff > 0 {
                                *backoff -= 1;
                                continue;
                            }
                        }

                        // Check silence duration
                        let elapsed = st.last_output.lock().unwrap().elapsed();
                        if elapsed.as_secs() >= wsecs as u64 {
                            // Kill the child process
                            if let Some(pid) = st.child_pid {
                                eprintln!(
                                    "[watchdog] {name}: silent for {}s (limit={wsecs}s) — killing pid {pid}",
                                    elapsed.as_secs()
                                );
                                #[cfg(target_os = "linux")]
                                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                            }
                            // Set exponential backoff for next restart
                            let restarts = st.restart_count;
                            *st.watchdog_backoff.lock().unwrap() =
                                watchdog_next_backoff(wsecs, restarts);
                            // Reset timer so we don't immediately re-trigger
                            *st.last_output.lock().unwrap() = std::time::Instant::now();
                        }
                    }
                }
            })
            .expect("spawn watchdog thread");
    }

    drop(inbox);
    drop(focused);
    drop(app_registry);

    for handle in waiter_handles {
        let _ = handle.join();
    }

    eprintln!("vyoma-supervisor: all apps completed, idling");
    loop { thread::park(); }
}

// ── Spawn writer + reader threads for one app (no waiter) ────────────────────

fn spawn_io_threads(
    name: &str,
    child_stdin:  ChildStdin,
    child_stdout: ChildStdout,
    msg_rx:       mpsc::Receiver<String>,
    has_display:  bool,
    win_region:   Option<(u32, u32, u32, u32)>,  // P21
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    thread::Builder::new()
        .name(format!("{name}-writer"))
        .spawn(move || {
            let mut stdin = child_stdin;
            // When the inbox entry is replaced on restart, the sender is dropped
            // and msg_rx.recv() returns Err — writer exits cleanly.
            while let Ok(msg) = msg_rx.recv() {
                if writeln!(stdin, "{msg}").is_err() {
                    break;
                }
            }
        })
        .expect("spawn writer thread");

    let inbox_r    = Arc::clone(inbox);
    let focused_r  = Arc::clone(focused);
    let registry_r = Arc::clone(app_registry);
    let name_r     = name.to_string();
    // P19: clone last_output Arc so the reader thread can update it cheaply
    let last_output_r: Arc<Mutex<Instant>> = {
        let reg = app_registry.lock().unwrap();
        reg.get(name)
            .map(|st| Arc::clone(&st.lock().unwrap().last_output))
            .unwrap_or_else(|| Arc::new(Mutex::new(Instant::now())))
    };
    thread::Builder::new()
        .name(format!("{name}-reader"))
        .spawn(move || {
            // P19: reset last_output to "now" so the watchdog timeout starts from
            // when the reader thread is actually ready (not from the earlier spawn
            // time, which can be many seconds before the first line arrives).
            *last_output_r.lock().unwrap() = Instant::now();

            // Open persistent log file (best-effort; errors are silently ignored)
            let _ = fs::create_dir_all(LOG_DIR);
            let log_path = format!("{LOG_DIR}/{name_r}.log");
            let mut log_file = std::fs::OpenOptions::new()
                .create(true).append(true)
                .open(&log_path)
                .ok();

            for line in BufReader::new(child_stdout).lines() {
                let line = match line { Ok(l) => l, Err(_) => break };

                // P19: touch last_output on every line — resets the watchdog timer
                *last_output_r.lock().unwrap() = Instant::now();

                // Write to persistent log file
                if let Some(ref mut f) = log_file {
                    let _ = writeln!(f, "{line}");
                }

                // Append to per-app log ring buffer
                {
                    let reg = registry_r.lock().unwrap();
                    if let Some(st) = reg.get(&name_r) {
                        let mut st = st.lock().unwrap();
                        st.log_buf.push_back(line.clone());
                        if st.log_buf.len() > LOG_BUF_SIZE {
                            st.log_buf.pop_front();
                        }
                    }
                }
                route_or_print(&line, &name_r, &inbox_r, has_display, win_region, &focused_r, &registry_r);
            }
        })
        .expect("spawn reader thread");
}

// ── Launch writer/reader/waiter threads for one app ──────────────────────────

fn launch_app_threads(
    app:          SpawnedApp,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> thread::JoinHandle<()> {
    let SpawnedApp { entry, name, child, msg_rx, child_stdin, child_stdout, has_display, is_shell: _, win_region } = app;

    spawn_io_threads(&name, child_stdin, child_stdout, msg_rx, has_display, win_region, inbox, focused, app_registry);

    // P29: send actual screen resolution to display apps before their first draw
    #[cfg(target_os = "linux")]
    if has_display {
        if let Some((w, h)) = display::screen_size() {
            if let Some(tx) = inbox.lock().unwrap().get(&name) {
                let _ = tx.send(format!("VYOMA_SYSTEM:screen:{w},{h}"));
            }
        }
    }

    // P33: newly launched display apps with windows go to the front of the Z-order
    if has_display && win_region.is_some() {
        z_order_push_front(&name);
    }

    let registry_w = Arc::clone(app_registry);
    let inbox_w    = Arc::clone(inbox);
    let focused_w  = Arc::clone(focused);

    thread::Builder::new()
        .name(format!("{name}-waiter"))
        .spawn(move || wait_app(entry, name, child, registry_w, inbox_w, focused_w))
        .expect("spawn waiter thread")
}

// ── Spawn one app process and register its state ─────────────────────────────

fn spawn_app(entry: &BootEntry, inbox: &Inbox, app_registry: &AppRegistry) -> Option<SpawnedApp> {
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
            eprintln!("vyoma-supervisor: WARN: rejected manifest {}: {e}", entry.manifest);
            return None;
        }
    };

    let name = manifest.app.name.clone();
    let caps = &manifest.capabilities;

    let net_port = caps.network_port.unwrap_or(8080);
    eprintln!(
        "vyoma-supervisor: [security] {name} capabilities — \
         stdio:{} fs:{} net:{} display:{} shell:{} mouse:{} seccomp:denylist",
        if caps.stdio      { "yes" } else { "no" },
        if caps.filesystem { "yes" } else { "no" },
        if caps.network    { format!("yes(port={net_port})") } else { "no".to_string() },
        if caps.display    { "yes" } else { "no" },
        if caps.shell      { "yes" } else { "no" },
        if caps.mouse      { "yes" } else { "no" },
    );

    let (tx, msg_rx) = mpsc::channel::<String>();
    inbox.lock().unwrap().insert(name.clone(), tx);

    let wasm_path = Path::new(&entry.manifest)
        .parent()
        .unwrap_or(Path::new("/apps"))
        .join(&manifest.app.wasm);

    // P28: SHA-256 integrity check — reject binary if hash declared and mismatches
    if let Some(expected) = &manifest.app.wasm_sha256 {
        match fs::read(&wasm_path) {
            Ok(bytes) => {
                let actual = format!("{:x}", Sha256::digest(&bytes));
                if actual != expected.to_lowercase() {
                    eprintln!(
                        "vyoma-supervisor: SECURITY: {name} rejected — SHA-256 mismatch\n  expected {expected}\n  actual   {actual}"
                    );
                    inbox.lock().unwrap().remove(&name);
                    return None;
                }
                eprintln!("vyoma-supervisor: [security] {name} wasm_sha256 verified OK");
            }
            Err(e) => {
                eprintln!("vyoma-supervisor: WARN: {name} cannot read wasm for hash check: {e}");
            }
        }
    }

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
        // -S inherit-network: grants wasi:sockets access for P2 components.
        // (The legacy -S tcplisten= flag only works for P1 modules.)
        // The component binds its own port; net_port is used only for QEMU hostfwd.
        cmd.args(["-S", "inherit-network"]);
    }
    cmd.arg("--").arg(&wasm_path);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let filter = seccomp::build();
        unsafe {
            cmd.pre_exec(move || {
                // P27: isolate mount + PID namespaces per app.
                // Non-fatal: ignored if kernel lacks CONFIG_NAMESPACES/CONFIG_PID_NS/CONFIG_MNT_NS.
                let _ = libc::unshare(libc::CLONE_NEWNS | libc::CLONE_NEWPID);
                seccomp::apply(&filter)
            });
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

    let child_pid    = child.id();
    let child_stdin  = child.stdin.take().expect("stdin pipe");
    let child_stdout = child.stdout.take().expect("stdout pipe");

    // Register per-app runtime state
    let state = Arc::new(Mutex::new(AppState {
        entry:            entry.clone(),
        status:           AppStatus::Running,
        start_time:       Instant::now(),
        restart_count:    0,
        log_buf:          VecDeque::new(),
        child_pid:        Some(child_pid),
        watchdog_secs:    caps.watchdog_secs,
        last_output:      Arc::new(Mutex::new(Instant::now())),
        watchdog_backoff: Arc::new(Mutex::new(0u64)),
        has_mouse:        caps.mouse,
        has_display:      caps.display,
        win_region:       manifest.window.map(|wr| (wr.x, wr.y, wr.w, wr.h)),
    }));
    app_registry.lock().unwrap().insert(name.clone(), state);

    Some(SpawnedApp {
        entry: entry.clone(),
        name,
        child,
        msg_rx,
        child_stdin,
        child_stdout,
        has_display: caps.display,
        is_shell: caps.shell,
        win_region: manifest.window.map(|wr| (wr.x, wr.y, wr.w, wr.h)),
    })
}

// ── IPC router + display dispatcher ──────────────────────────────────────────

fn route_or_print(
    line:         &str,
    sender:       &str,
    inbox:        &Inbox,
    has_display:  bool,
    win_region:   Option<(u32, u32, u32, u32)>,  // P21
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    #[cfg(target_os = "linux")]
    if has_display {
        if let Some(cmd) = line.strip_prefix("VYOMA_DRAW:") {
            handle_draw_command(cmd, sender, win_region);
            return;
        }
    }
    let _ = has_display;
    let _ = win_region;

    if let Some(rest) = line.strip_prefix('@') {
        if let Some((target, msg)) = rest.split_once(": ") {
            if target == "supervisor" {
                handle_supervisor_command(msg, sender, inbox, focused, app_registry);
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

// ── P12T03 / P13T01: @supervisor: command handler ────────────────────────────

fn handle_supervisor_command(
    cmd:          &str,
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    match parts[0] {
        // ── P12T03 legacy commands ────────────────────────────────────────────
        "list" => {
            let names = inbox.lock().unwrap().keys().cloned().collect::<Vec<_>>().join("|");
            send_reply(sender, &format!("REPLY:{names}"), inbox);
        }
        "status" => {
            let count = inbox.lock().unwrap().len();
            send_reply(sender, &format!("REPLY:{{\"running\":{count}}}"), inbox);
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
            match spawn_app(&entry, inbox, app_registry) {
                Some(app) => {
                    let app_name = app.name.clone();
                    launch_app_threads(app, inbox, focused, app_registry);
                    send_reply(sender, &format!("REPLY:launched {app_name}"), inbox);
                }
                None => {
                    send_reply(sender, &format!("REPLY:error: could not spawn {path}"), inbox);
                }
            }
        }

        // ── P13T01 process management commands ───────────────────────────────

        // ps-raw — machine-readable: name:status:uptime_secs:restarts per entry
        // Used by gui-demo's live dashboard for structured parsing.
        "ps-raw" => {
            let entries: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut rows: Vec<(String, String)> = reg.iter().map(|(name, st)| {
                    let st = st.lock().unwrap();
                    let uptime = st.start_time.elapsed().as_secs();
                    let status = match &st.status {
                        AppStatus::Running    => "run",
                        AppStatus::Stopped(_) => "stop",
                    };
                    let entry = format!("{}:{}:{}:{}", name, status, uptime, st.restart_count);
                    (name.clone(), entry)
                }).collect();
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                rows.into_iter().map(|(_, v)| v).collect()
            };
            send_reply(sender, &format!("REPLY:{}", entries.join("|")), inbox);
        }

        // ps — list all apps with status, uptime, restart count
        "ps" => {
            let entries: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut rows: Vec<(String, String)> = reg.iter().map(|(name, st)| {
                    let st = st.lock().unwrap();
                    let uptime = st.start_time.elapsed().as_secs();
                    let status_str = match &st.status {
                        AppStatus::Running    => "running".to_string(),
                        AppStatus::Stopped(c) => format!("stopped({})", c),
                    };
                    let wd_tag = if st.watchdog_secs > 0 {
                        format!(" [watchdog={}s]", st.watchdog_secs)
                    } else {
                        String::new()
                    };
                    let info = format!(
                        "{:<16} {:<12} {:>5}s  restarts:{}{}",
                        name, status_str, uptime, st.restart_count, wd_tag
                    );
                    (name.clone(), info)
                }).collect();
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                rows.into_iter().map(|(_, v)| v).collect()
            };
            send_reply(sender, &format!("REPLY:{}", entries.join("|")), inbox);
        }

        // kill <app> — send SIGKILL to a running app
        "kill" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: kill <app>", inbox);
                    return;
                }
            };
            let pid = {
                let reg = app_registry.lock().unwrap();
                reg.get(&app_name).and_then(|st| st.lock().unwrap().child_pid)
            };
            match pid {
                Some(pid) => {
                    #[cfg(target_os = "linux")]
                    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                    eprintln!("vyoma-supervisor: killed {app_name} (pid {pid})");
                    send_reply(sender, &format!("REPLY:killed {app_name}"), inbox);
                }
                None => {
                    send_reply(sender, &format!("REPLY:{app_name} not running"), inbox);
                }
            }
        }

        // restart <app> — kill + re-spawn an app
        "restart" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: restart <app>", inbox);
                    return;
                }
            };
            let (pid, entry) = {
                let reg = app_registry.lock().unwrap();
                match reg.get(&app_name) {
                    Some(st) => {
                        let st = st.lock().unwrap();
                        (st.child_pid, st.entry.clone())
                    }
                    None => {
                        send_reply(sender, &format!("REPLY:error: app {app_name} not found"), inbox);
                        return;
                    }
                }
            };
            // Kill old instance (if still running)
            if let Some(pid) = pid {
                #[cfg(target_os = "linux")]
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                eprintln!("vyoma-supervisor: restart: killed old {app_name} (pid {pid})");
            }
            // Spawn new instance
            match spawn_app(&entry, inbox, app_registry) {
                Some(app) => {
                    launch_app_threads(app, inbox, focused, app_registry);
                    eprintln!("vyoma-supervisor: restarted {app_name}");
                    send_reply(sender, &format!("REPLY:restarted {app_name}"), inbox);
                }
                None => {
                    send_reply(sender, &format!("REPLY:error: could not restart {app_name}"), inbox);
                }
            }
        }

        // P30: update <app> <url> — download new wasm, verify sha256, hot-swap, restart
        "update" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (app_name, url) = match rest.split_once(' ') {
                Some((a, u)) if !a.trim().is_empty() && !u.trim().is_empty() => {
                    (a.trim().to_string(), u.trim().to_string())
                }
                _ => {
                    send_reply(sender, "REPLY:error: usage: update <app> <url>", inbox);
                    return;
                }
            };
            let entry = {
                let reg = app_registry.lock().unwrap();
                reg.get(&app_name).map(|st| st.lock().unwrap().entry.clone())
            };
            let entry = match entry {
                Some(e) => e,
                None => {
                    send_reply(sender, &format!("REPLY:error: unknown app: {app_name}"), inbox);
                    return;
                }
            };
            send_reply(sender, &format!("REPLY:downloading {url}…"), inbox);
            eprintln!("vyoma-supervisor: @supervisor: update {app_name} from {url}");

            let sender_name  = sender.to_string();
            let inbox_bg     = Arc::clone(inbox);
            let focused_bg   = Arc::clone(focused);
            let registry_bg  = Arc::clone(app_registry);

            thread::spawn(move || {
                let bytes = match http_get(&url) {
                    Ok(b)  => b,
                    Err(e) => {
                        eprintln!("vyoma-supervisor: update {app_name}: download failed: {e}");
                        send_reply(&sender_name, &format!("REPLY:error: download failed: {e}"), &inbox_bg);
                        return;
                    }
                };

                let tmp = format!("/tmp/{app_name}.wasm.new");
                if let Err(e) = fs::write(&tmp, &bytes) {
                    send_reply(&sender_name, &format!("REPLY:error: write tmp failed: {e}"), &inbox_bg);
                    return;
                }

                // Verify SHA-256 if declared in manifest (reuses P28 Sha256)
                if let Ok(raw) = fs::read_to_string(&entry.manifest) {
                    if let Ok(m) = toml::from_str::<AppManifest>(&raw) {
                        if let Some(expected) = &m.app.wasm_sha256 {
                            let actual = format!("{:x}", Sha256::digest(&bytes));
                            if actual != expected.to_lowercase() {
                                let _ = fs::remove_file(&tmp);
                                send_reply(&sender_name, "REPLY:error: SHA-256 mismatch — update rejected", &inbox_bg);
                                return;
                            }
                            eprintln!("vyoma-supervisor: update {app_name}: SHA-256 verified OK");
                        }
                    }
                }

                let dest = Path::new(&entry.manifest)
                    .parent()
                    .unwrap_or(Path::new("/apps"))
                    .join(format!("{app_name}.wasm"));
                if let Err(e) = fs::copy(&tmp, &dest) {
                    let _ = fs::remove_file(&tmp);
                    send_reply(&sender_name, &format!("REPLY:error: install failed: {e}"), &inbox_bg);
                    return;
                }
                let _ = fs::remove_file(&tmp);
                eprintln!("vyoma-supervisor: update {app_name}: installed → {dest:?}");
                send_reply(&sender_name, &format!("REPLY:installed — restarting {app_name}…"), &inbox_bg);

                // Kill old instance then respawn
                {
                    let reg = registry_bg.lock().unwrap();
                    if let Some(st) = reg.get(&app_name) {
                        if let Some(pid) = st.lock().unwrap().child_pid {
                            #[cfg(target_os = "linux")]
                            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                        }
                    }
                }
                match spawn_app(&entry, &inbox_bg, &registry_bg) {
                    Some(app) => {
                        launch_app_threads(app, &inbox_bg, &focused_bg, &registry_bg);
                        eprintln!("vyoma-supervisor: update {app_name}: restarted OK");
                        send_reply(&sender_name, &format!("REPLY:updated {app_name} OK"), &inbox_bg);
                    }
                    None => {
                        send_reply(&sender_name, "REPLY:error: restart failed after update", &inbox_bg);
                    }
                }
            });
        }

        // reload — re-read boot.toml, launch any apps not currently running
        "reload" => {
            let boot_raw = match fs::read_to_string(BOOT_CONFIG_PATH) {
                Ok(s) => s,
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: cannot read boot.toml: {e}"), inbox);
                    return;
                }
            };
            let boot: BootConfig = match toml::from_str(&boot_raw) {
                Ok(c) => c,
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: malformed boot.toml: {e}"), inbox);
                    return;
                }
            };
            let mut launched = 0usize;
            for entry in &boot.apps {
                // Parse manifest to get name
                let name = match fs::read_to_string(&entry.manifest).ok()
                    .and_then(|s| toml::from_str::<AppManifest>(&s).ok())
                    .map(|m| m.app.name)
                {
                    Some(n) => n,
                    None => continue,
                };
                // Skip already-running apps
                let already_running = {
                    let reg = app_registry.lock().unwrap();
                    reg.get(&name).map(|st| {
                        matches!(st.lock().unwrap().status, AppStatus::Running)
                    }).unwrap_or(false)
                };
                if already_running { continue; }

                if let Some(app) = spawn_app(entry, inbox, app_registry) {
                    launch_app_threads(app, inbox, focused, app_registry);
                    launched += 1;
                }
            }
            send_reply(
                sender,
                &format!("REPLY:reload done — {launched} new app(s) started"),
                inbox,
            );
        }

        // log <app> — show last LOG_BUF_SIZE lines of an app's stdout
        "log" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: log <app>", inbox);
                    return;
                }
            };
            let reply = {
                let reg = app_registry.lock().unwrap();
                match reg.get(&app_name) {
                    Some(st) => {
                        let st = st.lock().unwrap();
                        if st.log_buf.is_empty() {
                            format!("REPLY:no output captured for {app_name}")
                        } else {
                            // Replace any | in log lines so they don't break the reply protocol
                            let lines: Vec<String> = st.log_buf.iter()
                                .map(|s| s.replace('|', " "))
                                .collect();
                            format!("REPLY:{}", lines.join("|"))
                        }
                    }
                    None => format!("REPLY:error: app {app_name} not found"),
                }
            };
            send_reply(sender, &reply, inbox);
        }

        // ── P16T01: persistent log commands ──────────────────────────────────

        // logs — list apps that have a log file in /data/logs/
        "logs" => {
            let mut names: Vec<String> = fs::read_dir(LOG_DIR)
                .map(|rd| {
                    rd.filter_map(|e| {
                        let e = e.ok()?;
                        let fname = e.file_name().into_string().ok()?;
                        fname.strip_suffix(".log").map(|s| s.to_string())
                    }).collect()
                })
                .unwrap_or_default();
            names.sort();
            if names.is_empty() {
                send_reply(sender, "REPLY:no logs yet (apps haven't produced output)", inbox);
            } else {
                send_reply(sender, &format!("REPLY:{}", names.join("|")), inbox);
            }
        }

        // logf <app> — last LOG_TAIL_LINES lines from /data/logs/<app>.log
        "logf" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: logf <app>", inbox);
                    return;
                }
            };
            let log_path = format!("{LOG_DIR}/{app_name}.log");
            match fs::read_to_string(&log_path) {
                Ok(content) => {
                    let all: Vec<&str> = content.lines().collect();
                    let start = all.len().saturating_sub(LOG_TAIL_LINES);
                    let recent: Vec<String> = all[start..]
                        .iter()
                        .map(|s| s.replace('|', " "))
                        .collect();
                    if recent.is_empty() {
                        send_reply(sender, &format!("REPLY:{app_name}.log is empty"), inbox);
                    } else {
                        send_reply(sender, &format!("REPLY:{}", recent.join("|")), inbox);
                    }
                }
                Err(_) => {
                    send_reply(sender, &format!("REPLY:no log file for {app_name}"), inbox);
                }
            }
        }

        // ── P14T01: package manager commands ─────────────────────────────────

        // pkg-list — show available packages with install status
        "pkg-list" => {
            let installed = read_installed_apps();
            let rows: Vec<String> = PACKAGES.iter().map(|(name, ver, desc)| {
                let mark = if installed.iter().any(|n| n == name) { "+" } else { " " };
                format!("[{mark}] {name} v{ver}  {desc}")
            }).collect();
            if rows.is_empty() {
                send_reply(sender, "REPLY:no packages in catalog", inbox);
            } else {
                send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
            }
        }

        // pkg-installed — list user-installed app names
        "pkg-installed" => {
            let installed = read_installed_apps();
            if installed.is_empty() {
                send_reply(sender, "REPLY:no packages installed", inbox);
            } else {
                send_reply(sender, &format!("REPLY:{}", installed.join("|")), inbox);
            }
        }

        // pkg-install <name> — copy app to /data/apps, register, launch
        "pkg-install" => {
            let name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: pkg install <name>", inbox);
                    return;
                }
            };
            if !PACKAGES.iter().any(|(n, _, _)| *n == name.as_str()) {
                send_reply(sender, &format!("REPLY:error: unknown package '{name}'  (try: pkg list)"), inbox);
                return;
            }
            if read_installed_apps().iter().any(|n| n == &name) {
                send_reply(sender, &format!("REPLY:{name} is already installed"), inbox);
                return;
            }
            match install_package(&name, inbox, focused, app_registry) {
                Ok(()) => send_reply(sender, &format!("REPLY:installed {name} — now running"), inbox),
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }

        // pkg-remove <name> — kill app, remove files, unregister
        "pkg-remove" => {
            let name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: pkg remove <name>", inbox);
                    return;
                }
            };
            if !read_installed_apps().iter().any(|n| n == &name) {
                send_reply(sender, &format!("REPLY:error: '{name}' is not installed"), inbox);
                return;
            }
            match remove_package(&name, app_registry) {
                Ok(()) => send_reply(sender, &format!("REPLY:removed {name}"), inbox),
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }

        // P35: wallpaper <rgba_hex> — fill screen with solid color
        "wallpaper" => {
            let color_str = parts.get(1).unwrap_or(&"").trim();
            let rgba = color_str
                .strip_prefix("0x").or_else(|| color_str.strip_prefix("0X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| color_str.parse::<u32>().ok())
                .unwrap_or(0x0D1117FF);
            #[cfg(target_os = "linux")]
            if let Some(fb_lock) = display::get() {
                let mut fb = fb_lock.lock().unwrap();
                let (w, h) = (fb.width, fb.height);
                fb.fill_rect(0, 0, w, h, rgba);
                fb.flush();
            }
            eprintln!("vyoma-supervisor: wallpaper set to {rgba:#010x}");
            send_reply(sender, &format!("REPLY:wallpaper {rgba:#010x}"), inbox);
        }

        // P33: raise/lower window in Z-order
        "raise" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: raise <app>", inbox);
                    return;
                }
            };
            z_order_push_front(&app_name);
            *focused.lock().unwrap() = Some(app_name.clone());
            send_reply(sender, &format!("REPLY:raised {app_name}"), inbox);
        }
        "lower" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: lower <app>", inbox);
                    return;
                }
            };
            z_order_push_back(&app_name);
            send_reply(sender, &format!("REPLY:lowered {app_name}"), inbox);
        }

        // P36: resize <app> <w> <h> — update win_region and notify app
        "resize" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let mut args = rest.splitn(3, ' ');
            let app_name = args.next().unwrap_or("").trim().to_string();
            let w_str    = args.next().unwrap_or("").trim();
            let h_str    = args.next().unwrap_or("").trim();
            if app_name.is_empty() || w_str.is_empty() || h_str.is_empty() {
                send_reply(sender, "REPLY:error: usage: resize <app> <w> <h>", inbox);
                return;
            }
            let (new_w, new_h) = match (w_str.parse::<u32>(), h_str.parse::<u32>()) {
                (Ok(w), Ok(h)) if w > 0 && h > 0 => (w, h),
                _ => {
                    send_reply(sender, "REPLY:error: w and h must be positive integers", inbox);
                    return;
                }
            };
            let updated = {
                let reg = app_registry.lock().unwrap();
                if let Some(state_arc) = reg.get(&app_name) {
                    let mut st = state_arc.lock().unwrap();
                    let (x, y) = st.win_region.map(|(x, y, _, _)| (x, y)).unwrap_or((0, 0));
                    st.win_region = Some((x, y, new_w, new_h));
                    true
                } else {
                    false
                }
            };
            if !updated {
                send_reply(sender, &format!("REPLY:error: app {app_name} not found"), inbox);
                return;
            }
            send_reply(&app_name, &format!("VYOMA_SYSTEM:resize:{new_w},{new_h}"), inbox);
            eprintln!("vyoma-supervisor: resize {app_name} → {new_w}×{new_h}");
            send_reply(sender, &format!("REPLY:resized {app_name} to {new_w}x{new_h}"), inbox);
        }

        other => {
            eprintln!("vyoma-supervisor: unknown @supervisor command from {sender}: {other}");
        }
    }
}

// ── P14T01: package manager helpers ──────────────────────────────────────────

/// Read the list of user-installed app names from /data/installed.txt.
fn read_installed_apps() -> Vec<String> {
    fs::read_to_string(USER_BOOT_PATH)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// Copy app files from initramfs /apps/<name>/ to /data/apps/<name>/,
/// register in /data/installed.txt, and launch immediately.
fn install_package(
    name:         &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> Result<(), String> {
    let src_dir = format!("/apps/{name}");
    let dst_dir = format!("{DATA_APPS_DIR}/{name}");

    // Create destination dir
    fs::create_dir_all(&dst_dir)
        .map_err(|e| format!("create_dir {dst_dir}: {e}"))?;

    // Copy vyoma.toml
    let toml_dst = format!("{dst_dir}/vyoma.toml");
    fs::copy(format!("{src_dir}/vyoma.toml"), &toml_dst)
        .map_err(|e| format!("copy vyoma.toml: {e}"))?;

    // Read manifest to get the wasm filename
    let manifest_raw = fs::read_to_string(&toml_dst)
        .map_err(|e| format!("read manifest: {e}"))?;
    let manifest: AppManifest = toml::from_str(&manifest_raw)
        .map_err(|e| format!("parse manifest: {e}"))?;

    // Copy wasm binary
    fs::copy(
        format!("{src_dir}/{}", manifest.app.wasm),
        format!("{dst_dir}/{}", manifest.app.wasm),
    ).map_err(|e| format!("copy wasm: {e}"))?;

    // Append name to /data/installed.txt
    {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true).append(true)
            .open(USER_BOOT_PATH)
            .map_err(|e| format!("open installed.txt: {e}"))?;
        writeln!(f, "{name}").map_err(|e| format!("write installed.txt: {e}"))?;
    }

    // Launch immediately (no reboot required)
    let entry = BootEntry { manifest: toml_dst, restart: "never".to_string() };
    if let Some(app) = spawn_app(&entry, inbox, app_registry) {
        launch_app_threads(app, inbox, focused, app_registry);
    }

    eprintln!("vyoma-supervisor: pkg: installed {name}");
    Ok(())
}

/// Kill the app (if running), remove its files, and unregister from installed.txt.
fn remove_package(name: &str, app_registry: &AppRegistry) -> Result<(), String> {
    // Kill running instance
    let pid = {
        let reg = app_registry.lock().unwrap();
        reg.get(name).and_then(|st| st.lock().unwrap().child_pid)
    };
    if let Some(pid) = pid {
        #[cfg(target_os = "linux")]
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
        eprintln!("vyoma-supervisor: pkg: killed {name} (pid {pid})");
    }

    // Rewrite /data/installed.txt without this entry
    let kept: Vec<String> = read_installed_apps()
        .into_iter()
        .filter(|n| n != name)
        .collect();
    let content = if kept.is_empty() {
        String::new()
    } else {
        kept.join("\n") + "\n"
    };
    fs::write(USER_BOOT_PATH, content)
        .map_err(|e| format!("write installed.txt: {e}"))?;

    // Remove app directory
    let dst_dir = format!("{DATA_APPS_DIR}/{name}");
    if Path::new(&dst_dir).exists() {
        fs::remove_dir_all(&dst_dir)
            .map_err(|e| format!("remove {dst_dir}: {e}"))?;
    }

    eprintln!("vyoma-supervisor: pkg: removed {name}");
    Ok(())
}

/// P30: minimal HTTP/1.1 GET over plain TCP. Returns the response body.
/// Only supports `http://` (no TLS). URL format: `http://host[:port]/path`.
fn http_get(url: &str) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let rest = url.strip_prefix("http://")
        .ok_or_else(|| "only http:// URLs are supported".to_string())?;

    let (hostport, path) = if let Some(idx) = rest.find('/') {
        (&rest[..idx], &rest[idx..])
    } else {
        (rest, "/")
    };
    let (host, port) = if let Some(c) = hostport.rfind(':') {
        let p: u16 = hostport[c + 1..]
            .parse()
            .map_err(|_| format!("invalid port in '{hostport}'"))?;
        (&hostport[..c], p)
    } else {
        (hostport, 80u16)
    };

    let mut stream = TcpStream::connect((host, port))
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;

    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes())
        .map_err(|e| format!("request write: {e}"))?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)
        .map_err(|e| format!("response read: {e}"))?;

    let sep = raw.windows(4).position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "no HTTP header boundary in response".to_string())?;

    let status: u16 = std::str::from_utf8(&raw[..sep])
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if status != 200 {
        return Err(format!("HTTP {status}"));
    }

    Ok(raw[sep + 4..].to_vec())
}

fn send_reply(target: &str, msg: &str, inbox: &Inbox) {
    if let Some(tx) = inbox.lock().unwrap().get(target) {
        let _ = tx.send(msg.to_string());
    }
}

/// Find the first /dev/input/eventN that supports pointer events (EV_REL or EV_ABS).
/// Uses EVIOCGBIT(0, 1) ioctl — returns 1 byte of event-type capability bitmask.
/// Bit 2 = EV_REL (relative mouse), bit 3 = EV_ABS (absolute pointer, virtio-mouse-pci).
/// Returns None on headless boot where no pointer input devices exist.
#[cfg(target_os = "linux")]
fn open_mouse_device() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    // EVIOCGBIT(0, 1) = _IOC(_IOC_READ=2, 'E'=0x45, nr=0x20, size=1)
    //                 = (2<<30)|(0x45<<8)|0x20|(1<<16) = 0x80014520
    // libc::Ioctl is i32 on musl/x86-64 and u64 on glibc — use `as _` to coerce.
    const EVIOCGBIT_TYPE: u32 = 0x80014520u32;
    for i in 0..8u32 {
        let path = format!("/dev/input/event{i}");
        let Ok(f) = std::fs::File::open(&path) else { continue };
        let mut bits = 0u8;
        let ret = unsafe {
            libc::ioctl(
                f.as_raw_fd(),
                EVIOCGBIT_TYPE as _,
                &mut bits as *mut u8 as *mut libc::c_void,
            )
        };
        if ret >= 0 && (bits & (1 << 2) != 0 || bits & (1 << 3) != 0) {
            eprintln!("vyoma-supervisor: mouse-input: using {path}");
            return Some(f);
        }
    }
    None
}

/// Dispatch a mouse event:
///  P32 — check close-button hit for all display apps; send VYOMA_SYSTEM:window_event:close
///  P33 — on click, raise topmost window under cursor in Z-order; set keyboard focus
///         dispatch VYOMA_INPUT:mouse to the topmost mouse-capable app under cursor
#[cfg(target_os = "linux")]
fn dispatch_mouse(
    cx: i32,
    cy: i32,
    btn: u8,
    inbox: &Inbox,
    app_registry: &AppRegistry,
    focused: &FocusedApp,
) {
    // P32: check close-button clicks (in the chrome region above each windowed app)
    if btn == 1 {
        let close_hit = {
            let reg = app_registry.lock().unwrap();
            let mut hit: Option<String> = None;
            for (name, state_arc) in reg.iter() {
                let st = state_arc.lock().unwrap();
                if !st.has_display { continue; }
                let Some((wx, wy, ww, _)) = st.win_region else { continue };
                if wy < 20 || ww < 20 { continue; }
                // Close button occupies (wx+ww-16, wy-14, 12, 12)
                let cbx = (wx + ww) as i32 - 16;
                let cby = wy as i32 - 14;
                if cx >= cbx && cx < cbx + 12 && cy >= cby && cy < cby + 12 {
                    hit = Some(name.clone());
                    break;
                }
            }
            hit
        };
        if let Some(name) = close_hit {
            send_reply(&name, "VYOMA_SYSTEM:window_event:close", inbox);
            return;
        }
    }

    // Snapshot z-order before locking registry (avoids lock ordering issues)
    let z_snapshot: Vec<String> = Z_ORDER.get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default();

    // P33: on click, raise topmost window under cursor and set keyboard focus
    if btn == 1 {
        let raise_target = {
            let reg = app_registry.lock().unwrap();
            // Iterate z-order front-to-back; first window that contains the click wins
            let mut found: Option<String> = None;
            for name in &z_snapshot {
                let Some(state_arc) = reg.get(name) else { continue };
                let st = state_arc.lock().unwrap();
                let Some((wx, wy, ww, wh)) = st.win_region else { continue };
                if cx >= wx as i32 && cy >= wy as i32
                    && cx < (wx + ww) as i32 && cy < (wy + wh) as i32 {
                    found = Some(name.clone());
                    break;
                }
            }
            found
        };
        if let Some(ref name) = raise_target {
            z_order_push_front(name);
            *focused.lock().unwrap() = Some(name.clone());
        }
    }

    // Dispatch mouse event to topmost mouse-capable app under cursor (z-order aware)
    let target = {
        let reg = app_registry.lock().unwrap();
        let mut found: Option<(String, i32, i32)> = None;
        // Try z-order first (preserves topmost-window semantics when windows overlap)
        for name in &z_snapshot {
            let Some(state_arc) = reg.get(name) else { continue };
            let st = state_arc.lock().unwrap();
            if !st.has_mouse { continue; }
            let Some((wx, wy, ww, wh)) = st.win_region else { continue };
            if cx >= wx as i32 && cy >= wy as i32
                && cx < (wx + ww) as i32 && cy < (wy + wh) as i32 {
                let lx = cx - wx as i32;
                let ly = cy - wy as i32;
                found = Some((name.clone(), lx, ly));
                break;
            }
        }
        // Fallback: apps not in z_order (no window region) still get events
        if found.is_none() {
            for (name, state_arc) in reg.iter() {
                let st = state_arc.lock().unwrap();
                if !st.has_mouse { continue; }
                let Some((wx, wy, ww, wh)) = st.win_region else { continue };
                if cx >= wx as i32 && cy >= wy as i32
                    && cx < (wx + ww) as i32 && cy < (wy + wh) as i32 {
                    let lx = cx - wx as i32;
                    let ly = cy - wy as i32;
                    found = Some((name.clone(), lx, ly));
                    break;
                }
            }
        }
        found
    };
    if let Some((name, lx, ly)) = target {
        send_reply(&name, &format!("VYOMA_INPUT:mouse:{lx},{ly},{btn}"), inbox);
    }
}

// ── VYOMA_DRAW command dispatcher ─────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn handle_draw_command(cmd: &str, sender: &str, win: Option<(u32, u32, u32, u32)>) {
    let Some(fb_lock) = display::get() else { return };

    if cmd == "flush" || cmd == "present" {
        let mut fb = fb_lock.lock().unwrap();
        // P32: paint window decoration chrome on top of app content before blit
        if let Some((wx, wy, ww, _wh)) = win {
            if wy >= 20 && ww >= 20 {
                // Title bar background
                fb.fill_rect(wx, wy - 20, ww, 20, 0x21262DFF);
                // App name label
                fb.draw_text(wx + 8, wy - 16, sender, 0xFFFFFFFF, font::FontSize::Medium);
                // Close button — red square
                fb.fill_rect(wx + ww - 16, wy - 14, 12, 12, 0xFF5F56FF);
            }
        }
        fb.flush();
        return;
    }

    if let Some(args) = cmd.strip_prefix("fill_rect:") {
        let v: Vec<u32> = args.split(',').filter_map(|s| s.parse().ok()).collect();
        if let [lx, ly, w, h, rgba] = v.as_slice() {
            let (ax, ay, aw, ah) = match win {
                None => (*lx, *ly, *w, *h),
                Some((wx, wy, ww, wh)) => {
                    let ax = wx + *lx;
                    let ay = wy + *ly;
                    let win_right  = wx + ww;
                    let win_bottom = wy + wh;
                    if ax >= win_right || ay >= win_bottom { return; }
                    let aw = (*w).min(win_right  - ax);
                    let ah = (*h).min(win_bottom - ay);
                    if aw == 0 || ah == 0 { return; }
                    (ax, ay, aw, ah)
                }
            };
            fb_lock.lock().unwrap().fill_rect(ax, ay, aw, ah, *rgba);
        } else {
            eprintln!("vyoma-display: [{sender}] bad fill_rect args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text:") {
        // Try new format first: x,y,rgba,size,text  (5 comma-fields, size = s/m/l)
        // Fall back to legacy:   x,y,rgba,text       (4 comma-fields, size = Medium)
        let parts5: Vec<&str> = args.splitn(5, ',').collect();
        let parts4: Vec<&str> = args.splitn(4, ',').collect();

        let parsed = if parts5.len() == 5 {
            if let (Ok(lx), Ok(ly), Ok(rgba), Some(sz)) = (
                parts5[0].parse::<u32>(),
                parts5[1].parse::<u32>(),
                parts5[2].parse::<u32>(),
                font::parse_size(parts5[3]),
            ) {
                Some((lx, ly, rgba, sz, parts5[4]))
            } else {
                None
            }
        } else {
            None
        };

        let parsed = parsed.or_else(|| {
            if parts4.len() == 4 {
                if let (Ok(lx), Ok(ly), Ok(rgba)) = (
                    parts4[0].parse::<u32>(),
                    parts4[1].parse::<u32>(),
                    parts4[2].parse::<u32>(),
                ) {
                    Some((lx, ly, rgba, font::FontSize::Medium, parts4[3]))
                } else {
                    None
                }
            } else {
                None
            }
        });

        if let Some((lx, ly, rgba, size, text)) = parsed {
            let (ax, ay) = match win {
                None => (lx, ly),
                Some((wx, wy, ww, wh)) => {
                    let ax = wx + lx;
                    let ay = wy + ly;
                    if ax >= wx + ww || ay >= wy + wh { return; }
                    (ax, ay)
                }
            };
            fb_lock.lock().unwrap().draw_text(ax, ay, text, rgba, size);
        } else {
            eprintln!("vyoma-display: [{sender}] bad draw_text args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("rect_border:") {
        let parts: Vec<&str> = args.splitn(5, ',').collect();
        if parts.len() == 5 {
            if let (Ok(lx), Ok(ly), Ok(w), Ok(h), Ok(rgba)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parts[3].parse::<u32>(),
                parts[4].parse::<u32>(),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, w, h),
                    Some((wx, wy, ww, wh)) => {
                        let ax = wx + lx;
                        let ay = wy + ly;
                        let win_right  = wx + ww;
                        let win_bottom = wy + wh;
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().rect_border(ax, ay, aw, ah, rgba);
            } else {
                eprintln!("vyoma-display: [{sender}] bad rect_border args: {args}");
            }
        } else {
            eprintln!("vyoma-display: [{sender}] bad rect_border args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("clear_region:") {
        let parts: Vec<&str> = args.splitn(4, ',').collect();
        if parts.len() == 4 {
            if let (Ok(lx), Ok(ly), Ok(w), Ok(h)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parts[3].parse::<u32>(),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, w, h),
                    Some((wx, wy, ww, wh)) => {
                        let ax = wx + lx;
                        let ay = wy + ly;
                        let win_right  = wx + ww;
                        let win_bottom = wy + wh;
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().clear_region(ax, ay, aw, ah);
            } else {
                eprintln!("vyoma-display: [{sender}] bad clear_region args: {args}");
            }
        } else {
            eprintln!("vyoma-display: [{sender}] bad clear_region args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text_wrap:") {
        // Format: x,y,max_w,rgba,size,text  (splitn 6)
        let parts: Vec<&str> = args.splitn(6, ',').collect();
        if parts.len() == 6 {
            if let (Ok(lx), Ok(ly), Ok(max_w), Ok(rgba), Some(size)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parts[3].parse::<u32>(),
                font::parse_size(parts[4]),
            ) {
                let text = parts[5];
                let (ax, ay, effective_max_w) = match win {
                    None => (lx, ly, max_w),
                    Some((wx, wy, ww, wh)) => {
                        let ax = wx + lx;
                        let ay = wy + ly;
                        if ax >= wx + ww || ay >= wy + wh { return; }
                        let effective_max_w = max_w.min(ww.saturating_sub(lx));
                        if effective_max_w == 0 { return; }
                        (ax, ay, effective_max_w)
                    }
                };
                fb_lock.lock().unwrap().draw_text_wrap(ax, ay, effective_max_w, text, rgba, size);
            } else {
                eprintln!("vyoma-display: [{sender}] bad draw_text_wrap args: {args}");
            }
        } else {
            eprintln!("vyoma-display: [{sender}] bad draw_text_wrap args: {args}");
        }
        return;
    }

    eprintln!("vyoma-display: [{sender}] unknown command: {cmd}");
}

// ── App waiter — handles exit + automatic restart policy ─────────────────────

fn wait_app(
    entry:        BootEntry,
    name:         String,
    mut child:    Child,
    app_registry: AppRegistry,
    inbox:        Inbox,
    focused:      FocusedApp,
) {
    let mut restart_count: u32 = 0;

    loop {
        let exit_code = match child.wait() {
            Ok(s) => {
                let code = s.code().unwrap_or(-1);
                eprintln!("vyoma-supervisor: {name} exited (code {code})");
                code
            }
            Err(e) => {
                eprintln!("vyoma-supervisor: wait failed for {name}: {e}");
                -1
            }
        };

        // Update status in registry
        {
            let reg = app_registry.lock().unwrap();
            if let Some(st) = reg.get(&name) {
                let mut st = st.lock().unwrap();
                st.status = AppStatus::Stopped(exit_code);
                st.child_pid = None;
            }
        }

        let should_restart = match entry.restart.as_str() {
            "always"     => true,
            "on-failure" => exit_code != 0,
            _            => false,
        };

        if !should_restart {
            break;
        }

        eprintln!(
            "vyoma-supervisor: restarting {name} (policy={}, count={})",
            entry.restart, restart_count + 1
        );

        match spawn_app(&entry, &inbox, &app_registry) {
            Some(app) => {
                restart_count += 1;
                // Update restart_count in the new AppState
                {
                    let reg = app_registry.lock().unwrap();
                    if let Some(st) = reg.get(&name) {
                        st.lock().unwrap().restart_count = restart_count;
                    }
                }
                // Spawn new IO threads; continue this loop as the new waiter
                let SpawnedApp { child: new_child, child_stdin, child_stdout, msg_rx, has_display, win_region, .. } = app;
                spawn_io_threads(
                    &name, child_stdin, child_stdout, msg_rx,
                    has_display, win_region, &inbox, &focused, &app_registry,
                );
                child = new_child;
            }
            None => {
                eprintln!("vyoma-supervisor: failed to restart {name}, giving up");
                break;
            }
        }
    }
}

// ── P03T02: mount virtual filesystems ────────────────────────────────────────

fn mount_filesystems() {
    mount_fs("proc",     "/proc", "proc",     0);
    mount_fs("sysfs",    "/sys",  "sysfs",    0);
    mount_fs("devtmpfs", "/dev",  "devtmpfs", 0);
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
