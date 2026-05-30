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
    pub name:        String,          // "vda"
    pub path:        String,          // "/dev/vda"
    pub serial:      String,          // from /sys/block/vda/device/serial (B4 fix)
    pub size_bytes:  u64,             // /sys/block/vda/size * 512
    pub block_size:  u32,             // /sys/block/vda/queue/logical_block_size
    pub removable:   bool,            // /sys/block/vda/removable == "1"
    pub model:       String,          // /sys/block/vda/device/model or "virtio-blk"
    pub fs_type:     FsType,          // probed via probe_fs_type()
    pub mount_point: Option<String>,  // populated by mount_table after mount
    pub partitions:  Vec<PartitionInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FsType {
    Ext4,
    Fat32,
    ExFat,
    VyomaFS,
    Unknown,
}

impl FsType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FsType::Ext4    => "ext4",
            FsType::Fat32   => "vfat",
            FsType::ExFat   => "exfat",
            FsType::VyomaFS => "vyomafs",
            FsType::Unknown => "unknown",
        }
    }

    pub fn mkfs_binary(&self) -> Option<&'static str> {
        match self {
            FsType::Ext4  => Some("/bin/mkfs.ext4"),
            FsType::Fat32 => Some("/bin/mkfs.vfat"),
            _             => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartitionInfo {
    pub name:       String,          // "vda1"
    pub path:       String,          // "/dev/vda1"
    pub start_lba:  u64,
    pub end_lba:    u64,
    pub size_bytes: u64,
    pub part_type:  u8,              // MBR type byte (0x83 = Linux, 0x0B = FAT32)
    pub fs_type:    FsType,
}
```

### 1.1 Superblock Probe (Pure Rust, No External Binary)

`probe_fs_type` reads the first 4096 bytes of the block device to identify the filesystem by magic bytes:

| Filesystem | Offset   | Magic Bytes                        |
|------------|----------|------------------------------------|
| ext2/3/4   | 0x438    | `0x53 0xEF` (s_magic little-endian)|
| FAT32      | 0x52     | `"FAT32   "` (8-byte ASCII string) |
| FAT16/12   | 0x36     | `"FAT"` prefix                     |
| ExFAT      | 0x03     | `"EXFAT   "` (8-byte ASCII string) |
| GPT        | LBA 1+0  | `"EFI PART"` (8-byte signature)    |
| MBR        | 0x1FE    | `0x55 0xAA` (boot signature)       |

```rust
pub fn probe_fs_type(device_path: &str) -> FsType {
    let mut f = match std::fs::File::open(device_path) {
        Ok(f) => f,
        Err(_) => return FsType::Unknown,
    };
    let mut buf = [0u8; 4096];
    if f.read(&mut buf).is_err() { return FsType::Unknown; }

    // ext4: superblock magic at offset 0x438
    if buf[0x438] == 0x53 && buf[0x439] == 0xEF {
        return FsType::Ext4;
    }
    // FAT32: OEM string at offset 0x52
    if &buf[0x52..0x5A] == b"FAT32   " {
        return FsType::Fat32;
    }
    // FAT16/12: OEM string at offset 0x36
    if buf[0x36..0x39] == *b"FAT" {
        return FsType::Fat32;
    }
    // ExFAT: at offset 3
    if &buf[3..11] == b"EXFAT   " {
        return FsType::ExFat;
    }
    FsType::Unknown
}
```

### 1.2 Boot Disk Identification (B4 Fix)

Boot disk identified by virtio serial number, NOT device node. Makefile sets:
```
-drive file=out/disk.img,if=virtio,format=raw,serial=vyoma-data-disk
```

Discovery reads `/sys/block/<name>/device/serial` and stores in `DiskDevice.serial`. The `ipc_handler` checks `device.serial == "vyoma-data-disk"` before any destructive operation — not `device.path == "/dev/vda"`. This is resilient to QEMU drive reordering across Makefile invocations.

### 1.3 Discovery Loop

```rust
pub fn start_discovery_thread(registry: Arc<DiskRegistry>) {
    std::thread::spawn(move || {
        let mut known: HashSet<String> = HashSet::new();
        loop {
            let current = scan_sys_block();
            let added: Vec<_> = current.difference(&known).cloned().collect();
            let removed: Vec<_> = known.difference(&current).cloned().collect();
            for name in &added   { on_device_attached(&name, &registry); }
            for name in &removed { on_device_detached(&name, &registry); }
            known = current;
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn scan_sys_block() -> HashSet<String> {
    std::fs::read_dir("/sys/block")
        .map(|rd| rd.filter_map(|e| e.ok())
                     .map(|e| e.file_name().to_string_lossy().into_owned())
                     .filter(|n| n.starts_with('v') || n.starts_with('s'))
                     .collect())
        .unwrap_or_default()
}
```

---

## 2. Registry

`ArcSwap<HashMap<String, DiskDevice>>` provides wait-free reads — discovery writes are infrequent, reads happen on every IPC command:

```rust
// supervisor/src/disk/registry.rs  (~60 lines)

use arc_swap::ArcSwap;

pub struct DiskRegistry(pub ArcSwap<HashMap<String, DiskDevice>>);

pub static DISK_REGISTRY: OnceLock<Arc<DiskRegistry>> = OnceLock::new();

impl DiskRegistry {
    pub fn new() -> Self {
        DiskRegistry(ArcSwap::from_pointee(HashMap::new()))
    }

    pub fn upsert(&self, dev: DiskDevice) {
        let mut map = (*self.0.load_full()).clone();
        map.insert(dev.name.clone(), dev);
        self.0.store(Arc::new(map));
    }

    pub fn remove(&self, name: &str) {
        let mut map = (*self.0.load_full()).clone();
        map.remove(name);
        self.0.store(Arc::new(map));
    }

    pub fn snapshot(&self) -> Arc<HashMap<String, DiskDevice>> {
        self.0.load_full()
    }
}
```

---

## 3. Space Monitoring

`statvfs(2)` is called on demand (never cached — stale usage data is misleading in a formatting UI).

```rust
// supervisor/src/disk/usage.rs  (~70 lines)

use libc::{statvfs as c_statvfs, statvfs as StatvfsBuf};

#[derive(Debug, Clone)]
pub struct DiskUsage {
    pub total_bytes: u64,
    pub free_bytes:  u64,
    pub used_bytes:  u64,
    pub inodes_total: u64,
    pub inodes_free:  u64,
}

pub fn query_usage(mount_point: &str) -> Option<DiskUsage> {
    let path = std::ffi::CString::new(mount_point).ok()?;
    let mut buf: StatvfsBuf = unsafe { std::mem::zeroed() };
    let rc = unsafe { c_statvfs(path.as_ptr(), &mut buf) };
    if rc != 0 { return None; }
    let block = buf.f_frsize as u64;
    Some(DiskUsage {
        total_bytes:  buf.f_blocks * block,
        free_bytes:   buf.f_bfree  * block,
        used_bytes:   (buf.f_blocks - buf.f_bfree) * block,
        inodes_total: buf.f_files,
        inodes_free:  buf.f_ffree,
    })
}
```

`query_usage` is called by `handle_disk_line` when processing `VYOMA_DISK:get_usage:<mount_point>`. The result is JSON-serialised and sent back as `VYOMA_DISK_REPLY:usage:<json>`.

---

## 4. Mount Management

Only supervisor calls `mount(2)`. Mount points are created under `/media/<sanitized_device_name>/`:
- Allowed characters: `[a-z0-9_-]`
- Max 16 characters
- No `/` or null bytes
- Collisions get a numeric suffix (`vda1`, `vda1_2`, …)

```rust
// supervisor/src/disk/mount_table.rs  (~120 lines)

#[derive(Debug, Clone)]
pub struct MountEntry {
    pub device_path: String,
    pub mount_point: String,
    pub fs_type:     FsType,
    pub mounted_by:  String,   // app_name that requested the mount
    pub read_only:   bool,
}

pub static MOUNT_TABLE: OnceLock<Arc<Mutex<Vec<MountEntry>>>> = OnceLock::new();

pub fn do_mount(device_path: &str, fs: &FsType, read_only: bool) -> Result<String, String> {
    // 1. verify block device exists
    let meta = std::fs::metadata(device_path)
        .map_err(|e| format!("stat failed: {e}"))?;
    if !meta.file_type().is_block_device() {
        return Err(format!("{device_path} is not a block device"));
    }
    // 2. derive and create mount point
    let mp = allocate_mount_point(device_path)?;
    std::fs::create_dir_all(&mp).map_err(|e| e.to_string())?;
    // 3. call mount(2)
    let flags: libc::c_ulong = if read_only { libc::MS_RDONLY } else { 0 };
    let dev_c  = std::ffi::CString::new(device_path).unwrap();
    let mp_c   = std::ffi::CString::new(mp.as_str()).unwrap();
    let fs_c   = std::ffi::CString::new(fs.as_str()).unwrap();
    let rc = unsafe {
        libc::mount(dev_c.as_ptr(), mp_c.as_ptr(), fs_c.as_ptr(), flags, std::ptr::null())
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        let _ = std::fs::remove_dir(&mp);
        return Err(format!("mount failed: {err}"));
    }
    Ok(mp)
}

pub fn do_unmount(mount_point: &str) -> Result<(), String> {
    let mp_c = std::ffi::CString::new(mount_point).unwrap();
    let rc = unsafe { libc::umount2(mp_c.as_ptr(), libc::MNT_DETACH) };
    if rc != 0 {
        return Err(format!("umount failed: {}", std::io::Error::last_os_error()));
    }
    // remove directory if empty (best-effort)
    let _ = std::fs::remove_dir(mount_point);
    Ok(())
}
```

### 4.1 Lock Ordering (B5 Fix)

Disk handlers snapshot state then release disk locks before calling `send_reply`. Never hold `MOUNT_TABLE` or `PENDING_FORMATS` lock while calling `inbox.lock()`:

```rust
// Every handle_disk_line command follows this pattern:
let snapshot = {
    MOUNT_TABLE.get().unwrap().lock().unwrap().clone()
};  // lock released here

let reply = compute_reply(&snapshot, ...);  // no locks held

send_reply(sender, &reply, inbox);  // acquires inbox lock only
```

This eliminates the ABBA deadlock described in B5: previously, `handle_disk_line` was holding `MOUNT_TABLE` while enqueuing a reply that required `inbox`, while the scheduler thread held `inbox` while calling `handle_disk_line`.

---

## 5. Protocol Routing (B1 Fix)

`VYOMA_DISK:` lines are routed in `router.rs` — NOT in `draw_cmd.rs`. This keeps `draw_cmd.rs` under 500 lines and gives `ipc_handler` access to all required global handles:

```rust
// supervisor/src/router.rs — added before VYOMA_DRAW: check
if let Some(cmd) = line.strip_prefix("VYOMA_DISK:") {
    disk::ipc_handler::handle_disk_line(
        cmd,
        sender,
        &DISK_REGISTRY,
        &MOUNT_TABLE,
        &PENDING_FORMATS,
        inbox,
    );
    return;
}
```

**Capability gate**: `disk_manage: bool` in the `Capabilities` struct (`manifest.rs`). All `VYOMA_DISK:` commands are rejected with `VYOMA_DISK_REPLY:error:EPERM:disk_manage required` if the capability is absent.

---

## 6. IPC Protocol Table

### 6.1 App → Supervisor (stdout verbs)

| Verb                                          | Description                                      |
|-----------------------------------------------|--------------------------------------------------|
| `VYOMA_DISK:list`                             | Return JSON array of all discovered DiskDevice   |
| `VYOMA_DISK:info:<device_path>`               | Return JSON for single device including usage    |
| `VYOMA_DISK:get_usage:<mount_point>`          | Return DiskUsage struct as JSON                  |
| `VYOMA_DISK:mount:<device_path>`              | Mount device read-write, auto-detect fs          |
| `VYOMA_DISK:mount:<device_path>:ro`           | Mount device read-only                           |
| `VYOMA_DISK:unmount:<mount_point>`            | Unmount by mount point path                      |
| `VYOMA_DISK:eject:<device_path>`             | Unmount all partitions then signal removal-safe  |
| `VYOMA_DISK:format:<device_path>:<fstype>`   | Begin two-leg format flow; receive token         |
| `VYOMA_DISK:format_confirm:<token>`          | Confirm format after user dialog                 |
| `VYOMA_DISK:mkpart:<device_path>:<mbr\|gpt>` | Write fresh partition table (one partition)      |
| `VYOMA_DISK:rmpart:<device_path>:<part>:<tok>` | Zero partition entry (two-leg token flow)      |

### 6.2 Supervisor → App (stdin replies)

| Reply line                                          | Meaning                                          |
|-----------------------------------------------------|--------------------------------------------------|
| `VYOMA_DISK_REPLY:list:<json_array>`                | Full device list                                 |
| `VYOMA_DISK_REPLY:info:<json_object>`               | Single device detail                             |
| `VYOMA_DISK_REPLY:usage:<json_object>`              | DiskUsage fields as JSON                         |
| `VYOMA_DISK_REPLY:mounted:<device_path>:<mp>`       | Successful mount confirmation                    |
| `VYOMA_DISK_REPLY:unmounted:<mount_point>`          | Successful unmount confirmation                  |
| `VYOMA_DISK_REPLY:ejected:<device_path>`            | Eject complete                                   |
| `VYOMA_DISK_REPLY:format_token:<hex>`               | 32-hex-char token for second leg                 |
| `VYOMA_DISK_REPLY:format_progress:<pct>`            | 0–100 percent complete during mkfs               |
| `VYOMA_DISK_REPLY:format_ok:<device_path>`          | Format succeeded                                 |
| `VYOMA_DISK_REPLY:error:<code>:<human_msg>`         | Operation failed (EPERM, EBUSY, ENOENT, …)       |

### 6.3 Broadcast (supervisor → all disk_manage apps)

| Broadcast line                                      | Trigger                                          |
|-----------------------------------------------------|--------------------------------------------------|
| `VYOMA_SYSTEM:disk_attached:<device_path>`          | New block device detected in `/sys/block/`       |
| `VYOMA_SYSTEM:disk_detached:<device_path>`          | Block device disappeared from `/sys/block/`      |
| `VYOMA_SYSTEM:disk_mounted:<mp>:<device_path>`      | Any mount completed (including auto-mount)       |
| `VYOMA_SYSTEM:disk_unmounted:<mp>`                  | Any unmount completed                            |

---

## 7. Formatting (Two-Leg Token Flow, B3 Fix)

**Token store**: `Mutex<HashMap<[u8;16], PendingFormat>>` — NOT `Vec` (enables atomic remove-and-check):

```rust
// supervisor/src/disk/format.rs  (~140 lines)

#[derive(Debug)]
pub struct PendingFormat {
    pub device:     String,
    pub fstype:     FsType,
    pub token:      [u8; 16],
    pub requester:  String,      // app_name that holds this pending format
    pub expires_at: Instant,     // token valid for 60 seconds
}

pub static PENDING_FORMATS: OnceLock<Arc<Mutex<HashMap<[u8;16], PendingFormat>>>>
    = OnceLock::new();
```

**Token generation** uses `/dev/urandom` (16 bytes):
```rust
fn generate_token() -> [u8; 16] {
    let mut token = [0u8; 16];
    let mut f = std::fs::File::open("/dev/urandom").expect("urandom");
    f.read_exact(&mut token).expect("read urandom");
    token
}
```

**Format flow**:
1. App sends `VYOMA_DISK:format:/dev/vdb:ext4`.
2. Supervisor verifies: device exists as a block device, not currently mounted, serial != `"vyoma-data-disk"`, one-in-flight limit per requester app (B3 fix).
3. Generates 128-bit random token, stores `PendingFormat` with `expires_at = Instant::now() + Duration::from_secs(60)`, replies `VYOMA_DISK_REPLY:format_token:<32-hex-chars>`.
4. App shows user confirmation dialog (Disk Utility: device name + size in red, user must type device name to unlock "Erase" button).
5. App sends `VYOMA_DISK:format_confirm:<token>`.
6. Supervisor atomically removes: `let entry = store.lock().remove(&token)` (B3 fix). Verifies `entry.requester == sender`, `Instant::now() < entry.expires_at`. Spawns format on worker thread.
7. Worker thread reports progress via `VYOMA_DISK_REPLY:format_progress:<pct>` broadcast to requester, emits `VYOMA_DISK_REPLY:format_ok:<device>` on success.

**One-in-flight limit (B3 fix)**: Before issuing a new token:
```rust
let already_pending = store.lock().values()
    .any(|p| p.requester == app_name && Instant::now() < p.expires_at);
if already_pending {
    send_reply(sender, "VYOMA_DISK_REPLY:error:EBUSY:format already pending", inbox);
    return;
}
```

**Actual formatting** verifies `meta.file_type().is_block_device()` then invokes mkfs:

```rust
fn run_format(device: &str, fstype: &FsType) -> Result<(), String> {
    let binary = fstype.mkfs_binary()
        .ok_or_else(|| format!("no mkfs for {}", fstype.as_str()))?;
    let status = std::process::Command::new(binary)
        .arg("-F")        // force (for ext4)
        .arg(device)
        .status()
        .map_err(|e| format!("spawn {binary}: {e}"))?;
    if status.success() { Ok(()) }
    else { Err(format!("{binary} exited with {status}")) }
}
```

**mkfs binaries in initramfs (B2 fix)** — added to `base/rootfs.sh`:
```bash
cp "$(which mkfs.ext4)" "$ROOTFS/bin/mkfs.ext4"
cp "$(which mkfs.vfat)"  "$ROOTFS/bin/mkfs.vfat"
```
Docker builder image ensures `e2fsprogs` and `dosfstools` are installed. Statically linked musl builds are preferred; `build.rs` verifies binaries exist at image build time. Pure-Rust fallback for `mcu-minimal` profile (no external binary dependency).

---

## 8. Partition Management

**Scope for R46**: Single-partition volumes only (full multi-partition GPT editor deferred to R47). Supported operations:

- `VYOMA_DISK:mkpart:<device>:mbr` — write MBR with one partition covering 100% of the disk (type 0x83 for ext4, 0x0B for FAT32). Pure Rust 512-byte sector write.
- `VYOMA_DISK:mkpart:<device>:gpt` — write GPT protective MBR + GPT header at LBA 1 + single partition entry. Pure Rust.
- `VYOMA_DISK:rmpart:<device>:<partname>:<token>` — zero partition table sector (same two-leg token flow as format, protects against accidental wipes).

```rust
// supervisor/src/disk/partition.rs  (~200 lines)

/// Write a single-partition MBR to `device_path`.
/// Partition type 0x83 (Linux ext4) or 0x0B (FAT32).
pub fn write_mbr(device_path: &str, fs_hint: &FsType) -> Result<(), String> {
    let mut sector = [0u8; 512];
    // Boot signature
    sector[510] = 0x55;
    sector[511] = 0xAA;
    // Partition entry at offset 446 (first of four 16-byte entries)
    let part_type: u8 = match fs_hint {
        FsType::Fat32 => 0x0B,
        _             => 0x83,  // Linux
    };
    let entry = &mut sector[446..462];
    entry[4] = part_type;        // partition type
    // LBA start = 2048 (1 MiB alignment), end = total_sectors - 1
    let size_sectors = disk_size_sectors(device_path)?;
    let lba_start: u32 = 2048;
    let lba_size:  u32 = (size_sectors - lba_start as u64) as u32;
    entry[8..12].copy_from_slice(&lba_start.to_le_bytes());
    entry[12..16].copy_from_slice(&lba_size.to_le_bytes());

    let mut f = std::fs::OpenOptions::new()
        .write(true).open(device_path)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    f.write_all(&sector).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}
```

All partition writes use the atomic pattern: write to a `512-byte` temp buffer, `seek(0)`, `write_all`, `sync_all` — no intermediate visible state.

---

## 9. Disk Arbitration (Auto-Mount Policy)

**OFF by default** — no untrusted device is auto-mounted without explicit configuration.

Opt-in via `boot.toml`:
```toml
[disk]
auto_mount              = ["vdb", "sdb"]
auto_mount_point_prefix = "/media"
auto_mount_read_only    = false
```

```rust
// supervisor/src/disk/auto_mount.rs  (~80 lines)

pub fn maybe_auto_mount(dev: &DiskDevice, config: &DiskConfig) {
    if !config.auto_mount.contains(&dev.name) { return; }
    if dev.serial == "vyoma-data-disk"        { return; }  // never auto-mount boot disk
    let read_only = config.auto_mount_read_only;
    match do_mount(&dev.path, &dev.fs_type, read_only) {
        Ok(mp) => {
            register_mount(&dev.path, &mp, &dev.fs_type, "auto", read_only);
            broadcast_disk_mounted(&mp, &dev.path);
        }
        Err(e) => eprintln!("[disk] auto_mount {}: {e}", dev.path),
    }
}
```

On device detach, supervisor unmounts all entries for that device path (best-effort `umount2` with `MNT_DETACH`) and broadcasts `VYOMA_SYSTEM:disk_detached:<device_path>` to all `disk_manage` apps.

---

## 10. Persistent Disk Config

Runtime disk metadata (per-device labels, auto-mount preferences, format history) is stored under `/data/.vyoma/disk/`. The data disk is always mounted at `/data` before any config is read.

```
/data/.vyoma/disk/
├── config.toml          # DiskConfig deserialized by DiskSubsystem::init()
├── labels.toml          # user-assigned labels keyed by serial number
└── format_log.jsonl     # append-only line-delimited format event log
```

**config.toml schema**:
```toml
[disk]
auto_mount              = []
auto_mount_point_prefix = "/media"
auto_mount_read_only    = false
```

**Atomic config write** follows the standard VyomaOS pattern:
```rust
fn persist_config(config: &DiskConfig) -> Result<(), String> {
    let path     = "/data/.vyoma/disk/config.toml";
    let tmp_path = "/data/.vyoma/disk/config.toml.tmp";
    let content  = toml::to_string(config).map_err(|e| e.to_string())?;
    std::fs::write(tmp_path, &content).map_err(|e| e.to_string())?;
    std::fs::rename(tmp_path, path).map_err(|e| e.to_string())?;
    Ok(())
}
```

**Format event log** (`format_log.jsonl`) records every confirmed format as a JSON object per line:
```json
{"ts":"2026-05-30T14:22:01Z","device":"/dev/vdb","serial":"usb-001","fstype":"ext4","by":"disk-utility"}
```

---

## 11. Disk Utility WASM App

```toml
# apps/disk-utility/vyoma.toml
[app]
name    = "disk-utility"
version = "0.1.0"
wasm    = "disk-utility.wasm"

[capabilities]
stdio       = true
display     = true
mouse       = true
disk_manage = true
```

Three-panel layout rendered via `VYOMA_DRAW:`:

- **Left sidebar** (200px): disk list with device name, model, size, removable icon; refreshes every 3 seconds via `VYOMA_DISK:list`
- **Center detail pane** (560px): selected disk properties — model, serial, size, block size, fs_type, mount point, partition table. Shows `DiskUsage` bar when mounted.
- **Action bar** (bottom 60px): Mount / Unmount / Erase… / Partition… buttons. "Erase…" opens a modal.

**Erase modal flow**:
1. Dropdown selects target filesystem (Ext4 / FAT32).
2. Warning text: "This will destroy all data on `<device_name>` (`<size_human>`)."
3. Text input: user must type the device name exactly to enable "Erase" button.
4. On click: send `VYOMA_DISK:format:<path>:<fstype>`, wait for `format_token` reply.
5. Send `VYOMA_DISK:format_confirm:<token>`, show progress bar from `format_progress` replies.
6. On `format_ok`: refresh device list, close modal.

```
apps/disk-utility/src/
├── main.rs     (~160 lines: event loop, stdin reader, VYOMA_DISK reply dispatch)
├── ui.rs       (~300 lines: VYOMA_DRAW calls, three-panel layout, modals)
├── state.rs    (~180 lines: DiskUtilState, JSON list parsing, selection)
└── actions.rs  (~120 lines: send_mount, send_format, two-leg erase confirmation)
```

---

## 12. Blocking Issue Resolution Summary

### B1: VYOMA_DISK routing in draw_cmd.rs exceeds 500-line limit

**Problem**: Adding disk command parsing to `draw_cmd.rs` would push it past the 500-line file limit and deny access to `DISK_REGISTRY` and `PENDING_FORMATS` which are not in scope there.

**Resolution**: Route `VYOMA_DISK:` in `router.rs` before the `VYOMA_DRAW:` check. `router.rs` has access to all global statics and invokes `disk::ipc_handler::handle_disk_line(...)` directly. **RESOLVED.**

```rust
// supervisor/src/router.rs
if let Some(cmd) = line.strip_prefix("VYOMA_DISK:") {
    disk::ipc_handler::handle_disk_line(
        cmd, sender, &DISK_REGISTRY, &MOUNT_TABLE, &PENDING_FORMATS, inbox
    );
    return;
}
```

---

### B2: mkfs.ext4 / mkfs.vfat absent from initramfs

**Problem**: The rootfs build script did not include `mkfs.ext4` or `mkfs.vfat`. Formatting operations would fail with `spawn /bin/mkfs.ext4: No such file or directory` at runtime.

**Resolution**: Added explicit copy directives to `base/rootfs.sh`:
```bash
cp "$(which mkfs.ext4)" "$ROOTFS/bin/mkfs.ext4"
cp "$(which mkfs.vfat)"  "$ROOTFS/bin/mkfs.vfat"
```
The Dockerfile installs `e2fsprogs` and `dosfstools` to guarantee the binaries exist in the builder image. A `build.rs` check verifies presence at image build time. For the `mcu-minimal` platform profile, a pure-Rust minimal ext4 superblock writer is used instead. **RESOLVED.**

---

### B3: Vec-based token store allows concurrent format attacks

**Problem**: Using `Vec<PendingFormat>` allowed an app to issue multiple simultaneous format requests, and token lookup/removal was not atomic — a TOCTOU window existed between checking and removing the token.

**Resolution**: Token store is `Mutex<HashMap<[u8;16], PendingFormat>>`. Removal is atomic via `store.lock().remove(&token)` — there is no window between check and remove. One-in-flight enforcement checks `store.values().any(|p| p.requester == app_name)` under the same lock. Tokens expire after 60 seconds. **RESOLVED.**

---

### B4: Boot disk identified by /dev/vda device node

**Problem**: Makefile QEMU invocation does not guarantee drive letter assignment order. If a new disk is added before `disk.img`, the boot data disk could appear as `/dev/vdb`, and the guard `device.path == "/dev/vda"` would fail to protect it.

**Resolution**: Makefile passes `serial=vyoma-data-disk` in the `-drive` flag for `out/disk.img`. Discovery reads `/sys/block/<name>/device/serial`. `ipc_handler` checks `device.serial == "vyoma-data-disk"` for all destructive operations (format, mkpart, rmpart, eject). Device node path is never used as an identity anchor. **RESOLVED.**

```rust
fn is_boot_disk(dev: &DiskDevice) -> bool {
    dev.serial.trim() == "vyoma-data-disk"
}
```

---

### B5: ABBA deadlock between MOUNT_TABLE and inbox

**Problem**: `handle_disk_line` held `MOUNT_TABLE` lock while calling `send_reply`, which requires `inbox.lock()`. The scheduler thread held `inbox` while dispatching stdout lines into `handle_disk_line`, which then tried to acquire `MOUNT_TABLE`. Classic ABBA deadlock under concurrent requests.

**Resolution**: All disk handlers take a snapshot of shared state, immediately release the lock, compute the reply with no locks held, then call `send_reply` (which acquires only `inbox`):

```rust
// Pattern enforced in every handle_disk_line branch:
let snapshot = {
    MOUNT_TABLE.get().unwrap().lock().unwrap().clone()
};  // MOUNT_TABLE lock released here — DO NOT hold across send_reply

let reply = build_reply(&snapshot);   // pure computation, no locks

send_reply(sender, &reply, inbox);    // acquires inbox lock only
```

**RESOLVED.**

---

## 13. File Layout

```
supervisor/src/disk/
├── mod.rs          (~40 lines: pub re-exports, DiskSubsystem::init, start_discovery_thread)
├── discovery.rs    (~180 lines: scan_sys_block, DiskDevice, FsType, PartitionInfo,
│                                probe_fs_type, read_serial, on_device_attached/detached)
├── registry.rs     (~60 lines: ArcSwap<HashMap> DiskRegistry, DISK_REGISTRY static)
├── usage.rs        (~70 lines: DiskUsage, query_usage via statvfs(2))
├── mount_table.rs  (~120 lines: MountEntry, MOUNT_TABLE, do_mount, do_unmount,
│                                allocate_mount_point, register_mount, lock-safe snapshot)
├── format.rs       (~140 lines: PendingFormat, PENDING_FORMATS HashMap (B3),
│                                generate_token, run_format, one-flight check, expiry)
├── partition.rs    (~200 lines: write_mbr, write_gpt, disk_size_sectors,
│                                do_mkpart, do_rmpart, zero_partition_entry)
├── auto_mount.rs   (~80 lines: DiskConfig, maybe_auto_mount, broadcast_disk_mounted,
│                                on_device_detached cleanup)
└── ipc_handler.rs  (~220 lines: handle_disk_line, capability check, dispatch to
                                 discovery/mount/format/partition/usage handlers,
                                 JSON serialisation of replies)
```

**Total supervisor disk subsystem**: ~1 110 lines across 9 files, all under the 500-line-per-file limit.

**WASM app**:
```
apps/disk-utility/src/
├── main.rs     (~160 lines)
├── ui.rs       (~300 lines)
├── state.rs    (~180 lines)
└── actions.rs  (~120 lines)
```

---

## 14. Integration Checklist

- [ ] `manifest.rs`: add `disk_manage: bool` field to `Capabilities` struct
- [ ] `router.rs`: add `VYOMA_DISK:` prefix check before draw commands
- [ ] `main.rs`: call `DiskSubsystem::init()` during supervisor startup after data disk is mounted
- [ ] `boot.toml`: add `[disk]` section to the rootfs template in `base/rootfs.sh`
- [ ] `base/rootfs.sh`: copy `mkfs.ext4` and `mkfs.vfat` into initramfs (B2 fix)
- [ ] `docker/Dockerfile`: add `RUN apt-get install -y e2fsprogs dosfstools` (B2 fix)
- [ ] `Makefile`: add `serial=vyoma-data-disk` to virtio-blk drive flag (B4 fix)
- [ ] `apps/disk-utility/`: create four-file Rust WASM app with `disk_manage = true`
- [ ] Unit tests: `supervisor/tests/disk_*.rs` covering token flow, mount lifecycle, serial guard
