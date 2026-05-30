> **Status: Archive** — This is an early design exploration document. The implemented code is the source of truth. See [docs/INDEX.md](../../docs/INDEX.md) for current documentation.

# Feature Specification: VyomaOS Production Refinement

**Feature Branch**: `001-vyomaos-refinement`

**Created**: 2026-05-24

**Status**: Draft

**Input**: User description: "Refine VyomaOS into production-grade software: introduce a proper test harness (cargo test for supervisor + integration smoke tests), enforce the capability-secure app model with validation, improve supervisor error handling and structured logging, document the VYOMA_DRAW protocol formally, add a CI-ready Makefile test target, and establish developer experience improvements (faster incremental builds, clearer error messages). Apply TDD throughout."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Supervisor Unit Test Coverage (Priority: P1)

A developer working on the supervisor wants to verify that individual logic units — manifest parsing, capability enforcement, IPC routing, and process lifecycle — behave correctly without booting a full VM. They run `cargo test` inside the supervisor crate and see a structured pass/fail report with clear error messages when regressions occur.

**Why this priority**: The supervisor is PID 1; a crash or logic error bricks the entire OS. Unit tests are the fastest regression net and unblock all other quality improvements.

**Independent Test**: Run `cargo test -p supervisor` and observe at least one passing test for each of the four subsystems (manifest, capabilities, IPC, lifecycle). Delivers standalone value as the core safety net.

**Acceptance Scenarios**:

1. **Given** a `vyoma.toml` with an unknown capability field, **When** the manifest parser processes it, **Then** it returns a structured error identifying the unknown field and the app name.
2. **Given** a correctly formed `vyoma.toml`, **When** the capability validator runs, **Then** it grants only declared capabilities and produces a zero-warning result.
3. **Given** an IPC message addressed to a non-existent app, **When** the broker routes it, **Then** it returns a `RouteError` with the unknown target name — no panic.
4. **Given** an app process that exits with a non-zero code and `restart = "never"`, **When** the lifecycle manager observes the exit, **Then** it logs a structured error line and does not restart the app.

---

### User Story 2 - Integration Smoke Test (Priority: P1)

A developer merges a change and wants confidence the full system still boots correctly. They run `make test` (or `make smoke`) and get a pass/fail result based on whether the VM boots to the supervisor prompt, all declared apps start without errors, and no manifest parse failures appear in the serial log — without needing to manually inspect QEMU output.

**Why this priority**: Equal priority to unit tests because integration tests catch system-level regressions that unit tests cannot (boot sequence, rootfs wiring, WASM loading).

**Independent Test**: Run `make test` on a clean build and confirm it exits 0 when the OS boots cleanly and exits non-zero when a deliberately broken manifest is introduced.

**Acceptance Scenarios**:

1. **Given** a clean `make build`, **When** `make test` runs, **Then** it boots VyomaOS in headless QEMU, waits for the supervisor ready line in serial output, and exits 0 within 30 seconds.
2. **Given** a `vyoma.toml` with a missing required field injected before the build, **When** `make test` runs, **Then** it exits non-zero and prints which app failed validation.
3. **Given** an app whose WASM binary is corrupt/missing from initramfs, **When** `make test` runs, **Then** it exits non-zero with the app name and failure reason.

---

### User Story 3 - Capability Validation at Build Time (Priority: P2)

A developer introduces a new app and mistakenly declares `network = true` but forgets `stdio = true`. Before the app even runs, the build pipeline validates all `vyoma.toml` files and reports the inconsistency with an actionable error message, so the developer fixes it before committing.

**Why this priority**: Catches misconfigurations early — before they cause silent runtime failures in the VM where debugging is harder.

**Independent Test**: Run the capability validator as a standalone `make check-manifests` target that reports errors on malformed manifests without requiring a full build.

**Acceptance Scenarios**:

1. **Given** a `vyoma.toml` declaring an unknown capability key, **When** `make check-manifests` runs, **Then** it prints the app name, the unknown key, and a suggestion of valid keys — exits non-zero.
2. **Given** all `vyoma.toml` files are valid, **When** `make check-manifests` runs, **Then** it prints a per-app summary and exits 0.
3. **Given** a `vyoma.toml` with `watchdog_secs` set to a negative value, **When** `make check-manifests` runs, **Then** it reports the invalid value with the allowed range.

---

### User Story 4 - Structured Supervisor Logging (Priority: P2)

An on-call developer boots VyomaOS in QEMU and needs to diagnose why an app failed to start. The serial console shows timestamped, structured log lines (level, subsystem, app name, event) so they can immediately identify the failure point without grepping unstructured text.

**Why this priority**: Inside QEMU there is no debugger; structured logs are the only diagnostic tool. This directly unblocks incident response.

**Independent Test**: Boot VyomaOS and observe structured log lines on stderr for at least: supervisor startup, each app spawn, each app exit, and each IPC route decision.

**Acceptance Scenarios**:

1. **Given** VyomaOS boots normally, **When** the serial log is captured, **Then** each log line contains: ISO timestamp, log level (INFO/WARN/ERROR), subsystem name, and a human-readable event description.
2. **Given** an app exits unexpectedly, **When** the supervisor logs it, **Then** the log line includes the app name, exit code, and whether a restart was attempted.
3. **Given** an IPC message is delivered, **When** the supervisor logs it, **Then** the log line shows sender, receiver, and message length — no message payload (to avoid log injection).
4. **Given** a capability grant or deny decision is made, **When** it is logged, **Then** the log line shows which capability was acted on and the outcome.

---

### User Story 5 - VYOMA_DRAW Protocol Documentation (Priority: P3)

A developer writing a new display app needs to understand every supported command, color format, coordinate system, and flush semantics without reading the supervisor source. They open `docs/vyoma-draw-protocol.md` and find a complete, example-rich reference with error cases documented.

**Why this priority**: Documentation unblocks app development without requiring source archaeology; lower priority than correctness improvements but essential for onboarding.

**Independent Test**: A developer writes a new display app using only `docs/vyoma-draw-protocol.md` as reference, with no supervisor source, and it renders correctly on first attempt.

**Acceptance Scenarios**:

1. **Given** the protocol doc exists, **When** a developer reads `fill_rect` semantics, **Then** they can determine coordinate origin, RGBA byte order, zero-size rect behavior, and out-of-bounds handling from the doc alone.
2. **Given** the protocol doc exists, **When** a developer reads `draw_text` semantics, **Then** they can determine font dimensions, multi-line handling, and character encoding requirements.
3. **Given** the protocol doc exists, **When** a developer reads `flush` semantics, **Then** they understand double-buffering behavior, what happens if flush is not called, and the recommended frame timing pattern.

---

### User Story 6 - Faster Incremental Builds (Priority: P3)

A developer changing only one app's source code runs `make apps` and sees only that app recompiled, not all apps. Build time for a single-app change drops from the full batch compile time to under 10 seconds.

**Why this priority**: Developer experience directly impacts iteration speed; faster feedback loops encourage more frequent testing.

**Independent Test**: Touch one app's `src/main.rs`, run `make apps`, confirm via build output that only that app's cargo invocation ran.

**Acceptance Scenarios**:

1. **Given** only `apps/hello-world/src/main.rs` is modified, **When** `make apps` runs, **Then** cargo recompiles only the `hello-world` crate; other apps show cached artifacts.
2. **Given** no source files have changed, **When** `make apps` runs, **Then** all cargo invocations report "Finished" with no recompilation.
3. **Given** `supervisor/src/main.rs` is modified, **When** `make supervisor` runs, **Then** only the supervisor crate recompiles; app binaries are untouched.

---

### Edge Cases

- What happens when a `vyoma.toml` is syntactically valid TOML but semantically invalid (e.g., `display = "yes"` instead of `display = true`)?
- How does the smoke test behave if QEMU is not installed on the host?
- When two apps declare the same `name` field, the first-registered app starts normally; the duplicate is rejected with a structured ERROR log line identifying the conflicting app and path — all other apps continue to start.
- What if a WASM binary exceeds the 50 KB size limit — is it a build error or a warning?
- What happens when `cargo test` is run inside the Docker container vs. on the host directly?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The supervisor crate MUST have a `tests/` module with unit tests covering manifest parsing (valid + invalid), capability validation, IPC routing, and process lifecycle state transitions.
- **FR-002**: `cargo test -p supervisor` MUST run inside the `vyomaos-builder` Docker container, pass with zero failures, and produce zero new compiler warnings on the `x86_64-unknown-linux-musl` target.
- **FR-003**: A `make test` target MUST exist that boots VyomaOS headless in QEMU inside the `vyomaos-builder` Docker container, captures serial output, asserts the supervisor ready signal appears, and exits 0/non-zero appropriately. QEMU MUST be installed in the builder image; no host QEMU dependency. The canonical ready signal is the exact log substring `[lifecycle] all apps spawned` — both the supervisor log call (FR-005) and the smoke test grep MUST use this string.
- **FR-004**: A `make check-manifests` target MUST exist that validates all `apps/*/vyoma.toml` files against the manifest schema and reports errors with app name + field + description.
- **FR-005**: The supervisor MUST emit structured log lines (ISO timestamp + level + subsystem + event) to stderr for: startup, app spawn, app exit, IPC route decisions, and capability wire/skip decisions. Capability wire/skip MUST produce one log line per app during WASM loading — listing which capabilities were wired (declared and active) and which were skipped (not declared). Note: the capability model is static; there are no runtime grant/deny decisions, only load-time wiring.
- **FR-006**: The supervisor MUST validate every `vyoma.toml` at startup and emit a structured ERROR line (not panic) for any schema violation or duplicate app name, then skip the offending app — all other apps MUST continue to start. For duplicate names, the first-registered app wins; the second is rejected.
- **FR-007**: The file `docs/vyoma-draw-protocol.md` MUST document all `VYOMA_DRAW:` commands, RGBA color format, coordinate semantics, flush behavior, and error handling with at least one working code example per command.
- **FR-008**: The Makefile MUST use per-app dependency tracking so that `make apps` recompiles only apps with changed source files.
- **FR-009**: New `cargo check` warnings in any modified crate MUST cause `make build` to exit non-zero (hard failure). Warnings are never silently swallowed and never non-blocking — zero new warnings is a mandatory quality gate enforced at build time.
- **FR-010**: All new non-trivial logic MUST follow Red → Green → Refactor: each unit test MUST be written before its implementation, with the failing state captured in a commit.

### Key Entities

- **Manifest** (`vyoma.toml`): Per-app configuration declaring name, version, wasm path, capabilities, and restart policy. Validated at build time and at supervisor startup.
- **Capability Set**: The finite set of permissions an app may declare (`stdio`, `filesystem`, `network`, `network_port`, `display`, `shell`, `mouse`, `watchdog_secs`). Any value outside this set is a validation error.
- **Structured Log Line**: A supervisor stderr emission with fields: timestamp (ISO 8601), level (INFO/WARN/ERROR), subsystem (manifest/ipc/lifecycle/display/capability), app name (if applicable), and event message.
- **Smoke Test Result**: A pass/fail signal produced by `make test` derived from QEMU serial output pattern matching, with a timeout of 30 seconds.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `cargo test -p supervisor` completes in under 5 seconds and reports 100% pass rate with zero warnings on a clean checkout.
- **SC-002**: `make test` reliably distinguishes a healthy boot from a broken one: 0 false negatives on a clean build, 0 false positives when a manifest is intentionally broken.
- **SC-003**: A developer introducing a manifest error sees an actionable error message (app name + field + suggested fix) within the first 10 lines of `make check-manifests` output.
- **SC-004**: Serial log output from a standard boot contains structured lines for every app lifecycle event — 100% coverage of spawn and exit events.
- **SC-005**: A developer writes a new display app using only `docs/vyoma-draw-protocol.md` (no supervisor source) and achieves correct rendering on the first attempt.
- **SC-006**: Single-app incremental build time drops to under 10 seconds when only one app's source changes (vs. full batch compile).
- **SC-007**: All previously passing apps continue to compile and boot without modification after refinements are merged.

## Assumptions

- The `x86_64-unknown-linux-musl` target for the supervisor and `wasm32-wasip2` for apps remain unchanged; test infrastructure targets these same platforms.
- `make test` MUST run entirely inside the `vyomaos-builder` Docker container; QEMU is bundled in the builder image. Host QEMU availability is not required.
- Incremental build improvement uses standard Makefile dependency rules (file timestamps) without introducing a new build tool (Bazel, Buck, etc.).
- The VYOMA_DRAW protocol specification matches current supervisor source behavior exactly; any discrepancy found during documentation is a supervisor bug to be fixed, not documented as "intended."
- `cargo test` for the supervisor MUST run inside the Docker builder container (`vyomaos-builder`) against the `x86_64-unknown-linux-musl` target, matching the release build environment exactly. WASM app tests are out of scope for this feature.
- Binary size limits (supervisor ≤ 1 MB, apps ≤ 50 KB) are enforced as CI warnings, not hard errors, in this iteration.

## Clarifications

### Session 2026-05-24

- Q: Where should `cargo test -p supervisor` run — host machine or inside Docker builder container? → A: Inside Docker builder container (`vyomaos-builder`), using the `x86_64-unknown-linux-musl` target, matching the release build environment exactly.
- Q: When `make test` runs and QEMU is not available in the current environment, what should happen? → A: Always run inside Docker; QEMU is bundled in the `vyomaos-builder` image — no host QEMU dependency.
- Q: When two apps declare the same `name` in `vyoma.toml`, what should the supervisor do? → A: First-registered wins; the duplicate is rejected with a structured ERROR — all other apps continue to start.
- Q: Should new `cargo check` warnings in a modified crate block the build or be non-blocking? → A: Hard failure — new warnings cause `make build` to exit non-zero, enforcing the quality gate.

### Analysis Remediations 2026-05-24

- H1: Canonical smoke test ready-signal string pinned to `[lifecycle] all apps spawned` in FR-003 and FR-005.
- H2: FR-005 "capability grant/deny" replaced with "capability wire/skip decisions" to match the supervisor's static capability model; one log line per app at WASM load time.
- M1: `network_port` added to Key Entities Capability Set list to align with data-model.md.
