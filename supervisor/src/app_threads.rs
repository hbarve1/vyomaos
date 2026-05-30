// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! App lifecycle helpers: spawn, IO threads, waiter thread.

use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};

use crate::{
    log_error, log_info, log_warn,
    AppRegistry, AppState, AppStatus, FocusedApp, Inbox, SpawnedApp,
    LAST_SENDER, LOG_BUF_SIZE, LOG_DIR,
};
use crate::{chrome, route_or_print, watchdog_next_backoff};
use supervisor::logging::Subsystem;
use supervisor::manifest::BootEntry;

// ── init_surface_for_region ───────────────────────────────────────────────────

/// Create or resize the per-window Surface for `st` to match the content area
/// implied by `region`.  Content area = region height minus chrome (title bar +
/// status strip) unless this is a system-layer window (z >= Z_DOCK).
fn init_surface_for_region(st: &mut AppState, region: (u32, u32, u32, u32)) {
    use crate::chrome::{TITLEBAR_H, Z_DOCK};
    let (_, _, ww, wh) = region;
    let is_system = st.win_z >= Z_DOCK;
    let chrome_h = if is_system { 0u32 } else { TITLEBAR_H };
    let content_h = wh.saturating_sub(chrome_h);
    if ww == 0 || content_h == 0 { return; }
    let needs_new = st.surface.as_ref()
        .map(|s| { let s = s.lock().unwrap(); s.width != ww || s.height != content_h })
        .unwrap_or(true);
    if needs_new {
        st.surface = Some(std::sync::Arc::new(std::sync::Mutex::new(
            crate::display::Surface::new(ww, content_h)
        )));
    }
}

// ── apply_tiling_layout ───────────────────────────────────────────────────────

/// Recompute tiled regions for all running display apps and write them into
/// the registry.  Called on every display-app spawn or exit.
///
/// Apps with `win_z >= Z_DOCK (100)` are pinned to a bottom strip of height
/// `DOCK_STRIP_H`.  All other display apps tile in the remaining usable area.
pub(crate) fn apply_tiling_layout(registry: &AppRegistry) {
    use supervisor::windows::compute_tiling_with_hints;
    use crate::chrome::{MENUBAR_H, Z_DOCK};

    const DOCK_STRIP_H: u32 = 64;

    const DEFAULT_SCREEN_W: u32 = 1440;
    const DEFAULT_SCREEN_H: u32 = 900;
    #[cfg(target_os = "linux")]
    let (sw, sh) = crate::display::screen_size().unwrap_or((DEFAULT_SCREEN_W, DEFAULT_SCREEN_H));
    #[cfg(not(target_os = "linux"))]
    let (sw, sh) = (DEFAULT_SCREEN_W, DEFAULT_SCREEN_H);

    // Collect all running display apps with z-layer and min-size hints.
    let apps: Vec<(String, u32, u32, u32)> = {
        let reg = registry.lock().unwrap();
        let mut v: Vec<(String, u32, u32, u32)> = reg.iter()
            .filter_map(|(name, st)| {
                let st = st.lock().unwrap();
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some((name.clone(), st.win_z, st.min_size.0, st.min_size.1))
                } else { None }
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    if apps.is_empty() { return; }

    // Partition: system-layer (dock, overlay) vs. regular tiled apps.
    let show_dock = crate::SHOW_DOCK.get().copied().unwrap_or(true);
    let (pinned, tiled): (Vec<_>, Vec<_>) = apps.iter().partition(|(_, z, _, _)| *z >= Z_DOCK);

    // Assign pinned apps to the reserved bottom strip (skipped when dock is hidden).
    let dock_h_reserved: u32 = if pinned.is_empty() || !show_dock { 0 } else { DOCK_STRIP_H };
    if show_dock {
        let reg = registry.lock().unwrap();
        for (name, _, _, _) in &pinned {
            if let Some(st) = reg.get(name.as_str()) {
                let region = (0, sh.saturating_sub(DOCK_STRIP_H), sw, DOCK_STRIP_H);
                let mut st_guard = st.lock().unwrap();
                st_guard.win_region = Some(region);
                init_surface_for_region(&mut st_guard, region);
                log_info!(Subsystem::Display, Some(name.as_str()),
                    "tiling: pinned dock strip ({},{},{},{})",
                    region.0, region.1, region.2, region.3);
            }
        }
    }

    // Tile remaining apps in usable area above the dock strip.
    if tiled.is_empty() { return; }
    let windowed = crate::WINDOWED_MODE.get().copied().unwrap_or(true);
    let menubar_h = if crate::SHOW_MENU_BAR.get().copied().unwrap_or(true) { MENUBAR_H } else { 0 };
    let usable_h = sh.saturating_sub(menubar_h).saturating_sub(dock_h_reserved);
    let regions: Vec<(u32, u32, u32, u32)> = if !windowed {
        // Fullscreen mode: every app gets the full usable area (only focused is visible).
        tiled.iter().map(|_| (0, menubar_h, sw, usable_h)).collect()
    } else {
        let min_sizes: Vec<(u32, u32)> = tiled.iter().map(|(_, _, mw, mh)| (*mw, *mh)).collect();
        compute_tiling_with_hints(tiled.len(), sw, usable_h, &min_sizes)
            .into_iter()
            .map(|(x, y, w, h)| (x, y + menubar_h, w, h))
            .collect()
    };
    {
        let reg = registry.lock().unwrap();
        for (i, (name, _, _, _)) in tiled.iter().enumerate() {
            if let Some(&region) = regions.get(i) {
                if let Some(st) = reg.get(name.as_str()) {
                    let mut st_guard = st.lock().unwrap();
                    st_guard.win_region = Some(region);
                    init_surface_for_region(&mut st_guard, region);
                    log_info!(Subsystem::Display, Some(name.as_str()),
                        "tiling: assigned ({},{},{},{})",
                        region.0, region.1, region.2, region.3);
                }
            }
        }
    }
    log_info!(Subsystem::Display, None, "layout reflow: {} tiled + {} pinned display app(s)",
        tiled.len(), pinned.len());
}

// ── spawn_io_threads ──────────────────────────────────────────────────────────

pub fn spawn_io_threads(
    name:         &str,
    child_stdin:  ChildStdin,
    child_stdout: ChildStdout,
    msg_rx:       mpsc::Receiver<String>,
    has_display:  bool,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    thread::Builder::new()
        .name(format!("{name}-writer"))
        .spawn(move || {
            let mut stdin = child_stdin;
            while let Ok(msg) = msg_rx.recv() {
                if writeln!(stdin, "{msg}").is_err() { break; }
            }
        })
        .expect("spawn writer thread");

    let inbox_r    = Arc::clone(inbox);
    let focused_r  = Arc::clone(focused);
    let registry_r = Arc::clone(app_registry);
    let name_r     = name.to_string();
    let last_output_r: Arc<Mutex<Instant>> = {
        let reg = app_registry.lock().unwrap();
        reg.get(name)
            .map(|st| Arc::clone(&st.lock().unwrap().last_output))
            .unwrap_or_else(|| Arc::new(Mutex::new(Instant::now())))
    };
    thread::Builder::new()
        .name(format!("{name}-reader"))
        .spawn(move || {
            *last_output_r.lock().unwrap() = Instant::now();
            let _ = fs::create_dir_all(LOG_DIR);
            let log_path = format!("{LOG_DIR}/{name_r}.log");
            let mut log_file = std::fs::OpenOptions::new()
                .create(true).append(true)
                .open(&log_path).ok();

            for line in BufReader::new(child_stdout).lines() {
                let line = match line { Ok(l) => l, Err(_) => break };
                *last_output_r.lock().unwrap() = Instant::now();
                if let Some(ref mut f) = log_file { let _ = writeln!(f, "{line}"); }
                {
                    let reg = registry_r.lock().unwrap();
                    if let Some(st) = reg.get(&name_r) {
                        let mut st = st.lock().unwrap();
                        st.log_buf.push_back(line.clone());
                        if st.log_buf.len() > LOG_BUF_SIZE { st.log_buf.pop_front(); }
                        // Broadcast to live log subscribers (spec-044 management server).
                        st.log_subscribers.retain(|tx| tx.send(line.clone()).is_ok());
                    }
                }
                let win_region = if has_display {
                    registry_r.lock().unwrap()
                        .get(&name_r)
                        .and_then(|st| st.lock().unwrap().win_region)
                } else { None };
                route_or_print(&line, &name_r, &inbox_r, has_display, win_region, &focused_r, &registry_r);
            }
        })
        .expect("spawn reader thread");
}

// ── launch_app_threads ────────────────────────────────────────────────────────

pub fn launch_app_threads(
    app:          SpawnedApp,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> thread::JoinHandle<()> {
    let SpawnedApp { entry, name, child, msg_rx, child_stdin, child_stdout, has_display, is_shell: _ } = app;

    if has_display {
        apply_tiling_layout(app_registry);
        #[cfg(target_os = "linux")]
        crate::chrome::repaint_all_borders(app_registry, focused);
    }

    spawn_io_threads(&name, child_stdin, child_stdout, msg_rx, has_display, inbox, focused, app_registry);

    #[cfg(target_os = "linux")]
    if has_display {
        // P29: send screen size from cached SCREEN_SIZE static, fall back to live query.
        let size = crate::SCREEN_SIZE.get().copied()
            .or_else(|| crate::display::screen_size());
        if let Some((w, h)) = size {
            if let Some(tx) = inbox.lock().unwrap().get(&name) {
                let _ = tx.send(format!("VYOMA_SYSTEM:screen:{w},{h}"));
            }
        }
        // T079: broadcast active display profile to every display app at spawn.
        let profile_kind = crate::DISPLAY_PROFILE.get().map(|s| s.as_str()).unwrap_or("desktop");
        if let Some(tx) = inbox.lock().unwrap().get(&name) {
            let _ = tx.send(format!("VYOMA_SYSTEM:display_profile:{profile_kind}"));
        }
    }

    if has_display {
        crate::chrome::z_order_push_front(&name);
    }

    let registry_w = Arc::clone(app_registry);
    let inbox_w    = Arc::clone(inbox);
    let focused_w  = Arc::clone(focused);

    thread::Builder::new()
        .name(format!("{name}-waiter"))
        .spawn(move || wait_app(entry, name, child, registry_w, inbox_w, focused_w))
        .expect("spawn waiter thread")
}

// ── spawn_app ─────────────────────────────────────────────────────────────────

pub fn spawn_app(entry: &BootEntry, inbox: &Inbox, app_registry: &AppRegistry) -> Option<SpawnedApp> {
    let manifest = match supervisor::manifest::parse_manifest(Path::new(&entry.manifest)) {
        Ok(m) => m,
        Err(e) => { log_warn!(Subsystem::Manifest, None, "{e}"); return None; }
    };

    let name = manifest.app.name.clone();
    let caps = &manifest.capabilities;
    let net_port = caps.network_port.unwrap_or(8080);

    {
        let mut wired:   Vec<&str> = Vec::new();
        let mut skipped: Vec<&str> = Vec::new();
        if caps.stdio      { wired.push("stdio")      } else { skipped.push("stdio") }
        if caps.filesystem { wired.push("filesystem")  } else { skipped.push("filesystem") }
        if caps.network    { wired.push("network")     } else { skipped.push("network") }
        if caps.display    { wired.push("display")     } else { skipped.push("display") }
        if caps.shell      { wired.push("shell")       } else { skipped.push("shell") }
        if caps.mouse      { wired.push("mouse")       } else { skipped.push("mouse") }
        let net_note = if caps.network { format!(" (port={net_port})") } else { String::new() };
        log_info!(Subsystem::Capability, Some(name.as_str()),
            "wired: {}{net_note}; skipped: {}", wired.join(" "), skipped.join(" "));
    }

    let (tx, msg_rx) = mpsc::channel::<String>();
    inbox.lock().unwrap().insert(name.clone(), tx);

    let wasm_path = Path::new(&entry.manifest)
        .parent().unwrap_or(Path::new("/apps"))
        .join(&manifest.app.wasm);

    if let Some(expected) = &manifest.app.wasm_sha256 {
        match crate::verify::verify_wasm_binary(wasm_path.to_str().unwrap_or(""), expected) {
            Ok(()) => {
                log_info!(Subsystem::Capability, Some(name.as_str()),
                    "[security] {name} wasm_sha256 verified OK");
            }
            Err(e) => {
                log_error!(Subsystem::Capability, Some(name.as_str()),
                    "SECURITY: {name} rejected — {e}");
                inbox.lock().unwrap().remove(&name);
                return None;
            }
        }
    }

    log_info!(Subsystem::Lifecycle, Some(name.as_str()),
        "spawning {} v{} (restart={})", name, manifest.app.version, entry.restart);

    let mut cmd = std::process::Command::new("/usr/bin/wasmtime");
    cmd.arg("run");
    if caps.filesystem { cmd.args(["--dir", "/data::/data"]); }
    if caps.network    { cmd.args(["-S", "inherit-network"]); }
    cmd.arg("--").arg(&wasm_path);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    cmd.env("TOKIO_WORKER_THREADS", "1");

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let filter = crate::seccomp::build();
        unsafe {
            cmd.pre_exec(move || {
                // P27: namespace isolation — call before seccomp (which denies unshare).
                if let Err(e) = crate::namespace::setup_app_namespace() {
                    // Non-fatal: log to stderr and continue without isolation.
                    eprintln!("[warn] [namespace] setup_app_namespace failed: {e}");
                }
                // P08: seccomp BPF denylist.
                crate::seccomp::apply(&filter)
            });
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log_error!(Subsystem::Lifecycle, Some(name.as_str()), "failed to spawn wasmtime for {name}: {e}");
            inbox.lock().unwrap().remove(&name);
            return None;
        }
    };

    let child_pid    = child.id();
    let child_stdin  = child.stdin.take().expect("stdin pipe");
    let child_stdout = child.stdout.take().expect("stdout pipe");
    let min_size = manifest.window.as_ref()
        .map(|wr| (wr.width.unwrap_or(0), wr.height.unwrap_or(0)))
        .unwrap_or((0, 0));
    let menu_items = manifest.menu_items.clone();

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
        win_region:       None,
        win_z:            manifest.window.as_ref().map(|w| w.z).unwrap_or(chrome::Z_APP),
        min_size,
        draw_ticks:       0,
        last_cpu_reset:   Instant::now(),
        minimized:           false,
        pre_minimize_region: None,
        // T053: pending_anim set below after construction
        pending_anim:        None,
        surface:             None,
        menu_items,
        log_subscribers:     Vec::new(),
    }));
    // T053: enqueue Open animation so the window fades in on spawn
    {
        use crate::display::animator::{Animation, AnimKind, now_ms};
        state.lock().unwrap().pending_anim =
            Some(Animation::new(AnimKind::Open, now_ms()));
    }
    app_registry.lock().unwrap().insert(name.clone(), state);

    Some(SpawnedApp { entry: entry.clone(), name, child, msg_rx, child_stdin, child_stdout,
                      has_display: caps.display, is_shell: caps.shell })
}

// ── wait_app ──────────────────────────────────────────────────────────────────

pub fn wait_app(
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
            Ok(s) => { let c = s.code().unwrap_or(-1); log_info!(Subsystem::Lifecycle, Some(name.as_str()), "{name} exited (code {c})"); c }
            Err(e) => { log_error!(Subsystem::Lifecycle, Some(name.as_str()), "wait failed for {name}: {e}"); -1 }
        };

        let (had_display, old_win_region) = {
            let reg = app_registry.lock().unwrap();
            if let Some(st) = reg.get(&name) {
                let mut st = st.lock().unwrap();
                st.status = AppStatus::Stopped(exit_code);
                st.child_pid = None;
                (st.has_display, st.win_region)
            } else { (false, None) }
        };

        #[cfg(target_os = "linux")]
        if let (true, Some((wx, wy, ww, wh))) = (had_display, old_win_region) {
            if let Some(fb_lock) = crate::display::get() {
                let mut fb = fb_lock.lock().unwrap();
                fb.fill_rect(wx, wy, ww, wh, 0x1C1C1EFF);
                fb.flush();
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = old_win_region;

        crate::toast::auto_transfer_focus(&name, &app_registry, &focused);

        if had_display {
            apply_tiling_layout(&app_registry);
            #[cfg(target_os = "linux")]
            crate::chrome::repaint_all_borders(&app_registry, &focused);
        }

        let should_restart = matches!(entry.restart.as_str(), "always" | "on-failure")
            && (entry.restart == "always" || exit_code != 0);

        if !should_restart {
            if exit_code != 0 { crate::toast::show_crash_toast(&name, exit_code); }
            break;
        }

        log_info!(Subsystem::Lifecycle, Some(name.as_str()),
            "restarting {name} (policy={}, count={})", entry.restart, restart_count + 1);

        match spawn_app(&entry, &inbox, &app_registry) {
            Some(app) => {
                restart_count += 1;
                { let reg = app_registry.lock().unwrap();
                  if let Some(st) = reg.get(&name) { st.lock().unwrap().restart_count = restart_count; } }
                if app.has_display {
                    apply_tiling_layout(&app_registry);
                    #[cfg(target_os = "linux")]
                    crate::chrome::repaint_all_borders(&app_registry, &focused);
                }
                let SpawnedApp { child: new_child, child_stdin, child_stdout, msg_rx, has_display, .. } = app;
                spawn_io_threads(&name, child_stdin, child_stdout, msg_rx, has_display, &inbox, &focused, &app_registry);
                child = new_child;
            }
            None => { log_error!(Subsystem::Lifecycle, Some(name.as_str()), "failed to restart {name}, giving up"); break; }
        }
    }

    // Remove from registry so Z_ORDER cleanup is consistent
    let _ = LAST_SENDER.get().and_then(|m| m.lock().ok()).map(|mut m| m.remove(&name));
}

// ── run_watchdog ──────────────────────────────────────────────────────────────

/// P19: watchdog loop — kills any app that has been silent longer than its
/// configured `watchdog_secs`.  Runs forever in its own named thread.
pub fn run_watchdog(registry: AppRegistry) {
    loop {
        thread::sleep(std::time::Duration::from_secs(1));
        let reg = registry.lock().unwrap();
        for (name, state_arc) in reg.iter() {
            let st = state_arc.lock().unwrap();
            let wsecs = st.watchdog_secs;
            if wsecs == 0 { continue; }
            {
                let mut backoff = st.watchdog_backoff.lock().unwrap();
                if *backoff > 0 { *backoff -= 1; continue; }
            }
            let elapsed = st.last_output.lock().unwrap().elapsed();
            if elapsed.as_secs() >= wsecs as u64 {
                if let Some(pid) = st.child_pid {
                    log_warn!(Subsystem::Lifecycle, Some(name.as_str()),
                        "silent for {}s (limit={wsecs}s) — killing pid {pid}", elapsed.as_secs());
                    crate::toast::show_crash_toast(name, -1);
                    #[cfg(target_os = "linux")]
                    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                }
                let restarts = st.restart_count;
                *st.watchdog_backoff.lock().unwrap() = watchdog_next_backoff(wsecs, restarts);
                *st.last_output.lock().unwrap() = Instant::now();
            }
        }
    }
}
