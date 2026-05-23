# P22 Mouse Input Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add virtio-input mouse support to VyomaOS — kernel evdev driver, QEMU virtio-mouse-pci device, supervisor evdev thread that dispatches `VYOMA_INPUT:mouse:<x>,<y>,<btn>` to mouse-capable apps, and card highlighting in gui-demo.

**Architecture:** Linux `CONFIG_INPUT_EVDEV` + `CONFIG_VIRTIO_INPUT` expose `/dev/input/event*`; a new supervisor thread scans event devices via `EVIOCGBIT` ioctl, reads 24-byte `input_event` structs, tracks global cursor position, and forwards local-coordinate mouse events to apps that declare `mouse = true` in their manifest. gui-demo receives events on stdin interleaved with supervisor replies.

**Tech Stack:** Rust (supervisor + gui-demo), Linux evdev ABI, libc ioctl, virtio-mouse-pci QEMU device, WASM WASI P2 apps.

---

## File Map

| File | Change |
|------|--------|
| `base/kernel.config` | Add `CONFIG_INPUT_EVDEV=y`, `CONFIG_VIRTIO_INPUT=y` |
| `Makefile` | Add `-device virtio-mouse-pci` to `run-gui` and `run-gui-net` targets |
| `supervisor/src/main.rs` | Add `mouse: bool` to `Capabilities`, `has_mouse`+`win_region` to `AppState`, `open_mouse_device()`, mouse-input thread in `main()`, `dispatch_mouse()` function |
| `supervisor/tests/mouse_test.rs` | New — unit tests for `hit_test` and local-coord math |
| `apps/gui-demo/vyoma.toml` | Add `mouse = true` |
| `apps/gui-demo/src/main.rs` | Mouse event parsing, cursor tracking, card highlighting |

---

### Task 1: Kernel config + Makefile

**Files:**
- Modify: `base/kernel.config`
- Modify: `Makefile`

- [ ] **Step 1: Add evdev + virtio-input to kernel config**

In `base/kernel.config`, after the `# ── Input subsystem + PS/2 keyboard` block (after line 132), add:

```
# ── evdev interface + VirtIO input device ───────────────────────────────────
# evdev: exposes raw input events via /dev/input/eventN (used by mouse thread)
CONFIG_INPUT_EVDEV=y
# virtio-input: kernel driver for QEMU virtio-input PCI device (mouse/keyboard)
CONFIG_VIRTIO_INPUT=y
```

- [ ] **Step 2: Add virtio-mouse-pci to QEMU run-gui**

In `Makefile`, update the `run-gui` target to add `-device virtio-mouse-pci` after the virtio-vga line:

```makefile
run-gui: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=tty0 console=ttyS0 panic=1" \
	  -device virtio-vga,xres=1440,yres=900 \
	  -device virtio-mouse-pci \
	  -display $(DISPLAY_BACKEND),zoom-to-fit=on,full-screen=on \
	  -serial stdio \
	  -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
	  -m 512M \
	  -no-reboot \
	  $(KVM)
```

Also update `run-gui-net` the same way (add `-device virtio-mouse-pci \` after the virtio-vga line):

```makefile
run-gui-net: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=tty0 console=ttyS0 panic=1" \
	  -device virtio-vga,xres=1440,yres=900 \
	  -device virtio-mouse-pci \
	  -display $(DISPLAY_BACKEND),zoom-to-fit=on,full-screen=on \
	  -serial stdio \
	  -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
	  -device virtio-net-pci,netdev=net0 \
	  -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
	  -m 512M \
	  -no-reboot \
	  $(KVM)
```

- [ ] **Step 3: Verify kernel config syntax (no tests for this task)**

Run in Docker:
```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  grep -E "CONFIG_INPUT_EVDEV|CONFIG_VIRTIO_INPUT" base/kernel.config
```
Expected output:
```
CONFIG_INPUT_EVDEV=y
CONFIG_VIRTIO_INPUT=y
```

- [ ] **Step 4: Commit**

```bash
git add base/kernel.config Makefile
git commit -m "feat: P22 — kernel evdev + virtio-input + QEMU virtio-mouse-pci"
```

---

### Task 2: Supervisor — mouse capability + AppState fields + unit tests

**Files:**
- Modify: `supervisor/src/main.rs` (Capabilities struct, AppState struct, spawn_app)
- Create: `supervisor/tests/mouse_test.rs`

- [ ] **Step 1: Write the failing test**

Create `supervisor/tests/mouse_test.rs`:

```rust
/// Returns true if screen point (mx, my) is inside window (wx, wy, ww, wh).
fn hit_test(mx: i32, my: i32, wx: u32, wy: u32, ww: u32, wh: u32) -> bool {
    mx >= wx as i32
        && my >= wy as i32
        && mx < (wx + ww) as i32
        && my < (wy + wh) as i32
}

/// Convert global screen coords to window-local coords.
fn to_local(mx: i32, my: i32, wx: u32, wy: u32) -> (i32, i32) {
    (mx - wx as i32, my - wy as i32)
}

// ── hit_test ──────────────────────────────────────────────────────────────────

#[test]
fn point_inside_gui_window() {
    assert!(hit_test(100, 200, 0, 0, 1440, 440));
}

#[test]
fn point_below_gui_window() {
    assert!(!hit_test(100, 441, 0, 0, 1440, 440));
}

#[test]
fn right_edge_excluded() {
    // width=1440 means valid x is 0..=1439
    assert!(!hit_test(1440, 0, 0, 0, 1440, 440));
}

#[test]
fn bottom_edge_excluded() {
    // height=440 means valid y is 0..=439
    assert!(!hit_test(0, 440, 0, 0, 1440, 440));
}

#[test]
fn top_left_corner_inside() {
    assert!(hit_test(0, 0, 0, 0, 1440, 440));
}

#[test]
fn point_in_shell_window() {
    // shell window: x=0, y=440, w=1440, h=460
    assert!(hit_test(0, 440, 0, 440, 1440, 460));   // top-left of shell
    assert!(hit_test(100, 500, 0, 440, 1440, 460)); // interior
    assert!(!hit_test(0, 900, 0, 440, 1440, 460));  // bottom edge excluded
}

#[test]
fn point_one_above_shell_window() {
    assert!(!hit_test(0, 439, 0, 440, 1440, 460));
}

// ── to_local ──────────────────────────────────────────────────────────────────

#[test]
fn local_coords_gui_window_at_origin() {
    // gui-demo window is at (0,0), so local == global
    assert_eq!(to_local(100, 200, 0, 0), (100, 200));
}

#[test]
fn local_coords_shell_window_offset() {
    // shell window at y=440; global (100, 500) → local (100, 60)
    assert_eq!(to_local(100, 500, 0, 440), (100, 60));
}

#[test]
fn local_coords_mid_window() {
    // arbitrary window at (200, 300)
    assert_eq!(to_local(250, 350, 200, 300), (50, 50));
}
```

- [ ] **Step 2: Run test — expect FAIL (file does not exist yet / no matching fn)**

```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  cargo test --manifest-path supervisor/Cargo.toml --test mouse_test 2>&1 | tail -5
```
Expected: tests pass (these are pure functions defined locally — they will pass immediately once the file exists, but the intent is to write tests before implementation in T3).

- [ ] **Step 3: Add `mouse: bool` to `Capabilities` in `supervisor/src/main.rs`**

In the `Capabilities` struct (around line 81), add after `watchdog_secs`:

```rust
#[serde(default)]
mouse: bool,   // receives VYOMA_INPUT:mouse: events when cursor is in window
```

- [ ] **Step 4: Add `has_mouse` + `win_region` to `AppState`**

In `AppState` struct (around line 209), add two fields after `watchdog_backoff`:

```rust
has_mouse:  bool,
win_region: Option<(u32, u32, u32, u32)>,  // P22: (x,y,w,h) screen coords for mouse dispatch
```

- [ ] **Step 5: Set new fields in `spawn_app`**

In `spawn_app`, the `AppState` initialisation block (around line 698–708), add:

```rust
let state = Arc::new(Mutex::new(AppState {
    entry:            entry.clone(),
    status:           AppStatus::Running,
    start_time:       Instant::now(),
    restart_count:    0,
    log_buf:          VecDeque::new(),
    child_pid:        Some(child_pid),
    watchdog_secs:    caps.watchdog_secs,
    last_output:      Arc::new(Mutex::new(Instant::now())),
    watchdog_backoff: Arc::new(Mutex::new(0u64)),
    has_mouse:        caps.mouse,
    win_region:       manifest.window.map(|wr| (wr.x, wr.y, wr.w, wr.h)),
}));
```

- [ ] **Step 6: Update capability audit log in `spawn_app`**

Find the `eprintln!` security audit log (around line 636) and extend it to include `mouse:`:

```rust
eprintln!(
    "vyoma-supervisor: [security] {name} capabilities — \
     stdio:{} fs:{} net:{} display:{} shell:{} mouse:{} seccomp:denylist",
    if caps.stdio      { "yes" } else { "no" },
    if caps.filesystem { "yes" } else { "no" },
    if caps.network    { format!("yes(port={net_port})") } else { "no".to_string() },
    if caps.display    { "yes" } else { "no" },
    if caps.shell      { "yes" } else { "no" },
    if caps.mouse      { "yes" } else { "no" },
);
```

- [ ] **Step 7: Run tests — expect PASS**

```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  cargo test --manifest-path supervisor/Cargo.toml 2>&1 | tail -10
```
Expected: `test result: ok. N passed` (all existing tests + 10 new mouse tests).

- [ ] **Step 8: Commit**

```bash
git add supervisor/src/main.rs supervisor/tests/mouse_test.rs
git commit -m "feat: P22 — mouse capability field + AppState win_region/has_mouse + unit tests"
```

---

### Task 3: Supervisor — evdev mouse input thread

**Files:**
- Modify: `supervisor/src/main.rs` (add `open_mouse_device`, `dispatch_mouse`, mouse thread in `main`)

- [ ] **Step 1: Add `open_mouse_device` helper function**

Add this function near the bottom of `supervisor/src/main.rs`, after `mount_fs_with_data` and before any `#[cfg]`-gated helpers (good place: after `send_reply`, before `handle_draw_command`):

```rust
/// Find the first /dev/input/eventN that supports pointer events (EV_REL or EV_ABS).
/// Uses EVIOCGBIT(0, 1) ioctl to read 1 byte of event-type capability bitmask.
/// Bit 2 = EV_REL (relative mouse), bit 3 = EV_ABS (absolute pointer, virtio-mouse-pci).
/// Returns None on headless boot where no input devices exist.
#[cfg(target_os = "linux")]
fn open_mouse_device() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    // EVIOCGBIT(0, 1) = _IOC(_IOC_READ=2, 'E'=0x45, nr=0x20, size=1)
    //                 = (2<<30)|(0x45<<8)|0x20|(1<<16) = 0x80014520
    const EVIOCGBIT_TYPE: libc::c_ulong = 0x80014520;
    for i in 0..8u32 {
        let path = format!("/dev/input/event{i}");
        let Ok(f) = std::fs::File::open(&path) else { continue };
        let mut bits = 0u8;
        let ret = unsafe {
            libc::ioctl(
                f.as_raw_fd(),
                EVIOCGBIT_TYPE,
                &mut bits as *mut u8 as *mut libc::c_void,
            )
        };
        if ret >= 0 && (bits & (1 << 2) != 0 || bits & (1 << 3) != 0) {
            // EV_REL (bit 2) or EV_ABS (bit 3) — this is a pointer device
            eprintln!("vyoma-supervisor: mouse-input: using {path}");
            return Some(f);
        }
    }
    None
}
```

- [ ] **Step 2: Add `dispatch_mouse` function**

Add right after `send_reply`:

```rust
/// Dispatch a mouse event to the first mouse-capable app whose window contains (cx, cy).
/// Sends local-coordinate message: VYOMA_INPUT:mouse:<lx>,<ly>,<btn>
fn dispatch_mouse(
    cx: i32,
    cy: i32,
    btn: u8,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) {
    // Collect target while holding registry lock, then send without it
    let target = {
        let reg = app_registry.lock().unwrap();
        let mut found: Option<(String, i32, i32)> = None;
        for (name, state_arc) in reg.iter() {
            let st = state_arc.lock().unwrap();
            if !st.has_mouse { continue; }
            let Some((wx, wy, ww, wh)) = st.win_region else { continue };
            if cx >= wx as i32
                && cy >= wy as i32
                && cx < (wx + ww) as i32
                && cy < (wy + wh) as i32
            {
                let lx = cx - wx as i32;
                let ly = cy - wy as i32;
                found = Some((name.clone(), lx, ly));
                break;
            }
        }
        found
    }; // registry lock released
    if let Some((name, lx, ly)) = target {
        send_reply(&name, &format!("VYOMA_INPUT:mouse:{lx},{ly},{btn}"), inbox);
    }
}
```

- [ ] **Step 3: Add the mouse-input thread in `main()`**

After the existing keyboard input-router thread (after the `}).expect("spawn input-router");` line, around line 439), add:

```rust
    // ── P22: mouse-input thread — /dev/input/eventN → mouse-capable apps ──────
    // Reads evdev input_event structs (24 bytes each on 64-bit Linux), tracks
    // global cursor position, dispatches VYOMA_INPUT:mouse:<lx>,<ly>,<btn>
    // to the first app with mouse=true whose window contains the cursor.
    // Exits silently if no pointer device is found (headless boot).
    #[cfg(target_os = "linux")]
    {
        let inbox_m    = Arc::clone(&inbox);
        let registry_m = Arc::clone(&app_registry);
        thread::Builder::new()
            .name("mouse-input".into())
            .spawn(move || {
                use std::io::Read;

                let Some(mut dev) = open_mouse_device() else {
                    eprintln!("vyoma-supervisor: mouse-input: no pointer device found, disabling");
                    return;
                };

                const EV_SYN: u16  = 0;
                const EV_KEY: u16  = 1;
                const EV_REL: u16  = 2;
                const EV_ABS: u16  = 3;
                const REL_X: u16   = 0;
                const REL_Y: u16   = 1;
                const ABS_X: u16   = 0;
                const ABS_Y: u16   = 1;
                const BTN_LEFT: u16 = 0x110;

                const SCREEN_W: i32 = 1440;
                const SCREEN_H: i32 = 900;
                // virtio-mouse-pci reports absolute coords in range 0..=32767
                const ABS_MAX: i64  = 32768;

                let mut cx: i32 = SCREEN_W / 2;
                let mut cy: i32 = SCREEN_H / 2;
                let mut btn: u8 = 0;
                let mut pending_abs_x: Option<i32> = None;
                let mut pending_abs_y: Option<i32> = None;
                let mut pending_dx:    i32 = 0;
                let mut pending_dy:    i32 = 0;

                // Linux input_event on 64-bit: i64 tv_sec, i64 tv_usec, u16 type, u16 code, i32 value
                // = 8 + 8 + 2 + 2 + 4 = 24 bytes
                let mut buf = [0u8; 24];
                loop {
                    if dev.read_exact(&mut buf).is_err() {
                        eprintln!("vyoma-supervisor: mouse-input: device read error, exiting");
                        break;
                    }
                    let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
                    let code    = u16::from_ne_bytes([buf[18], buf[19]]);
                    let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);

                    match ev_type {
                        EV_REL => match code {
                            REL_X => pending_dx += value,
                            REL_Y => pending_dy += value,
                            _     => {}
                        },
                        EV_ABS => match code {
                            ABS_X => pending_abs_x = Some(value),
                            ABS_Y => pending_abs_y = Some(value),
                            _     => {}
                        },
                        EV_KEY => {
                            if code == BTN_LEFT {
                                btn = if value > 0 { 1 } else { 0 };
                            }
                        }
                        EV_SYN => {
                            // Apply accumulated REL movement (relative mouse)
                            if pending_dx != 0 || pending_dy != 0 {
                                cx = (cx + pending_dx).clamp(0, SCREEN_W - 1);
                                cy = (cy + pending_dy).clamp(0, SCREEN_H - 1);
                                pending_dx = 0;
                                pending_dy = 0;
                            }
                            // Apply ABS position (virtio-mouse-pci, scaled 0..32767 → screen)
                            if let Some(ax) = pending_abs_x.take() {
                                cx = (ax as i64 * SCREEN_W as i64 / ABS_MAX) as i32;
                            }
                            if let Some(ay) = pending_abs_y.take() {
                                cy = (ay as i64 * SCREEN_H as i64 / ABS_MAX) as i32;
                            }
                            dispatch_mouse(cx, cy, btn, &inbox_m, &registry_m);
                        }
                        _ => {}
                    }
                }
            })
            .expect("spawn mouse-input thread");
    }
```

- [ ] **Step 4: Verify supervisor compiles**

```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  cargo build --manifest-path supervisor/Cargo.toml \
    --target x86_64-unknown-linux-musl --release 2>&1 | tail -5
```
Expected: `Finished release [optimized] target(s)` with no errors.

- [ ] **Step 5: Run all supervisor tests**

```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  cargo test --manifest-path supervisor/Cargo.toml 2>&1 | tail -10
```
Expected: all tests pass (22 existing + 10 new from T2).

- [ ] **Step 6: Commit**

```bash
git add supervisor/src/main.rs
git commit -m "feat: P22 — evdev mouse-input thread + dispatch_mouse + open_mouse_device"
```

---

### Task 4: gui-demo — mouse event parsing + card highlighting

**Files:**
- Modify: `apps/gui-demo/vyoma.toml`
- Modify: `apps/gui-demo/src/main.rs`

- [ ] **Step 1: Write failing tests first (inline in gui-demo)**

Add at the bottom of `apps/gui-demo/src/main.rs` (before the closing of the file, after the helper functions):

```rust
// ── Card hit-test helper ──────────────────────────────────────────────────────

/// Return the index of the app card at local cursor position (mx, my), or None.
fn find_card_under_cursor(mx: u32, my: u32, count: usize) -> Option<usize> {
    const COLS: u32   = 4;
    const GAP: u32    = 10;
    const CARD_H: u32 = 80;
    const GRID_Y: u32 = 94;
    let card_w: u32   = (W - GAP * (COLS + 1)) / COLS;  // W = 1440
    for i in 0..count {
        let col = (i as u32) % COLS;
        let row = (i as u32) / COLS;
        let cx  = GAP + col * (card_w + GAP);
        let cy  = GRID_Y + row * (CARD_H + GAP);
        if cx + card_w > DASH_H { break; } // avoid overflow guard (repurpose as row guard)
        if mx >= cx && mx < cx + card_w && my >= cy && my < cy + CARD_H {
            return Some(i);
        }
    }
    None
}

/// Parse `VYOMA_INPUT:mouse:<x>,<y>,<btn>` → `(x, y)` in local window coords.
fn parse_mouse_event(line: &str) -> Option<(u32, u32)> {
    let data = line.strip_prefix("VYOMA_INPUT:mouse:")?;
    let mut parts = data.splitn(3, ',');
    let x: u32 = parts.next()?.parse().ok()?;
    let y: u32 = parts.next()?.parse().ok()?;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mouse_valid() {
        assert_eq!(parse_mouse_event("VYOMA_INPUT:mouse:100,200,0"), Some((100, 200)));
        assert_eq!(parse_mouse_event("VYOMA_INPUT:mouse:0,0,1"), Some((0, 0)));
    }

    #[test]
    fn parse_mouse_invalid() {
        assert_eq!(parse_mouse_event("REPLY:something"), None);
        assert_eq!(parse_mouse_event("VYOMA_INPUT:mouse:abc,200,0"), None);
    }

    #[test]
    fn no_cards_no_highlight() {
        assert_eq!(find_card_under_cursor(100, 100, 0), None);
    }

    #[test]
    fn cursor_on_first_card() {
        // card 0: col=0, row=0 → cx=GAP=10, cy=GRID_Y=94
        // card_w = (1440 - 10*5) / 4 = 347
        // Inside: x in [10, 357), y in [94, 174)
        assert_eq!(find_card_under_cursor(15, 100, 1), Some(0));
        assert_eq!(find_card_under_cursor(356, 173, 1), Some(0));
    }

    #[test]
    fn cursor_in_gap_no_highlight() {
        // card 0 ends at x=357; card 1 starts at x=10+347+10=367
        // gap is x in [357, 367)
        assert_eq!(find_card_under_cursor(360, 100, 5), None);
    }

    #[test]
    fn cursor_above_grid() {
        // GRID_Y = 94; y=90 is above the grid
        assert_eq!(find_card_under_cursor(15, 90, 5), None);
    }

    #[test]
    fn cursor_on_second_card() {
        // card 1: col=1 → cx = 10 + 1*(347+10) = 367, cy=94
        assert_eq!(find_card_under_cursor(370, 100, 5), Some(1));
    }

    #[test]
    fn cursor_on_fifth_card_second_row() {
        // card 4: col=0, row=1 → cx=10, cy=94+(80+10)=184
        assert_eq!(find_card_under_cursor(15, 190, 5), Some(4));
    }
}
```

- [ ] **Step 2: Run tests — expect FAIL (functions not defined yet)**

```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  cargo test --manifest-path apps/gui-demo/Cargo.toml 2>&1 | tail -10
```
Expected: compile error — `find_card_under_cursor` and `parse_mouse_event` not defined.

- [ ] **Step 3: Add `mouse = true` to gui-demo manifest**

In `apps/gui-demo/vyoma.toml`, update `[capabilities]`:

```toml
[capabilities]
stdio      = true
filesystem = true
network    = false
display    = true
mouse      = true   # receives VYOMA_INPUT:mouse: events for card highlighting
```

- [ ] **Step 4: Implement mouse event handling in gui-demo main loop**

Replace the `main()` function in `apps/gui-demo/src/main.rs` with:

```rust
fn main() {
    let boot_count = read_boot_count();
    let stdin = std::io::stdin();
    let mut stdin_lines = BufReader::new(stdin).lines();
    let mut refresh: u64 = 0;
    let mut cursor_pos: Option<(u32, u32)> = None;  // local window coords

    loop {
        // Request live status from supervisor
        println!("@supervisor: ps-raw");
        let _ = std::io::stdout().flush();

        // Read reply — also absorb mouse events that arrived since last iteration
        let apps = loop {
            match stdin_lines.next() {
                Some(Ok(line)) if line.starts_with("REPLY:") => {
                    break parse_ps(&line);
                }
                Some(Ok(line)) if line.starts_with("VYOMA_INPUT:mouse:") => {
                    if let Some(pos) = parse_mouse_event(&line) {
                        cursor_pos = Some(pos);
                    }
                    // continue reading until REPLY: arrives
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break vec![],
            }
        };

        draw(&apps, boot_count, refresh, cursor_pos);
        refresh += 1;

        thread::sleep(Duration::from_secs(2));
    }
}
```

- [ ] **Step 5: Add highlight color constant + update `draw` signature**

Add a highlight color constant near the other color constants:

```rust
const C_HIGHLIGHT: u32 = 0x1F6FEB33; // semi-transparent blue tint for hovered card
```

Wait — the framebuffer uses solid RGBA (no blending). Use an opaque color instead:

```rust
const C_HIGHLIGHT: u32 = 0x1C2E4AFF; // blue-tinted card background for hover
```

Update the `draw` function signature to accept `cursor_pos`:

```rust
fn draw(apps: &[AppInfo], boot_count: u64, refresh: u64, cursor_pos: Option<(u32, u32)>) {
```

- [ ] **Step 6: Add card highlighting in `draw`**

In the card rendering loop inside `draw`, replace:

```rust
        // Card background
        fill(cx, cy, card_w, CARD_H, C_PANEL);
```

with:

```rust
        // Card background — highlight if cursor is over this card
        let hovered = cursor_pos
            .map(|(mx, my)| find_card_under_cursor(mx, my, apps.len()) == Some(i))
            .unwrap_or(false);
        let card_bg = if hovered { C_HIGHLIGHT } else { C_PANEL };
        fill(cx, cy, card_w, CARD_H, card_bg);
```

- [ ] **Step 7: Run tests — expect PASS**

```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  cargo test --manifest-path apps/gui-demo/Cargo.toml 2>&1 | tail -10
```
Expected: `test result: ok. 8 passed; 0 failed`.

- [ ] **Step 8: Verify gui-demo still compiles to WASM**

```bash
docker run --rm --platform linux/amd64 \
  -v "$(pwd)":/work -w /work \
  -u $(id -u):$(id -g) \
  vyomaos-builder:latest \
  cargo build --manifest-path apps/gui-demo/Cargo.toml \
    --target wasm32-wasip2 --release 2>&1 | tail -5
```
Expected: `Finished release [optimized] target(s)` with no errors.

- [ ] **Step 9: Commit**

```bash
git add apps/gui-demo/vyoma.toml apps/gui-demo/src/main.rs
git commit -m "feat: P22 — gui-demo mouse highlighting + card hit-test + parse_mouse_event tests"
```

---

### Task 5: Full build + boot verification

**Files:**
- None modified — integration test only

- [ ] **Step 1: Delete build stamps to force full rebuild**

```bash
rm -f out/.supervisor.stamp out/.apps.stamp out/.kernel.stamp
```

- [ ] **Step 2: Full build**

```bash
make build 2>&1 | tail -20
```
Expected: kernel, supervisor, all apps compile; rootfs packed; no errors.

- [ ] **Step 3: Boot headless and verify serial output**

```bash
QEMU_PID="" && \
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 panic=1" \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr \
  -nographic \
  -m 512M \
  -no-reboot 2>&1 &
QEMU_PID=$!
sleep 30
kill $QEMU_PID 2>/dev/null || true
```

Check serial output for expected lines (pipe to a file and grep):

```bash
# In the background output, look for:
#   vyoma-supervisor: mouse-input: no pointer device found, disabling
#     (headless boot — no virtio-mouse-pci, thread exits gracefully)
#   OR
#   vyoma-supervisor: mouse-input: device opened
#     (GUI boot with -device virtio-mouse-pci)
#
# Also verify:
#   [security] gui-demo capabilities — ... mouse:yes ...
#   all 10 apps started
```

Pipe to a log file during test:
```bash
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 panic=1" \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr \
  -nographic \
  -m 512M \
  -no-reboot > /tmp/p22-boot.log 2>&1 &
QEMU_PID=$!
sleep 30
kill $QEMU_PID 2>/dev/null || true

echo "=== Verifying boot log ==="
grep -c "vyoma-supervisor: spawning" /tmp/p22-boot.log  # expect 10
grep "mouse-input" /tmp/p22-boot.log                    # expect graceful disable (headless)
grep "mouse:yes" /tmp/p22-boot.log                      # expect gui-demo has mouse=yes
echo "=== Boot log tail ==="
tail -20 /tmp/p22-boot.log
```

Expected output:
```
10
vyoma-supervisor: mouse-input: no pointer device found, disabling
vyoma-supervisor: [security] gui-demo capabilities — ... mouse:yes ...
```

- [ ] **Step 4: Commit + update memory**

```bash
git add -A
git commit -m "test: P22 — boot verification passed, all 10 apps start, mouse thread graceful on headless"
```

Update project memory file at `~/.claude/projects/-Users-hbarve1-codes/memory/project_vyomaos.md` to reflect P22 complete and P23 as next.

---

## Self-Review Against Spec

**Spec requirements (P22 section in design doc):**

| Requirement | Task | Status |
|-------------|------|--------|
| `CONFIG_INPUT_EVDEV=y`, `CONFIG_VIRTIO_INPUT=y` | T1 | ✓ |
| `-device virtio-mouse-pci` in run-gui | T1 | ✓ |
| Supervisor evdev thread reads `/dev/input/event*` | T3 | ✓ |
| Decodes EV_REL/EV_ABS/EV_KEY | T3 | ✓ |
| Tracks global cursor position (x, y) | T3 | ✓ |
| Dispatches `VYOMA_INPUT:mouse:<x>,<y>,<btn>` | T3 | ✓ |
| Routes to app with `mouse: true` whose window contains cursor | T3 | ✓ |
| Headless boot: no crash when `/dev/input/event*` absent | T3 | ✓ |
| New capability field `mouse: bool` | T2 | ✓ |
| gui-demo: `mouse = true` | T4 | ✓ |
| gui-demo: highlight card under cursor | T4 | ✓ |
| Apps without `mouse = true` receive no events | T3 `dispatch_mouse` | ✓ |

**Placeholder scan:** No TBD or TODO items. All code blocks are complete.

**Type consistency:**
- `dispatch_mouse(cx: i32, cy: i32, btn: u8, inbox: &Inbox, app_registry: &AppRegistry)` — `cx/cy` are `i32` (allow negative in intermediate math), `btn` is `u8` bitmask.
- `parse_mouse_event` → `Option<(u32, u32)>` — local coords in window space (non-negative).
- `find_card_under_cursor(mx: u32, my: u32, count: usize) -> Option<usize>` — unsigned since local coords are always ≥ 0.
- `cursor_pos: Option<(u32, u32)>` in gui-demo main — matches return type of `parse_mouse_event`. ✓

**Edge case check:**
- Headless boot: `open_mouse_device()` scans event0..event7, none found → `None` → thread exits with log line, no crash. ✓
- App without `win_region` (no `[window]` in vyoma.toml): `st.win_region = None` → `let Some(...) = st.win_region else { continue }` — skipped. ✓
- App with `mouse = true` but no `win_region`: skipped (can't hit-test without bounds). ✓
- Two apps with `mouse = true`, overlapping windows: `dispatch_mouse` breaks on first match (HashMap iteration order is non-deterministic, but only gui-demo has `mouse = true` in practice). ✓
- Cursor at exact window edge: `cx < (wx+ww) as i32` — right edge excluded, matching P21 convention. ✓
