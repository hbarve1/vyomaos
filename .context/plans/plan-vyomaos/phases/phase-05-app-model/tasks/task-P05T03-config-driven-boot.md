# P05T03 — config-driven-boot

## Phase

Phase 05 — App Model

## Goal

Replace the supervisor's blind glob scan of `/apps/*.wasm` with an explicit `boot.toml` configuration file (embedded in the initramfs at `/etc/vyoma/boot.toml`) that lists each app's manifest path and its restart policy, so the set of apps launched at boot is declarative and reviewable without inspecting the filesystem.

## File to create / modify

```
base/modules/scripts/boot.toml   (new — source file, copied into initramfs at /etc/vyoma/)
supervisor/src/main.rs           (modify discovery logic)
supervisor/Cargo.toml            (add serde + toml dependencies if not already present)
```

## Implementation

### `base/modules/scripts/boot.toml`

This is the authoritative boot configuration. The initramfs build script (`base/modules/rootfs.sh`) must copy this file to `/etc/vyoma/boot.toml` inside the cpio archive.

```toml
# VyomaOS boot configuration
# Each [[apps]] entry declares one application to launch at startup.

[[apps]]
manifest = "/apps/hello-world/vyoma.toml"
restart  = "never"

[[apps]]
manifest = "/apps/calculator/vyoma.toml"
restart  = "never"

[[apps]]
manifest = "/apps/factorial/vyoma.toml"
restart  = "never"
```

Valid values for `restart`: `"never"` | `"on-failure"` | `"always"`.
The restart logic itself is implemented in P06T02; for Phase 05 the supervisor only reads the field and stores it.

---

### `supervisor/Cargo.toml` additions

```toml
[dependencies]
serde  = { version = "1", features = ["derive"] }
toml   = "0.8"
```

(These are also needed for P05T02 manifest parsing; add once, use for both.)

---

### `supervisor/src/main.rs` — updated discovery and boot logic

Replace the existing directory-scan loop with a function that reads `boot.toml`, then resolves each manifest path to find the `.wasm` binary.

```rust
use std::{fs, path::Path, process::Command};
use serde::Deserialize;

// ── Boot config structs ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BootConfig {
    apps: Vec<BootEntry>,
}

#[derive(Debug, Deserialize)]
struct BootEntry {
    manifest: String,
    #[serde(default = "default_restart")]
    restart: String,
}

fn default_restart() -> String { "never".to_string() }

// ── App manifest structs (also used by capability checking) ───────────────

#[derive(Debug, Deserialize)]
struct AppManifest {
    app: AppMeta,
    capabilities: Capabilities,
}

#[derive(Debug, Deserialize)]
struct AppMeta {
    name: String,
    version: String,
    wasm: String,
}

#[derive(Debug, Default, Deserialize)]
struct Capabilities {
    #[serde(default)] stdio: bool,
    #[serde(default)] filesystem: bool,
    #[serde(default)] network: bool,
}

// ── Boot sequence ──────────────────────────────────────────────────────────

const BOOT_CONFIG_PATH: &str = "/etc/vyoma/boot.toml";

fn main() {
    eprintln!("[supervisor] VyomaOS supervisor starting");

    // Mount /proc and /sys (implemented in Phase 03 — kept as-is)
    mount_pseudo_filesystems();

    // Read boot configuration
    let boot_raw = fs::read_to_string(BOOT_CONFIG_PATH).unwrap_or_else(|e| {
        eprintln!("[supervisor] FATAL: cannot read {}: {}", BOOT_CONFIG_PATH, e);
        std::process::exit(1);
    });

    let boot: BootConfig = toml::from_str(&boot_raw).unwrap_or_else(|e| {
        eprintln!("[supervisor] FATAL: malformed boot.toml: {}", e);
        std::process::exit(1);
    });

    eprintln!("[supervisor] Found {} app(s) in boot config", boot.apps.len());

    // Launch each app sequentially (Phase 06 makes this concurrent)
    for entry in &boot.apps {
        launch_app(entry);
    }

    eprintln!("[supervisor] All apps completed. Halting.");
}

fn launch_app(entry: &BootEntry) {
    // Read the app's vyoma.toml
    let manifest_raw = match fs::read_to_string(&entry.manifest) {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[supervisor] WARN: cannot read manifest {}: {}", entry.manifest, e);
            return;
        }
    };

    let manifest: AppManifest = match toml::from_str(&manifest_raw) {
        Ok(m)  => m,
        Err(e) => {
            eprintln!("[supervisor] WARN: malformed manifest {}: {}", entry.manifest, e);
            return;
        }
    };

    // Resolve wasm path relative to the manifest directory
    let manifest_dir = Path::new(&entry.manifest)
        .parent()
        .unwrap_or(Path::new("/apps"));
    let wasm_path = manifest_dir.join(&manifest.app.wasm);

    eprintln!(
        "[supervisor] Launching {} v{} (restart={})",
        manifest.app.name, manifest.app.version, entry.restart
    );

    // Build the wasmtime command
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

    cmd.arg(&wasm_path);

    match cmd.status() {
        Ok(status) => eprintln!(
            "[supervisor] {} exited with {}",
            manifest.app.name,
            status.code().unwrap_or(-1)
        ),
        Err(e) => eprintln!(
            "[supervisor] ERROR: failed to exec wasmtime for {}: {}",
            manifest.app.name, e
        ),
    }
}

fn mount_pseudo_filesystems() {
    // Retained from Phase 03 — mounts /proc and /sys via nix::mount
}
```

---

### `base/modules/rootfs.sh` addition

Inside the section that builds the cpio initramfs, add a step to embed `boot.toml`:

```sh
# Copy boot config into initramfs at /etc/vyoma/boot.toml
mkdir -p "${ROOTFS_DIR}/etc/vyoma"
cp "${SCRIPT_DIR}/scripts/boot.toml" "${ROOTFS_DIR}/etc/vyoma/boot.toml"
```

The `SCRIPT_DIR` variable should already point to `base/modules/`.

## Notes

- Placing `boot.toml` in `base/modules/scripts/` keeps it next to the other build-time scripts and makes it trivial to version-control changes to the boot sequence.
- The supervisor must exit with code 1 if `boot.toml` is missing or malformed, because there is nothing meaningful it can do without knowing which apps to run.
- The `restart` field is parsed and stored by the supervisor from this phase onwards but the actual restart loop is not implemented until P06T02. Storing it as a `String` avoids a compile dependency on the `RestartPolicy` enum (defined in P06T02).
- Sequential launch (apps run one after the other) is intentional for Phase 05. Concurrent launch with `std::thread::spawn` is introduced in P06T01.
- The `/etc/vyoma/` path mirrors conventional Unix init config conventions (`/etc/init.d/`, `/etc/systemd/`), which makes the layout familiar to operators.
- Cross-reference: P05T02 defines the `AppManifest` / `Capabilities` structs; they should be moved to a shared `supervisor/src/manifest.rs` module if the file grows.

## Verification

```sh
# 1. Confirm boot.toml source file exists and is valid TOML
test -f base/modules/scripts/boot.toml && echo "boot.toml source: present"

python3 -c "
try:
    import tomllib
except ImportError:
    import tomli as tomllib
with open('base/modules/scripts/boot.toml', 'rb') as f:
    cfg = tomllib.load(f)
assert 'apps' in cfg, 'missing apps array'
assert len(cfg['apps']) >= 3, 'expected at least 3 app entries'
for entry in cfg['apps']:
    assert 'manifest' in entry, 'missing manifest key'
    assert entry['manifest'].endswith('vyoma.toml'), 'manifest must point to vyoma.toml'
print('boot.toml: valid,', len(cfg['apps']), 'apps configured')
"

# 2. Confirm each manifest path referenced in boot.toml actually exists on disk
python3 -c "
import os
try:
    import tomllib
except ImportError:
    import tomli as tomllib
with open('base/modules/scripts/boot.toml', 'rb') as f:
    cfg = tomllib.load(f)
# Paths in boot.toml are absolute runtime paths; check relative equivalents
for entry in cfg['apps']:
    rel = entry['manifest'].lstrip('/')
    assert os.path.exists(rel), f'manifest not found on host: {rel}'
    print('exists:', rel)
"

# 3. Build the supervisor and confirm it compiles with serde + toml
(cd supervisor && cargo build 2>&1 | tail -5)

# 4. Smoke-test: run supervisor against a local /tmp mock tree
mkdir -p /tmp/vyoma-test/etc/vyoma
mkdir -p /tmp/vyoma-test/apps/hello-world
cp base/modules/scripts/boot.toml /tmp/vyoma-test/etc/vyoma/boot.toml
cp apps/hello-world/vyoma.toml    /tmp/vyoma-test/apps/hello-world/
cp apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm \
   /tmp/vyoma-test/apps/hello-world/
# Override BOOT_CONFIG_PATH at runtime via env (if supervisor honours it)
# or simply check that the supervisor binary exists and help text is sane
test -x supervisor/target/debug/supervisor && echo "supervisor binary: OK"

# 5. Confirm the rootfs build script references the boot.toml copy step
grep -q "boot.toml" base/modules/rootfs.sh && echo "rootfs.sh embeds boot.toml: OK"
```
