# P01T01 — root-makefile

## Phase

Phase 01 — Build Foundation

## Goal

Create a root `Makefile` that exposes five top-level targets (`kernel`, `rootfs`, `build`, `run`, `clean`) and wires them to the existing shell scripts in `base/modules/`, with `out/bzImage` as the tracked artifact for the kernel target.

## File to create / modify

```
Makefile
```

## Implementation

```makefile
# VyomaOS root Makefile
# Usage:
#   make kernel   — build the Linux kernel -> out/bzImage
#   make rootfs   — build the initramfs    -> out/initramfs.cpio.gz
#   make build    — kernel + rootfs
#   make run      — launch QEMU with out/bzImage + out/initramfs.cpio.gz
#   make clean    — remove out/

SHELL       := /bin/bash
OUT         := out
BZIMAGE     := $(OUT)/bzImage
INITRAMFS   := $(OUT)/initramfs.cpio.gz

KERNEL_SCRIPT := base/modules/kernel.sh
ROOTFS_SCRIPT := base/modules/rootfs.sh
KERNEL_CONFIG := base/kernel.config

# ── phony declarations ────────────────────────────────────────────────────────
.PHONY: kernel rootfs build run clean

# ── kernel ────────────────────────────────────────────────────────────────────
# Depends on the build script and the config file; out/bzImage is the real
# target so Make can skip the rebuild when nothing has changed (see P01T02).
kernel: $(BZIMAGE)

$(BZIMAGE): $(KERNEL_SCRIPT) $(KERNEL_CONFIG)
	@mkdir -p $(OUT)
	bash $(KERNEL_SCRIPT)

# ── rootfs ────────────────────────────────────────────────────────────────────
rootfs: $(INITRAMFS)

$(INITRAMFS): $(ROOTFS_SCRIPT)
	@mkdir -p $(OUT)
	bash $(ROOTFS_SCRIPT)

# ── build (both) ──────────────────────────────────────────────────────────────
build: kernel rootfs

# ── run ───────────────────────────────────────────────────────────────────────
run: build
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=ttyS0 panic=1" \
	  -nographic \
	  -m 512M \
	  -no-reboot

# ── clean ─────────────────────────────────────────────────────────────────────
clean:
	rm -rf $(OUT)
```

Key decisions:

- `$(BZIMAGE)` and `$(INITRAMFS)` are file targets (not phony), so Make's built-in timestamp comparison handles incrementality for free.
- `kernel` and `rootfs` are phony aliases that forward to the real file targets; this lets callers write `make kernel` naturally.
- `run` depends on `build` so it always ensures artifacts are up to date before launching QEMU.
- `SHELL := /bin/bash` is set explicitly because the build scripts use bash-isms.

## Notes

- The `base/modules/kernel.sh` script is expected to produce `out/bzImage` as its final artifact. If the script writes to a different path, update `BZIMAGE` accordingly.
- `base/modules/rootfs.sh` must produce `out/initramfs.cpio.gz`. Adjust `INITRAMFS` if the script uses a different name.
- P01T02 extends this Makefile with `$(wildcard)` source-tracking so a change to any `.c` file under the kernel source tree also triggers a rebuild.
- The `run` target uses `-nographic` and `console=ttyS0` — this matches the serial console configuration added in P02T01.
- Do not add `-enable-kvm` here; the CI Docker container may not expose `/dev/kvm`. Add it as an opt-in variable override (`make run KVM=-enable-kvm`) in a follow-up.

## Verification

```sh
# 1. Makefile is present and parseable — dry-run all targets
make -n kernel  2>&1 | grep -q "kernel.sh"
make -n rootfs  2>&1 | grep -q "rootfs.sh"
make -n build   2>&1 | grep -q "kernel.sh"
make -n clean   2>&1 | grep -q "rm -rf"

# 2. .PHONY declaration covers all five public targets
grep -E '^\.PHONY' Makefile | grep -q "kernel"
grep -E '^\.PHONY' Makefile | grep -q "rootfs"
grep -E '^\.PHONY' Makefile | grep -q "build"
grep -E '^\.PHONY' Makefile | grep -q "run"
grep -E '^\.PHONY' Makefile | grep -q "clean"

# 3. out/bzImage is a real file target (not phony)
make -p -f Makefile 2>/dev/null | grep -q "out/bzImage"

# 4. clean target removes the out/ directory
mkdir -p out && touch out/bzImage out/initramfs.cpio.gz
make clean
test ! -d out

# 5. Full build succeeds inside the Docker build environment
#    (run this only when docker/Dockerfile from P01T03 is complete)
# docker build -t vyomaos-builder docker/
# docker run --rm -v "$PWD":/work -w /work vyomaos-builder make build
```
