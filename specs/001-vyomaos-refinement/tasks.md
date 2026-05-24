# Tasks: VyomaOS Production Refinement

**Input**: Design documents from `specs/001-vyomaos-refinement/`

**Prerequisites**: plan.md ✅ | spec.md ✅ | research.md ✅ | data-model.md ✅ | contracts/ ✅

**Tests**: TDD throughout — all non-trivial logic requires failing test committed before implementation (Spec §FR-010, Constitution §II).

**Organization**: Tasks grouped by user story for independent implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: User story this task belongs to (US1–US6)

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create new directories and scaffold files needed before any user story begins.

- [ ] T001 Create `supervisor/src/manifest.rs` as an empty pub module stub and add `pub mod manifest;` to `supervisor/src/main.rs`
- [ ] T002 [P] Create `supervisor/src/logging.rs` as an empty pub module stub and add `pub mod logging;` to `supervisor/src/main.rs`
- [ ] T003 [P] Create `base/scripts/` directory and add placeholder `base/scripts/smoke-test.sh` (exits 1 with "NOT IMPLEMENTED")
- [ ] T004 [P] Create `tools/check-manifests/` directory with `Cargo.toml` and `src/main.rs` stub that prints "TODO" and exits 0

**Checkpoint**: `cargo check --manifest-path supervisor/Cargo.toml` passes with zero warnings after adding the two empty modules.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Move manifest data structures out of `supervisor/src/main.rs` into `supervisor/src/manifest.rs` so they are accessible to integration tests and the check-manifests tool. This MUST complete before US1, US2, and US3.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T005 Move `BootConfig`, `BootEntry`, `AppManifest`, `AppMeta`, `Capabilities`, `WindowRegion`, and `default_restart()` from `supervisor/src/main.rs` into `supervisor/src/manifest.rs`; add `use crate::manifest::*;` in `main.rs` to restore compilation
- [ ] T006 Add `pub fn parse_manifest(path: &std::path::Path) -> Result<AppManifest, String>` to `supervisor/src/manifest.rs` — body: read file, `toml::from_str`, return `Ok(manifest)` or `Err(e.to_string())`
- [ ] T007 Add `pub fn validate_manifest(m: &AppManifest, registered_names: &[&str]) -> Result<(), String>` to `supervisor/src/manifest.rs` — body: check for duplicate name in `registered_names`, return `Err` with message if found, else `Ok(())`
- [ ] T008 Update all call sites in `supervisor/src/main.rs` that currently inline TOML parsing (the `spawn_app` function, the boot config reader) to use `parse_manifest()` and `validate_manifest()` instead
- [ ] T009 Run `cargo check --manifest-path supervisor/Cargo.toml --target x86_64-unknown-linux-musl` inside Docker and confirm zero new warnings before proceeding

**Checkpoint**: Supervisor still compiles and all existing `supervisor/tests/*.rs` tests pass after extraction (`cargo test --manifest-path supervisor/Cargo.toml`).

---

## Phase 3: User Story 1 — Supervisor Unit Test Coverage (Priority: P1) 🎯 MVP

**Goal**: `cargo test -p supervisor` passes with coverage for manifest parsing, capability validation, IPC routing, and process lifecycle transitions.

**Independent Test**: Run `docker run --rm -v $(pwd):/work -w /work vyomaos-builder cargo test --manifest-path supervisor/Cargo.toml --target x86_64-unknown-linux-musl` — all tests pass, zero warnings.

> **TDD**: Write each test file so tests FAIL first — commit the failing state — then implement to make them pass.

### Tests (write failing first, commit, then implement)

- [ ] T010 [US1] Write `supervisor/tests/manifest_tests.rs` with 5 failing tests: (1) valid manifest parses OK, (2) unknown capability field returns Err, (3) missing `app.name` returns Err, (4) duplicate name returns Err from `validate_manifest`, (5) `watchdog_secs = 0` is valid. Commit with message `test(manifest): add failing manifest unit tests`
- [ ] T011 [US1] Write `supervisor/tests/ipc_tests.rs` with 3 failing tests: (1) message to unknown app returns descriptive error string (not panic), (2) empty target name returns error, (3) message format `@app: msg` is parsed correctly into (target, payload) pair. Commit with message `test(ipc): add failing IPC routing unit tests`
- [ ] T012 [US1] Write `supervisor/tests/lifecycle_tests.rs` with 2 failing tests: (1) `restart_policy("never")` returns false for "should restart", (2) `restart_policy("always")` returns true. Commit with message `test(lifecycle): add failing lifecycle unit tests`

### Implementation

- [ ] T013 [US1] Make `manifest_tests.rs` pass: `parse_manifest()` and `validate_manifest()` in `supervisor/src/manifest.rs` already exist from Phase 2 — ensure they handle all 5 test cases; add `wasm_sha256 = None` default and missing-field error paths
- [ ] T014 [US1] Make `ipc_tests.rs` pass: add `pub fn parse_ipc_target(line: &str) -> Result<(&str, &str), String>` to `supervisor/src/main.rs` or a new `supervisor/src/ipc.rs` module; function splits `@<target>: <msg>` and returns Err if target is empty or line does not match format
- [ ] T015 [US1] Make `lifecycle_tests.rs` pass: add `pub fn should_restart(policy: &str) -> bool` to `supervisor/src/main.rs`; returns `true` only for `"always"`, false for everything else
- [ ] T016 [US1] Run full test suite inside Docker and confirm all tests in `supervisor/tests/` pass with zero warnings

**Checkpoint**: `cargo test --manifest-path supervisor/Cargo.toml` inside Docker passes. SC-001 target: completes in under 5 seconds.

---

## Phase 4: User Story 4 — Structured Supervisor Logging (Priority: P2)

**Goal**: All supervisor lifecycle events emit structured log lines (ISO timestamp + level + subsystem + optional app name + message) to stderr.

**Independent Test**: Boot VyomaOS (`make run`) and capture serial output — every app spawn and exit produces a line matching `[YYYY-MM-DDTHH:MM:SS.mmmZ] [INFO ] [lifecycle] app=<name> <event>`.

> **TDD**: Write logging tests failing first, commit, then implement.

### Tests (write failing first, commit, then implement)

- [ ] T017 [US4] Write `supervisor/tests/logging_tests.rs` with 4 failing tests: (1) `format_log(Level::Info, Subsystem::Manifest, None, "msg")` produces correct prefix format, (2) timestamp field is non-empty and contains `T`, (3) `Level::Warn` produces `[WARN ]` (5 chars with trailing space), (4) `format_log` with `app = Some("hello-world")` includes `app=hello-world` in output. Commit with message `test(logging): add failing structured logging unit tests`

### Implementation

- [ ] T018 [US4] Implement `supervisor/src/logging.rs`: add `pub enum Level { Info, Warn, Error }`, `pub enum Subsystem { Manifest, Capability, Lifecycle, Ipc, Display, Input }`, and `pub fn format_log(level: Level, sub: Subsystem, app: Option<&str>, msg: &str) -> String` using `std::time::SystemTime` for the timestamp
- [ ] T019 [US4] Add `log_info!`, `log_warn!`, `log_error!` macros to `supervisor/src/logging.rs` that call `eprintln!("{}", format_log(...))` — macros accept `(subsystem, app_opt, msg)` where `app_opt` is `Option<&str>`
- [ ] T020 [US4] Make `logging_tests.rs` pass by running `cargo test` and fixing any format discrepancies
- [ ] T021 [US4] Replace all `eprintln!("vyoma-supervisor: ...")` calls in `supervisor/src/main.rs` with the appropriate `log_info!` / `log_warn!` / `log_error!` macro calls; assign each call site the correct `Subsystem` variant
- [ ] T022 [US4] Add the `"all apps spawned"` lifecycle log line (INFO, Subsystem::Lifecycle, app=None) immediately after the last `spawn_app()` call completes in `supervisor/src/main.rs` — this is the smoke test ready signal
- [ ] T023 [P] [US4] Run `cargo check` inside Docker and confirm zero new warnings after all replacements

**Checkpoint**: Supervisor builds cleanly. `supervisor/tests/logging_tests.rs` passes. Serial output shows structured log lines on a `make run` boot.

---

## Phase 5: User Story 2 — Integration Smoke Test (Priority: P1)

**Goal**: `make test` and `make smoke` boot VyomaOS headlessly inside Docker, detect the ready signal, and exit 0/non-zero correctly.

**Independent Test**: Run `make smoke` on a clean build — exits 0 within 30s. Introduce a broken `vyoma.toml` (unknown field), rebuild, run `make smoke` — exits non-zero.

> **Note**: US4 (T022) must be complete first — the smoke test depends on the "all apps spawned" ready signal.

### Implementation

- [ ] T024 [US2] Update `base/scripts/smoke-test.sh`: implement the full script — run `qemu-system-x86_64` headlessly with `-kernel out/bzImage -initrd out/initramfs.cpio.gz -nographic -serial stdio -append "console=ttyS0 quiet"`, pipe to `grep -m1 "all apps spawned"` via `timeout 30`, exit 0 on match and 1 on timeout/error; also detect "Kernel panic" string and exit 1 if found
- [ ] T025 [US2] Update `docker/Dockerfile` to install `qemu-system-x86_64` (add `qemu-system-x86` to the `apt-get install` line)
- [ ] T026 [US2] Add `make smoke` target to `Makefile`: `$(DOCKER_RUN) bash base/scripts/smoke-test.sh`; add `smoke` to `.PHONY`
- [ ] T027 [US2] Add `make unit-test` target to `Makefile`: `$(DOCKER_RUN) cargo test --manifest-path supervisor/Cargo.toml --target x86_64-unknown-linux-musl`; add `unit-test` to `.PHONY`
- [ ] T028 [US2] Add `make test` target to `Makefile` that runs `make unit-test` then `make smoke` sequentially; add `test` to `.PHONY`; add prerequisite dependency on `build` target
- [ ] T029 [US2] Run `make smoke` on a clean build and confirm it exits 0; then temporarily rename `apps/hello-world/vyoma.toml`, rebuild rootfs, run `make smoke` — confirm it exits non-zero (SC-002 validation)

**Checkpoint**: `make test` exits 0 on a clean build. `make smoke` exits non-zero when supervisor can't find a declared app.

---

## Phase 6: User Story 3 — Build-time Capability Validation (Priority: P2)

**Goal**: `make check-manifests` validates all 205 `apps/*/vyoma.toml` files and reports errors with app name, field, and description.

**Independent Test**: Run `make check-manifests` — exits 0, prints per-app OK lines. Temporarily add `unknown_cap = true` to one `vyoma.toml`, rerun — exits non-zero with that app's name and field name in output.

### Tests (write failing first, commit, then implement)

- [ ] T030 [US3] Write `tools/check-manifests/tests/validator_tests.rs` (or inline `#[cfg(test)]`) with 3 failing tests: (1) valid TOML parses and reports OK, (2) unknown field returns `ManifestValidationError` with correct `error_kind = UnknownField` and field name, (3) duplicate name across two manifests returns `DuplicateName` error for the second. Commit with message `test(check-manifests): add failing validator unit tests`

### Implementation

- [ ] T031 [US3] Update `tools/check-manifests/Cargo.toml` to add a path dependency on the supervisor crate: `supervisor = { path = "../../supervisor" }` and set `edition = "2021"`
- [ ] T032 [US3] Implement `tools/check-manifests/src/main.rs`: iterate `apps/*/vyoma.toml` via `glob` or `std::fs::read_dir`, call `supervisor::manifest::parse_manifest()` and `validate_manifest()` for each, collect errors into `ManifestValidationError` structs, print structured output (`OK` / `ERROR` lines per the contract), exit non-zero if any errors found
- [ ] T033 [US3] Make `validator_tests.rs` pass by running `cargo test` inside the `tools/check-manifests/` crate
- [ ] T034 [US3] Add `make check-manifests` target to `Makefile`: build the validator binary inside Docker, then run it against the repo root; add `check-manifests` to `.PHONY`
- [ ] T035 [US3] Wire `make check-manifests` as a prerequisite of `make apps` in the Makefile so capability validation runs before any WASM app compilation begins

**Checkpoint**: `make check-manifests` exits 0 for all 205 valid manifests. Exits non-zero with a clear error when one manifest has an unknown field. SC-003: error message appears in first 10 lines of output.

---

## Phase 7: User Story 5 — VYOMA_DRAW Protocol Documentation (Priority: P3)

**Goal**: `docs/vyoma-draw-protocol.md` is a complete, developer-usable reference for all VYOMA_DRAW commands.

**Independent Test**: A developer writes a new display app using only `docs/vyoma-draw-protocol.md` — no supervisor source — and it renders correctly on first attempt.

- [ ] T036 [P] [US5] Create `docs/vyoma-draw-protocol.md` from the contract spec at `specs/001-vyomaos-refinement/contracts/vyoma-draw-protocol.md` — copy the full content, adjusting any relative links for the `docs/` context
- [ ] T037 [US5] Cross-check every command example in `docs/vyoma-draw-protocol.md` against `handle_draw_command()` in `supervisor/src/main.rs` — if any discrepancy exists, fix the supervisor code to match the documented behaviour (not the other way)
- [ ] T038 [P] [US5] Update `docs/vyoma-manifest-schema.md` to add a cross-reference to `vyoma-draw-protocol.md` for apps using `display = true`

**Checkpoint**: `docs/vyoma-draw-protocol.md` exists and matches supervisor behavior. SC-005: the doc is self-contained for display app development.

---

## Phase 8: User Story 6 — Faster Incremental Builds (Priority: P3)

**Goal**: Touching one app's source file causes only that app to recompile; `make build` enforces zero-warning gate via `RUSTFLAGS=-D warnings`.

**Independent Test**: Touch `apps/hello-world/src/main.rs`, run `make apps` — only `hello-world` recompiles. All other apps show cached stamp files.

- [ ] T039 [US6] Add `RUSTFLAGS := -D warnings` near the top of `Makefile` (after the variable declarations block) so it applies to all `cargo build` and `cargo test` invocations via Docker
- [ ] T040 [US6] Build the complete `APP_NAMES` list in `Makefile`: extract all app directory names from `apps/` using a `$(shell ls apps/)` pattern or explicit enumeration; assign to `APP_NAMES` variable
- [ ] T041 [US6] Replace the single `$(APPS_STAMP)` target and its body in `Makefile` with a `define APP_RULE` / `$(eval $(call APP_RULE,$(1)))` pattern rule that creates `$(OUT)/.apps/$(1).stamp` per app, depending only on `$(wildcard apps/$(1)/src/*.rs) apps/$(1)/Cargo.toml`
- [ ] T042 [US6] Update the `apps:` phony target to depend on `$(foreach app,$(APP_NAMES),$(OUT)/.apps/$(app).stamp)` instead of the old `$(APPS_STAMP)`
- [ ] T043 [US6] Validate incremental behavior: `make apps` with no changes shows all stamps up to date; touch `apps/hello-world/src/main.rs` and rerun — confirm only `hello-world` cargo invocation fires (SC-006: under 10 seconds)

**Checkpoint**: `make build` fails on any new warning. Single-app touch → single-app rebuild. SC-007: all 205 apps still compile and boot.

---

## Phase 9: Polish & Cross-Cutting Concerns

**Purpose**: Final integration checks, regression validation, and spec-kit artifact updates.

- [ ] T044 [P] Run the full `make test` suite (unit + smoke) on a fully clean build (`make clean && make build && make test`) and confirm exit 0
- [ ] T045 [P] Verify supervisor binary size: `ls -lh supervisor/target/x86_64-unknown-linux-musl/release/supervisor` — confirm still ≤ 1 MB stripped (Constitution §III)
- [ ] T046 [P] Update `specs/001-vyomaos-refinement/checklists/implementation.md` — mark all CHK items that are now satisfied by the implementation; document any intentional deferrals with justification
- [ ] T047 Update `CLAUDE.md` Testing Strategy section (currently "No formal test suite yet") to describe the new `make test` / `make unit-test` / `make smoke` / `make check-manifests` workflow

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start immediately
- **Phase 2 (Foundational)**: Depends on Phase 1 completion — BLOCKS US1, US3
- **Phase 3 (US1)**: Depends on Phase 2; provides `parse_ipc_target`, `should_restart` functions
- **Phase 4 (US4)**: Depends on Phase 2; provides structured logging + "all apps spawned" signal
- **Phase 5 (US2)**: Depends on Phase 4 (T022 "all apps spawned" ready signal) and Phase 3 (build must pass)
- **Phase 6 (US3)**: Depends on Phase 2 (manifest module); can start in parallel with US4/US2
- **Phase 7 (US5)**: No code dependencies — can start any time after Phase 2
- **Phase 8 (US6)**: No code dependencies — can start any time after Phase 1
- **Phase 9 (Polish)**: Depends on all user story phases complete

### User Story Dependencies

| Story | Depends On | Can Parallelize With |
|---|---|---|
| US1 (P1) | Phase 2 | US4, US3, US5, US6 |
| US2 (P1) | Phase 2 + US4 (T022) | US3, US5, US6 |
| US3 (P2) | Phase 2 | US1, US4, US5, US6 |
| US4 (P2) | Phase 2 | US1, US3, US5, US6 |
| US5 (P3) | Phase 1 | All others |
| US6 (P3) | Phase 1 | All others |

### Within Each User Story (TDD Order)

1. Write failing tests → commit (`test(X): add failing Y tests`)
2. Implement to make tests pass → commit (`feat(X): implement Y`)
3. Refactor if needed → commit (`refactor(X): clean up Y`)
4. Run `cargo check` with zero warnings before marking story done

### Parallel Opportunities

```
After Phase 2 completes:
  Stream A: T010 → T013 → T014 → T015 → T016 (US1: manifest+IPC+lifecycle tests)
  Stream B: T017 → T018 → T019 → T020 → T021 → T022 → T023 (US4: logging)
  Stream C: T030 → T031 → T032 → T033 → T034 → T035 (US3: check-manifests)
  Stream D: T036 → T037 → T038 (US5: documentation — no code deps)
  Stream E: T039 → T040 → T041 → T042 → T043 (US6: incremental builds)

After US4 T022 complete:
  Stream F: T024 → T025 → T026 → T027 → T028 → T029 (US2: smoke test)
```

---

## Implementation Strategy

### MVP First (US1 + US2 Only)

1. Phase 1: Setup (T001–T004)
2. Phase 2: Foundational (T005–T009)
3. Phase 3: US1 unit tests (T010–T016) → `cargo test` green
4. Phase 4: US4 logging (T017–T023) → structured log ready signal added
5. Phase 5: US2 smoke test (T024–T029) → `make test` working
6. **STOP and VALIDATE**: Full test suite passes — production-grade test harness exists

### Incremental Delivery

1. Setup + Foundational → compile-clean supervisor with extracted modules
2. US1 → unit tests green (MVP test harness)
3. US4 → structured logging (enables smoke test)
4. US2 → `make test` / `make smoke` (full CI integration)
5. US3 → `make check-manifests` (build-time validation)
6. US5 → protocol documentation (developer experience)
7. US6 → incremental builds + warning gate (developer experience)

---

## Notes

- All TDD commits must follow: `test(scope): add failing X tests` → `feat(scope): implement X` → optional `refactor(scope): clean up X`
- `[P]` tasks modify different files and have no cross-task dependencies at that point
- Each phase checkpoint must pass `cargo check` with zero warnings before proceeding
- Supervisor binary size must be verified after Phase 4 (logging module adds code)
- The `tools/check-manifests/` crate uses a path dependency on `supervisor` — if supervisor module visibility changes, update accordingly
