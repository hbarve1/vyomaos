# VyomaOS Documentation Index

Central navigation page for all VyomaOS documentation.

---

## Getting Started

- [README](../README.md) -- Project overview, quick start, and vision statement
- [Contributing Guide](../CONTRIBUTING.md) -- How to contribute, coding standards, PR workflow
- [Git Workflow](git-workflow.md) -- Branching strategy, commit conventions, and release process

## Architecture & Design

- [Architecture Overview](../CLAUDE.md) -- System stack, supervisor design, build system, app model
- [Manifest Schema](vyoma-manifest-schema.md) -- vyoma.toml format, capability declarations, and examples
- [Draw Protocol](vyoma-draw-protocol.md) -- VYOMA_DRAW line-oriented display protocol specification

## Operations

- [Testing Strategy](testing-strategy.md) -- Unit tests, manifest validation, smoke tests, CI pipeline
- [Security Audit Checklist](security-audit-checklist.md) -- Capability enforcement, seccomp, attack surface review
- [Observability](observability.md) -- Structured logging, heartbeat emitter, JSON-line format
- [OTA Updates](ota-updates.md) -- A/B slot update manager, health checks, rollback procedures

## Platform Profiles

VyomaOS supports six platform profiles (mcu-minimal, iot-edge, robotics-rt, mobile, desktop-full, server-headless). Each profile is defined as a TOML file in [`supervisor/src/profile/profiles/`](../supervisor/src/profile/profiles/). See the [Architecture Overview](../CLAUDE.md) for build and run commands per platform.

## Vision & Roadmap

- [R1 Terminal Desktop Checklist](../TODOS.md) -- Active release scope, dependency-ordered tasks, acceptance gates, and progress handoff
- [Codex Work-Loop Proposal](codex-work-loop.md) -- Bounded milestone runs, persistent evidence, PR workflow, and prerequisites for recurring work
- [Complete OS Roadmap](complete-os-roadmap.md) -- Long-term graphical desktop and device expansion after terminal-first R1 (2026-09-29)
- [Complete OS Readiness Review](reviews/2026-09-29-complete-os-review.md) -- Source-backed gaps, executed checks, and review of existing plans
- [80-Subsystem Delivery Mapping](reviews/2026-09-29-subsystem-matrix.csv) -- Design status separated from implementation evidence and proposed milestones
- [Application Inventory](reviews/2026-09-29-app-inventory.csv) -- All 208 app manifests, source counts, capabilities, and proposed audit priorities
- [Comparison Matrix](comparison-matrix.md) -- VyomaOS vs Alpine, Flatcar, MirageOS positioning
- [Desktop OS Vision](superpowers/specs/desktop-os-vision/master-spec.md) -- 80-subsystem desktop OS master specification
- [Full OS Roadmap](superpowers/specs/2026-05-20-vyomaos-full-os-roadmap.md) -- Long-term roadmap for a complete general-purpose OS

## Implementation Plans

- [Historical Implementation Tracker](../.context/plans/plan-vyomaos/README.md) -- Original phase-by-phase tracker; phase checkmarks are not current release acceptance evidence

## Design Explorations (Archive)

Archived spec explorations from the early design phase. These informed current architecture but are not authoritative references.

- [specs/001-vyomaos-refinement](../specs/001-vyomaos-refinement) -- Initial architecture refinement and trade-off analysis
- [specs/002-windowing-mouse-input](../specs/002-windowing-mouse-input) -- Windowing system and mouse input design
- [specs/042-interactive-window-management](../specs/042-interactive-window-management) -- Interactive window management and focus model
- [specs/043-universal-modular-os](../specs/043-universal-modular-os) -- Universal modular OS subsystem decomposition
- [specs/044-developer-tooling](../specs/044-developer-tooling) -- Developer tooling and SDK design
- [specs/046-apple-ui-fidelity](../specs/046-apple-ui-fidelity) -- High-fidelity UI rendering (macOS chrome parity)

## Interactive

- [Architecture Mindmap](vyomaos-mindmap.html) -- Interactive D3-based visualization of the VyomaOS architecture
