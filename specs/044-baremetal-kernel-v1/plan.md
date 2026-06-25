# Implementation Plan: VyomaOS v1.0 — Bare-Metal Kernel

**Branch**: `docs/vyomaos-v1-baremetal-plan` | **Date**: 2026-06-25 | **Spec**: [spec.md](spec.md)

## Summary

Build a bare-metal Rust kernel that replaces Linux 5.10 as VyomaOS's hardware layer. ARM64 is the primary target (Sep 2026 demo). x86-64 and RISC-V ports follow (Dec 2026). All 77+ existing WASM apps run unchanged.

## Wave Structure

Work is organized into 10 waves. Waves 1–5 deliver the ARM64 QEMU demo (Sep 2026). Waves 6–10 add real hardware, networking, and platform ports (Dec 2026).

| Wave | Scope | Tasks | Target |
|------|-------|-------|--------|
| 1 | ARM64 hardware foundation | 1–9 | Aug 2026 |
| 2 | WASM runtime integration | 10–13 | Aug 2026 |
| 3 | Process & scheduling | 14–17 | Sep 2026 |
| 4 | IPC | 18 | Sep 2026 |
| 5 | Storage, VFS, userspace, demo | 19–29 | Sep 2026 |
| 6 | Networking | 30–31 | Oct 2026 |
| 7 | RPi4 real hardware | 32–34 | Oct 2026 |
| 8 | x86-64 port | 35–39 | Nov 2026 |
| 9 | RISC-V port | 40–43 | Nov–Dec 2026 |
| 10 | Cross-platform CI & release | 44–45 | Dec 2026 |

## Dependency Graph (Critical Path)

```
[1] ARM64 Boot
 ├──[2] Exception Vectors ──[6] GICv3 ──[7] Timer
 ├──[3] PMM ──[5] Heap ──[10] WASM RT ──[11] Loader ──[12] Syscall ABI ──[13] Syslib
 │                  └──[14] TCB
 ├──[4] MMU (← 2,3)
 └──[8] HAL Traits
      └──[9] ARM64 HAL (← 2,3,4,5,6,7,8)
           ├──[15] Ctx Switch (← 9,14) ──[16] Scheduler (← 7,15) ──[17] WASM Proc (← 12,16)
           │                                                              └──[18] IPC
           ├──[19] Blk Abstraction ──[20] Virtio-blk (← 9,19) ──[27] QEMU Image ──[28] Docker
           │                  └──[21] FAT32 ──[22] VFS ──[23] FS Syscalls (← 12,22)
           │                                                └──[24] Init (← 17,23)
           │                                                      └──[25] Shell (← 24,13)
           │                                                            └──[26] Utils
           ├──[32] RPi4 HAL ──[33] SDHCI ──[34] RPi4 Boot
           ├──[35] x86 Boot ──[36] x86 HAL ──[37] x86 Paging ──[38] x86 CtxSwitch ──[39] x86 QEMU
           └──[40] RISC-V Boot ──[41] RISC-V HAL ──[42] RISC-V CtxSwitch ──[43] RISC-V QEMU
                                                         └──[44] Image Builder ──[45] E2E CI
```

## Key Technical Decisions

### 1. WASM Runtime: wasmtime vs wasmer

Both support no_std embeds. Evaluate in task 10:
- **wasmtime**: Cranelift JIT (fast), larger binary, better maintained
- **wasmer**: Multiple backends (Singlepass for MCU), smaller
- Decision criterion: which compiles to bare-metal AArch64 without OS hooks

### 2. Memory Layout (ARM64)

```
0x0000_0000_0000_0000 – 0x0000_7FFF_FFFF_FFFF   User VA (TTBR0_EL1)
0xFFFF_8000_0000_0000 – 0xFFFF_FFFF_FFFF_FFFF   Kernel VA (TTBR1_EL1)
  └── 0xFFFF_8000_0000_0000   Kernel image load address
  └── 0xFFFF_8000_4000_0000   Kernel heap (slab allocator)
  └── 0xFFFF_8000_8000_0000   MMIO regions (UART, GIC, timer)
```

### 3. Syscall ABI

WASM imports under the `"vyoma"` namespace. All functions use i32/i64 params (wasm primitive types). Pointers are u32 offsets into WASM linear memory; kernel bounds-checks against `memory.size` before any dereference.

```
vyoma::sys_write(fd: i32, ptr: i32, len: i32) -> i32
vyoma::sys_read(fd: i32, ptr: i32, len: i32) -> i32
vyoma::sys_exit(code: i32) -> !
vyoma::sys_open(path_ptr: i32, path_len: i32, flags: i32) -> i32
vyoma::sys_close(fd: i32) -> i32
vyoma::sys_spawn(wasm_ptr: i32, wasm_len: i32, args_ptr: i32) -> i32
vyoma::sys_wait(pid: i32) -> i32
vyoma::sys_getpid() -> i32
vyoma::sys_sched_yield() -> i32
vyoma::sys_chan_create() -> i64          # returns (send_fd << 32 | recv_fd)
vyoma::sys_send(fd: i32, ptr: i32, len: i32) -> i32
vyoma::sys_recv(fd: i32, ptr: i32, len: i32) -> i32
vyoma::sys_socket(domain: i32, typ: i32) -> i32
vyoma::sys_connect(fd: i32, addr_ptr: i32, addr_len: i32) -> i32
```

### 4. HAL Trait Design

```rust
pub trait Console: Send + Sync {
    fn write_byte(&self, b: u8);
}

pub trait InterruptController: Send + Sync {
    fn enable(&self, irq: u32);
    fn disable(&self, irq: u32);
    fn ack(&self) -> u32;
    fn eoi(&self, irq: u32);
}

pub trait Timer: Send + Sync {
    fn set_periodic(&self, hz: u32);
    fn monotonic_ns(&self) -> u64;
}

pub trait AddressSpace: Send + Sync {
    fn map(&self, va: usize, pa: usize, flags: PageFlags);
    fn unmap(&self, va: usize);
    fn flush_tlb(&self);
}
```

### 5. Boot Sequence

```
Power on → UEFI firmware (or bare-metal reset vector)
  → startup.S: set EL1 stack, zero BSS, disable MMU cache
  → early_init(): PL011 UART polling, print "VyomaOS"
  → PMM init: parse UEFI memory map, build buddy allocator
  → heap init: GlobalAlloc backed by PMM
  → MMU enable: set TTBR0/TTBR1, TCR_EL1, MAIR_EL1, SCTLR_EL1
  → HAL init: GICv3, generic timer, UART IRQ
  → WASM runtime init: create Engine + Store
  → Load initrd from virtio-blk sector 0 → VFS mount FAT32
  → Spawn /init.wasm → PID 1
  → Scheduler starts → kernel becomes idle task
```

## Repository Layout

New directories added by this spec:

```
vyomaos/
  kernel/                    # new: bare-metal kernel crate (no_std)
    Cargo.toml
    src/
      main.rs                # entry point, boot sequence
      hal/
        mod.rs               # HAL traits
        arm64/               # ARM64 HAL impl
          boot.S             # startup.S
          gicv3.rs
          timer.rs
          mmu.rs
          uart.rs
        x86/                 # x86-64 HAL impl
        riscv/               # RISC-V HAL impl
      mm/
        pmm.rs               # buddy allocator
        vmm.rs               # page table management
        heap.rs              # slab allocator + GlobalAlloc
      proc/
        tcb.rs
        switch.S             # context switch (arch-specific)
        scheduler.rs
      wasm/
        runtime.rs           # wasmtime/wasmer embed
        loader.rs
        syscall.rs           # host function table
        process.rs           # WASM process model
      ipc/
        channel.rs
      blk/
        mod.rs               # BlockDevice trait
        virtio_blk.rs
        sdhci.rs             # RPi4 SDHCI
      fs/
        fat32.rs
        vfs.rs
        syscall.rs
      net/
        virtio_net.rs
        tcpip.rs             # smoltcp embed
  apps/syslib/               # new: WASM-side system library (vyoma-std)
    Cargo.toml
    src/lib.rs
  apps/init/                 # updated: port to bare-metal syscall ABI
  apps/shell/                # updated: port to bare-metal syscall ABI
  apps/utils/                # new: ls, cat, echo, ps, kill, env
  platforms/arm64/           # new: QEMU + RPi4 build configs
  platforms/x86/             # new: QEMU Q35 build config
  platforms/riscv/           # new: QEMU virt64 build config
  docker/bare-metal/         # new: Docker demo image
```
