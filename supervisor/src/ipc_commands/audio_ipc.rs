// Audio IPC command handlers — volume, volume-get, mute, unmute.

use crate::{log_info, log_warn, send_reply, Inbox};
use supervisor::logging::Subsystem;

/// Handle audio-related @supervisor IPC commands. Returns `true` if handled.
pub fn handle_audio_ipc(
    verb:   &str,
    parts:  &[&str],
    sender: &str,
    inbox:  &Inbox,
) -> bool {
    match verb {
        "volume" => {
            let vol_str = parts.get(1).unwrap_or(&"").trim();
            match vol_str.parse::<u32>() {
                Ok(v) if v <= 100 => {
                    let old_vol = crate::audio::audio_state().lock().unwrap().volume;
                    let reply = crate::audio::ipc_set_volume(v as u8);
                    crate::undo::capture_volume(old_vol, v as u8);
                    log_info!(Subsystem::Ipc, None, "volume set to {v} by {sender}");
                    send_reply(sender, &reply, inbox);
                }
                Ok(v) => {
                    log_warn!(Subsystem::Ipc, None, "volume {v} out of range (0-100)");
                    send_reply(sender, "REPLY:error: volume must be 0-100", inbox);
                }
                Err(_) => {
                    send_reply(sender, "REPLY:error: usage: volume <0-100>", inbox);
                }
            }
            true
        }
        "volume-get" => {
            let reply = crate::audio::ipc_get_volume();
            send_reply(sender, &reply, inbox);
            true
        }
        "mute" => {
            let was_muted = crate::audio::audio_state().lock().unwrap().muted;
            let reply = crate::audio::ipc_set_mute(true);
            crate::undo::capture_mute(was_muted);
            log_info!(Subsystem::Ipc, None, "audio muted by {sender}");
            send_reply(sender, &reply, inbox);
            true
        }
        "unmute" => {
            let was_muted = crate::audio::audio_state().lock().unwrap().muted;
            let reply = crate::audio::ipc_set_mute(false);
            crate::undo::capture_mute(was_muted);
            log_info!(Subsystem::Ipc, None, "audio unmuted by {sender}");
            send_reply(sender, &reply, inbox);
            true
        }
        _ => false,
    }
}
