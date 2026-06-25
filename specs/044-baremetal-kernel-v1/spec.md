# Specification: VyomaOS v1.0 — Bare-Metal Kernel

**Spec Number**: 044  
**Branch**: `docs/vyomaos-v1-baremetal-plan`  
**Created**: 2026-06-25  
**Status**: Planning  
**Deadline**: December 2026  
**ARM64 Milestone**: September 2026

---

## Overview

VyomaOS v1.0 replaces the Linux 5.10 hardware layer with a purpose-built bare-metal kernel written in Rust. The WASM-first application model and Wasmtime supervisor are preserved; what changes is the layer beneath them — from a Linux process to a Rust kernel running directly on hardware.

This removes the last non-VyomaOS dependency from the stack.

## Motivation

The current architecture (Linux kernel → Rust supervisor → WASM apps) achieves security at the application layer but inherits the full Linux attack surface below the supervisor. A bare-metal kernel:

- Eliminates the Linux kernel CVE surface entirely
- Gives VyomaOS full control of the boot sequence on all target hardware
- Enables deployment on hardware where Linux is unavailable or too heavy (MCUs, specialized ARM SoCs)
- Makes the "runtime is the OS boundary" claim literally true — no POSIX userland, no kernel modules, no shell

## Architecture

```
Hardware (ARM64 / x86-64 / RISC-V)
  └── VyomaOS Bare-Metal Kernel (Rust, no_std)
        ├── HAL layer (arch-specific: ARM64 / x86 / RISC-V)
        │     ├── Boot (UEFI / bare-metal startup.S)
        │     ├── MMU (page tables, virtual memory)
        │     ├── Interrupt controller (GICv3 / APIC / PLIC)
        │     ├── Timer (ARM generic / HPET / CLINT)
        │     └── UART console (PL011 / 16550)
        ├── Memory subsystem
        │     ├── PMM: buddy page-frame allocator
        │     ├── VMM: per-process virtual address spaces
        │     └── Kernel heap: slab allocator (GlobalAlloc)
        ├── Process scheduler
        │     ├── TCB + kernel thread structures
        │     ├── Context switch (arch-specific assembly)
        │     └── Preemptive round-robin (100 Hz timer)
        ├── WASM runtime (wasmtime / wasmer, no_std embed)
        │     ├── Module loader (.wasm binary parser)
        │     ├── Syscall ABI (vyoma namespace host functions)
        │     └── Process model (one WASM module = one process)
        ├── IPC: synchronous message-passing channels
        ├── Storage
        │     ├── Block device abstraction (HAL trait)
        │     ├── Drivers: virtio-blk (QEMU), SDHCI (RPi4)
        │     ├── FAT32 filesystem driver
        │     └── VFS: mount table, file descriptors, path resolution
        ├── Networking
        │     ├── virtio-net driver (QEMU)
        │     └── smoltcp TCP/IP stack
        └── Userspace (WASM modules)
              ├── init.wasm (PID 1, service manager)
              ├── shell.wasm (CLI)
              └── utils.wasm (ls, cat, echo, ps, kill, env)
```

## Target Platforms

| Platform | Priority | Target Date | Entry Point | Notes |
|----------|----------|-------------|-------------|-------|
| ARM64 QEMU virt | **Critical** | Aug 2026 | Bare-metal + UEFI | Primary dev target |
| Raspberry Pi 4 | High | Oct 2026 | U-Boot → supervisor | BCM2711, GIC-400 |
| x86-64 QEMU Q35 | High | Nov 2026 | UEFI (OVMF) | APIC, HPET |
| RISC-V QEMU virt | Medium | Dec 2026 | OpenSBI → supervisor | Sv39 MMU |

## Stack

| Component | Technology |
|-----------|------------|
| Kernel language | Rust (no_std, 2021 edition) |
| WASM runtime | wasmtime or wasmer (no_std embed — see task 10) |
| WASM target | wasm32-wasip2 (existing apps unchanged) |
| Boot (ARM64) | startup.S + UEFI |
| Boot (x86-64) | UEFI (PE binary) |
| Boot (RISC-V) | OpenSBI fw_jump |
| Block device (QEMU) | virtio-blk MMIO |
| Block device (RPi4) | SDHCI EMMC2 |
| Filesystem | FAT32 |
| Networking | virtio-net + smoltcp |
| CI | GitHub Actions + QEMU |

## Constraints

- Supervisor binary: ≤ 4 MB stripped (no Linux overhead)
- Boot time: ≤ 5s on ARM64 QEMU to shell prompt
- WASM cold start: ≤ 10ms for any app
- No C userland at any layer
- All apps remain unmodified wasm32-wasip2 binaries — kernel change is transparent to apps

## Milestones

| Milestone | Date | Definition of Done |
|-----------|------|--------------------|
| **ARM64 QEMU Demo** | Sep 2026 | Tasks 1–29: boots to shell in Docker/QEMU, CI green |
| **RPi4 Real Hardware** | Oct 2026 | Tasks 30–34: boots on physical RPi4, SD card boot |
| **x86-64 Port** | Nov 2026 | Tasks 35–39: QEMU Q35 boots to shell |
| **RISC-V Port** | Dec 2026 | Tasks 40–43: QEMU riscv64 boots to shell |
| **v1.0 Release** | Dec 2026 | Tasks 44–45: all platforms green in CI, Docker demo published |
