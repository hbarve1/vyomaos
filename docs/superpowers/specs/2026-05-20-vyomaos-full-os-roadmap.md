# VyomaOS — Full OS Roadmap (P31–P112)

**Date:** 2026-05-20
**Status:** active planning
**Current phase:** P23 complete (TUI widget primitives); P24–P26 next

---

## Goal

Advance VyomaOS from a working WASM prototype (P23) to a fully capable general-purpose OS
competitive with macOS, Linux, Android, and Windows — all apps as sandboxed WASM binaries,
all features delivered without breaking the capability-secure model.

---

## Completed milestones

| Phases | Milestone | Status |
|--------|-----------|--------|
| P01–P08 | Build foundation, kernel, supervisor, WASM runtime, IPC, seccomp, storage | ✅ |
| P09–P10 | DRM display, bitmap font, VYOMA_DRAW protocol | ✅ |
| P11–P12 | Networking (HTTP), interactive shell, keyboard routing | ✅ |
| P13–P17 | Process management, package manager, persistent logs, real-time TTY | ✅ |
| P18–P23 | Shell UX, watchdog, font scaling, window regions, mouse, TUI widgets | ✅ |

---

## Milestone A — System Apps (P24–P26, parallel)

Three independent apps that prove the windowed multi-app model.

### P24 — File Manager
Scrollable `/data` browser: ↑/↓ navigate, Enter open file, q quit.
Capabilities: `filesystem`, `display`, `shell`.

### P25 — Text Editor
Open `/data/<file>` from shell. Line buffer, cursor, insert/delete, Ctrl+S save.
Capabilities: `filesystem`, `display`, `shell`.

### P26 — System Monitor
Polls `@supervisor: ps-raw` every second. Renders per-app uptime/restarts/watchdog.
Replaces or supplements gui-demo dashboard.
Capabilities: `display`.

---

## Milestone B — Security Foundation (P27–P30, sequential)

### P27 — App Namespaces
Per-app Linux mount namespace (`CLONE_NEWNS`) + PID namespace (`CLONE_NEWPID`).
Each wasmtime child sees only its declared mounts.
Requires `CONFIG_NAMESPACES=y`, `CONFIG_PID_NS=y`, `CONFIG_MNT_NS=y`.

### P28 — Signed App Bundles
`wasm_sha256` field in vyoma.toml. Supervisor verifies binary at load; refuses on mismatch.
Enables safe install from untrusted sources.

### P29 — Multi-Resolution Display
Supervisor reads actual framebuffer resolution via `FBIOGET_VSCREENINFO` and broadcasts
`VYOMA_SYSTEM:screen:<w>,<h>` to all display apps before first draw.
Apps scale layouts instead of hardcoding 1440×900.

### P30 — OTA Hot-Swap
`@supervisor: update <app> <url>` — download WASM, verify SHA256, hot-replace binary, restart.
No reboot required.

---

## Milestone C — Window Manager (P31–P36)

### P31 — Double-Buffered Compositor
Replace single framebuffer writes with a back-buffer model: supervisor composites all
app window surfaces into one back-buffer per frame, then flip. Eliminates tearing.
Protocol: `VYOMA_DRAW:present` (app signals frame ready); supervisor composites.

### P32 — Window Decorations
Supervisor draws title bar, resize handle, close/min/max buttons around each app window.
Apps do not draw their own chrome. Window geometry in vyoma.toml becomes the *content* area.
Protocol: `VYOMA_SYSTEM:window_event:<type>:<data>` sent to app on resize/close.

### P33 — Z-Ordering
Window stack: `raise`, `lower`, `focus` reorders stacking.
`@supervisor: raise <app>` / `lower <app>` commands.
Click on a window automatically raises it.

### P34 — Window Manager App (WASM)
A WASM app that manages window positions, sizes, and Z-order by sending supervisor IPC
commands. Replaces hardcoded window regions with a dynamic layout engine.
Capabilities: `display`, `shell`, `mouse`.

### P35 — Desktop Wallpaper
`VYOMA_SYSTEM:wallpaper:<path>` IPC command to supervisor.
Supervisor renders a solid color or image fill as the bottommost layer.
Configurable in Settings (P40).

### P36 — Window Resize Events
Apps receive `VYOMA_SYSTEM:resize:<w>,<h>` when their window is resized.
Apps redraw at new dimensions. Enables fluid resizing.

---

## Milestone D — Desktop Shell (P37–P43)

### P37 — Taskbar App
Persistent WASM app docked at screen bottom. Shows running app icons, clock, system tray.
Click icon to raise/focus that app. Capabilities: `display`, `mouse`, `shell`.

### P38 — App Launcher
Keyboard shortcut (e.g., Meta key) or click opens a search overlay listing all installed apps.
Type to filter, Enter to launch. Capabilities: `display`, `shell`, `mouse`.

### P39 — Notifications Subsystem
`@supervisor: notify <app> <message>` IPC. Supervisor queues notifications.
Notification app renders toasts top-right; notification center stores history.

### P40 — Settings App
WASM app: display resolution, font size, wallpaper color, theme palette, boot apps list.
Writes config to `/data/settings.toml`; supervisor reloads on change.

### P41 — Power Manager
Supervisor handles ACPI `poweroff`/`reboot`/`sleep` requests from apps or keyboard shortcut.
`@supervisor: shutdown` / `@supervisor: reboot` IPC commands.

### P42 — Session Manager
Save/restore window positions and app state to `/data/session.toml` on shutdown.
Restore on boot: re-launch apps at their saved positions.

### P43 — Multi-Monitor
Supervisor enumerates DRM connectors. Each monitor gets its own framebuffer surface.
Apps can declare which monitor they appear on via `[window] monitor = <n>`.

---

## Milestone E — Rich Networking (P44–P50)

### P44 — DNS Resolver
WASM resolver app reads `/data/resolv.conf`. Apps with `network = true` route DNS through it.
Supervisor proxies DNS queries from app WASI sockets → resolver app → network.

### P45 — HTTPS / TLS
rustls (WASM-compatible) available as a library for WASM apps.
http-server and new apps can serve/request HTTPS. `network = true` + optional cert path.

### P46 — Basic Web Browser
WASM app: fetches HTML via HTTP/HTTPS, renders stripped text content.
No CSS/JS engine — text-only. Sufficient for docs, API responses, simple pages.

### P47 — SSH Client
WASM SSH client app using pure-Rust SSH library.
`@supervisor: ssh <host> <user>` launches the app with a shell window.

### P48 — Network Config UI
Settings sub-page: configure IP, gateway, DNS. Writes `/data/network.toml`.
Supervisor applies on reload.

### P49 — Download Manager
`@supervisor: download <url> <dest>` — background download to `/data`.
Progress shown in Notifications (P39).

### P50 — WebSocket Support
WASI socket extension for WebSocket handshake upgrade.
Enables real-time apps (chat, live dashboard from external services).

---

## Milestone F — Rich File System (P51–P57)

### P51 — VFS Abstraction
Supervisor exposes named mount points beyond `/data`: e.g., `/media`, `/tmp`, `/home`.
Apps declare which mounts they need in vyoma.toml: `mounts = ["/media"]`.

### P52 — File Associations
`/data/mime.toml`: map extension → app name. `@supervisor: open <path>` picks default app.
Shell `open <file>` command uses this. File manager Enter key uses this too.

### P53 — Clipboard Manager
`@supervisor: clipboard-set <data>` / `@supervisor: clipboard-get`.
Supervisor holds one clipboard slot. Apps with `clipboard = true` capability can access it.

### P54 — Drag and Drop
Mouse drag from one app window to another emits `VYOMA_INPUT:drag:<source_app>:<data>`.
Target app receives the drag data if it declares `drag_drop = true`.

### P55 — Archive Support
WASM app using `zip` / `tar` crates. Opens `.zip`/`.tar.gz` from file manager, extracts to `/data`.
Capabilities: `filesystem`, `display`, `shell`.

### P56 — Full-Text Search
WASM app indexes `/data` on a schedule. Provides `@supervisor: search <query>` IPC.
Results shown in app launcher overlay (P38) and file manager.

### P57 — Recycle Bin
File manager Delete key moves file to `/data/.trash/`. Supervisor tracks metadata.
`@supervisor: empty-trash` permanently removes.

---

## Milestone G — App Ecosystem (P58–P64)

### P58 — App Store Frontend
WASM app: browse categories, search, install with one click.
Fetches manifest list from registry over HTTPS.

### P59 — Package Registry Backend
Signed `.wasm` bundle hosting. Registry manifest format: name, version, sha256, capabilities.
Apps verified at install (P28). Registry served by http-server (self-hosted option).

### P60 — Runtime Capability Requests
Apps can request capabilities not declared at install: `@supervisor: request <cap>`.
Supervisor shows permission dialog (via notification overlay); user grants/denies.
Grant is remembered in `/data/permissions.toml`.

### P61 — App Permissions UI
Settings sub-page: per-app capability overrides. Toggle individual caps on/off.
Supervisor reloads permissions on save.

### P62 — Auto-Update Manager
Background daemon app. Checks registry for version bumps nightly.
Presents update available notification; applies via OTA (P30).

### P63 — Background Services
Daemon apps: `background = true` in vyoma.toml means no display, no focus, no TTY.
Supervisor doesn't draw anything for them. Enables services like sync, indexing, backup.

### P64 — App Inter-Op (Share Sheet)
`@supervisor: share <data> <mime>` — supervisor shows share sheet overlay listing
capable apps. User picks one; supervisor routes data to that app's stdin.

---

## Milestone H — Developer Tools (P65–P71)

### P65 — Full Terminal Emulator
WASM app: VTE-compatible ANSI escape codes, 256-color palette, cursor positioning,
scrollback buffer. Runs any WASM app that speaks ANSI in a proper terminal window.

### P66 — Code Editor
WASM app: syntax highlighting (Rust, TOML, Markdown), line numbers, find/replace,
bracket matching. Saves to `/data`. Uses multi-line text buffer with gap buffer.

### P67 — WASM Debugger / Inspector
Supervisor exposes `VYOMA_DEBUG:` protocol: breakpoints, step, inspect WASM linear memory,
call stack. Debugger WASM app connects via IPC.

### P68 — System REPL
WASM app: Lua or minimal scripting engine. `@supervisor` commands via IPC from scripts.
Enables system automation, macros, test scripts.

### P69 — Build Integration
Shell `build <app>` command: supervisor finds Cargo.toml in `/data/src/<app>`, runs
`cargo build --target wasm32-wasip2` via a build-runner WASM app, copies output to `/apps`.

### P70 — Profiler / Trace Viewer
Supervisor instruments VYOMA_DRAW call counts, IPC message rates, app CPU time.
Exposes `@supervisor: trace` → structured report. Trace viewer WASM app renders timeline.

### P71 — Vyoma SDK
`vyoma-sdk` CLI: scaffold a new app (Cargo.toml + vyoma.toml template), build, package,
sign for submission to registry. Distributable as a WASM app itself.

---

## Milestone I — Media & I/O (P72–P78)

### P72 — Audio Subsystem
Kernel: `CONFIG_VIRTIO_SND=y`. WASI audio interface: `audio-out = true` capability.
Supervisor routes PCM audio from apps → virtio-snd → host audio.
Sample app: tone generator, later: media player.

### P73 — Image Viewer
WASM app using `image` crate (PNG/JPEG/WebP decoder). Renders decoded pixels via
`VYOMA_DRAW:blit_pixels:<x>,<y>,<w>,<h>,<data_hex>` — new protocol command.
File manager opens image files via file associations (P52).

### P74 — Video Playback
WASM app: software video decoder (av1/h264 via dav1d/OpenH264 WASM port).
Renders frames via `blit_pixels` command. Audio via virtio-snd.

### P75 — Camera Support
Kernel: `CONFIG_V4L2_CORE=y`, `CONFIG_VIRTIO_VIDEO=y`.
WASI camera interface: `camera = true` capability → supervisor opens V4L2 device,
streams frames to app.

### P76 — USB HID
Kernel: `CONFIG_USB_HID=y`, `CONFIG_HID=y`. Supervisor reads `/dev/input/event*` for
USB keyboard/mouse (in addition to virtio-mouse already present from P22).
Enables real hardware keyboard/mouse without virtio.

### P77 — Bluetooth
Kernel: `CONFIG_BT=y`, `CONFIG_BT_HCIUART=y`. Bluetooth manager WASM app pairs devices.
`bluetooth = true` capability. Initial use case: audio output, keyboard pairing.

### P78 — Printing
WASM app renders document to PDF. `@supervisor: print <file>` routes to printer driver
(initially: save PDF to `/data`; later: IPP network printer support).

---

## Milestone J — Accessibility & I18N (P79–P84)

### P79 — Screen Reader
WASM app subscribes to `VYOMA_ACCESSIBILITY:` events (text drawn to screen, focus changes).
Reads aloud via audio subsystem (P72). Apps emit `accessibility_label` on VYOMA_DRAW text.

### P80 — High Contrast / Dark Mode
`/data/settings.toml` theme field: `dark`/`light`/`high-contrast`. Supervisor broadcasts
`VYOMA_SYSTEM:theme:<name>` to all apps. Apps switch color palettes accordingly.

### P81 — Global Font Scale
Settings option: base font size multiplier (1×/1.5×/2×). Supervisor broadcasts to apps.
Apps re-layout using new scale factor.

### P82 — Keyboard-Only Navigation
Focus-ring protocol: Tab key cycles focus between interactive elements within an app.
Supervisor routes Tab to focused app; app signals element boundaries via IPC.

### P83 — TTF Font Rendering
Replace 8×8/8×16/16×32 bitmap fonts with FreeType-based TTF renderer (WASM build).
`VYOMA_DRAW:draw_text_ttf:<x>,<y>,<rgba>,<size_px>,<font_id>,<text>`.
Ships with a subset of a permissively licensed font (e.g., Noto Sans subset).

### P84 — Unicode & RTL
Supervisor display module: Unicode text shaping via HarfBuzz (WASM build), bidirectional
text algorithm (UAX #9). Enables Arabic, Hebrew, CJK text in WASM apps.

---

## Milestone K — Security & Multi-User (P85–P91)

### P85 — User Accounts
`/data/users.toml`: name, password hash, home dir (`/data/users/<name>`).
Supervisor login screen (WASM app) at boot. Session token stored in supervisor state.

### P86 — Per-User Capability Tokens
Capabilities scoped to the logged-in user's session token.
App requesting `filesystem` gets `/data/users/<name>` not all of `/data`.

### P87 — Encrypted Storage
Per-user home directory encrypted via ChaCha20-Poly1305 (pure Rust/WASM).
Key derived from login password via Argon2. Mounted at login, unmounted at logout.

### P88 — Per-App Firewall
`network_allow = ["api.example.com:443"]` in vyoma.toml. Supervisor intercepts WASI
socket calls, checks destination against allow-list before forwarding.

### P89 — Full Capability Audit Log
Every capability access (filesystem open, network connect, IPC send) logged to
`/data/audit.log` with timestamp, app name, and action. Query via system monitor.

### P90 — 2FA Authentication
Login screen: optional TOTP second factor. Secret stored encrypted in user record.
`vyoma-sdk` generates QR code for setup.

### P91 — Secure Boot
UEFI Secure Boot signature for kernel + initramfs. Supervisor verifies WASM app
signatures (P28) against a trusted key ring stored in encrypted storage.

---

## Milestone L — Performance (P92–P97)

### P92 — Cranelift JIT
Switch wasmtime from interpreter mode to Cranelift JIT compilation.
Requires bumping wasmtime to a version with stable WASM-to-native compilation.
Expected: 5–10× app startup speed improvement.

### P93 — App Pre-Loading
Supervisor pre-compiles WASM modules to native code at boot (AOT cache in `/data/cache/`).
First run after install is slow; subsequent runs near-instant.

### P94 — Memory Pressure Manager
Supervisor tracks per-app memory usage via `/proc/<pid>/status`.
Under pressure: pause low-priority apps (SIGSTOP), swap their state to `/data/swap/`.
Resume on demand (SIGCONT).

### P95 — Battery / Power Management
ACPI battery events via `/sys/class/power_supply`. Supervisor broadcasts
`VYOMA_SYSTEM:battery:<percent>:<charging>` to apps. Settings app shows battery widget.
On critical level: auto-suspend.

### P96 — CPU Governor
`cpufreq` via `/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor`.
Settings app exposes performance/balanced/efficiency modes.
Supervisor throttles background apps in efficiency mode.

### P97 — GPU Acceleration
virtio-gpu virgl backend: supervisor renders VYOMA_DRAW via OpenGL ES instead of
software rasterization. `VYOMA_DRAW:gpu_blit` for GPU-accelerated pixel operations.
Enables smooth video, transitions, and GPU-backed compositing (P31 follow-up).

---

## Milestone M — Real Hardware (P98–P104)

### P98 — UEFI Boot
Replace QEMU `-kernel` direct boot with proper UEFI boot loader.
Generate UEFI-bootable disk image with EFI system partition + VyomaOS partition.
Tests on bare-metal x86_64 hardware.

### P99 — GRUB / systemd-boot
GRUB2 as first-stage bootloader for legacy BIOS hardware.
systemd-boot for UEFI. Both chain to VyomaOS kernel + initramfs.

### P100 — NVMe / SATA Driver
Kernel: `CONFIG_BLK_DEV_NVME=y`, `CONFIG_ATA=y`, `CONFIG_SATA_AHCI=y`.
Replace virtio-blk with real storage driver.
`/data` moves to native NVMe partition (ext4).

### P101 — USB HID (full stack)
Kernel: `CONFIG_USB=y`, `CONFIG_USB_XHCI_HCD=y`, `CONFIG_USB_HID=y`.
Full USB host stack: keyboards, mice, thumb drives via USB mass storage.
Thumb drives auto-mount as `/media/usb0`.

### P102 — Real NIC Drivers
Kernel: `CONFIG_E1000=y`, `CONFIG_R8169=y` (Intel e1000, Realtek RTL8169).
Remove virtio-net dependency for bare-metal networking.
Tests: `ping 8.8.8.8`, `curl http://...` from http-server app.

### P103 — WiFi Driver
Kernel: `CONFIG_MAC80211=y`, `CONFIG_CFG80211=y`, plus target NIC (e.g., ath9k, iwlwifi).
WiFi manager WASM app: scan networks, connect, store credentials in encrypted storage.

### P104 — Full ACPI
Kernel: `CONFIG_ACPI=y`. Power button, sleep state, wake events, thermal zones.
Supervisor handles ACPI events: lid close → sleep, power button → shutdown dialog.

---

## Milestone N — Production (P105–P112)

### P105 — OS Installer
TUI WASM app: select target disk, partition, install kernel + initramfs + supervisor.
Creates user account. Writes GRUB config. Reboots into installed system.

### P106 — Recovery Mode
Boot option: minimal environment with file manager + terminal only.
Can reset user password, restore from backup, check filesystem integrity.

### P107 — Backup / Restore
Background daemon: incremental backup of `/data` to `/media/backup/` (USB or network share).
Schedule in Settings. Restore via recovery mode (P106).

### P108 — Atomic Updates
A/B partition scheme: kernel + initramfs on partition A (active) and B (standby).
`@supervisor: update-os <url>` writes new image to B, sets next-boot to B.
Automatic rollback if new version fails to boot.

### P109 — Remote Desktop / VNC
WASM VNC server app: encodes framebuffer diffs, serves over TCP (port 5900).
`network = true`, `display = true` capabilities. Encrypted via TLS (P45).

### P110 — Virtualization Support
Kernel: `CONFIG_KVM=y`. Supervisor launches KVM VMs from WASM VM-manager app.
Enables running legacy Linux apps inside VyomaOS as nested VMs.

### P111 — Container Runtime
WASM app implementing OCI runtime spec: pull images from registry, run in Linux namespaces
(P27), export via WASI filesystem. Enables running Docker-style containers as WASM apps.

### P112 — App Store Launch
Public registry at `apps.vyomaos.dev`: curated, signed, capability-annotated bundles.
`vyoma-sdk publish` workflow. App review guidelines. In-OS App Store (P58) points to it.

---

## Execution order

```
NOW (Wave 3 — parallel):
  P24 File Manager
  P25 Text Editor
  P26 System Monitor

After Wave 3 merged (Wave 4 — sequential):
  P27 → P28 → P29 → P30

After Wave 4:
  Milestone C (P31–P36) — Window Manager
  Milestone D (P37–P43) — Desktop Shell
  Milestone E (P44–P50) — Rich Networking
  ...continue milestones in order F through N
```

Each milestone ships independently and improves daily usability.
The OS is fully self-hostable (i.e., can be developed on itself) after Milestone H.
Real-hardware target is achievable after Milestone M.
Public release readiness after Milestone N.
