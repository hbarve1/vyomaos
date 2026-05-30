# FINAL Spec: PDF Rendering Engine (Round 72)

**Subsystem**: PDF Rendering Engine  
**macOS Analogue**: PDFKit / Quartz PDF engine  
**Depends on**: R13 (font system — fontdue), R11 (display/Surface), miniz_oxide (already in tree), zune-jpeg (new dep)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

```
supervisor/src/pdf/
├── mod.rs            (~120 lines) — PdfEngine, OnceLock global, VYOMA_PDF: dispatch
├── parser.rs         (~480 lines) — XrefTable, PdfDocument, PdfObj enum, stream decode
├── page.rs           (~460 lines) — page tree walking, content stream tokenizer, Token enum
├── renderer.rs       (~490 lines) — GfxState, PathBuilder, scan-line fill, operator dispatch
├── shm.rs            (~120 lines) — memfd ShmSurface, pidfd_getfd to pass fd to app
└── text_render.rs    (~200 lines) — TextState, Tf/Tj/TJ operators, font encoding map
```

One PDF worker thread per active req_id, blocked on mpsc::Receiver<PdfCmd>. Router thread forwards VYOMA_PDF: lines to workers. No render work on router thread.

## 2. VYOMA_PDF Protocol

App → Supervisor:
```
VYOMA_PDF:load:<req_id>,<path>
VYOMA_PDF:render:<req_id>,<page_index>,<width_px>,<height_px>,<dpi>
VYOMA_PDF:close:<req_id>
```

Supervisor → App:
```
VYOMA_SYSTEM:pdf-shm-fd:<fd_num> w=<w> h=<h>
VYOMA_PDF:render-done:<req_id>:<page_index>
VYOMA_PDF:page-count:<req_id>:<n>
VYOMA_PDF:error:<req_id>:<reason>
```

Requires `pdf_render = true` AND `filesystem = true` capabilities.

## 3. Core Types

```rust
pub enum PdfCmd { Render { page: u32, width: u32, height: u32, dpi: u32 }, Close }

pub enum PdfObj {
    Null, Bool(bool), Int(i64), Real(f64), Name(String), Str(Vec<u8>),
    Array(Vec<PdfObj>), Dict(HashMap<String, PdfObj>),
    Stream { dict: HashMap<String, PdfObj>, data: Vec<u8> },
    Ref(u32, u16),
}

pub struct XrefTable { pub entries: HashMap<u32, (u64, u16)>, pub root_obj: u32 }
pub struct PdfDocument { pub raw: Vec<u8>, pub xref: XrefTable, pub catalog: HashMap<String, PdfObj>, obj_cache: HashMap<u32, PdfObj> }

#[derive(Clone)]
pub struct GfxState { pub ctm: [f64; 6], pub fill_rgba: u32, pub stroke_rgba: u32, pub line_width: f64 }

#[derive(Clone)]
pub struct TextState { pub font_name: String, pub font_size: f32, pub text_matrix: [f64; 6], pub text_line_matrix: [f64; 6], pub char_spacing: f64, pub word_spacing: f64, pub horizontal_scale: f64 }
```

## 4. PDF Parsing

### Cross-reference table
```rust
pub fn parse_xref(raw: &[u8], offset: u64) -> Result<XrefTable, String> {
    let peek = raw.get(offset as usize..offset as usize + 4).unwrap_or(&[]);
    if peek.starts_with(b"xref") { parse_xref_table(raw, offset) }
    else { parse_xref_stream(raw, offset) }  // PDF 1.5+ compressed xref
}
```

find_startxref: scan last 1024 bytes for "startxref" keyword, parse decimal offset.

### Stream decode
```rust
pub fn decode_stream(dict: &HashMap<String, PdfObj>, raw: Vec<u8>) -> Option<Vec<u8>> {
    // FlateDecode: miniz_oxide::inflate::decompress_to_vec (strip 2-byte zlib header + 4-byte Adler32)
    // DCTDecode: pass through (handled by zune_jpeg at image render time)
    // ASCII85Decode / ASCIIHexDecode: inline decoders
    // Unknown filter: log warning, return raw
}
```

### Object resolution
```rust
impl PdfDocument {
    pub fn resolve(&mut self, obj_num: u32) -> Option<PdfObj> {
        // Cache check → seek to xref offset → parse "N G obj" header → parse object
        // For stream objects: read Length, skip "stream\n", read raw bytes, decode_stream
        // Cache result; return cloned
    }
}
```

## 5. Page Rendering

### Graphics operator dispatch (key ops)
- `q`/`Q`: push/pop GfxState stack
- `cm`: multiply CTM
- `rg`/`RG`/`g`/`G`: set fill/stroke color (DeviceRGB, DeviceGray)
- `m`/`l`/`c`/`h`/`re`: path construction
- `f`/`S`/`B`/`n`: path painting (fill/stroke/both/discard)
- `BT`/`ET`: begin/end text block
- `Tf`/`Tm`/`Td`/`Tj`/`TJ`/`T*`: text state and show string
- `Do`: paint XObject (image)

### Coordinate transform
```rust
pub fn user_to_pixel(x: f64, y: f64, ctm: &[f64;6], sx: f64, sy: f64, page_h: f64) -> (f64, f64) {
    let xp = ctm[0]*x + ctm[2]*y + ctm[4];
    let yp = ctm[1]*x + ctm[3]*y + ctm[5];
    (xp * sx, (page_h - yp) * sy)  // Y-flip: PDF Y=0 bottom, surface Y=0 top
}
```

### Scan-line fill
Flatten path segments to (x0,y0,x1,y1) lines in pixel space. For each scan row: find X intersections, fill between pairs (non-zero winding). Uses surface.fill_rect.

### Image XObjects (Do operator)
- Look up XObject in /Resources dict
- FlateDecode → RGB → RGBA8; DCTDecode via zune_jpeg
- Scale to CTM-defined destination rect via nearest-neighbor

## 6. Text Rendering

Delegate glyph rasterization to R13 FontCache (fontdue). 

Font encoding map: build [u8;256]→char at Tf time. Base encodings: WinAnsiEncoding, MacRomanEncoding, StandardEncoding. /Differences array overrides individual slots via Adobe glyph name list (match statement, ~300 common names).

TJ kerning: negative adjustment = move text matrix right; positive = move left.

## 7. Shared Memory Output

```rust
pub struct ShmSurface { fd: RawFd, size: usize, pub width: u32, pub height: u32 }

impl ShmSurface {
    pub fn new(w: u32, h: u32) -> Result<Self, String> {
        // syscall SYS_memfd_create with MFD_CLOEXEC
        // ftruncate to w*h*4
        // store fd
    }
    pub fn write_from_surface(&self, surface: &Surface) -> Result<(), String> {
        // mmap MAP_SHARED + PROT_WRITE; convert BGRA→RGBA; munmap
    }
    pub fn send_to_app(&self, app_name: &str, inbox: &Inbox, child_pidfd: RawFd) -> Result<(), String> {
        // pidfd_getfd(child_pidfd, self.fd, 0) → child_fd number
        // send "VYOMA_SYSTEM:pdf-shm-fd:<child_fd> w=<w> h=<h>" to app stdin
    }
}
```

## 8. Manifest Capability & Cargo Deps

Add to Capabilities struct: `#[serde(default)] pub pdf_render: bool`

New Cargo deps:
```toml
zune-jpeg  = { version = "0.4", default-features = false }
miniz_oxide = { version = "0.8", default-features = false, features = ["with-alloc"] }
```

## 9. Blocking Issues (B1–B5)

**B1 — memfd/pidfd_getfd requires Linux 5.6+ and AppState.pidfd**  
Add `pidfd: Option<RawFd>` to AppState. Open via `syscall(SYS_pidfd_open, pid, 0)` immediately after child spawn. Close on child exit.

**B2 — PDF 1.5+ compressed xref streams (most modern PDFs)**  
`parse_xref()` detects classic `xref` keyword vs object header at startxref offset. For compressed streams: read W array (field widths), Index array, decompress stream, parse binary entries. /Root from stream dict, not trailer.

**B3 — Y-axis flip must apply AFTER CTM transform, not before**  
All path/text/image coordinate transforms must call `user_to_pixel(x, y, ctm, sx, sy, page_h)` which applies CTM first, then Y-flip. Never apply Y-flip inline in individual operators.

**B4 — Font encoding: raw bytes → wrong glyphs for non-ASCII encodings**  
Build [u8;256]→char map at Tf time from PDF encoding dict (WinAnsiEncoding / MacRomanEncoding / StandardEncoding / Differences array). Pass map to render_pdf_string instead of casting byte as char directly.

**B5 — Worker thread cleanup on app exit**  
`PdfEngine::close_all_for_app(app_name)` sends PdfCmd::Close to all workers keyed `"{app_name}/{req_id}"`. Called from app_threads.rs waiter thread on exit. Worker key format enables prefix scan. PDF_ENGINE stored as OnceLock<PdfEngine>.
