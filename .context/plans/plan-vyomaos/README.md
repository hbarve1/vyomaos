# Implementation Plan — vyomaos

## Goal

Build a minimal, production-quality OS that uses a Linux kernel for hardware abstraction and Wasmtime (WASI Preview 2) as the sole application platform. Progress from the current bash-scripted prototype to a Rust-supervised, capability-secure, multi-app WASM OS. Phases 1–4 constitute the minimal working prototype: a bootable system with a real Rust PID 1 supervisor executing real WASI apps.

## Phases

| # | Phase | Status | Tasks |
|---|---|---|---|
| [01 — Build Foundation](phases/phase-01-build-foundation/README.md) | Reproducible, incremental builds via Makefile + Docker | **complete** | 3 |
| [02 — Kernel Hardening](phases/phase-02-kernel-hardening/README.md) | Minimal allnoconfig kernel with virtio + 9p + DRM | **complete** | 3 |
| [03 — Rust Supervisor](phases/phase-03-rust-supervisor/README.md) | Rust PID 1 that mounts filesystems + discovers + runs apps | **complete** | 4 |
| [04 — WASM Runtime](phases/phase-04-wasm-runtime/README.md) | Real Wasmtime static binary with WASI Preview 2 | **complete** | 3 |
| [05 — App Model](phases/phase-05-app-model/README.md) | wasm32-wasip2 apps + capability manifests + config-driven boot | **complete** | 3 |
| [06 — Multi-App & IPC](phases/phase-06-multi-app-ipc/README.md) | Concurrent apps, restart policies, supervisor IPC broker | **complete** | 4 |
| [07 — Networking & Storage](phases/phase-07-networking-storage/README.md) | 9P virtio persistent storage | **complete** | 4 |
| [08 — Observability & Security](phases/phase-08-observability-security/README.md) | seccomp BPF denylist + capability audit log | **complete** | 4 |
| [09 — GUI Display](phases/phase-09-gui-display/README.md) | DRM/virtio-gpu + fbcon + VYOMA_DRAW framebuffer protocol + gui-demo | **complete** | 5 |
| [10 — Text Rendering](phases/phase-10-text-rendering/README.md) | Embedded 8×16 bitmap font, `draw_text` VYOMA_DRAW command, labelled gui-demo dashboard | **complete** | 3 |
| [11 — Networking](phases/phase-11-networking/README.md) | virtio-net + WASI sockets (`-S inherit-network`) + `http-server` app at localhost:8080 | **complete** | 4 |
| [12 — Interactive Shell](phases/phase-12-interactive-shell/README.md) | `/dev/tty0` keyboard routing, focus manager, `@supervisor:` commands, `shell` WASM app | **complete** | 4 |
| 13 — Process Management | `ps`, `ps-raw`, `kill`, `restart`, `reload`, `log`, `logf`, `logs` via supervisor IPC | **complete** | — |
| 14 — Package Manager | `pkg-install`, `pkg-remove`, `pkg-list`, `pkg-installed`; persists via `/data/installed.txt` | **complete** | — |
| 15 — Live Dashboard | gui-demo queries `@supervisor: ps-raw` every 2s, renders 4-col app status grid | **complete** | — |
| 16 — Persistent Logs | Supervisor writes each app stdout to `/data/logs/<name>.log`; ring buffer in memory | **complete** | — |
| 17 — Real-Time Input | Raw termios mode (ICANON+ECHO off), per-keypress forwarding to focused app | **complete** | — |
| 18 — Shell UX | Arrow keys + command history (50 entries, ↑/↓ navigation) | **complete** | 5 |
| 19 — Watchdog | Silent-app detection + restart with exponential backoff; `watchdog_secs` per-app | **complete** | 7 |
| 20 — Font Scaling | 8×8/8×16/16×32 text sizes via `draw_text` size field; gui-demo large header | **complete** | 6 |
| 21 — Window Regions | Per-app window regions with offset+clip; `region` field in boot.toml; supervisor clips all draw ops to app bounds | **complete** | — |
| 22 — Mouse Input | virtio-mouse-pci; evdev reader; VYOMA_INPUT:mouse events; per-app mouse capability | **complete** | — |
| 23 — TUI Widget Primitives | `rect_border`, `clear_region`, `draw_text_wrap` VYOMA_DRAW commands; shell panel uses all three | **complete** | — |
| 24 — File Manager | Scrollable /data browser; ↑/↓ navigate; Enter view file; q quit; full-screen window | **complete** | — |
| 25 — Text Editor | Path input → full edit mode; cursor nav; insert/delete/split/merge; Ctrl+W save; Ctrl+C save+quit | **complete** | — |
| 26 — System Monitor | Polls ps-raw every 1s; table of name/status/uptime/restarts; q to quit | **complete** | — |
| 27 — App Namespaces | `unshare(CLONE_NEWNS\|CLONE_NEWPID)` in pre_exec; kernel CONFIG_NAMESPACES+PID_NS+MNT_NS | **complete** | — |
| 28 — Signed App Bundles | `wasm_sha256` in AppMeta; supervisor sha2::Sha256 verify at load; rejects on mismatch | **complete** | — |
| 29 — Multi-Resolution Display | `display::screen_size()` via FBIOGET_VSCREENINFO; `VYOMA_SYSTEM:screen:<w>,<h>` sent to display apps at launch | **complete** | — |
| 30 — OTA Hot-Swap | `@supervisor: update <app> <url>`; raw TCP HTTP GET; SHA-256 verify; atomic replace; restart | **complete** | — |
| 31 — Double-Buffered Compositor | `Framebuffer.back: Vec<u8>`; all draw ops → back-buffer; `flush()`/`present` blit back→mmap; no tearing | **complete** | — |
| 32 — Window Decorations | Title bar + close button chrome painted at flush/present on top of app content; `VYOMA_SYSTEM:window_event:close` on click | **complete** | — |
| 33 — Z-Ordering | `Z_ORDER` global stack; click-to-raise sets focus; `@supervisor: raise/lower`; shell `raise`/`lower` commands | **complete** | — |
| 34 — Window Manager App | `apps/window-manager/`; queries list on start; raises in sorted order; focuses top app | **complete** | — |
| 35 — Desktop Wallpaper | Default `0x0D1117FF` at startup; `@supervisor: wallpaper <rgba>`; shell `wallpaper` command | **complete** | — |
| 36 — Window Resize Events | `@supervisor: resize <app> <w> <h>`; updates win_region; `VYOMA_SYSTEM:resize:<w>,<h>` to app | **complete** | — |
| 37 — Taskbar App | `apps/taskbar/` dock at y=860; ps-raw poll 2s; app buttons + clock; click-to-focus | **complete** | — |
| 38 — App Launcher | `apps/app-launcher/` full-screen overlay; pkg-list query; search filter; 4-col grid; Enter launches; Ctrl+C exits | **complete** | — |
| 39 — Notifications | `@supervisor: notify <title> <msg>`; toast at (1020,10,400×60); 0x21262DFF bg; auto-clear 3s; shell `notify` command | **complete** | — |
| 40 — Settings App | `apps/settings/` sidebar+content; Display/Font/Boot sections; reads+writes `/data/settings.toml`; Tab/Ctrl+W/Ctrl+C | **complete** | — |
| 41 — Power Manager | `@supervisor: shutdown`/`reboot`; `libc::reboot` with POWER_OFF/RESTART; 500ms delay; shell `shutdown`/`reboot` | **complete** | — |
| 42 — Session Manager | `@supervisor: session-save/restore`; writes/reads `/data/session.toml` [[window]] TOML; shell commands | **complete** | — |
| 43 — Multi-Monitor | `@supervisor: monitors`; counts `/sys/class/drm/card0-*`; fallback 1; shell `monitors` | **complete** | — |
| 44 — DNS Resolver | `apps/dns-resolver/` TUI (840×500); supervisor TCP DNS to 8.8.8.8:53; A-record parse; `dns-resolve`/`REPLY:dns`; shell `dns` | **complete** | — |
| 45 — HTTPS/TLS | `@supervisor: tls-info`; http-server /tls endpoint + cert check at startup; shell `tls-info` | **complete** | — |
| 46 — Basic Browser | `apps/browser/` URL bar + HTML strip + scroll; `@supervisor: http-get` fetches+truncates body; Ctrl+L/Up/Down | **complete** | — |
| 47 — SSH Client / TCP Tunnel | `apps/ssh-client/` form→terminal; `@supervisor: tcp-connect/send/recv/close`; TCP_CONNS pool; AtomicU32 IDs | **complete** | — |
| 48 — Network Config UI | `apps/network-config/` DHCP/Static toggle; 5 IP fields; reads+writes `/data/network.toml`; Tab/d/s/Ctrl+W/Ctrl+C | **complete** | — |
| 49 — Download Manager | `@supervisor: download <url> <dest>`; background thread; progress/done/error replies; shell `download` command | **complete** | — |
| 50 — Clipboard Manager | `static CLIPBOARD: OnceLock<Mutex<String>>`; `@supervisor: clipboard-set/get`; shell `clip-set`/`clip-get` | **complete** | — |
| 51 — Screenshot | `@supervisor: screenshot <path>`; reads back-buffer; writes P6 PPM (BGRA→RGB); shell `screenshot [path]` | **complete** | — |
| 52 — Virtual Keyboard | `apps/virtual-keyboard/` QWERTY on-screen; mouse click → `@supervisor: input <char>`; supervisor routes char to focused app; Ctrl+C quits | **complete** | — |
| 53 — Color Picker | `apps/color-picker/` 64×64 HSV gradient + value slider; mouse click; Ctrl+W outputs `color: #RRGGBBFF` | **complete** | — |
| 54 — Process Inspector | `apps/process-inspector/` table+detail view; ps-raw poll 2s; Enter → detail; `@supervisor: win-info <app>`; q/Ctrl+C back/quit | **complete** | — |
| 55 — Font Chooser | `apps/font-chooser/` 3-option list S/M/L; preview in each size; Enter → `@supervisor: font-size <s\|m\|l>`; `static FONT_SIZE` global | **complete** | — |
| 56 — App Store UI | `apps/app-store/` full-screen overlay; pkg-list search+grid; Enter install/remove; re-queries after action; search filter | **complete** | — |
| 57 — Audio Player Stub | `apps/audio-player/` track list from /data/*.raw; play/pause/prev/next; cosmetic progress bar; Ctrl+C quit | **complete** | — |
| 58 — Image Viewer | `apps/image-viewer/` PPM P6 loader; 4×4 block RLE rendering; ↑↓ scroll ←→ prev/next; list+image views | **complete** | — |
| 59 — Hex Editor | `apps/hex-editor/` path input → hex+ASCII view; 16 bytes/row; edit mode (2 hex digits); u/d page nav; Ctrl+W save | **complete** | — |
| 60 — Calendar Widget | `apps/calendar/` month view 7-col grid; marks today; ←→↑↓ navigate months; weekend color highlight | **complete** | — |
| 61 — Markdown Viewer | `apps/markdown-viewer/` lists /data/*.md; renders H1/H2/H3/bold/code/bullets; ↑↓ scroll ←→ prev/next | **complete** | — |
| 62 — Password Manager | `apps/password-manager/` XOR-encrypted vault; master password unlock; add/delete entries; copy password to clipboard | **complete** | — |
| 63 — Task Manager | `apps/task-manager/` TOML task list; add/delete/toggle done; ↑↓ nav; Ctrl+W save | **complete** | — |
| 64 — Clock Widget | `apps/clock/` digital HH:MM:SS (large font); date line; Instant-based elapsed time; ping-pong tick loop | **complete** | — |
| 65 — Weather App | `apps/weather/` reads /data/weather.toml [[day]] entries; condition/hi-lo/humidity display; forecast strip; ←→ nav; r=refresh | **complete** | — |
| 66 — Scientific Calculator | `apps/sci-calculator/` expression evaluator; sin/cos/tan/sqrt/log/ln/abs; deg/rad toggle; ANS var; keyboard entry | **complete** | — |
| 67 — Pomodoro Timer | `apps/pomodoro/` 25/5/15min work-break cycle; Instant-based elapsed; progress bar; pomodoro dot counter; Space/n/r keys | **complete** | — |
| 68 — Log Viewer | `apps/log-viewer/` lists /data/*.log; color by severity; follow-tail mode; substring filter; ping-pong reload | **complete** | — |
| 69 — Diff Viewer | `apps/diff-viewer/` two-step path input; LCS diff algorithm; +/- line coloring; row background tinting; ↑↓ scroll | **complete** | — |
| 70 — CSV Viewer | `apps/csv-viewer/` quoted-field CSV parser; fixed-width column table; header row highlighted; ←→ col scroll; ↑↓ row scroll | **complete** | — |
| 71 — JSON Viewer | `apps/json-viewer/` pure-char JSON pretty-printer; key/string/number/bool/null syntax colors; ↑↓ scroll; ←→ switch files | **complete** | — |
| 72 — Menu Bar | `apps/menu-bar/` top bar (y=0, h=28, no chrome); clock (ping-pong); focused app name (ps-raw poll); wifi/vol placeholders | **complete** | — |
| 73 — Dock | `apps/dock/` bottom dock (y=868, h=60); 9 app icons with color; running dot indicator (ps-raw); number keys 1–9 to launch | **complete** | — |
| 74 — Spotlight | `apps/spotlight/` Cmd+Space overlay (600×400); case-insensitive search; 31 apps; Enter to launch; Esc to close | **complete** | — |
| 75 — App Switcher | `apps/app-switcher/` Alt+Tab overlay (1000×200); thumbnail grid from ps-raw; Tab/→ cycle; Enter to focus | **complete** | — |
| 76 — Notification Center | `apps/notification-center/` right-panel (400×600); stores last 10 notifications; NOTIFY: lines via stdin; 'c' clear; Esc close | **complete** | — |
| 77 — Mission Control | `apps/mission-control/` full-screen overlay (1440×900); 3-col card grid of running apps from ps-raw; ↑↓←→ nav; Enter focus | **complete** | — |
| 78 — Desktop Icons | `apps/desktop/` desktop layer (1440×832, y=28); fixed folder icons + /data file icons; ↑↓←→ nav; Enter opens; n=new file | **complete** | — |
| 79 — Context Menu | `apps/context-menu/` floating menu (220px wide); MENU:x,y:items stdin; ↑↓ nav; Enter selects → context-reply; Esc closes | **complete** | — |
| 80 — Finder v2 | `apps/finder/` sidebar+grid (1280×760); 6 sidebar favorites; ls-data integration; file icon grid; s=toggle focus; Enter open | **complete** | — |
| 81 — System Preferences | `apps/system-preferences/` sidebar+pane (900×700); Appearance/Display/Sound/Network/Security/About panes; Ctrl+W saves | **complete** | — |
| 82 — Activity Monitor | `apps/activity-monitor/` process table (1200×700); ps-raw poll; CPU/mem bar charts; n/c/m sort; ↑↓ nav; q quit | **complete** | — |
| 83 — Quick Look | `apps/quick-look/` file preview overlay (1040×680); PREVIEW: stdin; md/json/csv/log/text rendering; ↑↓ scroll; Space/Esc close | **complete** | — |
| 84 — Spaces Switcher | `apps/spaces-switcher/` virtual desktop switcher (640×140); @supervisor: spaces-list/switch/create; ←→ nav; Enter switch; n new | **complete** | — |
| 85 — Screen Lock | `apps/screen-lock/` full-screen lock overlay (1440×900); large clock; padlock icon; any key unlocks | **complete** | — |
| 86 — Clipboard History | `apps/clipboard-history/` last-20 clipboard entries (420×560); Enter restore; c paste; Del remove | **complete** | — |
| 87 — Widget Board | `apps/widget-board/` dashboard panel (400×500); clock/stats/quick-launch/notes widgets; ↑↓ nav; Enter launch | **complete** | — |
| 88 — Terminal Multiplexer | `apps/tmux/` split-pane terminal (1440×872); 2-4 panes; Tab cycle; Ctrl+N new; Ctrl+W close; simulated shell | **complete** | — |
| 89 — Font Preview | `apps/font-preview/` font size debug tool (1040×680); renders all 95 printable ASCII chars in s/m/l sizes; ↑↓ scroll | **complete** | — |
| 90 — Draw Pad | `apps/draw-pad/` pixel canvas (960×760); 200×150 logical grid; arrow+Space draw; 10-color palette (0-9); Ctrl+W export | **complete** | — |
| 91 — World Clock | `apps/world-clock/` 6-zone world clock (960×500); 3×2 card grid; UTC/Eastern/Pacific/London/Tokyo/Kolkata; ping-pong updates | **complete** | — |
| 92 — Stopwatch | `apps/stopwatch/` digital stopwatch (640×480); HH:MM:SS.cc; Space start/stop; r reset; l lap; last-5 laps with delta | **complete** | — |
| 93 — Unit Converter | `apps/unit-converter/` 7-category converter (760×560); Length/Mass/Temp/Speed/Area/Volume/Time; Tab switch; ←→ category | **complete** | — |
| 94 — QR Code Viewer | `apps/qr-viewer/` QR code generator (760×680); Version 1 21×21; type text+Enter; c copy; simplified bit layout | **complete** | — |
| 95 — Emoji Picker | `apps/emoji-picker/` emoji grid (760×560); 6 categories × 15 emoji; search filter; Tab category; Enter copies | **complete** | — |
| 96 — Snake Game | `apps/snake/` classic snake game (800×680); 18px cells; arrow keys; ping-pong ticks; score+high score | **complete** | — |
| 97 — Minesweeper | `apps/minesweeper/` 16×16 grid; 96 mines; first-click safe; BFS flood-fill; F flag; number colors; Win/Loss overlay | **complete** | — |
| 98 — 15 Puzzle | `apps/fifteen-puzzle/` 4×4 sliding tiles; 200-step shuffle; arrow keys move blank; correct-tile highlight; Solved overlay | **complete** | — |
| 99 — Breakout | `apps/breakout/` classic Breakout; 5×10 bricks; angle-adjust paddle; ping-pong physics; lives; score; Win/Loss | **complete** | — |
| 100 — Memory Card Game | `apps/memory-game/` 4×4 face-down card grid; 8 pairs; flip/match logic; move counter; Solved overlay | **complete** | — |
| 101 — Tetris | `apps/tetris/` 7 tetrominoes; 4 rotations; wall kicks; ghost piece; hard drop; line clears; levels; next-piece preview | **complete** | — |
| 102 — Pong | `apps/pong/` player vs AI; angle-adjust paddle hit; AI lag tracking; score to 7; Win overlay | **complete** | — |
| 103 — Space Invaders | `apps/space-invaders/` 3×10 alien grid; march+drop; player bullet+3 alien bullets; ping-pong loop; Win/Loss overlay | **complete** | — |
| 104 — 2048 | `apps/2048/` 4×4 grid; slide+merge; LCG spawn; score+best; non-blocking Win; Game Over detection | **complete** | — |
| 105 — Wordle | `apps/wordle/` 60-word list; 6-guess rows; per-cell green/yellow/absent scoring; keyboard color tracker; Win/Loss overlay | **complete** | — |
| 106 — Sudoku | `apps/sudoku/` hardcoded puzzle+solution; cursor nav; digit entry; conflict highlighting; check/reset/solve modes | **complete** | — |
| 107 — Chess | `apps/chess/` standard chess board; full piece movement rules; pawn promotion; cursor+Enter select/move; Win on king capture | **complete** | — |
| 108 — Typing Tutor | `apps/typing-tutor/` 40-phrase list; per-char green/red feedback; WPM + accuracy; progress bar; ping-pong tick | **complete** | — |
| 109 — Paint | `apps/paint/` 160×120 pixel canvas (5×5 cells); 10-color palette; arrow-key cursor; Space=draw; e=erase; c=clear; Tab=color | **complete** | — |
| 110 — Music Visualizer | `apps/music-viz/` 32 animated bars; integer sin approximation; hue-spectrum colors; ping-pong driven; speed/pause/randomize | **complete** | — |
| 111 — Flashcard | `apps/flashcard/` 20 CS/programming Q&A cards; flip/nav; y/n scoring; shuffle via LCG; progress bar | **complete** | — |
| 112 — Budget Tracker | `apps/budget/` 20-entry ledger; income/expense; 8 categories; add form; delete; balance summary | **complete** | — |
| 113 — Habit Tracker | `apps/habit-tracker/` 10 habits; 7-day week grid; toggle today; streak counter; next-day shift; reset | **complete** | — |
| 114 — Recipe Browser | `apps/recipe/` 10 built-in recipes; two-panel list+detail; ingredients+steps; search filter | **complete** | — |
| 115 — Expense Split | `apps/expense-split/` 6 people; 20 expenses; greedy settlement; 3-tab view; add/delete forms | **complete** | — |
| 116 — Word Counter | `apps/word-counter/` multi-line text editor; words/chars/lines/sentences/paragraphs; avg metrics; stats panel | **complete** | — |
| 117 — Countdown Timer | `apps/countdown/` digit-input HHMMSS; ping-pong tick; progress bar; flash-done; pause/resume | **complete** | — |
| 118 — Quiz Game | `apps/quiz/` 30 trivia questions; 5 categories; A/B/C/D select; confirm; green/red reveal; score screen | **complete** | — |
| 119 — Dice Roller | `apps/dice/` d4/d6/d8/d10/d12/d20; 1-8 count; LCG roll; history log; critical hit flag | **complete** | — |
| 120 — Color Palette Generator | `apps/color-gen/` hue input; 5 harmony modes; HSV→RGB swatches; hex labels; clipboard copy | **complete** | — |
| 121 — Maze Generator | `apps/maze/` 25×25 recursive-backtracker; player nav; trail; R=reset; N=new maze | **complete** | — |
| 122 — Pixel Art Editor | `apps/pixel-art/` 32×32 canvas; 16-color palette; draw/erase/fill/undo; Tab=next color | **complete** | — |
| 123 — Simon Says | `apps/simon/` 4-color sequence memory; ping-pong animation; growing pattern; R/G/B/Y keys | **complete** | — |
| 124 — Hangman | `apps/hangman/` 40-word list; 6 wrong guesses; gallows drawing; a-z keys; hint key | **complete** | — |
| 125 — Typing Race | `apps/typing-race/` 10 phrases; 3 CPU racers (40/60/80 WPM); progress bars; ping-pong | **complete** | — |
| 126 — Asteroids | `apps/asteroids/` rocks+bullets+ship; fixed-point physics; wrap-around; split on hit | **complete** | — |
| 127 — Math Quiz | `apps/math-quiz/` 4 difficulties; +/-/×/÷; timer bar; streak; ping-pong tick | **complete** | — |
| 128 — Paint Pro | `apps/paint-pro/` 200×150 canvas; 3 brush sizes; flood-fill; undo; save/load /data | **complete** | — |
| 129 — Music Player | `apps/music-player/` reads /data/*.raw; waveform bars; ping-pong; vol +/-; next/prev | **complete** | — |
| 130 — Code Editor | `apps/code-editor/` multi-line; Rust syntax highlight; line numbers; save/load; scrollable | **complete** | — |
| 131 — Crypto Ticker | `apps/crypto-ticker/` 5 coins; LCG random walk; sparklines; alert >5%; sort by % | **complete** | — |
| 132 — Photo Filter | `apps/photo-filter/` PPM loader; 5 filters; 4×4 mosaic preview; save filtered.ppm | **complete** | — |
| 133 — Terminal Emulator | `apps/terminal/` Line-based shell; scrollback 200 lines; Tab complete; Up/Dn history; PgUp/Dn scroll; built-in cmds; ping-pong | **complete** | — |
| 134 — Map Viewer | `apps/map-viewer/` 80×40 procedural ASCII world map; 4 zoom levels; pan; 12 landmarks; legend; grid at ≥16px | **complete** | — |
| 135 — File Diff Tool | `apps/file-diff/` Two-step path input; LCS unified diff; +/- coloring; row tint; line numbers; ↑↓ scroll | **complete** | — |
| 136 — Spreadsheet | `apps/spreadsheet/` 10×20 grid; =SUM/AVG formula eval; cell refs; arrow nav; edit mode; CSV save/load | **complete** | — |
| 137 — Image Gallery | `apps/image-gallery/` PPM thumbnail grid (4 cols); 3×3 px blocks; ←→↑↓ nav; Enter fullscreen; prev/next; Esc back | **complete** | — |
| 138 — Text Adventure | `apps/text-adventure/` 10-room castle; take/drop/use/go commands; monster+key+sword puzzle; scrollback | **complete** | — |
| 139 — Music Composer | `apps/music-composer/` 12-note × 16-step piano roll; toggle cells; ping-pong playback; BPM adj; save/load | **complete** | — |
| 140 — Chat Simulator | `apps/chat/` 5 bots (LCG responses); @You mentions; scrollback 200; ping-pong ticks; color usernames | **complete** | — |
| 141 — Morse Code Trainer | `apps/morse/` Encode/Decode/Quiz modes; Tab switch; visual ●/━ display; score tracking; 26 letters + 10 digits | **complete** | — |
| 142 — ASCII Art Editor | `apps/ascii-art/` 60×25 canvas; 12×20 chars; brush palette; draw mode; type-in-place; C clear; save/load | **complete** | — |
| 143 — Stock Chart | `apps/stock-chart/` 5 fake stocks; LCG candlestick data; OHLC chart; volume bars; ping-pong ticks; 1-5 keys | **complete** | — |
| 144 — Code Runner | `apps/code-runner/` 10 Rust snippets; syntax highlight; 3-panel (list/code/output); Enter=run; PgUp/Dn scroll | **complete** | — |
| 145 — Genealogy Tree | `apps/genealogy/` 20 people × 4 gens; Manhattan-routed parent lines; ←→ siblings; ↑↓ parent/child | **complete** | — |
| 146 — Mind Map | `apps/mind-map/` VyomaOS root + 6 branches + 18 leaves; Bresenham lines; Tab/↑↓/R nav; color by branch | **complete** | — |
| 147 — Network Monitor | `apps/net-monitor/` LCG packet simulation; 60-sample throughput graph; 20-packet scrolling list; 5-proto filter; ping-pong ticks | **complete** | — |
| 148 — Kanban Board | `apps/kanban/` 3-column board (Todo/Doing/Done); card CRUD; priority badge; CSV save/load; column scroll | **complete** | — |
| 149 — Presentation Viewer | `apps/presentation/` 10 slides; title+bullets; ←/→ navigate; progress bar; fullscreen toggle; F key | **complete** | — |
| 150 — Note Taking App | `apps/notes/` sidebar list + editor; multi-note; Ctrl+N/D/W/L/F; search filter; CSV save/load | **complete** | — |
| 151 — Markdown Editor | `apps/md-editor/` split-pane editor+preview; H1/H2/H3/bold/code/bullet render; Ctrl+O open; Ctrl+W save | **complete** | — |
| 152 — Terminal Emulator v2 | `apps/term2/` 80+ col terminal; help/ls/cat/echo/clear/date/uname/history; scrollback 500; cursor blink | **complete** | — |

See [full OS roadmap](../../docs/superpowers/specs/2026-05-20-vyomaos-full-os-roadmap.md) for P31–P112 (window manager → desktop shell → networking → filesystem → app ecosystem → dev tools → media → accessibility → security → performance → real hardware → production release).

## Constraints

- Kernel: Linux 5.10.x LTS, `allnoconfig` base (not tinyconfig — tinyconfig silently drops forced deps), no loadable modules, static build
- Runtime: Wasmtime 43.0.0 **glibc** variant (not musl — musl build unavailable); glibc runtime bundled in initramfs
- Supervisor: Rust, static musl binary, replaces BusyBox shell as PID 1
- Apps: `wasm32-wasip2` target (WASI Preview 2), zero glibc dependencies
- Build: Reproducible via Docker; incremental via Makefile dependency tracking
- Boot time target: < 5 seconds in QEMU on standard laptop hardware
- Initramfs size target: < 25 MB (Wasmtime 43 glibc variant dominates at 61M stripped)

## References

- [Project README](../../README.md)
- [Base build system](../../base/README.md)
- [Apps README](../../apps/README.md)
- [Wasmtime WASI docs](../../.tessl/tiles/tessl/pypi-wasmtime/docs/wasi.md)
