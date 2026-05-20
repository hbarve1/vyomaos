# VyomaOS Auto-Build State
<!-- Owned by the autonomous loop. Each iteration reads this, does work, updates it. -->

last_updated: 2026-05-25
repo: /Users/hbarve1/codes/hbarve1/vyomaos

## Current batch
status: ready
phases:
  - P46 — Basic Browser
  - P47 — SSH Client
notes: |
  P46: Create apps/browser/ WASM app. Window x=0, y=20, w=1440, h=880.
       Capabilities: stdio=true, display=true, shell=true, network=true.
       Shows a URL bar at the top (input field). User types URL and presses Enter.
       App sends @supervisor: http-get <url> which returns REPLY:http-get <status> <body_truncated_at_4096_chars>.
       Supervisor adds http-get command: does raw TCP HTTP GET (reuse http_get() pattern),
       returns first 4096 chars of response body.
       Browser strips HTML tags from response and renders as plain text with text_wrap.
       Navigation: Up/Down scrolls, Ctrl+L focuses URL bar, Ctrl+C quits.
       Status bar at bottom shows current URL and response code.

  P47: Create apps/ssh-client/ WASM app. Window x=100, y=100, w=1240, h=700.
       Capabilities: stdio=true, display=true, shell=true, network=true.
       Since full SSH in WASM is complex, implement a useful subset:
       Shows a form: host input, port input (default 22), user input.
       On Enter, app sends @supervisor: tcp-connect <host>:<port> and gets back
       REPLY:tcp-connect <id> (or REPLY:tcp-connect error).
       Supervisor handles tcp-connect: opens raw TCP socket, assigns an ID (incrementing integer),
       stores in a HashMap<u32, TcpStream>. App then can send/receive via
       @supervisor: tcp-send <id> <data> and receives REPLY:tcp-recv <id> <data>.
       This gives apps a generic raw TCP channel. The SSH client app shows a terminal-like
       display and sends raw keystrokes. Note: actual SSH crypto is future work — this builds the TCP plumbing.

## Queue (implement in order after current batch)
- [ ] P48 — Network Config UI: settings app sub-page; /data/network.toml
- [ ] P49 — Download Manager: @supervisor: download <url> <dest>; background to /data
- [ ] P50 — WebSocket: WASI socket WS upgrade; real-time apps
- [ ] P51 — Clipboard Manager: supervisor clipboard buffer; @supervisor: clipboard-set/get
- [ ] P52 — Screenshot: @supervisor: screenshot <path>; blit back-buffer to raw PPM in /data
- [ ] P53 — Virtual Keyboard: on-screen keyboard WASM app for touch input
- [ ] P54 — Color Picker: WASM color picker widget; writes chosen RGBA hex to stdout
- [ ] P55 — Terminal Emulator: full VT100 WASM app; spawn shell via @supervisor: shell-spawn
- [ ] P56 — Process Inspector: detailed app info; threads, memory, uptime; click on name in ps

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
- [x] P38: App Launcher — apps/app-launcher/ full-screen overlay; pkg-list query; search filter; 4-col app grid; Enter launches; Ctrl+C exits; shell `run app-launcher`
- [x] P39: Notifications — @supervisor: notify <title> <msg>; toast at (1020,10,400,60); 0x21262DFF bg + 0x58A6FFFF border; auto-clear after 3s in background thread; shell `notify` command
- [x] P40: Settings App — apps/settings/ (w=1440,h=880,y=20); sidebar+content layout; Display/Font/Boot sections; /data/settings.toml read+write; Ctrl+W saves; Tab switches; shell `run settings`
- [x] P41: Power Manager — @supervisor: shutdown (libc::LINUX_REBOOT_CMD_POWER_OFF) + reboot (LINUX_REBOOT_CMD_RESTART); 500ms delay before action; shell `shutdown`/`reboot` commands
- [x] P42: Session Manager — @supervisor: session-save writes [[window]] TOML to /data/session.toml; session-restore parses and updates win_region + sends resize; shell session-save/session-restore
- [x] P43: Multi-Monitor — @supervisor: monitors counts /sys/class/drm/card0-* entries; fallback 1; shell `monitors` command
- [x] P44: DNS Resolver — apps/dns-resolver/ TUI app (840x500); @supervisor: dns-resolve <host> does TCP DNS to 8.8.8.8:53; parses A record; REPLY:dns <host> <ip>; shell `dns` command
- [x] P45: HTTPS/TLS groundwork — @supervisor: tls-info checks /data/cert.pem+key.pem; http-server checks at startup + serves /tls JSON endpoint; shell `tls-info` command

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
