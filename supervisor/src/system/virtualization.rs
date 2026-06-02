// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P110: Virtualization support framework — detects hypervisor type, nested
//! virtualization capability, KVM availability, container environments, and
//! exposes VM resource queries via IPC commands.

use crate::{log_info, Inbox};
use supervisor::logging::Subsystem;

// ── VM info struct ──────────────────────────────────────────────────────────

/// Collected virtualization information about the running environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmInfo {
    /// Whether we are running inside a virtual machine.
    pub is_virtual: bool,
    /// Detected hypervisor name (e.g. "kvm", "vmware", "none").
    pub hypervisor: String,
    /// Whether nested virtualization is possible (hardware extensions present
    /// inside a VM, or `/sys/module/kvm_intel/parameters/nested` == Y).
    pub nested_virt: bool,
    /// Whether `/dev/kvm` exists and is accessible.
    pub kvm_available: bool,
    /// Whether the CPU has hardware virtualization extensions (vmx or svm).
    pub cpu_virt_ext: bool,
}

// ── Hypervisor detection ────────────────────────────────────────────────────

/// Check `/proc/cpuinfo` content for the `hypervisor` flag in the flags line.
pub fn cpuinfo_has_hypervisor_flag(content: &str) -> bool {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("flags") {
            if let Some(flags_str) = rest.trim_start().strip_prefix(':') {
                return flags_str.split_whitespace().any(|f| f == "hypervisor");
            }
        }
    }
    false
}

/// Check `/proc/cpuinfo` content for hardware virtualization extensions
/// (`vmx` for Intel VT-x, `svm` for AMD-V).
pub fn cpuinfo_has_virt_extensions(content: &str) -> bool {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("flags") {
            if let Some(flags_str) = rest.trim_start().strip_prefix(':') {
                return flags_str
                    .split_whitespace()
                    .any(|f| f == "vmx" || f == "svm");
            }
        }
    }
    false
}

/// Parse the hypervisor type from `/sys/hypervisor/type` content.
pub fn parse_sys_hypervisor(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_lowercase())
    }
}

/// Parse DMI product name content to identify known hypervisors.
pub fn parse_dmi_product(content: &str) -> Option<String> {
    let lower = content.trim().to_lowercase();
    if lower.contains("virtualbox") {
        Some("virtualbox".to_string())
    } else if lower.contains("vmware") {
        Some("vmware".to_string())
    } else if lower.contains("kvm") || lower.contains("qemu") {
        Some("kvm".to_string())
    } else if lower.contains("hyper-v") || lower.contains("microsoft") {
        Some("hyper-v".to_string())
    } else if lower.contains("xen") {
        Some("xen".to_string())
    } else if lower.contains("bhyve") {
        Some("bhyve".to_string())
    } else {
        None
    }
}

/// Detect the hypervisor by checking multiple sources. Returns `None` when
/// running on bare metal.
///
/// Check order:
/// 1. `/sys/hypervisor/type` (Xen and some KVM setups)
/// 2. DMI product name (`/sys/class/dmi/id/product_name`)
/// 3. `/proc/cpuinfo` hypervisor flag (generic fallback)
pub fn detect_hypervisor() -> Option<String> {
    // 1. /sys/hypervisor/type
    if let Ok(content) = std::fs::read_to_string("/sys/hypervisor/type") {
        if let Some(hv) = parse_sys_hypervisor(&content) {
            return Some(hv);
        }
    }

    // 2. DMI product name
    if let Ok(content) = std::fs::read_to_string("/sys/class/dmi/id/product_name") {
        if let Some(hv) = parse_dmi_product(&content) {
            return Some(hv);
        }
    }

    // 3. /proc/cpuinfo hypervisor flag
    if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
        if cpuinfo_has_hypervisor_flag(&content) {
            // We know we are virtualized but cannot determine the exact hypervisor.
            return Some("unknown".to_string());
        }
    }

    None
}

// ── Container detection ─────────────────────────────────────────────────────

/// Check cgroup content (from `/proc/1/cgroup`) for Docker/LXC indicators.
pub fn cgroup_is_container(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("docker") || lower.contains("lxc") || lower.contains("kubepods")
}

/// Detect whether we are running inside a container (Docker, LXC, Kubernetes).
///
/// Checks:
/// 1. `/.dockerenv` file existence
/// 2. `/proc/1/cgroup` for docker/lxc/kubepods strings
/// 3. `/run/.containerenv` (Podman)
pub fn is_container() -> bool {
    if std::path::Path::new("/.dockerenv").exists() {
        return true;
    }
    if std::path::Path::new("/run/.containerenv").exists() {
        return true;
    }
    if let Ok(content) = std::fs::read_to_string("/proc/1/cgroup") {
        if cgroup_is_container(&content) {
            return true;
        }
    }
    false
}

// ── Nested virtualization detection ─────────────────────────────────────────

/// Check whether nested virtualization is enabled via sysfs.
fn nested_virt_enabled() -> bool {
    // Intel: /sys/module/kvm_intel/parameters/nested
    if let Ok(content) = std::fs::read_to_string("/sys/module/kvm_intel/parameters/nested") {
        let val = content.trim();
        if val == "Y" || val == "1" {
            return true;
        }
    }
    // AMD: /sys/module/kvm_amd/parameters/nested
    if let Ok(content) = std::fs::read_to_string("/sys/module/kvm_amd/parameters/nested") {
        let val = content.trim();
        if val == "Y" || val == "1" {
            return true;
        }
    }
    false
}

// ── KVM availability ────────────────────────────────────────────────────────

/// Check whether `/dev/kvm` exists.
fn kvm_available() -> bool {
    std::path::Path::new("/dev/kvm").exists()
}

// ── Full VM info ────────────────────────────────────────────────────────────

/// Read complete virtualization information from the live system.
pub fn read_vm_info() -> VmInfo {
    let hypervisor = detect_hypervisor();
    let is_virtual = hypervisor.is_some();
    let hv_name = hypervisor.unwrap_or_else(|| "none".to_string());
    let kvm = kvm_available();
    let nested = nested_virt_enabled();
    let cpu_ext = std::fs::read_to_string("/proc/cpuinfo")
        .map(|c| cpuinfo_has_virt_extensions(&c))
        .unwrap_or(false);

    VmInfo {
        is_virtual,
        hypervisor: hv_name,
        nested_virt: nested,
        kvm_available: kvm,
        cpu_virt_ext: cpu_ext,
    }
}

// ── VM resource queries ─────────────────────────────────────────────────────

/// Parse `/proc/cpuinfo` content to count the number of logical CPUs.
pub fn parse_cpu_count(content: &str) -> usize {
    let mut count = 0usize;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("processor") {
            if rest.trim_start().starts_with(':') {
                count += 1;
            }
        }
    }
    if count == 0 { 1 } else { count }
}

/// Read the number of logical CPUs visible to this VM/system.
pub fn vm_cpu_count() -> usize {
    let content = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    parse_cpu_count(&content)
}

/// Parse `/proc/meminfo` content to extract `MemTotal` in MB.
pub fn parse_mem_total_mb(content: &str) -> u64 {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let trimmed = rest.trim().trim_end_matches("kB").trim();
            if let Ok(kb) = trimmed.parse::<u64>() {
                return kb / 1024;
            }
        }
    }
    0
}

/// Read total system memory in MB from `/proc/meminfo`.
pub fn vm_memory_mb() -> u64 {
    let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    parse_mem_total_mb(&content)
}

// ── IPC command handlers ────────────────────────────────────────────────────

/// Handle virtualization-related `@supervisor:` IPC commands.
/// Returns `true` if the command was handled.
pub fn handle_vm_command(
    verb: &str,
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "vm-info" => {
            let info = read_vm_info();
            let container = is_container();
            let reply = format!(
                "REPLY:vm is_virtual={} hypervisor={} nested={} kvm={} virt_ext={} container={}",
                info.is_virtual,
                info.hypervisor,
                info.nested_virt,
                info.kvm_available,
                info.cpu_virt_ext,
                container,
            );
            log_info!(Subsystem::Ipc, None, "vm-info query from {sender}");
            crate::send_reply(sender, &reply, inbox);
        }
        "vm-resources" => {
            let cpus = vm_cpu_count();
            let mem = vm_memory_mb();
            let reply = format!("REPLY:vm-resources cpus={cpus} memory={mem}MB");
            log_info!(Subsystem::Ipc, None, "vm-resources query from {sender}");
            crate::send_reply(sender, &reply, inbox);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO_VM: &str = "processor\t: 0\nvendor_id\t: GenuineIntel\n\
        model name\t: QEMU Virtual CPU version 2.5+\n\
        flags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge hypervisor\n";

    const CPUINFO_BARE: &str = "processor\t: 0\nvendor_id\t: GenuineIntel\n\
        model name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz\n\
        flags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge vmx\n\n\
        processor\t: 1\nvendor_id\t: GenuineIntel\n\
        model name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz\n\
        flags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge vmx\n";

    const CPUINFO_AMD: &str = "processor\t: 0\nvendor_id\t: AuthenticAMD\n\
        model name\t: AMD EPYC 7R13 48-Core Processor\n\
        flags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep svm\n";

    #[test]
    fn hypervisor_flag_present() { assert!(cpuinfo_has_hypervisor_flag(CPUINFO_VM)); }
    #[test]
    fn hypervisor_flag_absent()  { assert!(!cpuinfo_has_hypervisor_flag(CPUINFO_BARE)); }
    #[test]
    fn hypervisor_flag_empty()   { assert!(!cpuinfo_has_hypervisor_flag("")); }

    #[test]
    fn virt_ext_vmx()     { assert!(cpuinfo_has_virt_extensions(CPUINFO_BARE)); }
    #[test]
    fn virt_ext_svm()     { assert!(cpuinfo_has_virt_extensions(CPUINFO_AMD)); }
    #[test]
    fn virt_ext_guest()   { assert!(!cpuinfo_has_virt_extensions(CPUINFO_VM)); }
    #[test]
    fn virt_ext_empty()   { assert!(!cpuinfo_has_virt_extensions("")); }

    #[test]
    fn dmi_virtualbox() { assert_eq!(parse_dmi_product("VirtualBox"), Some("virtualbox".into())); }
    #[test]
    fn dmi_vmware()     { assert_eq!(parse_dmi_product("VMware Virtual Platform"), Some("vmware".into())); }
    #[test]
    fn dmi_kvm()        { assert_eq!(parse_dmi_product("Standard PC (Q35 + ICH9, 2009) QEMU"), Some("kvm".into())); }
    #[test]
    fn dmi_hyperv()     { assert_eq!(parse_dmi_product("Microsoft Corporation Virtual Machine"), Some("hyper-v".into())); }
    #[test]
    fn dmi_xen()        { assert_eq!(parse_dmi_product("Xen HVM domU"), Some("xen".into())); }
    #[test]
    fn dmi_bhyve()      { assert_eq!(parse_dmi_product("bhyve"), Some("bhyve".into())); }
    #[test]
    fn dmi_bare_metal() { assert_eq!(parse_dmi_product("System Product Name"), None); }
    #[test]
    fn dmi_empty()      { assert_eq!(parse_dmi_product(""), None); }

    #[test]
    fn sys_hv_xen()   { assert_eq!(parse_sys_hypervisor("xen\n"), Some("xen".into())); }
    #[test]
    fn sys_hv_kvm()   { assert_eq!(parse_sys_hypervisor("KVM\n"), Some("kvm".into())); }
    #[test]
    fn sys_hv_empty() {
        assert_eq!(parse_sys_hypervisor(""), None);
        assert_eq!(parse_sys_hypervisor("  \n"), None);
    }

    #[test]
    fn cgroup_docker()  { assert!(cgroup_is_container("12:memory:/docker/abc123\n")); }
    #[test]
    fn cgroup_lxc()     { assert!(cgroup_is_container("12:memory:/lxc/mycontainer\n")); }
    #[test]
    fn cgroup_kubepods(){ assert!(cgroup_is_container("11:devices:/kubepods/pod-xyz\n")); }
    #[test]
    fn cgroup_bare()    { assert!(!cgroup_is_container("12:memory:/\n3:cpu:/\n")); }
    #[test]
    fn cgroup_empty()   { assert!(!cgroup_is_container("")); }

    #[test]
    fn cpu_count_two()   { assert_eq!(parse_cpu_count(CPUINFO_BARE), 2); }
    #[test]
    fn cpu_count_one()   { assert_eq!(parse_cpu_count(CPUINFO_VM), 1); }
    #[test]
    fn cpu_count_empty() { assert_eq!(parse_cpu_count(""), 1); }

    #[test]
    fn mem_16gb()   { assert_eq!(parse_mem_total_mb("MemTotal:       16384000 kB\n"), 16000); }
    #[test]
    fn mem_512mb()  { assert_eq!(parse_mem_total_mb("MemTotal:       524288 kB\n"), 512); }
    #[test]
    fn mem_empty()  { assert_eq!(parse_mem_total_mb(""), 0); }
    #[test]
    fn mem_none()   { assert_eq!(parse_mem_total_mb("SomeOtherField: 12345 kB\n"), 0); }

    #[test]
    fn vm_info_bare_metal() {
        let info = VmInfo {
            is_virtual: false, hypervisor: "none".into(),
            nested_virt: false, kvm_available: false, cpu_virt_ext: true,
        };
        assert!(!info.is_virtual);
        assert_eq!(info.hypervisor, "none");
    }

    #[test]
    fn vm_info_kvm_guest() {
        let info = VmInfo {
            is_virtual: true, hypervisor: "kvm".into(),
            nested_virt: true, kvm_available: true, cpu_virt_ext: false,
        };
        assert!(info.is_virtual);
        assert_eq!(info.hypervisor, "kvm");
        assert!(info.nested_virt);
        assert!(info.kvm_available);
    }
}
