# VyomaOS Auto-Build State
<!-- Owned by the autonomous loop. Each iteration reads this, does work, updates it. -->

last_updated: 2026-05-23  <!-- P189+P190 complete -->
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
  - P191 — Periodic Table
  - P192 — Roman Numerals
notes: |
  P191: Periodic Table. Create apps/periodic-table/ WASM app.
        Window w=1100, h=720. Capabilities: stdio=true, display=true.
        118 elements in standard grid layout (18 cols × 7 rows + lanthanides/actinides).
        Category colors: alkali=red, alkaline=orange, transition=blue, metalloid=yellow,
        nonmetal=green, noble=purple, lanthanide/actinide=teal.
        ←→↑↓ to select element; info panel shows atomic number, mass, electron config.
        Q=quit.

  P192: Roman Numerals. Create apps/roman/ WASM app.
        Window w=800, h=600. Capabilities: stdio=true, display=true.
        Two modes: Converter (type arabic → shows roman, type roman → shows arabic)
        and Quiz (random 1-3999, enter answer, score tracking).
        Tab=toggle mode, history of last 10 conversions, large font display.
        Q=quit.

## Queue (implement in order after current batch)
- [ ] P193 — Spirograph: apps/spirograph/ parametric curves; R/r/d sliders; animated draw; color cycle
- [ ] P194 — Anagram Solver: apps/anagram/ dictionary 1000 words; find all anagrams; timed challenge

## Completed (recent — full list in plan README)
- [x] P189 — ASCII Art Gallery: apps/ascii-art/ 10 images; 8 density charsets; invert; ←→/D/I/Q
- [x] P190 — Fibonacci Visualizer: apps/fib-viz/ Bar/Table/Spiral; φ convergence; zoom; ping-pong
- [x] P187 — Morse Code Trainer: apps/morse/ Practice+Decode; 36 symbols; score+streak; full table sidebar
- [x] P188 — Binary Clock: apps/binary-clock/ HH:MM:SS binary grid; 5 themes; bit labels; ping-pong
- [x] P185 — Pixel Clock: apps/pixel-clock/ 5×7 LED font; HH:MM:SS; 5 themes; blink colons; sim date
- [x] P186 — Typing Practice: apps/typing-practice/ 20 passages; per-char coloring; WPM; PB tracking
- [x] P183 — Habit Streak Calendar: apps/habit-streak/ 10 habits; GitHub heat map 52×7; streak/rate; Tab list
- [x] P184 — Password Generator: apps/pass-gen/ LCG; U/L/D/S charset; entropy bar; history 10; copy
- [x] P181 — 3D Cube Viewer: apps/cube3d/ integer fixed-point; sin/cos table; Bresenham; depth color; ping-pong
- [x] P182 — Recipe Generator: apps/recipe-gen/ 40 ingredients; LCG; 7 steps; history 10; category tags
- [x] P179 — Weather Dashboard: apps/weather-dash/ LCG 7-day forecast; hourly bars; animated icons; city selector
- [x] P180 — Code Diff Tool: apps/code-diff/ LCS diff; side-by-side; green/red/yellow tint; 5 pairs; Tab/scroll
- [x] P177 — Chess Puzzles: apps/chess-puzzles/ 20 positions; mate-in-1/2; arrow+Enter input; solution checker
- [x] P178 — Music Theory: apps/music-theory/ 3 tabs (scales/chords/quiz); note wheel; interval quiz; streak
- [x] P175 — Alarm Clock: apps/alarm/ 7-segment digital clock; 5 alarms; ping-pong tick; flash+notify on fire
- [x] P176 — Geo Quiz: apps/geo-quiz/ 60 questions; 4 MCQ; 15-tick timer; hint (-5pts); streak; end screen
- [x] P173 — Recipe Planner: apps/recipe-planner/ 60 recipes; 7×3 week grid; shopping list; R=randomize
- [x] P174 — Syntax Highlighter: apps/syntax-demo/ Rust/Python/JSON; split pane; live edit; colors
- [x] P171 — Star Map: apps/star-map/ 62 named stars; 12 constellations; spectral colors; pan/zoom
- [x] P172 — Pixel Font Editor: apps/font-editor/ 8×16 glyph editor; 95 ASCII; preview 1×/2×/4×; hex
- [x] P169 — Budget Planner v2: apps/budget2/ 12 months; 8 categories; bar chart; savings goal; +/-
- [x] P170 — Code Snippet Manager: apps/snippets/ 20 snippets; Rust/Python/Shell; search; copy
- [x] P167 — Timeline Viewer: apps/timeline/ 39 events 1440–2026; 5 categories; ←→ scroll; +/- zoom; info panel
- [x] P168 — Language Flashcards: apps/lang-flash/ 30 vocab; EN/ES/FR/DE; Y/N scoring; spaced rep; streak
- [x] P165 — Clipboard Pro: apps/clipboard-pro/ 50-entry history; categories; search; pin; preview panel
- [x] P166 — System Info: apps/system-info/ 7 sections; ASCII logo; build box; scrollable
- [x] P163 — Pomodoro Pro: apps/pomodoro-pro/ work/break/long; task list; stats; progress ring; ±interval
- [x] P164 — Astronomy Viewer: apps/astronomy/ 200 stars; 5 planets; 8 constellations; pan/zoom; info panel
- [x] P161 — E-Book Reader: apps/ebook/ 5 chapters; TOC panel; scroll; B/G bookmark; ←→ chapter
- [x] P162 — Network Speed Test: apps/speed-test/ ping/dl/ul phases; LCG; bar graphs; grade card
- [x] P159 — RSS Reader: apps/rss-reader/ 3 feeds × 10 articles; two-panel; Tab/↑↓/Enter/R
- [x] P160 — Video Player: apps/video-player/ LCG noise frames 40×30 @6px; 3 videos; Space/N/P/R/+/-
- [x] P157 — IRC Client: apps/irc/ 3 channels; LCG bots; /join /msg /quit; Tab switch; nick list
- [x] P158 — Photo Editor: apps/photo-editor/ PPM load; crop/resize/rotate90; brightness/contrast; save
- [x] P155 — File Archiver: apps/archiver/ custom .tar; two-panel; N/A/X/Del/Ctrl+W/Ctrl+L; demo.tar
- [x] P156 — System Logger: apps/syslog/ ring-buf 200; 7 tags; 5 levels; F1-F5 filter; P/R/C/E scroll
- [x] P153 — Spreadsheet v2: apps/spreadsheet2/ 20×15; =SUM/AVG/MIN/MAX/COUNT; arithmetic; circ detect
- [x] P154 — Drawing App v2: apps/draw2/ 300×200 canvas; brush/eraser/fill/line; undo; PPM save
- [x] P151 — Markdown Editor: apps/md-editor/ split-pane editor+preview; H1/H2/H3/bold/code; Ctrl+O/W/L
- [x] P152 — Terminal Emulator v2: apps/term2/ shell commands; scrollback 500; cursor blink; history
- [x] P149 — Presentation Viewer: apps/presentation/ 10 slides; bullets; progress bar; fullscreen
- [x] P150 — Note Taking App: apps/notes/ sidebar+editor; search; CSV save/load; Ctrl+N/D/W/L/F
- [x] P147 — Network Monitor: apps/net-monitor/ LCG packets; throughput graph; proto filter; ping-pong
- [x] P148 — Kanban Board: apps/kanban/ 3-col board; card CRUD; priority; CSV save/load
- [x] P145 — Genealogy Tree: apps/genealogy/ 20 people 4 gens; Manhattan lines; ←→ siblings; ↑↓ parent/child
- [x] P146 — Mind Map: apps/mind-map/ VyomaOS root + 6 branches + 18 leaves; Bresenham lines; Tab/nav
- [x] P143 — Stock Chart: apps/stock-chart/ 5 stocks; LCG OHLC candles; volume bars; ping-pong; 1-5 select
- [x] P144 — Code Runner: apps/code-runner/ 10 snippets; syntax highlight; 3-panel; Enter=run; scroll

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
- [x] P78: Desktop Icons — apps/desktop/ (1440×832, y=28); fixed folders + /data files; ↑↓←→; Enter opens; n=new file
- [x] P79: Context Menu — apps/context-menu/ (220px); MENU: stdin spec; ↑↓; Enter selects → context-reply
- [x] P80: Finder v2 — apps/finder/ (1280×760); sidebar+grid; 6 favorites; ls-data; file type → app routing; s=focus toggle
- [x] P81: System Preferences — apps/system-preferences/ (900×700); 6 panes; appearance/display/sound/network/security/about
- [x] P82: Activity Monitor — apps/activity-monitor/ (1200×700); ps-raw poll; CPU/mem bars; n/c/m sort; scroll
- [x] P83: Quick Look — apps/quick-look/ (1040×680); PREVIEW: stdin; md/json/csv/log/text render; ↑↓ scroll
- [x] P84: Spaces Switcher — apps/spaces-switcher/ (640×140); spaces-list/switch/create; ←→ nav; Enter switch
- [x] P85: Screen Lock — apps/screen-lock/ (1440×900); large clock + padlock icon; any key unlocks
- [x] P86: Clipboard History — apps/clipboard-history/ (420×560); last-20 entries; Enter restore; c paste; Del remove
- [x] P87: Widget Board — apps/widget-board/ (400×500); clock/stats/quick-launch/notes; ↑↓ nav; Enter to launch
- [x] P88: Terminal Multiplexer — apps/tmux/ (1440×872); 2-4 split panes; Tab/Ctrl+N/Ctrl+W; simulated shell
- [x] P89: Font Preview — apps/font-preview/ (1040×680); all 95 printable ASCII in s/m/l sizes; ↑↓ scroll
- [x] P90: Draw Pad — apps/draw-pad/ (960×760); 200×150 px canvas; 4px cells; 10-color palette; Space=draw; e=erase
- [x] P91: World Clock — apps/world-clock/ (960×500); 3×2 grid; 6 zones; ping-pong 1s update
- [x] P92: Stopwatch — apps/stopwatch/ (640×480); HH:MM:SS.cc; Space/r/l; last-5 laps with delta
- [x] P93: Unit Converter — apps/unit-converter/ (760×560); 7 categories; ratio factors + temp formula; Tab side; ←→ category
- [x] P94: QR Code Viewer — apps/qr-viewer/ (760×680); Version 1 21×21 QR; bit encoding; type+Enter; 'c' copies
- [x] P95: Emoji Picker — apps/emoji-picker/ (760×560); 6 categories × 15 emoji; search filter; Tab cat; Enter copies
- [x] P96: Snake — apps/snake/ (800×680); VecDeque snake; ping-pong ticks; wall/self collision; score+high score
- [x] P97: Minesweeper — apps/minesweeper/ (760×680); 16×16 grid; 96 mines; BFS flood-fill; first-click safe; F flag; number colors 1-8; Win/Loss overlay
- [x] P98: 15 Puzzle — apps/fifteen-puzzle/ (640×640); 4×4 tiles; 200-step shuffle; arrow keys slide blank; adjacent-tile highlight; "Solved! N moves" overlay
- [x] P99: Breakout — apps/breakout/ (800×800); 5×10 bricks; angle-adjust paddle; ping-pong physics; 3 lives; score; Win/Loss overlay
- [x] P100: Memory Card Game — apps/memory-game/ (760×680); 4×4 grid, 8 pairs; flip/match; face-up anti-cheat reset; Solved overlay; R reshuffle
- [x] P101: Tetris — apps/tetris/ (560×860); 7 tetrominoes × 4 rotations; LCG shuffle; wall kicks; ghost outline; hard drop; line-clear scoring; level speed-up
- [x] P102: Pong — apps/pong/ (800×700); player vs AI; angle-adjust on paddle hit; AI 4px/tick lag; score-to-7; Win/Loss overlay
- [x] P103: Space Invaders — apps/space-invaders/ (800×820); 3×10 alien grid; march+drop; 1 player bullet + 3 alien bullets; ping-pong tick; Win/Loss overlay
- [x] P104: 2048 — apps/2048/ (600×700); 4×4 grid; slide+merge; LCG spawn; score+best tracking; non-blocking Win; Game Over detection
- [x] P105: Wordle — apps/wordle/ (520×700); 60-word list; 6 guesses; per-cell green/yellow/absent; keyboard color tracker; Win/Loss overlay
- [x] P106: Sudoku — apps/sudoku/ (640×740); hardcoded puzzle+solution; cursor nav; digit entry; conflict detection; check/reset/solve
- [x] P107: Chess — apps/chess/ (680×720); standard board; full piece movement rules; pawn promotion; Win on king capture; rank/file labels
- [x] P108: Typing Tutor — apps/typing-tutor/ (880×560); 40-phrase list; per-char green/red; WPM+accuracy; progress bar; ping-pong tick
- [x] P109: Paint — apps/paint/ (960×760); 160×120 canvas; 5×5 cells; 10-color palette; arrow+Space draw; erase; clear; Tab color cycle
- [x] P110: Music Visualizer — apps/music-viz/ (800×560); 32 bars; integer sin approx; hue spectrum; ping-pong; speed/pause/randomize
- [x] P111: Flashcard — apps/flashcard/ (800×600); 20 CS Q&A cards; flip; y/n scoring; LCG shuffle; progress bar
- [x] P112: Budget Tracker — apps/budget/ (840×680); income/expense ledger; 8 categories; add form; balance summary
- [x] P113: Habit Tracker — apps/habit-tracker/ (840×640); 10 habits; 7-day grid; streak; next-day shift
- [x] P114: Recipe Browser — apps/recipe/ (920×760); 10 recipes; two-panel list+detail; ingredients+steps; search
- [x] P115: Expense Split — apps/expense-split/ (800×680); 6 people; 20 expenses; greedy settlement; 3-tab view
- [x] P116: Word Counter — apps/word-counter/ (880×640); multi-line editor; word/char/line/sentence/para counts; stats panel
- [x] P117: Countdown Timer — apps/countdown/ (720×560); digit-input HHMMSS; ping-pong tick; progress bar; flash-done; pause/resume
- [x] P118: Quiz Game — apps/quiz/ (920×680); 30 trivia Qs; 5 categories; A/B/C/D; LCG shuffle; score screen
- [x] P119: Dice Roller — apps/dice/ (760×620); d4/d6/d8/d10/d12/d20; 1-8 count; LCG; history log; critical flag
- [x] P120: Color Palette Generator — apps/color-gen/ (880×680); hue+/-; 5 harmony modes; HSV→RGB; clipboard copy
- [x] P121: Maze Generator — apps/maze/ (840×760); 25×25 iterative DFS; trail tracking; arrows nav; R=reset; N=new
- [x] P122: Pixel Art Editor — apps/pixel-art/ (1000×760); 32×32 canvas; 16-color palette; draw/erase/fill/undo(10)
- [x] P123: Simon Says — apps/simon/ (800×700); 4-quadrant colors; ping-pong flash animation; growing sequence; R/G/B/Y
- [x] P124: Hangman — apps/hangman/ (880×700); 40-word list; 6 wrongs; gallows rects; a-z keys; hint; win/loss overlay
- [x] P125: Typing Race — apps/typing-race/ (1040×640); 10 phrases; 3 CPU racers; ping-pong advance; WPM; win/loss overlay
- [x] P126: Asteroids — apps/asteroids/ (900×800); fixed-point physics; rocks split; bullets; 3 lives; wrap; waves
- [x] P127: Math Quiz — apps/math-quiz/ (840×660); 4 difficulties; +/-/×/÷ LCG questions; timer bar; streak/best; flash feedback
- [x] P128: Paint Pro — apps/paint-pro/ (1280×800); 200×150 canvas; 3 brush sizes; BFS fill; undo(10); save/load /data
- [x] P129: Music Player — apps/music-player/ (800×560); reads /data/*.raw; 32-bar waveform; ping-pong; vol; next/prev
- [x] P130: Code Editor — apps/code-editor/ (1200×800); Vec<String> lines; Rust syntax highlight; gutter; save/load
- [x] P131: Crypto Ticker — apps/crypto-ticker/ (960×640); 5 coins; LCG walk; sparklines ×40 pts; alert >5%; sort
- [x] P132: Photo Filter — apps/photo-filter/ (1040×760); PPM P6 loader; 5 filters; 4×4 mosaic; save filtered.ppm

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
