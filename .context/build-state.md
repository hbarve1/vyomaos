# VyomaOS Auto-Build State
<!-- Owned by the autonomous loop. Each iteration reads this, does work, updates it. -->

last_updated: 2026-05-20
repo: /Users/hbarve1/codes/hbarve1/vyomaos

## Current batch
status: ready
phases:
  - P64 — Clock Widget
  - P65 — Weather App
notes: |
  P64: Clock Widget. Create apps/clock/ WASM app.
       Window x=300, y=200, w=320, h=360. Capabilities: stdio=true, display=true.
       Shows a digital clock face: HH:MM:SS in large text (use 'l' font size).
       Below: date line (e.g., "Wednesday  20 May 2026").
       Since WASM has no system clock, use a hardcoded start datetime (2026-05-20 00:00:00)
       and advance it on each REPLY tick. App sends "@supervisor: tick" to get a REPLY every second.
       Actually: use a simpler approach — display a static time on first render, then on each
       keypress/REPLY advance the second counter. Poll with `@supervisor: ping` every ~1s if available,
       or just show a static time + note "press any key to tick".
       Ctrl+C: quit.

  P65: Weather App. Create apps/weather/ WASM app.
       Window x=200, y=100, w=760, h=480. Capabilities: stdio=true, display=true, filesystem=true.
       Reads /data/weather.toml for weather data. Format:
         [[day]]
         date = "2026-05-20"
         condition = "Sunny"
         temp_hi = 28
         temp_lo = 18
         humidity = 55
       Shows current day's data prominently. Left/Right navigate to prev/next day entries.
       'r' re-reads the file (refresh). Shows "No weather data" if file missing.
       Ctrl+C: quit.

## Queue (implement in order after current batch)
- [ ] P66 — Scientific Calculator: extends calculator; adds sin/cos/tan/sqrt/log/pow buttons; toggle deg/rad
- [ ] P67 — Pomodoro Timer: 25min work + 5min break cycle; visual countdown; spacebar start/pause; n for next phase
- [ ] P68 — Log Viewer: reads /data/*.log files; real-time tail (polls every 2s via re-read); grep filter; color severity

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
- [x] P46: Basic Browser — apps/browser/ (1440×880 y=20); @supervisor: http-get fetches URL + returns 4096-char body; HTML stripped; Up/Down scroll; Ctrl+L URL bar; status bar
- [x] P47: SSH Client / TCP Tunnel — apps/ssh-client/ (1240×700); @supervisor: tcp-connect/tcp-send/tcp-recv/tcp-close; TCP_CONNS global map; TCP_NEXT_ID atomic; form→connected terminal view
- [x] P48: Network Config UI — apps/network-config/ (1040×600); DHCP/Static toggle (d/s keys); 5 IP fields; reads+writes /data/network.toml; Tab/arrows/Ctrl+W/Ctrl+C
- [x] P49: Download Manager — @supervisor: download <url> <dest>; background thread; http_get(); REPLY:download-progress/done/error; shell `download` command
- [x] P50: Clipboard Manager — static CLIPBOARD: OnceLock<Mutex<String>>; @supervisor: clipboard-set/get; REPLY:clipboard-set ok / REPLY:clipboard <text>; shell clip-set/clip-get
- [x] P51: Screenshot — display::Framebuffer.screenshot() method; @supervisor: screenshot <path>; writes P6 PPM (BGRA→RGB); shell `screenshot [path]` defaults to /data/screenshot.ppm
- [x] P52: Virtual Keyboard — apps/virtual-keyboard/ (1000×320 at y=560); QWERTY + Space/Backspace/Enter; mouse click sends @supervisor: input <char>; supervisor P52 `input` cmd routes char to focused app stdin
- [x] P53: Color Picker — apps/color-picker/ (640×500); 64×64 HSV gradient (4px cells); value slider; pure integer HSV→RGB; Ctrl+W outputs "color: #RRGGBBFF"
- [x] P54: Process Inspector — apps/process-inspector/ (840×720); ps-raw poll 2s; scrollable table; Enter→detail; @supervisor: win-info added; q/Ctrl+C back/quit
- [x] P55: Font Chooser — apps/font-chooser/ (440×340); 3 buttons S/M/L with preview; Enter → @supervisor: font-size; static FONT_SIZE in supervisor
- [x] P56: App Store UI — apps/app-store/ (1440×880); pkg-list query; 4-col grid; search filter; Enter install/remove; re-queries after action; Ctrl+C exit
- [x] P57: Audio Player Stub — apps/audio-player/ (440×260); lists /data/*.raw; scrollable track list; play/pause/prev/next; cosmetic progress bar (advances on keypress when playing)
- [x] P58: Image Viewer — apps/image-viewer/ (1240×760); lists /data/*.ppm; PPM P6 loader; 4×4 block RLE rendering; ↑↓ scroll ←→ prev/next; list+image views
- [x] P59: Hex Editor — apps/hex-editor/ (1280×760); path input → hex+ASCII view; 16 bytes/row; edit mode (2 hex digits); u/d page nav; Ctrl+W save
- [x] P60: Calendar Widget — apps/calendar/ (640×520); month grid Sun–Sat; today highlighted; ←→↑↓ navigate months; weekend accent color
- [x] P61: Markdown Viewer — apps/markdown-viewer/ (1240×760); lists /data/*.md; H1/H2/H3/bold/code/bullet rendering; ↑↓ scroll ←→ prev/next
- [x] P62: Password Manager — apps/password-manager/ (960×640); XOR-encrypted vault.enc; master password unlock; add/delete entries; 'c' copy pass to clipboard
- [x] P63: Task Manager — apps/task-manager/ (640×560); TOML [[task]] list; add/delete/toggle-done; ↑↓ nav; Ctrl+W save

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
