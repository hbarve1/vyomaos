# Proposed Codex work loop for VyomaOS R1

Status: operating proposal, not an installed scheduler or an active background job.

## Recommendation

Use persistent, bounded milestone work to deliver the [terminal-first checklist](../TODOS.md). Give each run a concrete outcome and verification gate, preserve state in the repository, and deliver small PRs to `develop`. Prefer a completed and verified vertical slice over uninterrupted activity as the measure of progress.

Codex Goals preserve an objective across turns with completion criteria and lifecycle/budget controls. They are suitable for an individual milestone whose next action depends on test results. They are not a guarantee of indefinite unattended execution. [Official Goals guide](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex).

Scheduled tasks can start recurring work. Local-project tasks require the computer to remain on and the desktop app running; web tasks cannot directly operate on this local repository folder. Availability and permissions depend on the configured environment. [Official scheduled-task documentation](https://learn.chatgpt.com/docs/automations?surface=app).

## Proposed execution contract

1. Read `AGENTS.md`, `TODOS.md`, recent merged changes and open PRs before selecting work. Resume an existing task branch before creating duplicate work.
2. Select the next dependency-ready R1 task or a small coherent group with a single acceptance gate. Use an isolated worktree based on current `develop`.
3. Record the active task, branch, intended acceptance command and known prerequisites in the handoff table. Keep one active writer per task/worktree; scheduled starts must not overlap an existing run.
4. Implement the production path, add meaningful acceptance/regression tests, and run the applicable host/WASM/QEMU checks. Fix failures before expanding scope. Report missing prerequisites as blockers, never passes.
5. Record commands, results, commit and artifacts. Open/update a focused PR to `develop` when implementation PR creation is authorized. Do not mark the checklist item complete merely because the PR exists.
6. After review/merge, reconcile the checklist and continue with the next dependency-ready item. Pending review/merge is an integration dependency; work on independent items if useful. Stacked PRs require explicit dependencies and a tested base.
7. When a run ends, leave a concise handoff: completed work, test evidence, open PRs, failed attempts, unresolved blockers and the next exact task. Avoid leaving knowledge only in chat.

## Limits and operator decisions

- Set an explicit per-run token/time budget and a daily spend ceiling in the execution setup. Agree those values before enabling recurring execution; none is configured by this PR.
- Start with one milestone pilot, then enable recurrence after validating completion, failure and handoff behavior. Do not increase scope or budget automatically when a run exhausts its allowance.
- Limit repeated attempts at the same failure. After three materially equivalent failures, record the evidence and change approach or mark that task blocked; continue independent authorized work where possible.
- Keep review/merge and release authority explicit. The current request authorizes this planning PR; it does not configure automatic merges or publication of OS releases.
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
