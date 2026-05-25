# VyomaOS Security Audit Checklist

**Spec**: `specs/043-universal-modular-os/` | **Date**: 2026-05-25

Use this checklist when auditing a VyomaOS build for security correctness.
Each item is a pass/fail gate; a "FAIL" must be resolved before shipping a production image.

---

## 1. Capability Isolation Verification

Every capability type must have a deny test — an app that does NOT declare the capability must be structurally prevented from accessing the corresponding resource.

### Checklist

- [ ] **`network = false`**: Deploy a WASM app with `network = false` in its `vyoma.toml`. Attempt `TcpStream::connect()` from within the app. Confirm: the WASI socket import is not wired; the call returns an error or the module traps. No TCP connection is established.

- [ ] **`filesystem = false`**: Deploy an app without `filesystem = true`. Attempt to open `/data/test.txt`. Confirm: WASI `fd_open` returns `ERRNO_NOENT` or `ERRNO_ACCES`. The `/data` mount is not present in the module's namespace.

- [ ] **`display = false`**: An app without `display = true` writes `VYOMA_DRAW:fill_rect:0,0,100,100,0`. Confirm: the supervisor does not render the command (no framebuffer write occurs). The line is silently discarded.

- [ ] **`shell = false`**: An app without `shell = true` writes `@supervisor: kill other-app`. Confirm: the supervisor does not process the command. No process is killed.

- [ ] **`mouse = false`**: An app without `mouse = true` is focused. Move the mouse. Confirm: no `VYOMA_INPUT:mouse:` event is delivered to the app's stdin.

- [ ] **`touch = false`** (mobile profile): An app without `touch = true` is focused. Inject a touch tap. Confirm: no `VYOMA_INPUT:touch:` event is delivered to the app's stdin.

- [ ] **`gpio_pins` undeclared**: An app without `gpio_pins` in its manifest calls a GPIO HAL host function. Confirm: the supervisor returns an error response or the WASM module receives a trap. No GPIO state is toggled.

- [ ] **`i2c_bus` undeclared**: An app without `i2c_bus` in its manifest attempts an I2C read. Confirm: supervisor rejects the call before spawn or the host function returns `ERRNO_PERM`. No bus transaction occurs.

- [ ] **`spi_bus` undeclared**: Same as I2C — undeclared SPI bus access is denied.

- [ ] **`uart_port` undeclared**: Same — undeclared UART port access is denied.

- [ ] **`adc_channel` undeclared**: Same — undeclared ADC channel access is denied.

### How to Run

```bash
# Unit-level capability denial tests:
make unit-test
# supervisor/tests/peripheral_capability.rs — GPIO, I2C, SPI, UART, ADC deny
# supervisor/tests/capability_deny_peripheral.rs — undeclared peripheral trap
# supervisor/tests/manifest_tests.rs — network, filesystem policy layer

# Integration-level (requires QEMU):
make integration-test TEST=capability_deny
```

---

## 2. No Ambient Authority

No capability may be inferred from the platform or assumed by default. Every resource access must trace to an explicit field in the app's `vyoma.toml`.

### Checklist

- [ ] **Platform profile does not grant app capabilities**: The `[platform]` section of the profile TOML describes hardware and runtime configuration only. It does not grant capabilities to individual apps. Audit: search for any code path in `supervisor/src/profile/` that sets app capability flags based on profile values alone.

- [ ] **New hardware detected at runtime does not become accessible**: If the supervisor detects a hardware peripheral (e.g., an I2C device on bus 1) at boot, apps still require `i2c_bus = 1` in their manifest to access it. Detection ≠ grant.

- [ ] **`display = true` in profile does not imply `display = true` in app**: Even on `desktop-full` profile (which has a display driver), an app without `display = true` in its own `vyoma.toml` cannot draw to the framebuffer.

- [ ] **Network interfaces present ≠ network capability**: The `server-headless` profile has network enabled in the supervisor. An app with `network = false` on this platform still cannot open sockets. Verify by deploying such an app and attempting a TCP connection.

- [ ] **`shell` command capability is not inherited**: An app spawned by another app (if such a mechanism is ever added) does not inherit the parent's `shell = true` capability. Each app's capabilities come solely from its own manifest.

### How to Verify

```bash
# Check for any ambient authority leaks at the code level:
grep -r "profile\." supervisor/src/capability/ | grep -v test
# Any line that sets a capability from profile data (not manifest data) is suspect.

# Manual audit: read supervisor/src/main.rs capability wiring section
# Confirm: capabilities are set from parse_manifest() return value only
```

---

## 3. Peripheral Conflict Prevention

The `PeripheralRegistry` enforces exclusive-access to hardware peripherals. Two apps may not share the same physical resource (GPIO pin, I2C bus, SPI bus, UART port) unless explicitly documented as sharable (read-only I2C is the only planned exception).

### Checklist

- [ ] **Two apps claiming the same GPIO output pin**: Deploy `app-a` with `gpio_pins = [5]` and `app-b` with `gpio_pins = [5]`. Confirm: `app-b` spawn is rejected with a clear error: `[capability] GPIO pin 5 already claimed by app-a`. `app-a` continues running normally.

- [ ] **Two apps claiming the same GPIO input pin**: Same scenario with `direction = "input"`. GPIO input is also exclusive in the current model. Confirm rejection.

- [ ] **Two apps claiming the same UART port**: Deploy two apps with `uart_port = 1`. Confirm: second spawn is rejected. First app retains access.

- [ ] **Two apps claiming the same SPI bus**: Same — SPI bus is exclusive. Second spawn rejected.

- [ ] **PeripheralRegistry release on app exit**: When `app-a` exits, its GPIO pin claims are released. Confirm: a subsequently spawned `app-c` with the same `gpio_pins = [5]` succeeds.

- [ ] **PeripheralRegistry release on restart**: When `app-a` is restarted by the supervisor (restart policy = `always`), the registry releases the old claims before re-acquiring them. No "already claimed by app-a" error on restart.

### Test References

- `supervisor/tests/exclusive_peripheral.rs` — unit-level PeripheralRegistry mock tests
- `supervisor/tests/peripheral_capability.rs` — per-peripheral capability enforcement

---

## 4. wasm3 vs Wasmtime — Capability Gate Parity

Both runtime adapters must respect the same capability gates. The supervisor enforces capabilities before the runtime executes any app code. The runtime adapter is not responsible for capability enforcement — it only executes the binary that the supervisor has already validated.

### Checklist

- [ ] **Wasmtime adapter**: Capability enforcement happens in `supervisor/src/main.rs` before `wasmtime run` is invoked. The Wasmtime process is started with only the file descriptors and environment variables that correspond to declared capabilities. Audit: no capability bypass via environment injection or extra fd inheritance.

- [ ] **wasm3 adapter**: Same constraint. The wasm3 adapter in `supervisor/src/runtime/wasm3.rs` must not expose WASI imports that were not declared in the manifest. Audit: the WASI import resolver in the wasm3 adapter must check the capability set before registering each host function.

- [ ] **Capability parity test**: Run the same WASM app with the same manifest (e.g., `network = false`) on both the Wasmtime and wasm3 adapters. Attempt a TCP connection from within the app on each runtime. Both must fail with the same outcome (access denied, not a panic or hang).

- [ ] **No runtime-specific capability bypass**: Search for any `#[cfg(feature = "wasm3")]`-gated code that grants additional capabilities. There must be none.

### Test References

- `supervisor/tests/runtime_parity.rs` — same WASM binary, same output on wasm3 vs Wasmtime
- `supervisor/tests/wasm3_adapter.rs` — wasm3 adapter instantiate/execute/terminate

---

## 5. OTA Update Integrity

The OTA update pipeline must verify the binary before deploying to slot B, and must never leave the device in an unrecoverable state.

### Checklist

- [ ] **Hash verification before slot swap**: When `@supervisor: ota-update <name> <path> sha256:<hash>` is issued, the supervisor computes the SHA-256 of the file at `<path>` and compares it to `<hash>`. If they do not match, deployment is rejected. Slot A is not modified. Log line: `[ota] hash mismatch — deployment REJECTED`.

- [ ] **Truncated binary rejected**: If the file at `<path>` is truncated (e.g., partial download), Wasmtime/wasm3 fails to parse the WASM header. The supervisor catches this error, rejects the deployment, and keeps slot A active.

- [ ] **Slot A never modified during OTA**: The slot A binary and the `active` file are never written during the health check window. Only if the health check fully succeeds is slot A overwritten (with the previous active binary for rollback backup).

- [ ] **Automatic rollback on unhealthy**: Deploy a known-crashing binary (exits with code 1 after 5 s). Confirm: after `health_check_secs` elapses (or the crash is detected), the supervisor reverts to slot A without manual intervention.

- [ ] **Power-loss recovery**: If QEMU is killed during deployment to slot B (before the health check window completes), the next boot reads `active = a` and starts the last known-good version. Simulate: start OTA, kill QEMU mid-deploy, restart, confirm slot A is running.

- [ ] **Slot B cleanup on failed deploy**: If deployment to slot B fails (hash mismatch, parse error), the partial slot B file is removed. No corrupted binary remains at `/data/ota/<app>/slot-b/`.

### Test References

- `supervisor/tests/ota_rollback.rs` — A/B slot deploy, bad-module rollback
- `docs/ota-updates.md` — full OTA workflow documentation

---

## 6. 500-Line File Limit — No Hidden Complexity

Large source files accumulate complexity and security-relevant behavior that is hard to audit. No `.rs` file in `supervisor/src/` or `apps/*/src/` may exceed 500 lines. This limit is enforced in CI.

### Checklist

- [ ] **No file exceeds 500 lines**: Run the CI check manually:

  ```bash
  find supervisor/src apps -name '*.rs' | while read f; do
    lines=$(wc -l < "$f")
    if [ "$lines" -gt 500 ]; then
      echo "FAIL: $f has $lines lines (limit: 500)"
    fi
  done
  ```

  Expected output: no lines (all files under the limit).

- [ ] **Security-critical subsystems are single-responsibility**: Each file in `supervisor/src/capability/`, `supervisor/src/ota/`, and `supervisor/src/runtime/` addresses exactly one concern. A file that handles both capability enforcement and OTA must be split.

- [ ] **New subsystem modules follow the pattern**: Any new file added to the supervisor that implements a security boundary (capability check, peripheral claim, OTA slot management) must be:
  1. Under 500 lines.
  2. Covered by unit tests in `supervisor/tests/`.
  3. Mentioned in this checklist at the next audit.

### Rationale

The 500-line limit is a security control, not just a style rule. Security-critical code that cannot be read top-to-bottom in one sitting is more likely to contain subtle bugs. Keeping modules small and focused makes capability enforcement paths auditable in minutes, not hours.

---

## Audit Sign-Off

| Section | Status | Auditor | Date |
|---------|--------|---------|------|
| 1. Capability Isolation | | | |
| 2. No Ambient Authority | | | |
| 3. Peripheral Conflict Prevention | | | |
| 4. wasm3 vs Wasmtime Parity | | | |
| 5. OTA Update Integrity | | | |
| 6. 500-Line File Limit | | | |

Fill in `PASS`, `FAIL`, or `N/A` (with justification) for each section before releasing a production image.

---

## See Also

- `supervisor/src/capability/` — implementation of capability enforcement
- `supervisor/src/ota/` — OTA A/B slot implementation
- [docs/ota-updates.md](ota-updates.md) — OTA workflow documentation
- [docs/testing-strategy.md](testing-strategy.md) — Layer 7 capability denial test matrix
- `specs/043-universal-modular-os/spec.md` — FR-002, FR-018, FR-019, FR-020 security requirements
