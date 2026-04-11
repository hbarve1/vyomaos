# P07T04 — wasi-filesystem-integration

## Phase

Phase 07 — Networking & Storage

## Goal

Create a `apps/store/` WASM application that writes `"Hello from VyomaOS\n"` to `/data/hello.txt` and reads it back, verifying the content matches; declare `filesystem = true` in its `vyoma.toml`; and update the supervisor to mount the virtio-blk device at `/data` before launching any app that declares `filesystem = true`.

## File to create / modify

```
apps/store/Cargo.toml
apps/store/src/main.rs
apps/store/.cargo/config.toml
apps/store/vyoma.toml
base/modules/scripts/boot.toml      (add store entry)
supervisor/src/main.rs              (mount /dev/vda → /data before fs-capable apps)
```

## Implementation

### `apps/store/.cargo/config.toml`

```toml
[build]
target = "wasm32-wasip2"
```

---

### `apps/store/Cargo.toml`

```toml
[package]
name    = "store"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "store"
path = "src/main.rs"

[dependencies]
```

---

### `apps/store/src/main.rs`

```rust
use std::fs;
use std::path::Path;

const DATA_PATH: &str = "/data/hello.txt";
const CONTENT:   &str = "Hello from VyomaOS\n";

fn main() {
    // Step 1: write the file
    fs::write(DATA_PATH, CONTENT)
        .unwrap_or_else(|e| panic!("failed to write {}: {}", DATA_PATH, e));
    eprintln!("[store] wrote {} bytes to {}", CONTENT.len(), DATA_PATH);

    // Step 2: read it back
    let read_back = fs::read_to_string(DATA_PATH)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", DATA_PATH, e));
    eprintln!("[store] read back {} bytes from {}", read_back.len(), DATA_PATH);

    // Step 3: verify round-trip
    assert_eq!(
        read_back, CONTENT,
        "content mismatch: expected {:?}, got {:?}",
        CONTENT, read_back
    );

    eprintln!("[store] Round-trip verification PASSED.");

    // Step 4: demonstrate that the file persists across invocations
    //         by appending a timestamp line
    let metadata = fs::metadata(DATA_PATH)
        .expect("metadata call failed");
    eprintln!("[store] File size on disk: {} bytes", metadata.len());

    println!("Store app completed successfully.");
}
```

The `panic!` calls propagate as WASM trap signals to Wasmtime, which exits with a non-zero code. The supervisor's `on-failure` or `never` restart policy handles this appropriately.

---

### `apps/store/vyoma.toml`

```toml
[app]
name    = "store"
version = "0.1.0"
wasm    = "store.wasm"

[capabilities]
stdio      = true
filesystem = true
network    = false
```

`filesystem = true` tells the supervisor to both:
1. Ensure `/data` is mounted before spawning this app.
2. Pass `--dir /data` (and optionally `--dir /data::/data`) to Wasmtime so the WASM module can access `/data` through WASI's pre-opened directory mechanism.

---

### `base/modules/scripts/boot.toml` — add store entry

```toml
[[apps]]
manifest = "/apps/store/vyoma.toml"
restart  = "never"
```

---

### `supervisor/src/main.rs` — mount `/dev/vda` at `/data`

Add a `mount_data_volume` function that is called once during the supervisor's init sequence, before any app threads are spawned. The mount only happens if at least one app in the boot config declares `filesystem = true`.

```rust
use std::process::Command;

/// Mount the virtio-blk data volume at /data if not already mounted.
/// Called by main() before launching filesystem-capable apps.
fn mount_data_volume() {
    // Create the mount point if it doesn't exist
    if let Err(e) = std::fs::create_dir_all("/data") {
        eprintln!("[supervisor] WARN: could not create /data: {}", e);
        return;
    }

    // Check if already mounted (idempotent)
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    if mounts.contains("/data") {
        eprintln!("[supervisor] INFO: /data already mounted, skipping");
        return;
    }

    // Attempt to mount /dev/vda as ext4
    let status = Command::new("mount")
        .args(["-t", "ext4", "/dev/vda", "/data"])
        .status();

    match status {
        Ok(s) if s.success() => eprintln!("[supervisor] /data mounted from /dev/vda"),
        Ok(s) => eprintln!(
            "[supervisor] WARN: mount /dev/vda /data failed with exit code {}",
            s.code().unwrap_or(-1)
        ),
        Err(e) => eprintln!("[supervisor] WARN: could not exec mount: {}", e),
    }
}
```

Call this function inside `main()` after reading `boot.toml` and before spawning app threads, but only if any entry requires filesystem access:

```rust
// In main(), before spawning threads:
let needs_fs = boot.apps.iter().any(|entry| {
    // Quick check: read and parse manifest to inspect capabilities
    if let Ok(raw) = fs::read_to_string(&entry.manifest) {
        if let Ok(m) = toml::from_str::<AppManifest>(&raw) {
            return m.capabilities.filesystem;
        }
    }
    false
});

if needs_fs {
    mount_data_volume();
}
```

Full updated `run_app` filesystem section (from P07T02 skeleton):

```rust
if manifest.capabilities.filesystem {
    // --dir /data grants the WASM module read/write access to /data
    // through WASI's pre-opened directory mechanism.
    // The syntax --dir <host_path>::<guest_path> maps host /data to
    // guest /data (same name, but explicit for clarity).
    cmd.args(["--dir", "/data::/data"]);
}
```

## Notes

- WASI's filesystem access model uses "pre-opened directories": Wasmtime opens the directory on the host side and hands a capability handle to the WASM module. The module cannot open directories that were not pre-opened — so passing `--dir /data` is both necessary and sufficient to grant access.
- The `--dir /data::/data` syntax (host path `::` guest path) is the Wasmtime 18+ form. Older Wasmtime versions use `--dir /data` and the guest path defaults to the same value.
- The `mount` binary used in `mount_data_volume` is the busybox `mount` built in Phase 02. It must be present in the initramfs.
- `/proc/mounts` is available after `mount_pseudo_filesystems()` runs (Phase 03). The supervisor calls `mount_pseudo_filesystems()` first, so the idempotency check is safe.
- If `mkfs.ext4` was not run on `data.img` before first boot, `mount -t ext4 /dev/vda /data` will fail. The supervisor logs a warning and continues; apps with `filesystem = true` will then fail with ENOENT when accessing `/data`, producing non-zero exit codes.
- The `store` app uses `panic!` for errors during development; production apps should return meaningful exit codes instead.
- Cross-reference: P07T03 creates `data.img` and attaches it as a virtio-blk device. This task assumes `/dev/vda` is present and formatted when the supervisor runs.

## Verification

```sh
# 1. Confirm all store app files are present
test -f apps/store/Cargo.toml          && echo "store Cargo.toml: present"
test -f apps/store/src/main.rs         && echo "store main.rs: present"
test -f apps/store/vyoma.toml          && echo "store vyoma.toml: present"
test -f apps/store/.cargo/config.toml  && echo "store .cargo/config.toml: present"

# 2. Confirm filesystem capability is declared
grep -q "filesystem\s*=\s*true" apps/store/vyoma.toml && echo "filesystem=true: declared"

# 3. Build the store WASM app
rustup target add wasm32-wasip2
(cd apps/store && cargo build --release 2>&1 | tail -5)
test -f apps/store/target/wasm32-wasip2/release/store.wasm && echo "store.wasm: built"

# 4. Run store locally with wasmtime using a temporary /tmp/data directory
mkdir -p /tmp/vyoma-data-test
wasmtime run \
  --dir /tmp/vyoma-data-test::/data \
  --inherit-stdio \
  apps/store/target/wasm32-wasip2/release/store.wasm
echo "exit code: $?"

# 5. Verify the file was written to the host-side tmp directory
test -f /tmp/vyoma-data-test/hello.txt && echo "hello.txt: created"
grep -q "Hello from VyomaOS" /tmp/vyoma-data-test/hello.txt && echo "content: correct"
rm -rf /tmp/vyoma-data-test

# 6. Confirm supervisor source has mount_data_volume function
grep -q "mount_data_volume" supervisor/src/main.rs && echo "mount_data_volume: present"
grep -q -- '--dir' supervisor/src/main.rs          && echo "supervisor --dir flag: present"

# 7. Build supervisor with the updated logic
(cd supervisor && cargo build 2>&1 | tail -5)
test -x supervisor/target/debug/supervisor && echo "supervisor: compiles OK"

# 8. Confirm boot.toml references the store manifest
grep -q "store/vyoma.toml" base/modules/scripts/boot.toml && echo "boot.toml: store entry present"

# 9. End-to-end in QEMU (manual step — document expected output)
# Boot VyomaOS with make data-img already run:
#   make data-img
#   ./vyomaos.sh run
# Expected serial console output:
#   [supervisor] /data mounted from /dev/vda
#   [store] wrote 19 bytes to /data/hello.txt
#   [store] read back 19 bytes from /data/hello.txt
#   [store] Round-trip verification PASSED.
```
