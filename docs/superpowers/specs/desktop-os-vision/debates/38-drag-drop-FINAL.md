# FINAL Spec: Drag & Drop (Round 38)

**Subsystem**: Drag & Drop  
**macOS Analogue**: `NSDraggingSession` / `NSPasteboardItem` / `NSDraggingDestination`  
**Depends on**: R11 (compositor flush), R22 (app lifecycle), R32 (MOUSE_DRAG_CAPTURE, hit_test), R35 (SPSC channels), R36 (cursor shape), R37 (process exit hook)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

A drag session starts when a source app (which already holds `MOUSE_DRAG_CAPTURE` via R32)
sends `VYOMA_DRAG:start`. The supervisor mediates all payload transfer; apps never see each
other's stdin/stdout.

Key invariants:
- `DragSession` is **fully immutable** — every state transition (including hover-target change) allocates a fresh `DragSession` and CAS-swaps `ACTIVE_DRAG` (B1 fix).
- Payload transferred **out-of-band** via a WASI-mounted upload path, never inline in stdout (B4 fix).
- `MOUSE_DRAG_CAPTURE` released **immediately** on mouse-up; R38 tracks ACK-window separately (B3 fix).
- All `VYOMA_DRAG:` commands verified against sender identity; sessions identified by **128-bit random token** (B5 fix).
- R32 delivers clamped `mouse_move` to source during drag; non-droppable apps receive neither routing nor mouse events (B2 fix).

---

## 2. Session State Machine

```
Idle ──[VYOMA_DRAG:start + payload]──▶ Preparing
         │                                │
         │                                ├─[validation fail]──▶ Cancelled
         │                                │
         │                                └─[upload_done]──▶ Dragging ◀─────┐
         │                                                        │           │
         │                                              [cursor over           │leave
         │                                               droppable app]        │
         │                                                        ▼           │
         │                                                    Hovering ────────┘
         │                                                        │
         │                                              [mouse-up over target]
         │                                                        ▼
         │                                                    Dropped
         │                                                        │
         │                                              [target ack / timeout]
         │                                                        ▼
         └──[Escape/lock-rise/src-exit]───▶ Cancelled         (released)
```

**Every transition is a fresh `DragSession` CAS on `ACTIVE_DRAG`** (B1 fix). No in-place
field mutation.

```rust
// supervisor/src/drag/session.rs

#[derive(Clone)]
pub struct DragSession {
    pub token:        [u8; 16],          // B5: 128-bit random; apps use this, not an integer
    pub source:       String,
    pub types:        Vec<String>,       // MIME-like, max 8
    pub size_bytes:   u64,
    pub state:        DragState,
    pub hover_target: Option<String>,    // B1: immutable field; change = new DragSession
    pub preview:      Arc<Surface>,
    pub hotspot:      (i32, i32),
    pub staging_path: PathBuf,           // /run/vyoma/drag/<hex_token>/
    pub seq:          u64,               // B1: ABA protection on CAS
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragState { Preparing, Dragging, Hovering, Dropped, Cancelled }

pub static ACTIVE_DRAG: Lazy<ArcSwap<Option<DragSession>>> =
    Lazy::new(|| ArcSwap::from(Arc::new(None)));
```

---

## 3. Drag Start Protocol

Source already holds `MOUSE_DRAG_CAPTURE` (R32 mouse-down). It writes:

```
VYOMA_DRAG:start:<types>:<size_bytes>:<preview_hint>
```

Supervisor validates:
1. `sender == MOUSE_DRAG_CAPTURE owner` (B5 fix — rejected with `drag-error:not-capture-owner`).
2. `ACTIVE_DRAG.is_none()` (rejected with `drag-error:already-active`).
3. App has `drag = true` capability.
4. `INPUT_LOCK_LEVEL == None`.
5. `size_bytes <= 16 MiB`; types valid.

On success:
- Generates `token: [u8; 16]` via `getrandom`.
- Creates `/run/vyoma/drag/<hex_token>/upload` (tmpfs, mode 0600).
- Mounts that path into source app's WASI FS at `/drag-upload/<hex_token>` (B4 fix — upload path).
- CAS `ACTIVE_DRAG` to `Preparing` session.
- ACKs source: `VYOMA_DRAG:upload_ready:<hex_token>:<upload_path>`.

Source writes payload to `<upload_path>` via WASI fs writes, then sends:
```
VYOMA_DRAG:upload_done:<hex_token>
```

Supervisor `stat`s the file, validates size, seals it (chmod 0400). CAS `ACTIVE_DRAG` to
`Dragging`. Notifies source: `VYOMA_DRAG:started:<hex_token>`.

If `upload_done` is not received within 5 seconds: `cancel_drag(PayloadTimeout)`.

---

## 4. Drop Target Discovery (B2 Fix)

`on_drag_move(x, y)` runs on the **mouse dispatch worker** (same thread as R32's
`on_mouse_move`), called immediately after R32 processes the move.

**R32 layering rule during drag** (B2 fix):
- R32 delivers `mouse_move` to the `MOUSE_DRAG_CAPTURE` owner with coordinates
  **clamped to the source app's window bounds** (so source can scroll its content).
- Non-droppable apps under the cursor receive **no** `mouse_move` events (they don't
  own capture and aren't drop targets). This prevents cursor-position probing.
- Drop-target candidate apps receive `drag-enter/over/leave` events from R38 instead
  of `mouse_move`.

```rust
// supervisor/src/drag/routing.rs

fn on_drag_move(x: i32, y: i32) {
    loop {
        let snap = ACTIVE_DRAG.load_full();
        let session = match snap.as_ref() {
            Some(s) if matches!(s.state, DragState::Dragging | DragState::Hovering) => s,
            _ => return,
        };

        let hit = {
            let map = APPS_MAP.lock().unwrap();
            hit_test(x, y, &map)
        };
        let new_target = hit.as_ref()
            .filter(|h| app_accepts_types(&h.name, &session.types))
            .map(|h| h.name.clone());

        // B1 fix: build a new DragSession for any hover change
        let old_target = &session.hover_target;
        if old_target == &new_target && session.state == DragState::Hovering { break; }

        let new_state = if new_target.is_some() { DragState::Hovering } else { DragState::Dragging };
        let mut next = session.clone();
        next.hover_target = new_target.clone();
        next.state = new_state;
        next.seq += 1;

        // CAS: if someone else already transitioned, retry
        if ACTIVE_DRAG.compare_and_swap(&snap, Arc::new(Some(next))).ptr_eq(&snap) {
            // Successfully transitioned — send events
            match (old_target, &new_target) {
                (None,      Some(new)) => send_drag_enter(new, session, x, y),
                (Some(old), None)      => send_drag_leave(old, session),
                (Some(old), Some(new)) if old != new => {
                    send_drag_leave(old, session);
                    send_drag_enter(new, session, x, y);
                }
                (Some(_), Some(_)) => send_drag_over_throttled(session, x, y),
                _ => {}
            }
            break;
        }
        // CAS failed: loop to retry with fresh snapshot
    }
}
```

`send_drag_over` rate-limited to 60 Hz via `last_over_sent: AtomicU64` in session.

---

## 5. Data Transfer & Mouse-Up (B3 Fix)

**Mouse-up handling order** (B3 fix — `MOUSE_DRAG_CAPTURE` released immediately):

```rust
// supervisor/src/drag/transfer.rs

pub fn on_mouse_up(x: i32, y: i32) {
    // 1. R32 releases MOUSE_DRAG_CAPTURE immediately — no deferred store
    MOUSE_DRAG_CAPTURE.store(Arc::new(None));  // R32 proceeds normally from here

    // 2. R38: check active drag session
    let snap = ACTIVE_DRAG.load_full();
    let session = match snap.as_ref() {
        Some(s) if s.state == DragState::Hovering => s.clone(),
        Some(s) if s.state == DragState::Dragging => {
            cancel_drag(CancelReason::NoTarget);
            return;
        }
        _ => return,
    };

    // 3. Transition to Dropped
    let mut dropped = session.clone();
    dropped.state = DragState::Dropped;
    dropped.seq += 1;
    if !ACTIVE_DRAG.compare_and_swap(&snap, Arc::new(Some(dropped))).ptr_eq(&snap) {
        // Race: session was already cancelled; do nothing
        return;
    }

    let target = session.hover_target.as_ref().unwrap();

    // 4. Hardlink payload into target inbox
    let inbox_path = format!("/run/vyoma/inbox/{}/drag-{}", target, hex_token(&session.token));
    let _ = std::fs::hard_link(&session.staging_path.join("upload"), &inbox_path);

    // 5. Notify target via SPSC
    let line = format!("VYOMA_DRAG:drop:{}:{}:{}:{}:{}:{}\n",
        hex_token(&session.token), session.types.join(","),
        session.size_bytes, inbox_path, x, y);
    send_to_app_spsc(target, line);

    // 6. Start 2s ACK timeout
    spawn_drop_ack_timeout(session.token, 2000);
}
```

**ACK-window state** (B3 fix): `DROP_PENDING: Mutex<Option<DropPending>>` (separate from
`ACTIVE_DRAG`) tracks the target name + token during the 2-second window. R32 is fully
unlocked — new mouse events processed normally.

```rust
struct DropPending {
    token:  [u8; 16],
    source: String,
    target: String,  // B5: authoritative target name for ACK verification
}
```

---

## 6. ACK Verification (B5 Fix)

When supervisor stdout parser sees `VYOMA_DRAG:ack:<token>:<accepted|rejected>`:

```rust
// supervisor/src/drag/transfer.rs

fn on_drag_ack(sender: &str, token_hex: &str, accepted: bool) {
    let pending = DROP_PENDING.lock().unwrap();
    let p = match pending.as_ref() {
        Some(p) if p.token_hex() == token_hex => p,
        _ => {
            log_warn!(Subsystem::Drag, Some(sender),
                "drag-ack for unknown/stale token={}", token_hex);
            return; // B5: silently drop spoofed acks
        }
    };

    // B5 fix: verify sender is the authoritative target (not any arbitrary app)
    if sender != p.target {
        log_warn!(Subsystem::Drag, Some(sender),
            "drag-ack from non-target app (expected={})", p.target);
        return;
    }

    // Cancel timeout, notify source, cleanup
    cancel_drop_ack_timeout(&p.token);
    let result = if accepted { "accepted" } else { "rejected" };
    send_to_app_spsc(&p.source, &format!("VYOMA_DRAG:complete:{}:{}\n", token_hex, result));
    let _ = std::fs::remove_file(&format!("/run/vyoma/inbox/{}/drag-{}", p.target, token_hex));
    let _ = std::fs::remove_dir_all(&format!("/run/vyoma/drag/{}", token_hex));
    *pending = None;
    // If ACTIVE_DRAG is still in Dropped state, clear it
    let snap = ACTIVE_DRAG.load_full();
    if snap.as_ref().map(|s| s.state) == Some(DragState::Dropped) {
        ACTIVE_DRAG.store(Arc::new(None));
    }
}
```

**All `VYOMA_DRAG:` command authorization** (B5 fix):
- `start`: sender must == `MOUSE_DRAG_CAPTURE` owner.
- `upload_done`: sender must == `ACTIVE_DRAG.source`.
- `cancel`: sender must == `ACTIVE_DRAG.source`.
- `ack`: sender must == `DROP_PENDING.target`.
- Violation: log + drop; `session_id` is a 128-bit random token (not enumerable integer).

---

## 7. Cancellation

```rust
// supervisor/src/drag/cancel.rs

pub enum CancelReason {
    Escape, LockRise(u8), SourceExit, TargetExit,
    DropTimeout, PayloadTimeout, NoTarget,
}

pub fn cancel_drag(reason: CancelReason) {
    // B1 fix: CAS loop to claim the session
    loop {
        let snap = ACTIVE_DRAG.load_full();
        let session = match snap.as_ref() {
            Some(s) if !matches!(s.state, DragState::Cancelled | DragState::Dropped) => s.clone(),
            _ => return, // already terminal
        };
        let mut cancelled = session.clone();
        cancelled.state = DragState::Cancelled;
        cancelled.seq += 1;
        if ACTIVE_DRAG.compare_and_swap(&snap, Arc::new(Some(cancelled))).ptr_eq(&snap) {
            // We own the cancel — do cleanup
            if let Some(ref t) = session.hover_target {
                send_to_app_spsc(t, &format!("VYOMA_DRAG:leave:{}\n", hex_token(&session.token)));
            }
            if !matches!(reason, CancelReason::SourceExit) {
                send_to_app_spsc(&session.source,
                    &format!("VYOMA_DRAG:cancelled:{}:{}\n", hex_token(&session.token), reason.label()));
            }
            let _ = std::fs::remove_dir_all(&session.staging_path);
            // DROP_PENDING cleanup if in that window
            *DROP_PENDING.lock().unwrap() = None;
            break;
        }
    }
}
```

**Wired into**:
- `on_input_lock_rise()` (R32/R35): `cancel_drag(LockRise(new_level))` before other cancellations.
- Escape key handler in input thread pre-routing stage: `cancel_drag(Escape)`.
- `on_app_exit(app)` (R37 hook site): check if `app == source` → `SourceExit`; if `app == hover_target` → `TargetExit` (session continues, next `on_drag_move` will see None target).

---

## 8. Drag Image / Ghost Sprite

`DragSession.preview: Arc<Surface>` rendered by supervisor compositor, not source app.
Blitted in R11 flush pass after all windows and chrome at `(CURSOR_POS - hotspot)` with
80% global alpha. Above all z-layers (topmost).

- `preview_hint = text:<label>`: supervisor renders 200×24 rounded rect with medium font.
- `preview_hint = image:<w>x<h>`: source writes image as part of WASI upload file (separate named section); max 256×256 RGBA.

R36 cursor shape: target sends `drag-enter-ack:<token>:copy|link|move|reject` → R36 switches to `grab-copy`, `grab-link`, `grabbing`, or `not_allowed` cursor.

---

## 9. Capability Gate

```toml
# apps/my-app/vyoma.toml
[capabilities]
drag = true            # may start drag sessions (requires filesystem=true for upload path)

[capabilities.drop]
accepts = ["text/plain", "text/uri-list"]  # types this app can receive
```

`drag = true` requires `filesystem = true` (upload path needs WASI fs access).
Validation in `supervisor/src/manifest.rs`.

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: ACTIVE_DRAG mid-struct mutation breaks CAS atomicity | `DragSession` fully immutable; every state transition (including hover_target change) allocates fresh struct and CAS-loops with `seq: u64` ABA protection |
| B2: Drop target discovery silently swallows R32 mouse events | R32 delivers clamped `mouse_move` to source only; non-droppable apps under cursor receive neither mouse_move nor drag events during session |
| B3: Deferred MOUSE_DRAG_CAPTURE release races R32 mouse-up | `MOUSE_DRAG_CAPTURE` released immediately on mouse-up by R32; `DROP_PENDING` struct tracks ACK-window separately; R32 fully unblocked |
| B4: 16 MiB payload inline in stdout stalls stdout parser | Out-of-band via WASI-mounted upload path (`/run/vyoma/drag/<token>/upload`); supervisor stat's file after `upload_done`; 5s upload timeout |
| B5: Malicious app spoofs drag-ack for another session | All `VYOMA_DRAG:` commands verified against sender identity; `ack` verified sender==`DROP_PENDING.target`; session token is 128-bit random (not enumerable integer) |

---

## 11. File Layout

```
supervisor/src/drag/
├── mod.rs          (~100 lines: ACTIVE_DRAG, DROP_PENDING, public API)
├── session.rs      (~200 lines: DragSession (immutable), DragState, token generation)
├── start.rs        (~200 lines: VYOMA_DRAG:start validation, upload path mount, Preparing→Dragging)
├── routing.rs      (~250 lines: on_drag_move CAS loop (B1), hit-test, enter/leave/over, B2 layering)
├── transfer.rs     (~250 lines: on_mouse_up (B3), hardlink, ACK handler (B5), timeout)
├── cancel.rs       (~150 lines: cancel_drag CAS loop, CancelReason, wiring)
├── preview.rs      (~180 lines: ghost sprite render, compositor blit hook)
└── manifest.rs     (~100 lines: drag/drop capability parse, accepts type list)
```
