// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

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

mod app_threads;
mod chrome;
mod draw_cmd;
mod input_keys;
mod ipc_commands;
mod ipc_handlers;
mod mount;
mod mouse_input;
mod net;
mod packages;
#[cfg(target_os = "linux")]
mod seccomp;
mod toast;

use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::Path,
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
    time::Instant,
};

use supervisor::logging::{format_log, Level, Subsystem};
use supervisor::manifest::{BootConfig, BootEntry};

macro_rules! log_info {
    ($sub:expr, $app:expr, $($arg:tt)*) => {
        eprintln!("{}", format_log(Level::Info,  $sub, $app, &format!($($arg)*)))
    };
}
macro_rules! log_warn {
    ($sub:expr, $app:expr, $($arg:tt)*) => {
        eprintln!("{}", format_log(Level::Warn,  $sub, $app, &format!($($arg)*)))
    };
}
macro_rules! log_error {
    ($sub:expr, $app:expr, $($arg:tt)*) => {
        eprintln!("{}", format_log(Level::Error, $sub, $app, &format!($($arg)*)))
    };
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
    win_region:  Option<(u32, u32, u32, u32)>,  // supervisor-assigned; updated by apply_tiling_layout
    min_size:    (u32, u32),                     // (min_w, min_h) hint from manifest [window]
    draw_ticks:      u64,           // incremented each time the app issues a VYOMA_DRAW command
    last_cpu_reset:  std::time::Instant, // when draw_ticks was last zeroed
}

type AppRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<AppState>>>>>;

// ── IPC inbox map + focus state ───────────────────────────────────────────────

type Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
type FocusedApp = Arc<Mutex<Option<String>>>;

// ── P33: Global Z-order stack (index 0 = topmost / frontmost window) ─────────

static Z_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

// ── IPC reply routing: maps recipient_app → last sender_app ──────────────────

static LAST_SENDER: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

// ── P47: Raw TCP connection pool ──────────────────────────────────────────────

static TCP_CONNS: OnceLock<Mutex<std::collections::HashMap<u32, std::net::TcpStream>>> =
    OnceLock::new();
static TCP_NEXT_ID: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(1);

// ── P50: Clipboard ────────────────────────────────────────────────────────────

static CLIPBOARD: OnceLock<Mutex<String>> = OnceLock::new();

// ── P55: Global font size preference ─────────────────────────────────────────

static FONT_SIZE: OnceLock<Mutex<String>> = OnceLock::new();

// Drag-start state: Some((x, y)) while the left button is held, None otherwise.
// Used by the drag-delta logging stub (P031).
static MOUSE_DRAG_START: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
fn mouse_drag_start() -> &'static Mutex<Option<(i32, i32)>> {
    MOUSE_DRAG_START.get_or_init(|| Mutex::new(None))
}

// Boot timestamp — used to render the elapsed clock in the menu bar.
static BOOT_INSTANT: OnceLock<std::time::Instant> = OnceLock::new();

// Rate-limit: tracks (last_draw_instant, last_focused_name) for the menu bar.
// Menu bar is only repainted when ≥1 second has elapsed OR focused app changes.
static LAST_MENUBAR_DRAW: OnceLock<Mutex<(std::time::Instant, Option<String>)>> = OnceLock::new();

// Dirty-flag map: true if the app has issued at least one draw command since its
// last flush.  Avoids repainting the title bar for windows with no new content.
static APP_DIRTY: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

// Currently hovered app name (title bar hover, no button press).
// Updated on every mouse-motion event; None when cursor is not over any title bar.
static HOVERED_APP: OnceLock<Mutex<Option<String>>> = OnceLock::new();

// Per-app log level filter — set via @supervisor: loglevel <name> <level>.
static APP_LOG_LEVELS: OnceLock<Mutex<HashMap<String, supervisor::ipc::LogLevel>>> = OnceLock::new();
fn app_log_levels() -> &'static Mutex<HashMap<String, supervisor::ipc::LogLevel>> {
    APP_LOG_LEVELS.get_or_init(|| Mutex::new(HashMap::new()))
}

// Per-app flush rate tracking: maps app name → (flush_count, window_start).
// Logged every 5 seconds via `display::format_fps`.
static FLUSH_COUNTS: OnceLock<Mutex<HashMap<String, (u64, std::time::Instant)>>> =
    OnceLock::new();

fn flush_counts() -> &'static Mutex<HashMap<String, (u64, std::time::Instant)>> {
    FLUSH_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
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
}

// ── Main ──────────────────────────────────────────────────────────────────────

const BOOT_CONFIG_PATH:  &str = "/etc/vyoma/boot.toml";
const USER_BOOT_PATH:    &str = "/data/installed.txt";
const DATA_APPS_DIR:     &str = "/data/apps";
const LOG_DIR:           &str = "/data/logs";
const LOG_TAIL_LINES:    usize = 30;


// ── P19: watchdog backoff helper ─────────────────────────────────────────────

fn watchdog_next_backoff(watchdog_secs: u32, restarts: u32) -> u64 {
    let base = watchdog_secs as u64;
    let factor = 1u64 << restarts.min(8);
    (base * factor).min(300)
}

fn main() {
    BOOT_INSTANT.get_or_init(std::time::Instant::now);
    log_info!(Subsystem::Lifecycle, None, "starting");

    mount::mount_filesystems();
    log_info!(Subsystem::Lifecycle, None, "filesystems mounted");

    #[cfg(target_os = "linux")]
    if display::init() {
        log_info!(Subsystem::Display, None, "display ready");
        // P35: paint default desktop background before any app draws
        if let Some(fb_lock) = display::get() {
            let mut fb = fb_lock.lock().unwrap();
            let (w, h) = (fb.width, fb.height);
            fb.fill_rect(0, 0, w, h, 0x0D1117FF);
            // Draw initial menu bar (no focused app yet, no apps yet)
            chrome::draw_menubar(&mut *fb, w, 0, None, &[]);
            fb.flush();
        }
    }

    let boot_raw = match fs::read_to_string(BOOT_CONFIG_PATH) {
        Ok(s) => s,
        Err(e) => {
            log_error!(Subsystem::Manifest, None, "FATAL: cannot read {BOOT_CONFIG_PATH}: {e}");
            std::process::exit(1);
        }
    };

    let boot: BootConfig = match toml::from_str(&boot_raw) {
        Ok(c) => c,
        Err(e) => {
            log_error!(Subsystem::Manifest, None, "FATAL: malformed {BOOT_CONFIG_PATH}: {e}");
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
                log_warn!(Subsystem::Manifest, Some(name), "installed app missing manifest, skipping");
            }
        }
        if user_count > 0 {
            log_info!(Subsystem::Lifecycle, None, "{user_count} user-installed app(s) added");
        }
    }

    log_info!(Subsystem::Lifecycle, None, "{} app(s) total", all_entries.len());

    if all_entries.is_empty() {
        log_info!(Subsystem::Lifecycle, None, "no apps configured, idling");
        loop { thread::park(); }
    }

    let _ = Z_ORDER.set(Mutex::new(Vec::new()));
    let _ = TCP_CONNS.set(Mutex::new(std::collections::HashMap::new()));
    let _ = CLIPBOARD.set(Mutex::new(String::new()));
    let _ = FONT_SIZE.set(Mutex::new("m".to_string()));
    let _ = LAST_SENDER.set(Mutex::new(HashMap::new()));

    let inbox:        Inbox       = Arc::new(Mutex::new(HashMap::new()));
    let focused:      FocusedApp  = Arc::new(Mutex::new(None));
    let app_registry: AppRegistry = Arc::new(Mutex::new(HashMap::new()));

    // ── Pass 1: spawn all processes and register inbox entries ────────────────
    let mut spawned: Vec<SpawnedApp> = Vec::new();
    let mut registered_names: Vec<String> = Vec::new();
    for entry in all_entries {
        // Pre-parse for duplicate-name detection (FR-006 / Clarification Q3).
        match supervisor::manifest::parse_manifest(std::path::Path::new(&entry.manifest)) {
            Ok(m) => {
                let names_slice: Vec<&str> = registered_names.iter().map(|s| s.as_str()).collect();
                if let Err(e) = supervisor::manifest::validate_manifest(&m, &names_slice) {
                    log_error!(Subsystem::Manifest, None, "[manifest] {}: {e}", entry.manifest);
                    continue;
                }
                registered_names.push(m.app.name.clone());
            }
            Err(e) => {
                log_error!(Subsystem::Manifest, None, "[manifest] {e}");
                continue;
            }
        }
        match app_threads::spawn_app(&entry, &inbox, &app_registry) {
            Some(app) => spawned.push(app),
            None => log_warn!(Subsystem::Lifecycle, None, "failed to spawn {}, skipping", entry.manifest),
        }
    }

    // T022 [FR-003]: canonical ready-signal — smoke test greps for this exact substring.
    log_info!(Subsystem::Lifecycle, None, "all apps spawned");

    // ── Set default keyboard focus to the first shell app ────────────────────
    {
        let shell_name = spawned.iter().find(|a| a.is_shell).map(|a| a.name.clone());
        if let Some(ref name) = shell_name {
            log_info!(Subsystem::Input, Some(name.as_str()), "keyboard focus → {name}");
        }
        *focused.lock().unwrap() = shell_name;
    }

    // ── P17T01: input-router thread — /dev/tty0 → focused app (raw mode) ─────
    #[cfg(target_os = "linux")]
    {
        let inbox_input    = Arc::clone(&inbox);
        let focused_input  = Arc::clone(&focused);
        let registry_input = Arc::clone(&app_registry);
        thread::Builder::new()
            .name("input-router".into())
            .spawn(move || input_keys::run_input_router(inbox_input, focused_input, registry_input))
            .expect("spawn input-router");
    }

    // ── P22: mouse-input thread — /dev/input/eventN → mouse-capable apps ──────
    #[cfg(target_os = "linux")]
    {
        let inbox_m    = Arc::clone(&inbox);
        let registry_m = Arc::clone(&app_registry);
        let focused_m  = Arc::clone(&focused);
        thread::Builder::new()
            .name("mouse-input".into())
            .spawn(move || mouse_input::run_mouse_input(inbox_m, registry_m, focused_m))
            .expect("spawn mouse-input thread");
    }

    // ── Pass 2: start IO threads for each spawned app ─────────────────────────
    let mut waiter_handles = vec![];
    for app in spawned {
        let wh = app_threads::launch_app_threads(app, &inbox, &focused, &app_registry);
        waiter_handles.push(wh);
    }

    // ── P19: watchdog thread — kills apps silent longer than watchdog_secs ───
    {
        let registry_wd = Arc::clone(&app_registry);
        thread::Builder::new()
            .name("watchdog".into())
            .spawn(move || app_threads::run_watchdog(registry_wd))
            .expect("spawn watchdog thread");
    }

    drop(inbox);
    drop(focused);
    drop(app_registry);

    for handle in waiter_handles {
        let _ = handle.join();
    }

    log_info!(Subsystem::Lifecycle, None, "all apps completed, idling");
    loop { thread::park(); }
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
    if has_display {
        if let Some(cmd) = line.strip_prefix("VYOMA_DRAW:") {
            // Increment draw_ticks for CPU% tracking
            {
                let reg = app_registry.lock().unwrap();
                if let Some(st) = reg.get(sender) {
                    st.lock().unwrap().draw_ticks += 1;
                }
            }
            #[cfg(target_os = "linux")]
            {
                draw_cmd::handle_draw_command(cmd, sender, win_region, focused, app_registry);
            }
            #[cfg(not(target_os = "linux"))]
            let _ = cmd;
            return;
        }
    }
    let _ = has_display;
    let _ = win_region;

    if let Some(rest) = line.strip_prefix('@') {
        if let Some((target, msg)) = rest.split_once(": ") {
            if target == "supervisor" {
                ipc_handlers::handle_supervisor_command(msg, sender, inbox, focused, app_registry);
                return;
            }
            if supervisor::ipc::is_broadcast_target(target) {
                // Deliver message to every running app (including the sender).
                let map = inbox.lock().unwrap();
                for tx in map.values() {
                    let _ = tx.send(msg.to_string());
                }
                log_info!(Subsystem::Ipc, Some(sender), "broadcast: \"{}\" → {} app(s)", msg, map.len());
                return;
            }
            // @reply: routes message back to whichever app last sent an IPC message
            // to the current sender (i.e. LAST_SENDER[sender]).
            if supervisor::ipc::is_reply_target(target) {
                let reply_target = LAST_SENDER
                    .get()
                    .and_then(|m| m.lock().ok())
                    .and_then(|map| map.get(sender).cloned());
                match reply_target {
                    Some(orig) => {
                        let map = inbox.lock().unwrap();
                        if let Some(tx) = map.get(&orig) {
                            let _ = tx.send(msg.to_string());
                        }
                    }
                    None => {
                        log_warn!(Subsystem::Ipc, Some(sender),
                            "@reply: no last sender recorded for {sender}, message dropped");
                    }
                }
                return;
            }
            // Record that `sender` sent a message to `target` so `target` can @reply:.
            if let Some(ls) = LAST_SENDER.get() {
                if let Ok(mut map) = ls.lock() {
                    map.insert(target.to_string(), sender.to_string());
                }
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

fn send_reply(target: &str, msg: &str, inbox: &Inbox) {
    if let Some(tx) = inbox.lock().unwrap().get(target) {
        let _ = tx.send(msg.to_string());
    }
}

