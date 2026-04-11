# P12T04 — `shell` WASM App

## Phase
Phase 12 — Interactive Shell

## Goal
Build `apps/shell/` — a WASM app that presents an interactive command prompt in the QEMU window. Reads keyboard input via stdin (forwarded by the supervisor input thread), renders output using `VYOMA_DRAW:draw_text`, and dispatches commands via `@supervisor:` IPC.

## Files to create

```
apps/shell/Cargo.toml
apps/shell/src/main.rs
apps/shell/vyoma.toml
```

## Manifest

```toml
[app]
name    = "shell"
version = "0.1.0"
wasm    = "shell.wasm"

[capabilities]
stdio      = true
filesystem = false
network    = false
display    = true   # VYOMA_DRAW output
shell      = true   # receives keyboard focus by default
```

## Command set (MVP)

| Command | Description |
|---------|-------------|
| `help` | List available commands |
| `status` | Send `@supervisor: status`, display JSON response |
| `list` | Send `@supervisor: list`, display running app names |
| `clear` | Clear the shell panel (fill_rect + reset line counter) |
| `run <app>` | Send `@supervisor: run /apps/<app>/vyoma.toml` |
| `boot` | Show boot count from `/data/boot_count.txt` (if fs=true) |

## Layout

```
┌────────────────────────────────────┐
│ shell                              │  title bar (dark)
├────────────────────────────────────┤
│ > status                           │  previous command
│ {"running":7}                      │  command output
│                                    │
│ > list                             │
│ hello-world                        │
│ calculator                         │
│ ...                                │
│                                    │
│ >_                                 │  current prompt + cursor
└────────────────────────────────────┘
```

## Implementation sketch

```rust
fn main() {
    let (sw, sh) = (600u32, 400u32);  // shell panel dimensions
    let (px, py) = (32u32, 340u32);   // panel position on screen

    draw_panel(px, py, sw, sh);
    let mut lines: Vec<String> = vec![];
    let mut input = String::new();

    draw_prompt(px, py, sh, &input, &lines);

    for line in std::io::stdin().lines() {
        let cmd = match line { Ok(l) => l.trim().to_string(), Err(_) => break };
        lines.push(format!("> {cmd}"));

        match cmd.as_str() {
            "help"  => lines.push("commands: help status list clear".into()),
            "clear" => { lines.clear(); draw_panel(px, py, sw, sh); }
            "list"  => { println!("@supervisor: list"); /* wait for reply */ }
            "status"=> { println!("@supervisor: status"); }
            other   => lines.push(format!("unknown: {other}")),
        }

        draw_prompt(px, py, sh, "", &lines);
        flush();
    }
}
```

## Notes

- Reading `@supervisor: list` replies requires reading back from stdin — the supervisor sends the reply to this app's stdin, but currently the shell is also reading keyboard input from stdin. This conflict needs to be resolved (e.g., prefix supervisor replies with `REPLY:` so the shell can distinguish them from keyboard input).
- Blinking cursor: implemented as a `fill_rect` over the cursor position, toggled on each input event rather than a timer (WASM apps have no timer API without WASI poll)
- This is the most complex app in the project — build on top of a working P12T01-P12T03

## Verification

```sh
make run-gui DISPLAY_BACKEND=cocoa
# Type "help" → commands appear on screen
# Type "list" → running app names appear on screen
# Type "status" → JSON response appears on screen
```
