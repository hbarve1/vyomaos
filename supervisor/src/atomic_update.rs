// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P108: Atomic OS updates with A/B slot model.
//!
//! Extends the existing OTA subsystem with persistent slot state, atomic
//! activation/rollback, and automatic health-check based recovery.
//! Slot state is persisted at `/data/slots/slot-state.toml`.

use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use supervisor::logging::Subsystem;

// ── Slot model ───────────────────────────────────────────────────────────────

/// Status of an A/B update slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotStatus {
    Active,
    Standby,
    Pending,
    Invalid,
}

impl std::fmt::Display for SlotStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlotStatus::Active  => write!(f, "active"),
            SlotStatus::Standby => write!(f, "standby"),
            SlotStatus::Pending => write!(f, "pending"),
            SlotStatus::Invalid => write!(f, "invalid"),
        }
    }
}

/// Metadata for one update slot (A or B).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotInfo {
    pub slot: char,
    pub version: String,
    pub sha256: String,
    pub status: SlotStatus,
    pub boot_count: u32,
}

impl SlotInfo {
    fn new(slot: char) -> Self {
        Self {
            slot,
            version: String::new(),
            sha256: String::new(),
            status: if slot == 'A' { SlotStatus::Active } else { SlotStatus::Standby },
            boot_count: 0,
        }
    }
}

/// Persistent state for both slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotState {
    pub slot_a: SlotInfo,
    pub slot_b: SlotInfo,
}

// ── Constants ────────────────────────────────────────────────────────────────

const SLOT_STATE_PATH: &str = "/data/slots/slot-state.toml";
const SLOT_DIR_A: &str = "/data/slots/A";
const SLOT_DIR_B: &str = "/data/slots/B";
const MAX_BOOT_COUNT: u32 = 3;

// ── Persistence ──────────────────────────────────────────────────────────────

/// Load slot state from disk, or create a default if absent.
pub fn load_state() -> SlotState {
    load_state_from(SLOT_STATE_PATH)
}

fn load_state_from(path: &str) -> SlotState {
    match fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).unwrap_or_else(|_| default_state()),
        Err(_) => default_state(),
    }
}

fn default_state() -> SlotState {
    SlotState {
        slot_a: SlotInfo::new('A'),
        slot_b: SlotInfo::new('B'),
    }
}

/// Save slot state to disk atomically (write tmp + rename).
pub fn save_state(state: &SlotState) -> Result<(), String> {
    save_state_to(state, SLOT_STATE_PATH)
}

fn save_state_to(state: &SlotState, path: &str) -> Result<(), String> {
    let raw = toml::to_string_pretty(state)
        .map_err(|e| format!("serialize slot state: {e}"))?;
    let parent = Path::new(path).parent().unwrap_or(Path::new("/data/slots"));
    fs::create_dir_all(parent)
        .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, &raw)
        .map_err(|e| format!("write {tmp}: {e}"))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {tmp} -> {path}: {e}"))?;
    Ok(())
}

// ── SHA-256 helper ───────────────────────────────────────────────────────────

fn sha256_file(path: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

// ── Slot directory helper ────────────────────────────────────────────────────

fn slot_dir(slot: char) -> &'static str {
    if slot == 'A' { SLOT_DIR_A } else { SLOT_DIR_B }
}

fn standby_slot(state: &SlotState) -> &SlotInfo {
    if state.slot_a.status == SlotStatus::Active { &state.slot_b } else { &state.slot_a }
}

fn active_slot(state: &SlotState) -> &SlotInfo {
    if state.slot_a.status == SlotStatus::Active { &state.slot_a } else { &state.slot_b }
}

fn active_slot_mut(state: &mut SlotState) -> &mut SlotInfo {
    if state.slot_a.status == SlotStatus::Active { &mut state.slot_a } else { &mut state.slot_b }
}

fn standby_slot_mut(state: &mut SlotState) -> &mut SlotInfo {
    if state.slot_a.status == SlotStatus::Active { &mut state.slot_b } else { &mut state.slot_a }
}

// ── Core update flow ─────────────────────────────────────────────────────────

/// Stage a new initramfs image to the standby slot.
///
/// 1. Verify the SHA-256 of the source image.
/// 2. Copy to the standby slot directory.
/// 3. Mark the standby slot as `Pending`.
pub fn prepare_update(new_initramfs: &str, expected_sha256: &str) -> Result<(), String> {
    let mut state = load_state();
    prepare_update_inner(&mut state, new_initramfs, expected_sha256, SLOT_STATE_PATH)
}

fn prepare_update_inner(
    state: &mut SlotState,
    new_initramfs: &str,
    expected_sha256: &str,
    state_path: &str,
) -> Result<(), String> {
    // Verify source image integrity.
    let actual = sha256_file(new_initramfs)?;
    if actual != expected_sha256.to_lowercase() {
        return Err(format!(
            "SHA-256 mismatch: expected {expected_sha256}, got {actual}"
        ));
    }

    // Determine standby slot and its target directory.
    let target_slot = standby_slot(state).slot;
    let target_dir = slot_dir(target_slot);
    fs::create_dir_all(target_dir)
        .map_err(|e| format!("create {target_dir}: {e}"))?;

    let filename = Path::new(new_initramfs)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "initramfs.cpio.gz".to_string());
    let dest = format!("{target_dir}/{filename}");

    // Atomic copy: write to tmp, rename.
    let tmp = format!("{dest}.tmp");
    let bytes = fs::read(new_initramfs)
        .map_err(|e| format!("read {new_initramfs}: {e}"))?;
    fs::write(&tmp, &bytes)
        .map_err(|e| format!("write {tmp}: {e}"))?;
    fs::rename(&tmp, &dest)
        .map_err(|e| format!("rename {tmp} -> {dest}: {e}"))?;

    // Update standby slot metadata.
    let sb = standby_slot_mut(state);
    sb.status = SlotStatus::Pending;
    sb.sha256 = actual;
    sb.boot_count = 0;

    save_state_to(state, state_path)
}

/// Activate the pending standby slot, swapping it to active.
pub fn activate_update() -> Result<(), String> {
    let mut state = load_state();
    activate_update_inner(&mut state, SLOT_STATE_PATH)
}

fn activate_update_inner(state: &mut SlotState, state_path: &str) -> Result<(), String> {
    // Snapshot which slot is currently active before any mutation.
    let a_is_active = state.slot_a.status == SlotStatus::Active;
    let (active, standby) = if a_is_active {
        (&mut state.slot_a, &mut state.slot_b)
    } else {
        (&mut state.slot_b, &mut state.slot_a)
    };
    if standby.status != SlotStatus::Pending {
        return Err("no pending update to activate".to_string());
    }
    active.status = SlotStatus::Standby;
    standby.status = SlotStatus::Active;
    standby.boot_count = 0;

    save_state_to(state, state_path)
}

/// Roll back to the previous active slot.
pub fn rollback() -> Result<(), String> {
    let mut state = load_state();
    rollback_inner(&mut state, SLOT_STATE_PATH)
}

fn rollback_inner(state: &mut SlotState, state_path: &str) -> Result<(), String> {
    // Snapshot which slot is currently active before any mutation.
    let a_is_active = state.slot_a.status == SlotStatus::Active;
    let (active, standby) = if a_is_active {
        (&mut state.slot_a, &mut state.slot_b)
    } else {
        (&mut state.slot_b, &mut state.slot_a)
    };
    active.status = SlotStatus::Invalid;
    standby.status = SlotStatus::Active;

    save_state_to(state, state_path)
}

// ── Health check ─────────────────────────────────────────────────────────────

/// Increment boot count on the active slot. If it exceeds MAX_BOOT_COUNT,
/// trigger an automatic rollback and return `Err`.
pub fn check_health_on_boot() -> Result<(), String> {
    let mut state = load_state();
    check_health_on_boot_inner(&mut state, SLOT_STATE_PATH)
}

fn check_health_on_boot_inner(state: &mut SlotState, state_path: &str) -> Result<(), String> {
    active_slot_mut(state).boot_count += 1;
    let count = active_slot(state).boot_count;

    if count > MAX_BOOT_COUNT {
        eprintln!(
            "{}",
            supervisor::logging::format_log(
                supervisor::logging::Level::Warn,
                Subsystem::Lifecycle,
                None,
                &format!("boot_count={count} exceeds {MAX_BOOT_COUNT}, auto-rolling back"),
            )
        );
        rollback_inner(state, state_path)?;
        return Err(format!("auto-rollback: boot_count {count} > {MAX_BOOT_COUNT}"));
    }

    save_state_to(state, state_path)
}

/// Mark the current boot as successful. Resets boot_count to 0.
/// Should be called after all apps have spawned successfully.
pub fn mark_boot_successful() -> Result<(), String> {
    let mut state = load_state();
    mark_boot_successful_inner(&mut state, SLOT_STATE_PATH)
}

fn mark_boot_successful_inner(state: &mut SlotState, state_path: &str) -> Result<(), String> {
    active_slot_mut(state).boot_count = 0;
    save_state_to(state, state_path)
}

/// Return a human-readable status string for both slots.
pub fn format_status(state: &SlotState) -> String {
    format!(
        "slot_A: {} v={} sha={} boots={} | slot_B: {} v={} sha={} boots={}",
        state.slot_a.status, state.slot_a.version,
        truncate_sha(&state.slot_a.sha256), state.slot_a.boot_count,
        state.slot_b.status, state.slot_b.version,
        truncate_sha(&state.slot_b.sha256), state.slot_b.boot_count,
    )
}

fn truncate_sha(sha: &str) -> &str {
    if sha.len() > 12 { &sha[..12] } else { sha }
}

// ── IPC handlers ─────────────────────────────────────────────────────────────

/// Handle `@supervisor: update-prepare <path> <sha256>`.
pub fn handle_update_prepare(parts: &[&str], sender: &str, inbox: &crate::Inbox) {
    let rest = parts.get(1).unwrap_or(&"").trim();
    let (path, sha) = match rest.split_once(' ') {
        Some((p, s)) if !p.is_empty() && !s.is_empty() => (p.trim(), s.trim()),
        _ => {
            crate::send_reply(sender, "REPLY:error: usage: update-prepare <path> <sha256>", inbox);
            return;
        }
    };
    match prepare_update(path, sha) {
        Ok(()) => {
            crate::log_info!(Subsystem::Lifecycle, None, "update staged from {path}");
            crate::send_reply(sender, "REPLY:update-prepare ok", inbox);
        }
        Err(e) => crate::send_reply(sender, &format!("REPLY:error: {e}"), inbox),
    }
}

/// Handle `@supervisor: update-activate`.
pub fn handle_update_activate(sender: &str, inbox: &crate::Inbox) {
    match activate_update() {
        Ok(()) => {
            crate::log_info!(Subsystem::Lifecycle, None, "update activated");
            crate::send_reply(sender, "REPLY:update-activate ok", inbox);
        }
        Err(e) => crate::send_reply(sender, &format!("REPLY:error: {e}"), inbox),
    }
}

/// Handle `@supervisor: update-rollback`.
pub fn handle_update_rollback(sender: &str, inbox: &crate::Inbox) {
    match rollback() {
        Ok(()) => {
            crate::log_info!(Subsystem::Lifecycle, None, "update rolled back");
            crate::send_reply(sender, "REPLY:update-rollback ok", inbox);
        }
        Err(e) => crate::send_reply(sender, &format!("REPLY:error: {e}"), inbox),
    }
}

/// Handle `@supervisor: update-status`.
pub fn handle_update_status(sender: &str, inbox: &crate::Inbox) {
    let state = load_state();
    let status = format_status(&state);
    crate::send_reply(sender, &format!("REPLY:{status}"), inbox);
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_state_path(dir: &tempfile::TempDir) -> String {
        dir.path().join("slot-state.toml").to_string_lossy().to_string()
    }

    #[test]
    fn test_default_state() {
        let s = default_state();
        assert_eq!(s.slot_a.status, SlotStatus::Active);
        assert_eq!(s.slot_b.status, SlotStatus::Standby);
        assert_eq!(s.slot_a.boot_count, 0);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_state_path(&dir);
        let mut state = default_state();
        state.slot_a.version = "1.0.0".to_string();
        state.slot_a.sha256 = "abc123".to_string();
        save_state_to(&state, &path).unwrap();

        let loaded = load_state_from(&path);
        assert_eq!(loaded.slot_a.version, "1.0.0");
        assert_eq!(loaded.slot_a.sha256, "abc123");
        assert_eq!(loaded.slot_a.status, SlotStatus::Active);
    }

    #[test]
    fn test_prepare_sha_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = tmp_state_path(&dir);
        let img_path = dir.path().join("new-initramfs.cpio.gz");
        fs::write(&img_path, b"fake initramfs").unwrap();

        let mut state = default_state();
        let bad = prepare_update_inner(
            &mut state, img_path.to_str().unwrap(),
            "0000000000000000000000000000000000000000000000000000000000000000",
            &state_path,
        );
        assert!(bad.is_err());
        assert!(bad.unwrap_err().contains("SHA-256 mismatch"));
    }

    #[test]
    fn test_activate_requires_pending() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_state_path(&dir);
        let mut state = default_state();
        save_state_to(&state, &path).unwrap();
        let err = activate_update_inner(&mut state, &path);
        assert!(err.unwrap_err().contains("no pending update"));
    }

    #[test]
    fn test_activate_swaps_slots() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_state_path(&dir);
        let mut state = default_state();
        state.slot_b.status = SlotStatus::Pending;
        activate_update_inner(&mut state, &path).unwrap();
        assert_eq!(state.slot_b.status, SlotStatus::Active);
        assert_eq!(state.slot_a.status, SlotStatus::Standby);
    }

    #[test]
    fn test_rollback_marks_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_state_path(&dir);
        let mut state = default_state();
        rollback_inner(&mut state, &path).unwrap();
        assert_eq!(state.slot_a.status, SlotStatus::Invalid);
        assert_eq!(state.slot_b.status, SlotStatus::Active);
    }

    #[test]
    fn test_health_check_and_auto_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_state_path(&dir);
        let mut state = default_state();
        check_health_on_boot_inner(&mut state, &path).unwrap();
        assert_eq!(active_slot(&state).boot_count, 1);
        check_health_on_boot_inner(&mut state, &path).unwrap();
        assert_eq!(active_slot(&state).boot_count, 2);
        // Push to MAX_BOOT_COUNT, next should auto-rollback.
        state.slot_a.boot_count = MAX_BOOT_COUNT;
        let result = check_health_on_boot_inner(&mut state, &path);
        assert!(result.unwrap_err().contains("auto-rollback"));
        assert_eq!(state.slot_a.status, SlotStatus::Invalid);
        assert_eq!(state.slot_b.status, SlotStatus::Active);
    }

    #[test]
    fn test_mark_boot_successful() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_state_path(&dir);
        let mut state = default_state();
        state.slot_a.boot_count = 2;
        mark_boot_successful_inner(&mut state, &path).unwrap();
        assert_eq!(active_slot(&state).boot_count, 0);
    }

    #[test]
    fn test_format_status_and_display() {
        let state = default_state();
        let s = format_status(&state);
        assert!(s.contains("slot_A: active"));
        assert!(s.contains("slot_B: standby"));
        assert_eq!(format!("{}", SlotStatus::Pending), "pending");
        assert_eq!(format!("{}", SlotStatus::Invalid), "invalid");
    }

    #[test]
    fn test_full_update_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_state_path(&dir);
        let mut state = default_state();
        state.slot_b.status = SlotStatus::Pending;
        state.slot_b.version = "2.0.0".to_string();
        activate_update_inner(&mut state, &path).unwrap();
        assert_eq!(state.slot_b.status, SlotStatus::Active);
        check_health_on_boot_inner(&mut state, &path).unwrap();
        assert_eq!(active_slot(&state).boot_count, 1);
        mark_boot_successful_inner(&mut state, &path).unwrap();
        assert_eq!(active_slot(&state).boot_count, 0);
    }
}
