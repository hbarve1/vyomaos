# Phase 12 — Interactive Shell

## Goal

Make VyomaOS interactive. A `shell` WASM app runs in the foreground, reads keyboard input from the QEMU window, interprets commands, and renders output directly to the framebuffer via VYOMA_DRAW. The supervisor routes raw keyboard events (from `/dev/tty0` or `/dev/input/eventN`) to the focused app's stdin. This is the first true interactive OS behaviour — the user types, the OS responds on screen.

## Gate

```sh
make run-gui DISPLAY_BACKEND=cocoa
# QEMU window shows a blinking cursor in the shell panel
# User types: status     → shell lists running apps + boot count
# User types: run ping   → supervisor launches ping app on demand
# User types: clear      → shell redraws blank panel
# User types: help       → command list rendered on screen
```

## Architecture

```
QEMU keyboard input → /dev/tty0 (virtual console, via fbcon)
  └─ Supervisor input thread: reads /dev/tty0, routes to focused app stdin
       └─ shell WASM app (stdin: keypresses, stdout: VYOMA_DRAW + @supervisor: commands)
            ├─ VYOMA_DRAW:draw_text renders command output to framebuffer
            └─ @supervisor: run <app>   (new supervisor IPC command)
```

## Key design decisions

### Keyboard input source
`/dev/tty0` is the virtual console device (backed by fbcon). Reading it gives cooked-mode line input by default. For raw character-by-character input (needed for a real shell feel), the supervisor sets the tty to raw mode via `tcsetattr` before handing the fd to the shell app — or the shell app does it via WASI (if supported).

Alternative: `/dev/input/event0` gives raw evdev events (key codes), which requires kernel `CONFIG_INPUT=y` + `CONFIG_INPUT_EVDEV=y` and a key-code → ASCII table in the shell app.

**Recommended approach:** `/dev/tty0` in raw mode — simpler, no extra kernel config.

### Focus model
Single-focus: only one app receives keyboard input at a time. The supervisor tracks a `focused_app: Option<String>`. Default focus is the `shell` app. The `Tab` key cycles focus (future).

### `@supervisor:` IPC command extension
The shell app can issue supervisor commands via the existing IPC protocol:
```
@supervisor: run <manifest_path>   → supervisor spawns app on demand
@supervisor: kill <app_name>       → supervisor sends SIGTERM to app
@supervisor: list                  → supervisor sends back app list via shell's stdin
```

## Dependencies

- Phase 09 (VYOMA_DRAW text-drawing, P10 text rendering needed for output)
- Phase 10 (draw_text — shell output is text, not shapes)
- Kernel: `CONFIG_TTY=y` (already present), no new kernel config needed for tty0
- Supervisor: new input-routing thread + focus manager + `@supervisor:` command handler

## Tasks

### P12T01 — Supervisor Input Thread
[tasks/task-P12T01-supervisor-input-thread.md](tasks/task-P12T01-supervisor-input-thread.md)

Open `/dev/tty0` after devtmpfs mount. Spawn a dedicated input-reader thread. Reads lines (or raw chars in raw mode) and forwards to `focused_app`'s stdin via the existing inbox `mpsc` channel.

### P12T02 — Focus Manager
[tasks/task-P12T02-focus-manager.md](tasks/task-P12T02-focus-manager.md)

Add `focused_app: Arc<Mutex<Option<String>>>` to supervisor state. Default to the app with `shell = true` capability (new capability field). Tab-switching and `@supervisor: focus <app>` command change focus.

### P12T03 — `@supervisor:` Command Handler
[tasks/task-P12T03-supervisor-ipc-commands.md](tasks/task-P12T03-supervisor-ipc-commands.md)

Extend the IPC router to intercept messages addressed to the special `supervisor` target. Handle: `run <path>`, `kill <name>`, `list` (sends back newline-separated app names to the requesting app's stdin).

### P12T04 — `shell` WASM App
[tasks/task-P12T04-shell-app.md](tasks/task-P12T04-shell-app.md)

New `apps/shell/` crate. Reads stdin line-by-line. Maintains a display buffer (list of output lines). On each command:
- `status` → sends `@supervisor: list`, waits for reply, renders with draw_text
- `run <app>` → sends `@supervisor: run /apps/<app>/vyoma.toml`
- `clear` → fill_rect to clear shell panel, reset line counter
- `help` → render command list
- Unknown command → render "unknown: <cmd>"

Renders a prompt (`> `) and a blinking cursor via a VYOMA_DRAW fill_rect toggled by a timer (or simply on each input event).
