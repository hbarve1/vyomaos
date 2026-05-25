---
title: Creating an App
description: Build your first VyomaOS WASM app from scratch.
---

Every VyomaOS app is a standard Rust crate that compiles to `wasm32-wasip2`. Here's how to create one.

## 1. Create the App Directory

```bash
mkdir apps/my-app
cd apps/my-app
```

## 2. Create `Cargo.toml`

```toml
[package]
name    = "my-app"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "my-app"
path = "src/main.rs"

[dependencies]
# Keep minimal for small binary size
```

## 3. Write Your App

Create `src/main.rs`:

```rust
fn main() {
    println!("Hello from my-app!");
}
```

For a GUI app using the VYOMA_DRAW protocol:

```rust
const WHITE: u32 = 0xFFFFFFFF;
const BG: u32 = 0x1E1E2EFF;
const PURPLE: u32 = 0x7C3AEDFF;

fn main() {
    // Clear background
    println!("VYOMA_DRAW:fill_rect:0,0,960,700,{BG}");

    // Draw title
    println!("VYOMA_DRAW:draw_text:20,30,{WHITE},m,My App");

    // Draw a colored box
    println!("VYOMA_DRAW:fill_rect:20,60,200,100,{PURPLE}");
    println!("VYOMA_DRAW:draw_text:30,100,{WHITE},m,Hello World!");

    // Commit frame to display
    println!("VYOMA_DRAW:flush");

    // Keep running to receive input
    loop {
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            let input = line.trim();
            if input.starts_with("VYOMA_INPUT:key:") {
                let key = &input[16..];
                eprintln!("[my-app] key pressed: {key}");
            }
        }
    }
}
```

## 4. Create the Manifest

Create `vyoma.toml`:

```toml
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
stdio   = true      # Required for console output
display = true      # Required for VYOMA_DRAW
# filesystem = true # Mount /data for persistent storage
# network = true    # TCP sockets
# shell = true      # @supervisor: commands
# mouse = true      # Mouse input events
```

Only declare the capabilities your app needs. Undeclared capabilities are never wired up.

## 5. Build

```bash
cargo build --target wasm32-wasip2 --release
```

The output is at `target/wasm32-wasip2/release/my-app.wasm` (typically 1-10 KB).

## 6. Add to Boot

Update `base/rootfs.sh` to include your app in the initramfs. Follow the pattern used by existing apps like `hello-world`.

Then rebuild and boot:

```bash
make rootfs && make run
```

## Capabilities Reference

| Capability | What it enables |
|-----------|----------------|
| `stdio = true` | stdin/stdout/stderr (console I/O, keyboard input) |
| `filesystem = true` | Mount `/data` (9P, persistent across reboots) |
| `network = true` | WASI sockets (TCP, default port 8080) |
| `display = true` | Framebuffer access + VYOMA_DRAW protocol |
| `shell = true` | Issue `@supervisor:` process management commands |
| `mouse = true` | Receive `VYOMA_INPUT:mouse:` events |
| `watchdog_secs = N` | Supervisor kills app if silent for N seconds |

## IPC Communication

Apps can send messages to other apps via stdout:

```rust
// Send to a specific app
println!("@other-app: hello!");

// Broadcast to all apps
println!("@broadcast: system event");

// Reply to last sender
println!("@reply: got it");

// Supervisor commands
println!("@supervisor: list");
println!("@supervisor: ping");
```

## Tips

- Keep dependencies minimal — every dependency increases binary size
- Use `eprintln!` for debug logging (goes to supervisor's stderr log)
- Test with `cargo build --target wasm32-wasip2` before integrating
- The 500-line limit applies to all `.rs` files — split into modules early
