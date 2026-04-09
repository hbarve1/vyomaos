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

# ── kernel source tracking ────────────────────────────────────────────────────
# Track files that should trigger a kernel rebuild when changed.
# KERNEL_PATCHES is empty until patches are added — wildcard returns "" safely.
KERNEL_PATCHES := $(wildcard base/patches/kernel/*.patch)
KERNEL_DEPS    := $(KERNEL_SCRIPT) $(KERNEL_CONFIG) $(KERNEL_PATCHES)

# Stamp file records the last successful kernel build.
# Lives in out/ so `make clean` removes it automatically.
KERNEL_STAMP := $(OUT)/.kernel.stamp

# ── phony declarations ────────────────────────────────────────────────────────
.PHONY: kernel rootfs build run clean

# ── kernel ────────────────────────────────────────────────────────────────────
kernel: $(KERNEL_STAMP)

$(KERNEL_STAMP): $(KERNEL_DEPS)
	@mkdir -p $(OUT)
	bash $(KERNEL_SCRIPT)
	@touch $(KERNEL_STAMP)

# Fail loudly if kernel.sh didn't produce bzImage.
$(BZIMAGE): $(KERNEL_STAMP)
	@test -f $(BZIMAGE) || { echo "ERROR: $(BZIMAGE) not produced by kernel.sh"; exit 1; }

# ── rootfs ────────────────────────────────────────────────────────────────────
rootfs: $(INITRAMFS)

$(INITRAMFS): $(ROOTFS_SCRIPT)
	@mkdir -p $(OUT)
	bash $(ROOTFS_SCRIPT)

# ── build (both) ──────────────────────────────────────────────────────────────
build: kernel rootfs

# ── run ───────────────────────────────────────────────────────────────────────
# KVM is opt-in: `make run KVM=-enable-kvm`
KVM ?=
run: build
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=ttyS0 panic=1" \
	  -nographic \
	  -m 512M \
	  -no-reboot \
	  $(KVM)

# ── clean ─────────────────────────────────────────────────────────────────────
clean:
	rm -rf $(OUT)
