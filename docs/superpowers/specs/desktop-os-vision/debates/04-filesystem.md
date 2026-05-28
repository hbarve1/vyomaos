# Round 4 Architect Proposal: File System & VFS Layer

**Status:** Architect draft (2026-05-29)
**Subsystem:** Supervisor-mediated Virtual Filesystem for `wasm32-wasip2` apps under Wasmtime PID-1
**macOS equivalent:** APFS + VFS layer + `NSFileManager` + `NSOpenPanel` / `NSSavePanel` + `NSFileCoordinator` + FSEvents + Powerbox + xattrs + Trash
**Depends on:** Round 1 (`AppIdentity`, `InstanceId`, `BootPhase`), Round 2 (`SharedBuffer`, PSI), Round 3 (`IpcEnvelope`, sharded router, `ArcSwap<PolicySnapshot>`, WIT `vyoma:ipc@0.1.0`)

---

## 0. Executive Summary

The filesystem layer is the most user-visible and most security-critical surface in a desktop OS. macOS achieves a coherent experience by stacking five distinct subsystems on top of a single on-disk format (APFS):

1. A **VFS kernel layer** that abstracts over APFS / NFS / SMB / FAT / etc.
2. **`NSFileManager`** as the canonical app-facing API (Foundation), with sandbox containers under `~/Library/Containers/<bundle-id>/`.
3. **`NSOpenPanel` / `NSSavePanel`** rendered out-of-process by Powerbox (`pboxd`), returning security-scoped bookmarks so a sandboxed app gets exactly the path the user picked and nothing more.
4. **`NSFileCoordinator`** + `NSFilePresenter` for cooperative multi-process file access (used by iCloud, Versions, autosave).
5. **FSEvents** for change notifications and **xattrs** for tags, quarantine, Spotlight metadata.

VyomaOS must reproduce all five — on a Linux 5.10 kernel with 9P-virtio as the only persistent backend — without giving WASM apps direct kernel filesystem access (the supervisor is PID 1 and the only process with mount privileges). The WASI Preview 2 `wasi:filesystem/types@0.2.0` interface gives apps a familiar `open`/`read`/`write`/`readdir` surface, but it stops at single-file I/O. It has no concept of:

- Snapshots, clones, sparse files, xattrs
- Path sandboxing beyond the preopened-directory model
- File watching, file coordination, open/save panels
- Security-scoped bookmarks, trash, tags, quotas

This document proposes a **two-layer architecture**: a `Vfs` mediation layer that owns path resolution + sandboxing + xattrs + quotas + watching, sitting on top of pluggable `VfsBackend` implementations (9P, tmpfs, overlay). Apps reach the VFS through (a) the standard WASI P2 filesystem interface (for ordinary `open`/`read`/`write`) and (b) a new `vyoma:fs@0.1.0` WIT package for the macOS-equivalent richness (watchers, panels, coordination, bookmarks, xattrs, snapshots).

The design integrates with Round 1–3:

- **Path resolution is policy-snapshot-aware**: each app's sandbox map lives in `ArcSwap<PolicySnapshot>` (Round 3, §8). Capability changes hot-swap atomically; in-flight reads finish on the old snapshot.
- **Watcher events ride the IPC fabric**: `FsEvent` is an `IpcEnvelope` body delivered via the per-receiver dispatcher thread (Round 3, §2), with the same `BusyDeferred` semantics during `on-ipc` (Round 3, §7).
- **Large reads use `SharedBuffer`**: any read ≥ 64 KB or any panel-returned thumbnail rides `SharedBuffer` rather than copying through the WASM linear heap (Round 2, §`SharedBuffer`; Round 3, §6).
- **Panels and coordination respect `BootPhase`**: open-panel cannot be invoked before `BootPhase::DisplayReady`; coordination locks are released on `InstanceGone` via the lifecycle hooks (Round 1).

Total new code budget: **~6,200 LOC across 22 files**, all under the 500-line ceiling.

---

## Key Decisions

1. **Two-layer architecture: `Vfs` mediator + pluggable `VfsBackend`.** `Vfs` owns sandboxing, xattrs, quotas, watching, coordination. Backends own raw byte storage. Initial backends: `NinePBackend` (current `/data`), `TmpfsBackend` (RAM-backed, ephemeral), `OverlayBackend` (read-only lower + per-app writable upper, used for `/apps/<bundle>/` and snapshots), `SysReadOnlyBackend` (for `/sys/fonts`, `/sys/icons`, `/sys/themes`).

2. **Path namespace is a per-app `SandboxMap`, not chroot.** Each WASM instance sees a logical tree (`/data`, `/tmp`, `/apps`, `/sys`); the supervisor translates every path through a snapshot-resident `SandboxMap` before handing it to a backend. Escape via `..` or symlink is impossible by construction: resolution returns a `BackendPath` only if the canonicalized result lies under a `Mount.allowed_root`.

3. **WASI P2 preopens come from the same `SandboxMap`.** At instance launch the supervisor walks the sandbox map and calls `WasiCtxBuilder::preopened_dir` for every directory that the manifest's `[capabilities]` permits. No invisible writes; what the manifest declares is exactly what `wasi:filesystem` sees.

4. **xattrs are first-class, even when the backend doesn't store them.** A supervisor-side `XattrStore` (sled-backed key-value at `/data/.sys/xattrs.db`) shadows xattrs for backends that don't support them natively (9P, FAT). Backends that do (ext4, future btrfs) get write-through. The `xattr` namespace is `vyoma.*` for system, `user.*` for apps, `com.<bundle>.*` for app-private.

5. **Snapshots and clones are copy-on-write at the VFS layer.** On `clone_file`, the VFS records a `CloneEdge` in `XattrStore` (`vyoma.clone.parent = <inode>`); reads from either side follow the edge until either side is modified, at which point the modified side is materialized. On 9P (no reflink) this is emulated; on ext4/btrfs it lowers to `FICLONE`. Snapshots are batch clones over a subtree.

6. **File watching is supervisor-mediated inotify + 9P poll.** A single `WatcherCore` thread owns one `inotify` fd per backend that supports it (Linux ext4, overlay, tmpfs) and a 250 ms poll loop for backends that don't (9P). Events are coalesced in a 100 ms tumbling window per `(watch_id, path, kind)` tuple and delivered as `FsEvent` IPC envelopes on the `Normal` class with `coalesce` overflow policy (Round 3 §4).

7. **File coordination uses the existing IPC waiter table.** `coordinate_write(path)` is a request-reply IPC call with timeout; the supervisor maintains a `CoordinationLock` table keyed by canonical backend path. Reader-writer semantics. Priority inheritance: a writer waiting on a reader inherits the writer's priority class on the reader's dispatcher. Deadlock detection reuses the Round 3 wait-for graph.

8. **Open/Save panels render in the supervisor's chrome process and return security-scoped bookmarks, not paths.** The panel is a privileged supervisor-owned window. The app passes a `PanelRequest`; the supervisor returns a `BookmarkToken` that, when redeemed via `vfs.open_bookmark(token)`, grants the app a file handle but does **not** add the path to its sandbox map (Powerbox pattern). Bookmarks are MAC'd with the supervisor's session key.

9. **Quotas are hard-checked at every write, with per-block accounting.** Per-app `storage_quota_mb` from manifest (default: 256 MB). VFS maintains a `QuotaLedger` indexed by `(bundle_id, mount)`; every `write`, `truncate`, `clone_file`, and `rename` (cross-mount) is gated by the ledger. Ledger updates are batched per 100 ms and persisted to `/data/.sys/quota.db`.

10. **Trash is a per-bundle directory under `/data/<bundle>/.Trash/`.** `trash(path)` is atomic rename + xattr stamp (`vyoma.trash.original-path`, `vyoma.trash.timestamp`). A background `TrashSweeper` task purges entries older than the configured retention (default 30 d). `untrash(token)` is a guarded rename back, refusing if the destination exists.

11. **Bookmarks are signed and self-contained.** A `BookmarkToken` is a CBOR-encoded `(bundle_id, backend_path, expiry, scope, hmac)` tuple, HMAC-SHA-256'd with a supervisor session key. The token is opaque to apps; the supervisor re-resolves and re-authorizes on every redemption.

12. **Case-insensitive + NFC-normalizing path matching at the VFS layer.** All lookups are case-folded (Unicode `toCasefold`) and NFC-normalized before hashing the path. The on-disk representation preserves the user's original casing (like APFS default). A `PathBucket` index in `XattrStore` maps `casefold(nfc(path))` → canonical path.

---

## 1. VFS Architecture

### 1.1 Topology

```
┌──────────────────────────────────────────────────────────────────────┐
│                        WASM apps (wasm32-wasip2)                     │
│   wasi:filesystem/types@0.2.0     vyoma:fs/* (this round)           │
└──────────────────┬─────────────────────────┬───────────────────────┘
                   │ open/read/write/stat    │ watch/panel/coord/snapshot
                   ▼                         ▼
┌──────────────────────────────────────────────────────────────────────┐
│                    Supervisor: Vfs mediator                          │
│  ┌────────────┐ ┌────────────┐ ┌─────────────┐ ┌─────────────────┐ │
│  │ SandboxMap │ │ XattrStore │ │ QuotaLedger │ │ CoordinationLk  │ │
│  └────────────┘ └────────────┘ └─────────────┘ └─────────────────┘ │
│  ┌──────────────────┐ ┌──────────────────┐ ┌──────────────────────┐│
│  │ WatcherCore      │ │ PanelService     │ │ BookmarkService     ││
│  └──────────────────┘ └──────────────────┘ └──────────────────────┘│
└──────────────────────────┬───────────────────────────────────────────┘
                           │ trait VfsBackend
        ┌──────────────────┼──────────────────┬──────────────────┐
        ▼                  ▼                  ▼                  ▼
┌──────────────┐  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
│ NinePBackend │  │ TmpfsBackend │   │ OverlayBkend │   │ SysROBackend │
│   /data      │  │   /tmp       │   │   /apps      │   │   /sys       │
└──────────────┘  └──────────────┘   └──────────────┘   └──────────────┘
```

### 1.2 Core trait

```rust
// supervisor/src/fs/vfs.rs

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Backend-level path. NOT app-visible. Always absolute under
/// the backend's root, canonicalized, no `..`, no symlinks.
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

/// Open flags. Subset of POSIX with explicit semantics.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,      // O_EXCL
    pub truncate: bool,
    pub append: bool,
    pub sync: bool,           // O_SYNC
    pub no_follow: bool,      // O_NOFOLLOW for the final component
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FileKind { Regular, Directory, Symlink, Device, Fifo }

#[derive(Clone, Debug)]
pub struct FileStat {
    pub kind: FileKind,
    pub size: u64,
    pub allocated: u64,        // physical bytes; sparse files: < size
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub atime_ns: i64,
    pub btime_ns: i64,         // birth time, like APFS
    pub mode: u32,
    pub nlink: u32,
    pub inode: u64,            // backend-stable inode
    pub blocks: u64,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,          // NFC-normalized for storage; original preserved in xattr `vyoma.original-name`
    pub kind: FileKind,
    pub inode: u64,
}

/// Opaque file handle owned by a backend. The Vfs wraps this with
/// quota+coordination accounting before exposing to the app.
pub trait VfsFile: Send + Sync {
    fn read(&self, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError>;
    fn write(&self, offset: u64, buf: &[u8]) -> Result<usize, VfsError>;
    fn flush(&self) -> Result<(), VfsError>;
    fn sync(&self) -> Result<(), VfsError>;
    fn truncate(&self, len: u64) -> Result<(), VfsError>;
    fn stat(&self) -> Result<FileStat, VfsError>;
    /// Returns Some(slice) when the backend can give a zero-copy mmap; None otherwise.
    fn mmap_region(&self, offset: u64, len: u64) -> Option<MmapRegion>;
}

#[derive(Clone)]
pub struct MmapRegion {
    pub ptr: *const u8,
    pub len: usize,
    pub writable: bool,
    pub _keepalive: Arc<dyn std::any::Any + Send + Sync>,
}
unsafe impl Send for MmapRegion {}
unsafe impl Sync for MmapRegion {}

pub trait VfsBackend: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> BackendCaps;

    fn open(&self, path: &BackendPath, flags: OpenFlags) -> Result<Box<dyn VfsFile>, VfsError>;
    fn stat(&self, path: &BackendPath) -> Result<FileStat, VfsError>;
    fn readdir(&self, path: &BackendPath) -> Result<Vec<DirEntry>, VfsError>;
    fn mkdir(&self, path: &BackendPath, mode: u32) -> Result<(), VfsError>;
    fn unlink(&self, path: &BackendPath) -> Result<(), VfsError>;
    fn rmdir(&self, path: &BackendPath) -> Result<(), VfsError>;
    fn rename(&self, from: &BackendPath, to: &BackendPath) -> Result<(), VfsError>;
    fn truncate(&self, path: &BackendPath, len: u64) -> Result<(), VfsError>;
    fn symlink(&self, target: &str, link: &BackendPath) -> Result<(), VfsError>;
    fn readlink(&self, path: &BackendPath) -> Result<String, VfsError>;

    // Optional capabilities; default impls return BackendCaps-gated errors.
    fn clone_file(&self, src: &BackendPath, dst: &BackendPath) -> Result<(), VfsError> {
        Err(VfsError::Unsupported("clone_file"))
    }
    fn snapshot(&self, root: &BackendPath, name: &str) -> Result<SnapshotId, VfsError> {
        Err(VfsError::Unsupported("snapshot"))
    }
    fn xattr_get(&self, path: &BackendPath, key: &str) -> Result<Vec<u8>, VfsError> {
        Err(VfsError::Unsupported("xattr_get"))
    }
    fn xattr_set(&self, path: &BackendPath, key: &str, val: &[u8]) -> Result<(), VfsError> {
        Err(VfsError::Unsupported("xattr_set"))
    }
    fn xattr_list(&self, path: &BackendPath) -> Result<Vec<String>, VfsError> {
        Err(VfsError::Unsupported("xattr_list"))
    }

    /// Subscribe to raw backend events; supervisor's WatcherCore translates to FsEvent.
    fn subscribe_events(&self, _path: &BackendPath, _recursive: bool)
        -> Result<BackendWatchHandle, VfsError>
    {
        Err(VfsError::Unsupported("subscribe_events"))
    }
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

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct SnapshotId(pub u64);

#[derive(Debug)]
pub enum VfsError {
    NotFound,
    PermissionDenied,
    QuotaExceeded { used: u64, limit: u64 },
    InUse,                    // coordination lock held
    AlreadyExists,
    NotEmpty,
    InvalidName,
    PathEscape,
    PathTooLong,
    NotAbsolute,
    Unsupported(&'static str),
    BackendError(String),
    SnapshotConflict,
    SenderGone,               // for SharedBuffer-mediated reads
    BookmarkInvalid,
    BookmarkExpired,
    CoordinationTimeout,
    CoordinationCycle,        // wait-for graph cycle
    IoError(std::io::Error),
}
```

### 1.3 The `Vfs` mediator

```rust
// supervisor/src/fs/mod.rs (excerpt)

use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Vfs {
    /// Mounted backends keyed by mount point.
    mounts: ArcSwap<MountTable>,
    /// Per-app sandbox; lives inside PolicySnapshot but cached here too.
    sandbox: Arc<SandboxRegistry>,
    pub xattrs: Arc<XattrStore>,
    pub quotas: Arc<QuotaLedger>,
    pub watcher: Arc<WatcherCore>,
    pub coordination: Arc<CoordinationService>,
    pub bookmarks: Arc<BookmarkService>,
    pub trash: Arc<TrashService>,
    pub panel: Arc<PanelService>,
}

pub struct MountTable {
    /// Mount root → backend. Roots are app-visible paths like "/data", "/tmp".
    pub mounts: Vec<Mount>,
}

pub struct Mount {
    pub mount_point: PathBuf,
    pub backend: Arc<dyn VfsBackend>,
    pub allowed_root: BackendPath,   // hard ceiling: every BackendPath returned by resolve() must lie under this
    pub default_flags: MountFlags,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct MountFlags {
    pub read_only: bool,
    pub no_exec: bool,
    pub case_insensitive: bool,
    pub no_atime: bool,
}

impl Vfs {
    /// Top-level open. Takes the app's logical path, applies sandbox map,
    /// dispatches to backend.
    pub fn open(
        &self,
        actor: &ActorRef,
        app_path: &Path,
        flags: OpenFlags,
    ) -> Result<Arc<dyn VfsFile>, VfsError> {
        let (mount, bp) = self.resolve(actor, app_path)?;
        self.coordination.check_compatible(actor, &bp, flags)?;
        if flags.write { self.quotas.preflight(actor.bundle_id(), &mount)?; }
        let raw = mount.backend.open(&bp, flags)?;
        let wrapped: Arc<dyn VfsFile> = Arc::new(QuotaCoordFile::new(
            raw, actor.clone(), bp.clone(), mount.clone(),
            self.quotas.clone(), self.coordination.clone(),
        ));
        Ok(wrapped)
    }

    fn resolve(&self, actor: &ActorRef, app_path: &Path)
        -> Result<(Arc<Mount>, BackendPath), VfsError>
    {
        let snap = actor.policy_snapshot();
        let sandbox = &snap.fs_sandbox;
        let logical = normalize_logical(app_path, &sandbox.cwd)?;
        let mounts = self.mounts.load();
        let mount = mounts.find(&logical).ok_or(VfsError::NotFound)?;
        if !sandbox.is_allowed(&logical, mount.default_flags) {
            return Err(VfsError::PermissionDenied);
        }
        let stripped = logical.strip_prefix(&mount.mount_point).unwrap();
        let bp_inner = mount.allowed_root.as_path().join(stripped);
        let bp = BackendPath::new(canonicalize_no_symlink(&bp_inner)?)?;
        if !bp.as_path().starts_with(mount.allowed_root.as_path()) {
            return Err(VfsError::PathEscape);
        }
        Ok((mount, bp))
    }
}
```

The `resolve` path is the *only* place in the supervisor that turns an app-visible path into a backend path. Every other VFS entry point (mkdir, rename, stat, panel.return-path) goes through `resolve` or `resolve_pair`. This is the choke point for sandbox enforcement.

### 1.4 Node types and edge cases

| Node kind | Backend supports | VFS behavior |
|-----------|------------------|--------------|
| Regular file | All | Standard read/write/truncate |
| Directory | All | readdir, mkdir, rmdir |
| Symlink | 9P, ext4 | `no_follow` flag respected; resolution is supervisor-side, capped at 32 hops, and the final resolved target must still lie inside the same sandbox mount |
| Device | None for apps | `open` returns `PermissionDenied` even on `/dev/*`; the supervisor talks to devices directly |
| Named pipe (FIFO) | Tmpfs only | Apps can create FIFOs in `/tmp/<bundle>-<iid>/` for child-process patterns; FIFOs do not cross sandbox boundaries |
| Hard link | Backend-dep | `vfs.link(src, dst)` allowed only within same mount; quota cost charged once (`nlink>1`) |

### 1.5 Path resolution algorithm

```
fn resolve(actor, app_path):
    snap     = actor.policy_snapshot()
    sandbox  = snap.fs_sandbox
    cwd      = sandbox.cwd or "/"
    logical  = normalize(absolute_join(cwd, app_path))   // collapses .. and . at the LOGICAL layer
    if logical.has_dotdot_after_normalization():
        return Err(PathEscape)
    nfc      = nfc_normalize(logical)
    cased    = if sandbox.case_insensitive: casefold(nfc) else: nfc
    mount    = mounts.longest_prefix_match(cased)
    if mount is None: return Err(NotFound)
    if not sandbox.allows(cased, mount.flags): return Err(PermissionDenied)
    rel      = cased.strip_prefix(mount.mount_point)
    raw_bp   = mount.allowed_root.join(rel)
    canon    = canonicalize_no_symlink(raw_bp)    // resolves "." and rejects any ".."
    if not canon.starts_with(mount.allowed_root): return Err(PathEscape)   // belt + suspenders
    return Ok((mount, BackendPath::new(canon)))
```

Symlinks are resolved component-by-component by the *supervisor*, never by the backend's `open`. This is what makes the sandbox impossible to escape via a symlink the app planted earlier: each hop is re-checked against `allowed_root`.

---

## 2. App Sandbox & Path Namespacing

### 2.1 The `SandboxMap`

```rust
// supervisor/src/fs/sandbox.rs

use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct SandboxMap {
    pub entries: Arc<[SandboxEntry]>,    // sorted by mount_point desc, longest first
    pub cwd: PathBuf,
    pub case_insensitive: bool,
    pub nfc_normalize: bool,
}

#[derive(Clone, Debug)]
pub struct SandboxEntry {
    pub mount_point: PathBuf,            // app-visible, e.g. /data
    pub access: SandboxAccess,
    pub xattr_namespace: Option<String>, // e.g. com.example.app
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SandboxAccess {
    ReadOnly,
    ReadWrite,
    WriteOnly,                           // /dev/null-like
    Append,                              // log-only
    NoAccess,
}

impl SandboxMap {
    pub fn is_allowed(&self, path: &Path, mount_flags: MountFlags) -> bool {
        // longest-prefix match; default deny
        for e in self.entries.iter() {
            if path.starts_with(&e.mount_point) {
                return match (e.access, mount_flags.read_only) {
                    (SandboxAccess::NoAccess, _) => false,
                    (SandboxAccess::ReadOnly, _) => true,        // checked again at flags-level
                    (SandboxAccess::ReadWrite, false) => true,
                    (SandboxAccess::ReadWrite, true) => false,   // mount is RO
                    (SandboxAccess::WriteOnly, false) => true,
                    (SandboxAccess::WriteOnly, true) => false,
                    (SandboxAccess::Append, false) => true,
                    (SandboxAccess::Append, true) => false,
                };
            }
        }
        false
    }
}
```

`SandboxMap` lives inside the Round-3 `PolicySnapshot`:

```rust
// extension to round-3 PolicySnapshot
pub struct PolicySnapshot {
    // ... round-3 fields ...
    pub fs_sandbox: SandboxMap,
}
```

When the manifest is reloaded the supervisor builds a new `SandboxMap`, slots it into a new `PolicySnapshot`, and `ArcSwap::store`s. In-flight reads continue against the old snapshot; new opens see the new map. This is the same mechanism Round 3 uses for capability changes — no new lock, no new race.

### 2.2 Default sandbox

Every app gets this baseline if it declares `[capabilities.filesystem]`:

| Logical path | Access | Backend mount | Persistence |
|--------------|--------|---------------|-------------|
| `/data/<bundle>/` | ReadWrite | 9P (`/data/bundles/<bundle>/`) | Survives reboot |
| `/data/<bundle>/.Trash/` | ReadWrite (mediated) | 9P | Survives reboot |
| `/tmp/<bundle>-<iid>/` | ReadWrite | Tmpfs | Per-instance |
| `/apps/<bundle>/` | ReadOnly | Overlay (lower = bundle .wasm + assets) | Survives reboot |
| `/sys/fonts/` | ReadOnly | SysRO | Static |
| `/sys/icons/` | ReadOnly | SysRO | Static |
| `/sys/themes/` | ReadOnly | SysRO | Static |
| `/sys/timezones/` | ReadOnly | SysRO | Static |

Additional mounts unlocked by capability declarations:

| Capability | Logical path | Access |
|------------|--------------|--------|
| `filesystem_shared` | `/data/shared/` | ReadWrite |
| `filesystem_documents` | `/data/Documents/` | ReadWrite (post-Powerbox grant only) |
| `filesystem_full_disk` | `/data/` | ReadWrite (gated by user system-prefs grant; never available to v1 store apps) |
| `bookmarks` | any path the user picks via panel | ReadWrite via bookmark only |

### 2.3 Why not `chroot`?

Two reasons:

1. **One Wasmtime process, many WASM instances.** `chroot` is per-process. Switching it per call is expensive (and racy without unshare-namespacing). We need per-instance isolation in the same Linux process.
2. **The supervisor still needs the full namespace.** `chroot`ing the supervisor would defeat its role as broker. Per-instance namespacing via `SandboxMap` resolution is cleaner and gives us xattr/quota/coordination interception for free.

The WASI preopens we hand to Wasmtime are themselves derived from the `SandboxMap`: at instance launch, the supervisor calls `WasiCtxBuilder::preopened_dir` for each `SandboxEntry`, with file descriptors obtained from the *supervisor's* view of the backend. The WASM app never sees a path outside the preopens; the supervisor never sees a request that wasn't translated.

### 2.4 Path translation table (worked examples)

| App writes | After `cwd=/data/notes/`, sandbox | After resolve |
|------------|-----------------------------------|---------------|
| `"draft.md"` | `/data/notes/draft.md` | `9P:/bundles/com.example.notes/draft.md` |
| `"../sibling/x"` | `/data/sibling/x` | `PathEscape` (sibling not in sandbox) |
| `"/sys/fonts/Inter.ttf"` | `/sys/fonts/Inter.ttf` | `SysRO:/fonts/Inter.ttf` |
| `"/etc/passwd"` | `/etc/passwd` | `NotFound` (no mount) |
| `"/tmp/foo"` | `/tmp/foo` | `PathEscape` (no `<bundle>-<iid>` prefix) |
| `"/tmp/com.example.notes-7/foo"` | as written | `Tmpfs:/com.example.notes-7/foo` |

The last row is interesting: tmpfs is keyed by `bundle-iid`, so two instances of the same app cannot see each other's temp files. This matches macOS's per-bundle `NSTemporaryDirectory()`.

---

## 3. APFS-equivalent Features

### 3.1 Snapshots

```rust
// supervisor/src/fs/snapshot.rs

pub struct SnapshotManager {
    backends: Arc<MountTable>,
    xattrs: Arc<XattrStore>,
    /// Snapshot metadata: (bundle, snapshot_id) → SnapshotMeta
    meta: parking_lot::RwLock<HashMap<(BundleId, SnapshotId), SnapshotMeta>>,
}

pub struct SnapshotMeta {
    pub id: SnapshotId,
    pub root: BackendPath,
    pub created_ns: i64,
    pub bytes_unique: u64,          // CoW bytes diverged from parent
    pub bytes_total: u64,
    pub label: String,
}

impl SnapshotManager {
    /// Atomically snapshot the subtree at `root` (app path).
    /// Backend-native if available (btrfs subvol snapshot, ext4 reflinks).
    /// Otherwise emulated via xattr CloneEdge graph.
    pub fn snapshot(&self, actor: &ActorRef, root: &Path, label: &str)
        -> Result<SnapshotId, VfsError>
    {
        let (mount, bp) = self.vfs.resolve(actor, root)?;
        if mount.backend.capabilities().snapshots {
            let id = mount.backend.snapshot(&bp, label)?;
            self.meta.write().insert((actor.bundle_id(), id), SnapshotMeta {
                id, root: bp, created_ns: now_ns(), bytes_unique: 0,
                bytes_total: self.measure_tree(&bp)?, label: label.into(),
            });
            Ok(id)
        } else {
            self.emulated_snapshot(actor, &mount, &bp, label)
        }
    }
}
```

**Emulated snapshot on 9P:** a snapshot is a recursive `CloneEdge` graph rooted at the snapshot's backend path. Each child inode gets a `vyoma.clone.parent` xattr pointing at the original. Reads via the snapshot follow the parent edge; writes break the edge by materializing a copy. This is O(directory-count) at snapshot time, O(1) per file, and pay-as-you-go for divergence.

### 3.2 Sparse files

The VFS exposes `allocated` separately from `size` in `FileStat`. Writes of all-zero blocks ≥ 4 KB are punched out (`FALLOC_FL_PUNCH_HOLE` on backends that support it; tracked in xattr `vyoma.sparse.holes` otherwise). Reads from a hole return zeros without disk I/O.

```rust
pub trait VfsFile {
    // ... base methods ...
    fn allocate(&self, offset: u64, len: u64) -> Result<(), VfsError> { /* fallocate */ }
    fn punch_hole(&self, offset: u64, len: u64) -> Result<(), VfsError>;
    fn extent_map(&self) -> Result<Vec<Extent>, VfsError>;
}

#[derive(Copy, Clone, Debug)]
pub struct Extent { pub logical_offset: u64, pub physical_offset: u64, pub len: u64, pub flags: ExtentFlags }
```

### 3.3 Clones (reflinks)

```rust
pub fn clone_file(&self, actor: &ActorRef, src: &Path, dst: &Path) -> Result<(), VfsError> {
    let (src_mount, src_bp) = self.resolve(actor, src)?;
    let (dst_mount, dst_bp) = self.resolve_for_create(actor, dst)?;
    if !Arc::ptr_eq(&src_mount.backend, &dst_mount.backend) {
        // cross-mount: fall back to copy
        return self.copy_file(actor, src, dst);
    }
    if src_mount.backend.capabilities().clones {
        return src_mount.backend.clone_file(&src_bp, &dst_bp);
    }
    self.emulate_clone(actor, src_mount.clone(), src_bp, dst_bp)
}
```

Emulated clone on 9P: the destination is created as an empty file with xattr `vyoma.clone.parent = src.inode`. Reads on the destination follow the parent until the destination is written, at which point a `cow_break` materializes a real copy. Quota is charged at materialization time, not clone time — exactly matching APFS's CoW behavior.

### 3.4 Extended attributes (xattrs)

Three namespaces:

| Prefix | Owner | Examples |
|--------|-------|----------|
| `vyoma.*` | Supervisor only (apps get `PermissionDenied` on set) | `vyoma.trash.original-path`, `vyoma.clone.parent`, `vyoma.quarantine`, `vyoma.spotlight.indexed-at`, `vyoma.original-name` |
| `user.*` | App, persisted, visible to all apps with access to the file | `user.tags`, `user.color-label`, `user.notes-summary` |
| `com.<bundle>.*` | App-private, only the owning bundle reads/writes | `com.example.notes.last-cursor`, `com.example.notes.spell-cache` |

```rust
// supervisor/src/fs/xattr.rs

pub struct XattrStore {
    db: sled::Db,                 // /data/.sys/xattrs.db
    inflight: parking_lot::RwLock<HashMap<(BackendPath, String), Vec<u8>>>,
    backends_with_native: HashSet<String>,
}

impl XattrStore {
    pub fn get(&self, mount: &Mount, bp: &BackendPath, key: &str) -> Result<Vec<u8>, VfsError> {
        if mount.backend.capabilities().xattrs {
            match mount.backend.xattr_get(bp, key) {
                Ok(v) => return Ok(v),
                Err(VfsError::NotFound) => {}            // fall through to shadow
                Err(e) => return Err(e),
            }
        }
        let dbkey = format!("{}:{}::{}", mount.backend.name(), bp.as_path().display(), key);
        match self.db.get(dbkey.as_bytes()) {
            Ok(Some(v)) => Ok(v.to_vec()),
            Ok(None) => Err(VfsError::NotFound),
            Err(e) => Err(VfsError::BackendError(e.to_string())),
        }
    }
    pub fn set(&self, actor: &ActorRef, mount: &Mount, bp: &BackendPath, key: &str, val: &[u8])
        -> Result<(), VfsError>
    {
        self.check_namespace(actor, key)?;
        if val.len() > 64 * 1024 { return Err(VfsError::QuotaExceeded { used: val.len() as u64, limit: 64 * 1024 }); }
        if mount.backend.capabilities().xattrs {
            mount.backend.xattr_set(bp, key, val)?;
            return Ok(());
        }
        let dbkey = format!("{}:{}::{}", mount.backend.name(), bp.as_path().display(), key);
        self.db.insert(dbkey.as_bytes(), val).map_err(|e| VfsError::BackendError(e.to_string()))?;
        Ok(())
    }
    fn check_namespace(&self, actor: &ActorRef, key: &str) -> Result<(), VfsError> {
        if key.starts_with("vyoma.") { return Err(VfsError::PermissionDenied); }
        if let Some(rest) = key.strip_prefix("com.") {
            let bundle = actor.bundle_id().as_str();
            if !rest.starts_with(bundle) { return Err(VfsError::PermissionDenied); }
        }
        Ok(())
    }
}
```

### 3.5 Case-insensitive + Unicode-normalizing paths

Default behavior on `/data` mounts: paths are normalized to NFC and case-folded for lookup. On-disk names preserve the original. This matches APFS's default "case-insensitive, normalization-insensitive" mode.

Apps may opt out per-mount via manifest:
```toml
[capabilities.filesystem]
"/data/<bundle>/" = { case_sensitive = true, nfc_normalize = false }
```

This is required for git working trees, language servers, and any app that round-trips arbitrary user paths.

---

## 4. WASM App API: `vyoma:fs@0.1.0`

### 4.1 Why a parallel API?

We do not replace `wasi:filesystem/types@0.2.0`. Apps continue to use it for ordinary `open`/`read`/`write`. We *extend* it with `vyoma:fs/*` for the macOS-equivalent richness:

```
wasi:filesystem/types@0.2.0  →  open, read, write, readdir, stat, etc.   (per-byte syscalls)
vyoma:fs/snapshot            →  snapshot, clone-file, list-snapshots     (CoW operations)
vyoma:fs/xattr               →  get/set/list xattrs
vyoma:fs/watcher             →  file-watcher resource + on-fs-event callback
vyoma:fs/coord               →  coordinate-read/write, file-presenter resource
vyoma:fs/panel               →  open-panel, save-panel
vyoma:fs/bookmark            →  open-bookmark, save-bookmark
vyoma:fs/trash               →  trash, untrash, list-trash
vyoma:fs/quota               →  usage, limit
```

### 4.2 WIT surface (excerpt — full file at `wit/vyoma-fs.wit`)

```wit
package vyoma:fs@0.1.0;

interface types {
    record path {
        s: string,
    }

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

    enum file-kind { regular, directory, symlink, device, fifo }

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
    get: func(p: path, key: string) -> result<list<u8>, fs-error>;
    set: func(p: path, key: string, val: list<u8>) -> result<_, fs-error>;
    list: func(p: path) -> result<list<string>, fs-error>;
    remove: func(p: path, key: string) -> result<_, fs-error>;
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
    create: func(root: path, label: string) -> result<snapshot-id, fs-error>;
    list: func(root: path) -> result<list<snapshot-info>, fs-error>;
    delete: func(id: snapshot-id) -> result<_, fs-error>;
    restore: func(id: snapshot-id) -> result<_, fs-error>;
    clone-file: func(src: path, dst: path) -> result<_, fs-error>;
}

interface watcher {
    use types.{path, fs-error};
    enum watch-scope { file, directory, tree }

    record watch-event {
        watch-id: u64,
        path: string,
        kind: event-kind,
        cookie: option<u64>,           // for rename pairs
        seq: u64,
        timestamp-ns: s64,
    }
    enum event-kind { created, modified, deleted, renamed-from, renamed-to, attribute-changed, xattr-changed }

    resource file-watcher {
        constructor(p: path, scope: watch-scope);
        id: func() -> u64;
        // events are delivered via vyoma:ipc/on-ipc with body schema = fs.watch-event
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
        // pre-write, post-write, etc. events delivered via on-ipc
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
        bookmarks: list<list<u8>>,     // opaque bookmark tokens
        cancelled: bool,
    }
    open: func(req: open-request) -> result<panel-result, fs-error>;
    save: func(req: save-request) -> result<panel-result, fs-error>;
}

interface bookmark {
    use types.{fs-error};
    type bookmark-token = list<u8>;
    /// Redeems a bookmark and returns a wasi:filesystem descriptor.
    /// Lifetime: descriptor closes automatically on instance exit or bookmark expiry.
    open: func(tok: bookmark-token, read-only: bool) -> result<u64 /* wasi-fd */, fs-error>;
    /// Stores a bookmark in the app's private bookmark store.
    save: func(name: string, tok: bookmark-token) -> result<_, fs-error>;
    load: func(name: string) -> result<bookmark-token, fs-error>;
    list: func() -> result<list<string>, fs-error>;
}

interface trash {
    use types.{path, fs-error};
    record trash-entry {
        token: u64,
        original-path: string,
        trashed-at-ns: s64,
        size: u64,
    }
    trash:    func(p: path) -> result<u64 /* token */, fs-error>;
    untrash:  func(t: u64) -> result<string /* restored path */, fs-error>;
    list:     func() -> result<list<trash-entry>, fs-error>;
    purge:    func(t: u64) -> result<_, fs-error>;
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

### 4.3 Stdout escape hatch: `VYOMA_FS:` protocol

For symmetry with the existing `VYOMA_DRAW:` line protocol (and for apps that haven't migrated to WIT bindings yet), a minimal stdout protocol covers the user-visible operations:

```
VYOMA_FS:open_panel:<title>,<csv-extensions>,<mode>
VYOMA_FS:save_panel:<title>,<default-name>,<csv-extensions>
VYOMA_FS:trash:<path>
VYOMA_FS:reveal:<path>                  # opens parent dir in Files app
VYOMA_FS:share:<path>                   # opens system share sheet
```

These behave exactly like Round 3's `@supervisor:` commands: a `legacy_stdout_fs` boot flag (default `false`) and `[capabilities.fs_stdout]` manifest entry are required. New apps use WIT.

The result of an asynchronous panel call is delivered via `on-ipc` with envelope schema `fs.panel-result`. The app's stdout `VYOMA_FS:open_panel:` write is essentially a fire-and-receive-via-callback pattern; the request id is the IPC envelope id assigned by the router.

### 4.4 Zero-copy reads via `SharedBuffer`

Any single `read` ≥ 64 KB or any `panel.open` whose returned bookmark refers to a file ≥ 1 MB triggers the `SharedBuffer` path from Round 2/3:

```rust
// supervisor/src/fs/wasi_shim.rs (excerpt)

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

The `SharedBufferHandle` is returned to the app via the same `wasi:io/streams` resource the app already holds; an extension method `read_zero_copy(len) -> result<shared-buffer-handle, _>` is added to the stream resource. Apps that don't request it get the standard copying path.

---

## 5. File Watching (FSEvents equivalent)

### 5.1 Architecture

```
┌──────────────────────────────────────────┐
│            WatcherCore thread             │
│                                           │
│  inotify_fd (ext4/overlay/tmpfs)         │
│  poll_loop  (9P, 250 ms)                  │
│         │                                 │
│  ┌──────▼──────┐                          │
│  │ Coalescer    │  100ms tumbling window  │
│  └──────┬──────┘                          │
│         │                                 │
│  ┌──────▼──────┐                          │
│  │ Fanout       │  watch_id → [actors]    │
│  └──────┬──────┘                          │
│         │ IpcEnvelope                     │
└─────────┼─────────────────────────────────┘
          ▼
      Round-3 router (sharded)
```

### 5.2 Watch handles

```rust
// supervisor/src/fs/watcher.rs

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct WatchId(pub u64);

#[derive(Copy, Clone, Debug)]
pub enum WatchScope { File, Directory, Tree }

pub struct WatchSubscription {
    pub id: WatchId,
    pub owner: ActorRef,
    pub backend_path: BackendPath,
    pub scope: WatchScope,
    pub last_seq: AtomicU64,
}

pub struct WatcherCore {
    subs: dashmap::DashMap<WatchId, Arc<WatchSubscription>>,
    by_path: dashmap::DashMap<BackendPath, smallvec::SmallVec<[WatchId; 4]>>,
    next_id: AtomicU64,
    next_seq: AtomicU64,
    inotify: Option<inotify::Inotify>,
    coalescer: parking_lot::Mutex<Coalescer>,
    router: Arc<IpcRouter>,                 // round-3 router
}
```

### 5.3 Event coalescing

Each `(watch_id, path, kind)` tuple has a 100 ms tumbling window. The coalescer is a small ring buffer keyed by tuple; on insertion it bumps the `last_seq` of the existing entry rather than enqueuing a new one. At window-close (driven by a 50 ms tick), entries are flushed as `FsEvent` IPC envelopes.

Coalescing rules:

| Sequence | Result |
|----------|--------|
| `created` then `modified` (same window) | One `created` |
| `modified` then `modified` | One `modified` |
| `created` then `deleted` | Dropped (atomic temp file) |
| `renamed-from(A)` then `renamed-to(B)` | One `renamed-from(A)` + one `renamed-to(B)` with same `cookie` |
| `attribute-changed` then `modified` | One `modified` |

### 5.4 Delivery via Round-3 IPC

```rust
fn deliver(&self, ev: WatchEvent) {
    let body = encode_cbor(&ev);
    let env = IpcEnvelope {
        from: IpcAddr::Supervisor,
        to:   IpcAddr::Instance(ev.subscriber_iid),
        class: IpcClass::Normal,            // FsEvent is non-urgent
        priority: Priority::Normal,
        schema_id: schema!(fs.watch-event),
        seq: ev.seq,
        body,
        deadline_ns: None,
    };
    self.router.send_lossy(env, OverflowPolicy::Coalesce);
}
```

`Coalesce` overflow policy (Round 3 §4): if the receiver's `inbox_lo` is full, the router replaces the oldest pending `fs.watch-event` for the same `watch_id` rather than dropping. This is essential for a smooth Finder-style UI when a directory has high churn.

### 5.5 Watcher quotas

Per-app limits enforced at `file-watcher` construction:

| Limit | Default | Rationale |
|-------|---------|-----------|
| Max active watches | 64 | Inotify watch fd cost |
| Max recursive tree depth | 16 | Bound coalescer size |
| Max events/sec per watch | 50 (sustained) | Backpressure target |
| Max coalescer queue per app | 256 | Memory ceiling |

When a limit is hit, the watcher emits a `WatchOverflow` event (with `dropped_count`) and stops delivering until the app acks via a `watcher.acknowledge(watch_id)` WIT call.

---

## 6. File Coordination (NSFileCoordinator equivalent)

### 6.1 Why we need it

Two scenarios drive this:

1. **Multi-instance apps.** Two windows of the same Notes app both want to write `note.md`. Without coordination, one stomps the other.
2. **Cross-app collaboration.** A spell-check daemon (a separate WASM app) wants to read `note.md` while Notes is writing it; we want the daemon to wait for the write to complete, not see half-flushed bytes.

### 6.2 Lock model

```rust
// supervisor/src/fs/coordination.rs

pub enum LockMode { Read, Write }

pub struct CoordinationLock {
    pub path: BackendPath,
    pub mode: LockMode,
    pub readers: smallvec::SmallVec<[ActorRef; 4]>,
    pub writer: Option<ActorRef>,
    pub waiters: VecDeque<Waiter>,
    pub last_writer_seq: u64,          // bumped on every write release
}

pub struct Waiter {
    pub actor: ActorRef,
    pub mode: LockMode,
    pub deadline_ns: i64,
    pub req_id: u64,
    pub priority: Priority,
}

pub struct CoordinationService {
    locks: dashmap::DashMap<BackendPath, parking_lot::Mutex<CoordinationLock>>,
    /// Reuses Round-3 wait-for graph for cycle detection.
    wait_graph: Arc<WaitForGraph>,
}
```

### 6.3 Acquire algorithm

```
acquire(actor, path, mode, timeout):
    lock = locks.get_or_insert(path)
    guard = lock.lock()
    if compatible(guard, mode, actor):
        grant(guard, actor, mode)
        return Ok(CoordToken)
    // would block; check for cycle
    if wait_graph.would_cycle(actor, holders_of(guard)):
        return Err(CoordinationCycle)
    waiter = Waiter { actor, mode, deadline: now+timeout, req_id, priority }
    inherit_priority(guard, waiter)
    guard.waiters.push_back(waiter)
    drop(guard)
    suspend until granted or timeout
```

The `wait_graph.would_cycle` call is the same routine Round 3 uses for `call` cycle detection; we extend the graph nodes to include "actor X is waiting on lock holders Y, Z, ..." edges. This means we can detect cycles spanning IPC `call`s and file coordination locks together.

### 6.4 Priority inheritance

When a writer with `Priority::High` waits on a reader with `Priority::Normal`, the reader's dispatcher thread is briefly bumped to `Priority::High` (via `SCHED_FIFO`-equivalent on Linux, or just a flag the scheduler reads). The bump is revoked on release. This prevents the desktop equivalent of a priority inversion (e.g., the system-wide spell-checker daemon holding a read lock while the user is trying to save).

### 6.5 Release on `InstanceGone`

The Round-1 lifecycle hook `on_instance_gone(iid)` is wired through `CoordinationService::release_all(iid)`. Every lock the dying actor held is released; every waiter the actor had is cancelled with `IpcError::SenderGone`. This is the same guarantee Round 3 makes for in-flight `call`s.

---

## 7. Open/Save Panels (NSOpenPanel / NSSavePanel)

### 7.1 Architecture

The panel is a chrome-process window owned by the supervisor's compositor. The requesting app **never** sees the panel's pixels; it sees a single async `panel.open()` call that resolves to a list of bookmark tokens (or `cancelled = true`).

```
App                      Supervisor                       User
 │  panel.open(req)         │                               │
 │ ────────────────────────►│                               │
 │                          │ render PanelWindow            │
 │                          │ ──────────────────────────── ►│
 │                          │                               │ navigates, selects
 │                          │ ◄──────────────────────────── │ clicks "Open"
 │                          │ mint BookmarkToken(s)         │
 │ ◄────────────────────────│ panel-result                  │
 │  open-bookmark(tok)      │                               │
 │ ────────────────────────►│ validate, grant fd            │
 │ ◄────────────────────────│ wasi-fd                       │
```

### 7.2 Panel rendering

The panel UI is itself a built-in WASM app (`apps/panel/`) running with elevated capabilities (`[capabilities.chrome] = true`, `[capabilities.filesystem_full_disk] = true`). The supervisor spawns one instance per active panel. The panel app draws via the standard `VYOMA_DRAW:` protocol; it sees the full filesystem; it returns selected backend paths to the supervisor; the supervisor mints bookmarks scoped to the *requesting* app.

This means:
- The same compositor + input pipeline handles the panel
- The panel can be replaced/styled without rebuilding the supervisor
- The privilege boundary is clean: the panel app gets full-disk access; the requesting app gets only the bookmark

### 7.3 Bookmark scope

Each bookmark minted from a panel has these properties:

| Field | Value |
|-------|-------|
| `bundle_id` | The requesting app's bundle |
| `backend_path` | The exact resolved path (no glob) |
| `mode` | `read-only` or `read-write` based on the request type |
| `expiry` | `none` (persistent) or `until_instance_exit` |
| `granted_at_ns` | timestamp |
| `granted_by` | `user-via-panel` (always, for panel-minted) |
| `hmac` | SHA-256-HMAC of all above with supervisor session key |

The bookmark does **not** add the path to the app's `SandboxMap`. Instead, the app must explicitly call `bookmark.open(tok)` every time it wants to use the file, and gets back a `wasi-fd` whose lifetime is tied to the open. This is exactly the Powerbox pattern.

### 7.4 Panel UX

Built-in panel features (parity with macOS):

- Sidebar with: Recents, Documents, Downloads, Trash, app's container, mounted external (bookmarked) locations
- Search field (queries Spotlight-like index built from xattr `vyoma.spotlight.*`)
- Tile/list/column view toggles
- File-type filter dropdown from the requesting app's `extensions` list
- "Options" disclosure with: show hidden files, show package contents
- Tag filter (queries xattr `user.tags`)
- Quick Look preview pane (renders via image/PDF/text apps as embedded surfaces)

---

## 8. Bookmarks (Security-Scoped Bookmarks)

### 8.1 Token format

```rust
// supervisor/src/fs/bookmark.rs

#[derive(Clone)]
pub struct BookmarkToken(pub Vec<u8>);  // opaque CBOR-encoded

#[derive(Clone)]
struct BookmarkInner {
    pub version: u8,
    pub bundle_id: BundleId,
    pub backend_path: PathBuf,
    pub mount_name: String,
    pub mode: BookmarkMode,
    pub created_ns: i64,
    pub expires_ns: Option<i64>,
    pub granter: Granter,
    pub inode_hint: Option<u64>,        // for stale detection
    pub mac: [u8; 32],                  // HMAC-SHA-256
}

pub enum BookmarkMode { ReadOnly, ReadWrite, AppendOnly }
pub enum Granter { UserViaPanel, ManifestStatic, BookmarkRefresh }
```

The encoding is CBOR with deterministic field ordering. The HMAC is computed over the canonical CBOR of everything except `mac`, using a per-boot session key (`supervisor.bookmarks.session_key`, 32 random bytes regenerated at every boot). Persistent bookmarks survive boots by also being signed with a derived long-term key (HKDF from a key stored under `/data/.sys/keys/`, mode 0600, supervisor-only).

### 8.2 Persistent storage

Apps store bookmarks via `bookmark.save(name, token)`:

```rust
pub fn save(&self, actor: &ActorRef, name: &str, tok: &BookmarkToken) -> Result<(), VfsError> {
    self.validate(actor, tok)?;
    let path = format!("/data/{}/.bookmarks/{}", actor.bundle_id(), sanitize_name(name));
    let encrypted = self.encrypt_with_app_key(actor, tok)?;
    self.vfs.write_atomic(actor, &path, &encrypted)
}
```

The app's bookmark store is encrypted with a per-app key derived from the keychain (HKDF over the supervisor master key with `bundle_id` as info). This means dumping `/data/<bundle>/.bookmarks/` from another app (if it gains `filesystem_full_disk`) leaks neither the path nor the mac.

### 8.3 Stale bookmark refresh

If the user moves the bookmarked file:
- The bookmark contains `inode_hint`
- On `bookmark.open`, the supervisor first stats `backend_path`; if missing or inode mismatch, it triggers a `BookmarkRefresh` flow
- The supervisor shows a chrome banner: "*App* needs access to *file* which has moved. Locate it?" with a panel
- On success, a new bookmark is minted with `granter = BookmarkRefresh` and the app is given the new token via IPC

---

## 9. File Tags & Metadata

### 9.1 Tags

Tags are stored as xattr `user.tags` (CBOR-encoded `Vec<Tag>`):

```rust
#[derive(Clone, Debug)]
pub struct Tag { pub name: String, pub color: TagColor }

#[derive(Copy, Clone, Debug)]
pub enum TagColor { Red, Orange, Yellow, Green, Blue, Purple, Gray, None }
```

### 9.2 Tag index

A supervisor-side index in `/data/.sys/tag-index.db` (sled) maps `tag_name → Vec<(BackendPath, inode)>`. Updates are async — on `xattr.set("user.tags", ...)`, the supervisor enqueues an `IndexUpdate` task on a background thread. Queries from the panel/search UI hit the index directly.

### 9.3 Spotlight-like search

`vyoma.spotlight.*` xattrs are sphere-of-influence for the supervisor's indexer:

| Key | Value |
|-----|-------|
| `vyoma.spotlight.text` | Plain-text content extract (for full-text search) |
| `vyoma.spotlight.preview` | 256×256 PNG thumbnail |
| `vyoma.spotlight.indexed-at` | timestamp ns |
| `vyoma.spotlight.kind` | content kind (`text/markdown`, `image/png`, etc.) |
| `vyoma.spotlight.summary` | one-sentence summary |

Content extractors are themselves WASM apps (`apps/indexer-pdf`, `apps/indexer-image`, ...) that subscribe to FsEvents on `/data/` and write xattrs back. Each runs in its own sandbox with `xattr` capability.

---

## 10. Trash / Recycle Bin

### 10.1 Per-bundle trash

```rust
// supervisor/src/fs/trash.rs

pub struct TrashService {
    vfs: Arc<Vfs>,
    retention_days: u32,
    next_token: AtomicU64,
}

pub struct TrashEntry {
    pub token: u64,
    pub bundle: BundleId,
    pub original_path: PathBuf,
    pub trashed_at_ns: i64,
    pub size_bytes: u64,
    pub trashed_inode: u64,
    pub trash_path: BackendPath,        // where it lives now
}

impl TrashService {
    pub fn trash(&self, actor: &ActorRef, app_path: &Path) -> Result<u64, VfsError> {
        let (mount, bp) = self.vfs.resolve(actor, app_path)?;
        let stat = mount.backend.stat(&bp)?;
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);
        let trash_dir = format!("/data/{}/.Trash/", actor.bundle_id());
        let dst_name = format!("{}-{}", token, bp.as_path().file_name().unwrap().to_string_lossy());
        let dst = self.vfs.resolve_for_create(actor, Path::new(&format!("{trash_dir}/{dst_name}")))?.1;
        mount.backend.rename(&bp, &dst)?;
        self.vfs.xattrs.set_system(&mount, &dst, "vyoma.trash.original-path", bp.as_path().display().to_string().as_bytes())?;
        self.vfs.xattrs.set_system(&mount, &dst, "vyoma.trash.trashed-at-ns", &(now_ns()).to_be_bytes())?;
        Ok(token)
    }
}
```

### 10.2 Background sweep

A `TrashSweeper` task runs every hour. It walks `/data/*/.Trash/` and deletes entries whose `vyoma.trash.trashed-at-ns` is older than `retention_days * 86400 * 1e9`. Deletion is permanent (no second trash).

### 10.3 Cross-mount trash

If the original path is on `/data/shared/`, the trash destination is still under the actor's bundle (`/data/<bundle>/.Trash/`). The `vyoma.trash.original-path` xattr records the cross-mount path so `untrash` knows where to restore. If the original location no longer exists (parent removed), `untrash` errors with `NotFound` and the user is shown a chrome dialog to pick a new destination.

---

## 11. Quota & Usage

### 11.1 Per-bundle quotas

Manifest declares:
```toml
[storage]
quota_mb = 256              # default if absent
file_count_limit = 100000   # default
```

System apps may declare `quota_mb = "unlimited"` (gated by `[capabilities.supervisor] full_disk = true`).

### 11.2 Ledger

```rust
// supervisor/src/fs/quota.rs

pub struct QuotaLedger {
    /// (bundle_id, mount_name) → live usage
    live: dashmap::DashMap<(BundleId, String), QuotaCell>,
    /// Persisted snapshots at /data/.sys/quota.db
    db: sled::Db,
    /// Pending changes (batched every 100 ms)
    pending: parking_lot::Mutex<Vec<QuotaDelta>>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct QuotaCell {
    pub bytes_used: u64,
    pub bytes_limit: u64,
    pub files_used: u64,
    pub files_limit: u64,
    pub last_persisted_ns: i64,
}

pub struct QuotaDelta {
    pub bundle: BundleId,
    pub mount: String,
    pub bytes: i64,
    pub files: i32,
}
```

### 11.3 Enforcement points

| Operation | Quota check | Recovery on failure |
|-----------|-------------|---------------------|
| `open(WRITE \| CREATE)` | preflight (reserve `default_block_size`) | reservation released on close |
| `write(buf)` | exact bytes | partial write, return `QuotaExceeded` |
| `truncate(new_len)` | delta only | full fail |
| `clone_file` | 0 (CoW) | succeeds; subsequent write triggers materialize-fail |
| `xattr.set` | val.len() vs xattr budget (16 KB total) | full fail |
| `rename(cross-mount)` | size on dst, refund on src | atomic preflight |

### 11.4 Soft warning

At 80% of quota, the supervisor emits a `QuotaWarning` IPC event to the bundle. At 95%, panels and save dialogs prepend a "*App* is running low on storage" banner. At 100%, writes fail with `QuotaExceeded`.

### 11.5 Per-mount visibility

`vyoma:fs/quota.get()` returns the *current mount's* quota by default. Apps with shared access can query specific mounts. The reported quota is the most restrictive applicable limit:

```rust
fn get(&self, actor: &ActorRef, mount: Option<&str>) -> Usage {
    let cells = self.cells_for(actor, mount);
    cells.fold(Usage::max(), |acc, c| acc.min(c.into()))
}
```

---

## 12. Implementation Files

All files **≤ 500 lines**. New code total: **~6,200 LOC across 22 files**.

| File | Approx. LOC | Responsibility |
|------|-------------|----------------|
| `supervisor/src/fs/mod.rs` | 380 | `Vfs` struct, public entry points, wiring |
| `supervisor/src/fs/vfs.rs` | 460 | `VfsBackend` trait, `VfsFile`, `BackendPath`, `OpenFlags`, errors |
| `supervisor/src/fs/sandbox.rs` | 410 | `SandboxMap`, `SandboxEntry`, path resolution, NFC + casefold |
| `supervisor/src/fs/watcher.rs` | 490 | `WatcherCore`, inotify integration, coalescer, IPC delivery |
| `supervisor/src/fs/coordination.rs` | 420 | `CoordinationService`, locks, waiters, priority inheritance |
| `supervisor/src/fs/panel.rs` | 350 | `PanelService`, panel-app spawn, request/result plumbing |
| `supervisor/src/fs/bookmark.rs` | 440 | `BookmarkService`, mint/validate/refresh, encryption, persistence |
| `supervisor/src/fs/xattr.rs` | 320 | `XattrStore`, shadow DB, namespace checks |
| `supervisor/src/fs/quota.rs` | 380 | `QuotaLedger`, preflight, delta batching, persistence |
| `supervisor/src/fs/trash.rs` | 290 | `TrashService`, trash/untrash, sweeper |
| `supervisor/src/fs/snapshot.rs` | 410 | `SnapshotManager`, native + emulated, restore |
| `supervisor/src/fs/clone.rs` | 280 | CoW machinery, materialize-on-write, `cow_break` |
| `supervisor/src/fs/backends/mod.rs` | 80 | Backend registry |
| `supervisor/src/fs/backends/ninep.rs` | 470 | 9P/virtio backend wrapping the existing `/data` mount |
| `supervisor/src/fs/backends/tmpfs.rs` | 360 | In-RAM backend for `/tmp` |
| `supervisor/src/fs/backends/overlay.rs` | 480 | Lower (RO) + upper (RW) overlay, used for `/apps` and snapshots |
| `supervisor/src/fs/backends/sys_ro.rs` | 220 | Read-only backend for `/sys/fonts`, `/sys/icons`, `/sys/themes` |
| `supervisor/src/fs/wasi_shim.rs` | 460 | Bridges `Vfs` to `WasiCtxBuilder::preopened_dir`; intercepts large reads for `SharedBuffer` |
| `supervisor/src/fs/wit_bindings.rs` | 490 | Generated WIT bindings glue for `vyoma:fs@0.1.0` |
| `supervisor/src/fs/path_index.rs` | 240 | Case-fold + NFC index, `PathBucket` |
| `supervisor/src/fs/spotlight.rs` | 360 | Tag/spotlight indexer driver, query API for panel |
| `wit/vyoma-fs.wit` | 380 | Full WIT package definition (sketched in §4.2) |

Total: 22 files, average 360 LOC, max 490 LOC, well under the 500 ceiling.

### 12.1 Module wiring

```rust
// supervisor/src/fs/mod.rs (entry)

pub fn init(boot: &BootContext) -> Arc<Vfs> {
    let backends = MountTable::from_boot_config(&boot.boot_toml);
    let xattrs   = Arc::new(XattrStore::open("/data/.sys/xattrs.db"));
    let quotas   = Arc::new(QuotaLedger::open("/data/.sys/quota.db"));
    let watcher  = Arc::new(WatcherCore::new(boot.router.clone()));
    let coord    = Arc::new(CoordinationService::new(boot.wait_graph.clone()));
    let bookmark = Arc::new(BookmarkService::new(boot.session_keys.clone()));
    let trash    = Arc::new(TrashService::new(30));
    let panel    = Arc::new(PanelService::new(boot.compositor.clone()));
    let vfs = Arc::new(Vfs::new(backends, xattrs, quotas, watcher, coord, bookmark, trash, panel));
    boot.lifecycle.on_instance_gone(|iid| vfs.on_instance_gone(iid));
    boot.lifecycle.on_manifest_reloaded(|bundle, snap| vfs.on_manifest_reloaded(bundle, snap));
    vfs
}
```

Init order is constrained by Round 1 `BootPhase`: `Vfs::init` runs in `BootPhase::FilesystemMounted` (between `BootPhase::Mounted9P` and `BootPhase::DisplayReady`). Panels and bookmarks can't be used until `DisplayReady`; the `PanelService` rejects requests with `Unsupported("display-not-ready")` before that phase.

---

## 13. Open Questions for the Critic

These are the weaknesses I see in this design — places where I expect (and want) the Critic to push back hard. Listed in roughly decreasing severity.

### Q1 — Path-resolution cost on the hot path

Every `read`/`write` in WASI flows through a Wasmtime resource-table lookup, which we currently don't intercept (the descriptor is held by `wasi:filesystem`). But every `open`/`mkdir`/`rename`/`stat`/`readdir` hits `Vfs::resolve`, which does: cwd-join, NFC normalize, casefold, longest-prefix mount match, sandbox check, canonicalize-no-symlink, allowed-root check. That's six string-walks and a hashmap lookup per call. For an `ls`-style app that does `readdir` + N × `stat`, this is N+1 resolutions. Is the path-resolution cache (`PathBucket` index) sufficient, or do we need a per-instance hot-fd cache that bypasses resolve after the first lookup? What's the budget here vs. the Round-3 IPC budget of ~50–80 ns?

### Q2 — xattr storage races between native and shadow

For a backend that *partially* supports xattrs (e.g., a hypothetical future ext4 with size limits), some keys go native and some go shadow. If an app first reads a key from the shadow DB, then the backend later admits the key, we have two sources of truth. The current design says "native takes precedence" but doesn't specify migration. Is the shadow→native migration synchronous on first write, or do we ever return the shadow value when the native is now authoritative? And what about *concurrent* writes from two apps?

### Q3 — Snapshot semantics during ongoing writes

The `snapshot(root, label)` operation is described as "atomic." But on 9P we emulate it as a recursive xattr walk, which is fundamentally not atomic. If app A is writing `note.md` while app B snapshots `/data/shared/`, B's snapshot could capture half of A's write. Do we need to (a) acquire all coordination write locks under the root before snapshotting (likely deadlock-prone), (b) accept torn snapshots and document it, or (c) require snapshots to go through a coordinator API? The current design implicitly chose (b), which is probably wrong.

### Q4 — Bookmark token forgery via reboot

Session-key-signed bookmarks die at reboot (new key). We mitigate with a derived long-term key. But the long-term key lives in `/data/.sys/keys/` — which is itself in 9P, on a host directory the user can inspect. An attacker with host-level access to `data/` can forge any bookmark. Is this in our threat model? If we want to defend against it, we'd need a TPM-backed key or boot-time prompt, neither of which we have. The Critic should decide whether to scope the threat model explicitly or upgrade the key custody story.

### Q5 — Watcher fairness and the noisy-neighbor problem

A misbehaving app can `subscribe(/data, Tree)` and then write a million tiny files; the inotify storm hits the global `WatcherCore` thread which fan-outs to all subscribers of any prefix of `/data`. Even with coalescing, the *coalescer itself* gets pummeled. The current 50-events-per-second-per-watch budget is per *subscriber*, but the producer side has no rate limit. Should we (a) put per-producer rate limits at the VFS `write()`, (b) make `WatcherCore` sharded by mount, or (c) accept that a writer hot loop will degrade FS-events latency for everyone?

### Q6 — Coordination locks vs. WASI `wasi:filesystem` descriptors

The coordination service knows about `vfs.open()` callers because we wrap the `VfsFile` in `QuotaCoordFile`. But Wasmtime's standard `wasi:filesystem` host implementation opens descriptors directly against the supervisor's view of the mount — it doesn't go through our `QuotaCoordFile`. If app A acquires a write coordination lock and then *also* opens via the standard WASI path, A's own writes via WASI don't bump the lock's `last_writer_seq`, breaking coordination correctness. Do we have to fork Wasmtime's `wasi:filesystem` impl to route through `Vfs`, or can we structure the preopens such that WASI sees the wrapped descriptors? This is a significant amount of work either way.

### Q7 — Panel app as a privilege boundary

The panel is a WASM app with `[capabilities.filesystem_full_disk] = true`. This means the panel app is the largest concentration of FS authority in the system; a bug in it can leak any path to any requesting app. The current design relies on the panel only returning the *user's* selection through a typed channel, but the panel app is in the same trust domain as user-installed WASM. Should the panel be moved into the supervisor process itself (no WASM, harder to update, but smaller TCB)? Or is the security boundary actually fine because the panel can only return bookmarks scoped to the requester, and the supervisor mints those?

### Q8 — Quota accuracy under sparse files and clones

A clone is 0 bytes against quota until materialization. A sparse file's `allocated` is less than `size`. If app A clones a 1 GB file and then writes one byte in the middle, materialization on most backends will allocate the full 1 GB even though only a small range was actually touched (depends on `FICLONE` granularity, COW block size, etc.). Our quota check uses `allocated`, but the user's mental model is `size`. Which should we charge to the app, and what do we tell users when they see `du` show 100 MB but the quota system says 800 MB?

### Q9 — Tmpfs and PSI memory pressure

Tmpfs is RAM-backed and counts against process memory. Under Round 2's PSI-based pressure response, a tmpfs-heavy app can be flagged as a memory hog when really the bytes are in `/tmp/<bundle>-<iid>/`. Should we (a) attribute tmpfs bytes to the bundle's memory quota in addition to its storage quota, (b) keep them separate and risk PSI killing innocent apps, or (c) page tmpfs to disk under pressure (a third storage tier)?

### Q10 — File coordination scope vs. cross-OS expectations

`NSFileCoordinator` on macOS is *advisory*: an app that doesn't call it can still write the file. Our design treats coordination as mandatory only if both writers use it; an app that opens directly via `wasi:filesystem` bypasses coordination (see Q6 too). This is OK for apps in our ecosystem but bad for any app porting from a Mach world where the OS enforced coordination via the file presenter API. Do we make coordination mandatory at the VFS layer (any open while a write lock is held returns `InUse`)? Or stay advisory?

---

## 14. Performance budget

Target latencies (warm cache, no contention, x86-64 desktop profile):

| Operation | p50 | p99 | Notes |
|-----------|-----|-----|-------|
| `Vfs::resolve` (cached) | 250 ns | 1 µs | Hash + 2 compares |
| `Vfs::resolve` (cold) | 8 µs | 30 µs | Includes NFC + casefold + backend stat |
| `open` (9P, file exists) | 80 µs | 250 µs | Dominated by 9P RPC |
| `read` (4 KB, mmap) | 1.5 µs | 8 µs | Memcpy to WASM linear mem |
| `read` (64 KB, SharedBuffer) | 6 µs | 25 µs | Zero-copy publish |
| `xattr.get` (shadow hit) | 4 µs | 15 µs | sled lookup |
| `panel.open` (modal) | 80 ms | 400 ms | Includes panel-app startup + first frame |
| `watch.subscribe` | 12 µs | 40 µs | Inotify add_watch + table insert |
| `fs-event delivery` (coalesced) | 200 µs | 2 ms | 100 ms tumbling window |
| `coordinate_write` (uncontended) | 2 µs | 8 µs | Mutex acquire + insert |
| `coordinate_write` (contended) | window + RTT | bounded by timeout | priority-inherited |
| `bookmark.open` | 15 µs | 60 µs | Validate HMAC + open |
| `snapshot` (1 GB tree, 9P emulated) | 80 ms | 300 ms | xattr CloneEdge walk |
| `trash(file)` | 100 µs | 1 ms | rename + 2 xattr writes |

These budgets assume the cold-cache populated; first-use of any path adds 5–20 µs for the NFC + casefold + bucket insert. The `PathBucket` LRU is sized at 4096 entries per app.

---

## 15. Boot-time integration

The `Vfs` is created in `BootPhase::FilesystemMounted`, between Round-1's `BootPhase::Mounted9P` and `BootPhase::IpcReady`:

```
BootPhase::Mounted9P            → /data mounted via virtio-9P (existing supervisor/src/mount.rs)
BootPhase::XattrStoreReady      → sled DB opened, integrity-checked
BootPhase::FilesystemMounted    → Vfs::init returns; all backends online
BootPhase::WatcherReady         → WatcherCore thread spawned, inotify fds open
BootPhase::IpcReady             → Round-3 router ready (waits for Vfs because routes include /sys mounts)
BootPhase::DisplayReady         → panel/bookmark services unlocked
BootPhase::AppsLaunched         → first user-space WASM app starts
```

The `FilesystemMounted` phase has hard preconditions: 9P must be online (otherwise persistent storage is unavailable; supervisor falls back to tmpfs-only mode and logs a `degraded` health flag), and the XattrStore must integrity-check (sled checksum verification on open; corruption → rebuild from backend xattrs).

---

## 16. Failure modes and degraded operation

### 16.1 9P unavailable

If virtio-9P fails to mount (host config error, network filesystem disconnected), the supervisor:
1. Logs a critical event
2. Mounts tmpfs at `/data` as a fallback (current behavior in `mount.rs`)
3. Sets `Vfs.persistent_storage_available = false`
4. Rejects any `bookmark.save` and `xattr.set` for persistent xattrs (sled DB is on RAM tmpfs, so all data is lost on reboot anyway — better to make the loss explicit)
5. Allows apps to launch but the package manager rejects installs

### 16.2 XattrStore corruption

If sled's integrity check fails on `XattrStore::open`:
1. Rename the corrupt DB to `xattrs.db.broken.<timestamp>`
2. Create a new empty DB
3. Walk all backends with `xattrs` capability and rebuild the shadow index for them (lossy: backends without native xattrs lose their shadow entries)
4. Mark all bookmarks as needing refresh

### 16.3 Quota DB corruption

Less critical than xattrs (quotas are derivable by walking each bundle's tree):
1. Rename `quota.db.broken.<timestamp>`
2. Walk all bundle dirs in `/data/`, recompute usage
3. Re-initialize ledger from walk
4. This can take 30+ seconds on a full disk; supervisor reports `BootPhase::FilesystemMounted` is degraded until walk completes

### 16.4 Watcher inotify exhaustion

Linux `/proc/sys/fs/inotify/max_user_watches` is finite (default 8K). When exhausted:
1. New `subscribe` calls return `Unsupported("watch-limit-reached")`
2. Existing watches continue to function
3. The supervisor logs a guidance event; apps SHOULD use `WatchScope::File` rather than `Tree` where possible
4. We don't auto-bump the sysctl (requires root privileges that we have but shouldn't exercise quietly)

### 16.5 Coordination deadlock between FS and IPC

The Round-3 wait-for graph spans IPC `call`s. We extend it to include FS coordination locks. A cycle detection failure is returned as `IpcError::WouldDeadlock` (for IPC origins) or `VfsError::CoordinationCycle` (for FS origins). The detection is fully synchronous; there's no deferred deadlock detection.

---

## 17. Test plan

| Test | Scope | Location |
|------|-------|----------|
| Path-resolution unit tests | `SandboxMap`, escape attempts, NFC, casefold | `supervisor/tests/fs_resolve.rs` |
| VfsBackend trait conformance | Each backend implements all required methods correctly | `supervisor/tests/fs_backend_conformance.rs` |
| Concurrent open/coord | Two actors, write lock, priority inheritance | `supervisor/tests/fs_coord.rs` |
| Snapshot + clone CoW | Snapshot a tree, modify children, verify isolation | `supervisor/tests/fs_snapshot.rs` |
| Watcher coalescing | Hot write loop, verify 100ms window | `supervisor/tests/fs_watcher.rs` |
| Quota enforcement | Per-bundle, cross-mount, partial writes | `supervisor/tests/fs_quota.rs` |
| Bookmark validation | Forgery, expiry, refresh | `supervisor/tests/fs_bookmark.rs` |
| Trash sweep | Retention, restore, cross-mount | `supervisor/tests/fs_trash.rs` |
| Panel end-to-end | Spawn panel app, simulate user selection, redeem bookmark | `supervisor/tests/fs_panel_e2e.rs` |
| WASI shim integration | App opens via wasi:filesystem, sandbox checked | `supervisor/tests/fs_wasi.rs` |
| Power-loss simulation | Kill supervisor mid-write, verify XattrStore integrity | `supervisor/tests/fs_crash.rs` |
| 9P backend down | Boot with no virtio-9P, verify degraded mode | `supervisor/tests/fs_degraded.rs` |

Smoke test: a new WASM app `apps/fs-smoke/` that exercises every WIT entry point and asserts expected results; included in `make smoke`.

---

## 18. Migration path from current state

Current code:
- `supervisor/src/mount.rs` mounts 9P at `/data`
- Apps with `filesystem = true` get raw access via Wasmtime preopened-dir

Phase 1 (this round):
- Introduce `Vfs` with `NinePBackend` only; the manifest's `filesystem = true` is mapped to `/data/<bundle>/` ReadWrite
- Add `XattrStore`, `QuotaLedger`, `WatcherCore`, `CoordinationService`
- Old apps continue to work via WASI; new APIs are opt-in
- Quotas start unlimited; warnings only

Phase 2:
- Switch on quotas (default 256 MB)
- Spawn panel app; migrate file-picker patterns
- Trash + bookmark go live

Phase 3:
- Add `TmpfsBackend`, `OverlayBackend`, `SysROBackend`
- Snapshot/clone surface
- Spotlight indexer apps

Phase 4:
- Power user features: encrypted volumes, multi-user, network mounts (NFS, SMB)
- The `VfsBackend` trait was designed for these from day one; no architectural changes needed

---

## 19. Summary

The VyomaOS filesystem is a supervisor-mediated VFS over pluggable backends, exposing the full macOS feature set (sandboxing, snapshots, clones, xattrs, watchers, coordination, panels, bookmarks, trash, quotas) through a clean WIT package (`vyoma:fs@0.1.0`) that extends rather than replaces WASI's standard filesystem interface. The design integrates with Rounds 1–3:

- `SandboxMap` lives in `PolicySnapshot`, hot-swappable via `ArcSwap` (Round 3 §8)
- File events ride the sharded IPC router with `Coalesce` overflow (Round 3 §4)
- Coordination locks share the Round 3 wait-for graph for cycle detection
- Large reads use `SharedBuffer` (Round 2/3 §6)
- All lifecycle is tied to `BootPhase` + `on_instance_gone` (Round 1)

22 implementation files, all ≤ 500 LOC, total ≈ 6,200 LOC. Performance budget: sub-microsecond cached resolves, ≤ 80 µs file opens, ≤ 400 ms first-frame panels.

The 10 open questions for the Critic are listed in §13, in order of severity. The most important ones are (Q1) the cost of path resolution on the hot path, (Q3) snapshot atomicity, (Q6) the coordination/WASI bypass, and (Q10) whether coordination is advisory or mandatory.
