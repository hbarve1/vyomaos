# P09T03 — `vyoma:display` WIT interface definition

## What
Define the WIT interface that WASM apps use to draw to the screen. Generate Rust
bindings for both the host (supervisor) and guest (WASM app) sides.

## WIT definition — `wit/vyoma-display.wit`

```wit
package vyoma:display@0.1.0;

interface canvas {
  /// Fill a rectangle with a solid colour (RGBA packed as 0xRRGGBBAA).
  fill-rect: func(x: u32, y: u32, w: u32, h: u32, rgba: u32);

  /// Draw a UTF-8 string using the embedded bitmap font.
  draw-text: func(x: u32, y: u32, text: string, rgba: u32);

  /// Blit a raw RGBA pixel buffer at (x, y) with the given dimensions.
  blit: func(x: u32, y: u32, w: u32, h: u32, pixels: list<u8>);

  /// Push the back-buffer to the screen.
  flush: func();

  /// Return the framebuffer width in pixels.
  width: func() -> u32;

  /// Return the framebuffer height in pixels.
  height: func() -> u32;
}

/// World for apps that use the display.
world display-app {
  import canvas;
  export run: func();
}
```

## Rust host bindings (supervisor)
Use `wasmtime::component::bindgen!` macro:
```rust
wasmtime::component::bindgen!({
    world: "display-app",
    path:  "wit/vyoma-display.wit",
});
```

Implement the `CanvasHost` trait on a `FramebufferState` struct that holds the
mmap'd framebuffer pointer and screen dimensions.

## Rust guest bindings (WASM app)
Use `wit-bindgen` in `apps/gui-demo/`:
```toml
[build-dependencies]
wit-bindgen = "0.36"
```
```rust
wit_bindgen::generate!({ world: "display-app", path: "../../wit/vyoma-display.wit" });
```

## Location
`wit/vyoma-display.wit` at the project root (sibling to `supervisor/`, `apps/`).

## Gate
`cargo check` on both supervisor and apps/gui-demo succeeds with generated bindings.
