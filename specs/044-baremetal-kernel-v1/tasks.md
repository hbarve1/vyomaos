# Tasks: VyomaOS v1.0 — Bare-Metal Kernel

**Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)  
**Total tasks**: 45 | **ARM64 milestone (Sep 2026)**: tasks 1–29 | **v1.0 (Dec 2026)**: tasks 30–45

## Format

- `[P]` = can run in parallel with siblings that also have `[P]`
- Deps listed as task numbers
- Branch per task; merge to `044-baremetal-kernel-v1` feature branch

---

## Wave 1 — ARM64 Hardware Foundation

**Goal**: Supervisor executes at EL1 with MMU on, timer fires, GICv3 routes IRQs.

- [ ] T001 **ARM64 bare-metal boot** — linker script for kernel image, `kernel/src/hal/arm64/boot.S`: set EL1 stack, zero BSS, jump to `kernel_main()`. Polling PL011 UART for early output. UEFI entry point (EFI_MAIN). Branch: `feat/arm64-boot`.
  - Acceptance: QEMU AArch64 boots supervisor, prints "VyomaOS booting..." via UART, halts cleanly.

- [ ] T002 **ARM64 exception vectors** _(dep: T001)_ — `kernel/src/hal/arm64/exceptions.S`: 16-entry VBAR_EL1 table, dispatch to Rust handlers for sync/IRQ/FIQ/SError. Log EC + FAR_EL1 on unhandled fault. Branch: `feat/arm64-exceptions`.
  - Acceptance: Page fault and timer IRQ handled without triple fault; EC printed to UART.

- [ ] T003 **Physical memory manager** _(dep: T001)_ — `kernel/src/mm/pmm.rs`: parse UEFI memory map, buddy allocator for 4KB pages. `alloc_pages(n)` / `free_pages(ptr, n)`. Reserve kernel image + MMIO. Branch: `feat/pmm`.
  - Acceptance: PMM alloc/free round-trip covers 100% of detected RAM; double-free panics.

- [ ] T004 **ARM64 MMU** _(dep: T002, T003)_ — `kernel/src/mm/vmm.rs` + `kernel/src/hal/arm64/mmu.rs`: 4-level page tables (PGD/PUD/PMD/PTE), kernel VA at TTBR1_EL1 high address, user VA at TTBR0_EL1. Configure MAIR_EL1 / TCR_EL1 / SCTLR_EL1. TLB invalidation on map/unmap. Branch: `feat/vmm-arm64`.
  - Acceptance: Kernel executes at EL1 with MMU on; user VA fault caught correctly.

- [ ] T005 **[P] Kernel heap allocator** _(dep: T003)_ — `kernel/src/mm/heap.rs`: slab allocator for objects ≤ 2KB, buddy for larger. Implement `GlobalAlloc` so Rust `alloc` crate works. Spinlock-protected. Branch: `feat/heap-alloc`.
  - Acceptance: `kmalloc` + `kfree` pass stress test with no leak or corruption.

- [ ] T006 **[P] GICv3 driver** _(dep: T002)_ — `kernel/src/hal/arm64/gicv3.rs`: GICD + GICR init, `ICC_SRE_EL1` CPU interface, enable/disable/ack/eoi per IRQ ID. Works on QEMU virt MMIO base; MMIO address configurable via HAL platform config. Branch: `feat/gicv3`.
  - Acceptance: GICv3 routes timer IRQ to CPU 0; spurious IRQ count stays zero.

- [ ] T007 **[P] ARM generic timer** _(dep: T006)_ — `kernel/src/hal/arm64/timer.rs`: `CNTP_CTL_EL0` / `CNTP_TVAL_EL0` for 100 Hz periodic tick. Rearm on each IRQ. `monotonic_ns()` via `CNTPCT_EL0` + `CNTFRQ_EL0`. Branch: `feat/timer-arm64`.
  - Acceptance: Preemption tick fires at 100 Hz; verified by UART timestamp log.

- [ ] T008 **[P] HAL trait definitions** _(dep: T001)_ — `kernel/src/hal/mod.rs`: `Console`, `InterruptController`, `Timer`, `AddressSpace`, `Platform` traits. Object-safe. No arch code in this module. See plan.md §Key Technical Decisions §4. Branch: `feat/hal-traits`.
  - Acceptance: HAL traits compile for arm64 with no arch-specific leakage.

- [ ] T009 **ARM64 HAL implementation** _(dep: T002, T003, T004, T005, T006, T007, T008)_ — `kernel/src/hal/arm64/mod.rs`: implement all HAL traits using drivers from T001–T007. Static HAL instance, `Platform::init()` calls all init functions in correct order. Branch: `feat/hal-arm64`.
  - Acceptance: ARM64 HAL self-test exercises all traits (UART, IRQ, timer, MMU).

---

## Wave 2 — WASM Runtime Integration

**Goal**: Supervisor loads and executes a .wasm module with syscall access.

- [ ] T010 **WASM runtime embed** _(dep: T005)_ — `kernel/src/wasm/runtime.rs`: evaluate wasmtime vs wasmer for `no_std` bare-metal (see plan.md §Key Technical Decisions §1). Integrate chosen runtime: disable std features, hook into kernel allocator + panic handler. Test with `hello.wasm`. Branch: `feat/wasm-runtime`.
  - Acceptance: Supervisor loads + runs "hello world" .wasm; output on UART.

- [ ] T011 **[P] WASM module loader** _(dep: T010)_ — `kernel/src/wasm/loader.rs`: parse + validate `.wasm` magic/version/sections from a byte slice. Reject malformed binaries with typed errors. Module registry (name → bytes). Branch: `feat/wasm-loader`.
  - Acceptance: Malformed .wasm rejected with descriptive error; valid binary parses in <10ms on QEMU.

- [ ] T012 **Supervisor ↔ WASM syscall ABI** _(dep: T010, T011)_ — `kernel/src/wasm/syscall.rs`: implement all host functions listed in plan.md §3 under the `"vyoma"` WASM namespace. Unknown import → ENOSYS. Bounds-check all ptr/len pairs against WASM `memory.size`. Branch: `feat/syscall-abi`.
  - Acceptance: WASM module can call all defined host functions; unknown syscall returns ENOSYS.

- [ ] T013 **[P] WASM system library** _(dep: T012)_ — `apps/syslib/`: `vyoma-std` crate targeting `wasm32-unknown-unknown`. Safe Rust wrappers for all syscalls. `print!` / `println!` macros via `sys_write(fd=1)`. Kernel heap via `sys_mmap` + dlmalloc. Branch: `feat/wasm-syslib`.
  - Acceptance: `hello.wasm` compiled against syslib prints to UART via `write()` syscall.

---

## Wave 3 — Process & Scheduling

**Goal**: Multiple WASM processes preemptively scheduled; spawn/kill/wait working.

- [ ] T014 **[P] Task control block** _(dep: T005)_ — `kernel/src/proc/tcb.rs`: `Tcb` struct with tid, kernel stack, saved register frame, state (Running/Ready/Blocked/Zombie), priority, `wasm_instance` ptr, parent tid. Global task table. PID allocator. Branch: `feat/task-structs`.
  - Acceptance: TCB allocated per task; all fields correct under inspection.

- [ ] T015 **ARM64 context switch** _(dep: T009, T014)_ — `kernel/src/proc/switch.S`: `switch_to(prev, next)` saves x19–x28, fp, lr, sp to `prev.frame`; restores from `next.frame`. Switch TTBR0_EL1 for user space changes. TLB invalidation on space switch. Branch: `feat/ctx-switch-arm64`.
  - Acceptance: Context switch round-trips register state bit-for-bit in QEMU test harness.

- [ ] T016 **Preemptive scheduler** _(dep: T007, T015)_ — `kernel/src/proc/scheduler.rs`: per-CPU `VecDeque<Tid>` run queue, round-robin with 10ms time slice. Timer tick hook: decrement slice, call `schedule()` if expired. Idle task (halt loop). `block()` / `unblock()` for sync primitives. Branch: `feat/scheduler`.
  - Acceptance: Two WASM processes both get CPU time; no starvation after 5s in QEMU.

- [ ] T017 **WASM process model** _(dep: T012, T016)_ — `kernel/src/wasm/process.rs`: `sys_spawn(wasm_ptr, wasm_len, args_ptr) → pid`: allocate TCB, instantiate WASM module, add to scheduler. `sys_exit(code)`: cleanup WASM store, free linear memory, set Zombie. `sys_wait(pid) → code`: block caller until child exits. Branch: `feat/wasm-process`.
  - Acceptance: `spawn("hello.wasm")` returns PID; `kill(PID)` cleans up TCB and WASM linear memory.

---

## Wave 4 — IPC

- [ ] T018 **Message-passing IPC** _(dep: T017)_ — `kernel/src/ipc/channel.rs`: `sys_chan_create() → (send_fd, recv_fd)`. `sys_send(fd, ptr, len)`: copy to kernel ring buffer, block if full. `sys_recv(fd, ptr, len)`: copy from ring, block if empty. Named channel registry for well-known services. Max message 64 KB. Branch: `feat/ipc`.
  - Acceptance: Process A sends 1000 messages to B; all received in order, no deadlock.

---

## Wave 5 — Storage, VFS, Userspace & ARM64 Demo

**Goal**: Boots to interactive shell in Docker/QEMU. CI green. ARM64 milestone complete.

- [ ] T019 **[P] Block device abstraction** _(dep: T008)_ — `kernel/src/blk/mod.rs`: `BlockDevice` trait with `read_sectors` / `write_sectors` / `sector_size` / `sector_count`. `DmaBuf` type (physically contiguous, cache-coherent). Device registry `register("blk0", ...)`. Branch: `feat/blk-abstract`.
  - Acceptance: Block trait tested with stub driver; read/write at 512B sector boundaries work.

- [ ] T020 **Virtio-blk driver** _(dep: T009, T019)_ — `kernel/src/blk/virtio_blk.rs`: virtio-MMIO transport, feature negotiation (BLK_F_SEG_MAX), splitqueue (64 descriptors), read/write request via descriptor chain. Register as `"blk0"`. Branch: `feat/virtio-blk`.
  - Acceptance: Virtio-blk reads sector 0 of QEMU raw disk image; matches known content.

- [ ] T021 **FAT32 driver** _(dep: T019)_ — `kernel/src/fs/fat32.rs`: parse BPB, FAT table, directory entries (8.3 + LFN). `read_file` / `write_file` (FAT chain), `create_file` / `delete_file` / `list_dir`. Flush FAT + dir on write. Branch: `feat/fs-fat32`.
  - Acceptance: FAT32 mounts QEMU disk, reads root dir; file created + flushed survives QEMU restart.

- [ ] T022 **VFS layer** _(dep: T021)_ — `kernel/src/fs/vfs.rs`: mount table (mount point → `VfsNode` impl), path resolution (no symlinks for v1.0), per-process fd table, inode cache. `VfsNode` trait: `read` / `write` / `stat` / `readdir`. Branch: `feat/vfs`.
  - Acceptance: VFS mounts FAT32; `open("/foo.txt")` returns fd; read/write/close work.

- [ ] T023 **Filesystem syscalls** _(dep: T012, T022)_ — `kernel/src/fs/syscall.rs`: `sys_open` / `sys_close` / `sys_read` / `sys_write` / `sys_stat` / `sys_readdir`. All ptr/len args bounds-checked against WASM linear memory. Branch: `feat/fs-syscalls`.
  - Acceptance: WASM process opens/reads/writes a file via syscall; verified end-to-end in QEMU.

- [ ] T024 **WASM init system (PID 1)** _(dep: T017, T023)_ — `apps/init/`: reads `/etc/init.conf` (TOML: name, wasm_path, deps, restart policy). Spawns services in dependency order. Monitors children via `sys_wait` loop; applies restart policy. Logs lifecycle to `/var/log/init.log`. IPC channel for status queries. Branch: `feat/init-wasm`.
  - Acceptance: Init boots in <500ms; services in init.conf start in dependency order.

- [ ] T025 **CLI shell** _(dep: T024, T013)_ — `apps/shell/`: WASM module. Read-eval loop: print prompt, `sys_read(stdin)`, tokenize, dispatch. Built-ins: `cd`, `pwd`, `exit`, `help`. External commands: `sys_spawn`, `sys_wait`. Basic line editing (backspace). Branch: `feat/shell-wasm`.
  - Acceptance: Shell shows prompt, `echo hello` works, `ls /` lists real FAT32 contents.

- [ ] T026 **[P] Userspace utilities** _(dep: T025)_ — `apps/utils/`: separate WASM binary per utility: `ls` (readdir+stat), `cat` (open+read+write stdout), `echo`, `ps` (sys_ps extension), `kill` (sys_kill), `env` (sys_getenv). Install to `/bin/` on FAT32 initrd. Branch: `feat/utils-wasm`.
  - Acceptance: All utilities run correctly from shell in QEMU.

- [ ] T027 **QEMU ARM64 boot image** _(dep: T009, T020)_ — `platforms/arm64/Makefile`: `make qemu-arm64` builds supervisor ELF, creates FAT32 disk image with `/bin/` WASM utilities + `/etc/init.conf`, launches QEMU (`-machine virt -cpu cortex-a72 -m 512M -nographic`). No manual steps. Branch: `feat/qemu-arm64`.
  - Acceptance: `make qemu-arm64` boots to shell prompt without manual steps.

- [ ] T028 **Docker demo image** _(dep: T027)_ — `docker/bare-metal/Dockerfile`: `FROM ubuntu:24.04`, install `qemu-system-aarch64`, COPY pre-built images, `ENTRYPOINT` runs QEMU with serial to stdout. Publish to `ghcr.io/hbarve1/vyomaos-demo`. Branch: `feat/docker-demo`.
  - Acceptance: `docker run vyomaos-demo` boots to shell prompt; no host deps needed.

- [ ] T029 **QEMU CI harness** _(dep: T027, T025)_ — `.github/workflows/ci-arm64.yml`: launch QEMU via stdin/stdout pipe, test boot completes (prompt appears), `echo vyoma_test` outputs correctly, `ls /bin` lists expected files, kernel panic string → fail. Timeout: 30s. Branch: `feat/ci-qemu`.
  - Acceptance: CI runs on every push; fails if panic, shell unreachable, or boot >30s.

---

## Wave 6 — Networking

- [ ] T030 **Virtio-net driver** _(dep: T009, T019)_ — `kernel/src/net/virtio_net.rs`: virtio-MMIO transport, `NET_F_MAC` negotiation, RX/TX virtqueues, DMA Ethernet frame buffers. IRQ-driven RX, fallback polling. Branch: `feat/virtio-net`.
  - Acceptance: Virtio-net sends + receives a 1500B Ethernet frame via QEMU tap.

- [ ] T031 **TCP/IP stack** _(dep: T030)_ — `kernel/src/net/tcpip.rs`: embed smoltcp (`no_std`), wire to virtio-net via `smoltcp::phy::Device`. Syscalls: `sys_socket` / `sys_bind` / `sys_connect` / `sys_send` / `sys_recv`. DHCP via smoltcp DHCP socket on init. Branch: `feat/tcpip`.
  - Acceptance: WASM process opens TCP socket, connects to loopback echo server, round-trips 1KB data.

---

## Wave 7 — RPi4 Real Hardware

- [ ] T032 **RPi4 HAL** _(dep: T008)_ — `kernel/src/hal/arm64/rpi4.rs`: BCM2711 peripheral base `0xFE000000`, mini UART (AUX_MU) at 115200, GPIO pin mux, VideoCore mailbox for clock/power, GIC-400 (not GICv3). Implement all HAL traits. Branch: `feat/hal-rpi4`.
  - Acceptance: LED blinks via GPIO mailbox; mini UART prints at 115200 on real RPi4 board.

- [ ] T033 **SDHCI driver** _(dep: T032, T019)_ — `kernel/src/blk/sdhci.rs`: BCM2711 EMMC2 (Arasan SDHCI) driver. Card init sequence (CMD0/CMD8/ACMD41/CMD2/CMD3/CMD7). `CMD17`/`CMD24` single-sector r/w + `CMD18`/`CMD25` multi-sector. ADMA2 DMA. Register as `"sd0"`. Branch: `feat/sdhci`.
  - Acceptance: SDHCI reads RPi4 SD card sector 0 on real hardware; matches `dd` dump.

- [ ] T034 **RPi4 boot image** _(dep: T032, T033)_ — `platforms/arm64/rpi4/`: SD card layout: FAT32 boot partition with `config.txt` (`arm_64bit=1`, `enable_uart=1`, `kernel=supervisor.bin`), U-Boot binary, supervisor binary. `flash-sd.sh` script. USB-serial debug at 115200. Branch: `feat/rpi4-boot`.
  - Acceptance: RPi4 boots to supervisor via U-Boot from SD card; UART shows shell prompt on real board.

---

## Wave 8 — x86-64 Port

- [ ] T035 **x86-64 UEFI boot** _(dep: T008)_ — `kernel/src/hal/x86/boot.rs`: UEFI application entry (`EFI_MAIN`), parse memory map, exit boot services, set up 64-bit GDT, early 16550 UART (port `0x3F8`), jump to `kernel_main()`. Linker script for `x86_64-unknown-none`. Branch: `feat/x86-boot`.
  - Acceptance: x86-64 UEFI enters supervisor on QEMU q35; serial shows boot log.

- [ ] T036 **x86-64 HAL** _(dep: T035)_ — `kernel/src/hal/x86/`: full GDT (null, code, data, TSS), IDT (256 entries, CPU exceptions 0–31 + IRQ vectors), LAPIC init + calibration against HPET, HPET MMIO `monotonic_ns()`. x2APIC if supported. Wire to HAL traits. Branch: `feat/hal-x86`.
  - Acceptance: APIC timer fires; HPET reads monotonic time; serial UART confirmed.

- [ ] T037 **[P] x86-64 paging** _(dep: T035)_ — `kernel/src/hal/x86/mmu.rs`: 4-level PML4, kernel VA at `0xFFFF_8000_0000_0000`, user VA `0–0x0000_7FFF_FFFF_FFFF`. NX bit (`EFER.NXE`). PCID (`CR4.PCIDE`). Device memory (LAPIC, HPET) mapped as UC. Guard pages below kernel stacks. Branch: `feat/vmm-x86`.
  - Acceptance: x86-64 PML4 active; user fault handled without crash.

- [ ] T038 **x86-64 context switch** _(dep: T036, T037)_ — `kernel/src/hal/x86/switch.S`: `switch_to()` saves rbx/rbp/r12–r15/rsp to prev TCB frame, restores from next. Switch CR3, update TSS RSP0. IDT stub generators (push error code where not CPU-provided). Branch: `feat/ctx-switch-x86`.
  - Acceptance: Task A → B → A; all callee-saved registers preserved.

- [ ] T039 **x86-64 QEMU image** _(dep: T036, T037, T038)_ — `platforms/x86/Makefile`: `make qemu-x86` builds UEFI binary, creates OVMF-based boot disk (ESP + FAT32 data partition), launches QEMU (`-machine q35 -cpu qemu64 -m 512M -drive if=pflash,OVMF.fd -nographic`). Branch: `feat/qemu-x86`.
  - Acceptance: `make qemu-x86` boots to shell prompt on QEMU q35 without manual steps.

---

## Wave 9 — RISC-V Port

- [ ] T040 **RISC-V boot** _(dep: T008)_ — `kernel/src/hal/riscv/boot.rs`: OpenSBI `fw_jump` mode (M-mode → S-mode). Sv39 3-level page tables, kernel at `0xFFFF_FFFF_8000_0000`, enable MMU via `satp`. `stvec` trap vector. Early 16550 UART (QEMU virt MMIO). Branch: `feat/riscv-boot`.
  - Acceptance: RISC-V enters supervisor via OpenSBI; Sv39 MMU on; UART prints boot log.

- [ ] T041 **RISC-V HAL** _(dep: T040)_ — `kernel/src/hal/riscv/`: PLIC (enable sources, set priorities, claim/complete), CLINT timer via SBI ecall in S-mode, 16550 UART console. Wire HAL traits. Branch: `feat/hal-riscv`.
  - Acceptance: PLIC routes UART IRQ; timer fires at 100 Hz on QEMU riscv64 virt.

- [ ] T042 **RISC-V context switch** _(dep: T041)_ — `kernel/src/hal/riscv/switch.S`: `stvec` direct/vectored mode. Trap handler: save all regs to TrapFrame, dispatch to Rust handler, `sret`. `switch_to()`: save s0–s11, ra, sp to TCB frame; restore from next. Branch: `feat/ctx-switch-riscv`.
  - Acceptance: Task A → B → A; exception delegation from M-mode confirmed.

- [ ] T043 **RISC-V QEMU image** _(dep: T041, T042)_ — `platforms/riscv/Makefile`: `make qemu-riscv` builds `riscv64gc-unknown-none-elf` binary, bundles with `opensbi-fw_jump.bin`, creates virtio-blk disk image, launches QEMU (`-machine virt -cpu rv64 -m 512M -bios opensbi-fw_jump.bin -kernel supervisor.bin -nographic`). Branch: `feat/qemu-riscv`.
  - Acceptance: `make qemu-riscv` boots to shell prompt on QEMU riscv64 without manual steps.

---

## Wave 10 — Cross-Platform CI & v1.0 Release

- [ ] T044 **Cross-platform image builder** _(dep: T028, T039, T043)_ — root `Makefile`: `make all` builds arm64 + x86-64 + riscv64 images in parallel. Output: `dist/vyomaos-arm64.img`, `dist/vyomaos-x86.img`, `dist/vyomaos-riscv64.img` + Docker image. Reproducible builds (fixed timestamps). Git tag embedded as version string. Branch: `feat/image-builder`.
  - Acceptance: `make all` builds all 3 arch images; artifacts in `dist/`; version in each boot log.

- [ ] T045 **End-to-end CI** _(dep: T029, T034, T044)_ — `.github/workflows/ci-e2e.yml`: matrix `{arm64, x86, riscv}` runs QEMU boot tests (timeout 30s, shell reachable, `echo`/`ls` pass). RPi4 gate job: self-hosted runner with RPi4 + USB-serial; flashes image, boots, runs same suite. All 4 jobs must pass for release tag. Upload `dist/` artifacts on success. Branch: `feat/ci-e2e`.
  - Acceptance: All 3 QEMU arches pass CI; RPi4 gate blocks release if failing.

---

## Dependency Summary

| Task | Blocked by |
|------|-----------|
| T002 | T001 |
| T003 | T001 |
| T004 | T002, T003 |
| T005 | T003 |
| T006 | T002 |
| T007 | T006 |
| T008 | T001 |
| T009 | T002, T003, T004, T005, T006, T007, T008 |
| T010 | T005 |
| T011 | T010 |
| T012 | T010, T011 |
| T013 | T012 |
| T014 | T005 |
| T015 | T009, T014 |
| T016 | T007, T015 |
| T017 | T012, T016 |
| T018 | T017 |
| T019 | T008 |
| T020 | T009, T019 |
| T021 | T019 |
| T022 | T021 |
| T023 | T012, T022 |
| T024 | T017, T023 |
| T025 | T024, T013 |
| T026 | T025 |
| T027 | T009, T020 |
| T028 | T027 |
| T029 | T027, T025 |
| T030 | T009, T019 |
| T031 | T030 |
| T032 | T008 |
| T033 | T032, T019 |
| T034 | T032, T033 |
| T035 | T008 |
| T036 | T035 |
| T037 | T035 |
| T038 | T036, T037 |
| T039 | T036, T037, T038 |
| T040 | T008 |
| T041 | T040 |
| T042 | T041 |
| T043 | T041, T042 |
| T044 | T028, T039, T043 |
| T045 | T029, T034, T044 |
