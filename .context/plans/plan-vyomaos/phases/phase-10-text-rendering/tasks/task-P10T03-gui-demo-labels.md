# P10T03 — Update gui-demo with Text Labels

## Phase
Phase 10 — Text Rendering

## Goal
Rewrite `apps/gui-demo/src/main.rs` to render the OS title, per-app block labels, IPC status, and boot count using `VYOMA_DRAW:draw_text`. The dashboard goes from being coloured boxes to a real status display.

## Files to modify

```
apps/gui-demo/src/main.rs
apps/gui-demo/vyoma.toml    — add filesystem = true (to read boot_count.txt)
```

## Layout plan

```
┌─────────────────────────────────────────────────┐  y=0
│  ▪ VyomaOS                         boot #N      │  header (h=52)
├─────────────────────────────────────────────────┤  y=55
│ ┌──────────┐  ┌──────────┐  ┌──────────┐        │
│ │hello-    │  │calculat- │  │storage-  │        │
│ │world     │  │or        │  │demo      │        │
│ │ ✓ done   │  │ ✓ done   │  │ ✓ done   │        │
│ └──────────┘  └──────────┘  └──────────┘        │  y=80-180
│ ┌──────────────────┐  ┌──────────────────┐      │
│ │ping              │  │pong              │      │
│ │ IPC: 3 msgs      │  │ IPC: 3 msgs      │      │
│ └──────────────────┘  └──────────────────┘      │  y=200-280
│                                                  │
│  gui-demo  display:yes                           │  y=300
├─────────────────────────────────────────────────┤
│ ● running                                        │  footer
└─────────────────────────────────────────────────┘
```

## Implementation

```rust
fn main() {
    let boot_count = read_boot_count();  // from /data/boot_count.txt
    let w = 1280u32;
    let h = 800u32;

    // Background
    fill(0, 0, w, h, 0x0D1117FF);

    // Header
    fill(0, 0, w, 52, 0x161B22FF);
    fill(0, 52, w, 3, 0x58A6FFFF);
    fill(16, 10, 32, 32, 0x3FB950FF);  // logo block
    text(56, 18, 0xFFFFFFFF, "VyomaOS");
    text(w - 120, 18, 0x8B949EFF, &format!("boot #{boot_count}"));

    // App blocks row 1
    let apps1 = [("hello-world", 0x1F6FEBFF), ("calculator", 0x8957E5FF), ("storage-demo", 0xE3B341FF)];
    let col_w = (w - 96) / 3;
    for (i, (name, color)) in apps1.iter().enumerate() {
        let bx = 32 + i as u32 * col_w;
        fill(bx, 80, col_w - 8, 100, *color);
        text(bx + 8, 90,  0xFFFFFFFF, name);
        text(bx + 8, 114, 0xFFFFFFFFu32, "done");
    }

    // IPC row
    fill(32, 200, (w - 96) / 2 - 8, 80, 0x3FB950FF);
    text(40, 210, 0xFFFFFFFF, "ping");
    text(40, 228, 0xFFFFFFFF, "IPC: 3 msgs sent");

    fill(32 + (w - 96) / 2, 200, (w - 96) / 2 - 8, 80, 0xF78166FF);
    text(40 + (w - 96) / 2, 210, 0xFFFFFFFF, "pong");
    text(40 + (w - 96) / 2, 228, 0xFFFFFFFF, "IPC: 3 msgs handled");

    // gui-demo itself
    text(32, 300, 0x58A6FFFF, "gui-demo  display:yes");

    // Footer
    fill(0, h - 56, w, 3, 0x30363DFF);
    fill(0, h - 53, w, 53, 0x0D1117FF);
    fill(16, h - 36, 12, 12, 0x3FB950FF);
    text(36, h - 36, 0x8B949EFF, "running");

    flush();
    eprintln!("gui-demo: frame drawn");
}

fn read_boot_count() -> u64 {
    std::fs::read_to_string("/data/boot_count.txt")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
```

## Notes

- `filesystem = true` is needed in `vyoma.toml` to read `/data/boot_count.txt`
- Screen dimensions are read from supervisor FBIOGET_VSCREENINFO (1280×800 in QEMU); the app currently hard-codes these — a future improvement is to pass dims via an env var or a VYOMA_QUERY command

## Verification

```sh
make run-gui DISPLAY_BACKEND=cocoa
# QEMU window should show:
# - "VyomaOS" title in header
# - "boot #N" counter top-right
# - App name labels in each coloured block
# - "IPC: 3 msgs" in ping/pong blocks
```
