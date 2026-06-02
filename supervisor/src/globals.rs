// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex, OnceLock};

pub static Z_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
pub static LAST_SENDER: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
pub static TCP_CONNS: OnceLock<Mutex<HashMap<u32, std::net::TcpStream>>> = OnceLock::new();
pub static TCP_NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
pub static WS_CONNS: OnceLock<Mutex<HashMap<u32, super::websocket::WsConnection>>> =
    OnceLock::new();
pub static WS_NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
pub static CLIPBOARD: OnceLock<Mutex<String>> = OnceLock::new();
pub static LOCKED: OnceLock<Mutex<bool>> = OnceLock::new();
pub fn is_locked() -> bool { *LOCKED.get_or_init(|| Mutex::new(false)).lock().unwrap() }
pub fn set_locked(v: bool) { *LOCKED.get_or_init(|| Mutex::new(false)).lock().unwrap() = v; }
pub static FONT_SIZE: OnceLock<Mutex<String>> = OnceLock::new();
pub static MOUSE_DRAG_START: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
pub fn mouse_drag_start() -> &'static Mutex<Option<(i32, i32)>> {
    MOUSE_DRAG_START.get_or_init(|| Mutex::new(None))
}
pub static MGMT_INBOX: OnceLock<Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>> =
    OnceLock::new();
pub static EXEC_REPLY_CHANNELS: OnceLock<Mutex<HashMap<String, mpsc::Sender<String>>>> =
    OnceLock::new();
pub static BOOT_INSTANT: OnceLock<std::time::Instant> = OnceLock::new();
#[allow(dead_code)]
pub static LAST_MENUBAR_DRAW: OnceLock<Mutex<(std::time::Instant, Option<String>)>> =
    OnceLock::new();
pub static APP_DIRTY: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
pub static HOVERED_APP: OnceLock<Mutex<Option<String>>> = OnceLock::new();
pub static APP_LOG_LEVELS: OnceLock<Mutex<HashMap<String, supervisor::ipc::LogLevel>>> =
    OnceLock::new();
pub fn app_log_levels() -> &'static Mutex<HashMap<String, supervisor::ipc::LogLevel>> {
    APP_LOG_LEVELS.get_or_init(|| Mutex::new(HashMap::new()))
}
pub static FLUSH_COUNTS: OnceLock<Mutex<HashMap<String, (u64, std::time::Instant)>>> =
    OnceLock::new();
pub fn flush_counts() -> &'static Mutex<HashMap<String, (u64, std::time::Instant)>> {
    FLUSH_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}
pub static SHOW_MENU_BAR: OnceLock<bool> = OnceLock::new();
pub static SHOW_DOCK: OnceLock<bool> = OnceLock::new();
pub static WINDOWED_MODE: OnceLock<bool> = OnceLock::new();
pub static FOCUS_RING: OnceLock<bool> = OnceLock::new();
pub static DISPLAY_PROFILE: OnceLock<String> = OnceLock::new();
pub static SCREEN_SIZE: OnceLock<(u32, u32)> = OnceLock::new();

#[cfg(target_os = "linux")]
pub static FONT_CACHE: OnceLock<Mutex<super::font::cache::FontCache>> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn font_cache() -> &'static Mutex<super::font::cache::FontCache> {
    FONT_CACHE.get_or_init(|| {
        Mutex::new(super::font::cache::FontCache::load(
            "/fonts/Inter-Regular.ttf",
            "/fonts/Inter-Bold.ttf",
            "/fonts/IBMPlexMono-Regular.ttf",
        ))
    })
}

pub static IMAGE_CACHE: OnceLock<Mutex<super::image::ImageCache>> = OnceLock::new();

pub fn image_cache() -> &'static Mutex<super::image::ImageCache> {
    IMAGE_CACHE.get_or_init(|| Mutex::new(super::image::ImageCache::new()))
}
