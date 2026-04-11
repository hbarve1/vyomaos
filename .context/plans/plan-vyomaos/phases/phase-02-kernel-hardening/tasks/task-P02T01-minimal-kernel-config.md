# P02T01 — minimal-kernel-config

## Phase

Phase 02 — Kernel Hardening

## Goal

Create `base/kernel.config` — a minimal Linux kernel configuration derived from `tinyconfig` that contains exactly the options needed to boot in QEMU x86_64 with a serial console and an initramfs, and nothing more.

## File to create / modify

```
base/kernel.config
```

## Implementation

Start from `make tinyconfig` (the absolute minimum that produces a bootable kernel) and layer on the options below. The file is a complete `.config` fragment — copy it over the generated `tinyconfig` output or feed it to `make KCONFIG_ALLCONFIG=base/kernel.config tinyconfig` to merge.

```
# ── Architecture ──────────────────────────────────────────────────────────────
CONFIG_64BIT=y
CONFIG_X86_64=y

# ── ELF / executable support ──────────────────────────────────────────────────
CONFIG_BINFMT_ELF=y
CONFIG_BINFMT_SCRIPT=y

# ── Initramfs ─────────────────────────────────────────────────────────────────
CONFIG_BLK_DEV_INITRD=y
# Path left empty — kernel.sh passes the initramfs via -initrd QEMU flag
# CONFIG_INITRAMFS_SOURCE=""

# ── Core virtual filesystems ──────────────────────────────────────────────────
CONFIG_TMPFS=y
CONFIG_PROC_FS=y
CONFIG_SYSFS=y
CONFIG_DEVTMPFS=y
CONFIG_DEVTMPFS_MOUNT=y

# ── Serial console (8250 UART — standard QEMU ISA serial) ────────────────────
CONFIG_TTY=y
CONFIG_SERIAL_8250=y
CONFIG_SERIAL_8250_CONSOLE=y
# Only one legacy port needed for QEMU
CONFIG_SERIAL_8250_NR_UARTS=4
CONFIG_SERIAL_8250_RUNTIME_UARTS=4

# ── Console / printk ─────────────────────────────────────────────────────────
CONFIG_PRINTK=y
CONFIG_EARLY_PRINTK=y

# ── Clock source for x86 ─────────────────────────────────────────────────────
CONFIG_HZ_100=y

# ── PCI bus (required for virtio-pci added in P02T02) ────────────────────────
CONFIG_PCI=y
CONFIG_PCI_MSI=y

# ── Block layer (required for root device even with initramfs) ───────────────
CONFIG_BLOCK=y

# ── Disable loadable modules (reduces attack surface; all drivers built-in) ──
# CONFIG_MODULES is not set

# ── Networking (base layer; virtio-net added in P02T02) ──────────────────────
CONFIG_NET=y
CONFIG_INET=y
CONFIG_PACKET=y
CONFIG_UNIX=y

# ── Process management ───────────────────────────────────────────────────────
CONFIG_MULTIUSER=y
CONFIG_FUTEX=y
CONFIG_EPOLL=y
CONFIG_SIGNALFD=y
CONFIG_TIMERFD=y
CONFIG_EVENTFD=y

# ── File system basics ───────────────────────────────────────────────────────
CONFIG_EXT4_FS=y
# CONFIG_EXT4_USE_FOR_EXT2 is not set

# ── Disable swap (embedded: no swap) ─────────────────────────────────────────
# CONFIG_SWAP is not set

# ── Security: disable unnecessary features ───────────────────────────────────
# CONFIG_SECCOMP is not set          # added back in P08T02
# CONFIG_NAMESPACES is not set       # added back in P08T03
# CONFIG_AUDIT is not set
# CONFIG_IKCONFIG is not set
```

### How to apply during the kernel build

Inside `base/modules/kernel.sh`, after downloading and extracting the kernel tarball:

```sh
# Merge our config fragment on top of tinyconfig
make -C "$LINUX_SRC" O="$BUILD_DIR" KCONFIG_ALLCONFIG="$REPO_ROOT/base/kernel.config" tinyconfig
# Then build
make -C "$LINUX_SRC" O="$BUILD_DIR" -j"$(nproc)" bzImage
cp "$BUILD_DIR/arch/x86/boot/bzImage" "$REPO_ROOT/out/bzImage"
```

## Notes

- `CONFIG_MODULES=n` (not set) is a hard security boundary — no kernel modules can be loaded at runtime. All required drivers must be compiled in.
- `CONFIG_DEVTMPFS_MOUNT=y` causes the kernel to auto-mount devtmpfs at `/dev` before `init` is exec'd, so the supervisor does not need to mount `/dev` itself on pre-devtmpfs kernels.
- `CONFIG_EARLY_PRINTK=y` enables output before the full console subsystem initialises — essential for debugging boot failures.
- `CONFIG_PCI=y` is included here even though virtio options are in P02T02, because the kernel config is a single file and PCI must precede virtio in config ordering.
- `CONFIG_EXT4_FS=y` is a conservative inclusion; if only tmpfs/initramfs is used it can be removed to shrink the kernel image further.
- The `# CONFIG_X is not set` syntax is the canonical kconfig way to explicitly disable an option; simply omitting the line leaves the option at its `tinyconfig` default, which may differ.

## Verification

```sh
# 1. File exists
test -f base/kernel.config

# 2. All mandatory options are present and set to =y
for opt in CONFIG_64BIT CONFIG_X86_64 CONFIG_SERIAL_8250 \
           CONFIG_SERIAL_8250_CONSOLE CONFIG_BLK_DEV_INITRD \
           CONFIG_TMPFS CONFIG_PROC_FS CONFIG_SYSFS \
           CONFIG_DEVTMPFS CONFIG_PCI CONFIG_NET; do
  grep -q "^${opt}=y" base/kernel.config || \
    { echo "MISSING: $opt"; exit 1; }
done

# 3. Modules are explicitly disabled
grep -q "^# CONFIG_MODULES is not set" base/kernel.config

# 4. Validate the config can be used as KCONFIG_ALLCONFIG input
#    (requires a kernel source tree at $LINUX_SRC)
# LINUX_SRC=/tmp/linux-src  # set to actual path
# make -C "$LINUX_SRC" KCONFIG_ALLCONFIG=base/kernel.config tinyconfig
# grep -q "CONFIG_SERIAL_8250_CONSOLE=y" "$LINUX_SRC"/.config

# 5. Kernel builds and bzImage is produced
# make kernel
# test -f out/bzImage
# file out/bzImage | grep -q "Linux kernel"

# 6. QEMU boots to init prompt (integration test)
# make run  &
# sleep 15
# kill %1
```
