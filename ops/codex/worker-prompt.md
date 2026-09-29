Continue VyomaOS R1 terminal-first development using TODOS.md.

This is one bounded invocation of a recurring worker on the maintainer's always-on
Linux machine. Read AGENTS.md, TODOS.md, docs/local-build.md and
docs/codex-work-loop.md. Read VYOMA_RUN_ID and VYOMA_RUN_DEADLINE_UTC.
Checkpoint and finish before the deadline; never extend the wrapper's budget,
reset counters, remove STOP, change the timer, or spawn another worker.

1. Inspect git status, worktrees, current develop and open task PRs. Resume the
   most recent unfinished task before creating work. Never reset/stash/discard
   user changes. Leave the main checkout on develop and clean.
2. Reconcile merged tasks from actual GitHub merge status and recorded evidence.
   Select the next dependency-ready TODOS item. Work in .worktrees/<task>.
   Stacked PRs are authorized: branch from the tested parent tip, target that
   parent branch, and name parent PR/base SHA in the child description. Keep
   at most three open task PRs in a stack. Independent PRs target develop.
3. Implement a coherent testable slice. Run all applicable existing suites
   locally; never dispatch GitHub Actions. Include [skip ci] in commits while
   Actions is billing-blocked. Do not hide failures or count skips as passes.
   Test state must be disposable; no user disks/data or real-machine installation.
   Use Docker labels vyomaos.worker-run=$VYOMA_RUN_ID on test containers and
   remove your containers/VM processes before returning.
4. Review the diff, preserve logs/commands/versions/artifact digests, commit and
   open/update the PR. Use a body file for multiline descriptions. Do not merge,
   publish a release, change branch protections or auto-merge settings.
   Do not mark a task complete until acceptance evidence exists and it merged.
5. Continue to another slice if time permits, including stacking while a tested
   parent awaits review. Pause only when the stack reaches its limit, all useful
   work needs an external decision, or the run deadline approaches.
6. After a parent merges, preserve only the child's commits with an explicit
   rebase --onto origin/develop <recorded-old-parent-sha> <child>. This matters for
   squash merges. Retest, push with an explicit force-with-lease expected SHA,
   and retarget to develop. Never merge children into temporary parent branches.
   If a collaborator changed the branch, leave a blocker instead of overwriting.
7. Update TODOS handoff with branch/base/parent/head, PR, evidence, exact blocker
   and next action. Recover interrupted work from worktrees and logs; a zero
   process exit code alone is not acceptance. Avoid repeated equivalent failures.
   Do not deploy/enable/edit this worker's own control files.

Return the required JSON. status=progress means another invocation can continue,
waiting_review means the three-PR stack is full with no useful independent work,
and blocked means external input/environment repair is needed for all available
work. waiting_review/blocked pause the runner via STOP. Describe the unblocking
action. Near a time limit, preserve partial work and use progress.
