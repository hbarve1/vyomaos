// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Accessibility subsystem — high-contrast, large text, reduce-motion,
//! screen-reader announcements, and font scaling.
//!
//! Global state is stored in a `OnceLock<Mutex<A11yState>>` singleton.
//! IPC commands (`@supervisor: a11y ...`) are handled by [`handle_a11y_command`].

use std::sync::{Mutex, OnceLock};

use crate::{log_info, send_reply, AppRegistry, AppStatus, Inbox};
use supervisor::logging::Subsystem;

// ── Accessibility state ──────────────────────────────────────────────────────

/// Runtime accessibility preferences.
#[derive(Debug, Clone)]
pub struct A11yState {
    /// Boost contrast: pure white text on pure black, brighter accent.
    pub high_contrast: bool,
    /// Use larger font sizes (scaled by `font_scale`).
    pub large_text: bool,
    /// Disable animations system-wide.
    pub reduce_motion: bool,
    /// Emit text descriptions to a screen-reader app via VYOMA_SYSTEM protocol.
    pub screen_reader: bool,
    /// Font scale factor: 1.0 = normal, up to 3.0.
    pub font_scale: f32,
}

impl Default for A11yState {
    fn default() -> Self {
        Self {
            high_contrast: false,
            large_text: false,
            reduce_motion: false,
            screen_reader: false,
            font_scale: 1.0,
        }
    }
}

/// Global accessibility state singleton.
static A11Y: OnceLock<Mutex<A11yState>> = OnceLock::new();

fn a11y_lock() -> &'static Mutex<A11yState> {
    A11Y.get_or_init(|| Mutex::new(A11yState::default()))
}

/// Return a snapshot of the current accessibility state.
pub fn current() -> A11yState {
    a11y_lock().lock().unwrap().clone()
}

// ── High-contrast mode ───────────────────────────────────────────────────────

/// High-contrast theme overrides: pure white on pure black with bright accent.
pub struct HighContrastOverrides {
    pub bg: u32,
    pub text: u32,
    pub accent: u32,
    pub menubar: u32,
    pub title_active: u32,
    pub title_inactive: u32,
    pub border: u32,
    pub surface: u32,
    pub dim: u32,
}

/// Returns high-contrast colour overrides when high-contrast mode is enabled,
/// or `None` when it is disabled.
pub fn high_contrast_overrides() -> Option<HighContrastOverrides> {
    let state = a11y_lock().lock().unwrap();
    if !state.high_contrast {
        return None;
    }
    Some(HighContrastOverrides {
        bg:             0x000000FF, // pure black
        text:           0xFFFFFFFF, // pure white
        accent:         0x00FF00FF, // bright green for maximum visibility
        menubar:        0x000000FF,
        title_active:   0x1A1A1AFF,
        title_inactive: 0x0D0D0DFF,
        border:         0xFFFFFFFF, // white borders
        surface:        0x0A0A0AFF,
        dim:            0xCCCCCCFF, // brighter dim text
    })
}

/// Returns `true` when high-contrast mode is active.
pub fn is_high_contrast() -> bool {
    a11y_lock().lock().unwrap().high_contrast
}

// ── Large text / font scaling ────────────────────────────────────────────────

const MIN_FONT_SCALE: f32 = 1.0;
const MAX_FONT_SCALE: f32 = 3.0;

/// Apply the current font scale to a base point size.
/// When `large_text` is disabled the base size is returned unchanged.
pub fn scaled_pt(base_pt: u32) -> u32 {
    let state = a11y_lock().lock().unwrap();
    if !state.large_text {
        return base_pt;
    }
    let scaled = (base_pt as f32 * state.font_scale).round() as u32;
    scaled.max(1)
}

/// Clamp a font scale value to the valid range [1.0, 3.0].
fn clamp_scale(v: f32) -> f32 {
    v.clamp(MIN_FONT_SCALE, MAX_FONT_SCALE)
}

// ── Reduce motion ────────────────────────────────────────────────────────────

/// Returns `true` when animations should be played.
/// Returns `false` when reduce-motion is enabled (skip all animations).
pub fn animation_enabled() -> bool {
    !a11y_lock().lock().unwrap().reduce_motion
}

// ── Screen reader protocol ───────────────────────────────────────────────────

/// Emit a focus-change announcement to any running screen-reader app.
/// Message format: `VYOMA_SYSTEM:a11y:focus:<app_name>`
pub fn announce_focus(app_name: &str, inbox: &Inbox, registry: &AppRegistry) {
    let state = a11y_lock().lock().unwrap();
    if !state.screen_reader {
        return;
    }
    drop(state); // release lock before sending
    let msg = format!("VYOMA_SYSTEM:a11y:focus:{app_name}");
    broadcast_a11y_message(&msg, inbox, registry);
}

/// Emit a text announcement to any running screen-reader app.
/// Message format: `VYOMA_SYSTEM:a11y:announce:<text>`
pub fn announce_text(text: &str, inbox: &Inbox, registry: &AppRegistry) {
    let state = a11y_lock().lock().unwrap();
    if !state.screen_reader {
        return;
    }
    drop(state);
    let msg = format!("VYOMA_SYSTEM:a11y:announce:{text}");
    broadcast_a11y_message(&msg, inbox, registry);
}

/// Broadcast an accessibility system message to all running apps.
fn broadcast_a11y_message(msg: &str, inbox: &Inbox, registry: &AppRegistry) {
    let reg = registry.lock().unwrap();
    let inb = inbox.lock().unwrap();
    for (name, state_arc) in reg.iter() {
        let st = state_arc.lock().unwrap();
        if matches!(st.status, AppStatus::Running) {
            if let Some(tx) = inb.get(name) {
                let _ = tx.send(msg.to_string());
            }
        }
    }
}

// ── IPC command handler ──────────────────────────────────────────────────────

/// Handle `@supervisor: a11y <subcommand>` IPC commands.
///
/// Supported subcommands:
/// - `high-contrast <on|off>`
/// - `large-text <on|off>`
/// - `reduce-motion <on|off>`
/// - `screen-reader <on|off>`
/// - `font-scale <1.0-3.0>`
/// - (no argument) — return current state summary
///
/// Returns `true` (always handled).
pub fn handle_a11y_command(
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
    _app_registry: &AppRegistry,
) -> bool {
    let sub = parts.get(1).unwrap_or(&"").trim();

    if sub.is_empty() {
        // Report current state
        let st = a11y_lock().lock().unwrap();
        let summary = format!(
            "REPLY:a11y high-contrast={} large-text={} reduce-motion={} screen-reader={} font-scale={:.1}",
            if st.high_contrast { "on" } else { "off" },
            if st.large_text { "on" } else { "off" },
            if st.reduce_motion { "on" } else { "off" },
            if st.screen_reader { "on" } else { "off" },
            st.font_scale,
        );
        send_reply(sender, &summary, inbox);
        return true;
    }

    let value = parts.get(2).unwrap_or(&"").trim();

    match sub {
        "high-contrast" => {
            match parse_on_off(value) {
                Some(v) => {
                    a11y_lock().lock().unwrap().high_contrast = v;
                    log_info!(Subsystem::Display, None, "a11y high-contrast {}", on_off_str(v));
                    send_reply(sender, &format!("REPLY:a11y high-contrast {}", on_off_str(v)), inbox);
                }
                None => {
                    send_reply(sender, "REPLY:error: usage: a11y high-contrast <on|off>", inbox);
                }
            }
        }

        "large-text" => {
            match parse_on_off(value) {
                Some(v) => {
                    a11y_lock().lock().unwrap().large_text = v;
                    log_info!(Subsystem::Display, None, "a11y large-text {}", on_off_str(v));
                    send_reply(sender, &format!("REPLY:a11y large-text {}", on_off_str(v)), inbox);
                }
                None => {
                    send_reply(sender, "REPLY:error: usage: a11y large-text <on|off>", inbox);
                }
            }
        }

        "reduce-motion" => {
            match parse_on_off(value) {
                Some(v) => {
                    a11y_lock().lock().unwrap().reduce_motion = v;
                    log_info!(Subsystem::Display, None, "a11y reduce-motion {}", on_off_str(v));
                    send_reply(sender, &format!("REPLY:a11y reduce-motion {}", on_off_str(v)), inbox);
                }
                None => {
                    send_reply(sender, "REPLY:error: usage: a11y reduce-motion <on|off>", inbox);
                }
            }
        }

        "screen-reader" => {
            match parse_on_off(value) {
                Some(v) => {
                    a11y_lock().lock().unwrap().screen_reader = v;
                    log_info!(Subsystem::Display, None, "a11y screen-reader {}", on_off_str(v));
                    send_reply(sender, &format!("REPLY:a11y screen-reader {}", on_off_str(v)), inbox);
                }
                None => {
                    send_reply(sender, "REPLY:error: usage: a11y screen-reader <on|off>", inbox);
                }
            }
        }

        "font-scale" => {
            match value.parse::<f32>() {
                Ok(v) => {
                    let clamped = clamp_scale(v);
                    a11y_lock().lock().unwrap().font_scale = clamped;
                    log_info!(Subsystem::Display, None, "a11y font-scale {clamped:.1}");
                    send_reply(sender, &format!("REPLY:a11y font-scale {clamped:.1}"), inbox);
                }
                Err(_) => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: a11y font-scale <1.0-3.0>",
                        inbox,
                    );
                }
            }
        }

        _ => {
            send_reply(
                sender,
                "REPLY:error: unknown a11y subcommand (try: high-contrast, large-text, reduce-motion, screen-reader, font-scale)",
                inbox,
            );
        }
    }
    true
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn parse_on_off(s: &str) -> Option<bool> {
    match s {
        "on" | "true" | "1" => Some(true),
        "off" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn on_off_str(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state() {
        let st = A11yState::default();
        assert!(!st.high_contrast);
        assert!(!st.large_text);
        assert!(!st.reduce_motion);
        assert!(!st.screen_reader);
        assert!((st.font_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn font_scale_clamping() {
        assert!((clamp_scale(0.5) - 1.0).abs() < f32::EPSILON);
        assert!((clamp_scale(1.0) - 1.0).abs() < f32::EPSILON);
        assert!((clamp_scale(2.0) - 2.0).abs() < f32::EPSILON);
        assert!((clamp_scale(3.0) - 3.0).abs() < f32::EPSILON);
        assert!((clamp_scale(5.0) - 3.0).abs() < f32::EPSILON);
        assert!((clamp_scale(-1.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn scaled_pt_no_large_text() {
        // When large_text is off, scaled_pt returns the base value unchanged.
        // Note: this test relies on the global singleton defaulting to large_text=false.
        let base = 12u32;
        let result = scaled_pt(base);
        assert_eq!(result, base);
    }

    #[test]
    fn high_contrast_overrides_when_disabled() {
        // Default state has high_contrast=false → None.
        // OnceLock may already be initialised by other tests, so we just check the
        // function does not panic and returns consistently with the current state.
        let st = a11y_lock().lock().unwrap();
        let enabled = st.high_contrast;
        drop(st);
        let result = high_contrast_overrides();
        if enabled {
            assert!(result.is_some());
        } else {
            assert!(result.is_none());
        }
    }

    #[test]
    fn animation_enabled_default() {
        // Default reduce_motion is false → animations enabled.
        let st = a11y_lock().lock().unwrap();
        let rm = st.reduce_motion;
        drop(st);
        assert_eq!(animation_enabled(), !rm);
    }

    #[test]
    fn parse_on_off_values() {
        assert_eq!(parse_on_off("on"), Some(true));
        assert_eq!(parse_on_off("off"), Some(false));
        assert_eq!(parse_on_off("true"), Some(true));
        assert_eq!(parse_on_off("false"), Some(false));
        assert_eq!(parse_on_off("1"), Some(true));
        assert_eq!(parse_on_off("0"), Some(false));
        assert_eq!(parse_on_off("maybe"), None);
        assert_eq!(parse_on_off(""), None);
    }

    #[test]
    fn on_off_str_values() {
        assert_eq!(on_off_str(true), "on");
        assert_eq!(on_off_str(false), "off");
    }
}
