# FINAL Spec: Terminal & Shell (Round 77)

**macOS Analogue**: Terminal.app / zsh / bash
**Status**: FINAL — all blocking issues resolved
**Depends on**: R21 window manager, R28 focus/z-order, R31 keyboard input, R41 file manager, R50 package manager

---

## 1. Architecture

Two components: a self-contained WASM app that owns both the terminal emulator and the shell
interpreter, plus a minimal supervisor-side resize notification.

```
apps/terminal/src/
├── main.rs       (≤500 lines) — event loop, stdin dispatch, stdout emit, VYOMA_DRAW flush
├── emulator.rs   (≤500 lines) — VT100/ANSI parser, screen buffer, scrollback ring
├── shell.rs      (≤500 lines) — command parser, built-in commands, cwd state, history I/O
├── input.rs      (≤300 lines) — keyboard handler, line-edit buffer, history navigation, tab-complete
└── render.rs     (≤300 lines) — Cell grid → VYOMA_DRAW commands, cursor blink, color mapping
```

Supervisor change: one new notification line `VYOMA_SYSTEM:terminal-resize:<cols>,<rows>` emitted
from `supervisor/src/display.rs` whenever the focused window's pixel dimensions change.  No new
VYOMA_TERMINAL: verb is required — all shell IPC reuses the existing `@supervisor:` channel.

Data flow:

```
stdin  ──► main.rs ──► input.rs (key events)
                  ──► shell.rs  (REPLY: / text lines from supervisor)
                  ──► emulator.rs (write_str)
                                     │
                              screen buffer
                                     │
                              render.rs ──► VYOMA_DRAW: lines ──► stdout ──► supervisor
```

---

## 2. Terminal Emulator (`emulator.rs`)

### Screen Buffer Types

```rust
#[derive(Clone)]
pub struct Cell {
    pub ch:    char,
    pub fg:    u32,   // RGBA packed: (R<<24)|(G<<16)|(B<<8)|A
    pub bg:    u32,
    pub attrs: u8,    // bit 0 = bold, bit 1 = underline, bit 2 = reverse
}

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', fg: 0xCDD6F4FF, bg: 0x1E1E2EFF, attrs: 0 }
    }
}

pub struct TerminalSize {
    pub cols: u32,
    pub rows: u32,
}

impl Default for TerminalSize {
    fn default() -> Self { TerminalSize { cols: 80, rows: 24 } }
}

const SCROLLBACK_CAPACITY: usize = 1000;

pub struct Screen {
    pub size:      TerminalSize,
    grid:          Vec<Vec<Cell>>,       // [row][col]
    scrollback:    Vec<Vec<Cell>>,       // ring, oldest first
    pub cursor_x:  u32,
    pub cursor_y:  u32,
    cur_fg:        u32,
    cur_bg:        u32,
    cur_attrs:     u8,
    escape_buf:    String,
    in_escape:     bool,
}
```

### ANSI Escape Sequence Support

`Screen::write_str` iterates bytes. On `\x1b` it sets `in_escape = true` and starts collecting into
`escape_buf`. On `m` (SGR), `H`/`f` (cursor position), `A`/`B`/`C`/`D` (cursor moves), `J`
(erase display), `K` (erase line) it dispatches to private handlers and resets `in_escape`.

Supported sequences:

| Sequence        | Action                                      |
|-----------------|---------------------------------------------|
| `ESC[H`         | cursor home (0, 0)                          |
| `ESC[<r>;<c>H`  | cursor absolute position                    |
| `ESC[<n>A`      | cursor up n rows                            |
| `ESC[<n>B`      | cursor down n rows                          |
| `ESC[<n>C`      | cursor right n cols                         |
| `ESC[<n>D`      | cursor left n cols                          |
| `ESC[2J`        | clear screen, cursor home                   |
| `ESC[K`         | erase from cursor to end of line            |
| `ESC[0m`        | reset all attributes                        |
| `ESC[1m`        | bold on                                     |
| `ESC[30–37m`    | set foreground to standard 8 colors         |
| `ESC[40–47m`    | set background to standard 8 colors         |
| `ESC[90–97m`    | set foreground to bright 8 colors           |

Color mapping function (Catppuccin Mocha palette):

```rust
pub fn ansi_fg(code: u8) -> u32 {
    match code {
        30 => 0x45475AFF, 31 => 0xF38BA8FF, 32 => 0xA6E3A1FF,
        33 => 0xF9E2AFFF, 34 => 0x89B4FAFF, 35 => 0xF5C2E7FF,
        36 => 0x94E2D5FF, 37 => 0xBAC2DEFF,
        90 => 0x585B70FF, 91 => 0xF38BA8FF, 92 => 0xA6E3A1FF,
        93 => 0xF9E2AFFF, 94 => 0x89B4FAFF, 95 => 0xF5C2E7FF,
        96 => 0x94E2D5FF, 97 => 0xCDD6F4FF,
        _  => 0xCDD6F4FF,
    }
}
```

### Scrollback

On newline at the last row, the current top row is pushed onto `scrollback` (capped at
`SCROLLBACK_CAPACITY` by dropping the oldest entry), and all rows shift up one.

---

## 3. Built-in Shell (`shell.rs`)

```rust
pub struct Shell {
    pub cwd:     String,
    pub history: Vec<String>,
    history_path: String,
}

pub enum ShellOutput {
    Lines(Vec<String>),    // write to screen buffer
    IpcSend(String),       // write raw to stdout (goes to supervisor)
    Exit,
}
```

`Shell::execute(cmd: &str) -> ShellOutput` parses the first token as the command name and the rest
as args. Filesystem operations use WASI `std::fs`.

Built-in dispatch table:

```rust
match argv[0] {
    "cd"    => self.cmd_cd(&argv[1..]),
    "ls"    => self.cmd_ls(argv.get(1).copied()),
    "pwd"   => ShellOutput::Lines(vec![self.cwd.clone()]),
    "echo"  => ShellOutput::Lines(vec![argv[1..].join(" ")]),
    "cat"   => self.cmd_cat(&argv[1..]),
    "clear" => ShellOutput::Lines(vec!["\x1b[2J".to_string()]),
    "ps"    => ShellOutput::IpcSend("@supervisor: list\n".to_string()),
    "spawn" => ShellOutput::IpcSend(format!("@supervisor: spawn {}\n", argv[1..].join(" "))),
    "kill"  => ShellOutput::IpcSend(format!("@supervisor: kill {}\n", argv[1..].join(" "))),
    "log"   => ShellOutput::IpcSend(format!("@supervisor: log {}\n", argv[1..].join(" "))),
    "exit"  => ShellOutput::Exit,
    other   => ShellOutput::Lines(vec![format!("terminal: command not found: {}", other)]),
}
```

`cmd_ls` calls `std::fs::read_dir`, collects entry names, formats them space-separated in rows of
up to 5 entries. `cmd_cat` reads the file via `std::fs::read_to_string`; if the path is relative,
it is joined with `self.cwd`.

History is persisted at `/data/.vyoma/terminal/history.txt`, one entry per line, max 500 entries
trimmed from the front. Load on startup, append on every successful command execution.

---

## 4. Keyboard Input Handling (`input.rs`)

```rust
pub struct LineEditor {
    pub buf:      Vec<char>,
    pub cursor:   usize,       // byte index within buf
    history:      Vec<String>,
    hist_idx:     Option<usize>,
    saved_line:   String,      // preserved while navigating history
}
```

`LineEditor::handle_key(key: &str) -> Option<String>` returns `Some(line)` when Enter is pressed.

```rust
pub fn handle_key(&mut self, key: &str) -> Option<String> {
    match key {
        "Enter"     => { let s = self.buf.iter().collect(); self.buf.clear(); self.cursor = 0;
                         self.hist_idx = None; Some(s) }
        "Backspace" => { if self.cursor > 0 { self.cursor -= 1; self.buf.remove(self.cursor); } None }
        "Left"      => { if self.cursor > 0 { self.cursor -= 1; } None }
        "Right"     => { if self.cursor < self.buf.len() { self.cursor += 1; } None }
        "Up"        => { self.history_prev(); None }
        "Down"      => { self.history_next(); None }
        "ctrl-c"    => { self.buf.clear(); self.cursor = 0; self.hist_idx = None; None }
        "ctrl-l"    => { self.buf.clear(); self.cursor = 0; Some("\x1b[2J".to_string()) }
        "Tab"       => { self.tab_complete(); None }
        k if k.len() == 1 => {
            let ch = k.chars().next().unwrap();
            self.buf.insert(self.cursor, ch);
            self.cursor += 1;
            None
        }
        _ => None,
    }
}
```

`tab_complete` takes the last token in `buf`, treats it as a path prefix, calls
`std::fs::read_dir` on the parent, and appends the unique matching suffix if exactly one match
exists. Known app names (`terminal`, `gui-demo`, `http-server`, `calculator`) are also candidates
when the token is the first word.

---

## 5. Rendering (`render.rs`)

```rust
const TERM_BG:      u32 = 0x1E1E2EFF;
const TERM_FG:      u32 = 0xCDD6F4FF;
const PROMPT_COLOR: u32 = 0xA6E3A1FF;
const CURSOR_COLOR: u32 = 0xCDD6F4FF;
const CELL_W:       u32 = 8;
const CELL_H:       u32 = 16;
const TOP_MARGIN:   u32 = 24;   // reserved for system menu bar
const LEFT_MARGIN:  u32 = 4;

pub fn cols_rows(window_w: u32, window_h: u32) -> (u32, u32) {
    let cols = (window_w.saturating_sub(LEFT_MARGIN * 2)) / CELL_W;
    let rows = (window_h.saturating_sub(TOP_MARGIN + 8)) / CELL_H;
    (cols.max(20), rows.max(4))
}
```

`render_frame(screen: &Screen, editor: &LineEditor, frame: u64)` emits VYOMA_DRAW lines to
stdout in this order:

1. `fill_rect` covering the full terminal area with `TERM_BG`.
2. For each cell `(row, col)` where cell differs from `Cell::default()` or was previously non-default
   (dirty tracking): emit `fill_rect` for background if `cell.bg != TERM_BG`, then `draw_text` for
   the character.
3. Render the prompt on the last row: `draw_text` for `$ ` in `PROMPT_COLOR`, then `draw_text` for
   the chars in `editor.buf` up to `editor.cursor` in `TERM_FG`.
4. Cursor blink: `frame % 60 < 30` → draw a filled `fill_rect` at cursor position in
   `CURSOR_COLOR` at half-opacity `0xCDD6F47F`.
5. Emit `VYOMA_DRAW:flush`.
6. Immediately call `std::io::stdout().flush().unwrap()` — **required** (see B1).

All VYOMA_DRAW lines use `println!`. Text is emitted with size `m` (8×16):

```rust
println!("VYOMA_DRAW:draw_text:{},{},{},{},{}", x, y, cell.fg, "m", ch);
```

---

## 6. Event Loop (`main.rs`)

```rust
use std::io::{self, BufRead, Write};

fn main() {
    let mut screen  = Screen::new(TerminalSize::default());
    let mut shell   = Shell::new();
    let mut editor  = LineEditor::new(shell.history.clone());
    let mut frame: u64 = 0;

    let stdin = io::stdin();
    let stdout = io::stdout();

    loop {
        // Render current frame
        render::render_frame(&screen, &editor, frame);
        frame = frame.wrapping_add(1);

        // Read one line from stdin (blocks until newline)
        let mut line = String::new();
        let n = stdin.lock().read_line(&mut line).unwrap_or(0);
        if n == 0 { break; }  // EOF — supervisor closed pipe
        let line = line.trim_end_matches('\n').trim_end_matches('\r');

        if let Some(key) = line.strip_prefix("VYOMA_INPUT:key:") {
            if let Some(cmd) = editor.handle_key(key) {
                // ctrl-l sends an escape sequence as a "command"
                if cmd.starts_with('\x1b') {
                    screen.write_str(&cmd);
                } else if !cmd.trim().is_empty() {
                    let prompt_echo = format!("$ {}\r\n", cmd);
                    screen.write_str(&prompt_echo);
                    shell.history.push(cmd.clone());
                    shell.save_history();
                    match shell.execute(&cmd) {
                        ShellOutput::Lines(lines) => {
                            for l in lines { screen.write_str(&format!("{}\r\n", l)); }
                        }
                        ShellOutput::IpcSend(msg) => {
                            print!("{}", msg);
                            io::stdout().flush().unwrap();
                        }
                        ShellOutput::Exit => break,
                    }
                }
            }
        } else if let Some(resize) = line.strip_prefix("VYOMA_SYSTEM:terminal-resize:") {
            if let Some((c, r)) = resize.split_once(',') {
                let cols: u32 = c.trim().parse().unwrap_or(80);
                let rows: u32 = r.trim().parse().unwrap_or(24);
                screen.resize(TerminalSize { cols, rows });
            }
        } else if let Some(reply) = line.strip_prefix("REPLY:") {
            // Supervisor IPC response — display verbatim
            screen.write_str(&format!("{}\r\n", reply));
        } else {
            // Plain text from supervisor (e.g. forwarded app stdout)
            screen.write_str(&format!("{}\r\n", line));
        }
    }
}
```

---

## 7. Supervisor Side — Resize Notification

In `supervisor/src/display.rs`, when the framebuffer reports a size change or when tiling layout
recalculates window bounds, the focused app receives the resize notification. Add to the existing
focused-app stdin write path:

```rust
fn notify_resize(app_stdin: &mut dyn Write, cols: u32, rows: u32) -> io::Result<()> {
    writeln!(app_stdin, "VYOMA_SYSTEM:terminal-resize:{},{}", cols, rows)
}
```

`cols` and `rows` are computed from the new window pixel dimensions using the same formula as
`render::cols_rows`: subtract margins, divide by cell dimensions.

No new VYOMA_TERMINAL: protocol lines are required. All app-to-supervisor shell commands continue
to use the existing `@supervisor:` prefix handled by `supervisor/src/ipc_handlers.rs`.

---

## 8. App Manifest (`apps/terminal/vyoma.toml`)

```toml
[app]
name    = "terminal"
version = "0.1.0"
wasm    = "terminal.wasm"

[capabilities]
stdio      = true
display    = true
shell      = true
filesystem = true
mouse      = true
```

The `shell = true` capability allows the app to emit `@supervisor:` IPC commands (spawn, kill,
list, log). The `filesystem = true` capability mounts `/data` so history can be persisted at
`/data/.vyoma/terminal/history.txt`.

---

## 9. Cargo.toml (`apps/terminal/Cargo.toml`)

```toml
[package]
name    = "terminal"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "terminal"
path = "src/main.rs"

[profile.release]
opt-level = "z"
lto       = true
strip     = true
```

No external crate dependencies. All functionality uses `std` (WASI-backed): `std::io`,
`std::fs`, `std::path::Path`. The `wasm32-wasip2` target provides WASI Preview 2 host functions
for filesystem and stdio.

Build command:

```bash
cargo build --target wasm32-wasip2 --release
```

---

## 10. Blocking Issues B1–B5

### B1 — WASM stdout line-buffering drops VYOMA_DRAW frames

**Problem**: WASM stdout is line-buffered by default. A `VYOMA_DRAW:flush` emitted with `println!`
is followed immediately by more lines — but if the supervisor reads a partial frame before the OS
flushes the pipe, the frame is lost or corrupted.

**Fix**: After every `VYOMA_DRAW:flush` line, explicitly call:

```rust
std::io::stdout().flush().unwrap();
```

This is already encoded in `render::render_frame` (step 6 above). Never omit it. One
`stdout().flush()` per rendered frame.

---

### B2 — Stdin blocking prevents animation ticks

**Problem**: `stdin().lock().read_line(&mut buf)` blocks until a newline arrives. If no key is
pressed, the event loop stalls and the cursor never blinks.

**Fix**: Use a background thread for stdin reads, communicating via `std::sync::mpsc`:

```rust
use std::sync::mpsc;
use std::io::BufRead;

let (tx, rx) = mpsc::channel::<String>();
std::thread::spawn(move || {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        match line {
            Ok(l) => { if tx.send(l).is_err() { break; } }
            Err(_) => break,
        }
    }
});

loop {
    render::render_frame(&screen, &editor, frame);
    frame = frame.wrapping_add(1);

    // 16ms tick — roughly 60fps blink cycle
    match rx.recv_timeout(std::time::Duration::from_millis(16)) {
        Ok(line) => { /* dispatch line */ }
        Err(mpsc::RecvTimeoutError::Timeout) => continue,
        Err(mpsc::RecvTimeoutError::Disconnected) => break,
    }
}
```

This allows the render loop to run at ~60 Hz regardless of keyboard activity, enabling cursor blink
without busy-polling.

---

### B3 — ANSI escape sequences split across stdin reads

**Problem**: A single escape sequence such as `ESC[32;1m` may arrive as two separate stdin lines
if the supervisor flushes mid-sequence. This produces garbled output in the screen buffer.

**Fix**: The escape buffer in `Screen` is persistent across `write_str` calls. The `in_escape` flag
and `escape_buf` fields remain set between calls:

```rust
pub fn write_str(&mut self, s: &str) {
    for ch in s.chars() {
        if self.in_escape {
            self.escape_buf.push(ch);
            if ch.is_ascii_alphabetic() {
                self.dispatch_escape();
                self.in_escape = false;
                self.escape_buf.clear();
            }
            // non-alpha: keep accumulating
        } else if ch == '\x1b' {
            self.in_escape = true;
            self.escape_buf.clear();
        } else {
            self.put_char(ch);
        }
    }
    // in_escape remains true if sequence is incomplete — next write_str continues it
}
```

Because `Screen` is owned by the event loop and `write_str` is called every time a text chunk
arrives, incomplete sequences are seamlessly continued on the next call.

---

### B4 — Supervisor IPC response `REPLY:` prefix not stripped for display

**Problem**: When the terminal sends `@supervisor: list` and the supervisor responds, the response
line on stdin arrives as `REPLY:<content>`. If displayed verbatim, the user sees `REPLY:` as a
literal prefix.

**Fix**: In the event loop (main.rs), the `REPLY:` branch strips the prefix before writing to the
screen buffer:

```rust
} else if let Some(reply) = line.strip_prefix("REPLY:") {
    screen.write_str(&format!("{}\r\n", reply));
}
```

This must be checked *before* the generic text fallback branch. The order of `strip_prefix` checks
in the event loop is:

1. `VYOMA_INPUT:key:`
2. `VYOMA_SYSTEM:terminal-resize:`
3. `REPLY:`
4. Generic text

---

### B5 — Terminal cols/rows stale after window resize

**Problem**: The terminal renders with the initial 80×24 grid. If the supervisor resizes the window
(e.g. tiling layout change), the grid no longer fits the new pixel dimensions, causing clipped or
overflow text.

**Fix**: `Screen::resize` reflows the buffer to the new dimensions. Cells that fit are preserved;
the new trailing area is filled with `Cell::default()`. Scrollback is unaffected.

```rust
pub fn resize(&mut self, new_size: TerminalSize) {
    let mut new_grid = vec![
        vec![Cell::default(); new_size.cols as usize];
        new_size.rows as usize
    ];
    let copy_rows = self.size.rows.min(new_size.rows) as usize;
    let copy_cols = self.size.cols.min(new_size.cols) as usize;
    for r in 0..copy_rows {
        for c in 0..copy_cols {
            new_grid[r][c] = self.grid[r][c].clone();
        }
    }
    self.grid = new_grid;
    self.size = new_size;
    self.cursor_x = self.cursor_x.min(self.size.cols.saturating_sub(1));
    self.cursor_y = self.cursor_y.min(self.size.rows.saturating_sub(1));
}
```

The supervisor sends `VYOMA_SYSTEM:terminal-resize:<cols>,<rows>` immediately after recalculating
window bounds so the terminal grid is always in sync with the visible pixel area.
