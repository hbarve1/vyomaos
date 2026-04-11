# P02T03 — static-musl-busybox

## Phase

Phase 02 — Kernel Hardening

## Goal

Update `base/modules/rootfs.sh` to download the official musl-static BusyBox binary from busybox.net, verify its SHA-256 checksum, install it as `/bin/busybox` in the initramfs tree, and symlink only the minimal set of utilities the OS needs at boot (`sh`, `mount`, `poweroff`, `echo`).

## File to create / modify

```
base/modules/rootfs.sh
```

## Implementation

```sh
#!/usr/bin/env bash
# base/modules/rootfs.sh
# Builds the VyomaOS initramfs (out/initramfs.cpio.gz).
# Depends on: cpio gzip wget sha256sum
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$REPO_ROOT/out"
ROOTFS="$OUT/rootfs"

# ── BusyBox static binary ─────────────────────────────────────────────────────
# Pin to a specific release. Update BUSYBOX_SHA256 when bumping the version.
BUSYBOX_VERSION="1.36.1"
BUSYBOX_URL="https://www.busybox.net/downloads/binaries/${BUSYBOX_VERSION}-x86_64-linux-musl/busybox"
# SHA-256 for busybox-1.36.1 x86_64 musl static binary (verify on busybox.net)
BUSYBOX_SHA256="b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314"

BUSYBOX_CACHE="$OUT/cache/busybox-${BUSYBOX_VERSION}-x86_64-musl"

# ── Helper: download with checksum verification ───────────────────────────────
download_verified() {
  local url="$1" dest="$2" expected_sha256="$3"

  if [[ -f "$dest" ]]; then
    echo "[rootfs] Using cached: $dest"
  else
    mkdir -p "$(dirname "$dest")"
    echo "[rootfs] Downloading: $url"
    wget --quiet --show-progress -O "$dest.tmp" "$url"
    mv "$dest.tmp" "$dest"
  fi

  echo "[rootfs] Verifying SHA-256: $dest"
  local actual_sha256
  actual_sha256="$(sha256sum "$dest" | awk '{print $1}')"
  if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    echo "ERROR: SHA-256 mismatch for $dest"
    echo "  Expected: $expected_sha256"
    echo "  Actual:   $actual_sha256"
    rm -f "$dest"
    exit 1
  fi
  echo "[rootfs] SHA-256 OK"
}

# ── Build initramfs directory tree ────────────────────────────────────────────
echo "[rootfs] Creating rootfs skeleton at $ROOTFS"
rm -rf "$ROOTFS"
mkdir -p \
  "$ROOTFS"/{bin,sbin,usr/bin,usr/sbin,lib,lib64,dev,proc,sys,tmp,run,apps} \
  "$ROOTFS/etc"

# ── Install BusyBox ───────────────────────────────────────────────────────────
download_verified "$BUSYBOX_URL" "$BUSYBOX_CACHE" "$BUSYBOX_SHA256"

install -m 0755 "$BUSYBOX_CACHE" "$ROOTFS/bin/busybox"

# Symlink only the utilities needed at boot.
# Keeping this list minimal reduces the attack surface.
BUSYBOX_APPLETS=(
  sh       # shell — required by /init
  mount    # mount /proc /sys /dev
  poweroff # clean shutdown
  echo     # diagnostic output in init script
  ls       # basic directory listing (debugging aid)
  cat      # read files (debugging aid)
)

for applet in "${BUSYBOX_APPLETS[@]}"; do
  ln -sf /bin/busybox "$ROOTFS/bin/$applet"
done

# ── /init script ─────────────────────────────────────────────────────────────
# Minimal PID-1 that mounts virtual filesystems and execs the supervisor.
# The Rust supervisor (P03T01+) replaces the exec target once built.
cat > "$ROOTFS/init" << 'INIT_EOF'
#!/bin/sh
# VyomaOS /init — PID 1
set -e

/bin/mount -t proc  none /proc
/bin/mount -t sysfs none /sys

echo "VyomaOS booting..."

# Exec supervisor if present, else drop to a shell for debugging
if [ -x /usr/bin/supervisor ]; then
  exec /usr/bin/supervisor
else
  echo "WARNING: supervisor not found, dropping to shell"
  exec /bin/sh
fi
INIT_EOF
chmod 0755 "$ROOTFS/init"

# ── /etc/passwd and /etc/group (minimal — needed by some libc calls) ─────────
printf 'root:x:0:0:root:/root:/bin/sh\n' > "$ROOTFS/etc/passwd"
printf 'root:x:0:\n'                     > "$ROOTFS/etc/group"

# ── Pack the CPIO archive ─────────────────────────────────────────────────────
echo "[rootfs] Packing initramfs -> $OUT/initramfs.cpio.gz"
(
  cd "$ROOTFS"
  find . | cpio --quiet -H newc -o
) | gzip -9 > "$OUT/initramfs.cpio.gz"

echo "[rootfs] Done. Size: $(du -sh "$OUT/initramfs.cpio.gz" | cut -f1)"
```

## Notes

- The SHA-256 is pinned to `busybox-1.36.1`. When upgrading, re-fetch the binary, run `sha256sum` manually, and update `BUSYBOX_SHA256` in this script.
- The download is cached in `out/cache/` so repeat `make rootfs` calls skip the network fetch. `make clean` removes the cache.
- `BUSYBOX_APPLETS` is intentionally short. `sh`, `mount`, `poweroff`, and `echo` are the minimum needed by the `/init` script. `ls` and `cat` are added only as debugging aids; remove them for a production build to shrink the attack surface.
- The `/init` script uses `exec` to hand off to the supervisor so that PID 1 is the supervisor, not a shell. This matters for proper signal handling and zombie reaping (the supervisor will need to handle `SIGCHLD` — see P03T04).
- `cpio --quiet -H newc` produces the `newc` format expected by the Linux kernel's initramfs extractor. Do not use the `odc` (old) format.
- `/etc/passwd` and `/etc/group` are minimal stubs. Some libc functions (`getpwuid`, `getgrgid`) SIGSEGV without them even in a musl environment.

## Verification

```sh
# 1. Script is executable
test -x base/modules/rootfs.sh

# 2. Build the rootfs
bash base/modules/rootfs.sh

# 3. Output archive exists
test -f out/initramfs.cpio.gz

# 4. Archive is non-empty and gzip-valid
gzip -t out/initramfs.cpio.gz

# 5. busybox binary is inside the archive
zcat out/initramfs.cpio.gz | cpio -t --quiet | grep -q "bin/busybox"

# 6. Required symlinks are present
for applet in sh mount poweroff echo; do
  zcat out/initramfs.cpio.gz | cpio -t --quiet | grep -q "bin/${applet}" || \
    { echo "MISSING applet: $applet"; exit 1; }
done

# 7. /init is present and marked executable (mode 0755)
zcat out/initramfs.cpio.gz | cpio -tv --quiet | grep " ./init$" | grep -q "rwxr-xr-x"

# 8. SHA-256 verification runs without error on a fresh download
#    (clear cache first to force re-download)
rm -f out/cache/busybox-*
bash base/modules/rootfs.sh
test -f out/initramfs.cpio.gz

# 9. Kernel boots with the initramfs and prints "VyomaOS booting..."
#    (integration test — requires out/bzImage from P02T01/P02T02)
# make run 2>&1 | grep -q "VyomaOS booting"
```
