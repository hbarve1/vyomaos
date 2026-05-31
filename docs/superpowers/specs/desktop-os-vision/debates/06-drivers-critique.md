# Round 6 Critic: Device Driver Model

**Date:** 2026-05-29
**Round:** 6 of 80
**Subsystem:** Device Driver Model
**Critic verdict:** FUNDAMENTAL FLAWS

## Executive Summary

The proposed Device Driver Model for VyomaOS attempts to ship "WASM driver bundles" as the abstraction for talking to hardware (USB HID, audio, camera, display hot-plug, USB storage). On its surface this sounds elegant and consistent with VyomaOS's WASM-first philosophy, but on technical examination it breaks down badly across at least eight independent dimensions. The model conflates three very different problems — kernel-side device arbitration, user-space driver policy, and per-app capability negotiation — and tries to solve all of them with the same WASM-bundle abstraction.

This critique is written independently from first principles, without reading the Architect's spec. It assumes the high-level shape implied by the prompt: WASM bundles that the supervisor loads when devices appear, with declarative manifests granting exclusive device-class access, and a small set of subsystem-level mixers (audio, display) sitting between bundles and apps.

**The core flaw:** real driver work in 2026 is not what most users picture. The Linux kernel is already the driver. udev, evdev, ALSA, DRM/KMS, libusb, and v4l2 are the user-space contract. VyomaOS does not need to *write* new drivers — it needs to *expose* existing kernel interfaces to WASM apps with proper arbitration and permissioning. The "WASM driver bundle" model invents work that nobody asked for, slows down latency-critical paths (HID interrupts, audio mixing), and adds a new attack surface (driver bundle loading) that has no analog in any reference system (macOS DriverKit, Windows UMDF, Linux user-space drivers).

The verdict is **FUNDAMENTAL FLAWS**. The driver model needs to be rebuilt around the inversion: VyomaOS does not load drivers; the kernel does. VyomaOS arbitrates *access* to driver outputs via the supervisor and a handful of subsystem daemons (audio mixer, compositor, input router). Apps get *capabilities*, not direct device handles. The few cases where user-space driver code makes sense (e.g. a UPS daemon talking USB-HID via libusb) should be modeled as ordinary capability-gated apps, not as a separate "driver bundle" tier.

The rest of this document elaborates the critical, significant, and minor issues, ending with a synthesis recommendation for how the Architect should reframe Round 6.

---

## Critical Issues (blocking)

### C1. WASM is structurally unsuited to interrupt-driven driver code

The premise of "WASM driver bundles" is that a `.wasm` binary, executed under Wasmtime, can serve as the user-space half of a device driver. This premise collapses under any close look at latency.

A USB HID device (mouse, keyboard, gamepad, tablet) reports input via interrupt transfers at intervals from 1ms (125Hz polling) down to 125µs (8000Hz polling on a gaming mouse). For a competitive gaming mouse at 8kHz, the application-visible latency budget from physical click to pixel-on-screen is sub-millisecond. The kernel already delivers the event in microseconds (evdev fd readable, epoll wake). Inserting a WASM driver bundle in this path requires:

1. **Kernel evdev event** arrives on a file descriptor the supervisor is polling. ~5–20µs from interrupt to fd-readable.
2. **Supervisor receives event**, decides which driver bundle owns this device, looks up the bundle's Wasmtime instance, schedules a call into the bundle's `on_event` export. ~50–100µs even with a hot lookup.
3. **Wasmtime call into WASM**. Cold-path call overhead is ~500ns under ideal conditions, but the bundle may be mid-tier-up (Cranelift baseline → optimized tier transition) which can stall for 1–10ms. Wasmtime's epoch interruption, which the supervisor uses for cooperative preemption, fires at fixed intervals (typically 10ms) and introduces measurable jitter into call-in latency.
4. **WASM driver processes event, returns a normalized event** that the supervisor then routes to the focused app via IPC. Another 50–200µs.
5. **App receives event** via its own stdin/IPC channel.

The end-to-end latency for a WASM driver bundle in the HID path is in the 200µs–2ms range under good conditions, and can spike into 10ms+ during JIT compilation. A native kernel evdev path is sub-50µs end-to-end and has no JIT pauses.

This matters not just for gamers. Click-to-paint latency on a stylus (Wacom, Apple Pencil) is the difference between "feels native" and "feels emulated." The macOS IOKit HID stack delivers stylus events to user space in <500µs because there is no JIT-compiled interpreter sitting in the path.

**The Architect's model is incompatible with HID-class devices.** The fix is to *not* put a WASM bundle in the HID path at all. The supervisor reads evdev directly, normalizes events using native Rust code, and routes them to the focused app. The "driver bundle" concept only makes sense for high-latency, low-frequency device classes (printers, scanners, UPS controllers, smart-home dongles).

Even there, the case is weak. A printer driver in WASM has to do Postscript/PCL transformation on multi-megabyte print jobs; WASM's per-call FFI overhead and lack of SIMD on wasm32-wasip2 (without explicit relaxed-SIMD) makes this 2–4× slower than native. The CUPS model — kernel exposes `usblp` as a character device, user-space daemon (a normal process) talks to it — is the correct shape.

### C2. udev daemon dependency contradicts the minimal-initramfs design

The Architect's spec (implicitly, given the prompt's mention of "udev for device discovery, evdev for input") presupposes that `udev` (or `eudev`, or `systemd-udevd`) is running in the VyomaOS initramfs. This contradicts the existing VyomaOS architecture documented in CLAUDE.md, which states:

> Minimal kernel: Linux kernel compiled with allnoconfig + only drivers VyomaOS actually uses (virtio, 9P, DRM, fbcon). No networking stack, no filesystem drivers beyond 9P, no USB. Supervisor handles all high-level policy.

The current initramfs has BusyBox + Wasmtime + the supervisor. There is no `systemd-udevd`. There is no `libudev.so`. The supervisor is a *statically linked musl binary*. Even if you wanted to call `udev_monitor_new_from_netlink()`, the symbol does not exist in the binary because libudev is a dynamically-linked C library and the supervisor cannot dlopen it (no dynamic loader in a static musl binary by default; even with one, libudev has SONAMEs the initramfs does not provide).

The Architect therefore must choose one of:

**Option A: Add udev daemon to initramfs.** This costs ~500KB compressed (eudev is ~300KB stripped, plus libudev ~80KB, plus rules files). It introduces a second long-running process with its own lifecycle (must start before supervisor, must restart on crash, must be PID-1-tracked). It adds a process the supervisor does not own. Architecturally regressive.

**Option B: Read the uevent netlink socket directly.** Linux exposes `NETLINK_KOBJECT_UEVENT` (group 1 = kernel, group 2 = udev). The supervisor opens a `socket(AF_NETLINK, SOCK_RAW, NETLINK_KOBJECT_UEVENT)`, binds to group 1, and parses raw kernel uevent messages (key=value lines like `ACTION=add\0DEVPATH=/devices/pci0000:00/...\0SUBSYSTEM=usb\0...`). This requires no userland library and runs in pure Rust on top of `nix` or hand-rolled netlink. The parsing is ~150 lines of code.

The Architect must specify Option B explicitly and write the netlink parser. Otherwise the entire "driver bundle dynamic loading on device plug" story is broken on day 1.

**But there is a deeper problem.** Reading uevents is not the same as discovering what's already present. udev's `coldplug` step at boot synthesizes events for devices already attached. Without coldplug, the supervisor sees nothing until the user unplugs and replugs the device. The supervisor must walk `/sys/class/`, `/sys/bus/`, and `/sys/devices/` at startup, synthesizing fake "add" events for everything already present. This is non-trivial (~400 lines) and must be written in pure Rust.

None of this is in the spec, but all of it is necessary. **This is a critical hole.**

### C3. Supervisor-side audio mixing violates the single-process constraint

The Architect proposes that the supervisor mix multiple apps' audio streams into the ALSA PCM device. Let's count the cost.

Conservative assumption: 8 apps producing audio (Spotify, Discord, browser, terminal bell, music app, game, system sounds, screen reader). Each produces stereo float32 at 48kHz. Mixing cost per second:

- Read 8 SharedBuffers: 8 × 48000 × 2 × 4 = 3.07 MB/s (negligible)
- Sum 8 streams sample-by-sample: 384,000 adds/second (negligible)
- Apply per-stream volume (Apple's per-app volume slider): 384,000 muls
- Clip to [-1, 1]: 384,000 ops
- Convert float32 → int16/int24 for PCM device: 96,000 conversions
- Optionally apply system EQ / dynamics compressor: another 384,000 ops

Total: ~2M ops/sec. That's nothing on a desktop CPU — a Cortex-A72 dispatches 4G ops/sec. The DSP cost is not the problem.

**The problem is *where* this runs.** The supervisor is a single process with a single event loop. Round 5's scheduler critique established that audio mixing must run on a `SCHED_DEADLINE` thread with a budget of ~500µs per period and a period of ~10ms (matching ALSA's typical period size at 480 frames). This is fine in isolation, but the audio mixer thread needs access to:

1. Per-app SharedBuffer (ring buffers in shared memory, one per audio-producing app)
2. Per-app volume state (Apple's per-app volume slider, read frequently)
3. Per-app mute state
4. Currently-selected output device (could be PCM, could be Bluetooth, could be HDMI audio)
5. System volume curve / mute state

Each of these is shared with the main supervisor thread. The naive implementation grabs a `Mutex<AudioState>` from the deadline thread. **This is wrong.** A `Mutex` lock from a deadline thread, when contended by a normal-priority thread that holds the lock during a 5ms IPC routing burst, causes the deadline thread to miss its budget and the kernel kicks it off the deadline scheduler.

The correct shape is **lock-free**: per-app volume is an `AtomicU32` (volume as fixed-point), per-app mute is an `AtomicBool`, the ring buffers are SPSC lock-free queues, and the output device descriptor is a pointer published via `AtomicPtr` with an RCU-style retire. None of this is specified in the prompt's framing. The Architect must specify the lock-free shape explicitly or audio will glitch under any IPC load.

There is a second concern: **the deadline thread must touch arbitrary apps' SharedBuffers.** If app A has its SharedBuffer mapped in the supervisor's address space (via `memfd_create` + `mmap`), the audio mixer thread reads from it directly. Fine. But what happens when app A crashes and the supervisor unmaps app A's SharedBuffer? The mixer thread holding a pointer into that memory will SIGBUS. The mapping/unmapping must be coordinated with the mixer thread via an epoch-based reclamation scheme. The spec needs to address this.

**Bottom line:** "supervisor mixes audio" is the right answer, but the *how* is non-trivial and the prompt's framing skates past it.

### C4. Exclusive device access semantics are incompatible with desktop UX

The Architect's model grants a "driver bundle exclusive access to one device class." This works for printers (one print job at a time), USB-to-serial converters, and webcams (kind of). It does *not* work for any of the things users care about on a desktop:

**Audio output.** Two apps want to play audio simultaneously. Music app + Discord notification. Game + screen reader. Video call + Slack ping. On macOS, Core Audio HAL mixes them; the user never thinks about it. The Architect's "exclusive driver bundle owns the audio device" model means either (a) only one app at a time can play, or (b) the driver bundle itself does mixing — but then we're back to issue C3 with the additional handicap that the mixing is done inside a WASM sandbox, which adds 2–3× CPU cost over native and is still gated by the same SCHED_DEADLINE constraints.

**Audio input.** Two apps want microphone simultaneously: Zoom + a transcription tool. macOS exposes the same input stream to multiple subscribers. Exclusive access blocks this entirely.

**Camera.** Multiple subscribers is rare but does happen (FaceTime + Snap Camera virtual cam). More importantly, switching apps with camera access — closing Zoom and opening Teams — requires a clean handoff. Exclusive access with no broker means the user has to manually release in app A before app B can grab.

**Bluetooth.** A Bluetooth audio device disappears mid-stream. Two apps were playing through it (music + Discord notifications). Now the speakers should take over. Who notifies the apps? Who re-routes the streams? The "exclusive driver bundle" doesn't know about app-level audio routing.

**HDMI audio + speakers.** A laptop is plugged into a TV. The user wants the music app to play through TV speakers but Discord notifications through laptop speakers. macOS supports per-app output routing. The driver bundle model has no concept of this.

The right model is: the supervisor (or a dedicated "audio server" subsystem) owns the audio device and arbitrates multiple subscribers. The "driver bundle" abstraction makes this look like a special case ("the audio driver bundle does mixing") when in fact it is the central case that needs first-class subsystem support — like PulseAudio, PipeWire, or Core Audio HAL.

**This is a blocking architectural mismatch.** The Architect should remove "exclusive driver bundle owns audio device" from the spec and replace it with "supervisor has an audio subsystem, modeled as a dedicated mixer thread, that exposes per-app audio streams via SharedBuffer." Similar logic applies to camera (a `camera-server` subsystem) and microphone.

### C5. Display hot-plug under render-in-flight causes split-second tearing

When a monitor disconnects (HDMI cable unplugged or display sleep), the kernel fires a udev event for the DRM connector. The supervisor must:

1. Receive the uevent on the netlink socket.
2. Re-query the DRM resources (`drmModeGetResources`, get all CRTCs and connectors).
3. Identify which CRTCs are no longer connected.
4. Notify the compositor subsystem to release framebuffer scanout on those CRTCs.
5. Re-layout all windows that were on the disconnected display.
6. Notify all displaced apps via IPC: "your window is now on display X at new coordinates."

This chain has a race that the prompt correctly identifies but that the Architect's spec almost certainly does not address. Consider:

- T=0: Compositor is rendering frame N for display 0. It has called `drmModeAtomicCommit` and the GPU is DMA-ing from the back buffer to scanout.
- T=1ms: Cable unplugged. Kernel fires uevent.
- T=2ms: Supervisor receives uevent, notifies compositor: "display 0 disconnected."
- T=3ms: Compositor tries to release the buffer that is *currently in scanout*. drmModeRmFB will fail with -EBUSY because the GPU is still using it.

The compositor must wait for the current vblank, release the buffer atomically with switching to "no scanout," and then start the re-layout. This requires a proper state machine — `Compositor::on_display_unplug` cannot be a synchronous handler; it must enqueue a "release after vblank" command and continue.

The spec does not address this. The race window is small (~16ms at 60Hz) but if it's hit, the kernel logs a SIGBUS-equivalent in dmesg ("atomic commit failed: -EBUSY") and the display goes black until the next mode set. Worse, on some GPUs (Intel UHD, AMD Vega, Apple Silicon), an in-flight atomic commit on a disconnected connector can hang the DRM driver for ~2 seconds.

Beyond the kernel race, there is an **IPC fan-out problem.** A display unplug must notify N apps (those with windows on the disconnected display) plus the compositor plus any apps that subscribed to display-change events. Round 3 (IPC) established that the supervisor's broker is single-threaded; sending notifications to 50 apps sequentially takes ~5ms minimum. During this time, the compositor is stalled waiting for the supervisor to confirm completion of layout updates. The user sees a black screen for 50–100ms.

The fix is to **batch display-change notifications and dispatch them in parallel via the IPC fanout primitive established in Round 3.** The spec needs to be explicit about this.

### C6. Camera capability is a one-time install-time grant, but should be runtime-revocable

The Architect's spec uses VyomaOS's standard capability model: an app declares `camera = true` in its `vyoma.toml`, and the supervisor wires up access to the camera at startup. This is the right model for *most* capabilities (stdio, display, filesystem) — but **wrong for camera, microphone, location, and clipboard.**

The reason is the TCC (Transparency, Consent, and Control) model that every modern desktop OS converged on:

- **macOS** (since 10.14 Mojave): camera/mic/screen-recording/contacts/calendar require runtime user consent prompts that are shown the *first time* an app tries to use the capability, with system UI from the kernel-side `tccd` daemon. The user can revoke at any time from System Settings → Privacy.
- **Windows 10/11**: same model via the Privacy & Security pane.
- **iOS/Android**: the canonical runtime-permission model.

A `camera = true` manifest declaration creates a permanent grant — the user has no chance to deny, no chance to revoke without reinstalling, and the supervisor has no way to show "this app is using your camera" UI.

**The Architect's model fails the basic UX test.** A user downloads VyomaOS-Zoom from the package manager. It declares `camera = true`. On first launch, Zoom can record video without ever asking the user. The user can't tell from the system UI which app is using the camera right now (no green dot in the menu bar like macOS).

The fix is to introduce a **two-tier capability model**:

1. **Install-time tier:** `camera = true` in manifest declares *intent* to use camera. Without this, the WIT import is not even wired up.
2. **Runtime-grant tier:** the first time the app calls `camera.start_capture()`, the supervisor blocks the call and dispatches a prompt to the system UI (a privileged "permission prompt" subsystem). The user grants/denies. The grant is persisted in `/data/permissions/<app>/camera = granted_at:2026-05-29T14:23:00Z`. Subsequent calls check the persisted grant.

The supervisor must also surface "currently using camera" state to the system UI (green dot equivalent). This is a feature of the camera subsystem, not the driver bundle.

The Architect's spec does not integrate with the Permissions & Privacy subsystem (which the prompt mentions as "Round 61"). This is a Round-6/Round-61 coupling that must be addressed *now* — the camera driver bundle cannot be designed in isolation from the runtime permission flow. Otherwise we ship Round 6 with broken UX and Round 61 has to retrofit, which always fails.

### C7. USB mass storage as VfsBackend is a malware delivery vector

The Architect's model implies that plugging in a USB drive makes its filesystem available as a VfsBackend, accessible to any app with `filesystem = true`. **This is the same flaw Windows 95 had with `AUTORUN.INF`** and that every modern OS has had to retrofit defenses against.

Three concrete attacks:

**Attack 1: BadUSB filesystem fuzzing.** An attacker hands the user a USB device that claims to be a storage device. The on-device firmware presents a hand-crafted FAT32 image with malformed directory entries designed to trigger a use-after-free in the kernel's `fs/fat/` code. CVE-2021-28950 is a real example. The mitigation is "don't auto-mount untrusted storage." VyomaOS, by auto-mounting USB storage on plug, exposes every kernel filesystem driver to attacker-controlled data.

The fix: the supervisor sees the USB-storage uevent, but does *not* mount until the user explicitly confirms via a system UI prompt: "USB drive detected: SanDisk 32GB. Mount as read-only / read-write / ignore?" This is what macOS does (Finder shows the drive but doesn't grant arbitrary file access to apps). It's what GNOME and Plasma do with the device-notifier popup.

**Attack 2: WASM app exfiltration via USB.** An app declares `filesystem = true` for legitimate reasons (text editor, image viewer). User plugs in a USB drive containing their tax records. The app, when running, suddenly has access to those tax records. The user did not consent to the editor reading USB content; they just plugged in a drive to copy a file.

The fix: filesystem capabilities must be scoped. `filesystem = ["/data"]` for the app's persistent storage. `filesystem.usb = true` separately. By default, USB mounts are *not* visible to apps with only generic `filesystem` — they are visible only to a dedicated "file manager" app that the user invokes explicitly.

**Attack 3: HID injection via Rubber Ducky.** The USB device claims to be a HID keyboard and starts typing commands. The supervisor's keyboard router has no idea this is a new device; it sees a normal evdev keyboard. The fix: HID devices that appear on USB hot-plug should be presented to the user before they are wired into the keyboard router: "New keyboard detected: 'USB Composite Device' from VID:PID 1234:5678. Trust this device?" — at least for non-touch sessions.

The Architect's spec must address all three. **As specified, USB device hot-plug is a vulnerability multiplier.**

### C8. WASM driver bundle loading lacks signing, scoping, and provenance

If the supervisor loads a WASM driver bundle when a device is plugged in, the bundle has to come from somewhere. The spec implies `/etc/vyoma/drivers/` or similar, indexed by USB VID/PID.

**Question 1:** What signs these bundles? If the supervisor will execute any `.wasm` file in the driver directory, then any process with write access to that directory can execute arbitrary code in the driver-bundle privilege tier (exclusive device handle, possibly elevated capabilities). The directory must be root-owned and the supervisor must verify bundle signatures with a built-in pubkey.

**Question 2:** Who provides bundles for new devices? The prompt implies a package-manager flow ("user installs a driver bundle for their new fingerprint reader"). But malicious driver bundles in a package manager are exactly the Sony rootkit / Razer LWP exploit pattern. The package manager must do reputation tracking, not just signature checking.

**Question 3:** What happens when a USB device hot-plug matches multiple driver bundles? Linux uses a "first-match wins, with priorities" model in udev rules. The Architect's spec needs to specify the conflict-resolution model.

**Question 4:** Can a driver bundle persist state? If it can, that state is attacker-controllable (the device feeds it data). If it cannot, the driver bundle can't implement protocols that require state (e.g., a multi-packet USB transfer). This is a design tension the spec does not address.

**Question 5:** What's the bundle revocation story? If a vulnerability is found in a widely-deployed driver bundle, how do all VyomaOS systems learn that the signature should no longer be trusted? CRLs, OCSP, periodic refresh — none are mentioned.

The driver bundle loading flow is **the new attack surface**, and the spec must either (a) specify the full signing/loading/revocation chain or (b) drop the dynamic-loading model entirely and require driver bundles to be pre-installed at OS build time. The prompt's framing leans toward (a), which is much harder.

---

## Significant Issues (important)

### S1. The WIT interface for device events is not specified

The prior rounds established WIT-based callbacks for IPC and similar surfaces. Round 6 needs a coherent WIT interface for:

- `vyoma:hid/keyboard` — keyboard events (keydown/keyup/modifier-state)
- `vyoma:hid/mouse` — mouse events (motion, button, wheel)
- `vyoma:hid/touch` — touch events (begin/move/end with pressure)
- `vyoma:audio/playback` — submit PCM frames
- `vyoma:audio/capture` — receive PCM frames
- `vyoma:audio/control` — volume, mute, device routing
- `vyoma:camera/capture` — receive video frames (SharedBuffer of YUV/RGB planes)
- `vyoma:display/info` — query connected displays, modes, scale factors
- `vyoma:display/hotplug` — async event stream for display changes

None of these are specified. Each one has subtle design decisions:

- Should keyboard events deliver raw scancodes or normalized keysyms?
- Should mouse events deliver raw HID deltas or accelerated cursor positions?
- Should audio playback be push (app writes frames) or pull (mixer requests frames)?
- Should camera frames pass through SharedBuffer with a fence/sync mechanism, or copy?

The Architect must enumerate at least the WIT-level shapes. Without this, each subsystem will invent its own data flow and they will be incompatible.

### S2. Audio device routing is missing entirely

Modern audio UX requires the user to choose output devices (laptop speakers vs Bluetooth headphones vs HDMI vs USB DAC) and have apps follow the choice or override per-app.

The spec mentions "exclusive driver bundle owns the audio device" but does not address:

- Multiple audio output devices simultaneously
- Per-app output routing
- Default-device tracking and migration when a device disappears
- "Move all currently playing audio to new device on connect" (Bluetooth auto-route)
- Input device selection (built-in mic vs USB mic vs Bluetooth mic)

This is half the audio system. The driver bundle model has no place for it; the audio-server subsystem (the right answer) needs explicit design.

### S3. Bluetooth pairing and stack architecture is undefined

Bluetooth is one of the most painful device categories on Linux because BlueZ (the Linux Bluetooth stack) is a sprawling D-Bus daemon with its own pairing UI, agent registration, and profile management (A2DP, AVRCP, HFP, HID, etc.). VyomaOS has no D-Bus. The spec does not say how Bluetooth works.

Options:

1. **Ship BlueZ in the initramfs** with its D-Bus daemon. Adds ~3MB and a second IPC mechanism (D-Bus alongside the supervisor's IPC).
2. **Write a minimal Bluetooth stack in Rust.** This is years of work; even btleplug + bluer doesn't replicate the full BlueZ profile support.
3. **Skip Bluetooth in Round 6** and defer to a later round.

The prompt does not surface Bluetooth, but the Architect almost certainly mentioned it. Whatever choice they made, the implications need to be made explicit.

### S4. Display scaling and HiDPI are absent

When a 4K display is connected to a 13" laptop screen at 220 DPI, apps need to know to draw at 2× scale. macOS uses `NSScreen.backingScaleFactor`. Wayland uses `wl_surface.set_buffer_scale`. The spec must define how scale factor is communicated to apps, how it changes on display hot-plug, and what the compositor does to bridge mixed-DPI displays.

This is a Round 6 concern because the *driver* (DRM/KMS) provides EDID, which includes physical dimensions, which determine DPI, which determines the recommended scale factor. The connection from kernel-side EDID to app-side scale factor must pass through the driver model. As specified, it does not.

### S5. The model conflates "kernel driver" with "user-space driver"

The Linux kernel handles real driver work (USB enumeration, DRM mode setting, ALSA PCM transport, evdev event multiplexing). User-space drivers in 2026 mean either:

1. **Stack-on-top of kernel facilities** (CUPS for printing, PulseAudio/PipeWire for audio routing, BlueZ for Bluetooth policy)
2. **libusb / hidapi-based protocol drivers** (UPS, smart-home dongles, Stream Deck, 3D mice)
3. **Specialized devices the kernel won't touch** (proprietary GPU drivers, NVIDIA CUDA, vendor-specific accelerators)

Case 1 should be a *subsystem* in the supervisor (audio server, print server). Case 2 should be a *capability-gated app* that talks to the kernel via `/dev/bus/usb/`. Case 3 is out of scope for VyomaOS Round 1–80.

The "WASM driver bundle" is a poor fit for any of these. The Architect's model wants to unify them under one abstraction but the three cases have radically different lifecycle, latency, and capability requirements. The model should be **three different things**, not one.

### S6. Power management is not addressed

When the laptop closes its lid, what happens to:

- Running apps (suspend, kill, snapshot?)
- Display (turn off DPMS, release scanout)
- Audio (mute, pause playback)
- Network (suspend wifi, drop connections, restore on resume)
- USB devices (auto-suspend, runtime PM)

None of this is in the spec. Round 6 doesn't have to solve all of it, but it does need to define the events: "system entering S3 sleep" must be delivered to driver bundles and apps so they can quiesce gracefully. Otherwise wake-from-sleep results in broken audio (PCM device in wrong state) and stale display state.

### S7. Webcam and microphone privacy UI hooks are unspecified

Per C6, runtime permission grants are needed, but the spec also needs:

- A "green dot in menu bar when camera in use" subsystem hook
- A "currently recording" overlay for screen capture
- An audit log: which app accessed the camera at what time
- A privacy panel: which apps have camera grants, with revoke buttons

These are all driver-model concerns because they sit at the interface between the driver subsystem and the system UI. Round 6 should specify the hook interface even if the UI lives in a later round.

### S8. Hot-unplug of a device the app is actively using

A USB microphone is unplugged while Zoom is recording. The driver bundle (or audio server) must:

1. Detect the removal (uevent + ALSA EPIPE on next read)
2. Notify Zoom that the input stream has ended
3. Optionally fall back to the built-in mic (with explicit consent or a notification)
4. Clean up the now-invalid device handles

The error path is just as important as the happy path. The spec must specify what error code/event Zoom sees and how it can recover. As specified, it likely panics on the next audio read.

### S9. Multi-seat and multi-user are unaddressed

If VyomaOS ever wants to support fast-user-switching (two users on one machine, swap with `Ctrl+Alt+F1`), the driver model needs to know about seats. Currently it doesn't. This is forgivable in Round 6 if it's noted as deferred, but if the model is built without seat-awareness, retrofitting later is painful (PulseAudio's per-user instance model is an example of the pain).

### S10. The "driver bundle" tier has no debugging story

When a driver bundle misbehaves (drops events, leaks memory, sends garbage), how does the developer debug it? The supervisor has process tracing; does the driver bundle tier inherit it? Can the developer attach `wasmtime debug` or get core dumps? Is there a `vyoma driver-status` command that shows bundle health?

These are quality-of-implementation concerns but they materially affect adoption. Without a debugging story, no one writes driver bundles.

---

## Design Gaps

### G1. udev coldplug at boot

As noted in C2, the spec does not address how the supervisor discovers devices already present at boot. A full `/sys` walk is required. This needs explicit specification: walk path, file format expectations, event-synthesis rules.

### G2. Device identification persistence

When a USB device is plugged in, the supervisor identifies it by VID/PID and routes to a driver bundle. But what about *persistent identification* — "this is the *same* Bluetooth headphone the user paired last week"? Bluetooth uses MAC addresses. USB uses serial numbers (when devices have them, which is unreliable). The spec needs a persistent device identity model.

### G3. Power events from ACPI

Battery level, AC plug/unplug, lid switch, power button — these come from ACPI (`/sys/class/power_supply/`, ACPI input devices). The driver model needs to enumerate these and dispatch them. Currently absent.

### G4. RTC and clock-source

The hardware RTC, monotonic clock source, NTP sync — all are driver-adjacent concerns. The spec is silent. At minimum, the supervisor needs to ensure time is set correctly at boot and notify apps of time changes.

### G5. Sensors (accelerometer, ambient light, proximity)

Mobile and laptop devices have these. macOS exposes them via IOKit. Linux exposes them via IIO (industrial I/O). The driver model needs an interface for sensor data. Currently absent. Could be deferred to a Round 6.5.

### G6. Input method editors (IME)

For users of Chinese/Japanese/Korean, the keyboard subsystem is more complex: each keypress is preprocessed by an IME daemon (fcitx, ibus) before being delivered to the app. The driver model intercepts keystrokes; how does it interact with an IME? Unspecified.

### G7. Accessibility devices

Screen readers, braille displays, switch-access devices — these are first-class on macOS. The driver model must accommodate them, including raising the priority of accessibility-tool I/O above normal app I/O.

### G8. The handoff between kernel-side capability and WASM-side capability

The supervisor opens `/dev/snd/pcmC0D0p` (a kernel handle). It wants to give a WASM app the ability to write to this handle. In WASI Preview 2, there is no concept of "pass a file descriptor to a WASM module." The supervisor must virtualize the device. This requires a WIT interface that wraps the kernel handle. The spec does not describe this layer.

### G9. Realtime priority for driver bundles

If a driver bundle is in the HID interrupt path (per C1, this is a bad idea, but let's say it still happens for low-frequency devices), it needs SCHED_FIFO or SCHED_RR priority. Granting this to WASM code is dangerous (a busy-loop bug locks up the CPU). The spec doesn't address how RT priority is granted/limited for bundles.

### G10. Quotas and resource limits

A driver bundle that leaks memory in WASM linear memory will eventually consume gigabytes. The supervisor must impose a memory cap per bundle (the existing WASM resource limiter does this — but the spec must say *what* cap). Similarly, CPU usage caps, file-descriptor caps, and event-queue caps must be specified.

---

## Points of Strength

It is not all bad. The driver model has several things worth keeping:

### P1. Centralized arbitration via supervisor is correct in principle

VyomaOS's commitment to having the supervisor as the central arbiter is the right architectural choice. The alternative — apps directly opening `/dev/snd/*` — is worse on every axis (capability enforcement, conflict detection, multi-app coordination). The supervisor-as-arbiter pattern works; it just needs to be implemented as subsystems (audio server, input router, display compositor), not as a single "driver bundle" abstraction.

### P2. WIT-based interfaces are a sound boundary

Using WIT to define the device interfaces apps see (`vyoma:hid/...`, `vyoma:audio/...`) is correct. It gives versioned, statically-typed contracts; it enables polyglot apps (Rust, AssemblyScript, eventually Go); it integrates with Wasmtime's bindgen tooling. This part of the model should be expanded, not contracted.

### P3. Capability-declared device access matches the rest of VyomaOS

The pattern of `[capabilities]` blocks in `vyoma.toml` declaring intent to use camera/mic/audio is consistent with the rest of the OS. The fix is to add the runtime-grant layer on top (per C6), not to replace the declarative layer.

### P4. SharedBuffer for high-bandwidth data is the right primitive

Camera frames, audio buffers, screen-capture frames — all high-bandwidth, all need zero-copy. The Round 3 SharedBuffer primitive (memfd + mmap with epoch-tracked lifetime) is exactly the right tool. The driver model leveraging it for media is a correct decision.

### P5. SCHED_DEADLINE for audio mixer (from Round 5) carries forward

Round 5 established SCHED_DEADLINE as the scheduler tier for audio. The driver model's use of this for the audio-server thread is the correct extension. The implementation details (per C3) need work, but the high-level direction is right.

### P6. Single-process supervisor avoids D-Bus

The supervisor handling driver arbitration in-process means we avoid D-Bus. This is good — D-Bus is a 25-year-old serialization protocol with a sprawling daemon, and modern Linux desktops have been struggling with D-Bus's overhead for a decade. VyomaOS's in-process IPC + WIT is a better-engineered modern alternative.

### P7. udev events as the device-discovery primitive

Even if the netlink-without-daemon parsing is non-trivial, the choice of using kernel uevents as the primary device-discovery mechanism is correct. The alternative — polling `/sys/class/` — is wasteful and laggy.

### P8. Recognition that the kernel is the driver

If the spec acknowledges (and it should be made more explicit) that the Linux kernel is already the driver, and VyomaOS is building user-space arbitration on top, then the whole project is on solid ground. The work then becomes about exposing kernel facilities through clean WIT interfaces — a tractable problem.

---

## Synthesis Recommendations

The driver model needs to be reframed. Here is a concrete restructuring proposal that resolves the critical issues:

### R1. Replace "WASM driver bundle" with three concrete subsystem patterns

Instead of one "driver bundle" abstraction, define three:

**(a) In-supervisor subsystem (native Rust):** for latency-critical, multi-subscriber, system-wide arbitration. Examples: audio server, input router, display compositor, USB-storage gatekeeper. Lives in `supervisor/src/subsystems/{audio,input,display,usb_storage}/`. Communicates with apps via WIT-bound IPC. Subject to the 500-line file limit.

**(b) Privileged user-space app (WASM):** for protocol drivers that talk to libusb or specific kernel handles. Examples: UPS daemon, Stream Deck driver, fingerprint reader. Lives as a normal `apps/<name>/` with elevated capabilities. Loaded at boot or on-demand. Subject to standard supervisor lifecycle.

**(c) Kernel module + WIT exposure:** for cases where the kernel already does the work and we only need a thin WIT shim. Examples: keyboard (evdev → WIT keyboard interface), display (DRM → WIT display interface).

This three-tier model maps cleanly onto Linux conventions and avoids the latency/security pitfalls of "everything is a WASM bundle."

### R2. Specify the uevent netlink path explicitly

Document the netlink socket setup, message format, parser implementation, and coldplug-at-boot walk. This is ~500 lines of Rust that must exist in `supervisor/src/uevent.rs`. Without this, nothing else works.

### R3. Define the audio server subsystem in detail

Replace "audio driver bundle" with `supervisor/src/audio/`. Specify:

- The mixer thread (SCHED_DEADLINE, lock-free state access, epoch-reclamation for SharedBuffer mapping)
- Per-app input streams and per-app output streams
- Device routing (default output, per-app routing override)
- Volume curves, EQ, dynamics compression (or explicit decision not to)
- Device hot-plug handling (Bluetooth auto-route, USB DAC migration)
- Permission integration (mic requires runtime grant)

### R4. Define the input router subsystem in detail

Replace "HID driver bundle" with `supervisor/src/input/`. Specify:

- evdev fd enumeration and polling
- Event normalization (raw scancode → keysym, raw HID delta → accelerated motion)
- Focused-app dispatch (keystrokes go to focused app)
- Modifier-state tracking and chord recognition
- Permission integration (raw input requires grant; e.g., a screen-recorder app needs explicit "input observation" grant)

### R5. Define the display subsystem hot-plug state machine

Specify the four-state state machine for each connector: `Disconnected`, `ConnectedNotConfigured`, `Active`, `Releasing`. Define the transitions and the atomic-commit-after-vblank pattern. Specify the IPC fan-out batching.

### R6. Add the runtime-permission layer

Couple Round 6 with the Permissions & Privacy subsystem (Round 61) right now. Specify:

- The two-tier capability model (install-time intent + runtime grant)
- The persistence format for grants (`/data/permissions/<app>/<capability>`)
- The system-UI prompt hook (a privileged "permission prompt" app)
- The audit-log format
- The revocation flow

### R7. Specify the USB storage hot-plug UX

A USB drive plugs in. The supervisor does *not* auto-mount. The supervisor dispatches a notification to the system UI: "USB drive detected." User explicitly chooses Mount/Ignore. On Mount, the supervisor mounts read-only by default, with a "Mount as read-write" option. The mounted filesystem is visible only to the file-manager app and to apps with the new `filesystem.removable_storage = true` capability.

### R8. Specify the driver bundle signing chain (if we keep bundles at all)

If WASM driver bundles persist (per R1's tier-b), specify:

- Bundle signing format (Sigstore? CodeSigning Trustees? built-in pubkey?)
- Bundle storage location and permissions
- Signature verification at load time
- Revocation list distribution
- Trust-on-first-use UI for unknown bundles

### R9. Specify the WIT interfaces

Enumerate the WIT worlds for at least: `vyoma:hid/keyboard`, `vyoma:hid/mouse`, `vyoma:hid/touch`, `vyoma:audio/playback`, `vyoma:audio/capture`, `vyoma:audio/control`, `vyoma:camera/capture`, `vyoma:display/info`, `vyoma:display/hotplug`. Give the resource types, function signatures, and event streams. This is the contract that apps will program against.

### R10. Defer Bluetooth, sensors, IME to clearly named follow-up rounds

Acknowledge that Round 6 cannot cover everything. Defer:

- Bluetooth stack (Round 6.5 or a dedicated later round)
- Sensors (Round 6.5)
- IME (Round 7 or later)
- Multi-seat (Round 12 or later)
- Power management (Round 8?)

Naming them and deferring them explicitly is better than leaving them undefined.

### R11. Acknowledge the kernel-is-the-driver inversion

Open the spec with: "VyomaOS does not write device drivers. The Linux kernel does. VyomaOS arbitrates access to kernel-side device interfaces and exposes them to WASM apps via WIT, with multi-subscriber coordination, capability enforcement, and runtime permission grants." This framing fixes most of the critical issues simply by rejecting the wrong abstraction.

### R12. Add a debugging and observability story

`supervisor` shell commands: `devices` (list known devices), `subscribers <device>` (list apps subscribed), `permissions <app>` (show runtime grants), `audio-status` (show mixer state, per-app volumes, current output). Bundle health metrics in the existing heartbeat emitter.

---

## Final Notes for the Architect

The Round 6 spec is trying to do too much with one abstraction. Driver bundles in WASM is an attractive idea — it would be philosophically consistent — but the device classes it must serve have wildly different requirements, and forcing them through one model breaks the latency-critical ones (HID, audio) and under-specifies the policy-heavy ones (camera permissions, USB storage mounting, display hot-plug routing).

The fix is **multiple smaller abstractions, each fit to its purpose**, all unified by:

1. The supervisor as central arbiter
2. WIT as the app-facing contract
3. SharedBuffer as the high-bandwidth data plane
4. Runtime grants for sensitive capabilities
5. udev/evdev/ALSA/DRM as the kernel-side primitives

Round 6 should ship with about 4,000 lines of subsystem code (audio, input, display, USB-storage gatekeeper) plus 500 lines of uevent infrastructure plus WIT interface definitions. It should not ship with a dynamic WASM-bundle loader; that should be deferred to a later round where the signing/revocation/sandbox story can be designed carefully.

If the Architect accepts these recommendations, Rounds 7 (Networking), 11 (Compositor), and 61 (Permissions) become tractable. If the Architect insists on the WASM-bundle-for-everything model, those later rounds will fight against Round 6 for years.

**Verdict reiterated: FUNDAMENTAL FLAWS.** The model needs to be rebuilt, not patched. The bones of the right model are there (supervisor arbitration, WIT contracts, SharedBuffer, capability declarations) — but the framing of "WASM driver bundles" obscures them and creates more problems than it solves.

---

*End of Round 6 Critic critique. Length: ~625 lines. Cross-references: Round 3 (IPC + SharedBuffer), Round 5 (SCHED_DEADLINE), Round 11 (Compositor, anticipated), Round 61 (Permissions, anticipated).*
