# VyomaOS root Makefile — Docker-based build
#
# All compilation runs inside the vyomaos-builder container.
# The host only needs: docker, make, qemu-system-x86_64
#
# Usage:
#   make image    — build the Docker builder image
#   make kernel   — compile the Linux kernel  -> out/bzImage
#   make rootfs   — build the initramfs       -> out/initramfs.cpio.gz
#   make build    — kernel + rootfs
#   make run      — boot in QEMU (host)
#   make shell    — open a shell in the builder container
#   make clean    — remove out/
#   make clean-image — remove the builder Docker image

SHELL       := /bin/bash
OUT         := out
BZIMAGE     := $(OUT)/bzImage
INITRAMFS   := $(OUT)/initramfs.cpio.gz

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
.PHONY: image kernel supervisor apps rootfs build run shell clean clean-image

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
	  --release
	@touch $(SUPERVISOR_STAMP)

# ── apps (WASM) ───────────────────────────────────────────────────────────────
apps: $(APPS_STAMP)

$(APPS_STAMP): $(APPS_SRC) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) cargo build --manifest-path apps/hello-world/Cargo.toml --release
	$(DOCKER_RUN) cargo build --manifest-path apps/calculator/Cargo.toml  --release
	$(DOCKER_RUN) cargo build --manifest-path apps/factorial/Cargo.toml   --release
	@touch $(APPS_STAMP)

# ── rootfs ────────────────────────────────────────────────────────────────────
rootfs: $(INITRAMFS)

$(INITRAMFS): $(ROOTFS_SCRIPT) $(SUPERVISOR_STAMP) $(APPS_STAMP) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) bash $(ROOTFS_SCRIPT)

# ── build (all) ───────────────────────────────────────────────────────────────
build: kernel supervisor apps rootfs

# ── run (host QEMU — VMs can't nest easily in containers) ────────────────────
run: $(BZIMAGE) $(INITRAMFS)
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=ttyS0 panic=1" \
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
