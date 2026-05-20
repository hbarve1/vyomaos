# VyomaOS Auto-Build State
<!-- Owned by the autonomous loop. Each iteration reads this, does work, updates it. -->

last_updated: 2026-05-21
repo: /Users/hbarve1/codes/hbarve1/vyomaos

## Current batch
status: ready
phases:
  - P32 — Window Decorations
  - P33 — Z-Ordering
notes: |
  P32: supervisor/src/main.rs — before dispatching VYOMA_DRAW commands for a display app,
       draw a title bar chrome above the app's window region: thin bar (height=20) at
       (win_x, win_y - 20, win_w, 20) filled with 0x21262DFF, app name in 0xFFFFFFFF at
       (win_x+8, win_y-16), close button red circle at (win_x+win_w-16, win_y-14) radius 6
       drawn as a 12×12 fill_rect 0xFF5F56FF. Draw this chrome only once at app launch
       (when VYOMA_SYSTEM:screen is sent, or when app first seen). On VYOMA_DRAW:flush/present,
       redraw the chrome on top so apps can't overwrite it.
       Add VYOMA_SYSTEM:window_event:close to app stdin when the close button region is clicked
       (requires checking mouse click coords — reuse mouse input path).
       Read supervisor/src/main.rs lines 870-920 (the VYOMA_DRAW dispatch path) for context.
  P33: supervisor/src/main.rs — add a Z-order stack: Vec<String> of app names ordered
       front-to-back. When supervisor dispatches VYOMA_DRAW to an app, it draws at the app's
       window region. On mouse click, find the topmost app whose window contains the click point
       and raise it to front of the stack (move to index 0). Add @supervisor: raise <app> and
       @supervisor: lower <app> IPC commands. On each flush/present, re-composite windows in
       Z-order (back-to-front) by doing nothing special — the back-buffer already handles it
       since windows are independent regions. The key work: track z_order: Vec<String>, on click
       raise clicked app, send VYOMA_SYSTEM:focus:<app> to the newly-raised app.
       Shell: add `raise <app>` and `lower <app>` commands routing to @supervisor: raise/lower.

## Queue (implement in order after current batch)
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
- [x] P27: App Namespaces — libc::unshare(CLONE_NEWNS|CLONE_NEWPID) in pre_exec; kernel config updated
- [x] P28: Signed Bundles — AppMeta.wasm_sha256: Option<String>; sha2::Sha256 verify before spawn; sha2 dep added
- [x] P29: Multi-Resolution — display::screen_size() reads FBIOGET_VSCREENINFO; launch_app_threads sends VYOMA_SYSTEM:screen:<w>,<h> to display apps
- [x] P30: OTA Hot-Swap — @supervisor:update <app> <url>; http_get() raw TCP; sha256 verify; atomic copy; restart in background thread; shell `update` command
- [x] P31: Double-Buffered Compositor — Framebuffer.back: Vec<u8>; all draw ops write to back; flush()/present blit back→mmap; VYOMA_DRAW:present alias added

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
