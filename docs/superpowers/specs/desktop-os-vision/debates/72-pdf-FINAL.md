# FINAL Spec: PDF Rendering Engine (Round 72)

**Subsystem**: PDF Rendering Engine  
**macOS Analogue**: PDFKit / Quartz PDF engine  
**Depends on**: R13 (font system — fontdue), R11 (display/Surface), miniz_oxide (already in tree), zune-jpeg (new dep)  
**Status**: FINAL — all blocking issues resolved  
**Target**: <500ms for typical A4 page at 96dpi on x86_64

---

## 1. Architecture

### Module Tree

```
supervisor/src/pdf/
├── mod.rs            (~120 lines) — PdfEngine global, OnceLock, VYOMA_PDF: dispatch, worker routing
├── parser.rs         (~480 lines) — XrefTable (classic + compressed PDF 1.5+), PdfDocument, PdfObj enum
├── page.rs           (~460 lines) — page tree walking, content stream tokenizer, Token enum
├── renderer.rs       (~490 lines) — GfxState, PathBuilder, scan-line fill, operator dispatch table
├── shm.rs            (~120 lines) — ShmSurface (memfd + pidfd_getfd), lifecycle
└── text_render.rs    (~200 lines) — TextState, Tf/Tj/TJ operators, font encoding map [u8;256]→char
```

All files obey the 500-line hard limit. One worker thread per active `req_id`, blocked on `mpsc::Receiver<PdfCmd>`. A single router thread owns all VYOMA_PDF line parsing and forwards commands to workers via `HashMap<String, mpsc::Sender<PdfCmd>>`. Worker key format: `"{app_name}/{req_id}"` — enables prefix scan on app exit.

Global: `static PDF_ENGINE: OnceLock<PdfEngine> = OnceLock::new();`  
`PdfEngine` holds a `Mutex<HashMap<String, mpsc::Sender<PdfCmd>>>` for worker senders.

---

## 2. VYOMA_PDF Protocol

### App → Supervisor (exact strings)

```
VYOMA_PDF:load:<req_id>,<path>
VYOMA_PDF:render:<req_id>,<page_index>,<width_px>,<height_px>,<dpi>
VYOMA_PDF:page_count:<req_id>
VYOMA_PDF:close:<req_id>
```

### Supervisor → App (exact strings)

```
VYOMA_SYSTEM:pdf-shm-fd:<fd_num> w=<w> h=<h>
VYOMA_PDF:render-done:<req_id>:<page_index>
VYOMA_PDF:page-count:<req_id>:<n>
VYOMA_PDF:error:<req_id>:<reason>
```

`pdf-shm-fd` is written to app stdin immediately after render completes; `render-done` follows on the same tick. `fd_num` is valid in the app's fd table (transferred via `pidfd_getfd`). Error `reason` is short ASCII (no colons): e.g. `file-not-found`, `bad-xref`, `page-oob`, `capability-denied`.

### Required Capabilities

```toml
[capabilities]
pdf_render  = true   # enables VYOMA_PDF: command dispatch
filesystem  = true   # app must reach the PDF file path
```

Both must be present; missing either produces `VYOMA_PDF:error:<req_id>:capability-denied`.

---

## 3. Core Types

```rust
// supervisor/src/pdf/mod.rs
pub enum PdfCmd {
    Render { page: u32, width: u32, height: u32, dpi: u32, tx: mpsc::Sender<PdfResult> },
    PageCount { tx: mpsc::Sender<u32> },
    Close,
}

pub enum PdfResult {
    Done { fd: RawFd, w: u32, h: u32 },
    Error(String),
}

// supervisor/src/pdf/parser.rs
#[derive(Clone)]
pub enum PdfObj {
    Null, Bool(bool), Int(i64), Real(f64), Name(String), Str(Vec<u8>),
    Array(Vec<PdfObj>), Dict(HashMap<String, PdfObj>),
    Stream { dict: HashMap<String, PdfObj>, data: Vec<u8> },
    Ref(u32, u16),
}

pub struct XrefEntry { pub offset: u64, pub gen: u16, pub in_use: bool }

pub struct XrefTable {
    pub entries: HashMap<u32, XrefEntry>,
    pub trailer: HashMap<String, PdfObj>,
}

pub struct PdfDocument {
    pub raw:     Vec<u8>,
    pub xref:    XrefTable,
    pub catalog: HashMap<String, PdfObj>,
    obj_cache:   HashMap<u32, PdfObj>,
}
```

---

## 4. PDF Parser Details

### Cross-Reference Table Detection

`find_startxref`: scan last 1024 bytes for `"startxref"` keyword, parse decimal offset.

```rust
pub fn parse_xref(raw: &[u8], offset: u64) -> Result<XrefTable, String> {
    let start = offset as usize;
    let peek = raw.get(start..start + 4).unwrap_or(&[]);
    if peek.starts_with(b"xref") {
        parse_xref_table(raw, offset)   // classic xref table
    } else {
        parse_xref_stream(raw, offset)  // PDF 1.5+ compressed xref stream
    }
}
```

**Classic xref table**: parse sections `obj_num count` → `offset gen f|n` lines. `f` = free, `n` = in-use. Trailer dict follows last section, prefixed `trailer`. Follow `Prev` chain for updated PDFs.

**Compressed xref stream (PDF 1.5+)**: object at startxref offset is a stream with `/Type /XRef`. Read `W` array (field byte widths), `Index` array (defaults to `[0, /Size]`), decompress via FlateDecode. Parse binary entries: type 0 = free, type 1 = offset object, type 2 = compressed object. `/Root` comes from the stream dict itself, not a separate trailer.

### Stream Decoding

```rust
pub fn decode_stream(dict: &HashMap<String, PdfObj>, data: Vec<u8>) -> Result<Vec<u8>, String> {
    match dict.get("Filter") {
        None => Ok(data),
        Some(PdfObj::Name(name)) => apply_filter(name, data),
        Some(PdfObj::Array(filters)) => {
            filters.iter().try_fold(data, |buf, f| {
                if let PdfObj::Name(n) = f { apply_filter(n, buf) } else { Ok(buf) }
            })
        }
        _ => Err("Filter: unexpected type".into()),
    }
}

fn apply_filter(name: &str, data: Vec<u8>) -> Result<Vec<u8>, String> {
    match name {
        "FlateDecode"    => {
            // strip 2-byte zlib header if present (0x78 magic)
            let payload = if data.first() == Some(&0x78) { &data[2..] } else { &data[..] };
            miniz_oxide::inflate::decompress_to_vec(payload)
                .map_err(|e| format!("FlateDecode: {:?}", e))
        }
        "DCTDecode"      => Ok(data),  // returned as-is; caller uses zune-jpeg at image render time
        "ASCII85Decode"  => decode_ascii85(&data),
        "ASCIIHexDecode" => decode_asciihex(&data),
        other            => { eprintln!("[pdf] unknown filter {other}"); Ok(data) }
    }
}
```

### Object Resolution

```rust
impl PdfDocument {
    pub fn resolve_ref(&mut self, obj_num: u32) -> Option<PdfObj> {
        if let Some(cached) = self.obj_cache.get(&obj_num) { return Some(cached.clone()); }
        let entry = self.xref.entries.get(&obj_num)?.clone();
        if !entry.in_use { return None; }
        // parse "N G obj ... endobj" at entry.offset; decode stream if present
        let obj = parse_object_at(&self.raw, entry.offset).ok()?;
        self.obj_cache.insert(obj_num, obj.clone());
        Some(obj)
    }
}
```

---

## 5. Renderer Details

### Core Types

```rust
// supervisor/src/pdf/renderer.rs
#[derive(Clone)]
pub struct GfxState {
    pub ctm:          [f64; 6],        // [a,b,c,d,e,f] current transformation matrix
    pub ctm_stack:    Vec<[f64; 6]>,   // q/Q push/pop
    pub fill_color:   [f64; 3],        // RGB 0.0–1.0
    pub stroke_color: [f64; 3],
    pub line_width:   f64,
    pub fill_rule:    FillRule,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FillRule { NonZero, EvenOdd }

pub struct PathBuilder {
    pub points:  Vec<(f64, f64)>,
    pub verbs:   Vec<PathVerb>,
    pub current: Option<(f64, f64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PathVerb { MoveTo, LineTo, CurveTo, Close }
```

### Coordinate Transform

```rust
/// Convert PDF user-space (x,y) to pixel coordinates.
/// sx = width_px/page_w_pts * 72, sy = height_px/page_h_pts * 72
/// Y-flip applied AFTER CTM — never inline in individual operators.
pub fn user_to_pixel(
    x: f64, y: f64, ctm: &[f64;6], sx: f64, sy: f64, page_h: f64,
) -> (i32, i32) {
    // 1. Apply CTM: x' = a*x + c*y + e,  y' = b*x + d*y + f
    let px = ctm[0]*x + ctm[2]*y + ctm[4];
    let py = ctm[1]*x + ctm[3]*y + ctm[5];
    // 2. Scale from points to pixels
    let px = px * sx / 72.0;
    let py = py * sy / 72.0;
    // 3. Y-flip: PDF origin bottom-left; surface origin top-left
    let py = (page_h * sy / 72.0) - py;
    (px as i32, py as i32)
}
```

### Graphics Operator Dispatch (key operators)

| Operator | Action |
|----------|--------|
| `q`/`Q` | Push/pop GfxState clone |
| `cm` | Pre-multiply CTM |
| `w` | Set line_width |
| `rg`/`RG`/`g`/`G` | Set fill/stroke color (RGB/Gray) |
| `m`/`l`/`c`/`v`/`y`/`h`/`re` | Path construction |
| `f`/`f*`/`S`/`B`/`B*`/`n` | Path paint (fill/stroke/both/discard) |
| `BT`/`ET` | Begin/end text block |
| `Tf`/`Tm`/`Td`/`TD`/`T*` | Text state |
| `Tj`/`TJ`/`"` | Show strings |
| `Do` | Paint XObject (image or form) |
| `cs`/`CS`/`sc`/`SC` | Colorspace and color |

### Scan-Line Fill

Flatten path segments to pixel-space line segments (Bezier curves via de Casteljau at 0.5px flatness). For each scan row from `min_y` to `max_y`: find X intersections with all segments, sort, fill between pairs (non-zero winding or even-odd per `fill_rule`). Uses `surface.fill_rect(x, y, w, 1, color)`.

### Image XObjects (Do Operator)

Decode stream: FlateDecode → RGB bytes; DCTDecode via `zune_jpeg`. Scale decoded pixels to CTM-defined destination rect using nearest-neighbour. Map unit square `(0,0)-(1,1)` through CTM to get destination rect in pixel space via `user_to_pixel`.

---

## 6. Text Rendering

```rust
// supervisor/src/pdf/text_render.rs
pub struct TextState {
    pub font_name:    String,
    pub font_size:    f64,
    pub text_matrix:  [f64; 6],      // Tm
    pub line_matrix:  [f64; 6],      // Tlm for Td/T* line moves
    pub encoding_map: [char; 256],   // built at Tf time
    pub leading:      f64,
    pub word_spacing: f64,
    pub char_spacing: f64,
    pub h_scale:      f64,           // Tz, default 100.0
}
```

### Encoding Map Construction (at Tf time)

```rust
pub fn build_encoding_map(encoding: Option<&PdfObj>) -> [char; 256] {
    let mut map: [char; 256] = std::array::from_fn(|i| i as u8 as char);
    match encoding {
        Some(PdfObj::Name(base)) => apply_base_encoding(&mut map, base),
        Some(PdfObj::Dict(d)) => {
            if let Some(PdfObj::Name(base)) = d.get("BaseEncoding") {
                apply_base_encoding(&mut map, base);
            }
            if let Some(PdfObj::Array(diffs)) = d.get("Differences") {
                apply_differences(&mut map, diffs);  // override by glyph name
            }
        }
        _ => {}
    }
    map
}
```

Base encodings: `WinAnsiEncoding` (Windows-1252 → Unicode), `MacRomanEncoding`, `StandardEncoding` (Adobe Standard Latin) — each a complete `const [char; 256]` array. `Differences` array overrides individual slots: alternating `Int(code)` and `Name(glyph_name)`, mapped via Adobe glyph name list (~300 common names in a `match` statement).

### String Rendering

`render_pdf_string(data: &[u8], ts: &mut TextState, ...)`: maps each byte through `encoding_map`, applies text matrix + CTM via `user_to_pixel`, renders glyph via R13 `FontCache::draw_char`, advances text matrix by glyph advance width + char_spacing (+ word_spacing for space).

`render_tj_array`: iterates TJ array; `Str` items call `render_pdf_string`; `Int`/`Real` items apply kerning: `disp = -adj/1000.0 * font_size`, advance `text_matrix[4,5]` by `disp * text_matrix[0,1]`. Negative adj = move right; positive = move left.

---

## 7. ShmSurface

```rust
// supervisor/src/pdf/shm.rs
pub struct ShmSurface {
    pub fd:     RawFd,
    pub ptr:    *mut u8,
    pub size:   usize,
    pub width:  u32,
    pub height: u32,
}

impl ShmSurface {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let size = (width * height * 4) as usize;
        let fd = unsafe {
            libc::syscall(libc::SYS_memfd_create, b"pdf-surface\0".as_ptr(), libc::MFD_CLOEXEC)
        } as RawFd;
        if fd < 0 { return Err(format!("memfd_create: {}", std::io::Error::last_os_error())); }
        unsafe { libc::ftruncate(fd, size as libc::off_t) };
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), size,
                libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        } as *mut u8;
        Ok(ShmSurface { fd, ptr, size, width, height })
    }

    pub fn write_pixels(&self, pixels: &[u8]) {
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.ptr, pixels.len().min(self.size)) };
    }

    /// Transfer fd into target app's fd table via pidfd_getfd.
    /// Returns the fd number as seen by the app process.
    pub fn send_to_app(pidfd: RawFd, self_fd: RawFd) -> Result<RawFd, String> {
        let app_fd = unsafe {
            libc::syscall(libc::SYS_pidfd_getfd, pidfd, self_fd, 0)
        } as RawFd;
        if app_fd < 0 { Err(format!("pidfd_getfd: {}", std::io::Error::last_os_error())) }
        else { Ok(app_fd) }
    }
}

impl Drop for ShmSurface {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size); libc::close(self.fd); }
    }
}
```

---

## 8. Integration

**AppState extension**: add `pub pidfd: Option<RawFd>` to `AppState` in `app_threads.rs`.

**Dispatch** (in `ipc_handlers.rs`): snapshot `app_state.pidfd` and capabilities under the apps lock before acquiring `PDF_ENGINE` mutex — prevents ABBA deadlock. Check `pdf_render && filesystem` capabilities; send `capability-denied` error if missing.

**App exit cleanup** (waiter thread in `app_threads.rs`):
```rust
if let Some(engine) = PDF_ENGINE.get() { engine.close_all_for_app(&app_name); }
if let Some(pidfd) = state.pidfd.take() { unsafe { libc::close(pidfd) }; }
```

**Compositor integration**: After `render-done`, the RGBA buffer in ShmSurface is composited as the app's window surface via R11 surface routing at next `VYOMA_DRAW:flush`.

**Font integration (R13)**: At `Tf` operator: resolve PDF `/BaseFont` name → `FontCache::find_by_name` (falls back to built-in bitmap font). Build encoding_map from `/Encoding` dict. Glyph rasterization delegated to fontdue via existing `FontCache`.

**Cargo deps** (`supervisor/Cargo.toml`):
```toml
zune-jpeg   = { version = "0.4", default-features = false }
miniz_oxide = { version = "0.8", default-features = false, features = ["with-alloc"] }
```

**Manifest** (`supervisor/src/manifest.rs` `Capabilities` struct):
```rust
#[serde(default)]
pub pdf_render: bool,
```
Must be added — `#[serde(deny_unknown_fields)]` will reject any `vyoma.toml` with `pdf_render = true` until this field exists.

---

## 9. Performance

| Target | Metric | Strategy |
|--------|--------|----------|
| <500ms | A4 page at 96dpi, x86_64 | One worker thread per req_id |
| Zero-copy | Pixel buffer delivery to app | memfd + pidfd_getfd |
| Low memory | Multi-PDF open | Lazy page tree walk (don't pre-parse all pages) |
| No global lock during render | Throughput | PDF_ENGINE mutex held only for worker map insert/remove |

Object cache (`PdfDocument::obj_cache`) is per-document, lives on the worker thread — no shared state between workers.

---

## 10. Blocking Issues

### B1 — AppState.pidfd Missing

`pidfd_getfd` (Linux 5.6+) requires the supervisor to hold a `pidfd` for each child. Without it `ShmSurface::send_to_app` cannot transfer the memfd into the app's fd table. VyomaOS runs kernel 5.10, so the syscall is available.

```rust
// supervisor/src/app_threads.rs — immediately after wasmtime child spawn
let child_pid = child.id().expect("child pid") as libc::pid_t;
let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, child_pid, 0) as RawFd };
state.pidfd = if pidfd >= 0 { Some(pidfd) } else {
    eprintln!("[supervisor] pidfd_open failed for {}: {}", app_name,
              std::io::Error::last_os_error());
    None
};
// In waiter thread, after child.wait() returns:
if let Some(pidfd) = state.pidfd.take() { unsafe { libc::close(pidfd) }; }
```

### B2 — Compressed Xref Detection for PDF 1.5+

The majority of modern PDFs use compressed cross-reference streams. Checking for `xref` keyword at startxref offset is wrong — PDF 1.5+ places an object header (`N G obj`) there instead. The fix is the two-branch `parse_xref` above. For compressed streams, `/Root` is extracted from the stream dict (it doubles as the trailer); do NOT search for a separate `trailer` keyword.

### B3 — Y-Axis Flip Must Apply After CTM Transform

Applying the Y-flip before CTM, or inline in individual path/text operators, produces mirrored/mispositioned output. The flip must be the final step of `user_to_pixel`. Every path operator (`m`, `l`, `c`, `re`) and every text position calculation calls `user_to_pixel`. No Y-flip logic anywhere else.

```rust
// CORRECT — flip after CTM:
let py = (page_h * sy / 72.0) - py;   // last line of user_to_pixel

// WRONG — never do this in an operator:
// let y = page_h - y;   // flip before CTM
```

### B4 — Font Encoding: Raw `byte as char` Gives Wrong Glyphs

Casting a raw PDF string byte directly to `char` fails for all non-ASCII bytes. WinAnsiEncoding (Windows-1252) and MacRomanEncoding differ from Unicode in the 0x80–0xFF range, producing wrong characters for `©`, `é`, `—`, curly quotes, etc.

```rust
// CORRECT — use encoding map built at Tf time:
let ch = ts.encoding_map[byte as usize];

// WRONG — never cast byte directly:
// let ch = byte as char;
```

The three base encoding tables (`WIN_ANSI_MAP`, `MAC_ROMAN_MAP`, `STANDARD_MAP`) must be complete `const [char; 256]` arrays covering all code points including the 0x80–0x9F range.

### B5 — Worker Cleanup on App Exit

Workers hold open memfd fds and mpsc senders. If the app exits without sending `VYOMA_PDF:close`, workers stay alive indefinitely, leaking threads and fds. Worker key format `"{app_name}/{req_id}"` enables prefix scan at app exit:

```rust
// supervisor/src/pdf/mod.rs
impl PdfEngine {
    pub fn close_all_for_app(&self, app_name: &str) {
        let prefix = format!("{app_name}/");
        let mut workers = self.workers.lock().unwrap();
        let keys: Vec<String> = workers.keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned().collect();
        for key in keys {
            if let Some(tx) = workers.remove(&key) {
                let _ = tx.send(PdfCmd::Close);
                // worker receives Close, drops PdfDocument + ShmSurface, exits
            }
        }
    }
}
```

Called from the app waiter thread immediately before closing pidfd.
