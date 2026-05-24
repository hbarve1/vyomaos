<!--
SYNC IMPACT REPORT
==================
Version change: (new) → 1.0.0
Added sections: Core Principles (I–VI), Quality Gates, Development Workflow, Governance
Removed sections: N/A (initial version from template)
Templates requiring updates:
  ✅ .specify/templates/plan-template.md — Constitution Check section applies
  ✅ .specify/templates/spec-template.md — User Scenarios mandate aligns with Principle III
  ✅ .specify/templates/tasks-template.md — Test tasks required per Principle III
Deferred TODOs: None
-->

# VyomaOS Constitution

## Core Principles

### I. Capability-Secure by Default (NON-NEGOTIABLE)
Every app MUST declare all capabilities it needs in `vyoma.toml` before accessing
any system resource. Capabilities not declared MUST NOT be accessible at runtime —
no overrides, no fallbacks, no ambient authority.

**Rationale**: The capability model is VyomaOS's primary security boundary. Any
deviation defeats the entire trust model and cannot be justified by convenience.

### II. Test-First Development (NON-NEGOTIABLE)
All non-trivial logic MUST follow Red → Green → Refactor:
- Write a failing test that captures the intended behaviour.
- Implement the minimum code to make it pass.
- Refactor for clarity without breaking the test.

`cargo check` MUST pass with zero new warnings before any commit. Integration
smoke test (`make build && make run`) MUST pass before merging to `develop`.

**Rationale**: VyomaOS is system-level software; regressions in the supervisor or
IPC broker can brick the entire OS. Tests are the primary safety net.

### III. Minimal Surface Area
Every component — kernel config, supervisor binary, WASM app, Cargo dependency —
MUST justify its existence. Unused code, unused capabilities, and unused
dependencies MUST be removed. Target: supervisor ≤ 1 MB stripped, each app ≤ 50 KB.

**Rationale**: Bloat in a boot-critical binary increases attack surface and boot
latency. YAGNI is enforced, not advisory.

### IV. Hermetic, Reproducible Builds
All compilation MUST run inside the `vyomaos-builder` Docker container. Host-only
builds are permitted only for `cargo check` (type-checking). Release artifacts
MUST be byte-reproducible across clean container runs.

**Rationale**: Reproducibility is a prerequisite for supply-chain integrity and
deterministic WASM binary output across machines.

### V. Explicit Over Implicit
All behaviour MUST be driven by explicit configuration (`vyoma.toml`, `boot.toml`,
`settings.toml`). Magic defaults, environment-variable-driven behaviour, and
runtime feature detection are forbidden in production paths.

**Rationale**: An OS whose behaviour depends on implicit state cannot be reasoned
about, audited, or reproduced reliably.

### VI. Observability First
Every supervisor subsystem MUST emit structured log lines to stderr at startup and
on state changes (app spawn, kill, restart, IPC route, capability grant/deny).
Silent failures in PID 1 are unrecoverable; logs are the primary debug tool.

**Rationale**: Inside QEMU there is no debugger attach — structured logs are the
only window into system behaviour during development and production.

## Quality Gates

These gates MUST be satisfied before any pull request is merged:

1. **Type-check**: `cargo check` passes for all modified crates (zero new warnings).
2. **Capability audit**: New/modified apps declare only the minimum required
   capabilities in `vyoma.toml`.
3. **Binary size**: New apps MUST use `opt-level = "z"` and `strip = true`.
4. **Smoke test**: `make build && make run` boots to supervisor prompt without
   errors (verified via serial log output).
5. **Spec-kit artifacts**: Feature branches MUST have current `spec.md`, `plan.md`,
   and `tasks.md` in `specs/<branch>/` before implementation tasks are closed.
6. **No regressions**: All previously passing apps MUST still compile and boot.

## Development Workflow

```
feature branch
  → /speckit-specify   (define what to build)
  → /speckit-clarify   (de-risk ambiguities)
  → /speckit-plan      (technical design)
  → /speckit-checklist (quality checklist)
  → /speckit-tasks     (actionable task list)
  → /speckit-analyze   (cross-artifact consistency)
  → /speckit-implement (execute)
  → PR to develop
  → merge to main (releases only, tagged)
```

All feature work happens on branches `NNN-short-description` off `develop`.
Direct commits to `main` are forbidden except for release merges from `develop`.
Commit format: `type(scope): description` (Conventional Commits).

## Governance

- This constitution supersedes all prior implicit conventions in this repository.
- Amendments require: a PR updating this file, a semver version bump
  (MAJOR = principle removal/redefinition; MINOR = new principle added;
  PATCH = wording/clarification), and a one-paragraph rationale in the PR.
- All PRs MUST include a "Constitution Check" section confirming compliance with
  Principles I–VI and all Quality Gates.
- Complexity beyond what a principle requires MUST be explicitly justified in the
  PR description or the PR will be rejected at review.
- Use `CLAUDE.md` for runtime development guidance; this file governs principles only.

**Version**: 1.0.0 | **Ratified**: 2026-05-24 | **Last Amended**: 2026-05-24
