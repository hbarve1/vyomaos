# FINAL Spec: Clipboard & Pasteboard (Round 40)

**Subsystem**: Clipboard & Pasteboard  
**macOS Analogue**: `NSPasteboard` / `UIPasteboard` / clipboard history  
**Depends on**: R35 (Cmd+C/V hotkeys), R38 (drag payload storage), R39 (text field insert, fetch_selection)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Four named pasteboards mirror the macOS model exactly:

| Name | Purpose | History cap |
|------|---------|------------|
| `general` | System clipboard (Cmd+C/V) | 20 entries |
| `find` | Cmd+F search term | 1 entry |
| `drag` | Active drag payload (R38 unified store) | 1 entry |
| `ruler` | Style copy/paste | 4 entries |

One `ClipboardEntry` per pasteboard write; one entry carries multiple typed
representations simultaneously. `change_count: u64` per pasteboard increments on
every write. History is a ring buffer (`VecDeque`) bounded by each pasteboard's
cap. The `drag` pasteboard is always separate from `general`; promoting a drag
drop to clipboard is an explicit `commit_write(General, drag_entry.clone())`
call, copying the `Arc` without duplicating data.

---

## 2. Core Data Types

```rust
// supervisor/src/clipboard/entry.rs

use std::time::SystemTime;

/// Typed payload for one MIME representation.
#[derive(Clone, Debug)]
pub enum ClipboardData {
    /// UTF-8 text (text/plain, text/html, text/rtf, …)
    Text(String),
    /// Raw bytes for any binary MIME type
    Binary(Vec<u8>),
    /// Ordered list of absolute paths (text/uri-list)
    FileList(Vec<String>),
    /// RGBA pixel buffer
    Image {
        width:  u32,
        height: u32,
        rgba:   Vec<u8>,   // length == width * height * 4
    },
}

/// One typed representation within a clipboard entry.
#[derive(Clone, Debug)]
pub struct ClipboardItem {
    /// IANA MIME type, e.g. "text/plain;charset=utf-8"
    pub mime_type:  String,
    /// Inline payload for items ≤ 64 KiB; otherwise None and path is set.
    pub data:       Option<ClipboardData>,
    /// Filesystem path under /run/vyoma/pb/<token>/<index> for large items.
    pub path:       Option<String>,
    pub size:       u64,
}

/// A single clipboard write, potentially carrying N typed representations.
#[derive(Clone, Debug)]
pub struct ClipboardEntry {
    /// 128-bit random token used as token-dir name.
    pub token:        [u8; 16],
    /// All MIME representations provided by the source app in one write.
    /// Consumers choose the richest type they can handle.
    pub items:        Vec<ClipboardItem>,
    /// Originating app name (redacted from observer notifications by default).
    pub source_app:   String,
    /// Wall time of the write.
    pub timestamp:    SystemTime,
    /// Monotonically increasing counter from the owning pasteboard.
    pub change_count: u64,
    /// If true: never enters history, consumed on first read, zero-filled on expiry.
    pub secure:       bool,
    /// Absolute expiry for secure entries (None for normal entries).
    pub expire_at:    Option<std::time::Instant>,
}
```

### 2.1 Multi-Type Items

A single Cmd+C from a rich text editor produces one `ClipboardEntry` with
multiple `ClipboardItem` representations, in priority order:

```
items[0]: mime_type = "text/html",              data = Text("<b>Hello</b>")
items[1]: mime_type = "text/rtf",               data = Binary([...rtf bytes...])
items[2]: mime_type = "text/plain;charset=utf-8", data = Text("Hello")
items[3]: mime_type = "image/png",              path = Some("/run/vyoma/pb/<tok>/3")
```

The consumer calls `get_types` to enumerate available MIMEs and then requests
the richest type it understands. A plain-text consumer always finds `text/plain`
as a fallback.

---

## 3. Pasteboard State

```rust
// supervisor/src/clipboard/mod.rs

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Default)]
pub struct PasteboardState {
    pub current:      Option<Arc<ClipboardEntry>>,
    /// Ring buffer of previous entries (newest first).
    pub history:      VecDeque<Arc<ClipboardEntry>>,
    pub change_count: u64,
    /// App names registered via `watch` verb.
    pub observers:    Vec<String>,
}

/// Named pasteboard indices.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pasteboard {
    General = 0,
    Find    = 1,
    Drag    = 2,
    Ruler   = 3,
}

impl Pasteboard {
    pub fn history_cap(self) -> usize {
        match self {
            Pasteboard::General => 20,
            Pasteboard::Find    => 1,
            Pasteboard::Drag    => 1,
            Pasteboard::Ruler   => 4,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "general" => Some(Self::General),
            "find"    => Some(Self::Find),
            "drag"    => Some(Self::Drag),
            "ruler"   => Some(Self::Ruler),
            _         => None,
        }
    }
}

/// Global pasteboard table. Indexed by Pasteboard as usize.
static PASTEBOARDS: OnceLock<[Arc<Mutex<PasteboardState>>; 4]> = OnceLock::new();

pub fn pasteboards() -> &'static [Arc<Mutex<PasteboardState>>; 4] {
    PASTEBOARDS.get_or_init(|| {
        [
            Arc::new(Mutex::new(PasteboardState::default())),
            Arc::new(Mutex::new(PasteboardState::default())),
            Arc::new(Mutex::new(PasteboardState::default())),
            Arc::new(Mutex::new(PasteboardState::default())),
        ]
    })
}

/// Refcount table for token-dirs in /run/vyoma/pb/<token>/.
/// Incremented before replying to a read request; decremented on release.
/// GC fires only when count == 0 AND no pasteboard holds that token as current.
static TOKEN_REFS: OnceLock<Arc<Mutex<std::collections::HashMap<[u8; 16], u32>>>> =
    OnceLock::new();

pub fn token_refs() -> &'static Arc<Mutex<std::collections::HashMap<[u8; 16], u32>>> {
    TOKEN_REFS.get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
}
```

---

## 4. VYOMA_CLIPBOARD IPC Protocol

All clipboard commands travel over the existing supervisor IPC channel as
`VYOMA_CLIPBOARD:<verb>` prefixed lines on app stdout. Responses arrive on
app stdin.

### 4.1 Protocol Verb Table

| Verb | Direction | Purpose |
|------|-----------|---------|
| `begin:<pb>:<n_items>` | app→supervisor | Start a two-phase write on pasteboard `<pb>` |
| `item:<tok>:<mime_b64>:inline:<b64>` | app→supervisor | Upload small item (≤ 64 KiB) inline |
| `item:<tok>:<mime_b64>:path:<size>` | app→supervisor | Announce large item written to WASI staging dir |
| `commit:<tok>[:secure[:ttl_secs]]` | app→supervisor | Atomically publish the staged entry |
| `abort:<tok>` | app→supervisor | Cancel a staged but uncommitted write |
| `paste:<pb>:<rid>[:<mime_pref_b64>]` | app→supervisor | Request current entry, optionally with MIME preference |
| `paste_as_type:<pb>:<rid>:<mime_b64>` | app→supervisor | Request a specific MIME type from current entry |
| `get_types:<pb>:<rid>` | app→supervisor | Enumerate available MIME types without fetching data |
| `get_history:<pb>:<index>:<rid>` | app→supervisor | Read history entry at index (0 = most recent past) |
| `get_change_count:<pb>:<rid>` | app→supervisor | Poll pasteboard change_count without reading data |
| `release:<rid>` | app→supervisor | Release a path-backed read handle |
| `clear:<pb>` | app→supervisor | Clear pasteboard (requires clipboard capability) |
| `watch:<pb>` | app→supervisor | Subscribe to change notifications |
| `unwatch:<pb>` | app→supervisor | Unsubscribe from change notifications |
| `writing:<hex_tok>` | supervisor→app | ACK for `begin`, includes assigned token |
| `reply:<rid>:<cc>:<mime_b64>:inline:<b64>` | supervisor→app | Inline read response |
| `reply:<rid>:<cc>:<mime_b64>:path:<wasi_path>:<size>` | supervisor→app | Path-backed read response |
| `types:<rid>:<cc>:<mime_list_b64>` | supervisor→app | Response to `get_types` |
| `empty:<rid>` | supervisor→app | Pasteboard is empty |
| `denied:<rid>:<reason>` | supervisor→app | Read or write rejected |

### 4.2 Change Notifications

Apps subscribed via `watch` receive a broadcast on every pasteboard write:

```
VYOMA_CLIPBOARD_CHANGED:<pb>:<change_count>:<n_items>:<primary_mime_b64>
```

`source_app` is omitted by default (B3 fix). Observers holding
`clipboard_observer_unredacted = true` receive a supplemental line:

```
VYOMA_CLIPBOARD_CHANGED_SOURCE:<pb>:<change_count>:<source_app_b64>
```

Secure entries broadcast `primary_mime_b64 = "***secure***"` and never emit the
`_SOURCE` line regardless of capability.

---

## 5. Write Protocol — Atomic Staging (B1 + B2 Fix)

### 5.1 Phase A: Begin

App writes: `VYOMA_CLIPBOARD:begin:<pb>:<n_items>`

Supervisor:
1. Verifies app holds `clipboard = true` (or `clipboard_provider = true` within 250ms Cmd+C grant).
2. Generates `token: [u8; 16]` via `OsRng`.
3. Creates **staging dir** `/run/vyoma/pb/.staging/<hex(token)>/` mode 0700. The staging
   dir is invisible to readers — no path exists under `/run/vyoma/pb/<token>/` until commit.
4. Grants writing app a WASI directory descriptor scoped to the staging dir via
   Wasmtime resource table (B2 fix — no fd-over-line-protocol needed):

```rust
// supervisor/src/clipboard/write.rs

fn grant_staging_descriptor(app: &str, token: &[u8; 16]) {
    let dir = format!("/run/vyoma/pb/.staging/{}/", hex_token(token));
    // Add pre-opened dir to this app's WASI resource table under a fixed alias.
    wasmtime_resource_table(app).add_preopened_dir(
        dir,
        "/vyoma/pb-out/".to_string(),
        DirPerms::CREATE | DirPerms::WRITE,
        FilePerms::WRITE,
    );
}
```

5. ACKs app: `VYOMA_CLIPBOARD:writing:<hex_token>`.

A 5-second watchdog per open staging dir GCs stale uncommitted writes and
revokes the WASI descriptor if the app does not call `commit` or `abort`.

### 5.2 Phase B: Item Upload

Small inline (≤ 64 KiB):
```
VYOMA_CLIPBOARD:item:<token>:<mime_b64>:inline:<base64_payload>
```

Large items: app writes file directly to WASI path `/vyoma/pb-out/<index>`, then
announces the item:
```
VYOMA_CLIPBOARD:item:<token>:<mime_b64>:path:<size_bytes>
```

Supervisor records index → staging path mapping; does not read data yet.

### 5.3 Phase C: Commit (B1 Fix)

```
VYOMA_CLIPBOARD:commit:<token>[:secure[:ttl_secs]]
```

Supervisor commit sequence:

```rust
// supervisor/src/clipboard/write.rs

fn handle_commit(app: &str, token: [u8; 16], secure: bool, ttl_secs: Option<u64>) {
    let staging = format!("/run/vyoma/pb/.staging/{}/", hex_token(&token));
    let final_dir = format!("/run/vyoma/pb/{}/", hex_token(&token));

    // 1. fsync each item file in staging to ensure durability.
    for item_path in list_staged_items(&staging) {
        let file = File::open(&item_path).expect("staged item vanished");
        file.sync_all().expect("fsync failed");
        // B1: verify advertised size matches actual st_size
        let meta = file.metadata().expect("stat failed");
        assert_eq!(meta.len(), advertised_size(&token, &item_path),
                   "B1: size mismatch after fsync");
    }

    // 2. Atomic rename: staging → final (B1 fix)
    std::fs::rename(&staging, &final_dir).expect("rename failed");

    // 3. Revoke staging WASI descriptor for this app.
    wasmtime_resource_table(app).remove_preopened_dir("/vyoma/pb-out/");

    // 4. Build ClipboardEntry and commit to pasteboard state.
    let entry = Arc::new(build_entry(token, app, secure, ttl_secs));
    let pb = staged_pasteboard(&token);
    let new_cc = commit_write(pb, entry.clone());

    // 5. Notify observers (source_app redacted — B3 fix).
    broadcast_changed(pb, new_cc, &entry);
}

fn commit_write(pb: Pasteboard, entry: Arc<ClipboardEntry>) -> u64 {
    let state_lock = &pasteboards()[pb as usize];
    let mut state = state_lock.lock().unwrap();
    state.change_count = state.change_count.wrapping_add(1);
    let mut entry_mut = (*entry).clone();
    entry_mut.change_count = state.change_count;
    let entry = Arc::new(entry_mut);

    if let Some(prev) = state.current.replace(entry.clone()) {
        if !prev.secure {
            state.history.push_front(prev);
            state.history.truncate(pb.history_cap());
        }
    }
    state.change_count
}
```

---

## 6. Read Protocol

### 6.1 Supervisor-Initiated Paste (Cmd+V)

1. R35 Cmd+V fires; supervisor loads `general` pasteboard current entry.
2. Picks `text/plain` (or `text/rtf` if focused field declared rtf-aware via R39).
3. Sends `VYOMA_TEXT:insert:<field_id>:<seq>:<base64>` directly to focused app (R39 path).
4. The focused app never sees clipboard surface API — no `clipboard` capability required for Cmd+V.

### 6.2 App-Initiated Read (B3 Fix)

```
VYOMA_CLIPBOARD:paste:<pb>:<rid>[:<mime_pref_b64>]
```

**Authorization gate** (B3 fix — removes `clipboard_read_unattended`):
- Within 250ms Cmd+V grant window: allowed immediately.
- Otherwise: supervisor displays chrome banner "Allow `<app>` to read clipboard?
  [Allow once] [Always] [Deny]". Choice persisted in `/data/clipboard-grants.toml`.
- `clipboard_provider = true` apps: always allowed (they wrote the data they are reading).

**Delivery** (B4 fix — increment refcount before reply):

```rust
// supervisor/src/clipboard/read.rs

pub fn on_read_request(
    app:      &str,
    pb:       Pasteboard,
    rid:      &str,
    mime_pref: Option<&str>,
) {
    let state = pasteboards()[pb as usize].lock().unwrap();
    let entry = match &state.current {
        Some(e) => e.clone(),
        None => { reply_empty(app, rid); return; }
    };
    drop(state); // release lock before I/O

    // B4 fix: increment refcount BEFORE handing path to app
    token_refs().lock().unwrap()
        .entry(entry.token)
        .and_modify(|c| *c += 1)
        .or_insert(1);

    let item = select_item(&entry.items, mime_pref);

    match &item.data {
        Some(_) => {
            // Inline: deliver directly, decrement refcount immediately
            reply_inline(app, rid, &entry, item);
            decrement_token_ref(&entry.token);
        }
        None => {
            // Path-backed: bind read-only WASI descriptor, decrement on release
            let wasi_path = bind_path_for_app(app, rid, &entry.token, item);
            reply_path(app, rid, &entry, item, &wasi_path);
            // 30-second safety timer in case app never sends release
            schedule_release_timeout(app, rid, &entry.token, 30);
        }
    }
}
```

Supervisor reply lines:
```
VYOMA_CLIPBOARD:reply:<rid>:<change_count>:<mime_b64>:inline:<b64>
VYOMA_CLIPBOARD:reply:<rid>:<change_count>:<mime_b64>:path:<wasi_path>:<size>
```

App releases path-backed handle: `VYOMA_CLIPBOARD:release:<rid>`

### 6.3 Type Enumeration

```
VYOMA_CLIPBOARD:get_types:<pb>:<rid>
```

Reply (B3 fix — no data, only type list; no auth gate needed):
```
VYOMA_CLIPBOARD:types:<rid>:<change_count>:<mime_list_b64>
```

`mime_list_b64` is a base64-encoded newline-separated list of MIME types
available in the current entry. Apps use this to pick their preferred type
before issuing `paste_as_type`.

### 6.4 Token-Dir GC (B4 Fix)

```rust
// supervisor/src/clipboard/read.rs

pub fn decrement_token_ref(token: &[u8; 16]) {
    let mut refs = token_refs().lock().unwrap();
    if let Some(count) = refs.get_mut(token) {
        *count -= 1;
        if *count == 0 {
            refs.remove(token);
            drop(refs); // release lock before touching pasteboard state
            // GC only when no pasteboard still holds this as current
            if !any_pasteboard_has_token(token) {
                schedule_token_dir_delete(token);
            }
        }
    }
}

fn any_pasteboard_has_token(token: &[u8; 16]) -> bool {
    pasteboards().iter().any(|pb_lock| {
        pb_lock.lock().unwrap()
            .current
            .as_ref()
            .map_or(false, |e| e.token == *token)
    })
}

fn schedule_token_dir_delete(token: &[u8; 16]) {
    // Zero-fill all files, then unlink, then fsync parent dir.
    let dir = format!("/run/vyoma/pb/{}/", hex_token(token));
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(entry.path()) {
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                f.write_all(&vec![0u8; len as usize]).ok();
                f.sync_all().ok();
            }
            std::fs::remove_file(entry.path()).ok();
        }
    }
    std::fs::remove_dir(&dir).ok();
}
```

---

## 7. Clipboard History

History is per-pasteboard and uses `VecDeque` as a fixed-cap ring buffer. The
`general` pasteboard retains the last 20 entries (excluding secure entries).

### 7.1 Reading History

```
VYOMA_CLIPBOARD:get_history:<pb>:<index>:<rid>
```

Index 0 is the most recent past entry (not the current entry). Requires the app
to hold `clipboard_observer = true` or to be within an active Cmd+V grant.

Delivery follows the same inline/path-backed split as §6.2. The supervisor
increments the token refcount before replying.

### 7.2 Change Observation

```
VYOMA_CLIPBOARD:watch:<pb>
VYOMA_CLIPBOARD:unwatch:<pb>
```

Requires `clipboard_observer = true`. App is added to
`PasteboardState::observers` and will receive `VYOMA_CLIPBOARD_CHANGED` lines
on every write to the subscribed pasteboard. Supervisor iterates observers
under a cloned snapshot of the observer list — no lock held during dispatch.

---

## 8. Secure Clipboard

```
VYOMA_CLIPBOARD:commit:<token>:secure[:ttl_secs]
```

- Default TTL: 60 seconds. Maximum: 120 seconds.
- Entry never enters `PasteboardState::history`.
- `VYOMA_CLIPBOARD_CHANGED` emits `primary_mime_b64 = "***secure***"`.
- `_SOURCE` supplemental line is never emitted for secure entries.
- Secure sweep: runs at 1 Hz and on every focus-change event. On expiry:
  zero-fill all bytes in token-dir files, unlink files, `fsync` parent dir,
  then `state.current = None`.
- Reading a secure entry consumes it: after inline delivery, the supervisor
  immediately clears `state.current` and calls `schedule_token_dir_delete`.
- Requires `secure_clipboard = true` capability.

---

## 9. Cmd+C Provider Protocol (B5 Fix)

Apps with `clipboard_provider = true` own their Cmd+C and produce multi-MIME
entries. Apps without it receive supervisor-assisted text-only copy.

### 9.1 Provider Path (rich apps)

1. R35 Cmd+C fires; supervisor sends `VYOMA_HOTKEY:copy:<field_id>` to focused app stdin.
2. App responds with a full §5 two-phase write within 250ms grant window,
   providing as many MIME types as it supports (e.g. text/plain + text/rtf + text/html + image/png).
3. App receives a one-time `clipboard = true` grant for this write only. No
   persistent `clipboard` capability is required.
4. If 250ms elapses without `commit`, supervisor revokes staging descriptor and
   GCs the token dir.

### 9.2 Fallback Path (non-provider apps)

1. R35 requests `VYOMA_CLIPBOARD:fetch_selection:<field_id>` (R39 path).
2. App replies with plain-text bytes.
3. Supervisor performs the §5 write on behalf of app — `text/plain` only.
4. Chrome shows a "Plain text copy" indicator to inform the user that rich
   formatting was not captured (B5 fix).

### 9.3 Drag-to-Clipboard Promotion

R38 drag payload resides at `/run/vyoma/drag/<token>/`. On drop (when target app
confirms acceptance), supervisor promotes the drag entry to `general` pasteboard:

```rust
// supervisor/src/clipboard/write.rs

pub fn promote_drag_to_general(drag_entry: Arc<ClipboardEntry>) {
    // Reuse the same Arc — no data copy.
    commit_write(Pasteboard::General, drag_entry);
}
```

The `drag` pasteboard retains the entry until the next drag operation begins.

---

## 10. Security Model

### 10.1 Capability Gate

```toml
[capabilities]
clipboard                     = true   # two-phase write + read within Cmd+V grant
clipboard_provider            = true   # owns Cmd+C; always-allowed read of own writes
clipboard_observer            = false  # receive changed: events + history metadata
secure_clipboard              = false  # :secure commits (password managers)
clipboard_observer_unredacted = false  # see source_app in changed: (user opt-in via Settings)
```

Standard apps need none of these; supervisor drives Cmd+C/V on their behalf.
`clipboard_provider = true` covers rich text editors. `clipboard_observer = true`
covers clipboard manager apps. Password managers add `secure_clipboard = true`.

### 10.2 Sandbox Paste Restrictions

By default, sandboxed apps (those lacking `clipboard = true`) can only receive
`text/plain` data via the supervisor-driven Cmd+V path. Binary, image, and
file-list MIME types require an explicit `clipboard = true` grant. This prevents
a malicious app from silently extracting binary credentials placed on the
clipboard by a password manager.

### 10.3 Source Redaction

Observer apps never learn the source application of a clipboard write unless the
user has explicitly enabled `clipboard_observer_unredacted = true` through the
system Settings panel. This prevents inter-app surveillance by clipboard
monitoring utilities.

---

## 11. Large Image Memory Safety (B1 Supplement)

An `image/png` clipboard item of 4K resolution (3840×2160×4 bytes = ~31 MB) is
never held inline in supervisor memory. It is written to the staging dir by the
app as a file and delivered to readers via a read-only WASI path binding. The
supervisor never materializes the pixel buffer in its own heap.

```rust
// supervisor/src/clipboard/entry.rs

/// Maximum size for inline ClipboardData storage (64 KiB).
/// Larger payloads must use the path-backed route.
pub const INLINE_THRESHOLD: u64 = 64 * 1024;

impl ClipboardItem {
    pub fn is_inline(&self) -> bool {
        self.size <= INLINE_THRESHOLD
    }
}
```

The `ClipboardData::Image` variant is therefore never instantiated in the
supervisor process for path-backed images. It exists only for items small enough
to be transmitted inline (e.g., small icons, thumbnails).

---

## 12. Blocking Issue Resolution Summary

| Issue | Root Cause | Resolution |
|-------|-----------|-----------|
| **B1**: Write not atomic — readers could observe a half-built entry | Supervisor published token dir before all files were written | Staging dir `/run/vyoma/pb/.staging/<token>/`; `fsync` all items + verify sizes; atomic `rename` to `/run/vyoma/pb/<token>/`; entry invisible to readers until rename completes |
| **B2**: WASI descriptor grant was hand-waved in earlier drafts | No concrete mechanism to give app write access to staging dir without leaking it broadly | `OnceLock`-initialized pre-opened dir added to Wasmtime resource table on `begin` ACK; scoped to staging path only; revoked on `commit` or `abort` |
| **B3**: `clipboard_read_unattended` capability enabled clipboard exfiltration; `changed:` notifications leaked `source_app` | Over-permissive capability; privacy-unsafe default | Removed `clipboard_read_unattended` entirely; reads outside Cmd+V grant window require user-visible chrome banner; `changed:` omits `source_app` by default; `_SOURCE` supplemental line requires explicit `clipboard_observer_unredacted` opt-in |
| **B4**: Token-dir GC didn't coordinate with active reader lifetimes — a reader could hold a WASI path whose underlying dir was deleted | No refcount between read reply and app's release call | `TOKEN_REFS: OnceLock<Arc<Mutex<HashMap<[u8;16], u32>>>>` refcount table; incremented before reply, decremented on release or 30s safety timeout; GC fires only when count == 0 AND no pasteboard holds that token as `current` |
| **B5**: Supervisor-side Cmd+C fell back to `text/plain` only, silently dropping rich formatting | Supervisor had no protocol for apps to provide multi-MIME writes | `clipboard_provider = true` apps receive `VYOMA_HOTKEY:copy` and perform full §5 two-phase write within 250ms; fallback path produces `text/plain` with "Plain text copy" chrome indicator so the user knows formatting was dropped |

---

## 13. File Layout

```
supervisor/src/clipboard/
├── mod.rs          (~130 lines: PASTEBOARDS, TOKEN_REFS, Pasteboard enum, pasteboards()/token_refs() accessors, public API re-exports)
├── entry.rs        (~160 lines: ClipboardData enum, ClipboardItem, ClipboardEntry, INLINE_THRESHOLD, select_item())
├── write.rs        (~260 lines: handle_begin/item/commit/abort, grant_staging_descriptor, commit_write, broadcast_changed, staging watchdog)
├── read.rs         (~210 lines: on_read_request, authorization gate, bind_path_for_app, reply_inline/reply_path, decrement_token_ref, any_pasteboard_has_token, schedule_token_dir_delete, release handler)
├── history.rs      (~130 lines: get_history handler, watch/unwatch, observer broadcast loop, changed notification formatting)
├── secure.rs       (~130 lines: secure sweep task at 1 Hz, consume-on-read, zero-fill + unlink, focus-change hook)
└── cmd_copy.rs     (~160 lines: clipboard_provider Cmd+C path, 250ms grant window, fetch_selection fallback, promote_drag_to_general, "Plain text copy" chrome indicator)
```

All files are well within the 500-line repository limit. The `write.rs` file at
~260 lines is the largest; if it approaches the limit during implementation it
splits into `write_begin.rs` (begin/item handlers) and `write_commit.rs`
(commit/abort + broadcast).
