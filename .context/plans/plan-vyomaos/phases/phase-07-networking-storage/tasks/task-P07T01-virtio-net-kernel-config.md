# P07T01 — virtio-net-kernel-config

## Phase

Phase 07 — Networking & Storage

## Goal

Extend `base/kernel.config` with the kernel configuration options required to bring up a functional TCP/IP network stack over a QEMU virtio-net NIC, enabling WASM apps in VyomaOS to make and accept network connections.

## File to create / modify

```
base/kernel.config
```

## Implementation

Add the following configuration block to `base/kernel.config`. If conflicting entries (e.g. `# CONFIG_NET is not set`) already exist, replace them.

### Networking stack options to add / set

```
# ── Core networking ─────────────────────────────────────────────────────────
CONFIG_NET=y
CONFIG_PACKET=y
CONFIG_UNIX=y
CONFIG_INET=y
CONFIG_IP_MULTICAST=y

# ── IPv4 boot-time configuration (DHCP / kernel command-line IP=) ───────────
CONFIG_IP_PNP=y
CONFIG_IP_PNP_DHCP=y
CONFIG_IP_PNP_BOOTP=y

# ── TCP and UDP ──────────────────────────────────────────────────────────────
CONFIG_TCP_CONG_CUBIC=y
CONFIG_DEFAULT_TCP_CONG="cubic"

# ── Netfilter (disabled — not needed for a minimal server) ──────────────────
# CONFIG_NETFILTER is not set

# ── virtio network driver ────────────────────────────────────────────────────
CONFIG_VIRTIO=y
CONFIG_VIRTIO_PCI=y
CONFIG_VIRTIO_NET=y

# ── Network device support ───────────────────────────────────────────────────
CONFIG_NETDEVICES=y
CONFIG_NET_CORE=y
CONFIG_ETHERNET=y

# ── Needed by virtio-net ─────────────────────────────────────────────────────
CONFIG_PCI=y
CONFIG_VIRTIO_BALLOON=n
```

### Updated QEMU launch flags (in `base/modules/qemu.sh`)

The kernel alone is not enough; QEMU must be told to expose a virtio-net device. Add these flags to the `qemu-system-x86_64` invocation in `base/modules/qemu.sh`:

```sh
-netdev user,id=net0,hostfwd=tcp::8080-:8080 \
-device virtio-net-pci,netdev=net0           \
```

`hostfwd=tcp::8080-:8080` forwards host port 8080 to guest port 8080, which is the port the `server` app (P07T02) listens on.

### Updated kernel command line (in `base/modules/qemu.sh`)

Append `ip=dhcp` to the kernel `append` line so the kernel's built-in DHCP client (enabled by `CONFIG_IP_PNP_DHCP=y`) automatically configures the `eth0` interface at boot:

```sh
-append "console=ttyS0 loglevel=3 ip=dhcp"
```

### Full diff of the affected section in `base/modules/qemu.sh`

```sh
qemu-system-x86_64 \
    -kernel "$KERNEL_FILE" \
    -initrd "$INITRAMFS_FILE" \
    -cpu qemu64 \
    -m "$memory" \
    -accel tcg \
    -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
    -device virtio-net-pci,netdev=net0           \
    -append "console=ttyS0 loglevel=3 ip=dhcp"  \
    -nographic
```

## Notes

- `CONFIG_IP_PNP=y` enables the kernel's built-in PnP IP configuration; without it the `ip=dhcp` kernel parameter is silently ignored and `eth0` stays unconfigured.
- `CONFIG_VIRTIO_PCI=y` is the bus driver for all virtio-pci devices (net, blk, etc.); it must be enabled before either `CONFIG_VIRTIO_NET` or `CONFIG_VIRTIO_BLK` (added in P07T03).
- `CONFIG_PCI=y` is the underlying PCI bus layer; omitting it causes a build error when `CONFIG_VIRTIO_PCI=y` is set.
- `CONFIG_PACKET=y` allows raw packet sockets (used by `ip` / `ifconfig` utilities in busybox), which is useful for debugging inside the VM.
- `CONFIG_UNIX=y` enables Unix domain sockets; needed by some WASI socket implementations and by in-process IPC.
- Netfilter/iptables (`CONFIG_NETFILTER`) is explicitly disabled to keep the kernel image small. Re-enable it in a future phase if firewall rules are needed.
- `CONFIG_IP_MULTICAST=y` is a low-cost addition that avoids missing-symbol build warnings from other NET subsystems.
- After changing `kernel.config`, a full kernel rebuild is required: `./vyomaos.sh build` or `make kernel` depending on which phase's Makefile is in use.
- Cross-reference: P07T02 uses the virtio-net interface to expose a TCP server from a WASM app. P07T03 adds `CONFIG_VIRTIO_BLK` for the block storage driver.

## Verification

```sh
# 1. Confirm the networking config keys are present in kernel.config
for key in CONFIG_NET CONFIG_INET CONFIG_VIRTIO_NET CONFIG_IP_PNP CONFIG_VIRTIO_PCI CONFIG_PCI; do
  grep -q "^${key}=y" base/kernel.config \
    && echo "${key}: set to y" \
    || echo "MISSING: ${key}"
done

# 2. Confirm no conflicting "is not set" lines remain for these options
for key in CONFIG_NET CONFIG_INET CONFIG_VIRTIO_NET; do
  ! grep -q "# ${key} is not set" base/kernel.config \
    && echo "${key}: no conflicting disable" \
    || echo "CONFLICT: ${key} is disabled"
done

# 3. Rebuild the kernel and confirm it includes the virtio_net module
./vyomaos.sh build 2>&1 | grep -E "(virtio_net|VIRTIO_NET|error:)" | head -20

# 4. Confirm the kernel image was produced after rebuild
test -f base/output/bzImage && echo "kernel image: present"

# 5. Confirm qemu.sh contains the virtio-net-pci device line
grep -q "virtio-net-pci" base/modules/qemu.sh && echo "qemu.sh virtio-net device: present"
grep -q "hostfwd"        base/modules/qemu.sh && echo "qemu.sh port forwarding: present"
grep -q "ip=dhcp"        base/modules/qemu.sh && echo "qemu.sh ip=dhcp: present"

# 6. Boot VyomaOS in QEMU and verify eth0 is configured
# (Run this manually; automated boot testing requires background QEMU + serial capture)
# Expected kernel log lines:
#   virtio_net virtio0 eth0: renamed from ...
#   IP-Config: Got DHCP answer from ...
#   IP-Config: Complete: device=eth0, address=10.0.2.15, ...
#
# Quick smoke test: boot, wait 10s, send an HTTP request to the forwarded port
# (only works after P07T02's server app is deployed):
# ./vyomaos.sh run &
# sleep 10
# curl -s --max-time 3 http://localhost:8080 | grep -q "VyomaOS" && echo "network: OK"
```
