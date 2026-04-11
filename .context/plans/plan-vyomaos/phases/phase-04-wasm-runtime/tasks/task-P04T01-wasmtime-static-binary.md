# P04T01 — wasmtime-static-binary

## Phase

Phase 04 — WASM Runtime

## Goal

Update `base/modules/rootfs.sh` to download the official Wasmtime pre-built static binary for `x86_64-linux-musl` from the Bytecode Alliance GitHub releases, verify its SHA-256, strip debug symbols to minimise initramfs size, and install it to `/usr/bin/wasmtime` in the rootfs tree.

## File to create / modify

```
base/modules/rootfs.sh
```

## Implementation

Add the following block to `base/modules/rootfs.sh`, after the BusyBox installation section (P02T03) and before the CPIO packaging step.

```sh
# ── Wasmtime static binary ────────────────────────────────────────────────────
# Pin to a specific release for reproducibility.
# Bump WASMTIME_VERSION and WASMTIME_SHA256 together when upgrading.
# Latest release: https://github.com/bytecodealliance/wasmtime/releases
WASMTIME_VERSION="28.0.0"
WASMTIME_TARBALL="wasmtime-v${WASMTIME_VERSION}-x86_64-linux-musl.tar.xz"
WASMTIME_URL="https://github.com/bytecodealliance/wasmtime/releases/download/v${WASMTIME_VERSION}/${WASMTIME_TARBALL}"
# Update this checksum when bumping WASMTIME_VERSION.
# Obtain via: wget -qO- "${WASMTIME_URL}.sha256" or sha256sum the downloaded file.
WASMTIME_SHA256="REPLACE_WITH_ACTUAL_SHA256_FOR_v${WASMTIME_VERSION}"

WASMTIME_CACHE="$OUT/cache/${WASMTIME_TARBALL}"

echo "[rootfs] Installing Wasmtime v${WASMTIME_VERSION}"

# Download (with caching)
if [[ ! -f "$WASMTIME_CACHE" ]]; then
  mkdir -p "$OUT/cache"
  echo "[rootfs] Downloading: $WASMTIME_URL"
  wget --quiet --show-progress -O "${WASMTIME_CACHE}.tmp" "$WASMTIME_URL"
  mv "${WASMTIME_CACHE}.tmp" "$WASMTIME_CACHE"
else
  echo "[rootfs] Using cached: $WASMTIME_CACHE"
fi

# Verify SHA-256
echo "[rootfs] Verifying SHA-256 for wasmtime tarball"
ACTUAL_SHA256="$(sha256sum "$WASMTIME_CACHE" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$WASMTIME_SHA256" ]]; then
  echo "ERROR: SHA-256 mismatch for wasmtime tarball"
  echo "  Expected: $WASMTIME_SHA256"
  echo "  Actual:   $ACTUAL_SHA256"
  rm -f "$WASMTIME_CACHE"
  exit 1
fi
echo "[rootfs] SHA-256 OK"

# Extract the wasmtime binary from the tarball
# The tarball layout is: wasmtime-v{VERSION}-x86_64-linux-musl/wasmtime
WASMTIME_EXTRACT_DIR="$OUT/cache/wasmtime-extract-${WASMTIME_VERSION}"
rm -rf "$WASMTIME_EXTRACT_DIR"
mkdir -p "$WASMTIME_EXTRACT_DIR"
tar -xf "$WASMTIME_CACHE" -C "$WASMTIME_EXTRACT_DIR" --strip-components=1

# The extracted binary name is just "wasmtime"
WASMTIME_BIN="$WASMTIME_EXTRACT_DIR/wasmtime"
test -f "$WASMTIME_BIN" || {
  echo "ERROR: wasmtime binary not found after extraction"
  ls -la "$WASMTIME_EXTRACT_DIR"
  exit 1
}

# Strip debug symbols to reduce rootfs size
# The musl-static release binaries include DWARF info; stripping saves ~10 MB.
echo "[rootfs] Stripping debug symbols from wasmtime"
strip --strip-all "$WASMTIME_BIN" || true  # non-fatal: strip may not be available

# Install into rootfs
mkdir -p "$ROOTFS/usr/bin"
install -m 0755 "$WASMTIME_BIN" "$ROOTFS/usr/bin/wasmtime"

WASMTIME_SIZE="$(du -sh "$ROOTFS/usr/bin/wasmtime" | cut -f1)"
echo "[rootfs] Wasmtime installed at /usr/bin/wasmtime (${WASMTIME_SIZE})"
```

### Obtaining the correct SHA-256

After setting `WASMTIME_VERSION`, run once without the check to download, then:

```sh
sha256sum out/cache/wasmtime-v28.0.0-x86_64-linux-musl.tar.xz
```

Paste the output hash into `WASMTIME_SHA256`.

Alternatively, GitHub publishes a `.sha256` sidecar file alongside each release asset — fetch that and compare:

```sh
wget -qO- "https://github.com/bytecodealliance/wasmtime/releases/download/v28.0.0/wasmtime-v28.0.0-x86_64-linux-musl.tar.xz.sha256"
```

## Notes

- **Target: `x86_64-linux-musl`** — this is the fully statically linked variant. Do not use `x86_64-linux` (glibc), which would require `ld-linux-x86-64.so.2` inside the initramfs.
- **v28+** is specified because WASI Preview 2 (`wasm32-wasip2`) support is stable from Wasmtime 14+, but v28 is the earliest release with stable component model linking needed for Phase 06.
- The tarball is cached in `out/cache/` so repeated `make rootfs` calls skip the network fetch. `make clean` removes the cache.
- `strip --strip-all` is non-fatal (`|| true`) because on macOS hosts the system `strip` does not understand Linux ELF binaries. The Docker build environment has the correct `strip` from `binutils` which is always present in Ubuntu 22.04.
- The unstripped wasmtime binary is typically 40–60 MB; stripped it drops to 25–35 MB. For an initramfs loaded entirely into RAM, minimising size is important.
- In Phase 08, wasmtime should be further constrained with seccomp filters and namespace isolation before executing untrusted WASM modules.

## Verification

```sh
# 1. Script contains wasmtime download logic
grep -q 'wasmtime' base/modules/rootfs.sh
grep -q 'bytecodealliance' base/modules/rootfs.sh

# 2. SHA-256 verification is present in the script
grep -q 'sha256sum' base/modules/rootfs.sh
grep -q 'SHA-256 mismatch' base/modules/rootfs.sh

# 3. Install path is /usr/bin/wasmtime
grep -q 'usr/bin/wasmtime' base/modules/rootfs.sh

# 4. Build the rootfs (requires network access for first run)
bash base/modules/rootfs.sh

# 5. wasmtime is inside the built initramfs
zcat out/initramfs.cpio.gz | cpio -t --quiet | grep -q 'usr/bin/wasmtime'

# 6. Extracted wasmtime is an ELF binary
zcat out/initramfs.cpio.gz | cpio -i --quiet --to-stdout './usr/bin/wasmtime' \
  > /tmp/wasmtime-check
file /tmp/wasmtime-check | grep -q "ELF 64-bit"

# 7. Binary is statically linked (no dynamic library dependencies)
file /tmp/wasmtime-check | grep -q "statically linked"

# 8. Integration: wasmtime --version works inside the VM
# make run 2>&1 | grep -q "wasmtime"
```
