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
| `title` | string | no       | Optional window title for supervisor chrome              |
| `width` | u32    | no       | Logical width hint (informational, not enforced)         |
| `height`| u32    | no       | Logical height hint (informational, not enforced)        |

The default framebuffer resolution is 960×720. Window decorations (title bar,
close button) are painted by the supervisor in the top 20 px of each window;
apps should not draw in that strip.

For the full rendering protocol see [VYOMA_DRAW Protocol](vyoma-draw-protocol.md).

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
