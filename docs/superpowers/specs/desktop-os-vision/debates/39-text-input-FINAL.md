# FINAL Spec: Text Input & Selection Model (Round 39)

**Subsystem**: Text Input & Selection Model  
**macOS Analogue**: `NSTextView` / `TSM` / `TextKit` / `NSTextInputClient`  
**Depends on**: R30 (AX), R34 (IME cursor_rect), R38 (drag text/plain), R40 (Clipboard)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

**Thin model**: supervisor does NOT own the text buffer, undo ring, or glyph metrics. Apps own all of that. Supervisor owns a **TextContext shadow** — a small per-field cache of observable metadata required for:
- R34 (IME): caret position for candidate window placement
- R30 (AX/VoiceOver): selection text for screen reader
- R38 (Drag): insert text at drop point
- R40 (Clipboard): coordinate cut/copy/paste operations
- §5 system text services: spell check, smart substitution, data detector

---

## 2. TextContext — Arc-Swap Snapshot (B1 Fix)

```rust
// supervisor/src/text/context.rs

#[derive(Clone)]
pub struct TextContextSnapshot {
    pub app:             String,
    pub field_id:        ArrayString<32>,
    pub kind:            FieldKind,
    pub traits:          TextTraits,     // b0=autocap b1=autocorrect b2=spellcheck
                                         // b3=smart_quotes b4=smart_dashes b5=data_detector
                                         // b6=secure b7=continuous_spellcheck b8=hit_test
    pub caret_rect:      Option<Rect>,
    pub selection_range: Option<Range<usize>>,  // B3: always-accurate metadata
    pub selection_len:   usize,                  // B3: full utf8 byte count
    pub selection_preview: Option<Arc<str>>,     // B3: ≤4096 bytes, may be truncated
    pub is_dirty:        bool,           // B2: set on insert/delete/select_range; cleared on ack
    pub seq:             u64,
    pub last_ns:         u64,
}

// B1 fix: ArcSwap per-field → lock-free reads by AX/IME threads
pub struct TextRegistry {
    // Rare mutations: RwLock ok
    fields:  RwLock<HashMap<(String, ArrayString<32>), ArcSwap<TextContextSnapshot>>>,
    focused: ArcSwap<Option<(String, ArrayString<32>)>>,
}
```

Stdout reader (per-app thread) builds a fresh `TextContextSnapshot` clone, mutates it, and
`ArcSwap::store`s it — wait-free for all readers. AX/IME threads call `load()` to get a
coherent immutable snapshot.

---

## 3. Text Field Focus

```
VYOMA_TEXT:focus:<field_id>:<kind>:<traits_u32>
VYOMA_TEXT:blur:<field_id>
```

`kind` ∈ `single_line|multi_line|password|search|numeric|email|url`.

**Secure field masking (B4 fix)**: when `kind == password` OR bit6(`secure`) is set,
supervisor forcibly zeros traits bits 0–7 (all text services) on ingress, regardless of
what the app sent. Context window lines from secure fields are silently dropped.

On focus change:
1. Send `VYOMA_TEXT:blur:<field_id>` to previous app (it can hide caret blink).
2. Send `VYOMA_IME_CONTEXT:focus_will_change` to active IME (R34 §5).
3. Update `TextRegistry.focused`.
4. Emit `VYOMA_AX:focus_changed` to AX tree (R30).

---

## 4. Cursor Rect Protocol

```
VYOMA_TEXT:cursor_rect:<field_id>:<x>,<y>,<w>,<h>:<baseline_y>:<line_height>
VYOMA_TEXT:selection_rects:<field_id>:<count>:<x1>,<y1>,<w1>,<h1>;...
```

All coordinates screen-absolute. `line_height` and `baseline_y` used by IME panel
alignment and AX magnifier. Up to 64 selection rects (visible portion only).

Supervisor coalesces at 60 Hz max per field. Forwards derived `VYOMA_IME:cursor_rect`
to active IME when composition is live (R34 compatibility).

---

## 5. Selection Reporting (B2 + B3 Fix)

**Two-line protocol** (B3 fix — separates metadata from preview):

```
VYOMA_TEXT:selection_range:<field_id>:<start>:<end>:<utf8_byte_count>
VYOMA_TEXT:selection_preview:<field_id>:<base64_text>   # ≤4096 bytes; optional
```

`selection_range` is always-accurate. `selection_preview` is optional truncated copy for
AX/VoiceOver. Spell check and data detector tag results with `partial:true` when
`utf8_byte_count > len(preview)`.

**Secure fields**: `selection_preview` lines from secure fields dropped on ingress. AX
receives `<secure>` placeholder.

**Round-trip ack contract (B2 fix)**: every supervisor→app mutating command carries a
`<seq>` token:

```
VYOMA_TEXT:insert:<field_id>:<seq>:<position>:<base64>
VYOMA_TEXT:delete:<field_id>:<seq>:<start>:<end>
VYOMA_TEXT:select_range:<field_id>:<seq>:<start>:<end>
VYOMA_TEXT:undo:<field_id>:<seq>
VYOMA_TEXT:redo:<field_id>:<seq>
```

App must reply (after applying mutation + emitting updated selection_range + cursor_rect):
```
VYOMA_TEXT:ack:<field_id>:<seq>
```

Supervisor sets `snapshot.is_dirty = true` between sending and receiving ack. During dirty
window, AX/IME reads return the pre-mutation snapshot with `is_stale: true`. If ack not
received within 1s, field marked unresponsive and reported to AX.

---

## 6. Clipboard & Drag Integration

**Clipboard copy** (R40): on Cmd+C, supervisor sends fresh
`VYOMA_CLIPBOARD:fetch_selection:<field_id>` to app — app replies with full untruncated
bytes. Never uses cached preview for clipboard (B3 fix).

**Text drop** (R38, B5 fix):

```rust
// supervisor/src/text/drop.rs

fn on_text_drop(drop_app: &str, token: &[u8;16], x: i32, y: i32, text: &str) {
    let focused = TEXT_REGISTRY.focused.load();
    let key = match focused.as_ref() {
        Some(k) => k,
        None => { /* no text field focused — deliver raw VYOMA_DRAG:drop */ return; }
    };

    // B5 fix: verify field belongs to the drop-target app (not a different app)
    if key.0 != drop_app {
        log_warn!(Subsystem::Text, Some(drop_app),
            "drop-text cross-app field mismatch: focused_app={}, drop_app={}", key.0, drop_app);
        send_to_app_spsc(drop_app,
            &format!("VYOMA_DRAG:drop_rejected:{}\n", hex_token(token)));
        return;
    }

    let snap = TEXT_REGISTRY.load_field(key);
    if snap.traits.hit_test() {
        // Ask app to resolve screen coord to byte offset first
        let seq = next_seq();
        send_to_app_spsc(drop_app,
            &format!("VYOMA_TEXT:hit_test_request:{}:{},{}\n", key.1, x, y));
        // App replies VYOMA_TEXT:hit_test_result:<field_id>:<x,y>:<offset>
        // Then supervisor sends insert at that offset
        PENDING_HIT_TEST.store(Arc::new(Some(PendingHitTest { token: *token, text: text.to_string(), seq })));
    } else {
        let seq = next_seq();
        let b64 = base64_encode(text.as_bytes());
        send_to_app_spsc(drop_app,
            &format!("VYOMA_TEXT:insert:{}:{}:caret:{}\n", key.1, seq, b64));
    }
}
```

---

## 7. System Text Services

**7.1 Spell check**: `traits.spellcheck` set + `kind != password`. Supervisor invokes
spell-check WASM service (separate Wasmtime instance, `text_service=true` capability,
no display/network/fs) with `selection_preview` + context window. 250 ms debounce.
Results: `VYOMA_TEXT:spell_suggestion:<field_id>:<start>:<end>:<word>|<sug1>|<sug2>`.

**7.2 Smart substitution**: Intercepts `'`, `"`, `-` in `VYOMA_INPUT:key:` stream before
delivery. Consults `context_window` (≤256 bytes before/after caret). Rewrites to smart
quotes/dashes inline. Skipped for secure fields (B4 fix — traits masked to 0).

**7.3 Data detector**: 750 ms stability after last `seq` bump. Regex over `selection_preview`.
Tags `url|phone|email|date|address`. `partial:true` appended when selection exceeds preview.
Result: `VYOMA_TEXT:data_detected:<field_id>:<kind>:<start>:<end>:<value>:<partial>`.

**Service security (B4 fix)**: services receive `field_kind` parameter and must return
`Error::SecureField` if `kind == password`. Supervisor panics the service on violation.

---

## 8. Undo Routing

Supervisor does NOT own the undo ring. Rationale: undo semantics are application-specific;
holding them in supervisor doubles keystroke IPC traffic.

Supervisor does route `Cmd+Z` / `Cmd+Shift+Z` when a text field is focused:
- Delivers `VYOMA_TEXT:undo:<field_id>:<seq>` / `redo` instead of raw key event.
- App that wants raw `Cmd+Z` (canvas undo) sets manifest `text_undo_passthrough = true`.

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: TextContext RW from stdout thread + read from AX/IME without barrier | Per-field `ArcSwap<TextContextSnapshot>`; stdout reader builds fresh clone + stores atomically; readers `load()` snapshot — wait-free |
| B2: Selection cache desync after supervisor-side insert/paste | Every mutating command carries `<seq>` token; app acks with `VYOMA_TEXT:ack:<seq>`; `is_dirty` flag blocks AX/IME reads until ack; 1s watchdog |
| B3: 4096-byte cap creates clipboard/AX inconsistency | `selection_range` (always-accurate) separate from `selection_preview` (truncated); clipboard uses fresh `fetch_selection` request; services tag `partial:true` |
| B4: Secure field leaks via smart substitution / context_window | Supervisor zeros all service trait bits on ingress for `kind==password` or `secure` bit; context_window dropped; services reject secure fields |
| B5: Cross-app selection exfiltration via drag+focus race | `on_text_drop` verifies `focused.app == drop_app`; cross-app mismatch → `VYOMA_DRAG:drop_rejected`; AX/clipboard reads hard-bound to `app_pid` |

---

## 10. File Layout

```
supervisor/src/text/
├── mod.rs          (~80 lines: TextRegistry, TEXT_REGISTRY static, public API)
├── context.rs      (~200 lines: TextContextSnapshot, ArcSwap pattern (B1), FieldKind, TextTraits)
├── protocol.rs     (~280 lines: VYOMA_TEXT: stdout parser; secure masking (B4))
├── services.rs     (~200 lines: spell check, smart substitution, data detector dispatch)
├── drop.rs         (~150 lines: on_text_drop, cross-app verify (B5), hit_test flow)
└── ack.rs          (~120 lines: seq token, dirty flag, 1s watchdog (B2))
```
