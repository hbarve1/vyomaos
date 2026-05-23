# Contributing to VyomaOS

VyomaOS is an open research project exploring what a WASM-first OS looks like from the ground up. Contributions of all kinds are welcome — new apps, supervisor improvements, documentation, design discussions, and bug reports.

## Quick Start

**Prerequisites**: Docker, QEMU, Rust (stable), `wasm32-wasip2` target

```sh
git clone https://github.com/hbarve1/vyomaos
cd vyomaos

# Add the WASM target once
rustup target add wasm32-wasip2

# Build everything and boot
make build
make run-gui DISPLAY_BACKEND=cocoa   # macOS
make run-gui DISPLAY_BACKEND=sdl     # Linux
```

The build is fully hermetic: kernel, supervisor, and all apps compile inside a Docker container. The only host requirements are Docker and QEMU.

## Architecture in 60 Seconds

```
Linux 5.10 (hardware layer only)
  └── Rust supervisor (PID 1, static musl binary)
        ├── reads /etc/vyoma/boot.toml (which apps to start)
        ├── spawns one Wasmtime process per app
        ├── brokers IPC: app writes "@target: msg" → supervisor delivers to target's stdin
        ├── renders display: app writes "VYOMA_DRAW:..." → supervisor paints framebuffer
        └── manages focus, z-order, keyboard routing
              └── WASM apps (wasm32-wasip2, each self-contained)
```

Each app is an ordinary Rust program that reads stdin and writes stdout. It never makes syscalls directly — Wasmtime intercepts them and enforces the capability manifest.

**Key files**:
- `supervisor/src/main.rs` — everything about app lifecycle, IPC, display, keyboard
- `apps/<name>/src/main.rs` — each app's full logic
- `apps/<name>/vyoma.toml` — capability manifest (what the app can access)
- `base/rootfs.sh` — assembles the initramfs

## Writing a New App

The fastest way: copy an existing app as a starting point.

```sh
cp -r apps/clock apps/my-app
```

**Minimal structure**:

```
apps/my-app/
├── Cargo.toml      # standard Rust crate, target wasm32-wasip2
├── vyoma.toml      # capability manifest + window position
└── src/main.rs     # your code
```

**Cargo.toml template**:
```toml
[package]
name    = "my-app"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "my-app"
path = "src/main.rs"

[profile.release]
opt-level = "z"
strip     = true
```

**vyoma.toml template**:
```toml
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
stdio   = true      # read stdin (keyboard), write stdout (draw commands, IPC)
display = true      # use VYOMA_DRAW: framebuffer protocol
shell   = true      # issue @supervisor: commands

[window]
x = 100
y = 60
w = 800
h = 500
```

**Display helpers** (copy these into your app):
```rust
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
```

**Keyboard input** (read from stdin line-by-line):
```rust
"\x1b[A"  // Up arrow
"\x1b[B"  // Down arrow
"\x1b[C"  // Right arrow
"\x1b[D"  // Left arrow
""        // Enter
"\x7f"    // Backspace
"\x03"    // Ctrl+C
"\x1b"    // Escape
```

**Wire it up**:
1. Add a build line to `Makefile` (inside the `apps` target):
   ```make
   $(DOCKER_RUN) cargo build --manifest-path apps/my-app/Cargo.toml --target wasm32-wasip2 --release
   ```
2. Add your app to `base/rootfs.sh` so it's included in the initramfs (follow the `hello-world` pattern).
3. Optionally add it to `base/modules/scripts/boot.toml` if it should auto-start at boot.

**Build and test**:
```sh
make apps && make rootfs && make run-gui DISPLAY_BACKEND=cocoa
```

## IPC and Supervisor Commands

Apps communicate with the supervisor and each other via stdout:

```
@supervisor: ps-raw          → REPLY:ps-raw:name:status:uptime:restarts|...
@supervisor: run <app>       → launches the named app
@supervisor: focus <app>     → gives keyboard focus to the named app
@supervisor: raise <app>     → brings app window to front
@supervisor: notify <title> <msg>  → shows a notification toast
@supervisor: ping            → REPLY:pong (use for periodic redraws)
@<app-name>: <message>       → delivers message to that app's stdin
```

Replies arrive on the same app's stdin prefixed with `REPLY:`.

## Color Palette

The OS uses a consistent GitHub dark-mode palette:

| Name | Value | Use |
|------|-------|-----|
| Background | `0x0D1117FF` | Window fill |
| Surface | `0x161B22FF` | Card/panel fill |
| Border | `0x30363DFF` | Borders, dividers |
| Accent | `0x58A6FFFF` | Selection, focus, links |
| Text | `0xE6EDF3FF` | Primary text |
| Dim | `0x8B949EFF` | Secondary text |
| Hint | `0x6E7681FF` | Placeholder, hints |
| Green | `0x3FB950FF` | Running indicators, success |
| Orange | `0xFFA657FF` | Numbers, warnings |

## Capabilities Reference

| Capability | Effect |
|-----------|--------|
| `stdio` | App can read stdin (keyboard) and write stdout (draw + IPC) |
| `filesystem` | App mounts `/data` (9P, persistent across reboots) |
| `network` | App gets WASI sockets (TCP) |
| `display` | App window is registered; gets `VYOMA_SYSTEM:screen:` at startup |
| `shell` | App can issue `@supervisor:` management commands |
| `mouse` | App receives `VYOMA_INPUT:mouse:` events |

## Contribution Areas

### Adding apps

The best way to learn the system and make a meaningful contribution. Pick something useful:
- A terminal emulator that embeds a proper VT100 parser
- A music player that actually plays audio (once audio capability exists)
- A code editor with syntax highlighting
- A real web browser using a WASM-compiled HTML engine
- A game (Tetris, Snake, etc.)

### Supervisor improvements

`supervisor/src/main.rs` is the most impactful file in the repo. Good targets:
- Multi-monitor window placement
- Mouse event routing (clicks → window focus, context menus)
- Animation / compositing (alpha blending, transition effects)
- Proper window chrome (resize handles, window snapping)
- `@supervisor: context-menu` for right-click menus

### Infrastructure

- CI/CD: GitHub Actions workflow that builds + runs a boot smoke test in QEMU
- Testing: a test harness that boots the OS and checks supervisor output
- Kernel: newer LTS version, additional hardware drivers
- WASI: exposing new capabilities (audio, USB, camera) as typed WASI interfaces

### Documentation

- Architecture diagrams (supervisor state machine, IPC flow, display pipeline)
- Tutorial: "Build your first VyomaOS app in 30 minutes"
- Comparison: VyomaOS vs Redox OS vs Unikraft vs Fuchsia

## Code Style

- Rust: standard `rustfmt` defaults, no custom config needed
- Comments: only when the WHY is non-obvious. No docstrings on simple helpers.
- App binaries: keep dependencies minimal — zero dependencies is the goal where possible
- Error handling: apps can panic freely (supervisor catches exits); supervisor code should be robust

## Submitting Changes

1. Fork the repo and create a branch: `git checkout -b feat/my-feature`
2. Make your changes; build and boot to verify
3. Open a pull request against `develop` with a clear description of what and why
4. For new apps: include a brief description in the PR of what the app does and what keys it handles

**Commit message format** (for consistency with the auto-build log):
```
feat(app-name): one-line description

Optional body for non-obvious decisions.
```

## Discussion

Open a GitHub Issue for:
- Feature proposals (new capabilities, new supervisor commands)
- Architecture questions ("should this be in supervisor or an app?")
- Bug reports (include the QEMU serial output)
- Anything that affects multiple apps or the supervisor

The project is early-stage and design decisions are still fluid. There are no wrong questions.
