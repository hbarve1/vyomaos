# P09T05 — `gui-demo` WASM app

## What
A `wasm32-wasip2` app that calls the `vyoma:display/canvas` host interface to render
a static frame: dark background, coloured rectangle, and "Hello VyomaOS" text.

## App structure

```
apps/gui-demo/
  Cargo.toml
  src/main.rs
  vyoma.toml
```

### vyoma.toml
```toml
[app]
name    = "gui-demo"
version = "0.1.0"
wasm    = "gui-demo.wasm"

[capabilities]
stdio      = false
filesystem = false
network    = false
display    = true
```

### src/main.rs (guest side)
```rust
wit_bindgen::generate!({
    world: "display-app",
    path:  "../../wit/vyoma-display.wit",
});

struct GuidemoApp;

impl Guest for GuidemoApp {
    fn run() {
        let w = canvas::width();
        let h = canvas::height();

        // Dark background
        canvas::fill_rect(0, 0, w, h, 0x1a1a2eff);

        // Accent rectangle
        canvas::fill_rect(40, 40, w - 80, 60, 0x5865f2ff);

        // Title text
        canvas::draw_text(56, 56, "VyomaOS", 0xffffffff);

        // Subtitle
        canvas::draw_text(40, 130, "WASM-native OS — Phase 09", 0xaaaabbff);

        canvas::flush();
    }
}

export!(GuidemoApp);
```

### boot.toml entry
```toml
[[apps]]
manifest = "/apps/gui-demo/vyoma.toml"
restart  = "never"
```

## Gate
QEMU SDL window shows:
- Dark (#1a1a2e) background
- Blue (#5865f2) banner rectangle
- White "VyomaOS" text
- Grey subtitle text

Serial console shows `vyoma-supervisor: gui-demo exited cleanly`.

## Notes
- `display=true` apps are launched via the Wasmtime Component Model path, not the
  `Command`-based path.  The supervisor detects `capabilities.display=true` and
  switches launch strategy.
- `stdio=false` is intentional: gui-demo writes nothing to stdout/stderr.
- Colours use RGBA packed as 0xRRGGBBAA matching the WIT interface definition.
