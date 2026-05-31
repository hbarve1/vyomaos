# Critique: App Lifecycle Management (Round 22)

## Verdict

The spec is well-motivated and covers all the right territory: state enumeration,
graceful termination, restart backoff, reload, WIT interface, watchdog integration, and
WM coupling. The decision to store lifecycle state directly in `AppState` rather than
a separate lifecycle registry avoids unnecessary lock layering at this scale. The non-
blocking termination design — recording a `termination_deadline` and polling it on each
supervisor tick — is the correct pattern for a single-threaded event loop and avoids
the classic pitfall of blocking the compositor during a shutdown race.

However, five blocking issues prevent safe implementation: the state transition graph is
incomplete and closes off valid paths that real apps will exercise (Suspended → Background
transition undefined; kill-while-Launching undefined); the `will_terminate` grace period
creates a watchdog double-kill window that is only partially addressed by the spec and
requires an in-thread state re-check to close; the reload sequence has an undefined failure
mode when a per-app vyoma.toml is malformed mid-edit, causing a partial kill-without-start
outcome that violates the reload atomicity guarantee; the backoff reset rule uses
`spawn_time` in its body but the 60-second alive-check is never evaluated for apps that
have not yet exceeded any backoff step, creating an invisible gap that requires an explicit
tick-function call site in §12; and the `notify-ready` vs first-flush ambiguity means two
independent code paths can race to transition the same app from `Launching` to `Running`,
with the second transition being a silent no-op that masks bugs unless a consistent
APPS_MAP write-lock discipline is specified. None of these require redesigning the core
model. They are precision gaps requiring targeted additions to §2, §4, §6, §5, and §8.

---

## Blocking Issues

### B1: Incomplete State Transition Graph — Suspended Cannot Reach Background

Section 9.3 provides a transition table for space-switch events but it is not a complete
transition graph. It defines the happy path only. Two critical omissions will manifest in
the first week of real use.

**Omission A: Suspended → Background transition.**
Consider an app that starts before the user has set background policy (no WIT call, no
`VYOMA_LIFECYCLE:background_opt_in` on stdout). The app's space becomes inactive:
supervisor transitions it to `Suspended`. The app eventually starts (once it becomes
`Running` again after a space switch back) and calls `set-background-policy(true)` for
future space-switch events. Now the user switches away from the space again. Section 9.1
defines the rule for "app has NOT set background policy → Suspended" and section 9.2
defines "app HAS set background policy → Background." But the transition table in §9.3
does not have a row for `Suspended → Background` caused by calling `set-background-policy`
while already Suspended. Section 7 (`set-background-policy`) says: "If called while
already Suspended, the app transitions immediately to Background." But §9.3's table
contradicts this by not including the row. The implementation will have a branching
inconsistency between the WIT host code (which would implement the §7 text) and the
space-switch handler (which reads the §9.3 table). One of them will be wrong.

The fix: add the following rows to the §9.3 transition table:

| Current State | Event                                 | Next State  |
|---------------|---------------------------------------|-------------|
| Suspended     | `set-background-policy(true)` called  | Background  |
| Background    | `set-background-policy(false)` called | Suspended   |
| Launching     | `kill` issued                         | Terminating |

The third row closes the Launching → Terminating gap (Omission C). All three rows are
reachable in normal operation and must be handled in the WIT host, the IPC kill handler,
and the space-switch handler respectively. Omitting them means the implementation will
reach an unhandled state match arm and either panic or silently do nothing.

**Omission B: Terminating app receives IPC message.**
Section 2.2 shows that IPC is NOT delivered in `Terminating` state. But §4.1 does not
specify what happens to an IPC message that arrives in the IPC broker for an app that
has just transitioned to `Terminating`. The broker may have queued the message before
the state flip. Does it: (a) discard the message silently, (b) return an error to the
sender, or (c) deliver it anyway because the stdin pipe is still open? The `will_terminate`
sequence explicitly keeps stdin open (to write the `will_terminate` event). A message
queued at t=0 and delivered at t=0.001 — after state flip — may arrive in the stdin
buffer alongside `VYOMA_LIFECYCLE:will_terminate`. The app may interpret this as: "I got
a message AND a terminate signal; I should handle the message first." That delay pushes
the voluntary exit past the 2-second window, triggering SIGKILL unnecessarily.

The fix: when an app transitions to `Terminating`, the IPC broker must atomically stop
enqueuing new messages for that app. Messages already in the queue at the time of state
transition are dropped. The `will_terminate` write must be the LAST write to stdin.

**Omission C: What if the target app is in Launching state when kill is issued?**
Section 4.1 Step 1 says "Validate app exists and is not already Terminating or Terminated"
but does not mention `Launching`. If kill is issued before the app produces its first
output (i.e., while in `Launching`), should the supervisor skip the `will_terminate` event
(the app may not yet be reading stdin) and go straight to SIGKILL after the 2-second wait,
or should it write `will_terminate` optimistically? The spec must add `Launching` to the
transition table as a valid source state for the kill command, with the note that
`will_terminate` is written optimistically and the 2-second wait applies normally.

---

### B2: `will_terminate` Race with Watchdog — Partial Fix Is Insufficient

Section 10.2 states: "The watchdog is paused in `Terminating` state." This is the correct
fix. But the spec does not specify the ordering between the watchdog pause and the
`will_terminate` write. The race is:

```
Thread A (event loop):          Thread B (output-reading thread):
─────────────────────────────   ──────────────────────────────────
1. Transition to Terminating
2. Write will_terminate → stdin
                                3. App reads will_terminate, stops output
                                4. Output silence starts (t=0 from watchdog perspective)
5. Set watchdog_paused = true   ← RACE: this happens AFTER step 2
```

If the watchdog runs on the same thread as the event loop (as implied by the single-
threaded model), steps 1-2-5 happen atomically within a single tick and the race does not
exist. But the output-reading thread in the supervisor runs on a separate OS thread (each
app has a dedicated output-reading thread, per the architecture). The watchdog check is
inside that output-reading thread. So the window is:

- Step 2 completes at time T.
- The app stops writing stdout at time T+ε (response to `will_terminate`).
- Step 5 (`watchdog_paused = true`) occurs at time T + (one event loop tick latency).
- If the event loop tick latency exceeds `watchdog_secs` (unlikely, but possible if the
  event loop is under load processing many other apps), the watchdog fires before
  `watchdog_paused` is set.

For apps with `watchdog_secs = 1` (very tight watchdog), the tick latency does not need
to be large; a single stalled app could delay the event loop by 100–200 ms, leaving a
900ms window where the watchdog can fire.

The fix is simple: in the output-reading thread's watchdog check, before firing, re-read
`AppState.lifecycle` under a read lock. If the state is `Terminating`, `Suspended`, or
`Launching`, skip the watchdog kill regardless of `watchdog_paused`. This dual-check
makes the watchdog state-aware in-thread, not just event-loop-aware, eliminating the
race entirely. The `watchdog_paused` field then becomes a derived optimisation rather than
the sole guard. The dual-check has a negligible performance cost: a single read-lock
acquisition and enum comparison, executed at most once per `watchdog_secs` interval per
app. For the default watchdog_secs of 30, this is effectively zero overhead.

The spec must add a code sample showing the watchdog check:

```rust
// Inside output-reading thread watchdog logic
let state = apps_map.read().get(app_name).map(|a| a.lifecycle);
if matches!(state, Some(Suspended | Terminating | Launching)) {
    continue; // skip watchdog kill regardless of silence duration
}
```

---

### B3: Reload Atomicity — Truncated File Leaves Removed Apps Killed, New Apps Unstarted

Section 6.4 states: "if boot.toml is partially written, the TOML parser returns a parse
error at Step 1, reload aborts, no app state is changed." This is true for Step 1.
However, the spec allows per-app manifest parse errors in Step 2 to be partial failures:
"skip that app, log warning, continue with others."

This creates a combined failure mode that is worse than either a full abort or a full
success:

1. boot.toml parses successfully (valid TOML).
2. The new boot.toml lists apps A, B, C. The currently running apps are A, B, D.
3. D has been removed, so Step 3 marks D for killing.
4. C is new. C's `vyoma.toml` is malformed (user is still editing it).
5. Step 4: D is gracefully killed (permanent, D's process is gone).
6. Step 5: C is skipped (malformed manifest). A and B are unchanged.
7. Result: D is dead, C was never started. The system is in a partially-reloaded state
   with a missing app — the opposite of the intended outcome.

The spec says reload is "best-effort per app" but does not define the semantics of
"best-effort" when the missing app is critical. It also does not define whether D's
removal is rolled back if C's manifest fails. There is no rollback mechanism.

The fix requires choosing one of two explicit policies:

**Policy A (all-or-nothing at Step 2)**: Before killing any app (Step 4), validate ALL
manifests in the new boot.toml. If any manifest is unparseable, abort the reload entirely.
No app is killed, no app is started. Return an error listing which manifests failed.

**Policy B (commit only what is safe)**: Keep the best-effort model, but only kill
removed apps AFTER the new apps have been verified to start successfully (start them
first, wait for `VYOMA_LIFECYCLE:ready`, then kill old apps). This is the zero-downtime
approach described in §16.2 (R28), which R22 explicitly defers.

Policy A is recommended for R22. It is simpler, matches the stated invariant I8
("reload aborts cleanly on TOML parse error with no state change"), and aligns with the
principle of least surprise. The spec should change "partial failure" semantics to
"if any manifest in the new set fails to parse, reload aborts without killing any app."

The spec must also address an edge case in the Step 4 timeout: if the graceful termination
of removed apps takes more than 5 seconds (Step 4 timeout), the new apps in Step 5 may
start while the old apps are still running (surviving SIGKILL in D state, as described in
§4.4). For apps that bind a port or hold an exclusive resource (e.g., `/dev/fb0`), two
instances of the same app running simultaneously would cause immediate startup failure in
the new instance. The spec should note this as a known limitation: "reload does not
guarantee exclusive resource handoff; if an old instance survives SIGKILL, the new instance
may fail to start and will be logged as an error."

---

### B4: Backoff Reset Race — 60-Second Check Is Never Evaluated for Apps Under the Cap

Section 5.2 states: "when an app has been alive continuously for ≥ 60 seconds,
`restart_count_window` is reset to 0." The check is evaluated "on each supervisor tick."

The gap: this check is only relevant for apps that have been restarted at least once
(i.e., `restart_count_window > 0`). For apps on their first launch (fresh boot, never
crashed), `restart_count_window` is already 0. Resetting it to 0 is a no-op, so the
tick-based check does nothing harmful. But the spec never states WHERE in the event loop
this check runs. It says "checked on each supervisor tick" without specifying the tick
function or call site.

More critically: the check must run even when the app has NOT exited. It is not triggered
by an exit event; it is a time-based poll. If the supervisor's tick function only runs the
backoff-reset check when processing an exit event (the natural place to think about
restarts), the reset will never fire for apps that are running correctly. The entire
backoff reset relies on a proactive time-based scan.

The spec must explicitly state which function performs this check and that it runs on
every supervisor tick unconditionally, not only on app exit. The recommended call site:

```rust
// In supervisor/src/lifecycle/restart.rs
pub fn check_backoff_resets(apps: &mut AppsMap, now: Instant) {
    for app in apps.values_mut() {
        if app.restart_count_window == 0 { continue; }
        let Some(spawn_time) = app.spawn_time else { continue; };
        if now.duration_since(spawn_time) >= Duration::from_secs(60) {
            app.restart_count_window = 0;
            app.next_restart_at = None;
        }
    }
}
```

This function must be called from the main event loop tick, not from the exit handler.
The spec must add this call to §12.1 "Integration Points in Existing Files."

A second precision gap: the spec defines `spawn_time` as `Option<Instant>` — it is `None`
for apps that have never been spawned (new entries created at boot before first launch).
If the check runs before the first spawn, `spawn_time` is `None` and the check must skip
the app. The spec does not mention this guard. The fix: add `let Some(spawn_time) = ...
else { continue; }` guard explicitly to the spec's pseudocode.

A third precision gap: what should happen to `restart_count_window` during the period
between spawn and the first supervisor tick that runs the reset check? The field starts
at 0. If the app crashes before the first tick, it restarts with `restart_count_window = 1`
(correct). If the app survives 60 seconds and the tick resets it to 0, then the app
crashes, it restarts at `restart_count_window = 0` → delay 0 (first restart with no
penalty). This is the intended behaviour but the spec does not trace this path explicitly.
The spec should add a numbered trace of the nominal crash-reset-crash sequence to confirm
the design intent, e.g.: "App crashes at t=0 → `restart_count_window=1`, restart delay=1s.
App survives until t=61 → reset to 0. App crashes at t=62 → `restart_count_window=1`,
restart delay=1s (treated as fresh first crash)." This trace makes the design testable.

---

### B5: `notify-ready` vs First Flush — Race on `Launching → Running` Transition

Section 2.3 defines two triggers for the `Launching → Running` transition:

1. `VYOMA_LIFECYCLE:ready` on stdout.
2. `VYOMA_DRAW:flush` on stdout.

Both are parsed from the same stdout stream in the same output-reading thread. In most
cases they will not race because a single-threaded app produces them sequentially. But
two scenarios cause genuine ambiguity:

**Scenario A: flush before ready.** A display app writes its splash screen
(`VYOMA_DRAW:fill_rect ...`, `VYOMA_DRAW:flush`) during initialisation, then calls
`notify-ready` via WIT (not via stdout). The flush arrives on the output-reading thread,
triggers `Launching → Running`. The WIT `notify-ready` arrives on the Wasmtime calling
thread, also tries to trigger `Launching → Running`. The second transition hits a state
that is already `Running`. Section 2.3 says "calling this when already Running is a no-op"
for the WIT call — so this is handled. But the same guarantee is not stated for the stdout
`VYOMA_LIFECYCLE:ready` parser path. The display.rs parser calls
`lifecycle::events::on_ready(app)` — is that function also a no-op if state is already
Running? The spec does not state this.

**Scenario B: ready before flush, but display expects flush to gate Running.** Consider
a daemon app that has `display = false`. It correctly writes `VYOMA_LIFECYCLE:ready` when
it finishes initialisation. The supervisor transitions it `Launching → Running`. Good.
Now consider a display app that writes `VYOMA_LIFECYCLE:ready` first (before any flush)
because it wants to receive input events immediately (e.g., it renders based on input).
The spec says the first flush also triggers `Running`. Both are valid triggers. But neither
takes precedence over the other in the spec. If the app writes `ready` at t=0 and the
compositor reads it and transitions to `Running`, then later at t=1 the app writes its
first `VYOMA_DRAW:flush` — the flush trigger path sees state=Running and should skip the
transition. The spec must explicitly state that `on_ready` (both WIT and stdout paths) and
`on_first_flush` are both no-ops when state is already `Running`.

**The deeper issue**: the spec defines `Running` transition as "whichever arrives first"
but does not specify the critical path: what if `notify-ready` arrives while the output-
reading thread is in the middle of parsing a multi-line `VYOMA_DRAW:` sequence? The WIT
host runs on the Wasmtime execution thread, which is different from the output-reading
thread. Both threads share `APPS_MAP`. Without a clear lock-ordering statement, the
transition is a data race.

The fix: `on_ready` (in `lifecycle/events.rs`) must acquire a write lock on `APPS_MAP`,
check `state == Launching`, and only then transition to `Running`. This must be explicitly
stated in the spec with the lock name. The same `APPS_MAP` write lock used by
`ipc_handlers.rs` must be used here — the spec must call out that this is the same lock,
not a new lock, to prevent ABBA deadlocks.

A consistent locking protocol for all lifecycle transitions — always under `APPS_MAP`
write lock, never under any other lock — is the simplest way to eliminate all races in
the lifecycle subsystem. The spec should state this as an implementation constraint in
§12: "All `LifecycleState` transitions must occur under an exclusive write lock on
`APPS_MAP`. No lifecycle transition may be performed while holding any other lock."
This single rule eliminates the B5 race, eliminates the B2 partial-fix concern, and
provides a clear audit trail for future reviewers.

---

## Non-Blocking Issues

**N1: 500ms Launching fallback is under-specified.**
Section 2.3 says apps stuck in `Launching` for 500ms are transitioned to `Running` with
a `WARN` log. But the timer for this fallback is not mentioned in §12.1 (integration
points). It needs to be a time-based check in the main event loop tick, similar to the
backoff-reset check. Every app that enters `Launching` must record a `launch_start_time`
in `AppState`. The event loop tick must compare `now - launch_start_time` against 500ms.
If exceeded, transition to `Running` and emit the warn log. The spec should explicitly
list this check in §12 "Integration Points" alongside the backoff-reset check.

**N2: `restart <app>` operator command resets hourly cap — security implication.**
An operator who keeps issuing `restart <app>` manually can bypass the hourly cap
indefinitely. This is acceptable in the R22 threat model (trusted operator console) but
should be noted as a known deviation from the cap's intent. A future round should add
a `--force` flag to distinguish "I know it exceeded the cap" from "I want normal restart
semantics." Without this distinction, the hourly cap provides no protection against
operators inadvertently clearing it during routine maintenance.

**N3: `Background` apps emit draw commands that are parsed but discarded.**
Section 9.2 says "Draw commands during Background state are discarded after parsing."
Parsing is not free — the supervisor still does string parsing on every `VYOMA_DRAW:` line
from the background app. A misbehaving background app could emit draw commands at 60fps
and burn CPU on the parsing side. The spec should note that a future round will add rate-
limiting for draw commands from Background/Suspended apps, or that the background app's
stdout pipe should be drained without parsing draw commands. One concrete option: prefix
parsing should branch earlier — if the first token is `VYOMA_DRAW:` and the app is in
`Background` or `Suspended` state, skip the full parse and discard. This still requires
a single string comparison per line but avoids the full `draw_cmd.rs` parse path.

**N4: `lifecycle <app>` command output format.**
Section 14.3 defines an inline format `state=Running watchdog_active=true ...`. For
consistency with all other supervisor IPC responses (which use unstructured natural
language), consider whether this should also be natural language or explicitly a key=value
format for machine parsing by future inspector apps. The R25 process inspector app (noted
in §16.3) will machine-parse this output. If the format is not locked down now, R25 will
have to reverse-engineer or re-specify it. This is a style question, but locking down the
format here prevents a R22/R25 compatibility break. The recommendation: define the output
as a structured key=value line with a guaranteed field order.

**N5: WIT `get-state` latency under high APPS_MAP contention.**
`get-state` acquires a read lock on `APPS_MAP`. If the compositor's flush pass holds a
read lock on the same map (which it does, per R21 design), `get-state` from a Wasmtime
thread will contend with the compositor. This is expected to be infrequent, but apps that
call `get-state` in a tight polling loop will add read-lock pressure to the compositor
path. The spec should recommend that apps call `get-state` only in response to lifecycle
events, not in a polling loop. An alternative implementation that avoids this contention:
cache the lifecycle state in a separate `AtomicU8` per app that the WIT host reads without
acquiring APPS_MAP. The compositor-facing read of full AppState fields (surface, z-order,
etc.) would still go through APPS_MAP as normal. This optimisation is not required for
R22 but the spec should note it as a future improvement path.

**N6: Termination audit log.**
The spec says the supervisor logs when an app is forcibly killed (`[lifecycle] <app>
forcibly killed after 2s grace period expired`). But there is no equivalent log for clean
voluntary exits. Voluntary exits are equally important for debugging (an app that exits
immediately may be crashing, not finishing its work). The spec should require a structured
log line for ALL app exits — voluntary and forced — including the exit code and the
lifecycle state at the time of exit. Format:
`[lifecycle] <app> exited code=<n> state=<state> uptime=<s>s`

---

## What the Spec Gets Right

**Correct non-blocking termination model.** The decision to record
`termination_deadline` and poll it on each supervisor tick is exactly right. It avoids
the antipattern of `thread::sleep(2s)` inside the kill handler, which would stall the
compositor for 2 seconds on every kill command. This is the correct architecture for an
event-loop-based supervisor. The PendingRestart list approach for restart scheduling
(§5.4) uses the same pattern — monotonic timestamps polled on each tick — maintaining a
consistent event-loop discipline throughout the lifecycle subsystem.

**Correct `spawn_time` choice for backoff reset.** The spec explicitly justifies using
`spawn_time` rather than `last_output_time` for the 60-second alive check. A quiet daemon
that does its job silently should have its backoff reset after 60 seconds even if it
produces no output. The footnote explaining this choice will prevent an implementer from
"optimising" to `last_output_time` and breaking quiet daemons. This level of explicit
justification for a non-obvious decision is good spec hygiene.

**Correct watchdog pause semantics.** Pausing the watchdog in Suspended, Terminating,
and Launching states, while keeping it active in Background, is the right design. A
Background app is making a claim that it remains responsive; holding it to the watchdog
contract enforces that claim. The table in §2.2 and the text in §10.2 are consistent
with each other on this point. The rule "last_output_time is reset to now when watchdog
is un-paused" (§10.3) correctly prevents the false-alarm scenario where a long-suspended
app is immediately watchdog-killed the instant it resumes.

**Right decision to exclude cross-app lifecycle queries from WIT.**
Limiting `get-state` to self-queries only is correct for R22. Cross-app lifecycle
inspection would require capability enforcement (app A should not spy on app B's state),
which is a non-trivial capability policy question deferred appropriately. The WIT
interface is minimal and purposeful: ready, terminate, background policy. Nothing more.

**Correct reload abort semantics at the TOML parse level.**
Section 6.4's "if boot.toml is unparseable, no state changes" rule is implementable as
a clean read-parse-validate-then-act sequence. The invariant I8 is verifiable in unit
tests. This is the right design for an operator-facing command that could be run on a
broken config file. The explicit list of what counts as a "changed" manifest (§6.3) is
also well-defined and avoids the ambiguity of "any difference triggers restart" vs
"only breaking differences trigger restart."

**Sensible hourly cap value.**
10 restarts per hour for `restart = "always"` apps is a reasonable balance between
fault tolerance and resource protection. It allows up to 10 crash-restart cycles before
giving up, which is enough for apps that fail transiently during boot (e.g., waiting for
a 9P mount to become available) without allowing unbounded crash loops. The choice to
use a rolling hourly window (rather than a fixed clock-hour window) prevents the edge
case where an app crashes 9 times at 11:59pm and 9 times at 12:00am without triggering
the cap in either clock-hour window.

**Invariant table (§17) is testable.**
The 12 invariants in §17 are stated in a verifiable form (iff, exact state names, exact
behaviour). Each can be translated directly into a unit test assertion. This level of
precision in the invariants section is above average for this spec series and will
accelerate test authorship.

---

## Open Questions

**Q1.** Should `reload` support a `--dry-run` flag that prints the diff (started=N,
killed=M, unchanged=K) without applying it? This is useful for operators who want to
preview the impact before committing. R22 does not need to implement it, but the reload
sequence in §6.2 should reserve the concept so that the command parser can be extended
without a breaking syntax change.

**Q2.** When an app in `Terminated` state (due to hourly cap exceeded) is revived by
`restart <app>`, what lifecycle state does it start in: `Launching` or `Running`? The
spec implies `Launching` (it's a fresh spawn), but this should be stated explicitly.
This also determines whether the 500ms fallback timer starts immediately.

**Q3.** The spec says `VYOMA_LIFECYCLE:running` is reused as a focus-gained notification
(§11.2). This conflates two concepts: state change and focus change. When R23 introduces
`VYOMA_LIFECYCLE:focus_gained`, apps that implement R22 lifecycle handling will need to
differentiate "I was in background and came to foreground" from "I already was Running
and just gained focus." Will R23 deprecate the `running` re-use for focus gain, and if so,
will that be a breaking protocol change for R22-era apps? The spec should note the intent
to replace this with a dedicated event in R23 to set expectations for app authors.

**Q4.** What happens to the 9P filesystem connection for an app that is Suspended? The
9P mount is provided by Wasmtime; the Wasmtime process is still alive during Suspended.
Filesystem access by a Suspended app (if it somehow runs code — e.g., via a Wasm timer
or a WASI thread still running) is technically possible. The spec does not address whether
filesystem I/O is throttled or blocked for Suspended apps. Is this an enforcement gap or
intentional? If intentional, add it to the "What R22 Does Not Do" list in §1.

**Q5.** The spec defines `Suspended` as "receives no keyboard or mouse input." But what
about the app that holds keyboard focus? If the active window (focused app) is on Space 1
and the user switches to Space 2, does the focused app get Suspended? If so, keyboard
events typed on Space 2 go... nowhere? Or does focus automatically shift to the topmost
Running app on Space 2? Section 11.2 covers the termination focus-shift case but not the
space-switch focus-shift case. The WM integration section should specify that a space
switch triggers the same focus-reassignment logic as termination: find the topmost Running
app in the newly active space and assign focus.

---

## Closing

The R22 spec is implementable once the five blocking issues are resolved. The fixes are
additive: no section needs to be rewritten, only extended with explicit transition rows
for Suspended → Background and Launching → Terminating (B1), a thread-safe dual-check
for watchdog using an in-thread state read (B2), all-or-nothing manifest validation in
reload before any kill is issued (B3), an explicit tick-function call for backoff reset
with spawn_time None guard and nominal-path trace (B4), and an explicit APPS_MAP write-
lock ordering statement plus no-op guarantee for the dual ready-trigger paths (B5).

The non-blocking issues are primarily about precision and forward compatibility rather
than correctness. The `lifecycle <app>` command format (N4) and the focus-on-space-switch
gap (Q5) should be addressed in the next revision to avoid retrofitting them when R23
and R25 arrive. The 500ms fallback timer gap (N1) is the most likely to cause a test
failure at integration time; it should be treated as blocking-adjacent and resolved
before implementation begins.

Overall, the spec represents a significant leap in VyomaOS supervisor sophistication.
Formalising lifecycle states removes the last major source of undefined behaviour in the
app model: what the supervisor does when an app is silent, dying, or moved to an inactive
space. After R22, the supervisor will have a complete, testable, documented answer to
every lifecycle question that R23 (chrome) and R24 (dock) will need to build on. The
12-invariant table, the non-blocking termination model, and the explicit backoff justification
make this one of the more carefully reasoned rounds in the series. Resolve the five
blocking issues and this spec is ready to implement.

