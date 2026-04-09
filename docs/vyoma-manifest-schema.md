# vyoma.toml — App Manifest Schema

Every VyomaOS WASM application must ship a `vyoma.toml` alongside its
`.wasm` binary. The supervisor reads this file before launching the app and
uses the declared capabilities to construct the `wasmtime` invocation with
only the permitted WASI imports.

## [app] table (required)

| Key       | Type   | Required | Description                                              |
|-----------|--------|----------|----------------------------------------------------------|
| `name`    | string | yes      | Human-readable app identifier (no spaces)                |
| `version` | string | yes      | SemVer string, e.g. `"0.1.0"`                           |
| `wasm`    | string | yes      | Filename of the `.wasm` binary relative to this manifest |

## [capabilities] table (required)

All keys default to `false` when omitted.

| Key          | Type | Default | Supervisor action when `true`                      |
|--------------|------|---------|----------------------------------------------------|
| `stdio`      | bool | false   | Passes `--inherit-stdio` to wasmtime               |
| `filesystem` | bool | false   | Mounts the app's data directory via `--dir /data`  |
| `network`    | bool | false   | Passes `--tcplisten 0.0.0.0:8080` to wasmtime      |

## Future keys (reserved, not yet implemented)

```toml
[capabilities]
clock     = false   # access to wall-clock and monotonic time
random    = false   # WASI random-get
env       = false   # inherit host environment variables
```

## Minimal valid example

```toml
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
stdio = true
```
