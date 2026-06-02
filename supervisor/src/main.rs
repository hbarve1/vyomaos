#![allow(unused_imports, unused_variables)]
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

mod types;
pub(crate) use types::*;
mod globals;
pub(crate) use globals::*;
mod paths;
pub(crate) use paths::*;

#[allow(dead_code)] mod accessibility;
mod archive_ipc;
mod app_threads;
#[allow(dead_code)] mod bg_service;
#[allow(dead_code)] mod audio;
#[allow(dead_code)] mod acpi;
#[allow(dead_code)] mod battery;
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
#[allow(dead_code)] mod namespace;
mod network;
pub(crate) use network::net;
pub(crate) use network::websocket;
mod packages;
#[allow(dead_code)] mod recovery;
#[cfg(target_os = "linux")]
mod seccomp;
mod theme;
mod toast;
#[allow(dead_code)] mod trash;
mod tray;
mod verify;
#[allow(dead_code)] mod secure_boot;
mod mgmt_protocol;
mod mgmt_server;
mod mgmt_handlers;
mod router;
mod ota_update;
#[allow(dead_code)] mod atomic_update;
mod auto_update;
mod resize;
mod drag_drop;
#[allow(dead_code)] mod share;
mod session;
#[allow(dead_code)] mod undo;
mod win_actions;
#[allow(dead_code)] mod screenshot;
#[allow(dead_code)] mod wallpaper;
#[allow(dead_code)] mod file_assoc;
#[allow(dead_code)] mod vfs;
#[allow(dead_code)] mod encrypted_store;
#[allow(dead_code)] mod pkg_registry;
#[allow(dead_code)] mod workspace;
#[allow(dead_code)] mod audit;
#[allow(dead_code)] mod cap_request;
#[allow(dead_code)] mod totp;
#[allow(dead_code)] mod user;
#[allow(dead_code)] mod user_caps;
#[allow(dead_code)] mod firewall;
#[allow(dead_code)] mod memory;
#[allow(dead_code)] mod cpu;
#[allow(dead_code)] mod installer;
#[allow(dead_code)] mod jit_config;
#[allow(dead_code)] mod backup;
#[allow(dead_code)] mod uefi;
#[allow(dead_code)] mod virtualization;
#[allow(dead_code)]
pub(crate) use network::vnc;
#[allow(dead_code)] mod store;

use std::{
    collections::HashMap,
    fs,
    path::Path,
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

// ── Main ──────────────────────────────────────────────────────────────────────

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

    // ── P91: Secure boot chain verification ──────────────────────────────────
    secure_boot::run_boot_verification();

    // ── P106: Recovery mode — increment boot counter, check triggers ─────────
    recovery::increment_boot_count();
    recovery::check_recovery_mode();

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
    // Pre-warm common pt sizes used by chrome (13, 15, 16) to avoid first-frame lag.
    #[cfg(target_os = "linux")]
    {
        let fc = font_cache();
        let mut cache = fc.lock().unwrap();
        for &pt in &[13u32, 15, 16] {
            for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789:".chars() {
                cache.rasterize(ch, pt, false, false);
            }
        }
        drop(cache);
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

    // ── P106: In recovery mode, restrict to shell app only ─────────────────
    recovery::enter_recovery_mode(&mut all_entries);

    log_info!(Subsystem::Lifecycle, None, "{} app(s) total", all_entries.len());

    if all_entries.is_empty() {
        log_info!(Subsystem::Lifecycle, None, "no apps configured, idling");
        loop { thread::park(); }
    }

    // P88: load per-app firewall rules from /data/firewall.toml
    firewall::load_rules();

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

    // ── P106: Clear boot counter — successful boot confirmed ─────────────────
    recovery::clear_boot_count();

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
                let mut last = display::screen_size().unwrap_or((1920, 1080)); // DEFAULT_SCREEN_W/H fallback
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

    // ── P94: memory pressure monitor thread ─────────────────────────────────
    memory::spawn_pressure_monitor(&inbox, &app_registry);

    // ── P95: battery monitor thread (every 30s) ──────────────────────────────
    battery::spawn_battery_monitor(&inbox, &app_registry, &focused);

    // ── P104: thermal monitor thread (every 10s) ────────────────────────────
    acpi::spawn_thermal_monitor(&inbox, &app_registry);

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
