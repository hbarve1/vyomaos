---
title: Building Your First VyomaOS App
description: Step-by-step tutorial for creating a VyomaOS WASM app that draws to the screen using the VYOMA_DRAW protocol.
order: 3
---

# Building Your First VyomaOS App

This tutorial walks you through creating a VyomaOS application from scratch. You will build a hello-world app that draws colored rectangles and text to the screen, then run it inside QEMU.

By the end, you will understand the three files every VyomaOS app needs, how the capability manifest works, and how to use the VYOMA_DRAW display protocol.

## Prerequisites

You need Rust with the `wasm32-wasip2` target installed:

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the WASM target
rustup target add wasm32-wasip2
```

You also need the VyomaOS repository cloned and a successful base build:

```bash
git clone https://github.com/hbarve1/vyomaos.git
cd vyomaos
make build
```

This compiles the kernel, supervisor, and all existing apps. Once it succeeds, you are ready to create your own app.

## Step 1: Create the app directory

Every VyomaOS app lives in its own directory under `apps/`:

```bash
mkdir -p apps/my-first-app/src
```

## Step 2: Write Cargo.toml

Create `apps/my-first-app/Cargo.toml`:

```toml
[package]
name    = "my-first-app"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "my-first-app"
path = "src/main.rs"
```

Keep dependencies minimal. VyomaOS apps target `wasm32-wasip2`, so only pure-Rust crates that support WASM will work. For a hello-world app, you need no external dependencies.

## Step 3: Write the app manifest

Create `apps/my-first-app/vyoma.toml`:

```toml
[app]
name    = "my-first-app"
version = "0.1.0"
wasm    = "my-first-app.wasm"
restart = "never"

[capabilities]
stdio   = true
display = true

[window]
title  = "My First App"
x      = 50
y      = 50
width  = 400
height = 300
```

This manifest tells the supervisor:

- **`stdio = true`**: The app can read from stdin and write to stdout. Required for all apps that produce output.
- **`display = true`**: The app can draw to the framebuffer using the VYOMA_DRAW protocol.
- **`restart = "never"`**: The app runs once and exits. Use `"always"` for long-running services.

Everything not declared is unavailable. This app cannot access the filesystem, network, or IPC shell commands.

The `[window]` section defines where the app's window appears on screen. Position `(50, 50)` with size `400x300` pixels.

## Step 4: Write the application code

Create `apps/my-first-app/src/main.rs`:

```rust
use std::io::{self, BufRead};

// Color constants (RGBA packed as u32)
const BG: u32      = 0x1E1E2EFF; // Dark background
const WHITE: u32   = 0xFFFFFFFF; // White text
const BLUE: u32    = 0x89B4FAFF; // Accent blue
const GREEN: u32   = 0xA6E3A1FF; // Success green
const SURFACE: u32 = 0x313244FF; // Card surface

fn main() {
    // Clear the window background
    println!("VYOMA_DRAW:fill_rect:0,0,400,300,{BG}");

    // Draw a header bar
    println!("VYOMA_DRAW:fill_rect:0,0,400,40,{BLUE}");
    println!("VYOMA_DRAW:draw_text:12,12,{BG},m,My First VyomaOS App");

    // Draw a content card with rounded corners
    println!("VYOMA_DRAW:fill_rect_r:20,60,360,100,{SURFACE},8");

    // Draw text inside the card
    println!("VYOMA_DRAW:draw_text:36,76,{WHITE},m,Hello from WebAssembly!");
    println!("VYOMA_DRAW:draw_text:36,100,{GREEN},s,Running as a wasm32-wasip2 binary");
    println!("VYOMA_DRAW:draw_text:36,120,{GREEN},s,Inside the VyomaOS supervisor");

    // Draw a status section
    println!("VYOMA_DRAW:fill_rect_r:20,180,360,80,{SURFACE},8");
    println!("VYOMA_DRAW:draw_text:36,196,{BLUE},m,System Info");
    println!("VYOMA_DRAW:draw_text:36,220,{WHITE},s,Capabilities: stdio, display");
    println!("VYOMA_DRAW:draw_text:36,240,{WHITE},s,Window: 400x300 at (50, 50)");

    // Commit the frame to the screen
    println!("VYOMA_DRAW:flush");

    // Keep the app alive to display the window
    // Read from stdin to block (supervisor sends input events)
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        match line {
            Ok(input) => {
                if input.trim() == "quit" {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}
```

### How VYOMA_DRAW works

The display protocol is line-oriented. Your app writes commands to stdout, and the supervisor parses them:

| Command | Format | Description |
|---------|--------|-------------|
| `fill_rect` | `x,y,w,h,rgba` | Fill a solid rectangle |
| `fill_rect_r` | `x,y,w,h,rgba,radius` | Fill a rounded rectangle |
| `draw_text` | `x,y,rgba,size,text` | Draw text at position |
| `draw_glyph` | `x,y,rgba,size,codepoint` | Draw a single Unicode glyph |
| `draw_image` | `x,y,w,h,format,base64data` | Blit an inline image |
| `flush` | (no args) | Commit the current frame to the screen |

Text sizes: `s` = 4x8 pixels, `m` = 8x16 pixels, `l` = 16x32 pixels.

Colors are packed RGBA as a `u32`: `(R << 24) | (G << 16) | (B << 8) | A`, printed as a decimal integer.

Every frame must end with `flush` to appear on screen.

## Step 5: Build the app

Compile for the WASM target:

```bash
cd apps/my-first-app
cargo build --target wasm32-wasip2 --release
```

This produces `apps/my-first-app/target/wasm32-wasip2/release/my-first-app.wasm`. Check the binary size:

```bash
ls -lh target/wasm32-wasip2/release/my-first-app.wasm
# Typically 1-5 KB for a simple app
```

## Step 6: Add to the rootfs

Edit `base/rootfs.sh` to include your new app. Find the section where other apps are copied and add:

```bash
# My first app
cp /work/apps/my-first-app/target/wasm32-wasip2/release/my-first-app.wasm \
   "$ROOTFS/opt/vyoma/apps/my-first-app/my-first-app.wasm"
cp /work/apps/my-first-app/vyoma.toml \
   "$ROOTFS/opt/vyoma/apps/my-first-app/vyoma.toml"
```

Also add the app to `base/boot.toml` so the supervisor starts it:

```toml
[[apps]]
name = "my-first-app"
path = "/opt/vyoma/apps/my-first-app"
```

## Step 7: Build and run

Rebuild the rootfs and boot:

```bash
cd /path/to/vyomaos
make rootfs
make run-gui
```

Your app window should appear at position `(50, 50)` with a blue header bar, dark content cards, and green status text.

## Step 8: Add interactivity

Let's make the app respond to keyboard input. Update `src/main.rs`:

```rust
use std::io::{self, BufRead};

const BG: u32      = 0x1E1E2EFF;
const WHITE: u32   = 0xFFFFFFFF;
const BLUE: u32    = 0x89B4FAFF;
const GREEN: u32   = 0xA6E3A1FF;
const SURFACE: u32 = 0x313244FF;

fn draw_frame(counter: u32) {
    println!("VYOMA_DRAW:fill_rect:0,0,400,300,{BG}");
    println!("VYOMA_DRAW:fill_rect:0,0,400,40,{BLUE}");
    println!("VYOMA_DRAW:draw_text:12,12,{BG},m,Interactive App");

    println!("VYOMA_DRAW:fill_rect_r:20,60,360,80,{SURFACE},8");
    println!("VYOMA_DRAW:draw_text:36,76,{WHITE},m,Press any key to increment");
    let msg = format!("Counter: {counter}");
    println!("VYOMA_DRAW:draw_text:36,100,{GREEN},l,{msg}");

    println!("VYOMA_DRAW:flush");
}

fn main() {
    let mut counter: u32 = 0;
    draw_frame(counter);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        match line {
            Ok(_) => {
                counter += 1;
                draw_frame(counter);
            }
            Err(_) => break,
        }
    }
}
```

The supervisor routes keyboard events to the focused app's stdin. Each keypress triggers a redraw with the updated counter.

## Understanding the build pipeline

Here is what happens when your app runs:

1. `cargo build --target wasm32-wasip2 --release` compiles Rust to a `.wasm` binary
2. `make rootfs` packages the binary into `initramfs.cpio.gz`
3. QEMU boots the Linux kernel, which starts the supervisor as PID 1
4. The supervisor reads `boot.toml`, finds your app, loads its `vyoma.toml`
5. The supervisor spawns Wasmtime with only the declared WASI imports
6. Your app writes VYOMA_DRAW commands to stdout
7. The supervisor parses these commands and renders to the framebuffer

## Next steps

Now that you have a working app, explore more VyomaOS capabilities:

### IPC -- communicate with other apps

Send messages to other running apps through the supervisor's IPC broker:

```rust
// Send a message to another app
println!("@calculator: compute 2+2");

// Receive messages on stdin (supervisor delivers them)
// The supervisor strips the @sender: prefix before delivery
```

### Filesystem -- persist data across reboots

Declare `filesystem = true` in your manifest to access `/data`:

```toml
[capabilities]
filesystem = true
```

```rust
use std::fs;
fs::write("/data/my-app/state.json", "{\"count\": 42}").unwrap();
let data = fs::read_to_string("/data/my-app/state.json").unwrap();
```

### Networking -- serve HTTP or make requests

Declare `network = true` and boot with `make run-net`:

```toml
[capabilities]
network = true
```

```rust
use std::net::TcpListener;
let listener = TcpListener::bind("0.0.0.0:8080").unwrap();
for stream in listener.incoming() {
    // Handle HTTP requests
}
```

### Mouse input -- respond to clicks

Declare `mouse = true` to receive mouse events:

```toml
[capabilities]
mouse = true
```

The supervisor delivers `VYOMA_INPUT:mouse:` events to your app's stdin when the cursor is over your window.

### Shell commands -- manage processes

Declare `shell = true` to issue supervisor commands:

```rust
println!("@supervisor: ps");       // List running apps
println!("@supervisor: kill foo"); // Terminate an app
```

## Troubleshooting

**App does not appear on screen**: Verify `display = true` in `vyoma.toml` and that you call `VYOMA_DRAW:flush` after drawing commands.

**Build fails with target error**: Run `rustup target add wasm32-wasip2` to install the WASM target.

**App exits immediately**: If using `restart = "never"`, the app must block on stdin (or loop) to stay alive. Without blocking, it exits after `main()` returns and the window disappears.

**Window is black**: Check that color values are valid RGBA u32 integers. A common mistake is using hex notation in the protocol string instead of decimal.

**IPC messages not arriving**: The target app must be running and listed in `boot.toml`. Messages use the format `@<app-name>: <message>` with exactly one space after the colon.
