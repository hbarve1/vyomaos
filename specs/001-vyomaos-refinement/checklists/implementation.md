# Implementation Release Gate Checklist: VyomaOS Production Refinement

**Purpose**: Release gate validating that all requirements for the production refinement are complete, clear, consistent, and measurable before merging to `develop`. Covers all 7 implementation phases (A–G) equally. Equal weight on test isolation, build system regression, and capability security risks.
**Created**: 2026-05-24
**Feature**: [spec.md](../spec.md) | [plan.md](../plan.md) | [research.md](../research.md) | [data-model.md](../data-model.md)

---

## Requirement Completeness

- [ ] CHK001 — Are the exact failing test scenarios for manifest parsing (valid, unknown field, missing field, duplicate name, watchdog=0) all enumerated in the spec as acceptance criteria? [Completeness, Spec §US-1]
- [ ] CHK002 — Is the `ManifestError` type's complete set of variants (`UnknownField`, `MissingField`, `InvalidValue`, `DuplicateName`, `ParseError`) fully specified in the data model? [Completeness, data-model.md §ManifestValidationError]
- [ ] CHK003 — Are requirements defined for all six structured log subsystems (`manifest`, `capability`, `lifecycle`, `ipc`, `display`, `input`) — including which events each subsystem MUST log? [Completeness, Spec §FR-005, data-model.md §StructuredLogLine]
- [ ] CHK004 — Is the "all apps spawned" ready signal — the specific log message the smoke test greps for — formally defined as a requirement, not just a design note? [Completeness, research.md §Decision 3, Spec §FR-003]
- [ ] CHK005 — Are requirements defined for what `make check-manifests` outputs when ALL manifests are valid (summary format) as well as when errors exist? [Completeness, contracts/makefile-targets.md §check-manifests, Spec §FR-004]
- [ ] CHK006 — Does the spec define which existing `eprintln!` call sites in `main.rs` MUST be replaced by structured log macros, or only that new log calls must use the new format? [Completeness, Spec §FR-005]
- [ ] CHK007 — Are requirements defined for the `tools/check-manifests/` crate's binary size and compilation time, given Principle III (Minimal Surface Area)? [Gap, constitution §III]
- [ ] CHK008 — Is there a requirement specifying that `make test` depends on `make build` having succeeded — i.e., that the smoke test cannot run against a stale or partial build? [Gap, contracts/makefile-targets.md §make-test]

---

## Requirement Clarity

- [ ] CHK009 — Is the exact structured log line format (field order, padding widths, separator characters, timestamp precision) unambiguously specified, or could two implementors produce different-but-valid output? [Clarity, data-model.md §StructuredLogLine]
- [ ] CHK010 — Is "first-registered wins" for duplicate app names defined in terms of a concrete ordering (e.g., order of entries in `boot.toml`) rather than an implementation-dependent detail like filesystem sort order? [Clarity, Spec §FR-006, Spec §Clarifications]
- [ ] CHK011 — Is the `parse_manifest()` function's exact signature (input type, return type, error type) specified in the plan/data model, or left to implementor discretion? [Clarity, plan.md §Phase A, data-model.md §AppManifest]
- [ ] CHK012 — Is the phrase "surface prominently" in FR-009 fully resolved to "exits non-zero" as the single measurable definition, with no residual ambiguity? [Clarity, Spec §FR-009, Spec §Clarifications Q4]
- [ ] CHK013 — Is the smoke test "PASS" condition stated as: ready signal found within 30 seconds AND no kernel panic line detected — or only the positive signal? [Clarity, research.md §Decision 3, data-model.md §SmokeTestResult]
- [ ] CHK014 — Is the VYOMA_DRAW `draw_text` size parameter (`s`/`m`/`l`) specified with its exact pixel dimensions (4×8, 8×16, 16×32) in the protocol documentation, not just mentioned as "small/medium/large"? [Clarity, contracts/vyoma-draw-protocol.md §draw_text]
- [ ] CHK015 — Is "actionable error message" in SC-003 defined with a concrete measurability criterion beyond "within the first 10 lines of output"? For example: does it require the suggested fix to appear, or only the field name? [Clarity, Spec §SC-003]

---

## Requirement Consistency

- [ ] CHK016 — Are the test execution environment requirements (Docker + musl target) stated consistently across FR-002 (unit tests), FR-003 (smoke test), and the Assumptions section — with no contradictions? [Consistency, Spec §FR-002, §FR-003, §Assumptions, Spec §Clarifications Q1 Q2]
- [ ] CHK017 — Does the `make check-manifests` contract define the same error format (`ERROR apps/… [name] ErrorKind: message`) as the data model's `ManifestValidationError` entity output format? [Consistency, contracts/makefile-targets.md §check-manifests, data-model.md §ManifestValidationError]
- [ ] CHK018 — Are the capability field names listed in the data model (`stdio`, `filesystem`, `network`, `network_port`, `display`, `shell`, `mouse`, `watchdog_secs`) consistent with the manifest schema documented in `docs/vyoma-manifest-schema.md` and `CLAUDE.md`? [Consistency, data-model.md §Capabilities, Spec §FR-001]
- [ ] CHK019 — Does SC-001 ("cargo test completes in under 5 seconds") remain consistent with the requirement to run inside Docker, given container startup overhead? Is the 5s target measured from test execution start or from `docker run` invocation? [Consistency, Spec §SC-001, Spec §Clarifications Q1]
- [ ] CHK020 — Is the RUSTFLAGS=-D warnings gate applied consistently to ALL `cargo build` invocations in the Makefile (supervisor, apps, check-manifests tool) or only to the supervisor? [Consistency, Spec §FR-009, contracts/makefile-targets.md §make-build]

---

## Acceptance Criteria Quality

- [ ] CHK021 — Can SC-002 ("0 false negatives on clean build, 0 false positives on broken manifest") be objectively measured with a reproducible test procedure — i.e., is there a defined method to introduce a "broken manifest" and verify the smoke test catches it? [Measurability, Spec §SC-002]
- [ ] CHK022 — Is SC-006 ("single-app incremental build time drops to under 10 seconds") measurable with a defined baseline procedure (which app, what change, measured how)? [Measurability, Spec §SC-006]
- [ ] CHK023 — Does SC-007 ("all previously passing apps compile and boot without modification") have a defined reference baseline — i.e., is there a known list of "previously passing apps" against which regression is measured? [Measurability, Spec §SC-007]
- [ ] CHK024 — Is the acceptance scenario for US-1 ("Given a vyoma.toml with unknown capability field, When manifest parser processes it, Then returns structured error") fully specified to distinguish between a parse-time error and a startup-time error? [Acceptance Criteria, Spec §US-1]
- [ ] CHK025 — Are the TDD "Red phase" requirements objectively verifiable — e.g., is there a requirement to commit the failing test before the implementation commit, and if so, how is this enforced or verified? [Measurability, Spec §FR-010, constitution §II]

---

## Scenario Coverage

- [ ] CHK026 — Are requirements defined for the scenario where `cargo test` itself fails to compile (distinct from test failures at runtime) — should this be treated as a build failure or a test failure? [Coverage, Gap, Spec §FR-002]
- [ ] CHK027 — Are requirements defined for the scenario where the smoke test QEMU process hangs without producing any output (neither ready signal nor error) — does `timeout` handle this, and is this specified? [Coverage, Spec §US-2, research.md §Decision 3]
- [ ] CHK028 — Are requirements defined for the IPC routing scenario where the sender app exits mid-transmission — does the router need to handle partial messages? [Coverage, Gap, Spec §US-1 §FR-001]
- [ ] CHK029 — Is the scenario covered where a new app is added to `apps/` but not listed in `APP_NAMES` in the Makefile — will it silently not compile, or is there a discovery/warning mechanism required? [Coverage, Gap, plan.md §Phase D]
- [ ] CHK030 — Are requirements defined for the recovery path if the manifest module extraction (Phase A) breaks existing supervisor behavior — specifically, is SC-007 (no regressions) sufficient as a regression gate, or should explicit rollback criteria be defined? [Coverage, Exception Flow, Spec §SC-007]

---

## Edge Case Coverage

- [ ] CHK031 — Is the edge case where `watchdog_secs` is set to a negative value addressed in requirements? (Note: the spec's edge cases section mentions it but data-model states "negative values cannot be represented" due to `u32` type — are the requirements fully consistent on this?) [Edge Case, Spec §Edge Cases, data-model.md §Capabilities]
- [ ] CHK032 — Is the edge case where a WASM binary is present but zero-bytes (empty file) handled by requirements — is this distinct from "missing file" in the validation logic? [Edge Case, Gap, Spec §FR-001]
- [ ] CHK033 — Are out-of-bounds coordinate requirements specified for `fill_rect` and `draw_text` in both the windowed and full-screen contexts — specifically, are negative coordinate values (`i32` → `u32` conversion) addressed? [Edge Case, contracts/vyoma-draw-protocol.md §Coordinate System]
- [ ] CHK034 — Is the edge case where the entire `[capabilities]` section is omitted from `vyoma.toml` (not just individual fields) specified — should it default to all-false or be a validation error? [Edge Case, Gap, data-model.md §Capabilities]

---

## Non-Functional Requirements

- [ ] CHK035 — Is the binary size constraint (supervisor ≤ 1 MB stripped) specified as a requirement that must be verified after the manifest.rs and logging.rs modules are added — i.e., does the size gate apply post-refactor? [NFR, Spec §Assumptions, constitution §III]
- [ ] CHK036 — Is there a requirement specifying how many test files / test cases constitute "sufficient" coverage for the supervisor unit tests (FR-001), or is coverage implicitly defined by the 4-subsystem enumeration in US-1? [NFR, Coverage, Spec §FR-001, §US-1]
- [ ] CHK037 — Are security requirements defined for the structured log's "no message payload" rule — specifically, is there a requirement to sanitize or truncate app-supplied content that appears in log messages (e.g., app names from vyoma.toml)? [NFR, Security, data-model.md §StructuredLogLine, constitution §I]
- [ ] CHK038 — Is there a performance requirement for `make check-manifests` — e.g., maximum acceptable run time for 205 manifests — to ensure it doesn't become a build bottleneck? [NFR, Spec §FR-004]

---

## Dependencies & Assumptions

- [ ] CHK039 — Is the assumption that QEMU is (or will be) installed in the `vyomaos-builder` Docker image traceable to a concrete Dockerfile change requirement — i.e., is there a requirement that the Dockerfile be updated, not just assumed to have QEMU? [Assumption, Spec §Assumptions, Spec §Clarifications Q2]
- [ ] CHK040 — Is the assumption that `cargo test` for the supervisor can compile against `x86_64-unknown-linux-musl` inside Docker (without a running Linux kernel) validated — specifically, are there any `#[cfg(target_os = "linux")]` guards that would prevent test compilation? [Assumption, research.md §Decision 2, Spec §Assumptions]
- [ ] CHK041 — Is the path dependency from `tools/check-manifests/` to `supervisor/src/manifest.rs` specified as a requirement — or is it an implementation detail that could result in code duplication if not enforced? [Dependency, plan.md §Phase G]
- [ ] CHK042 — Is the assumption that all 205 apps in `apps/` currently compile without warnings validated — i.e., is there a known baseline before `RUSTFLAGS=-D warnings` is added, or could enabling the flag immediately break the build? [Assumption, Spec §FR-009, constitution §QG-1]

---

## Ambiguities & Conflicts

- [ ] CHK043 — Is the scope of FR-010 (TDD Red→Green→Refactor for "all non-trivial logic") unambiguous — specifically, does replacing `eprintln!` calls with log macro calls constitute "non-trivial logic" requiring a failing test, or is it a mechanical substitution? [Ambiguity, Spec §FR-010, constitution §II]
- [ ] CHK044 — Is there a potential conflict between FR-008 (per-app incremental builds using Makefile timestamps) and the Docker run pattern (each `docker run` starts a fresh container) — specifically, are stamp files written inside or outside the container, and is this specified? [Conflict, Spec §FR-008, research.md §Decision 4]
- [ ] CHK045 — Is the relationship between `make test` (unit tests + smoke) and `make build` (the full build gate) clearly specified — can `make test` be run without first running `make build`, and if not, is this documented as an explicit prerequisite? [Ambiguity, contracts/makefile-targets.md §make-test, Spec §FR-003]

---

## Notes

- Mark items complete with `[x]` as each is verified during implementation and PR review
- `[Gap]` items represent missing requirements that should be resolved before implementation begins
- `[Ambiguity]` items should be resolved via spec update before Phase implementation starts
- All CHK001–CHK045 items must be resolved (or explicitly deferred with justification) before merging `001-vyomaos-refinement` → `develop`
