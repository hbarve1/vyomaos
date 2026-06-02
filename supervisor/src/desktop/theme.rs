// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Theme engine — dark/light/auto support with runtime switching.
//!
//! Provides a `Theme` struct containing all UI chrome colours and two built-in
//! palettes (dark and light).  The active theme name is stored in a global
//! `OnceLock<Mutex<&'static str>>` and can be switched at runtime via the
//! `@supervisor: theme <dark|light|auto>` IPC command.

use std::sync::{Mutex, OnceLock};
use crate::lock_or_recover;

// ── Theme struct ─────────────────────────────────────────────────────────────

/// All chrome colours used by the supervisor UI (menu bar, title bars, text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg:             u32,
    pub surface:        u32,
    pub border:         u32,
    pub accent:         u32,
    pub text:           u32,
    pub dim:            u32,
    pub menubar:        u32,
    pub title_active:   u32,
    pub title_inactive: u32,
}

// ── Predefined palettes ──────────────────────────────────────────────────────

/// Dark theme — matches the original chrome.rs hardcoded values.
pub static DARK: Theme = Theme {
    bg:             0x1C1C1EFF,
    surface:        0x2A2A2AFF,
    border:         0x3A3A3CFF,
    accent:         0x0A84FFFF,
    text:           0xEBEBEBFF,
    dim:            0x8E8E93FF,
    menubar:        0x2A2A2AFF,
    title_active:   0x323232FF,
    title_inactive: 0x282828FF,
};

/// Light theme — bright variant with dark text.
pub static LIGHT: Theme = Theme {
    bg:             0xF5F5F7FF,
    surface:        0xFFFFFFFF,
    border:         0xD1D1D6FF,
    accent:         0x007AFFFF,
    text:           0x1C1C1EFF,
    dim:            0x8E8E93FF,
    menubar:        0xF2F2F7FF,
    title_active:   0xE8E8EDFF,
    title_inactive: 0xF0F0F5FF,
};

// ── Active theme state ───────────────────────────────────────────────────────

/// The active theme name: `"dark"`, `"light"`, or `"auto"`.
static ACTIVE_THEME: OnceLock<Mutex<&'static str>> = OnceLock::new();

fn active_lock() -> &'static Mutex<&'static str> {
    ACTIVE_THEME.get_or_init(|| Mutex::new("dark"))
}

/// Return the name of the active theme (`"dark"`, `"light"`, or `"auto"`).
pub fn active_name() -> &'static str {
    *lock_or_recover(&active_lock())
}

/// Resolve `"auto"` to a concrete theme.  For now auto always maps to dark;
/// a future implementation could read a time-of-day or user preference file.
fn resolve_auto() -> &'static Theme {
    &DARK
}

/// Return the currently-active `Theme` (resolving `"auto"` if needed).
pub fn current_theme() -> &'static Theme {
    match active_name() {
        "light" => &LIGHT,
        "auto"  => resolve_auto(),
        _       => &DARK,    // "dark" or any unknown value
    }
}

/// Set the active theme name.  Returns `true` if the name was valid and
/// changed, `false` if it was already set to the same value or invalid.
pub fn set_active(name: &str) -> bool {
    let static_name: &'static str = match name {
        "dark"  => "dark",
        "light" => "light",
        "auto"  => "auto",
        _       => return false,
    };
    let mut lock = lock_or_recover(&active_lock());
    if *lock == static_name {
        return false;
    }
    *lock = static_name;
    true
}

/// Valid theme names accepted by `set_active`.
pub const VALID_NAMES: &[&str] = &["dark", "light", "auto"];

// ── Broadcast helper ─────────────────────────────────────────────────────────

/// Broadcast `VYOMA_SYSTEM:theme:<name>` to every running display app.
///
/// `inbox` and `registry` are the shared supervisor maps. This iterates all
/// apps, checks `has_display && Running`, and sends the system message.
pub fn broadcast_theme_change(
    name: &str,
    inbox: &crate::Inbox,
    registry: &crate::AppRegistry,
) {
    let msg = format!("VYOMA_SYSTEM:theme:{name}");
    let reg = lock_or_recover(&registry);
    let inb = lock_or_recover(&inbox);
    for (app_name, state_arc) in reg.iter() {
        let st = lock_or_recover(&state_arc);
        if st.has_display && matches!(st.status, crate::AppStatus::Running) {
            if let Some(tx) = inb.get(app_name) {
                let _ = tx.send(msg.clone());
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_dark() {
        // OnceLock ensures first access initialises to "dark".
        let t = current_theme();
        assert_eq!(t.bg, DARK.bg);
        assert_eq!(t.text, DARK.text);
        assert_eq!(t.menubar, DARK.menubar);
    }

    #[test]
    fn switch_to_light_returns_correct_colors() {
        // Force initialise in case another test ran first.
        let _ = active_lock();
        set_active("light");
        let t = current_theme();
        assert_eq!(t.bg, LIGHT.bg);
        assert_eq!(t.text, LIGHT.text);
        assert_eq!(t.menubar, LIGHT.menubar);
        assert_eq!(t.title_active, LIGHT.title_active);
        // Reset to dark so we don't leak state to other tests.
        set_active("dark");
    }

    #[test]
    fn auto_resolves_to_dark() {
        let _ = active_lock();
        set_active("auto");
        let t = current_theme();
        assert_eq!(t.bg, DARK.bg);
        set_active("dark");
    }

    #[test]
    fn invalid_name_rejected() {
        assert!(!set_active("purple"));
    }

    #[test]
    fn set_same_returns_false() {
        let _ = active_lock();
        set_active("dark");
        assert!(!set_active("dark"));
    }

    #[test]
    fn valid_names_list() {
        assert_eq!(VALID_NAMES, &["dark", "light", "auto"]);
    }

    #[test]
    fn dark_and_light_differ() {
        assert_ne!(DARK.bg, LIGHT.bg);
        assert_ne!(DARK.text, LIGHT.text);
        assert_ne!(DARK.menubar, LIGHT.menubar);
    }
}
