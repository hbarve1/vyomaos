# Tasks: Mobile / Tablet Target (US3.5 — between P2 and P3)

**Parent spec**: `specs/043-universal-modular-os/`
**Branch**: `feat/043-mobile`
**Prerequisite**: `feat/043-core` merged to `develop` first

**Goal**: VyomaOS runs on ARM64 mobile/tablet hardware with touch input, a mobile platform profile,
and WASM apps that adapt to screen orientation and touch events. Same `.wasm` binary as desktop.

**Rollout position**: Follows Robotics (P2) and precedes Desktop/Server (P3) per bottom-up strategy.

**Independent Test**: Build a touch-aware WASM app, deploy to `mobile` profile under QEMU ARM64,
verify touch events are delivered to the app and the screen renders at correct mobile resolution.

---

## Phase: Mobile Platform & Touch Input

### Tests

- [ ] TM01 [P] Write failing test for touch event parsing in `supervisor/tests/touch_input.rs` —
  parse `VYOMA_INPUT:touch:tap:<x>,<y>` and `VYOMA_INPUT:touch:swipe:<dx>,<dy>` lines,
  verify they produce correct `TouchEvent` structs
- [ ] TM02 [P] Write failing test for mobile profile loading in `supervisor/tests/mobile_profile.rs` —
  load `mobile.toml` profile, verify `display = true`, `touch = true`, `network = true`,
  `mouse = false` capability set; verify screen resolution is 1080×2340 (portrait)

### Implementation

- [ ] TM03 Add `touch` capability to `supervisor/src/capability/mod.rs` — new `touch = true/false`
  manifest field; supervisor routes `VYOMA_INPUT:touch:` events only to apps that declare it
- [ ] TM04 Implement touch event parser in `supervisor/src/touch_input.rs` — parse tap, swipe,
  pinch event lines from stdin pipe; dispatch to focused app. Make TM01 pass
- [ ] TM05 Update mobile platform profile `supervisor/src/profile/profiles/mobile.toml` —
  set runtime = "wasmtime", display = true, touch = true, network = true, mouse = false,
  screen_w = 1080, screen_h = 2340, orientation = "portrait"
- [ ] TM06 Create mobile platform build config in `platforms/mobile-arm64/Makefile` and
  `platforms/mobile-arm64/kernel.config` — ARM64 kernel with DRM + virtio-gpu + virtio-touchscreen
- [ ] TM07 Create mobile rootfs script in `platforms/mobile-arm64/rootfs.sh` — supervisor with
  `mobile.toml` profile; include touch input driver module
- [ ] TM08 Create sample mobile app in `apps/touch-demo/` — displays a canvas; renders colored
  circles where tap events land; swipe scrolls the canvas. Declares `display = true, touch = true`
- [ ] TM09 Wire touch events into supervisor event loop in `supervisor/src/main.rs` — read
  `VYOMA_INPUT:touch:` lines from virtio-touchscreen device, route to focused app stdin
- [ ] TM10 Verify mobile profile under QEMU ARM64: launch `touch-demo.wasm`, simulate touch
  events via QEMU monitor, verify circles appear at correct coordinates. Make TM02 pass

**Checkpoint**: Touch events parsed and dispatched. Mobile platform profile loads correctly.
`touch-demo.wasm` renders tap circles on the framebuffer. `make build PLATFORM=mobile-arm64` passes.

---

## Acceptance Criteria

1. `VYOMA_INPUT:touch:tap:540,1000` delivered to focused app's stdin when `touch = true` in manifest
2. App with `touch = false` manifest does NOT receive touch events (capability isolation)
3. `mobile.toml` profile excludes `mouse` capability and includes `touch` capability
4. `touch-demo.wasm` binary is identical to what would run on desktop — no recompilation; only
   the capability wiring differs per profile
5. `make build PLATFORM=mobile-arm64` produces a bootable ARM64 image

## Key files to touch

- `supervisor/src/touch_input.rs` (new)
- `supervisor/src/capability/mod.rs` (add `touch` field)
- `supervisor/src/profile/profiles/mobile.toml` (update — stub created in T008 by core branch)
- `supervisor/src/main.rs` (wire touch events into event loop)
- `platforms/mobile-arm64/Makefile` (new)
- `platforms/mobile-arm64/kernel.config` (new)
- `platforms/mobile-arm64/rootfs.sh` (new)
- `apps/touch-demo/src/main.rs` (new)
- `apps/touch-demo/vyoma.toml` (new)
- `apps/touch-demo/Cargo.toml` (new)
- `supervisor/tests/touch_input.rs` (new)
- `supervisor/tests/mobile_profile.rs` (new)

## Notes

- Touch events use the same VYOMA_INPUT pipe as mouse events; supervisor demultiplexes by
  prefix (`VYOMA_INPUT:mouse:` vs `VYOMA_INPUT:touch:`)
- Portrait vs landscape orientation is a profile field; apps read screen dimensions via
  a `@supervisor: screen-info` IPC query
- Multi-touch (pinch-to-zoom) is out of scope for this phase; only tap and swipe
- Physical hardware target is ARM64 SBC (e.g., Raspberry Pi 4 with touchscreen) run via
  QEMU ARM64 for CI
