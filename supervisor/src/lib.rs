// Supervisor library root — exposes pure modules for integration tests and tools.
pub mod manifest;
pub mod logging;
pub mod ipc;
pub mod lifecycle;
pub mod windows;
pub mod statusbar;
pub mod chrome {
    //! Z-layer constants re-exported for tests and external crates.
    //! The full chrome module (title bars, menu bar, etc.) lives in the binary crate.
    pub const Z_DESKTOP:  u32 = 0;    // desktop wallpaper — always background
    pub const Z_APP:      u32 = 10;   // default app window layer
    pub const Z_DOCK:     u32 = 100;  // dock — always above app windows
    pub const Z_OVERLAY:  u32 = 255;  // notifications, system overlays
}

// 043: Core foundation modules
pub mod runtime;
pub mod hal;
pub mod profile;
pub mod ota;
pub mod observability;
pub mod capability;

pub mod archive;
pub mod undo_public;
pub mod file_assoc;

#[cfg(target_os = "linux")]
pub mod font;
#[cfg(target_os = "linux")]
pub mod display;

// ── Color parsing (platform-independent) ────────────────────────────────────

/// Parse a u32 color value from a string.
///
/// Accepts decimal (`"4294967295"`) or hex with `0x`/`0X` prefix
/// (`"0x0d1117ff"`, `"0X0D1117FF"`).  Leading/trailing whitespace is
/// trimmed.  Returns `None` for empty, non-numeric, or overflow values.
#[inline]
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}
