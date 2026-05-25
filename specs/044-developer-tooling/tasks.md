# Tasks: VyomaOS Developer Tooling (vyoma CLI)

**Input**: Design documents from `/specs/044-developer-tooling/`

**Prerequisites**: plan.md ✅ spec.md ✅ research.md ✅ data-model.md ✅ contracts/ ✅ quickstart.md ✅

**Organization**: Tasks grouped by user story for independent implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1=push, US2=logs+ps, US3=monitor, US4=exec)

---

## Phase 1: Setup (Project Initialization)

**Purpose**: Create the CLI workspace and shared protocol types that all subsequent work depends on.

- [ ] T001 Add `"cli"` to `members` array in root `Cargo.toml` workspace
- [ ] T002 Create `cli/Cargo.toml` with dependencies: `clap = { version = "4", features = ["derive", "cargo"] }`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `crossterm = "0.27"`, `indicatif = "0.17"` — target `x86_64-unknown-linux-musl` in `[profile.release]` with `lto = true`, `strip = true`
- [ ] T003 [P] Create `cli/src/main.rs` with clap `#[derive(Parser)]` struct: global flags `--host` (default `localhost`), `--port` (default `9090`), `--socket` (optional), subcommand enum `Commands { Ps, Logs { app: String }, Push { path: PathBuf, name: Option<String>, sha256: Option<String> }, Monitor, Exec { app: String, message: String, timeout: u64 } }` — each arm calls a `todo!()` placeholder
- [ ] T004 [P] Create `supervisor/src/mgmt_protocol.rs` — empty module with `pub mod mgmt_protocol;` placeholder; add `mod mgmt_protocol;` to `supervisor/src/main.rs`

**Checkpoint**: `cargo check` passes for both supervisor and cli crates.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core NDJSON types, TCP transport, and management server skeleton. All user story work depends on this phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T005 [P] Implement `MgmtRequest` enum in `supervisor/src/mgmt_protocol.rs` — serde `#[serde(tag = "type", rename_all = "snake_case")]` variants: `Ps`, `Logs { app: String }`, `PushStart { name: String, size: u64, sha256: Option<String> }`, `HeartbeatStream`, `Exec { app: String, msg: String, timeout_ms: Option<u64> }`
- [ ] T006 [P] Implement `MgmtResponse` enum in `supervisor/src/mgmt_protocol.rs` — same serde tag pattern, variants: `PsRow { name, status, uptime_s, restarts, mem_kb }`, `PsDone`, `LogLine { app, line }`, `PushAck { state, elapsed_s, health_status: Option<String> }`, `PushResult { ok: bool, message: String }`, `Heartbeat { module, uptime_s, mem_kb, status, last_error }`, `ExecReply { reply: String }`, `Error { code: String, message: String }` — plus `AppStatus` enum (`running/stopped/degraded/unhealthy`)
- [ ] T007 [P] Create `cli/src/protocol.rs` — mirror of supervisor's types but in the CLI crate: same `MgmtRequest` and `MgmtResponse` enums with identical serde attributes so JSON round-trips correctly
- [ ] T008 Create `cli/src/connection.rs` — `VyomaConnection` struct with `reader: BufReader<Box<dyn Read + Send>>` and `writer: Box<dyn Write + Send>`. Implement `VyomaConnection::connect(host: &str, port: u16) -> Result<Self>` (TCP, `TcpStream::connect_timeout` 10s, exponential backoff up to 10s on connection refused) and `VyomaConnection::connect_socket(path: &Path) -> Result<Self>` (UnixStream). Implement `send(&mut self, req: &MgmtRequest) -> Result<()>` (serialize to NDJSON line) and `recv(&mut self) -> Result<MgmtResponse>` (read one line, deserialize)
- [ ] T009 Create `supervisor/src/mgmt_server.rs` — `MgmtServer` struct holding `Arc<Mutex<AppRegistry>>`. `MgmtServer::new(registry: Arc<Mutex<AppRegistry>>) -> Self`. `MgmtServer::start(self, addr: SocketAddr)` — `TcpListener::bind(addr)?`, then `loop { let (stream, _) = listener.accept()?; let reg = Arc::clone(&self.registry); thread::spawn(move || handle_client(stream, reg)); }`
- [ ] T010 Create `supervisor/src/mgmt_handlers.rs` — `handle_client(stream: TcpStream, registry: Arc<Mutex<AppRegistry>>)` that reads one NDJSON line, deserializes to `MgmtRequest`, dispatches to stub handler functions: `handle_ps`, `handle_logs`, `handle_push`, `handle_heartbeat_stream`, `handle_exec` — each stub writes `MgmtResponse::Error { code: "NOT_IMPLEMENTED".into(), message: "...".into() }` and returns
- [ ] T011 Wire `MgmtServer` into `supervisor/src/main.rs` — after `AppRegistry` is initialized, call `MgmtServer::new(Arc::clone(&registry)).start("0.0.0.0:9090".parse().unwrap())` on a dedicated thread via `thread::spawn`; add `mod mgmt_server;` and `mod mgmt_handlers;` declarations

**Checkpoint**: `cargo build` succeeds for both supervisor and cli. Supervisor starts and binds port 9090 (verify with `telnet localhost 9090`).

---

## Phase 3: User Story 1 — OTA Push (Priority: P1) 🎯 MVP

**Goal**: `vyoma push <path.wasm>` transfers a WASM binary to the running VM, triggers the OTA A/B slot update, streams health-check progress, and exits 0 on commit or non-zero on rollback.

**Independent Test**: Build two WASM versions (v1 prints "hello v1", v2 prints "hello v2"). Run VyomaOS with v1. Run `vyoma push v2.wasm`. Confirm VM transitions to v2 and CLI exits 0. See `quickstart.md` US1 scenario.

- [ ] T012 [P] [US1] Write failing unit test in `supervisor/tests/mgmt_push.rs` — `test_push_handler_commits()`: open loopback TCP connection to a `MgmtServer` running on a random port with a stub `AppRegistry`; send `{"type":"push_start","name":"test-app","size":4,"sha256":null}` followed by 4 raw bytes `\x00asm`; assert responses include at least one `push_ack` line and a final `push_result` with `ok: true`. Run `cargo test` to confirm it FAILS before implementation.
- [ ] T013 [US1] Implement `handle_push` in `supervisor/src/mgmt_handlers.rs` — (1) read `size` bytes from the stream into a temp file `/data/ota/<name>-incoming.wasm`; (2) if `sha256` present, compute SHA-256 of temp file, send `MgmtResponse::Error { code: "HASH_MISMATCH" }` and return if mismatch; (3) call `OtaManager::initiate_update(&name, &temp_path)` which copies to slot B and starts health check; (4) in a loop, send `PushAck { state: "health_check", elapsed_s, health_status }` every 10 seconds; (5) on health check completion send `PushResult { ok, message }`. If another push is in progress, send `Error { code: "OTA_IN_PROGRESS" }`.
- [ ] T014 [US1] Make `supervisor/tests/mgmt_push.rs` pass — run `cargo test test_push_handler_commits` and fix until green.
- [ ] T015 [P] [US1] Create `cli/src/commands/push.rs` — `pub fn run(conn: &mut VyomaConnection, path: &Path, name: &str, sha256: Option<&str>) -> anyhow::Result<()>`. (1) read file bytes; (2) send `MgmtRequest::PushStart { name, size, sha256 }`; (3) stream `size` bytes over the connection writer; (4) loop reading `MgmtResponse`: on `PushAck` print progress line `[{elapsed}s] ...`; on `PushResult { ok: true }` print `SUCCESS: ...` and return `Ok(())`; on `PushResult { ok: false }` print `ROLLBACK: ...` and return `Err()`; on `Error` print to stderr and return `Err()`.
- [ ] T016 [US1] Wire `Commands::Push` arm in `cli/src/main.rs` — resolve app name (from `--name` or filename stem), call `commands::push::run(&mut conn, &path, &name, sha256.as_deref())?`

**Checkpoint**: `vyoma push apps/notes/target/wasm32-wasip2/release/notes.wasm` against a running VyomaOS VM transfers binary, prints health-check progress lines, and exits 0. SC-001 verified (first progress line within 5s).

---

## Phase 4: User Story 2 — Logs & ps (Priority: P2)

**Goal**: `vyoma ps` lists all running apps with status/uptime/restarts; `vyoma logs <app>` streams live stdout from a named app, auto-resuming on restart.

**Independent Test**: Run an app that emits a timestamped line every second. Run `vyoma logs <app>` — verify lines appear within 2s. Run `vyoma ps` — verify all running apps appear. See `quickstart.md` US2 scenario.

- [ ] T017 [P] [US2] Write failing unit test in `supervisor/tests/mgmt_ps.rs` — `test_ps_returns_rows()`: spin up `MgmtServer` with a fake `AppRegistry` containing 2 apps; send `{"type":"ps"}`; assert responses contain exactly 2 `ps_row` lines followed by one `ps_done`. Run `cargo test` to confirm FAIL.
- [ ] T018 [P] [US2] Write failing unit test in `supervisor/tests/mgmt_logs.rs` — `test_logs_streams_lines()`: register a fake app that has 3 pre-buffered log lines; send `{"type":"logs","app":"fake-app"}`; assert 3 `log_line` responses arrive before client disconnects. Run `cargo test` to confirm FAIL.
- [ ] T019 [P] [US2] Implement `handle_ps` in `supervisor/src/mgmt_handlers.rs` — acquire lock on `AppRegistry`, iterate over all entries, for each send `MgmtResponse::PsRow { name, status, uptime_s, restarts, mem_kb }`, then send `MgmtResponse::PsDone`. Release lock before sending.
- [ ] T020 [P] [US2] Make `supervisor/tests/mgmt_ps.rs` pass — run `cargo test test_ps_returns_rows` and fix until green.
- [ ] T021 [US2] Implement `handle_logs` in `supervisor/src/mgmt_handlers.rs` — look up app in registry; if missing send `Error { code: "NOT_FOUND" }` and return; otherwise subscribe to the app's log channel (add a `Vec<Sender<String>>` log broadcast to `AppEntry`); in a loop read from the channel and send `MgmtResponse::LogLine { app, line }`. On app restart the channel resets; loop continues because the subscription is re-established on reconnect.
- [ ] T022 [US2] Make `supervisor/tests/mgmt_logs.rs` pass — run `cargo test test_logs_streams_lines` and fix until green.
- [ ] T023 [P] [US2] Create `cli/src/commands/ps.rs` — `pub fn run(conn: &mut VyomaConnection) -> anyhow::Result<()>`. Send `MgmtRequest::Ps`. Collect `PsRow` responses until `PsDone`. Print header `NAME          STATUS     UPTIME       RESTARTS` then one formatted row per app using `format_uptime(uptime_s)` helper. Exit 0.
- [ ] T024 [P] [US2] Create `cli/src/commands/logs.rs` — `pub fn run(conn: &mut VyomaConnection, app: &str) -> anyhow::Result<()>`. Send `MgmtRequest::Logs { app }`. Loop reading `MgmtResponse`: on `LogLine { app, line }` print `[{app}] {line}`; on `Error { code: "NOT_FOUND" }` print to stderr and exit 3; on EOF print reconnect notice and retry connect + re-send request (implements FR-003 auto-resume on restart).
- [ ] T025 [US2] Wire `Commands::Ps` and `Commands::Logs` arms in `cli/src/main.rs` — call `commands::ps::run(&mut conn)?` and `commands::logs::run(&mut conn, &app)?` respectively

**Checkpoint**: `vyoma ps` returns all running apps within 1 second (SC-003). `vyoma logs notes` streams lines within 2s latency (SC-002). Auto-resume works after app restart.

---

## Phase 5: User Story 3 — Live Monitor Dashboard (Priority: P3)

**Goal**: `vyoma monitor` shows a live crossterm terminal dashboard with one row per app, refreshed every 2 seconds, reflecting heartbeat status changes. Exits cleanly on `q` or Ctrl+C and restores terminal.

**Independent Test**: Run VyomaOS with 3+ apps. Run `vyoma monitor`. Force a watchdog timeout on one app. Dashboard reflects status change within 4 seconds. Press `q` — terminal restored. See `quickstart.md` US3 scenario.

- [ ] T026 [US3] Implement `handle_heartbeat_stream` in `supervisor/src/mgmt_handlers.rs` — subscribe to the supervisor's existing heartbeat broadcast channel (the one `observability/HeartbeatEmitter` writes to); in a loop, when a heartbeat arrives send `MgmtResponse::Heartbeat { module, uptime_s, mem_kb, status, last_error }` over the connection. Runs until client disconnects.
- [ ] T027 [US3] Create `cli/src/commands/monitor.rs` — `pub fn run(conn: &mut VyomaConnection, host: &str, port: u16) -> anyhow::Result<()>`. (1) `crossterm::terminal::enable_raw_mode()?`; (2) send `MgmtRequest::HeartbeatStream`; (3) maintain `HashMap<String, AppHandle>` updated on each `Heartbeat` response; (4) every 2 seconds call `redraw(&apps)` which clears the screen with `execute!(stdout(), Clear(ClearType::All), cursor::MoveTo(0,0))` and prints the 5-column table (name, status with color, uptime, mem_kb, last_hb); (5) in a second thread, poll for keypresses — on `q` or Ctrl+C set a flag; (6) on exit: `crossterm::terminal::disable_raw_mode()?`, `execute!(stdout(), cursor::Show)`, clear screen.
- [ ] T028 [US3] Wire `Commands::Monitor` arm in `cli/src/main.rs` — call `commands::monitor::run(&mut conn, &host, port)?`

**Checkpoint**: `vyoma monitor` dashboard displays all running apps. Status change from `healthy` to `degraded` visible within 4 seconds (SC-004). `q` cleanly restores terminal.

---

## Phase 6: User Story 4 — Ad-hoc IPC Exec (Priority: P4)

**Goal**: `vyoma exec <app> "<message>"` routes an IPC message to a running app via the supervisor broker and prints the reply. Times out after configurable seconds (default 5).

**Independent Test**: Run `pong` app. Run `vyoma exec pong "hello"` — reply appears within 2s. Run `vyoma exec nonexistent "hi"` — exits 3 with error message. See `quickstart.md` US4 scenario.

- [ ] T029 [P] [US4] Write failing unit test in `supervisor/tests/mgmt_exec.rs` — `test_exec_delivers_message()`: register a fake app with a mock IPC handler that echoes "pong" to any message; send `{"type":"exec","app":"echo-app","msg":"hello"}`; assert response is `exec_reply` with `reply: "pong"`. Run `cargo test` to confirm FAIL.
- [ ] T030 [US4] Implement `handle_exec` in `supervisor/src/mgmt_handlers.rs` — (1) look up app in registry; if missing send `Error { code: "NOT_FOUND" }`; (2) send the message to the app's stdin pipe via the existing IPC broker; (3) wait for a reply on a one-shot channel with `timeout_ms` duration; (4) on reply send `MgmtResponse::ExecReply { reply }`; (5) on timeout send `MgmtResponse::Error { code: "TIMEOUT" }`.
- [ ] T031 [US4] Make `supervisor/tests/mgmt_exec.rs` pass — run `cargo test test_exec_delivers_message` and fix until green.
- [ ] T032 [P] [US4] Create `cli/src/commands/exec.rs` — `pub fn run(conn: &mut VyomaConnection, app: &str, msg: &str, timeout_secs: u64) -> anyhow::Result<()>`. Send `MgmtRequest::Exec { app, msg, timeout_ms: Some(timeout_secs * 1000) }`. Read one `MgmtResponse`: on `ExecReply { reply }` print reply and exit 0; on `Error { code: "NOT_FOUND" }` print to stderr and exit 3; on `Error { code: "TIMEOUT" }` print to stderr and exit 4.
- [ ] T033 [US4] Wire `Commands::Exec` arm in `cli/src/main.rs` — call `commands::exec::run(&mut conn, &app, &message, timeout)?`

**Checkpoint**: `vyoma exec pong "hello"` prints app reply within 2 seconds (SC-005). Non-existent app returns exit code 3. Timeout returns exit code 4.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Transport completeness, error handling polish, and documentation.

- [ ] T034 [P] Add QEMU Unix socket transport to `cli/src/connection.rs` — implement `VyomaConnection::connect_socket(path: &Path) -> Result<Self>` using `std::os::unix::net::UnixStream::connect(path)?`; wrap in same `BufReader`/`Box<dyn Write>` interface; wire into `cli/src/main.rs` global flag parsing (if `--socket` or `VYOMA_SOCKET` set, call `connect_socket` instead of `connect`)
- [ ] T035 [P] Add retry with exponential backoff to `cli/src/connection.rs` — in `VyomaConnection::connect()` loop: on `ConnectionRefused` sleep 100ms, 200ms, 400ms, 800ms, 1600ms up to 10s total; on other errors fail immediately; print "Connecting to localhost:9090..." on first attempt
- [ ] T036 [P] Update `Makefile` — add `run-net-mgmt` target that forwards host:9090 to guest:9090 (`-netdev user,...,hostfwd=tcp::9090-:9090`); add note to `docs/ota-updates.md` that `vyoma push` wraps the `@supervisor: ota-update` IPC command via the management server
- [ ] T037 [P] Update `docs/observability.md` — add section "Management Server Heartbeat Stream" documenting the `{"type":"heartbeat_stream"}` request and the `heartbeat` NDJSON response format; link to `specs/044-developer-tooling/contracts/mgmt-protocol.md`
- [ ] T038 Verify CLI binary size — run `cargo build --release --target x86_64-unknown-linux-musl` inside Docker builder; run `ls -lh cli/target/x86_64-unknown-linux-musl/release/vyoma`; assert size ≤ 10 MB (SC-008). If over limit: enable `lto = "fat"` in Cargo.toml and recheck.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately
- **Foundational (Phase 2)**: Depends on Phase 1 completion — **BLOCKS all user stories**
- **US1/Push (Phase 3)**: Depends on Phase 2 only
- **US2/Logs+ps (Phase 4)**: Depends on Phase 2 only — can run in parallel with Phase 3
- **US3/Monitor (Phase 5)**: Depends on Phase 2 only — can run in parallel with Phases 3–4
- **US4/Exec (Phase 6)**: Depends on Phase 2 only — can run in parallel with Phases 3–5
- **Polish (Phase 7)**: Depends on all user story phases complete

### User Story Dependencies

- **US1 (P1, push)**: Foundational complete → independently buildable and testable
- **US2 (P2, logs+ps)**: Foundational complete → independently buildable and testable
- **US3 (P3, monitor)**: Foundational complete → independently buildable and testable; consumes heartbeat broadcast added in US2 (T021 adds the log broadcast; T026 uses the existing heartbeat emitter)
- **US4 (P4, exec)**: Foundational complete + IPC broker wired in supervisor → independently buildable

### Within Each Phase

- Protocol structs (T005–T007) before connection/server (T008–T010)
- Supervisor handler implemented before CLI command that exercises it
- Tests written (confirmed failing) before handler implementation

### Parallel Opportunities

Within Phase 2:
- T005 and T006 and T007 can all run in parallel (different files)
- T008, T009, T010 can start after T005+T006 finish (depend on protocol types)

Within Phase 3:
- T012 (write test) and T015 (CLI command) can run in parallel

Within Phase 4:
- T017, T018, T019 can run in parallel (different test files)
- T023, T024 can run in parallel (different CLI command files)

Within Phase 7:
- T034, T035, T036, T037 can all run in parallel

---

## Parallel Example: User Story 2

```bash
# Run in parallel — different files, no shared dependency:
Task T017: Write failing test supervisor/tests/mgmt_ps.rs
Task T018: Write failing test supervisor/tests/mgmt_logs.rs

# After T017/T018 exist:
Task T019: Implement handle_ps in supervisor/src/mgmt_handlers.rs  ← serial (same file as T021)
Task T020: Make mgmt_ps.rs test pass

# After T019 green:
Task T021: Implement handle_logs in supervisor/src/mgmt_handlers.rs

# Once both handlers implemented, CLI commands can run in parallel:
Task T023: cli/src/commands/ps.rs
Task T024: cli/src/commands/logs.rs
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001–T004)
2. Complete Phase 2: Foundational (T005–T011)
3. Complete Phase 3: US1 Push (T012–T016)
4. **STOP and VALIDATE**: `vyoma push` end-to-end against running VM
5. Confirm SC-001 (first progress line within 5s) passes

### Incremental Delivery

1. Phase 1 + 2 → Foundation ready (`cargo build` passes, supervisor binds port 9090)
2. + Phase 3 → `vyoma push` working → Developer inner loop unlocked (MVP)
3. + Phase 4 → `vyoma ps` + `vyoma logs` working → Observability unlocked
4. + Phase 5 → `vyoma monitor` working → Operator dashboard
5. + Phase 6 → `vyoma exec` working → IPC debugging
6. + Phase 7 → Polish + both transports + docs

### Parallel Team Strategy (2 developers)

After Phase 2 complete:
- Dev A: Phase 3 (push) → Phase 5 (monitor)
- Dev B: Phase 4 (logs+ps) → Phase 6 (exec)
Both finish independently; Phase 7 polish done together.

---

## Notes

- `[P]` tasks touch different files with no in-flight dependencies — safe to run concurrently
- `[Story]` label maps directly to user story for traceability to spec acceptance criteria
- All test tasks (T012, T017, T018, T029) must be **confirmed failing** before their paired implementation task begins
- `mgmt_handlers.rs` is the one file multiple phases write to — tasks within the same phase that write to it are NOT marked [P] and must be done sequentially
- The supervisor's `mgmt_protocol.rs` and CLI's `protocol.rs` must have byte-identical JSON serialization; verify with a round-trip test in Phase 2
- Commit after each completed task or logical group to keep history bisectable
- 500-line rule: `mgmt_handlers.rs` risks growing large; split into `mgmt_handlers/ps.rs`, `mgmt_handlers/logs.rs`, etc. if it approaches 400 lines during US2–US4 implementation
