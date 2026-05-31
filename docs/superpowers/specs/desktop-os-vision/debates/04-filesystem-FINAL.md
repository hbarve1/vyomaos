# Round 4 Final: File System & VFS Layer

**Status:** ✅ Debated & synthesized (2026-05-29)
**Debate:** [Architect: 1576 lines] [Critic: 583 lines] [Final: this]
**Subsystem:** Supervisor-mediated VFS for `wasm32-wasip2` apps under Wasmtime PID-1
**macOS equivalent:** APFS + VFS layer + `NSFileManager` + `NSOpenPanel`/`NSSavePanel` + `NSFileCoordinator` + FSEvents + Powerbox + xattrs + Trash

---

## 0. Executive Summary

The file system is the most user-visible and most security-critical surface in a desktop OS. macOS
achieves a coherent experience by stacking five distinct subsystems on top of a single on-disk format
(APFS): VFS / `NSFileManager` / `NSOpen`+`NSSavePanel` via Powerbox / `NSFileCoordinator` / FSEvents.
VyomaOS must reproduce this on a Linux 5.10 kernel that today exposes a single 9P/virtio mount of a
host directory at `/data`, where all I/O is funneled through a Rust PID-1 supervisor.

The Architect proposed a VFS mediator with pluggable `VfsBackend`s, supervisor-mediated runtime path
rewriting on top of WASI Preview 2, userspace copy-on-write for snapshots on the 9P backend, and full
APFS-equivalent xattrs / coordination / panels / bookmarks. The Critic correctly identified seven
blocking flaws: (C1) 9P RTT puts the system at a ~666 ops/sec ceiling that is 30 % below realistic
desktop demand; (C2) `wasi:filesystem` has no public seam for the proposed runtime interception; (C3)
userspace copy-on-write is grossly storage-inefficient at file granularity and an entire userspace
block layer at block granularity; (C4) runtime path rewriting contradicts WASI Preview 2's
capability-by-descriptor model; (C5) panel-minted bookmarks lack a cryptographic identity binding and
are forgeable via line-oriented IPC; (C6) file coordination has no specified crash semantics and no
cycle detection across IPC + locks; (C7) `inotify` does not fire on 9P mounts at all, and even host-side
polling at 200 ms saturates the 9P channel.

This Final synthesizes both into a buildable specification. The headline reshaping:

- **Storage backend is virtio-blk + ext4 in v1; 9P is demoted to a legacy fallback.** The supervisor
  formats a 4-8 GB ext4 image on a virtio-blk device at first boot and mounts it at `/data`. The
  existing 9P mount, when present, is exposed as `/host` for developer convenience only and is
  *never* the path of an app-facing `wasi:filesystem` preopen. This restores per-op latency to
  50-200 µs and unlocks `inotify`, `fallocate`, hardlinks, sparse files, and atomic rename.
- **Sandbox enforcement is preopened-FD-driven, not runtime path rewriting.** The supervisor's
  `SandboxMap` is a *static input to `WasiCtxBuilder::preopened_dir`* at instance launch. Once
  Wasmtime owns the preopens, the standard `wasmtime-wasi` `wasi:filesystem` host impl handles
  every `read`/`write`/`readdir`/`stat` with no supervisor mediation on the hot path. The
  supervisor only re-enters the picture for the *vyoma extensions*: watchers, coordination,
  bookmarks, panels, xattrs, snapshots, quotas, and trash. Mediation strategy is **(c) hybrid via
  preopen-as-sandbox + custom shim for cross-cutting concerns** per Critic R3, picked
  unambiguously.
- **Quota enforcement uses a per-instance `WriteShim` wrapping the preopened directory's
  `wasi:filesystem/types/descriptor.write`** — a single host-import override that intercepts
  exactly two methods (`write` and `set-size`) and consults `Arc<QuotaLedger>`. Other methods pass
  straight through Wasmtime's stock impl. Total override surface: ~150 lines, not 5000.
- **Snapshots are deferred from v1.** What we ship in v1 is "snapshot-like" semantics built from
  atomic `rename` + hardlink + ext4 reflinks (`FICLONE`). The `vyoma:fs/snapshot` interface exists
  but with a `Unsupported("snapshots-defer-v2")` error from `snapshot.create`. `clone-file` works
  via `FICLONE`. v2 enables btrfs subvolumes when the kernel adds `CONFIG_BTRFS_FS`. This honestly
  scopes Critic C3.
- **Bookmarks are HMAC-SHA-256 over `(app_signing_key, canonical_path, expiry, nonce)` where the
  app signing key is derived per-bundle from a TPM-or-disk-resident supervisor master key.**
  Forgery requires the app's bundle-specific key, which only the supervisor knows. The bookmark
  store is a sled DB at `/data/.vyoma/bookmarks.db` with WAL durability. Critic C5 resolved.
- **File coordination uses a write-ahead log + supervisor crash recovery via `on_instance_gone`
  from Round 1.** Locks are reader-writer, path-keyed, with priority inheritance and cycle
  detection in the Round 3 `WaitForGraph`. On app crash, the supervisor's `on_instance_gone(iid)`
  drains all held locks and cancels all waiters with `IpcError::SenderGone`. On supervisor crash,
  the WAL is replayed: any lock not re-acquired within 5 s of replay is released. Critic C6
  resolved.
- **Watchers go through a `WatcherBackend` trait.** `Ext4InotifyBackend` for the primary `/data`
  mount uses inotify directly. `NinePPollBackend` is a 250 ms tumbling-window poll loop, used
  only for the legacy `/host` mount. `TmpfsInotifyBackend` for `/tmp`. Events are coalesced and
  delivered as `FsEvent` IPC envelopes via the Round 3 sharded router with `OverflowPolicy::Coalesce`.
  Critic C7 resolved.

22 new files. ~6,400 LOC. All under the 500-line ceiling. Per-op p50 budget: `open` 80 µs, `read` 4 KB
2 µs, `write` 4 KB 3 µs, `xattr.get` 4 µs, `panel.open` 80 ms first-frame, `coordinate_write` 2 µs
uncontended.

---

## Key Decisions

1. **Storage backend is virtio-blk + ext4 in v1; 9P is `/host` legacy only.** A single
   `/data/disk.img` (initially 4 GB, online-growable) is mounted via virtio-blk at `/data`. The
   former 9P mount, if configured, appears at `/host` with `read-only` default and is never an
   app-facing preopen target. Restores ~5 000-20 000 ops/sec ceiling and enables inotify, sparse
   files, `FICLONE`. Critic C1 resolved.

2. **Mediation strategy is HYBRID: preopens-as-sandbox + thin write-shim for cross-cutting
   concerns.** Sandbox is enforced by Wasmtime preopens (no runtime path rewriting). Quotas,
   coordination, and snapshots are added by a per-descriptor `WriteShim` resource that wraps the
   stock `wasi:filesystem` descriptor for *write* paths only. Reads are untouched. Critic C2 and
   C4 resolved. Picks option (c) from Critic R3, NOT (a) or (b).

3. **`SandboxMap` drives `WasiCtxBuilder::preopened_dir`, not runtime translation.** At instance
   launch, the supervisor walks the manifest's `[capabilities.filesystem]` map and opens
   real-host directory handles via `cap-std`, then hands them to `WasiCtxBuilder::preopened_dir`.
   The descriptor IS the capability. No path can escape because Wasmtime's resolver is in
   charge. Critic C4 resolved.

4. **`vyoma:fs@0.1.0` WIT package extends, never replaces, `wasi:filesystem`.** Apps use
   `wasi:filesystem` for `open`/`read`/`write`/`readdir`/`stat`. They use `vyoma:fs/*` for
   watchers, coordination, panels, bookmarks, snapshots, xattrs, quotas, and trash. Coexistence is
   clean because each interface targets different host imports.

5. **Snapshots deferred to v2; v1 provides clone (FICLONE) + atomic-rename for "snapshot-like"
   semantics.** `vyoma:fs/snapshot.clone-file` calls ext4 `FICLONE` (O(1) on ext4 since 5.10);
   `snapshot.create` returns `Unsupported("snapshots-defer-v2")`. Apps that want
   point-in-time copies can clone a subtree (1 ms per file). This honestly scopes Critic C3
   without retracting the feature surface.

6. **Bookmarks bind app identity cryptographically via HMAC-SHA-256.** Token =
   `HMAC-SHA-256(app_signing_key, canonical_path || expiry || nonce || mode)`. The
   `app_signing_key` is HKDF-derived from a supervisor master key (sealed in
   `/data/.vyoma/keys/master.key`, mode 0600, supervisor-only). Forging a bookmark requires the
   per-bundle key, which only the supervisor knows. The store is a sled DB with WAL durability.
   Critic C5 resolved.

7. **File coordination uses a WAL-backed lock table with supervisor + app crash recovery.**
   Reader-writer locks keyed by canonical path. On app crash, the Round 1 lifecycle hook
   `on_instance_gone(iid)` drains held locks and cancels waiters. On supervisor crash, the WAL
   at `/data/.vyoma/coord.wal` is replayed at boot; locks marked `held` for instances that no
   longer exist are released. The Round 3 `WaitForGraph` is extended to span both `call` edges
   and lock-acquire edges; cycle detection is synchronous on every acquire. Critic C6 resolved.

8. **Watchers go through a `WatcherBackend` trait.** `Ext4InotifyBackend` is the default for
   `/data` and `/tmp`. `NinePPollBackend` (250 ms tumbling-window stat-poll) is the fallback for
   `/host`. Events are coalesced in a 100 ms tumbling window per `(watch_id, path, kind)` tuple
   before being delivered as `FsEvent` IPC envelopes on the Round 3 `Normal` class with
   `OverflowPolicy::Coalesce`. Critic C7 resolved.

9. **xattrs are first-class via a shadow `XattrStore` (sled).** ext4 supports xattrs natively, so
   most writes go through to the backend. The shadow store is used for the legacy `/host` (9P)
   mount and for `vyoma.*` system xattrs that we want to keep out of user-visible `lsattr`. Three
   namespaces: `vyoma.*` (system-only), `user.*` (any app with access), `com.<bundle>.*`
   (bundle-private).

10. **Quotas are atomic `AtomicU64` per `(bundle_id, mount)` with WAL-backed persistence.** Every
    write checks the counter under a CAS loop; the `WriteShim` rejects with
    `QuotaExceeded` synchronously. Persistence: a delta log is appended on every 64 KB chunk;
    full snapshot every 60 s. Crash recovery walks the bundle's tree once at boot if the WAL is
    inconsistent. Resolves Critic S1.

11. **Case sensitivity follows the backend: ext4 is case-sensitive (Linux native).** No
    supervisor-side Unicode case-fold layer — apps that need case-insensitive matching handle
    it at the app layer. Critic S2 / R8 resolved by acknowledging Linux native semantics. The
    file-picker UI (Powerbox) MAY apply Unicode-aware ordering at the display layer only.

12. **Path resolution cache: per-instance `PathBucket` LRU (4 096 entries).** Caches
    `(logical_path → BackendPath, mount_id)` to amortize NFC + sandbox-check work. Invalidated on
    `ManifestReloaded` (rare). First-resolve cost: ~3-8 µs; cached: ~150-300 ns. Resolves the
    perf concern in Architect Q1.

13. **Panel app runs in supervisor-elevated WASM with `filesystem_full_disk` + chrome cap.**
    Privilege boundary: panel returns canonical paths to the supervisor; the supervisor (NOT the
    panel app) mints the bookmark scoped to the *requesting* app. Compromise of the panel app
    leaks paths the user has navigated to, not arbitrary unrestricted FDs.

14. **Trash is per-bundle at `/data/<bundle>/.Trash/`.** `trash()` is atomic rename + two xattr
    writes (`vyoma.trash.original-path`, `vyoma.trash.timestamp`). Background `TrashSweeper`
    purges entries older than 30 d. `untrash` restores via rename and refuses overwrite. Cross-mount
    trash records original path in xattr.

15. **Boot phase integration: `BootPhase::FilesystemMounted` runs after `Round1::Mounted9P` and
    before `BootPhase::IpcReady`.** `Vfs::init` opens the ext4 backend, replays the coord WAL,
    integrity-checks the sled DBs (xattrs, bookmarks, quota), and exposes `Arc<Vfs>` to the rest
    of the supervisor.

---

## 1. Storage Backend Strategy — Fixed

### 1.1 The pivot away from 9P-as-primary

The Architect's design assumed `/data` was a 9P mount and tried to graft snapshots, inotify,
xattrs, and parallel I/O on top. The Critic showed this is architecturally infeasible:
9P serializes through one virtqueue, has no inotify, and offers no atomic primitives beyond
`rename`. The fix is to **promote virtio-blk + ext4 to the primary backend** and demote 9P to a
legacy developer-mount.

```
┌───────────────────────────────────────────────────────────────┐
│  WASM apps (wasm32-wasip2)                                    │
│   wasi:filesystem/types@0.2.0       vyoma:fs/* (Round 4)     │
└───────────┬───────────────────────────────┬───────────────────┘
            │                               │
            │ stock Wasmtime impl           │ supervisor host-impl
            │ (with WriteShim override)     │ (panels, watchers, ...)
            ▼                               ▼
┌───────────────────────────────────────────────────────────────┐
│ supervisor: VfsBackend layer (this round)                     │
│ ┌──────────────────┐  ┌────────────────┐  ┌─────────────────┐│
│ │ Ext4Backend      │  │ TmpfsBackend   │  │ NinePBackend    ││
│ │ /data (primary)  │  │ /tmp           │  │ /host (legacy)  ││
│ │ virtio-blk image │  │ RAM            │  │ optional        ││
│ └──────────────────┘  └────────────────┘  └─────────────────┘│
└───────────────────────────────────────────────────────────────┘
            ▼
┌───────────────────────────────────────────────────────────────┐
│ Linux 5.10 kernel: VFS + ext4 + tmpfs + 9p (read-only fallback)│
└───────────────────────────────────────────────────────────────┘
            ▼
┌───────────────────────────────────────────────────────────────┐
│  virtio-blk: /data/disk.img (host file, 4 GB initially)        │
│  tmpfs:      RAM                                              │
│  virtio-9p:  /host (optional, RO for dev workflows)           │
└───────────────────────────────────────────────────────────────┘
```

### 1.2 Disk image creation and mount

The current `disk.img` is already an ext4 image; v1 promotes it to be the only persistent backend
visible to apps. The supervisor's `BootPhase::FilesystemMounted` performs:

```rust
// supervisor/src/fs/backends/ext4.rs

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Ext4Backend {
    /// Mount point in the supervisor's namespace, e.g. /data
    mount_point: PathBuf,
    /// cap-std root descriptor; sandbox boundary
    root: Arc<cap_std::fs::Dir>,
    caps: BackendCaps,
}

impl Ext4Backend {
    /// Called from BootPhase::FilesystemMounted.
    pub fn open_or_init(image_path: &Path, mount_point: &Path)
        -> Result<Arc<Self>, VfsError>
    {
        if !image_path.exists() {
            Self::format_image(image_path, 4 * GIB)?;
        }
        Self::mount(image_path, mount_point)?;
        Self::fsck_if_unclean(mount_point)?;
        let root = cap_std::fs::Dir::open_ambient_dir(
            mount_point, cap_std::ambient_authority(),
        ).map_err(|e| VfsError::BackendError(e.to_string()))?;
        Ok(Arc::new(Self {
            mount_point: mount_point.to_path_buf(),
            root: Arc::new(root),
            caps: BackendCaps {
                xattrs: true,
                clones: true,           // FICLONE works on ext4 >=4.5
                snapshots: false,       // deferred to v2
                sparse: true,
                mmap: true,
                inotify: true,
                case_sensitive: true,
                read_only: false,
                max_filename: 255,
                max_path: 4096,
            },
        }))
    }

    fn format_image(path: &Path, size: u64) -> Result<(), VfsError> {
        // 1. truncate to N bytes
        let f = std::fs::File::create(path)?;
        f.set_len(size)?;
        drop(f);
        // 2. mkfs.ext4
        let status = std::process::Command::new("mkfs.ext4")
            .args(&["-F", "-O", "metadata_csum,extent,64bit"])
            .arg(path)
            .status()
            .map_err(|e| VfsError::BackendError(format!("mkfs.ext4 spawn: {e}")))?;
        if !status.success() {
            return Err(VfsError::BackendError("mkfs.ext4 failed".into()));
        }
        Ok(())
    }

    fn mount(image: &Path, mount_point: &Path) -> Result<(), VfsError> {
        std::fs::create_dir_all(mount_point)?;
        // Use mount(2) directly; supervisor runs as PID 1 with full caps
        nix::mount::mount(
            Some(image),
            mount_point,
            Some("ext4"),
            nix::mount::MsFlags::MS_NOATIME | nix::mount::MsFlags::MS_NOSUID,
            None::<&str>,
        ).map_err(|e| VfsError::BackendError(format!("mount: {e}")))?;
        Ok(())
    }

    fn fsck_if_unclean(mount_point: &Path) -> Result<(), VfsError> {
        // tune2fs -l to check the clean bit; e2fsck if dirty.
        // Implementation: ~30 LOC; omitted here for brevity.
        Ok(())
    }
}
```

### 1.3 `VfsBackend` trait — refined

The trait below is what every backend implements. The signature is intentionally narrow because
most operations are routed through `wasi:filesystem` directly to the backend's preopened dir;
the trait is only used by the supervisor for *vyoma:fs* extensions (xattrs, snapshots, watcher
subscription) and for the panel/picker app.

```rust
// supervisor/src/fs/vfs.rs

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BackendPath(PathBuf);

impl BackendPath {
    pub fn new(p: PathBuf) -> Result<Self, VfsError> {
        if !p.is_absolute() { return Err(VfsError::NotAbsolute); }
        if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(VfsError::PathEscape);
        }
        Ok(Self(p))
    }
    pub fn as_path(&self) -> &Path { &self.0 }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub truncate: bool,
    pub append: bool,
    pub sync: bool,
    pub no_follow: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FileKind { Regular, Directory, Symlink, Fifo, Device }

#[derive(Clone, Debug)]
pub struct FileStat {
    pub kind: FileKind,
    pub size: u64,
    pub allocated: u64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub atime_ns: i64,
    pub btime_ns: i64,
    pub mode: u32,
    pub nlink: u32,
    pub inode: u64,
    pub blocks: u64,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct BackendCaps {
    pub xattrs: bool,
    pub clones: bool,
    pub snapshots: bool,
    pub sparse: bool,
    pub mmap: bool,
    pub inotify: bool,
    pub case_sensitive: bool,
    pub read_only: bool,
    pub max_filename: u32,
    pub max_path: u32,
}

#[derive(Debug)]
pub enum VfsError {
    NotFound,
    PermissionDenied,
    QuotaExceeded { used: u64, limit: u64 },
    InUse,
    AlreadyExists,
    NotEmpty,
    InvalidName,
    PathEscape,
    PathTooLong,
    NotAbsolute,
    Unsupported(&'static str),
    BackendError(String),
    SnapshotConflict,
    SenderGone,
    BookmarkInvalid,
    BookmarkExpired,
    CoordinationTimeout,
    CoordinationCycle,
    IoError(std::io::Error),
}

impl From<std::io::Error> for VfsError {
    fn from(e: std::io::Error) -> Self { VfsError::IoError(e) }
}

/// VfsBackend is the supervisor-side abstraction. Apps reach it ONLY for
/// vyoma:fs extensions; raw read/write/open go through Wasmtime's stock
/// wasi:filesystem impl against the preopened cap-std handle.
pub trait VfsBackend: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> BackendCaps;

    /// cap-std root used as the source for WasiCtxBuilder preopens.
    fn root_dir(&self) -> Arc<cap_std::fs::Dir>;

    // Operations the supervisor itself initiates (panel, indexer, watcher).
    fn stat(&self, path: &BackendPath) -> Result<FileStat, VfsError>;
    fn readdir(&self, path: &BackendPath) -> Result<Vec<DirEntry>, VfsError>;
    fn mkdir(&self, path: &BackendPath, mode: u32) -> Result<(), VfsError>;
    fn unlink(&self, path: &BackendPath) -> Result<(), VfsError>;
    fn rmdir(&self, path: &BackendPath) -> Result<(), VfsError>;
    fn rename(&self, from: &BackendPath, to: &BackendPath) -> Result<(), VfsError>;

    // Extensions; default returns Unsupported.
    fn clone_file(&self, _src: &BackendPath, _dst: &BackendPath) -> Result<(), VfsError> {
        Err(VfsError::Unsupported("clone_file"))
    }
    fn xattr_get(&self, _path: &BackendPath, _key: &str) -> Result<Vec<u8>, VfsError> {
        Err(VfsError::Unsupported("xattr_get"))
    }
    fn xattr_set(&self, _path: &BackendPath, _key: &str, _val: &[u8]) -> Result<(), VfsError> {
        Err(VfsError::Unsupported("xattr_set"))
    }
    fn xattr_list(&self, _path: &BackendPath) -> Result<Vec<String>, VfsError> {
        Err(VfsError::Unsupported("xattr_list"))
    }

    /// Subscribe to raw backend events; the WatcherCore translates them.
    fn subscribe_events(&self, _path: &BackendPath, _recursive: bool)
        -> Result<BackendWatchHandle, VfsError>
    {
        Err(VfsError::Unsupported("subscribe_events"))
    }
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub kind: FileKind,
    pub inode: u64,
}

pub struct BackendWatchHandle {
    pub fd: i32,
    pub backend_name: &'static str,
}
```

### 1.4 Backends

| Backend | Mount | Persistence | inotify | xattrs | clones (`FICLONE`) | Notes |
|---------|-------|-------------|---------|--------|--------------------|-------|
| `Ext4Backend` | `/data` | yes (image) | yes | yes | yes | Primary; default for all `[capabilities.filesystem]` |
| `TmpfsBackend` | `/tmp` | no | yes | yes | no | Per-instance `/tmp/<bundle>-<iid>/` |
| `NinePBackend` | `/host` | dev host fs | no | shadow only | no | Optional, RO default |
| `SysRoBackend` | `/sys/fonts`, `/sys/icons`, `/sys/themes`, `/sys/timezones` | yes (initramfs) | n/a | shadow only | no | Read-only assets |

`9P` is **not** the primary backend, never gets `wasi:filesystem` preopens unless an app explicitly
declares `[capabilities.filesystem_host]` (gated behind `legacy_9p_host` boot flag, default off).

### 1.5 9P-mode failure semantics

If the host-side 9P mount is unavailable at boot (developer-mode VM started without
`-virtfs`), the supervisor:

1. Skips mounting `/host`.
2. Logs `degraded=host-fs-unavailable`.
3. The `NinePBackend` registry entry is absent; any app declaring `[capabilities.filesystem_host]`
   fails manifest validation at spawn.

ext4 backend failure is a hard boot failure (no persistent storage at all):

1. `BootPhase::FilesystemMounted` returns `Err`; supervisor enters degraded mode with `/data`
   mounted as tmpfs.
2. All `bookmark.save`, `xattr.set`, `quota` updates are accepted but lost on next boot; flagged
   as `persistent_storage_available = false`.
3. Package manager refuses installs.

---

## 2. WASI P2 Mediation Strategy — Fixed

### 2.1 The strategy: HYBRID (preopens-as-sandbox + WriteShim)

The Architect's design accidentally smuggled in three different mediation strategies (preopens,
custom host impl, runtime path rewriting) without choosing. The Critic forced the choice
explicit. We pick **hybrid**:

- **Sandbox enforcement: pure preopens.** At instance launch, the supervisor walks the manifest's
  `[capabilities.filesystem]` map and opens cap-std `Dir` handles. These handles are added to
  the Wasmtime `WasiCtxBuilder` via `preopened_dir`. The standard `wasmtime-wasi` filesystem
  impl handles every `wasi:filesystem` call. The supervisor does NOT see ordinary
  `read`/`write`/`open`/`readdir`/`stat`/`unlink`/`rename` in the hot path. Wasmtime's
  resolver guarantees no escape because no descriptor outside the preopens is ever in scope.

- **Cross-cutting concerns (quota, coordination) added by `WriteShim`.** The supervisor wraps the
  preopened cap-std dir in a thin descriptor wrapper that overrides only two methods on the
  `wasi:filesystem/types/descriptor` resource: `write` and `set-size`. Reads, opens, stats,
  truncates-via-open-truncate, readdirs, unlinks, renames, etc. pass straight through. The
  override surface is two methods, ~150 lines, NOT a full re-implementation of
  `wasmtime-wasi::filesystem`.

- **Watchers, panels, bookmarks, snapshots, xattrs, coordination, quotas, trash** live entirely
  under `vyoma:fs/*` — a separate WIT package with separate host imports under supervisor control.

### 2.2 Why hybrid wins

Pros over (a) "pure preopens":
- Gains quota enforcement and write-side coordination at minimal cost.

Pros over (b) "full custom host impl":
- 95 % less code (we override 2 methods, not 30+).
- Tracks upstream `wasmtime-wasi` releases for free on the read path.
- Supervisor is not on the read hot path — bypasses the C1 throughput concern entirely.

Pros over runtime path rewriting:
- No TOCTOU between supervisor stat and Wasmtime open.
- Symlink containment is Wasmtime's job; we don't duplicate it.
- Relative paths from held descriptors are handled by Wasmtime; we don't see them.

### 2.3 WasiCtx construction

```rust
// supervisor/src/fs/wasi_shim.rs

use cap_std::fs::Dir;
use std::sync::Arc;
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::DirPerms;
use wasmtime_wasi::FilePerms;

pub struct VfsWasiBuilder {
    actor: ActorRef,
    vfs: Arc<Vfs>,
    quotas: Arc<QuotaLedger>,
    coord: Arc<CoordinationService>,
}

impl VfsWasiBuilder {
    pub fn build(self) -> Result<WasiCtxBuilder, VfsError> {
        let mut b = WasiCtxBuilder::new();
        let sandbox = self.actor.policy_snapshot().fs_sandbox.clone();
        for entry in sandbox.entries.iter() {
            let mount = self.vfs.lookup_mount(&entry.mount_point)
                .ok_or(VfsError::NotFound)?;
            let dir: Arc<Dir> = mount.backend.root_dir();
            // Subpath inside the backend, derived from the sandbox entry.
            let sub = self.vfs.subpath_for_actor(&self.actor, &entry, &mount)?;
            let leaf = dir.open_dir(&sub).map_err(VfsError::IoError)?;
            let (dperm, fperm) = perms_from_access(entry.access);
            b.preopened_dir(leaf, dperm, fperm, &entry.guest_path)
                .map_err(|e| VfsError::BackendError(e.to_string()))?;
        }
        // Hook in the WriteShim. See §2.4.
        Ok(b)
    }
}

fn perms_from_access(a: SandboxAccess) -> (DirPerms, FilePerms) {
    match a {
        SandboxAccess::ReadOnly  => (DirPerms::READ,                    FilePerms::READ),
        SandboxAccess::ReadWrite => (DirPerms::READ | DirPerms::MUTATE, FilePerms::READ | FilePerms::WRITE),
        SandboxAccess::WriteOnly => (DirPerms::MUTATE,                  FilePerms::WRITE),
        SandboxAccess::Append    => (DirPerms::READ | DirPerms::MUTATE, FilePerms::READ | FilePerms::WRITE),
        SandboxAccess::NoAccess  => (DirPerms::empty(),                 FilePerms::empty()),
    }
}
```

The `WasiCtxBuilder::preopened_dir(handle, dir_perms, file_perms, guest_path)` is `wasmtime-wasi`'s
public API. `guest_path` is the path the app sees (e.g. `/data`), `handle` is the cap-std
descriptor anchored at the backend's actual location (e.g. `/data/bundles/com.example.notes/`).

### 2.4 The `WriteShim`

`wasmtime-wasi` exposes a `WasiView` trait. We override the descriptor's `write` and
`set-size` only. The hook works by interposing on the host-impl registration in the `Linker`:

```rust
// supervisor/src/fs/write_shim.rs

use wasmtime::component::{Linker, ResourceTable};
use std::sync::Arc;

pub struct VfsView {
    pub stock: wasmtime_wasi::WasiCtx,
    pub table: ResourceTable,
    pub actor: ActorRef,
    pub quotas: Arc<QuotaLedger>,
    pub coord:  Arc<CoordinationService>,
    pub watcher: Arc<WatcherCore>,
}

impl wasmtime_wasi::WasiView for VfsView {
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx { &mut self.stock }
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
}

/// Adds the WriteShim by re-registering the two methods we want to intercept.
/// Other ~30 methods on Descriptor (read, stat, readdir, unlink, etc.) come
/// from the stock impl.
pub fn add_to_linker(linker: &mut Linker<VfsView>) -> wasmtime::Result<()> {
    // 1. Stock impl: registers everything.
    wasmtime_wasi::add_to_linker_async(linker)?;
    // 2. Override the two methods. Wasmtime allows replacing host impls by
    //    re-`func_wrap_async` on the same interface; the later registration wins.
    let mut iface = linker.instance("wasi:filesystem/types@0.2.0")?;
    iface.func_wrap_async(
        "[method]descriptor.write",
        |mut store, params: (wasmtime::component::Resource<wasmtime_wasi::filesystem::Descriptor>, Vec<u8>, u64)| {
            Box::new(async move {
                let (desc, buf, offset) = params;
                let view: &mut VfsView = store.data_mut();
                let inode = view.stock.descriptor_inode(&desc).await?;
                let bundle = view.actor.bundle_id().clone();
                let mount  = view.stock.descriptor_mount(&desc).await?;
                // 1. Quota preflight (CAS).
                view.quotas.try_reserve(&bundle, &mount, buf.len() as u64)?;
                // 2. Coordination check: is a writer holding a lock on this path?
                view.coord.assert_writable(&view.actor, inode)?;
                // 3. Forward to stock impl.
                let n = view.stock.descriptor_write_through(desc, &buf, offset).await?;
                // 4. Commit quota (delta = n).
                view.quotas.commit_reservation(&bundle, &mount, n);
                // 5. Notify watcher core.
                view.watcher.note_write(inode);
                Ok(n)
            })
        },
    )?;
    iface.func_wrap_async(
        "[method]descriptor.set-size",
        |mut store, params: (wasmtime::component::Resource<wasmtime_wasi::filesystem::Descriptor>, u64)| {
            Box::new(async move {
                let (desc, new_size) = params;
                let view: &mut VfsView = store.data_mut();
                let inode = view.stock.descriptor_inode(&desc).await?;
                let bundle = view.actor.bundle_id().clone();
                let mount  = view.stock.descriptor_mount(&desc).await?;
                let cur    = view.stock.descriptor_size(&desc).await?;
                if new_size > cur {
                    view.quotas.try_reserve(&bundle, &mount, new_size - cur)?;
                }
                view.coord.assert_writable(&view.actor, inode)?;
                view.stock.descriptor_set_size_through(desc, new_size).await?;
                if new_size > cur {
                    view.quotas.commit_reservation(&bundle, &mount, new_size - cur);
                } else if new_size < cur {
                    view.quotas.refund(&bundle, &mount, cur - new_size);
                }
                view.watcher.note_write(inode);
                Ok(())
            })
        },
    )?;
    Ok(())
}
```

Note `descriptor_write_through` / `descriptor_set_size_through` / `descriptor_inode` /
`descriptor_mount` / `descriptor_size` are helpers we add to a thin extension trait over
`wasmtime_wasi::WasiCtx`. They use `cap_std` operations directly on the underlying descriptor
without re-running Wasmtime's access checks (which already succeeded). Their implementation
is ~50 LOC in `wasi_shim.rs`.

### 2.5 Read path is supervisor-cold

Reads go: `app → wasi:filesystem.read → wasmtime-wasi stock impl → cap-std → ext4 → virtio-blk`.
The supervisor is **not** on the hot path. This is how we beat the Critic's C1 budget: most
syscalls don't touch the supervisor. Only writes do, and only for ~150 LOC of overhead per write
(quota CAS + coord-check + watcher notify), all of which are local atomic operations.

Performance projection at 50 apps, mixed workload:
- Pure-read ops: backed by ext4 + page cache, ~5 µs p50.
- Write ops: stock + WriteShim ~6 µs p50 (CAS on `AtomicU64`, hash lookup on `coord` table).
- System-wide ceiling: 200 k+ ops/sec on a 4-core x86 desktop (ext4 + page cache).

This is 300× the 9P ceiling and comfortably absorbs the 860 ops/sec Critic-projected demand.

---

## 3. Sandbox & Path Namespacing — Fixed

### 3.1 `SandboxMap` is preopen-input only

The Architect's `SandboxMap` (§2.1) becomes a **declarative input to preopens at instance
launch**, not a runtime filter. The map is read once when the `WasiCtx` is built. The
descriptor IS the capability.

```rust
// supervisor/src/fs/sandbox.rs

use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct SandboxMap {
    pub entries: Arc<[SandboxEntry]>,
}

#[derive(Clone, Debug)]
pub struct SandboxEntry {
    /// What the app sees: /data, /tmp, /sys/fonts, etc.
    pub guest_path: PathBuf,
    /// What the supervisor opens on the backend's real root.
    /// e.g. for /data with bundle=com.example.notes → "bundles/com.example.notes"
    pub backend_subpath: PathBuf,
    /// Which backend this entry belongs to (by name).
    pub backend_name: String,
    pub access: SandboxAccess,
    /// xattr namespace prefix for app-private xattrs.
    pub xattr_namespace: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SandboxAccess {
    ReadOnly,
    ReadWrite,
    WriteOnly,
    Append,
    NoAccess,
}

/// The supervisor builds this from the manifest on instance launch.
pub struct SandboxBuilder<'a> {
    pub manifest: &'a Manifest,
    pub bundle_id: &'a BundleId,
    pub instance_id: InstanceId,
}

impl<'a> SandboxBuilder<'a> {
    pub fn build(&self) -> Result<SandboxMap, VfsError> {
        let mut entries = Vec::new();
        // 1. Default bundle dir.
        if self.manifest.capabilities.filesystem {
            entries.push(SandboxEntry {
                guest_path:      PathBuf::from(format!("/data/{}", self.bundle_id)),
                backend_subpath: PathBuf::from(format!("bundles/{}", self.bundle_id)),
                backend_name:    "ext4".into(),
                access:          SandboxAccess::ReadWrite,
                xattr_namespace: Some(format!("com.{}", self.bundle_id)),
            });
        }
        // 2. Per-instance tmpfs scratch.
        entries.push(SandboxEntry {
            guest_path:      PathBuf::from(format!("/tmp/{}-{}", self.bundle_id, self.instance_id.0)),
            backend_subpath: PathBuf::from(format!("{}-{}", self.bundle_id, self.instance_id.0)),
            backend_name:    "tmpfs".into(),
            access:          SandboxAccess::ReadWrite,
            xattr_namespace: None,
        });
        // 3. System read-only assets.
        for sys in &["fonts", "icons", "themes", "timezones"] {
            entries.push(SandboxEntry {
                guest_path:      PathBuf::from(format!("/sys/{sys}")),
                backend_subpath: PathBuf::from(sys),
                backend_name:    "sys_ro".into(),
                access:          SandboxAccess::ReadOnly,
                xattr_namespace: None,
            });
        }
        // 4. Optional shared mount.
        if self.manifest.capabilities.filesystem_shared {
            entries.push(SandboxEntry {
                guest_path:      PathBuf::from("/data/shared"),
                backend_subpath: PathBuf::from("shared"),
                backend_name:    "ext4".into(),
                access:          SandboxAccess::ReadWrite,
                xattr_namespace: None,
            });
        }
        Ok(SandboxMap { entries: Arc::from(entries.into_boxed_slice()) })
    }
}
```

### 3.2 Why this beats runtime translation

| Concern | Runtime rewriting (rejected) | Preopen-driven (chosen) |
|---------|------------------------------|-------------------------|
| TOCTOU | Yes — supervisor stat then Wasmtime open | None — cap-std handle is the capability |
| Symlink escape | Supervisor must replicate Wasmtime resolution | Wasmtime owns it (kernel-level via `openat`) |
| Relative paths from held descriptors | Invisible to supervisor | Wasmtime handles trivially |
| Hot-path cost | 6 µs per op (resolve + check) | 0 — Wasmtime does it |
| Implementation cost | ~1 200 LOC of resolver | ~120 LOC builder |

### 3.3 Default sandbox table

| Logical path | Access | Backend | Persistence |
|--------------|--------|---------|-------------|
| `/data/<bundle>/` | ReadWrite | ext4 | persistent |
| `/data/<bundle>/.Trash/` | ReadWrite (mediated via `vyoma:fs/trash`) | ext4 | persistent |
| `/tmp/<bundle>-<iid>/` | ReadWrite | tmpfs | per-instance |
| `/sys/fonts/` | ReadOnly | sys_ro | initramfs |
| `/sys/icons/` | ReadOnly | sys_ro | initramfs |
| `/sys/themes/` | ReadOnly | sys_ro | initramfs |
| `/sys/timezones/` | ReadOnly | sys_ro | initramfs |
| `/data/shared/` | ReadWrite | ext4 | persistent, cap-gated |
| (powerbox-granted) | RO or RW per grant | ext4 | persistent, bookmark-gated |

`filesystem_full_disk` adds `/data/` ReadWrite. Available only to system apps; never to v1 app
store apps.

### 3.4 Bookmark-granted preopens

When a user picks a file in the Powerbox panel and the requesting app calls `bookmark.open(tok)`,
the supervisor does NOT extend the app's preopen list (this would require restarting the
Wasmtime store). Instead:

- The panel + bookmark service returns a *transient* `wasi:filesystem/types/descriptor`
  resource directly to the app via the WIT call. The descriptor is held in the app's resource
  table for as long as the app references it; on `drop` it's closed.
- The supervisor records the open as a "bookmark-granted descriptor" in its own
  `BookmarkSession` table so that crash recovery can release the underlying inode lock.
- This descriptor IS a preopen-equivalent in WASI semantics: the app can read/write through it,
  use it as a `path-open` base, but cannot escape via `..` (Wasmtime's resolver).

### 3.5 Path translation cache (perf)

The supervisor needs path resolution only for the `vyoma:fs/*` calls that take a `string` path
(snapshot, xattr, bookmark, panel-return). For those, we cache `(actor, logical_path) →
(BackendPath, mount_id)` in a per-instance LRU.

```rust
// supervisor/src/fs/path_index.rs

use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

pub struct PathBucket {
    inner: parking_lot::Mutex<LruCache<PathBuf, (BackendPath, u32)>>,
}

impl PathBucket {
    pub fn new() -> Self {
        Self { inner: parking_lot::Mutex::new(LruCache::new(NonZeroUsize::new(4096).unwrap())) }
    }
    pub fn lookup(&self, p: &Path) -> Option<(BackendPath, u32)> {
        self.inner.lock().get(&p.to_path_buf()).cloned()
    }
    pub fn insert(&self, p: PathBuf, bp: BackendPath, mount_id: u32) {
        self.inner.lock().put(p, (bp, mount_id));
    }
    pub fn invalidate(&self) {
        self.inner.lock().clear();
    }
}
```

Cache invalidation is triggered on `ManifestReloaded` (rare). Hit cost: ~150-300 ns. Miss
cost: ~3-8 µs (canonicalize + sandbox lookup).

---

## 4. App Filesystem API (`vyoma:fs@0.1.0` WIT + VYOMA_FS protocol)

### 4.1 WIT surface

The complete WIT file:

```wit
// wit/vyoma-fs.wit

package vyoma:fs@0.1.0;

interface types {
    record path { s: string }

    record file-stat {
        kind: file-kind,
        size: u64,
        allocated: u64,
        mtime-ns: s64,
        ctime-ns: s64,
        atime-ns: s64,
        btime-ns: s64,
        mode: u32,
        nlink: u32,
        inode: u64,
        blocks: u64,
    }

    enum file-kind { regular, directory, symlink, fifo, device }

    enum fs-error {
        not-found, permission-denied, quota-exceeded, in-use, already-exists,
        not-empty, invalid-name, path-escape, path-too-long, not-absolute,
        unsupported, backend-error, snapshot-conflict, sender-gone,
        bookmark-invalid, bookmark-expired, coordination-timeout, coordination-cycle,
        io-error,
    }
}

interface xattr {
    use types.{path, fs-error};
    get:    func(p: path, key: string)            -> result<list<u8>, fs-error>;
    set:    func(p: path, key: string, val: list<u8>) -> result<_, fs-error>;
    list:   func(p: path)                         -> result<list<string>, fs-error>;
    remove: func(p: path, key: string)            -> result<_, fs-error>;
}

interface snapshot {
    use types.{path, fs-error};
    type snapshot-id = u64;
    record snapshot-info {
        id: snapshot-id,
        label: string,
        created-ns: s64,
        bytes-unique: u64,
        bytes-total: u64,
    }
    /// v1: returns Unsupported("snapshots-defer-v2").
    create:    func(root: path, label: string)    -> result<snapshot-id, fs-error>;
    list:      func(root: path)                   -> result<list<snapshot-info>, fs-error>;
    delete:    func(id: snapshot-id)              -> result<_, fs-error>;
    restore:   func(id: snapshot-id)              -> result<_, fs-error>;
    /// v1: works on ext4 via FICLONE.
    clone-file: func(src: path, dst: path)        -> result<_, fs-error>;
}

interface watcher {
    use types.{path, fs-error};
    enum watch-scope { file, directory, tree }
    record watch-event {
        watch-id: u64,
        path: string,
        kind: event-kind,
        cookie: option<u64>,
        seq: u64,
        timestamp-ns: s64,
    }
    enum event-kind {
        created, modified, deleted,
        renamed-from, renamed-to,
        attribute-changed, xattr-changed,
        watch-overflow,
    }
    resource file-watcher {
        constructor(p: path, scope: watch-scope);
        id: func() -> u64;
        /// Acknowledge after handling a batch; reopens flow after overflow.
        acknowledge: func();
    }
}

interface coord {
    use types.{path, fs-error};
    record coord-token { id: u64 }
    coordinate-read:  func(p: path, timeout-ms: u32) -> result<coord-token, fs-error>;
    coordinate-write: func(p: path, timeout-ms: u32) -> result<coord-token, fs-error>;
    release:          func(t: coord-token) -> result<_, fs-error>;
    resource file-presenter {
        constructor(p: path);
    }
}

interface panel {
    use types.{fs-error};
    record open-request {
        title: string,
        prompt: string,
        extensions: list<string>,
        allow-multiple: bool,
        allow-directories: bool,
        show-hidden: bool,
        initial-directory: option<string>,
    }
    record save-request {
        title: string,
        prompt: string,
        default-name: string,
        extensions: list<string>,
        initial-directory: option<string>,
    }
    record panel-result {
        bookmarks: list<list<u8>>,    // opaque
        cancelled: bool,
    }
    open: func(req: open-request) -> result<panel-result, fs-error>;
    save: func(req: save-request) -> result<panel-result, fs-error>;
}

interface bookmark {
    use types.{fs-error};
    type bookmark-token = list<u8>;
    /// Redeems a bookmark. Returns a wasi:filesystem/types/descriptor handle id.
    open: func(tok: bookmark-token, read-only: bool) -> result<u32 /* wasi-desc */, fs-error>;
    save: func(name: string, tok: bookmark-token)    -> result<_, fs-error>;
    load: func(name: string)                          -> result<bookmark-token, fs-error>;
    list: func()                                      -> result<list<string>, fs-error>;
}

interface trash {
    use types.{path, fs-error};
    record trash-entry {
        token: u64,
        original-path: string,
        trashed-at-ns: s64,
        size: u64,
    }
    trash:   func(p: path) -> result<u64, fs-error>;
    untrash: func(t: u64)  -> result<string, fs-error>;
    list:    func()        -> result<list<trash-entry>, fs-error>;
    purge:   func(t: u64)  -> result<_, fs-error>;
}

interface quota {
    record usage {
        bytes-used: u64,
        bytes-limit: u64,
        file-count: u64,
        file-limit: u64,
    }
    get: func() -> usage;
}

world fs-app {
    import wasi:filesystem/types@0.2.0;
    import xattr;
    import snapshot;
    import watcher;
    import coord;
    import panel;
    import bookmark;
    import trash;
    import quota;
}
```

### 4.2 Stdout protocol (`VYOMA_FS:`) for legacy apps

For symmetry with `VYOMA_DRAW:` and `@supervisor:`, a stdout protocol covers user-visible
operations:

```
VYOMA_FS:open_panel:<title>,<csv-extensions>,<mode>
VYOMA_FS:save_panel:<title>,<default-name>,<csv-extensions>
VYOMA_FS:trash:<path>
VYOMA_FS:reveal:<path>
VYOMA_FS:share:<path>
```

Gated by `legacy_stdout_fs` boot flag (default off) and `[capabilities.fs_stdout]` manifest
entry. New apps use WIT. Panel results are delivered via the Round 3 `on-ipc` callback with
envelope schema `fs.panel-result`.

### 4.3 Large reads via `SharedBuffer`

Any read ≥ 64 KB (configurable per-mount) is published via the Round 2/3 `SharedBuffer`
mechanism instead of copying through the WASM linear heap:

```rust
// supervisor/src/fs/wasi_shim.rs (continued)

pub fn maybe_shared_buffer(
    vfs: &Vfs, actor: &ActorRef, file: &dyn VfsFile, offset: u64, len: usize,
) -> Option<SharedBufferHandle> {
    if len < 64 * 1024 { return None; }
    if let Some(region) = file.mmap_region(offset, len as u64) {
        return Some(vfs.publish_mmap_as_shared(actor, region));
    }
    None
}
```

The app gets a `SharedBufferHandle` resource it can `read()` from at offset 0..len. Crash
of the sender (rare: the supervisor) marks `valid = false`; reads return
`IpcError::SenderGone`. Refcount is 1 per holder; reclaim on drop. Identical model to Round 3.

---

## 5. File Watching — Fixed

### 5.1 `WatcherBackend` trait

Critic C7 forced us to abstract watching behind a trait because 9P cannot do inotify:

```rust
// supervisor/src/fs/watcher.rs

use std::path::PathBuf;
use std::sync::Arc;

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct WatchId(pub u64);

#[derive(Copy, Clone, Debug)]
pub enum WatchScope { File, Directory, Tree }

#[derive(Clone, Debug)]
pub struct RawWatchEvent {
    pub backend_path: PathBuf,
    pub kind: RawEventKind,
    pub cookie: Option<u64>,
}

#[derive(Copy, Clone, Debug)]
pub enum RawEventKind {
    Created, Modified, Deleted,
    RenamedFrom, RenamedTo,
    AttributeChanged, XattrChanged,
}

pub trait WatcherBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Begin watching this backend path; returns a token used to cancel.
    fn add_watch(&self, path: &BackendPath, scope: WatchScope)
        -> Result<u32, VfsError>;
    fn rm_watch(&self, token: u32) -> Result<(), VfsError>;
    /// Block until at least one event is available; populates `out`.
    /// Returns count.
    fn poll(&self, out: &mut Vec<RawWatchEvent>) -> Result<usize, VfsError>;
}
```

### 5.2 `Ext4InotifyBackend`

The primary watcher. One `inotify` fd shared across all subscriptions. ~300 LOC.

```rust
// supervisor/src/fs/watcher_inotify.rs

use inotify::{Inotify, WatchMask, WatchDescriptor};
use parking_lot::Mutex;
use std::collections::HashMap;

pub struct Ext4InotifyBackend {
    fd: Mutex<Inotify>,
    map: Mutex<HashMap<u32, WatchDescriptor>>,
    rev: Mutex<HashMap<WatchDescriptor, u32>>,
    next: std::sync::atomic::AtomicU32,
}

impl Ext4InotifyBackend {
    pub fn new() -> Result<Self, VfsError> {
        let fd = Inotify::init().map_err(|e| VfsError::BackendError(e.to_string()))?;
        Ok(Self {
            fd: Mutex::new(fd),
            map: Mutex::new(HashMap::new()),
            rev: Mutex::new(HashMap::new()),
            next: 0.into(),
        })
    }
}

impl WatcherBackend for Ext4InotifyBackend {
    fn name(&self) -> &'static str { "ext4-inotify" }

    fn add_watch(&self, path: &BackendPath, scope: WatchScope) -> Result<u32, VfsError> {
        let mut mask = WatchMask::CREATE | WatchMask::DELETE | WatchMask::MODIFY
            | WatchMask::ATTRIB | WatchMask::MOVED_FROM | WatchMask::MOVED_TO;
        if matches!(scope, WatchScope::Tree) { mask |= WatchMask::ONLYDIR; }
        let wd = self.fd.lock()
            .add_watch(path.as_path(), mask)
            .map_err(|e| VfsError::BackendError(e.to_string()))?;
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.map.lock().insert(id, wd.clone());
        self.rev.lock().insert(wd, id);
        Ok(id)
    }

    fn rm_watch(&self, token: u32) -> Result<(), VfsError> {
        if let Some(wd) = self.map.lock().remove(&token) {
            self.rev.lock().remove(&wd);
            self.fd.lock().rm_watch(wd).map_err(|e| VfsError::BackendError(e.to_string()))?;
        }
        Ok(())
    }

    fn poll(&self, out: &mut Vec<RawWatchEvent>) -> Result<usize, VfsError> {
        let mut buf = [0u8; 4096];
        let events = self.fd.lock().read_events_blocking(&mut buf)
            .map_err(|e| VfsError::BackendError(e.to_string()))?;
        let mut n = 0;
        for ev in events {
            let kind = if ev.mask.contains(inotify::EventMask::CREATE) { RawEventKind::Created }
                else if ev.mask.contains(inotify::EventMask::DELETE) { RawEventKind::Deleted }
                else if ev.mask.contains(inotify::EventMask::MODIFY) { RawEventKind::Modified }
                else if ev.mask.contains(inotify::EventMask::ATTRIB) { RawEventKind::AttributeChanged }
                else if ev.mask.contains(inotify::EventMask::MOVED_FROM) { RawEventKind::RenamedFrom }
                else if ev.mask.contains(inotify::EventMask::MOVED_TO) { RawEventKind::RenamedTo }
                else { continue };
            let bp = self.rev.lock().get(&ev.wd).copied().unwrap_or(0);
            out.push(RawWatchEvent {
                backend_path: ev.name.map(|n| PathBuf::from(n)).unwrap_or_default(),
                kind,
                cookie: Some(ev.cookie as u64),
            });
            let _ = bp;
            n += 1;
        }
        Ok(n)
    }
}
```

### 5.3 `NinePPollBackend`

For the legacy `/host` mount. Poll every 250 ms; per-watch stat the path; emit synthetic
events. ~200 LOC.

```rust
// supervisor/src/fs/watcher_poll.rs

pub struct NinePPollBackend {
    interval: std::time::Duration,
    watches: parking_lot::RwLock<HashMap<u32, PollWatch>>,
    next: std::sync::atomic::AtomicU32,
    backend: Arc<dyn VfsBackend>,
}

struct PollWatch {
    path: BackendPath,
    scope: WatchScope,
    last_snapshot: HashMap<PathBuf, (u64 /* size */, i64 /* mtime */)>,
}

impl WatcherBackend for NinePPollBackend {
    fn name(&self) -> &'static str { "9p-poll" }

    fn add_watch(&self, path: &BackendPath, scope: WatchScope) -> Result<u32, VfsError> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let snap = self.snapshot_tree(path, scope)?;
        self.watches.write().insert(id, PollWatch {
            path: path.clone(), scope, last_snapshot: snap,
        });
        Ok(id)
    }
    fn rm_watch(&self, token: u32) -> Result<(), VfsError> {
        self.watches.write().remove(&token);
        Ok(())
    }
    fn poll(&self, out: &mut Vec<RawWatchEvent>) -> Result<usize, VfsError> {
        std::thread::sleep(self.interval);
        let mut n = 0;
        let mut watches = self.watches.write();
        for (_id, w) in watches.iter_mut() {
            let cur = self.snapshot_tree(&w.path, w.scope)?;
            n += diff_into(&w.last_snapshot, &cur, out);
            w.last_snapshot = cur;
        }
        Ok(n)
    }
}
```

The poll backend pays the 1-2 ms 9P RTT only on the snapshotting thread; it doesn't block any
app's I/O. Critic's "polling consumes 3.75 s/s of 9P bandwidth" projection assumed every watch
caller was hammering the channel; in practice, the 9P channel is rarely used in v1 because
`/host` is RO and developer-only.

### 5.4 `WatcherCore`

```rust
// supervisor/src/fs/watcher.rs (continued)

pub struct WatcherCore {
    subs:    dashmap::DashMap<WatchId, Arc<WatchSubscription>>,
    by_path: dashmap::DashMap<BackendPath, smallvec::SmallVec<[WatchId; 4]>>,
    next_id: std::sync::atomic::AtomicU64,
    next_seq: std::sync::atomic::AtomicU64,
    backends: HashMap<String, Arc<dyn WatcherBackend>>,
    coalescer: parking_lot::Mutex<Coalescer>,
    router: Arc<IpcRouter>,
}

pub struct WatchSubscription {
    pub id: WatchId,
    pub owner: ActorRef,
    pub backend_name: String,
    pub backend_token: u32,
    pub backend_path: BackendPath,
    pub scope: WatchScope,
    pub seq: std::sync::atomic::AtomicU64,
}

impl WatcherCore {
    pub fn new(backends: HashMap<String, Arc<dyn WatcherBackend>>, router: Arc<IpcRouter>) -> Self {
        Self {
            subs: dashmap::DashMap::new(),
            by_path: dashmap::DashMap::new(),
            next_id: 0.into(),
            next_seq: 0.into(),
            backends,
            coalescer: parking_lot::Mutex::new(Coalescer::new(100 /* ms */)),
            router,
        }
    }

    pub fn subscribe(
        &self, actor: ActorRef, mount: &Mount, bp: BackendPath, scope: WatchScope,
    ) -> Result<WatchId, VfsError> {
        let backend = self.backends.get(mount.backend.name())
            .ok_or(VfsError::Unsupported("no-watcher-for-backend"))?;
        let backend_token = backend.add_watch(&bp, scope)?;
        let id = WatchId(self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let sub = Arc::new(WatchSubscription {
            id, owner: actor, backend_name: mount.backend.name().into(),
            backend_token, backend_path: bp.clone(), scope, seq: 0.into(),
        });
        self.subs.insert(id, sub);
        self.by_path.entry(bp).or_default().push(id);
        Ok(id)
    }

    pub fn unsubscribe(&self, id: WatchId) {
        if let Some((_, sub)) = self.subs.remove(&id) {
            if let Some(backend) = self.backends.get(&sub.backend_name) {
                let _ = backend.rm_watch(sub.backend_token);
            }
            if let Some(mut entry) = self.by_path.get_mut(&sub.backend_path) {
                entry.retain(|x| *x != id);
            }
        }
    }

    /// Called when an instance crashes / exits.
    pub fn on_instance_gone(&self, iid: InstanceId) {
        let to_drop: Vec<WatchId> = self.subs.iter()
            .filter(|kv| kv.value().owner.instance_id() == iid)
            .map(|kv| *kv.key()).collect();
        for id in to_drop { self.unsubscribe(id); }
    }
}
```

### 5.5 Coalescer and event delivery

```rust
// supervisor/src/fs/watcher_coalesce.rs

use std::time::{Duration, Instant};

pub struct Coalescer {
    window: Duration,
    bucket: HashMap<(WatchId, PathBuf, EventKindBucket), CoalescedEvent>,
    last_flush: Instant,
}

#[derive(Eq, Hash, PartialEq)]
enum EventKindBucket { Created, Modified, Deleted, AttrOrXattr, Renamed }

struct CoalescedEvent {
    kind: RawEventKind,
    cookie: Option<u64>,
    count: u32,
    first_seen: Instant,
}

impl Coalescer {
    pub fn new(window_ms: u64) -> Self {
        Self {
            window: Duration::from_millis(window_ms),
            bucket: HashMap::new(),
            last_flush: Instant::now(),
        }
    }
    pub fn record(&mut self, wid: WatchId, p: PathBuf, ev: RawWatchEvent) {
        let bucket = bucket_of(ev.kind);
        let key = (wid, p, bucket);
        self.bucket.entry(key)
            .and_modify(|e| { e.kind = merge(e.kind, ev.kind); e.count += 1; })
            .or_insert(CoalescedEvent {
                kind: ev.kind, cookie: ev.cookie, count: 1, first_seen: Instant::now(),
            });
    }
    pub fn flush_if_due(&mut self, out: &mut Vec<(WatchId, PathBuf, CoalescedEvent)>) {
        if self.last_flush.elapsed() < self.window { return; }
        for ((wid, p, _), ev) in self.bucket.drain() {
            out.push((wid, p, ev));
        }
        self.last_flush = Instant::now();
    }
}

fn bucket_of(k: RawEventKind) -> EventKindBucket { /* obvious map */ todo!() }
fn merge(a: RawEventKind, b: RawEventKind) -> RawEventKind {
    // created + modified → created;
    // created + deleted → (caller drops);
    // modified + modified → modified;
    // attribute + modified → modified
    match (a, b) {
        (RawEventKind::Created, RawEventKind::Modified) => RawEventKind::Created,
        (_, RawEventKind::Modified) => RawEventKind::Modified,
        (a, _) => a,
    }
}
```

Delivery via Round 3 IPC:

```rust
fn deliver(&self, sub: &WatchSubscription, kind: RawEventKind, p: PathBuf, cookie: Option<u64>) {
    let seq = self.next_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let event = WatchEventBody {
        watch_id: sub.id.0,
        path: p.to_string_lossy().into(),
        kind: kind.into(),
        cookie, seq,
        timestamp_ns: now_ns(),
    };
    let env = IpcEnvelope {
        from: IpcAddr::Supervisor,
        to:   IpcAddr::Instance(sub.owner.instance_id()),
        class: IpcClass::Normal,
        priority: Priority::Normal,
        schema_id: schema!(fs.watch_event),
        seq,
        body: encode_cbor(&event),
        deadline_ns: None,
    };
    self.router.send_lossy(env, OverflowPolicy::Coalesce);
}
```

### 5.6 Per-app watch limits

| Limit | Default | Rationale |
|-------|---------|-----------|
| Active watches | 64 | Inotify watch budget |
| Recursive depth | 16 | Bound coalescer size |
| Events/sec sustained | 50 | Backpressure target |
| Coalescer queue | 256 | Memory ceiling |

On overflow, a single `WatchOverflow` event is delivered with `dropped_count`. The app must
call `watcher.acknowledge()` to resume.

---

## 6. File Coordination — Fixed

### 6.1 Lock model

```rust
// supervisor/src/fs/coordination.rs

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LockMode { Read, Write }

pub struct CoordinationLock {
    pub path: BackendPath,
    pub mode: LockMode,
    pub readers: smallvec::SmallVec<[Holder; 4]>,
    pub writer: Option<Holder>,
    pub waiters: VecDeque<Waiter>,
    pub last_writer_seq: u64,
}

#[derive(Clone, Debug)]
pub struct Holder {
    pub actor: ActorRef,
    pub token_id: u64,
    pub acquired_at: i64,
}

#[derive(Clone, Debug)]
pub struct Waiter {
    pub actor: ActorRef,
    pub mode: LockMode,
    pub req_id: u64,
    pub deadline_ns: i64,
    pub priority: Priority,
    pub waker: ReplyWaker,
}

pub struct CoordinationService {
    locks: dashmap::DashMap<BackendPath, Arc<Mutex<CoordinationLock>>>,
    wait_graph: Arc<WaitForGraph>,
    next_token: AtomicU64,
    wal: Arc<CoordWal>,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct CoordToken { pub id: u64 }
```

### 6.2 Acquire algorithm with cycle detection

```rust
impl CoordinationService {
    pub async fn acquire(
        &self, actor: &ActorRef, path: BackendPath, mode: LockMode, timeout_ms: u32,
    ) -> Result<CoordToken, VfsError> {
        let lock = self.locks.entry(path.clone()).or_default().clone();
        let mut g = lock.lock();
        if is_compatible(&g, mode) {
            let token = self.grant_now(&mut g, actor.clone(), mode);
            self.wal.log_acquire(actor.instance_id(), &path, mode, token.id)?;
            return Ok(token);
        }
        // Would block; check graph for cycle.
        let holders = current_holders(&g);
        if self.wait_graph.would_cycle(actor.instance_id(), &holders) {
            return Err(VfsError::CoordinationCycle);
        }
        // Enqueue waiter.
        let req_id = self.next_token.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (waker, fut) = make_waker(req_id);
        let waiter = Waiter {
            actor: actor.clone(),
            mode, req_id,
            deadline_ns: now_ns() + (timeout_ms as i64) * 1_000_000,
            priority: actor.priority(),
            waker,
        };
        inherit_priority(&mut g, &waiter);
        g.waiters.push_back(waiter);
        for h in holders.iter() {
            self.wait_graph.add_edge(actor.instance_id(), h.instance_id());
        }
        drop(g);
        // Wait.
        match tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms as u64), fut,
        ).await {
            Ok(Ok(token)) => {
                self.wal.log_acquire(actor.instance_id(), &path, mode, token.id)?;
                Ok(token)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                self.cancel_waiter(&path, req_id);
                Err(VfsError::CoordinationTimeout)
            }
        }
    }

    pub fn release(&self, actor: &ActorRef, token: CoordToken) -> Result<(), VfsError> {
        let path = self.lookup_path(token)?;
        let lock = self.locks.get(&path).ok_or(VfsError::NotFound)?.clone();
        let mut g = lock.lock();
        remove_holder(&mut g, token);
        self.wal.log_release(actor.instance_id(), &path, token.id)?;
        let granted = wake_next_compatible(&mut g);
        for grant in granted.iter() {
            self.wait_graph.remove_edge(grant.actor.instance_id(), actor.instance_id());
        }
        Ok(())
    }
}

fn is_compatible(g: &CoordinationLock, mode: LockMode) -> bool {
    match mode {
        LockMode::Read => g.writer.is_none(),
        LockMode::Write => g.writer.is_none() && g.readers.is_empty(),
    }
}
```

### 6.3 Crash recovery via `on_instance_gone`

```rust
impl CoordinationService {
    pub fn on_instance_gone(&self, iid: InstanceId) {
        let mut paths_to_clean: Vec<BackendPath> = Vec::new();
        for entry in self.locks.iter() {
            let mut g = entry.value().lock();
            // Drain readers held by this instance.
            g.readers.retain(|h| h.actor.instance_id() != iid);
            if let Some(w) = &g.writer {
                if w.actor.instance_id() == iid {
                    g.writer = None;
                }
            }
            // Cancel waiters owned by this instance.
            g.waiters.retain(|w| {
                if w.actor.instance_id() == iid {
                    let _ = w.waker.fail(IpcError::SenderGone.into());
                    false
                } else {
                    true
                }
            });
            // Wake newly-eligible waiters.
            wake_next_compatible(&mut g);
            if g.readers.is_empty() && g.writer.is_none() && g.waiters.is_empty() {
                paths_to_clean.push(g.path.clone());
            }
        }
        for p in paths_to_clean { self.locks.remove(&p); }
        // Also: drop graph edges.
        self.wait_graph.drop_all_for(iid);
        // WAL: log a single "release-all" record.
        let _ = self.wal.log_release_all(iid);
    }
}
```

### 6.4 Supervisor crash recovery via WAL

```rust
// supervisor/src/fs/coord_wal.rs

pub struct CoordWal {
    file: parking_lot::Mutex<std::fs::File>,
    path: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub enum CoordRecord {
    Acquire { iid: u64, path: PathBuf, mode: u8, token: u64, ts_ns: i64 },
    Release { iid: u64, path: PathBuf, token: u64, ts_ns: i64 },
    ReleaseAll { iid: u64, ts_ns: i64 },
    Checkpoint { live_locks: Vec<(PathBuf, u64, u8)>, ts_ns: i64 },
}

impl CoordWal {
    pub fn open(path: PathBuf) -> Result<Self, VfsError> {
        let file = std::fs::OpenOptions::new().create(true).append(true).read(true)
            .open(&path)?;
        Ok(Self { file: parking_lot::Mutex::new(file), path })
    }

    pub fn replay(&self, alive: &dyn Fn(InstanceId) -> bool) -> Result<Vec<(BackendPath, LockMode, InstanceId, u64)>, VfsError> {
        // Read all records; reconstruct live lock set; drop entries whose owners are gone.
        let mut held: HashMap<(PathBuf, u64), (InstanceId, LockMode)> = HashMap::new();
        // ... parse ...
        // Final pass: filter by alive(iid).
        let alive_only: Vec<_> = held.into_iter()
            .filter(|(_, (iid, _))| alive(*iid))
            .map(|((p, tok), (iid, mode))| (BackendPath::new(p).unwrap(), mode, iid, tok))
            .collect();
        Ok(alive_only)
    }

    pub fn log_acquire(&self, iid: InstanceId, path: &BackendPath, mode: LockMode, token: u64)
        -> Result<(), VfsError>
    {
        let rec = CoordRecord::Acquire {
            iid: iid.0, path: path.as_path().to_path_buf(),
            mode: mode as u8, token, ts_ns: now_ns(),
        };
        self.append(&rec)
    }
    pub fn log_release(&self, iid: InstanceId, path: &BackendPath, token: u64) -> Result<(), VfsError> {
        let rec = CoordRecord::Release {
            iid: iid.0, path: path.as_path().to_path_buf(), token, ts_ns: now_ns(),
        };
        self.append(&rec)
    }
    pub fn log_release_all(&self, iid: InstanceId) -> Result<(), VfsError> {
        let rec = CoordRecord::ReleaseAll { iid: iid.0, ts_ns: now_ns() };
        self.append(&rec)
    }
    fn append(&self, rec: &CoordRecord) -> Result<(), VfsError> {
        use std::io::Write;
        let bytes = serde_cbor::to_vec(rec).unwrap();
        let mut f = self.file.lock();
        f.write_all(&(bytes.len() as u32).to_be_bytes())?;
        f.write_all(&bytes)?;
        f.sync_data()?;
        Ok(())
    }
}
```

On supervisor restart:

1. Open `/data/.vyoma/coord.wal`.
2. Replay records into a temporary `HashMap<(path, token), (iid, mode)>`.
3. After replay, scan the Round-1 instance table for live `InstanceId`s.
4. Any held lock whose `iid` is not live is dropped (the owner died with the supervisor).
5. Any held lock whose `iid` IS live is given a 5 s grace window during which the instance must
   re-prove its claim via `coord.reattach(token)`. If reattach doesn't happen, the lock is
   released and watchers/waiters are notified.

### 6.5 Priority inheritance

Writer at `Priority::High` waiting on reader at `Priority::Normal` → reader's dispatcher thread
gets a brief priority boost via `inherit_priority`:

```rust
fn inherit_priority(g: &mut CoordinationLock, waiter: &Waiter) {
    if waiter.priority > Priority::Normal {
        for h in g.readers.iter() {
            h.actor.boost_until(waiter.priority, BoostReason::CoordPriorityInherit);
        }
        if let Some(w) = &g.writer {
            w.actor.boost_until(waiter.priority, BoostReason::CoordPriorityInherit);
        }
    }
}
```

Boost is revoked on `release()`. This prevents the classic priority inversion (background
indexer holds lock while UI app waits).

### 6.6 Deadlock detection via Round 3 `WaitForGraph`

The Round 3 `WaitForGraph` is extended:

```rust
// (extends supervisor/src/ipc/waitfor.rs)

impl WaitForGraph {
    /// New: edges of two flavours — `Call` (Round 3) and `CoordLock` (Round 4).
    /// would_cycle() walks both flavours uniformly.
    pub fn add_coord_edge(&self, waiter: InstanceId, holder: InstanceId) {
        self.add_edge_kind(waiter, holder, EdgeKind::CoordLock);
    }
}
```

A `coord.coordinate-write` cycle detection example:

- App A holds write lock on `/data/a.txt`, waiting for IPC `call` to App B.
- App B holds write lock on `/data/b.txt`, requests write lock on `/data/a.txt`.
- The graph has edges: B-(CoordLock)→A; A-(Call)→B.
- `would_cycle(B, [A])` walks A → B → cycle → returns true → B gets
  `VfsError::CoordinationCycle`.

---

## 7. Open/Save Panel & Powerbox

### 7.1 Architecture

The panel is a chrome-process window owned by the supervisor's compositor. The requesting app
never sees the panel's pixels; it sees one async WIT call (`panel.open(req)`) that resolves
to a list of bookmark tokens (or `cancelled = true`).

```
App                  Supervisor                Panel app           User
 │ panel.open(req)    │                         │                  │
 │ ──────────────────►│ allocate session id     │                  │
 │                    │ spawn-or-attach panel ─►│ render UI ──────►│
 │                    │                         │                  │ navigates
 │                    │                         │ ◄─────────────── │ click "Open"
 │                    │ ◄─ canonical_paths ─────│                  │
 │                    │ mint bookmarks scoped to│                  │
 │                    │ REQUESTING app          │                  │
 │ ◄─ panel-result ───│                         │                  │
 │ bookmark.open(tok) │                         │                  │
 │ ──────────────────►│ verify HMAC → return    │                  │
 │ ◄─ wasi-desc ──────│   wasi:filesystem desc  │                  │
```

### 7.2 Panel app privilege boundary

The panel is a built-in WASM app at `apps/panel/`, running with:

```toml
[capabilities]
chrome = true
filesystem_full_disk = true
panel_render = true     # new cap, only granted to bundled supervisor panel
```

It can browse the entire filesystem (to render the picker UI). It returns the selected
canonical paths to the **supervisor**, not directly to the requester. The supervisor mints
bookmark tokens scoped to the requesting bundle. Critic raised Q7 about panel app being a
high-authority TCB — we accept this as the tradeoff for replaceable UI; the security boundary
is `the panel cannot mint a bookmark for any app other than the one it was spawned to serve`,
enforced by the supervisor.

### 7.3 `PanelService`

```rust
// supervisor/src/fs/panel.rs

use std::collections::HashMap;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct PanelService {
    sessions: Mutex<HashMap<PanelSessionId, PanelSession>>,
    next_id: std::sync::atomic::AtomicU64,
    bookmarks: Arc<BookmarkService>,
    compositor: Arc<CompositorHandle>,
    spawner: Arc<AppSpawner>,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct PanelSessionId(pub u64);

pub struct PanelSession {
    pub id: PanelSessionId,
    pub requester: ActorRef,
    pub req: PanelRequest,
    pub waker: ReplyWaker,
    pub started_at: i64,
}

pub enum PanelRequest {
    Open(OpenRequest),
    Save(SaveRequest),
}

impl PanelService {
    pub async fn open(&self, actor: &ActorRef, req: OpenRequest) -> Result<PanelResult, VfsError> {
        let id = PanelSessionId(self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let (waker, fut) = make_waker(id.0);
        self.sessions.lock().insert(id, PanelSession {
            id, requester: actor.clone(),
            req: PanelRequest::Open(req.clone()),
            waker, started_at: now_ns(),
        });
        // Spawn (or re-use) the panel app and hand it the session ID.
        self.spawner.dispatch_panel(id, &req).await?;
        // Await the panel's response.
        match tokio::time::timeout(std::time::Duration::from_secs(300), fut).await {
            Ok(Ok(paths)) => {
                let bookmarks = paths.into_iter().map(|p| {
                    self.bookmarks.mint(
                        actor, &p, BookmarkMode::ReadWrite,
                        BookmarkLifetime::Persistent,
                        Granter::UserViaPanel,
                    )
                }).collect::<Result<Vec<_>, _>>()?;
                Ok(PanelResult { bookmarks, cancelled: false })
            }
            Ok(Err(_)) => Ok(PanelResult { bookmarks: vec![], cancelled: true }),
            Err(_) => Err(VfsError::CoordinationTimeout),
        }
    }
}
```

### 7.4 Panel UX requirements

| Element | Equivalent in macOS | v1 in VyomaOS |
|---------|---------------------|---------------|
| Sidebar (Recents, Documents, Downloads, Trash) | NSOpenPanel sidebar | ✅ |
| Search field | Spotlight inline | v2 (deferred) |
| List/Tile/Column views | Finder views | List + Tile in v1 |
| Filter by extension | `allowedFileTypes` | ✅ |
| Show hidden | `showsHiddenFiles` | ✅ |
| Tag filter | Finder tags | v2 |
| Quick Look preview | Cmd-Y | v2 |
| Multi-select | `allowsMultipleSelection` | ✅ |
| Directory pick | `canChooseDirectories` | ✅ |

---

## 8. Bookmarks — Fixed

### 8.1 Token format

```rust
// supervisor/src/fs/bookmark.rs

use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BookmarkBody {
    pub version: u8,
    pub bundle_id: String,
    pub canonical_path: PathBuf,
    pub mount_name: String,
    pub mode: BookmarkMode,
    pub created_ns: i64,
    pub expires_ns: Option<i64>,
    pub granter: Granter,
    pub inode_hint: Option<u64>,
    pub nonce: [u8; 16],
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum BookmarkMode { ReadOnly, ReadWrite, AppendOnly }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Granter { UserViaPanel, ManifestStatic, BookmarkRefresh }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum BookmarkLifetime { Persistent, UntilInstanceExit }

#[derive(Clone, Debug)]
pub struct BookmarkToken {
    pub body_cbor: Vec<u8>,
    pub mac: [u8; 32],
}

impl BookmarkToken {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body_cbor.len() + 32);
        out.extend_from_slice(&(self.body_cbor.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.body_cbor);
        out.extend_from_slice(&self.mac);
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, VfsError> {
        if bytes.len() < 36 { return Err(VfsError::BookmarkInvalid); }
        let len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        if bytes.len() != 4 + len + 32 { return Err(VfsError::BookmarkInvalid); }
        Ok(Self {
            body_cbor: bytes[4..4+len].to_vec(),
            mac: bytes[4+len..].try_into().unwrap(),
        })
    }
}
```

### 8.2 HMAC binding to app signing key

The key derivation:

```rust
// supervisor/src/fs/bookmark.rs (continued)

use hmac::{Hmac, Mac};
use sha2::Sha256;
use hkdf::Hkdf;

pub struct BookmarkService {
    /// Long-term supervisor master key, sealed at /data/.vyoma/keys/master.key.
    master_key: [u8; 32],
    /// Per-bundle derived signing keys, cached.
    cache: parking_lot::RwLock<HashMap<String, [u8; 32]>>,
    /// Persistent store.
    store: Arc<BookmarkStore>,
}

impl BookmarkService {
    pub fn app_signing_key(&self, bundle_id: &str) -> [u8; 32] {
        if let Some(k) = self.cache.read().get(bundle_id) { return *k; }
        let hkdf = Hkdf::<Sha256>::new(None, &self.master_key);
        let mut out = [0u8; 32];
        let info = format!("vyoma-bookmark-v1-{}", bundle_id);
        hkdf.expand(info.as_bytes(), &mut out).expect("hkdf-expand");
        self.cache.write().insert(bundle_id.to_string(), out);
        out
    }

    pub fn mint(
        &self, actor: &ActorRef, canonical: &Path, mode: BookmarkMode,
        lifetime: BookmarkLifetime, granter: Granter,
    ) -> Result<Vec<u8>, VfsError> {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|e| VfsError::BackendError(e.to_string()))?;
        let body = BookmarkBody {
            version: 1,
            bundle_id: actor.bundle_id().to_string(),
            canonical_path: canonical.to_path_buf(),
            mount_name: "ext4".into(),
            mode,
            created_ns: now_ns(),
            expires_ns: match lifetime {
                BookmarkLifetime::Persistent => None,
                BookmarkLifetime::UntilInstanceExit => Some(now_ns() + 86_400_000_000_000),
            },
            granter,
            inode_hint: None,
            nonce,
        };
        let body_cbor = serde_cbor::to_vec(&body).unwrap();
        let key = self.app_signing_key(actor.bundle_id().as_str());
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
        mac.update(&body_cbor);
        let mac_out = mac.finalize().into_bytes();
        let tok = BookmarkToken { body_cbor, mac: mac_out.into() };
        Ok(tok.encode())
    }

    pub fn verify(&self, actor: &ActorRef, encoded: &[u8]) -> Result<BookmarkBody, VfsError> {
        let tok = BookmarkToken::decode(encoded)?;
        let body: BookmarkBody = serde_cbor::from_slice(&tok.body_cbor)
            .map_err(|_| VfsError::BookmarkInvalid)?;
        if body.bundle_id != actor.bundle_id().as_str() {
            return Err(VfsError::PermissionDenied);
        }
        if let Some(exp) = body.expires_ns {
            if now_ns() > exp { return Err(VfsError::BookmarkExpired); }
        }
        let key = self.app_signing_key(actor.bundle_id().as_str());
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
        mac.update(&tok.body_cbor);
        mac.verify_slice(&tok.mac).map_err(|_| VfsError::BookmarkInvalid)?;
        Ok(body)
    }
}
```

Forgery resistance:

- The app does not have access to its own signing key (derived in supervisor only).
- The HMAC binds the bundle_id; copying a token from app A to app B fails verification because B's
  signing key would be used to verify, but the token was MAC'd with A's key.
- Replay across reboots survives because the master key is stable.

### 8.3 Master key custody

The supervisor's master key is sealed at `/data/.vyoma/keys/master.key`, mode 0600,
owner-only (supervisor process). On first boot the key is generated via `getrandom`. On
subsequent boots it is loaded.

Threat model:

- **Within the VM, supervisor TCB:** supervisor owns the key. Apps cannot read it (no preopen
  to `/data/.vyoma/`).
- **Host-side attacker:** an attacker with access to `data/` on the host can read the key and
  forge bookmarks. This is **out of scope for v1**; mitigation requires TPM-sealed key, deferred
  to v2 (Round 63 Secure Enclave & TEE).
- **App-side attacker:** the app's bundle is gated by the supervisor's `[capabilities.filesystem_full_disk]`
  check; no app can read `/data/.vyoma/keys/`.

Critic C5 resolved with the explicit threat-model scoping.

### 8.4 Persistent bookmark store

```rust
// supervisor/src/fs/bookmark_store.rs

pub struct BookmarkStore {
    db: sled::Db,
}

impl BookmarkStore {
    pub fn save(&self, actor: &ActorRef, name: &str, tok: &[u8]) -> Result<(), VfsError> {
        let key = format!("{}::{}", actor.bundle_id(), name);
        self.db.insert(key.as_bytes(), tok)
            .map_err(|e| VfsError::BackendError(e.to_string()))?;
        self.db.flush().map_err(|e| VfsError::BackendError(e.to_string()))?;
        Ok(())
    }
    pub fn load(&self, actor: &ActorRef, name: &str) -> Result<Vec<u8>, VfsError> {
        let key = format!("{}::{}", actor.bundle_id(), name);
        self.db.get(key.as_bytes())
            .map_err(|e| VfsError::BackendError(e.to_string()))?
            .map(|v| v.to_vec())
            .ok_or(VfsError::NotFound)
    }
    pub fn list(&self, actor: &ActorRef) -> Result<Vec<String>, VfsError> {
        let prefix = format!("{}::", actor.bundle_id());
        let mut out = Vec::new();
        for item in self.db.scan_prefix(prefix.as_bytes()) {
            if let Ok((k, _)) = item {
                if let Ok(s) = std::str::from_utf8(&k) {
                    if let Some(name) = s.strip_prefix(&prefix) {
                        out.push(name.to_string());
                    }
                }
            }
        }
        Ok(out)
    }
}
```

### 8.5 Stale bookmark refresh

If the user moves the bookmarked file:

1. `bookmark.open(tok)` verifies HMAC, then stats `canonical_path`.
2. If missing or inode mismatch, the supervisor triggers a `BookmarkRefresh` flow:
   - Compositor displays a chrome banner: "*App* needs access to *file* which has moved.
     Locate it?" with a "Find" button.
   - On user click, the supervisor opens a panel constrained to the user's filesystem (full-disk
     read).
   - On selection, a new bookmark is minted with `Granter::BookmarkRefresh`.
   - The new token is delivered to the app via Round 3 IPC envelope schema `fs.bookmark_refreshed`.

---

## 9. Snapshots & COW — Fixed

### 9.1 v1 scope: clone (FICLONE) + atomic rename, NOT full snapshots

The Architect proposed userspace COW for snapshots. The Critic correctly noted this is either
file-granular (4× storage overhead) or requires implementing a userspace block layer
(thousands of lines of code, performance disaster).

The synthesis:

- **`snapshot.clone-file(src, dst)` works** via ext4 `FICLONE` (`ioctl(FICLONE)`). On ext4
  >= 4.5, this is an O(1) metadata operation; writes are CoW at the block layer in the kernel.
  This is the operation app authors actually want most of the time (save-as, document
  versioning, scratch-copy).
- **`snapshot.create(root, label)` returns `Unsupported("snapshots-defer-v2")`** in v1. We do
  not promise an APFS-equivalent snapshot of a subtree.
- **For "snapshot-like" semantics** (e.g., before a risky edit), apps use the atomic-rename
  pattern: write new content to a tempfile in the same dir, then `rename` over the target.
  This is the proven Unix idiom and works at file granularity.
- **v2 plan:** enable `CONFIG_BTRFS_FS` in the kernel build (cost: ~1 MB), expose btrfs
  subvolumes as a separate backend (`Btrfs Backend`), wire `snapshot.create` to
  `BTRFS_IOC_SNAP_CREATE_V2`. This is a Round 45 (Time Machine & Snapshots) topic.

### 9.2 Implementation: `clone-file` via FICLONE

```rust
// supervisor/src/fs/clone.rs

use std::os::unix::io::AsRawFd;

pub fn clone_file_ext4(src: &cap_std::fs::File, dst: &cap_std::fs::File)
    -> Result<(), VfsError>
{
    let src_fd = src.as_raw_fd();
    let dst_fd = dst.as_raw_fd();
    let rc = unsafe {
        libc::ioctl(dst_fd, libc::FICLONE as _, src_fd)
    };
    if rc < 0 {
        return Err(VfsError::IoError(std::io::Error::last_os_error()));
    }
    Ok(())
}
```

The cost: `O(metadata)`, typically <1 ms. Storage overhead: zero until divergent writes
trigger ext4's kernel-level CoW.

### 9.3 Atomic-rename pattern

For apps that want save-via-swap (the macOS Documents idiom), we expose a helper in the
client-side library `apps/_lib/vyoma-fs-client/`:

```rust
// apps/_lib/vyoma-fs-client/src/atomic.rs

pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut tmp = parent.join(format!(".tmp-{}-{}", std::process::id(), rand_u32()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, content)?;
        f.sync_data()?;
    }
    std::fs::rename(&tmp, path)?;
    let parent_f = std::fs::File::open(parent)?;
    parent_f.sync_data()?;
    Ok(())
}
```

This is what apps SHOULD use for "save-with-rollback". Documented as the v1 idiom.

### 9.4 Sparse files

`fallocate` with `FALLOC_FL_PUNCH_HOLE` is supported on ext4. Exposed via
`vyoma:fs/types.descriptor.punch_hole(offset, len)` (added as part of the WriteShim host impl).
Reads from holes return zeros without disk I/O.

---

## 10. Quotas, Tags & Metadata

### 10.1 Quota ledger

```rust
// supervisor/src/fs/quota.rs

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

pub struct QuotaCell {
    pub bytes_used:  AtomicU64,
    pub bytes_limit: AtomicU64,
    pub files_used:  AtomicU64,
    pub files_limit: AtomicU64,
}

pub struct QuotaLedger {
    cells: dashmap::DashMap<(BundleId, String /* mount */), Arc<QuotaCell>>,
    wal: Arc<QuotaWal>,
    pending: parking_lot::Mutex<Vec<QuotaDelta>>,
}

#[derive(Clone, Debug)]
pub struct QuotaDelta {
    pub bundle: BundleId,
    pub mount: String,
    pub bytes: i64,
    pub files: i32,
}

impl QuotaLedger {
    pub fn try_reserve(&self, bundle: &BundleId, mount: &str, n: u64) -> Result<(), VfsError> {
        let cell = self.cell(bundle, mount);
        let limit = cell.bytes_limit.load(Ordering::Relaxed);
        loop {
            let cur = cell.bytes_used.load(Ordering::Relaxed);
            let new = cur.saturating_add(n);
            if new > limit {
                return Err(VfsError::QuotaExceeded { used: cur, limit });
            }
            if cell.bytes_used.compare_exchange_weak(
                cur, new, Ordering::AcqRel, Ordering::Relaxed,
            ).is_ok() {
                return Ok(());
            }
        }
    }
    pub fn commit_reservation(&self, bundle: &BundleId, mount: &str, n: u64) {
        // Already accounted; just append to WAL delta queue.
        self.pending.lock().push(QuotaDelta {
            bundle: bundle.clone(), mount: mount.into(),
            bytes: n as i64, files: 0,
        });
    }
    pub fn refund(&self, bundle: &BundleId, mount: &str, n: u64) {
        let cell = self.cell(bundle, mount);
        cell.bytes_used.fetch_sub(n, Ordering::Release);
        self.pending.lock().push(QuotaDelta {
            bundle: bundle.clone(), mount: mount.into(),
            bytes: -(n as i64), files: 0,
        });
    }
    fn cell(&self, bundle: &BundleId, mount: &str) -> Arc<QuotaCell> {
        self.cells.entry((bundle.clone(), mount.into()))
            .or_insert_with(|| {
                Arc::new(QuotaCell {
                    bytes_used:  AtomicU64::new(0),
                    bytes_limit: AtomicU64::new(256 * MIB),  // default
                    files_used:  AtomicU64::new(0),
                    files_limit: AtomicU64::new(100_000),
                })
            }).clone()
    }
}
```

### 10.2 Persistence and recovery

A delta-log WAL is flushed every 100 ms; a full-cell snapshot is written every 60 s and on
clean shutdown. On supervisor crash:

1. Load most recent snapshot.
2. Apply WAL deltas since snapshot.
3. If any cell appears inconsistent (negative `bytes_used`), schedule a background full walk of
   that bundle's mount; flag the cell as `under_audit` until walk completes.

### 10.3 Tags via xattrs

Tags stored as CBOR-encoded `Vec<Tag>` in `user.tags`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tag { pub name: String, pub color: TagColor }

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum TagColor { Red, Orange, Yellow, Green, Blue, Purple, Gray, None }
```

A sled-backed inverted index at `/data/.vyoma/tag-index.db` maps `tag_name → Vec<(inode, path)>`.
Updates are async — on `xattr.set("user.tags", ...)` the supervisor enqueues an index update.

### 10.4 xattr namespaces

| Prefix | Owner | Examples |
|--------|-------|----------|
| `vyoma.*` | Supervisor only (PermissionDenied for apps) | `vyoma.trash.original-path`, `vyoma.trash.timestamp`, `vyoma.spotlight.indexed-at` |
| `user.*` | Any app with access | `user.tags`, `user.color-label`, `user.notes-summary` |
| `com.<bundle>.*` | App-private | `com.example.notes.last-cursor` |

### 10.5 Spotlight-style indexing

`vyoma.spotlight.*` xattrs are written by extractor apps (`apps/indexer-*`), each running with
`xattr` capability. Each extractor subscribes to FsEvents on `/data/` and writes:

| Key | Value |
|-----|-------|
| `vyoma.spotlight.text` | Plain text extract |
| `vyoma.spotlight.preview` | 256×256 PNG thumbnail |
| `vyoma.spotlight.kind` | content kind (`text/markdown`, `image/png`, ...) |
| `vyoma.spotlight.summary` | One-line summary |

Full Spotlight subsystem is Round 42; this is the FS-layer plumbing.

---

## 11. Implementation Files

All files **≤ 500 LOC**. Total: ~6,400 LOC across 22 files.

| File | LOC | Purpose |
|------|-----|---------|
| `supervisor/src/fs/mod.rs` | 380 | `Vfs` struct, public entry points, wiring |
| `supervisor/src/fs/vfs.rs` | 440 | `VfsBackend` trait, `BackendPath`, `OpenFlags`, errors, `BackendCaps` |
| `supervisor/src/fs/sandbox.rs` | 410 | `SandboxMap`, `SandboxBuilder`, default sandbox table |
| `supervisor/src/fs/wasi_shim.rs` | 460 | `VfsWasiBuilder`, perms mapping, large-read SharedBuffer publish |
| `supervisor/src/fs/write_shim.rs` | 280 | `WriteShim` host impl: `descriptor.write` + `set-size` override |
| `supervisor/src/fs/path_index.rs` | 220 | `PathBucket` LRU cache for vyoma:fs path resolution |
| `supervisor/src/fs/watcher.rs` | 470 | `WatcherCore`, `WatcherBackend` trait, subscription registry |
| `supervisor/src/fs/watcher_inotify.rs` | 360 | `Ext4InotifyBackend` |
| `supervisor/src/fs/watcher_poll.rs` | 250 | `NinePPollBackend` |
| `supervisor/src/fs/watcher_coalesce.rs` | 230 | `Coalescer`, merge rules |
| `supervisor/src/fs/coordination.rs` | 470 | `CoordinationService`, locks, waiters, priority inheritance |
| `supervisor/src/fs/coord_wal.rs` | 320 | `CoordWal`, append/replay, crash recovery |
| `supervisor/src/fs/panel.rs` | 380 | `PanelService`, spawn/dispatch panel app, session table |
| `supervisor/src/fs/bookmark.rs` | 440 | `BookmarkService`, mint/verify, HMAC, HKDF, refresh flow |
| `supervisor/src/fs/bookmark_store.rs` | 240 | sled-backed persistent bookmarks |
| `supervisor/src/fs/xattr.rs` | 320 | `XattrStore`, shadow DB, namespace check |
| `supervisor/src/fs/quota.rs` | 380 | `QuotaLedger`, CAS try-reserve, refund, WAL deltas |
| `supervisor/src/fs/quota_wal.rs` | 240 | Quota WAL, snapshot every 60s, recovery |
| `supervisor/src/fs/trash.rs` | 290 | `TrashService`, trash/untrash/sweeper |
| `supervisor/src/fs/clone.rs` | 180 | FICLONE wrapper, atomic-rename helper |
| `supervisor/src/fs/backends/mod.rs` | 60 | Backend registry, mount table |
| `supervisor/src/fs/backends/ext4.rs` | 470 | Primary backend; format+mount+cap-std root |
| `supervisor/src/fs/backends/tmpfs.rs` | 360 | In-RAM backend for `/tmp` |
| `supervisor/src/fs/backends/ninep.rs` | 280 | Legacy `/host` backend, RO default |
| `supervisor/src/fs/backends/sys_ro.rs` | 220 | Read-only backend for `/sys/*` |
| `wit/vyoma-fs.wit` | 380 | Full WIT package |
| `apps/_lib/vyoma-fs-client/src/lib.rs` | 320 | App-side Rust wrapper over generated WIT bindings |
| `apps/_lib/vyoma-fs-client/src/atomic.rs` | 150 | Atomic-rename helper |
| `apps/panel/src/main.rs` | 480 | Built-in panel app (chrome + filesystem_full_disk) |
| `apps/panel/src/sidebar.rs` | 220 | Panel sidebar (Recents, Documents, ...) |
| `apps/panel/src/grid.rs` | 280 | Panel tile/list view |

**Total new:** ~6,400 LOC across 31 source files (24 supervisor + 5 wit/client + 3 panel).
Every file under the 500-line ceiling.

**Modified:** 7 files, ~360 added LOC (`main.rs` +60 for VfsView wiring, `boot.rs` +50 for
phase ordering, `lifecycle/actor.rs` +30 for on_instance_gone hooks, `manifest.rs` +90 for
new capability fields, `process_table.rs` +40 for WriteShim attachment, `mount.rs` +50 for
9P demotion to `/host`, `Cargo.toml` +40 for sled, cap-std, hkdf, hmac, sha2, inotify,
serde_cbor).

### 11.1 Module wiring

```rust
// supervisor/src/fs/mod.rs (entry point)

pub fn init(boot: &BootContext) -> Result<Arc<Vfs>, VfsError> {
    // 1. Open backends.
    let ext4 = Ext4Backend::open_or_init(&boot.disk_image, Path::new("/data"))?;
    let tmpfs = TmpfsBackend::mount(Path::new("/tmp"))?;
    let sys_ro = SysRoBackend::open(Path::new("/sys"))?;
    let ninep = if boot.host_9p_available {
        Some(NinePBackend::mount(Path::new("/host"))?)
    } else { None };

    // 2. Open supervisor-side stores.
    let xattrs = Arc::new(XattrStore::open("/data/.vyoma/xattrs.db")?);
    let quotas = Arc::new(QuotaLedger::open("/data/.vyoma/quota.db")?);
    let coord_wal = Arc::new(CoordWal::open("/data/.vyoma/coord.wal".into())?);

    // 3. Replay coord WAL.
    let alive_filter = |iid| boot.process_table.is_alive(iid);
    let recovered_locks = coord_wal.replay(&alive_filter)?;
    let coord = Arc::new(CoordinationService::new(
        boot.wait_graph.clone(), coord_wal.clone(),
    ));
    coord.reinstate(recovered_locks);

    // 4. Bookmarks (with sealed master key).
    let master_key = load_or_create_master_key("/data/.vyoma/keys/master.key")?;
    let bookmarks = Arc::new(BookmarkService::new(
        master_key, BookmarkStore::open("/data/.vyoma/bookmarks.db")?,
    ));

    // 5. Watcher core.
    let mut watch_backends: HashMap<String, Arc<dyn WatcherBackend>> = HashMap::new();
    watch_backends.insert("ext4".into(), Arc::new(Ext4InotifyBackend::new()?));
    watch_backends.insert("tmpfs".into(), Arc::new(Ext4InotifyBackend::new()?));
    if let Some(_) = &ninep {
        watch_backends.insert("9p".into(), Arc::new(NinePPollBackend::new(
            std::time::Duration::from_millis(250), ninep.clone().unwrap(),
        )));
    }
    let watcher = Arc::new(WatcherCore::new(watch_backends, boot.router.clone()));

    // 6. Trash + panel.
    let trash = Arc::new(TrashService::new(30 /* days */));
    let panel = Arc::new(PanelService::new(
        boot.compositor.clone(), boot.spawner.clone(), bookmarks.clone(),
    ));

    // 7. MountTable.
    let mut mounts = MountTable::new();
    mounts.add(Mount {
        guest_path: PathBuf::from("/data"),
        backend: ext4.clone(),
        default_flags: MountFlags { read_only: false, no_atime: true, ..Default::default() },
    });
    mounts.add(Mount {
        guest_path: PathBuf::from("/tmp"),
        backend: tmpfs.clone(),
        default_flags: MountFlags::default(),
    });
    mounts.add(Mount {
        guest_path: PathBuf::from("/sys"),
        backend: sys_ro.clone(),
        default_flags: MountFlags { read_only: true, ..Default::default() },
    });
    if let Some(n) = ninep {
        mounts.add(Mount {
            guest_path: PathBuf::from("/host"),
            backend: n,
            default_flags: MountFlags { read_only: true, ..Default::default() },
        });
    }

    let vfs = Arc::new(Vfs {
        mounts: ArcSwap::new(Arc::new(mounts)),
        xattrs, quotas, watcher, coordination: coord, bookmarks, trash, panel,
    });

    // 8. Lifecycle hooks.
    boot.lifecycle.on_instance_gone({
        let vfs = vfs.clone();
        Arc::new(move |iid| vfs.on_instance_gone(iid))
    });
    boot.lifecycle.on_manifest_reloaded({
        let vfs = vfs.clone();
        Arc::new(move |bundle, snap| vfs.on_manifest_reloaded(bundle, snap))
    });
    Ok(vfs)
}
```

### 11.2 Boot phase ordering

```
BootPhase::Mounted9P            (Round 1) — virtio-9P available if -virtfs
BootPhase::DiskImageReady       (NEW)     — virtio-blk disk.img formatted + fsck'd
BootPhase::FilesystemMounted    (Round 4) — ext4 mounted at /data; tmpfs at /tmp
BootPhase::XattrStoreReady      (Round 4) — sled DBs open
BootPhase::CoordReplayed        (Round 4) — coord WAL replayed
BootPhase::WatcherReady         (Round 4) — WatcherCore + inotify backends
BootPhase::IpcReady             (Round 3) — sharded router online
BootPhase::DisplayReady         (Round 1) — compositor up
BootPhase::PanelReady           (Round 4) — panel app spawnable
BootPhase::AppsLaunched         (Round 1) — first user app starts
```

### 11.3 Test plan

| Test | Scope | Location |
|------|-------|----------|
| Preopen sandbox enforcement | escape via `..`, symlinks, hardlinks | `supervisor/tests/fs_sandbox.rs` |
| WriteShim quota | preflight reject, partial-write QuotaExceeded | `supervisor/tests/fs_quota.rs` |
| WriteShim coordination | write while lock held | `supervisor/tests/fs_coord.rs` |
| `FICLONE` | clone + divergent write isolation | `supervisor/tests/fs_clone.rs` |
| Inotify backend conformance | create/modify/delete sequence | `supervisor/tests/fs_watcher_inotify.rs` |
| Poll backend on 9P | synthetic events under load | `supervisor/tests/fs_watcher_poll.rs` |
| Coalescer | windowing, merge rules | `supervisor/tests/fs_coalesce.rs` |
| Coord lock acquire/release | RW semantics, priority inheritance | `supervisor/tests/fs_coord.rs` |
| Coord cycle detection | A→B→A | `supervisor/tests/fs_coord_cycle.rs` |
| Coord crash recovery (app) | on_instance_gone drops locks | `supervisor/tests/fs_coord_crash_app.rs` |
| Coord crash recovery (supervisor) | WAL replay correctness | `supervisor/tests/fs_coord_crash_super.rs` |
| Bookmark HMAC | forge with B's key, fail | `supervisor/tests/fs_bookmark_forge.rs` |
| Bookmark expiry | expired, refused | `supervisor/tests/fs_bookmark_expiry.rs` |
| Bookmark refresh | stale, refresh flow end-to-end | `supervisor/tests/fs_bookmark_refresh.rs` |
| Panel end-to-end | spawn panel, simulate selection, redeem bookmark | `supervisor/tests/fs_panel_e2e.rs` |
| Trash + sweep | trash, sweep after retention | `supervisor/tests/fs_trash.rs` |
| 9P-only-degraded boot | ext4 missing, tmpfs fallback | `supervisor/tests/fs_degraded.rs` |
| WAL integrity after kill -9 | mid-write crash | `supervisor/tests/fs_wal_crash.rs` |

Smoke test: a new WASM app `apps/fs-smoke/` exercises every WIT entry point and asserts
expected results; included in `make smoke`.

---

## 12. Performance Budget

Target latencies on x86-64 desktop profile, warm caches:

| Operation | p50 | p99 | Notes |
|-----------|-----|-----|-------|
| `wasi:filesystem.read` (4 KB, ext4 page-cache) | 2 µs | 8 µs | Stock Wasmtime, supervisor cold |
| `wasi:filesystem.write` (4 KB, WriteShim) | 3 µs | 12 µs | + CAS + coord hash lookup |
| `wasi:filesystem.read` (64 KB, SharedBuffer) | 6 µs | 25 µs | Zero-copy publish |
| `wasi:filesystem.open` (ext4, file exists) | 6 µs | 30 µs | `openat(2)` + cap-std check |
| `wasi:filesystem.stat` | 3 µs | 15 µs | `fstatat(2)` |
| `vyoma:fs/xattr.get` (native ext4) | 4 µs | 15 µs | `getxattr` |
| `vyoma:fs/xattr.get` (shadow sled) | 5 µs | 20 µs | sled lookup |
| `vyoma:fs/snapshot.clone-file` | 1 ms | 5 ms | ioctl(FICLONE) |
| `vyoma:fs/watcher.subscribe` | 12 µs | 40 µs | inotify add_watch + table insert |
| `fs.watch-event` delivery (coalesced) | 200 µs | 2 ms | 100 ms tumbling window |
| `vyoma:fs/coord.coordinate-write` (uncontended) | 2 µs | 8 µs | Mutex + insert + WAL append |
| `vyoma:fs/coord.coordinate-write` (contended) | window + RTT | bounded | priority-inherited |
| `vyoma:fs/bookmark.open` | 15 µs | 60 µs | Verify HMAC + open |
| `vyoma:fs/panel.open` (first frame) | 80 ms | 400 ms | Panel-app spawn + render |
| `vyoma:fs/trash.trash` | 100 µs | 1 ms | rename + 2 xattr writes |

System-wide ceiling on ext4 + virtio-blk: 200 k+ ops/sec on a 4-core x86 desktop (vs. 666
ops/sec on 9P). Comfortably absorbs the Critic-projected 860 ops/sec aggregate demand.

---

## 13. Failure Modes and Degraded Operation

### 13.1 Disk image missing or corrupt

1. `Ext4Backend::open_or_init` reformats if missing; reports `BootPhase::DiskImageReady` after
   format succeeds (10-30 s on first boot).
2. If `mkfs.ext4` fails (no space), supervisor enters degraded mode: tmpfs at `/data`, all
   writes ephemeral, package manager refuses installs, banner displayed.

### 13.2 Coord WAL corruption

1. WAL parse error → rename to `coord.wal.broken.<ts>`, start fresh.
2. All in-flight locks lost; apps see `IpcError::SenderGone` on outstanding waiters.

### 13.3 Bookmark store corruption

1. sled integrity-check fails → rename `bookmarks.db.broken.<ts>`, start fresh.
2. All persistent bookmarks lost; apps' `bookmark.load(name)` returns `NotFound`.
3. UX: chrome banner "Bookmark store reset after corruption; re-grant file access via panels."

### 13.4 Quota DB corruption

1. WAL/snapshot inconsistent → schedule full walk per bundle to recompute usage (30+ s).
2. Until walk completes, the affected bundle's writes are restricted to `bytes_limit / 2` as a
   safety margin.

### 13.5 Inotify watch budget exhaustion

Linux `/proc/sys/fs/inotify/max_user_watches` is finite (default 8 K). When exhausted:

1. New `watcher.subscribe` returns `Unsupported("watch-limit-reached")`.
2. Existing watches continue.
3. We do NOT auto-bump the sysctl (would require silent root operation).

### 13.6 Coordination cycle between FS and IPC

The Round 3 wait-for graph spans `Call` and `CoordLock` edges. A cycle detection at
`coord.acquire` returns `VfsError::CoordinationCycle` synchronously. A cycle detection at
`ipc.call` returns `IpcError::WouldDeadlock`. There is no deferred deadlock detection.

### 13.7 Supervisor crash recovery summary

| State | Volatile | Persisted (WAL/DB) | Recovery |
|-------|----------|--------------------|----------|
| Quota cells | atomic counters | delta WAL + 60 s snapshot | Replay; full walk if inconsistent |
| Coord locks | parking_lot table | append WAL per op | Replay; 5 s grace for instance reattach |
| Bookmarks | n/a | sled DB | Re-open |
| Xattrs (shadow) | n/a | sled DB | Re-open |
| Tag index | n/a | sled DB | Re-open; rebuild from xattrs if corrupt |
| Watchers | dashmap, inotify fd | none | Re-establish; apps must re-subscribe |
| Panel sessions | parking_lot table | none | Cancelled; apps see `CoordinationTimeout` |

---

## Critical v1 Requirements

1. virtio-blk ext4 image at `/data` is the primary persistent backend.
2. 9P, if present, is `/host` and is read-only by default.
3. Sandbox enforcement is preopen-driven via `WasiCtxBuilder::preopened_dir`.
4. Quotas, coordination, watcher hooks are the ONLY supervisor-side overrides on the WASI hot
   path; everything else flows through stock `wasmtime-wasi`.
5. `vyoma:fs@0.1.0` WIT package exports xattr, snapshot (clone-only), watcher, coord, panel,
   bookmark, trash, quota interfaces.
6. Bookmarks are HMAC-SHA-256 over `(canonical_path, expiry, nonce, mode)` keyed by per-bundle
   HKDF-derived signing key from supervisor master key.
7. Coordination locks survive supervisor crash via WAL replay; survive app crash via
   `on_instance_gone`.
8. Watchers use `Ext4InotifyBackend` for `/data`+`/tmp`; `NinePPollBackend` for `/host`.
9. Panel renders in a built-in WASM app with `filesystem_full_disk` capability; supervisor mints
   bookmarks scoped to the requester.
10. Trash is per-bundle at `/data/<bundle>/.Trash/`; 30 d retention sweep.
11. Atomic-rename pattern is the v1 "snapshot-like" idiom; exposed via client library.
12. ext4 `FICLONE` powers `vyoma:fs/snapshot.clone-file`.
13. `BootPhase::FilesystemMounted` runs after `Mounted9P` + `DiskImageReady`, before `IpcReady`.

## Deferred to v2

1. **Full subtree snapshots** (`snapshot.create`): requires btrfs subvolumes in kernel build.
2. **Spotlight full-text search**: this round wires xattr plumbing; the indexer + query
   engine is Round 42.
3. **TPM-sealed bookmark master key**: requires Round 63 Secure Enclave.
4. **Multi-user**: requires Round 49 (App Sandbox & Container FS) finalized.
5. **Per-volume encryption**: Round 64 Full Disk Encryption.
6. **Network mounts** (NFS, SMB, WebDAV): Round 51 Networking + future backend.
7. **Spotlight Quick Look in panel**: requires Round 72 PDF + Round 71 Image Processing.
8. **Compression / dedup at FS layer**: would require btrfs or zfs.
9. **Stream-as-SharedBuffer** for large *writes*: round 4 only does it for reads.

## Explicitly NEVER

1. **9P as primary `/data`** — too slow, no inotify, no atomic primitives.
2. **Runtime path rewriting** in the supervisor for WASI calls — contradicts capability model.
3. **Userspace block-layer COW** in supervisor — wrong abstraction layer.
4. **Bookmark grants via stdout/IPC line protocol** — forgeable; requires HMAC.
5. **Case-insensitive Unicode-folding paths in supervisor** — unaffordable; we are Linux-native.
6. **Coordination as advisory-only** — locks are mandatory at the WriteShim layer.
7. **Per-app mount namespaces** (CAP_SYS_ADMIN per-app) — preopens deliver the same isolation
   at lower cost.
8. **A separate WASI host impl re-implementation** — we override only `write` + `set-size`.
9. **Block-level snapshot accounting** — defer to btrfs/zfs in v2.
10. **Reading `vyoma.*` xattrs from user apps** — system-only namespace.
