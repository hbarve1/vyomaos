// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

// P08T01: seccomp BPF denylist applied to every wasmtime child process.

#[repr(C)]
pub struct SockFilter {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

#[repr(C)]
pub struct SockFprog {
    pub len: u16,
    pub filter: *const SockFilter,
}

const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
// Return ENOSYS (38) so glibc falls back from clone3 -> clone when threading.
const SECCOMP_RET_ERRNO_ENOSYS: u32 = 0x0005_0000 | 38;

const OFF_NR: u32 = 0;
const OFF_ARCH: u32 = 4;

const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

// Syscalls that return ENOSYS so libc falls back gracefully.
const ENOSYS_FALLBACK: &[u32] = &[
    435, // clone3 — glibc falls back to clone(2) when clone3 returns ENOSYS
];

const DENIED: &[u32] = &[
    101, // ptrace
    169, // reboot
    246, // kexec_load
    248, // add_key
    249, // request_key
    250, // keyctl
    272, // unshare
    317, // seccomp
];

macro_rules! stmt {
    ($code:expr, $k:expr) => {
        SockFilter { code: $code, jt: 0, jf: 0, k: $k }
    };
}
macro_rules! jump {
    ($code:expr, $k:expr, $jt:expr, $jf:expr) => {
        SockFilter { code: $code, jt: $jt, jf: $jf, k: $k }
    };
}

pub fn build() -> Vec<SockFilter> {
    let mut f = vec![
        stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_ARCH),
        jump!(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_X86_64, 1, 0),
        stmt!(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
        stmt!(BPF_LD | BPF_W | BPF_ABS, OFF_NR),
    ];
    for &nr in ENOSYS_FALLBACK {
        f.push(jump!(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1));
        f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_ERRNO_ENOSYS));
    }
    for &nr in DENIED {
        f.push(jump!(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1));
        f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));
    }
    f.push(stmt!(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
    f
}

pub unsafe fn apply(filter: &[SockFilter]) -> std::io::Result<()> {
    if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINVAL) {
            return Err(e);
        }
    }
    let prog = SockFprog {
        len: filter.len() as u16,
        filter: filter.as_ptr(),
    };
    let ret = libc::prctl(
        libc::PR_SET_SECCOMP,
        libc::SECCOMP_MODE_FILTER as libc::c_ulong,
        &prog as *const SockFprog as *const libc::c_void,
        0,
        0,
    );
    if ret < 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EINVAL) {
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}
