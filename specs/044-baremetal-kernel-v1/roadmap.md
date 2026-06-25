# Roadmap: VyomaOS v1.0 — Bare-Metal Kernel

**Deadline**: December 2026 | **ARM64 Priority Milestone**: September 2026

## Timeline

```
Jun 2026   Jul 2026       Aug 2026          Sep 2026        Oct 2026     Nov 2026    Dec 2026
    │           │              │                 │               │            │           │
    │  [PLAN]   │── Wave 1 ───►│── Wave 2,3,4 ──│── Wave 5 ─────┤            │           │
    │           │ ARM64 HW     │ WASM + Sched    │ Storage+Shell  │           │           │
    │           │              │                 │◄── ARM64 DEMO ─┤           │           │
    │           │              │                 │                │── Wave 6,7│           │
    │           │              │                 │                │ Net+RPi4  │── Wave 8 ─┤
    │           │              │                 │                │           │  x86-64   │
    │           │              │                 │                │           │           │── Wave 9,10
    │           │              │                 │                │           │           │ RISC-V + Release
    │           │              │                 │                │           │           │◄── v1.0
```

## Milestones

### M1 — ARM64 QEMU Demo (Sep 2026)
Tasks 1–29. ARM64 bare-metal kernel boots to interactive CLI shell in Docker + QEMU.

**Done when:**
- `docker run ghcr.io/hbarve1/vyomaos-demo` boots to shell prompt
- `make qemu-arm64` works from a clean clone
- QEMU CI green on every push
- Shell, ls, cat, echo, ps, kill all working

### M2 — RPi4 Real Hardware (Oct 2026)
Tasks 30–34. Boots on physical Raspberry Pi 4 from SD card.

**Done when:**
- SD card image boots on RPi4 to shell prompt over USB-serial
- SDHCI reads/writes files on SD card
- TCP/IP working on QEMU (virtio-net + smoltcp)

### M3 — x86-64 Port (Nov 2026)
Tasks 35–39. Same supervisor binary boots on QEMU Q35.

**Done when:**
- `make qemu-x86` boots to shell prompt
- All Wave 5 userspace works on x86-64

### M4 — RISC-V Port + v1.0 Release (Dec 2026)
Tasks 40–45. All platforms green. Docker demo published. v1.0 tag.

**Done when:**
- `make qemu-riscv` boots to shell prompt
- `make all` builds all 3 arch images
- GitHub Actions matrix (arm64 + x86 + riscv) all green
- RPi4 hardware gate in CI
- `dist/` artifacts uploaded on release tag

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| wasmtime/wasmer no_std support incomplete | Medium | High | Evaluate both early (T010); fallback: custom minimal WASM interpreter |
| RPi4 SDHCI driver complexity | High | Medium | Start with read-only mode; write support in M3 |
| WASM apps need changes for bare-metal ABI | Low | Medium | Compatibility shim in syslib mapping WASI → VyomaOS syscalls |
| x86-64 UEFI toolchain complexity | Medium | Low | Use `uefi-rs` crate if raw PE entry is too complex |
| Context switch correctness on ARM64 | Medium | High | Extensive QEMU testing; compare against reference implementations |
| smoltcp no_std integration | Low | Medium | smoltcp is designed for embedded; well-documented path |

## What Stays Unchanged

- All 77+ existing WASM apps continue to run without modification
- `wasm32-wasip2` target is preserved
- `vyoma.toml` capability manifest format unchanged
- IPC `@target: message` protocol unchanged (backed by T018 channels)
- Package manager, OTA update model unchanged at the app layer
