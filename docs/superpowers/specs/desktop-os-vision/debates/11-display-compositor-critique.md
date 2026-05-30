# Round 11 Critique — Display Server & Compositor

**Role:** Critic
**Date:** 2026-05-29
**Verdict:** REJECT AND REDESIGN — the design lands the right structural moves at a high level (per-window registry, damage tracking, vsync-driven thread, FontProvider trait) but the underpinnings cannot survive contact with the rest of VyomaOS as already shipped. The **single registry Mutex held across an entire composite pass**, the **stdout-text protocol now carrying 14 verbs at 60 Hz**, the **SharedMemory aliasing trick**, the **DRM page-flip path under virtio-gpu**, and the **chrome-draws-into-FB approach** each have a concrete failure mode that breaks one or more shipped subsystems. Six BLOCKING issues require structural changes; eight NON-BLOCKING concerns need spec-level tightening.

---

## 1. Verdict summary

- **The registry Mutex is the design's worst defect.** §2.1 holds `Mutex<WindowRegistry>` for "~3–8 ms" per composite pass while parser threads must wait. At 60 Hz with 10 apps and steady-state draw rates measured in the current `app_threads.rs` at ~50 draw lines per app per frame, that lock collides on every frame. The architect assumes parsers are "I/O bound on the pipe" but the current code path holds `app_registry` to mark `draw_ticks`, `APP_DIRTY`, and the per-app `surface` Arc on *every single VYOMA_DRAW line* (see `supervisor/src/router.rs:30-32`, `supervisor/src/draw_cmd.rs:55-67`). A single 8 ms compositor pass blocks 500+ draws across all apps. The "Mutex for v1, RwLock for v2" answer in §2.1 is not a v1-shippable answer; v1 will deadlock-prone-livelock under steady use.

- **The proposal embeds the compositor in PID 2 but never accounts for the IPC broker and watchdog ticks that already live there.** §1.1 cites QoS to bless this colocation, but the current supervisor has the IPC route path running in the *same thread* as the parser (router.rs:18 onwards). A compositor pass that holds the FB mutex for 8 ms holds it across `draw_chrome_onto` (chrome.rs), which itself walks the registry. Under load, watchdog ticks (which depend on per-app `last_output` updates also gated by registry locks) miss their deadlines, and the supervisor erroneously kills the wrong app. There is no acknowledgement of this lock graph anywhere in §1 or §2.

- **VYOMA_DRAW Protocol v2 on stdout pipes is a regression, not an extension.** §3 doubles the verb count, introduces a stateful handshake (`hello:` → `VYOMA_DISPLAY_READY:`), adds per-app parser state machines that span frames (`AppParserState.current_iid`), and routes everything through `route_or_print` which is a single-threaded line parser already doing IPC routing. The cost model in §5.1 admits 2.7 ms/frame just to parse 720p pixels via fill_rect; this is dismissed by recommending SharedBuffer, but SharedBuffer doesn't exist yet (it's WASI Preview 3-adjacent). The v1 design ships a known-too-slow path with a "use SharedBuffer if you care about perf" workaround that is itself a blocked future feature.

- **SharedBuffer in §5 is based on a Wasmtime mechanism that does not work the way the design claims.** `wasmtime::SharedMemory` requires the WASM module's memory to be declared as `shared` in its WAT/source. `wasm32-wasip2` targets in stable Rust do **not** emit shared memories — the toolchain has no `-C target-feature=+atomics,+bulk-memory,+shared-memory` story for `wasip2` today. The architect's "alias the Surface buffer with WASM linear memory" requires either a custom WIT hostcall returning a buffer the WASM module can write to (which doesn't help — the host can't observe the writes without polling) or a fundamental change to how apps are compiled (`wasm32-wasi-threads`, not `wasm32-wasip2`). Either way, the path described in §5.2-§5.3 will not compile against the current toolchain.

- **DRM page-flip vsync (§8.1) silently doesn't work under virtio-gpu in QEMU,** which is the only currently-supported display path. QEMU's virtio-gpu DRM driver historically does NOT report VBLANK events through the page-flip event mechanism (only synthetic timestamps on commit). The "fallback timer" the architect adds is then the actual path on every QEMU boot — and `std::thread::sleep(Duration::from_micros(...))` at 16,666 µs target has ±2-5 ms jitter on stock Linux, which delivers 30-45 Hz effective rate. The design states "60 Hz on desktop" as if it is the default; under QEMU it is not.

- **Chrome painting into the framebuffer back buffer (§10.3) breaks damage-rect culling.** §10.3 paints chrome directly into `fb.back` via `fb.fill_rect`, *outside* the Surface. But the screen-damage union in §4.1 is built from window-local damage that excludes chrome. A focus change (one app becoming key) requires a chrome repaint that is not in any window's `damage`. The compositor in §4.1 step 7 calls `flush_region(&screen_damage)` — which will *not* flush the chrome change, because chrome painted "for free" outside the damage union. The architect notes this in question 7 as a known issue and proposes deferral, but it makes the v1 design produce visually wrong frames on focus change.

- **Window resize is admitted to flash blank for one frame (§15 question 8),** which is unacceptable for a "macOS-fidelity" target. There is no `swap_in_resize` mechanism specified. This is not a minor polish issue; it is a user-visible regression versus the existing v1 system, which resizes by re-tiling without losing pixels because the pre-resize Surface is reused.

- **The `IidKey: u64` namespace allows another defect class: stale handle reuse race.** §2 has `IidKey::new()` using `AtomicU64`, never reusing. Fine. But §7.2 has `state.local_to_iid: HashMap<u32, IidKey>` where the app's `local_id` is `u32`. The dispatch in §7.2 never validates that the app's `local_id` is still alive — a `SelectWin { local_id: 5 }` after that window has closed (and the registry GC'd it) silently re-points `state.current_iid` to a stale `IidKey`. The next `FillRect` calls `get_mut(iid)` which returns `None` and silently drops the draw. The app sees no error; the pixels just don't appear. This is exactly the bug class the v2 protocol was meant to eliminate.

- **No back-pressure on the stdout pipe.** The architect proposes a 1024 bad-lines-in-60-s circuit breaker (§3.5) for *malformed* lines but says nothing about *valid* lines under flood. An app that writes 100,000 `fill_rect` lines per second (trivial to write) overwhelms the parser thread, which holds the registry mutex briefly per line and re-acquires it 100,000 times. Compositor passes are starved; other apps' parsers wait. The current code is already vulnerable to this; the v2 design makes it worse by increasing per-line work.

---

## 2. BLOCKING issues (must fix before synthesis)

### 2.1 Single registry Mutex held across composite pass is a livelock generator

§2.1 reads:

> "The compositor pass holds the registry mutex for the duration of one frame (~3–8 ms). During the pass, app parser threads that want to mutate their Surface or push a damage rectangle must wait. This is acceptable because parser threads are I/O bound..."

This is wrong. The current code path that parser threads take (see `supervisor/src/draw_cmd.rs:55-67`) holds `app_registry.lock().unwrap()` *twice* per draw line — once to read `win_z` and once to mark `APP_DIRTY` — and the proposed v2 path adds a third call to update window-local damage. At a sustained 50 draws/app/frame across 10 apps, that is 1500 registry lock acquisitions per frame *that the compositor blocks*. With an 8 ms compositor hold, the cumulative parser wait is 8 ms × (15 apps queued in worst case) = ~120 ms. The compositor itself is now starved on the *next* frame because parsers are racing to drain their queued lock requests.

**Quantitative back-of-envelope:** Linux `pthread_mutex_lock` under contention costs ~200 ns when uncontended, ~2 µs when contended (futex syscall + scheduler park). At 1500 acquisitions/frame × 2 µs = 3 ms of pure lock-wait per frame, on top of the 8 ms compositor hold. That is 11 ms just on lock contention out of a 16.6 ms budget — leaving 5.6 ms for actual draws, parses, and IPC routing. The first frame where any individual draw command runs slow (a large `fill_rect` is 600 µs at 720p) pushes total work past the 16.6 ms ceiling. Compositor starts to fall behind; next frame's `vsync.wait()` returns immediately with the missed deadline; cascading frame drops follow.

There is also an **ABBA deadlock risk** the proposal does not address. The current code (per the most recent commit, `55fd121 fix(compositor): eliminate ABBA deadlock in flush pass`) already had to be patched to break a lock-ordering cycle between `app_registry` and the framebuffer mutex. The v2 design re-introduces this risk by having the compositor hold the registry lock *and* the FB lock simultaneously for the entire pass, while parser threads acquire them in `app_registry → FB` order in `draw_cmd.rs:55,71`. Any future refactor that touches both locks in the wrong order resurrects the deadlock. The architect needs to specify a global lock-ordering invariant ("FB Mutex < Registry Mutex always") and audit all callers.

This is a textbook "convoy" pattern. The mitigations the architect names are insufficient:

- "Parsers are I/O bound on the pipe" is false at the per-frame granularity that matters. They read whole bursts of lines at once (the pipe has 64 KB buffer) and process them in tight loops; they're CPU-bound for the burst.
- "Hold the lock < 100 µs per command" (§7.4) doesn't help when the *compositor* holds it for 8 ms.
- "We'll go to RwLock in v2" leaves v1 unshippable.

**Mandatory fix:**

The compositor pass must NOT hold the registry lock for the duration of the pass. Required structure:

```rust
// Phase A: snapshot under lock (< 200 µs)
let snapshot = {
    let reg = registry().lock().unwrap();
    reg.snapshot_for_composite()  // clones WindowRecord refs, damage rects, z-order
};

// Phase B: composite without lock (3-8 ms)
self.composite_from_snapshot(&snapshot, &mut fb);

// Phase C: clear damage under lock (< 100 µs)
{
    let mut reg = registry().lock().unwrap();
    for iid in snapshot.iids_composited() {
        if let Some(win) = reg.get_mut(iid) {
            win.damage = win.damage.subtract(snapshot.damage_at(iid));
        }
    }
}
```

The Surface itself cannot be cheaply cloned (it's MBs). The fix requires:

- Surface owned by `Arc<Mutex<Surface>>` (as currently in `AppState.surface: Option<Arc<Mutex<Surface>>>` — see `supervisor/src/app_threads.rs:44`).
- WindowRegistry stores `Arc<Mutex<Surface>>`, not `Surface` inline. The proposed `pub surface: Surface` field in `WindowRecord` (§2) blocks this.
- Compositor takes the Arc, releases the registry lock, then locks the Surface for the blit duration.

The architect must either accept this restructuring or prove the 8 ms hold is safe with measurements (not assertions). Without it, the system livelocks under realistic load on `desktop-full` and falls below 15 Hz on `mobile`.

### 2.2 Compositor in PID 2 starves IPC broker and watchdog

The supervisor currently routes IPC messages on the same threads that parse stdout (see `supervisor/src/router.rs`). A compositor thread holding the framebuffer Mutex *and* the registry Mutex blocks both:

- `chrome::repaint_all_borders` which walks the registry and touches `app_registry.lock()` (used in `app_threads.rs:214` after every spawn).
- IPC routing decisions that need to look up `win_region` for senders (see `route_or_print` in router.rs).
- Watchdog `last_output` updates (in the wait thread; see `app_threads.rs:338-339`).

The architect claims §1.2 that QoS solves this. It does not. QoS lets the compositor *run* — it does not let other threads *acquire* a lock that the compositor holds. SCHED_FIFO priority 80 on the compositor means lock contention is even worse because a parser at SCHED_OTHER nice 0 cannot preempt and clean its work; it just queues.

**Mandatory fix:**

Either:

(a) **Move IPC broker to a dedicated thread** that never touches the framebuffer or display registry. The current colocation in `route_or_print` must split.

(b) **Make the display registry a read-mostly structure** so the compositor never holds the lock during composition (per 2.1's fix). This is the more surgical change.

The architect should state explicitly which threads in the supervisor are forbidden from acquiring the framebuffer Mutex, and prove that the IPC broker and watchdog reaper are in the "forbidden" set.

### 2.3 VYOMA_DRAW Protocol v2 over stdout is the wrong transport

§3 ships 14 new verbs, a handshake, and a per-app stateful parser, all over `child.stdout` lines parsed by `route_or_print`. Three problems:

1. **Throughput.** Each line costs ~50 ns of formatting in the app + a syscall write to the pipe + the supervisor's read loop calling `String::from_utf8_lossy` on each line + `split_once(':')` + parsing every field as a string. At 60 fps × 20 draws/window × 10 windows = 12,000 lines/sec, the parser thread is dominated by allocation and UTF-8 validation.

2. **Injection.** `route_or_print` (router.rs:28) checks `line.strip_prefix("VYOMA_DRAW:")`. Any log message an app writes to stdout that happens to start with `VYOMA_DRAW:` is dispatched as a draw command. An app's `eprintln!` is safe (stderr), but its `println!` is a vector. The proposal's circuit breaker (§3.5) only fires on *parse errors* — a *valid* injection (a syntactically correct `VYOMA_DRAW:fill_rect:0,0,1920,1080,FF000000`) trips no breaker and overwrites the user's screen.

3. **No framing.** A single line longer than the pipe buffer is split mid-line by the kernel. The architect's parser in §3.4 returns `ProtoError::BadArgs` on a truncated line; this is a silent partial-frame loss. There is no end-of-frame marker that allows the supervisor to detect "I got `begin_frame` but not `end_frame`" and reject the half-frame. `EndFrame` may simply never arrive if the pipe was full during writeback.

4. **No back-pressure.** If the compositor is slow draining damage and the parser is fast accepting draws, the parser thread races ahead and the registry's damage rect grows to cover the whole window every frame, defeating the damage optimization. There is no `EWOULDBLOCK` semantic on the stdout pipe — the WASM app's `println!` blocks the entire WASM thread when the pipe is full, which is a hidden source of jitter that violates the architect's frame-budget breakdown in §4.5.

5. **`String::from_utf8_lossy` per line.** The `BufReader::read_line` in `app_threads.rs` (line 8 imports `BufRead`) allocates a `String` per line. At 12,000 lines/sec across all apps, that is 12,000 allocations/sec going through the system allocator. Each allocation churns the allocator's freelist and pollutes L1. The current v1 protocol survives this because it has only 4 verbs and short lines; v2's `draw_text` carries arbitrary UTF-8 strings that can exceed the 64-byte SSO threshold.

**Mandatory fix:**

Either:

(a) **Frame the protocol.** Use a length-prefixed binary protocol over a *separate* fd (the supervisor opens a second pipe per display-capable app; the WASM app gets a fd as a WASI Preview 2 resource). Each frame is `[u32 length][u8 verb][u8... payload]`. The parser allocates once per frame, not once per command. The injection vector closes because stdout no longer carries display commands.

(b) **Or strictly partition stdout.** Apps writing to stdout *cannot* emit `VYOMA_DRAW:`-prefixed text from their println path. The supervisor synthesizes a separate file descriptor (via WASI) for display, and parses display commands only from there. This is the simpler v1 path; it requires a manifest extension (`display_fd = 3`) and a WIT hostcall.

The current "everything on stdout" model worked at v1 with 4 verbs and no statefulness. v2 with 14 verbs, frame delimiters, and shared state requires real framing.

### 2.4 SharedBuffer based on `wasmtime::SharedMemory` is not how WASI Preview 2 works

§5.2 reads:

```rust
let mem_ty = wasmtime::MemoryType::shared(npages as u32, npages as u32);
let mem = SharedMemory::new(engine, mem_ty)?;
```

For this to work, the WASM module must declare its linear memory as shared in its compiled binary. The `wasm32-wasip2` target in stable Rust does not produce shared memories — it produces single-threaded linear memory. The toolchain to compile a Rust WASM app with `--shared-memory` is `wasm32-wasi-threads` (which is unstable), not `wasm32-wasip2`. The proposed design therefore:

- Either requires every shared-buffer-capable app to retarget to `wasm32-wasi-threads`, splitting the toolchain matrix.
- Or requires patches to Wasmtime to lie to the linker about memory sharedness, which is upstream work.

Beyond the toolchain, the *aliasing* claim — that `SharedMemory.data()` pointer is the same as `Surface.buf` pointer — is not what the API does. `SharedMemory::new` allocates fresh pages; you cannot mmap an existing `Vec<u8>` into it. The architect's `std::ptr::copy_nonoverlapping(surf.as_ptr(), mem.data()...)` in §5.2 is a *copy at setup time* — exactly the opposite of zero-copy. To compose from the shared memory, the compositor must `read_bgra` from the SharedMemory's address space, which is a different `Vec` than `Surface.buf`. The current code in `display/surface.rs:120` (`blit_surface`) reads from `surface.buf`. The proposal does not specify how the blit changes.

The actual mechanism for zero-copy of WASM memory to a Surface requires one of:

1. **Wasmtime custom memory creator** (`Config::with_host_memory`) that lets the supervisor own the allocation and hand it to Wasmtime. Wasmtime supports this for `Memory`, not for `SharedMemory`. Doable but invasive.

2. **WASI hostcall returning a buffer**: WASM calls `display::get_surface_ptr()` which traps if the buffer isn't ready; the host writes into Wasmtime memory directly using `Memory::data_mut`. No "sharing" — the host writes during the hostcall. This is the model that actually works with `wasm32-wasip2`.

3. **mmap a shared file** between the supervisor and a forked-out WASM child (back to the dedicated-process model the architect rejected in §1.1).

The proposal must replace §5.2-§5.3 with one of these, and the new mechanism must be evaluated against the toolchain reality. As written, the SharedBuffer section is a non-implementable optimization, which means §5.1's 2.7 ms/frame parse cost stays — and that 2.7 ms exceeds the entire 4 ms p99 frame budget at 250 Hz, and is 16% of the 16.6 ms budget at 60 Hz, *before* the actual composite work runs.

Additionally, the double-buffering scheme in §5.3 has a subtle correctness bug:

```rust
pub fn swap(&mut self) {
    std::mem::swap(&mut self.front, &mut self.back);
    self.front_idx.fetch_xor(1, Ordering::AcqRel);
}
```

`std::mem::swap` on a `SharedMemory` value swaps the *handles*, but the WASM module's linear-memory binding was set up against one specific handle at instantiation. After `swap`, the WASM module still writes to the *original* memory (now called "front" after the swap), not to what's logically the "back". The double-buffering doesn't actually flip from the WASM module's perspective unless the supervisor re-binds the import — which Wasmtime does not support after instantiation. The `swap` operation as drawn is a no-op for the WASM-visible memory; the compositor reads whatever the app most-recently wrote, including partial writes.

Real double-buffering with WASM requires either:

- Two separate linear-memory imports (`mem_front`, `mem_back`) and an app-cooperative protocol that says "after `swap` signal, write to the *other* import name". Requires source changes in every WASM app.
- A single linear memory + an offset: front = bytes [0..N], back = bytes [N..2N]. App writes to the back-offset, then `swap` toggles a `front_offset` value the compositor uses. This works but requires the app to track the offset itself.

Neither matches the "transparent zero-copy" framing the proposal claims. The architect must either pick one and document the app-side contract, or admit SharedBuffer is single-buffered (with tearing) in v1.

### 2.5 DRM page-flip vsync does not work under virtio-gpu

§8.1 describes `DrmVsyncSource` that reads VBLANK events from `/dev/dri/card0`. The virtio-gpu driver under QEMU (which is the *only* GPU path in the current build matrix; see Makefile `run-gui DISPLAY_BACKEND=cocoa|sdl`) emits page-flip completion events but its vblank reporting depends on host-side compositor support. Under QEMU + cocoa or QEMU + sdl, the virtio-gpu DRM driver emits "synthetic" vblanks at the configured refresh rate, but the timing of these is the *Linux scheduler's* timing of the kernel thread that fakes the vblank — not a hardware-derived clock.

The architect's "fallback to TimerVsyncSource" thus becomes the *de facto* path in QEMU, and:

- `std::thread::sleep` has documented ±5 ms jitter on Linux 5.10 with `CONFIG_HZ=250` (which the VyomaOS kernel uses per current allnoconfig).
- A 16,666 µs sleep at 5 ms jitter delivers 11,666 to 21,666 µs frames = 46 to 86 Hz, mean 58 Hz, p99 way off target.
- The `self.next_us = self.next_us.max(now) + self.period_us` in §8.1 has a subtle bug: if a previous frame ran long, `now > next_us`, and `next_us.max(now) + period_us` resets the schedule from `now` — discarding accumulated catch-up. After a slow frame, the next frame schedules from the new now, dropping the frame and not recovering. This is the "30 fps lock-in" anti-pattern.

**Mandatory fix:**

- Use `timerfd_create(CLOCK_MONOTONIC, 0)` with `TFD_TIMER_ABSTIME` and an absolute deadline, *not* `std::thread::sleep`. Read the timerfd in the compositor thread (or via R10's TimerLoop).
- The "next_us" calculation must accumulate: `next_us += period_us` regardless of `now`, allowing the scheduler to catch up by running multiple frames back-to-back if it falls behind. Cap the catch-up to 3 frames to avoid runaway.
- Test under all three QEMU display backends (cocoa, sdl, headless) and report measured p99 jitter before declaring 60 Hz.
- Document that on virtio-gpu, vblank timestamps are scheduler-derived, not hardware-derived, and that the design accepts this jitter as best-effort.

### 2.6a Memory budget for per-window Surfaces overshoots `mobile` cap

§5.7 quotes 3.5 MB per 1280×720 RGBA surface. The proposal is "one Surface per window" (§0). On `mobile` (256 MB RAM ceiling per CLAUDE.md), 10 concurrent windows × 3.5 MB = 35 MB for Surfaces, plus 7 MB framebuffer back-buffer (1920×1080 at full HD) + at-least the same again for the front-buffer view = ~50 MB just for display state. That is 19.5% of total mobile RAM before any WASM heap, before Wasmtime overhead (~5 MB per instance), before the supervisor's own working set.

On `mcu-minimal` (128 KB ceiling), a single 320×240 RGBA surface is 307,200 bytes — already 240% of the available RAM. The proposal acknowledges this only obliquely ("SharedBuffer unsupported on mcu-minimal" in §5.7) but says nothing about the *base* Surface. The mcu-minimal target must use either an 8bpp paletted format, a 1bpp monochrome surface, or no off-screen Surface at all (direct framebuffer drawing). The proposal does not specify which.

**Mandatory fix:** Per-platform Surface pixel format and dimensional cap, declared in the platform profile TOML. Default 32bpp RGBA for desktop/mobile/server; 16bpp RGB565 for iot-edge/robotics-rt; 1bpp for mcu-minimal with no off-screen surface (direct draw). The Surface API must be generic over pixel format, or the platform profile must select a Surface implementation at boot.

### 2.6 Chrome painted outside the Surface invalidates damage-rect culling

§10.3 paints chrome directly into `fb.back` via `fb.fill_rect(win.bounds.x as u32, win.bounds.y as u32, win.bounds.w, TITLEBAR_H, bar_color)`. This is *not* within any Surface. The compositor pass in §4.1 step 4 composites window content using the union of *window-local damage* — it knows nothing of chrome.

Failure modes:

- **Focus change.** User clicks window B. WM updates `is_key`. No window's content changed; no window has damage. Compositor in §4.1 step 1 returns "nothing to do". But chrome of A needs to repaint (dim titlebar) and chrome of B needs to repaint (bright titlebar). Neither happens.

- **Window drag.** WM updates `bounds.x`. The window's Surface didn't change. The window's chrome did (titlebar text position). Damage doesn't capture this. Cursor moves over the new chrome location; old chrome remains; new chrome doesn't appear.

- **Background fill in damage union.** §4.1 step 3 paints background over the damage union. The chrome is *outside* any window's surface damage but *inside* the window's bounds. If a small region inside the window content has damage, the damage union expands only that small region. The chrome above it is untouched, *as intended* — but if the entire window moves, the chrome must follow, and the damage tracking doesn't know to dirty the old chrome strip.

**Mandatory fix:**

Choose one:

(a) **Chrome lives in a sibling Surface owned by the registry per-window.** `WindowRecord` has both `content_surface` and `chrome_surface`. WM signals "chrome dirty" by setting damage on the chrome_surface. The composite pass blits both. This doubles per-window surface allocation cost but cleanly composes.

(b) **Chrome is painted into the same Surface as content.** WindowRecord.surface is window bounds × bounds + titlebar; content draws into a sub-region; chrome paints into the top strip. App-side `(0,0)` is the top-left of content (titlebar height offset added internally). The compositor blits the entire Surface; chrome is in the damage union naturally.

(c) **WM signals "chrome damage" via the registry.** A separate `chrome_damage: DamageRect` field tracks bbox-of-chrome-needing-repaint, unioned into screen damage by the compositor.

The architect's question 7 acknowledges this; the answer is not "defer" — it is a v1-shippable choice between (a)/(b)/(c). I recommend (b) as the simplest.

Note that the current codebase already has the issue: `draw_chrome_onto` (chrome.rs) paints directly into FB, and `repaint_all_borders` is called explicitly on z-order changes (app_threads.rs:214). The v2 design must explicitly *replace* this path, not paper over it. If the synthesis picks (b), the existing `chrome.rs` becomes a Surface-aware module that paints into `WindowRecord.surface`, and `draw_chrome_onto`/`repaint_all_borders` are deleted. If (a) or (c), the chrome.rs API changes shape but the call sites stay.

### 2.7 No explicit `create_window` handshake — first-draw race

Even with v2's `create_window` verb, the proposal in §7.3 ships an *implicit* default-window synthesis for v1 apps: "the first time a v1 app emits any non-hello command, the dispatcher auto-creates a default window". This is a race:

- WASM app `println!("VYOMA_DRAW:fill_rect:0,0,1920,1080,FF000000FF")` immediately after process spawn.
- Supervisor's parser thread has not yet received the `apply_tiling_layout` callback (it runs on the lifecycle thread).
- Default window is created at full-screen size before the layout system claims its region.
- Tiling layout subsequently runs and shrinks the window; the Surface allocated by the default-window path is the wrong size.

The current code in `app_threads.rs:211-215` calls `apply_tiling_layout` *synchronously* before spawning IO threads, so the window region is set before any draw arrives. The proposal does not specify whether default-window-synthesis runs *before* or *after* tiling, and gets this wrong either way:

- Before tiling: Surface is full-screen, then tiling shrinks it. Reallocation + re-render.
- After tiling: first draw arrives before tiling completes; supervisor either drops the draw or creates a wrong-sized Surface.

**Mandatory fix:** Explicit `create_window` is required *before any draw command* for v2 apps. For v1 apps, the supervisor synthesizes the window *eagerly at spawn time*, before any draw lines can arrive, and sizes it to whatever `apply_tiling_layout` decides. The spec must say: "v1 default-window is created during `launch_app_threads` immediately after `apply_tiling_layout`, before parser threads start consuming stdout".

---

## 3. NON-BLOCKING concerns

### 3.1 `IidKey` stale-handle race in `local_to_iid`

§7.2's `local_to_iid: HashMap<u32, IidKey>` is never cleared when a window closes. After the registry GC's a Closed window, the app's local-handle map still has the old `IidKey`. A `SelectWin { local_id }` to that handle silently re-binds `current_iid` to a defunct key; subsequent `FillRect` calls `get_mut(iid)` returns None and is dropped. No error to the app. The app sees its draws vanish.

**Fix:** on GC, send a `VYOMA_DISPLAY_EVENT:window_closed:<local_id>` callback to the owner app, and require the app to remove its local mapping. The supervisor refuses `SelectWin` for unknown `local_id`s with a structured error reply.

### 3.1a `last_frame_ms: u64` is wall-clock, not monotonic

§2's `WindowRecord.last_frame_ms` field is set "wall-clock of last begin_frame". Wall clock can step backward under NTP adjustment, sleep/wake, or virtio-rtc resync. Frame-budget logic and animation timing must use `CLOCK_MONOTONIC`. The existing `animator::now_ms` (`supervisor/src/display/animator.rs`) appears to use monotonic time; the new `last_frame_ms` should be specified to use the same source explicitly.

**Fix:** Rename to `last_frame_monotonic_us` and require it sourced from `display::animator::now_ms` or equivalent.

### 3.2 `next_top_z()` linear scan per create

§2's `next_top_z` is `O(N)` over all windows. Plus `rebuild_z_cache` is `O(N log N)` on every create/raise/lower. For a desktop with 20 windows and a Window Manager doing focus-cycling, this is fine, but on a long-running session with many short-lived sheets/popups, fragmentation of `z_order` integers will overflow `i32` eventually (yes, billions of raises, but a misbehaving app cycling focus is a DoS).

**Fix:** Z-order is a sorted Vec<IidKey>, not a `z_order: i32` field. Raise/lower is `Vec::remove + Vec::push`. No integer arithmetic, no rebuild.

### 3.3 `subtract_rect` returning the original rect "unchanged unless subtraction produces another rectangle"

§6.4 says "conservative: returns the original rect unchanged unless the subtraction produces another rectangle". This means *most* of the time, occlusion does nothing. Two windows offset by half: subtraction produces an L-shape, not a rectangle, so occlusion gives up and composites both. On a typical desktop, occlusion is dead code.

**Fix:** Either ship real region subtraction (regions-as-vec-of-rects, ~150 lines, not 300) in v1 or admit occlusion doesn't fire in v1 and remove it from the spec to save the implementation cost. The "v1 ships an optimization that almost never fires" path is the worst of both options.

### 3.3a `is_opaque` predicate `surface_is_opaque(win)` is not specified

§6.4 mentions `surface_is_opaque(win)` as the gate on occlusion culling. The proposal does not say how this is computed. Three candidates:

1. **Scan the surface every frame** for any pixel with alpha < 255. O(W×H) per window per frame. At 1280×720 × 10 windows × 60 Hz = 553 MPixel/s of scan-only work. Defeats the optimization.

2. **App declares opacity** in a manifest field (`opaque_content = true`) and the compositor trusts it. Faster but wrong: an opaque-declared app that draws a transparent pixel produces incorrect compositing.

3. **Track per-Surface "any-transparent" bit** updated by `fill_rect` when alpha < 255. The compositor checks the bit. The bit is "sticky": once any non-opaque draw lands, the bit stays set until the next `clear()`. This is the workable middle ground but is not in the proposal.

**Fix:** Pick option 3 and specify the `Surface.has_alpha: bool` field, sticky-reset on `clear()`.

### 3.4 `should_skip` uses a global static `THERMAL_SKIP_MASK` without coordinating with vsync wait

§9's `should_skip` is called *after* `vsync.wait()` returned. The compositor has already paid the cost of waking on a vblank, then decides to skip the frame. Wasteful: the wakeup cost on virtio-gpu is ~50 µs, paid for nothing.

**Fix:** Vsync skip is implemented by the vsync source itself (`AnyVsync::skip_next(n)` actually skips n events; doesn't return until the n+1th). The current `skip_next(&mut self)` taking unit is too coarse — it can't express "skip 3 in 4 at tier 2".

### 3.5 `ProtoVersion::V1` and `V2` enum will fossilize at the worst time

§2's `ProtoVersion: V1 | V2` is exhaustive. When v3 ships (multi-layer compositing, per §3.3), the registry and every match must update simultaneously. The current versioning is flag-day.

**Fix:** Use `u8` for protocol version with a const range; add unknown-version fallback to v1. Match arms explicitly handle "if proto >= 2" rather than "if proto == V2".

### 3.6 `Framebuffer::new_for_test` and `Framebuffer::screenshot` referenced in §14 — do they exist?

§14.1 says "All of these use `Framebuffer::new_for_test` (already exists)". I checked `supervisor/src/display/mod.rs`; the file has no `new_for_test` constructor (the `mmaped: bool` field hints at one, but no `pub fn new_for_test`). §14.2's `Framebuffer::screenshot("/tmp/...ppm")` also is not visible. The test plan presumes infrastructure that may not exist.

**Fix:** Either reference the actual symbol or specify it as part of the implementation.

### 3.7 Shared `MutexGuard` lifetime on `iter_back_to_front_mut`

§4.1 step 8 has `for win in reg.iter_back_to_front_mut() { win.damage = DamageRect::empty(); }`. Rust's borrow checker disallows `iter_back_to_front_mut` that yields `&mut WindowRecord` from a `HashMap<IidKey, WindowRecord>` keyed by sorted Vec<IidKey> — you can't have a mut iterator over HashMap entries in arbitrary order via a separate Vec<Key>. The architect's API as drawn does not compile.

**Fix:** Specify the iter as `iter_back_to_front_mut(&mut self) -> impl Iterator<Item = &mut WindowRecord>`, which is implementable via `self.z_order_cache.iter().filter_map(move |k| self.windows.get_mut(k))` — wait, no, that's a double borrow. The correct API is:

```rust
pub fn for_each_visible_mut<F: FnMut(&mut WindowRecord)>(&mut self, mut f: F) { ... }
```

This is a doc-level fix but it points at a deeper issue: the registry data model conflates ordering and ownership and forces awkward iteration.

### 3.8 `BitmapFontProvider::measure` returns `(gw * chars, gh)` but `chars` includes non-printable characters

§11's `chars().count()` includes control characters (`\n`, `\t`, etc.) that don't advance pen position. The measure differs from the rendered width, breaking layout for any text containing punctuation that the font handles specially. The current `font.rs` already deals with this; the new abstraction loses it.

**Fix:** `measure` walks chars same as `render_into` and skips zero-width chars; or document that `BitmapFontProvider` does not support non-printable input and require apps to sanitize.

### 3.9 Mouse cursor restore-under-cursor race with composite

§4.1 step 2 calls `fb.restore_under_cursor()` then step 6 calls `fb.draw_cursor()`. Between them, the composite blits Surfaces that overlap the cursor area. The pixels under the cursor are now the *new* composited content, but `saved_under` was captured *before* the composite. When `draw_cursor` saves the new pixels under the new cursor position into `saved_under`, that's correct. But if the cursor moves *during* the composite (mouse_input.rs runs on a different thread and updates `cursor.cx/cy`), the next frame's `restore_under_cursor` restores stale pixels to the old position, producing a cursor trail.

**Fix:** Cursor draw/restore must be atomic with the composite under the framebuffer Mutex. The mouse_input thread must not be allowed to update `cursor.cx/cy` while the compositor holds the FB lock. Currently mouse_input does take the FB lock (see `display::set_cursor_pos` in mod.rs:96) so this is OK, but the architect should state the invariant explicitly.

### 3.10 `flush_region` not yet implemented; current `flush()` blits everything

§4.1 step 7 calls `fb.flush_region(&screen_damage)`, which §12.1 says is added by this round. The current `flush()` (display/mod.rs `flush`, not shown but referenced by `draw_cmd.rs:109`) uses `dirty_top`/`dirty_bottom` from `Framebuffer`. The new `flush_region` is a different API; the proposal doesn't say what happens to the existing `dirty_top`/`dirty_bottom` fields. Probably dead code, but spec should call this out.

**Fix:** Specify deprecation of `dirty_top`/`dirty_bottom` and replacement with `screen_damage` from the registry; or specify how both coexist.

### 3.11 SharedBuffer memory budget on `mobile`

§5.7 says SharedBuffer is "unsupported on mcu-minimal". It does not say anything about `mobile` (256 MB). Two 1280×720 RGBA surfaces per shared-buffer window = 7 MB. A typical mobile session with 5 such apps = 35 MB. Plus the per-window Surface (non-shared-buffer) for the rest. The proposal needs a per-platform memory cap.

**Fix:** Manifest validator enforces `shared_buffer = true` only on platforms that can afford it; document the cap.

### 3.12 No story for window minimize preserving Surface

§2's `WindowState::Minimised` says "surface preserved". But minimize lasts indefinitely — a user can minimize an app and not return for hours. 8.3 MB held per minimized window on `desktop-full` is OK; on `mobile` (256 MB) with 5 minimized windows, it's 32 MB of pure overhead. The proposal doesn't specify whether minimize *discards* the Surface and rerenders on restore (slow but cheap), or keeps it (fast but expensive).

**Fix:** Per-platform policy: `desktop-full` keeps Surface; `mobile` discards on min and forces the app to repaint on restore (via `VYOMA_DISPLAY_EVENT:restore`).

### 3.13 §3.4's parser uses `String::String` allocation per command

`ProtoCmd::DrawText { ..., text: String }` allocates per draw_text command. At 60 fps × 5 texts/frame × 10 windows = 3,000 allocations/sec, this is hot-path GC pressure (jemalloc/system malloc). The current code in `draw_cmd.rs:39` takes `cmd: &str` and dispatches without allocation.

**Fix:** The parsed command keeps a borrowed `&str` slice into the source line; the parser doesn't own the string. `ProtoCmd<'a>` with lifetime parameter. This requires the dispatcher to be called within the same scope as the parser, which the current model already supports.

### 3.13a Resize atomicity (§15 question 8 should be BLOCKING-adjacent)

The architect's own question 8 admits resize flashes blank for one frame. This is downgraded to "non-blocking" only because there's a path forward, but the spec must commit to one:

```rust
// supervisor/src/display/dispatch.rs (resize handler)
ProtoCmd::ResizeWin { w, h } => {
    let mut r = reg.lock().unwrap();
    if let Some(win) = r.get_mut(iid) {
        let old_surface = std::mem::replace(&mut win.surface, Surface::new(w, h));
        win.resize_in_progress = Some(old_surface);  // keep old visible
        win.bounds.w = w;
        win.bounds.h = h;
        // App is expected to redraw and emit end_frame; on first end_frame,
        // resize_in_progress is dropped.
    }
}
```

The compositor pass blits `resize_in_progress` if it exists *and* the new surface has not yet been damaged. This preserves visual continuity at the cost of one extra Surface allocation during resize. Without this mechanism, every resize is a one-frame flash, which fails the "macOS-fidelity" target stated in §0.

### 3.13b `Mutex<WindowRegistry>` vs `RwLock<WindowRegistry>`: writer starvation

§2.1 mentions upgrading to RwLock in v2. But RwLock on Linux with `parking_lot` is *not* writer-fair by default — under heavy reader load, a writer can starve indefinitely. The compositor is a writer (clears damage post-pass). Parser threads are mixed (read for blit, write for damage). Naive RwLock yields starvation; the architect must pick a writer-preferring RwLock (`parking_lot::RwLock` with `--feature deadlock_detection,fair`) or explicit ordering.

This is an implementation detail that needs to be in the spec because a casual swap from `Mutex` to `RwLock` can make things *worse*, not better.

### 3.14 No `present_layer` or animator integration documented for v1

§3.3 defers multi-layer compositing to v3. But §13.9 says R12 (Animations) will sample animations and "write the resulting scale/alpha into WindowRecord.global_alpha and a forthcoming `transform: Affine2D` field". This is a v2-ish feature that the v1 design must accommodate, because the existing `animator.rs` is in the codebase and used for window open/close fades (see `app_threads.rs:354-359`). The v1 design needs to either:

- Confirm that the existing fade animation still works via `global_alpha`.
- Confirm that the existing close animation triggers the `Dematerialising → Closed` transition.

The proposal mentions these in §6.5 but doesn't wire the existing `animator.rs` to the new state machine. Risk: existing fade animations break under v1 of the new design.

**Fix:** §6.5 should explicitly say "animator.rs's `pending_anim` field on AppState is replaced by `animation: Option<Animation>` on WindowRecord; the compositor pass samples it before composing and writes global_alpha".

---

## 4. What the architect got right

- **Per-window damage rectangles** are the right primitive. The DamageRect API (union, clip) is clean. The compositor's "no damage → no work" early exit is correct.

- **Compositor as a dedicated thread at UserInteractive QoS.** Decoupling composite cadence from app draw cadence is correct. The R5 integration (`set_thread_qos`) is the right hook.

- **WindowRegistry with IidKey as stable identifier.** The decision to make windows owned by the registry, not by AppState, is correct — it lets the registry survive app death cleanly. The proposed lifecycle FSM (`Materialising → Visible → Minimised → Dematerialising → Closed`) maps cleanly to user-visible animation states.

- **FontProvider trait separating R11 from R13** is good engineering. It lets R13 plug in fontdue without re-architecting the display pipeline. Trait-based dependency injection is the right call here.

- **Backward compatibility for v1 protocol** via the default-window synthesis in §7.3 is well-thought-out. Existing apps keep working with zero code change, and they get a full-screen window automatically. This is the macOS approach for legacy NSWindow apps.

- **Splitting `display/mod.rs` into `framebuffer.rs` + `fb_flush.rs` + `mod.rs`** (§12.1) is necessary and overdue — `mod.rs` is currently at exactly 500 lines (`wc -l` confirmed).

- **Capability gating for `shared_buffer`** is the right manifest extension. Even if the underlying mechanism in §5.2 is unworkable today (issue 2.4), the *interface* of opt-in via manifest is correct and lets the SharedBuffer path land later without protocol changes.

- **Thermal tier-driven framerate degradation** (60/30/15) is the right behavior, and the lock-free `THERMAL_SKIP_MASK: AtomicU32` is a clean implementation.

- **Default window for v1 apps and the `hello:` handshake** is exactly the macOS / Wayland model: legacy apps get a synthesized window, new apps participate in negotiation.

- **Test strategy with golden PPM diff** is the right level of integration test — fast, deterministic, catches regressions in the composite pipeline cleanly. Tolerance of ±4 in pixel values is a reasonable bound given bitmap-font anti-aliasing variations.

- **Explicit acknowledgement of v3 deferrals** (§3.3: layers, hardware overlay planes, GPU-accelerated text, HDR) is honest and well-scoped. Rounds 12, 13, 19, 29 are correctly identified as the homes for these. Better to ship a constrained v1 that works than an unconstrained v2 that doesn't.

- **Separation of mechanism (Display Server) from policy (Window Manager)** in §10.1 mirrors X11/Wayland's separation and macOS's separation of WindowServer from the Dock + Finder. The table in §10.1 is the right cut — Display Server doesn't decide what's-focused, just paints what-the-WM-says-is-focused.

- **Frame-budget breakdown in §4.5** is the right level of detail for v1: numbers attached to each pipeline stage, total compared against the 16.6 ms ceiling. The breakdown itself is wrong (issue 2.1 shows it understates lock cost), but the discipline of providing one is correct and easier to refine than to invent.

---

## 5. Questions for synthesis

1. **Surface ownership in WindowRecord (issue 2.1).** Is the `Surface` field replaced with `Arc<Mutex<Surface>>` so the compositor can release the registry lock during the blit? Confirm yes/no; if no, explain how the 8 ms hold doesn't livelock parsers.

2. **IPC broker thread isolation (issue 2.2).** Is the IPC broker moved out of the parser thread to a dedicated thread that never acquires the framebuffer Mutex? Confirm the new thread topology in the synthesis.

3. **Protocol transport (issue 2.3).** Does v2 ship over stdout or over a dedicated framed channel? If stdout, what is the injection mitigation and the framing-on-partial-write recovery? If dedicated channel, what is the manifest extension and the WASI Preview 2 resource model?

4. **SharedBuffer mechanism (issue 2.4).** Concretely, which of the three real mechanisms is chosen (custom memory creator, hostcall + Memory::data_mut, fork+mmap), and what toolchain changes does each require? If none of these is acceptable for v1, is SharedBuffer deferred to v2, and does v1 ship the 2.7 ms/frame parse overhead?

5. **Vsync source (issue 2.5).** Is the implementation switched to timerfd with absolute-deadline + accumulating next_us, and does the spec acknowledge virtio-gpu's scheduler-derived vblank? What is the measured p99 jitter on each QEMU display backend before declaring 60 Hz?

6. **Chrome and damage tracking (issue 2.6).** Is chrome painted into the Surface (option b) or into a separate chrome_surface (option a) or signaled via chrome_damage (option c)? Pick one.

7. **`local_to_iid` stale-handle cleanup (issue 3.1).** Specify the close-notification protocol from supervisor to app.

8. **Z-order representation (issue 3.2).** Switch to sorted Vec<IidKey>, or keep `z_order: i32`?

9. **Occlusion region algebra (issue 3.3).** Ship real region subtraction in v1 (~150 lines), or remove occlusion claims from v1 spec entirely?

10. **`flush_region` and existing `dirty_top`/`dirty_bottom` (issue 3.10).** Specify deprecation and migration.

11. **Per-platform Surface budget (issue 3.11, 3.12).** Add explicit RAM cap per platform for `shared_buffer`-eligible windows, and decide minimize-discard policy on `mobile`/`mcu-minimal`.

12. **`Framebuffer::new_for_test` and `screenshot` (issue 3.6).** Confirm these are part of the implementation deliverables; if they don't exist today, list them in §12.2.

13. **`ProtoCmd` borrowing vs owning (issue 3.13).** Use `ProtoCmd<'a>` with borrowed slices to avoid per-command allocation.

14. **`animator.rs` integration (issue 3.14).** Confirm the existing fade animation paths continue to work, and document the new `WindowRecord.animation` field.

15. **Mouse cursor and FB lock (issue 3.9).** Document the invariant: `cursor.cx/cy` updates *must* hold the FB Mutex; the compositor's restore_under_cursor → composite → draw_cursor sequence runs atomically.

16. **Protocol version forward compatibility (issue 3.5).** Use `u8` not `enum V1 | V2`; specify unknown-version fallback.

17. **Resize without blank frame (BLOCKING-adjacent, raised in §15 question 8).** What is the visible behavior during `resize_window`? Old surface kept until first new-size end_frame, or accept the blank frame as v1 limitation?

---

## Closing assessment

The design is structurally correct and matches macOS's WindowServer abstraction well at the conceptual level: per-window surfaces, damage tracking, Z-order compositing, vsync-paced render thread, capability-gated zero-copy fast path. The decomposition into 10 files under 500 lines apiece is clean. The integration with R5 QoS, R7 thermal, R9 HAL, and R13 fonts is well-thought-out.

But the implementation specifics will not survive contact with the rest of the supervisor:

1. The single Mutex around WindowRegistry held across composite passes will livelock under steady draw load on `desktop-full` and miss latency budgets on `mobile`.
2. Co-locating compositor and IPC broker in PID 2 without lock partitioning will starve watchdog ticks.
3. Stretching the stdout-line protocol to 14 stateful verbs reintroduces the injection vector and the partial-write problem that v1 avoided by accident.
4. The SharedBuffer mechanism as specified does not match how `wasmtime::SharedMemory` or `wasm32-wasip2` actually work; the optimization is non-implementable today.
5. DRM page-flip vsync silently falls back to a jittery timer-based path under QEMU virtio-gpu, which is the only currently-supported display path.
6. Chrome painted directly into the framebuffer bypasses the damage tracking machinery and produces visually wrong frames on focus change.

The fixes are not "tune a constant"; they require:

- Restructuring the registry so the compositor can release the lock during the blit (issue 2.1).
- Splitting IPC routing from display parsing (issue 2.2).
- Choosing a real framed-channel or fd-partitioned transport for v2 (issue 2.3).
- Picking a workable SharedBuffer mechanism (custom memory creator vs hostcall vs fork) and rewriting §5 (issue 2.4).
- Adopting timerfd with accumulating deadline as the canonical vsync, with DRM page-flip as an optional optimization (issue 2.5).
- Putting chrome inside the Surface or as a sibling Surface so damage tracking covers it (issue 2.6).

If the synthesis commits to all six structural fixes plus the non-blocking concerns 3.1, 3.2, 3.5, 3.7, 3.10, 3.13, 3.14, the design becomes shippable as v1. Without them, the design is a paper architecture that the existing supervisor's lock graph, toolchain reality, and QEMU's vsync emulation will defeat the first day of integration.

The right outcome is REJECT AND REDESIGN with the six blocking issues resolved before synthesis; the structural skeleton (WindowRegistry, IidKey, compositor thread, FontProvider trait, capability gating) carries forward; the failing details (Mutex scope, SharedMemory aliasing, DRM vblank assumption, chrome path, stdout transport) get replaced with mechanisms that match what actually compiles and runs in VyomaOS today.
