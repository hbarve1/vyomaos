# Round 4 Critic: File System & VFS Layer

**Date:** 2026-05-29
**Round:** 4 of 80
**Subsystem:** File System & VFS Layer
**Critic verdict:** FUNDAMENTAL FLAWS

## Executive Summary

The proposed File System & VFS Layer for VyomaOS attempts to deliver an APFS-equivalent experience — snapshots, copy-on-write, file coordination, security-scoped bookmarks, Powerbox-style open/save panels, FSEvents-style watching, and Unicode case-insensitive matching — on top of a stack whose foundation is fundamentally incompatible with these semantics: a single-channel synchronous 9P virtio mount of a host directory, with a Linux 5.10 kernel that has no native COW filesystem support enabled (the kernel was deliberately built `allnoconfig` with no btrfs, no zfs, no APFS), and with all I/O funneled through a Rust PID-1 supervisor that must also handle window management, IPC routing, display compositing, and input dispatch.

The design is conceptually elegant if you assume the supervisor can magically multiplex 1000 FS operations per second across 50 apps through a 1-2 ms-per-roundtrip pipe, can transparently shadow-copy every write for snapshot semantics on a backing store that doesn't support snapshots, can intercept WASI Preview 2 filesystem calls that are already implemented by Wasmtime as a host capability (and thus opaque to user-space code), and can deadlock-free coordinate file locks across crash-prone untrusted WASM apps. None of these assumptions survive contact with the actual constraints.

This critique identifies **seven blocking issues** that require a fundamental rethink of the FS layer (not just incremental fixes), **nine significant issues** that demand redesign of specific subsystems, and **eleven design gaps** that the Architect's document either glosses over or treats as out-of-scope when they are actually in-scope. The single most damaging flaw is the assumption that the supervisor can sit on the synchronous path of every FS operation; this transforms what should be a parallelizable workload into a serialized one, and the resulting throughput ceiling (~500-1000 ops/sec) is two orders of magnitude below what a desktop-class OS requires.

The path forward is not to abandon the goals — desktop OSes do need snapshots, file coordination, sandbox enforcement, and file watching — but to (a) replace the 9P backing with a real Linux filesystem image (ext4 + LVM thin snapshots, or a dedicated overlayfs hierarchy), (b) push capability enforcement into Wasmtime's pre-opened-directory model rather than runtime path filtering, (c) accept that snapshot/COW semantics require kernel-level support and either build/enable it or scope the feature, and (d) move file watching to per-app inotify FDs rather than supervisor-mediated polling. The Architect's document, in its current form, would either ship a system that performs at 1990s-era throughput, or it would silently degrade APFS-equivalent semantics to "best effort" without admitting it.

## Critical Issues (blocking)

### C1. The 9P single-channel bottleneck makes supervisor-mediated FS architecturally infeasible

The 9P/virtio protocol used by the current VyomaOS storage stack is a **synchronous, single-channel RPC** between the guest and the host. Each FS operation (open, read, write, stat, close, readdir) is one or more 9P messages, each of which incurs a virtqueue round-trip — empirically ~1-2 ms on QEMU with KVM, slower without. The protocol itself does not multiplex independent operations across multiple queues; while the host can process them in parallel internally, the guest's 9P client serializes requests through the mount point's request queue.

Now layer the Architect's proposed VFS-mediation model on top:

1. App calls `wasi:filesystem.read(fd, offset, len)`
2. Wasmtime host import dispatches into supervisor's mediated VFS layer
3. Supervisor checks sandbox map → translates path → checks quota → checks file lock → performs `read()` syscall on `/data/...` 9P mount
4. Kernel 9P client sends `Tread` to host, awaits `Rread`
5. Supervisor copies bytes back into WASM linear memory
6. Returns

Even if every supervisor-side step is zero-cost (it is not), the 9P round-trip dominates. At 1.5 ms/op:

- **Single app, sequential reads:** ~666 ops/sec maximum
- **50 apps, mixed FS load:** the 9P pipe is the bottleneck; throughput remains ~666 ops/sec **system-wide**, divided across 50 apps = 13 ops/sec per app
- **Realistic mixed workload** (one app indexing 10k files while others read configs): the indexer alone needs ~10k stats + 10k opens = 20k ops, taking 30 seconds while every other app starves

The Architect's document mentions "supervisor mediates VFS" without engaging with the fact that this places the supervisor on the **synchronous critical path of every FS operation**. The supervisor is already responsible for:
- IPC routing (Round 3)
- Window compositing and damage flushing (Phase 17 — known performance gap per `MEMORY.md`)
- Mouse/keyboard input routing
- Process lifecycle and crash recovery (Round 1)
- Virtual memory accounting (Round 2)

Adding "synchronous arbiter of all FS ops" to this list creates a single point of contention that will choke the entire system. On macOS, VFS calls go through the kernel — which has multi-core lock-free fast paths. The Rust supervisor is a userspace process with conventional locking. Even with sharded RwLocks the ABBA risk is high (the recent compositor deadlock fix `55fd121` is a warning sign).

**Why this is blocking:** No amount of micro-optimization rescues a design where every FS op pays a 1.5 ms 9P RTT plus supervisor-mediation overhead. The fix requires either (a) replacing the 9P backing with a block-device-backed real filesystem (ext4 image on a virtio-blk device) so the supervisor can use ordinary parallel POSIX I/O, or (b) abandoning supervisor mediation and pushing capability enforcement entirely into Wasmtime's pre-opened-directory model.

### C2. WASI P2 filesystem interception is not a supported extension point

The Architect proposes that the supervisor "extend" or "wrap" `wasi:filesystem/types@0.2.0`. This betrays a misunderstanding of how Wasmtime implements WASI Preview 2:

- `wasi:filesystem` is part of `wasmtime-wasi`, a Rust crate that provides the host implementation of the WASI Preview 2 component-model interfaces
- The host implementation is registered with the `Linker` at engine setup time via `wasmtime_wasi::add_to_linker_async`
- It uses Wasmtime's resource/handle system; descriptors are typed component-model resources, not raw file descriptors
- Once registered, the dispatch from guest WASM into the host function is **internal to Wasmtime** — there is no public seam where a third-party supervisor can intercept the call between guest and host without modifying Wasmtime itself

The supervisor has three actual options, none of which the Architect's document discusses:

1. **Use `WasiCtxBuilder::preopened_dir()` to grant scoped directory handles, then let Wasmtime's stock filesystem impl handle everything.** This is the WASI-native model. The supervisor controls *what* the app can see (which preopened dirs) but not *how* each operation is mediated. Quota enforcement, file coordination, snapshots — none of these can be inserted into the stock impl.

2. **Implement a custom `WasiView`/`WasiFilesystem` host impl that replaces the stock one.** The supervisor would write its own filesystem backend that satisfies the `wasi:filesystem/types` interface. This is a substantial engineering project — Wasmtime's stock impl is ~3000+ lines covering descriptors, metadata types, advise/sync, symlinks, file/dir distinction, error mapping. And it must remain compatible with future WASI P2 revisions. Across 50+ apps, every call into this custom impl runs in the supervisor's address space, so the supervisor's own crash takes everything down.

3. **Pre-filter via filesystem isolation: bind-mount or chroot a per-app view, then let stock WASI hit it.** The Linux kernel does isolation; Wasmtime sees only the per-app view. This is the cleanest model but requires the supervisor to set up per-app mount namespaces, which means CAP_SYS_ADMIN, which means the supervisor runs as root (it does in VyomaOS — PID 1 in initramfs).

The Architect's document conflates all three of these without picking one, and the snapshot/coordination/quota features described require option (2) — a complete reimplementation of WASI filesystem inside the supervisor — which is a multi-thousand-line addition that the 500-line-per-file rule will fragment across dozens of files and which will lag every Wasmtime release.

**Why this is blocking:** The document promises VFS features that require option (2), but does not budget for the engineering cost or acknowledge the maintenance burden. Without picking the implementation strategy explicitly, the design is not actionable.

### C3. Snapshot semantics cannot be implemented above the backing store

The Architect proposes APFS-equivalent snapshots — atomic, instantaneous, space-efficient (COW), and queryable as read-only mounts. This is a **kernel-level filesystem feature**. APFS implements it because it controls the block layout: when you snapshot, no blocks are copied; the snapshot is a metadata pointer to the current block tree, and subsequent writes COW to new blocks.

The proposed VyomaOS backing is 9P over the host filesystem, plus an ext4 image for `/data`. Neither supports snapshots:

- **9P:** No snapshot protocol. The host filesystem may be ext4/APFS/btrfs but the guest doesn't see those features through 9P.
- **ext4 in `disk.img`:** ext4 does not have native snapshots. LVM thin-pool snapshots could work but require LVM in the kernel (`allnoconfig` does not enable LVM; you'd need to enable `CONFIG_MD`, `CONFIG_BLK_DEV_DM`, `CONFIG_DM_THIN_PROVISIONING`). The kernel size grows significantly.

The Architect's mitigation — "Copy-on-write semantics even if backend doesn't support it" — means the supervisor must:

1. Intercept every `write()` to every file
2. Before applying the write, check whether any snapshot references the about-to-be-overwritten blocks
3. If so, copy those blocks to a shadow location
4. Maintain a per-snapshot block-map data structure pointing to either the live blocks or the shadow blocks
5. On snapshot read, redirect reads to the correct (live or shadow) location

This is **reimplementing a COW filesystem in userspace** on top of a non-COW backing store. The data structures alone (per-file block bitmap, snapshot metadata, shadow extent allocator) are thousands of lines. The performance impact: every write that touches snapshot-referenced data triggers a read-modify-copy cycle. Storage overhead: a fully written-over file held in a snapshot occupies 2× the disk space (original + shadow copy).

Worse, the granularity is wrong. APFS COWs at the block level (4 KB). The supervisor can only see file-level operations through WASI P2 (read/write on fd, offset, len). To track which blocks of which files have been overwritten since a snapshot, the supervisor must impose its own block layer, or COW at file granularity (snapshot a 1 GB file when one byte changes → full file copy).

**Why this is blocking:** Userspace COW on a non-COW backing store is either grossly storage-inefficient (file-granular) or requires implementing a userspace block layer (massive complexity). The document treats this as a small implementation detail; it is a multi-quarter project that will dominate the supervisor's complexity budget.

### C4. Sandbox enforcement model is incompatible with WASI P2 capability model

The Architect proposes "translating all app paths through a sandbox map" at runtime. This contradicts how WASI Preview 2 grants filesystem capabilities:

- Apps receive **pre-opened directory descriptors** at startup
- Once an app has a descriptor for `/data/myapp/`, it can use that descriptor to open files relative to it, traverse subdirectories, follow relative paths within the tree
- The capability is the **descriptor itself**, not a path string
- Wasmtime enforces the boundary at the descriptor level: opening a file requires either an absolute path (which Wasmtime rejects unless it matches a preopen) or a relative path from a held descriptor (which Wasmtime confines to that descriptor's tree)

The Architect's "translate paths through sandbox map" model would require the supervisor to intercept every `path-open`, `path-stat`, etc. and rewrite the path. But (a) Wasmtime is already doing this enforcement, and (b) the app may use relative paths from held descriptors, which never appear as strings the supervisor can rewrite.

Furthermore, the document does not address:

- **Symlinks that escape the sandbox.** If the app's preopen contains a symlink pointing to `/data/other-app/`, what happens? Wasmtime's stock impl by default does **not** follow symlinks outside the preopen (correct). If the supervisor's mediation re-resolves the symlink at the path-rewriting layer, it must also enforce the no-escape rule, replicating logic Wasmtime already has.
- **`..` traversal.** Same as above.
- **Race conditions in TOCTOU.** App calls `path-stat` (supervisor rewrites path, stats, returns), then `path-open` (supervisor rewrites again — but the path may have been replaced with a symlink in between).
- **Hardlinks.** Linux allows hardlinking across directories. If two app sandboxes share the host filesystem, hardlinks can break isolation.

**Why this is blocking:** The sandbox model contradicts the WASI P2 capability model. The fix is to **use Wasmtime's preopens as the sandbox** and stop trying to do runtime path rewriting. The Architect's document needs to be rewritten to align with how WASI P2 actually grants filesystem access.

### C5. Powerbox/Open Panel security has no cryptographic foundation

The proposal for a supervisor-rendered file picker that grants user-driven access to files outside the app's sandbox is conceptually correct — this is the right model for cross-sandbox file access. But the implementation as described has a glaring security hole:

1. User picks `/data/notes/diary.txt` in the supervisor's file picker
2. Supervisor must communicate "App X is now allowed to read `/data/notes/diary.txt`" to the WASI runtime
3. Future reads must check this allowance

The Architect's design uses IPC for this, but **IPC in VyomaOS is line-based, untrusted, and forgeable**. Any app with `shell = true` can send `@supervisor: ...` messages. If the bookmark mechanism is also IPC-based (`@supervisor: grant_bookmark app=mail file=/data/notes/diary.txt`), then any app can forge a grant for itself.

Even without forgery via IPC, the document doesn't address:

- **Persistence of bookmarks.** APFS security-scoped bookmarks survive across app restarts and even reboots. VyomaOS would need to persist `(app_id, file_path, granted_at)` tuples in a tamper-proof store. The `/data/installed.txt` model is plaintext and editable.
- **App identity binding.** A bookmark grant should be bound to the app's identity (Round 1 `AppIdentity`). If App X is uninstalled and reinstalled (or replaced with a malicious version), the bookmark should not transfer. This requires content-addressable app identity (WASM module hash), not just app name.
- **Revocation.** User uninstalls Notes app. Mail's bookmark to `/data/notes/diary.txt` should be revoked. How?
- **Powerbox's own filesystem access.** The supervisor's file picker needs to browse the entire user-accessible filesystem to render the picker UI. This means the supervisor has unconstrained read access to everything — making the supervisor a single point of total-compromise.

**Why this is blocking:** The security model for cross-sandbox file access via Powerbox is not viable without cryptographic app identity (which the Round 1 architect supposedly defined, but the FS document does not reference) and a tamper-proof bookmark store. The current sketch would allow any app to grant itself arbitrary file access.

### C6. File coordination with crash recovery is under-specified to the point of unsafety

NSFileCoordinator on macOS provides advisory locking with deadlock detection and is famously hard to use correctly. The Architect proposes a supervisor-mediated equivalent, but does not address:

- **What is the lock granularity?** File? Byte-range? Inode? Path?
- **What are the semantics under crash?** App A acquires write lock, crashes mid-write. Supervisor detects crash via the Round 1 `CrashKind` mechanism. Does it:
  - Release the lock immediately? (Risk: half-written file is now visible to other readers)
  - Mark the file as corrupted? (Need a recovery mechanism)
  - Roll back to a checkpoint? (Need transactional FS, which 9P/ext4 don't provide)
- **Priority inversion.** Low-priority background indexer holds read lock on a file; high-priority UI app needs write lock. The indexer holds for 30 seconds. UI freezes. macOS has lock priority promotion; the VyomaOS supervisor would need the same, plus a definition of "priority" that doesn't exist elsewhere in the system.
- **ABBA deadlock between locks.** App A acquires lock on file X then requests Y; App B acquires Y then requests X. The supervisor needs cycle detection in its lock graph. Not mentioned.
- **Lock leak across IPC boundaries.** App A holds lock; sends file path to App B via IPC; B tries to write. Is the lock transferred? Inherited? Re-acquired? Undefined.
- **Performance.** A central lock manager in the supervisor adds another mutex to every file open/close.

**Why this is blocking:** File coordination is one of the hardest features in any OS. The current sketch lacks the mechanics for deadlock detection, crash recovery semantics, priority handling, and lock-graph management. Shipping it as designed would produce silent data corruption.

### C7. File watching cannot use `inotify` on the 9P mount point

The Architect proposes FSEvents-style file watching with low-latency change notifications. The natural Linux primitive is `inotify`. But:

- 9P mounts in Linux **do not consistently support inotify**. The 9P kernel client does not propagate change events from the host filesystem to guest inotify watchers, because the 9P protocol has no event/notification mechanism — it is request/response only.
- Even on the host side, changes made *by the host* (not the guest) to the underlying directory are invisible to the guest until the guest opens the file or stats it (and even then, caching can hide them).
- Changes made *by the guest* via 9P are visible to other guest processes only after they invalidate their caches — and inotify on the 9P mount may or may not fire.

The fallback is polling. The Architect's document hand-waves polling, but the math is brutal:

- 50 apps × 10 watchers each = 500 watched paths
- Polling interval of 200 ms (already noticeably laggy compared to APFS FSEvents)
- 500 paths × 5 polls/sec = 2500 stat ops/sec, **just for file watching**
- Through the 9P bottleneck at 1.5 ms/op: 2500 × 1.5 ms = **3.75 seconds of 9P bandwidth per second** — i.e., polling alone exceeds the 9P channel's capacity

Even a sparser polling regime (every 1 second) consumes most of the 9P bandwidth and gives users a 1-second perceived lag for file change notifications — wildly worse than macOS's FSEvents (~10 ms typical latency).

**Why this is blocking:** File watching at desktop scale on a 9P mount is infeasible. The fix is to (a) move off 9P to a backing where inotify works (ext4 on virtio-blk), or (b) have the supervisor watch on the host side (it has 9P access too) and push events into the guest via IPC, which moves the problem to "how does the supervisor get host-side inotify events into the guest." Neither is in the document.

## Significant Issues (important)

### S1. Quota enforcement race condition has no atomic primitive

The proposed quota model checks usage, then writes, then increments. This is the classic check-then-act race. The Architect would need:

- A per-app atomic usage counter (probably `AtomicU64` per app)
- Compare-and-swap on every write operation
- Reservation tokens for multi-chunk writes (reserve N bytes, write, commit or release)

The supervisor's 500-line file limit will fragment this logic. And because the supervisor is the sole arbiter, the quota counter becomes another contention point. With 50 apps writing concurrently, the CAS loop will see high retry rates under load.

Additionally, the quota must be **persistent**. Crash mid-write must not produce a quota underflow. This means writing usage updates to disk on every change (too slow) or accepting that crashes may briefly over-report usage (drift) and reconciling on startup (full filesystem walk to recompute usage).

### S2. Case-insensitive Unicode path matching is far more expensive than implied

APFS uses NFD normalization + Unicode case folding. The Architect's "do this in the supervisor" approach requires:

- Every path lookup: normalize the input path (NFD)
- Every directory traversal: list the directory, normalize and case-fold every entry, compare
- Cache invalidation: when an entry is added/removed, the cache for that directory is invalidated

Performance on a directory with 10,000 entries: each path lookup is O(N) in directory size. Cumulative cost for an indexer scanning the filesystem is catastrophic.

APFS gets away with this because it stores normalized+casefolded keys in the on-disk B-tree, so lookup is O(log N). VyomaOS on ext4 has no such index — ext4 stores raw bytes case-sensitively. The supervisor would need to maintain a parallel index, doubling write costs and consuming RAM.

The simpler answer: **document that VyomaOS is case-sensitive**, like Linux. This breaks compatibility with apps that expect APFS semantics but is the only honest choice given the backing store.

### S3. Symlink and hardlink semantics across sandboxes are unaddressed

If two apps' sandboxes share the underlying filesystem:

- Hardlink from `/data/app-a/secret.txt` to `/data/app-b/exposed.txt` breaks app-a's confidentiality
- Symlink from inside one sandbox to another path is rejected by Wasmtime (good) but the file still exists in the backing store

If sandboxes are separated via mount namespaces (one per app), hardlinks across cannot exist (different filesystems). But mount namespaces in the supervisor require CAP_SYS_ADMIN and complicate the file picker (which needs a unified view).

The document does not pick a model.

### S4. Concurrent open of the same file by multiple apps is undefined

The Architect treats files as if they have one "owning" app per sandbox. But:

- Shared files (config, dictionaries, user data) are accessed by many apps
- POSIX semantics: concurrent readers fine, concurrent writers race
- The file coordination layer (C6) is supposed to mediate this — but as noted, it is under-specified

A specific case: 10 apps simultaneously open the same shared dictionary file for read. Each opens a separate fd. The supervisor's mediated VFS now holds 10 descriptors for the same file. Memory cost per fd, lock cost on opens, and read cost (do they share a buffer cache? if so, cache invalidation across apps is supervisor's problem; if not, 10× memory).

### S5. Persistent storage corruption recovery is not specified

If the supervisor crashes mid-write to a file, the 9P/ext4 backing may have a partial write. The Architect should specify:

- Are writes atomic? (POSIX says no for >1 sector)
- Is there a write-ahead log?
- Is there a `fsync` policy?
- What happens on next boot — is there a file system check?

The current VyomaOS already creates `disk.img` once and reuses it. There is no fsck on boot. Corruption is silent. The proposed VFS layer makes this worse by introducing more state (snapshot metadata, quota counters, lock tables) that can be inconsistent with on-disk reality after a crash.

### S6. The supervisor's own crash takes down all FS state

If the supervisor mediates everything and the supervisor crashes:
- All in-flight FS ops are lost
- All locks held by apps are silently leaked
- The supervisor restarts (per Round 1 PID-1 recovery) and has no record of who held what
- Snapshot metadata in supervisor memory is lost unless persisted

The document does not specify which supervisor state is persisted and which is volatile. Without this, supervisor crash recovery cannot restore the FS layer to a consistent state.

### S7. No model for "files larger than RAM"

WASI P2 reads return bytes to the WASM linear memory. The supervisor mediation must buffer the read. For a 1 GB file:
- App requests `read(fd, 0, 1_000_000_000)`
- Supervisor reads 1 GB from disk
- Copies 1 GB into the WASM instance's linear memory (which has its own size limits)

A 32-bit WASM linear memory is capped at 4 GB. Even with WASM64 (rare), the supervisor's own RSS would grow by the file size. The Architect must specify a streaming model (small reads, application-level buffering) and document that mmap is not available.

### S8. Atime/mtime updates create write amplification

Every read may update access time (`noatime` mount mitigates). Every write updates mtime. On 9P, each metadata update is a round-trip. For a workload doing many small reads, atime updates dominate.

The document should specify mount options (`noatime`, `relatime`, `lazytime`) and the semantic implications.

### S9. Directory listing performance is bounded by 9P

`readdir` on 9P walks the directory and ships entries over the channel. For 10k-entry directories, this is many round-trips. The supervisor's file picker (Powerbox) listing the user's data tree may take seconds. The document does not address pagination, async listing, or background indexing.

## Design Gaps

### G1. No specification of `/proc`-like introspection

A desktop OS needs to expose process and system information through the filesystem (macOS uses `sysctl` and various pseudo-filesystems). VyomaOS apps may want to query their own status, but the document does not specify whether there is a `/proc` analog.

### G2. No model for device files

`/dev/fb0` (framebuffer) is currently accessed by display-capable apps via raw open. The proposed VFS layer should specify how device files are exposed (or not) through WASI P2, given that WASI does not have a notion of device files.

### G3. No model for FIFOs / Unix sockets in filesystem

IPC in VyomaOS uses `@<app>:` stdout protocol (Round 3). But many Unix apps expect to communicate via sockets/pipes in the filesystem. The document doesn't address whether `mkfifo` or `bind()` to filesystem paths is allowed.

### G4. No specification for `tmpfs` / scratch space

Apps need ephemeral storage (caches, temp files). The document mentions persistent `/data` but not `/tmp` or per-app scratch directories. RAM-backed scratch is essential for performance.

### G5. No model for compressed/deduplicated storage

APFS does inline compression and dedup. The Architect's design implies bit-exact storage in the backing. Documenting that "no compression" is a deliberate choice (or proposing how to add it) would be useful.

### G6. No model for encryption at rest

APFS supports per-volume and per-file encryption. The VyomaOS document does not address encryption — neither at the supervisor mediation layer nor at the backing layer. For a desktop OS handling user data, this is a significant gap.

### G7. No model for spotlight-style indexing

Desktop search requires a metadata index. The document does not specify whether VyomaOS will index file content, where the index lives, who maintains it, and how it integrates with the file picker.

### G8. No model for extended attributes

macOS xattrs are heavily used (quarantine bits, custom icons, Spotlight metadata, security-scoped bookmarks). WASI P2 does not expose xattrs. The document doesn't say whether VyomaOS supports them.

### G9. No model for filesystem events to the desktop shell

The window manager, dock, file picker, and other shell components need to see file events. The watching mechanism in the document focuses on per-app subscriptions, but the shell's own observation needs are not addressed.

### G10. No model for read-only vs read-write mounts

Some directories should be read-only (system binaries, base assets). Some are per-app writable. Some are shared. The document does not specify the mount-point hierarchy or which roots are RO/RW.

### G11. No backpressure / flow control

If one app does heavy I/O, what happens to others? The 9P channel is a shared resource. Without explicit flow control or rate limiting, one misbehaving app can starve the rest of the system. The document does not specify a fair-share scheduling discipline.

## Points of Strength

Despite the blocking issues, the Architect's document contains several worthwhile foundations:

- **Recognition that desktop semantics require more than POSIX:** the document correctly identifies coordination, snapshots, watching, sandboxing, and Powerbox as essential features beyond raw POSIX. This sets the right scope, even if implementation is under-specified.

- **The aspiration toward APFS-equivalent semantics is correct:** users have come to expect snapshots and Time Machine-like rollback. Aiming for these (even if the path is unclear) is better than capitulating to POSIX minimums.

- **Supervisor-mediated Powerbox is the right user model:** putting the file picker in a trusted UI surface (the supervisor) rather than the app is the only secure way to do cross-sandbox file access. This is the right *user-facing* model, even if the *security mechanics* are under-specified.

- **Per-app sandbox quotas are the right policy:** quotas are needed to prevent one app from exhausting disk. The policy is right; the implementation needs work.

- **File watching as a first-class capability is correct:** apps need change notifications, and treating this as a VFS feature (not bolted on) is the right design call.

- **Reuse of Round 1 `CrashKind` and `AppIdentity`:** the document implicitly references prior rounds, which is good for design coherence — but it should make these dependencies explicit and verify the contracts still hold.

## Synthesis Recommendations

### R1. Replace the 9P backing with a real Linux filesystem

The single most impactful change is to **stop using 9P virtio as the primary backing**. Instead:

- Mount the `disk.img` as a **virtio-blk device** with ext4 (already partially done; promote this to the primary `/data` backing)
- Use the 9P mount only for **host-developer-facing** access (e.g., the developer's `data/` directory mounted into the VM for inspection during build/dev)
- All app-facing FS operations go through the local ext4
- ext4 supports `inotify` natively, has parallel I/O, and removes the 1.5 ms-per-op ceiling

This alone resolves C1 (bottleneck), C7 (inotify), partially C3 (snapshot path becomes possible via LVM thin or by enabling btrfs/zfs), and S9 (directory listing performance).

### R2. Make Wasmtime's preopened directories the sandbox enforcement primitive

Stop proposing runtime path rewriting. Instead:

- At app spawn, the supervisor opens each capability-granted directory via `WasiCtxBuilder::preopened_dir()`
- App receives a descriptor; Wasmtime's stock impl enforces all path resolution within that descriptor
- Bookmark grants from Powerbox translate to **additional preopens** for that app's WASI ctx (requires component-model resource transfer — feasible via `wasi:filesystem/preopens` extension)
- This resolves C4 and most of S3

### R3. Pick one of three FS-mediation strategies explicitly, and budget accordingly

Choose:

(a) **Pure preopen model (minimal mediation):** No quota, no snapshot, no coordination at the FS layer. Apps see what the supervisor preopened; Wasmtime handles everything else. Simple, fast, but loses APFS-equivalent features.

(b) **Custom WASI host impl (full mediation):** Supervisor implements every WASI filesystem call. Maximum control, supports all desktop features, but is ~5000+ LoC across many supervisor modules and must be maintained against Wasmtime upstream.

(c) **Hybrid via per-app namespaces + custom shim for cross-cutting concerns:** Use Linux mount namespaces for sandboxing; intercept only specific operations (open, write) via a shim layer for quota/snapshot/coordination. Middle ground in complexity.

The document must pick one and engage with its tradeoffs. **Recommendation: start with (a) for v1, document the loss of features, and plan (c) for v2.** Do not promise APFS semantics on (a).

### R4. Defer snapshot/COW until a kernel-level filesystem feature is enabled

Either:

- Enable btrfs in the kernel build (`CONFIG_BTRFS_FS`), pay the kernel size cost (~1 MB), use btrfs subvolume snapshots
- Or enable LVM thin provisioning, use LVM snapshots
- Or defer the snapshot feature entirely until a future phase

**Do not attempt userspace COW on ext4.** It is too complex and too slow. Document the deferral explicitly. The "snapshot semantics even if backend doesn't support it" promise must be retracted.

### R5. Specify the security-scoped bookmark format with cryptographic binding

Define explicitly:

```
Bookmark := {
  app_id: ContentHash(wasm_module),    // from Round 1
  resolved_path: PathBuf,
  granted_at: SystemTime,
  granted_by_user: bool,
  hmac: Hmac<Sha256>,                  // keyed by supervisor secret
}
```

- Persisted in a supervisor-controlled `bookmarks.db`
- HMAC prevents IPC forgery
- Bound to content hash of the WASM module (uninstall + reinstall of a *different* module breaks the bookmark)
- Resolves C5

### R6. Replace the proposed file coordination with copy-on-write atomic rename

Instead of NSFileCoordinator-style locks (which are deadlock-prone and complex), use the proven Unix idiom:

- Apps write to a temp file in the same directory
- On commit, atomically `rename()` over the target
- Reads see either the old or new version, never a partial write
- No locks; no coordination

For apps that genuinely need shared-write semantics (databases, logs), expose a higher-level "shared journal" capability rather than generic file coordination. Keep the FS layer simple.

### R7. Specify a per-app `/tmp` scratch directory backed by `tmpfs`

Mount `tmpfs` at `/tmp` inside the guest, expose a per-app subdir as a WASI preopen. Resolves G4.

### R8. Document case sensitivity, encoding, and Unicode semantics

State explicitly: "VyomaOS is case-sensitive, byte-oriented, with no Unicode normalization at the FS layer. Apps that need normalization handle it at the application layer." This is honest and avoids the supervisor-side Unicode performance disaster (S2).

### R9. Specify backpressure and fairness

Add to the design: per-app I/O rate limits enforced by the supervisor (max bytes/sec, max ops/sec). Deficit round-robin scheduling for fairness. This prevents one app from starving the rest. Resolves G11.

### R10. Persist supervisor FS state to a small write-ahead log

Quota counters, lock tables, snapshot metadata: all of this must survive supervisor crashes. Persist to a small WAL in `/data/.vyoma/state.log`, replay on supervisor start. Specify the WAL format and recovery semantics. Resolves S5, S6.

### R11. Make file watching push-based via supervisor inotify aggregation

The supervisor owns one set of `inotify` watches (against the now-ext4 backing). On events, it dispatches to subscribed apps via IPC. Apps subscribe via a `wasi:vyoma/fs-watch` interface (custom extension). Resolves C7 properly.

### R12. Acknowledge the engineering scale

The FS layer as proposed (even after these recommendations) is a multi-quarter project. The 500-line file rule will fragment it across 20-30 modules. The maintenance burden grows with every Wasmtime release. The document should include a phased rollout plan:

- **Phase A:** Replace 9P with ext4 backing; preopen-based sandboxing; basic per-app dirs (minimal viable FS)
- **Phase B:** Powerbox + bookmarks (cross-sandbox)
- **Phase C:** Watching, quotas, atomic rename pattern
- **Phase D:** Snapshots (only after kernel filesystem upgrade)
- **Phase E:** Indexing, search, encryption

Without this phased structure, the design is unbuildable as a single deliverable.

---

## Verdict Detail

The verdict of **FUNDAMENTAL FLAWS** is assigned because the design as written:

1. Sits the supervisor on the synchronous critical path of all FS ops (C1) — unfixable without architectural change
2. Misunderstands the WASI P2 capability model (C2, C4) — requires rewriting the document's enforcement model
3. Promises features (snapshots, COW) that the backing store cannot provide (C3) — requires retraction or kernel upgrade
4. Has security holes in the Powerbox/bookmark model (C5) — requires cryptographic foundation
5. Under-specifies file coordination to the point of being unsafe (C6) — requires either redesign (atomic rename) or major addition (deadlock detection, priority promotion)
6. Cannot deliver low-latency file watching on the 9P backing (C7) — requires backing replacement

Each of these is a "go back and rethink" issue, not a "tweak in review" issue. The Synthesis Recommendations above provide a viable path: replace 9P with ext4 on virtio-blk, use Wasmtime preopens for sandboxing, defer snapshots, use atomic rename instead of coordination, cryptographically bind bookmarks, aggregate watching in the supervisor. With these changes, a workable FS layer is feasible.

Without these changes, the FS layer will either ship at 1990s-throughput performance or silently fail to deliver its promised semantics. Neither outcome is acceptable for a desktop OS targeting APFS-equivalent user experience.

The Architect's next iteration must:
- Explicitly pick one mediation strategy (R3)
- Replace 9P (R1)
- Align with WASI preopens (R2)
- Retract or scope snapshots (R4)
- Specify bookmark cryptography (R5)
- Adopt atomic rename in lieu of coordination (R6)
- Provide a phased rollout (R12)

If the next round delivers on these, the verdict can be revised to SIGNIFICANT ISSUES. As written, the FS layer is not buildable as a coherent system.

## Appendix A: Concrete Performance Math

### A.1 9P round-trip cost breakdown

A single 9P `Tread`/`Rread` round-trip in QEMU/KVM costs approximately:

- Guest issues `read()` syscall → kernel 9P client → ~10 µs
- 9P client constructs `Tread` message → virtqueue ring buffer write → ~5 µs
- Virtqueue notify (eventfd or MMIO write) → host vCPU exit → ~20 µs
- Host QEMU 9P server processes `Tread` → reads from underlying host FS → 100-500 µs (depends on host cache state)
- Host writes `Rread` to virtqueue → notifies guest → ~20 µs
- Guest 9P client wakes, copies bytes to user → ~10 µs
- **Total: ~165 µs best case (warm host cache), ~1500 µs typical (cold cache, contention)**

This assumes no syscall-level batching. For small reads, the per-op overhead dominates the actual data transfer.

For workloads doing **many small ops** (the desktop common case — opening configs, reading icons, statting paths in a directory listing), the per-op cost is the binding constraint, not bandwidth.

### A.2 Throughput projections at 50 apps

Assume 50 apps with a Pareto distribution of FS demand:

- 5 "heavy" apps (file indexer, media editor, code editor): 100 ops/sec each = 500 ops/sec
- 15 "moderate" apps (browser, mail, terminal): 20 ops/sec each = 300 ops/sec
- 30 "light" apps (background daemons, status apps): 2 ops/sec each = 60 ops/sec
- **Aggregate demand: 860 ops/sec**

With 9P at 1.5 ms/op average: **666 ops/sec ceiling**

The aggregate demand (860) exceeds the ceiling (666) by ~30%. The system runs in saturation. Latency for any individual op grows unbounded as the queue depth increases. Tail latencies spike into seconds.

With ext4 on virtio-blk (typical 50-200 µs/op): **5000-20000 ops/sec ceiling** — comfortably absorbs the load.

This is the most direct quantitative argument for R1 (replace 9P).

### A.3 Storage overhead for userspace COW

Consider a 1 GB user file (e.g., a video being edited) with three snapshots taken at different points. The user edits the file: each edit touches a different region.

- Snapshot 1 (T=0): references all blocks of the 1 GB file
- User edits bytes 0-1000: supervisor shadow-copies the first block (4 KB) for snapshot 1
- Snapshot 2 (T=1): references current blocks; first block is new, rest are shared with T=0
- User edits bytes 100MB-101MB: supervisor shadow-copies that region for snapshot 1 *and* snapshot 2
- ... and so on

At file-granular COW (the simpler implementation), every snapshot pins a full 1 GB. Three snapshots = 4 GB storage for 1 GB of user data.

At block-granular COW (4 KB blocks, supervisor-implemented), the supervisor needs:
- Per-file block bitmap: 1 GB / 4 KB = 256K blocks → 256K bits = 32 KB bitmap per file per snapshot
- Block extent map for shadows: hash table / btree, ~16 bytes per shadowed block
- For 1000 files × 3 snapshots: 1000 × 3 × 32 KB = 96 MB of bitmap metadata in supervisor RAM
- Plus shadow extent metadata: scales with edit volume

The supervisor's RSS grows with user data size. This is unsustainable for any reasonably-sized workload.

### A.4 Lock manager contention

Consider 50 apps, each acquiring/releasing file locks at ~10 ops/sec:

- 500 lock ops/sec hitting a central lock manager
- If implemented as a single `Mutex<HashMap<PathBuf, LockState>>`: contention spikes
- If implemented as sharded RwLock (e.g., 64 shards by path hash): ~8 ops/sec/shard, lower contention but more cache misses
- Each lock acquire must also check the lock-graph for cycles (deadlock detection) — graph traversal cost is O(V+E) in worst case
- Crash-recovery: on app crash, supervisor must walk lock table to find all locks held by the crashed app and release them — O(N) in total lock count

This is a non-trivial subsystem with its own performance profile. The 500-line file limit will force it across multiple modules: `lock_manager.rs`, `lock_graph.rs`, `lock_recovery.rs`, `lock_priority.rs` — each adding its own state to be persisted on supervisor crash.

## Appendix B: Comparison with Reference Implementations

### B.1 macOS APFS + VFS

APFS provides snapshots, COW, encryption, case-folding, and inline compression — all at the kernel block layer. The VFS above it (Mach VFS) is multi-decade-mature. NSFileCoordinator runs in userspace via a system daemon (`filecoordinationd`) using XPC; it has known deadlock failure modes but is generally usable. FSEvents uses kernel-level event taps; latency is sub-10 ms.

VyomaOS proposes implementing equivalents in a Rust userspace supervisor on top of a 9P mount. The implementation gap is enormous; the Architect's document does not acknowledge it.

### B.2 Windows NTFS + IO Manager

NTFS provides snapshots (VSS), journaling, ACLs, alternate data streams. The IO Manager mediates filesystem access via a stacked filter-driver model. ReadDirectoryChangesW provides file watching. Process-Monitor-style ETW tracing exposes every FS op for debugging.

VyomaOS lacks an equivalent filter-driver layer; the supervisor-mediated model is the analog but is single-threaded.

### B.3 Linux ext4 + VFS + inotify

The reference Linux model: ext4 on block device, kernel VFS, inotify for watching, fanotify for filtering, flock/fcntl for locks. This is what VyomaOS should reach toward in v1 — drop the 9P, use ext4 + inotify, layer per-app sandboxing via mount namespaces or Wasmtime preopens. APFS-equivalent snapshots come later if at all.

### B.4 Fuchsia FIDL filesystem

Fuchsia's filesystem is a service exposing FIDL protocols (`fuchsia.io.Directory`, `fuchsia.io.File`). Sandboxing is by capability — each component receives only the directories its manifest grants. This is the closest analog to what VyomaOS should aim for, and the model the Architect's document gestures at but does not commit to.

## Appendix C: Suggested Module Layout

If the Architect adopts the recommendations, the supervisor's FS subsystem under `supervisor/src/fs/` might look like:

- `mod.rs` — public types (`FsSubsystem`, error types) — under 100 lines
- `backing.rs` — virtio-blk + ext4 mount setup — ~200 lines
- `preopen.rs` — per-app preopen builder, integration with `WasiCtxBuilder` — ~200 lines
- `bookmark.rs` — security-scoped bookmark store + HMAC — ~300 lines
- `bookmark_store.rs` — persistence + WAL — ~250 lines
- `powerbox.rs` — file picker UI — ~400 lines
- `quota.rs` — atomic per-app counters + reservation tokens — ~250 lines
- `quota_persist.rs` — WAL replay — ~150 lines
- `watch.rs` — supervisor-side inotify aggregator — ~300 lines
- `watch_dispatch.rs` — per-app event routing via IPC — ~200 lines
- `atomic_rename.rs` — helper for safe writes — ~100 lines
- `tmpfs.rs` — per-app scratch dir setup — ~150 lines

Total: ~12 modules, ~2500 lines. Snapshots intentionally omitted from v1.

This is a realistic engineering estimate. The current document does not budget for it.

## Appendix D: Open Questions for the Architect

The following must be answered before this design can proceed:

1. Which mediation strategy (R3) is chosen? Pure preopen, custom WASI host impl, or hybrid?
2. Is 9P being replaced by ext4 on virtio-blk, or retained?
3. Are snapshots in scope for v1, deferred, or out of scope entirely?
4. What is the AppIdentity format from Round 1, and how does it bind to bookmarks?
5. What is the file coordination model — atomic rename only, or full lock manager?
6. Does the supervisor have CAP_SYS_ADMIN? (Affects mount-namespace options for sandboxing.)
7. What is the supervisor-state persistence model — WAL, snapshots, none?
8. Case sensitivity: case-sensitive (Linux native) or case-folded (APFS-equivalent, expensive)?
9. Is `/tmp` (tmpfs) in scope? Per-app, shared, or both?
10. What is the encryption-at-rest story, if any?

Until these are answered, the FS layer cannot be implemented.

---

**End of Round 4 Critic critique.**

