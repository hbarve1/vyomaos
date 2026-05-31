// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS supervisor — PID 1
//!
//! Boots apps, routes IPC, manages display, input, process lifecycle,
//! OTA updates (P29/P30), and dynamic resolution detection.

#[cfg(target_os = "linux")]
mod display;
mod font;
mod image;

mod accessibility;
mod app_threads;
mod audio;
mod chrome;
mod draw_cmd;
mod i18n;
mod menus;
mod input_keys;
mod ipc_commands;
mod ipc_handlers;
mod mount;
mod mouse_input;
#[cfg(target_os = "linux")]
mod namespace;
mod net;
mod websocket;
mod packages;
#[cfg(target_os = "linux")]
mod seccomp;
mod theme;
mod toast;
mod trash;
mod tray;
mod verify;
mod mgmt_protocol;
mod mgmt_server;
mod mgmt_handlers;
mod router;
mod ota_update;
mod auto_update;
mod resize;
mod drag_drop;
mod session;
mod undo;
mod win_actions;
mod screenshot;
mod wallpaper;
mod file_assoc;
mod vfs;
mod workspace;

use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::Path,
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
    time::Instant,
};

use supervisor::logging::Subsystem;
use supervisor::manifest::{BootConfig, BootEntry};
use supervisor::profile;

#[macro_export]
macro_rules! log_info {
    ($sub:expr, $app:expr, $($arg:tt)*) => {
        eprintln!("{}", supervisor::logging::format_log(supervisor::logging::Level::Info,  $sub, $app, &format!($($arg)*)))
    };
}
#[macro_export]
macro_rules! log_warn {
    ($sub:expr, $app:expr, $($arg:tt)*) => {
        eprintln!("{}", supervisor::logging::format_log(supervisor::logging::Level::Warn,  $sub, $app, &format!($($arg)*)))
    };
}
#[macro_export]
macro_rules! log_error {
    ($sub:expr, $app:expr, $($arg:tt)*) => {
        eprintln!("{}", supervisor::logging::format_log(supervisor::logging::Level::Error, $sub, $app, &format!($($arg)*)))
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
    has_audio:   bool,
    win_region:  Option<(u32, u32, u32, u32)>,  // supervisor-assigned; updated by apply_tiling_layout
    win_z:       u32,  // z-layer; 0=desktop, 10=default, 100=dock, 200+=system
    min_size:    (u32, u32),                     // (min_w, min_h) hint from manifest [window]
    draw_ticks:      u64,           // incremented each time the app issues a VYOMA_DRAW command
    last_cpu_reset:  std::time::Instant, // when draw_ticks was last zeroed
    // spec-042: minimize/restore state
    minimized:           bool,
    pre_minimize_region: Option<(u32, u32, u32, u32)>,
    // T052: pending window animation (Open/Close/Minimize)
    pub pending_anim: Option<crate::display::animator::Animation>,
    /// Per-window pixel surface buffer (content area only, no chrome).
    /// None until the first tiling layout assigns a win_region.
    pub surface: Option<std::sync::Arc<std::sync::Mutex<crate::display::Surface>>>,
    /// P31: set to true when the app sends `present` or `flush`, cleared after composite.
    pub frame_ready: bool,
    /// Declarative menu items from the app manifest (T058).
    menu_items: Vec<supervisor::manifest::MenuItem>,
    // spec-044: management server live log subscribers
    log_subscribers: Vec<mpsc::Sender<String>>,
}

type AppRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<AppState>>>>>;

// ── IPC inbox map + focus state ───────────────────────────────────────────────

type Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
type FocusedApp = Arc<Mutex<Option<String>>>;

// ── Global statics ────────────────────────────────────────────────────────────

static Z_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static LAST_SENDER: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static TCP_CONNS: OnceLock<Mutex<std::collections::HashMap<u32, std::net::TcpStream>>> =
    OnceLock::new();
static TCP_NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
static WS_CONNS: OnceLock<Mutex<std::collections::HashMap<u32, websocket::WsConnection>>> =
    OnceLock::new();
static WS_NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
static CLIPBOARD: OnceLock<Mutex<String>> = OnceLock::new();
/// Lock screen state: when true, keyboard input is only routed to screen-lock.
pub static LOCKED: OnceLock<Mutex<bool>> = OnceLock::new();
pub fn is_locked() -> bool { *LOCKED.get_or_init(|| Mutex::new(false)).lock().unwrap() }
pub fn set_locked(v: bool) { *LOCKED.get_or_init(|| Mutex::new(false)).lock().unwrap() = v; }
static FONT_SIZE: OnceLock<Mutex<String>> = OnceLock::new();
static MOUSE_DRAG_START: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
fn mouse_drag_start() -> &'static Mutex<Option<(i32, i32)>> {
    MOUSE_DRAG_START.get_or_init(|| Mutex::new(None))
}
/// Shared inbox Arc reference for the exec handler (set once after inbox is created).
static MGMT_INBOX: OnceLock<Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>> = OnceLock::new();
/// One-shot reply channels registered by the exec handler, keyed by app name.
static EXEC_REPLY_CHANNELS: OnceLock<Mutex<HashMap<String, mpsc::Sender<String>>>> =
    OnceLock::new();
static BOOT_INSTANT: OnceLock<std::time::Instant> = OnceLock::new();
#[allow(dead_code)]
static LAST_MENUBAR_DRAW: OnceLock<Mutex<(std::time::Instant, Option<String>)>> = OnceLock::new();
static APP_DIRTY: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
static HOVERED_APP: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static APP_LOG_LEVELS: OnceLock<Mutex<HashMap<String, supervisor::ipc::LogLevel>>> = OnceLock::new();
fn app_log_levels() -> &'static Mutex<HashMap<String, supervisor::ipc::LogLevel>> {
    APP_LOG_LEVELS.get_or_init(|| Mutex::new(HashMap::new()))
}
static FLUSH_COUNTS: OnceLock<Mutex<HashMap<String, (u64, std::time::Instant)>>> = OnceLock::new();
fn flush_counts() -> &'static Mutex<HashMap<String, (u64, std::time::Instant)>> {
    FLUSH_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}
/// Gate: false = suppress menu bar rendering on non-desktop profiles.
pub static SHOW_MENU_BAR: OnceLock<bool> = OnceLock::new();
/// Gate: false = suppress dock strip rendering on profiles without a dock.
pub static SHOW_DOCK: OnceLock<bool> = OnceLock::new();
/// Gate: false = single-app fullscreen mode (phone, watch, TV profiles).
pub static WINDOWED_MODE: OnceLock<bool> = OnceLock::new();
/// Gate: true = draw a focus ring around the focused window (TV/Vision).
pub static FOCUS_RING: OnceLock<bool> = OnceLock::new();
/// Active display profile name broadcast to apps via VYOMA_SYSTEM:display_profile.
pub static DISPLAY_PROFILE: OnceLock<String> = OnceLock::new();
/// P29: cached screen resolution set once during display init, queried by broadcast.
pub static SCREEN_SIZE: OnceLock<(u32, u32)> = OnceLock::new();

// ── T023: Scalable font cache (Linux-only) ────────────────────────────────────
#[cfg(target_os = "linux")]
static FONT_CACHE: OnceLock<Mutex<font::cache::FontCache>> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn font_cache() -> &'static Mutex<font::cache::FontCache> {
    FONT_CACHE.get_or_init(|| {
        Mutex::new(font::cache::FontCache::load(
            "/fonts/Inter-Regular.ttf",
            "/fonts/Inter-Bold.ttf",
            "/fonts/IBMPlexMono-Regular.ttf",
        ))
    })
}

// ── T030/T031: PNG image cache ────────────────────────────────────────────────
static IMAGE_CACHE: OnceLock<Mutex<image::ImageCache>> = OnceLock::new();

pub fn image_cache() -> &'static Mutex<image::ImageCache> {
    IMAGE_CACHE.get_or_init(|| Mutex::new(image::ImageCache::new()))
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

// ── T017: Platform profile loader ────────────────────────────────────────────

const PLATFORM_PROFILE_DIR: &str = "/etc/vyoma/profiles";
const DEFAULT_PROFILE_NAME: &str = "desktop-full";

/// Load the active platform profile (`PLATFORM` env var, default `desktop-full`).
fn load_platform_profile() -> Option<profile::PlatformProfile> {
    let name = std::env::var("PLATFORM")
        .unwrap_or_else(|_| DEFAULT_PROFILE_NAME.to_string());
    let path = format!("{PLATFORM_PROFILE_DIR}/{name}.toml");
    match profile::load_profile(std::path::Path::new(&path)) {
        Ok(p) => {
            log_info!(Subsystem::Lifecycle, None,
                "platform profile loaded: {} (runtime={:?}, ram={}KB)",
                p.platform.name, p.platform.runtime, p.platform.min_ram_kb);
            Some(p)
        }
        Err(profile::ProfileError::Io(_)) => {
            log_info!(Subsystem::Lifecycle, None, "no platform profile at {path}, using defaults");
            None
        }
        Err(e) => {
            log_warn!(Subsystem::Lifecycle, None, "platform profile error: {e}");
            None
        }
    }
}

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

    // ── T017: Load platform profile (PLATFORM env var or default) ────────────
    let _active_profile = load_platform_profile();
    if let Some(ref p) = _active_profile {
        let _ = SHOW_MENU_BAR.set(p.display.show_menu_bar);
        let _ = SHOW_DOCK.set(p.display.show_dock);
        let _ = WINDOWED_MODE.set(p.display.windowed_mode);
        let _ = FOCUS_RING.set(p.display.focus_ring);
        let _ = DISPLAY_PROFILE.set(p.display.profile.clone());
    }

    #[cfg(target_os = "linux")]
    if display::init() {
        log_info!(Subsystem::Display, None, "display ready");
        // P29: cache screen resolution for broadcast to display apps
        if let Some(sz) = display::screen_size() { let _ = SCREEN_SIZE.set(sz); }
        // P35: paint default desktop background before any app draws
        if let Some(fb_lock) = display::get() {
            let mut fb = fb_lock.lock().unwrap();
            let (w, h) = (fb.width, fb.height);
            fb.fill_rect(0, 0, w, h, 0x1C1C1EFF);
            // Draw initial menu bar (no focused app yet, no apps yet)
            chrome::draw_menubar(&mut *fb, w, 0, None, &[]);
            fb.flush();
        }
    }
    // T023: Initialize scalable font cache (warm-up; graceful if fonts missing)
    #[cfg(target_os = "linux")]
    { let _ = font_cache(); }

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
    let _ = WS_CONNS.set(Mutex::new(std::collections::HashMap::new()));
    let _ = CLIPBOARD.set(Mutex::new(String::new()));
    let _ = FONT_SIZE.set(Mutex::new("m".to_string()));
    let _ = LAST_SENDER.set(Mutex::new(HashMap::new()));
    let _ = EXEC_REPLY_CHANNELS.set(Mutex::new(HashMap::new()));

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

    // ── Boot-time session restore — apply saved window positions ─────────────
    {
        let entries = session::restore_session();
        if !entries.is_empty() {
            let restored = session::apply_session(&entries, &app_registry, &focused, &inbox);
            log_info!(Subsystem::Lifecycle, None, "boot session restore: {restored} window(s) applied");
        }
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
            .spawn(move || mouse_input::run_mouse_input(inbox_m, focused_m, registry_m))
            .expect("spawn mouse-input thread");
    }

    // ── spec-044: share inbox Arc with mgmt exec handler ────────────────────
    let _ = MGMT_INBOX.set(Arc::clone(&inbox));

    // ── spec-044: management server thread — binds 0.0.0.0:9090 ─────────────
    {
        let registry_mgmt = Arc::clone(&app_registry);
        thread::Builder::new()
            .name("mgmt-server".into())
            .spawn(move || {
                let addr: std::net::SocketAddr = "0.0.0.0:9090".parse().unwrap();
                mgmt_server::MgmtServer::new(registry_mgmt).start(addr);
            })
            .expect("spawn mgmt-server thread");
    }

    // ── T018 [US2]: screen-resize poll thread — detects framebuffer size changes ──
    #[cfg(target_os = "linux")]
    {
        let inbox_resize   = Arc::clone(&inbox);
        let registry_resize = Arc::clone(&app_registry);
        thread::Builder::new()
            .name("screen-poll".into())
            .spawn(move || {
                let mut last = display::screen_size().unwrap_or((1440, 900)); // DEFAULT_SCREEN_W/H fallback
                loop {
                    thread::sleep(std::time::Duration::from_secs(1));
                    if let Some((w, h)) = display::screen_size() {
                        if (w, h) != last {
                            log_info!(Subsystem::Display, None,
                                "screen resize detected: {}×{} → {}×{}", last.0, last.1, w, h);
                            last = (w, h);
                            // Broadcast new dimensions to all running display apps.
                            let msg = format!("VYOMA_SYSTEM:screen:{},{}", w, h);
                            let reg = registry_resize.lock().unwrap();
                            let inb = inbox_resize.lock().unwrap();
                            for (name, state_arc) in reg.iter() {
                                let st = state_arc.lock().unwrap();
                                if st.has_display && matches!(st.status, AppStatus::Running) {
                                    if let Some(tx) = inb.get(name) {
                                        let _ = tx.send(msg.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .expect("spawn screen-poll thread");
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

    // ── Auto-update background checker (hourly) ─────────────────────────────
    auto_update::spawn_background_checker(&inbox, &app_registry);

    // ── P31: compositor tick thread — polls frame_ready flags and recomposites ──
    #[cfg(target_os = "linux")]
    {
        let registry_comp = Arc::clone(&app_registry);
        let focused_comp  = Arc::clone(&focused);
        thread::Builder::new()
            .name("compositor-tick".into())
            .spawn(move || draw_cmd::run_compositor_tick(&registry_comp, &focused_comp))
            .expect("spawn compositor-tick thread");
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

fn route_or_print(
    line: &str, sender: &str, inbox: &Inbox, has_display: bool,
    win_region: Option<(u32, u32, u32, u32)>, focused: &FocusedApp, app_registry: &AppRegistry,
) { router::route_or_print(line, sender, inbox, has_display, win_region, focused, app_registry); }

fn send_reply(target: &str, msg: &str, inbox: &Inbox) { router::send_reply(target, msg, inbox); }
