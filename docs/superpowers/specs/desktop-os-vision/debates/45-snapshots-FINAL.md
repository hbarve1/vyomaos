# FINAL Spec: Time Machine & Snapshots (Round 45)

**Subsystem**: Time Machine & Snapshots  
**macOS Analogue**: `Time Machine` / `APFS snapshots` / `tmutil`  
**Depends on**: R04 (VFS), R41 (VYOMA_FS:watch, transactional writes), R43 (quiesce protocol), R46 (backup disk mount)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Snapshot Primitive

**Hardlink-tree copy** (Time Machine on HFS+ model) — NOT btrfs, NOT APFS CoW, NOT tar.

Justification: `/data` is ext4 over 9P virtio; switching FS breaks storage architecture. Tar snapshots are opaque (single-file restore requires full extraction). The hardlink-tree model works on ext4: `cp -al` creates a directory tree where unchanged files are hardlinks to the previous snapshot's inode — deduplicating storage. Snapshots ARE real directory trees, enabling O(1) single-file browse/restore.

**9P mtime unreliability (B1 fix)**: `security_model=mapped-xattr` makes `st_mtime` unreliable for `/data` files. Change detection uses a persistent journal at `/data/.tm/journal.toml` recording `path → (size, first_4KB_sha256)` — NOT mtime. The journal is updated atomically after each snapshot via R41 `write_begin`/`write_commit`. Small files (&lt;256KB): content-hash comparison. Large files: size-only.

**Destination**: separate ext4 disk image `out/backup.img` (default 256 MB), mounted at `/backup`. New Makefile targets: `make backup-disk` (creates image), `make run-backup` (adds `-drive file=$(BACKUP_DISK),...,serial=vyoma-backup`).

---

## 2. Backup Destination

```rust
// supervisor/src/snapshot/dest.rs  (~80 lines)
pub fn try_mount_backup() -> bool {
    for dev in &["/dev/vdb", "/dev/vdc", "/dev/sdb"] {
        let ret = unsafe {
            libc::mount(c_dev, c"/backup", c"ext4", libc::MS_NOATIME|MS_NODEV|MS_NOEXEC|MS_NOSUID, ptr::null())
        };
        if ret == 0 { log_info!(...); return true; }
    }
    log_warn!(..., "no backup volume found — snapshots disabled");
    false
}
```

`/backup` pre-created as empty dir in initramfs (`rootfs.sh: mkdir -p /backup`). Mounted `MS_NODEV|MS_NOEXEC|MS_NOSUID`.

---

## 3. Snapshot Schedule

Dedicated `snapshot-timer` thread wakes every 60s. Retention policy:
- Keep all hourlies for 24h
- Keep one daily (first snapshot of UTC day) for 30 days
- Keep one weekly (first of ISO week) for 52 weeks
- Delete everything older

**Prune-before-snapshot (B3 fix)**: prune runs BEFORE the new snapshot pass to reclaim space first. Pre-flight free space check via `libc::statvfs` — skip snapshot if free &lt; 10% of backup volume, emit `VYOMA_SNAP:disk-full`. `prune_failed: bool` flag in scheduler retries prune on next tick.

```rust
// supervisor/src/snapshot/scheduler.rs  (~120 lines)
pub struct SnapshotScheduler {
    last_hourly:  Option<u64>,
    prune_failed: bool,    // B3 fix
}
impl SnapshotScheduler {
    pub fn tick(&mut self, now_unix: u64, engine: &SnapshotEngine) {
        self.prune(now_unix, engine);    // B3: prune first
        if !self.check_free_space() {   // B3: space check
            engine.notify_disk_full();
            return;
        }
        if self.should_run_hourly(now_unix) {
            if let Ok(path) = engine.run(now_unix) {
                self.last_hourly = Some(now_unix);
            }
        }
    }
}
```

---

## 4. Incremental Backup Engine

```rust
// supervisor/src/snapshot/engine.rs  (~200 lines)

pub struct SnapshotEngine { registry: AppRegistry }

impl SnapshotEngine {
    pub fn run(&self, now_unix: u64) -> Result<String, String> {
        let snap_dir = format!("/backup/snapshots/{}", unix_to_iso8601(now_unix));
        let prev_dir = self.latest_snapshot_dir();
        let journal  = Journal::load("/data/.tm/journal.toml");  // B1 fix
        let excl     = load_exclusions();
        fs::create_dir_all(&snap_dir)?;
        copy_or_link("/data", &snap_dir, prev_dir.as_deref(), &journal, &excl)?;
        journal.save_atomic("/data/.tm/journal.toml")?;  // B1: R41 write_commit
        Ok(snap_dir)
    }
}

fn copy_or_link(src: &str, dst: &str, prev: Option<&str>,
                journal: &Journal, excl: &Exclusions) -> Result<(), String> {
    for entry in fs::read_dir(src)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if excl.is_excluded(&format!("{src}/{name}")) { continue; }
        let meta = entry.metadata()?;
        if meta.is_dir() {
            fs::create_dir_all(&format!("{dst}/{name}"))?;
            copy_or_link(&format!("{src}/{name}"), &format!("{dst}/{name}"),
                prev.map(|p| format!("{p}/{name}")).as_deref(), journal, excl)?;
        } else if meta.is_file() {
            let changed = journal.is_changed(&format!("{src}/{name}"), &meta);  // B1
            if changed || prev.is_none() {
                fs::copy(&format!("{src}/{name}"), &format!("{dst}/{name}"))?;
            } else {
                fs::hard_link(&format!("{}/{name}", prev.unwrap()),
                              &format!("{dst}/{name}"))?;  // within /backup only
            }
        }
    }
    Ok(())
}
```

Note: all hardlinks are within `/backup/snapshots/` (same ext4 volume). The `/data` side is never hardlinked — only used as the source for fresh copies.

---

## 5. Quiesce Protocol (B2 Fix)

Full restore quiesces running apps before bulk-copy. `AppState` gains `pub quiesced: bool`.

Watchdog (B2 fix): checks `st.quiesced` and skips enforcement for quiesced apps.

Restore dispatch runs in dedicated `snapshot-restore` thread (NOT the IPC handler thread — B4 fix). Global `RESTORE_ACK_TX: OnceLock<Mutex<Option<mpsc::Sender<String>>>>` collects acks:

```rust
// supervisor/src/snapshot/restore.rs  (~160 lines)
// IPC handler sees "fs-quiesce-ack" → forwards to RESTORE_ACK_TX channel
// Restore thread: set quiesced=true, broadcast VYOMA_SYSTEM:quiesce, wait for acks,
//                 then bulk-copy, then SIGCONT, reset quiesced=false, reset last_output
```

**Protocol**:
```
VYOMA_SYSTEM:quiesce        (supervisor → apps with filesystem=true)
@supervisor: fs-quiesce-ack  (app → supervisor, within 5s)
VYOMA_SYSTEM:restore-done    (supervisor → all filesystem apps, after bulk-copy)
```

Full restore: SIGSTOP all wasmtime children after acks → bulk copy → SIGCONT → send `restore-done` → reset `last_output`.

---

## 6. Browse & Restore UI

`apps/time-machine/` WASM app. Capability: `snapshot = true` (gated — only this app may issue `snap-*` commands).

**Protocol** (app → supervisor via `@supervisor: snap-*`):
```
@supervisor: snap-list
@supervisor: snap-info <iso8601>
@supervisor: snap-restore-file <iso8601> <rel_path>
@supervisor: snap-restore-all <iso8601>
@supervisor: snap-delete <iso8601>
@supervisor: snap-exclude-add <path>
@supervisor: snap-verify <iso8601>
@supervisor: snap-cancel <job_id>
```

Reply (supervisor → app stdin):
```
VYOMA_SNAP:list:<json_b64>
VYOMA_SNAP:restore-progress:<pct>
VYOMA_SNAP:restore-done:<path>
VYOMA_SNAP:disk-full
VYOMA_SNAP:verify-result:<iso8601>:<ok|fail>:<details_b64>
```

Single-file restore uses R41 atomic-rename pattern:
```rust
pub fn restore_file(snap_iso: &str, rel_path: &str) -> Result<(), String> {
    let src = format!("/backup/snapshots/{snap_iso}/data/{rel_path}");
    let dst = format!("/data/{rel_path}");
    let tmp = format!("{dst}.tm_restore");
    fs::copy(&src, &tmp)?;
    fsync_path(&tmp)?;
    fs::rename(&tmp, &dst)?;  // atomic
    Ok(())
}
```

---

## 7. Exclusions (B5 Fix)

`[backup]` is a **top-level table** in `AppManifest` — NOT inside `[capabilities]` (avoids `deny_unknown_fields` conflict on `Capabilities`):

```rust
// manifest.rs
#[derive(Debug, Default, Deserialize, Clone)]
pub struct BackupConfig {
    #[serde(default)]
    pub exclude: Vec<String>,
}

pub struct AppManifest {
    // ... existing fields ...
    #[serde(default)]
    pub backup: BackupConfig,   // NEW — does not touch Capabilities deny_unknown_fields
}
```

Exclusions stored at `/data/.tm/exclusions.toml`:
```toml
[[exclude]]
path   = "/data/logs"
source = "system"

[[exclude]]
path   = "/data/apps/my-app/cache"
source = "app:my-app"
```

Validation: `backup.exclude` paths must not contain `..` components.

---

## 8. Security

WASM apps cannot access `/backup` — it is not preopened in any app's WASI capabilities. All backup data access flows through `@supervisor: snap-*` IPC, checked against `snapshot = true` capability. Backup volume mounted `MS_NODEV|MS_NOEXEC|MS_NOSUID`.

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: 9P mtime unreliable for change detection | Journal at `/data/.tm/journal.toml` with (size, sha256_first_4KB); hardlinks only within /backup/snapshots (same ext4 volume) |
| B2: SIGSTOP races with watchdog, kills apps mid-restore | `AppState.quiesced` flag; watchdog skips quiesced apps; SIGCONT + `last_output` reset after restore |
| B3: Prune fails → /backup fills → snapshot loop silently stops | Prune-before-snapshot; free space check (statvfs, skip if <10%); `prune_failed` retry flag; `VYOMA_SNAP:disk-full` event |
| B4: snap-restore-all blocks IPC handler thread (can't receive acks) | Restore runs in dedicated thread; `RESTORE_ACK_TX` global forwards acks from IPC handler |
| B5: `backup_exclude` in Capabilities breaks `deny_unknown_fields` | `[backup]` top-level table in `AppManifest`, not in `Capabilities`; path traversal validation |

---

## 10. File Layout

```
supervisor/src/snapshot/
├── mod.rs          (~40 lines)
├── engine.rs       (~200 lines: copy_or_link, journal-based change detection (B1))
├── scheduler.rs    (~120 lines: prune-first (B3), free-space check, hourly/daily/weekly)
├── restore.rs      (~160 lines: restore_file (atomic), restore_all (B2 quiesce, B4 thread))
├── exclusions.rs   (~100 lines: Exclusions, load/save, app exclusion registration)
└── dest.rs         (~80 lines: try_mount_backup)

supervisor/src/ipc_commands/snapshot.rs  (~180 lines: snap-* handlers, snapshot capability check)
apps/time-machine/src/
├── main.rs         (~200 lines)
├── timeline.rs     (~250 lines)
├── restore.rs      (~200 lines)
└── browse.rs       (~200 lines)
```
