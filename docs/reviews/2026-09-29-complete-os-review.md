# VyomaOS: readiness review for a complete OS

Date: 2026-09-29. Source baseline: `418359d3` on local `develop`.

**Scope update after this review:** the first iteration is a terminal-first desktop OS. [TODOS.md](../../TODOS.md) defines R1 acceptance and priority. Findings below retain the broader complete-OS review; graphical browser/media gaps apply to the later graphical release, while security, persistence, real console operation and recovery remain R1 requirements.

## Assessment

VyomaOS has a substantial desktop prototype and a useful WASM application model. It does not yet have the installation, authority boundaries, durable storage, trustworthy updates, hardware validation, or application compatibility needed for a complete daily-use OS. The next investment should close these foundations and prove a small set of real user workflows. Adding more application names will not close those gaps.

Keep Linux for hardware and low-level OS mechanisms, Rust for trusted services, and WASM for the application platform. Preserve the existing compositor and apps as migration inputs. Do not restart the project or implement all 80 design chapters simultaneously.

The proposed execution plan is [complete-os-roadmap.md](../complete-os-roadmap.md). It preserves the long-term desktop/mobile/embedded ambition while defining a first desktop release with explicit acceptance gates.

## Scope and evidence limits

This is a repository-wide architecture and delivery review, not a line-by-line audit of every application or a penetration test. Work covered the build/CI chain, boot and launch paths, IPC dispatch, security policy, management API, persistence, installation/update paths, rendering, input, representative applications, host CLI, website organization, tests, and existing roadmaps. All app crates/manifests and the 80 design subsystems were inventoried; individual app functionality was sampled.

No clean Docker image build, current-image QEMU boot, hardware test, browser compatibility test, or adversarial guest execution was performed. Source-derived findings below are distinguished from executed checks. Existing `out/` artifacts were not treated as proof for this commit. No runtime source was changed by this review.

### Repository inventory

| Area | Observed inventory | Interpretation |
|---|---|---|
| Supervisor | 126 Rust source files; 56 files under `supervisor/tests/` | Broad service implementation; test-file placement does not prove live integration |
| Applications | 208 Cargo crates and 208 `vyoma.toml` files; 224 Rust source files | Portfolio includes utilities, games, demos, and overlapping app implementations |
| Default boot | `desktop`, `dock`, `clock` in `base/modules/scripts/boot.toml` | 208 available app directories do not mean 208 running applications |
| Capabilities | 208 stdio, 201 display, 104 shell, 43 filesystem, 14 mouse, 7 network declarations | Declarations are inventory data, not proof of enforcement or input completeness |
| Platform profiles | 11 TOML profiles | More profile names than documented, without corresponding validated hardware ports |
| Host tooling | CLI and manifest/GUI tools | CLI has a verified compile failure |
| Website | Separate Next.js 16 / React 19 application and Markdown documentation copy | Useful distribution surface; not evidence for OS runtime readiness |

See [all-app inventory](2026-09-29-app-inventory.csv) and [80-subsystem mapping](2026-09-29-subsystem-matrix.csv). Inventory flags are search hints, not automatic quality judgments.

### Checks executed

| Check | Result | Limit |
|---|---|---|
| `RUSTFLAGS='-D warnings' cargo test --offline --locked -p supervisor --target x86_64-unknown-linux-musl` | Blocked during `ring` compilation: missing `x86_64-linux-musl-gcc` | Environment/toolchain blocker; production tests did not run |
| `cargo test --offline --locked -p supervisor --target x86_64-unknown-linux-gnu --no-run` | Passed; unused `focused` warning at `supervisor/src/draw_cmd/mod.rs:31` | Compiles tests, does not execute them; warning would conflict with a warning-as-error build |
| Same host target, `--lib` | 11 passed | Small library-unit subset |
| Same host target, eight named test binaries | 125 passed | `manifest_tests`, `lifecycle_tests`, `compositor_test`, `surface_test`, `platform_profile`, `peripheral_capability`, `ipc_tests`, `window_layout_test`; logic coverage, not OS acceptance |
| `cargo test --offline --locked -p vyoma --no-run` | Failed: `Arg::env` unavailable at `cli/src/main.rs:29` | `cli/Cargo.toml:11` enables Clap `derive,cargo`, not `env` |
| `python3 -m pytest tests/e2e/test_harness.py tests/e2e/test_assertions.py -q -p no:cacheprovider` | Blocked: `No module named pytest` | No Python tests executed |
| Python TOML parse over app manifests | 208 parsed | Syntax parsing only; does not replace the Rust schema validator |

Cargo also warns that release profiles in member manifests are ignored by the root workspace. No test results from a remote CI run were inspected. The total executed Rust test count here is **136**, not the complete repository suite.

## Release-blocking findings

Severity refers to the proposed general-purpose release, not a claim that this research checkout is deployed. P0 blocks handling untrusted apps or user secrets; P1 blocks a dependable desktop release.

### F01 — P0: privileged IPC bypasses the intended capability boundary

[`router.rs:67`](../../supervisor/src/router.rs) forwards every app's `@supervisor:` message directly to the command handler. [`ipc_handlers/mod.rs:25`](../../supervisor/src/ipc_handlers/mod.rs) has no central authorization check and records commands as allowed before dispatch. [`ipc_commands/mod.rs:58`](../../supervisor/src/ipc_commands/mod.rs) exposes network operations; [`network_cmd.rs:33`](../../supervisor/src/ipc_commands/network_cmd.rs) performs HTTP requests, and its download path writes a caller-supplied destination at line 81 without checking the caller's filesystem/network rights.

An app denied direct WASI network access can still ask the privileged supervisor to make a request. Path-based file operations, process management, screenshots, clipboard, installation, and power operations need the same review. WASM memory isolation does not authorize host services on the app's behalf. This is a source-traced confused-deputy path; no exploit was executed. Wasmtime's own boundary is the interfaces made available to the guest. [Wasmtime security model](https://docs.wasmtime.dev/security.html).

Fix: bind an unforgeable app-instance identity and effective grants to every request; deny by default at one policy layer before side effects; apply it to both legacy IPC and future typed APIs. Carry sender identity through inter-app delivery and use request IDs instead of global last-sender reply state. Test denials through a real malicious WASM app, including brokered and direct operations.

### F02 — P0: filesystem grants expose shared OS and user state

[`app_threads.rs:333`](../../supervisor/src/app_threads.rs) gives every filesystem-enabled app the entire `/data` preopen. Accounts, app installations, audit logs, settings, and update state also live under `/data`. The namespace setup is explicitly disabled at line 345. This does not remove Wasmtime's sandbox; it makes the granted directory much broader than the intended per-app/per-user model.

Fix: separate protected system state, per-user documents, and private app containers; use explicit document grants. Enforce equivalent restrictions in brokered file operations, with symlink/traversal/race tests. Add process-level containment as defense in depth and prove that runtime libraries remain accessible without exposing system data.

### F03 — P0: management API is unauthenticated and accepts unbounded input

[`main.rs:350`](../../supervisor/src/main.rs) starts a listener on `0.0.0.0:9090`. [`mgmt_handlers.rs:42`](../../supervisor/src/system/mgmt_handlers.rs) dispatches requests without authentication. At line 190, upload size directly determines a `Vec` allocation, and the supplied name is interpolated into a filesystem path. The initial request line also has no visible size limit. Reachability depends on VM/host networking; the guest service itself has no access-control boundary.

Fix: disable remote administration in release images by default; provide an authenticated, authorized development transport; bound request sizes, transfers, clients and timeouts; validate bundle IDs and stage through safe paths. Test unauthenticated, oversized, slow, and traversal-bearing requests against the real service.

### F04 — P0: secret storage and authentication are prototype mechanisms

[`encrypted_store.rs:27`](../../supervisor/src/security/encrypted_store.rs) explicitly implements repeating-key XOR; [`apps/password-manager/src/main.rs:40`](../../apps/password-manager/src/main.rs) uses the same class of construction. [`user.rs:53`](../../supervisor/src/security/user.rs) hashes PINs with SHA-256 and supplies a default admin PIN `0000` at line 90. These cannot support production secret-storage claims.

The IPC handler logs complete command strings before dispatch, while encryption/login-related commands can contain secrets. [`audit.rs:76`](../../supervisor/src/security/audit.rs) persists audit events in `/data/audit.log`, within the broad filesystem preopen from F02.

Fix: remove secret-handling demos from the release set until replaced; redact structured audit fields before persistence; use a reviewed authenticated-encryption implementation, a salted password KDF, rate limiting, and first-boot credential enrollment. Separate keychain encryption from full-disk encryption. Define recovery and key rotation, including a safe migration story for any existing prototype files. [Authenticated encryption reference](https://doc.libsodium.org/secret-key_cryptography/aead), [Argon2 specification](https://www.rfc-editor.org/info/rfc9106/).

### F05 — P1: update push reports success without deploying

[`mgmt_handlers.rs:219`](../../supervisor/src/system/mgmt_handlers.rs) simulates a health check, returns “committed to slot B,” and deletes the incoming file. It does not activate the uploaded app. [`atomic_update.rs:201`](../../supervisor/src/system/atomic_update.rs) changes slot metadata; the inspected QEMU boot path still uses a fixed kernel/initramfs. Neither is proof of reboot-safe OS update selection.

Fix: separate app transactions from whole-OS boot slots; report success only after real activation and health validation. Connect OS slots to the bootloader, boot-attempt counters and rollback. Include durable writes and parent-directory synchronization, data-schema compatibility, interrupted downloads, crashes during each transaction phase, and reboot after rollback.

### F06 — P1: hashes are being treated as signatures and secure boot

[`verify.rs:17`](../../supervisor/src/verify.rs) checks a digest supplied by a manifest; [`app_threads.rs:310`](../../supervisor/src/app_threads.rs) makes this check optional. [`secure_boot.rs:112`](../../supervisor/src/security/secure_boot.rs) treats absent verification metadata as verified in advisory mode. A hash alongside a modifiable payload does not establish a trusted publisher; a running supervisor inspecting itself does not establish firmware-to-kernel trust.

Fix: sign package content plus authority-bearing manifests against provisioned trust roots; add key rotation, revocation and downgrade policy. Establish a separate UEFI boot trust chain and explicit development mode. Adopt a reviewed update metadata design covering rollback/freeze protection. [TUF specification](https://theupdateframework.io/spec/).

### F07 — P1: installation and persistence stop at the VM boundary

[`installer.rs:185`](../../supervisor/src/system/installer.rs) logs formatting and returns success; bootloader installation at line 202 is a placeholder. [`mount.rs:19`](../../supervisor/src/system/mount.rs) attempts the host's 9P share and falls back to tmpfs. Attaching an ext4 disk alone does not make this mount path persistent.

Fix: implement and test installation to disposable disk images first; boot through UEFI from that disk without `-kernel` or host shares; mount persistent state by stable identifier. When required storage is unavailable, enter an explicit recovery/live mode rather than presenting a normal persistent desktop. Prove create-save-reboot-open, disk-full handling, backups, restore, and power-loss recovery.

### F08 — P1: CI and test assertions can overstate readiness

The checked-in [CI workflow](../../.github/workflows/ci.yml) builds rootfs then invokes smoke without building/restoring a kernel. [`smoke-test.sh:12`](../../base/scripts/smoke-test.sh) requires `out/bzImage`. `Makefile:216` pipes profile tests to `tail` then prints success, masking failures. `Makefile:347` changes into `tests/e2e`, while fixture defaults resolve `out/` relative to the current directory and skip if images are absent.

[`test_input.py:38`](../../tests/e2e/test_input.py) accepts any nonempty serial log after input; boot output alone can satisfy it. Window tests accept nonblack regions. [`router_test.rs`](../../supervisor/tests/router_test.rs) copies routing logic instead of invoking the production router. [`mgmt_push.rs`](../../supervisor/tests/mgmt_push.rs) uses a hand-written reply server, so it does not catch F05. Runtime adapter tests use `/bin/true` in place of Wasmtime.

Fix: make clean-checkout build → boot → interaction a required gate, fail on missing expected artifacts/skipped mandatory cases, and exercise the actual service implementations. Retain useful logic tests, but do not count mock-protocol success as integration readiness. Archive serial/QEMU stderr, screenshots, checksums and environment versions.

### F09 — P1: resource isolation and runtime architecture are unfinished

The live path uses direct Wasmtime subprocesses, unbounded `mpsc::channel` inboxes at [`app_threads.rs:306`](../../supervisor/src/app_threads.rs), and line-oriented output. It does not apply the design's explicit per-app CPU/memory/I/O budgets. The separate runtime adapter ignores its capability argument at [`runtime/wasmtime.rs:95`](../../supervisor/src/runtime/wasmtime.rs); its unchecked trait-object cast at line 121 also needs replacement before adoption.

Fix: bound messages, queues, logs, surfaces and concurrent operations; isolate a busy or crashing app from PID 1 and the desktop. Prototype a runtime worker process embedding Wasmtime for typed imports and limits. Decide process topology using crash, memory and latency measurements. Do not move every guest into PID 1 simply because the master spec proposes an embedded runtime. [Wasmtime resource/interruption configuration](https://docs.wasmtime.dev/api/wasmtime/struct.Config.html).

### F10 — P1: platform profiles are ahead of platform implementations

[`platforms/iot-rpi/Makefile`](../../platforms/iot-rpi/Makefile), the MCU Makefile, and ARM64 rootfs scripts contain explicit stubs. The root Makefile uses a different platform naming scheme from runtime profiles. [`kernel.sh:16`](../../base/modules/kernel.sh) hardcodes the base config and builds an x86 `bzImage`; its existence check at line 8 can skip changed inputs. The live launcher always selects `/usr/bin/wasmtime`.

Fix: validate one x86-64 desktop end to end, then one ARM64 Linux target. Make unknown/unsupported targets fail explicitly. Document artifact, ABI, driver, resource and test support for every target. A WASIp2 application requires a component-capable runtime; do not promise identical desktop binaries on MCU interpreters without a demonstrated compatibility path. [Rust target requirements](https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip2.html).

### F11 — P1: daily-use applications and hardware services need real backends

[`audio.rs:7`](../../supervisor/src/audio.rs) states there is no audio-device interaction. [`apps/browser/src/main.rs:44`](../../apps/browser/src/main.rs) strips HTML to text rather than providing a general web engine. Accessibility code provides preferences/announcements, not evidence of an end-to-end semantic accessibility tree. Many apps hardcode dimensions and duplicate drawing/input helpers. Multiple apps with similar names are not substitute implementations of system contracts.

Fix: select a maintained core app set and shared SDK; implement document lifecycle, dialogs, focus, Unicode/text composition, scaling and accessibility together. Make modern browser feasibility, real audio/video and hardware drivers explicit delivery workstreams. Verify required workflows with real files, network services and devices. Do not declare unsupported web or media workflows complete.

### F12 — P1: build provenance and maintenance are incomplete

[`base/config.sh`](../../base/config.sh) pins Linux `5.10.113`; the Dockerfile installs floating Rust `stable` and uses a mutable base tag. The kernel download helper has no integrity check in the inspected path. Workspace release profiles are ignored, and rootfs assembly silently skips missing app binaries at [`rootfs.sh:143`](../../base/modules/rootfs.sh). WASM output alone does not guarantee reproducible builds.

Fix: select a maintained kernel branch, pin the complete toolchain/dependency/artifact inputs, validate download provenance, use release manifests and locked dependency resolution, and fail image assembly for missing selected apps. Track kernel/runtime security advisories and publish a patch policy, SBOM, source/build provenance and restore instructions. Choose versions at implementation time from [upstream kernel maintenance information](https://www.kernel.org/releases.html).

## Existing-plan review

| Plan | Keep | Revise |
|---|---|---|
| [Original implementation tracker](../../.context/plans/plan-vyomaos/README.md) | Foundation history and completed work references | Phase status is historical, not a current release checklist |
| [P31–P112 roadmap](../superpowers/specs/2026-05-20-vyomaos-full-os-roadmap.md) | Broad coverage of apps, hardware, recovery, accessibility | Move security/persistence/install/recovery early; distinguish hash checks from signatures; replace feature names with observable outcomes |
| [80-subsystem master spec](../superpowers/specs/desktop-os-vision/master-spec.md) | Identity, typed interfaces, bounded work, resource limits, document model, platform breadth | Treat as design input. Reconcile embedded-runtime vs subprocess isolation, pointer/shared-memory assumptions, raw-ioctl/dependency policy, global locks vs actors, and competing lifecycle models through tested ADRs |
| [Design state tracker](../superpowers/specs/desktop-os-vision/state.json) | Record of the design/debate exercise | `completed` means design round completed, not code implemented, integrated, or accepted |
| [June test plan](../superpowers/plans/2026-06-01-test-strategy.md) | Real integration harness, adversarial input, regression testing | Refresh module counts; prioritize production-path tests; remove test-count ratios and hour estimates as completion proxies |
| [Testing strategy](../testing-strategy.md) | Layered logic/integration/boot/hardware validation | Separate proposed platform tests from passing jobs; make fault recovery and data preservation release gates |

There are also direct documentation contradictions: `AGENTS.md` still reports a duplicate tool `[workspace]` and the old supervisor binary location, but both are corrected in current source. README/CLAUDE/tracker phase numbers and app totals disagree. Establish one release ledger that generates public feature status, while preserving old designs as archives.

## Recommended decision

Proceed with a Linux-based, WASM-first desktop on one supported hardware configuration. Deliver a secure, persistent, installable system with a small proven application set, then expand compatibility and device classes. Treat web compatibility, native system-service policy, hardware selection and team capacity as explicit decisions. The new roadmap defines the dependency order, acceptance gates, first implementation tickets, and coverage of the full vision.
