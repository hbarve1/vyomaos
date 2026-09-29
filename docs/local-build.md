# Pinned local build environment

The supported development target is Linux x86-64: a musl supervisor/CLI and `wasm32-wasip2` apps. Build and test locally while GitHub Actions is billing-blocked.

## Input ownership

| Input | Authoritative file / identity |
|---|---|
| Rust and installed targets | Root `rust-toolchain.toml`: 1.96.0, minimal profile, musl + WASI Preview 2 |
| Ubuntu base | `docker/Dockerfile`: 22.04 linux/amd64 manifest digest `281c5745f657873d78e5531fc5ba8575f46ab7769b94550ac99543f122679986` |
| Ubuntu packages | Same Dockerfile: snapshot `20260928T000000Z`; archive signatures and TLS remain enabled |
| Initial CA bundle | Checksum-pinned Ubuntu CA package in Dockerfile, required before snapshot HTTPS can work |
| rustup installer | Same Dockerfile: 1.27.1 plus SHA-256 |
| Linux, BusyBox, Wasmtime | `base/versions.sh`: versions and mandatory SHA-256 checks |
| Workspace Rust dependencies | Root `Cargo.lock` |
| Standalone app/tool dependencies | Each app/tool's committed `Cargo.lock`; build/test with `--locked` |
| Python tests | `tests/e2e/requirements.txt`: direct/transitive version pins and artifact hashes |

The kernel remains 5.10.113, BusyBox 1.35.0 and Wasmtime 43.0.0, preserving the previous development baseline. Pinning establishes input identity; it does not establish release security qualification or bit-for-bit reproducible images.

Ubuntu snapshots freeze package resolution; their retention is finite, so archive/mirror retention belongs in the longer-term release supply chain. Historical metadata expiry is disabled only for these snapshot sources; signature and certificate checks stay active. See [Canonical's snapshot documentation](https://snapshot.ubuntu.com/).

## Clean builder

Prerequisites on the host: Docker with BuildKit and Linux amd64 support, Git, Make. For host VM tests also install QEMU x86, socat and Python >=3.10 with venv support. Do not change Docker socket permissions or disable the Codex sandbox to run builds.

From the repository root:

```bash
docker build --no-cache --platform linux/amd64 \
  -t vyomaos-builder:t0-02 -f docker/Dockerfile .
docker run --rm vyomaos-builder:t0-02 rustc --version
docker run --rm vyomaos-builder:t0-02 cat /opt/vyoma/builder-packages.txt
```

The build context includes only the Dockerfile and toolchain file. It never imports a host Cargo registry, toolchain, compiled target or Python venv. Network downloads are still needed on the first build. Rustup installs the version/targets in the same file used for host development; no floating `stable` resolution occurs.

The builder's default Rust linker uses the bundled musl CRT. `musl-gcc` is installed for C dependencies; do not force it as Cargo's final linker (see [T0-01 evidence](validation/2026-09-29-t0-01.md)).

## Local build and test

A helper isolates tests that write absolute state paths:

```bash
vyoma_builder() {
  docker run --rm --platform linux/amd64 --user "$(id -u):$(id -g)" \
    --tmpfs /data:rw,mode=1777 --tmpfs /apps:rw,mode=1777 \
    -v "$PWD":/work -w /work vyomaos-builder:t0-02 "$@"
}
vyoma_builder env RUSTFLAGS='-D warnings' cargo test --locked \
  --workspace --target x86_64-unknown-linux-musl --no-fail-fast -- --test-threads=1
vyoma_builder env RUSTFLAGS='-D warnings' cargo build --locked \
  --workspace --target x86_64-unknown-linux-musl --release
for app in desktop dock gui-demo shell; do
  vyoma_builder cargo test --locked --manifest-path "apps/$app/Cargo.toml" \
    --target x86_64-unknown-linux-musl -- --test-threads=1
done
for tool in check-manifests gui-test; do
  vyoma_builder cargo test --locked --manifest-path "tools/$tool/Cargo.toml" \
    --target x86_64-unknown-linux-musl
done
vyoma_builder cargo run --locked --manifest-path tools/check-manifests/Cargo.toml
bash tests/build/download_test.sh
```

To build all images, `make build IMAGE_TAG=t0-02` uses the pinned Dockerfile and locked Cargo resolution. T0-03/T0-04 still need to repair kernel input propagation, stale artifact handling and image selection. For a clean gate use a new worktree; do not reuse another branch's compiled kernel/initramfs. Download archives may be reused only because the scripts recheck their digests. A corrupt cache fails explicitly; remove/replace that file before retrying.

Python on the host:

```bash
bash scripts/setup-tests.sh
BZIMAGE="$PWD/out/bzImage" INITRAMFS="$PWD/out/initramfs.cpio.gz" \
  PYTHONDONTWRITEBYTECODE=1 out/test-venv/bin/python -m pytest \
  tests/e2e -v --continue-on-collection-errors
```

If the host lacks Python venv support, use `vyoma_builder bash scripts/setup-tests.sh` and execute that venv inside the same builder. A venv created in Ubuntu 22.04 uses Python 3.10 and must not be executed with the host's Python 3.12. The builder includes QEMU and socat, so the entire Python/VM suite can also run there (TCG unless KVM is explicitly passed through). Dependencies include conditional Python 3.10 packages and install with `--require-hashes`. Missing prerequisites are errors.

Then run real WASM and VM checks:

```bash
vyoma_builder env XDG_CACHE_HOME=/tmp/wasmtime-cache \
  WASMTIME=/work/out/cache/wasmtime bash scripts/test-gui-protocol.sh
bash base/scripts/smoke-test.sh
bash base/scripts/test-e2e-gui.sh
```

The [T0-01 report](validation/2026-09-29-t0-01.md) records known Python API/shared-state failures and the clock guest's unsupported thread/restart loop. Pinning dependencies does not resolve those failures. Preserve exit codes and distinguish passes, failures and skips.

## Updating pins deliberately

1. Update Rust in `rust-toolchain.toml`; update the builder base/snapshot and CA/rustup digest together when needed.
2. Obtain download digests from upstream over authenticated HTTPS. Kernel checksum source: [kernel.org SHA-256 list](https://www.kernel.org/pub/linux/kernel/v5.x/sha256sums.asc). Change `base/versions.sh` and review version-specific URLs.
3. Run Cargo's lockfile update explicitly only for intended dependency changes. Commit standalone lockfiles; a normal build must not rewrite them.
4. Resolve Python requirements on supported Python versions, collect artifact SHA-256 values from PyPI release metadata, and reinstall into an empty venv with `--require-hashes`. Do not replace pins with minimum versions.
5. Build a new image with `--no-cache`, run the local suites, record image ID/package inventory/commit/artifact hashes, then open a focused PR. Never substitute a cached successful binary for a failing rebuild.
