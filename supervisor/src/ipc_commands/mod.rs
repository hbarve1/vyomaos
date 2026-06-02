// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Extended @supervisor IPC commands — thin dispatcher to domain submodules.

mod audio_ipc;
mod drag_drop_cmd;
mod focus_cmd;
mod lifecycle_cmd;
mod lock;
mod network_cmd;
mod pkg_cmd;
mod share_cmd;
mod system_cmd;
mod tcp;
mod theme_cmd;
mod undo_cmd;
mod websocket;
mod window_cmd;
mod workspace_cmd;

use crate::{AppRegistry, FocusedApp, Inbox};

/// Handle an extended @supervisor command. Returns `true` if handled, `false` if unknown.
pub fn handle_extended_command(
    verb:         &str,
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> bool {
    let detail = parts.get(1).unwrap_or(&"").trim();
    crate::audit::log_audit(
        sender,
        "ipc",
        &format!("{verb} {detail}"),
        crate::audit::AuditResult::Allowed,
    );

    match verb {
        // Package manager
        "pkg-list" | "pkg-installed" | "pkg-install" | "pkg-remove" => {
            return pkg_cmd::handle(verb, parts, sender, inbox, focused, app_registry);
        }

        // Window management
        "wallpaper" | "raise" | "lower" | "resize" => {
            return window_cmd::handle(verb, parts, sender, inbox, focused, app_registry);
        }

        // Lifecycle & session
        "shutdown" | "reboot" | "notify" | "session-save" | "session-restore" => {
            return lifecycle_cmd::handle(verb, parts, sender, inbox, focused, app_registry);
        }

        // Network
        "dns-resolve" | "tls-info" | "http-get" | "download" => {
            return network_cmd::handle(verb, parts, sender, inbox);
        }

        // TCP pool
        "tcp-connect" | "tcp-send" | "tcp-recv" | "tcp-close" => {
            return tcp::handle_tcp(verb, parts, sender, inbox);
        }

        // WebSocket
        "ws-connect" | "ws-send" | "ws-close" => {
            return websocket::handle_ws(verb, parts, sender, inbox);
        }

        // System, clipboard, display, file ops
        "uptime" | "loglevel" | "ping" | "version" | "screen-size" | "monitors"
        | "clipboard-set" | "clipboard-get" | "clipboard-clear" | "screenshot"
        | "locale" | "open" | "assoc-list" | "assoc-set" => {
            return system_cmd::handle(verb, parts, sender, inbox, app_registry);
        }

        // Audio
        "volume" | "volume-get" | "mute" | "unmute" => {
            return audio_ipc::handle_audio_ipc(verb, parts, sender, inbox);
        }

        // Workspace
        "workspace" | "workspace-move" | "workspace-get" => {
            return workspace_cmd::handle_workspace_command(
                verb, parts, sender, inbox, focused, app_registry,
            );
        }

        // Theme
        "theme" => {
            return theme_cmd::handle_theme_command(parts, sender, inbox, app_registry);
        }

        // Lock/unlock
        "lock" | "unlock" => {
            lock::handle_lock_command(verb, sender, inbox, focused, app_registry);
        }

        // Drag & drop
        "drag-start" | "drag-cancel" => {
            return drag_drop_cmd::handle_drag_drop(verb, &parts, sender, inbox);
        }

        // Share
        "share" | "share-accept" | "share-cancel" => {
            return share_cmd::handle_share(verb, parts, sender, inbox, app_registry);
        }

        // Accessibility
        "a11y" => {
            return crate::accessibility::handle_a11y_command(
                parts, sender, inbox, app_registry,
            );
        }

        // Undo/redo
        "undo" | "redo" | "undo-history" => {
            return undo_cmd::handle_undo_redo(verb, sender, inbox, focused, app_registry);
        }

        // Focus cycling
        "focus-next" | "focus-prev" => {
            return focus_cmd::handle_focus_command(verb, sender, inbox, focused, app_registry);
        }

        // OTA updates
        "update-local" => {
            crate::ota_update::handle_update_local(parts, sender, inbox, focused, app_registry);
        }
        "update-prepare" => { crate::atomic_update::handle_update_prepare(parts, sender, inbox); }
        "update-activate" => { crate::atomic_update::handle_update_activate(sender, inbox); }
        "update-rollback" => { crate::atomic_update::handle_update_rollback(sender, inbox); }
        "update-status" => { crate::atomic_update::handle_update_status(sender, inbox); }
        "check-updates" => { crate::auto_update::handle_check_updates(sender, inbox, app_registry); }
        "auto-update" => { crate::auto_update::handle_auto_update(parts, sender, inbox, focused, app_registry); }

        // VFS
        "vfs-list" | "vfs-stat" | "vfs-mounts" => {
            return crate::vfs::handle_vfs_command(verb, parts, sender, inbox, app_registry);
        }

        // Encrypted storage
        "encrypt" | "decrypt" | "encrypted-write" => {
            return crate::encrypted_store::handle_encrypted_command(verb, parts, sender, inbox);
        }

        // Archive
        "extract" | "archive-list" => {
            return crate::archive_ipc::handle_archive_command(verb, parts, sender, inbox);
        }

        // Trash
        "trash" | "trash-list" | "trash-restore" | "trash-empty" | "trash-size" => {
            return crate::trash::handle_trash_command(verb, parts, sender, inbox);
        }

        // Package registry
        "registry-search" | "registry-list" | "registry-add" => {
            return crate::pkg_registry::handle_registry_command(verb, parts, sender, inbox);
        }

        // App store
        "store-fetch" | "store-search" | "store-install" | "store-uninstall"
        | "store-config" => {
            return crate::store::handle_store_command(verb, parts, sender, inbox);
        }

        // Audit
        "audit-list" | "audit-search" | "audit-clear" => {
            return crate::audit::handle_audit_command(verb, &parts, sender, inbox);
        }

        // Capabilities
        "request-cap" => { crate::cap_request::handle_request_cap(parts, sender, inbox, app_registry); }
        "revoke-cap" => { crate::cap_request::handle_revoke_cap(parts, sender, inbox, app_registry); }
        "list-caps" => { crate::cap_request::handle_list_caps(parts, sender, inbox); }

        // Firewall
        "firewall-list" | "firewall-add" | "firewall-remove" | "firewall-check" => {
            return crate::firewall::handle_firewall_command(verb, parts, sender, inbox);
        }

        // Hardware: memory, CPU, battery, ACPI
        "memory-info" | "memory-apps" | "memory-pressure" => {
            return crate::memory::handle_memory_command(verb, sender, inbox, app_registry);
        }
        "cpu-info" | "cpu-usage" | "cpu-governor" => {
            return crate::cpu::handle_cpu_command(verb, parts, sender, inbox);
        }
        "battery" | "power-profile" => {
            return crate::battery::handle_battery_command(verb, parts, sender, inbox);
        }
        "acpi-tables" | "acpi-thermal" | "acpi-info" => {
            return crate::acpi::handle_acpi_command(verb, parts, sender, inbox);
        }

        // Background services
        "bg-list" | "bg-start" | "bg-stop" => {
            return crate::bg_service::handle_bg_command(
                verb, parts, sender, inbox, focused, app_registry,
            );
        }

        // User accounts
        "user-list" | "user-add" | "user-remove" | "user-login" | "user-logout"
        | "user-whoami" => {
            return crate::user::handle_user_command(verb, parts, sender, inbox);
        }

        // User capabilities
        "user-caps" | "user-set-quota" => {
            return crate::user_caps::handle_user_caps_command(verb, parts, sender, inbox);
        }

        // JIT config
        "jit-config" | "jit-set-opt" | "jit-cache-clear" | "jit-set-fuel" => {
            return crate::jit_config::handle_jit_command(verb, parts, sender, inbox);
        }

        // 2FA
        "2fa-enable" | "2fa-disable" | "2fa-status" => {
            return crate::totp::handle_2fa_command(verb, parts, sender, inbox);
        }

        // Recovery
        "recovery-status" | "recovery-reset" | "recovery-repair" | "recovery-exit" => {
            return crate::recovery::handle_recovery_command(verb, sender, inbox);
        }

        // Secure boot
        "boot-verify" => { crate::secure_boot::handle_boot_verify(sender, inbox); }
        "boot-manifest-generate" => { crate::secure_boot::handle_boot_manifest_generate(sender, inbox); }

        // Backup
        "backup-create" | "backup-list" | "backup-restore" | "backup-delete" => {
            return crate::backup::handle_backup_command(verb, parts, sender, inbox);
        }

        // Installer
        "install-detect-disks" | "install-plan" | "install-execute" => {
            return crate::installer::handle_installer_command(
                verb, parts, sender, inbox, app_registry,
            );
        }

        // UEFI
        "uefi-info" | "uefi-entries" => {
            return crate::uefi::handle_uefi_command(verb, sender, inbox);
        }

        // Virtualization
        "vm-info" | "vm-resources" => {
            return crate::virtualization::handle_vm_command(verb, sender, inbox);
        }

        // VNC
        "vnc-start" | "vnc-stop" | "vnc-status" => {
            return crate::vnc::handle_vnc_command(verb, parts, sender, inbox);
        }

        _ => return false,
    }
    true
}
