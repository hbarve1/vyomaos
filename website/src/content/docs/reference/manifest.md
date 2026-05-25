---
title: App Manifest
description: The vyoma.toml capability manifest reference.
---

Every VyomaOS app has a `vyoma.toml` manifest file that declares its identity and capabilities.

## Format

```toml
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
stdio      = true
filesystem = true
network    = true
display    = true
shell      = true
mouse      = true
watchdog_secs = 30
```

## `[app]` Section

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | App identifier (must match directory name) |
| `version` | string | yes | Semantic version |
| `wasm` | string | yes | WASM binary filename (relative to app dir) |

## `[capabilities]` Section

All capabilities default to `false` (not wired up). Only declare what your app needs.

### `stdio`

**Type**: `bool`

Wires up stdin, stdout, and stderr. Required for:
- Console output (`println!`)
- Receiving keyboard input via stdin
- Using the VYOMA_DRAW display protocol (writes to stdout)
- IPC messaging (writes `@<target>: msg` to stdout)

### `filesystem`

**Type**: `bool`

Mounts the persistent `/data` directory (9P virtio, backed by host `data/` folder). Files written here survive VM reboots.

### `network`

**Type**: `bool`

Enables WASI sockets support. App can bind TCP ports (default: 8080). Requires booting with `make run-net` or `make run-gui-net`.

### `display`

**Type**: `bool`

Grants access to the framebuffer. App can use the `VYOMA_DRAW:` protocol to render graphics. The supervisor manages window chrome around the app's drawing area.

### `shell`

**Type**: `bool`

Allows the app to issue `@supervisor:` IPC commands for process management (list, kill, restart, focus, etc.).

### `mouse`

**Type**: `bool`

App receives `VYOMA_INPUT:mouse:` events when the cursor is within the app's window region. Events include click, move, and drag with coordinates.

### `watchdog_secs`

**Type**: `integer`

If set to a non-zero value, the supervisor kills the app if it produces no output for N seconds. Set to `0` or omit to disable.

## Validation

The supervisor validates all manifests at startup. Invalid manifests produce clear error messages:

- Unknown fields in `[capabilities]` → error with field name
- Missing `[app]` section → parse error
- Missing required fields → parse error
- TOML syntax errors → line/column reported

Run `make check-manifests` to validate all app manifests without booting.

## Examples

### Minimal app (console only)

```toml
[app]
name    = "hello-world"
version = "0.1.0"
wasm    = "hello-world.wasm"

[capabilities]
stdio = true
```

### GUI app with mouse input

```toml
[app]
name    = "paint"
version = "0.1.0"
wasm    = "paint.wasm"

[capabilities]
stdio   = true
display = true
mouse   = true
```

### Full-featured app

```toml
[app]
name    = "file-manager"
version = "0.1.0"
wasm    = "file-manager.wasm"

[capabilities]
stdio      = true
filesystem = true
display    = true
shell      = true
mouse      = true
```
