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

**Why thin?** Holding the actual buffer in supervisor would double IPC traffic for every keystroke (app → supervisor to mutate, supervisor → app to render). WASM apps are already responsible for glyph layout and rendering via `VYOMA_DRAW:`. The supervisor shadows only the metadata it needs to perform its role as system broker. Any mutation the supervisor initiates (paste, drag-drop insert, IME commit) is delivered as a `VYOMA_TEXT:` command and acknowledged by the app; the app remains the single source of truth for buffer state.

**Scope of this spec**:
- `TextBuffer` struct and editing operations (§2 — lives in app-side helpers, mirrored here for protocol contract)
- `TextContext` shadow that supervisor maintains per focused field (§3)
- `VYOMA_TEXT:` IPC protocol verb table (§4)
- Selection model: cursor + anchor (§5)
- IME preedit integration (§6)
- Undo/redo routing (§7)
- System text services: spell check, smart substitution, data detector (§8)
- Clipboard and drag-drop integration (§9)
- File layout under `supervisor/src/text_input/` (§10)
- Blocking issue resolution (§11)

---

## 2. TextBuffer — App-Side Struct

Apps that want the VyomaOS-standard editing behaviour import the `vyoma-text` helper crate (optional; apps may implement their own buffer). The struct contract is fixed because the supervisor's IPC protocol is expressed in terms of it.

```rust
// apps/shared/vyoma-text/src/buffer.rs

/// Single-line or multi-line text buffer with cursor, anchor, and IME preedit.
pub struct TextBuffer {
    /// Canonical buffer content as Unicode scalar values.
    /// Using Vec<char> avoids all mid-grapheme-cluster byte-index bugs.
    pub content:      Vec<char>,

    /// Insertion/caret point (char index, 0..=content.len()).
    pub cursor:       usize,

    /// Selection anchor (char index). None = collapsed (cursor == anchor).
    /// Selection spans min(cursor,anchor)..max(cursor,anchor).
    pub anchor:       Option<usize>,

    /// Optional hard cap on content.len(). Enforced on every insert.
    pub max_len:      Option<usize>,

    /// Active IME preedit string (not yet committed to content).
    pub preedit_text: Option<String>,

    /// Cursor position within the preedit string (char index into preedit_text).
    pub preedit_cursor: usize,

    /// Single-line vs multi-line mode.
    pub mode:         TextInputMode,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextInputMode {
    /// Enter key submits; newlines rejected.
    SingleLine,
    /// Enter key inserts newline; Shift+Enter also valid.
    MultiLine,
}
```

### 2.1 Selection Model

The selection is defined by two independent positions: `cursor` and `anchor`. When the user presses an arrow key without Shift, `anchor` is cleared (selection collapses to cursor). When the user holds Shift and moves, `anchor` stays fixed at its original position while `cursor` moves. The selected range is always `min(cursor, anchor)..max(cursor, anchor)`.

This asymmetry matters for directional-delete and for visual caret drawing: the visible caret always tracks `cursor`, not the anchor, regardless of which end is "earlier" in the buffer.

```rust
impl TextBuffer {
    pub fn selection_range(&self) -> Option<std::ops::Range<usize>> {
        let anchor = self.anchor?;
        if anchor == self.cursor { return None; }
        let lo = self.cursor.min(anchor);
        let hi = self.cursor.max(anchor);
        Some(lo..hi)
    }

    pub fn has_selection(&self) -> bool {
        self.anchor.map_or(false, |a| a != self.cursor)
    }

    pub fn selected_chars(&self) -> &[char] {
        match self.selection_range() {
            Some(r) => &self.content[r],
            None    => &[],
        }
    }

    pub fn selected_text(&self) -> String {
        self.selected_chars().iter().collect()
    }
}
```

### 2.2 Editing Operations

```rust
impl TextBuffer {
    /// Insert a single Unicode scalar at cursor, replacing selection if any.
    /// Respects max_len. Returns false if insert was rejected (max_len exceeded).
    pub fn insert_char(&mut self, ch: char) -> bool {
        self.delete_selection();
        if let Some(max) = self.max_len {
            if self.content.len() >= max { return false; }
        }
        self.content.insert(self.cursor, ch);
        self.cursor += 1;
        self.anchor = None;
        true
    }

    /// Insert a string slice. Rejects mid-grapheme splits by operating on chars.
    pub fn insert_str(&mut self, s: &str) -> usize {
        let mut inserted = 0;
        for ch in s.chars() {
            if self.insert_char(ch) { inserted += 1; } else { break; }
        }
        inserted
    }

    /// Delete the character immediately before the cursor (Backspace).
    /// If selection exists, deletes the selection instead.
    pub fn delete_backward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        if self.cursor > 0 {
            self.cursor -= 1;
            self.content.remove(self.cursor);
        }
    }

    /// Delete the character immediately after the cursor (Forward-Delete / Fn+Backspace).
    pub fn delete_forward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        if self.cursor < self.content.len() {
            self.content.remove(self.cursor);
        }
    }

    fn delete_selection(&mut self) {
        if let Some(r) = self.selection_range() {
            self.content.drain(r.clone());
            self.cursor = r.start;
            self.anchor = None;
        }
    }

    /// Move cursor left/right by `n` characters.
    /// `extend` = true keeps or sets anchor (Shift held).
    pub fn move_cursor(&mut self, delta: isize, extend: bool) {
        if !extend { self.anchor = None; }
        else if self.anchor.is_none() { self.anchor = Some(self.cursor); }

        let new_pos = (self.cursor as isize + delta)
            .clamp(0, self.content.len() as isize) as usize;
        self.cursor = new_pos;
    }

    /// Move cursor to previous word boundary (Opt+Left on macOS).
    pub fn move_word_left(&mut self, extend: bool) {
        if !extend { self.anchor = None; }
        else if self.anchor.is_none() { self.anchor = Some(self.cursor); }
        self.cursor = self.prev_word_boundary(self.cursor);
    }

    /// Move cursor to next word boundary (Opt+Right on macOS).
    pub fn move_word_right(&mut self, extend: bool) {
        if !extend { self.anchor = None; }
        else if self.anchor.is_none() { self.anchor = Some(self.cursor); }
        self.cursor = self.next_word_boundary(self.cursor);
    }

    /// Select the entire buffer (Cmd+A).
    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.content.len();
    }

    /// Double-click / Ctrl+W style: select the word under the cursor.
    pub fn select_word_at_cursor(&mut self) {
        let lo = self.prev_word_boundary(self.cursor);
        let hi = self.next_word_boundary(self.cursor);
        if lo < hi {
            self.anchor = Some(lo);
            self.cursor = hi;
        }
    }

    /// Move to beginning of current line (Home / Cmd+Left).
    pub fn move_line_start(&mut self, extend: bool) {
        if !extend { self.anchor = None; }
        else if self.anchor.is_none() { self.anchor = Some(self.cursor); }
        self.cursor = self.line_start(self.cursor);
    }

    /// Move to end of current line (End / Cmd+Right).
    pub fn move_line_end(&mut self, extend: bool) {
        if !extend { self.anchor = None; }
        else if self.anchor.is_none() { self.anchor = Some(self.cursor); }
        self.cursor = self.line_end(self.cursor);
    }
}
```

### 2.3 Word and Line Boundary Detection

Word characters are defined as alphanumeric plus underscore, matching POSIX `\w`. This is intentionally simple and locale-independent; a future grapheme-cluster-aware boundary detector may supersede it.

```rust
impl TextBuffer {
    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    fn prev_word_boundary(&self, from: usize) -> usize {
        let mut pos = from;
        // Skip trailing non-word chars
        while pos > 0 && !Self::is_word_char(self.content[pos - 1]) { pos -= 1; }
        // Skip word chars
        while pos > 0 && Self::is_word_char(self.content[pos - 1])  { pos -= 1; }
        pos
    }

    fn next_word_boundary(&self, from: usize) -> usize {
        let mut pos = from;
        let len = self.content.len();
        // Skip leading non-word chars
        while pos < len && !Self::is_word_char(self.content[pos]) { pos += 1; }
        // Skip word chars
        while pos < len && Self::is_word_char(self.content[pos])  { pos += 1; }
        pos
    }

    fn line_start(&self, from: usize) -> usize {
        let mut pos = from;
        while pos > 0 && self.content[pos - 1] != '\n' { pos -= 1; }
        pos
    }

    fn line_end(&self, from: usize) -> usize {
        let mut pos = from;
        while pos < self.content.len() && self.content[pos] != '\n' { pos += 1; }
        pos
    }
}
```

---

## 3. TextContext — Arc-Swap Snapshot (B1 Fix)

```rust
// supervisor/src/text_input/context.rs

use arc_swap::ArcSwap;
use arrayvec::ArrayString;
use std::sync::{Arc, RwLock};
use std::collections::HashMap;

#[derive(Clone)]
pub struct TextContextSnapshot {
    pub app:              String,
    pub field_id:         ArrayString<32>,
    pub kind:             FieldKind,
    /// Packed trait bits:
    ///   b0=autocap b1=autocorrect b2=spellcheck b3=smart_quotes
    ///   b4=smart_dashes b5=data_detector b6=secure b7=continuous_spellcheck
    ///   b8=hit_test
    pub traits:           TextTraits,
    pub caret_rect:       Option<Rect>,
    pub selection_range:  Option<std::ops::Range<usize>>,  // always-accurate char-index range
    pub selection_len:    usize,                             // full utf8 byte count of selection
    pub selection_preview: Option<Arc<str>>,                 // ≤4096 bytes, may be truncated
    pub is_dirty:         bool,          // set on mutating command; cleared on ack
    pub seq:              u64,
    pub last_ns:          u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKind {
    SingleLine, MultiLine, Password, Search, Numeric, Email, Url,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct TextTraits(pub u32);

impl TextTraits {
    pub fn autocap(self)           -> bool { self.0 & (1 << 0) != 0 }
    pub fn autocorrect(self)       -> bool { self.0 & (1 << 1) != 0 }
    pub fn spellcheck(self)        -> bool { self.0 & (1 << 2) != 0 }
    pub fn smart_quotes(self)      -> bool { self.0 & (1 << 3) != 0 }
    pub fn smart_dashes(self)      -> bool { self.0 & (1 << 4) != 0 }
    pub fn data_detector(self)     -> bool { self.0 & (1 << 5) != 0 }
    pub fn secure(self)            -> bool { self.0 & (1 << 6) != 0 }
    pub fn continuous_spellcheck(self) -> bool { self.0 & (1 << 7) != 0 }
    pub fn hit_test(self)          -> bool { self.0 & (1 << 8) != 0 }
    /// B4 fix: zero all service bits for password fields regardless of what app sent.
    pub fn masked_for_secure(self) -> Self { TextTraits(0) }
}

// B1 fix: ArcSwap per-field → lock-free reads by AX/IME threads
pub struct TextRegistry {
    // Rare mutations (register/unregister field): RwLock ok
    fields:  RwLock<HashMap<(String, ArrayString<32>), ArcSwap<TextContextSnapshot>>>,
    focused: ArcSwap<Option<(String, ArrayString<32>)>>,
}

impl TextRegistry {
    pub fn load_field(&self, key: &(String, ArrayString<32>))
        -> Option<Arc<TextContextSnapshot>>
    {
        let guard = self.fields.read().unwrap();
        guard.get(key).map(|s| s.load_full())
    }

    pub fn update_snapshot<F>(&self, key: &(String, ArrayString<32>), f: F)
    where F: FnOnce(&mut TextContextSnapshot)
    {
        let guard = self.fields.read().unwrap();
        if let Some(slot) = guard.get(key) {
            let mut snap = (*slot.load_full()).clone();
            f(&mut snap);
            slot.store(Arc::new(snap));
        }
    }
}
```

Stdout reader (per-app thread) builds a fresh `TextContextSnapshot` clone, mutates it, and `ArcSwap::store`s it — wait-free for all readers. AX/IME threads call `load()` to get a coherent immutable snapshot without taking any lock.

---

## 4. VYOMA_TEXT: IPC Protocol

All verbs are line-oriented, terminated with `\n`. App → supervisor direction is written to app stdout; supervisor → app direction is written to app stdin via the SPSC channel.

### 4.1 Field Lifecycle (App → Supervisor)

| Verb | Arguments | Description |
|------|-----------|-------------|
| `VYOMA_TEXT:focus` | `<field_id>:<kind>:<traits_u32>` | App reports a text field has gained focus |
| `VYOMA_TEXT:blur` | `<field_id>` | App reports a text field has lost focus |
| `VYOMA_TEXT:register` | `<field_id>:<kind>:<traits_u32>` | Declare field existence before focus (optional) |
| `VYOMA_TEXT:unregister` | `<field_id>` | Field destroyed |

`kind` values: `single_line`, `multi_line`, `password`, `search`, `numeric`, `email`, `url`.

**Secure field masking (B4 fix)**: when `kind == password` or bit6(`secure`) is set, supervisor forcibly applies `TextTraits::masked_for_secure()` on ingress, zeroing all trait bits regardless of what the app sent. Context window lines from secure fields are silently dropped.

### 4.2 Cursor and Selection Reporting (App → Supervisor)

| Verb | Arguments | Description |
|------|-----------|-------------|
| `VYOMA_TEXT:cursor_rect` | `<field_id>:<x>,<y>,<w>,<h>:<baseline_y>:<line_height>` | Screen-absolute caret bounding rect |
| `VYOMA_TEXT:selection_rects` | `<field_id>:<count>:<x1>,<y1>,<w1>,<h1>;...` | Up to 64 visible selection rects |
| `VYOMA_TEXT:selection_range` | `<field_id>:<start>:<end>:<utf8_byte_count>` | Always-accurate char-index range + total byte size |
| `VYOMA_TEXT:selection_preview` | `<field_id>:<base64_text>` | Optional: truncated copy ≤4096 bytes for AX/spell-check |
| `VYOMA_TEXT:context_window` | `<field_id>:<base64_before>:<base64_after>` | ≤256 bytes before/after caret for smart substitution |

`selection_range` and `selection_preview` are **two separate lines** (B3 fix). This separates always-accurate metadata from the potentially-truncated text preview. Services that need accuracy use `selection_range`; services that need text content use `selection_preview` and tag their results `partial:true` when `utf8_byte_count > len(preview)`.

### 4.3 Mutating Commands (Supervisor → App)

All mutating commands carry a `<seq>` token. The seq counter increments globally (one `AtomicU64` in supervisor). The app must send `VYOMA_TEXT:ack:<field_id>:<seq>` after applying the mutation and emitting updated `selection_range` and `cursor_rect` lines.

| Verb | Arguments | Description |
|------|-----------|-------------|
| `VYOMA_TEXT:set_text` | `<field_id>:<seq>:<base64>` | Replace entire buffer content |
| `VYOMA_TEXT:get_text` | `<field_id>:<seq>` | Request full buffer; app replies with `VYOMA_TEXT:text_result` |
| `VYOMA_TEXT:set_cursor` | `<field_id>:<seq>:<char_offset>` | Move caret to absolute char offset |
| `VYOMA_TEXT:get_selection` | `<field_id>:<seq>` | Request current selection text; app replies with `VYOMA_TEXT:selection_result` |
| `VYOMA_TEXT:insert` | `<field_id>:<seq>:<position>:<base64>` | Insert text at position (`caret`, `start`, `end`, or decimal char offset) |
| `VYOMA_TEXT:delete` | `<field_id>:<seq>:<start>:<end>` | Delete char-index range |
| `VYOMA_TEXT:select_range` | `<field_id>:<seq>:<start>:<end>` | Set cursor=end, anchor=start |
| `VYOMA_TEXT:commit_preedit` | `<field_id>:<seq>` | Finalize active IME preedit into buffer |
| `VYOMA_TEXT:cancel_preedit` | `<field_id>:<seq>` | Discard active IME preedit without committing |
| `VYOMA_TEXT:undo` | `<field_id>:<seq>` | Trigger undo in app |
| `VYOMA_TEXT:redo` | `<field_id>:<seq>` | Trigger redo in app |
| `VYOMA_TEXT:hit_test_request` | `<field_id>:<x>,<y>` | Ask app to convert screen coord to char offset |

### 4.4 App Replies (App → Supervisor)

| Verb | Arguments | Description |
|------|-----------|-------------|
| `VYOMA_TEXT:ack` | `<field_id>:<seq>` | Mutation applied; snapshot updated |
| `VYOMA_TEXT:text_result` | `<field_id>:<seq>:<base64>` | Reply to `get_text` |
| `VYOMA_TEXT:selection_result` | `<field_id>:<seq>:<base64>` | Reply to `get_selection` |
| `VYOMA_TEXT:hit_test_result` | `<field_id>:<x>,<y>:<char_offset>` | Reply to `hit_test_request` |

---

## 5. Text Field Focus Management

On `VYOMA_TEXT:focus` received from app:

1. If another app's field is focused: send `VYOMA_TEXT:blur:<old_field_id>` to previous app.
2. Send `VYOMA_IME_CONTEXT:focus_will_change` to active IME service (R34 §5) so it can dismiss its candidate window cleanly.
3. Update `TextRegistry.focused` via `ArcSwap::store` (wait-free, no lock).
4. Apply secure-field masking to `traits` if `kind == password` (B4 fix).
5. Emit `VYOMA_AX:focus_changed:<app>:<field_id>:<kind>` to AX tree (R30).

On `VYOMA_TEXT:blur`:
1. Clear `TextRegistry.focused` if it matches this field.
2. Cancel any pending ack watchdog for the field.
3. Notify IME that composition should be committed or cancelled.

---

## 6. IME Preedit Integration

IME input follows a three-phase model: **preedit** (in-progress composition shown inline), **candidate selection** (IME panel over the app), and **commit** (final string inserted into buffer). The supervisor coordinates candidate window placement via cursor_rect (R34) and delivers preedit/commit events as `VYOMA_TEXT:` commands.

```rust
// supervisor/src/text_input/ime.rs

pub struct PreeditState {
    pub field_id:       ArrayString<32>,
    pub preedit_text:   String,
    pub preedit_cursor: usize,   // char offset within preedit_text
    pub seq:            u64,
}

// Received from IME service (a WASM service with text_service=true capability):
//   VYOMA_IME:preedit_update:<field_id>:<base64_text>:<cursor_char_offset>
// → supervisor stores PreeditState, forwards to app:
//   VYOMA_TEXT:preedit_update:<field_id>:<seq>:<base64_text>:<cursor_offset>
// App renders preedit inline (typically underlined) at cursor position.
// App does NOT insert it into TextBuffer.content yet.

// Received from IME service:
//   VYOMA_IME:preedit_commit:<field_id>:<base64_final_text>
// → supervisor sends:
//   VYOMA_TEXT:commit_preedit:<field_id>:<seq>
//   VYOMA_TEXT:insert:<field_id>:<seq+1>:caret:<base64_final_text>
// App replaces in-progress preedit with the committed text in its buffer.

// Cursor rect forwarding (R34):
// Whenever supervisor receives VYOMA_TEXT:cursor_rect for the focused field
// and preedit is active, it also emits:
//   VYOMA_IME:cursor_rect:<x>,<y>,<w>,<h>,<baseline_y>,<line_height>
// to the active IME service so its candidate window floats correctly.
```

The app's `TextBuffer.preedit_text` field holds the current in-progress string from the IME. `preedit_cursor` is the visual caret position within that string (not the buffer cursor). When `commit_preedit` arrives, the app calls `TextBuffer::insert_str` with the committed text and clears `preedit_text`.

---

## 7. Undo / Redo

Supervisor does NOT own the undo ring. Rationale: undo semantics are application-specific; a canvas app has a different undo granularity than a text field. Holding the ring in supervisor would double IPC traffic for every keystroke.

Supervisor does route `Cmd+Z` / `Cmd+Shift+Z` when a text field is focused:
- Delivers `VYOMA_TEXT:undo:<field_id>:<seq>` / `VYOMA_TEXT:redo:<field_id>:<seq>` instead of raw key event.
- App that wants raw `Cmd+Z` (e.g. canvas undo) sets manifest `text_undo_passthrough = true`.

Apps that use the `vyoma-text` helper crate get a standard undo/redo stack at no extra cost:

```rust
// apps/shared/vyoma-text/src/undo.rs

const MAX_UNDO_DEPTH: usize = 50;

#[derive(Clone)]
pub struct TextSnapshot {
    pub content:  Vec<char>,
    pub cursor:   usize,
    pub anchor:   Option<usize>,
}

pub struct UndoStack {
    snapshots:   Vec<TextSnapshot>,
    current_idx: usize,   // points at the currently-applied state
}

impl UndoStack {
    pub fn new(initial: TextSnapshot) -> Self {
        Self { snapshots: vec![initial], current_idx: 0 }
    }

    /// Call after every committed user edit. Truncates redo tail.
    pub fn push(&mut self, snap: TextSnapshot) {
        // Discard any redo states above current
        self.snapshots.truncate(self.current_idx + 1);
        self.snapshots.push(snap);
        if self.snapshots.len() > MAX_UNDO_DEPTH + 1 {
            self.snapshots.remove(0);
        } else {
            self.current_idx += 1;
        }
    }

    /// Returns the state to restore, or None if already at oldest.
    pub fn undo(&mut self) -> Option<&TextSnapshot> {
        if self.current_idx == 0 { return None; }
        self.current_idx -= 1;
        Some(&self.snapshots[self.current_idx])
    }

    /// Returns the state to restore, or None if already at newest.
    pub fn redo(&mut self) -> Option<&TextSnapshot> {
        if self.current_idx + 1 >= self.snapshots.len() { return None; }
        self.current_idx += 1;
        Some(&self.snapshots[self.current_idx])
    }

    pub fn can_undo(&self) -> bool { self.current_idx > 0 }
    pub fn can_redo(&self) -> bool { self.current_idx + 1 < self.snapshots.len() }
}
```

Undo granularity recommendations (app-side policy, not enforced by supervisor):
- Push snapshot after every word completion (space/punctuation) or after a deliberate pause (>500 ms).
- Push snapshot before paste, drag-drop insert, and IME commit.
- Do NOT push a snapshot for every single character (creates unusable micro-undo history).

---

## 8. Seq Token and Dirty-Flag Ack Contract (B2 Fix)

```rust
// supervisor/src/text_input/ack.rs

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static GLOBAL_SEQ: AtomicU64 = AtomicU64::new(1);

pub fn next_seq() -> u64 {
    GLOBAL_SEQ.fetch_add(1, Ordering::Relaxed)
}

pub struct PendingAck {
    pub seq:        u64,
    pub field_key:  (String, arrayvec::ArrayString<32>),
    pub sent_at:    Instant,
}

const ACK_TIMEOUT: Duration = Duration::from_secs(1);

/// Called on every iteration of the supervisor main loop (or a dedicated ack-watchdog thread).
pub fn check_ack_timeouts(
    pending: &Mutex<Vec<PendingAck>>,
    registry: &super::context::TextRegistry,
) {
    let now = Instant::now();
    let mut guard = pending.lock().unwrap();
    guard.retain(|ack| {
        if now.duration_since(ack.sent_at) >= ACK_TIMEOUT {
            log::warn!("[text_input] ack timeout seq={} field={:?}",
                ack.seq, ack.field_key.1);
            // Mark field unresponsive in snapshot
            registry.update_snapshot(&ack.field_key, |s| {
                s.is_dirty = false; // stop blocking AX reads; field is assumed broken
            });
            false // remove from pending list
        } else {
            true
        }
    });
}
```

Supervisor sets `snapshot.is_dirty = true` between sending a mutating command and receiving the ack. During the dirty window, AX/IME reads return the pre-mutation snapshot tagged with `is_stale: true`. If ack is not received within 1 s, the field is marked unresponsive, `is_dirty` is cleared, and an `VYOMA_AX:field_unresponsive` event is emitted.

---

## 9. System Text Services

### 9.1 Spell Check

Trigger conditions: `traits.spellcheck()` is true and `kind != password`.  
Debounce: 250 ms after last `seq` bump on the field.  
Input: `selection_preview` + `context_window` (before+after caret).  
Service: a separate `text_spellcheck` Wasmtime instance with `text_service = true` capability; no display, network, or filesystem access.

Result line:
```
VYOMA_TEXT:spell_suggestion:<field_id>:<start>:<end>:<word>|<sug1>|<sug2>|<sug3>
```

When `selection_len > len(selection_preview)`, service tags `partial:true` as a sixth field.

### 9.2 Smart Substitution

Smart substitution intercepts characters in the `VYOMA_INPUT:key:` stream **before** delivery to the app when the focused field has the relevant trait bits set.

| Input | Condition | Substitution |
|-------|-----------|--------------|
| `'` (U+0027) | `smart_quotes` + context indicates opening | `'` (U+2018) |
| `'` (U+0027) | `smart_quotes` + context indicates closing | `'` (U+2019) |
| `"` (U+0022) | `smart_quotes` + context indicates opening | `"` (U+201C) |
| `"` (U+0022) | `smart_quotes` + context indicates closing | `"` (U+201D) |
| `--` (two hyphens) | `smart_dashes` | `—` (U+2014 em dash) |

Context window (≤256 bytes before caret) from `context_window` line is consulted. Substitution is completely skipped for secure fields (B4 fix — all trait bits already zeroed on ingress, so `smart_quotes` and `smart_dashes` are never true for password fields).

### 9.3 Data Detector

Trigger: 750 ms stability after last `seq` bump. Regex match over `selection_preview`.

Tags: `url`, `phone`, `email`, `date`, `address`.

Result line:
```
VYOMA_TEXT:data_detected:<field_id>:<tag>:<start>:<end>:<value>:<partial>
```

`partial` = `true` or `false`. When true, the matched region extends beyond `selection_preview` and the value may be incomplete.

### 9.4 Service Security (B4 Fix)

All text services receive `field_kind` as a parameter. Services must return `Error::SecureField` if `kind == password`. Supervisor validates this: if a service returns a result for a password field, supervisor discards the result and panics the service instance. The service is restarted clean on the next non-secure field request.

---

## 10. Clipboard and Drag Integration

### 10.1 Clipboard Copy (R40 Integration)

On `Cmd+C` with a text field focused:

1. Supervisor sends `VYOMA_CLIPBOARD:fetch_selection:<field_id>:<seq>` to the focused app.
2. App replies with full untruncated selection bytes (no 4096-byte cap).
3. Supervisor writes to clipboard ring (R40).

This never uses `selection_preview` for clipboard (B3 fix): the preview is only for AX and spell-check hints.

### 10.2 Clipboard Paste (R40 Integration)

On `Cmd+V` with a text field focused:

1. Supervisor reads text from clipboard ring (R40).
2. Encodes as base64.
3. Sends `VYOMA_TEXT:insert:<field_id>:<seq>:caret:<base64>` to focused app.
4. Waits for ack.

### 10.3 Text Drop (R38 Integration, B5 Fix)

```rust
// supervisor/src/text_input/drop.rs

fn on_text_drop(drop_app: &str, token: &[u8; 16], x: i32, y: i32, text: &str) {
    let focused = TEXT_REGISTRY.focused.load();
    let key = match focused.as_ref().as_ref() {
        Some(k) => k.clone(),
        None => {
            // No text field focused — deliver raw VYOMA_DRAG:drop
            return;
        }
    };

    // B5 fix: verify the focused field belongs to the drop-target app.
    // Without this check, a malicious app could read another app's clipboard
    // content by winning a drag+focus race: focus a field in app-A, then
    // drag text onto app-B's window — supervisor would insert into app-A's field.
    if key.0 != drop_app {
        log::warn!(
            "[text_input] drop-text cross-app mismatch: focused={} drop_app={}",
            key.0, drop_app
        );
        send_to_app_spsc(
            drop_app,
            &format!("VYOMA_DRAG:drop_rejected:{}\n", hex_token(token)),
        );
        return;
    }

    let snap = TEXT_REGISTRY.load_field(&key);
    let seq = next_seq();

    if snap.as_ref().map_or(false, |s| s.traits.hit_test()) {
        // App supports coordinate-to-offset hit testing; ask it first.
        send_to_app_spsc(
            drop_app,
            &format!("VYOMA_TEXT:hit_test_request:{}:{},{}\n", key.1, x, y),
        );
        // App replies VYOMA_TEXT:hit_test_result:<field_id>:<x,y>:<offset>
        // Supervisor then sends VYOMA_TEXT:insert at that offset.
        PENDING_HIT_TEST.store(Arc::new(Some(PendingHitTest {
            token: *token,
            text: text.to_string(),
            seq,
        })));
    } else {
        // No hit-test support: insert at current caret.
        let b64 = base64_encode(text.as_bytes());
        send_to_app_spsc(
            drop_app,
            &format!("VYOMA_TEXT:insert:{}:{}:caret:{}\n", key.1, seq, b64),
        );
    }
}
```

---

## 11. Blocking Issue Resolution

### B1: TextContext concurrent read/write without synchronisation barrier

**Problem**: The per-app stdout reader thread writes to `TextContextSnapshot` fields (e.g. updating `caret_rect`, `selection_range`) at the same time as AX and IME threads read those fields. Using a plain `Mutex<TextContextSnapshot>` creates lock contention and risks priority inversion: a slow app stdout parse could delay an IME candidate window update.

**Fix**: Per-field `ArcSwap<TextContextSnapshot>`. The stdout reader clones the current snapshot, mutates the clone, and calls `ArcSwap::store` — a wait-free atomic pointer swap. AX and IME threads call `ArcSwap::load_full()` to get a coherent immutable snapshot. No blocking between readers and writers at any point.

**Status**: RESOLVED — see `supervisor/src/text_input/context.rs` §3.

---

### B2: Selection cache desync after supervisor-initiated insert/paste

**Problem**: After supervisor sends `VYOMA_TEXT:insert:...:caret:<base64>` (e.g. for a clipboard paste), the `TextContextSnapshot` held in supervisor still shows the old `selection_range` and `caret_rect` until the app processes the mutation and emits updated `selection_range` and `cursor_rect` lines. During this window, AX reads the wrong selection and IME places its candidate window at the wrong position.

**Fix**: Every mutating command carries a `<seq>` token. Supervisor sets `snapshot.is_dirty = true` immediately after sending. AX and IME reads that arrive while `is_dirty` return the snapshot with `is_stale: true`, indicating they should defer or tolerate potential inaccuracy. The app sends `VYOMA_TEXT:ack:<field_id>:<seq>` after applying the mutation and emitting updated metadata. On ack receipt, `is_dirty` is cleared. A 1-second watchdog escalates to `VYOMA_AX:field_unresponsive` if ack never arrives.

**Status**: RESOLVED — see `supervisor/src/text_input/ack.rs` §8.

---

### B3: 4096-byte selection preview cap creates clipboard/AX inconsistency

**Problem**: If the selected text is 50 KB and the cached preview is capped at 4096 bytes, clipboard paste would copy only the preview, not the full selection. Similarly, a spell-check service operating on truncated text would generate incorrect byte-offset suggestions.

**Fix**: The protocol separates `selection_range` (always-accurate char-index range + `utf8_byte_count`) from `selection_preview` (optional truncated copy). Clipboard copy bypasses the preview entirely: supervisor sends `VYOMA_CLIPBOARD:fetch_selection:<field_id>:<seq>` and waits for the full selection from the app. Services that use the preview tag their results with `partial:true` when `utf8_byte_count > len(preview)` so callers know to treat suggestions as approximate.

**Status**: RESOLVED — see §4.2 and §10.1.

---

### B4: Secure field (password) leaks via smart substitution and context window

**Problem**: Smart substitution consumes the `context_window` (up to 256 bytes of text before/after caret) to decide whether to insert an opening or closing smart quote. If the focused field is a password field, this lets the supervisor reconstruct partial plaintext from the context window lines the app emits. Data detector similarly receives `selection_preview` which would be the actual password characters.

**Fix**: On `VYOMA_TEXT:focus` ingress, if `kind == password` or bit6(`secure`) is set, supervisor calls `TextTraits::masked_for_secure()` which returns `TextTraits(0)` — all trait bits zeroed. Since `smart_quotes`, `smart_dashes`, `spellcheck`, and `data_detector` bits are all zero, none of these services are invoked. `context_window` lines from secure fields are silently dropped on ingress. WASM text service instances that receive a secure-field request must return `Error::SecureField`; supervisor panics the service on any violation.

**Status**: RESOLVED — see §3 (`TextTraits::masked_for_secure`) and §8 service security.

---

### B5: Cross-app selection exfiltration via drag-and-focus race

**Problem**: If the user starts a drag from app-A, then during the drag-in-progress interval focuses a text field in app-B (e.g. by hovering over it), a naive `on_text_drop` implementation would insert the dragged text into app-B's field. But the `TextRegistry.focused` still points at app-A's field (focus update races the drop delivery). Result: dropped text is inserted into a field in a different app than the one the user dropped onto — a cross-app data injection.

**Fix**: `on_text_drop` receives `drop_app` (the app whose window received the mouse-up event) as an explicit argument. It loads `TextRegistry.focused` and checks `focused.app == drop_app`. If they differ, it rejects the drop with `VYOMA_DRAG:drop_rejected:<token>` and logs a warning. AX and clipboard reads are similarly hard-bound to the `app_pid` that owns the focused field, not to whatever app happens to be focused when the read arrives.

**Status**: RESOLVED — see `supervisor/src/text_input/drop.rs` §10.3.

---

## 12. File Layout

```
supervisor/src/text_input/
├── mod.rs          (~90 lines)
│     TextRegistry static (OnceLock<Arc<Mutex<TextRegistry>>>),
│     public API: get_registry(), focused_field(), register_field(),
│     unregister_field(), update_focused()
│
├── context.rs      (~200 lines)
│     TextContextSnapshot, ArcSwap pattern (B1 fix),
│     FieldKind, TextTraits (with masked_for_secure),
│     Rect, TextRegistry struct and methods
│
├── protocol.rs     (~280 lines)
│     VYOMA_TEXT: stdout line parser (per-app thread entry point),
│     secure field masking on focus ingress (B4 fix),
│     selection_range + selection_preview two-line parser (B3 fix),
│     cursor_rect → IME forwarding (R34)
│
├── ack.rs          (~130 lines)
│     next_seq() AtomicU64, PendingAck struct,
│     check_ack_timeouts() watchdog (B2 fix),
│     dirty-flag set/clear helpers
│
├── drop.rs         (~160 lines)
│     on_text_drop() with cross-app verify (B5 fix),
│     PendingHitTest state, hit_test_result handler,
│     base64_encode helper
│
└── services.rs     (~200 lines)
      spell_check_debounce() (250 ms, spawns text_spellcheck WASM),
      smart_substitution_intercept() (pre-delivery key rewrite),
      data_detector_debounce() (750 ms),
      service security checks (B4 fix — reject secure fields,
      panic-restart on violation)
```

All six files stay within the 500-line limit. `mod.rs` re-exports the public surface; internal types stay `pub(super)` to prevent cross-subsystem coupling.

---

## 13. Global Static

```rust
// supervisor/src/text_input/mod.rs

use std::sync::{Arc, OnceLock};
use arc_swap::ArcSwap;

static TEXT_REGISTRY: OnceLock<Arc<super::context::TextRegistry>> = OnceLock::new();

pub fn get_registry() -> &'static Arc<super::context::TextRegistry> {
    TEXT_REGISTRY.get_or_init(|| {
        Arc::new(super::context::TextRegistry {
            fields:  std::sync::RwLock::new(std::collections::HashMap::new()),
            focused: ArcSwap::new(Arc::new(None)),
        })
    })
}
```

`OnceLock<Arc<...>>` (not `OnceLock<Arc<Mutex<...>>>`) because `TextRegistry` uses `RwLock` internally for its field map (rare writes) and `ArcSwap` for per-field snapshots (frequent reads). The outer `Arc` is needed only for tests that want to swap in a test registry.

---

## 14. Blocking Issue Resolution Summary

| Issue | Resolution | Location |
|-------|-----------|----------|
| B1: TextContext RW from stdout thread + read from AX/IME without barrier | Per-field `ArcSwap<TextContextSnapshot>`; stdout reader builds fresh clone + stores atomically; readers `load_full()` — wait-free | `context.rs` |
| B2: Selection cache desync after supervisor-side insert/paste | Every mutating command carries `<seq>`; app acks with `VYOMA_TEXT:ack:<seq>`; `is_dirty` flag blocks accurate AX/IME reads until ack; 1 s watchdog escalates to `field_unresponsive` | `ack.rs` |
| B3: 4096-byte preview cap creates clipboard/AX inconsistency | `selection_range` (always-accurate) separate from `selection_preview` (truncated); clipboard uses fresh `VYOMA_CLIPBOARD:fetch_selection` request; services tag `partial:true` | `protocol.rs`, §10.1 |
| B4: Secure field leaks via smart substitution and context_window | `TextTraits::masked_for_secure()` zeros all trait bits on `focus` ingress for `password`/`secure` fields; `context_window` lines dropped; services reject secure fields or are panicked | `context.rs`, `services.rs` |
| B5: Cross-app selection exfiltration via drag+focus race | `on_text_drop` verifies `focused.app == drop_app`; cross-app mismatch → `VYOMA_DRAG:drop_rejected`; AX/clipboard reads hard-bound to `app_pid` of focused field owner | `drop.rs` |
