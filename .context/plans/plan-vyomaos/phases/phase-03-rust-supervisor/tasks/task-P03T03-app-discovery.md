# P03T03 — app-discovery

## Phase

Phase 03 — Rust Supervisor

## Goal

Implement `discover_apps()` in `supervisor/src/main.rs` that scans `/apps/*.wasm` using `std::fs::read_dir`, returns a sorted `Vec<PathBuf>` of discovered WASM module paths, and logs each found module to stderr.

## File to create / modify

```
supervisor/src/main.rs
```

## Implementation

Add the following function and integrate it into `main()`. This builds on the file from P03T02.

### New function to add to `supervisor/src/main.rs`

```rust
use std::path::{Path, PathBuf};

/// Scan /apps/ for WASM modules and return their paths in sorted order.
///
/// Returns an empty Vec (not an error) when /apps/ is missing or empty —
/// the supervisor should still boot cleanly with no apps to run.
///
/// Sorting by filename gives deterministic boot order across restarts,
/// which matters for apps with startup dependencies (e.g. a logger that
/// must come up before other apps write logs).
fn discover_apps() -> Vec<PathBuf> {
    const APPS_DIR: &str = "/apps";

    let dir = match std::fs::read_dir(APPS_DIR) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("vyoma-supervisor: warning: cannot read {APPS_DIR}: {e}");
            return Vec::new();
        }
    };

    let mut apps: Vec<PathBuf> = dir
        .filter_map(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("vyoma-supervisor: warning: read_dir entry error: {e}");
                    return None;
                }
            };

            let path = entry.path();

            // Accept only regular files with a .wasm extension.
            // Symlinks to .wasm files are also accepted (is_file() follows symlinks).
            if path.is_file() && path.extension().map_or(false, |ext| ext == "wasm") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    // Sort by filename (not full path) for stable ordering.
    apps.sort_by(|a, b| {
        a.file_name()
            .cmp(&b.file_name())
    });

    for app in &apps {
        eprintln!(
            "vyoma-supervisor: discovered app: {}",
            app.display()
        );
    }

    if apps.is_empty() {
        eprintln!("vyoma-supervisor: no .wasm apps found in {APPS_DIR}");
    } else {
        eprintln!("vyoma-supervisor: {} app(s) discovered", apps.len());
    }

    apps
}
```

### Updated `main()` calling the new function

```rust
fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();           // P03T02
    eprintln!("vyoma-supervisor: filesystems mounted");

    let apps = discover_apps();   // P03T03

    // P03T04 will replace this placeholder with the actual run loop
    eprintln!("vyoma-supervisor: would run {} app(s)", apps.len());

    loop {
        std::thread::park();
    }
}
```

## Notes

- `/apps/` must exist as a directory in the initramfs skeleton. `base/modules/rootfs.sh` already creates it (P02T03). At build time, `rootfs.sh` should also copy any built `.wasm` binaries from `out/apps/` into `$ROOTFS/apps/`.
- The function returns `Vec<PathBuf>` rather than `Result<Vec<PathBuf>, _>` because a missing or empty `/apps/` directory is a valid operational state (the supervisor should still boot). Errors during directory iteration are logged as warnings, not panics.
- `path.is_file()` follows symlinks intentionally — this allows `apps/` to contain symlinks to `.wasm` files stored elsewhere, which is useful during development.
- Sorting is done by `OsStr` (raw filename bytes) rather than `String` to avoid UTF-8 conversion overhead and to handle exotic filenames safely.
- This is a synchronous/blocking scan. For Phase 06 (multi-app), a file-system watch (inotify) can be layered on top to discover hot-added apps without rebooting.

## Verification

```sh
# 1. discover_apps function is defined
grep -q 'fn discover_apps' supervisor/src/main.rs

# 2. Uses std::fs::read_dir (not a shell glob or find command)
grep -q 'read_dir' supervisor/src/main.rs

# 3. Returns Vec<PathBuf>
grep -q 'Vec<PathBuf>' supervisor/src/main.rs

# 4. Filters by .wasm extension
grep -q '"wasm"' supervisor/src/main.rs

# 5. Compiles cleanly for the musl target
cargo build --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl 2>&1 | grep -v "^warning"

# 6. Unit test: discover_apps on a temp directory with mixed files
cargo test --manifest-path supervisor/Cargo.toml 2>&1 | grep -E "^test |PASSED|FAILED|ok$"
# Add this test to src/main.rs to enable the above:
# #[cfg(test)]
# mod tests {
#     use super::*;
#     use std::fs;
#     #[test]
#     fn discovers_wasm_files_only() {
#         let dir = tempfile::tempdir().unwrap();
#         fs::write(dir.path().join("app.wasm"), b"").unwrap();
#         fs::write(dir.path().join("not-wasm.txt"), b"").unwrap();
#         // discover_apps is hardcoded to /apps; test via integration or
#         // refactor to accept a path parameter.
#     }
# }

# 7. Integration test: boot VM with a .wasm file in /apps/
#    (requires full build + rootfs with a wasm file present)
# make build
# make run 2>&1 | grep -q "discovered app"
```
