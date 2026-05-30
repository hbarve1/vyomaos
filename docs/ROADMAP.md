# VyomaOS Roadmap & Development Approach

## Current Status (2026-05-30)

- **Desktop OS spec**: 80/80 subsystems FINAL (merged via PR #97)
- **Implementation**: Phase 17 complete — working supervisor, GUI, IPC, shell, HTTP server
- **Active branch**: `046-apple-ui-fidelity` — scalable fonts, PNG icons, alpha compositing, window chrome

---

## Chosen Approach: Implement First, Spec Other Targets Later

### Why not spec all 6 targets first?

The 5 remaining targets (Mobile, IoT/Edge, Robotics RT, Server Headless, MCU Minimal) share ~70% of the Desktop architecture. Speccing them now risks:

1. **Spec-before-reality**: Implementation will surface architectural gaps that are invisible on paper. Those gaps will invalidate multi-target specs written today.
2. **Premature divergence**: Mobile/IoT peripheral models (GPIO, I2C, touch) should be specced against a *working* Desktop baseline, not a hypothetical one.
3. **Split focus**: Neither track makes meaningful progress when split too early.

### The actual plan

| Phase | Work | When |
|-------|------|------|
| **Now** | Implement Desktop OS — work through the 80 subsystem specs in priority order | Active |
| **~60% Desktop** | Begin Mobile spec in parallel (second session) | Once compositor + window manager are implemented |
| **Desktop MVP** | Spec IoT/Edge + Robotics RT (share embedded HAL, differ in peripherals) | After Desktop MVP ships |
| **Embedded** | MCU Minimal + Server Headless specs + implementation | Final phase |

### Two-terminal parallel sessions
Valid strategy — but only once Desktop implementation is past the compositor/window manager stage. At that point:
- **Session A**: Desktop implementation
- **Session B**: Mobile spec (diffing against working Desktop, not against spec alone)

---

## Platform Notes

### macOS (current dev machine)
- `make run-gui DISPLAY_BACKEND=cocoa`
- Rosetta overhead if running on Apple Silicon (though project targets x86-64 guest)

### Intel Ubuntu (recommended for performance)
- `make run-gui DISPLAY_BACKEND=sdl`
- **KVM acceleration**: add `-enable-kvm` to QEMU args → 5–10× faster boot, near-native perf
- Docker builds identical — same hermetic environment
- SDL display backend is native on Linux, no translation layer
- Better for long QEMU sessions (no macOS memory pressure management)

To enable KVM on the Ubuntu dev machine, add to `Makefile` `run-gui` target:
```makefile
QEMU_ACCEL ?= $(shell [ -e /dev/kvm ] && echo "-enable-kvm -cpu host" || echo "")
# then add $(QEMU_ACCEL) to the qemu-system-x86_64 invocation
```

---

## Next Implementation Priorities (Desktop)

Ordered by dependency depth — foundational subsystems unlock everything above them:

1. **046-apple-ui-fidelity** ← active: scalable fonts, PNG icons, compositor, window chrome
2. **Windowed compositor** — per-window surfaces, Z-order, damage regions (R11, R21)
3. **Menu bar + Dock** — system chrome (R23, R24)
4. **App lifecycle** — full FSM, watchdog, focus management (R22, R28)
5. **Input routing** — keyboard focus, mouse events per window (R31, R32)
6. **File manager** — open/save panels (R41)
7. **Notifications** — banner + history panel (R73)
