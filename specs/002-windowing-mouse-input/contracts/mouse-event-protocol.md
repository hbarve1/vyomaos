# Contract: Mouse Event Protocol (VYOMA_INPUT)

**Version**: 2.0  
**Replaces**: `VYOMA_INPUT:mouse:lx,ly,btn` (v1, incompatible — format change)

## Overview

The supervisor writes mouse events as newline-terminated UTF-8 strings to the stdin of any app that:
1. Declares `capabilities.mouse = true` in its `vyoma.toml`
2. Has a `win_region` assigned by the supervisor
3. Has the physical cursor inside that region at the time of the event

## Message Formats

### Move Event

```
VYOMA_INPUT:mouse:move:<x>,<y>
```

| Field | Type | Description |
|-------|------|-------------|
| `x` | non-negative integer | Cursor X relative to window left edge (pixels) |
| `y` | non-negative integer | Cursor Y relative to window top edge (pixels) |

**Invariants**:
- `0 ≤ x < window_width`
- `0 ≤ y < window_height`
- Delivered on every `EV_SYN` event where cursor position changed and cursor is inside the window
- Rate: at least 30 events/second while mouse is moving (hardware-dependent; typically 125–1000 Hz)

**Example**: `VYOMA_INPUT:mouse:move:142,87`

### Click Event

```
VYOMA_INPUT:mouse:click:<x>,<y>:<button>
```

| Field | Type | Values | Description |
|-------|------|--------|-------------|
| `x` | non-negative integer | — | Cursor X at click, relative to window |
| `y` | non-negative integer | — | Cursor Y at click, relative to window |
| `button` | string | `left`, `right`, `middle` | Which button was pressed |

**Invariants**:
- Delivered on button press (EV_KEY value=1), not on release
- `0 ≤ x < window_width`
- `0 ≤ y < window_height`
- Cursor must be inside the window at press time

**Examples**:
```
VYOMA_INPUT:mouse:click:142,87:left
VYOMA_INPUT:mouse:click:200,50:right
VYOMA_INPUT:mouse:click:100,100:middle
```

## Parsing Contract (App Side)

An app receiving mouse events should parse stdin lines matching the prefix `VYOMA_INPUT:mouse:`:

```rust
if line.starts_with("VYOMA_INPUT:mouse:move:") {
    let coords = &line["VYOMA_INPUT:mouse:move:".len()..];
    // parse "x,y"
} else if line.starts_with("VYOMA_INPUT:mouse:click:") {
    let rest = &line["VYOMA_INPUT:mouse:click:".len()..];
    // rest is "x,y:button" — split on ':' to separate coords and button
}
```

## Non-Delivery Guarantees

- Apps without `capabilities.mouse = true` receive **no** mouse events, ever
- Apps with `win_region = None` receive **no** mouse events
- Cursor outside an app's window → that app receives **no** events
- Mouse events are **never** broadcast; exactly one app receives each event (topmost in Z-order if windows overlap)

## Version Migration

| Field | v1 (old) | v2 (new, this spec) |
|-------|----------|---------------------|
| Move format | `VYOMA_INPUT:mouse:lx,ly,0` | `VYOMA_INPUT:mouse:move:lx,ly` |
| Click format | `VYOMA_INPUT:mouse:lx,ly,1` | `VYOMA_INPUT:mouse:click:lx,ly:left` |
| Button encoding | `btn` integer (0=move, 1=left) | Named string (`left`, `right`, `middle`) |

Apps must be updated to handle v2 format. The supervisor will not emit v1 format after this feature lands.
