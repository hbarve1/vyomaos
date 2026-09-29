# Local continuous-worker verification

Date: 2026-09-29. Tested worker commit: `ebb8614eda438aa0ab6e05be90c6cbd5656b4670`. Parent: PR #217, branch `build/t0-pinned-inputs`, base `baab51b5`. Worker branch: `ops/continuous-worker`.

**Prepared and locally tested; not installed or enabled.** This report proves wrapper behavior with stubs, not a successful unattended Codex development cycle. No model job, timer installation, account change or budget configuration occurred.

## Host inspection

The maintainer confirmed this is the always-on machine. Read-only checks found:

- systemd PID 1; user manager reports running; user lingering is enabled.
- Codex CLI 0.159.0 at `/home/hbarve1/.local/bin/codex`, signed in using ChatGPT.
- The CLI supports `exec --approve-for-me --json --output-schema --output-last-message`.
- `flock`, GNU timeout, Python, Docker, QEMU and GitHub access are available.
- Systemd's PATH needs the explicit `~/.local/bin` addition supplied in the service.

These checks do not establish remaining account quota, the cost of a run, or actual unattended approval/MCP behavior.

## Local verification

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests/automation -v
bash -n scripts/codex-worker.sh
systemd-analyze --user verify ops/codex/vyomaos-worker.service ops/codex/vyomaos-worker.timer
git diff --check build/t0-pinned-inputs...HEAD
```

All pass. Eleven regression tests use temporary directories and stub Codex/Docker commands: successful handoff/logs (including private file mode), required/invalid limits, persisted daily allowance, STOP file, lock exclusion, three-failure pause, malformed result, timeout exit propagation, review wait and external blocker. The timeout case stubs the timeout result; it does not run a real 45-minute job.

The tests were rerun after rebasing onto the parent and changing log permissions. Logs: `/home/hbarve1/codes/vyomaos/.worktrees/ops-continuous-worker/out/validation/worker/`. No OS production sources changed in this child; its parent supplies [the full local OS suite evidence](2026-09-29-t0-02.md), including the unchanged Python/guest failures. No remote Actions were used.

## Remaining activation work

Follow [the operating plan](../codex-work-loop.md): integrate the parent then child, choose run/time limits, install user units/configuration, manually run one bounded real pilot, inspect its tool access and recovery, and only then enable recurrence.

The wrapper mechanically limits starts/time and concurrency; PR queue depth, task scope and merge restrictions are prompt policies. Dollar/token ceilings are not implemented. Docker cleanup depends on the mandated per-run labels. A pilot must check these boundaries, authentication refresh, actual approval review and interrupted work recovery. Keep maintainer review and branch protections.

The next implementation item is T0-03; it may stack directly on the tested T0-02 parent while this operational change awaits review. The worker must not self-enable, adjust its controls or merge/release on its own.
