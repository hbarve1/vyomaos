// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P105: OS installer framework — detect disks, plan partitioning, execute
//! installation (rootfs copy, bootloader placeholder, data partition), and
//! broadcast progress via `VYOMA_SYSTEM:install-progress` messages.

use crate::{log_info, log_warn, send_reply, AppRegistry, Inbox};
use supervisor::logging::Subsystem;

// ── Install target model ────────────────────────────────────────────────────

/// Describes the target device and partitioning plan for an OS install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTarget {
    pub device: String,
    pub partition_table: String, // "gpt" or "mbr"
    pub root_size_mb: u64,
    pub data_size_mb: u64,
}

/// Disk information parsed from `/proc/partitions` or `/sys/block/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    pub name: String,       // e.g., "vda"
    pub size_mb: u64,       // total size in megabytes
    pub removable: bool,    // true if the device is removable (USB stick, etc.)
}

/// Installation progress tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallProgress {
    pub step: String,
    pub percent: u8,
    pub status: InstallStatus,
}

/// Status of an individual installation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStatus {
    Pending,
    InProgress,
    Done,
    Failed(String),
}

impl std::fmt::Display for InstallStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallStatus::Pending => write!(f, "pending"),
            InstallStatus::InProgress => write!(f, "in-progress"),
            InstallStatus::Done => write!(f, "done"),
            InstallStatus::Failed(msg) => write!(f, "failed:{msg}"),
        }
    }
}

// ── Disk detection ──────────────────────────────────────────────────────────

/// Parse `/proc/partitions` content and return a list of whole disks (no
/// partition suffixes like `vda1`).  Each line after the header has the format:
/// `major minor  #blocks  name`.
pub fn parse_proc_partitions(content: &str) -> Vec<DiskInfo> {
    let mut disks = Vec::new();
    for line in content.lines().skip(2) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[3];
        // Skip partitions (e.g., vda1, sda2) — only keep whole disks.
        if name.chars().last().map_or(true, |c| c.is_ascii_digit())
            && name.len() > 2
            && name[..name.len() - 1]
                .chars()
                .last()
                .map_or(false, |c| c.is_ascii_alphabetic())
            && !is_whole_disk_name(name)
        {
            continue;
        }
        // Skip ram disks, loop devices, and other non-installable devices.
        if name.starts_with("ram")
            || name.starts_with("loop")
            || name.starts_with("dm-")
        {
            continue;
        }
        let blocks: u64 = parts[2].parse().unwrap_or(0);
        let size_mb = blocks / 1024; // 1 block = 1 KB
        if size_mb == 0 {
            continue;
        }
        let removable = read_removable(name);
        disks.push(DiskInfo {
            name: name.to_string(),
            size_mb,
            removable,
        });
    }
    disks
}

/// Returns true if `name` looks like a whole disk (no trailing digit after
/// a letter prefix), e.g., "vda", "sda", "nvme0n1".
fn is_whole_disk_name(name: &str) -> bool {
    // nvme devices: nvme0n1 is a whole disk, nvme0n1p1 is a partition.
    if name.starts_with("nvme") {
        return !name.contains('p') || {
            // nvme0n1 has no 'p' after 'n', nvme0n1p1 does.
            let after_n = name.rfind('n').map(|i| &name[i + 1..]).unwrap_or("");
            !after_n.contains('p')
        };
    }
    // Standard disks: vda, sda, hda — whole disk ends with a letter.
    name.chars().last().map_or(false, |c| c.is_ascii_alphabetic())
}

/// Detect disks by reading `/proc/partitions` on the live system.
pub fn detect_disks() -> Vec<DiskInfo> {
    match std::fs::read_to_string("/proc/partitions") {
        Ok(content) => parse_proc_partitions(&content),
        Err(e) => {
            log_warn!(Subsystem::Lifecycle, None, "cannot read /proc/partitions: {e}");
            Vec::new()
        }
    }
}

/// Check if a device is removable by reading `/sys/block/<name>/removable`.
fn read_removable(name: &str) -> bool {
    let path = format!("/sys/block/{name}/removable");
    std::fs::read_to_string(path)
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

// ── Partition planning ──────────────────────────────────────────────────────

/// Default root partition size in MB (256 MB — enough for kernel + initramfs).
const DEFAULT_ROOT_MB: u64 = 256;
/// Minimum data partition size in MB.
const MIN_DATA_MB: u64 = 64;
/// Minimum total disk size in MB to be installable.
const MIN_DISK_MB: u64 = DEFAULT_ROOT_MB + MIN_DATA_MB;

/// Generate an install plan for the given device.
pub fn plan_install(device: &str, disks: &[DiskInfo]) -> Result<InstallTarget, String> {
    let disk = disks
        .iter()
        .find(|d| d.name == device || format!("/dev/{}", d.name) == device)
        .ok_or_else(|| format!("device '{device}' not found"))?;

    if disk.size_mb < MIN_DISK_MB {
        return Err(format!(
            "disk {} is too small ({} MB, need >= {} MB)",
            disk.name, disk.size_mb, MIN_DISK_MB
        ));
    }

    let root_size_mb = DEFAULT_ROOT_MB;
    let data_size_mb = disk.size_mb - root_size_mb;
    let partition_table = if disk.size_mb > 2 * 1024 * 1024 {
        "gpt".to_string()
    } else {
        "mbr".to_string()
    };

    Ok(InstallTarget {
        device: format!("/dev/{}", disk.name),
        partition_table,
        root_size_mb,
        data_size_mb,
    })
}

// ── Install steps (logged / placeholder) ────────────────────────────────────

/// Broadcast an install progress message to the sender via IPC.
fn broadcast_progress(sender: &str, step: &str, percent: u8, inbox: &Inbox) {
    send_reply(sender, &format!("VYOMA_SYSTEM:install-progress:{step}:{percent}"), inbox);
}

/// Format the disk (placeholder -- logs the partitioning plan).
pub fn format_disk(target: &InstallTarget) -> Result<(), String> {
    log_info!(Subsystem::Lifecycle, None,
        "install: format_disk {} table={} root={}MB data={}MB",
        target.device, target.partition_table, target.root_size_mb, target.data_size_mb);
    Ok(())
}

/// Copy the initramfs to the root partition (placeholder -- validates path).
pub fn install_rootfs(target: &InstallTarget, initramfs_path: &str) -> Result<(), String> {
    let exists = std::path::Path::new(initramfs_path).exists();
    log_info!(Subsystem::Lifecycle, None,
        "install: install_rootfs {} initramfs={} exists={}", target.device, initramfs_path, exists);
    if !exists { return Err(format!("initramfs not found at {initramfs_path}")); }
    Ok(())
}

/// Install the bootloader (placeholder for GRUB/syslinux).
pub fn install_bootloader(target: &InstallTarget) -> Result<(), String> {
    log_info!(Subsystem::Lifecycle, None,
        "install: install_bootloader {} (placeholder)", target.device);
    Ok(())
}

/// Create and format the data partition as ext4 (placeholder).
pub fn create_data_partition(target: &InstallTarget) -> Result<(), String> {
    log_info!(Subsystem::Lifecycle, None,
        "install: create_data_partition {} size={}MB (placeholder)", target.device, target.data_size_mb);
    Ok(())
}

/// Run the full installation sequence, broadcasting progress to the sender.
pub fn run_install(
    target: &InstallTarget,
    initramfs_path: &str,
    sender: &str,
    inbox: &Inbox,
) -> Result<(), String> {
    let steps = [
        ("format-disk", 10u8),
        ("install-rootfs", 40),
        ("install-bootloader", 70),
        ("create-data-partition", 90),
        ("complete", 100),
    ];

    broadcast_progress(sender, steps[0].0, steps[0].1, inbox);
    format_disk(target)?;

    broadcast_progress(sender, steps[1].0, steps[1].1, inbox);
    install_rootfs(target, initramfs_path)?;

    broadcast_progress(sender, steps[2].0, steps[2].1, inbox);
    install_bootloader(target)?;

    broadcast_progress(sender, steps[3].0, steps[3].1, inbox);
    create_data_partition(target)?;

    broadcast_progress(sender, steps[4].0, steps[4].1, inbox);
    log_info!(Subsystem::Lifecycle, None, "install: complete on {}", target.device);
    Ok(())
}

// ── IPC command handlers ────────────────────────────────────────────────────

const DEFAULT_INITRAMFS: &str = "/boot/initramfs.cpio.gz";

/// Handle installer-related `@supervisor:` IPC commands.
/// Returns `true` if the command was handled.
pub fn handle_installer_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
    _app_registry: &AppRegistry,
) -> bool {
    match verb {
        "install-detect-disks" => {
            let disks = detect_disks();
            if disks.is_empty() {
                send_reply(sender, "REPLY:install-disks: none found", inbox);
            } else {
                let rows: Vec<String> = disks
                    .iter()
                    .map(|d| {
                        let rem = if d.removable { " (removable)" } else { "" };
                        format!("{} {}MB{rem}", d.name, d.size_mb)
                    })
                    .collect();
                send_reply(
                    sender,
                    &format!("REPLY:install-disks: {}", rows.join(" | ")),
                    inbox,
                );
            }
            log_info!(Subsystem::Lifecycle, None, "install-detect-disks from {sender}: {} disk(s)", disks.len());
        }

        "install-plan" => {
            let device = match parts.get(1).map(|s| s.trim()) {
                Some(d) if !d.is_empty() => d.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: install-plan <device>", inbox);
                    return true;
                }
            };
            let disks = detect_disks();
            match plan_install(&device, &disks) {
                Ok(target) => {
                    let reply = format!(
                        "REPLY:install-plan: device={} table={} root={}MB data={}MB",
                        target.device, target.partition_table,
                        target.root_size_mb, target.data_size_mb,
                    );
                    send_reply(sender, &reply, inbox);
                }
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }

        "install-execute" => {
            let device = match parts.get(1).map(|s| s.trim()) {
                Some(d) if !d.is_empty() => d.to_string(),
                _ => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: install-execute <device>",
                        inbox,
                    );
                    return true;
                }
            };
            let disks = detect_disks();
            let target = match plan_install(&device, &disks) {
                Ok(t) => t,
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                    return true;
                }
            };
            match run_install(&target, DEFAULT_INITRAMFS, sender, inbox) {
                Ok(()) => {
                    send_reply(
                        sender,
                        &format!("REPLY:install-complete on {}", target.device),
                        inbox,
                    );
                }
                Err(e) => {
                    send_reply(
                        sender,
                        &format!("REPLY:install-failed: {e}"),
                        inbox,
                    );
                }
            }
        }

        _ => return false,
    }
    true
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PARTITIONS: &str = "\
major minor  #blocks  name

 253        0   20971520 vda
 253        1   20970496 vda1
   8        0  104857600 sda
   8        1  104856576 sda1
   1        0      65536 ram0
   7        0          0 loop0
";

    #[test]
    fn parse_detects_whole_disks_only() {
        let disks = parse_proc_partitions(SAMPLE_PARTITIONS);
        let names: Vec<&str> = disks.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"vda"), "should contain vda, got {names:?}");
        assert!(names.contains(&"sda"), "should contain sda, got {names:?}");
        assert!(!names.contains(&"vda1"), "should not contain vda1");
        assert!(!names.contains(&"sda1"), "should not contain sda1");
        assert!(!names.contains(&"ram0"), "should not contain ram0");
        assert!(!names.contains(&"loop0"), "should not contain loop0");
    }

    #[test]
    fn parse_disk_sizes() {
        let disks = parse_proc_partitions(SAMPLE_PARTITIONS);
        let vda = disks.iter().find(|d| d.name == "vda").unwrap();
        assert_eq!(vda.size_mb, 20971520 / 1024); // ~20480 MB
        let sda = disks.iter().find(|d| d.name == "sda").unwrap();
        assert_eq!(sda.size_mb, 104857600 / 1024); // ~102400 MB
    }

    #[test]
    fn parse_empty_partitions() {
        let disks = parse_proc_partitions("");
        assert!(disks.is_empty());
    }

    #[test]
    fn parse_header_only() {
        let content = "major minor  #blocks  name\n\n";
        let disks = parse_proc_partitions(content);
        assert!(disks.is_empty());
    }

    #[test]
    fn plan_install_success() {
        let disks = vec![DiskInfo {
            name: "vda".to_string(),
            size_mb: 1024,
            removable: false,
        }];
        let target = plan_install("vda", &disks).unwrap();
        assert_eq!(target.device, "/dev/vda");
        assert_eq!(target.partition_table, "mbr");
        assert_eq!(target.root_size_mb, DEFAULT_ROOT_MB);
        assert_eq!(target.data_size_mb, 1024 - DEFAULT_ROOT_MB);
    }

    #[test]
    fn plan_install_dev_prefix_and_errors() {
        let disks = vec![
            DiskInfo { name: "sda".to_string(), size_mb: 2048, removable: false },
            DiskInfo { name: "vda".to_string(), size_mb: 100, removable: false },
        ];
        // /dev/ prefix works
        assert_eq!(plan_install("/dev/sda", &disks).unwrap().device, "/dev/sda");
        // Too-small disk rejected
        assert!(plan_install("vda", &disks).unwrap_err().contains("too small"));
        // Unknown device rejected
        assert!(plan_install("sdb", &disks).unwrap_err().contains("not found"));
    }

    #[test]
    fn plan_install_partition_table_selection() {
        let small = vec![DiskInfo { name: "sda".to_string(), size_mb: 500 * 1024, removable: false }];
        assert_eq!(plan_install("sda", &small).unwrap().partition_table, "mbr");
        let large = vec![DiskInfo { name: "sda".to_string(), size_mb: 3 * 1024 * 1024, removable: false }];
        assert_eq!(plan_install("sda", &large).unwrap().partition_table, "gpt");
    }

    #[test]
    fn is_whole_disk_standard() {
        assert!(is_whole_disk_name("vda"));
        assert!(is_whole_disk_name("sda"));
        assert!(is_whole_disk_name("hda"));
        assert!(!is_whole_disk_name("vda1"));
        assert!(!is_whole_disk_name("sda2"));
    }

    #[test]
    fn is_whole_disk_nvme() {
        assert!(is_whole_disk_name("nvme0n1"));
        assert!(!is_whole_disk_name("nvme0n1p1"));
        assert!(!is_whole_disk_name("nvme0n1p2"));
    }

    #[test]
    fn install_status_display() {
        assert_eq!(format!("{}", InstallStatus::Pending), "pending");
        assert_eq!(format!("{}", InstallStatus::InProgress), "in-progress");
        assert_eq!(format!("{}", InstallStatus::Done), "done");
        assert_eq!(format!("{}", InstallStatus::Failed("io".into())), "failed:io");
    }

    #[test]
    fn install_progress_and_placeholders() {
        let p = InstallProgress { step: "x".into(), percent: 10, status: InstallStatus::InProgress };
        assert_eq!(p.percent, 10);

        let t = InstallTarget {
            device: "/dev/vda".into(), partition_table: "gpt".into(),
            root_size_mb: 256, data_size_mb: 768,
        };
        assert!(format_disk(&t).is_ok());
        assert!(install_bootloader(&t).is_ok());
        assert!(create_data_partition(&t).is_ok());
        assert!(install_rootfs(&t, "/nonexistent/initramfs").unwrap_err().contains("not found"));
    }

    #[test]
    fn parse_nvme_partitions() {
        let content = "\
major minor  #blocks  name

 259        0  500107608 nvme0n1
 259        1     524288 nvme0n1p1
 259        2  499582296 nvme0n1p2
";
        let disks = parse_proc_partitions(content);
        let names: Vec<&str> = disks.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"nvme0n1"), "should contain nvme0n1, got {names:?}");
        assert!(!names.contains(&"nvme0n1p1"), "should not contain nvme0n1p1");
        assert!(!names.contains(&"nvme0n1p2"), "should not contain nvme0n1p2");
    }
}
