# Round 13 Critique — Font System & Typography
**Role:** Critic | **Date:** 2026-05-29
**Verdict:** REJECT AND REDESIGN — seven blocking issues span safety, correctness, capacity, and i18n. The trait surface and atlas geometry are sound; the security envelope, fallback semantics, and RTL handling require a substantial rewrite before any code lands.

## 1. Verdict summary

- **Safety envelope is fictional.** `catch_unwind` around `Font::from_bytes` catches Rust panics but ignores `abort`, stack overflow (which `abort`s on musl), and SIGSEGV from miscompiled inner loops. PID-2 is one malformed TTF away from death.
- **Validation happens at parse, not at rasterization.** `fontdue::Font::rasterize` can panic on glyphs `from_bytes` accepts; the worker has no second `catch_unwind`, so a poisoned glyph kills the worker thread and freezes every app using TrueType.
- **GPU atlas has no eviction path.** Shelf-pack is one-shot. An adversarial app requesting 10 000 unique (char, size) tuples fills the 2048² page, after which every additional glyph silently falls back to CPU. The architect calls this "logging `font.atlas.full`." It is a permanent denial of GPU rendering for every app sharing the registry.
- **`measure_text` vs `render_glyph` skew is a foreign function call away.** Bitmap and TrueType providers return different advance widths for the same logical font name at the same size; an upgrade between `measure` and `render` (a `select-font` call mid-frame) produces wrap bugs that look like flicker.
- **32 MiB total / 8 MiB per-font cap is incompatible with CJK.** Noto Sans CJK SC is 9–16 MiB on disk and ~32 MiB pre-parsed in fontdue. Per-font cap rejects it outright. Apps targeting Chinese/Japanese/Korean cannot load a primary font on VyomaOS.
- **RTL stub renders backwards Arabic and Hebrew.** Detecting RTL, logging a warning, and rendering LTR is not a stub — it is unreadable text for 400 M+ users. The mitigation is binary: refuse, or reverse the logical order. Logging is the worst option.
- **Fallback terminator silently swallows non-Latin glyphs.** When all TrueType fallbacks miss and the chain hits `BitmapFontProvider`, codepoints outside `0x20..=0x7E` render as blank space. Users see nothing — not a tofu box, not `?`. Bugs become invisible.
- **Single worker thread serializes the whole supervisor's typography.** A 80×24 cold-cache terminal pre-warm (32 ms) blocks every other measurement call. Two GUI apps measuring text concurrently double the latency. The bounded(32) channel turns transient bursts into supervisor stalls.

## 2. BLOCKING issues

### 2.1 `catch_unwind` is not a sandbox for adversarial TTF parsing

**Severity:** Critical — PID-2 stability under untrusted input.

The architect repeats four times that `fontdue` is `#![forbid(unsafe_code)]` and therefore safe. This conflates *memory safety* with *crash safety*. `forbid(unsafe_code)` guarantees no UB; it does not guarantee that `fontdue` will not:

1. **Call `panic!` from a code path not covered by `catch_unwind`.** `from_bytes` panics are caught; `Font::rasterize`, `Font::horizontal_line_metrics`, `Font::lookup_glyph_index`, and `Font::metrics` are all hot paths called from `render_glyph`/`measure_text`/`glyph_advance`/`has_glyph` with **no** `catch_unwind` wrap. The `RuntimeError: arithmetic overflow` panics reported against fontdue (see fontdue issues #103, #117) trigger here, not at load. Even debug-mode arithmetic-overflow panics in release-mode builds turn into wrapping behavior that produces garbage glyphs without a panic, propagating corruption rather than crashing.

2. **Trigger `std::process::abort()` indirectly.** fontdue's allocator interactions on malformed cmap subtables can request multi-GB Vecs; on a musl static binary with no overcommit, the alloc returns null, `Vec::reserve` aborts. `catch_unwind` does **not** catch `abort`. PID-2 dies. The kernel panics with `Attempted to kill init!`. The supervisor's own panic hook (if any) does not run because `abort` skips unwinding entirely. Recovery requires a full kernel reboot — there is no userspace path back.

3. **Stack-overflow on deeply nested TrueType composite glyphs.** The TTF spec caps composite depth at 16, but fontdue does not enforce a limit. A composite glyph referring recursively (legal under the binary format if the cycle check is loose) exhausts the 2 MiB worker thread stack. On Linux, stack overflow delivers SIGSEGV via guard page; on musl, this `abort`s. Same outcome as #2. Mitigation requires either an explicit recursion-depth check inside the rasterizer (not exposed by fontdue's API) or a per-thread stack guard page with a SIGSEGV handler that longjmps — neither of which exists in the spec.

4. **Hang.** Quadratic glyph-table parsers accepting a 256 KB subtable with `n=200 000` glyph indices run for tens of seconds. The worker thread is single-threaded — every other app waits. The bounded(32) channel fills, and subsequent app calls block. Within 100 ms, the supervisor watchdog (if any covers font calls — the spec does not say) trips. If no watchdog covers font calls, the supervisor appears hung. The user's only recourse is to reset the VM.

5. **Algorithmic-complexity attacks via crafted hinting bytecode.** TrueType hinting is a Turing-complete stack machine. fontdue interprets a limited subset; a crafted font triggering pathological loop bounds (legal under the spec) burns CPU without making progress. There is no instruction-count budget in fontdue's interpreter.

**Required fix:** Either (a) run fontdue in a separate subprocess with seccomp and a hard CPU/RAM rlimit, parsing/rasterizing across an IPC channel (high cost, correct), or (b) keep it in-process but wrap **every** fontdue entry point in `catch_unwind`, set a 100 ms per-call timeout via a watchdog thread that kills the worker, and respawn the worker. Option (b) is the realistic v1; option (a) is mandatory before allowing user-provided fonts via the WIT `load-font` call. Option (b) also requires that the worker holds **no** locks across the fontdue call — otherwise the watchdog kill leaves a poisoned `RwLock` and the registry deadlocks.

The current spec does **neither**. It validates once at parse and pretends the rest of the lifetime is safe. The §10 prose claims the worker thread "is distinct from the compositor and the IPC broker, so a slow glyph delays one frame for one app, not the whole supervisor." This is wrong because (i) the bounded channel backpressures all apps once full, and (ii) a worker thread crash on Linux without `SIGCHLD` handling does not auto-respawn; the channel sender side keeps accepting messages that are never consumed.

### 2.2 GPU atlas has no realistic eviction strategy

**Severity:** Critical — adversarial DoS and CJK display failure.

The shelf-packing allocator (`GpuGlyphAtlas::alloc`) is append-only: shelves grow monotonically, never compact. The spec acknowledges this — §4 says "Single-page in v1: when the GPU atlas overflows we log `font.atlas.full` and fall back to CPU rendering for that frame." This is not eviction; it is permanent failure. Once the atlas is full, **every subsequent glyph for the rest of the boot session** falls back to CPU, even if 99 % of the entries are stale.

Worst case under adversarial load:

- App A renders 4096 CJK glyphs at 24 px (~30×40 px each = 4800 B; 4096 × 4800 B = 19.6 MiB on the 4 MiB page → atlas full before 10 % of the working set is mapped).
- App B renders the next glyph and CPU-falls-back forever.
- App C does the same. Now three apps are CPU-bound for text; the GPU compositor pipeline (R12) becomes the bottleneck-free path that the workload never takes.

Worst case under benign load:

- User switches between 5 documents at 5 different point sizes (10, 12, 14, 16, 18 px). 95 ASCII glyphs × 5 sizes × 3 fonts = 1425 entries before non-ASCII even starts. A 24 px CJK font adds 3500+ glyphs over a session; the page fills in 30 minutes of normal use.
- A long-running terminal app rotates through a chat log; every distinct character at every distinct size accumulates. The session naturally fills the atlas in hours, never seconds, but the cliff is permanent.

Shelf-pack also wastes space inside each shelf. A shelf opened for a 32 px glyph stays open at 32 px height; subsequent 12 px glyphs allocated into it waste `(32-12) * advance` per glyph. Over a session mixing UI sizes (12, 14, 16, 24, 32 px), the effective utilization drops to 40–50 % — meaning the cliff arrives 2× sooner than the raw byte count suggests.

The architect's `take_dirty()` is a frame-coherence flag, not an eviction signal. There is no `regions` map invalidation, no `shelves` rebuild, no reference counting. `regions.read()` on line 423 is the hot read path but never compacts; `regions.write()` on line 440 only inserts. There is no `evict()`, no `compact()`, no `clear_unused()`.

**Required fix:** v1 must either (a) declare a hard glyph-count cap on the GPU atlas (e.g., 2048 entries) and re-pack from scratch on overflow via a full GPU upload — costs ~16 ms one-shot and is acceptable as a backstop, or (b) move to a dynamic packer (Skyline-BL or guillotine with reference counts). Both have well-known implementations (`etagere` crate, `rect_packer` crate); the spec proposes neither. The "log and CPU-fall-back forever" path is unacceptable because (i) CPU fallback breaks the R12 throughput model, (ii) the failure mode is silent to the user, and (iii) the failure mode is *adversarial-app-attributable but blame-broadcast-to-all-apps*.

Also: the atlas is shared across all apps (§12), so the failure is **not per-app**. App A exhausting the atlas degrades every other app. This contradicts the capability isolation that is the foundation of the supervisor. A correct design either (i) gives each app its own atlas slice with a quota, or (ii) accounts atlas occupancy to PIDs and reclaims the largest-occupying PID's regions when overflow is imminent. The current design accomplishes neither.

### 2.3 `measure_text` and `render_glyph` advance widths can desynchronize

**Severity:** High — layout corruption that looks like a flicker bug.

`BitmapFontProvider::glyph_advance` returns `4.0/8.0/16.0` depending on size bucket. `TrueTypeFontProvider::glyph_advance` returns `font.metrics(ch, size_px).advance_width`, which is a float in fractional pixels. The two are not interchangeable, yet the layout engine (§7) calls `chain.resolve(ch, registry).glyph_advance(...)` per character — and the resolved provider can change between calls if the fallback chain changes (e.g., a `select-font` mid-frame, or a font finishing async load).

Concrete bug:

```
text:     "Hello, 世界"
provider: VyomaSans (TrueType)  → advance 'H' = 8.7
provider: BitmapFont (terminator) → advance '世' = 8.0
layout:   measures Hello with TrueType, '世界' with Bitmap → 5×8.7 + 2×8.0 = 59.5
render:   `select-font` arrives between measure and render
render:   re-measures '世' with newly loaded NotoCJK → advance = 16.0
result:   measured 59.5 px; rendered 76 px → text overruns its container.
```

The spec has no contract that pins `(font_id, size_px) → advance` for the duration of a layout pass. There is no snapshot, no version, no lock. Apps doing UI layout (text fields, buttons, list rows) will see intermittent wrapping defects that disappear on reflow.

A second class of skew arises from quantization: `measure_text` does not quantize size (line 274 calls `font.metrics(ch, size_px)` with the raw f32), but `render_glyph` quantizes via `Self::quantize_size(size_px)` for the cache key (line 287). The cached bitmap was rasterized at size `quantize_size(size_px) / 4 = floor(size_px * 4) / 4`, while the advance was measured at the unquantized size. At 16.1 px, measurement uses 16.1, rendering uses 16.0 — a 0.6 % advance discrepancy per glyph that compounds over a line.

**Required fix:** Either (a) `layout_text` must take an exclusive read lock on the registry for its duration, returning a `LayoutSnapshot` that binds font IDs to providers, or (b) `measure_text` and `render_glyph` must be called through a single `LayoutSession` object that caches per-glyph advance and bitmap together, so the second call reads from the session, not the registry. Additionally, `measure_text` must call the same `quantize_size` path as `render_glyph` so the advance returned matches the advance of the bitmap that will actually render. The current trait design pretends providers are stateless and registry lookups are idempotent; in the presence of `select-font`, `load-font`, and quantization, neither is true.

### 2.4 32 MiB total / 8 MiB per-font is undersized for CJK

**Severity:** High — locks VyomaOS out of CJK markets.

Real-world CJK font sizes (full coverage, hinted, subset to common 8000 glyphs):

| Font | TTF size | fontdue parsed RAM (est) |
|---|---|---|
| Noto Sans CJK SC Regular | 16.1 MB | 38 MB |
| Noto Sans CJK JP Regular | 16.5 MB | 39 MB |
| Source Han Sans SC | 14.2 MB | 34 MB |
| HarmonyOS Sans SC | 15.8 MB | 37 MB |

`MAX_FONT_BYTES = 8 * 1024 * 1024` (§5, line 474) rejects every one of these at the registry boundary. Even if the per-font cap were lifted, `MAX_TOTAL_FONT_RAM = 32 * 1024 * 1024` accommodates fewer than one CJK font once parsed.

The architect provides no per-platform scaling and no documentation that VyomaOS is Latin-only by design. Apps cannot work around this — the cap is enforced inside the supervisor, before any app-side capability can request relief.

**Required fix:** Per-platform caps, declared in the platform profile TOML (`supervisor/src/profile/profiles/`):

```toml
# desktop-full.toml
[fonts]
max_font_bytes = 33554432         # 32 MiB
max_total_ram = 134217728         # 128 MiB
max_glyph_cache_entries = 16384

# mobile.toml
[fonts]
max_font_bytes = 16777216         # 16 MiB
max_total_ram = 67108864          # 64 MiB
max_glyph_cache_entries = 8192

# iot-edge.toml
[fonts]
max_font_bytes = 4194304          # 4 MiB
max_total_ram = 4194304           # 4 MiB
max_glyph_cache_entries = 256

# mcu-minimal.toml
# no TrueType — bitmap only
```

And the spec must explicitly call out that CJK requires `desktop-full` or `server-headless`. Without this, the Phase 17 architecture (which already supports 6 platforms) gets contradicted by Round 13.

Secondary point: even for Latin scripts, the 32 MiB total combined with 4096-entry per-font LRU at ~2 KiB/glyph max is internally inconsistent. 4096 × 2 KiB = 8 MiB per font for the cache alone, plus the parsed font (1–4 MiB), plus the raw bytes (held via the FontRegistry for `bytes_held` accounting?). A single TrueType font at maximum cache occupancy consumes 10–12 MiB. Three loaded fonts blow the 32 MiB cap before any non-trivial CJK is even attempted. The cap accounting (line 532: `inner.total_bytes += data.len()`) tracks only the raw bytes, not the parsed structure or the glyph cache — the actual RAM footprint can be 3–5× the tracked count. The cap is therefore a fiction: it does not bound the real allocation.

Correct accounting would track:
- Raw file bytes (cheap to measure, but doesn't reflect runtime RAM).
- Parsed `fontdue::Font` size (no public API to measure, but estimable from the input file size × 2).
- Glyph cache estimated size (4096 × avg glyph footprint).

…and either expose a `RegisteredFont::estimated_ram()` method that the registry sums on every load and eviction, or pre-allocate a fixed pool per font and reject when the pool is at quota.

### 2.5 RTL rendering as a "logged stub" is a user-visible bug

**Severity:** High — produces unreadable text for Arabic and Hebrew users.

§7, line 695:

```rust
if detect_rtl(text) {
    log::warn!("font.layout.rtl_unsupported: rendering LTR as a stub (v1 limitation)");
}
```

For an Arabic string "مرحبا بالعالم" ("Hello, world"), the spec:

1. Detects RTL via the first strong-direction codepoint check (§7, `detect_rtl`).
2. Logs a warning to the supervisor stderr (the user never sees this).
3. Calls `layout_wrap_char` or `layout_wrap_word`, which iterates `text.char_indices()` left-to-right and renders glyphs left-to-right.

The result on screen: "ا ب ل ا ع ل ا ب ا ب ح ر م" — letters in logical (memory) order rather than visual order. Arabic glyphs are also disconnected (no joining forms), because no shaping happens. For a Hebrew string, the same defect.

A user typing into an Arabic text field sees their input fly to the wrong end of the field and disconnect into isolated letterforms. This is not a degradation; it is a wrong answer. The current spec then *records the wrong answer in the log file* with the comment "(v1 limitation)" — implying it is a known partial implementation. Partial implementation suggests it works partially. It does not work at all.

**Required fix:** One of the following, in order of preference:

1. (Preferred) `layout_text` returns `Err(LayoutError::RtlUnsupported)` for RTL input. Apps surface a clean error and have a known contract. The supervisor logs an `font.layout.rejected_rtl` metric so app authors can see the failure rate.
2. Reverse the byte order of the RTL string before layout, producing visually-ordered (but unjoined) glyphs. Arabic remains unreadable but at least scans right-to-left. Hebrew is partially correct (Hebrew has no joining requirement). UTF-8 char-reverse is correct (operate on `char` iter, not byte iter), and the layout engine processes the reversed sequence LTR — the visual order matches the conventional right-to-left scan for the reader.
3. Pre-process Arabic text via a static substitution table from isolated forms to initial/medial/final forms — a 100-line lookup table covers basic Arabic — then reverse. Not full bidi, but usable for short labels (button text, menu items, notifications). Not suitable for paragraphs (no line-break opportunities, no kashida insertion, no diacritic positioning).

The current "log and render LTR" must be removed before this proposal merges. An additional consideration: the `detect_rtl` predicate (line 794) returns true on the first strong-RTL codepoint and false on the first strong-LTR. Mixed-direction text ("see مرحبا below") is classified by whichever direction appears first. Pure Latin text with an embedded Arabic word is currently mis-classified as LTR (correct), but pure Arabic text with an embedded English word is classified as RTL (correct), and the predicate processes neither correctly. The Unicode Bidirectional Algorithm (UAX #9) defines the correct paragraph-direction heuristic; the spec's first-strong is the right *first approximation*, but it needs documentation of what "RTL detection" means and what kinds of text it gets wrong.

### 2.6 Bitmap terminator silently drops non-ASCII

**Severity:** High — invisible bugs.

`BitmapFontProvider::has_glyph` returns `false` for any codepoint outside `0x20..=0x7E` (§2, line 141). `FallbackChain::resolve` (§6, line 627) iterates the chain, finds `has_glyph` false for every provider including the terminator, and falls through to `registry.get(BITMAP_FONT_ID).expect(...)`. That provider then renders `'?'` via the substitution at `BitmapFontProvider::render_glyph` (line 125 — "if (ch as u32) < 0x20 || > 0x7E → render '?'").

Wait — the bitmap **does** render `'?'` for unknown glyphs. But the fallback chain does not know that, because `has_glyph` lied. So intermediate fallbacks never get a chance to render via the bitmap-as-`?` path; they fall through to the terminator which renders `?`. That part works.

But — the bitmap `'?'` is 8 px wide at native size. A 24 px UI showing "Hello, 世界" gets a 24 px-rendered "Hello, " followed by two **8 px** `?` glyphs. The visual asymmetry signals "broken font," but the layout engine measured the run as 16 px wide per `'世'` (assuming TrueType was probed first). The advance and the bitmap are again desynchronized — same bug as 2.3, but downstream of the fallback rather than from `select-font`.

Worse: if the user installs a font missing only the rare codepoint U+00FF (ÿ) and the bitmap returns false for it (it is in Latin-1 Supplement, > 0x7E), then `ÿ` is rendered as bitmap-`?`, even though the *intended* font has the glyph. The architect's `has_glyph` for the bitmap is wrong: it should return `true` for all codepoints, since the bitmap will always render *something*. But then the fallback chain stops at the bitmap for every codepoint, which is also wrong.

**Required fix:** Introduce an explicit "renders as substitution" return from `has_glyph`. Use a tri-state: `GlyphStatus::Native`, `GlyphStatus::Substitution`, `GlyphStatus::Absent`. `FallbackChain::resolve` prefers Native over Substitution. The terminator returns Substitution for non-ASCII and Native for ASCII. This gives intermediate TrueType fonts a chance to provide the glyph natively before falling back to bitmap-`?`.

Additionally, the bitmap must render U+FFFD (`�`, the replacement character) as a visible boxed-question-mark instead of `'?'`, so missing glyphs are visually distinguishable from literal `'?'` in the source text. Adding U+FFFD as a 96th glyph in the bitmap table is a 16-byte change to `bitmap_data.rs`.

Third related defect: the architect's `font.rasterize` for `'.notdef'` returns an empty bitmap (zero width, zero height). The current `FallbackChain::resolve` checks `has_glyph` (returns false for missing) and falls through to the bitmap. But if `has_glyph` is implemented via `lookup_glyph_index != 0` (line 312), a font with a properly-installed `.notdef` glyph at index 0 returns false — fine. A font *without* `.notdef` (rare but legal) returns 0 for any unknown glyph, which is also index 0 — same result. But a font that maps an unknown codepoint to a *non-zero* glyph index (some Asian fonts intentionally do this for unmapped Latin to a stylized box) returns true — and renders a stylized box, while the fallback chain thinks the font has the glyph and skips fallback. Cascading misrender.

The architect needs a stronger glyph-presence test than `lookup_glyph_index`. Either check the `cmap` table directly via fontdue's `chars()` API (returns the set of supported codepoints), or maintain an explicit per-font character set. The cost is one HashSet<char> per loaded font (~1 KB for 400 glyphs), well within budget.

### 2.7 Validation at parse time, not at rasterization

**Severity:** Critical — circumvents the entire security argument.

`validate_font_file` (§5, line 564) is the architect's primary defense against malicious fonts. It calls `catch_unwind(|| fontdue::Font::from_bytes(...))`. Pass → font enters registry.

But the actual hot path that runs on rendered text — `TrueTypeFontProvider::render_glyph` → `font.rasterize(ch, size_px)` — is **not** wrapped in `catch_unwind`. If a glyph parses cleanly but rasterizes incorrectly (a malformed `glyf` table the parser allowed but the rasterizer trips on), the panic propagates up through `render_glyph` → `chain.render_run` → the compositor thread, killing the compositor. The compositor is **not** the worker thread — it is a separate thread that processes flushes from all apps. Its death stops all rendering, even for apps not using TrueType.

This is not hypothetical. fontdue's history of CVE-style issues (the project's CHANGELOG references multiple panic fixes between 0.7 and 0.9) shows parsing and rasterization have **independent** crash surfaces. The `from_bytes` validation pass exercises only the table-of-contents parsing; it does not touch the `glyf`/`cff` outline data at all. A font with a perfectly-formed header and pathological outline data passes validation and bombs at first glyph.

Additionally: the worker thread calls `registry.get(font)` → `p.render_glyph(...)`. The `render_glyph` call goes through the Arc<dyn FontProvider> indirection to `TrueTypeFontProvider::render_glyph`, which calls `font.rasterize` directly with **no** catch_unwind. If the rasterize panics, the worker thread panics. crossbeam's channel does not propagate panics across threads; the sender side keeps queueing requests that never get processed. The reply channel sender (line 1027) `let _ = reply.send(result);` is the last line before the panic — the reply is never sent, the caller blocks forever (no timeout on `recv`), and the compositor freezes.

**Required fix:** Every fontdue entry point — `rasterize`, `metrics`, `horizontal_line_metrics`, `lookup_glyph_index` — must be wrapped. Either:

```rust
fn safe_rasterize(font: &fontdue::Font, ch: char, size: f32)
    -> Result<(Metrics, Vec<u8>), FontError>
{
    std::panic::catch_unwind(AssertUnwindSafe(|| font.rasterize(ch, size)))
        .map_err(|_| FontError::RasterPanic)
}
```

…called for every glyph, or move the entire fontdue interaction behind the worker thread with a watchdog. The cost is a `catch_unwind` per glyph (~50 ns); the LRU absorbs this overhead for warm reads. Cold reads pay 50 ns extra on a 40 µs raster — a 0.1 % tax for not crashing the supervisor. Reply channels must use `recv_timeout` (e.g., 200 ms) on the caller side; on timeout, return a substitution glyph from the bitmap fallback. The worker must respawn on panic via a parent monitor thread that joins the worker handle and detects the panic via `is_finished()` + `join().is_err()`.

The spec presents `catch_unwind` at load as sufficient. It is not. It is a single layer of safety on the *least likely* failure path, leaving the most likely failure path (per-glyph rasterization) completely exposed.

## 3. NON-BLOCKING concerns

### 3.1 Single global worker is a structural bottleneck

§10 line 1006: `let (tx, rx) = bounded::<FontReq>(32);`. One worker drains. The 80×24 terminal pre-warm enqueues 95 glyphs. Other apps wait 32 ms (4 ms latency for an ASCII char × 8 deep queue) before any of their measurements return. Under 4 GUI apps doing concurrent text measurement, p99 latency multiplies.

Recommendation: 2–4 worker threads keyed by `FontId mod N`. Each font's glyph stream is serialized (preserving cache coherence on the LRU), but inter-font contention parallelizes. Open Question 6 acknowledges this; it should be answered "yes" in v1, not deferred.

### 3.2 `peek` in `render_glyph` does not bump LRU

`render_glyph` (§3, line 288) uses `self.cache.read().peek(&key)`. `lru::LruCache::peek` deliberately does **not** update the access order. After 4096 unique glyphs, the next `put` evicts whatever the LRU thinks is least-recently-used — but `peek` never updated MRU, so the actual MRU glyph (the one the cursor character at 14 px Regular) is evicted on the next miss. The LRU degrades to FIFO under read-heavy load.

Fix: use `get` instead of `peek`, accept the write-lock upgrade cost, or maintain a separate hot-set tracker.

### 3.3 `peek` returns `Arc<GlyphBitmap>` then `clone()`s the inner

Line 289: `return (*bmp).clone();` — dereferences the Arc and clones the inner `GlyphBitmap` (which contains a `Vec<u8>` of up to 2 KiB). The whole point of `Arc` was zero-copy sharing. The signature returns `GlyphBitmap` by value, forcing the clone. Change the return type to `Arc<GlyphBitmap>` and update the trait.

Saved on every warm read: ~500 bytes memcpy + Vec allocation. Per frame at 1920 glyphs, ~1 MB allocation churn at 60 Hz.

### 3.4 `metrics_cache` is declared but never used

§3, line 208: `metrics_cache: RwLock<HashMap<(u8, u16), f32>>` is initialized but no method reads from or writes to it. Either remove it (dead code) or wire it into `line_height` / `glyph_advance`. If wired, the key `(u8, u16)` is too narrow — `u8` for weight and `u16` for size_q implies metrics depend only on size and weight, not on glyph; that is true for `line_height` but not `glyph_advance`. The design is incomplete.

### 3.5 Quantization to 0.25 px is too coarse for small sizes

Open Question 1 asks about quantization. At 12 px, 0.25 px error is 2 % — noticeable on baseline alignment in tabular data. Use 8.8 fixed-point (1/256 px) as the architect suggests; the cache-key space gains `log2(64)` = 6 bits but the working set is still bounded by the LRU cap.

### 3.6 `font_for(weight)` silently aliases Bold → Regular

§3, line 240–244: `FontWeight::Bold → self.bold.as_ref().unwrap_or(&self.regular)`. An app requests Bold; the system silently renders Regular. No log, no error, no metric. UI bugs where headings appear normal-weight are invisible until a designer notices. At minimum: log once per (font, weight) mismatch; better: return an `Option<&fontdue::Font>` so the layout engine can synthesize bold via stroke-widening when the weight is unavailable.

### 3.7 `BitmapFontProvider::render_glyph` ignores requested size for non-buckets

§2 returns 4×8, 8×16, or 16×32 — the size is rounded to the nearest bucket. An app requesting 18 px (between m=16 and l=20) gets 8×16 — a 12 % size error. Compositor was promised pixel-precise text via `draw_text_scaled`; it gets bucketed bitmap text. This contradicts R11.

Fix: scale the bitmap by the exact ratio (`size_px / 16.0`) with bilinear filtering, or document explicitly that bitmap is bucketed and TrueType is precise — so apps know to require TrueType for HiDPI.

### 3.8 Open Questions defer decisions that block implementation

Of the 10 open questions, 5 are blocking for v1:

- Q1 (quantization granularity) — affects cache key layout.
- Q2 (CpuGlyphAtlas eviction) — current "drop random fraction" is incorrect; LRU required.
- Q5 (RTL policy) — see Issue 2.5; must be decided before merging.
- Q6 (worker pool) — see Issue 3.1; bottleneck unblocked by pool.
- Q9 (atlas overflow) — see Issue 2.2; current strategy is incorrect.

Q3 (system font choice), Q4 (bitmap range semantics), Q7 (hot-reload), Q8 (color), Q10 (default wrap) can defer. The proposal should commit on Q1/Q2/Q5/Q6/Q9 before merging.

### 3.9 `FontId = u8` with 64-slot cap is small but justified — except for slot 0 collision

`BITMAP_FONT_ID = 0` and the WIT `load-font` returns `u32`. An app expecting to compare returned IDs against 0 to detect "no font loaded" gets a false positive — slot 0 *is* a valid font. Use `Option<u32>` in the WIT result or reserve slot 0 as "no font" and place the bitmap at slot 1. This is a one-line change with high downstream impact.

### 3.10 `unload_owned_by` does not invalidate the glyph atlas

§5, line 543: dropping an app's font slot leaves `GpuGlyphAtlas::regions` referencing glyphs from that font. Subsequent renders read stale UV coordinates against a possibly-recycled atlas region. The atlas must be told `font_id` is gone and its glyphs invalidated. Currently nothing calls `invalidate_font` on the atlas from the registry.

### 3.11 `Vec<Option<RegisteredFont>>` for 64 slots is fine; `position(Option::is_none)` is O(64) under contention

§5 acquires the write lock and linearly scans for an empty slot. 64 slots under a write lock is < 1 µs — acceptable. But the write lock is held for the entire `from_bytes` call (multi-millisecond for a 4 MB font), serializing all `load_font` calls. Split: scan and reserve a slot under a short lock, do parsing without holding the lock, then commit under a second short lock. Pattern: optimistic reservation with rollback.

### 3.12 `LayoutOpts` and `TextLine` lifetimes do not survive the WIT boundary

§7 returns `Vec<TextLine<'a>>` borrowing from the input `&'a str`. The WIT host-side handler (§8) receives `text: String` (owned, copied across the WASM boundary). The lifetime is the local stack frame; the layout result cannot be returned to the WASM guest. Either return `Vec<OwnedTextLine>` (with owned `String`s — extra allocation), or expose the layout iteratively via a stateful session. The current `'a` design is purely in-supervisor; it does not extend to apps.

### 3.13 `worker.rs` 130 LOC for a long-running thread without a shutdown signal

The worker loop `while let Ok(req) = rx.recv()` exits when all senders drop. The architect's `FontWorker { tx: Sender<FontReq> }` is held by the supervisor for the process lifetime. There is no shutdown path. Hot-reloading the registry (e.g., during platform-profile swap) cannot drain the worker. Add a `Shutdown` request variant and join the thread on supervisor exit.

### 3.14 No metrics for atlas hit rate or LRU eviction frequency

R11/R12 invested heavily in structured logging. Round 13 logs `font.system.loaded`, `font.system.error`, `font.atlas.full`, `font.worker.queue_full`. None of: cache hit/miss rate, LRU evictions/sec, atlas occupancy %, p50/p99 raster latency. The proposal cannot be tuned in production without these. Add a `FontMetrics` struct emitted via the supervisor's `observability/` subsystem (per CLAUDE.md, this exists since spec-043).

### 3.15 Subsetting of Inter/JetBrains Mono to ~200 KiB is not specified

§5 says "all subsets contain Latin + Latin-Extended-A + common punctuation (~400 glyphs each), total system-font footprint under 2 MiB." The subsetting tool (pyftsubset? fonttools?) is not specified, no reproducible script is committed, and "common punctuation" is hand-wavy. Build determinism (a stated VyomaOS goal) requires a checked-in subset spec and a deterministic subsetting step in the Dockerfile.

### 3.16 `select_font` race between WIT call and stdout protocol

§8 sets `*store.data().selected.lock() = font as u8`. The VYOMA_DRAW_V2 parser reads this on the next `draw_text_scaled`. Between the WIT call's atomic write and the next protocol line, the app can write thousands of bytes of draw commands. Earlier glyphs use the previous font; later glyphs use the new font. Same line of text, mixed fonts. The protocol needs a flush/barrier semantic: after `select_font` returns, all subsequent `draw_text_scaled` lines use the new font, no in-flight buffering.

### 3.17 Font weight enum lacks `Medium`/`Semibold`/`Black`

UI design systems (the Apple-fidelity goal of branch 046) routinely require 4–6 weights. `FontWeight { Regular, Bold, Light }` is undersized for "Apple UI fidelity." San Francisco ships in 9 weights. The WIT enum is hardcoded at `1.0.0`; adding weights post-stabilization breaks the manifest.

Defer is reasonable, but the spec should reserve enum space (e.g., `u8`-encoded weight 100–900 per CSS) instead of a 3-variant enum, so v2 is additive not breaking.

### 3.18 `FallbackChain::with_intermediate` is O(N) insert

Acceptable at N≤4 fallbacks. But the API allows arbitrary chain length and offers no cap. An app constructing a 20-deep fallback chain pays 20 `has_glyph` calls per character. Cap at MAX_FALLBACK_DEPTH = 8 with a hard error.

### 3.19 No font-file integrity check between validation and use

The validate→load flow reads the file from disk once for validation, again for parsing. Between the two reads, the file can change (`/data` is on 9P, writeable). TOCTOU vulnerability if validation enforces invariants the parser relies on. Read once, validate from the in-memory buffer.

Actually — re-reading the code, both validate and parse use the same `&[u8]` passed in. Fine. But the comment "validate without panicking" implies a separate step; if the architect adds a separate file read in the future, the TOCTOU appears. Document the requirement.

### 3.20 `crossbeam::channel` adds a dependency not currently in supervisor

`supervisor/Cargo.toml` does not currently include `crossbeam`. The spec adds it implicitly. Compare against `std::sync::mpsc` — slower but no new dep. For 32-deep bounded channel with bounded backpressure, `crossbeam::channel::bounded` is the right call, but the dependency add must be flagged for review. `crossbeam::channel` is ~80 KB stripped — non-trivial against the 697 KB supervisor binary budget mentioned in CLAUDE.md.

### 3.21 `lru` crate dependency similarly unstated

§3 uses `lru::LruCache`. This adds the `lru` crate (~15 KB), `hashbrown` indirectly, etc. The dependency footprint of the font subsystem grows: fontdue (~85 KB) + lru (~15 KB) + crossbeam (~80 KB) + thiserror (~20 KB) = ~200 KB additional binary size. The architect should sum these and reconcile against the supervisor binary size target.

### 3.22 No reproducibility for `fontdue` non-determinism

Anti-aliasing thresholds in fontdue are platform-independent (pure CPU floats), but the order of glyph rasterization can affect cache state. Two different boots rendering the same text in different scroll orders produce identical bitmaps but different cache contents — fine. However, if a future fontdue version changes its rasterization algorithm (sub-pixel positioning, hinting heuristics), pre-rendered golden bitmaps in tests break silently. Pin fontdue version and document the binary-stability contract.

### 3.23 `BitmapFontProvider::measure_text` returns wrong baseline at 4×8 bucket

Line 121: `TextMetrics { width, height, baseline: height * 0.8 }`. At height=8, baseline=6.4. At native 8×16, baseline=12.8 (close to the `bearing_y: 13` in `expand_1x`). At upsampled 16×32, baseline=25.6 vs `bearing_y: 26` — close. But these are decoupled magic numbers; a refactor that changes the bitmap font metrics breaks `measure_text` silently. Derive baseline from a single shared constant or struct, not from independent fudge factors.

### 3.24 `MAX_FONT_BYTES` check is duplicated and inconsistent

`load_app_font` (line 512) checks `data.len() > MAX_FONT_BYTES` and returns `FontError::TooLarge`. `validate_font_file` (line 565) does the same check. `load_internal` does not. `load_system_font` does not check at all — system fonts can exceed the cap. Consolidate: one check at the registry boundary, applied uniformly. Currently, a 16 MB system font would be loaded despite the cap.

### 3.25 `font_for(weight)` on `Light` falls back to `Regular`, not `Light`-via-synthesis

Light → Regular substitution produces visually heavier text than the app requested. For light-on-light themes (which the Apple-fidelity branch likely uses for hover states), this is a visible defect. Document the substitution rule, or synthesize light by thinning Regular at raster time (~30 LOC via erosion morph on the glyph bitmap).

### 3.26 `RegisteredFont.bytes_held` accounting is incomplete

The registry tracks `bytes_held = data.len()` — the raw font file size. But the actual RAM held by `Arc<dyn FontProvider>` includes the parsed `fontdue::Font` structure (1–4× raw size), the LRU glyph cache (up to 2 MiB at full occupancy), and the empty slots in `Vec<Option<RegisteredFont>>`. The `MAX_TOTAL_FONT_RAM = 32 MiB` cap is enforced against a metric that under-counts by 3–5×. The user is sold a 32 MiB ceiling and gets a 100 MiB ceiling.

### 3.27 `peek` vs `get` race in `TrueTypeFontProvider::render_glyph`

Lines 288–298: `peek` under read lock; if miss, rasterize without holding lock; write the result under write lock. Two concurrent calls for the same glyph both rasterize. The second `put` overwrites the first's entry. Wasted work, no correctness bug — but at fontdue's 40 µs raster cost, this matters on cold-cache scroll bursts where many threads hit miss simultaneously. Use `get_or_insert_with` semantics: a `parking_lot::Mutex<HashMap>` with double-check on entry. Or pre-warm.

### 3.28 No backpressure differentiation between IPC and rendering

The bounded(32) channel mixes `Rasterize` and `Validate` requests in one queue. A 4 MB font validation (slow, ~5 ms) blocks all in-flight rasterizations. Split into two queues, or prioritize raster requests (which are on the user-visible critical path) over validation (which is asynchronous).

### 3.29 Atlas `pixels.write()` holds the lock during the full memcpy

Lines 427–434 hold a write lock on the 4 MB atlas pixels Vec for the entire glyph upload. During upload, no reader (including `get_or_insert` for other glyphs) can proceed. For a 32×32 glyph, the memcpy is ~3 µs; under a write lock, every other thread serializes. Use atomic offset bumping and lockless memcpy via raw pointer + `Cell` discipline — or accept the lock cost and batch uploads into frame boundaries.

### 3.30 Manifest `[capabilities.fonts]` does not declare a fallback chain shape

The architect's manifest spec lists `system` and `bundle` fonts as flat lists. The resolved `FallbackChain` (§9 `FontsCapability::resolve`) uses manifest order as fallback order. But the app might want a different *primary* font versus *fallback* font. E.g., `primary = "MyCustomFont"`, `fallback = ["VyomaSans-Regular", "VyomaMono-Regular"]`. The current schema cannot express this distinction — every loaded font is both primary and fallback in declaration order.

### 3.31 No protocol for runtime font discovery from apps

The WIT interface exposes `load-font(name)` which loads a system font *or* fails. There is no `list-fonts()` to discover what is available. An app that wants to render Bengali either knows ahead of time that `NotoSansBengali` is on the system (brittle) or attempts loads in a predefined order until one succeeds (chatty). Add `list-system-fonts() -> list<string>` to the WIT.

### 3.32 `unload-font` is a no-op (§8 line 870)

`unload_font` accepts a font handle and does nothing. An app that loads 60 bundled fonts and "unloads" them never frees RAM — the registry retains them until process exit. `unload_owned_by` (line 543) is called on process exit, but during a long-running app's lifetime, font RAM accumulates without recovery. Either implement `unload-font` or document it as advisory and rename to `release-font-handle`.

### 3.33 No tracing of per-glyph cost for the user

When a UI feels slow because of font rasterization, the user has no diagnostic. The structured-log entries (`font.system.loaded`, `font.atlas.full`) are coarse. Add a per-frame counter: glyph-cache hits, glyph-cache misses, atlas uploads, rasterization microseconds. Surfaced via `ps`-style introspection or a `/proc/vyoma/font_stats` virtual file.

### 3.34 `Arc<dyn FontProvider>` cloning per glyph in `FallbackChain::resolve`

`resolve` clones an `Arc<dyn FontProvider>` per glyph (line 629). Arc clone is one atomic increment (~5 ns) — cheap but not free. Across 1920 glyphs × 60 Hz = 115 200 Arc clones/sec on the hot path. Acceptable but worth noting; a `&dyn FontProvider` borrow would avoid it if the lifetime can be plumbed through `RunGlyph`.

### 3.35 The `font_name` accessor leaks PII for app-loaded fonts

`provider.font_name()` returns the human-readable name. For an app-bundled font, this name can encode user identity ("hbarve-personal-font.ttf"). The `RunGlyph::provider_name` field (line 656) propagates this into rendering output, where it can leak across app boundaries via the compositor's shared atlas metadata. Either redact app-loaded font names in cross-app contexts or document the leak.

### 3.36 Default `FontSettings::default()` may not be deterministic across versions

`fontdue::FontSettings::default()` (lines 220, 567) uses the crate's defaults at compile time. A `cargo update` that pulls a new fontdue minor version can change defaults (e.g., scale, collection index), producing different glyph bitmaps. Bake an explicit `FontSettings { collection_index: 0, scale: 40.0 }` so the rasterization parameters are part of VyomaOS's spec, not fontdue's.

### 3.37 `lru::LruCache::peek` does not return a clone — but `Arc<GlyphBitmap>` does

Line 288 returns `self.cache.read().peek(&key).cloned()` — this clones the `Option<Arc<GlyphBitmap>>`. The clone of an Arc is cheap; the clone of the inner GlyphBitmap is the expensive one (Issue 3.3). The two issues compound: on a hit, we clone the Arc (cheap), unwrap it, dereference, clone the bitmap (expensive). The fix is to return Arc through the trait — but that breaks the trait surface and requires changing every caller.

### 3.38 No charset coverage assertion at system-font load

When `boot_load_system_fonts` succeeds for `VyomaSans-Regular`, the supervisor assumes the font covers ASCII + Latin-Extended-A. There is no runtime check. A subsetted variant of Inter that accidentally drops "ÿ" passes loading and silently fails at first use. Add a post-load assertion: for each system font, verify the expected character coverage matches the manifest declaration; on mismatch, demote to a warning and continue (not refuse — partial coverage is better than no font).

### 3.39 `FontRegistry::get` returns Arc, not &dyn — locks the read lock for cloning

§5 line 536: `get` acquires `inner.read()` and returns a cloned Arc. The clone happens under the read lock. Fine for one call, but in `FallbackChain::resolve` (called per glyph), the lock is acquired and released per character. For a 1920-glyph terminal redraw, 1920 read-lock acquisitions. `parking_lot::RwLock` read acquisition is ~10 ns uncontended — 19 µs total — acceptable. Under writer starvation, this can spike. Cache the resolved Arc per layout pass to avoid repeated lookups.

### 3.40 No fuzz harness for fontdue input

Given the safety critique above, the spec should commit to a `cargo fuzz` harness that feeds arbitrary bytes to `validate_font_file` and to each fontdue entry point used by `TrueTypeFontProvider`. A reasonable target: 24 hours of fuzzing before the first release, with corpus seeded from common malformed-TTF databases (e.g., font-test-suite, ots-fuzzer corpus). Without this, the safety claims are unverified.

### 3.41 The `name` field in `RegisteredFont` is duplicated

§5 line 484–489: `RegisteredFont` stores `name: String`, and the wrapped `provider: Arc<dyn FontProvider>` also exposes `font_name() -> &str`. Two sources of truth; if they diverge (e.g., the provider's internal name differs from the registration name), introspection lies. Remove `name` and delegate to `provider.font_name()` — saves 24 bytes per slot and one source of inconsistency.

### 3.42 Per-frame budget for font work is not allocated

R12 has its own frame budget. R13 introduces additional CPU work (rasterization on miss, atlas upload on dirty). The two budgets are not reconciled. The spec needs a target ("font subsystem may consume at most 2 ms of any frame") and an enforcement mechanism (timer + early-fallback-to-bitmap when budget is exceeded). Without this, R12's frame pacing degrades silently under font cold loads.

### 3.43 `WrapMode::Ellipsis` returns `TextLine` without the ellipsis character

§7 `layout_ellipsis` (lines 773–791) returns `TextLine { text: line_text, ... }` where `text` is the truncated substring without `…`. The comment "the caller draws `…` at `line.width - ell_w`" (line 806) puts the burden on the caller. The rendering loop iterates `text.chars()` and draws each — there is no hook to inject the trailing `…`. Either (a) `TextLine::text` should be a `String` that already contains the `…`, or (b) `TextLine` grows a `trailing_glyph: Option<char>` field that the renderer appends after the main text.

### 3.44 Empty input is undefined behavior in the layout API
`layout_text("", opts, ...)` returns `vec![]`. Apps that measure-then-render an empty string get an empty Vec, then read `lines[0].y_offset` and panic on OOB. Either return a single empty `TextLine` for empty input, or document the empty-vec convention explicitly.

## 4. What the architect got right

1. **Trait surface is minimal and correct.** `FontProvider` with 6 methods, all `&self`, `Send + Sync` — clean dependency boundary. The decision to identify glyphs by `char` rather than font-internal glyph IDs makes fallback chains tractable without runtime glyph-ID remapping. The trait avoids the common mistake of leaking the backend's API (no `&fontdue::Font` accessor) — meaning HarfBuzz can drop in later.

2. **`BitmapFontProvider` as guaranteed terminator** is the right pattern. The constitutional guarantee that ASCII *always* renders simplifies higher-layer logic — the layout engine can call `glyph_advance` without `Option<f32>` plumbing because at worst it gets a bitmap advance. The terminator's `expect("bitmap font always present")` in `FallbackChain::resolve` (line 633) is justified.

3. **Quantization of size at the cache key** is the correct general approach (the granularity is wrong, but the principle is right). Without quantization, animating a font size at 60 Hz would re-rasterize 95 glyphs per frame — pathological. The trade-off framing (cache key space vs. animation smoothness) is the right vocabulary.

4. **CPU + GPU atlas split with shared addressing.** The decision to share atlas content across apps because glyph bitmaps are not per-app secrets is correct and saves 60× memory under 60 concurrent apps. The implicit content-addressing (same glyph bytes regardless of requesting PID) is a clean economy.

5. **Per-PID font ownership and `unload_owned_by` on process exit.** Right pattern for capability-secure resource accounting; combines naturally with the existing supervisor process-exit hook. Prevents one common failure mode (crashed app leaks 8 MiB).

6. **WIT interface as opt-in for sophisticated apps.** Existing stdout protocol consumers continue working; only apps that need precise typography pay the cost of the WIT import. Aligns with VyomaOS's "capabilities not declared are not wired up" principle. The versioned `vyoma:fonts@1.0.0` is correctly forward-compatible for future shaping APIs.

7. **`fontdue` as the v2 backend** is a defensible choice — pure Rust, `forbid(unsafe_code)`, ~85 KB, no FFI. The alternatives (HarfBuzz + FreeType via FFI) would have introduced C dependencies and broken the static-musl supervisor build. The justification holds even after the safety critique above; the issue is not fontdue's choice but the surrounding envelope.

8. **Explicit deferral list.** Variable fonts, bidi, color emoji, dynamic font downloading — all called out as out-of-scope for v1 with justification. Good discipline. The deferrals are honest about future cost.

9. **Performance budget table** with cold/warm separation is the right framing for evaluating whether the LRU is doing its job. The acknowledgment that a 80×24 cold terminal redraw exceeds budget (76 ms) and needs explicit pre-warm is correct engineering.

10. **`MAX_FONT_BYTES`-style hard caps at the registry boundary** — even though the chosen values are wrong (Issue 2.4) and the accounting under-counts (Issue 3.26), the *pattern* of enforcing caps at the boundary before any expensive operation is correct.

11. **`Arc<dyn FontProvider>` indirection** allows future backends without changing call sites. Combined with `Send + Sync`, this is the right shape for a long-lived, shared, thread-safe resource.

12. **R11 / R12 integration sketches** correctly identify the dispatch points and respect the boundary between font subsystem and display protocol. The proposed `handle_draw_text_scaled` change is in the right place and routes through the fallback chain rather than hardcoding the bitmap.

13. **Open Questions list** is genuinely engaged with the hard problems. The architect surfaces RTL, eviction, hot-reload, quantization granularity, color glyphs — the right surface area. The critique above is largely an answer-set for these questions, not a rejection of their relevance.

14. **No `unsafe` in the spec.** Even with adversarial input, the trait and provider implementations introduce no unsafe blocks. The safety story (apart from the rasterization gap) is built on language-level guarantees, not audit discipline.

15. **The IPC `bounded(32)` channel signals intent.** Even if the bound is too tight for the workload, the architect correctly identifies that unbounded queueing on a worker thread is a DoS vector. The pattern is right; the parameters need tuning.

## 5. Questions for synthesis

1. **Crash safety:** Are we accepting in-process fontdue with per-call `catch_unwind` + watchdog + worker respawn (option b in 2.1), or moving to subprocess sandbox (option a)? The subprocess option costs ~3 ms RTT per uncached glyph; the in-process option requires complete `catch_unwind` audit.

2. **Atlas eviction:** v1 commits to which of: (a) hard cap + full re-pack, (b) Skyline-BL + reference counts, (c) document atlas as "best effort, may CPU-fall-back"? If (c), how do we explain the silent perf cliff to app authors?

3. **Per-platform caps:** Do we accept platform-profile-driven font limits, including disabling TrueType entirely on `mcu-minimal`? If yes, the profile schema in `supervisor/src/profile/profiles/` needs a `[fonts]` section, and the registry needs construction-time wiring.

4. **RTL policy:** Refuse, reverse, or partial-shape? Each choice has a different surface area in the layout engine.

5. **Tri-state `GlyphStatus`:** Are we accepting the trait change to distinguish Native/Substitution/Absent? This is a one-time trait break before stabilization; deferring it means a v2 trait migration.

6. **Worker pool:** Single worker or N=fontid_mod_N pool? Affects channel topology and FontWorker struct.

7. **WIT FontId encoding:** Reserve 0 as "no font" (shifting bitmap to 1), or keep current and document the gotcha?

8. **Subsetting reproducibility:** Commit the subsetting script and inputs to the repo, or document the subset spec and trust the bundled `.ttf`?

9. **GlyphBitmap return type:** Trait returns `Arc<GlyphBitmap>` (avoiding clones) or `GlyphBitmap` by value (current)? The Arc change is contagious to every caller.

10. **`FontWeight` width:** 3-variant enum (current), 9-variant CSS-style (100..=900), or `u16` mapped weight value? Decision before WIT 1.0.0 stabilizes.

11. **Validation timing:** Per-call `catch_unwind` on every fontdue API, or a `RasterSession` abstraction that wraps a batch of glyphs in one `catch_unwind`? Latter is cheaper but coarser-grained failure.

12. **Worker shutdown:** Add `Shutdown` request variant and supervisor-exit join? Or accept the worker as never-shutdown (current implicit choice)?

13. **Metrics surface:** Per-font hit/miss counters, atlas occupancy gauge, LRU eviction counter — emit via `observability/` heartbeat or via a separate `vyoma:font/metrics` WIT interface for app-side introspection?

14. **`Light` weight fallback:** Currently aliases to Regular if missing. Should it instead synthesize via reduced stroke-width (fontdue does not support this directly — would require pre-rendered and post-processed bitmap), or hard-error so apps know?

15. **R12 integration:** Who calls `take_dirty()` and uploads the atlas? The R12 compositor's flush pass, or the font subsystem's own thread? Determines whether the atlas upload is on the critical path of the frame.

16. **Atlas-per-app vs. atlas-shared:** If we accept per-PID quota for atlas occupancy (to fix the multi-app DoS in 2.2), do we keep the single shared atlas with per-PID accounting, or split into per-PID atlases? Per-PID atlases simplify eviction but multiply memory at 60 concurrent apps.

17. **Cold-cache pre-warm protocol:** The §10 mitigation pre-warms all ASCII on focus. Should pre-warm be (a) implicit (supervisor pre-warms on app focus), (b) explicit (app calls a `prewarm-glyphs` WIT method), or (c) inferred from measure-then-render patterns? Implicit risks pre-warming unused glyphs; explicit shifts complexity to apps.

18. **GlyphBitmap padding for sub-pixel positioning:** v1 spec returns axis-aligned glyph bitmaps with integer bearings. Sub-pixel positioning (for crisper rendering at small sizes) requires either pre-rendering at multiple sub-pixel offsets (×3 atlas cost) or signed-distance-field rendering (different bitmap format). Do we commit to a future SDF path now (different `GlyphBitmap` variant) or close that door?

19. **Manifest-declared fallback chain vs. system-declared:** Should the fallback chain be configurable at the platform level (e.g., desktop-full ships with NotoSans CJK in the default chain), or only at the app level via manifest? Platform-level is friendlier but breaks deterministic per-app behavior across platforms.

20. **`measure_text` allocations:** The current TrueType `measure_text` allocates nothing (iterates chars, accumulates floats). Good. But the bitmap `measure_text` also allocates nothing. The architect should add a test asserting zero allocations on the measurement hot path — easy to introduce a `.collect()` regression that drops a frame.

21. **Font subsystem testability:** The spec does not describe a testing strategy. Unit-testing `FontProvider` is straightforward; unit-testing the fallback chain requires a mock provider. Should the trait include a `#[cfg(test)] MockFontProvider`? Integration testing the WIT path requires spinning up a Wasmtime instance per test — slow.

22. **Performance regression CI:** The architect commits to specific p99 numbers in §10. Are these enforced in CI via a microbenchmark suite, or aspirational? If enforced, the supervisor's `make unit-test` invocation grows by ~2 minutes. If aspirational, they will silently regress.

23. **Heap-vs-stack glyph data:** `GlyphBitmap::data: Vec<u8>` heap-allocates for every miss. A small-glyph optimization (`SmallVec<[u8; 256]>`) keeps 16×16 glyphs on the stack/inline, saving allocator pressure on the burst miss path. Worth the dependency?

24. **License compliance for bundled fonts:** Inter is SIL OFL 1.1; JetBrains Mono is SIL OFL 1.1. Both require shipping the LICENSE alongside the font file. The initramfs build script (`rootfs.sh`) must include `OFL.txt` for each. Currently the spec does not mention licensing — a legal gap.

25. **Font subsystem boot dependencies:** §5 system fonts load at `BootPhase::Display` before first app spawn. Does the supervisor have a boot-phase dependency graph today? If not, where is `BootPhase::Display` declared? The integration with the boot sequencer (described in P09 of the phase plan) needs explicit wiring.

The architect's spec is a faithful sketch of the *shape* of a typography subsystem and identifies the right components. It is not yet a production design — the safety envelope is decorative, the i18n stubs produce visible defects, and the caps lock CJK users out. The trait surface and atlas geometry are reusable; everything around the security model and the fallback semantics needs a redesign before code lands.

A reasonable v1 redesign would: replace the load-time-only `catch_unwind` with per-call `catch_unwind` plus a worker watchdog and respawn protocol; replace the single shelf-pack atlas with a bounded-glyph atlas that re-packs on overflow (or a Skyline-BL packer with reference counts and per-PID quotas); replace the global `MAX_FONT_BYTES`/`MAX_TOTAL_FONT_RAM` constants with per-platform-profile values and track actual RAM rather than raw bytes; replace the RTL stub with either explicit refusal or character-reversed rendering; replace the bitmap `has_glyph` with a tri-state that distinguishes native vs. substitution and add U+FFFD to the bitmap data; replace the single global worker with a small pool keyed on FontId. Each is a clear, scoped change. None requires a full architectural rethink — but the combination is large enough that "fix during code review" is not viable. The architect should respin the spec as v1.1 with these changes before any task in this round becomes implementable.
