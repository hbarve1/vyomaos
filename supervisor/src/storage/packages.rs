// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Package manager helpers (P14T01): install, remove, list installed apps.

use std::{fs, path::Path};

use crate::{log_info, AppRegistry, FocusedApp, Inbox};
use supervisor::logging::Subsystem;
use supervisor::manifest::{AppManifest, BootEntry};

use crate::{DATA_APPS_DIR, USER_BOOT_PATH};

/// Built-in package catalog — apps shipped in the initramfs, available to install.
/// Tuple: (name, version, description)
pub const PACKAGES: &[(&str, &str, &str)] = &[
    ("calculator",   "0.1.0", "Basic arithmetic calculator"),
    ("factorial",    "0.1.0", "Recursive factorial computation"),
    ("gui-demo",     "0.1.0", "Framebuffer dashboard demo"),
    ("hello-world",  "0.1.0", "Hello World demo app"),
    ("http-server",  "0.1.0", "HTTP status server on :8080"),
    ("ping",         "0.1.0", "IPC demo — send side (pair with pong)"),
    ("pong",         "0.1.0", "IPC demo — recv side (pair with ping)"),
    ("storage-demo", "0.1.0", "Persistent storage demo"),
];

/// Read the list of user-installed app names from /data/installed.txt.
pub fn read_installed_apps() -> Vec<String> {
    fs::read_to_string(USER_BOOT_PATH)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// Copy app files from initramfs /apps/<name>/ to /data/apps/<name>/,
/// register in /data/installed.txt, and launch immediately.
pub fn install_package(
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
    if let Some(app) = crate::app_threads::spawn_app(&entry, inbox, app_registry) {
        crate::app_threads::launch_app_threads(app, inbox, focused, app_registry);
    }

    log_info!(Subsystem::Lifecycle, Some(name), "pkg: installed {name}");
    Ok(())
}

/// Kill the app (if running), remove its files, and unregister from installed.txt.
pub fn remove_package(name: &str, app_registry: &AppRegistry) -> Result<(), String> {
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
