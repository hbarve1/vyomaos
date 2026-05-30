# FINAL Spec: Disk Management & Formatting (Round 46)

**Subsystem**: Disk Management & Formatting  
**macOS Analogue**: `Disk Utility` / `DiskArbitration` / `diskutil`  
**Depends on**: R04 (VFS), R21 (window manager), R41 (file_access_grants), R43 (coordinator)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Disk Discovery

Poll `/sys/block/` every 2 seconds (no udevd — QEMU virtio-blk devices appear there at boot). Discovery thread diffs current vs. previous set to detect attach/detach.

```rust
// supervisor/src/disk/discovery.rs  (~180 lines)

#[derive(Debug, Clone, PartialEq)]
pub struct DiskDevice {
    pub name:       String,          // "vda"
    pub path:       String,          // "/dev/vda"
    pub serial:     String,          // from /sys/block/vda/device/serial (B4 fix)
    pub size_bytes: u64,             // /sys/block/vda/size * 512
    pub removable:  bool,            // /sys/block/vda/removable == "1"
    pub model:      String,          // /sys/block/vda/device/model or "virtio-blk"
    pub fs_type:    Option<String>,  // probed via probe_fs_type()
    pub partitions: Vec<PartitionInfo>,
}
```

**Superblock probe** (pure Rust, no external binary):
- ext2/3/4: bytes 56–57 = `0x53 0xEF`
- vfat: bytes 54–57 = `"FAT"` or bytes 82–85 = `"FAT32   "`
- GPT: LBA 1 bytes 0–7 = `"EFI PART"`
- MBR: bytes 510–511 = `0x55 0xAA`

**Boot disk identification (B4 fix)**: Boot disk identified by virtio serial number, NOT device node. Makefile sets `-drive file=out/disk.img,...,serial=vyoma-data-disk`. Discovery reads `/sys/block/<name>/device/serial` and stores in `DiskDevice.serial`. `ipc_handler` checks `device.serial == "vyoma-data-disk"` for protection — not `device.path == "/dev/vda"`.

**Registry**: `ArcSwap<HashMap<String, DiskDevice>>` for wait-free reads:
```rust
// supervisor/src/disk/registry.rs  (~60 lines)
pub struct DiskRegistry(pub ArcSwap<HashMap<String, DiskDevice>>);
pub static DISK_REGISTRY: OnceLock<DiskRegistry> = OnceLock::new();
```

---

## 2. Protocol Routing (B1 Fix)

`VYOMA_DISK:` lines are routed in `router.rs` — NOT in `draw_cmd.rs`. This keeps `draw_cmd.rs` under 500 lines and provides access to all global handles needed by `ipc_handler`:

```rust
// supervisor/src/router.rs — added before VYOMA_DRAW: check
if let Some(cmd) = line.strip_prefix("VYOMA_DISK:") {
    disk::ipc_handler::handle_disk_line(
        cmd, sender, &DISK_REGISTRY, &MOUNT_TABLE, &PENDING_FORMATS, inbox
    );
    return;
}
```

**Capability gate**: `disk_manage: bool` in `Capabilities` struct (manifest.rs). All `VYOMA_DISK:` commands rejected with `VYOMA_DISK_REPLY:error:EPERM:disk_manage required` if cap absent.

---

## 3. Protocol Lines

App stdout → supervisor:
```
VYOMA_DISK:list
VYOMA_DISK:mount:<device_path>
VYOMA_DISK:mount:<device_path>:ro
VYOMA_DISK:unmount:<mount_point>
VYOMA_DISK:info:<device_path>
VYOMA_DISK:format:<device_path>:<fstype>:<token>
VYOMA_DISK:format_confirm:<token>
VYOMA_DISK:mkpart:<device_path>:<mbr|gpt>
VYOMA_DISK:rmpart:<device_path>:<partname>:<token>
```

Supervisor → app stdin:
```
VYOMA_DISK_REPLY:list:<json_array>
VYOMA_DISK_REPLY:mounted:<device_path>:<mount_point>
VYOMA_DISK_REPLY:unmounted:<mount_point>
VYOMA_DISK_REPLY:format_token:<token>
VYOMA_DISK_REPLY:format_ok:<device_path>
VYOMA_DISK_REPLY:error:<code>:<human_msg>
```

---

## 4. Mount Management

Only supervisor calls `mount(2)`. Mount points created under `/media/<sanitized_device_name>/` (only `[a-z0-9]`, max 16 chars, no `/`):

```rust
// supervisor/src/disk/mount_table.rs  (~120 lines)
pub struct MountEntry {
    pub device_path: String,
    pub mount_point: String,
    pub fs_type:     String,
    pub mounted_by:  String,
    pub read_only:   bool,
}
pub static MOUNT_TABLE: OnceLock<Mutex<Vec<MountEntry>>> = OnceLock::new();
```

Unmount: `umount2(path, MNT_DETACH)` via libc; directory removed if empty.

**Lock ordering (B5 fix)**: Disk handlers snapshot state then release disk locks before calling `send_reply`. Never hold `MOUNT_TABLE` or `PENDING_FORMATS` lock while calling `inbox.lock()`:

```rust
// Every handle_disk_line command follows this pattern:
let snapshot = { mount_table.lock().unwrap().clone() };  // take, drop
let reply = compute_reply(&snapshot, ...);                // no locks held
send_reply(sender, &reply, inbox);                        // inbox only
```

---

## 5. Formatting (Two-Leg Token Flow, B3 Fix)

**Token store**: `Mutex<HashMap<[u8;16], PendingFormat>>` — NOT `Vec` (enables atomic remove-and-check):

```rust
// supervisor/src/disk/format.rs  (~140 lines)
pub struct PendingFormat {
    pub device:    String,
    pub fstype:    String,
    pub token:     [u8; 16],
    pub requester: String,     // app_name
    pub expires_at: Instant,
}
```

Flow:
1. App sends `VYOMA_DISK:format:/dev/vdb:ext4`.
2. Supervisor verifies: device not mounted, not the boot serial (`vyoma-data-disk`), one-in-flight limit per app (B3 fix).
3. Issues 128-bit random token, stores `PendingFormat`, replies `VYOMA_DISK_REPLY:format_token:<hex>`.
4. App shows user confirmation dialog (Disk Utility displays device name + size, red "Erase" button requires user to type device name).
5. App sends `VYOMA_DISK:format_confirm:<token>`.
6. Supervisor: `let entry = store.lock().remove(&token)` (atomic remove, B3 fix). Verifies `entry.requester == sender`, token not expired. Runs format on worker thread.

**One-in-flight limit (B3 fix)**: Before issuing a new token, check `store.values().any(|p| p.requester == app_name)` → reject with `VYOMA_DISK_REPLY:error:EBUSY:format already pending`.

**Actual formatting** calls `/bin/mkfs.ext4`, `/bin/mkfs.vfat` after verifying block device (`meta.file_type().is_block_device()`).

**mkfs binaries in initramfs (B2 fix)**: Added to `base/rootfs.sh`:
```bash
cp "$(which mkfs.ext4)" "$ROOTFS/bin/mkfs.ext4"
cp "$(which mkfs.vfat)"  "$ROOTFS/bin/mkfs.vfat"
```
Docker builder image ensures `e2fsprogs` and `dosfstools` are installed. Statically linked musl builds preferred; `build.rs` verifies. Pure-Rust fallback for `mcu-minimal` profile (no external binary dependency).

---

## 6. Partition Management

**Scope for R46**: Single-partition volumes only (full GPT editor deferred). Supported:

- `VYOMA_DISK:mkpart:<device>:mbr` — write MBR with one partition covering 100% (type 0x83 ext4 / 0x0B vfat). Pure Rust 512-byte sector write.
- `VYOMA_DISK:mkpart:<device>:gpt` — write GPT protective MBR + header. Pure Rust.
- `VYOMA_DISK:rmpart:<device>:<partname>:<token>` — zero partition table sector (same two-leg token flow as format).

```rust
// supervisor/src/disk/partition.rs  (~200 lines)
// Pure Rust MBR/GPT: std::fs::OpenOptions::new().write(true).open(device)
// + seek + write 512 bytes. No external binary.
```

---

## 7. Disk Utility WASM App

```toml
[capabilities]
stdio = true; display = true; mouse = true; disk_manage = true
```

Three-panel UI: left sidebar (disk list, 3s refresh), center detail pane (size, fs, mount status), action bar (Mount / Unmount / Erase… / Partition…). Erase flow: modal dialog with format selector + typed-name confirmation before `format_confirm`.

```
apps/disk-utility/src/
├── main.rs     (~160 lines)
├── ui.rs       (~300 lines: VYOMA_DRAW calls, panel layout)
├── state.rs    (~180 lines: DiskUtilState, list reply parsing)
└── actions.rs  (~120 lines: send_mount, send_format, two-leg erase flow)
```

---

## 8. Auto-Mount Policy

**OFF by default** (no untrusted device auto-mounted). Opt-in via `boot.toml`:
```toml
[disk]
auto_mount = ["vdb"]
auto_mount_point_prefix = "/media"
```

On detect: supervisor mounts to `/media/<device_name>`, broadcasts `VYOMA_SYSTEM:disk_mounted:/media/vdb` to all `disk_manage = true` apps.

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Adding VYOMA_DISK: to draw_cmd.rs exceeds 500 lines + missing global handles | Routed in `router.rs` before draw commands; full global access available there |
| B2: mkfs.ext4/vfat binaries absent from initramfs | Added to `rootfs.sh` copy directives; Dockerfile ensures e2fsprogs/dosfstools installed |
| B3: Vec-based token store allows concurrent format attacks | `HashMap<[u8;16], PendingFormat>` + atomic `remove()` + one-in-flight limit per app |
| B4: Boot disk identified by `/dev/vda` node — breaks if QEMU drive order changes | Boot disk identified by virtio serial `"vyoma-data-disk"` (Makefile + sysfs read) |
| B5: ABBA deadlock: MOUNT_TABLE + inbox held simultaneously | Snapshot disk state first, release lock, compute reply, then send_reply (inbox only) |

---

## 10. File Layout

```
supervisor/src/disk/
├── mod.rs          (~40 lines: pub re-exports, DiskSubsystem init)
├── discovery.rs    (~180 lines: /sys/block/ poll, DiskDevice, probe_fs_type, serial check)
├── registry.rs     (~60 lines: ArcSwap DiskRegistry, DISK_REGISTRY static)
├── mount_table.rs  (~120 lines: MountEntry, do_mount, do_unmount, lock-safe pattern (B5))
├── format.rs       (~140 lines: PendingFormat, HashMap token store (B3), do_format, one-flight limit)
├── partition.rs    (~200 lines: pure Rust MBR/GPT write, do_mkpart, do_rmpart)
├── ipc_handler.rs  (~220 lines: handle_disk_line, capability check, command dispatch)
└── auto_mount.rs   (~80 lines: boot.toml config, maybe_auto_mount)
```
