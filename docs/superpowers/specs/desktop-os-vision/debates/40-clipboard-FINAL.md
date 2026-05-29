# FINAL Spec: Clipboard & Pasteboard (Round 40)

**Subsystem**: Clipboard & Pasteboard  
**macOS Analogue**: `NSPasteboard` / `UIPasteboard` / clipboard history  
**Depends on**: R35 (Cmd+C/V hotkeys), R38 (drag payload storage), R39 (text field insert, fetch_selection)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Four named pasteboards:

| Name | Purpose | History cap |
|------|---------|------------|
| `general` | System clipboard (Cmd+C/V) | 16 entries |
| `find` | Cmd+F search term | 1 entry |
| `drag` | Active drag payload (R38 unified store) | 1 entry |
| `ruler` | Style copy/paste | 4 entries |

One `ClipboardEntry` per pasteboard write; one entry carries multiple typed
representations. `change_count: u64` per pasteboard increments on every write.

```rust
// supervisor/src/pasteboard/mod.rs

#[derive(Clone)]
pub struct ClipboardItem {
    pub mime:   String,
    pub inline: Option<Vec<u8>>,   // ≤64 KiB
    pub path:   Option<String>,    // /run/vyoma/pb/<token>/<index>
    pub size:   u64,
}

#[derive(Clone)]
pub struct ClipboardEntry {
    pub token:        [u8; 16],   // 128-bit random
    pub items:        Vec<ClipboardItem>,
    pub source_app:   String,
    pub change_count: u64,
    pub secure:       bool,
    pub expire_at:    Option<Instant>,
}

#[derive(Default, Clone)]
pub struct PasteboardState {
    pub current:      Option<Arc<ClipboardEntry>>,
    pub history:      Vec<Arc<ClipboardEntry>>,
    pub change_count: u64,
}

// One ArcSwap per pasteboard — no cross-pasteboard locking
pub static PASTEBOARDS: [Lazy<ArcSwap<PasteboardState>>; 4] = /* ... */;

// B4: explicit refcount table — GC only when refcount=0 AND not current
pub static TOKEN_REFS: Lazy<Mutex<HashMap<[u8;16], u32>>> = Lazy::new(|| Mutex::new(HashMap::new()));
```

---

## 2. Write Protocol — Atomic Staging (B1 + B2 Fix)

### 2.1 Phase A: Begin

App writes: `VYOMA_PASTEBOARD:begin:<pb>:<n_items>`

Supervisor:
1. Verifies `clipboard = true` (or `clipboard_provider = true` within 250ms Cmd+C grant, B5).
2. Generates `token: [u8; 16]`.
3. Creates **staging dir** `/run/vyoma/pb/.staging/<hex(token)>/` mode 0700 (B1 fix — unobservable until commit).
4. Grants the writing app a WASI descriptor scoped to the staging dir via Wasmtime resource table (B2 fix — no fd-over-line-protocol needed):
   ```rust
   // supervisor/src/pasteboard/write.rs
   fn grant_staging_descriptor(app: &str, token: &[u8; 16]) {
       let dir = format!("/run/vyoma/pb/.staging/{}/", hex_token(token));
       wasmtime_resource_table(app).add_dir(dir, DirPerms::WRITE, FilePerms::WRITE);
       // App accesses via pre-agreed WASI path "/vyoma/pb-out/<hex_token>/"
   }
   ```
5. ACKs app: `VYOMA_PASTEBOARD:writing:<hex_token>`.

### 2.2 Phase B: Item Upload

Small inline (≤64 KiB, on stdout):
```
VYOMA_PASTEBOARD:item:<token>:<mime_b64>:inline:<base64_payload>
```

Large items: app writes to WASI path `/vyoma/pb-out/<hex_token>/<index>` directly.
Then announces: `VYOMA_PASTEBOARD:item:<token>:<mime_b64>:path:<size_bytes>`

### 2.3 Phase C: Commit (B1 Fix)

```
VYOMA_PASTEBOARD:commit:<token>[:secure[:ttl_secs]]
```

Supervisor:
1. `fsync` each item file in staging.
2. Validate each file's `st_size == advertised_size` (B1: size check after fsync).
3. `rename("/run/vyoma/pb/.staging/<token>/", "/run/vyoma/pb/<token>/")` — atomic (B1 fix: entry unobservable until rename completes).
4. Revoke the WASI staging descriptor.
5. CAS-loop `commit_write` (see §3).
6. Emit `VYOMA_PASTEBOARD:changed:<pb>:<change_count>:<n_items>:<primary_mime_b64>` to observers (source_app REDACTED — B3 fix).

5-second watchdog on open staging dirs — GC stale uncommitted writes.

```rust
fn commit_write(pb: Pasteboard, entry: Arc<ClipboardEntry>) -> u64 {
    loop {
        let old = PASTEBOARDS[pb as usize].load_full();
        let mut next = (*old).clone();
        next.change_count = next.change_count.wrapping_add(1);
        if !entry.secure {
            if let Some(prev) = next.current.replace(entry.clone()) {
                next.history.insert(0, prev);
                next.history.truncate(pb.history_cap());
            }
        } else {
            // B3: secure entries skip history; previous current moves to history normally
            let prev = next.current.replace(entry.clone());
            if let Some(p) = prev { next.history.insert(0, p); next.history.truncate(pb.history_cap()); }
        }
        if PASTEBOARDS[pb as usize]
            .compare_and_swap(&old, Arc::new(next)).as_ptr() == Arc::as_ptr(&old)
        {
            return entry.change_count;
        }
    }
}
```

---

## 3. Read Protocol

### 3.1 Supervisor-initiated Paste (Cmd+V)

1. R35 Cmd+V fires; supervisor loads `general` pasteboard.
2. Picks `text/plain` (or `text/rtf` if field declared rtf-aware via R39).
3. Sends `VYOMA_TEXT:insert:<field_id>:<seq>:<base64>` to focused app (R39 path).
4. App never sees the clipboard surface — no `clipboard` capability required for Cmd+V.

### 3.2 App-initiated Read (B3 Fix)

```
VYOMA_PASTEBOARD:read:<pb>:<request_id>[:<mime_preference>]
```

**Authorization gate** (B3 fix — removes `clipboard_read_unattended`):
- Within 250ms Cmd+V grant window: allowed.
- Otherwise: supervisor displays chrome banner "Allow <app> to read clipboard? [Allow once] [Always] [Deny]". Persisted in `/data/clipboard-grants.toml`.
- `clipboard_provider = true` apps: always allowed (they wrote the data they're reading).

**Delivery** (B4 fix — increment refcount before reply):
```rust
fn on_read_request(app: &str, pb: Pasteboard, rid: &str, mime_pref: Option<&str>) {
    let snap = PASTEBOARDS[pb as usize].load_full();
    let entry = match &snap.current { Some(e) => e.clone(), None => { reply_empty(app, rid); return; } };
    // B4 fix: increment refcount before reply
    TOKEN_REFS.lock().unwrap().entry(entry.token).and_modify(|c| *c += 1).or_insert(1);
    let item = select_item(&entry.items, mime_pref);
    if item.inline.is_some() {
        reply_inline(app, rid, &entry, item);
        // inline: decrement immediately (no bind needed)
        decrement_token_ref(&entry.token);
    } else {
        // path: bind /run/vyoma/pb/<token>/<i> read-only into app namespace
        bind_path_for_app(app, rid, &entry.token, item);
        // refcount decremented on release: or 30s timer
    }
}
```

Supervisor replies:
```
VYOMA_PASTEBOARD:reply:<rid>:<change_count>:<mime_b64>:inline:<b64>
VYOMA_PASTEBOARD:reply:<rid>:<change_count>:<mime_b64>:path:<wasi_path>:<size>
```

App releases path-backed reads: `VYOMA_PASTEBOARD:release:<rid>`.

### 3.3 Token-Dir GC (B4 Fix)

```rust
fn decrement_token_ref(token: &[u8; 16]) {
    let mut refs = TOKEN_REFS.lock().unwrap();
    if let Some(c) = refs.get_mut(token) {
        *c -= 1;
        if *c == 0 {
            refs.remove(token);
            // Check no pasteboard still has this as current
            if !any_pasteboard_has_token(token) {
                schedule_token_dir_delete(token);
            }
        }
    }
}
// GC only when refcount=0 AND not current for any pasteboard
```

---

## 4. Clipboard History & Observers

Observers (require `clipboard_observer = true`):

```
VYOMA_PASTEBOARD:observe:<pb>
```

Changed notification (B3 fix — source_app REDACTED from default delivery):

```
VYOMA_PASTEBOARD:changed:<pb>:<change_count>:<n_items>:<primary_mime_b64>
```

`source_app` omitted unless observer has `clipboard_observer_unredacted = true` (user opt-in via Settings panel). Secure entries: primary_mime always `***secure***`.

History read: `VYOMA_PASTEBOARD:history_read:<pb>:<index>:<rid>` — same read path as §3.2 but against `history[index]`.

---

## 5. Secure Clipboard

```
VYOMA_PASTEBOARD:commit:<token>:secure[:ttl_secs]
```

- Default TTL 60s, max 120s.
- Never enters history.
- `changed:` emits `primary_mime_b64 = "***secure***"`.
- Secure sweep: 1 Hz task + on every focus change. On expiry: zero-fill token dir bytes, unlink, fsync, `ArcSwap::store(None as current)`.
- Reading a secure entry **consumes** it (current → None after reply).

---

## 6. Cmd+C Provider Protocol (B5 Fix)

Apps with `clipboard_provider = true` own their Cmd+C:

1. R35 Cmd+C sends `VYOMA_HOTKEY:copy:<field_id>` to focused app's stdin.
2. App responds with a full §2 two-phase pasteboard write (multi-MIME: text/plain + text/rtf + text/html) within 250ms grant window.
3. App gets one-time `clipboard = true` grant for this write only; no persistent capability needed.

Apps **without** `clipboard_provider`:
1. R35 requests `VYOMA_CLIPBOARD:fetch_selection:<field_id>` (R39).
2. App replies with plain-text bytes.
3. Supervisor performs §2 write on behalf of app — `text/plain` only.
4. Chrome shows "Plain text copy" indicator so user knows formatting was dropped (B5 fix).

R38 integration: drag payload at `/run/vyoma/drag/<token>/` is unified with pasteboard token dirs. Promoting a drag to clipboard: `commit_write(General, drag_entry.clone())` — same entry, no copy.

---

## 7. Capability Gate

```toml
[capabilities]
clipboard           = true   # two-phase write + read within Cmd+V grant
clipboard_provider  = true   # owns Cmd+C; always-allowed read
clipboard_observer  = false  # receive changed: events + history metadata
secure_clipboard    = false  # :secure commits
clipboard_observer_unredacted = false  # see source_app in changed: (user opt-in)
```

Standard apps need none. `clipboard_provider = true` covers rich editors. Clipboard managers need `clipboard_observer`. Password managers need `secure_clipboard`.

---

## 8. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Write not atomic — readers see half-built entry | Staging dir `/run/vyoma/pb/.staging/<token>/`; `fsync` + `rename` before ArcSwap; entry unobservable until rename |
| B2: WASI descriptor grant hand-waved | Fixed preopen `/vyoma/pb-out` at startup; supervisor adds scoped subdir descriptor to Wasmtime resource table on `begin` ack; revoked on `commit`/`abort` |
| B3: `clipboard_read_unattended` enables exfiltration; `changed:` leaks source_app | Removed `clipboard_read_unattended`; chrome banner for first non-Cmd+V read; `changed:` omits `source_app` by default; opt-in `clipboard_observer_unredacted` via Settings |
| B4: Token dir GC doesn't coordinate with reader lifetimes | `TOKEN_REFS: Mutex<HashMap<[u8;16], u32>>`; increment before reply, decrement on release/timeout; GC only when refcount=0 AND not current |
| B5: Supervisor-side Cmd+C only copies text/plain | `clipboard_provider = true` gets `VYOMA_HOTKEY:copy` for own multi-MIME write; fallback produces `text/plain` with "Plain text copy" chrome indicator |

---

## 9. File Layout

```
supervisor/src/pasteboard/
├── mod.rs          (~120 lines: PASTEBOARDS, TOKEN_REFS, public API)
├── entry.rs        (~150 lines: ClipboardEntry, ClipboardItem, PasteboardState)
├── write.rs        (~250 lines: begin/item/commit handlers, staging rename (B1), WASI descriptor grant (B2))
├── read.rs         (~200 lines: on_read_request, refcount (B4), bind path, release)
├── history.rs      (~120 lines: history_read, observer subscription, changed: emission (B3))
├── secure.rs       (~120 lines: secure sweep task, consume-on-read, TTL)
└── cmd_copy.rs     (~150 lines: clipboard_provider Cmd+C path (B5), fetch_selection fallback)
```
