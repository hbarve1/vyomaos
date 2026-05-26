# Research: Apple-Platform UI Fidelity

## Decision 1: Font Rasterizer

**Decision**: Use `fontdue` (v0.7+)

**Rationale**: Pure Rust, no C dependencies, compiles cleanly to `x86_64-unknown-linux-musl`. Built-in rasterization returns a `Vec<u8>` coverage buffer (0–255 per pixel) in one call. Supports `no_std` with `alloc`. Active maintenance, Apache-2.0/MIT dual-licensed.

**API**:
```rust
let font = fontdue::Font::from_bytes(font_bytes, Default::default()).unwrap();
let (metrics, coverage) = font.rasterize('A', 16.0); // 16pt
// coverage: Vec<u8>, metrics.width × metrics.height pixels, value = alpha
```

**Binary size delta**: ~600 KB added to supervisor.

**Alternatives considered**:
- `ab_glyph`: requires std, slightly smaller, but less ergonomic API
- `rusttype`: older, comparable size, less active
- `ttf-parser`: parsing only, no rasterization — would need a separate rasterizer layer

---

## Decision 2: PNG Decoder

**Decision**: Use `lodepng` crate

**Rationale**: Pure Rust, zero C dependencies, direct RGBA buffer output via `decode32()`. ~0.5–0.8 MB binary delta. Proven algorithm (LodePNG), reliable for production use. OFL/Zlib license.

**API**:
```rust
let data = std::fs::read(path)?;
let image = lodepng::decode32(&data)?;
// image.buffer: Vec<RGBA<u8>>, image.width, image.height
```

**Alternatives considered**:
- `image` crate: full-featured but adds ~2MB to binary and many unused codecs
- `png` crate: pure Rust but more complex API; slightly larger than lodepng for our use case
- `minipng`: smallest but less mature for production app icons

---

## Decision 3: Alpha Compositing Algorithm

**Decision**: Porter-Duff "over" operator with premultiplied alpha; separable box blur (2 passes, radius 4) for drop shadows; semi-transparent dark overlay for frosted glass.

**Porter-Duff "over" formula**:
```
α_out = α_src + α_dst × (1 − α_src)
C_out = (C_src × α_src + C_dst × α_dst × (1 − α_src)) / α_out
```

Applied per-pixel, back-to-front across z-layers. Premultiply alpha to avoid division in the hot path.

**Drop shadow**: Offset window shape mask by (4, 6)px; apply 2 passes of separable box blur (radius 4); composite at 50% alpha beneath window layer. Total cost: O(w·h) per frame, acceptable for CPU.

**Frosted glass**: Semi-transparent dark overlay (`rgba(0,0,0,180)`) over the background content. No true blur of the layer beneath — impractical on CPU without GPU.

**Rounded corner mask**: Per-pixel signed-distance to corner arc; coverage = `clamp(1 - max(0, dist - radius), 0, 1)`. Render at 2× supersampling, average to 1× for anti-aliasing.

---

## Decision 4: Typeface

**Decision**: **Inter** (UI text) + **IBM Plex Mono** (terminal/code)

**Rationale**: Inter is the most widely cited open-source match to SF Pro — identical x-height, geometric proportions, excellent hinting for small sizes. IBM Plex Mono pairs naturally with Inter and covers all monospace needs.

| Font | Variants | Regular TTF | Bold TTF | License |
|------|----------|------------|----------|---------|
| Inter | Regular, Bold, Italic, Bold-Italic | ~185 KB | ~192 KB | OFL |
| IBM Plex Mono | Regular, Bold | ~130 KB | ~135 KB | OFL |

Total initramfs font footprint: ~642 KB for all variants. Well within budget.

**Alternative**: Geist (110 KB regular, cleaner at small sizes, less SF-like proportions) — keep as backup if initramfs size becomes critical.

---

## Decision 5: Display Profile Storage

**Decision**: Profiles are TOML files at `supervisor/src/profile/profiles/<name>.toml` (already established by spec-043). The `display_profile` field in the active profile TOML determines shell chrome layout. No new mechanism needed — extend existing profile system.

**Shell layouts per profile**:
- `desktop`: dock (bottom) + menu bar (top) + floating windows
- `phone`: status bar (top) + full-screen single app + home indicator (bottom)
- `tablet`: same as desktop but touch-optimized margins
- `watch`: no chrome; single full-screen app; crown = scroll
- `tv`: no chrome; focus-ring navigation; overscan-safe margins
- `vision`: spatial anchor grid; floating window panels; gaze target highlight

---

## Decision 6: VYOMA_DRAW Protocol Extensions

New protocol commands needed (additive, backwards-compatible):

| Command | Syntax | Notes |
|---------|--------|-------|
| `draw_glyph` | `VYOMA_DRAW:draw_glyph:<x>,<y>,<rgba>,<size_pt>,<text>` | Replaces draw_text; uses fontdue |
| `draw_image` | `VYOMA_DRAW:draw_image:<x>,<y>,<w>,<h>,<path>` | PNG decode + scale + blit |
| `fill_rect_r` | `VYOMA_DRAW:fill_rect_r:<x>,<y>,<w>,<h>,<rgba>,<radius>` | Rounded rect with alpha |
| `set_alpha` | `VYOMA_DRAW:set_alpha:<value>` | Sets compositing alpha for subsequent draws |

Old `draw_text` with s/m/l size codes remains supported for backwards compatibility.
