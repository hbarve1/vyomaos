#![allow(dead_code)]
// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Audio subsystem — state management and VYOMA_AUDIO protocol handler.
//!
//! No real audio device interaction — just state management and logging.
//! Apps with `audio = true` in their vyoma.toml can use VYOMA_AUDIO: commands.

use std::sync::{Mutex, OnceLock};

use supervisor::logging::Subsystem;

/// Global audio state.
pub struct AudioState {
    /// Volume level 0-100.
    pub volume: u8,
    /// Whether audio output is muted.
    pub muted: bool,
    /// Sample rate in Hz.
    pub sample_rate: u32,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            volume: 50,
            muted: false,
            sample_rate: 44100,
        }
    }
}

static AUDIO_STATE: OnceLock<Mutex<AudioState>> = OnceLock::new();

/// Get or initialize the global audio state.
pub fn audio_state() -> &'static Mutex<AudioState> {
    AUDIO_STATE.get_or_init(|| Mutex::new(AudioState::default()))
}

/// Handle a VYOMA_AUDIO command from an app's stdout.
///
/// `cmd` is the portion after the `VYOMA_AUDIO:` prefix.
/// Returns `true` if the command was recognized, `false` otherwise.
pub fn handle_audio_command(cmd: &str, sender: &str) -> bool {
    if let Some(args) = cmd.strip_prefix("beep:") {
        handle_beep(args, sender);
        return true;
    }

    if let Some(vol_str) = cmd.strip_prefix("volume:") {
        handle_volume_set(vol_str, sender);
        return true;
    }

    if cmd == "mute" {
        let mut state = audio_state().lock().unwrap();
        state.muted = true;
        crate::log_info!(Subsystem::Audio, Some(sender), "audio muted");
        return true;
    }

    if cmd == "unmute" {
        let mut state = audio_state().lock().unwrap();
        state.muted = false;
        crate::log_info!(Subsystem::Audio, Some(sender), "audio unmuted");
        return true;
    }

    crate::log_warn!(Subsystem::Audio, Some(sender), "unknown VYOMA_AUDIO command: {cmd}");
    false
}

fn handle_beep(args: &str, sender: &str) {
    let parts: Vec<&str> = args.splitn(2, ',').collect();
    if parts.len() != 2 {
        crate::log_error!(Subsystem::Audio, Some(sender), "bad beep args: {args} (expected freq,duration_ms)");
        return;
    }
    let freq: u32 = match parts[0].trim().parse() {
        Ok(f) => f,
        Err(_) => {
            crate::log_error!(Subsystem::Audio, Some(sender), "bad beep freq: {}", parts[0]);
            return;
        }
    };
    let duration_ms: u32 = match parts[1].trim().parse() {
        Ok(d) => d,
        Err(_) => {
            crate::log_error!(Subsystem::Audio, Some(sender), "bad beep duration: {}", parts[1]);
            return;
        }
    };
    let state = audio_state().lock().unwrap();
    let muted_tag = if state.muted { " (muted)" } else { "" };
    crate::log_info!(Subsystem::Audio, Some(sender),
        "beep: freq={freq}Hz duration={duration_ms}ms volume={}{muted_tag}",
        state.volume);
}

fn handle_volume_set(vol_str: &str, sender: &str) {
    let vol: u8 = match vol_str.trim().parse::<u32>() {
        Ok(v) if v <= 100 => v as u8,
        Ok(v) => {
            crate::log_warn!(Subsystem::Audio, Some(sender),
                "volume {v} out of range, clamping to 100");
            100
        }
        Err(_) => {
            crate::log_error!(Subsystem::Audio, Some(sender), "bad volume value: {vol_str}");
            return;
        }
    };
    audio_state().lock().unwrap().volume = vol;
    crate::log_info!(Subsystem::Audio, Some(sender), "volume set to {vol}");
}

/// Set volume from an IPC command. Returns a reply string.
pub fn ipc_set_volume(vol: u8) -> String {
    let clamped = vol.min(100);
    audio_state().lock().unwrap().volume = clamped;
    format!("REPLY:volume {clamped}")
}

/// Get current volume. Returns a reply string.
pub fn ipc_get_volume() -> String {
    let state = audio_state().lock().unwrap();
    let muted_tag = if state.muted { " (muted)" } else { "" };
    format!("REPLY:volume {}{muted_tag}", state.volume)
}

/// Set muted state. Returns a reply string.
pub fn ipc_set_mute(muted: bool) -> String {
    audio_state().lock().unwrap().muted = muted;
    if muted {
        "REPLY:muted".to_string()
    } else {
        "REPLY:unmuted".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_state_default() {
        let state = AudioState::default();
        assert_eq!(state.volume, 50);
        assert!(!state.muted);
        assert_eq!(state.sample_rate, 44100);
    }

    #[test]
    fn test_volume_set_get() {
        let reply = ipc_set_volume(75);
        assert_eq!(reply, "REPLY:volume 75");

        let reply = ipc_get_volume();
        assert!(reply.starts_with("REPLY:volume 75"));
    }

    #[test]
    fn test_volume_clamp() {
        let reply = ipc_set_volume(200);
        assert_eq!(reply, "REPLY:volume 100");
    }

    #[test]
    fn test_mute_toggle() {
        let reply = ipc_set_mute(true);
        assert_eq!(reply, "REPLY:muted");
        assert!(audio_state().lock().unwrap().muted);

        let reply = ipc_set_mute(false);
        assert_eq!(reply, "REPLY:unmuted");
        assert!(!audio_state().lock().unwrap().muted);
    }

    #[test]
    fn test_handle_audio_command_beep() {
        assert!(handle_audio_command("beep:440,100", "test-app"));
    }

    #[test]
    fn test_handle_audio_command_volume() {
        assert!(handle_audio_command("volume:80", "test-app"));
        assert_eq!(audio_state().lock().unwrap().volume, 80);
    }

    #[test]
    fn test_handle_audio_command_mute_unmute() {
        assert!(handle_audio_command("mute", "test-app"));
        assert!(audio_state().lock().unwrap().muted);

        assert!(handle_audio_command("unmute", "test-app"));
        assert!(!audio_state().lock().unwrap().muted);
    }

    #[test]
    fn test_handle_audio_command_unknown() {
        assert!(!handle_audio_command("foobar", "test-app"));
    }
}
