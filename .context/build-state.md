# VyomaOS Auto-Build State
<!-- Owned by the autonomous loop. Each iteration reads this, does work, updates it. -->

last_updated: 2026-05-24
repo: /Users/hbarve1/codes/hbarve1/vyomaos

## Current batch
status: ready
phases:
  - P38 — App Launcher
  - P39 — Notifications
notes: |
  P38: Create apps/app-launcher/ WASM app. A full-screen overlay (x=0, y=0, w=1440, h=900)
       that shows available apps from @supervisor: pkg-list and installed apps. Semi-transparent
       background (0x000000CC — use fill with rgba that shows through). Shows a grid of app
       name buttons. Typing filters app names. Enter or click launches selected app via
       @supervisor: run /apps/<name>/vyoma.toml (or /data/apps/<name>/vyoma.toml for installed).
       Listens for keyboard input: printable chars append to search buffer, Backspace removes,
       Ctrl+C or Escape exits (quit self). Shows "App Launcher" title at top. Capabilities:
       stdio=true, display=true, shell=true. Keyboard focus must be given to it via shell: run app-launcher.

  P39: supervisor/src/main.rs — handle new @supervisor: command "notify <title> <msg>".
       Supervisor draws a toast notification overlay directly on the framebuffer:
       a 400×60 rect at top-right (x=1020, y=10) with background 0x21262DFF, border 0x58A6FFFF,
       title text in 0xFFFFFFFF at (1028, 18), message text in 0x8B949EFF at (1028, 34).
       After drawing, flush to screen. Toast stays for 3 seconds then supervisor clears it
       (fill_rect the same region with wallpaper black 0x0D1117FF and flush) in a background thread.
       Shell: add `notify <title> <msg>` command routing to @supervisor: notify.

## Queue (implement in order after current batch)
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
- [x] P27: App Namespaces — libc::unshare(CLONE_NEWNS|CLONE_NEWPID) in pre_exec; kernel config updated
- [x] P28: Signed Bundles — AppMeta.wasm_sha256: Option<String>; sha2::Sha256 verify before spawn; sha2 dep added
- [x] P29: Multi-Resolution — display::screen_size() reads FBIOGET_VSCREENINFO; launch_app_threads sends VYOMA_SYSTEM:screen:<w>,<h> to display apps
- [x] P30: OTA Hot-Swap — @supervisor:update <app> <url>; http_get() raw TCP; sha256 verify; atomic copy; restart in background thread; shell `update` command
- [x] P31: Double-Buffered Compositor — Framebuffer.back: Vec<u8>; all draw ops write to back; flush()/present blit back→mmap; VYOMA_DRAW:present alias added
- [x] P32: Window Decorations — chrome (title bar + close button) painted at flush/present; VYOMA_SYSTEM:window_event:close on close-button click; guard wy>=20
- [x] P33: Z-Ordering — Z_ORDER OnceLock<Mutex<Vec<String>>>; z_order_push_front/back; click-to-raise sets focus; @supervisor: raise/lower; shell raise/lower commands
- [x] P34: Window Manager App — apps/window-manager/; queries list on start; raises apps in sorted order; focuses top app; shell `retile` via stdin
- [x] P35: Desktop Wallpaper — default 0x0D1117FF painted at startup; @supervisor: wallpaper <rgba>; shell `wallpaper <rgba>`
- [x] P36: Window Resize Events — @supervisor: resize <app> <w> <h>; updates AppState.win_region; sends VYOMA_SYSTEM:resize:<w>,<h> to app; shell `resize` command
- [x] P37: Taskbar App — apps/taskbar/ at y=860 h=40; ps-raw poll every 2s; app buttons with click-to-focus; elapsed clock; brand label

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
