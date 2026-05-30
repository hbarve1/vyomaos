> **Status: Archive** — This is an early design exploration document. The implemented code is the source of truth. See [docs/INDEX.md](../../docs/INDEX.md) for current documentation.

# Feature Specification: VyomaOS Developer Tooling (vyoma CLI)

**Feature Branch**: `044-developer-tooling`

**Created**: 2026-05-25

**Status**: Draft

**Input**: User description: "VyomaOS Developer Tooling — a host-side CLI (`vyoma`) that makes developing, deploying, and monitoring WASM apps on a running VyomaOS instance as ergonomic as `kubectl` or `fly`. Primary commands: (1) `vyoma ps` — list running apps with status, uptime, restart count; (2) `vyoma logs <app>` — stream app logs from the running VM; (3) `vyoma push <app.wasm>` — OTA-push a new WASM binary to a running VM via the existing `@supervisor: ota-update` IPC; (4) `vyoma monitor` — live dashboard showing all app heartbeats, FPS, and resource usage updated every 2s; (5) `vyoma exec <app> <msg>` — send an IPC message to a running app and print the reply. The CLI communicates with VyomaOS over a TCP connection (port 9090) or via QEMU monitor socket. Out of scope: GUI, authentication, multi-VM management."

---

## User Scenarios & Testing *(mandatory)*

<!--
  User stories are prioritized as independent value slices.
  Each story can be developed, tested, and demonstrated independently.
-->

### User Story 1 - Deploy a New App Version to a Running VM Without SSH (Priority: P1)

A developer has just fixed a bug in their WASM app. Without entering the VM or restarting the system, they run `vyoma push my-app.wasm` from the host machine, and the running VM picks up the new binary, starts it alongside the A/B health check, and the developer sees confirmation that the update succeeded — all within 5 seconds.

**Why this priority**: The single biggest friction in developing for VyomaOS today is that updating a running app requires copying a binary into the data directory, entering the VM, and issuing an IPC command by hand. Removing that friction makes the inner development loop fast enough to be practical, which is a prerequisite for every other improvement.

**Independent Test**: Build two versions of a WASM app (v1 prints "hello v1", v2 prints "hello v2"). Run VyomaOS with v1 loaded. From the host, run `vyoma push v2.wasm`. Confirm the running VM transitions to v2 and prints "hello v2" without rebooting.

**Acceptance Scenarios**:

1. **Given** a VyomaOS instance reachable on port 9090, **When** the developer runs `vyoma push my-app.wasm`, **Then** the new binary is delivered to the VM, the OTA health check window starts, and the CLI prints the progress within 5 seconds of the command being issued.
2. **Given** a push in progress, **When** the new app version passes its health check, **Then** the CLI prints a success confirmation and exits with code 0.
3. **Given** a push in progress, **When** the new app version fails its health check (exits immediately), **Then** the VM automatically rolls back to the previous version, the CLI prints a rollback notification, and exits with a non-zero code.
4. **Given** the CLI cannot reach the VM on port 9090, **When** the developer runs `vyoma push`, **Then** the CLI prints a clear error message identifying the connectivity failure and exits with a non-zero code.
5. **Given** a push command is issued with an optional SHA-256 hash, **When** the received binary does not match the hash, **Then** the deployment is rejected before the health check starts and slot A is left unchanged.

---

### User Story 2 - Stream App Logs from the Host Without Entering the VM (Priority: P2)

A developer needs to debug an app that is producing unexpected output. They run `vyoma logs my-app` on the host and see a live stream of the app's stdout/stderr lines appear in their terminal, just as if they were tailing a log file. They can also run `vyoma ps` to quickly see which apps are running, their uptime, and how many times each has restarted.

**Why this priority**: Observability is the second most important developer loop feedback. Without log access from the host, developers must enter the VM or rely on the supervisor serial console, which is fragile and mixes all app output together.

**Independent Test**: Run a WASM app that prints a timestamped line every second. On the host, run `vyoma logs <app>`. Confirm that the lines appear on the host terminal within 2 seconds of being emitted inside the VM. Also run `vyoma ps` and confirm all running apps appear in the output with their uptime and restart count.

**Acceptance Scenarios**:

1. **Given** a running app that emits stdout lines, **When** the developer runs `vyoma logs <app>`, **Then** each line emitted by the app appears on the host terminal within 2 seconds of emission.
2. **Given** `vyoma logs` is running, **When** the app restarts, **Then** the log stream resumes automatically without the developer restarting the CLI.
3. **Given** a VyomaOS instance with 5 running apps, **When** the developer runs `vyoma ps`, **Then** the output lists all 5 apps with their name, status (running/stopped/degraded), uptime in human-readable form, and restart count.
4. **Given** the developer requests logs for an app that is not running, **When** the CLI connects to the VM, **Then** it prints a clear error and exits rather than hanging.

---

### User Story 3 - Live Fleet Health Dashboard (Priority: P3)

An operator needs to verify that all apps on a running VyomaOS instance are healthy after a deployment. They run `vyoma monitor` and see a terminal dashboard that refreshes every 2 seconds, showing each app's heartbeat status, memory usage, and uptime — without writing a custom monitoring script.

**Why this priority**: While logs and push are critical for individual developer workflows, the monitor command addresses the operator use case of confirming overall system health at a glance. It consumes the existing heartbeat JSON-line stream (documented in `docs/observability.md`) and presents it in a human-readable form.

**Independent Test**: Run VyomaOS with 3 or more apps. Run `vyoma monitor` on the host. Confirm the dashboard shows all running apps. Mark one app as degraded (force a watchdog timeout). Confirm the dashboard reflects the status change within the next 2-second refresh cycle.

**Acceptance Scenarios**:

1. **Given** a running VyomaOS instance with multiple apps, **When** the developer runs `vyoma monitor`, **Then** the terminal displays a dashboard with one row per app showing: name, status (healthy/degraded/unhealthy), uptime, memory usage (KB), and time since last heartbeat.
2. **Given** the dashboard is running, **When** an app's heartbeat status changes from `healthy` to `degraded`, **Then** the dashboard reflects this within the next 2-second refresh cycle.
3. **Given** the dashboard is running, **When** a new app starts or an existing app stops, **Then** the row count updates at the next refresh cycle without the user restarting the CLI.
4. **Given** the dashboard is running, **When** the user presses `q` or `Ctrl+C`, **Then** the CLI exits cleanly and restores the terminal to its prior state.

---

### User Story 4 - Ad-hoc IPC Debugging (Priority: P4)

A developer is debugging an inter-app protocol and wants to send a raw IPC message to a running app from the host — without writing a temporary WASM app to do it. They run `vyoma exec pong @pong: hello` and see the app's reply printed in their terminal within 1 second.

**Why this priority**: IPC debugging today requires either writing a throw-away WASM app or modifying the shell app inside the VM. The `exec` command reduces ad-hoc protocol testing to a single CLI invocation, completing the developer tooling story without requiring GUI or auth work.

**Independent Test**: Run the built-in `pong` app. From the host, run `vyoma exec pong "hello"`. Confirm the reply message appears in the host terminal. Then send a message to a non-existent app and confirm the CLI returns an error rather than hanging.

**Acceptance Scenarios**:

1. **Given** a running app that responds to IPC messages, **When** the developer runs `vyoma exec <app> "<message>"`, **Then** the message is delivered to the app via the supervisor IPC broker and the app's reply appears in the host terminal within 2 seconds.
2. **Given** a `vyoma exec` command, **When** the target app does not respond within a configurable timeout (default 5 s), **Then** the CLI prints a timeout error and exits with a non-zero code.
3. **Given** a `vyoma exec` command targeting an app that is not running, **When** the command is issued, **Then** the CLI prints an error stating the app is not running and exits immediately.

---

### Edge Cases

- What happens when the VM is reachable but the supervisor TCP listener on port 9090 has not started yet? The CLI retries for up to 10 seconds with exponential backoff and then reports a connection failure.
- What happens when a `vyoma push` binary is larger than available VM disk space? The supervisor rejects the transfer before writing slot B; the CLI reports the error.
- What happens when `vyoma logs <app>` is running and the VM is rebooted mid-stream? The CLI detects the connection drop, prints a reconnection notice, and attempts to reconnect.
- What happens when two developers simultaneously issue `vyoma push` for the same app? The supervisor processes requests sequentially; the second push waits or is rejected with an "OTA in progress" message.
- What happens if `vyoma monitor` is run against a VM with the observability tier set to `baseline` (no heartbeat export URL)? The CLI reads heartbeats from the supervisor's stderr stream via the same TCP connection; no separate endpoint is required.
- What happens when the WASM binary passed to `vyoma push` is corrupt or not a valid WASM file? The supervisor's existing parse validation rejects it before the health check starts.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST provide a `vyoma ps` command that lists all running apps on the connected VM, showing for each: app name, status (running/stopped/degraded/unhealthy), uptime in human-readable form, and restart count.
- **FR-002**: The CLI MUST provide a `vyoma logs <app>` command that streams live stdout and stderr output from the named app to the host terminal with a latency of no more than 2 seconds per line.
- **FR-003**: The `vyoma logs` command MUST automatically resume streaming if the target app restarts, without requiring the developer to re-run the command.
- **FR-004**: The CLI MUST provide a `vyoma push <path/to/app.wasm>` command that transfers a WASM binary to the connected VM and triggers the supervisor's existing `ota-update` IPC command (`@supervisor: ota-update <name> <path>`) to initiate an A/B slot update.
- **FR-005**: The `vyoma push` command MUST accept an optional `--name <app-name>` flag to identify the target app; when omitted, the app name MUST be inferred from the binary filename (without extension).
- **FR-006**: The `vyoma push` command MUST accept an optional `--sha256 <hash>` flag to pass an expected hash to the supervisor for binary verification; when provided, deployment MUST be rejected if the hash does not match.
- **FR-007**: The `vyoma push` command MUST display progress during the health check window (minimum: a status line updated every 10 seconds showing elapsed time and current health state) and exit with code 0 on successful commit or non-zero on rollback.
- **FR-008**: The CLI MUST provide a `vyoma monitor` command that displays a terminal dashboard showing one row per running app, refreshed every 2 seconds, with columns: app name, status, uptime, memory usage (KB), and seconds since last heartbeat.
- **FR-009**: The `vyoma monitor` dashboard MUST reflect app status changes (healthy → degraded → unhealthy) within one 2-second refresh cycle of the supervisor emitting the updated heartbeat.
- **FR-010**: The `vyoma monitor` dashboard MUST update the app list (adding new rows or removing rows) when apps start or stop, without requiring a CLI restart.
- **FR-011**: The CLI MUST provide a `vyoma exec <app> "<message>"` command that sends the message to the named app via the supervisor IPC broker and prints the app's reply to stdout.
- **FR-012**: The `vyoma exec` command MUST time out and exit with a non-zero code if no reply is received within a configurable duration (default 5 seconds, overridable via `--timeout <secs>`).
- **FR-013**: The CLI MUST support connecting to a VyomaOS instance via TCP on a configurable host and port (default: `localhost:9090`), overridable with `--host <addr>` and `--port <n>` flags or a `VYOMA_HOST` and `VYOMA_PORT` environment variable.
- **FR-014**: The CLI MUST support connecting via a QEMU monitor socket path as an alternative transport, specified via `--socket <path>` or the `VYOMA_SOCKET` environment variable.
- **FR-015**: All CLI commands MUST exit with code 0 on success and a non-zero code on any error; error messages MUST be written to stderr and MUST identify the cause (connection failure, app not found, timeout, hash mismatch, OTA in progress).
- **FR-016**: The CLI MUST operate as a single statically-linked binary with no runtime dependencies required on the host machine beyond a POSIX-compatible operating system (Linux and macOS).

### Key Entities

- **VyomaConnection**: A handle to a connected VyomaOS instance. Encapsulates the transport layer (TCP or QEMU socket), the target address, and connection state. Shared across all commands within a single invocation.
- **AppHandle**: A representation of a running app as seen from the CLI — name, status, uptime, restart count, and memory usage. Populated from the supervisor's `ps`-equivalent IPC response and from heartbeat JSON lines.
- **HeartbeatStream**: A continuous sequence of JSON-line heartbeat records received from the supervisor. Consumed by `vyoma monitor` and used to populate the `AppHandle` status fields in real time.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `vyoma push` delivers a new WASM binary to a running VM and begins the OTA health check within 5 seconds of the command being issued on the host, measured from command invocation to the first health-check progress line appearing in the terminal.
- **SC-002**: `vyoma logs` displays each app log line within 2 seconds of the line being emitted inside the VM, measured end-to-end from the app's write to the host terminal display.
- **SC-003**: `vyoma ps` returns a complete, accurate list of all running apps within 1 second of invocation on a VM with up to 20 concurrent apps.
- **SC-004**: `vyoma monitor` reflects an app status change (e.g., healthy → degraded) within 4 seconds (one 2-second refresh cycle after the supervisor emits the updated heartbeat plus up to 2 seconds of network latency).
- **SC-005**: `vyoma exec` delivers a message and prints the reply within 2 seconds for apps that respond synchronously, measured end-to-end from CLI invocation.
- **SC-006**: A developer unfamiliar with VyomaOS internals can install the CLI, connect to a running VM, and deploy an updated app binary within 10 minutes following the quickstart documentation.
- **SC-007**: All 5 commands (`ps`, `logs`, `push`, `monitor`, `exec`) function correctly with both TCP (port 9090) and QEMU socket transports, verified by running the same acceptance scenarios against both connection types.
- **SC-008**: The CLI binary is no larger than 10 MB on Linux x86-64 and no larger than 10 MB on macOS ARM64, ensuring fast distribution and installation.

## Assumptions

- The VyomaOS supervisor exposes a TCP listener on port 9090 (or a QEMU monitor socket) that accepts IPC commands and returns structured responses; this listener is part of the supervisor's networking subsystem and is already specified in `docs/ota-updates.md` and `docs/observability.md`.
- The supervisor's `ota-update` IPC command (`@supervisor: ota-update <name> <path>`) is the only API surface for OTA deployments; the CLI wraps this command rather than implementing a new update path.
- Heartbeats emitted by the supervisor follow the JSON-line format defined in `docs/observability.md` (`type`, `module`, `uptime_s`, `mem_kb`, `status`, `last_error`); the CLI does not need to interpret any additional fields.
- The host machine running the CLI has network connectivity to the VyomaOS instance on port 9090, or can access the QEMU monitor socket directly (local development case).
- Authentication is out of scope for v1; the TCP listener accepts connections from any host without credentials.
- Multi-VM management (connecting to and switching between multiple VyomaOS instances in a single CLI session) is out of scope for v1; one CLI invocation connects to one VM.
- A graphical user interface for the CLI is out of scope; the terminal dashboard in `vyoma monitor` is the extent of the UI.
- Windows host support is out of scope for v1; the CLI targets Linux and macOS hosts.
- The CLI does not need to build or compile WASM apps; it only transfers pre-built binaries.
- The `vyoma push` command assumes the binary is already built by the developer and available as a local file path on the host.
