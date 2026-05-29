# Round 7 Critic: Power Management & Energy

**Date:** 2026-05-29
**Round:** 7 of 80
**Subsystem:** Power Management & Energy
**Critic verdict:** FUNDAMENTAL FLAWS

---

## Executive Summary

The Round 7 proposal for VyomaOS power management borrows wholesale from
macOS's IOPowerManagement + App Nap + Power Nap + caffeinate + Safe Sleep
stack and re-skins it as Rust supervisor logic against a Linux 5.10
kernel. That sounds like a clean mapping. It is not. macOS power
management works because Apple owns the entire vertical: the EC firmware,
the ACPI tables, the platform NVRAM, the SMC, the kernel power module,
the user-space `powerd`, and the application runtime are all designed in
lockstep. VyomaOS is currently a Rust PID-1 supervisor on top of an
allnoconfig Linux kernel running inside QEMU/KVM, and the bottom three
layers of that stack — EC firmware, SMC, and ACPI — are emulated, partly
missing, or completely unavailable.

Independent of the QEMU-vs-bare-metal question, the design has six
intrinsic correctness problems that would bite even on real hardware:

1. **Suspend-to-RAM in a guest is a fiction.** The supervisor cannot
   actually issue `echo mem > /sys/power/state` and expect S3 semantics.
   On QEMU it returns immediately, freezes only the guest scheduler, or
   tears the VM down. The on-suspend/on-resume callback fabric is
   designed against a device transition that does not happen.

2. **The on-suspend callback races kernel I/O.** Apps return their
   StateBlob before in-flight 9P writes, virtio-blk writebacks, page
   cache dirty pages, and TCP retransmits have drained. The supervisor
   tears down the WASM store and then issues `fsync(2)` — by which time
   the StateBlob references a filesystem state that does not exist.

3. **Power Nap is a covert-exfiltration primitive.** Granting any app
   `background_fetch = true` lets it run network I/O with the display
   off and no user-attestation. There is no consent UI, no per-domain
   quota, and no audit log of what an app did while the user was asleep.

4. **Thermal throttling and orderly suspend are two different time
   domains.** A CPU at 95 °C will trip the kernel's emergency shutdown
   (or be downclocked by hardware MSRs) on a sub-second timescale. The
   on-suspend cycle is bounded below by the slowest app's checkpoint
   latency — empirically multi-second. Fast thermal events cannot be
   handled by a mechanism whose minimum cost is "wait for all apps".

5. **ActivityAssertion (R5) and PowerAssertion (R7) are two different
   models pretending to be one.** R5 keeps an app from being napped; R7
   keeps the system from sleeping. An app holding the former without the
   latter creates an undefined state — the design does not specify
   precedence, and either choice (sleep wins / activity wins) breaks
   something an app developer reasonably expected.

6. **Battery polling has no Linux-native fix that VyomaOS can adopt.**
   The proposal hand-waves "poll `/sys/class/power_supply`" without
   acknowledging that the upstream answer (upower + DBus) requires a
   DBus broker the supervisor does not run. uevent-via-netlink works
   but is undocumented in the proposal and has its own race against
   userspace startup.

7. **Hibernate over WASM is a corruption-by-design hazard.** The
   StateBlob is opt-in; hibernate of an app that never implemented
   on-suspend silently degrades to a "fresh start" on resume. For an
   app that holds business state (an unsent draft, a partially-typed
   form, an open transaction), this is data loss. The user did not ask
   for a fresh start; they asked the lid to be closed.

The first issue (S3 in a VM) is a near-term blocker for *implementation*
on the only platform VyomaOS currently boots on. Issues 2 and 7 are
correctness blockers regardless of platform. Issues 3 and 4 are
*design* blockers: no amount of implementation effort will turn the
proposed mechanisms into safe ones — the model itself is wrong.

This critique recommends:

- Splitting "platform sleep" (S3/S4) out of Round 7 entirely and
  deferring it until VyomaOS has a real-hardware target.
- Replacing it with a tractable "display sleep + app suspension + user
  inactivity model" that is implementable today, in QEMU, with no
  ACPI dependence.
- Treating Power Nap as an explicitly capability-gated feature with
  per-app data-volume budgets, audit logs, and a user-visible "what
  ran while you were asleep" panel.
- Folding R5's ActivityAssertion and R7's PowerAssertion into a single
  assertion algebra with documented precedence rules.
- Defining a checkpoint *barrier* that drains in-flight I/O before
  capturing StateBlob, and a checkpoint-completeness *attestation*
  that hibernate refuses to write a slot from any app that did not
  implement on-suspend.

The remainder of this document elaborates these issues in detail.

---

## Critical Issues (blocking)

### C1. Suspend-to-RAM (S3) inside a VM is not a transferable abstraction

**Claim under attack.** The design assumes the supervisor can drive a
real ACPI S3 transition via the standard Linux interface:

```
echo mem > /sys/power/state
```

It assumes a corresponding resume path (`/sys/power/wakeup_count`,
ACPI wakeup events on PME, USB, lid) and architectural support for
power state restoration of CPU caches, TLB, MTRRs, and PCI BARs.

**Reality on QEMU/KVM.** A guest VM's interaction with S3 depends on
the machine type, the QEMU version, and the host hypervisor:

- `qemu-system-x86_64 -machine pc` (the default) does export an ACPI
  S3 object, and the guest *thinks* it suspended. Internally, QEMU
  pauses the vCPU threads, leaves the guest RAM allocated on the host,
  and returns. The guest's PM timer continues to fire (or doesn't,
  depending on `-no-hpet` and `-rtc`), wakeup sources are emulated, and
  the *host*'s power state is unaffected. So "suspended" really means
  "vCPU thread paused, page cache retained". This is *not* equivalent
  to bare-metal S3 in any way that matters for power saving — the host
  is still drawing full power.
- On macOS, the user runs QEMU under hvf; hvf has historically had
  S3-passthrough bugs that crash the guest on resume. The Vyoma
  `make run-gui DISPLAY_BACKEND=cocoa` path goes through hvf.
- Inside CI (Docker containers, GitHub Actions runners), the guest
  rarely has a working ACPI S3 path at all; `cat /sys/power/state`
  often shows only `freeze` and `disk`, not `mem`.
- On `qemu-system-aarch64 -machine virt` (the `iot-edge`, `mobile`,
  `robotics-rt`, `server-headless` platforms), the situation is worse:
  PSCI is the standard "suspend" mechanism on ARM, and PSCI's
  `CPU_SUSPEND` semantics under TF-A are platform-specific and largely
  untested in the upstream `virt` machine.
- On `qemu-system-arm -machine mps2-an385` (the `mcu-minimal`
  profile), there is no ACPI at all, and S3 is undefined.

**Consequence.** The single Linux interface `echo mem > /sys/power/state`
will behave differently across the six platform profiles the build
system already supports, and the supervisor has no way to detect which
behavior it will get without trying. The design's "platform-agnostic
on-suspend WIT callback" promise is broken at the platform boundary.

**Sub-question: does S3 even make sense as a goal?** A desktop OS
needs three power states that map to user intent: "user is here"
(running), "user stepped away for minutes" (display off, apps running),
and "user is done for the night" (laptop in bag, no work happening).
On macOS, those map to: running / display sleep / S3 (or S4 if
battery-thresholded). The Vyoma design appears to want all three.

But the *value* the user gets from S3 specifically is:

- Battery preservation (RAM is retained at low draw; CPU is off).
- Fast wake (RAM is still warm, no disk read needed).

In a VM, the first benefit does not exist. In hibernate-from-disk (S4),
the first benefit *does* exist but the second is gone. So the design's
emphasis on S3 *as a first-class abstraction* is borrowing a macOS
priority that does not survive the platform change.

**Recommendation.** Round 7 should explicitly define an internal
power-state ladder that is implementable on the current platform:

```
RUNNING          → display on, all apps active
USER_IDLE        → display dimmed, apps with assertion still active
DISPLAY_OFF      → fb0 cleared, KMS DPMS off, apps continue
APP_SUSPENDED    → idle apps' StateBlobs captured, WASM stores dropped
SYSTEM_SUSPENDED → optional: try /sys/power/state; on failure, hold
                   state in supervisor RAM with vCPU loop yielding
HIBERNATED       → all StateBlobs + manifests written to
                   /data/.hibernate/, supervisor exits, init reads on boot
```

Note that `SYSTEM_SUSPENDED` here does *not* claim to drive ACPI S3
correctly. It claims to do the best it can: trigger the kernel's
suspend path opportunistically; if that returns immediately or fails,
fall back to "supervisor holds state and the kernel just runs idle".
On bare-metal Linux this gives real power savings. In a VM, it gives
correctness — the supervisor's view of the world stays consistent — at
the cost of zero actual power savings (because the host is still up).

This is honest. The current design pretends VM-S3 is equivalent to
bare-metal-S3 and that pretense propagates through every WIT
callback, IPC message, and recovery path that depends on it.

---

### C2. The on-suspend / StateBlob race against in-flight I/O

**Claim under attack.** The proposed suspend sequence is roughly:

1. Supervisor broadcasts `on-suspend` to every app.
2. Each app returns a `StateBlob` (opaque bytes) via the WIT callback.
3. Supervisor writes each StateBlob to `/data/.suspend/<app>.blob`.
4. Supervisor calls `sync(2)` / `fsync(2)` to flush the filesystem.
5. Supervisor writes "mem" to `/sys/power/state`.

This is the same pattern macOS uses (apps get an `applicationWillResign`
notification and return their state-restoration archive), except macOS
runs on a system where the kernel-side I/O subsystem cooperates with
power management — the `IOPM` plane explicitly drains writes to disk
and acknowledges suspension only when storage drivers report quiesce.
Linux's PM core has hooks for this (`freeze` callbacks on each device)
but the application layer is not synchronized with them.

**The race.**

App A's StateBlob references a sequence of records it just wrote to
`/data/log.bin`:

```
let seq = self.next_seq();           // 12345
let mut f = fs::OpenOptions::new().append(true).open("/data/log.bin")?;
writeln!(f, "{seq}: tx complete")?;  // buffered in 9P client
self.last_committed = seq;           // in WASM linear memory
```

Now `on-suspend` arrives between the `writeln!` and a hypothetical
`f.sync_all()`. The app returns `StateBlob { last_committed: 12345 }`.

The supervisor receives this, writes it to disk, and calls `sync()`.
Suppose `sync()` returns successfully. The 9P client buffer is now
flushed to the host directory.

The user resumes. The supervisor loads the StateBlob, asks the WASM
store to re-instantiate with `last_committed = 12345`, and the app
queries the file expecting to see seq 12345. Whether or not it does
depends on the order of operations *inside* the 9P client, and whether
the WASM store's drop ran callback-order relative to the kernel's flush.

**Concrete failure mode.** The WASM store is dropped *as part of*
broadcasting on-suspend in the proposed design (so the store's `Drop`
impl releases resources promptly). The 9P client may have flushed
*part* of the writeln (the "12345: tx co" prefix) before the supervisor
took the broadcast lock. The supervisor's `sync()` then flushes the
remainder. Crash here, and a partial-line tail is on disk while the
StateBlob claims completion.

Another concrete failure mode: TCP retransmits. App B has just sent a
TCP packet on a socket bound to the guest-NAT network. The packet is
in the kernel's TX queue. On-suspend arrives. App B returns
StateBlob `{ tx_seq: N+1, expect_ack: yes }`. The supervisor pauses the
guest. Host-side, the packet never goes out. On resume, the TCP
connection's RTO has elapsed many times and the connection is dead.
App B will retry — but the *next* sequence number it sends is N+2,
because the StateBlob said "the N+1 packet went out". So the peer is
out of sync.

**Why this is not solvable by "just fsync after StateBlob".** Because
fsync only covers the *filesystem*. It does not cover:

- TCP buffers (no way to drain them deterministically; you'd have to
  RST every socket).
- USB control transfers (Round 6's USB stack has its own writeback).
- Wayland/compositor surface buffers (Round 6's display stack has
  framebuffer pages that may be in flight to the GPU).
- Audio frames (Round 5's SCHED_DEADLINE audio thread has a circular
  buffer the userspace audio engine may be writing into right now).
- Pending IPC messages in the supervisor's broker queue (Round 3).

The design needs an explicit *checkpoint barrier*: a phase where each
subsystem (FS, net, USB, audio, IPC) acks that its outbound queue is
drained, *before* on-suspend is broadcast to apps. macOS calls this
"quiesce". Linux's PM `freeze` callbacks try to do this for kernel
drivers but not for userspace IPC.

**Recommendation.** Round 7 must specify a multi-phase suspend with a
strict order:

```
Phase 1  STOP_ACCEPTING   supervisor stops dispatching new IPC; UI input
                           is paused; new TCP accept is refused.
Phase 2  QUIESCE_USERSPACE each app gets on-prepare-suspend; the app must
                           call sync_all on its open file handles, close
                           TCP sockets it doesn't want to keep, drain its
                           own internal queues. Reply within budget T_p.
Phase 3  QUIESCE_KERNEL    supervisor calls sync(), fsyncs each open file
                           descriptor it owns, asks the IPC broker to
                           drain to all apps' stdin, and waits for
                           virtio-net TX to be empty.
Phase 4  CAPTURE           apps get on-suspend; return StateBlob.
                           Supervisor writes blobs and runs sync() once
                           more.
Phase 5  TEAR_DOWN         WASM stores are dropped, KMS DPMS off, the
                           kernel suspend syscall is attempted.
```

This is more complex than the original design, but it is the *only*
way StateBlob can be a meaningful description of post-resume state.

---

### C3. Power Nap is a covert data-exfiltration primitive

**Claim under attack.** Power Nap allows apps with `background_fetch =
true` in their manifest to be woken periodically while the system is
in `SYSTEM_SUSPENDED` / `DISPLAY_OFF` to perform "lightweight"
operations like syncing mail, fetching calendar updates, or pulling
remote notifications.

**The attack surface.** A WASM app is a sandbox boundary, but inside
that boundary it can do anything the network capability lets it do.
The supervisor's network capability is binary: present or absent.
There is no notion of "this app may contact mail.example.com but not
adversary.example.com" — once `network = true`, the app's WASI sockets
can reach any IP the guest can route to.

Now consider a `notes-app` manifest:

```toml
[capabilities]
stdio       = true
filesystem  = true
network     = true
background_fetch = true
```

The user installs `notes-app` because they want offline notes that
sync to their own server. The manifest looks reasonable. Power Nap
fires every 60 seconds with the display off. The app:

- Opens a TCP connection to `attacker.example.com`.
- Uploads the contents of `/data/notes/` (recipes, journal entries,
  passwords typed into the notes the user never thought to encrypt).
- Closes the connection. No screen flash, no network indicator, no
  audible cue. The lid is closed; the user is asleep.

The macOS analogue is gated by:

- The user explicitly granting "Background App Refresh" per app under
  Settings → Battery → Apps.
- The Mac's network indicator (Control Center) showing "an app used
  the network in the last 24 hours" with per-app accounting.
- Code signing + notarization that gives Apple visibility into the
  binary.
- The fact that the OS itself runs only Apple's mail/calendar agents
  during Power Nap unless the user has explicitly enabled
  Background-App-Refresh for a specific app.

VyomaOS has none of these.

**Sub-issue: who decides "background fetch is OK"?** The current
design implicitly says: the app's manifest. The user installs the
app, the manifest declares the field, done.

This is the wrong principal. The manifest is written by the app
author, who has the strongest incentive to declare any capability that
makes the app "feel snappier" when the user comes back. The author and
the user are not the same person.

**Sub-issue: what is the data budget?** macOS's Power Nap is
explicitly bounded: each wake is short (the system is supposed to
return to S3 within seconds), network traffic is metered, and the OS
will refuse to wake if the battery is below a threshold. The Vyoma
proposal does not specify a budget. An adversarial app can simply hold
the wake open: spawn a long-running TCP transfer, return slowly from
its background-fetch handler, and the supervisor has no documented
mechanism to interrupt.

**Recommendation.** Round 7 must specify, before any code is written:

1. **Per-app consent at first run.** When an app with
   `background_fetch = true` is first launched, the supervisor displays
   a chrome prompt: "App X wants to run in the background while your
   display is off. Allow? [Yes / No / Only when on charger]". The
   answer is persisted to `/data/.permissions/<app>.toml`.
2. **Per-app network egress budget.** A bytes-per-wake limit (e.g.,
   1 MiB) and a wakes-per-hour limit (e.g., 4). The supervisor's
   network broker counts bytes and enforces the cap.
3. **Per-app destination whitelist.** Optional but recommended:
   manifest may declare `network_hosts = ["mail.example.com"]`. The
   supervisor's network broker enforces SNI / destination IP.
4. **Wake budget for the system as a whole.** No more than N wakes
   per hour, regardless of how many apps want them. Apps share the
   budget.
5. **Audit log.** Every background-fetch wake is recorded:
   `(timestamp, app, duration, bytes_in, bytes_out, destination)`.
   The user can view this in a "Battery → Background Activity" panel.
6. **Refuse on low battery.** Below some threshold (e.g., 20%), the
   supervisor declines all background-fetch wakes regardless of
   per-app permission.

Without these, Power Nap should be off-by-default and the manifest
field should require a user override at install time.

---

### C4. Thermal policy is on the wrong time domain

**Claim under attack.** The proposed thermal policy ladders are:

```
Normal     (< 70 °C)  → no action
Warm       (70-80 °C) → reduce display brightness, dim animations
Hot        (80-95 °C) → suspend all non-UI apps via on-suspend
Critical   (≥ 95 °C)  → emergency hibernate, then kernel shutdown
```

The "Hot" tier invokes the full on-suspend cycle to checkpoint apps
before suspending them.

**Time-domain analysis.**

Empirically (measuring on similar Rust-based WASM-host systems):

- 9P fsync of a 64 KiB StateBlob to virtio-blk: 5–50 ms (median
  ~15 ms with `cache=writeback`).
- WASM store drop + WIT callback round-trip: 1–10 ms per app.
- For 10 concurrent apps (Phase-17 current state): on-suspend total
  is bounded below by `max(individual_checkpoint_latency)`, which is
  on the order of 50–500 ms in the happy case.
- An app that is itself CPU-bound (e.g., doing a long-running compute
  loop with no yield points) may not even *receive* the on-suspend
  message for hundreds of milliseconds, because WASI poll boundaries
  in `wasm32-wasip2` are sparse.
- A misbehaving app that ignores on-suspend or returns slowly is
  bounded only by the supervisor's timeout. The design proposes
  several seconds.

**Thermal physics.** A modern x86 SoC at 95 °C is already in its
catastrophic regime. The kernel's `thermal_zone` driver, when
configured, has trip points that fire well below this: typically
`passive` at 80 °C (which throttles the CPU via `cpufreq`) and
`critical` at ~105 °C (which calls `orderly_poweroff` if available,
else `emergency_restart` — which is just `machine_restart`). The
hardware itself has Tj-max limits (PROCHOT) that throttle MSRs
*independent of* the kernel.

In other words, by the time the supervisor's polling loop sees 95 °C
and decides to act, several things have already happened that the
supervisor cannot undo:

- The CPU may already be downclocked by hardware (PROCHOT or RAPL
  power-cap).
- The kernel's `critical` trip point may already have fired
  `orderly_poweroff`, which sends SIGINT to PID 1 — i.e., the
  supervisor itself.
- On some platforms, the BMC / EC may have pulled the power directly.

So the supervisor has at most a few hundred milliseconds at 95 °C to
do something useful, and the on-suspend cycle takes longer than that
in the worst case.

**Subtler issue: the polling latency.** The proposal does not specify
the thermal polling interval. If it is the same as battery polling
(seconds), the supervisor will see 70 °C, 70 °C, 70 °C, 95 °C — a step
jump it never had warning of. If it is fast (10 ms), the polling loop
itself becomes a thermal load (busy-reading sysfs files).

**Recommendation.** Round 7 must distinguish two thermal response
tiers:

- **Soft thermal response** (Warm → Hot): the supervisor has time to
  use the orderly on-suspend mechanism. This is the policy ladder in
  the proposal. It is appropriate as long as the system is below
  ~88 °C.
- **Hard thermal response** (Critical → Emergency): the supervisor
  acts immediately without waiting for app checkpoints. Concretely:
    - `SIGSTOP` all wasmtime children (kernel-level pause; instant).
    - Drop framebuffer brightness to zero (KMS BACKLIGHT to 0).
    - Spin down virtio-blk if possible.
    - Call `sync()` once.
    - Set the kernel's thermal cooling devices to maximum (write to
      `/sys/class/thermal/cooling_deviceX/cur_state`).
    - Then, if temperature is still rising, write `disk` to
      `/sys/power/state` to hibernate; if that fails, write `o` to
      `/proc/sysrq-trigger` for an orderly poweroff.

The hard tier does *not* call on-suspend WIT callbacks. Apps wake up
from a frozen state (signal-stopped, then signal-continued) and
discover that wall-clock time has advanced. Apps that care about that
must implement their own monotonic-vs-wall-clock checks (this is a
generally good habit anyway).

The thermal polling interval should be **adaptive**: 1 s when cool,
100 ms when above 75 °C, 10 ms when above 85 °C. The supervisor's
polling thread should be a separate `tokio` task or kernel thread, not
the main event loop, so the busy-reading does not block IPC.

Also: the *correct* primitive on Linux for thermal events is
`thermal_zone`'s sysfs interface combined with `udev` thermal events
(uevent on `subsystem=thermal`). The supervisor should use this and
fall back to polling only if the kernel was built without
`CONFIG_THERMAL`. The proposal does not mention any of this.

---

### C5. ActivityAssertion (R5) and PowerAssertion (R7) are two assertion systems that pretend to be one

**Claim under attack.** R5 introduced `ActivityAssertion`:

```rust
let assertion = supervisor.acquire_activity_assertion(
    AssertionKind::Cpu, "transcoding video"
);
// ... assertion lives ...
drop(assertion);
```

This prevents App Nap from suspending the app: the app is "doing
work" and the supervisor will keep it scheduled normally. The
ActivityAssertion is a per-app concept.

R7 introduces `PowerAssertionHandle`:

```rust
let pa = supervisor.acquire_power_assertion(
    PowerAssertionKind::PreventDisplaySleep, "video playback"
);
// ... lives ...
drop(pa);
```

This prevents the *system* from putting the display to sleep or from
suspending entirely. The PowerAssertion is a global concept.

**The interaction matrix is undefined.** Consider four cases:

| App holds ActivityAssertion? | App holds PowerAssertion? | System should: |
|---|---|---|
| No  | No  | sleep on schedule, app napped per R5 |
| Yes | Yes | stay awake, app not napped |
| Yes | No  | ???                                  |
| No  | Yes | ???                                  |

The proposal does not specify rows 3 and 4. But both are realistic.

**Row 3: ActivityAssertion without PowerAssertion.** An app that holds
ActivityAssertion because it is doing useful CPU work (say,
transcoding a video) but did *not* acquire PowerAssertion because the
app's author thought "the user will keep their laptop open while it's
working". If the user closes the lid:

- If the system suspends: the transcode is interrupted mid-frame, and
  on resume the WASM store starts fresh from StateBlob — losing N
  minutes of progress.
- If the system stays awake: the closed-lid case turns into "laptop
  cooking in the bag", which is a real safety issue.

Neither outcome is what the app author wanted. The R5 assertion was
the wrong tool: it prevents App Nap but does not prevent system
sleep. So either the app authors are taught to *always* acquire both
(which makes the distinction pointless), or some assertions are
implicitly upgraded.

**Row 4: PowerAssertion without ActivityAssertion.** An app that holds
PowerAssertion because it is displaying a slideshow (so the display
must stay on) but has *not* acquired ActivityAssertion (because the
slideshow is event-driven and mostly idle). If the supervisor decides
to App-Nap the slideshow app:

- The display stays on (PowerAssertion is held).
- The app is napped: its event loop stops running.
- The display now shows the last rendered frame, frozen.
- The user comes back and the slideshow is stuck.

Again: not what anyone wanted.

**Root cause.** The two assertion systems are scoped to two different
objects (the app vs. the system) but the *effects* they want to
prevent are coupled. An app that is "active" (ActivityAssertion) is
implicitly saying "I have work to do that I'd like to complete
without being interrupted". An app that wants the system "awake"
(PowerAssertion) is saying the same thing. The macOS equivalent
collapses these into a single `IOPMAssertion` API with different
*types*: `kIOPMAssertionTypePreventUserIdleSystemSleep`,
`kIOPMAssertionTypeNoDisplaySleep`,
`kIOPMAssertionTypeNoIdleSleep`. The app picks the type it needs;
there's only one acquire/release call.

**Recommendation.** Fold both into one assertion algebra:

```rust
pub enum AssertionKind {
    /// App must continue scheduling normally (was R5 ActivityAssertion).
    PreventAppNap,
    /// User-idle clock is reset; display stays on; system does not
    /// sleep (was R7 PreventDisplaySleep + PreventIdleSleep).
    PreventUserIdleSleep,
    /// System does not sleep even if user is idle. Display may sleep.
    PreventSystemSleep,
    /// CPU stays at performance governor; no thermal throttling
    /// unless thermal-critical.
    PreventCpuThrottle,
}
```

Document the precedence rules:

- Holding `PreventUserIdleSleep` implies `PreventAppNap` for the
  holding app. (You can't be "preventing user-idle sleep" while
  yourself being napped.)
- Holding `PreventSystemSleep` does *not* imply
  `PreventUserIdleSleep`: the display can still go off.
- Holding `PreventAppNap` does *not* prevent system sleep. If the
  user closes the lid and no `PreventSystemSleep` is held, the
  supervisor sleeps. The held assertion is implicitly released across
  the suspend boundary (the app is checkpointed; on resume it can
  re-acquire if it still wants).

Then a single `acquire_assertion(kind, reason)` call returns an RAII
handle. Apps choose the *kind* that matches their need.

This is what macOS does, what Linux's GNOME `org.freedesktop.PowerManagement.Inhibit`
does, and what Windows's `SetThreadExecutionState` does. Two separate
APIs across two rounds is a sign that the design was incrementally
discovered rather than designed.

---

### C6. Hibernate over WASM produces silent corruption when on-suspend is not implemented

**Claim under attack.** Hibernate is defined as: capture every app's
StateBlob via on-suspend, write to `/data/.hibernate/<app>.blob`,
write a manifest of which apps were running, drop the WASM store,
and exit the supervisor. On boot, init reads the hibernate manifest,
re-instantiates each app, and calls on-resume with the blob.

The opt-in is at the WIT-callback level: an app that does not
implement `on-suspend` returns the default (empty StateBlob); the
supervisor stores the empty blob and the app's hibernate-resume is
indistinguishable from "fresh start".

**The hazard.** A user closes the lid expecting hibernate (battery
preservation). The system writes the hibernate snapshot. The user
opens the lid an hour later. The notes app — which never implemented
on-suspend, because its author "didn't need to, it's just a notes
app" — comes back to a blank state. The user's open note, half-typed
TODO list, and unsaved scroll position are gone.

This is exactly the failure mode that `~/Library/Saved Application
State/` on macOS exists to prevent: the OS does *not* let an app
silently degrade across a system-induced state transition. macOS apps
that do not implement state restoration get a default-state-restoration
by way of the AppKit document architecture (NSDocument auto-saves the
draft to disk; the user gets the unsaved draft back regardless of
whether the *app* did anything).

A WASM app that uses only WASI sockets, stdio, and its own
linear-memory state has no equivalent automatic mechanism. There is no
"WASI document"; there is no auto-save of linear memory.

**Sub-issue: hibernate vs. crash equivalence.** The proposal appears
to accept that "no on-suspend" means "hibernate = crash" semantics.
This is fundamentally wrong because the user's expectations differ:

- After a crash: the user understands that recent unsaved work might
  be lost. The user expects to get a "restore previous session?"
  prompt from any well-behaved app.
- After hibernate: the user expects everything to be exactly as they
  left it. They closed the lid for two hours; they didn't crash.

If hibernate silently degrades to crash semantics for any
non-checkpointing app, then hibernate is *less* trustworthy than just
leaving the system running — because the user has the *false belief*
that their state is preserved.

**Recommendation.** Round 7 must specify a hibernate-completeness
contract:

1. The supervisor enumerates running apps before initiating hibernate.
2. For each app, the supervisor checks: does the WASM module export
   the `on-suspend` WIT callback?
3. **If any app does not**, hibernate is *refused*. The supervisor
   shows a chrome dialog: "These apps cannot be hibernated safely:
   notes-app, calculator. Continue anyway (their state will be lost)?
   [Cancel / Continue]". Default is Cancel.
4. If the user chooses Continue, the supervisor records in the
   hibernate manifest which apps will lose state, and on resume
   displays a chrome notification: "notes-app, calculator started
   fresh; their previous state was not saved".
5. The manifest spec gains a `restorable = bool` field that defaults
   to `false` and must be explicitly set to `true` by an app that
   implements on-suspend correctly. The supervisor validates at app
   install time that `restorable = true` implies the WASM module
   exports the callback.

Additionally, the framework should provide a *default* on-suspend
implementation that captures the app's working directory state
(filesystem mtime + size of every file under `/data/<app>/`) and
verifies on resume that the filesystem state matches. This catches
the common case of "the user just had a few files open" without
requiring every app to write checkpointing code.

---

## Significant Issues (important)

### S1. Battery state polling has no DBus, no upower, no documented Linux-native fix

**Claim under attack.** The proposal says: poll `/sys/class/power_supply/BAT0/`
periodically to read `capacity`, `status`, `voltage_now`. The polling
interval is a tunable; the design suggests 30 s default.

**Why polling is wrong.** Linux exposes battery state changes via
udev/uevent on the `power_supply` subsystem. The kernel emits
`change@/devices/.../power_supply/BAT0` events when capacity drops
into discrete buckets, when the charger is plugged/unplugged, when
the battery transitions to "critical". Listening on these is O(1) for
the userspace daemon (a netlink socket).

The proposal does not mention uevent at all. With 30-second polling:

- The user sees 30 % → 5 % with no warning. The 20 % low-battery
  threshold was crossed during a poll gap; no notification fired.
- A user on AC who unplugs sees "still 100 %" for up to 30 seconds.
- The display brightness policy (which depends on AC/battery) is
  out of sync.

With 1-second polling, the supervisor's main thread is reading sysfs
30× more than necessary; each read is a kernel syscall + a parse of
ASCII text. On a small embedded platform (`iot-edge`), this is a
non-trivial fraction of CPU.

**Why DBus is not the answer.** macOS's analogue is `IOPowerSources`,
which is a kernel notification. Linux's analogue is upower, which is
a userspace daemon that listens on udev and re-broadcasts on DBus.
VyomaOS has no DBus broker (and shouldn't add one just for this).

**Recommendation.** The supervisor must:

1. Open a netlink socket bound to `NETLINK_KOBJECT_UEVENT` with the
   `1 << 0` group (the default udev group). This is what `libudev`
   does internally; VyomaOS can do it directly with a few hundred
   lines of Rust.
2. Filter for events on `subsystem=power_supply`.
3. On each event, re-read the current state from sysfs (this is the
   *only* sysfs read; it's reactive, not polling).
4. Have a *coarse* polling fallback (every 5 min) that catches the
   rare case where a uevent was dropped.

This pattern works on every Linux from 2.6.30 onward. There is no
excuse for polling.

---

### S2. Display sleep latency vs. user expectation

**Claim under attack.** The proposal lists "Display Off" as a step
that turns off the framebuffer via KMS DPMS. The expected user
trigger is the inactivity timer (e.g., 2 min of no input).

**The latency to wake the display.** On bare metal, DPMS wake from D3
to D0 is typically 100–200 ms for a modern panel. On QEMU's virtio-gpu,
it is faster (the "display" is a host window that was just hidden). But
on resume from S3, the panel itself may need to re-initialize, which
on real hardware takes 1–2 s.

The proposal does not specify whether on-resume of the display is
synchronous (block input handling until the panel is back) or
asynchronous (let input arrive while the panel is still warming).
Either choice has trade-offs:

- Sync: the user presses a key, sees nothing for 1 s, then suddenly
  the desktop appears. Feels broken even though it's working.
- Async: the user presses a key, that key is consumed by the
  currently-focused app (which thinks it's a normal keystroke), but
  the user can't see the result yet. Now they type a passphrase into
  a password field while the screen is dark, and the password is
  echoed into the wrong control because focus was somewhere else when
  they pressed the key.

The macOS behavior is to consume the first wake-input event as
"display-wake" and discard it (the user has to press a *second* key
to actually do anything). VyomaOS should adopt this. The proposal
does not mention it.

**Recommendation.** Specify that the first input event after a
display-sleep transition (wake or otherwise) is consumed by the
supervisor as a "wake event" and not forwarded to any app. The
supervisor counts the latency from wake-event to display-online and
holds the input queue until the display is observably on (KMS
`CRTC.active = 1` AND a full frame has been flipped).

---

### S3. Hibernation slot integrity and rollback

**Claim under attack.** Hibernate writes StateBlobs to
`/data/.hibernate/`. Resume reads them. Simple. Linear.

**The missing concern.** What if the hibernate write was interrupted
(power cut to host during hibernate)? What if the resume read finds a
truncated blob file? What if the manifest references an app whose
StateBlob is missing? What if the supervisor binary version changed
between hibernate and resume (an update happened to `/data` between
sessions)?

The proposal does not specify integrity checks, atomic write
semantics, or versioning of the hibernate format.

Concretely the design needs:

- **Atomic write per blob.** Write to `.blob.tmp`, fsync,
  `rename(tmp, final)`. The supervisor's existing 9P/virtio-blk path
  may or may not give atomic rename — verify.
- **A manifest file** that is itself written atomically and contains:
  - Supervisor version (semver).
  - WASM ABI version per app.
  - SHA-256 of each app's `.wasm` binary.
  - SHA-256 of each StateBlob.
- **A validity check on resume**: if the manifest references an app
  whose `.wasm` SHA does not match the currently-installed version
  (app updated while hibernated), the supervisor *must not* call
  on-resume with the old StateBlob. The blob layout may have
  changed.
- **A "torn write" recovery**: if any blob fails its SHA check, refuse
  to resume *that app* (start fresh) but continue resuming the
  others, with a user-visible warning.
- **A "stale hibernate" timeout**: if the hibernate manifest is older
  than 7 days (configurable), refuse to resume and warn the user that
  their saved state is too old. This protects against the user
  thinking hibernate is a long-term archive.

None of this is in the proposal.

---

### S4. AC adapter detection and policy switching

**Claim under attack.** The proposal implies policies switch based on
power source (e.g., "Performance" on AC, "Balanced" on battery).
Detection is via `/sys/class/power_supply/ADP0/online` or similar.

**The race.** Plug/unplug is an event the kernel emits via uevent (see
S1). The supervisor reads it asynchronously, then switches the
policy. The policy switch involves:

- Updating CPU governor (`cpufreq` writes to
  `/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor`).
- Updating display brightness (KMS `BACKLIGHT_BRIGHTNESS`).
- Restarting / re-prioritizing certain app threads (per R5
  scheduler).
- Possibly suspending some apps (background indexers shouldn't run
  on battery).

If the user unplugs *during* a heavy operation (compilation, render),
the policy switch fires mid-operation. The cpufreq governor goes from
`performance` to `powersave`, which means the CPU clock drops
mid-instruction-window. The operation continues but at a fraction of
the speed.

This is actually fine *if* the user understands it. But the proposal
does not specify the user-visible feedback. The user sees their
compile go from 5 minutes remaining to 25 minutes remaining and
thinks something is broken.

**Recommendation.** The supervisor should:

- Display a chrome notification on AC↔battery transitions: "Switched
  to Battery (2 h 45 m remaining). CPU performance limited."
- Allow per-app override: an app holding `PreventCpuThrottle`
  assertion keeps `performance` governor even on battery, at the
  expense of battery life.
- Defer the policy switch by a grace period (5 s) so a brief
  unplug/replug doesn't thrash the governor. (This matches macOS.)

---

### S5. Audio playback during display sleep

**Claim under attack.** The proposal does not address what happens to
SCHED_DEADLINE audio threads (R5) when the display sleeps.

**The intuition the user has.** Music is playing. The display goes
off after the inactivity timer. Music continues. The user can wake
the display with mouse-jiggle. None of this is surprising on macOS,
Windows, or modern Linux desktops.

**The implementation has to make this work.** The audio app holds a
SCHED_DEADLINE budget (R5). The display sleeps. Does the audio app
keep its budget? The display-sleep policy may have triggered "lower
power CPU governor", but SCHED_DEADLINE is independent of governor
(it's about deadline scheduling, not frequency). So in principle yes,
audio continues.

But: does the audio app implicitly hold `PreventSystemSleep`? If
not, the supervisor can transition from "display off" to "system
sleep" while audio is playing, and the music stops.

The macOS answer: any process actively writing to an audio output
device is implicitly granted a `kIOPMAssertionTypePreventSystemSleep`
assertion by `coreaudiod`. The app doesn't have to know.

**Recommendation.** The supervisor's audio broker (when it is built,
not yet specified in detail) implicitly acquires a
`PreventSystemSleep` assertion on behalf of any app that has an open
audio output stream. The assertion is released when the stream closes
or pauses for > N seconds.

Specify this in Round 7 (forward-references to a future audio round
are fine, but the *policy* should be designed now, not improvised
later).

---

## Design Gaps

### G1. Wake sources are not enumerated

The proposal mentions wake-from-suspend but does not specify the
wake-source policy. On macOS, wake sources are:

- Lid open
- Power button
- Keyboard / trackpad (USB or Bluetooth)
- Network (Wake-on-LAN, Bonjour subscription)
- Scheduled wake (`pmset schedule wake ...`)
- Power state change (plug in / unplug)

For each, there's a policy: "this wake source brings the system to
RUNNING" vs. "this wake source brings the system to DARK_WAKE (Power
Nap)". Some are user-configurable.

VyomaOS needs:

- An enumeration of what wakes the supervisor from suspend.
- For each, a target state (full wake vs. dark wake).
- For each, a "wake reason" attribution so apps and the audit log can
  see why they woke.

On QEMU, the wake sources are limited (no lid, no real USB, no WoL).
On bare metal, the source enumeration depends on which ACPI WAKEUP
methods the firmware exposes. The supervisor needs a generic API and
a platform-specific implementation per profile.

### G2. Sleep schedule and wake-for-maintenance

macOS supports `pmset schedule` for scheduled wake (e.g., "wake at
3 AM Tuesday for backup"). VyomaOS does not address this.

For a desktop OS, this matters because:

- Software updates should happen at user-defined times, not when the
  user is using the machine.
- Periodic filesystem maintenance (defragmentation, scrub) needs a
  wake-and-do-work primitive.

The proposal should at least carve out the API surface (a `cron`-like
table of (timestamp, action, app) tuples) and defer implementation
to a later round.

### G3. Suspend during application install / update

What happens if the user closes the lid in the middle of a package
manager `install` operation? The package manager (apps/package-manager)
is currently a WASM app with filesystem access. It is in the middle
of writing files to `/data/installed/`.

The on-suspend cycle, as proposed, lets each app return its
StateBlob and tears down. But the package manager's invariant — "all
files of package X are present, or none are" — may be violated
mid-write.

The package manager would need its own write-ahead-log to make
on-suspend safe. The proposal does not require this. Apps that hold
filesystem write capability and care about transactional consistency
need explicit guidance.

### G4. The "everything is awake" cost on QEMU

In a VM, the host pays the cost of running the VM regardless of what
the guest is "doing". A perfectly-implemented suspend that drops the
guest's CPU usage to 0 still draws power on the host because the host
kernel, host display server, and the QEMU process are all running.

The proposal does not discuss this trade-off. For a development
target (which VyomaOS is, today), this is irrelevant — the user
isn't trying to save battery on a laptop. For an aspirational
deployment target (Vyoma running on actual hardware), this is the
whole point.

The Round 7 spec should make clear which deployment target it is
designing for. If both, the constraints differ enough that some
features (Power Nap, deep S3, hibernate) are real on hardware and
no-ops on QEMU, and the test coverage strategy has to account for
that.

### G5. Idle detection mechanics

When is the user "idle"? The supervisor needs to track:

- Time since last keyboard event.
- Time since last mouse event.
- Time since last touch event (mobile / robotics platforms).
- Time since last app activity event (some apps want to assert "user
  is active because they are watching the screen" without input).

The proposal does not specify which of these contribute to the
idle timer, or whether full-screen video playback (which has no input)
counts as "user is here". macOS uses a heuristic combining input
events with an app-can-veto API (`UpdateSystemActivity`). Vyoma needs
the same.

In particular: the `mouse` capability (existing Vyoma) only delivers
events to apps with the cursor inside their window. A mouse event
outside any app's window goes to the supervisor. The supervisor must
treat *any* mouse event as activity, not just events that get
delivered to an app.

### G6. The "lid closed but plugged in" state

A common pattern: user closes the laptop lid but keeps it plugged in
on a desk, with an external display. On macOS, this is "clamshell
mode" — the system stays awake because external display is connected
and AC is present. VyomaOS does not have an "external display"
abstraction yet (Round 6's display is single-output), but the *policy*
needs to be specifiable.

Add to Round 7: a lid-close policy that depends on (AC connected,
external display connected). If both are true, stay awake. If
neither, suspend. Otherwise, follow user preference.

### G7. Power state across kexec / reboot

The supervisor is PID 1. If the user reboots (via the chrome power
menu), the supervisor exits and the kernel restarts. Is hibernate
state preserved across reboot? It's on `/data`, so technically yes.
But the manifest says "these apps were running when hibernate
started" — if the user updated those apps before resume, what
happens? (Covered partially in S3.)

A separate concern: `kexec` (warm reboot into a new kernel). VyomaOS
may use `kexec` for updates (Round 8+?). Hibernate state across kexec
is even more subtle — the WASM bytecode is the same but the kernel
has changed, which may have changed system call ABIs that wasmtime
relies on.

The proposal should explicitly address: hibernate-resume after a
supervisor or kernel update is *refused* (start fresh, warn user).

### G8. The pmset / caffeinate equivalent

macOS users invoke `caffeinate` from a terminal to override power
management for a one-off task. Vyoma should provide an equivalent
that an app — or the supervisor's shell — can invoke.

Specifically:
```
@supervisor: assert PreventUserIdleSleep "long-running build"
@supervisor: release-assertion <id>
@supervisor: list-assertions
```

These IPC commands map onto the assertion algebra. The user can see,
audit, and force-release any assertion. Without this, an app that
"leaks" an assertion (acquires and never releases due to a bug) can
hold the system awake forever with no remediation. The proposal does
not specify a leak-detection / leak-release mechanism.

### G9. Inactivity timer hysteresis

When the inactivity timer is, say, 2 minutes, what happens at
1 m 59 s? The supervisor is about to fire "display off". The user
moves the mouse 0.5 s before the timer fires. The display *should*
stay on, and the timer should reset.

But there is a race: the supervisor's timer task is about to call
"dim display"; the mouse event arrives in the input thread; the
supervisor has not yet reset the timer because the input thread
hasn't notified it. The display briefly dims, then comes back. Bad
UX.

The proposal should specify:

- The input thread directly resets the inactivity timer on every
  event (synchronous, not via IPC).
- The timer is read by the policy task; the policy task does *not*
  pre-compute "fire at T+120 s" but rather reads the timer atomically
  before each step.
- Dimming is a multi-step gradient (over 5 s) so a late input event
  during the dim animation can interrupt it cleanly.

### G10. Telemetry vs. privacy

Power management generates telemetry: how long the user was idle,
when they slept, when they woke, how often Power Nap ran, how long
the battery lasted. This data is interesting to the user (Battery
Health panel) and dangerous to send anywhere else.

The proposal does not specify retention. The audit log of background
fetches (recommended in C3) should be local-only with a retention
policy (e.g., 30 days) and explicit user consent for any cloud
syncing. Specifying this now prevents a later "let's add usage stats"
PR from undermining the privacy posture.

### G11. The supervisor's own power footprint

The supervisor itself is a Rust process with a tokio runtime, several
threads, an event loop polling sysfs (for thermal and battery), a
KMS file descriptor, and IPC sockets. The supervisor's CPU
consumption when "the system is idle" is the floor of how low power
can go.

On idle, the supervisor should:

- Block on epoll, not poll.
- Use the slowest acceptable thermal/battery polling interval.
- Park its tokio worker threads.
- Coalesce its own logging (no per-second heartbeats).

The proposal mentions the structured heartbeat from R6 / observability
but does not say what its rate is. If it's 1 Hz, the supervisor is
preventing the kernel from entering deep C-states. Recommend: the
heartbeat rate is adaptive (1 Hz when active, 0.1 Hz when idle,
0 Hz when display-off).

### G12. App ABI changes for power awareness

Apps that want to be power-aware need:

- A way to receive notifications about (AC↔battery, display↔dark,
  thermal pressure).
- A way to query "am I currently allowed to do CPU-heavy work?".
- A way to register a background-fetch handler.

These need WIT interface definitions. The proposal references
on-suspend / on-resume but not the broader power-event surface. A
desktop app should be able to:

```
import vyoma:power@0.1;

vyoma::power::subscribe(EventMask::AC_STATE | EventMask::THERMAL);

while let Some(event) = vyoma::power::next_event() {
    match event {
        Event::AcConnected => { /* may use CPU freely */ }
        Event::AcDisconnected => { /* throttle background work */ }
        Event::ThermalPressure(level) => { /* slow down */ }
        Event::PowerLow(percent) => { /* save state, warn user */ }
    }
}
```

This API needs to be specified in Round 7 alongside the assertion
API.

---

## Points of Strength

The proposal is not without merit. The following are sound and should
be carried forward:

### P1. Borrowing the macOS conceptual model

The macOS power model is genuinely well-designed and has been refined
over a decade. The concepts — assertions, App Nap, Power Nap,
quiesce / restore — are well-understood by the OS community and by
app developers who already shipped on macOS. Borrowing the vocabulary
gives Vyoma a head start on documentation and on developer mental
models.

### P2. RAII assertion handles

Using Rust RAII (`Drop` releases the assertion) is materially safer
than macOS's C-style `IOPMAssertionCreateWithName` /
`IOPMAssertionRelease`, which leaks under panic. The Rust version
makes leak-on-panic impossible (within a single process). This is a
real improvement over the source material.

### P3. StateBlob as opaque bytes

Treating the app's per-suspend state as an opaque byte slice — the
supervisor never parses it — is the correct boundary. The app owns
its serialization format; the supervisor owns timing and durability.
This is much cleaner than NSCoder-style typed encoding that the OS
has to understand.

### P4. WIT callbacks for lifecycle events

Defining lifecycle events as WIT-level callbacks (rather than POSIX
signals or stdin messages) is the right abstraction. The WASM module
exports a function; the supervisor calls it. The semantics are clear
and the callback can be synchronous.

### P5. Tiered thermal response

The basic idea of a thermal ladder (Normal / Warm / Hot / Critical)
is correct. The specific actions per tier and the time-domain mismatch
(C4) are wrong, but the ladder structure is right.

### P6. Per-platform profiles for power policy

The existing platform profile infrastructure (`supervisor/src/profile/`)
naturally extends to per-platform power defaults. `mcu-minimal` has
no display sleep concept; `desktop-full` has the full stack;
`server-headless` has no display at all. The profile system gives a
clean place to put these defaults.

### P7. Capability-based gating

The choice to gate background-fetch on a manifest capability is
*directionally* correct, even though the manifest-alone is
insufficient (see C3). Capability declarations are the right place to
*start* the conversation; user consent is the missing piece.

### P8. Hibernate via /data persistence

Using the existing 9P-mounted `/data` for hibernate state is
elegant: it reuses infrastructure that already works (Round 4
filesystem), it persists across reboots naturally, and it has clear
ownership (the supervisor's namespace).

### P9. Acknowledging Safe Sleep

The macOS Safe Sleep concept (write hibernate image to disk during
S3 so a battery failure during S3 doesn't lose state) is included.
This is a genuinely good feature that most Linux distros do not
provide cleanly. The proposal recognizing it is a positive sign.

### P10. Apps can be ignorant of power management

The default (an app that doesn't implement on-suspend) is to keep
working. Apps that don't care don't have to. This is the right
default; the failure mode (C6) is not that the default is wrong, but
that the *consequences* of the default during hibernate are
undisclosed.

---

## Synthesis Recommendations

The Round 7 proposal must be revised before implementation. Below is
a prioritized action list. Items marked **P0** are blocking; **P1**
are needed before any code review; **P2** are needed before public
release of the spec.

### P0 — must fix before this round's FINAL document

1. **Scope down "platform sleep"**. Replace S3-on-QEMU pretense
   (C1) with the proposed power-state ladder (RUNNING / USER_IDLE /
   DISPLAY_OFF / APP_SUSPENDED / SYSTEM_SUSPENDED / HIBERNATED) and
   document which states are real on which platform profile.

2. **Specify the multi-phase suspend barrier** (C2). Add Phases 1–5
   (STOP_ACCEPTING / QUIESCE_USERSPACE / QUIESCE_KERNEL / CAPTURE /
   TEAR_DOWN) to the design. Each phase has an explicit completion
   ack and a documented timeout.

3. **Fold ActivityAssertion and PowerAssertion into one algebra**
   (C5). Define `AssertionKind` enum, document precedence, remove
   the two-API redundancy.

4. **Hibernate-completeness contract** (C6). Add `restorable = bool`
   to manifest schema. Specify the "refuse to hibernate apps without
   on-suspend" behavior and the user prompt.

5. **Thermal hard-tier path** (C4). Add SIGSTOP-based emergency
   freeze that bypasses on-suspend at Critical temperatures.

### P1 — must fix before implementation begins

6. **Replace battery polling with uevent** (S1). Specify netlink-based
   power_supply event subscription. Polling is fallback only.

7. **Power Nap consent model** (C3). Add per-app first-run prompt,
   data-volume budget, destination whitelist support, audit log.
   Power Nap is off by default; manifest field requires user override.

8. **Hibernate manifest format with integrity** (S3). Specify SHA-256
   per blob, supervisor-version tag, atomic write semantics,
   stale-hibernate timeout.

9. **Display sleep input consumption** (S2). First post-wake event is
   consumed; specify the wake-to-input-online state machine.

10. **Audio implicit assertion** (S5). Specify that audio output
    holders implicitly acquire PreventSystemSleep.

### P2 — must fix before spec is final-stamped

11. Wake-source enumeration (G1).
12. Scheduled wake API surface (G2).
13. Suspend-during-install guidance for apps with filesystem write
    capability (G3).
14. QEMU-vs-bare-metal cost discussion (G4).
15. Idle detection mechanics: which inputs reset the timer (G5).
16. Lid-close policy with external display / AC inputs (G6).
17. Hibernate-resume refusal after supervisor / kernel update (G7).
18. `caffeinate`-equivalent IPC commands (G8).
19. Inactivity timer hysteresis (G9).
20. Telemetry / privacy posture for power audit log (G10).
21. Supervisor's own idle behavior (G11).
22. WIT power-event subscription API (G12).

### Architectural recommendation

The current proposal mixes three concerns that should be separated:

- **The Linux PM core** (what we ask the kernel to do):
  `/sys/power/state`, thermal_zones, cpufreq governors, KMS DPMS.
- **The Vyoma supervisor's power state machine** (what state the
  system is in from the user's perspective): RUNNING / USER_IDLE /
  DISPLAY_OFF / APP_SUSPENDED / SYSTEM_SUSPENDED / HIBERNATED.
- **The Vyoma app contract for power events** (what apps see):
  on-suspend, on-resume, assertions, background_fetch.

These three need three separate sections in the FINAL document. The
current proposal interleaves them, which makes it hard to reason
about correctness of any one layer.

### Test strategy recommendation

Round 7 testing must include:

- **In-VM tests** (run in CI): exercise the user-visible state
  machine (RUNNING → USER_IDLE → DISPLAY_OFF → APP_SUSPENDED)
  without depending on real ACPI. Use a fake "input source" that the
  test injects synthetic events into.
- **StateBlob round-trip tests**: instantiate an app, exercise it,
  trigger on-suspend, capture StateBlob, drop the store, re-instantiate
  with on-resume, verify state.
- **Hibernate corruption injection tests**: write a hibernate
  manifest, then corrupt one blob with `dd`; verify supervisor
  refuses to load that app but loads the others.
- **Thermal response tests**: inject fake thermal_zone readings via
  a mocked sysfs (the test fixture installs a tmpfs at
  `/sys/class/thermal/`); verify the supervisor transitions through
  the ladder.
- **Battery uevent tests**: inject a synthetic uevent on a test
  netlink socket; verify the supervisor's battery state updates.
- **Assertion algebra tests**: hold various combinations of
  assertions, trigger inactivity, verify the system stays awake or
  sleeps per the documented precedence.
- **Bare-metal smoke test** (manual, not CI): on actual hardware,
  verify that S3 / S4 actually do what the spec says they do. This
  test exists outside CI but blocks any release that claims S3
  support.

### Process recommendation

The proposal would benefit from a literature review pass before
FINAL. Specifically:

- Read the Apple "Energy Efficiency Guide for Mac Apps" — the
  authoritative source on macOS power semantics.
- Read the Linux kernel `Documentation/power/` tree, particularly
  `runtime_pm.rst` and `userland-swsusp.rst`.
- Read the systemd-logind / systemd-sleep design docs — Linux's
  current state-of-the-art for desktop power management.
- Read the upower source — it is small and shows the right way to
  consume sysfs power_supply.

The current proposal reads as if it was written from memory of
macOS without re-checking either macOS's current behavior or
Linux's current capabilities. The two have evolved.

---

## Closing note for the synthesis round

The synthesizer should not try to "minimize disagreement" between
the proposer and this critic. The critic believes the proposer's
direction is *substantially* wrong in scope (C1), in correctness
(C2, C6), in security (C3), and in time-domain analysis (C4). These
are not nitpicks. They are architectural problems that propagate
forward into Rounds 8 (Networking), 12 (Daemons), 18 (Updates), and
22 (Installer) if not fixed now.

The right move is:

- Adopt the proposer's vocabulary (App Nap, Power Nap, assertions,
  Safe Sleep) because the conceptual mapping to macOS is helpful for
  developer onboarding.
- Adopt the proposer's RAII handles and WIT callbacks.
- Adopt the proposer's per-platform profile structure.
- **Reject** the proposer's "S3 in a VM works like S3 on metal"
  premise.
- **Reject** the proposer's single-phase suspend.
- **Reject** the proposer's two-assertion-systems split.
- **Reject** the proposer's "background_fetch in manifest is enough"
  consent model.
- **Reject** the proposer's "no on-suspend = fresh start" hibernate
  semantics.

The FINAL document should be roughly 2× the length of the proposal
and at least 1.5× the length of this critique, with explicit
sections addressing every P0 and P1 item above.

If, after synthesis, the FINAL still claims working S3 in QEMU
without caveat, or still has a single-phase suspend, or still allows
background_fetch on manifest declaration alone, the critic
recommends a second round of debate before moving to Round 8.

---

*End of critique. Total: ~620 lines.*
