# Tasks: Desktop / Server Target (US3 — P3)

**Parent spec**: `specs/043-universal-modular-os/`
**Branch**: `feat/043-desktop-server`
**Prerequisite**: `feat/043-core` merged to `develop` first

**Goal**: Same productivity app binary runs on desktop (VYOMA_DRAW display) and server (HTTP headless) using profile-selected capabilities. No code changes to the app.

**Independent Test**: Build one WASM app, deploy to `desktop-full` and `server-headless` profiles, verify correct mode activates on each.

---

## Phase 5: User Story 3 — Enterprise Cross-Platform Desktop + Server

### Tests

- [ ] T041 [P] Write failing test for profile-dependent behavior in `supervisor/tests/profile_behavior.rs` — same binary uses VYOMA_DRAW on desktop profile, HTTP on server profile

### Implementation

- [ ] T042 Create server platform build config in `platforms/server-arm64/Makefile` and `platforms/server-arm64/kernel.config` — headless, network-enabled ARM64 kernel
- [ ] T043 Create server rootfs script in `platforms/server-arm64/rootfs.sh` — supervisor with `server-headless.toml` profile, no display modules
- [ ] T044 Create sample dual-mode app in `apps/doc-viewer/` — renders via VYOMA_DRAW when display capability is available, serves via HTTP when network capability is available
- [ ] T045 Verify dual-mode behavior: deploy `doc-viewer.wasm` to `desktop-full` and `server-headless` profiles, confirm correct mode activation. Make T041 pass
- [ ] T046 Verify capability enforcement: on server profile with `network = false` manifest, confirm app cannot open sockets

**Checkpoint**: Same binary adapts to desktop and server environments. Capability enforcement works across profiles.

---

## Acceptance Criteria

1. `doc-viewer.wasm` on `desktop-full` profile → renders using `VYOMA_DRAW:draw_text`
2. `doc-viewer.wasm` on `server-headless` profile → serves content over HTTP
3. App with `network = false` on server profile → supervisor blocks socket creation
4. `make build PLATFORM=server-arm64` succeeds and produces bootable image

## Key files to touch

- `platforms/server-arm64/Makefile` (new)
- `platforms/server-arm64/kernel.config` (new)
- `platforms/server-arm64/rootfs.sh` (new)
- `apps/doc-viewer/src/main.rs` (new app)
- `apps/doc-viewer/vyoma.toml` (new manifest)
- `apps/doc-viewer/Cargo.toml` (new)
- `supervisor/tests/profile_behavior.rs` (new)
