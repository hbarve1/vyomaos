# VyomaOS Auto-Build State
<!-- Owned by the autonomous loop. Each iteration reads this, does work, updates it. -->

last_updated: 2026-05-21
repo: /Users/hbarve1/codes/hbarve1/vyomaos

## Goal: macOS-like OS
The next major milestone is a macOS-like desktop experience:
- Menu Bar (top bar): app name, menus, clock, system tray icons
- Dock (bottom): app icons, running indicator dots, click to launch/focus
- Spotlight: Cmd+Space overlay for instant app search + launch
- Desktop: wallpaper + file icons, right-click context
- App Switcher: Alt+Tab window cycling
- Notification Center: slide-in panel from right edge
- Mission Control: bird's-eye view of all windows
- Finder v2: sidebar + breadcrumbs + icon grid

## Current batch
status: ready
phases:
  - P78 — Desktop Icons
  - P79 — Context Menu
notes: |
  P78: Desktop Icons. Create apps/desktop/ WASM app.
       Window x=0, y=28, w=1440, h=832 (between menu bar and dock).
       Capabilities: stdio=true, display=true, shell=true, filesystem=true.
       Background: 0x0D1117FF (same as wallpaper). No border/chrome (y>0 so chrome appears — guard: display at y=28).
       Actually draw at y=28 by using fill from 0,0 within the window coord space.
       Lists files in /data/ via @supervisor: ls-data (supervisor responds REPLY:ls-data <space-sep filenames>).
       Fallback if supervisor doesn't support ls-data: show a set of fixed icons (Desktop, Downloads, Documents).
       Each icon: 80×80 box at grid positions; 8 per row; y starting at 20 within window.
       Icon shows: colored folder/file icon (fill square + border), filename below (truncated to 10 chars).
       ↑↓←→ to navigate; Enter → opens file with appropriate viewer based on extension:
         .md → run markdown-viewer, .json → run json-viewer, .csv → run csv-viewer,
         .ppm → run image-viewer, .log → run log-viewer, else → run text-editor.
       'n' → prompt for new filename (show text input at bottom); Enter → create /data/<name> (write @supervisor: touch <path>).

  P79: Context Menu. Create apps/context-menu/ WASM app.
       Window x=0, y=0, w=200, h=auto (8px padding + items*28). Capabilities: stdio=true, display=true, shell=true.
       Launched by @supervisor: run context-menu; receives menu spec via stdin within first 100ms:
         "MENU:<x>,<y>:<item1>|<item2>|<item3>..."
       Positions window at (x, y) by printing @supervisor: reposition context-menu <x> <y>.
       Draws floating menu: dark bg + border, each item is a 28px row.
       ↑↓ navigate, Enter selects → prints "@supervisor: context-reply <item>" + exits.
       Esc/Ctrl+C → exits without reply.
       Clicking outside (not possible without mouse) → Esc handles it.
       Items highlighted in C_SEL on cursor row.

## Queue (implement in order after current batch)
- [ ] P80 — Finder v2: sidebar (Favorites: Desktop/Downloads/Documents), breadcrumb path bar, icon grid view, file open on Enter
- [ ] P78 — Desktop Icons: file listing on desktop background; icons for /data files; Enter opens with appropriate app; 'n' to create new file
- [ ] P79 — Context Menu: supervisor support for @supervisor: context-menu x,y item1|item2|...; floating menu window; result sent back as REPLY:context-menu <item>
- [ ] P80 — Finder v2: sidebar (Favorites: Desktop/Downloads/Documents), breadcrumb path bar, icon grid view, double-click to open

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
- [x] P64: Clock Widget — apps/clock/ (320×360); digital HH:MM:SS large font; date line; Instant elapsed; ping-pong tick loop
- [x] P65: Weather App — apps/weather/ (760×480); reads /data/weather.toml [[day]] entries; hi/lo/humidity; forecast strip; ←→ nav; r=refresh
- [x] P66: Scientific Calculator — apps/sci-calculator/ (600×520); expression parser; sin/cos/tan/sqrt/log/ln/abs; deg/rad toggle; ANS; keyboard entry
- [x] P67: Pomodoro Timer — apps/pomodoro/ (440×380); 25/5/15min work-break cycle; Instant elapsed; progress bar; pomodoro dots; Space/n/r
- [x] P68: Log Viewer — apps/log-viewer/ (1360×760); color by severity; follow-tail mode; ping-pong reload; substring filter; ↑↓ scroll
- [x] P69: Diff Viewer — apps/diff-viewer/ (1360×760); two-step path input; LCS diff; +/- coloring; row tinting; ↑↓ scroll
- [x] P70: CSV Viewer — apps/csv-viewer/ (1360×760); quoted-field CSV parser; header row; scrollable table; ←→ col scroll
- [x] P71: JSON Viewer — apps/json-viewer/ (1360×760); pure-char JSON pretty-printer; key/string/number/bool/null colors; ↑↓ scroll ←→ files
- [x] P72: Menu Bar — apps/menu-bar/ (1440×28, y=0); clock via ping-pong; focused app name via ps-raw; wifi/vol placeholders; no chrome
- [x] P73: Dock — apps/dock/ (800×60, y=868); 9 app icons; running dot indicator; number keys 1–9 to launch/focus
- [x] P74: Spotlight — apps/spotlight/ (600×400); case-insensitive search over 31 apps; Enter launch; Esc close
- [x] P75: App Switcher — apps/app-switcher/ (1000×200); ps-raw thumbnail grid; Tab/→ cycle; Enter focus
- [x] P76: Notification Center — apps/notification-center/ (400×600); last-10 store; NOTIFY: stdin; 'c' clear; Esc close
- [x] P77: Mission Control — apps/mission-control/ (1440×900); 3-col card grid from ps-raw; ↑↓←→ nav; Enter focus

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
