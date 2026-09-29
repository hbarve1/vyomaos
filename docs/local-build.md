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

To build all images, `make build IMAGE_TAG=t0-02` uses the pinned Dockerfile and locked Cargo resolution. Kernel input propagation, cache checks and the R1 image allowlist are described below. For a clean gate use a new worktree; do not reuse another branch's compiled kernel/initramfs. Download archives may be reused only because the scripts recheck their digests. A corrupt cache fails explicitly; remove/replace that file before retrying.

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

## Kernel selection and rebuilds (T0-03)

`make kernel` and `make kernel PLATFORM=desktop-x86` use `base/kernel.config`.
Other platform names fail explicitly: the ARM/MCU profile directories are roadmap
placeholders, not supported cross-compilation targets. The desktop placeholder
config/rootfs files are not used. Override the fragment with
`make kernel KERNEL_CONFIG=path/to/fragment.config`; relative paths resolve from
the repository root. The direct `base/modules/kernel.sh` entry point accepts the
same environment variables. `KERNEL_JOBS` bounds direct-script compile parallelism.

Every invocation checks config path/content, pinned source version/digest,
lexically ordered `base/patches/kernel/*.patch` paths/content, build scripts and
GCC/binutils/Make identities. It also verifies the published image and effective
config digests. Unchanged input/output content reuses the image; switching configs,
editing a file with an old timestamp, or deleting/corrupting output cannot reuse a
stale image. Patches apply with `patch -p1 --batch` to a source tree keyed by source,
patch and toolchain identity; removing a patch returns to an unpatched tree.

The builder serializes concurrent invocations with `flock`, regenerates config
with `allnoconfig`, and publishes `out/bzImage`, `out/kernel.config`,
`out/kernel.inputs` and `out/.kernel.stamp` only after successful compilation.
A failed rebuild removes the acceptance stamp; any previous image is retained for
diagnosis and must not be treated as a successful current build. Source/download
caches live only under `out/`; a corrupt download still fails digest verification.
No bit-for-bit reproducibility claim is made. Inspect the effective config to
check Linux's dependency resolution for a custom fragment.

Run `PYTHONDONTWRITEBYTECODE=1 python3 tests/build/test_kernel.py` for the isolated
build regressions and `bash tests/build/download_test.sh` for download validation.
These fixture tests use the production builder but do not replace a real Linux
compile/config-switch/boot test; see the T0-03 evidence report.

## R1 image payload (T0-04)

`base/r1-apps.txt` is the explicit required app allowlist. `make apps` builds
only those apps, and rootfs assembly installs only those apps even when optional
WASM artifacts remain from earlier development. The initial payload is `shell`.
Desktop, dock, clock and GUI/demo apps are excluded. `make test-gui-protocol`
explicitly builds its optional desktop/dock fixtures without adding them to R1.

`make rootfs` builds the supervisor and selected apps before assembly; direct
`bash base/modules/rootfs.sh` requires those artifacts already built. Every
assembly checks the supervisor is nonempty/executable and each selected WASM
binary and manifest is nonempty. Missing artifacts cause an actionable failure
before downloads/output replacement. Invalid, duplicate or empty allowlists
also fail. Existing initramfs files cannot bypass preflight. Packing failure
preserves the previous image but returns failure; do not treat that old image as
acceptance for the failed invocation.

Boot configuration is generated from the same allowlist with `restart="never"`,
and the list is embedded at `/etc/vyoma/r1-apps.txt`. The old desktop boot template
is retained as historical development input, but is not copied into R1. This
change establishes payload selection only: the current shell still emits the
legacy drawing protocol, the supervisor still initializes display services,
and usable local/serial terminal sessions remain T1 work. Excluding clock also
does not resolve its thread/restart defect or the general T2-07 restart policy.

Run `PYTHONDONTWRITEBYTECODE=1 python3 tests/build/test_rootfs.py` for isolated
assembly regressions. They use local fixture binaries and real tar/cpio/gzip;
real production builds and missing-artifact checks are recorded in the T0-04
validation report.
