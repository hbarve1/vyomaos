# P12T01 — Supervisor Input Thread

## Phase
Phase 12 — Interactive Shell

## Goal
Open `/dev/tty0` in the supervisor after devtmpfs is mounted, spawn a dedicated input-reader thread, and forward each line (or raw character) to the currently focused app's stdin via the existing `inbox` mpsc channel.

## Files to modify

```
supervisor/src/main.rs    — open tty0, spawn input thread, add focused_app state
```

## Implementation sketch

```rust
// After mount_filesystems() in main():
#[cfg(target_os = "linux")]
let focused: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

// After Pass 1 spawn (set default focus to the app with shell=true capability):
{
    let mut f = focused.lock().unwrap();
    *f = spawned.iter().find(|a| a.is_shell).map(|a| a.name.clone());
}

// Input thread:
let inbox_input = Arc::clone(&inbox);
let focused_input = Arc::clone(&focused);
thread::Builder::new()
    .name("input-router".into())
    .spawn(move || {
        use std::io::BufRead;
        let tty = std::fs::File::open("/dev/tty0").expect("open tty0");
        for line in std::io::BufReader::new(tty).lines() {
            let line = match line { Ok(l) => l, Err(_) => break };
            let target = focused_input.lock().unwrap().clone();
            if let Some(name) = target {
                let map = inbox_input.lock().unwrap();
                if let Some(tx) = map.get(&name) {
                    let _ = tx.send(line);
                }
            }
        }
    })
    .expect("spawn input-router");
```

## Raw mode consideration

For character-by-character input (shell feel), set tty0 to raw mode before the read loop:

```rust
use libc::{tcgetattr, tcsetattr, TCSANOW};
// ... set c_lflag &= !(ICANON | ECHO)
// ... set c_cc[VMIN] = 1, c_cc[VTIME] = 0
```

This is optional for MVP (line-buffered input still works for command dispatch).

## Notes

- `/dev/tty0` is the current virtual console — backed by fbcon when running with `-vga virtio`
- Requires Phase 10 text rendering to be useful (shell output needs draw_text)
- The `focused_app` state is shared with the focus manager (P12T02)
