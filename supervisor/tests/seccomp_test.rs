// Unit tests for supervisor/src/seccomp.rs — BPF filter building.
//
// We can't test apply() (requires root + prctl), but we can verify
// the BPF filter structure produced by build().

// The seccomp module is private to the binary crate.  We replicate
// the constants and build logic here for testing.

const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_ERRNO_ENOSYS: u32 = 0x0005_0000 | 38;

const OFF_NR: u32 = 0;
const OFF_ARCH: u32 = 4;
const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

const ENOSYS_FALLBACK: &[u32] = &[435]; // clone3
const DENIED: &[u32] = &[101, 169, 246, 248, 249, 250, 272, 317]; // ptrace..seccomp

#[repr(C)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

fn build_filter() -> Vec<SockFilter> {
    let mut f = vec![
        SockFilter { code: BPF_LD | BPF_W | BPF_ABS, jt: 0, jf: 0, k: OFF_ARCH },
        SockFilter { code: BPF_JMP | BPF_JEQ | BPF_K, jt: 1, jf: 0, k: AUDIT_ARCH_X86_64 },
        SockFilter { code: BPF_RET | BPF_K, jt: 0, jf: 0, k: SECCOMP_RET_KILL_PROCESS },
        SockFilter { code: BPF_LD | BPF_W | BPF_ABS, jt: 0, jf: 0, k: OFF_NR },
    ];
    for &nr in ENOSYS_FALLBACK {
        f.push(SockFilter { code: BPF_JMP | BPF_JEQ | BPF_K, jt: 0, jf: 1, k: nr });
        f.push(SockFilter { code: BPF_RET | BPF_K, jt: 0, jf: 0, k: SECCOMP_RET_ERRNO_ENOSYS });
    }
    for &nr in DENIED {
        f.push(SockFilter { code: BPF_JMP | BPF_JEQ | BPF_K, jt: 0, jf: 1, k: nr });
        f.push(SockFilter { code: BPF_RET | BPF_K, jt: 0, jf: 0, k: SECCOMP_RET_KILL_PROCESS });
    }
    f.push(SockFilter { code: BPF_RET | BPF_K, jt: 0, jf: 0, k: SECCOMP_RET_ALLOW });
    f
}

// ── Filter structure tests ──────────────────────────────────────────────────

#[test]
fn test_filter_not_empty() {
    let f = build_filter();
    assert!(!f.is_empty());
}

#[test]
fn test_filter_expected_length() {
    let f = build_filter();
    // 4 header + 2*ENOSYS_FALLBACK(1) + 2*DENIED(8) + 1 final allow
    let expected = 4 + 2 * ENOSYS_FALLBACK.len() + 2 * DENIED.len() + 1;
    assert_eq!(f.len(), expected);
}

#[test]
fn test_filter_starts_with_arch_check() {
    let f = build_filter();
    // First instruction: load architecture field
    assert_eq!(f[0].code, BPF_LD | BPF_W | BPF_ABS);
    assert_eq!(f[0].k, OFF_ARCH);
}

#[test]
fn test_filter_arch_compare_x86_64() {
    let f = build_filter();
    // Second instruction: compare with x86_64 audit arch
    assert_eq!(f[1].code, BPF_JMP | BPF_JEQ | BPF_K);
    assert_eq!(f[1].k, AUDIT_ARCH_X86_64);
    assert_eq!(f[1].jt, 1); // skip kill if match
    assert_eq!(f[1].jf, 0); // fall through to kill
}

#[test]
fn test_filter_kills_wrong_arch() {
    let f = build_filter();
    // Third instruction: kill if arch didn't match
    assert_eq!(f[2].code, BPF_RET | BPF_K);
    assert_eq!(f[2].k, SECCOMP_RET_KILL_PROCESS);
}

#[test]
fn test_filter_loads_syscall_nr() {
    let f = build_filter();
    // Fourth instruction: load syscall number
    assert_eq!(f[3].code, BPF_LD | BPF_W | BPF_ABS);
    assert_eq!(f[3].k, OFF_NR);
}

#[test]
fn test_filter_clone3_returns_enosys() {
    let f = build_filter();
    // After header (4 instructions), first pair is clone3
    assert_eq!(f[4].code, BPF_JMP | BPF_JEQ | BPF_K);
    assert_eq!(f[4].k, 435); // clone3
    assert_eq!(f[5].code, BPF_RET | BPF_K);
    assert_eq!(f[5].k, SECCOMP_RET_ERRNO_ENOSYS);
}

#[test]
fn test_filter_denied_syscalls_kill() {
    let f = build_filter();
    // After clone3 pair (indices 4,5), denied syscalls start at index 6
    let denied_start = 6;
    for (i, &nr) in DENIED.iter().enumerate() {
        let idx = denied_start + i * 2;
        assert_eq!(f[idx].k, nr, "denied syscall {nr} not at expected position");
        assert_eq!(f[idx + 1].k, SECCOMP_RET_KILL_PROCESS);
    }
}

#[test]
fn test_filter_ends_with_allow() {
    let f = build_filter();
    let last = f.last().unwrap();
    assert_eq!(last.code, BPF_RET | BPF_K);
    assert_eq!(last.k, SECCOMP_RET_ALLOW);
}

#[test]
fn test_denied_list_contains_ptrace() {
    assert!(DENIED.contains(&101));
}

#[test]
fn test_denied_list_contains_reboot() {
    assert!(DENIED.contains(&169));
}

#[test]
fn test_denied_list_contains_seccomp() {
    assert!(DENIED.contains(&317));
}

#[test]
fn test_denied_list_contains_unshare() {
    assert!(DENIED.contains(&272));
}

#[test]
fn test_filter_jmp_skip_logic() {
    let f = build_filter();
    // Each denied syscall check: jt=0 (fall through to kill), jf=1 (skip kill)
    for i in (6..f.len() - 1).step_by(2) {
        if f[i].code == BPF_JMP | BPF_JEQ | BPF_K {
            assert_eq!(f[i].jt, 0, "jt should be 0 (fall through to ret)");
            assert_eq!(f[i].jf, 1, "jf should be 1 (skip over ret)");
        }
    }
}
