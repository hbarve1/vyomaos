# Round 17 FINAL — VYOMA_DRAW v3 Advanced Rendering
## VyomaOS Subsystem 17: Advanced 2D Rendering API (macOS equiv: Quartz 2D / CoreGraphics)

**Status**: FINAL — all 5 blocking issues resolved
**Date**: 2026-05-29
**Blocking issues resolved**: B1 (v2 rate limit + v3 migration path), B2 (rasterizer threading model), B3 (protocol state recovery), B4 (color space contract), B5 (WIT batch path submission)

---

## Overview

VYOMA_DRAW v3 is the complete 2D rendering API for VyomaOS, covering three tiers:

- **v1** (`VYOMA_DRAW:` text, R11): `fill_rect`, `draw_text`, `flush` — simple status/info apps, retained as-is
- **v2** (`VYOMA_DRAW:` text extensions): Paths, gradients, transforms via text lines. **Rate-limited: < 30Hz or < 20 path segments/frame.** Not suitable for 60Hz animated UI. Use as a stepping stone to v3 only.
- **v3** (`vyoma:draw@3.0.0` WIT + `submit-path-buffer` for batched paths): Production UI, animations, complex documents. Apps with any animated drawing MUST use v3.

**Why v3 exists (B1)**: A moderately complex UI frame draws ~66 text lines. At 60Hz with 4 apps = ~15,840 stdout lines/sec = ~24,000 f32 parse operations/sec through the supervisor's string parser. CoreGraphics passes a binary `CGPathRef`. v3 uses either direct WIT calls (for small paths) or a binary-packed buffer via `submit-path-buffer` (for large paths). This is not a premature optimization — it is the only viable architecture for 60Hz production UI.

---

## B4 Fix: Color Space Contract (applies to all color parameters throughout)

**Normative rule — applies to ALL `vyoma:draw@3.0.0` WIT functions and v2 text commands:**

> All color values are **linear-light sRGB with premultiplied alpha**, f32 range [0.0, 1.0].
> Apps working with sRGB (gamma-encoded) u8 values (e.g., `0xFF8000FF` from v1) MUST convert via R15 `color-convert` before calling v2/v3 draw functions. Passing gamma-encoded values as linear produces colors 2× darker than expected.

Conversion helper (document in SDK):
```rust
// sRGB u8 (0–255) → linear-light f32
fn srgb_u8_to_linear(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}
// Premultiply after linearizing:
// r_pm = r_linear * alpha_linear
```

This rule applies to: `set-fill-color`, `set-stroke-color`, `add-gradient-stop`, `fill:` (v2), `stroke:` (v2), `gradient_stop:` (v2), `set-background-color` in v2 `draw_image` tint.

---

## Core Rust Types

```rust
// supervisor/src/draw/mod.rs

pub type CtxHandle = u32;
pub type Pid = u32;

// B4: All color f32 values are linear-light sRGB, premultiplied alpha
#[derive(Clone, Debug)]
pub struct LinearColor {
    pub r: f32, pub g: f32, pub b: f32, pub a: f32,  // premultiplied, [0.0, 1.0]
}

#[derive(Clone, Debug)]
pub struct AffineTransform {
    pub a: f32, pub b: f32,
    pub c: f32, pub d: f32,
    pub tx: f32, pub ty: f32,
}

impl AffineTransform {
    pub fn identity() -> Self { AffineTransform { a:1.0, b:0.0, c:0.0, d:1.0, tx:0.0, ty:0.0 } }
    pub fn transform_point(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
    pub fn concat(&self, other: &AffineTransform) -> AffineTransform { /* matrix multiply */ *self }
}

#[derive(Clone, Debug)]
pub enum PathSegment {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CurveTo(f32, f32, f32, f32, f32, f32),   // cp1x, cp1y, cp2x, cp2y, ex, ey
    QuadTo(f32, f32, f32, f32),              // cpx, cpy, ex, ey
    ArcTo { cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32 },
    Close,
}

pub const MAX_PATH_SEGMENTS: usize = 10_000;
pub const MAX_GRADIENT_STOPS: usize = 64;
pub const MAX_SAVE_DEPTH: usize = 64;
pub const PATH_STATE_TIMEOUT_MS: u64 = 100;  // B3: discard open path after 100ms

#[derive(Clone, Debug)]
pub struct VyomaPath {
    pub segments: Vec<PathSegment>,
    // N2 FIX: bbox in device space (post-transform) to avoid stale cache on transform change
    pub device_bbox: Option<[f32; 4]>,
    pub transform_at_cache: Option<AffineTransform>,
}

#[derive(Clone, Debug)]
pub struct GradientStop {
    pub position: f32,          // 0.0..=1.0
    pub color: LinearColor,     // B4: linear-light sRGB
}

#[derive(Clone, Debug)]
pub enum VyomaPaint {
    SolidColor(LinearColor),    // B4: linear-light sRGB, premultiplied
    LinearGradient {
        x0: f32, y0: f32, x1: f32, y1: f32,
        stops: Vec<GradientStop>,
        // All gradient interpolation in linear-light sRGB (B4)
    },
    RadialGradient {
        cx: f32, cy: f32, r: f32,
        focal_x: f32, focal_y: f32,
        stops: Vec<GradientStop>,
    },
    Pattern(PatternRef),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatternRef(pub u32);  // index into ctx.pattern_table

#[derive(Clone, Copy, Debug)]
pub enum BlendMode {
    Normal, Multiply, Screen, Overlay, Darken, Lighten,
    ColorDodge, ColorBurn, HardLight, SoftLight, Difference, Exclusion, Clear,
}

#[derive(Clone, Copy, Debug)]
pub enum FillRule { EvenOdd, NonZero }

#[derive(Clone, Copy, Debug)]
pub enum LineCap { Butt, Round, Square }

#[derive(Clone, Copy, Debug)]
pub enum LineJoin { Miter(f32), Round, Bevel }  // Miter(limit)

#[derive(Clone, Debug)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub dash_pattern: Vec<f32>,  // alternating on/off lengths
    pub dash_offset: f32,
}

// N1 FIX: DrawCommand holds path by reference (Arc), not owned clone
// Rasterizer applies transform during edge-table construction — no per-draw allocation
#[derive(Clone, Debug)]
pub enum DrawCommand {
    FillPath { path: std::sync::Arc<VyomaPath>, paint: VyomaPaint, rule: FillRule },
    StrokePath { path: std::sync::Arc<VyomaPath>, paint: VyomaPaint, style: StrokeStyle },
    FillRect { x: f32, y: f32, w: f32, h: f32, paint: VyomaPaint },
    StrokeRect { x: f32, y: f32, w: f32, h: f32, paint: VyomaPaint, style: StrokeStyle },
    FillEllipse { cx: f32, cy: f32, rx: f32, ry: f32, paint: VyomaPaint },
    DrawText { text: String, font_desc: FontDescriptor, size: f32, x: f32, y: f32, paint: VyomaPaint },
    DrawImage { image: u32, src: [f32;4], dst: [f32;4], alpha: f32 },
    ClipPath { path: std::sync::Arc<VyomaPath>, rule: FillRule },
    ClipRect { x: f32, y: f32, w: f32, h: f32 },
    SaveState,
    RestoreState,
    SetTransform(AffineTransform),
    ConcatTransform(AffineTransform),
    ResetTransform,
}

#[derive(Clone, Debug)]
pub struct FontDescriptor {
    pub family: String,
    pub weight: u32,   // 100–900 (CSS weight scale)
    pub style: u32,    // 0=normal, 1=italic, 2=oblique
}

pub struct DrawSurface {
    pub width: u32,
    pub height: u32,
    pub stride: u32,       // bytes per row (width * 4 for BGRA32)
    pub pixels: Vec<u8>,   // BGRA32, premultiplied alpha
}

pub struct GraphicsContext {
    pub surface: DrawSurface,
    // B2: command_queue filled by protocol/WIT dispatch (any thread)
    // Rasterized at vsync in parallel via Rayon
    pub command_queue: Vec<DrawCommand>,
    // State stack (save/restore)
    pub state_stack: Vec<ContextState>,
    pub current_state: ContextState,
    pub pid: Pid,
    pub handle: CtxHandle,
    // B3: timestamp of last protocol command (for path timeout detection)
    pub last_command_at: std::time::Instant,
}

#[derive(Clone, Debug)]
pub struct ContextState {
    pub transform: AffineTransform,
    pub fill_paint: VyomaPaint,
    pub stroke_style: StrokeStyle,
    pub stroke_paint: VyomaPaint,
    pub clip_mask: Option<ClipMask>,  // N4: rasterized alpha mask (not path intersection)
    pub blend_mode: BlendMode,
    pub global_alpha: f32,
    pub font: FontDescriptor,
    pub path_state: PathBuildState,   // B3: tracks open path for v2 protocol
}

// B3: v2 protocol state machine
#[derive(Clone, Debug)]
pub enum PathBuildState {
    Idle,
    Building { segments: Vec<PathSegment>, started_at: std::time::Instant },
}

// N4 FIX: clip mask is a rasterized alpha buffer, not a path intersection operation
// Matches Skia/CoreGraphics model: clip path → rasterize to u8 mask → multiply with coverage
pub struct ClipMask {
    pub x: i32, pub y: i32,
    pub width: u32, pub height: u32,
    pub alpha: Vec<u8>,  // per-pixel coverage [0..255]
}
```

---

## B2 Fix: Threading Model — Parallel Rasterization

**Rule**: Path rasterization never runs on the vsync compositor thread.

**Architecture**:
1. WIT calls and v2 protocol parsing push `DrawCommand`s into `ctx.command_queue` (any thread — stdout parser or WIT linker thread)
2. At vsync tick, R11's compositor collects all dirty `GraphicsContext`s
3. Compositor spawns one Rayon task per dirty context: `rayon::scope(|s| { for ctx in dirty_ctxs { s.spawn(|_| rasterize_context(ctx)); } })`
4. Each task runs `rasterize_context()` which drains `command_queue`, rasterizes to `ctx.surface.pixels`
5. After all tasks join, compositor blits dirty surfaces to framebuffer

**Budget math (with parallelism)**:
```
16.6ms vsync budget
  - R16 animation tick: ~0.5ms
  - R11 surface blit (composite all dirty surfaces): ~0.5ms
  - Available for rasterization: ~15.6ms  
  - With Rayon parallelism across 4 CPU cores: 4 apps × 3.5ms each = fits comfortably
  - Bottleneck: single-app with MAX_PATH_SEGMENTS (10,000) ≈ 8ms (acceptable, single core)
```

**CPU core availability**: Supervisor runs on Linux. On desktop (4+ cores), Rayon's global thread pool uses `num_cpus - 1` threads. On mobile (2 cores), Rayon uses 1 worker thread (rasterize serially but off vsync thread).

```rust
// supervisor/src/draw/mod.rs

pub fn rasterize_all_dirty_contexts(
    contexts: &mut HashMap<CtxHandle, GraphicsContext>,
    dirty: &[CtxHandle],
) {
    use rayon::prelude::*;
    dirty.par_iter().for_each(|handle| {
        if let Some(ctx) = contexts.get_mut(handle) {
            rasterize_context(ctx);
        }
    });
}

fn rasterize_context(ctx: &mut GraphicsContext) {
    let cmds: Vec<DrawCommand> = ctx.command_queue.drain(..).collect();
    // Reset state for this frame
    ctx.current_state = ContextState::default();
    for cmd in cmds {
        execute_draw_command(ctx, cmd);
    }
}
```

---

## B3 Fix: Protocol State Recovery

**Three failure modes and their handling:**

```rust
// supervisor/src/draw/protocol.rs

impl ProtocolParser {
    pub fn handle_line(&mut self, ctx: &mut GraphicsContext, line: &str) {
        // B3: Check for path state timeout before processing any line
        if let PathBuildState::Building { started_at, .. } = &ctx.current_state.path_state {
            if started_at.elapsed().as_millis() > PATH_STATE_TIMEOUT_MS as u128 {
                log::warn!("pid {}: path state timed out after {}ms, discarding", ctx.pid, PATH_STATE_TIMEOUT_MS);
                ctx.current_state.path_state = PathBuildState::Idle;
            }
        }

        if line.starts_with("VYOMA_DRAW:begin_path") {
            ctx.current_state.path_state = PathBuildState::Building {
                segments: Vec::new(),
                started_at: std::time::Instant::now(),
            };
            return;
        }

        if line.starts_with("VYOMA_DRAW:curve_to:") {
            match self.parse_curve_to(line) {
                Ok(seg) => {
                    // Add segment if path is open
                    match &mut ctx.current_state.path_state {
                        PathBuildState::Building { segments, .. } => {
                            if segments.len() < MAX_PATH_SEGMENTS {
                                segments.push(seg);
                            } else {
                                log::warn!("pid {}: MAX_PATH_SEGMENTS reached, segment dropped", ctx.pid);
                            }
                        }
                        PathBuildState::Idle => {
                            // B3 Case 3: v2 segment outside begin_path → warn, ignore
                            log::warn!("pid {}: curve_to outside begin_path, ignored", ctx.pid);
                        }
                    }
                }
                Err(e) => {
                    // B3 Case 2: parse error → log + discard segment + CONTINUE building path
                    // Do NOT abort the entire path — creates fragile app behavior
                    log::warn!("pid {}: parse error in curve_to: {} — segment skipped", ctx.pid, e);
                }
            }
            return;
        }

        // B3 Case 3: v1 command arrives while path is open
        if self.is_v1_command(line) {
            if matches!(ctx.current_state.path_state, PathBuildState::Building { .. }) {
                log::warn!("pid {}: v1 command '{}' interleaved with open path — path discarded", ctx.pid, &line[..line.find(':').unwrap_or(20)]);
                ctx.current_state.path_state = PathBuildState::Idle;
            }
            // Execute the v1 command normally after discarding open path
            self.handle_v1_command(ctx, line);
            return;
        }

        // ... other v2 commands
    }
}
```

**State machine summary**:

| Event | In `Idle` | In `Building` |
|-------|-----------|---------------|
| `begin_path` | → `Building` | Discard current, → new `Building` (warn) |
| `curve_to` / `line_to` / etc | Warn + ignore | Add segment (skip bad parses, log) |
| `fill` / `stroke` | Warn + no-op | Rasterize path → `Idle` |
| v1 command | Execute normally | Discard path + warn + execute v1 |
| App crash / exit | n/a | `PathBuildState = Idle` (supervisor cleans up) |
| Timeout (100ms) | n/a | Discard path + warn → `Idle` |

---

## B5 Fix: Batch Path Submission

**Problem**: 1,000 path segments = 1,002 WIT calls × ~100ns each = ~100µs overhead before any rasterization.

**Fix**: `submit-path-buffer` accepts a compact binary encoding. One WIT call with a `list<u8>` payload.

**Binary encoding** (little-endian):
```
Per segment: [1 byte opcode] [up to 6 × f32 = 24 bytes] = max 25 bytes/segment
Opcode values:
  0x01 = MoveTo     (2 floats: x, y)
  0x02 = LineTo     (2 floats: x, y)
  0x03 = CurveTo    (6 floats: cp1x, cp1y, cp2x, cp2y, ex, ey)
  0x04 = QuadTo     (4 floats: cpx, cpy, ex, ey)
  0x05 = ArcTo      (6 floats: cx, cy, rx, ry, start_rad, sweep_rad)
  0x06 = Close      (0 floats)
```

Example: 1,000 cubic segments = 1,000 × 25 bytes = 25KB. One WIT call.

```wit
// In vyoma:draw@3.0.0 WIT interface:

// B5 FIX: submit packed binary path buffer — one call for arbitrarily complex paths
// Returns path handle (u32) that can be passed to fill-stored-path / stroke-stored-path
// Binary format: sequence of [u8 opcode, f32... params] little-endian
submit-path-buffer: func(ctx: u32, data: list<u8>) -> result<u32, string>;

// Use a stored path handle (avoids re-submitting same path every frame)
fill-stored-path: func(ctx: u32, path: u32, rule: u8) -> result<_, string>;
stroke-stored-path: func(ctx: u32, path: u32) -> result<_, string>;
drop-stored-path: func(ctx: u32, path: u32);
```

```rust
// supervisor/src/draw/protocol.rs (binary decoder)
pub fn decode_path_buffer(data: &[u8]) -> Result<VyomaPath, String> {
    let mut segments = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if segments.len() >= MAX_PATH_SEGMENTS {
            return Err(format!("MAX_PATH_SEGMENTS ({}) exceeded", MAX_PATH_SEGMENTS));
        }
        let opcode = data[i]; i += 1;
        match opcode {
            0x01 => { let (x,y) = read_f32_2(data, &mut i)?; segments.push(PathSegment::MoveTo(x,y)); }
            0x02 => { let (x,y) = read_f32_2(data, &mut i)?; segments.push(PathSegment::LineTo(x,y)); }
            0x03 => { let v = read_f32_6(data, &mut i)?; segments.push(PathSegment::CurveTo(v[0],v[1],v[2],v[3],v[4],v[5])); }
            0x04 => { let v = read_f32_4(data, &mut i)?; segments.push(PathSegment::QuadTo(v[0],v[1],v[2],v[3])); }
            0x05 => { let v = read_f32_6(data, &mut i)?; segments.push(PathSegment::ArcTo { cx:v[0], cy:v[1], rx:v[2].max(0.0), ry:v[3].max(0.0), start:v[4], sweep:v[5].clamp(-std::f32::consts::TAU, std::f32::consts::TAU) }); }
            0x06 => { segments.push(PathSegment::Close); }
            _ => return Err(format!("unknown path opcode 0x{:02x} at byte {}", opcode, i-1)),
        }
    }
    Ok(VyomaPath { segments, device_bbox: None, transform_at_cache: None })
}
```

---

## N5 Fix: set-font WIT Function

```wit
record font-descriptor {
    family: string,
    weight: u32,   // 100–900
    style: u32,    // 0=normal 1=italic 2=oblique
}

set-font: func(ctx: u32, font: font-descriptor, size: f32) -> result<_, string>;

// Updated draw-text includes optional font override (None uses ctx current font)
draw-text: func(ctx: u32, text: string, x: f32, y: f32) -> result<_, string>;
```

---

## N4 Fix: Clip Mask Model

Clip path is rasterized to a per-pixel alpha buffer (the "clip mask"), not computed as a path intersection. This matches Skia and CoreGraphics:

```rust
// When clip-path is called:
// 1. Rasterize the clip path to a u8 alpha buffer covering the dirty rect
// 2. Store as ClipMask in ContextState.clip_mask
// At paint time: multiply source coverage by clip_mask.alpha[pixel]
// Multiple clips: intersect by MIN(existing_mask[px], new_mask[px])
```

This is O(pixels) not O(path_complexity²), and handles all edge cases correctly.

---

## N6 Fix: Cross-Interface Image Handle Contract

Explicit normative statement added to spec and WIT:

```wit
// NOTE: Image handles used in draw-image are the same handle integers returned by
// vyoma:images@1.0.0 load-image(). The supervisor maintains a single global image
// handle table shared between R14 (loading) and R17 (drawing). An image loaded via
// R14 is immediately valid in R17 draw-image calls for the same app PID.
// Handle lifetime: image is ref-counted. R17 does not increment the ref count;
// the app must keep the R14 handle alive for as long as the image is drawn.
draw-image: func(ctx: u32, image: u32, sx: f32, sy: f32, sw: f32, sh: f32,
                 dx: f32, dy: f32, dw: f32, dh: f32) -> result<_, string>;
```

---

## Complete WIT Interface `vyoma:draw@3.0.0`

```wit
package vyoma:draw@3.0.0;

// B4: All color f32 values are linear-light sRGB with premultiplied alpha, range [0.0, 1.0]

record font-descriptor { family: string, weight: u32, style: u32 }
record gradient-stop { position: f32, r: f32, g: f32, b: f32, a: f32 }

interface ctx {
    // Context lifecycle
    create-context: func(surface: u32) -> result<u32, string>;
    destroy-context: func(ctx: u32);
    flush: func(ctx: u32);  // commits command_queue for rasterization at next vsync

    // State
    save-state: func(ctx: u32) -> result<_, string>;    // max depth MAX_SAVE_DEPTH
    restore-state: func(ctx: u32) -> result<_, string>;

    // Transform
    set-transform: func(ctx: u32, a: f32, b: f32, c: f32, d: f32, tx: f32, ty: f32);
    concat-transform: func(ctx: u32, a: f32, b: f32, c: f32, d: f32, tx: f32, ty: f32);
    reset-transform: func(ctx: u32);

    // Font
    set-font: func(ctx: u32, font: font-descriptor, size: f32) -> result<_, string>;

    // Paint
    set-fill-color: func(ctx: u32, r: f32, g: f32, b: f32, a: f32);  // linear-light sRGB, premult
    set-stroke-color: func(ctx: u32, r: f32, g: f32, b: f32, a: f32);
    set-line-width: func(ctx: u32, w: f32);
    set-line-cap: func(ctx: u32, cap: u8);   // 0=butt 1=round 2=square
    set-line-join: func(ctx: u32, join: u8, miter-limit: f32);  // 0=miter 1=round 2=bevel
    set-line-dash: func(ctx: u32, pattern: list<f32>, offset: f32);
    set-blend-mode: func(ctx: u32, mode: u8);
    set-global-alpha: func(ctx: u32, alpha: f32);

    // Gradients
    create-linear-gradient: func(ctx: u32, x0: f32, y0: f32, x1: f32, y1: f32,
                                  stops: list<gradient-stop>) -> result<u32, string>;
    create-radial-gradient: func(ctx: u32, cx: f32, cy: f32, r: f32,
                                  fx: f32, fy: f32,
                                  stops: list<gradient-stop>) -> result<u32, string>;
    set-fill-gradient: func(ctx: u32, gradient: u32);
    drop-gradient: func(ctx: u32, gradient: u32);

    // Path building (individual WIT calls — for simple paths; use submit-path-buffer for complex)
    new-path: func(ctx: u32);
    move-to: func(ctx: u32, x: f32, y: f32);
    line-to: func(ctx: u32, x: f32, y: f32);
    curve-to: func(ctx: u32, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, ex: f32, ey: f32);
    quad-to: func(ctx: u32, cpx: f32, cpy: f32, ex: f32, ey: f32);
    arc-to: func(ctx: u32, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32);
    close-path: func(ctx: u32);

    // B5 FIX: batch path submission — one call for complex paths
    submit-path-buffer: func(ctx: u32, data: list<u8>) -> result<u32, string>;
    fill-stored-path: func(ctx: u32, path: u32, rule: u8) -> result<_, string>;
    stroke-stored-path: func(ctx: u32, path: u32) -> result<_, string>;
    drop-stored-path: func(ctx: u32, path: u32);

    // Draw ops
    fill-path: func(ctx: u32, rule: u8);          // 0=even-odd 1=non-zero
    stroke-path: func(ctx: u32);
    fill-rect: func(ctx: u32, x: f32, y: f32, w: f32, h: f32);
    stroke-rect: func(ctx: u32, x: f32, y: f32, w: f32, h: f32);
    fill-ellipse: func(ctx: u32, cx: f32, cy: f32, rx: f32, ry: f32);
    draw-text: func(ctx: u32, text: string, x: f32, y: f32) -> result<_, string>;
    // N6: cross-interface image handle contract documented — R14 handles valid here
    draw-image: func(ctx: u32, image: u32, sx: f32, sy: f32, sw: f32, sh: f32,
                     dx: f32, dy: f32, dw: f32, dh: f32) -> result<_, string>;

    // Clip (N4: rasterized mask model)
    clip-path: func(ctx: u32, rule: u8);
    clip-rect: func(ctx: u32, x: f32, y: f32, w: f32, h: f32);
}

world vyoma-app { import ctx; }
```

---

## B1 Fix: v2 Protocol Rate Limits and Migration

**Added to Section 5 of the spec — normative constraints:**

> **v2 text protocol rate limit**: Applications MUST NOT use v2 path commands at frame rates exceeding 30Hz, or with more than 20 path segments per frame. Exceeding these limits will cause the supervisor's stdout parser thread to become the throughput bottleneck for all apps. Applications with ANY animated drawing MUST use the v3 WIT interface.

**Migration example** (v2 rounded-rect → v3):
```
# v2 (text, ~10 lines/frame, OK for static UI):
VYOMA_DRAW:begin_path
VYOMA_DRAW:move_to:10,20
VYOMA_DRAW:curve_to:10,10,20,10,30,10
...
VYOMA_DRAW:fill:0.2,0.5,0.9,1.0

# v3 WIT (binary, single call via submit-path-buffer, OK for 60Hz):
let buf = encode_rounded_rect(x, y, w, h, radius);  // ~200 bytes
submit_path_buffer(ctx, &buf)?;
fill_stored_path(ctx, path_handle, FILL_EVEN_ODD)?;
```

---

## Rasterizer Algorithm

```rust
// supervisor/src/draw/rasterizer.rs

// B2: Rasterizer runs in Rayon task, never on vsync thread.
// Transform applied during edge table construction — no per-draw path clone (N1 fix).

pub fn rasterize_fill(
    surface: &mut DrawSurface,
    path: &VyomaPath,
    transform: &AffineTransform,
    paint: &VyomaPaint,
    rule: FillRule,
    clip: Option<&ClipMask>,
) {
    // 1. Flatten bezier/arc segments via de Casteljau (flatness threshold: 0.5px in device space)
    // 2. Build edge table: sorted by min_y, each edge stores {x_start, dx_per_scanline, min_y, max_y}
    // 3. For each scanline y:
    //    a. Add edges from edge table where min_y == y into active edge table (AET)
    //    b. Sort AET by x_intercept
    //    c. Fill spans using FillRule (even-odd: toggle in/out at each crossing; non-zero: track winding count)
    //    d. AA: 4×4 MSAA — sample 16 sub-pixel positions, compute coverage [0..16] → alpha
    //    e. Composite with clip mask (N4): coverage *= clip.alpha[pixel] / 255
    //    f. Blend into surface pixels using BlendMode
    //    e. Remove edges where max_y == y from AET
    //    g. Update x_intercepts for remaining AET edges
}

// De Casteljau subdivision — recurse until max displacement from chord < flatness_threshold
fn flatten_cubic(p0: (f32,f32), p1: (f32,f32), p2: (f32,f32), p3: (f32,f32),
                 threshold: f32, depth: u32, out: &mut Vec<(f32,f32)>) {
    if depth > 32 { out.push(p3); return; }  // N7: depth 32 is fine for UI; sub-pixel accuracy
    // Compute chord distance
    let chord_sq = dist_sq(p0, p3);
    let mid_deviation = deviation_from_chord(p0, p1, p2, p3);
    if mid_deviation < threshold * threshold || chord_sq < 1.0 {
        out.push(p3);
        return;
    }
    // Split at t=0.5
    let (left, right) = split_cubic(p0, p1, p2, p3, 0.5);
    flatten_cubic(left.0, left.1, left.2, left.3, threshold, depth+1, out);
    flatten_cubic(right.0, right.1, right.2, right.3, threshold, depth+1, out);
}
```

---

## Security Model

| Threat | Mitigation |
|--------|-----------|
| Path with 1M segments | `MAX_PATH_SEGMENTS = 10_000` cap; excess segments dropped with warning |
| Gradient with 1000 stops | `MAX_GRADIENT_STOPS = 64` cap |
| Deep save/restore nesting | `MAX_SAVE_DEPTH = 64`; 65th `save_state` returns `Err` |
| Hung app with open path | `PATH_STATE_TIMEOUT_MS = 100` — auto-discard after 100ms |
| Malformed arc (sweep > 360°) | `sweep.clamp(-TAU, TAU)` in `ArcTo` decode |
| Context handle spoofing | Per-app `CtxHandleTable` validates PID match (same pattern as R16 R17) |
| Binary path buffer with bad opcode | Return `Err(String)` on unknown opcode — no partial execution |

---

## Platform Matrix

| Feature | mcu | iot | robotics | server | mobile | desktop |
|---------|-----|-----|----------|--------|--------|---------|
| v1 text protocol | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| v2 path commands | — | basic | basic | — | ✓ | ✓ |
| v3 WIT interface | — | — | — | — | ✓ | ✓ |
| `submit-path-buffer` | — | — | — | — | ✓ | ✓ |
| Gradients | — | — | — | — | ✓ | ✓ |
| Blend modes | — | — | — | — | ✓ | ✓ |
| Parallel rasterization | — | — | — | — | 1 worker | num_cpus-1 |

---

## Implementation Files (supervisor/src/draw/, all ≤500 LOC)

| File | Responsibility |
|------|---------------|
| `mod.rs` | Public types, DrawCommand, GraphicsContext, constants |
| `context.rs` | State stack, clip mask (N4: rasterized model), font state, PatternRef table (N3) |
| `rasterizer.rs` | Scanline fill, de Casteljau flatten, MSAA anti-aliasing; runs in Rayon tasks (B2) |
| `stroke.rs` | Stroke expansion: caps, joins, dash pattern |
| `gradient.rs` | Linear/radial gradient evaluation in linear-light sRGB (B4) |
| `blend.rs` | Porter-Duff + 12 blend modes; all math in linear-light |
| `text.rs` | FillText: call R13 LayoutSession → glyph positions → blit from atlas |
| `image.rs` | draw_image: bilinear scale, source crop; image handle contract (N6) |
| `transform.rs` | AffineTransform multiply/push/pop; bbox invalidation on change (N2) |
| `protocol.rs` | v2 text parser with B3 state recovery; binary path buffer decoder (B5) |
| `wit_handlers.rs` | v3 WIT linker; `submit-path-buffer` decode + store; `set-font` (N5) |

`Cargo.toml`: No new external dependencies. Rayon already available in supervisor.

---

## Required Unit Tests

1. `fill_rule_even_odd` — star shape fills correctly with even-odd rule
2. `fill_rule_non_zero` — star shape fills correctly with non-zero winding
3. `bezier_flatten_accuracy` — de Casteljau output deviates < 0.5px from true curve
4. `path_buffer_decode_cubic` — 1000-segment binary buffer decodes to correct path
5. `path_buffer_unknown_opcode` — returns Err, no partial execution
6. `parallel_rasterize_no_race` — 4 contexts rasterized in parallel, pixel results match serial
7. `path_state_timeout_b3` — open path auto-discarded after 100ms
8. `v1_interleave_b3` — v1 fill_rect during open path discards path, executes fill_rect
9. `parse_error_skip_segment_b3` — malformed curve_to skips segment, path continues building
10. `color_space_gradient_b4` — gradient stop at sRGB 0.5 interpolated in linear space
11. `clip_mask_rasterized_n4` — clip path produces correct alpha buffer for complex shape
12. `transform_bbox_invalidation_n2` — bbox cache invalidated when transform changes
13. `save_depth_cap` — 65th save-state returns Err
14. `path_segment_cap` — segment 10,001 dropped with warning
15. `gradient_stop_cap` — stop 65 rejected
16. `draw_image_cross_interface_n6` — R14 handle valid in R17 draw-image call
