---
title: Manifest Reference
description: Complete reference for the vyoma.toml app manifest format.
order: 1
---

# Manifest Reference (vyoma.toml)

Every VyomaOS app requires a `vyoma.toml` manifest that declares its identity and capabilities.

## Structure

```toml
[app]
name = "my-app"
version = "0.1.0"
wasm = "my-app.wasm"

[capabilities]
stdio = true
filesystem = false
network = false
display = false
shell = false
mouse = false
watchdog_secs = 0

[window]
title = "My App"
x = 0
y = 20
width = 480
height = 360
```

## `[app]` section

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Application identifier (must match binary name) |
| `version` | string | yes | Semantic version |
| `wasm` | string | yes | WASM binary filename |

## `[capabilities]` section

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `stdio` | bool | false | App inherits supervisor stdin/stdout |
| `filesystem` | bool | false | App mounts `/data` (9P persistent storage) |
| `network` | bool | false | App gets WASI sockets support |
| `display` | bool | false | App can use VYOMA_DRAW protocol |
| `shell` | bool | false | App can issue `@supervisor:` commands |
| `mouse` | bool | false | App receives mouse events |
| `watchdog_secs` | int | 0 | Kill app if silent for N seconds (0 = disabled) |

## Peripheral capabilities (embedded/robotics)

```toml
[capabilities]
touch = true

[capabilities.gpio]
pins = [4, 17]
direction = "output"

[capabilities.i2c]
bus = 1

[capabilities.spi]
bus = 0

[capabilities.uart]
port = 0

[capabilities.adc]
channel = 0
```

| Field | Type | Description |
|-------|------|-------------|
| `touch` | bool | Receive `VYOMA_INPUT:touch:` events |
| `gpio.pins` | [u8] | Exclusive access to GPIO pins |
| `gpio.direction` | string | `"input"` or `"output"` |
| `i2c.bus` | u8 | I2C bus number |
| `spi.bus` | u8 | SPI bus number |
| `uart.port` | u8 | UART port number |
| `adc.channel` | u8 | ADC channel number |

## `[window]` section (optional)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `title` | string | app name | Window title bar text |
| `x` | int | 0 | Initial X position |
| `y` | int | 20 | Initial Y position |
| `width` | int | 480 | Window width in pixels |
| `height` | int | 360 | Window height in pixels |

## Restart policies

Apps declare restart behavior in `[app]`:

```toml
[app]
restart = "always"   # Restart on exit (default for services)
# restart = "never"  # One-shot execution (default for tools)
```

## Validation

The supervisor validates manifests at startup. Invalid manifests produce errors:

```
[manifest] ERROR: unknown field "nework" in capabilities for app "my-app"
[manifest] ERROR: missing required field "name" in [app] for "my-app"
```

Run validation manually:

```bash
make check-manifests
```
