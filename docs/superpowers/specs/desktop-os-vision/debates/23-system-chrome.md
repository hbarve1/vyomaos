# Round 23 — Menu Bar & System Chrome (Architect)

**Status**: Draft
**Round**: 23
**Subsystem**: Menu Bar & System Chrome
**Analogue**: macOS `NSMenuBar`, `SystemUIServer` (clock, battery, wifi, menu extras in top menu bar)
**Author**: Architect
**Date**: 2026-05-29

---

## 1. Overview and Motivation

Every production desktop operating system draws a reserved strip of screen real estate that
belongs to the OS, not to applications. On macOS this is the 24-pixel tall menu bar at the
very top of the screen. It presents the focused application's name and menu commands on the
left, and a system tray of status icons (clock, battery, wifi, Bluetooth, volume, input
source) on the right. No application window can draw over this strip; the OS composites the
menu bar above all application content.

VyomaOS through Round 22 has no such reserved region. The compositor blits all app surfaces
at their declared window positions with no exclusion zone. An app declaring its window at
`(0, 0)` can overwrite the top of the screen entirely. There is no visual indication of
which app is focused, no persistent clock, and no trusted UI surface for consent dialogs
such as those introduced in R18 (screen capture consent) and R19 (virtual display consent).

Round 23 introduces **system chrome**: a 24-pixel tall menu bar strip at the top of the
logical screen, a system tray on its right side, an app menu area on its left side, and a
consent dialog overlay driven from the same trusted chrome surface. The mechanism that
implements this chrome is a **privileged WASM app** named `chrome` — not a compiled-in
supervisor module. This preserves the VyomaOS principle that all UI is rendered by WASM
apps using the `VYOMA_DRAW:` protocol. The chrome app is privileged in exactly two ways:
its window occupies space 0 (always on top, per R21) and the supervisor routes
`VYOMA_CHROME:` prefixed lines from any app's stdout directly into chrome's stdin, giving
chrome visibility into focus events, timer ticks, and app-declared menu items that ordinary
IPC does not provide.

### What R23 Does Not Do

- No clickable menu items (deferred to R32: menu item hit-test and callback routing).
- No real battery or wifi readings (deferred to R07 power subsystem and R55 networking).
- No animated slide-in notification banners (deferred to R73, but surface region is reserved).
- No multi-monitor chrome (each monitor gets its own chrome in a future multi-display round).
- No dark/light mode switching (deferred to R41 theming).
- No app icon in the menu bar left area (deferred to R24 app branding).
- No Spotlight / search bar in menu bar (deferred to R48).
- No menu bar customisation API for apps to add tray icons (deferred to R56 menu extras).

### Relationship to Prior Rounds

| Round | Contribution reused or extended |
|-------|---------------------------------|
| R11   | IPC broker `@<app>: msg` pattern; chrome receives events via its stdin |
| R17   | `VYOMA_DRAW:` protocol; chrome renders itself using the same command set |
| R20   | `DisplayConfig` logical points and `scale_factor`; menu bar height is 24 logical pts |
| R21   | space=0 always-on-top window model; `PerSpaceZ`; `win_space` field in `AppState` |
| R22   | `LifecycleState` (chrome has its own lifecycle); focus change events from WM |

---

## 2. Chrome App Architecture

### 2.1 The Chrome App as a Privileged WASM App

System chrome is implemented as a `wasm32-wasip2` binary located at `apps/chrome/`. It
runs under the same Wasmtime runtime as every other app. It draws its UI exclusively via
`VYOMA_DRAW:` commands written to its stdout. It reads events from its stdin. From
Wasmtime's perspective it is indistinguishable from the `gui-demo` app.

What makes chrome privileged is not its runtime environment but its **supervisor registration**.
In `boot.toml`, the chrome app entry has two additional fields that no ordinary app has:

```toml
[[app]]
name        = "chrome"
wasm        = "chrome.wasm"
space       = 0
chrome      = true    # privileged flag: supervisor enables VYOMA_CHROME: routing to this app

[app.capabilities]
stdio       = true
display     = true
shell       = true    # needed to read current app list on startup
```

The `chrome = true` flag in the manifest causes the supervisor to:

1. Register this app as the system chrome recipient. Only one app may have `chrome = true`;
   if two are listed, the supervisor logs an error and boots without chrome.
2. Route all `VYOMA_CHROME:` lines that appear in any app's stdout to chrome's stdin,
   prepended with the sending app name: `VYOMA_CHROME_FROM:<sender>:<rest_of_line>`.
3. Send supervisor-generated chrome events (focus changes, timer ticks, consent requests) to
   chrome's stdin directly, without the `VYOMA_CHROME_FROM:` prefix.
4. Clip all non-chrome app surface blits to `y >= CHROME_HEIGHT_PX` (24 logical points
   converted to physical pixels via `scale_factor`). This clipping is enforced in the
   compositor flush pass; see §10.

### 2.2 Window Configuration

The chrome app declares its window in `vyoma.toml`:

```toml
[app]
name    = "chrome"
version = "1.0.0"
wasm    = "chrome.wasm"

[capabilities]
stdio   = true
display = true
shell   = true

[window]
space       = 0
x           = 0
y           = 0
width       = 0        # 0 = "full logical screen width" — resolved at startup
height      = 24       # logical points
manual      = true     # not managed by tiling WM; chrome controls its own geometry
z_order     = 65535    # maximum Z in space 0; no other window may request Z >= 65535
```

The supervisor resolves `width = 0` to the current `DisplayConfig.logical_width` when
chrome is launched. If the screen is resized (future multi-monitor work), the supervisor
sends `VYOMA_CHROME:display_resized:<new_w>,<new_h>` to chrome's stdin so it can redraw.

### 2.3 Surface Allocation

Chrome receives a `Surface` (R11) of dimensions `(logical_w * scale_factor, 24 *
scale_factor)` pixels in physical coordinates. The surface is allocated by the supervisor
at `chrome` app spawn time, just as for any other display-capable app. Chrome clears it
with the menu bar background colour and draws text into it on every relevant event. It
calls `VYOMA_DRAW:flush` to commit the surface to the compositor.

Because chrome's `z_order` is 65535 and it is in space 0 (always on top per R21), it is
always the topmost surface in the compositor's Z-sorted blit list. The compositor blits
chrome's surface last, after all application surfaces. The y-clip on other apps (§10)
is a redundant safety guard; chrome being blitted last means it would visually overwrite
any app pixel that leaked into the top 24px regardless of clipping.

### 2.4 Supervisor Changes Required

```
supervisor/src/
  chrome_router.rs    new — route VYOMA_CHROME: lines from any app stdout to chrome stdin
  display/
    compositor.rs     modified — apply y >= CHROME_HEIGHT clip to non-chrome blit calls
  process.rs          modified — parse `chrome = true` field from boot.toml; register chrome app name
  lifecycle/
    state.rs          no change — chrome has normal LifecycleState
  ipc.rs              modified — chrome tick timer (§5.3); focus-change event dispatch (§6)
```

---

## 3. Menu Bar Layout

### 3.1 Physical Dimensions

The menu bar is a 24 logical-point tall horizontal strip at `y = 0` (top of the logical
screen). At `scale_factor = 2.0` (HiDPI, R20) this occupies 48 physical pixels of height.
At `scale_factor = 1.0` it occupies 24 physical pixels. All layout coordinates in this
section are in **logical points**; the chrome app multiplies by `scale_factor` when issuing
`VYOMA_DRAW:` commands.

```
┌─────────────────────────────────────────────────────────────────────┐  y=0
│  [App Name]  [Menu 1]  [Menu 2]  ...      [Clock]  [Wifi]  [Batt]  │  24pt
└─────────────────────────────────────────────────────────────────────┘  y=24
│                    Application Windows                               │
│                    (y >= 24 logical pts)                             │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.2 Left Section — App Menu Area

The left section spans from `x = 0` to `x = logical_w / 2`. It is divided as follows:

| Segment      | x start | x end    | Content |
|--------------|---------|----------|---------|
| App name     | 8pt     | 120pt    | Focused app's name, bold, size `m` |
| Menu item 1  | 128pt   | 200pt    | First menu item (if declared), normal weight |
| Menu item 2  | 208pt   | 280pt    | Second menu item |
| Menu item 3  | 288pt   | 360pt    | Third menu item |
| (overflow)   | —       | —        | Items beyond three are clipped; R32 adds a "More" arrow |

Menu item widths are fixed in v1 at 72pt per slot. R32 will replace this with
measured-text layout using the font metrics available after R24.

When no app is focused (initial state, before any `focus_changed` event), the left section
shows `VyomaOS` in the app name slot and no menu items.

### 3.3 Right Section — System Tray

The right section spans from `x = logical_w / 2` to `x = logical_w`. Items are
right-aligned, laid out from the right edge inward:

| Item    | Width | Content |
|---------|-------|---------|
| Clock   | 80pt  | `HH:MM` in 12-hour format with AM/PM; updated each tick event |
| Wifi    | 40pt  | Static `Wifi` placeholder text in v1; R55 replaces with icon |
| Battery | 48pt  | Static `Batt` placeholder text in v1; R07 replaces with percentage |

All right-section items have 8pt padding between them and 8pt from the right edge.

### 3.4 Colours and Typography

| Element         | Colour (RGBA u32) | Font size |
|-----------------|--------------------|-----------|
| Background      | `0x1E1E2EFF`       | —         |
| App name        | `0xCDD6F4FF`       | `m` (8×16) |
| Menu items      | `0xBAC2DEFF`       | `m`       |
| Clock           | `0xCDD6F4FF`       | `m`       |
| Placeholders    | `0x6C7086FF`       | `s` (4×8) |
| Separator line  | `0x313244FF`       | —         |

A 1-pixel horizontal separator line in colour `0x313244FF` is drawn at `y = 23` (bottom
edge of the menu bar, in logical points) to visually separate it from application content.

### 3.5 Redraw Trigger Events

Chrome redraws the full menu bar surface on receipt of any of the following stdin events:

| Event                              | Redraws left | Redraws right |
|------------------------------------|--------------|---------------|
| `VYOMA_CHROME:focus_changed:<app>` | Yes          | No            |
| `VYOMA_CHROME_FROM:<app>:menu_set` | Yes          | No            |
| `VYOMA_CHROME:tick:<timestamp>`    | No           | Yes           |
| `VYOMA_CHROME:display_resized:`    | Yes          | Yes           |
| `VYOMA_CHROME:wifi_changed:`       | No           | Yes (R55)     |
| `VYOMA_CHROME:power_changed:`      | No           | Yes (R07)     |

For efficiency, chrome may choose to redraw only the changed section. But since the menu
bar is only 24pt tall (at most 48 physical pixel rows at 2x HiDPI), a full redraw costs
approximately 960 × 48 × 4 = 184 320 bytes of surface write per frame — negligible.

---

## 4. App Menu Protocol

### 4.1 Protocol Design Goals

The app menu protocol must satisfy three constraints:

1. **App-side simplicity**: an app declares its menu items with a single `println!` call;
   no additional capability, no WIT function, no synchronous call.
2. **Chrome receives structured data**: chrome parses a delimited list of item labels and
   stores them for display.
3. **Future extensibility**: R32 will add keyboard shortcuts, item enabled/disabled state,
   and submenus. The protocol must have a versioned format that allows these additions
   without breaking v1 parsers.

### 4.2 `VYOMA_CHROME:menu_set` — App Declares Its Menu Items

Any app with `display = true` (or any `VYOMA_CHROME:` access) may write to its stdout:

```
VYOMA_CHROME:menu_set:<item1>|<item2>|<item3>
```

Rules:
- Items are separated by `|`. A maximum of 8 items is allowed in v1.
- Item labels must not contain `|`, `\n`, `:`, or non-ASCII characters in v1.
- An empty list (`VYOMA_CHROME:menu_set:`) clears all menu items for the app.
- The message is sent once at startup and whenever the app's menu items change.
- Only the currently focused app's menu items are displayed (chrome discards menu_set
  messages from non-focused apps after storing them; focus change causes chrome to display
  the newly focused app's stored menu items).

Example (Rust app):
```rust
println!("VYOMA_CHROME:menu_set:File|Edit|View|Help");
```

### 4.3 Routing Path

The supervisor's `chrome_router.rs` module intercepts any line from any app's stdout that
begins with `VYOMA_CHROME:`. Before forwarding to the `VYOMA_DRAW:` pipeline (which would
ignore unrecognised prefixes anyway), the router:

1. Strips the line from the normal stdout processing path (so it is NOT displayed on the
   serial console and NOT passed to the display subsystem as a draw command).
2. Prepends `VYOMA_CHROME_FROM:<sender_app_name>:` to the line.
3. Writes the modified line to chrome's stdin pipe, if chrome is in `Running` state.
4. If chrome is not in `Running` state (see §5 — startup race handling), the message is
   placed in a bounded queue (capacity: 64 messages). When chrome transitions to `Running`,
   the queue is drained to chrome's stdin in arrival order before any new events.

### 4.4 `VYOMA_CHROME:menu_clear` — App Clears Its Menu

```
VYOMA_CHROME:menu_clear
```

Equivalent to `menu_set` with an empty list. The chrome app removes the app's stored menu
items. If the app is currently focused, chrome redraws the left section showing only the
app name with no items.

### 4.5 Menu Item Selection Callback (R32 Preview)

When R32 implements hit-testing on menu item clicks, the chrome app will write to the
originating app's stdin:

```
VYOMA_CHROME_SELECTED:<item_label>
```

This is forwarded by the supervisor from chrome's stdout to the target app's stdin using the
standard IPC `@<app>: <msg>` mechanism. In R23 this is a specification reservation only;
no input event processing is implemented.

---

## 5. System Tray

### 5.1 Clock

The clock displays the current time in `HH:MM` format (12-hour with AM/PM suffix, e.g.
`2:47 PM`). It occupies the rightmost 80pt of the menu bar, right-aligned with 8pt padding
from the right edge.

The clock is updated once per second by a tick event sent from the supervisor to chrome's
stdin:

```
VYOMA_CHROME:tick:2026-05-29T14:47:32Z
```

Chrome parses the ISO 8601 timestamp, formats it for display, and redraws the right section
of the menu bar. The display format is: `HH:MM AM/PM` (e.g. `2:47 PM`), using size `m`
font in colour `0xCDD6F4FF`.

### 5.2 Battery Placeholder

In v1, the battery slot displays the static string `Batt` in the system tray using size `s`
font in colour `0x6C7086FF`. This is a recognised placeholder indicating that the R07 power
subsystem has not yet been integrated. When R07 is implemented, it will send
`VYOMA_CHROME:power_changed:<level_pct>,<charging_bool>` to chrome's stdin. Chrome will then
replace the placeholder with a percentage string (e.g. `78%`) and optionally a charging
indicator glyph.

### 5.3 Wifi Placeholder

In v1, the wifi slot displays the static string `Wifi` in the system tray using size `s`
font in colour `0x6C7086FF`. When R55 is implemented, the supervisor's network subsystem
will send `VYOMA_CHROME:wifi_changed:<ssid>,<signal_dbm>` to chrome's stdin.

### 5.4 Tick Mechanism

The supervisor's main event loop runs on each vsync tick (approximately 60 Hz). A monotonic
timestamp counter is compared against the last tick emission time on each iteration. When
the elapsed time since the last `tick` exceeds 1000 milliseconds, the supervisor writes
`VYOMA_CHROME:tick:<iso_timestamp>` to chrome's stdin pipe and updates the last-tick time.

Implementation site: `supervisor/src/ipc.rs`, inside the main event loop, after the
vsync/flush pass. The tick is generated unconditionally regardless of chrome's current
focus or display state. If chrome is Suspended (R22), the tick is queued in the 64-message
bounded queue (§4.3) and delivered when chrome returns to Running.

```rust
// supervisor/src/ipc.rs — inside main event loop tick
let now = SystemTime::now();
if now.duration_since(last_chrome_tick).unwrap_or_default() >= Duration::from_secs(1) {
    let ts = format_iso8601(now);
    if let Some(chrome) = apps.get("chrome") {
        write_to_app_stdin(chrome, &format!("VYOMA_CHROME:tick:{ts}\n"));
    }
    last_chrome_tick = now;
}
```

---

## 6. Focus Change Integration

### 6.1 Event Dispatch

When the user issues `focus <app>` via the shell command or when the window manager
automatically transfers focus (R21 space-switch), the supervisor sends to chrome's stdin:

```
VYOMA_CHROME:focus_changed:<app_name>
```

This is a supervisor-generated event (not routed from any app's stdout) and therefore does
NOT have the `VYOMA_CHROME_FROM:` prefix.

### 6.2 Chrome's Response

On receiving `focus_changed`, chrome:
1. Updates its internal `focused_app` state variable.
2. Looks up the stored menu items for `<app_name>` (set by a prior `menu_set` from that app
   or an empty list if no `menu_set` was ever received).
3. Clears the left section of the menu bar surface (fills with background colour
   `0x1E1E2EFF`).
4. Draws the app name in the app name slot.
5. Draws up to three menu item labels in their fixed-width slots.
6. Calls `VYOMA_DRAW:flush` to commit the updated surface.

### 6.3 Initial State

At chrome startup, before any `focus_changed` event is received, chrome draws `VyomaOS` in
the app name slot and no menu items. The first `focus_changed` event replaces this with the
focused app's name.

### 6.4 Shell and System App Filtering

The following app names are mapped to display names for the menu bar left section:

| Internal name | Display name     |
|---------------|------------------|
| `chrome`      | (never shown; chrome cannot focus itself) |
| `shell`       | `Terminal`       |
| `http-server` | `HTTP Server`    |
| `gui-demo`    | `Demo`           |
| `calculator`  | `Calculator`     |
| (any other)   | raw `app_name`   |

This mapping is hardcoded in `apps/chrome/src/main.rs` in v1. R24 (app branding) will
replace this with a display name field in `vyoma.toml`.

---

## 7. Consent Dialogs

### 7.1 Chrome as Trusted Consent UI

VyomaOS R18 introduced screen capture consent and R19 introduced virtual display consent.
Both require a trusted, tamper-proof UI to present the consent question to the user; an
ordinary app cannot be trusted because a malicious app could draw a fake consent dialog.
The chrome app is the designated consent UI because it runs in space 0 with the highest Z
order and the supervisor enforces that no other app can draw over it.

### 7.2 Consent Request Protocol

When the supervisor needs user consent, it writes to chrome's stdin:

```
VYOMA_CHROME:consent_request:<type>,<app_name>,<detail>
```

| Field      | Values                                 |
|------------|----------------------------------------|
| `type`     | `capture` (R18) or `virtual_display` (R19) |
| `app_name` | The app requesting the permission      |
| `detail`   | Human-readable context string (URL-percent-encoded to avoid comma collisions) |

Example:
```
VYOMA_CHROME:consent_request:capture,screen-recorder,Record%20the%20full%20screen
```

### 7.3 Modal Overlay Rendering

On receiving a `consent_request`, chrome:

1. Extends its Surface height from 24pt to the full logical screen height. This is done by
   requesting a surface resize via `VYOMA_DRAW:resize_surface:<w>,<h>` (a new draw command
   introduced in R23; see §9 for the full protocol). The surface then covers the full screen.
2. Draws a semi-transparent dark overlay: `VYOMA_DRAW:fill_rect:0,24,<w>,<h-24>,0x000000AA`.
3. Draws the consent dialog box centred on screen at approximately
   `(logical_w/2 - 160, logical_h/2 - 80, 320, 160)`.
4. Draws the dialog background, border, title (`Permission Request`), app name, detail
   text, and two buttons: `Allow` and `Deny`.
5. Calls `VYOMA_DRAW:flush`.

The chrome app then enters an **input capture mode** in which it ignores all events except
`VYOMA_INPUT:key:` events (from its stdin, routing is handled by supervisor's keyboard
router). The `Tab` key moves focus between Allow/Deny buttons. `Enter`/`Return` selects the
focused button. Arrow keys also navigate. No other keyboard input is routed to any other app
while chrome is in input capture mode; see §7.5 for how this is enforced.

### 7.4 Consent Response

When the user selects Allow or Deny, chrome writes to its stdout:

```
@supervisor: consent_capture_grant:<app_name>
@supervisor: consent_capture_deny:<app_name>
@supervisor: consent_virtual_display_grant:<app_name>
@supervisor: consent_virtual_display_deny:<app_name>
```

The supervisor's IPC handler processes these `@supervisor:` messages, sets the appropriate
capability flag for the target app, and resumes normal operation. Chrome then:

1. Redraws its Surface back to `(logical_w, 24)` dimensions via `VYOMA_DRAW:resize_surface`.
2. Exits input capture mode, restoring normal keyboard routing.
3. Redraws the standard menu bar and calls `VYOMA_DRAW:flush`.

### 7.5 Input Suspension During Consent Modal

Chrome is a WASM app and cannot directly intercept the supervisor's keyboard routing path.
To prevent other apps from receiving keystrokes while a consent dialog is visible, chrome
must signal the supervisor to temporarily redirect all keyboard input to chrome. It does
this via the shell capability:

```
@supervisor: input_lock_chrome
@supervisor: input_unlock_chrome
```

The supervisor's keyboard router adds an `input_locked_to: Option<String>` field. When set,
all `VYOMA_INPUT:key:` events are routed exclusively to the named app (chrome). Other apps
receive no keyboard events until `input_unlock_chrome` is issued. The supervisor validates
that only an app with `chrome = true` in its manifest may issue `input_lock_chrome`; any
other app's attempt is logged and discarded.

---

## 8. WIT Interface `vyoma:chrome@1.0.0`

The chrome WIT interface is a future-looking definition for the chrome app itself. In R23,
chrome does not use WIT bindings; it communicates entirely through stdin/stdout using the
line-oriented `VYOMA_CHROME:` protocol. The WIT interface is specified here to:

1. Document the intended structured API that will replace line-based I/O in a future WIT
   upgrade round.
2. Establish stable names for operations that will be referenced by R32 (menu item callbacks)
   and R56 (menu extras).

```wit
// wit/chrome.wit
package vyoma:chrome@1.0.0;

interface system-chrome {
    /// Set the menu items for the calling app. Max 8 items.
    set-menu-items: func(items: list<string>) -> result<_, string>;

    /// Clear all menu items for the calling app.
    clear-menu-items: func() -> result<_, string>;

    /// Get the name of the currently focused application.
    get-focused-app: func() -> option<string>;

    /// Send a consent dialog response (chrome app only, privileged).
    /// Returns error if caller is not the registered chrome app.
    send-consent-response: func(
        consent-type: consent-type,
        app-name: string,
        granted: bool,
    ) -> result<_, string>;

    /// Update a system tray slot value (chrome app only, privileged).
    update-system-tray: func(slot: tray-slot, value: string) -> result<_, string>;

    /// Lock keyboard input to the chrome app (for modal dialogs).
    /// Chrome app only, privileged.
    input-lock: func() -> result<_, string>;

    /// Release keyboard input lock.
    input-unlock: func() -> result<_, string>;
}

enum consent-type {
    capture,
    virtual-display,
}

enum tray-slot {
    clock,
    battery,
    wifi,
    custom-1,
    custom-2,
}
```

This WIT file is checked into `wit/chrome.wit` but no host bindings or guest bindings are
generated in R23. The interface is used for documentation and future code generation only.

---

## 9. VYOMA_CHROME: Protocol Reference

### 9.1 Lines from App Stdout (routed to chrome by supervisor)

| Command | Format | Description |
|---------|--------|-------------|
| `menu_set` | `VYOMA_CHROME:menu_set:<i1>|<i2>|...` | Declare menu items (up to 8) |
| `menu_clear` | `VYOMA_CHROME:menu_clear` | Clear all menu items for this app |

These lines are intercepted by the supervisor before display processing. Chrome receives
them prefixed as `VYOMA_CHROME_FROM:<app>:<original_line>`.

### 9.2 Events from Supervisor to Chrome Stdin (unprefixed)

| Event | Format | Description |
|-------|--------|-------------|
| Focus changed | `VYOMA_CHROME:focus_changed:<app>` | Focused app changed |
| Tick | `VYOMA_CHROME:tick:<iso_ts>` | 1-second timer tick |
| Display resized | `VYOMA_CHROME:display_resized:<w>,<h>` | Logical screen size changed |
| Consent request | `VYOMA_CHROME:consent_request:<type>,<app>,<detail>` | User consent needed |
| Wifi changed | `VYOMA_CHROME:wifi_changed:<ssid>,<dbm>` | (R55) |
| Power changed | `VYOMA_CHROME:power_changed:<pct>,<charging>` | (R07) |

### 9.3 Commands from Chrome Stdout (processed by supervisor)

| Command | Format | Effect |
|---------|--------|--------|
| Focus transfer | `@supervisor: focus <app>` | Standard WM focus (R21) |
| Input lock | `@supervisor: input_lock_chrome` | All kbd input → chrome |
| Input unlock | `@supervisor: input_unlock_chrome` | Restore normal kbd routing |
| Consent grant | `@supervisor: consent_<type>_grant:<app>` | Grant capability |
| Consent deny | `@supervisor: consent_<type>_deny:<app>` | Deny capability |

### 9.4 New VYOMA_DRAW: Command for Chrome: `resize_surface`

```
VYOMA_DRAW:resize_surface:<w>,<h>
```

Requests the supervisor to resize the calling app's Surface to `(w, h)` logical points.
The supervisor re-allocates the surface buffer and re-registers it in the compositor. The
app must redraw and flush after issuing this command.

Constraints:
- Only the chrome app may request a Surface taller than 24 logical points. Other apps
  attempting to resize above their manifest-declared `height` receive no error (the command
  is silently ignored) to avoid leaking chrome privilege information.
- The maximum height for a resize is `logical_screen_height` (chrome cannot exceed the
  screen).
- Width resizing is not supported in v1; only height.

---

## 10. Security and Clipping

### 10.1 Threat Model

The threat addressed in this section is: a malicious or buggy WASM app drawing UI in the
top 24 logical points of the screen, obscuring the menu bar and potentially presenting a
fake consent dialog or clock that misleads the user.

### 10.2 Defence in Depth

Two independent mechanisms prevent apps from drawing over the menu bar:

**Mechanism 1 — Compositor Z-order.**
Chrome's surface has `z_order = 65535` and is in space 0. The compositor blits surfaces in
ascending Z order. Chrome is blitted last. Any pixel an ordinary app draws in the top 24
physical-pixel rows is overwritten by chrome's surface on the same frame. This is a
sufficient visual defence even without clipping.

**Mechanism 2 — Surface clip at blit time.**
As a redundant defence, the compositor's `blit_surface` function applies a source/dest clip
when blitting non-chrome surfaces. The clip rule is:

```
if app_name != chrome_app_name {
    dest_y_min = max(dest_y_min, CHROME_HEIGHT_PX)
}
```

Where `CHROME_HEIGHT_PX = (24.0 * scale_factor).round() as u32`.

This clip is applied in `supervisor/src/display/compositor.rs` in the `flush_pass` function,
immediately before calling `blit_surface`. It does not require changes to the `Surface`
type or to individual apps. It affects only the output of the blit operation, not the app's
surface buffer (the app can still write pixels at any offset in its own surface).

### 10.3 Chrome Cannot Be Drawn Over

Because `z_order = 65535` is the maximum value reserved for chrome, and the supervisor
rejects any ordinary app's `vyoma.toml` that requests `z_order >= 65535` (logging a warning
and clamping to 65534), no application can legitimately place its window above chrome in the
Z order. Combined with the y-clip, chrome is protected by two independent mechanisms.

### 10.4 `VYOMA_CHROME:` Line Injection Prevention

A malicious app could attempt to trick the supervisor into delivering spoofed events to
chrome by writing lines formatted as supervisor-generated events (without the
`VYOMA_CHROME_FROM:` prefix). However, because supervisor-generated events bypass the
`chrome_router` entirely (they are written directly to chrome's stdin pipe by the supervisor
process, not routed through the stdout-intercepting code path), there is no way for an app
to inject a bare `VYOMA_CHROME:focus_changed:` event. All app-sourced lines are prefixed
with `VYOMA_CHROME_FROM:<sender>:` unconditionally.

---

## 11. Notification Banner (Reserved, R73)

Notification banners are out of scope for R23 but the Surface region is reserved to avoid
a future incompatible geometry change.

### 11.1 Geometry Reservation

The vertical region from `y = 24` to `y = 72` (48 logical points) is reserved for
notification banners. Application windows on the tiling WM should declare their usable
area starting from `y = 72` to accommodate future banners without repositioning. In R23,
this reservation is advisory only; the tiling WM still places windows starting at `y = 24`.
R73 will enforce the reservation by adjusting the WM's tiling origin.

### 11.2 Chrome Surface Extension for Banners

When R73 is implemented, the chrome app will use `VYOMA_DRAW:resize_surface:<w>,72` to
extend its surface to include the banner region. It will slide in a 48pt tall banner strip
from `y = 24` to `y = 72` using the animation primitives introduced in R16. Banners will
auto-dismiss after 5 seconds via the tick counter.

### 11.3 Banner Protocol (Reserved)

The following `VYOMA_CHROME:` event format is reserved for R73:

```
VYOMA_CHROME:notify:<app_name>,<title>,<body>
```

Chrome will receive this from the supervisor (triggered by an app writing
`VYOMA_CHROME:notify:<title>,<body>` to its stdout). The `<title>` and `<body>` fields are
URL-percent-encoded.

---

## 12. Platform Matrix

System chrome is appropriate only for platforms that have a display and an interactive user.
The following table defines chrome behaviour per platform profile (R43):

| Platform        | Chrome | Menu Bar | System Tray | Consent Modal |
|-----------------|--------|----------|-------------|---------------|
| `desktop-full`  | Yes    | Full, 24pt | Clock + Placeholders | Yes |
| `mobile`        | Partial | Bottom status bar, 20pt | Clock + Battery | Yes |
| `server-headless` | No   | —        | —           | —             |
| `iot-edge`      | No     | —        | —           | —             |
| `robotics-rt`   | No     | —        | —           | —             |
| `mcu-minimal`   | No     | —        | —           | —             |

### 12.1 Mobile Chrome Variant

On the `mobile` platform profile, the chrome app is included but the menu bar moves to the
**bottom** of the screen (status bar pattern, as on iOS). The status bar is 20 logical
points tall, positioned at `y = logical_h - 20`. The y-clip protection applies to
`y > logical_h - 20` for the bottom status bar (app surfaces are clipped above the status
bar at their bottom edge).

The mobile chrome does not have an app menu area on the left (there is no concept of a
persistent menu bar on mobile). It shows only: clock (centred), battery percentage (right),
and wifi signal strength (right). The `menu_set` protocol is still accepted but items are
not displayed.

### 12.2 Server, IoT, Robotics, MCU

On headless platforms (`server-headless`, `iot-edge`, `robotics-rt`, `mcu-minimal`), the
`chrome` app is not listed in `boot.toml`. The `chrome = true` field is absent. The
supervisor operates without a chrome registration; `VYOMA_CHROME:` lines from apps are
processed by the router, which discards them (logging at debug level) when no chrome app is
registered. This is the correct fallback: headless platforms do not have a menu bar but
may still run apps that issue `VYOMA_CHROME:menu_set` (the app code is shared between
platform profiles).

---

## 13. File Layout

### 13.1 New Files

```
apps/chrome/
  Cargo.toml              — [package], [[bin]], [dependencies]
  vyoma.toml              — capabilities: stdio, display, shell; window: space=0, z=65535
  src/
    main.rs               — event loop: read stdin, dispatch to handlers (≤500 lines)
    menu_bar.rs           — draw_menu_bar(), draw_left_section(), draw_right_section()
    consent.rs            — draw_consent_modal(), handle_consent_input()
    tray.rs               — draw_clock(), draw_battery(), draw_wifi()
    state.rs              — ChromeState struct: focused_app, menu_items HashMap, time_str

wit/
  chrome.wit              — vyoma:chrome@1.0.0 WIT definition (documentation only in R23)
```

### 13.2 Modified Files

```
supervisor/src/
  process.rs              — parse `chrome = true`; register chrome_app_name: Option<String>
  chrome_router.rs        — NEW: intercept VYOMA_CHROME: from stdout; route to chrome stdin
                            startup queue (capacity 64); drain on chrome Running
  ipc.rs                  — add 1-second tick emit; add input_lock_chrome / input_unlock_chrome handlers
                            add consent_grant / consent_deny handlers
  display/compositor.rs   — apply CHROME_HEIGHT_PX y-clip to non-chrome blit calls
  manifest.rs             — parse `chrome = true` field; validate at most one chrome app

base/rootfs.sh            — add chrome.wasm to initramfs; add to boot.toml with chrome=true
```

### 13.3 Line Budget

Each new source file must stay within the 500-line limit:

| File | Estimated lines |
|------|----------------|
| `apps/chrome/src/main.rs` | ~200 |
| `apps/chrome/src/menu_bar.rs` | ~120 |
| `apps/chrome/src/consent.rs` | ~150 |
| `apps/chrome/src/tray.rs` | ~80 |
| `apps/chrome/src/state.rs` | ~60 |
| `supervisor/src/chrome_router.rs` | ~120 |
| Total chrome app | ~610 → split across 5 files, each under 500 ✓ |

---

## 14. Invariants and Guarantees

The following invariants must hold at all times after R23 is implemented:

| ID  | Invariant |
|-----|-----------|
| I1  | At most one app has `chrome = true` in `boot.toml`. |
| I2  | The chrome app's Surface z_order is 65535; no other app may be granted z >= 65535. |
| I3  | The chrome app's Surface y-extent begins at 0; its minimum height is 24 logical points. |
| I4  | All `VYOMA_CHROME:` lines from app stdout are stripped before reaching the display subsystem. |
| I5  | Supervisor-generated chrome events (focus, tick, consent) are written directly to chrome's stdin, not routed through app stdout interception. |
| I6  | `input_lock_chrome` and `input_unlock_chrome` are honoured only when issued by the chrome app. |
| I7  | The 64-message startup queue ensures no `menu_set` or `focus_changed` event is lost during chrome's Launching state. |
| I8  | On headless platforms, `VYOMA_CHROME:` lines are silently discarded; apps do not error. |
| I9  | The y-clip in `blit_surface` is applied to all non-chrome apps regardless of their declared window geometry. |
| I10 | After a consent modal closes, chrome's Surface is resized back to 24pt height before `input_unlock_chrome` is issued. |

---

## 15. Open Questions and Deferred Decisions

1. **Multiple-monitor chrome (R+future)**: On a dual-monitor system, does each monitor get
   its own chrome app instance, or does a single chrome app manage both menu bars? Deferred.

2. **Chrome app crash recovery**: If the chrome app crashes and LifecycleState transitions
   to `Terminated`, should the supervisor restart it (restart = "always" in its vyoma.toml)?
   Tentative answer: yes, with a short 500ms backoff. During the restart window, the top
   24px show a fallback solid fill from the supervisor's blit clip code. Not specified here;
   deferred to R23 FINAL.

3. **Menu item click regions**: R32 will need hit-test rectangles for each menu item. The
   menu bar's fixed 72pt-per-slot layout makes this straightforward, but the pixel math
   depends on scale_factor. R32 should use the same fixed-slot geometry defined in §3.2.

4. **System tray extensibility (R56)**: Third-party apps will want to add their own status
   icons to the right tray area. The `tray-slot` enum in the WIT interface reserves
   `custom-1` and `custom-2` for this. The routing and permission model is deferred to R56.

5. **Accessibility**: The menu bar has no screen reader support. Deferred to a dedicated
   accessibility round.

---

## 16. Implementation Sequence

The following ordering minimises integration risk. Each step is independently verifiable
before the next step starts.

### Step 1 — Supervisor: parse `chrome = true`, register chrome app name

**Files**: `supervisor/src/manifest.rs`, `supervisor/src/process.rs`

Add `chrome: Option<bool>` to the `AppManifest` struct. In `process.rs`, after parsing
`boot.toml`, iterate all app entries and check for `chrome = true`. If found, store the
app name in a supervisor-level `chrome_app_name: Option<String>` field. If more than one
app declares `chrome = true`, log an error (`[chrome] duplicate chrome app: {name}, ignoring`)
and keep only the first. This step requires no display changes and no new app binary.

**Verification**: Start the supervisor with a modified `boot.toml` that sets `chrome = true`
on the shell app. Confirm that `[chrome] registered chrome app: shell` appears in the
supervisor log. Confirm that a second `chrome = true` entry produces the duplicate warning.

### Step 2 — Supervisor: startup queue + chrome_router stub

**Files**: `supervisor/src/chrome_router.rs` (new)

Create the `chrome_router.rs` module with:
- A `ChromeRouter` struct holding `chrome_app_name: Option<String>`, a `VecDeque<String>`
  startup queue (capacity 64, drop-oldest policy), and a reference to the `apps_map`.
- A `route_line(sender: &str, line: &str)` method that detects `VYOMA_CHROME:` prefix,
  formats the `VYOMA_CHROME_FROM:` prefixed message, and either writes to chrome's stdin
  (if chrome is Running) or queues it.
- A `drain_queue()` method called when chrome's LifecycleState transitions to Running.

At this stage, the router logs all routed messages but does not actually wire into chrome's
stdin (chrome does not exist yet). This lets the routing logic be unit-tested in isolation.

**Verification**: Unit test in `supervisor/tests/chrome_router.rs` that:
1. Queues 70 messages when chrome is not Running; confirms only the last 64 are retained.
2. Calls `drain_queue()` with a mock stdin writer; confirms all 64 messages are delivered
   in order with the `VYOMA_CHROME_FROM:sender:` prefix.

### Step 3 — Supervisor: y-clip in compositor flush_pass

**Files**: `supervisor/src/display/compositor.rs`

Add `chrome_app_name: Option<&str>` parameter to the `flush_pass` function (or thread it
through a context struct). In the per-app blit loop, after computing `win_y_px`, apply:

```rust
let clip_top = if Some(app_name) == chrome_app_name {
    0i32
} else {
    chrome_height_px as i32
};
let effective_y = win_y_px.max(clip_top);
let src_y_skip = (effective_y - win_y_px).max(0) as u32;
let blittable_h = surface.height_px.saturating_sub(src_y_skip);
if blittable_h > 0 {
    blit_surface(fb, &surface.buffer, win_x_px, effective_y,
                 surface.width_px, blittable_h, src_y_skip);
}
```

Also add the supervisor fallback fill: before the app blit loop, if `chrome_surface_ready`
is false, fill the top `chrome_height_px` rows of the framebuffer with `0x1E1E2E` (opaque).

**Verification**: Unit test that a surface with `win_y = 0` and height 100px, belonging to
a non-chrome app at scale_factor 1.0, blits only rows 0–75 of the source (skipping the
first 24 source rows) to destination y=24. Confirm that a chrome-app surface blits from
source row 0 to destination y=0.

### Step 4 — Build the chrome WASM app (minimal v1)

**Files**: `apps/chrome/src/`

Implement chrome in the order: `state.rs` → `tray.rs` → `menu_bar.rs` → `consent.rs` →
`main.rs`. The initial version draws a static menu bar with `VyomaOS` in the app name slot
and `--:-- --` as the clock placeholder (until the first tick event arrives).

The main event loop reads stdin line by line (blocking read):

```rust
for line in stdin().lock().lines() {
    let line = line.unwrap_or_default();
    if line.starts_with("VYOMA_CHROME:tick:") {
        state.update_time(&line["VYOMA_CHROME:tick:".len()..]);
        menu_bar::draw_right(&state);
    } else if line.starts_with("VYOMA_CHROME:focus_changed:") {
        state.set_focused_app(&line["VYOMA_CHROME:focus_changed:".len()..]);
        menu_bar::draw_left(&state);
    } else if line.starts_with("VYOMA_CHROME_FROM:") {
        handle_from_line(&mut state, &line);
    } else if line.starts_with("VYOMA_CHROME:consent_request:") {
        consent::handle_request(&mut state, &line);
    }
    // Unknown lines are silently ignored.
}
```

The first thing chrome does on startup (before entering the event loop) is draw the initial
menu bar and call `VYOMA_DRAW:flush`. This ensures chrome's surface is registered in the
compositor as early as possible, minimising the fallback-fill visible duration.

**Verification**: Boot VyomaOS in QEMU. Confirm the menu bar strip is visible at y=0.
Confirm `VyomaOS` text appears in the left section. Confirm the clock updates once per second.

### Step 5 — Wire focus_changed events from supervisor

**Files**: `supervisor/src/ipc.rs`

In the WM focus-transfer code path (wherever `focused_app` is updated after a `focus <app>`
shell command or space-switch), add a call to `chrome_router.send_supervisor_event(
&format!("VYOMA_CHROME:focus_changed:{app_name}\n"))`. This writes directly to chrome's
stdin, bypassing the startup queue (since this only runs after chrome is Running).

**Verification**: In the QEMU shell, run `focus calculator`. Confirm the menu bar left
section updates to show `Calculator`.

### Step 6 — Wire 1-second tick in event loop

**Files**: `supervisor/src/ipc.rs`

Add `last_chrome_tick: SystemTime` to the supervisor's main state, initialised to
`SystemTime::now()`. On each event loop iteration, check:

```rust
if last_chrome_tick.elapsed().unwrap_or_default() >= Duration::from_secs(1) {
    let ts = utc_now_iso8601();
    chrome_router.send_supervisor_event(&format!("VYOMA_CHROME:tick:{ts}\n"));
    last_chrome_tick = SystemTime::now(); // absorb any overage
}
```

**Verification**: Observe the menu bar clock in QEMU for 10 seconds. Confirm it updates
once per second with the correct time. Confirm no burst of updates occurs after a brief
event-loop stall.

### Step 7 — Implement consent modal

This step is the most complex and should be done last, after the basic menu bar is working
and stable. It requires implementing the `VYOMA_DRAW:resize_surface` command in the
supervisor, the modal drawing logic in `apps/chrome/src/consent.rs`, and the
`input_lock_chrome` / `input_unlock_chrome` IPC commands.

Reserve `resize_surface` as a draw command stub in `draw_cmd.rs` that logs a warning
(`[chrome] resize_surface not yet implemented`) and returns without error, so that chrome's
consent code path does not crash the app if consent logic runs before Step 7 is complete.

---

## 17. Testing Plan

### Unit Tests (supervisor)

| Test | File | What it verifies |
|------|------|-----------------|
| `chrome_router_queue_drop_oldest` | `tests/chrome_router.rs` | Queue overflow drops oldest message |
| `chrome_router_drain_order` | `tests/chrome_router.rs` | Drain delivers messages in arrival order |
| `compositor_yclip_non_chrome` | `tests/compositor.rs` | Non-chrome surface blitted with src_y_skip |
| `compositor_yclip_chrome_exempt` | `tests/compositor.rs` | Chrome surface blitted from y=0 |
| `compositor_fallback_fill` | `tests/compositor.rs` | Fallback fill applied when chrome not ready |
| `chrome_input_lock_auto_release` | `tests/ipc.rs` | Lock released on chrome lifecycle change |
| `tick_single_emission` | `tests/ipc.rs` | At most one tick per loop iteration |
| `tick_init_nonearly` | `tests/ipc.rs` | No tick in first iteration after init |

### Integration Tests (QEMU smoke)

- Boot to menu bar visible within 5 seconds (existing smoke test extended).
- Clock shows correct hour and minute on boot.
- `focus <app>` updates the menu bar left section within one vsync frame.
- An app writing `VYOMA_CHROME:menu_set:File|Edit` causes its menu items to appear in the
  menu bar when it becomes focused.
- A non-chrome app drawing at `y=0` does not visually corrupt the menu bar (chrome's
  surface overwrites any leaked pixels on next flush).

### Manual Verification Checklist

- [ ] Menu bar visible at top of screen in QEMU with `make run-gui`.
- [ ] Background colour is `#1E1E2E` (Catppuccin base).
- [ ] Clock shows `HH:MM AM/PM` format, updates once per second.
- [ ] `Wifi` and `Batt` placeholders visible in right section.
- [ ] After `focus shell`, left section shows `Terminal`.
- [ ] After `focus calculator`, left section shows `Calculator`.
- [ ] `gui-demo` sending `VYOMA_CHROME:menu_set:File|Edit|View` causes those items to appear
  when gui-demo is focused.
- [ ] No visual corruption in top 24px from other apps.
- [ ] Menu bar remains visible after running chrome for 5 minutes (no memory leak in
  Surface re-renders).
