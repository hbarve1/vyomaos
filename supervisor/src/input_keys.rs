// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Keyboard shortcut classification, shortcut-help overlay, and input-router thread.

use crate::{AppRegistry, FocusedApp, Inbox};
use crate::font;
use supervisor::logging::Subsystem;

/// Decoded action for a raw TTY input byte sequence.
#[derive(Debug, PartialEq)]
pub enum InputAction {
    AltTab,       // Alt+Tab      — cycle focus forward
    AltShiftTab,  // Alt+Shift+Tab — cycle focus backward
    AltW,         // Alt+W        — close focused window
    AltF4,        // Alt+F4       — close/kill focused app (P82)
    AltF,         // Alt+F        — toggle fullscreen for focused window (P82)
    AltQuestion,  // Alt+?        — show keyboard shortcut overlay toast
    #[allow(dead_code)]
    CtrlL,        // Ctrl+L       — lock the screen (handled as raw 0x0c byte)
    PassThrough,  // Everything else: forward to the focused app as-is
}

/// Classify a raw input byte sequence (starting from the first byte, including ESC).
///
/// Recognised sequences:
///   `[0x1B, 0x09]`        → AltTab        (ESC + TAB)
///   `[0x1B, 0x5B, 0x5A]`  → AltShiftTab   (ESC + [ + Z  i.e. \x1b[Z)
///   `[0x1B, 0x77]`        → AltW          (ESC + 'w')
///   `[0x1B, 0x5B, 0x31, 0x34, 0x7E]` → AltF4 (ESC + CSI F4 i.e. \x1b[14~)
///   `[0x1B, 0x66]`        → AltF          (ESC + 'f')
///   `[0x1B, 0x3F]`        → AltQuestion   (ESC + '?')
///   anything else         → PassThrough
pub fn classify_input_sequence(bytes: &[u8]) -> InputAction {
    match bytes {
        [0x1B, 0x09]                    => InputAction::AltTab,
        [0x1B, 0x5B, 0x5A]             => InputAction::AltShiftTab,
        [0x1B, 0x77]                    => InputAction::AltW,
        // Alt+F4: ESC + CSI sequence for F4 = \x1b[14~
        [0x1B, 0x5B, 0x31, 0x34, 0x7E] => InputAction::AltF4,
        [0x1B, 0x66]                    => InputAction::AltF,
        [0x1B, 0x3F]                    => InputAction::AltQuestion,
        _                               => InputAction::PassThrough,
    }
}

/// Return the static help text listing all window-management keyboard shortcuts.
///
/// Pure function — no side effects, no allocation.
pub fn shortcut_help_text() -> &'static str {
    "Alt+Tab: next  Alt+W/F4: close  Alt+F: fullscreen  Ctrl+S: screenshot  Alt+?: help"
}

/// Draw a 3-second shortcut-help toast in the bottom-right corner of the screen.
///
/// Renders a filled panel with a border and the shortcut help text, then spawns
/// a background thread to clear the region after 3 seconds.
///
/// Only compiled on Linux (where `/dev/fb0` is available).
#[cfg(target_os = "linux")]
pub fn show_shortcut_overlay() {
    use crate::display;
    use std::thread;

    const OW: u32 = 500;
    const OH: u32 = 60;
    if let Some((screen_w, screen_h)) = display::screen_size() {
        let ox: u32 = screen_w.saturating_sub(OW + 20);
        let oy: u32 = screen_h.saturating_sub(OH + 20);
        if let Some(fb_lock) = display::get() {
            let mut fb = fb_lock.lock().unwrap();
            fb.fill_rect(ox, oy, OW, OH, 0x21262DFF);
            fb.rect_border(ox, oy, OW, OH, 0x58A6FFFF);
            fb.draw_text(ox + 8, oy + 8,  "Keyboard Shortcuts", 0xFFFFFFFF, font::FontSize::Medium);
            fb.draw_text(ox + 8, oy + 28, shortcut_help_text(),  0x8B949EFF, font::FontSize::Medium);
            fb.flush();
        }
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(3));
            if let Some(fb_lock) = display::get() {
                let mut fb = fb_lock.lock().unwrap();
                fb.fill_rect(ox, oy, OW, OH, 0x0D1117FF);
                fb.flush();
            }
        });
    }
}

/// P82: Kill the currently focused app and move focus to the next windowed app.
#[cfg(target_os = "linux")]
fn handle_alt_f4_close(focused: &FocusedApp, registry: &AppRegistry, _inbox: &Inbox) {
    use crate::{log_info, chrome};

    let focused_name = focused.lock().unwrap().clone();
    if let Some(ref name) = focused_name {
        let pid = {
            let reg = registry.lock().unwrap();
            reg.get(name).and_then(|st| st.lock().unwrap().child_pid)
        };
        if let Some(pid) = pid {
            log_info!(Subsystem::Input, Some(name.as_str()), "alt+f4: closing {name}");
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
        }
        // Move focus to next remaining windowed app
        let names: Vec<String> = {
            let reg = registry.lock().unwrap();
            let mut v: Vec<String> = reg.iter()
                .filter(|(n, st)| n.as_str() != name.as_str()
                    && st.lock().unwrap().win_region.is_some())
                .map(|(n, _)| n.clone()).collect();
            v.sort(); v
        };
        let next = names.into_iter().next();
        if let Some(ref n) = next {
            chrome::z_order_push_front(n);
        }
        *focused.lock().unwrap() = next;
    }
}

/// P82: Toggle the focused app between fullscreen and its previous tiled region.
#[cfg(target_os = "linux")]
fn handle_fullscreen_toggle(
    focused: &FocusedApp,
    registry: &AppRegistry,
) {
    use crate::{log_info, display};
    use crate::chrome::{MENUBAR_H, repaint_all_borders};

    let focused_name = focused.lock().unwrap().clone();
    let Some(ref name) = focused_name else { return };
    let (sw, sh) = display::screen_size().unwrap_or((1920, 1080));

    let reg = registry.lock().unwrap();
    let Some(st_arc) = reg.get(name) else { return };
    let mut st = st_arc.lock().unwrap();

    if st.is_fullscreen {
        // Restore to pre-fullscreen region
        if let Some(prev) = st.pre_fullscreen_region.take() {
            st.win_region = Some(prev);
        }
        st.is_fullscreen = false;
        log_info!(Subsystem::Input, Some(name.as_str()), "alt+f: restored {name} from fullscreen");
    } else {
        // Save current region and go fullscreen
        st.pre_fullscreen_region = st.win_region;
        let fs_region = (0, MENUBAR_H, sw, sh.saturating_sub(MENUBAR_H));
        st.win_region = Some(fs_region);
        st.is_fullscreen = true;
        log_info!(Subsystem::Input, Some(name.as_str()), "alt+f: fullscreen {name}");
    }
    drop(st);
    drop(reg);
    repaint_all_borders(registry, focused);
}

/// P17T01: body of the input-router thread — raw /dev/tty0 → focused app dispatch.
///
/// Only compiled on Linux. Spawned by `main()` as a named thread.
#[cfg(target_os = "linux")]
pub fn run_input_router(inbox: Inbox, focused: FocusedApp, registry: AppRegistry) {
    use crate::chrome::{cycle_focus_backward, cycle_focus_forward, repaint_all_borders,
                        windowed_apps_sorted, MENUBAR_H};
    use crate::{log_error, log_info, log_warn};
    use crate::display;
    use std::io::Read;
    use std::os::unix::io::AsRawFd;

    let mut tty = match std::fs::File::open("/dev/tty0") {
        Ok(f) => f,
        Err(e) => { log_error!(Subsystem::Input, None, "cannot open /dev/tty0: {e}"); return; }
    };

    let fd = tty.as_raw_fd();
    let raw_ok = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) == 0 {
            t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHOE | libc::ECHOK
                         | libc::ECHONL | libc::ISIG | libc::IEXTEN);
            t.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
            t.c_cflag |= libc::CS8;
            t.c_cc[libc::VMIN  as usize] = 1;
            t.c_cc[libc::VTIME as usize] = 0;
            libc::tcsetattr(fd, libc::TCSANOW, &t) == 0
        } else { false }
    };
    if raw_ok { log_info!(Subsystem::Input, None, "input-router: raw tty mode active"); }
    else      { log_warn!(Subsystem::Input, None, "input-router: raw mode unavailable, using line mode"); }

    let mut buf = [0u8; 1];
    loop {
        match tty.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                // T062: When dropdown is open, intercept keys for navigation.
                if crate::chrome::is_dropdown_open() {
                    let key_str = if buf[0] == 0x1B {
                        let mut b1 = [0u8; 1];
                        if tty.read(&mut b1).unwrap_or(0) == 0 { continue; }
                        if b1[0] == 0x5B {
                            let mut b2 = [0u8; 1];
                            if tty.read(&mut b2).unwrap_or(0) == 0 { continue; }
                            match b2[0] {
                                0x41 => "\x1b[A".to_string(), // ArrowUp
                                0x42 => "\x1b[B".to_string(), // ArrowDown
                                _    => "\x1b".to_string(),
                            }
                        } else {
                            "\x1b".to_string() // plain Escape
                        }
                    } else if buf[0] == 0x0D || buf[0] == 0x0A {
                        "\r".to_string()
                    } else {
                        String::from(buf[0] as char)
                    };
                    if let Some((app, action)) = crate::chrome::handle_dropdown_key_action(&key_str) {
                        // Send the menu action as IPC to the app
                        let msg = format!("VYOMA_SYSTEM:menu_action:{action}");
                        if let Some(tx) = inbox.lock().unwrap().get(&app) {
                            let _ = tx.send(msg);
                        }
                    }
                    continue;
                }

                // When locked, route all input to screen-lock only (except Ctrl+L)
                if crate::is_locked() && buf[0] != 0x0c {
                    let msg: Option<String> = match buf[0] {
                        0x0D | 0x0A => Some(String::new()),
                        0x7F | 0x08 => Some("\x7f".to_string()),
                        0x20..=0x7E => Some(String::from(buf[0] as char)),
                        _           => None,
                    };
                    if let Some(msg) = msg {
                        if let Some(tx) = inbox.lock().unwrap().get("screen-lock") {
                            let _ = tx.send(msg);
                        }
                    }
                    continue;
                }

                if buf[0] == 0x1B {
                    let mut b1 = [0u8; 1];
                    if tty.read(&mut b1).unwrap_or(0) == 0 { continue; }

                    if b1[0] == 0x5B {
                        let mut b2 = [0u8; 1];
                        if tty.read(&mut b2).unwrap_or(0) == 0 { continue; }

                        // Ctrl+Arrow: \x1b[1;5C (right) / \x1b[1;5D (left)
                        // F4: \x1b[14~ (Alt+F4 close, P82)
                        // Read extended CSI: b2='1', then ';', '5', final char
                        if b2[0] == b'1' {
                            let mut ext = [0u8; 3];
                            let n = tty.read(&mut ext).unwrap_or(0);
                            // F4 key: \x1b[14~ → ext = ['4', '~', ...]
                            if n >= 2 && ext[0] == b'4' && ext[1] == b'~' {
                                // P82: Alt+F4 / F4 → close focused app
                                handle_alt_f4_close(&focused, &registry, &inbox);
                                continue;
                            }
                            if n == 3 && ext[0] == b';' && ext[1] == b'5' {
                                match ext[2] {
                                    b'C' => {
                                        // Ctrl+Right → next workspace
                                        let ws = crate::workspace::switch_next();
                                        log_info!(Subsystem::Input, None,
                                            "ctrl+right: workspace → {}", ws);
                                        crate::draw_cmd::force_repaint(&registry, &focused);
                                        continue;
                                    }
                                    b'D' => {
                                        // Ctrl+Left → previous workspace
                                        let ws = crate::workspace::switch_prev();
                                        log_info!(Subsystem::Input, None,
                                            "ctrl+left: workspace → {}", ws);
                                        crate::draw_cmd::force_repaint(&registry, &focused);
                                        continue;
                                    }
                                    _ => {} // unknown modifier combo, fall through
                                }
                            }
                            // Not a recognized Ctrl+Arrow, discard consumed bytes
                            continue;
                        }

                        let fwd: Option<&str> = match b2[0] {
                            0x41 => Some("\x1b[A"),
                            0x42 => Some("\x1b[B"),
                            _    => None,
                        };
                        if let Some(msg) = fwd {
                            if let Some(name) = focused.lock().unwrap().clone() {
                                if let Some(tx) = inbox.lock().unwrap().get(&name) {
                                    let _ = tx.send(msg.to_string());
                                }
                            }
                        } else if let InputAction::AltShiftTab =
                            classify_input_sequence(&[0x1B, 0x5B, b2[0]])
                        {
                            let names = windowed_apps_sorted(&registry);
                            if !names.is_empty() {
                                let cur = focused.lock().unwrap().clone();
                                if let Some(ref name) = cycle_focus_backward(&names, cur.as_deref()) {
                                    log_info!(Subsystem::Input, Some(name.as_str()), "alt+shift+tab: focus → {name}");
                                    crate::chrome::z_order_push_front(name);
                                    *focused.lock().unwrap() = Some(name.clone());
                                    repaint_all_borders(&registry, &focused);
                                }
                            }
                        }
                    } else {
                        match classify_input_sequence(&[0x1B, b1[0]]) {
                            InputAction::AltTab => {
                                let names = windowed_apps_sorted(&registry);
                                if !names.is_empty() {
                                    let cur = focused.lock().unwrap().clone();
                                    if let Some(ref name) = cycle_focus_forward(&names, cur.as_deref()) {
                                        log_info!(Subsystem::Input, Some(name.as_str()), "alt+tab: focus → {name}");
                                        crate::chrome::z_order_push_front(name);
                                        *focused.lock().unwrap() = Some(name.clone());
                                        repaint_all_borders(&registry, &focused);
                                    }
                                }
                            }
                            InputAction::AltW => {
                                let focused_name = focused.lock().unwrap().clone();
                                if let Some(ref name) = focused_name {
                                    let pid = { let reg = registry.lock().unwrap();
                                        reg.get(name).and_then(|st| st.lock().unwrap().child_pid) };
                                    if let Some(pid) = pid {
                                        log_info!(Subsystem::Input, Some(name.as_str()), "alt+w: closing {name}");
                                        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                                    }
                                    let names: Vec<String> = {
                                        let reg = registry.lock().unwrap();
                                        let mut v: Vec<String> = reg.iter()
                                            .filter(|(n, st)| n.as_str() != name.as_str()
                                                && st.lock().unwrap().win_region.is_some())
                                            .map(|(n, _)| n.clone()).collect();
                                        v.sort(); v
                                    };
                                    *focused.lock().unwrap() = names.into_iter().next();
                                }
                            }
                            InputAction::AltF => {
                                // P82: toggle fullscreen for focused app
                                handle_fullscreen_toggle(&focused, &registry);
                            }
                            InputAction::AltQuestion => {
                                log_info!(Subsystem::Input, None, "alt+?: showing shortcut overlay");
                                show_shortcut_overlay();
                            }
                            _ => {}
                        }
                    }
                } else if buf[0] == 0x03 {
                    // Ctrl+C: send copy signal to focused app
                    if let Some(name) = focused.lock().unwrap().clone() {
                        let msg = supervisor::ipc::format_copy_signal();
                        if let Some(tx) = inbox.lock().unwrap().get(&name) {
                            let _ = tx.send(msg);
                        }
                    }
                } else if buf[0] == 0x0c {
                    // Ctrl+L: lock screen
                    if !crate::is_locked() {
                        // Synthesize a lock command via IPC
                        if let Some(tx) = inbox.lock().unwrap().values().next().cloned() {
                            // Route lock through supervisor command handler directly
                            drop(tx);
                        }
                        crate::set_locked(true);
                        // Launch screen-lock app
                        let entry = supervisor::manifest::BootEntry {
                            manifest: "/apps/screen-lock/vyoma.toml".to_string(),
                            restart: "never".to_string(),
                        };
                        let already_running = {
                            let reg = registry.lock().unwrap();
                            reg.get("screen-lock").map(|st| {
                                matches!(st.lock().unwrap().status, crate::AppStatus::Running)
                            }).unwrap_or(false)
                        };
                        if !already_running {
                            if let Some(app) = crate::app_threads::spawn_app(&entry, &inbox, &registry) {
                                crate::app_threads::launch_app_threads(app, &inbox, &focused, &registry);
                            }
                        }
                        crate::chrome::z_order_push_front("screen-lock");
                        *focused.lock().unwrap() = Some("screen-lock".to_string());
                        log_info!(Subsystem::Lifecycle, None, "screen locked via Ctrl+L");
                    }
                } else if buf[0] == 0x13 {
                    // Ctrl+S: capture screenshot
                    #[cfg(target_os = "linux")]
                    {
                        use crate::display;
                        if let Some(fb_lock) = display::get() {
                            let fb = fb_lock.lock().unwrap();
                            match crate::screenshot::capture_screenshot(&fb) {
                                Ok(path) => {
                                    log_info!(Subsystem::Display, None, "screenshot saved to {path}");
                                    crate::toast::enqueue_banner(
                                        "supervisor",
                                        "Screenshot",
                                        &format!("Saved to {path}"),
                                    );
                                }
                                Err(e) => {
                                    log_error!(Subsystem::Display, None, "screenshot failed: {e}");
                                }
                            }
                        }
                    }
                } else if buf[0] == 0x1A {
                    // Ctrl+Z: undo last reversible command
                    log_info!(Subsystem::Input, None, "ctrl+z: undo");
                    crate::ipc_handlers::handle_supervisor_command(
                        "undo", "keyboard", &inbox, &focused, &registry,
                    );
                } else if buf[0] == 0x16 {
                    // Ctrl+V: paste clipboard contents to focused app
                    if let Some(name) = focused.lock().unwrap().clone() {
                        let text = crate::CLIPBOARD.get()
                            .map(|c| c.lock().unwrap().clone())
                            .unwrap_or_default();
                        let msg = supervisor::ipc::format_paste_message(&text);
                        if let Some(tx) = inbox.lock().unwrap().get(&name) {
                            let _ = tx.send(msg);
                        }
                    }
                } else {
                    let msg: Option<String> = match buf[0] {
                        0x0D | 0x0A => Some(String::new()),
                        0x7F | 0x08 => Some("\x7f".to_string()),
                        0x20..=0x7E => Some(String::from(buf[0] as char)),
                        _           => None,
                    };
                    if let Some(msg) = msg {
                        if let Some(name) = focused.lock().unwrap().clone() {
                            if let Some(tx) = inbox.lock().unwrap().get(&name) {
                                let _ = tx.send(msg);
                            }
                        }
                    }
                }
            }
        }
    }
}

// Suppress unused-import warnings on non-Linux builds.
#[cfg(not(target_os = "linux"))]
fn _dummy_imports() {
    let _: fn(&AppRegistry) = |_| {};
    let _: fn(&FocusedApp)  = |_| {};
    let _: fn(&Inbox)       = |_| {};
    let _: fn(Subsystem)    = |_| {};
    let _: fn(&font::FontSize) = |_| {};
}
