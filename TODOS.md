# VyomaOS TODOs — first terminal desktop release

**Release:** R1, terminal-first PC operating system. **Status:** planned; implementation acceptance is still open.

**Scope:** boot a supported x86-64 PC into a usable local console/TUI, backed by the Rust supervisor and WASM applications. Provide a real shell, file tools, text editor, networking, package management, durable storage, installation and recovery. QEMU is the automated reference target; qualify one named physical PC before claiming hardware support.

“Terminal-first” means the normal session works without the Vyoma graphical desktop/compositor. A kernel text console or minimal terminal display backend is allowed. A GUI window displaying a simulated shell is insufficient. Serial console is an additional development/recovery interface, not a substitute for the local keyboard/display experience.

Keep Linux for hardware, memory and scheduling, Rust for trusted services, and WASM for the application platform. R1 does not promise POSIX/Bash compatibility, native Linux application compatibility, a self-hosted compiler toolchain, a modern web browser, audio/video, a graphical desktop, or mobile/MCU support. Those remain later milestones in the [long-term roadmap](docs/complete-os-roadmap.md).

## Tracking rules

- An unchecked box is pending. Use the active-work table below for work in progress or blocked work.
- Mark an implementation item complete only when its acceptance evidence is recorded and the change is merged into `develop`. A PR may say “implementation verified; awaiting merge” while its box stays unchecked.
- Evidence must identify the task, commit, commands, target/configuration, result and artifact/log location. A proposed design, mock reply or screenshot alone does not prove completion.
- Run tests through production code and actual WASM/VM paths where the contract crosses those boundaries. Mandatory release tests must fail when artifacts are missing, not silently skip.
- GitHub Actions is billing-blocked: run all validation locally until the maintainer changes this instruction. Record local commands/results in PRs; remote CI success is not available evidence.
- Follow repository worktree/PR conventions. Keep security, persistence and recovery requirements when reducing interface scope.
- `T0`–`T7` are the R1 execution order. Existing `M*`, `P*` and `OS-*` numbers remain historical/long-term references, not competing R1 queues.

## Active work / handoff

| Task | State | Branch / PR | Evidence / blocker | Next action |
|---|---|---|---|---|
| R1 planning | Merged into `develop` | [PR #215](https://github.com/hbarve1/vyomaos/pull/215), `548bc134` | [Source review](docs/reviews/2026-09-29-complete-os-review.md) | Plan is the active R1 backlog |
| T0-01 | Merged into `develop` | [PR #216](https://github.com/hbarve1/vyomaos/pull/216), `e447db5c` | [Build/test evidence](docs/validation/2026-09-29-t0-01.md); compilation gate accepted | Complete; broader T0 failures remain tracked below |
| T0-02 | Merged into `develop` | [PR #217](https://github.com/hbarve1/vyomaos/pull/217), `build/t0-pinned-inputs` | [Clean-builder evidence](docs/validation/2026-09-29-t0-02.md); 1,154 Rust tests pass, existing Python failures remain | Complete; GitHub merge verified 2026-09-29; continue T0-03 |
| Continuous local worker | PR merged; this bounded invocation authorized by maintainer | [PR #218](https://github.com/hbarve1/vyomaos/pull/218), `ops/continuous-worker`, stacked on PR #217 at `baab51b5` | [Operating plan](docs/codex-work-loop.md); [wrapper evidence](docs/validation/2026-09-29-continuous-worker.md); explicit run limits and real pilot still pending | Control files unchanged by implementation worker; operator owns activation/limits |
| T0-03 | Locally verified; awaiting review; run `20260929T165353Z-326060` | `fix/t0-kernel-inputs`; base `857caca5`; parent: none; tested head `939a67e7` | [Evidence](docs/validation/2026-09-29-t0-03.md): 14 regressions, real config switch/rebuild/KVM boot pass; inherited Python failures remain | Review/merge; continue T0-04 on this tested tip |
| T0-05 / T0-06 | Existing harness failures confirmed; pending | Not started | Same evidence report: assertion API mismatch, shared serial-log deletion, black window capture | Repair API/state/readiness checks; retain failures until verified |
| T0-04 / T0-GATE | Guest health blocker confirmed; pending | Not started | `clock.wasm` panics on thread creation and repeatedly restarts; weak smoke checks still pass | Define R1 app set and assert sustained guest health; see T2-07 for restart limits |

Worker PR #218 is merged. This bounded run is explicitly maintainer-authorized; implementation work does not change runner activation or limits. See the [operating plan](docs/codex-work-loop.md) for operator-owned controls.

## T0 — Make build and verification reliable

Dependencies: none. Start here. Reuse the review baseline at local commit `418359d3`; recheck it against current `develop` when implementation begins.

- [x] **T0-01** Fix the host CLI's missing Clap `env` feature and the supervisor warning; move release profiles to the workspace root. **Accept:** CLI and production-target supervisor compile under the documented warning gate. **Evidence:** [PR #216](https://github.com/hbarve1/vyomaos/pull/216), merged as `e447db5c`; [local results](docs/validation/2026-09-29-t0-01.md).
- [x] **T0-02** Pin the supported Rust/kernel/runtime/container inputs and lock dependency resolution; document Docker, musl and Python test setup. **Accept:** a clean builder runs the same declared toolchain without relying on host caches or floating Rust `stable`.
- [ ] **T0-03** Repair kernel rebuild/config propagation and stale-artifact reuse; make unsupported target names fail explicitly. **Accept:** changing the selected kernel input rebuilds the correct artifact.
- [ ] **T0-04** Define an R1 image/app allowlist; make rootfs assembly fail on missing selected binaries/manifests. **Accept:** deleting a required artifact causes an actionable build failure; optional GUI/demo apps are excluded from the R1 image.
- [ ] **T0-05** Fix clean CI to build the kernel before smoke, preserve profile-test exit status, and resolve E2E image paths independently of working directory. **Accept:** clean build → boot executes in CI; required missing images/tests fail instead of skip.
- [ ] **T0-06** Add isolated test-state paths, disposable VM disks, captured serial/QEMU stderr, command transcripts and artifact digests. **Accept:** tests do not use personal data or depend on an existing `out/` directory.
- [ ] **T0-GATE** Reproduce build and real WASM startup from a fresh checkout in the documented environment; publish baseline boot time, idle RSS and image size. **Evidence:** exact commands, configuration, commit and logs.

## T1 — Boot to a real console session

Dependencies: `T0-GATE`. This first console slice is a development milestone; secure user operation also requires T2.

- [ ] **T1-01** Add an explicit terminal desktop boot profile with a minimal app/service set; avoid starting desktop/dock/window services. **Accept:** supervisor and shell start with the graphical compositor absent.
- [ ] **T1-02** Provide usable local keyboard/display console and serial console I/O, including terminal size, Unicode text, echo/raw mode, scrollback behavior and resize handling. **Accept:** enter commands and read results on both configured endpoints; physical input is later qualified in T7.
- [ ] **T1-03** Separate terminal byte streams, logs and privileged control messages. **Accept:** ordinary output containing `@supervisor:` or draw-protocol-looking text cannot execute commands; one app's output cannot impersonate another app/control event.
- [ ] **T1-04** Define foreground-session ownership, input routing and terminal restoration after normal exit, panic, cancellation or disconnect. **Accept:** exiting/crashing a TUI returns a usable prompt without lost or cross-session input.
- [ ] **T1-05** Replace simulated/hardcoded shell data with real command results; make help accurately list the supported command language. **Accept:** `pwd`, directory listing, identity and process information reflect actual state.
- [ ] **T1-06** Add command-specific VM assertions. **Accept:** a test enters a unique command/nonce and observes its exact result; boot-log growth alone cannot satisfy the test.
- [ ] **T1-GATE** Boot the development console profile, run a real WASM command, observe stdout/stderr/exit status, cancel a foreground app and successfully run another command.

## T2 — Enforce security and contain app failures

Dependencies: T0 harness and T1 real guest I/O. These items are release blockers, including for a text-only OS.

- [ ] **T2-01** Define app/bundle, instance and session identity plus an operation-to-permission policy; deny privileged broker operations by default. **Accept:** all dispatch paths authorize before side effects, including legacy IPC.
- [ ] **T2-02** Replace whole-`/data` app grants with private app storage and explicit user-document grants. **Accept:** another app's data, OS policy, audit logs and update state are inaccessible; traversal/symlink/race cases are covered.
- [ ] **T2-03** Enforce the same network/filesystem/admin policy on direct WASI calls and supervisor-mediated requests. **Accept:** a real denied guest cannot bypass restrictions through HTTP/download/file/process/power commands.
- [ ] **T2-04** Disable remote management by default in release mode; authenticate and authorize developer access, validate bundle IDs/paths, and limit connections, request lines, transfer sizes and timeouts. **Accept:** real-service unauthenticated/oversize/slow/traversal requests fail safely.
- [ ] **T2-05** Implement first-boot credential enrollment and login/logout/lock; remove the default admin PIN and bind privileges to the requesting session. **Accept:** unauthorized administration fails; logout revokes session grants; login attempts are rate-limited.
- [ ] **T2-06** Remove XOR-based secret handling from the release path, use reviewed password/secret-storage primitives, and redact audit fields before persistence. **Accept:** fixture passwords/keys never appear in logs; wrong credentials and tampered secrets fail; recovery behavior is documented.
- [ ] **T2-07** Bound CPU/memory, terminal output, queues, logs, open files and concurrent requests; apply restart backoff. **Accept:** busy, flooding, blocked and crashing guests cannot take down PID 1 or another session.
- [ ] **T2-08** Resolve the runtime topology with a small experiment: retain process isolation, prove any embedded-worker/WIT path with real components, and remove unsafe adapter assumptions before adoption. **Accept:** measured crash/limit/callback tests and an ADR; no mandatory whole-supervisor rewrite.
- [ ] **T2-GATE** Run an adversarial guest suite against production launch and broker paths. **Accept:** all deny/containment cases pass while an authorized editing session remains usable.

## T3 — Make storage, installation and recovery independent

Dependencies: `T0-GATE`, `T1-GATE`, and T2 identity/storage policy. Exercise the installer only on disposable test disks during automated development.

- [ ] **T3-01** Specify an EFI/OS/recovery/state disk layout with room for updates and stable partition identifiers. **Accept:** documented layout matches generated test images.
- [ ] **T3-02** Implement real terminal installer steps: preflight, target confirmation, partition/filesystem creation, image copy, bootloader configuration and first-user setup. **Accept:** no placeholder operation reports success; invalid targets fail before writes.
- [ ] **T3-03** Boot through UEFI from the installed disk with no QEMU `-kernel` shortcut or host 9P share. **Accept:** remove installation media and reach the local/serial login path.
- [ ] **T3-04** Mount durable system/user/app storage by stable identifier; distinguish installed/live/recovery modes. **Accept:** missing required storage enters explicit recovery rather than a normal-looking volatile session.
- [ ] **T3-05** Implement durable save/rename/metadata updates, disk-full handling, quotas, shutdown flush and consistency checks. **Accept:** interrupted writes retain a valid previous or new document, with errors reported to the user.
- [ ] **T3-06** Provide terminal backup/restore and bootable recovery with integrity and credential-recovery rules. **Accept:** restore a backup to another disposable disk and open the recovered documents.
- [ ] **T3-07** Record the disk-encryption threat model and use a maintained disk-encryption implementation if encrypted-at-rest protection is included. **Accept:** lost-disk protection is tested if claimed; a secret vault is never described as full-disk encryption.
- [ ] **T3-GATE** Install → boot → log in → save document/settings → shutdown → reboot → reopen with identical contents; pass defined power-loss, disk-full and restore cases.

## T4 — Deliver useful shell, tools and a TUI editor

Dependencies: T1 terminal contract, T2 policy and T3 persistent paths. Implement a documented WASM shell contract; do not silently introduce arbitrary native execution or claim full POSIX compatibility.

- [ ] **T4-01** Implement command parsing, quoting/escaping, environment, working directory, history, completion and meaningful exit codes. **Accept:** tests cover spaces, Unicode, malformed input and nonexistent commands/files.
- [ ] **T4-02** Launch real WASM tools with arguments, environment and separate stdin/stdout/stderr; preserve exit status. **Accept:** launch independent tools from the installed registry and report real success/failure.
- [ ] **T4-03** Implement bounded streaming pipelines/redirection and a documented foreground/background job model, including cancellation and cleanup. **Accept:** large streams do not require buffering the whole result; failed/cancelled jobs release resources.
- [ ] **T4-04** Supply real file utilities: `pwd`, `ls`, `cd`, `cat`, `mkdir`, `cp`, `mv`, `rm`, text search and a pager (or documented equivalents). **Accept:** each operates on actual permitted files and reports access/I/O errors correctly.
- [ ] **T4-05** Supply a usable terminal editor with open/save, cursor movement, selection or documented editing equivalents, undo, search, Unicode and unsaved-change prompts. **Accept:** edit a multi-page file through keyboard input, save and reopen after reboot.
- [ ] **T4-06** Provide terminal process/service/log/storage tools with actual measurements. **Accept:** inspect, cancel/restart and diagnose a failing app with authorization enforced.
- [ ] **T4-07** Define a small versioned terminal SDK/API; implement terminal sizing, key events and TUI lifecycle without mixing control and data streams. **Accept:** an external sample repository builds and runs a TUI WASM app with a recoverable unsupported-ABI error.
- [ ] **T4-08** Make all R1 operations keyboard-accessible; test the chosen screen-reader/accessible terminal route and documented text/locale support. **Accept:** core install/login/edit/recovery workflows need no mouse and expose readable terminal output.
- [ ] **T4-GATE** Complete a real console session: create directories, inspect/search files, edit/save, pipe/filter/redirect output, run/cancel jobs, inspect logs and reopen saved work after restart.

## T5 — Connect, transfer files and support development

Dependencies: T2 policy, T3 state, T4 shell/tools. R1 networking is wired Ethernet on the selected target; Wi-Fi/laptop promises require their own acceptance work.

- [ ] **T5-01** Implement wired interface configuration/address acquisition, DNS, clock synchronization and offline/reconnect errors. **Accept:** connect to controlled test services, lose/recover connectivity, and retain a usable console.
- [ ] **T5-02** Implement HTTPS download/upload tools with certificate and hostname validation, timeouts, bounded transfers and safe destinations. **Accept:** valid transfers match digests; wrong/expired certificates and denied destinations fail.
- [ ] **T5-03** Repair and document host-side create/build/run/push/log/inspect workflows using the secured management transport. **Accept:** an independent developer builds a WASM tool and observes its actual execution in the VM.
- [ ] **T5-04** Define terminal access/file-transfer scope explicitly; if adding SSH, use a maintained implementation and test authentication, host keys and cancellation. **Accept:** claimed remote workflows work against a real endpoint; simulated `ssh-client` output does not count. SSH can remain explicitly unsupported in R1 if the authenticated developer transport supplies the required workflow.
- [ ] **T5-GATE** Fetch a real HTTPS fixture into permitted storage, inspect/edit it, and use the host SDK/CLI to run and debug a real WASM app over authenticated transport.

## T6 — Install and update real signed applications and OS images

Dependencies: T2 trust/policy, T3 disk/boot/recovery, T4 command/SDK contracts and T5 transfer path. Offline bundle installation should also work.

- [ ] **T6-01** Define signed bundles covering app identity/version, ABI, capabilities and payloads; provision protected trust roots. **Accept:** altered content/manifests, untrusted signers and unsupported ABI versions are rejected.
- [ ] **T6-02** Implement transactional terminal app install/list/update/remove: staging, validation, approval of changed grants, activation, health checks, rollback and state migration. **Accept:** installed/updated code really executes; failed operations preserve the previous usable app.
- [ ] **T6-03** Replace simulated management push success with the real transaction service. **Accept:** success identifies the activated version/digest; a rejected/discarded upload cannot be reported as committed.
- [ ] **T6-04** Wire signed OS updates to actual bootloader slot selection, trial-boot counters, health confirmation and rollback/recovery. **Accept:** reboot executes the selected image and automatically recovers from a bad trial boot.
- [ ] **T6-05** Implement metadata expiry/downgrade rules, key rotation/revocation, durable transaction state and rollback-compatible data schemas. **Accept:** expired/revoked/downgraded updates fail according to documented policy.
- [ ] **T6-GATE** Test interruption at each app/OS update phase, offline boot, bad signatures, failed health checks and rollback. **Accept:** the next boot has valid old/new code and intact documents.

## T7 — Qualify the first terminal desktop release

Dependencies: all T0–T6 gates. Hardware-dependent items remain blocked/unverified until the actual machine and test access are available.

- [ ] **T7-01** Name the reference PC and qualify UEFI, storage, local display/text console, USB keyboard and wired NIC; record firmware/driver versions. **Accept:** documented install/boot/session/recovery tests pass on that machine.
- [ ] **T7-02** Run 20 disposable-VM install/reboot/update/rollback cycles and a 24-hour mixed terminal workload. **Accept:** no document loss, PID 1 failure, stuck console or sustained unexplained memory growth.
- [ ] **T7-03** Establish measured boot, input-response, memory and image-size budgets from the R1 profile. **Accept:** publish hardware/workload and results; existing GUI/512-MiB/<5-second claims are not inherited without evidence.
- [ ] **T7-04** Pass the complete install/login/files/editor/jobs/network/packages/recovery/accessibility/SDK suite through real production paths. **Accept:** required tests are green, run on the release candidate, and do not silently skip.
- [ ] **T7-05** Resolve release-blocking security/data-loss defects; review dependencies, licenses, firmware/fonts and build provenance. **Accept:** no insecure placeholder is reachable in the supported release feature set.
- [ ] **T7-06** Prepare signed images, checksums, SBOM, recovery media, release notes, supported-command/hardware lists, known limitations and patch/reporting policy. **Accept:** a second person can reproduce installation, backup, recovery and a test update using the instructions.
- [ ] **T7-GATE** Review the release evidence and obtain maintainer release approval. **Accept:** publish only the explicitly supported terminal-first scope; release publication is separate from opening implementation PRs.

## First implementation order

1. `T0-01` → `T0-02` → `T0-03`/`T0-04` → `T0-05`/`T0-06` → `T0-GATE`.
2. Deliver `T1-01`–`T1-06` as the first real console development slice.
3. Complete T2 authorization/containment before presenting the console as safe for untrusted apps or secrets.
4. Complete T3 durability, then the T4 real tool/editor workflow, T5 connectivity and T6 transactions.
5. Qualify T7 using the exact release artifacts. Do not turn unperformed physical tests into checked boxes.

Scope and contract work can proceed alongside implementation; keep dependency changes explicit. GUI/browser/media work is not on the R1 critical path.

## After R1 — preserve the complete OS ambition

- [ ] **LATER-01** Deliver the graphical desktop/shared UI SDK, windowing, pointer interaction, scaling and semantic accessibility.
- [ ] **LATER-02** Qualify a maintained modern web engine and real browser workflows.
- [ ] **LATER-03** Deliver audio/video, PDF/image applications, camera and printing with actual backends.
- [ ] **LATER-04** Expand hardware, GPU/multi-monitor, Wi-Fi, Bluetooth, laptop suspend/battery, localization and multi-user administration.
- [ ] **LATER-05** Qualify ARM64/server/edge, then mobile/tablet and separately scoped embedded/robotics/MCU products.

Use the [full roadmap](docs/complete-os-roadmap.md), [12 evidence-backed findings](docs/reviews/2026-09-29-complete-os-review.md), [208-app inventory](docs/reviews/2026-09-29-app-inventory.csv), and [80-subsystem map](docs/reviews/2026-09-29-subsystem-matrix.csv) as supporting references. This file controls R1 scope and priority if those broader documents describe a graphical first release.
