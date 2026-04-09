# VyomaOS root Makefile — Docker-based build
#
# All compilation runs inside the vyomaos-builder container.
# The host only needs: docker, make, qemu-system-x86_64
#
# Usage:
#   make image    — build the Docker builder image
#   make kernel   — compile the Linux kernel  -> out/bzImage
#   make rootfs   — build the initramfs       -> out/initramfs.cpio.gz
#   make disk     — create blank ext4 data disk -> out/disk.img (64 MB)
#   make build    — kernel + rootfs + disk
#   make run      — boot in QEMU (host) with virtio-blk data disk
#   make shell    — open a shell in the builder container
#   make clean    — remove out/
#   make clean-image — remove the builder Docker image

SHELL       := /bin/bash
OUT         := out
BZIMAGE     := $(OUT)/bzImage
INITRAMFS   := $(OUT)/initramfs.cpio.gz
DISK        := $(OUT)/disk.img
DISK_SIZE   := 64  # MB
DATA_DIR    := data

IMAGE       := vyomaos-builder
IMAGE_TAG   := latest
DOCKERFILE  := docker/Dockerfile

KERNEL_SCRIPT    := base/modules/kernel.sh
ROOTFS_SCRIPT    := base/modules/rootfs.sh
KERNEL_CONFIG    := base/kernel.config
SUPERVISOR_SRC   := $(shell find supervisor/src -name '*.rs' 2>/dev/null)
SUPERVISOR_BIN   := supervisor/target/x86_64-unknown-linux-musl/release/supervisor
SUPERVISOR_STAMP := $(OUT)/.supervisor.stamp

APPS_SRC         := $(shell find apps -name '*.rs' -o -name 'Cargo.toml' 2>/dev/null)
APPS_STAMP       := $(OUT)/.apps.stamp

# ── kernel source tracking ────────────────────────────────────────────────────
KERNEL_PATCHES := $(wildcard base/patches/kernel/*.patch)
KERNEL_DEPS    := $(KERNEL_SCRIPT) $(KERNEL_CONFIG) $(KERNEL_PATCHES)
KERNEL_STAMP   := $(OUT)/.kernel.stamp

# ── Docker run helper ─────────────────────────────────────────────────────────
# Mounts the project root read-write at /work inside the container.
# UID/GID passthrough ensures out/ files are owned by the host user.
DOCKER_RUN := docker run --rm \
	--platform linux/amd64 \
	-v "$(CURDIR)":/work \
	-w /work \
	-u $$(id -u):$$(id -g) \
	$(IMAGE):$(IMAGE_TAG)

# ── KVM passthrough (opt-in for run target) ───────────────────────────────────
KVM ?=

# ── phony declarations ────────────────────────────────────────────────────────
.PHONY: image kernel supervisor apps rootfs disk build run shell clean clean-image data

# ── Docker image ──────────────────────────────────────────────────────────────
image: $(DOCKERFILE)
	docker build --platform linux/amd64 -t $(IMAGE):$(IMAGE_TAG) -f $(DOCKERFILE) .

# ── kernel ────────────────────────────────────────────────────────────────────
kernel: $(KERNEL_STAMP)

$(KERNEL_STAMP): $(KERNEL_DEPS) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) bash $(KERNEL_SCRIPT)
	@touch $(KERNEL_STAMP)

$(BZIMAGE): $(KERNEL_STAMP)
	@test -f $(BZIMAGE) || { echo "ERROR: $(BZIMAGE) not produced by kernel.sh"; exit 1; }

# ── supervisor ───────────────────────────────────────────────────────────────
supervisor: $(SUPERVISOR_STAMP)

$(SUPERVISOR_STAMP): supervisor/Cargo.toml supervisor/.cargo/config.toml $(SUPERVISOR_SRC) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) cargo build \
	  --manifest-path supervisor/Cargo.toml \
	  --target x86_64-unknown-linux-musl \
	  --release
	@touch $(SUPERVISOR_STAMP)

# ── apps (WASM) ───────────────────────────────────────────────────────────────
apps: $(APPS_STAMP)

$(APPS_STAMP): $(APPS_SRC) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) cargo build --manifest-path apps/hello-world/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/calculator/Cargo.toml     --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/factorial/Cargo.toml      --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/ping/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/pong/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/storage-demo/Cargo.toml   --target wasm32-wasip2 --release
	@touch $(APPS_STAMP)

# ── rootfs ────────────────────────────────────────────────────────────────────
rootfs: $(INITRAMFS)

$(INITRAMFS): $(ROOTFS_SCRIPT) $(SUPERVISOR_STAMP) $(APPS_STAMP) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) bash $(ROOTFS_SCRIPT)

# ── data disk (ext4, 64 MB) ───────────────────────────────────────────────────
# Created inside the builder so mkfs.ext4 is available.
# The disk is NOT recreated if it already exists (data is preserved across runs).
disk: $(DISK)

$(DISK): | image
	@mkdir -p $(OUT)
	@if [ -f $(DISK) ]; then \
	  echo "ℹ️  disk.img already exists, skipping (delete manually to recreate)"; \
	else \
	  echo "ℹ️  Creating $(DISK_SIZE)M ext4 data disk..."; \
	  $(DOCKER_RUN) bash -c \
	    "dd if=/dev/zero of=/work/$(DISK) bs=1M count=$(DISK_SIZE) status=none && \
	     mkfs.ext4 -q -L vyoma-data /work/$(DISK) && \
	     echo '✅ disk.img created'"; \
	fi

# ── host data directory (9P share — persistent across reboots) ───────────────
# The VM mounts this directory at /data via virtio-9p (trans=virtio).
# Files written by WASM apps to /data persist on the host across VM reboots.
data:
	@mkdir -p $(DATA_DIR)
	@echo "ℹ️  Host data directory ready: $(DATA_DIR)/"

# ── build (all) ───────────────────────────────────────────────────────────────
build: kernel supervisor apps rootfs disk data

# ── run (host QEMU — VMs can't nest easily in containers) ────────────────────
run: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=ttyS0 panic=1" \
	  -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
	  -nographic \
	  -m 512M \
	  -no-reboot \
	  $(KVM)

# ── shell ─────────────────────────────────────────────────────────────────────
shell: | image
	docker run --rm -it \
	  --platform linux/amd64 \
	  -v "$(CURDIR)":/work \
	  -w /work \
	  $(IMAGE):$(IMAGE_TAG) \
	  /bin/bash

# ── clean ─────────────────────────────────────────────────────────────────────
clean:
	rm -rf $(OUT)

clean-image:
	docker rmi $(IMAGE):$(IMAGE_TAG) 2>/dev/null || true
