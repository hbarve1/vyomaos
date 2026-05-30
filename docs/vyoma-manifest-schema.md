# vyoma.toml — App Manifest Schema

Every VyomaOS WASM application must ship a `vyoma.toml` alongside its
`.wasm` binary. The supervisor reads this file before launching the app and
uses the declared capabilities to construct the `wasmtime` invocation with
only the permitted WASI imports.

## [app] table (required)

| Key          | Type   | Required | Description                                                            |
|--------------|--------|----------|------------------------------------------------------------------------|
| `name`       | string | yes      | Human-readable app identifier (no spaces, must be unique across boot)  |
| `version`    | string | yes      | SemVer string, e.g. `"0.1.0"`                                         |
| `wasm`       | string | yes      | Filename of the `.wasm` binary relative to this manifest               |
| `wasm_sha256`| string | no       | Optional SHA-256 hex digest for binary integrity verification          |
| `icon`       | string | no       | Relative path to a PNG icon (e.g. `"icon.png"`), resolved from the manifest directory |

## [capabilities] table (optional)

All keys default to `false` / `0` when omitted. Unknown keys are rejected at
startup with a `[manifest]` error — declare only capabilities the app actually
needs.

| Key             | Type | Default | Supervisor action when `true`                                            |
|-----------------|------|---------|--------------------------------------------------------------------------|
| `stdio`         | bool | false   | Passes `--inherit-stdio` to wasmtime; app can write to console and receive keyboard input |
| `filesystem`    | bool | false   | Mounts `/data` (9P, persistent host volume) via `--dir /data`           |
| `network`       | bool | false   | Passes `--tcplisten 0.0.0.0:<port>` to wasmtime                         |
| `network_port`  | u16  | 8080    | TCP port used when `network = true`                                      |
| `display`       | bool | false   | App may write `VYOMA_DRAW:` commands to stdout; supervisor routes them to the framebuffer driver. See [VYOMA_DRAW Protocol](vyoma-draw-protocol.md). |
| `shell`         | bool | false   | App may issue `@supervisor:` IPC commands (process management)           |
| `mouse`         | bool | false   | App receives `VYOMA_INPUT:mouse:` events when the cursor is in its window region |
| `watchdog_secs` | u32  | 0       | Supervisor kills and (if restart=always) restarts the app if it produces no stdout for N seconds; `0` disables |

## [window] table (optional)

Required for apps using the display system. Defines the screen region where
the app renders. The supervisor clips all draw commands to this region.

| Key     | Type   | Required | Description                                              |
|---------|--------|----------|----------------------------------------------------------|
| `x`     | u32    | yes      | Left edge of window region in pixels                     |
| `y`     | u32    | yes      | Top edge of window region in pixels (0 = top of screen)  |
| `w`     | u32    | yes      | Width of window region in pixels                         |
| `h`     | u32    | yes      | Height of window region in pixels                        |
| `z`     | u32    | no       | Z-layer for window stacking order (default `10`). Well-known layers: `0` = desktop background, `10` = default app, `100` = dock, `200` = menu bar, `255` = overlay |
| `title` | string | no       | Optional window title for supervisor chrome              |
| `width` | u32    | no       | Logical width hint (informational, not enforced)         |
| `height`| u32    | no       | Logical height hint (informational, not enforced)        |

The default framebuffer resolution is 960×720. Window decorations (title bar,
close button) are painted by the supervisor in the top 20 px of each window;
apps should not draw in that strip.

For the full rendering protocol see [VYOMA_DRAW Protocol](vyoma-draw-protocol.md).

## [[menu_items]] array of tables (optional)

Declarative menu items shown in the global menu bar when this app is focused.
Each entry is a `[[menu_items]]` TOML array-of-tables element.

| Key        | Type   | Required | Default | Description                                                  |
|------------|--------|----------|---------|--------------------------------------------------------------|
| `label`    | string | yes      | --      | Text displayed in the menu bar                               |
| `action`   | string | yes      | --      | Action identifier sent to the app via stdin when the item is clicked |
| `enabled`  | bool   | no       | `true`  | Whether the menu item is clickable                           |
| `shortcut` | string | no       | --      | Keyboard shortcut hint (e.g. `"Ctrl+S"`), displayed but not enforced by supervisor |

## Peripheral capability sub-tables (optional)

Apps on embedded and robotics platforms may declare hardware peripheral
capabilities as sub-tables under `[capabilities]`. Each sub-table grants the
app exclusive access to the specified peripheral. Peripheral sub-tables are
parsed separately from the flat capability fields and enforced by the
`PeripheralEnforcer` at spawn time.

### [capabilities.gpio]

| Key         | Type     | Required | Description                                                      |
|-------------|----------|----------|------------------------------------------------------------------|
| `pins`      | `[u8]`   | yes      | GPIO pin numbers the app may access (empty array = no pins)      |
| `direction` | string   | no       | Allowed direction: `"input"`, `"output"`, or omit for both       |

### [capabilities.i2c]

| Key       | Type | Required | Description                                                         |
|-----------|------|----------|---------------------------------------------------------------------|
| `bus`     | u8   | yes      | I2C bus number                                                      |
| `address` | u8  | no       | Device address (`0x00`--`0x7F`); omit to allow any address on the bus |

### [capabilities.spi]

| Key      | Type | Required | Description                                             |
|----------|------|----------|---------------------------------------------------------|
| `bus`    | u8   | yes      | SPI bus number                                          |
| `cs_pin` | u8  | no       | Chip-select pin; omit to allow any CS pin on the bus    |

### [capabilities.uart]

| Key    | Type | Required | Description                                                |
|--------|------|----------|------------------------------------------------------------|
| `port` | u8   | yes      | UART port number                                           |
| `baud` | u32  | no       | Baud rate (e.g. `115200`); omit to allow any baud rate     |

### [capabilities.adc]

| Key        | Type   | Required | Description                                            |
|------------|--------|----------|--------------------------------------------------------|
| `channels` | `[u8]` | yes      | ADC channel indices the app may sample                 |

## Restart policy (boot.toml)

Restart policy is set per-entry in `/etc/vyoma/boot.toml`, not in the app
manifest:

| Value    | Behavior                                              |
|----------|-------------------------------------------------------|
| `never`  | App runs once; supervisor logs exit code (default)    |
| `always` | Supervisor restarts app immediately on any exit       |

## Examples

### Minimal headless app

```toml
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
stdio = true
```

### Display app with window

```toml
[app]
name    = "my-gui"
version = "0.1.0"
wasm    = "my-gui.wasm"

[capabilities]
display = true
stdio   = true

[window]
x = 0
y = 20
w = 960
h = 700
title = "My GUI App"
```

### Network server

```toml
[app]
name         = "http-server"
version      = "0.1.0"
wasm         = "http-server.wasm"

[capabilities]
network      = true
network_port = 8080
filesystem   = true
stdio        = true
```

### App with menu items

```toml
[app]
name    = "editor"
version = "0.1.0"
wasm    = "editor.wasm"
icon    = "editor-icon.png"

[capabilities]
display = true
stdio   = true

[window]
x     = 0
y     = 20
w     = 960
h     = 700
z     = 10
title = "Editor"

[[menu_items]]
label    = "Save"
action   = "file:save"
shortcut = "Ctrl+S"

[[menu_items]]
label   = "Undo"
action  = "edit:undo"
enabled = true
```

### Embedded app with peripheral capabilities

```toml
[app]
name    = "sensor-reader"
version = "0.1.0"
wasm    = "sensor-reader.wasm"

[capabilities]
stdio = true

[capabilities.gpio]
pins      = [4, 17]
direction = "input"

[capabilities.i2c]
bus     = 1
address = 0x48

[capabilities.adc]
channels = [0, 1, 2]
```
