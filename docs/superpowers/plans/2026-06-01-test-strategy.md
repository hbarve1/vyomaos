Now I have a thorough understanding of the codebase. Let me synthesize the final plan.

# VyomaOS Comprehensive Test Strategy

## 1. Testing Pyramid

**Target ratio: 70% unit : 20% integration : 10% E2E**

Current state: 43 unit test files covering 62 of 82 modules. Zero integration tests. 6 E2E tests across 3 files.

Target state after full implementation:

| Layer | Current | Target | New Tests | New Files |
|-------|---------|--------|-----------|-----------|
| Unit | ~350 tests, 43 files | ~490 tests, 52 files | ~140 | 9 |
| Integration | 0 | ~35 tests, 1 file | ~35 | 1 |
| E2E | 6 tests, 3 files | ~22 tests, 6 files | ~16 | 3 |

Total new test count: ~191 (down from the raw proposals' 236, after removing tautological tests, deduplicating coverage, and reclassifying misplaced tests).

---

## 2. Priority Queue

### Week 1: Security + Easy Wins (Sprint 1)

**Goal**: Cover the two security-critical modules and the zero-refactoring OTA module.

| Task | Module | Tests | Effort |
|------|--------|-------|--------|
| 1a | `seccomp.rs` -- BPF filter construction | 12 | 4h |
| 1b | `ota/*` -- pure data structures | 26 | 3h |
| 1c | Extract `parse_color` from `#[cfg]` gate | 0 (prerequisite) | 0.5h |
| 1d | `draw_cmd.rs` -- `parse_color` tests (already pure) | 6 | 1h |
| 1e | E2E harness: fix `test_boot_under_5s` assertion lie | 0 (fix) | 0.5h |
| 1f | E2E harness: pipe QEMU stderr to log file | 0 (infra) | 0.5h |

**Week 1 output**: 44 new tests, security modules covered.

### Week 2: IPC Pipeline (Sprint 2)

**Goal**: Cover the router and IPC handler parsing paths.

| Task | Module | Tests | Effort |
|------|--------|-------|--------|
| 2a | Extract `classify_line` from `router.rs` into `lib.rs` `ipc` module | 0 (prerequisite) | 2h |
| 2b | `router.rs` -- `classify_line` unit tests | 14 | 2h |
| 2c | Extract argument parsers from `ipc_handlers.rs` into lib | 0 (prerequisite) | 3h |
| 2d | `ipc_handlers.rs` -- argument parsing + reply formatting tests | 22 | 3h |
| 2e | Deduplicate `ipc_update.rs` (delegate from `ipc_handlers.rs` update arm) | 0 (prerequisite) | 2h |
| 2f | `ipc_update.rs` -- parse + SHA-256 verification tests | 8 | 2h |

**Week 2 output**: 44 new tests, IPC pipeline covered.

### Week 3: Display Parsing + Integration Harness (Sprint 3)

**Goal**: Cover draw command parsers and build the integration test harness.

| Task | Module | Tests | Effort |
|------|--------|-------|--------|
| 3a | Extract all `draw_cmd.rs` parsers outside `#[cfg(target_os = "linux")]` | 0 (prerequisite) | 6h |
| 3b | `draw_cmd.rs` -- parser tests (fill_rect, draw_text, rect_border, etc.) | 24 | 4h |
| 3c | Build integration test harness (mock Inbox, mock channel helpers) | 0 (infra) | 3h |
| 3d | Integration: IPC round-trip tests (list, status, ps, kill parsing, focus) | 10 | 3h |

**Week 3 output**: 34 new tests, draw parsers + integration harness.

### Week 4: Window Math + E2E Expansion (Sprint 4)

**Goal**: Cover remaining modules and expand E2E suite.

| Task | Module | Tests | Effort |
|------|--------|-------|--------|
| 4a | Extract coordinate math from `win_actions.rs` | 0 (prerequisite) | 1h |
| 4b | `win_actions.rs` -- drag position + minimize strip tests | 12 | 2h |
| 4c | Integration: security enforcement tests (cap-request, firewall, audit) | 10 | 3h |
| 4d | Integration: cross-module tests (broadcast, reply routing, clipboard) | 8 | 3h |
| 4e | E2E: `test_shortcuts.py` -- QMP key combo validation + 3 shortcuts | 4 | 3h |
| 4f | E2E: `test_ipc.py` -- shell command round-trips | 4 | 2h |
| 4g | E2E: `test_recovery.py` -- crash recovery + recovery mode | 3 | 2h |

**Week 4 output**: 41 new tests across all layers.

### Post-Sprint: Ongoing

| Task | Description | Effort |
|------|-------------|--------|
| 5a | Add `proptest` harness for `parse_color`, `parse_ipc_target`, draw command parsers | 4h |
| 5b | Adversarial input tests: null bytes, special chars in app names, u32::MAX coords | 3h |
| 5c | E2E: `test_platform.py` -- mobile profile boot (when ARM QEMU available) | 2h |

---

## 3. Unit Tests: Per-Module Specifications

### 3.1 seccomp.rs (P1 -- CRITICAL, 12 tests)

**File**: `supervisor/tests/seccomp_test.rs`

The `build()` function is pure -- it returns `Vec<SockFilter>`. No refactoring needed; structs and `build()` are already `pub`.

**Test helper**: Write a `bpf_action_for_syscall(filter: &[SockFilter], nr: u32) -> Option<u32>` that walks the BPF instruction array simulating the two-instruction JEQ+RET pattern. Document it as "mirrors the structure of `build()`, will break if BPF program structure changes."

```
seccomp_test.rs:
  build_filter_not_empty                    -- filter.len() > 0
  build_filter_len_under_u16_max            -- filter.len() < 65535 (u16 truncation guard)
  build_filter_starts_with_arch_check       -- instruction[0] loads OFF_ARCH (k=4)
  build_filter_arch_mismatch_kills          -- instruction[2] returns SECCOMP_RET_KILL_PROCESS
  build_filter_loads_syscall_nr             -- instruction[3] loads offset 0 (syscall number)
  build_filter_denies_ptrace                -- bpf_action_for_syscall(101) == KILL_PROCESS
  build_filter_denies_reboot                -- bpf_action_for_syscall(169) == KILL_PROCESS
  build_filter_denies_kexec_load            -- bpf_action_for_syscall(246) == KILL_PROCESS
  build_filter_denies_seccomp_syscall       -- bpf_action_for_syscall(317) == KILL_PROCESS
  build_filter_clone3_returns_enosys        -- bpf_action_for_syscall(435) == RET_ERRNO(38)
  build_filter_ends_with_allow              -- last instruction is RET_ALLOW
  build_filter_allowed_syscall_passes       -- bpf_action_for_syscall(0) [read] == ALLOW
```

**Dropped from original proposal**: `build_filter_length_matches_structure` (too brittle, breaks on any filter change), `build_filter_no_duplicate_rules` (covered implicitly by the per-syscall checks), `build_filter_all_denied_syscalls_present` (redundant with individual checks).

**Coverage target**: 95% of `build()`. `apply()` remains smoke-test only.

### 3.2 namespace.rs (DEPRIORITIZED -- 0 unit tests)

The critic correctly identified that the 6 proposed tests are tautologies (testing that `libc::CLONE_NEWNS == libc::CLONE_NEWNS`). This is a 20-line function with zero branching logic. All meaningful behavior requires root + fork context.

**Action**: Covered by the existing `make smoke` QEMU boot test. No unit tests needed. If mount namespace isolation verification is needed in the future, write a dedicated integration test that forks a child process on a Linux CI runner.

### 3.3 router.rs (P2 -- HIGH, 14 tests)

**File**: `supervisor/tests/router_test.rs`

**Prerequisite refactoring**: Extract `classify_line()` into `supervisor::ipc` (lib.rs). This is the pure routing logic without any channel/global-state dependencies.

```rust
// Add to supervisor/src/ipc.rs:
pub enum LineKind<'a> {
    Draw(&'a str),
    Audio(&'a str),
    Supervisor(&'a str),
    Mgmt(&'a str),
    Broadcast(&'a str),
    Reply(&'a str),
    AppMessage(&'a str, &'a str),  // (target, message)
    Plain,
}

pub fn classify_line(line: &str, has_display: bool) -> LineKind<'_> { ... }
```

```
router_test.rs:
  classify_vyoma_draw_with_display          -- "VYOMA_DRAW:fill_rect:..." + has_display=true -> Draw
  classify_vyoma_draw_without_display       -- "VYOMA_DRAW:fill_rect:..." + has_display=false -> Plain
  classify_vyoma_audio_command              -- "VYOMA_AUDIO:play:..." -> Audio("play:...")
  classify_supervisor_command               -- "@supervisor: ps" -> Supervisor("ps")
  classify_mgmt_target                      -- "@__mgmt__: reply" -> Mgmt("reply")
  classify_broadcast                        -- "@broadcast: hello" -> Broadcast("hello")
  classify_reply                            -- "@reply: data" -> Reply("data")
  classify_app_message                      -- "@shell: input x" -> AppMessage("shell", "input x")
  classify_plain_text                       -- "hello world" -> Plain
  classify_at_without_colon                 -- "@nocol" -> Plain (no ": " separator)
  classify_empty_string                     -- "" -> Plain
  classify_draw_strips_prefix               -- "VYOMA_DRAW:flush" -> Draw("flush")
  classify_audio_strips_prefix              -- "VYOMA_AUDIO:stop" -> Audio("stop")
  classify_at_empty_target                  -- "@: message" -> handled by parse_ipc_target -> Plain
```

**Dropped from original proposal**: `@all: hello` -> Broadcast test. The critic correctly identified that `is_broadcast_target()` only matches `"broadcast"`, NOT `"all"`. The code comment in `router.rs` line 8 mentions `@all:` but it is not implemented. Tests must match actual behavior.

**Coverage target**: 95% of `classify_line`. The `route_or_print` orchestration (channel delivery, LAST_SENDER tracking) is covered by integration tests.

### 3.4 ipc_handlers.rs (P3 -- HIGH, 22 tests)

**File**: `supervisor/tests/ipc_handlers_test.rs`

**Prerequisite refactoring**: Extract argument parsing into `supervisor::ipc` (lib.rs). Split `ipc_handlers.rs` to comply with 500-line limit (it is currently 511 lines). Deduplicate the `update` command -- the arm at lines 276-372 duplicates `ipc_update.rs`. After dedup, the `update` match arm should delegate to `ipc_update::handle_update()`.

```rust
// Add to supervisor/src/ipc.rs:
pub fn parse_kill_args(parts: &[&str]) -> Result<String, &'static str>
pub fn parse_restart_args(parts: &[&str]) -> Result<String, &'static str>
pub fn parse_run_args(parts: &[&str]) -> Result<String, &'static str>
pub fn parse_log_args(parts: &[&str]) -> Result<String, &'static str>
pub fn parse_update_args(rest: &str) -> Result<(String, String), &'static str>
pub fn format_status_reply(count: usize) -> String
pub fn format_font_size_reply(size: &str) -> Result<String, &'static str>
pub fn tail_log_lines(content: &str, n: usize) -> Vec<String>
```

```
ipc_handlers_test.rs:
  -- Argument parsing --
  parse_kill_args_valid                     -- ["kill", "my-app"] -> Ok("my-app")
  parse_kill_args_missing_name              -- ["kill"] -> Err
  parse_kill_args_whitespace_only           -- ["kill", "  "] -> Err
  parse_restart_args_valid                  -- ["restart", "shell"] -> Ok("shell")
  parse_restart_args_missing                -- ["restart"] -> Err
  parse_run_args_valid                      -- ["run", "/apps/foo/vyoma.toml"] -> Ok
  parse_run_args_missing                    -- ["run"] -> Err
  parse_log_args_valid                      -- ["log", "shell"] -> Ok("shell")
  parse_log_args_missing                    -- ["log"] -> Err
  parse_update_args_valid                   -- "my-app https://x.com/a.wasm" -> Ok
  parse_update_args_no_url                  -- "my-app" -> Err
  parse_update_args_empty                   -- "" -> Err

  -- Reply formatting --
  format_status_reply_zero                  -- 0 -> {"running":0}
  format_status_reply_nonzero               -- 5 -> {"running":5}
  format_font_size_valid_s                  -- "s" -> Ok
  format_font_size_valid_m                  -- "m" -> Ok
  format_font_size_valid_l                  -- "l" -> Ok
  format_font_size_invalid                  -- "xl" -> Err

  -- Log tailing --
  tail_log_lines_empty_content              -- "" -> []
  tail_log_lines_fewer_than_n              -- 3 lines, n=10 -> 3 lines
  tail_log_lines_more_than_n               -- 20 lines, n=10 -> last 10
  tail_log_lines_pipe_chars_escaped         -- "a|b" -> "a b"
```

**Dropped from original proposal**: `format_ps_raw_entry` tests (coupled to exact string format -- use struct-based assertions in integration tests instead). `dispatch_list_returns_pipe_separated` and `dispatch_unknown_command_logged` (these are integration tests, not unit tests).

**Added from critic**: pipe injection test (`tail_log_lines_pipe_chars_escaped`).

**Coverage target**: 90% of extracted parsing logic. Honest reporting: this covers ~20% of the 511-line module. The remaining 80% (orchestration, spawn, kill syscalls) is covered by integration + E2E tests.

### 3.5 ipc_update.rs (P4 -- MEDIUM, 8 tests)

**File**: `supervisor/tests/ipc_update_test.rs`

**Prerequisite**: Deduplicate with `ipc_handlers.rs` update arm first. Test only the single implementation.

```rust
// Add to supervisor/src/ipc.rs:
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool
pub fn compute_install_dest(manifest_path: &str, app_name: &str) -> std::path::PathBuf
```

```
ipc_update_test.rs:
  verify_sha256_match                       -- known bytes + correct hash -> true
  verify_sha256_mismatch                    -- known bytes + wrong hash -> false
  verify_sha256_case_insensitive            -- uppercase hex matches -> true
  verify_sha256_empty_bytes                 -- empty &[] -> computes hash of empty
  compute_install_dest_with_parent          -- "/apps/foo/vyoma.toml" -> "/apps/foo/my-app.wasm"
  compute_install_dest_no_parent            -- "vyoma.toml" -> "/apps/my-app.wasm"
  compute_install_dest_nested               -- "/etc/vyoma/apps/bar/vyoma.toml" -> correct
  compute_install_dest_root                 -- "/vyoma.toml" -> "/my-app.wasm"
```

**Coverage target**: 90% of parsing/verification logic.

### 3.6 draw_cmd.rs (P5 -- MEDIUM, 30 tests)

**File**: `supervisor/tests/draw_cmd_test.rs`

**Prerequisite refactoring (biggest single refactor)**: Move ALL parser functions outside `#[cfg(target_os = "linux")]`. Currently `parse_color` and every parser is inside the cfg gate. Create a new `supervisor::draw_parse` module in lib.rs with platform-independent parsers. The binary crate's `handle_draw_command()` becomes: call parser -> match result -> call framebuffer.

Estimated effort: 6-8 hours (not 3h as originally proposed). Each parser is interleaved with framebuffer calls and must be disentangled.

```rust
// New: supervisor/src/draw_parse.rs (added to lib.rs)
pub fn parse_color(s: &str) -> Option<u32>
pub fn parse_fill_rect(args: &str) -> Result<(u32, u32, u32, u32, u32), ParseError>
pub fn parse_draw_text(args: &str) -> Result<DrawTextArgs, ParseError>
pub fn parse_draw_text_wrap(args: &str) -> Result<DrawTextWrapArgs, ParseError>
pub fn parse_fill_rect_r(args: &str) -> Result<(u32, u32, u32, u32, u32, u32), ParseError>
pub fn parse_draw_glyph(args: &str) -> Result<DrawGlyphArgs, ParseError>
pub fn parse_draw_image(args: &str) -> Result<(u32, u32, u32, u32, String), ParseError>
pub fn parse_rect_border(args: &str) -> Result<(u32, u32, u32, u32, u32), ParseError>
pub fn parse_clear_region(args: &str) -> Result<(u32, u32, u32, u32), ParseError>
```

```
draw_cmd_test.rs:
  -- parse_color --
  parse_color_decimal                       -- "4294967295" -> Some(0xFFFFFFFF)
  parse_color_hex_lower                     -- "0x0d1117ff" -> Some(0x0D1117FF)
  parse_color_hex_upper                     -- "0X0D1117FF" -> Some(0x0D1117FF)
  parse_color_zero                          -- "0" -> Some(0)
  parse_color_invalid                       -- "abc" -> None
  parse_color_empty                         -- "" -> None

  -- parse_fill_rect --
  parse_fill_rect_valid                     -- "10,20,100,50,4294967295" -> Ok
  parse_fill_rect_too_few_args              -- "10,20,100" -> Err
  parse_fill_rect_non_numeric               -- "a,b,c,d,e" -> Err
  parse_fill_rect_hex_color                 -- "0,0,10,10,0xFF0000FF" -> Ok

  -- parse_draw_text --
  parse_draw_text_5field                    -- "10,20,255,m,Hello" -> Ok with Medium
  parse_draw_text_4field_legacy             -- "10,20,255,Hello" -> Ok with Medium default
  parse_draw_text_commas_in_text            -- "10,20,255,m,Hello, World" -> text="Hello, World"
  parse_draw_text_invalid_size_fallback     -- "10,20,255,x,text" -> falls back to 4-field

  -- parse_fill_rect_r --
  parse_fill_rect_r_valid                   -- "0,0,100,50,255,8" -> Ok with radius=8
  parse_fill_rect_r_zero_radius             -- radius=0 -> Ok

  -- parse_draw_glyph --
  parse_draw_glyph_normal                   -- "10,20,255,16,normal,A" -> bold=false, mono=false
  parse_draw_glyph_bold                     -- "10,20,255,16,bold,B" -> bold=true
  parse_draw_glyph_mono                     -- "10,20,255,16,mono,X" -> mono=true

  -- parse_draw_image --
  parse_draw_image_valid                    -- "0,0,64,64,/icons/app.png" -> Ok
  parse_draw_image_path_with_commas         -- splitn(5) captures full path

  -- parse_rect_border --
  parse_rect_border_valid                   -- "0,0,100,50,255" -> Ok
  parse_rect_border_bad_args                -- "0,0" -> Err

  -- parse_clear_region --
  parse_clear_region_valid                  -- "0,0,100,50" -> Ok
  parse_clear_region_bad_args               -- "0,0,100" -> Err

  -- Adversarial inputs (from critic) --
  parse_fill_rect_negative_string           -- "-10,20,100,50,255" -> Err (u32 parse fails)
  parse_fill_rect_u32_max_coords            -- "4294967295,4294967295,1,1,255" -> Ok (no overflow in parser)
  parse_color_whitespace_trimmed            -- " 255 " -> Some(255)
  parse_draw_text_empty_text                -- "10,20,255,m," -> Ok with text=""
  set_layer_alpha_silently_ignored          -- verify it returns without error
```

**Coverage target**: 90% of all parsers.

### 3.7 ota/* (P6 -- MEDIUM, 26 tests)

**File**: `supervisor/tests/ota_test.rs`

No refactoring needed. All three files are pure data structures already exposed via `pub mod ota` in lib.rs.

```
ota_test.rs:
  -- SlotLabel --
  slot_a_other_is_b                         -- A.other() == B
  slot_b_other_is_a                         -- B.other() == A
  slot_a_display                            -- format A
  slot_b_display                            -- format B

  -- AbSlot --
  ab_slot_new_starts_at_a                   -- new().active == A
  ab_slot_active_path_returns_slot_a        -- initial active_path == initial_path
  ab_slot_swap_changes_to_b                 -- after swap(), active == B
  ab_slot_swap_clears_health                -- after swap(), health_confirmed == false
  ab_slot_rollback_reverts_slot             -- A -> swap -> B -> rollback -> A
  ab_slot_rollback_sets_health_confirmed    -- rollback() -> health_confirmed = true
  ab_slot_double_swap                       -- swap twice -> back to A
  ab_slot_active_path_after_swap            -- set slot_b_path, swap, active_path returns it

  -- HealthChecker --
  health_new_starts_at_zero                 -- passed == 0
  health_record_one_not_enough              -- required=3, record 1 -> Failed
  health_record_enough_passes               -- required=3, record 3 -> Passed
  health_evaluate_before_any                -- evaluate() -> Failed
  health_evaluate_after_enough              -- record 3, evaluate -> Passed
  health_is_not_timed_out_long              -- timeout=3600 -> false immediately

  -- OtaManager --
  ota_new_empty                             -- records empty
  ota_begin_update_returns_index_0          -- first -> 0
  ota_begin_update_returns_index_1          -- second -> 1
  ota_set_status_verifying                  -- status changes
  ota_set_status_out_of_bounds              -- no panic on invalid index
  ota_record_health_check                   -- health_checks_passed increments
  ota_rollback_sets_status                  -- status == RolledBack
  ota_latest_for_found                      -- returns most recent
  ota_latest_for_not_found                  -- returns None
```

**Dropped**: `health_is_timed_out_immediate` with timeout=0 (critic correctly identified it as inherently racy due to `Instant::now()` resolution). `ota_rollback_sets_reason` (already implicitly covered by `ota_rollback_sets_status`).

**Coverage target**: 100%.

### 3.8 win_actions.rs (P7 -- LOW, 12 tests)

**File**: `supervisor/tests/win_actions_test.rs`

**Prerequisite**: Extract coordinate math into `supervisor::windows` (lib.rs). Currently the computation at lines 56-60 of `apply_drag_update` is inline inside `#[cfg(target_os = "linux")]`.

```rust
// Add to supervisor/src/windows.rs:
pub fn compute_drag_position(
    cursor_start: (i32, i32), cursor_now: (i32, i32),
    win_start: (u32, u32, u32, u32),
    screen_w: i32, screen_h: i32, menubar_h: u32,
) -> (u32, u32)

pub fn compute_minimize_strip_position(
    minimized_count: u32, screen_w: u32, screen_h: u32,
) -> (u32, u32)
```

```
win_actions_test.rs:
  -- compute_drag_position --
  drag_no_movement                          -- delta=(0,0) -> same position
  drag_right_10px                           -- cursor moves right 10 -> win_x + 10
  drag_clamp_left_edge                      -- past x=0 -> clamps to 0
  drag_clamp_right_edge                     -- past screen_w -> clamps
  drag_clamp_top_at_menubar                 -- can't drag above menubar_h
  drag_clamp_bottom_edge                    -- can't drag below screen_h - wh
  drag_large_delta                          -- 500px drag applies

  -- compute_minimize_strip_position --
  strip_first_minimized                     -- count=0 -> (0, screen_h - 28)
  strip_second_minimized                    -- count=1 -> (240, screen_h - 28)
  strip_wraps_to_next_row                   -- count fills row -> y decrements by 28
  strip_many_minimized                      -- 10 windows positioned correctly

  -- DragState --
  drag_state_initially_none                 -- drag_state() starts as None
```

**Coverage target**: 90% of coordinate math.

### 3.9 mount.rs (P8 -- LOW, 0 unit tests)

The critic correctly identified that this module is thin libc wrappers with no testable logic. The only branching is EBUSY handling and 9P-to-tmpfs fallback, both of which require libc::mount to test.

**Action**: Covered by `make smoke` (QEMU boot mounts all filesystems). No unit tests.

---

## 4. Integration Tests

### 4.1 Harness Design

**File**: `supervisor/tests/integration_harness.rs` (single binary to minimize compilation cost)

The core blocker is that `AppState`, `AppRegistry`, `Inbox`, `FocusedApp` are defined in the binary crate (`main.rs`), not `lib.rs`. Two options were evaluated:

**Option A** (adopted): Write integration tests as `#[cfg(test)] mod integration` inside the binary crate's module tree. This avoids the multi-day refactor of moving `AppState` and its 30+ fields (including platform-conditional types like `Surface`, `Animation`) to lib.rs.

**Option B** (deferred): Move core types to lib.rs. This is the correct long-term architecture but requires touching 40+ source files and resolving `#[cfg(target_os = "linux")]` type dependencies. Defer to a dedicated refactoring sprint.

**Practical approach**: Create `supervisor/src/integration_tests.rs` with `#[cfg(test)]` guard, included via `mod integration_tests;` in `main.rs`. Tests in this module have access to all binary crate types.

### 4.2 Mock Factories

```rust
// supervisor/src/integration_tests.rs

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};
    use std::collections::HashMap;

    type Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
    type FocusedApp = Arc<Mutex<Option<String>>>;

    fn mock_inbox(names: &[&str]) -> (Inbox, HashMap<String, mpsc::Receiver<String>>) {
        let mut senders = HashMap::new();
        let mut receivers = HashMap::new();
        for &name in names {
            let (tx, rx) = mpsc::channel();
            senders.insert(name.to_string(), tx);
            receivers.insert(name.to_string(), rx);
        }
        (Arc::new(Mutex::new(senders)), receivers)
    }

    fn mock_focused(name: Option<&str>) -> FocusedApp {
        Arc::new(Mutex::new(name.map(|s| s.to_string())))
    }

    fn collect_replies(rx: &mpsc::Receiver<String>) -> Vec<String> {
        let mut msgs = vec![];
        while let Ok(msg) = rx.try_recv() { msgs.push(msg); }
        msgs
    }
}
```

**Note on AppRegistry mocking**: For tests that need to call `handle_supervisor_command` directly, we must construct real `AppState` structs. This is feasible inside the binary crate (all types are in scope), but `AppState` has no `Default` implementation. Add `#[cfg(test)] impl Default for AppState` to reduce test boilerplate and insulate tests from field additions.

### 4.3 Side Effect Inventory

Before each test, document which side effects the handler produces:

| Command | Global state | Filesystem I/O | Process syscalls |
|---------|-------------|----------------|------------------|
| list | Inbox lock (read) | None | None |
| status | Inbox lock (read) | None | None |
| apps | AppRegistry lock (read) | None | None |
| focus | FocusedApp lock (write) | None | None |
| ps / ps-raw | AppRegistry lock (read) | None | None |
| kill | AppRegistry lock (read), audit log | None | `libc::kill` (cfg-gated) |
| restart | AppRegistry lock, spawn_app | Manifest read | Command::new |
| log | AppRegistry lock (read) | None | None |
| logf | None | `fs::read_to_string` | None |

**Safe for mock-only testing**: list, status, apps, focus, ps, ps-raw, log
**Require filesystem mock or skip**: logf, logs, reload
**Require process mock or cfg-gate**: kill, restart, run, update

### 4.4 Integration Test Scenarios (35 tests)

