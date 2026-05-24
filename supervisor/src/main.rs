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

use sha2::{Digest, Sha256};

use supervisor::lifecycle::format_ps_line;
use supervisor::logging::{format_log, Level, Subsystem};
use supervisor::manifest::{AppManifest, BootConfig, BootEntry};

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
    // Return ENOSYS (38) so glibc falls back from clone3 → clone when threading.
    const SECCOMP_RET_ERRNO_ENOSYS: u32 = 0x0005_0000 | 38;

    const OFF_NR: u32 = 0;
    const OFF_ARCH: u32 = 4;

    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

    // Syscalls that return ENOSYS so libc falls back gracefully.
    const ENOSYS_FALLBACK: &[u32] = &[
        435, // clone3 — glibc falls back to clone(2) when clone3 returns ENOSYS
    ];

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
        for &nr in ENOSYS_FALLBACK {
            f.push(jump!(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1));
            f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_ERRNO_ENOSYS));
        }
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

// ── Window-management keyboard shortcuts ──────────────────────────────────────

/// Decoded action for a raw TTY input byte sequence.
#[derive(Debug, PartialEq)]
enum InputAction {
    AltTab,       // Alt+Tab      — cycle focus forward
    AltShiftTab,  // Alt+Shift+Tab — cycle focus backward
    AltW,         // Alt+W        — close focused window
    AltF,         // Alt+F        — maximize focused window (stub)
    AltQuestion,  // Alt+?        — show keyboard shortcut overlay toast
    PassThrough,  // Everything else: forward to the focused app as-is
}

/// Classify a raw input byte sequence (starting from the first byte, including ESC).
///
/// Recognised sequences:
///   `[0x1B, 0x09]`        → AltTab        (ESC + TAB)
///   `[0x1B, 0x5B, 0x5A]`  → AltShiftTab   (ESC + [ + Z  i.e. \x1b[Z)
///   `[0x1B, 0x77]`        → AltW          (ESC + 'w')
///   `[0x1B, 0x66]`        → AltF          (ESC + 'f')
///   `[0x1B, 0x3F]`        → AltQuestion   (ESC + '?')
///   anything else         → PassThrough
fn classify_input_sequence(bytes: &[u8]) -> InputAction {
    match bytes {
        [0x1B, 0x09]        => InputAction::AltTab,
        [0x1B, 0x5B, 0x5A] => InputAction::AltShiftTab,
        [0x1B, 0x77]        => InputAction::AltW,
        [0x1B, 0x66]        => InputAction::AltF,
        [0x1B, 0x3F]        => InputAction::AltQuestion,
        _                   => InputAction::PassThrough,
    }
}

/// Return the static help text listing all window-management keyboard shortcuts.
///
/// Pure function — no side effects, no allocation.
pub fn shortcut_help_text() -> &'static str {
    "Alt+Tab: next window  Alt+W: close  Alt+F: snap  Alt+?: help"
}

/// Draw a 3-second shortcut-help toast in the bottom-right corner of the screen.
///
/// Renders a filled panel with a border and the shortcut help text, then spawns
/// a background thread to clear the region after 3 seconds.
///
/// Only compiled on Linux (where `/dev/fb0` is available).
#[cfg(target_os = "linux")]
fn show_shortcut_overlay() {
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

// ── macOS-inspired chrome ─────────────────────────────────────────────────────

const MENUBAR_H:       u32 = 24;   // global menu bar height
const TITLEBAR_H:      u32 = 28;   // per-window title bar height
const STATUS_H:        u32 = 16;   // per-window status strip height (bottom of window)
const TL_DOT:          u32 = 12;   // traffic-light dot size (px)

const MAC_MENUBAR:        u32 = 0x1C1C1EFF; // system background (menubar)
const MAC_TITLE_ACT:      u32 = 0x3A3A3CFF; // active window title bar
const MAC_TITLE_INACT:    u32 = 0x2C2C2EFF; // inactive window title bar
const MAC_TITLE_HOVER:    u32 = 0x444C56FF; // hovered title bar (midpoint between active and inactive)
const MAC_SEP:            u32 = 0x48484AFF; // separator line
const MAC_LABEL:       u32 = 0xFFFFFFFF; // primary label (white)
const MAC_LABEL2:      u32 = 0x8E8E93FF; // secondary label (gray)
const TL_CLOSE:        u32 = 0xFF5F57FF; // traffic light red
const TL_MINIMIZE:     u32 = 0xFEBC2EFF; // traffic light yellow
const TL_MAXIMIZE:     u32 = 0x28C840FF; // traffic light green
const TL_GRAY:         u32 = 0x4D4D4DFF; // inactive traffic lights

// Kept for repaint compat; unused after chrome redesign.
#[allow(dead_code)] const BORDER_FOCUSED:   u32 = 0x89B4FAFF;
#[allow(dead_code)] const BORDER_UNFOCUSED: u32 = 0x45475AFF;

// ── US1: tiled window layout helpers ─────────────────────────────────────────

/// Return names of all Running display-capable apps, sorted for deterministic layout.
/// Recompute tiled regions for all running display apps and write them into
/// the registry.  Called on every display-app spawn or exit.
fn apply_tiling_layout(registry: &AppRegistry) {
    use supervisor::windows::compute_tiling_with_hints;

    let apps: Vec<(String, u32, u32)> = {
        let reg = registry.lock().unwrap();
        let mut v: Vec<(String, u32, u32)> = reg.iter()
            .filter_map(|(name, st)| {
                let st = st.lock().unwrap();
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some((name.clone(), st.min_size.0, st.min_size.1))
                } else {
                    None
                }
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };

    if apps.is_empty() { return; }

    #[cfg(target_os = "linux")]
    let (sw, sh) = display::screen_size().unwrap_or((1440, 900));
    #[cfg(not(target_os = "linux"))]
    let (sw, sh) = (1440u32, 900u32);

    let min_sizes: Vec<(u32, u32)> = apps.iter().map(|(_, mw, mh)| (*mw, *mh)).collect();
    // Reserve MENUBAR_H pixels at the top; shift all regions down accordingly.
    let usable_h = sh.saturating_sub(MENUBAR_H);
    let regions: Vec<(u32, u32, u32, u32)> = compute_tiling_with_hints(apps.len(), sw, usable_h, &min_sizes)
        .into_iter()
        .map(|(x, y, w, h)| (x, y + MENUBAR_H, w, h))
        .collect();

    {
        let reg = registry.lock().unwrap();
        for (i, (name, _, _)) in apps.iter().enumerate() {
            if let Some(st) = reg.get(name) {
                if let Some(&region) = regions.get(i) {
                    let mut st = st.lock().unwrap();
                    st.win_region = Some(region);
                    log_info!(Subsystem::Display, Some(name.as_str()),
                        "tiling: assigned ({},{},{},{})", region.0, region.1, region.2, region.3);
                }
            }
        }
    }

    let n = apps.len();
    log_info!(Subsystem::Display, None, "layout reflow: {n} display app(s) tiled");
}

/// Return the title bar background colour for the given window state.
///
/// Priority: hovered > focused > inactive; `hovered` wins regardless of focus.
///
/// Pure function — no side-effects, no global state reads.
pub fn titlebar_color_for_state(focused: bool, hovered: bool) -> u32 {
    if hovered      { MAC_TITLE_HOVER } // hover — slightly lighter for affordance
    else if focused { MAC_TITLE_ACT   } // active window
    else            { MAC_TITLE_INACT } // inactive window
}

/// Draw a macOS-style title bar at (wx, wy, ww, TITLEBAR_H).
/// Traffic lights are colored when focused, gray otherwise.
/// App name is centered in the bar.
/// `is_hovered` makes the background slightly lighter for mouse-hover affordance.
#[cfg(target_os = "linux")]
fn draw_titlebar(fb: &mut display::Framebuffer, wx: u32, wy: u32, ww: u32, is_focused: bool, is_hovered: bool, name: &str) {
    let bg = titlebar_color_for_state(is_focused, is_hovered);
    fb.fill_rect(wx, wy, ww, TITLEBAR_H, bg);
    fb.fill_rect(wx, wy + TITLEBAR_H - 1, ww, 1, MAC_SEP);

    // 2px focus border — drawn around the full window frame (titlebar + content)
    let bc = display::border_color(is_focused);
    // top edge
    fb.fill_rect(wx, wy, ww, 2, bc);
    // bottom edge
    if wh >= 2 { fb.fill_rect(wx, wy + wh - 2, ww, 2, bc); }
    // left edge
    fb.fill_rect(wx, wy, 2, wh, bc);
    // right edge
    if ww >= 2 { fb.fill_rect(wx + ww - 2, wy, 2, wh, bc); }

    // Traffic lights — 12×12, left-aligned, vertically centered
    let tl_y = wy + (TITLEBAR_H - TL_DOT) / 2;
    let (c1, c2, c3) = if is_focused {
        (TL_CLOSE, TL_MINIMIZE, TL_MAXIMIZE)
    } else {
        (TL_GRAY, TL_GRAY, TL_GRAY)
    };
    fb.fill_rect(wx + 8,  tl_y, TL_DOT, TL_DOT, c1);
    fb.fill_rect(wx + 24, tl_y, TL_DOT, TL_DOT, c2);
    fb.fill_rect(wx + 40, tl_y, TL_DOT, TL_DOT, c3);

    // Accent color dot — deterministic per-app identity marker (12×12 at x+60)
    let accent = display::app_accent_color(name);
    fb.fill_rect(wx + 60, tl_y, TL_DOT, TL_DOT, accent);

    // App name centered (medium font = 8 px/char, 16 px tall)
    let nlen = name.len().min(20) as u32;  // cap to avoid overflow
    let name_w = nlen * 8;
    if ww > name_w + 60 {
        let nx = wx + (ww - name_w) / 2;
        let ny = wy + (TITLEBAR_H - 16) / 2;
        let col = if is_focused { MAC_LABEL } else { MAC_LABEL2 };
        fb.draw_text(nx, ny, &name[..name.len().min(20)], col, font::FontSize::Medium);
    }
}

/// Draw the global menu bar at y=0 across the full screen width.
/// Shows "VyomaOS" on the left, app-switcher labels after it, focused app in
/// the center, and clock on the right.
///
/// `apps` is an ordered slice of display-app names drawn as clickable labels
/// immediately after the brand name.  The same ordering is used by
/// [`supervisor::windows::menubar_hit_app`] for click-to-focus hit detection.
#[cfg(target_os = "linux")]
fn draw_menubar(
    fb: &mut display::Framebuffer,
    sw: u32,
    elapsed_secs: u64,
    focused: Option<&str>,
    apps: &[String],
) {
    use supervisor::windows::{MENUBAR_APPS_START_X, menubar_label_width};

    fb.fill_rect(0, 0, sw, MENUBAR_H, MAC_MENUBAR);
    fb.fill_rect(0, MENUBAR_H - 1, sw, 1, MAC_SEP);

    let ty = (MENUBAR_H - 16) / 2; // vertical center of 16-px font in 24-px bar

    // Left: brand
    fb.draw_text(12, ty, "VyomaOS", MAC_LABEL, font::FontSize::Medium);

    // App-switcher labels: drawn immediately after the brand name.
    // Highlighted (white) when focused, dimmed otherwise.
    let mut lx = MENUBAR_APPS_START_X as u32;
    for app in apps {
        let is_focused = focused.map_or(false, |f| f == app.as_str());
        let color = if is_focused { MAC_LABEL } else { MAC_LABEL2 };
        fb.draw_text(lx + 8, ty, app, color, font::FontSize::Medium);
        lx += menubar_label_width(app.len()) as u32;
    }

    // Center: focused app name
    if let Some(name) = focused {
        let nlen = name.len().min(20) as u32;
        let nx = sw.saturating_sub(nlen * 8) / 2;
        fb.draw_text(nx, ty, &name[..name.len().min(20)], MAC_LABEL, font::FontSize::Medium);
    }

    // Right: elapsed clock HH:MM:SS
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;
    let clock = format!("{h:02}:{m:02}:{s:02}");
    let cw = clock.len() as u32 * 8;
    if sw > cw + 20 {
        fb.draw_text(sw - cw - 12, ty, &clock, MAC_LABEL2, font::FontSize::Medium);
    }
}

/// Draw a 16px status strip at the very bottom of a window's chrome.
/// Background: `0x161B22FF` (dark).  Text: `0x8B949EFF` (grey).
/// Shows `[name]  up <uptime_secs>s` in small font, left-aligned with 4px inset.
#[cfg(target_os = "linux")]
fn draw_statusbar(
    fb:           &mut display::Framebuffer,
    name:         &str,
    uptime_secs:  u64,
    wx:           u32,
    wy:           u32,
    ww:           u32,
    wh:           u32,
) {
    const STATUS_BG:   u32 = 0x161B22FF; // very dark navy background
    const STATUS_FG:   u32 = 0x8B949EFF; // muted grey text

    // Fill the status strip
    let sy = wy + wh - STATUS_H;
    fb.fill_rect(wx, sy, ww, STATUS_H, STATUS_BG);

    // Build and draw the label using the public helper
    let label = supervisor::statusbar::format_status_text(name, uptime_secs);
    // Vertically center 8px small font in the 16px strip: (16 - 8) / 2 = 4
    fb.draw_text(wx + 4, sy + 4, &label, STATUS_FG, font::FontSize::Small);
}

/// Immediately repaint title bars for all windowed apps (called on focus changes).
fn repaint_all_borders(registry: &AppRegistry, focused: &FocusedApp) {
    let focused_name = focused.lock().unwrap().clone();
    let hovered_name = HOVERED_APP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .clone();
    let (regions, display_apps): (Vec<(String, (u32, u32, u32, u32))>, Vec<String>) = {
        let reg = registry.lock().unwrap();
        let mut regions = Vec::new();
        let mut display_apps: Vec<String> = reg.iter()
            .filter_map(|(name, st)| {
                let st = st.lock().unwrap();
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        display_apps.sort();
        for (name, st) in reg.iter() {
            let st = st.lock().unwrap();
            if let Some(r) = st.win_region {
                regions.push((name.clone(), r));
            }
        }
        (regions, display_apps)
    };
    let Some(fb_lock) = display::get() else { return };
    let mut fb = fb_lock.lock().unwrap();
    for (name, (wx, wy, ww, wh)) in &regions {
        if *ww < 60 { continue; }
        let is_focused = focused_name.as_deref() == Some(name.as_str());
        let is_hovered = hovered_name.as_deref() == Some(name.as_str());
        #[cfg(target_os = "linux")]
        draw_titlebar(&mut *fb, *wx, *wy, *ww, is_focused, is_hovered, name);
    }
    #[cfg(target_os = "linux")]
    {
        let sw = fb.width;
        let elapsed = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
        draw_menubar(&mut *fb, sw, elapsed, focused_name.as_deref(), &display_apps);
    }
}

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

// ── Window-management focus helpers ──────────────────────────────────────────

/// Return all windowed app names (those with `win_region = Some(_)`) sorted alphabetically.
fn windowed_apps_sorted(registry: &AppRegistry) -> Vec<String> {
    let reg = registry.lock().unwrap();
    let mut v: Vec<String> = reg.iter()
        .filter(|(_, st)| st.lock().unwrap().win_region.is_some())
        .map(|(n, _)| n.clone())
        .collect();
    v.sort();
    v
}

/// Advance focus to the next app in `names` (wrapping).  If `current` is None or
/// not in `names`, return the first element.
fn cycle_focus_forward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(idx) => Some(names[(idx + 1) % names.len()].clone()),
        None      => Some(names[0].clone()),
    }
}

/// Move focus to the previous app in `names` (wrapping).  If `current` is None or
/// not in `names`, return the last element.
fn cycle_focus_backward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(0)   => Some(names[names.len() - 1].clone()),
        Some(idx) => Some(names[idx - 1].clone()),
        None      => Some(names[names.len() - 1].clone()),
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
    BOOT_INSTANT.get_or_init(std::time::Instant::now);
    log_info!(Subsystem::Lifecycle, None, "starting");

    mount_filesystems();
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
            draw_menubar(&mut *fb, w, 0, None, &[]);
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
        match spawn_app(&entry, &inbox, &app_registry) {
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
    // Raw mode: each keystroke is forwarded immediately as a single-char message
    // instead of waiting for a full line.  The shell accumulates characters into
    // a live input buffer and redraws on every keystroke.
    #[cfg(target_os = "linux")]
    {
        let inbox_input    = Arc::clone(&inbox);
        let focused_input  = Arc::clone(&focused);
        let registry_input = Arc::clone(&app_registry);
        thread::Builder::new()
            .name("input-router".into())
            .spawn(move || {
                use std::io::Read;
                use std::os::unix::io::AsRawFd;

                let mut tty = match std::fs::File::open("/dev/tty0") {
                    Ok(f) => f,
                    Err(e) => {
                        log_error!(Subsystem::Input, None, "cannot open /dev/tty0: {e}");
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
                    log_info!(Subsystem::Input, None, "input-router: raw tty mode active");
                } else {
                    log_warn!(Subsystem::Input, None, "input-router: raw mode unavailable, using line mode");
                }

                let mut buf = [0u8; 1];
                loop {
                    match tty.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if buf[0] == 0x1B {
                                // ESC — read next byte to determine sequence type.
                                let mut b1 = [0u8; 1];
                                if tty.read(&mut b1).unwrap_or(0) == 0 {
                                    // Lone ESC: discard.
                                    continue;
                                }

                                if b1[0] == 0x5B {
                                    // CSI sequence (\x1b[…): read the final byte.
                                    let mut b2 = [0u8; 1];
                                    if tty.read(&mut b2).unwrap_or(0) == 0 {
                                        continue; // incomplete, discard
                                    }
                                    let fwd: Option<&str> = match b2[0] {
                                        0x41 => Some("\x1b[A"), // ↑ up arrow
                                        0x42 => Some("\x1b[B"), // ↓ down arrow
                                        _    => None,           // check for window-mgmt below
                                    };
                                    if let Some(msg) = fwd {
                                        let target = focused_input.lock().unwrap().clone();
                                        if let Some(name) = target {
                                            let map = inbox_input.lock().unwrap();
                                            if let Some(tx) = map.get(&name) {
                                                let _ = tx.send(msg.to_string());
                                            }
                                        }
                                    } else if let InputAction::AltShiftTab =
                                        classify_input_sequence(&[0x1B, 0x5B, b2[0]])
                                    {
                                        // Alt+Shift+Tab (\x1b[Z) — cycle focus backward.
                                        let names = windowed_apps_sorted(&registry_input);
                                        if !names.is_empty() {
                                            let cur = focused_input.lock().unwrap().clone();
                                            let prev = cycle_focus_backward(&names, cur.as_deref());
                                            if let Some(ref name) = prev {
                                                log_info!(Subsystem::Input, Some(name.as_str()), "alt+shift+tab: focus → {name}");
                                                *focused_input.lock().unwrap() = Some(name.clone());
                                                repaint_all_borders(&registry_input, &focused_input);
                                            }
                                        }
                                    }
                                    // All other CSI sequences: discard.
                                } else {
                                    // Alt+key sequences (\x1bX where X is not '[').
                                    match classify_input_sequence(&[0x1B, b1[0]]) {
                                        InputAction::AltTab => {
                                            // Cycle keyboard focus forward (sorted-by-name, wrapping).
                                            let names = windowed_apps_sorted(&registry_input);
                                            if !names.is_empty() {
                                                let cur = focused_input.lock().unwrap().clone();
                                                let next = cycle_focus_forward(&names, cur.as_deref());
                                                if let Some(ref name) = next {
                                                    log_info!(Subsystem::Input, Some(name.as_str()), "alt+tab: focus → {name}");
                                                    *focused_input.lock().unwrap() = Some(name.clone());
                                                    repaint_all_borders(&registry_input, &focused_input);
                                                }
                                            }
                                        }
                                        InputAction::AltW => {
                                            // Close the focused display app.
                                            let focused_name = focused_input.lock().unwrap().clone();
                                            if let Some(ref name) = focused_name {
                                                let pid = {
                                                    let reg = registry_input.lock().unwrap();
                                                    reg.get(name).and_then(|st| st.lock().unwrap().child_pid)
                                                };
                                                if let Some(pid) = pid {
                                                    log_info!(Subsystem::Input, Some(name.as_str()), "alt+w: closing {name}");
                                                    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                                                }
                                                // Transfer focus to the next available display app.
                                                let names: Vec<String> = {
                                                    let reg = registry_input.lock().unwrap();
                                                    let mut v: Vec<String> = reg.iter()
                                                        .filter(|(n, st)| {
                                                            n.as_str() != name.as_str()
                                                                && st.lock().unwrap().win_region.is_some()
                                                        })
                                                        .map(|(n, _)| n.clone())
                                                        .collect();
                                                    v.sort();
                                                    v
                                                };
                                                let next = names.into_iter().next();
                                                *focused_input.lock().unwrap() = next;
                                            }
                                        }
                                        InputAction::AltF => {
                                            // Snap focused app to left 2/3; others share right 1/3.
                                            use supervisor::windows::compute_snap_layout;
                                            let focused_name = focused_input.lock().unwrap().clone();
                                            if let Some(ref name) = focused_name {
                                                log_info!(Subsystem::Input, Some(name.as_str()), "alt+f: snap {name}");

                                                // Collect sorted display-app names (same order as tiling).
                                                let apps: Vec<String> = {
                                                    let reg = registry_input.lock().unwrap();
                                                    let mut v: Vec<String> = reg.iter()
                                                        .filter(|(_, st)| st.lock().unwrap().win_region.is_some())
                                                        .map(|(n, _)| n.clone())
                                                        .collect();
                                                    v.sort();
                                                    v
                                                };

                                                let focused_idx = apps.iter().position(|n| n == name);
                                                if let Some(idx) = focused_idx {
                                                    #[cfg(target_os = "linux")]
                                                    let (sw, sh) = display::screen_size().unwrap_or((1440, 900));
                                                    #[cfg(not(target_os = "linux"))]
                                                    let (sw, sh) = (1440u32, 900u32);

                                                    let snap = compute_snap_layout(sw, sh, MENUBAR_H, idx, apps.len());
                                                    {
                                                        let reg = registry_input.lock().unwrap();
                                                        for (i, app_name) in apps.iter().enumerate() {
                                                            if let Some(st) = reg.get(app_name) {
                                                                if let Some(&region) = snap.get(i) {
                                                                    st.lock().unwrap().win_region = Some(region);
                                                                    log_info!(Subsystem::Display, Some(app_name.as_str()),
                                                                        "snap: assigned ({},{},{},{})", region.0, region.1, region.2, region.3);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    repaint_all_borders(&registry_input, &focused_input);
                                                }
                                            }
                                        }
                                        InputAction::AltQuestion => {
                                            // Show keyboard shortcut overlay toast for 3 seconds.
                                            log_info!(Subsystem::Input, None, "alt+?: showing shortcut overlay");
                                            #[cfg(target_os = "linux")]
                                            show_shortcut_overlay();
                                        }
                                        // AltShiftTab handled in the CSI branch above.
                                        // PassThrough: unknown Alt+key — discard.
                                        _ => {}
                                    }
                                }
                            } else {
                                let msg: Option<String> = match buf[0] {
                                    0x0D | 0x0A => Some(String::new()),              // Enter → execute
                                    0x7F | 0x08 => Some("\x7f".to_string()),         // Backspace / DEL
                                    0x03        => Some("\x03".to_string()),          // Ctrl+C
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
                    log_warn!(Subsystem::Input, None, "mouse-input: no pointer device found, disabling");
                    return;
                };

                // Enable cursor sprite now that we have a mouse device.
                display::enable_cursor();

                const EV_SYN: u16    = 0;
                const EV_KEY: u16    = 1;
                const EV_REL: u16    = 2;
                const EV_ABS: u16    = 3;
                const REL_X: u16     = 0;
                const REL_Y: u16     = 1;
                const ABS_X: u16     = 0;
                const ABS_Y: u16     = 1;
                const BTN_LEFT: u16  = 0x110;
                const BTN_RIGHT: u16 = 0x111;
                const BTN_MID: u16   = 0x112;

                // Use actual screen resolution; fall back to 1440×900
                let (sw, sh) = display::screen_size().unwrap_or((1440, 900));
                let screen_w: i32 = sw as i32;
                let screen_h: i32 = sh as i32;
                // virtio-mouse-pci reports ABS coords in range 0..=32767
                const ABS_MAX: i64  = 32768;

                let mut cx: i32 = screen_w / 2;
                let mut cy: i32 = screen_h / 2;
                let mut pending_click_mask:   u8 = 0;
                let mut pending_release_mask: u8 = 0;
                let mut btn_held:             u8 = 0; // bitmask of currently held buttons
                let mut pending_abs_x: Option<i32> = None;
                let mut pending_abs_y: Option<i32> = None;
                let mut pending_dx:    i32 = 0;
                let mut pending_dy:    i32 = 0;

                // Linux input_event on 64-bit:
                //   i64 tv_sec + i64 tv_usec + u16 type + u16 code + i32 value = 24 bytes
                let mut buf = [0u8; 24];
                loop {
                    if dev.read_exact(&mut buf).is_err() {
                        log_error!(Subsystem::Input, None, "mouse-input: device read error, exiting");
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
                            let bit = match code {
                                BTN_LEFT  => 1u8,
                                BTN_RIGHT => 2u8,
                                BTN_MID   => 4u8,
                                _         => 0u8,
                            };
                            if value == 1 {
                                // Button pressed
                                pending_click_mask |= bit;
                            } else if value == 0 && bit != 0 {
                                // Button released
                                pending_release_mask |= bit;
                            }
                        }
                        EV_SYN => {
                            let mut pos_changed = false;
                            // Apply REL movement (regular mouse)
                            if pending_dx != 0 || pending_dy != 0 {
                                let new_cx = (cx + pending_dx).clamp(0, screen_w - 1);
                                let new_cy = (cy + pending_dy).clamp(0, screen_h - 1);
                                pos_changed = new_cx != cx || new_cy != cy;
                                cx = new_cx; cy = new_cy;
                                pending_dx = 0; pending_dy = 0;
                            }
                            // Apply ABS position (virtio-mouse-pci, scaled 0..32767 → screen)
                            if let Some(ax) = pending_abs_x.take() {
                                let new_cx = (ax as i64 * screen_w as i64 / ABS_MAX) as i32;
                                if new_cx != cx { pos_changed = true; cx = new_cx; }
                            }
                            if let Some(ay) = pending_abs_y.take() {
                                let new_cy = (ay as i64 * screen_h as i64 / ABS_MAX) as i32;
                                if new_cy != cy { pos_changed = true; cy = new_cy; }
                            }
                            display::set_cursor_pos(cx, cy);
                            if pending_click_mask != 0 {
                                // Button pressed — store drag-start position
                                if pending_click_mask & 1 != 0 {
                                    *mouse_drag_start().lock().unwrap() = Some((cx, cy));
                                }
                                btn_held |= pending_click_mask;
                                dispatch_mouse(cx, cy, pending_click_mask, &inbox_m, &registry_m, &focused_m);
                                pending_click_mask = 0;
                            } else if pos_changed {
                                // Mouse moved — if left button held, log drag delta
                                if btn_held & 1 != 0 {
                                    let drag_start = *mouse_drag_start().lock().unwrap();
                                    if let Some((sx, sy)) = drag_start {
                                        let (dx, dy) = supervisor::windows::drag_delta(sx, sy, cx, cy);
                                        // Find the topmost mouse-capable app under cursor for logging
                                        let z_snap: Vec<String> = Z_ORDER.get()
                                            .map(|m| m.lock().unwrap().clone())
                                            .unwrap_or_default();
                                        let app_name: Option<String> = {
                                            let reg = registry_m.lock().unwrap();
                                            let mut found: Option<String> = None;
                                            for name in &z_snap {
                                                let Some(st_arc) = reg.get(name) else { continue };
                                                let st = st_arc.lock().unwrap();
                                                let Some((wx, wy, ww, wh)) = st.win_region else { continue };
                                                if cx >= wx as i32 && cy >= wy as i32
                                                    && cx < (wx + ww) as i32 && cy < (wy + wh) as i32
                                                {
                                                    found = Some(name.clone());
                                                    break;
                                                }
                                            }
                                            found
                                        };
                                        let name = app_name.as_deref().unwrap_or("none");
                                        log_info!(Subsystem::Input, None, "drag: app={name} dx={dx} dy={dy}");
                                    }
                                }
                                dispatch_mouse(cx, cy, 0, &inbox_m, &registry_m, &focused_m);
                            }
                            // Button released — clear drag-start state
                            if pending_release_mask != 0 {
                                if pending_release_mask & 1 != 0 {
                                    *mouse_drag_start().lock().unwrap() = None;
                                }
                                btn_held &= !pending_release_mask;
                                pending_release_mask = 0;
                            }
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
                                log_warn!(Subsystem::Lifecycle, Some(name.as_str()), "silent for {}s (limit={wsecs}s) — killing pid {pid}", elapsed.as_secs());
                                show_crash_toast(name, -1);
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

    log_info!(Subsystem::Lifecycle, None, "all apps completed, idling");
    loop { thread::park(); }
}

// ── Spawn writer + reader threads for one app (no waiter) ────────────────────

fn spawn_io_threads(
    name: &str,
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
                // Look up win_region dynamically so tiling layout updates are reflected.
                let win_region = if has_display {
                    registry_r.lock().unwrap()
                        .get(&name_r)
                        .and_then(|st| st.lock().unwrap().win_region)
                } else {
                    None
                };
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
    let SpawnedApp { entry, name, child, msg_rx, child_stdin, child_stdout, has_display, is_shell: _ } = app;

    // T014: recompute tiled layout when a new display app is added
    if has_display {
        apply_tiling_layout(app_registry);
        #[cfg(target_os = "linux")]
        repaint_all_borders(app_registry, focused);
    }

    spawn_io_threads(&name, child_stdin, child_stdout, msg_rx, has_display, inbox, focused, app_registry);

    // Send actual screen resolution to display apps before their first draw
    #[cfg(target_os = "linux")]
    if has_display {
        if let Some((w, h)) = display::screen_size() {
            if let Some(tx) = inbox.lock().unwrap().get(&name) {
                let _ = tx.send(format!("VYOMA_SYSTEM:screen:{w},{h}"));
            }
        }
    }

    // Newly launched display apps go to the front of the Z-order (after tiling assigns region)
    if has_display {
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
    let manifest = match supervisor::manifest::parse_manifest(std::path::Path::new(&entry.manifest)) {
        Ok(m) => m,
        Err(e) => {
            log_warn!(Subsystem::Manifest, None, "{e}");
            return None;
        }
    };

    let name = manifest.app.name.clone();
    let caps = &manifest.capabilities;

    let net_port = caps.network_port.unwrap_or(8080);

    // T021a [FR-005]: one log line per app at WASM load time listing wired vs skipped capabilities.
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
            "wired: {}{net_note}; skipped: {}",
            wired.join(" "), skipped.join(" ")
        );
    }

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
                    log_error!(Subsystem::Capability, Some(name.as_str()), "SECURITY: {name} rejected — SHA-256 mismatch\n  expected {expected}\n  actual   {actual}");
                    inbox.lock().unwrap().remove(&name);
                    return None;
                }
                log_info!(Subsystem::Capability, Some(name.as_str()), "[security] {name} wasm_sha256 verified OK");
            }
            Err(e) => {
                log_warn!(Subsystem::Capability, Some(name.as_str()), "cannot read wasm for hash check: {e}");
            }
        }
    }

    log_info!(Subsystem::Lifecycle, Some(name.as_str()), "spawning {} v{} (restart={})", name, manifest.app.version, entry.restart);

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

    // Limit tokio's multi-thread pool to 1 worker; prevents the OS-thread
    // cap from causing EINVAL on clone() inside the VM's constrained kernel.
    cmd.env("TOKIO_WORKER_THREADS", "1");

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let filter = seccomp::build();
        unsafe {
            // P08T01: apply seccomp denylist. clone3 returns ENOSYS so glibc
            // falls back to clone(2) — fixes EINVAL on Linux 5.10 + BASE_SMALL.
            // CLONE_NEWPID unshare removed: it persists across exec and confuses
            // tokio thread spawning on this kernel config.
            cmd.pre_exec(move || seccomp::apply(&filter));
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

    // Read preferred min dimensions from manifest [window] section (optional hints).
    let min_size = manifest.window.as_ref()
        .map(|wr| (wr.width.unwrap_or(0), wr.height.unwrap_or(0)))
        .unwrap_or((0, 0));

    // Register per-app runtime state (win_region starts None; assigned by apply_tiling_layout)
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
        min_size,
        draw_ticks:       0,
        last_cpu_reset:   Instant::now(),
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
                handle_draw_command(cmd, sender, win_region, focused, app_registry);
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
                handle_supervisor_command(msg, sender, inbox, focused, app_registry);
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

        // 030-ipc-list-apps: return newline-separated list of running app names
        "apps" => {
            let names: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut v: Vec<String> = reg.iter()
                    .filter(|(_, st)| matches!(st.lock().unwrap().status, AppStatus::Running))
                    .map(|(name, _)| name.clone())
                    .collect();
                v.sort();
                v
            };
            let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            let reply = supervisor::ipc::format_app_list(&name_refs);
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
            log_info!(Subsystem::Ipc, None, "apps list sent to {sender}: {} apps", names.len());
        }

        "focus" => {
            if let Some(name) = parts.get(1).map(|s| s.trim()) {
                *focused.lock().unwrap() = Some(name.to_string());
                log_info!(Subsystem::Input, Some(name), "focus → {name}");
            }
        }

        // P54: win-info <app> — return window region of an app
        "win-info" => {
            let app_name = parts.get(1).unwrap_or(&"").trim().to_string();
            let reg = app_registry.lock().unwrap();
            if let Some(st) = reg.get(&app_name) {
                let region = st.lock().unwrap().win_region;
                let coords = match region {
                    Some((x, y, w, h)) => format!("{x},{y},{w},{h}"),
                    None => "none".to_string(),
                };
                send_reply(sender, &format!("REPLY:win-info {app_name} {coords}"), inbox);
            } else {
                send_reply(sender, &format!("REPLY:win-info {app_name} not-found"), inbox);
            }
        }

        // P55: font-size <s|m|l> — store global font size preference
        "font-size" => {
            let size = parts.get(1).unwrap_or(&"m").trim().to_string();
            let valid = matches!(size.as_str(), "s" | "m" | "l");
            if valid {
                *FONT_SIZE.get().unwrap().lock().unwrap() = size.clone();
                log_info!(Subsystem::Lifecycle, None, "font-size → {size}");
                send_reply(sender, &format!("REPLY:font-size {size}"), inbox);
            } else {
                send_reply(sender, "REPLY:font-size error invalid-size", inbox);
            }
        }

        // P52: input <char> — forward a character to the focused app's stdin
        "input" => {
            let ch = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
            let target = focused.lock().unwrap().clone();
            if let Some(name) = target {
                let map = inbox.lock().unwrap();
                if let Some(tx) = map.get(&name) {
                    let _ = tx.send(ch);
                }
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
            log_info!(Subsystem::Ipc, None, "@supervisor: run {path}");
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

        // ps — list all apps with status, uptime, restart count, cpu%
        "ps" => {
            let entries: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut rows: Vec<(String, String)> = reg.iter().map(|(name, st)| {
                    let st = st.lock().unwrap();
                    let pid = st.child_pid.unwrap_or(0);
                    let uptime_secs = st.start_time.elapsed().as_secs();
                    let uptime_str = supervisor::lifecycle::format_uptime(uptime_secs);
                    let state_str = match &st.status {
                        AppStatus::Running    => "running".to_string(),
                        AppStatus::Stopped(c) => format!("stopped({})", c),
                    };
                    let wd_tag = if st.watchdog_secs > 0 {
                        format!(" [watchdog={}s]", st.watchdog_secs)
                    } else {
                        String::new()
                    };
                    let cpu = supervisor::lifecycle::format_cpu(
                        st.draw_ticks,
                        st.last_cpu_reset.elapsed().as_millis() as u64,
                    );
                    let base = format_ps_line(name, pid, &state_str, st.restart_count);
                    let info = format!("{}{} up:{} cpu:{}", base, wd_tag, uptime_str, cpu);
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
            let (pid, running_names) = {
                let reg = app_registry.lock().unwrap();
                let pid = reg.get(&app_name).and_then(|st| st.lock().unwrap().child_pid);
                let names: Vec<String> = reg.keys().cloned().collect();
                (pid, names)
            };
            let running_refs: Vec<&str> = running_names.iter().map(|s| s.as_str()).collect();
            if !supervisor::ipc::validate_kill_target(&app_name, &running_refs) {
                log_info!(Subsystem::Ipc, Some(app_name.as_str()), "kill: app={app_name} not found or not running");
                send_reply(sender, &format!("REPLY:{app_name} not running"), inbox);
                return;
            }
            match pid {
                Some(pid) => {
                    #[cfg(target_os = "linux")]
                    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                    log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "killed {app_name} (pid {pid})");
                    send_reply(sender, &format!("REPLY:killed {app_name}"), inbox);
                }
                None => {
                    log_info!(Subsystem::Ipc, Some(app_name.as_str()), "kill: app={app_name} not found or not running");
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
                log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "restart: killed old {app_name} (pid {pid})");
            }
            // Spawn new instance
            match spawn_app(&entry, inbox, app_registry) {
                Some(app) => {
                    launch_app_threads(app, inbox, focused, app_registry);
                    log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "restarted {app_name}");
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
            log_info!(Subsystem::Ipc, Some(app_name.as_str()), "@supervisor: update {app_name} from {url}");

            let sender_name  = sender.to_string();
            let inbox_bg     = Arc::clone(inbox);
            let focused_bg   = Arc::clone(focused);
            let registry_bg  = Arc::clone(app_registry);

            thread::spawn(move || {
                let bytes = match http_get(&url) {
                    Ok(b)  => b,
                    Err(e) => {
                        log_error!(Subsystem::Lifecycle, Some(app_name.as_str()), "update {app_name}: download failed: {e}");
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
                            log_info!(Subsystem::Capability, Some(app_name.as_str()), "update {app_name}: SHA-256 verified OK");
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
                log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "update {app_name}: installed → {dest:?}");
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
                        log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "update {app_name}: restarted OK");
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
            log_info!(Subsystem::Display, None, "wallpaper set to {rgba:#010x}");
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
            log_info!(Subsystem::Display, Some(app_name.as_str()), "resize {app_name} → {new_w}×{new_h}");
            send_reply(sender, &format!("REPLY:resized {app_name} to {new_w}x{new_h}"), inbox);
        }

        // P41: shutdown — power off the system
        "shutdown" => {
            log_info!(Subsystem::Lifecycle, None, "shutdown requested by {sender}");
            send_reply(sender, "REPLY:shutting down...", inbox);
            thread::spawn(|| {
                thread::sleep(std::time::Duration::from_millis(500));
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
                }
            });
        }

        // P41: reboot — restart the system
        "reboot" => {
            log_info!(Subsystem::Lifecycle, None, "reboot requested by {sender}");
            send_reply(sender, "REPLY:rebooting...", inbox);
            thread::spawn(|| {
                thread::sleep(std::time::Duration::from_millis(500));
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::reboot(libc::LINUX_REBOOT_CMD_RESTART);
                }
            });
        }

        // P39: notify <title> <msg> — draw toast overlay, auto-clear after 3s
        "notify" => {
            let rest = parts.get(1).unwrap_or(&"").trim().to_string();
            let (title, msg) = rest
                .split_once(' ')
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .unwrap_or_else(|| (rest.clone(), String::new()));
            #[cfg(target_os = "linux")]
            {
                const NX: u32 = 1020;
                const NY: u32 = 10;
                const NW: u32 = 400;
                const NH: u32 = 60;
                if let Some(fb_lock) = display::get() {
                    let mut fb = fb_lock.lock().unwrap();
                    fb.fill_rect(NX, NY, NW, NH, 0x21262DFF);
                    fb.rect_border(NX, NY, NW, NH, 0x58A6FFFF);
                    fb.draw_text(NX + 8, NY + 8, &title, 0xFFFFFFFF, font::FontSize::Medium);
                    fb.draw_text(NX + 8, NY + 28, &msg, 0x8B949EFF, font::FontSize::Medium);
                    fb.flush();
                }
                thread::spawn(move || {
                    thread::sleep(std::time::Duration::from_secs(3));
                    if let Some(fb_lock) = display::get() {
                        let mut fb = fb_lock.lock().unwrap();
                        fb.fill_rect(NX, NY, NW, NH, 0x0D1117FF);
                        fb.flush();
                    }
                });
            }
            log_info!(Subsystem::Display, None, "notify title={title:?} msg={msg:?}");
            send_reply(sender, "REPLY:notified", inbox);
        }

        // P42: session-save — persist all window positions to /data/session.toml
        "session-save" => {
            let mut toml = String::new();
            {
                let reg = app_registry.lock().unwrap();
                for (name, state_arc) in reg.iter() {
                    let st = state_arc.lock().unwrap();
                    if let Some((x, y, w, h)) = st.win_region {
                        toml.push_str(&format!(
                            "[[window]]\nname = \"{name}\"\nx = {x}\ny = {y}\nw = {w}\nh = {h}\n\n"
                        ));
                    }
                }
            }
            let _ = fs::write("/data/session.toml", &toml);
            log_info!(Subsystem::Lifecycle, None, "session saved ({} bytes)", toml.len());
            send_reply(sender, "REPLY:session saved", inbox);
        }

        // P42: session-restore — reload window positions from /data/session.toml
        "session-restore" => {
            let content = fs::read_to_string("/data/session.toml").unwrap_or_default();
            let mut windows: Vec<(String, u32, u32, u32, u32)> = Vec::new();
            let (mut cur_name, mut cx, mut cy, mut cw, mut ch, mut in_win) =
                (String::new(), 0u32, 0u32, 0u32, 0u32, false);
            for line in content.lines() {
                let l = line.trim();
                if l == "[[window]]" {
                    if in_win && !cur_name.is_empty() {
                        windows.push((cur_name.clone(), cx, cy, cw, ch));
                    }
                    cur_name.clear();
                    (cx, cy, cw, ch, in_win) = (0, 0, 0, 0, true);
                } else if in_win {
                    if let Some(v) = l.strip_prefix("name = ") {
                        cur_name = v.trim_matches('"').to_string();
                    } else if let Some(v) = l.strip_prefix("x = ") { cx = v.parse().unwrap_or(0); }
                    else if let Some(v) = l.strip_prefix("y = ")  { cy = v.parse().unwrap_or(0); }
                    else if let Some(v) = l.strip_prefix("w = ")  { cw = v.parse().unwrap_or(0); }
                    else if let Some(v) = l.strip_prefix("h = ")  { ch = v.parse().unwrap_or(0); }
                }
            }
            if in_win && !cur_name.is_empty() {
                windows.push((cur_name, cx, cy, cw, ch));
            }
            let mut restored = 0usize;
            for (name, x, y, w, h) in windows {
                let updated = {
                    let reg = app_registry.lock().unwrap();
                    if let Some(state_arc) = reg.get(&name) {
                        let mut st = state_arc.lock().unwrap();
                        st.win_region = Some((x, y, w, h));
                        true
                    } else { false }
                };
                if updated {
                    send_reply(&name, &format!("VYOMA_SYSTEM:resize:{w},{h}"), inbox);
                    restored += 1;
                }
            }
            log_info!(Subsystem::Lifecycle, None, "session restored {restored} windows");
            send_reply(sender, &format!("REPLY:restored {restored} windows"), inbox);
        }

        // P43: monitors — count DRM connectors via /sys/class/drm
        "monitors" => {
            let count = count_drm_connectors();
            log_info!(Subsystem::Display, None, "monitors = {count}");
            send_reply(sender, &format!("REPLY:monitors {count}"), inbox);
        }

        // P44: dns-resolve <hostname> — synchronous DNS A-record lookup via 8.8.8.8:53
        "dns-resolve" => {
            let hostname = parts.get(1).unwrap_or(&"").trim().to_string();
            if hostname.is_empty() {
                send_reply(sender, "REPLY:dns error: no hostname", inbox);
                return;
            }
            let ip = dns_resolve_a(&hostname).unwrap_or_else(|| "NXDOMAIN".to_string());
            log_info!(Subsystem::Ipc, None, "dns {hostname} -> {ip}");
            send_reply(sender, &format!("REPLY:dns {hostname} {ip}"), inbox);
        }

        // P45: tls-info — check for cert/key in /data
        "tls-info" => {
            let cert = Path::new("/data/cert.pem").exists();
            let key  = Path::new("/data/key.pem").exists();
            let msg = if cert && key {
                "TLS: cert.pem and key.pem present in /data — ready for TLS termination proxy"
            } else {
                "TLS: no cert/key found — place cert.pem and key.pem in /data/ to enable TLS"
            };
            send_reply(sender, &format!("REPLY:{msg}"), inbox);
        }

        // P46: http-get <url> — fetch URL, return status code + first 4096 chars of body
        "http-get" => {
            let url = parts.get(1).unwrap_or(&"").trim().to_string();
            if url.is_empty() {
                send_reply(sender, "REPLY:http-get error no-url", inbox);
                return;
            }
            match http_get(&url) {
                Ok(bytes) => {
                    let raw = String::from_utf8_lossy(&bytes);
                    let status_code = raw.lines().next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("200")
                        .to_string();
                    let body = if let Some(p) = raw.find("\r\n\r\n") { &raw[p + 4..] }
                              else if let Some(p) = raw.find("\n\n") { &raw[p + 2..] }
                              else { &raw };
                    let escaped: String = body.chars().take(4096)
                        .collect::<String>()
                        .replace('\r', "")
                        .replace('\n', "\\n");
                    send_reply(sender, &format!("REPLY:http-get {status_code} {escaped}"), inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:http-get error {e}"), inbox);
                }
            }
        }

        // P47: tcp-connect <host:port> — open raw TCP connection, return ID
        "tcp-connect" => {
            let addr = parts.get(1).unwrap_or(&"").trim().to_string();
            if addr.is_empty() {
                send_reply(sender, "REPLY:tcp-connect error no-addr", inbox);
                return;
            }
            match std::net::TcpStream::connect(&addr) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
                    let id = TCP_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    TCP_CONNS.get().unwrap().lock().unwrap().insert(id, stream);
                    log_info!(Subsystem::Ipc, None, "tcp-connect {addr} id={id}");
                    send_reply(sender, &format!("REPLY:tcp-connect {id}"), inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:tcp-connect error {e}"), inbox);
                }
            }
        }

        // P47: tcp-send <id> <data> — write data to TCP connection
        "tcp-send" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (id_str, data) = rest.split_once(' ').unwrap_or((rest, ""));
            let id: u32 = id_str.parse().unwrap_or(0);
            let map = TCP_CONNS.get().unwrap().lock().unwrap();
            if let Some(stream) = map.get(&id) {
                use std::io::Write as IoWrite;
                let payload = format!("{data}\n");
                let _ = (&*stream as &std::net::TcpStream).write_all(payload.as_bytes());
                send_reply(sender, &format!("REPLY:tcp-send {id} ok"), inbox);
            } else {
                send_reply(sender, &format!("REPLY:tcp-send error not-found"), inbox);
            }
        }

        // P47: tcp-recv <id> — read available data from TCP connection (non-blocking)
        "tcp-recv" => {
            let id: u32 = parts.get(1).unwrap_or(&"0").trim().parse().unwrap_or(0);
            let map = TCP_CONNS.get().unwrap().lock().unwrap();
            if let Some(stream) = map.get(&id) {
                use std::io::Read as IoRead;
                let mut buf = vec![0u8; 1024];
                match (&*stream as &std::net::TcpStream).read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let data: String = buf[..n].iter()
                            .filter(|&&b| b >= 32 || b == b'\n' || b == b'\r')
                            .map(|&b| b as char)
                            .collect::<String>()
                            .replace('\r', "")
                            .replace('\n', "\\n");
                        send_reply(sender, &format!("REPLY:tcp-recv {id} {data}"), inbox);
                    }
                    _ => send_reply(sender, &format!("REPLY:tcp-recv {id} "), inbox),
                }
            } else {
                send_reply(sender, &format!("REPLY:tcp-recv error not-found"), inbox);
            }
        }

        // P47: tcp-close <id> — close TCP connection
        "tcp-close" => {
            let id: u32 = parts.get(1).unwrap_or(&"0").trim().parse().unwrap_or(0);
            TCP_CONNS.get().unwrap().lock().unwrap().remove(&id);
            send_reply(sender, &format!("REPLY:tcp-close {id} ok"), inbox);
        }

        // P50: clipboard-set <text> — store text in global clipboard
        "clipboard-set" => {
            let text = parts.get(1).unwrap_or(&"").trim().to_string();
            *CLIPBOARD.get().unwrap().lock().unwrap() = text;
            send_reply(sender, "REPLY:clipboard-set ok", inbox);
        }

        // P50: clipboard-get — return current clipboard contents
        "clipboard-get" => {
            let text = CLIPBOARD.get().unwrap().lock().unwrap().clone();
            send_reply(sender, &format!("REPLY:clipboard {text}"), inbox);
        }

        // P51: screenshot <path> — write back-buffer as PPM to path
        "screenshot" => {
            let path = parts.get(1).unwrap_or(&"").trim().to_string();
            if path.is_empty() {
                send_reply(sender, "REPLY:screenshot error no-path", inbox);
                return;
            }
            #[cfg(target_os = "linux")]
            {
                match display::get() {
                    Some(fb_lock) => {
                        let fb = fb_lock.lock().unwrap();
                        match fb.screenshot(&path) {
                            Ok(()) => {
                                log_info!(Subsystem::Display, None, "screenshot saved to {path}");
                                send_reply(sender, &format!("REPLY:screenshot ok {path}"), inbox);
                            }
                            Err(e) => send_reply(sender, &format!("REPLY:screenshot error {e}"), inbox),
                        }
                    }
                    None => send_reply(sender, "REPLY:screenshot error no-display", inbox),
                }
            }
            #[cfg(not(target_os = "linux"))]
            send_reply(sender, "REPLY:screenshot error linux-only", inbox);
        }

        // P49: download <url> <dest> — fetch URL, write to dest path in background
        "download" => {
            let rest = parts.get(1).unwrap_or(&"").trim().to_string();
            let (url, dest) = match rest.split_once(' ') {
                Some((u, d)) => (u.trim().to_string(), d.trim().to_string()),
                None => {
                    send_reply(sender, "REPLY:download-error  missing dest", inbox);
                    return;
                }
            };
            if url.is_empty() || dest.is_empty() {
                send_reply(sender, "REPLY:download-error  missing url or dest", inbox);
                return;
            }
            let sender_name = sender.to_string();
            let inbox_clone = Arc::clone(inbox);
            let dest_clone  = dest.clone();
            thread::spawn(move || {
                let dest = dest_clone;
                send_reply(&sender_name, &format!("REPLY:download-progress {dest} 0"), &inbox_clone);
                match http_get(&url) {
                    Ok(bytes) => {
                        let n = bytes.len();
                        send_reply(&sender_name, &format!("REPLY:download-progress {dest} {n}"), &inbox_clone);
                        match fs::write(&dest, &bytes) {
                            Ok(()) => {
                                log_info!(Subsystem::Lifecycle, None, "download done {dest} ({n} bytes)");
                                send_reply(&sender_name, &format!("REPLY:download-done {dest}"), &inbox_clone);
                            }
                            Err(e) => {
                                send_reply(&sender_name, &format!("REPLY:download-error {dest} {e}"), &inbox_clone);
                            }
                        }
                    }
                    Err(e) => {
                        send_reply(&sender_name, &format!("REPLY:download-error {dest} {e}"), &inbox_clone);
                    }
                }
            });
            send_reply(sender, &format!("REPLY:download-progress {dest} 0"), inbox);
        }

        // uptime — reply with how long the supervisor has been running
        "uptime" => {
            let secs = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
            let reply = supervisor::lifecycle::format_system_uptime(secs);
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
            log_info!(Subsystem::Ipc, None, "uptime query from {sender}: {reply}");
        }

        // 023: loglevel <app> <level> — set per-app log level filter
        "loglevel" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let sub_parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if sub_parts.len() == 2 {
                let app_name = sub_parts[0].trim();
                let level_str = sub_parts[1].trim();
                if let Some(lvl) = supervisor::ipc::parse_log_level(level_str) {
                    app_log_levels().lock().unwrap().insert(app_name.to_string(), lvl);
                    log_info!(Subsystem::Ipc, Some(app_name),
                        "app={} log_level set to {:?}", app_name, lvl);
                } else {
                    log_warn!(Subsystem::Ipc, None,
                        "loglevel: unknown level {:?}", level_str);
                }
            } else {
                log_warn!(Subsystem::Ipc, None,
                    "loglevel: usage: loglevel <app> <debug|info|warn|error>");
            }
        }

        // ping — reply to sender with "pong <timestamp_ms>"
        "ping" => {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let reply = supervisor::ipc::format_pong_reply(ms);
            log_info!(Subsystem::Ipc, None, "ping from={sender} reply={reply}");
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
        }

        // version — reply with the VyomaOS version string
        "version" => {
            let v = supervisor::ipc::format_version(0, 19, 0);
            send_reply(sender, &format!("REPLY:{v}"), inbox);
            log_info!(Subsystem::Ipc, None, "version query from {sender}: {v}");
        }

        other => {
            log_warn!(Subsystem::Ipc, None, "unknown @supervisor command from {sender}: {other}");
        }
    }
}

fn count_drm_connectors() -> usize {
    fs::read_dir("/sys/class/drm")
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("card0-"))
                .count()
        })
        .unwrap_or(1)
        .max(1)
}

// P44: synchronous DNS A-record lookup over TCP to 8.8.8.8:53
fn dns_resolve_a(hostname: &str) -> Option<String> {
    use std::io::{Read, Write as _};
    use std::net::TcpStream;

    // Build DNS query packet
    let mut q: Vec<u8> = Vec::new();
    q.extend_from_slice(&[0x12, 0x34, 0x01, 0x00]); // ID + flags (RD)
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // counts
    for label in hostname.trim_end_matches('.').split('.') {
        let b = label.as_bytes();
        q.push(b.len() as u8);
        q.extend_from_slice(b);
    }
    q.push(0x00);                                // root label
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // Type A, Class IN

    // TCP DNS: 2-byte big-endian length prefix
    let mut msg: Vec<u8> = vec![(q.len() >> 8) as u8, (q.len() & 0xFF) as u8];
    msg.extend_from_slice(&q);

    let mut stream = TcpStream::connect("8.8.8.8:53").ok()?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(4))).ok()?;
    stream.write_all(&msg).ok()?;

    // Read response length then body
    let mut lbuf = [0u8; 2];
    stream.read_exact(&mut lbuf).ok()?;
    let rlen = u16::from_be_bytes(lbuf) as usize;
    let mut resp = vec![0u8; rlen];
    stream.read_exact(&mut resp).ok()?;

    if resp.len() < 12 { return None; }
    let ancount = u16::from_be_bytes([resp[6], resp[7]]) as usize;
    if ancount == 0 { return None; }

    // Skip question section
    let mut pos = 12usize;
    pos = dns_skip_name(&resp, pos)?;
    pos += 4; // type + class

    // Parse first answer record
    pos = dns_skip_name(&resp, pos)?;
    if pos + 10 > resp.len() { return None; }
    let rtype  = u16::from_be_bytes([resp[pos],   resp[pos+1]]);
    pos += 8; // type(2) + class(2) + ttl(4)
    let rdlen  = u16::from_be_bytes([resp[pos], resp[pos+1]]) as usize;
    pos += 2;

    if rtype == 1 && rdlen == 4 && pos + 4 <= resp.len() {
        return Some(format!("{}.{}.{}.{}", resp[pos], resp[pos+1], resp[pos+2], resp[pos+3]));
    }
    None
}

fn dns_skip_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        if pos >= buf.len() { return None; }
        let b = buf[pos] as usize;
        if b == 0                 { return Some(pos + 1); }
        if (b & 0xC0) == 0xC0    { return Some(pos + 2); } // compressed pointer
        pos += b + 1;
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

    log_info!(Subsystem::Lifecycle, Some(name), "pkg: installed {name}");
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
        log_info!(Subsystem::Lifecycle, Some(name), "pkg: killed {name} (pid {pid})");
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

    log_info!(Subsystem::Lifecycle, Some(name), "pkg: removed {name}");
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
            log_info!(Subsystem::Input, None, "mouse-input: using {path}");
            return Some(f);
        }
    }
    None
}

/// Which traffic-light button was hit.
#[derive(Debug, PartialEq)]
pub enum TrafficLight {
    Close,
    Minimize,
    Maximize,
}

/// Return the traffic-light button hit by screen point `(cx, cy)` for a window
/// whose top-left corner is at `(wx, wy)`, or `None` if no button was hit.
///
/// Layout (matches `draw_titlebar`):
///   close    at wx+ 8, tl_y  (12×12 px)
///   minimize at wx+24, tl_y  (12×12 px)
///   maximize at wx+40, tl_y  (12×12 px)
/// where tl_y = wy + (TITLEBAR_H − TL_DOT) / 2 = wy + 8.
pub fn traffic_light_hit(cx: i32, cy: i32, wx: u32, wy: u32) -> Option<TrafficLight> {
    let tl_y = wy as i32 + 8; // (TITLEBAR_H=28 - TL_DOT=12) / 2 = 8
    if cy < tl_y || cy >= tl_y + 12 {
        return None;
    }
    let wx = wx as i32;
    if cx >= wx + 8  && cx < wx + 20  { return Some(TrafficLight::Close);    }
    if cx >= wx + 24 && cx < wx + 36  { return Some(TrafficLight::Minimize); }
    if cx >= wx + 40 && cx < wx + 52  { return Some(TrafficLight::Maximize); }
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
    // Snapshot z-order before locking registry (avoids lock ordering issues)
    let z_snapshot: Vec<String> = Z_ORDER.get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default();

    // ── Title-bar hover highlight (motion only, no button press) ─────────────
    // On every motion event (btn == 0), determine which app's title bar (if any)
    // the cursor is over.  If the hovered window changed, redraw the previous
    // title bar (un-highlight) and the new one (highlight).  We guard with a
    // `new_hover != old_hover` check so no extra work is done when the cursor
    // stays in the same title bar.
    if btn == 0 {
        // Determine which app's title bar the cursor is currently over.
        let new_hover: Option<String> = {
            let reg = app_registry.lock().unwrap();
            let mut found: Option<String> = None;
            // Check z-order first so topmost window wins.
            for name in &z_snapshot {
                let Some(state_arc) = reg.get(name) else { continue };
                let st = state_arc.lock().unwrap();
                let Some((wx, wy, ww, _wh)) = st.win_region else { continue };
                // Title bar spans y in [wy, wy + TITLEBAR_H) and x in [wx, wx + ww).
                if cx >= wx as i32 && cx < (wx + ww) as i32
                    && cy >= wy as i32 && cy < (wy + TITLEBAR_H) as i32 {
                    found = Some(name.clone());
                    break;
                }
            }
            // Fallback: apps not in z_order
            if found.is_none() {
                for (name, state_arc) in reg.iter() {
                    let st = state_arc.lock().unwrap();
                    let Some((wx, wy, ww, _wh)) = st.win_region else { continue };
                    if cx >= wx as i32 && cx < (wx + ww) as i32
                        && cy >= wy as i32 && cy < (wy + TITLEBAR_H) as i32 {
                        found = Some(name.clone());
                        break;
                    }
                }
            }
            found
        };

        let old_hover = {
            HOVERED_APP
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap()
                .clone()
        };

        if new_hover != old_hover {
            // Update hover state.
            *HOVERED_APP.get_or_init(|| Mutex::new(None)).lock().unwrap() = new_hover.clone();

            // Collect the regions we need to redraw (previous and new hovered window).
            let focused_name = focused.lock().unwrap().clone();
            let mut to_redraw: Vec<(String, u32, u32, u32)> = Vec::new();
            {
                let reg = app_registry.lock().unwrap();
                for candidate in old_hover.iter().chain(new_hover.iter()) {
                    if let Some(state_arc) = reg.get(candidate.as_str()) {
                        let st = state_arc.lock().unwrap();
                        if let Some((wx, wy, ww, _wh)) = st.win_region {
                            if ww >= 60 {
                                to_redraw.push((candidate.clone(), wx, wy, ww));
                            }
                        }
                    }
                }
            }

            if !to_redraw.is_empty() {
                if let Some(fb_lock) = display::get() {
                    let mut fb = fb_lock.lock().unwrap();
                    for (name, wx, wy, ww) in &to_redraw {
                        let is_focused = focused_name.as_deref() == Some(name.as_str());
                        let is_hovered = new_hover.as_deref() == Some(name.as_str());
                        draw_titlebar(&mut *fb, *wx, *wy, *ww, is_focused, is_hovered, name);
                    }
                    fb.flush();
                }
            }
        }
    }

    // Menu-bar click: if the user clicks inside the MENUBAR_H-pixel band at the
    // top of the screen on an app-name label, raise and focus that app without
    // also triggering window hit-test or mouse-event dispatch.
    if btn != 0 && cy >= 0 && cy < MENUBAR_H as i32 {
        use supervisor::windows::menubar_hit_app;
        let display_apps: Vec<String> = {
            let reg = app_registry.lock().unwrap();
            let mut v: Vec<String> = reg.iter()
                .filter_map(|(name, st)| {
                    let st = st.lock().unwrap();
                    if st.has_display && matches!(st.status, AppStatus::Running) {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
                .collect();
            v.sort();
            v
        };
        let app_refs: Vec<&str> = display_apps.iter().map(String::as_str).collect();
        if let Some(name) = menubar_hit_app(cx, cy, MENUBAR_H, &app_refs) {
            let name = name.to_string();
            z_order_push_front(&name);
            *focused.lock().unwrap() = Some(name.clone());
            log_info!(Subsystem::Input, Some(name.as_str()), "menubar click: focus → {name}");
            repaint_all_borders(app_registry, focused);
            return;
        }
    }

    // Traffic-light hit-test: on any click, check if a dot was hit before
    // falling through to the focus/raise and mouse-event dispatch logic.
    if btn != 0 {
        let tl_hit = {
            let reg = app_registry.lock().unwrap();
            let mut result: Option<(String, TrafficLight)> = None;
            for name in &z_snapshot {
                let Some(state_arc) = reg.get(name) else { continue };
                let st = state_arc.lock().unwrap();
                let Some((wx, wy, _ww, _wh)) = st.win_region else { continue };
                if let Some(dot) = traffic_light_hit(cx, cy, wx, wy) {
                    result = Some((name.clone(), dot));
                    break;
                }
            }
            result
        };
        if let Some((name, dot)) = tl_hit {
            match dot {
                TrafficLight::Close => {
                    let pid = {
                        let reg = app_registry.lock().unwrap();
                        reg.get(&name).and_then(|st| st.lock().unwrap().child_pid)
                    };
                    if let Some(pid) = pid {
                        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                        log_info!(Subsystem::Lifecycle, Some(name.as_str()), "traffic-light close: killed {name} (pid {pid})");
                    }
                }
                TrafficLight::Minimize => {
                    log_info!(Subsystem::Display, Some(name.as_str()), "minimize requested (stub)");
                }
                TrafficLight::Maximize => {
                    log_info!(Subsystem::Display, Some(name.as_str()), "maximize requested (stub)");
                }
            }
            return;
        }
    }

    // On click, raise topmost window under cursor and set keyboard focus
    if btn != 0 {
        let raise_target = {
            let reg = app_registry.lock().unwrap();
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
            repaint_all_borders(app_registry, focused);
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
        let msg = if btn == 0 {
            format!("VYOMA_INPUT:mouse:move:{lx},{ly}")
        } else {
            let bname = match btn { 1 => "left", 2 => "right", 4 => "middle", _ => "left" };
            format!("VYOMA_INPUT:mouse:click:{lx},{ly}:{bname}")
        };
        send_reply(&name, &msg, inbox);
    }
}

// ── VYOMA_DRAW command dispatcher ─────────────────────────────────────────────

/// Parse a u32 that may be decimal ("218169855") or hex ("0x0d1117ff" / "0X0D1117FF").
#[cfg(target_os = "linux")]
#[inline]
fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

#[cfg(target_os = "linux")]
fn handle_draw_command(
    cmd: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    focused: &FocusedApp,
    app_registry: &AppRegistry,
) {
    let Some(fb_lock) = display::get() else { return };

    // Mark this app dirty for any draw command other than flush/present.
    // The flush handler checks and clears this flag before repainting the title bar.
    if cmd != "flush" && cmd != "present" {
        APP_DIRTY
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .insert(sender.to_string(), true);
    }

    if cmd == "flush" || cmd == "present" {
        let focused_name = focused.lock().unwrap().clone();
        let is_focused = focused_name.as_deref() == Some(sender);
        let mut fb = fb_lock.lock().unwrap();

        // Per-window macOS-style title bar — only repaint if this app has
        // issued draw commands since its last flush (dirty flag).
        let is_dirty = {
            let mut dirty_map = APP_DIRTY
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .unwrap();
            let dirty = dirty_map.get(sender).copied().unwrap_or(false);
            if dirty {
                dirty_map.insert(sender.to_string(), false);
            }
            dirty
        };

        if is_dirty {
            if let Some((wx, wy, ww, wh)) = win {
                if ww >= 60 {
                    let is_hovered = HOVERED_APP
                        .get_or_init(|| Mutex::new(None))
                        .lock()
                        .unwrap()
                        .as_deref() == Some(sender);
                    draw_titlebar(&mut *fb, wx, wy, ww, is_focused, is_hovered, sender);
                }
                // Per-window status strip — redrawn on every flush so uptime advances.
                if ww > 0 && wh > STATUS_H {
                    let uptime_secs: u64 = {
                        let reg = app_registry.lock().unwrap();
                        reg.get(sender)
                            .map(|st| st.lock().unwrap().start_time.elapsed().as_secs())
                            .unwrap_or(0)
                    };
                    draw_statusbar(&mut *fb, sender, uptime_secs, wx, wy, ww, wh);
                }
            }
        }

        // Global menu bar — only repaint when ≥1 second has elapsed since the
        // last draw OR the focused app name has changed.
        let sw = fb.width;
        let elapsed = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
        let should_draw_menubar = {
            let mut last = LAST_MENUBAR_DRAW
                .get_or_init(|| {
                    Mutex::new((std::time::Instant::now() - std::time::Duration::from_secs(2), None))
                })
                .lock()
                .unwrap();
            let (ref mut last_instant, ref mut last_focused) = *last;
            let time_elapsed = last_instant.elapsed() >= std::time::Duration::from_secs(1);
            let focus_changed = last_focused.as_deref() != focused_name.as_deref();
            if time_elapsed || focus_changed {
                *last_instant = std::time::Instant::now();
                *last_focused = focused_name.clone();
                true
            } else {
                false
            }
        };

        if should_draw_menubar {
            let display_apps: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut v: Vec<String> = reg.iter()
                    .filter_map(|(name, st)| {
                        let st = st.lock().unwrap();
                        if st.has_display && matches!(st.status, AppStatus::Running) {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                v.sort();
                v
            };
            draw_menubar(&mut *fb, sw, elapsed, focused_name.as_deref(), &display_apps);
        }

        fb.flush();

        // ── FPS tracking ─────────────────────────────────────────────────────
        // Increment the flush counter for this app; log FPS every 5 seconds.
        {
            const FPS_WINDOW_MS: u64 = 5_000;
            let mut map = flush_counts().lock().unwrap();
            let entry = map
                .entry(sender.to_string())
                .or_insert_with(|| (0, std::time::Instant::now()));
            entry.0 += 1;
            let elapsed_ms = entry.1.elapsed().as_millis() as u64;
            if elapsed_ms >= FPS_WINDOW_MS {
                let fps_str = display::format_fps(entry.0, elapsed_ms);
                log_info!(Subsystem::Display, Some(sender), "fps: {fps_str}");
                *entry = (0, std::time::Instant::now());
            }
        }

        return;
    }

    if let Some(args) = cmd.strip_prefix("fill_rect:") {
        let p: Vec<&str> = args.splitn(5, ',').collect();
        if let [lx_s, ly_s, w_s, h_s, rgba_s] = p.as_slice() {
            if let (Ok(lx), Ok(ly), Ok(w), Ok(h), Some(rgba)) = (
                lx_s.parse::<u32>(), ly_s.parse::<u32>(),
                w_s.parse::<u32>(),  h_s.parse::<u32>(),
                parse_color(rgba_s),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, w, h),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let win_right  = wx + ww;
                        // Clip content bottom to exclude the status strip.
                        let win_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().fill_rect(ax, ay, aw, ah, rgba);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad fill_rect args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad fill_rect args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text:") {
        // Try new format first: x,y,rgba,size,text  (5 comma-fields, size = s/m/l)
        // Fall back to legacy:   x,y,rgba,text       (4 comma-fields, size = Medium)
        let parts5: Vec<&str> = args.splitn(5, ',').collect();
        let parts4: Vec<&str> = args.splitn(4, ',').collect();

        let parsed = if parts5.len() == 5 {
            if let (Ok(lx), Ok(ly), Some(rgba), Some(sz)) = (
                parts5[0].parse::<u32>(),
                parts5[1].parse::<u32>(),
                parse_color(parts5[2]),
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
                if let (Ok(lx), Ok(ly), Some(rgba)) = (
                    parts4[0].parse::<u32>(),
                    parts4[1].parse::<u32>(),
                    parse_color(parts4[2]),
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
                    let content_wy = wy + TITLEBAR_H;
                    let ax = wx + lx;
                    let ay = content_wy + ly;
                    // Clip content bottom to exclude the status strip.
                    let content_bottom = (wy + wh).saturating_sub(STATUS_H);
                    if ax >= wx + ww || ay >= content_bottom { return; }
                    (ax, ay)
                }
            };
            fb_lock.lock().unwrap().draw_text(ax, ay, text, rgba, size);
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad draw_text args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("rect_border:") {
        let parts: Vec<&str> = args.splitn(5, ',').collect();
        if parts.len() == 5 {
            if let (Ok(lx), Ok(ly), Ok(w), Ok(h), Some(rgba)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parts[3].parse::<u32>(),
                parse_color(parts[4]),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, w, h),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let win_right  = wx + ww;
                        // Clip content bottom to exclude the status strip.
                        let win_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().rect_border(ax, ay, aw, ah, rgba);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad rect_border args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad rect_border args: {args}");
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
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let win_right  = wx + ww;
                        // Clip content bottom to exclude the status strip.
                        let win_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().clear_region(ax, ay, aw, ah);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad clear_region args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad clear_region args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text_wrap:") {
        // Format: x,y,max_w,rgba,size,text  (splitn 6)
        let parts: Vec<&str> = args.splitn(6, ',').collect();
        if parts.len() == 6 {
            if let (Ok(lx), Ok(ly), Ok(max_w), Some(rgba), Some(size)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parse_color(parts[3]),
                font::parse_size(parts[4]),
            ) {
                let text = parts[5];
                let (ax, ay, effective_max_w) = match win {
                    None => (lx, ly, max_w),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        // Clip content bottom to exclude the status strip.
                        let content_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= wx + ww || ay >= content_bottom { return; }
                        let effective_max_w = max_w.min(ww.saturating_sub(lx));
                        if effective_max_w == 0 { return; }
                        (ax, ay, effective_max_w)
                    }
                };
                fb_lock.lock().unwrap().draw_text_wrap(ax, ay, effective_max_w, text, rgba, size);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad draw_text_wrap args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad draw_text_wrap args: {args}");
        }
        return;
    }

    log_warn!(Subsystem::Display, Some(sender), "unknown command: {cmd}");
}

// ── Crash / watchdog toast ────────────────────────────────────────────────────

fn show_crash_toast(name: &str, code: i32) {
    let title = format!("{name} crashed");
    let msg = if supervisor::lifecycle::is_watchdog_kill(code) {
        "killed by watchdog (no output)".to_string()
    } else {
        format!("exited with code {code}")
    };
    #[cfg(target_os = "linux")]
    {
        const NX: u32 = 1020;
        const NY: u32 = 10;
        const NW: u32 = 400;
        const NH: u32 = 60;
        if let Some(fb_lock) = display::get() {
            let mut fb = fb_lock.lock().unwrap();
            fb.fill_rect(NX, NY, NW, NH, 0x21262DFF);
            fb.rect_border(NX, NY, NW, NH, 0x58A6FFFF);
            fb.draw_text(NX + 8, NY + 8, &title, 0xFFFFFFFF, font::FontSize::Medium);
            fb.draw_text(NX + 8, NY + 28, &msg, 0x8B949EFF, font::FontSize::Medium);
            fb.flush();
        }
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(3));
            if let Some(fb_lock) = display::get() {
                let mut fb = fb_lock.lock().unwrap();
                fb.fill_rect(NX, NY, NW, NH, 0x0D1117FF);
                fb.flush();
            }
        });
    }
    log_info!(Subsystem::Display, None, "crash toast: {title:?} — {msg:?}");
}

// ── Focus transfer on app exit ────────────────────────────────────────────────

fn auto_transfer_focus(exiting: &str, app_registry: &AppRegistry, focused: &FocusedApp) {
    let is_focused = focused.lock().unwrap().as_deref() == Some(exiting);
    if !is_focused { return; }

    // Find first other Running display-or-shell app (sorted for determinism)
    let next = {
        let reg = app_registry.lock().unwrap();
        let mut names: Vec<String> = reg.iter()
            .filter(|(name, state_arc)| {
                if name.as_str() == exiting { return false; }
                let st = state_arc.lock().unwrap();
                matches!(st.status, AppStatus::Running) && st.has_display
            })
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names.into_iter().next()
    };

    *focused.lock().unwrap() = next;
}

// ── Crash toast — visible notification when a restart=never app exits badly ───

fn show_crash_toast(name: &str, code: i32) {
    let title = format!("{name} crashed");
    let msg   = format!("exited with code {code}");
    #[cfg(target_os = "linux")]
    {
        const NX: u32 = 1020;
        const NY: u32 = 10;
        const NW: u32 = 400;
        const NH: u32 = 60;
        if let Some(fb_lock) = display::get() {
            let mut fb = fb_lock.lock().unwrap();
            fb.fill_rect(NX, NY, NW, NH, 0x21262DFF);
            fb.rect_border(NX, NY, NW, NH, 0x58A6FFFF);
            fb.draw_text(NX + 8, NY + 8, &title, 0xFFFFFFFF, font::FontSize::Medium);
            fb.draw_text(NX + 8, NY + 28, &msg,  0x8B949EFF, font::FontSize::Medium);
            fb.flush();
        }
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(3));
            if let Some(fb_lock) = display::get() {
                let mut fb = fb_lock.lock().unwrap();
                fb.fill_rect(NX, NY, NW, NH, 0x0D1117FF);
                fb.flush();
            }
        });
    }
    log_info!(Subsystem::Display, None, "crash toast: {title:?} — {msg:?}");
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
                log_info!(Subsystem::Lifecycle, Some(name.as_str()), "{name} exited (code {code})");
                code
            }
            Err(e) => {
                log_error!(Subsystem::Lifecycle, Some(name.as_str()), "wait failed for {name}: {e}");
                -1
            }
        };

        // Update status in registry; capture win_region before layout reflow
        let (had_display, old_win_region) = {
            let reg = app_registry.lock().unwrap();
            if let Some(st) = reg.get(&name) {
                let mut st = st.lock().unwrap();
                st.status = AppStatus::Stopped(exit_code);
                st.child_pid = None;
                (st.has_display, st.win_region)
            } else {
                (false, None)
            }
        };

        // Gap 1: clear the vacated region immediately so ghost chrome does not linger
        #[cfg(target_os = "linux")]
        if let (true, Some((wx, wy, ww, wh))) = (had_display, old_win_region) {
            if let Some(fb_lock) = display::get() {
                let mut fb = fb_lock.lock().unwrap();
                fb.fill_rect(wx, wy, ww, wh, 0x0D1117FF);
                fb.flush();
            }
        }

        // Gap 2: transfer keyboard focus if exiting app held it
        auto_transfer_focus(&name, &app_registry, &focused);

        // T015: recompute tiled layout when a display app exits
        if had_display {
            apply_tiling_layout(&app_registry);
            // Gap 3: repaint all borders at their new positions after reflow
            #[cfg(target_os = "linux")]
            repaint_all_borders(&app_registry, &focused);
        }

        let should_restart = match entry.restart.as_str() {
            "always"     => true,
            "on-failure" => exit_code != 0,
            _            => false,
        };

        if !should_restart {
            if exit_code != 0 {
                show_crash_toast(&name, exit_code);
            }
            break;
        }

        log_info!(Subsystem::Lifecycle, Some(name.as_str()), "restarting {name} (policy={}, count={})", entry.restart, restart_count + 1);

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
                // Recompute layout now that this app is running again
                if app.has_display {
                    apply_tiling_layout(&app_registry);
                    #[cfg(target_os = "linux")]
                    repaint_all_borders(&app_registry, &focused);
                }
                // Spawn new IO threads; continue this loop as the new waiter
                let SpawnedApp { child: new_child, child_stdin, child_stdout, msg_rx, has_display, .. } = app;
                spawn_io_threads(
                    &name, child_stdin, child_stdout, msg_rx,
                    has_display, &inbox, &focused, &app_registry,
                );
                child = new_child;
            }
            None => {
                log_error!(Subsystem::Lifecycle, Some(name.as_str()), "failed to restart {name}, giving up");
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
            log_warn!(Subsystem::Lifecycle, None, "9P share not available — /data will be empty tmpfs");
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
                log_info!(Subsystem::Lifecycle, None, "{target} already mounted, skipping");
                return;
            }
            panic!("mount({source} -> {target}, {fstype}) failed: {err}");
        }

        log_info!(Subsystem::Lifecycle, None, "mounted {target} ({fstype})");
    }

    #[cfg(not(target_os = "linux"))]
    log_info!(Subsystem::Lifecycle, None, "[dev build] skipping mount {target} ({fstype})");
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
        log_info!(Subsystem::Lifecycle, None, "mounted {target} ({fstype})");
        true
    } else {
        let err = std::io::Error::last_os_error();
        log_error!(Subsystem::Lifecycle, None, "mount({source} -> {target}, {fstype}) failed: {err}");
        false
    }
}
