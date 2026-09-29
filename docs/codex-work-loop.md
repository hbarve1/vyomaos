# Continuous local Codex work and stacked PRs

Status (2026-09-29): implementation and stacked PRs are authorized on this always-on machine. Runner and systemd templates are prepared, **not installed or enabled**. No recurring model job has started; run/time limits remain unset pending the usage-limit decision.

## Recommendation

Use persistent, bounded milestone work to deliver the [terminal-first checklist](../TODOS.md). Give each run a concrete outcome and verification gate, preserve state in the repository, and deliver small PRs to `develop`. Prefer a completed and verified vertical slice over uninterrupted activity as the measure of progress.

Codex Goals preserve an objective across turns with completion criteria and lifecycle/budget controls. They are suitable for an individual milestone whose next action depends on test results. They are not a guarantee of indefinite unattended execution. [Official Goals guide](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex).

Scheduled tasks can start recurring work. Local-project tasks require the computer to remain on and the desktop app running; web tasks cannot directly operate on this local repository folder. Availability and permissions depend on the configured environment. [Official scheduled-task documentation](https://learn.chatgpt.com/docs/automations?surface=app).

## Proposed execution contract

1. Read `AGENTS.md`, `TODOS.md`, recent merged changes and open PRs before selecting work. Resume an existing task branch before creating duplicate work.
2. Select the next dependency-ready R1 task or a small coherent group with a single acceptance gate. Use an isolated worktree based on current `develop`, or on the tested parent tip for an explicitly linked stacked PR.
3. Record the active task, branch, intended acceptance command and known prerequisites in the handoff table. Keep one active writer per task/worktree; scheduled starts must not overlap an existing run.
4. Implement the production path, add meaningful acceptance/regression tests, and run the applicable host/WASM/QEMU checks. Fix failures before expanding scope. Report missing prerequisites as blockers, never passes.
5. Record commands, results, commit and artifacts. Open/update a focused PR to `develop`, or its immediate parent branch for a stack. Do not mark the checklist item complete merely because the PR exists.
6. Continue dependent work while its tested parent awaits review; do not wait for every merge. Keep at most three open PRs per stack. Reconcile merged parents, restack and retest children before retargeting to `develop`.
7. When a run ends, leave a concise handoff: completed work, test evidence, open PRs, failed attempts, unresolved blockers and the next exact task. Avoid leaving knowledge only in chat.

## Limits and operator decisions

- Set an explicit per-run token/time budget and a daily spend ceiling in the execution setup. Agree those values before enabling recurring execution; none is configured by this PR.
- Start with one milestone pilot, then enable recurrence after validating completion, failure and handoff behavior. Do not increase scope or budget automatically when a run exhausts its allowance.
- Limit repeated attempts at the same failure. After three materially equivalent failures, record the evidence and change approach or mark that task blocked; continue independent authorized work where possible.
- The maintainer authorized implementation and stacked PRs on 2026-09-29. Merges, automatic merge settings and OS release publication remain maintainer-controlled.
- Run disk/installer fault tests only on disposable images or specifically authorized test devices. A real PC, firmware/boot changes and physical acceptance tests need an actual test environment; an agent cannot infer their results from VM success.
- Preserve sandbox and permission controls. Set up only the filesystem, network and builder access required for the task; do not disable controls to make a run appear unattended.
- A recurring run must not start new work if another run owns the task, the budget is exhausted, or mandatory dependencies are unavailable. Leave a visible status instead of repeatedly opening duplicate PRs.

The maintained record should include task ID, owner/run ID, branch, PR, last tested commit, evidence paths, blocker and next action. A scheduler additionally needs an execution lock/lease and stale-run recovery. Those mechanisms are setup work, not features that this document installs.

## First pilot objective

Use this after the planning PR is integrated and implementation is authorized:

```text
Complete TODOS.md T0-01 through T0-GATE for VyomaOS R1.
Use the terminal-first release scope and repository worktree conventions.
Fix the CLI/supervisor build baseline, pin/document the builder, repair
clean-build/kernel/rootfs/CI behavior, and prove real WASM startup from
a fresh build. Execute the documented acceptance checks and preserve
logs, target configuration, artifact digests and commit IDs.
Work in small task branches; open focused PRs against develop if authorized.
Respect pending review/merge dependencies and existing permission controls.
Never mark missing, skipped or mock-only acceptance checks as passed.
Keep TODOS.md handoff state current. If external dependencies prevent
completion, report evidence, attempts, the exact blocker and next action.
```

The pilot is complete when the T0 gate is verified and integrated, not when a session has run for a particular number of hours. Then apply the same process to T1 console I/O and T2 authorization.

## Concrete deployment on this machine

Use `/home/hbarve1/codes/vyomaos`, user `hbarve1`. Verified locally: systemd is PID 1, the user manager is running with `Linger=yes`, Codex CLI 0.159.0 is installed at `/home/hbarve1/.local/bin/codex` and signed in through ChatGPT, and Docker/QEMU/GitHub access works. The service explicitly includes `~/.local/bin`, which is missing from the current systemd PATH.

Use a user systemd timer to start one bounded CLI invocation, then wait five minutes after completion before the next opportunity. Progress survives in worktrees, PRs, TODOS and evidence rather than depending on one process or chat. Codex supports noninteractive scripts, JSONL events and final-message files. [Official noninteractive documentation](https://learn.chatgpt.com/docs/non-interactive-mode).

The installed CLI exposes `--approve-for-me`: the runner uses workspace-write sandboxing with automatic approval review, never a sandbox bypass. Denied prerequisites become blockers. A systemd-managed CLI worker does not require an open desktop app; the desktop scheduled-task alternative does. [Official scheduling documentation](https://learn.chatgpt.com/docs/automations).

Files: [runner](../scripts/codex-worker.sh), [task prompt](../ops/codex/worker-prompt.md), [result schema](../ops/codex/result.schema.json), [service](../ops/codex/vyomaos-worker.service), [timer](../ops/codex/vyomaos-worker.timer), [configuration example](../ops/codex/worker.env.example).

### Enforced controls

- Explicit limits: 1–48 starts per UTC day and 1–45 minutes/run. Missing/invalid limits refuse to launch. Suggested pilot: two runs/day, thirty minutes/run; no values are configured yet.
- An exclusive persistent `flock` prevents overlapping worker invocations. A STOP file prevents model calls. Daily slots are reserved before launch; failed/interrupted launches count.
- GNU timeout bounds each run, with a second systemd timeout and process-group cleanup. Preserve JSONL, stderr, final JSON, exit status and the latest-run path.
- Pause after three failed invocations, a reported external blocker, or a full review queue with no useful independent work. Resolve the cause before removing STOP. Exhausted daily allowance retries cheaply until the next UTC day without model calls.
- Remove only Docker containers carrying that invocation's unique `vyomaos.worker-run` label. The prompt requires labels; unlabelled leftovers remain a pilot-test risk, not a cleanup guarantee.

These caps bound starts/time, **not dollars or exact tokens**. Observe actual ChatGPT usage during the pilot; use an independently enforced billing mechanism if a hard monetary ceiling is required. Do not infer price/quota from the login method. The runner does not change models or account settings. 24/7 means host availability; review backlog, rate limits, auth expiry, broken prerequisites and unavailable physical hardware still stop progress.

### Stack mechanics

```text
develop
  └─ build/t0-pinned-inputs       PR A → develop
       ├─ ops/continuous-worker  PR B → build/t0-pinned-inputs
       └─ fix/t0-kernel-inputs    future child → build/t0-pinned-inputs
```

Record parent PR, exact parent/base SHA, tested head SHA, task and evidence. Independent PRs target develop. Final integration is always into develop; targeting an immediate parent is the maintainer-authorized exception to the older direct-base rule. Never merge a child into a temporary parent branch.

Merge parent-first. After a parent merges, fetch and move only the child commits; this avoids replaying parent commits after squash/rebase merges:

```bash
git fetch origin develop
git rebase --onto origin/develop OLD_PARENT_SHA CHILD_BRANCH
# Retest the resulting tree before publishing it.
git push --force-with-lease=refs/heads/CHILD_BRANCH:EXPECTED_REMOTE_SHA origin CHILD_BRANCH
gh pr edit CHILD_PR --base develop
```

Unexpected remote changes require inspection, not overwrite. Restack grandchildren onto refreshed parents, record new SHAs and retest. Never force-push develop/main. Keep `[skip ci]` in commits while billing blocks Actions; run all validation locally. Report inherited and new failures separately without suppressing either. PR count, task scope and merge restrictions are prompt policies; repository protections and maintainer review remain necessary.

### Activation checklist

- [x] Check this machine's systemd user manager, lingering, CLI/auth and local build access.
- [x] Prepare runner, prompt/schema, service/timer and local regression tests.
- [ ] Review/integrate the worker stack so control files exist in the main checkout.
- [ ] Choose daily run count and per-run minutes in the environment file.
- [ ] Run one manual pilot; check actual approvals, GitHub, Docker/QEMU, cleanup and interrupted-run recovery.
- [ ] Enable the timer; inspect its first recurring run and usage before increasing limits.

These are future operator steps, not commands already executed:

```bash
cd /home/hbarve1/codes/vyomaos
mkdir -p ~/.config/systemd/user
install -m 600 ops/codex/worker.env.example ~/.config/vyomaos-worker.env
# Fill VYOMA_MAX_RUNS_PER_DAY and VYOMA_RUN_MINUTES with the chosen limits.
install -m 644 ops/codex/vyomaos-worker.service ~/.config/systemd/user/
install -m 644 ops/codex/vyomaos-worker.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user start vyomaos-worker.service
journalctl --user -u vyomaos-worker.service -n 100
# After the pilot is verified:
systemctl --user enable --now vyomaos-worker.timer
```

Inspect and stop:

```bash
systemctl --user status vyomaos-worker.timer vyomaos-worker.service
cat /home/hbarve1/codes/vyomaos/out/codex-worker/latest
systemctl --user disable --now vyomaos-worker.timer
systemctl --user stop vyomaos-worker.service
```

For a pause between runs, create `out/codex-worker/STOP` in the main checkout. Resolve its reason before removing it. Never delete the state directory as a recovery step: it holds daily counters and the ownership lock. UTC is the budget day despite the Asia/Kolkata host display timezone. Lingering is already enabled; no root service is needed.

### Validation and next work

Run `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests/automation -v`. Eleven tests cover logs, explicit/invalid limits, daily persistence, STOP, lock exclusion, repeated failures, malformed results, timeout and review/blocker pauses. Codex and Docker are stubs: no model usage or service installation. A real pilot still needs to establish account limits, approval behavior, MCP initialization, labelled-container cleanup and interruption recovery.

Use [the pinned local build](local-build.md). T0-01 is merged and T0-02 supplies the tested parent. Continue with T0-03 kernel/config/rebuild correctness and T0-04 R1 image selection, then T0-05/T0-06 harness APIs/isolation/readiness. The current baseline's 1,154 passing Rust tests do not erase failing Python/guest checks. Resolve those before T0-GATE. Keep local logs private; prune only reviewed old run directories, preserving active locks and counters.
