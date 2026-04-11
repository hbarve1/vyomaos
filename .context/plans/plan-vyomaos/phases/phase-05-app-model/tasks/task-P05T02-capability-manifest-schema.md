# P05T02 — capability-manifest-schema

## Phase

Phase 05 — App Model

## Goal

Define and document a `vyoma.toml` manifest schema in TOML format that each app ships alongside its `.wasm` binary, declaring its identity (name, version, wasm filename) and the set of WASI capabilities it requires, so the supervisor can grant only the declared imports instead of providing full WASI access by default.

## File to create / modify

```
apps/hello-world/vyoma.toml   (new)
apps/calculator/vyoma.toml    (new)
apps/factorial/vyoma.toml     (new)
docs/vyoma-manifest-schema.md (new)
```

## Implementation

### `apps/hello-world/vyoma.toml`

```toml
[app]
name    = "hello-world"
version = "0.1.0"
wasm    = "hello-world.wasm"

[capabilities]
stdio      = true
filesystem = false
network    = false
```

### `apps/calculator/vyoma.toml`

```toml
[app]
name    = "calculator"
version = "0.1.0"
wasm    = "calculator.wasm"

[capabilities]
stdio      = true
filesystem = false
network    = false
```

### `apps/factorial/vyoma.toml`

```toml
[app]
name    = "factorial"
version = "0.1.0"
wasm    = "factorial.wasm"

[capabilities]
stdio      = true
filesystem = false
network    = false
```

---

### `docs/vyoma-manifest-schema.md`

This file documents the full schema so future app authors know every valid key.

```markdown
# vyoma.toml — App Manifest Schema

Every VyomaOS WASM application must ship a `vyoma.toml` alongside its
`.wasm` binary.  The supervisor reads this file before launching the app and
uses the declared capabilities to construct the `wasmtime` invocation with
only the permitted WASI imports.

## [app] table (required)

| Key       | Type   | Required | Description                                      |
|-----------|--------|----------|--------------------------------------------------|
| `name`    | string | yes      | Human-readable app identifier (no spaces)        |
| `version` | string | yes      | SemVer string, e.g. `"0.1.0"`                   |
| `wasm`    | string | yes      | Filename of the `.wasm` binary relative to this manifest |

## [capabilities] table (required)

All keys default to `false` when omitted.

| Key          | Type | Default | Supervisor action when `true`                          |
|--------------|------|---------|--------------------------------------------------------|
| `stdio`      | bool | false   | Passes `--inherit-stdio` to wasmtime                  |
| `filesystem` | bool | false   | Mounts the app's data directory via `--dir /data`     |
| `network`    | bool | false   | Passes `--tcplisten 0.0.0.0:8080` to wasmtime         |

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
```

---

### Supervisor integration (pseudocode — implemented in P05T03)

```rust
// In supervisor/src/main.rs, after reading boot.toml:
let manifest: AppManifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;

let mut cmd = Command::new("wasmtime");
cmd.arg("run");

if manifest.capabilities.stdio {
    cmd.arg("--inherit-stdio");
}
if manifest.capabilities.filesystem {
    cmd.args(["--dir", "/data"]);
}
if manifest.capabilities.network {
    cmd.args(["--tcplisten", "0.0.0.0:8080"]);
}

let wasm_path = manifest_dir.join(&manifest.app.wasm);
cmd.arg(wasm_path);
```

The Rust structs that correspond to the TOML schema:

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AppManifest {
    pub app: AppMeta,
    pub capabilities: Capabilities,
}

#[derive(Debug, Deserialize)]
pub struct AppMeta {
    pub name: String,
    pub version: String,
    pub wasm: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub stdio: bool,
    #[serde(default)]
    pub filesystem: bool,
    #[serde(default)]
    pub network: bool,
}
```

Add `serde` and `toml` to `supervisor/Cargo.toml`:

```toml
[dependencies]
serde  = { version = "1", features = ["derive"] }
toml   = "0.8"
```

## Notes

- The `[capabilities]` table is intentionally minimal for Phase 05. Additional keys (clock, random, env) are reserved in the schema doc but not wired up until later phases.
- The `wasm` key holds a filename (not an absolute path) so the manifest is portable across build environments; the supervisor resolves it relative to the directory that contains the `vyoma.toml`.
- `filesystem = true` uses the virtio-blk `/data` mount introduced in Phase 07; setting it to `true` in Phase 05 is harmless — the supervisor will just pass `--dir /data` which is a no-op if the directory does not exist yet.
- All three existing apps only need `stdio = true`. This keeps their attack surface minimal.
- The schema documentation lives in `docs/` rather than a root `README.md` to keep it versioned and linkable without polluting the project root.

## Verification

```sh
# 1. Confirm all three manifest files are present and parseable as TOML
for f in apps/hello-world/vyoma.toml apps/calculator/vyoma.toml apps/factorial/vyoma.toml; do
  python3 -c "
import sys
try:
    import tomllib
except ImportError:
    import tomli as tomllib  # pip install tomli for Python < 3.11
with open('$f', 'rb') as fh:
    data = tomllib.load(fh)
assert 'app' in data and 'capabilities' in data, 'missing top-level tables'
assert data['app']['wasm'].endswith('.wasm'), 'wasm key must end in .wasm'
print('$f: OK', data['app']['name'], data['app']['version'])
"
done

# 2. Alternative: use the `toml` CLI if available (cargo install toml-cli)
# toml get apps/hello-world/vyoma.toml app.name

# 3. Confirm name/version/wasm fields are present in each manifest
grep -E "^name\s*=" apps/hello-world/vyoma.toml
grep -E "^version\s*=" apps/hello-world/vyoma.toml
grep -E "^wasm\s*=" apps/hello-world/vyoma.toml

# 4. Confirm capabilities table exists and stdio = true for all three apps
grep "stdio\s*=\s*true" apps/hello-world/vyoma.toml
grep "stdio\s*=\s*true" apps/calculator/vyoma.toml
grep "stdio\s*=\s*true" apps/factorial/vyoma.toml

# 5. Confirm filesystem and network are false (or absent) for all three
grep -v "filesystem\s*=\s*true" apps/hello-world/vyoma.toml > /dev/null
grep -v "network\s*=\s*true"    apps/hello-world/vyoma.toml > /dev/null

# 6. Confirm schema doc exists
test -f docs/vyoma-manifest-schema.md && echo "schema doc: present"
grep -q "\[capabilities\]" docs/vyoma-manifest-schema.md && echo "schema doc: capabilities section found"
```
