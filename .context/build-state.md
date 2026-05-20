# VyomaOS Auto-Build State
<!-- Owned by the autonomous loop. Each iteration reads this, does work, updates it. -->

last_updated: 2026-05-20
repo: /Users/hbarve1/codes/hbarve1/vyomaos

## Current batch
status: ready
phases:
  - P27 — App Namespaces
  - P28 — Signed Bundles
notes: |
  P27: supervisor/src/main.rs — wrap wasmtime child spawn with unshare(CLONE_NEWNS|CLONE_NEWPID).
       kernel base/kernel.config — add CONFIG_NAMESPACES=y CONFIG_PID_NS=y CONFIG_MNT_NS=y.
       Use nix crate for unshare syscall OR raw libc::unshare call in the supervisor.
       Check current supervisor Cargo.toml for existing deps before adding nix.
  P28: vyoma.toml — add optional wasm_sha256 field in [app] section.
       supervisor/src/main.rs — after reading wasm path, sha256sum the binary, compare.
       Use sha2 crate (add to supervisor/Cargo.toml). Refuse spawn if mismatch.
       Keep backward-compat: if wasm_sha256 absent, skip verification (warn only).
  These touch supervisor and kernel config. Read supervisor/Cargo.toml before editing.
  For P27 unshare: the supervisor itself is PID 1 musl binary. Use libc crate (likely already present).
  Raw syscall: libc::unshare(libc::CLONE_NEWNS | libc::CLONE_NEWPID) before execvp of wasmtime.

## Queue (implement in order after current batch)
- [ ] P29 — Multi-Resolution: supervisor reads FBIOGET_VSCREENINFO; broadcasts VYOMA_SYSTEM:screen:<w>,<h> to display apps before first draw; apps use it instead of hardcoded 1440x900
- [ ] P30 — OTA Hot-Swap: @supervisor: update <app> <url>; HTTP GET via reqwest-wasm or built-in downloader; verify sha256; replace /data/apps/<app>/<app>.wasm; restart app
- [ ] P29 — Multi-Resolution: supervisor reads FBIOGET_VSCREENINFO; broadcasts VYOMA_SYSTEM:screen:<w>,<h> to display apps before first draw; apps use it instead of hardcoded 1440x900
- [ ] P30 — OTA Hot-Swap: @supervisor: update <app> <url>; HTTP GET via reqwest-wasm or built-in downloader; verify sha256; replace /data/apps/<app>/<app>.wasm; restart app
- [ ] P31 — Double-Buffered Compositor: supervisor back-buffer; VYOMA_DRAW:present to flip; eliminates tearing
- [ ] P32 — Window Decorations: supervisor draws title bar/close/min/max chrome around app windows; VYOMA_SYSTEM:window_event to app on close
- [ ] P33 — Z-Ordering: window stack; @supervisor: raise/lower; click raises
- [ ] P34 — Window Manager App: WASM app manages layout/Z-order via supervisor IPC
- [ ] P35 — Desktop Wallpaper: VYOMA_SYSTEM:wallpaper IPC; solid color or image fill as bottom layer
- [ ] P36 — Window Resize Events: VYOMA_SYSTEM:resize:<w>,<h> to app on resize; apps redraw at new dims
- [ ] P37 — Taskbar App: dock, running app icons, clock, system tray; click to focus
- [ ] P38 — App Launcher: Meta key overlay, search + launch; queries app registry
- [ ] P39 — Notifications: toast overlay; @supervisor: notify <app> <msg>
- [ ] P40 — Settings App: display/font/theme/boot config; writes /data/settings.toml
- [ ] P41 — Power Manager: ACPI shutdown/reboot via @supervisor: shutdown|reboot
- [ ] P42 — Session Manager: save/restore window positions to /data/session.toml
- [ ] P43 — Multi-Monitor: enumerate DRM connectors; per-monitor framebuffer surface
- [ ] P44 — DNS Resolver: WASM resolver app; supervisor proxies DNS queries
- [ ] P45 — HTTPS/TLS: rustls in WASM apps; http-server serves HTTPS
- [ ] P46 — Basic Browser: WASM app; fetch HTML via HTTP; render stripped text
- [ ] P47 — SSH Client: WASM SSH client; pure-Rust SSH library
- [ ] P48 — Network Config UI: settings sub-page; /data/network.toml
- [ ] P49 — Download Manager: @supervisor: download <url> <dest>; background to /data
- [ ] P50 — WebSocket: WASI socket WS upgrade; real-time apps

## Completed
- [x] P01–P08: Build foundation, kernel, supervisor, WASM runtime, IPC, seccomp, storage
- [x] P09–P10: DRM display, bitmap font, VYOMA_DRAW protocol
- [x] P11–P12: Networking (HTTP), interactive shell, keyboard routing
- [x] P13–P17: Process management, package manager, persistent logs, real-time TTY
- [x] P18–P20: Shell UX (history+arrows), watchdog, font scaling
- [x] P21–P23: Window regions, mouse input, TUI widget primitives (rect_border, clear_region, draw_text_wrap)
- [x] P24: File Manager — apps/file-manager/ complete; shell `run` auto-focuses; Makefile wired
- [x] P25: Text Editor — apps/text-editor/ complete; path-input → edit mode; Ctrl+W save; Ctrl+C save+quit
- [x] P26: System Monitor — apps/system-monitor/ complete; polls ps-raw every 1s; table view; q to quit

## Reference patterns (minimise file reads each iteration)
app_structure: |
  apps/<name>/Cargo.toml  (name, edition, [[bin]], [profile.release] opt-level="z" strip=true)
  apps/<name>/vyoma.toml  ([app], [capabilities], [window])
  apps/<name>/src/main.rs (VYOMA_DRAW helpers: fill/text/border/clear_region/text_wrap/flush)
  Makefile: add line in apps target — $(DOCKER_RUN) cargo build --manifest-path apps/<name>/Cargo.toml --target wasm32-wasip2 --release
  plan README: .context/plans/plan-vyomaos/README.md — add row and mark in_progress→done

draw_helpers: |
  fill(x,y,w,h,rgba)      → VYOMA_DRAW:fill_rect:x,y,w,h,rgba
  text(x,y,rgba,s)        → VYOMA_DRAW:draw_text:x,y,rgba,m,s
  border(x,y,w,h,rgba)    → VYOMA_DRAW:rect_border:x,y,w,h,rgba
  clear_region(x,y,w,h)   → VYOMA_DRAW:clear_region:x,y,w,h
  text_wrap(x,y,mw,rgba,s)→ VYOMA_DRAW:draw_text_wrap:x,y,mw,rgba,m,s
  flush()                 → VYOMA_DRAW:flush + stdout flush

keyboard_input: |
  Apps read stdin line-by-line (BufRead).
  "\x1b[A" = Up, "\x1b[B" = Down, "\x1b[C" = Right, "\x1b[D" = Left
  ""       = Enter, "\x7f" = Backspace, "\x03" = Ctrl+C
  "REPLY:<data>" = supervisor reply to @supervisor: commands
  Single printable char = keypress

screen: 1440x900 (virtio-gpu, declared in vyoma.toml [window])

## Token discipline for each iteration
1. Read only this file + CLAUDE.md (200 lines) at start
2. For unknown patterns, read ONE reference file (e.g., apps/shell/src/main.rs)
3. Implement fully — no stubs, no TODOs
4. Update this file: check off completed phases, move next batch from queue
5. Update .context/plans/plan-vyomaos/README.md rows
6. **Commit + push** — stage only changed/new files (never .claude/ .gemini/ *.lock),
   commit message format: `feat(pNN-pMM): <short description>\n\nCo-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>`
   then `git push origin develop`
7. If all queue items done: write "status: COMPLETE — all phases done" and stop
