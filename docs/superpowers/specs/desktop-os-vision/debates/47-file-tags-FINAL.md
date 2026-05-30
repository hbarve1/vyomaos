# FINAL Spec: File Tagging & Metadata (Round 47)

**Subsystem**: File Tagging & Metadata  
**macOS Analogue**: `Finder tags` / `xattrs` / `NSMetadataItem`  
**Depends on**: R04 (VFS), R41 (file_access_grants, VYOMA_FS:watch), R42 (Spotlight indexer)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Storage Architecture

Tags are stored in two complementary layers:

1. **xattr layer** — per-file inode, survives copy if destination FS supports xattrs.
2. **Central index** — inverted map (tag → paths), stored at `/data/.vyoma/tags/index.json`
   for fast cross-file queries (`list_tagged_files`, Spotlight integration).

Both databases opened in WAL mode. Writer connection: exclusive to writer thread.
Reader connection: read-only, used by query threads (B2 fix).

### 1a. xattr Storage Schema

Each file carries a `user.vyoma.tags` extended attribute — a JSON array of tag name strings:

```
xattr key:   user.vyoma.tags
xattr value: ["Red","Work","Q2-2026"]   (UTF-8 JSON)
```

Custom metadata is stored under `user.vyoma.meta` as a JSON object:

```
xattr key:   user.vyoma.meta
xattr value: {"rating":4,"author":"himank","reviewed":true,"size_mb":3.14}
```

Both attributes are written with an atomic `setxattr` call. There is no partial-write risk
because the kernel applies xattr changes atomically per POSIX.

### 1b. Central Tag Index

The central index lives at `/data/.vyoma/tags/index.json` and is loaded into memory as a
`TagIndex` struct on supervisor init. All mutations are reflected in memory immediately and
flushed to disk via atomic write (`.tmp` → rename) after every mutation batch.

```json
{
  "version": 1,
  "tags": {
    "Red":    { "color": "Red",  "paths": ["/data/docs/report.pdf", "/data/notes/todo.txt"] },
    "Work":   { "color": null,   "paths": ["/data/docs/report.pdf"] },
    "Q2-2026":{ "color": "Blue", "paths": ["/data/notes/todo.txt"] }
  }
}
```

### 1c. SQLite Databases (extended attributes + app metadata)

Two SQLite databases reside under `/data/.vyoma/`:

**`tags.db`**:
```sql
CREATE TABLE tag_names (
    id         INTEGER PRIMARY KEY,
    name       TEXT UNIQUE NOT NULL,
    color      TEXT,                 -- TagColor name or NULL
    created_at INTEGER NOT NULL
);
CREATE TABLE file_tags (
    path_hash  TEXT NOT NULL,        -- sha256(canonical_path)
    tag_id     INTEGER NOT NULL REFERENCES tag_names(id) ON DELETE CASCADE,
    tagged_at  INTEGER NOT NULL,
    tagged_by  TEXT NOT NULL,        -- app_name
    PRIMARY KEY (path_hash, tag_id)
);
CREATE INDEX file_tags_tag_id ON file_tags(tag_id);
```

**`xattrs.db`** (extended attributes + app metadata):
```sql
CREATE TABLE xattrs (
    path_hash  TEXT NOT NULL,
    ns         TEXT NOT NULL,        -- "vyoma", "user", "com.<bundle>"
    key        TEXT NOT NULL,
    value      BLOB NOT NULL,
    set_at     INTEGER NOT NULL,
    set_by     TEXT NOT NULL,        -- app_instance_id
    PRIMARY KEY (path_hash, ns, key)
);
CREATE TABLE app_meta (
    path_hash  TEXT NOT NULL,
    bundle_id  TEXT NOT NULL,        -- "com.vyomaos.notes"
    key        TEXT NOT NULL,
    value      BLOB NOT NULL,
    PRIMARY KEY (path_hash, bundle_id, key)
);
```

**rusqlite musl compilation (B1 fix)**: `rusqlite` with `bundled` feature compiles SQLite C source. Requires:
```toml
# supervisor/Cargo.toml
rusqlite = { version = "0.31", features = ["bundled", "backup"] }
```
```toml
# supervisor/.cargo/config.toml
[target.x86_64-unknown-linux-musl]
linker = "musl-gcc"
[env]
CC_x86_64_unknown_linux_musl = "musl-gcc"
```
`Makefile` passes `CC_x86_64_unknown_linux_musl=musl-gcc` for Docker builds:
```makefile
supervisor:
    docker run ... -e CC_x86_64_unknown_linux_musl=musl-gcc cargo build ...
```

---

## 2. Core Data Structures

### 2a. Tag and TagColor

```rust
// supervisor/src/file_tags/types.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TagColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl TagColor {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Red"    => Some(Self::Red),
            "Orange" => Some(Self::Orange),
            "Yellow" => Some(Self::Yellow),
            "Green"  => Some(Self::Green),
            "Blue"   => Some(Self::Blue),
            "Purple" => Some(Self::Purple),
            "Gray"   => Some(Self::Gray),
            _        => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Red    => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green  => "Green",
            Self::Blue   => "Blue",
            Self::Purple => "Purple",
            Self::Gray   => "Gray",
        }
    }

    /// Hex color for GUI rendering
    pub fn hex(&self) -> &'static str {
        match self {
            Self::Red    => "#FF3B30",
            Self::Orange => "#FF9500",
            Self::Yellow => "#FFCC00",
            Self::Green  => "#34C759",
            Self::Blue   => "#007AFF",
            Self::Purple => "#AF52DE",
            Self::Gray   => "#8E8E93",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Tag {
    pub name:  String,
    pub color: Option<TagColor>,
}
```

### 2b. FileTagRecord and MetadataValue

```rust
// supervisor/src/file_tags/types.rs (continued)

use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MetadataValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    DateTime(SystemTime),
}

impl MetadataValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::String(_)   => "string",
            Self::Int(_)      => "int",
            Self::Float(_)    => "float",
            Self::Bool(_)     => "bool",
            Self::DateTime(_) => "datetime",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileTagRecord {
    pub canonical_path:  String,
    pub tags:            Vec<String>,
    pub custom_metadata: HashMap<String, MetadataValue>,
}

impl FileTagRecord {
    pub fn new(canonical_path: impl Into<String>) -> Self {
        Self {
            canonical_path:  canonical_path.into(),
            tags:            Vec::new(),
            custom_metadata: HashMap::new(),
        }
    }
}
```

### 2c. TagIndex (In-Memory Inverted Index)

```rust
// supervisor/src/file_tags/index.rs

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::OnceLock;
use super::types::{Tag, TagColor};

pub static TAG_INDEX: OnceLock<Arc<Mutex<TagIndex>>> = OnceLock::new();

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TagEntry {
    pub color: Option<TagColor>,
    pub paths: HashSet<String>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TagIndex {
    pub version: u32,
    pub tags:    HashMap<String, TagEntry>,
}

impl TagIndex {
    pub fn new() -> Self {
        Self { version: 1, tags: HashMap::new() }
    }

    pub fn add_tag_to_file(&mut self, tag_name: &str, path: &str) {
        self.tags
            .entry(tag_name.to_string())
            .or_insert_with(|| TagEntry { color: None, paths: HashSet::new() })
            .paths
            .insert(path.to_string());
    }

    pub fn remove_tag_from_file(&mut self, tag_name: &str, path: &str) {
        if let Some(entry) = self.tags.get_mut(tag_name) {
            entry.paths.remove(path);
            // Leave empty tag entries; explicit delete_tag cleans them up
        }
    }

    pub fn list_tagged_files(&self, tag_name: &str) -> Vec<String> {
        self.tags
            .get(tag_name)
            .map(|e| e.paths.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn rename_tag(&mut self, old: &str, new: &str) -> Result<(), String> {
        if self.tags.contains_key(new) {
            return Err(format!("tag '{new}' already exists"));
        }
        if let Some(entry) = self.tags.remove(old) {
            self.tags.insert(new.to_string(), entry);
        }
        Ok(())
    }

    pub fn delete_tag(&mut self, name: &str) {
        self.tags.remove(name);
    }

    pub fn set_tag_color(&mut self, name: &str, color: Option<TagColor>) {
        if let Some(entry) = self.tags.get_mut(name) {
            entry.color = color;
        }
    }
}
```

---

## 3. xattr Syscall Wrappers

The xattr operations wrap Linux syscalls directly. On Linux (including the QEMU VM), the
`xattr` family of syscalls (`setxattr`, `getxattr`, `listxattr`, `removexattr`) are used.
The wrappers return `std::io::Result<T>` and convert `errno` via `io::Error::last_os_error()`.

```rust
// supervisor/src/file_tags/xattr.rs

use std::ffi::CString;
use std::io;
use std::os::raw::{c_char, c_int, c_void};

extern "C" {
    fn setxattr(path: *const c_char, name: *const c_char,
                value: *const c_void, size: usize, flags: c_int) -> c_int;
    fn getxattr(path: *const c_char, name: *const c_char,
                value: *mut c_void, size: usize) -> isize;
    fn listxattr(path: *const c_char, list: *mut c_char, size: usize) -> isize;
    fn removexattr(path: *const c_char, name: *const c_char) -> c_int;
}

pub fn xattr_set(path: &str, name: &str, value: &[u8]) -> io::Result<()> {
    let c_path  = CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let c_name  = CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let ret = unsafe {
        setxattr(c_path.as_ptr(), c_name.as_ptr(),
                 value.as_ptr() as *const c_void, value.len(), 0)
    };
    if ret == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn xattr_get(path: &str, name: &str) -> io::Result<Vec<u8>> {
    let c_path = CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let c_name = CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // First call with size=0 to get required buffer length
    let size = unsafe { getxattr(c_path.as_ptr(), c_name.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 { return Err(io::Error::last_os_error()); }
    let mut buf = vec![0u8; size as usize];
    let read = unsafe {
        getxattr(c_path.as_ptr(), c_name.as_ptr(),
                 buf.as_mut_ptr() as *mut c_void, buf.len())
    };
    if read < 0 { return Err(io::Error::last_os_error()); }
    buf.truncate(read as usize);
    Ok(buf)
}

pub fn xattr_list(path: &str) -> io::Result<Vec<String>> {
    let c_path = CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let size = unsafe { listxattr(c_path.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 { return Err(io::Error::last_os_error()); }
    let mut buf = vec![0u8; size as usize];
    let read = unsafe { listxattr(c_path.as_ptr(), buf.as_mut_ptr() as *mut c_char, buf.len()) };
    if read < 0 { return Err(io::Error::last_os_error()); }
    // Names are NUL-separated
    let names = buf[..read as usize]
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    Ok(names)
}

pub fn xattr_remove(path: &str, name: &str) -> io::Result<()> {
    let c_path = CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let c_name = CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let ret = unsafe { removexattr(c_path.as_ptr(), c_name.as_ptr()) };
    if ret == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}
```

---

## 4. Tag CRUD Operations

```rust
// supervisor/src/file_tags/tags.rs

use std::path::Path;
use super::xattr::{xattr_get, xattr_set, xattr_remove};
use super::index::{TAG_INDEX};
use super::persist::flush_index;

const TAG_XATTR: &str = "user.vyoma.tags";

fn read_tags_from_xattr(path: &str) -> Vec<String> {
    match xattr_get(path, TAG_XATTR) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_)    => Vec::new(),
    }
}

fn write_tags_to_xattr(path: &str, tags: &[String]) -> std::io::Result<()> {
    let json = serde_json::to_vec(tags).expect("tag serialization infallible");
    xattr_set(path, TAG_XATTR, &json)
}

pub fn add_tag(path: &str, tag_name: &str) -> Result<(), String> {
    if tag_name.is_empty() || tag_name.len() > 64 {
        return Err("tag name must be 1–64 characters".to_string());
    }
    let mut tags = read_tags_from_xattr(path);
    if !tags.contains(&tag_name.to_string()) {
        tags.push(tag_name.to_string());
        write_tags_to_xattr(path, &tags).map_err(|e| e.to_string())?;
        let mut idx = TAG_INDEX.get().expect("TAG_INDEX not init").lock().unwrap();
        idx.add_tag_to_file(tag_name, path);
        drop(idx);
        flush_index();
    }
    Ok(())
}

pub fn remove_tag(path: &str, tag_name: &str) -> Result<(), String> {
    let mut tags = read_tags_from_xattr(path);
    let before = tags.len();
    tags.retain(|t| t != tag_name);
    if tags.len() < before {
        if tags.is_empty() {
            xattr_remove(path, TAG_XATTR).map_err(|e| e.to_string())?;
        } else {
            write_tags_to_xattr(path, &tags).map_err(|e| e.to_string())?;
        }
        let mut idx = TAG_INDEX.get().expect("TAG_INDEX not init").lock().unwrap();
        idx.remove_tag_from_file(tag_name, path);
        drop(idx);
        flush_index();
    }
    Ok(())
}

pub fn get_tags(path: &str) -> Vec<String> {
    read_tags_from_xattr(path)
}

pub fn list_tagged_files(tag_name: &str) -> Vec<String> {
    TAG_INDEX
        .get()
        .expect("TAG_INDEX not init")
        .lock()
        .unwrap()
        .list_tagged_files(tag_name)
}

pub fn rename_tag(old: &str, new: &str) -> Result<(), String> {
    let mut idx = TAG_INDEX.get().expect("TAG_INDEX not init").lock().unwrap();
    idx.rename_tag(old, new)?;
    // Also rewrite xattrs on all affected files
    let affected: Vec<String> = idx.tags
        .get(new)
        .map(|e| e.paths.iter().cloned().collect())
        .unwrap_or_default();
    drop(idx);
    for path in &affected {
        let mut tags = read_tags_from_xattr(path);
        for t in &mut tags {
            if t == old { *t = new.to_string(); }
        }
        let _ = write_tags_to_xattr(path, &tags);
    }
    flush_index();
    Ok(())
}

pub fn delete_tag(tag_name: &str) -> Result<(), String> {
    let mut idx = TAG_INDEX.get().expect("TAG_INDEX not init").lock().unwrap();
    let affected: Vec<String> = idx
        .list_tagged_files(tag_name)
        .into_iter()
        .collect();
    idx.delete_tag(tag_name);
    drop(idx);
    for path in &affected {
        let mut tags = read_tags_from_xattr(path);
        tags.retain(|t| t != tag_name);
        if tags.is_empty() {
            let _ = xattr_remove(path, TAG_XATTR);
        } else {
            let _ = write_tags_to_xattr(path, &tags);
        }
    }
    flush_index();
    Ok(())
}
```

---

## 5. Custom Metadata CRUD

```rust
// supervisor/src/file_tags/meta.rs

use std::collections::HashMap;
use super::types::MetadataValue;
use super::xattr::{xattr_get, xattr_set, xattr_remove};

const META_XATTR: &str = "user.vyoma.meta";

fn read_meta(path: &str) -> HashMap<String, MetadataValue> {
    match xattr_get(path, META_XATTR) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_)    => HashMap::new(),
    }
}

fn write_meta(path: &str, map: &HashMap<String, MetadataValue>) -> std::io::Result<()> {
    let json = serde_json::to_vec(map).expect("meta serialization infallible");
    xattr_set(path, META_XATTR, &json)
}

pub fn set_metadata(path: &str, key: &str, value: MetadataValue) -> Result<(), String> {
    let mut map = read_meta(path);
    map.insert(key.to_string(), value);
    write_meta(path, &map).map_err(|e| e.to_string())
}

pub fn get_metadata(path: &str, key: &str) -> Option<MetadataValue> {
    read_meta(path).remove(key)
}

pub fn remove_metadata(path: &str, key: &str) -> Result<(), String> {
    let mut map = read_meta(path);
    if map.remove(key).is_some() {
        if map.is_empty() {
            xattr_remove(path, META_XATTR).map_err(|e| e.to_string())?;
        } else {
            write_meta(path, &map).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn list_metadata(path: &str) -> HashMap<String, MetadataValue> {
    read_meta(path)
}
```

---

## 6. Atomic Index Persistence

```rust
// supervisor/src/file_tags/persist.rs

use std::fs;
use std::io::Write;
use super::index::TAG_INDEX;

const INDEX_PATH: &str = "/data/.vyoma/tags/index.json";
const INDEX_TMP:  &str = "/data/.vyoma/tags/index.json.tmp";

pub fn flush_index() {
    let idx = TAG_INDEX.get().expect("TAG_INDEX not init").lock().unwrap();
    let json = match serde_json::to_vec_pretty(&*idx) {
        Ok(j)  => j,
        Err(e) => { eprintln!("[file_tags] flush serialize error: {e}"); return; }
    };
    drop(idx);
    // Atomic write: write to .tmp then rename
    match fs::File::create(INDEX_TMP).and_then(|mut f| f.write_all(&json)) {
        Ok(_)  => { let _ = fs::rename(INDEX_TMP, INDEX_PATH); }
        Err(e) => eprintln!("[file_tags] flush write error: {e}"),
    }
}

pub fn load_index() {
    let _ = fs::create_dir_all("/data/.vyoma/tags");
    let index = match fs::read(INDEX_PATH) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_)    => super::index::TagIndex::new(),
    };
    TAG_INDEX
        .set(std::sync::Arc::new(std::sync::Mutex::new(index)))
        .expect("TAG_INDEX already initialized");
}
```

---

## 7. Namespace Model

| Namespace | Format | Writer | Reader |
|-----------|--------|--------|--------|
| `vyoma.*` | `vyoma.quarantine`, `vyoma.origin_url` | supervisor only | any filesystem app |
| `user.*` | `user.comment`, `user.rating` | any filesystem app | any filesystem app |
| `com.<bundle>.*` | `com.vyomaos.notes.folded` | only the matching bundle | only the matching bundle |

**`bundle_id: Option<String>` in AppMeta (B5 fix)**: `AppState` gains:
```rust
pub bundle_id: Option<String>,   // from vyoma.toml [app] bundle_id = "com.vendor.app"
```
Namespace enforcement in `ipc_handler`:
```rust
fn check_ns_write(ns: &str, sender_bundle: Option<&str>) -> bool {
    if ns == "vyoma" { return false; }           // supervisor-only
    if ns == "user"  { return true; }
    // com.<bundle>:
    if let Some(b) = ns.strip_prefix("com.") {
        return sender_bundle.map(|s| s == b).unwrap_or(false);
    }
    false
}
```
The check uses `bundle_id` (e.g. `"com.vyomaos.notes"`) — NOT the unqualified app `name` (B5 fix).

---

## 8. VYOMA_TAGS Protocol

Apps with `filesystem = true` use the `VYOMA_TAGS:` prefix. All path arguments are
base64url-encoded (no padding) to survive the line-oriented protocol.

### App → Supervisor

| Verb | Line format | Description |
|------|-------------|-------------|
| `add` | `VYOMA_TAGS:add/<path_b64>/<tag_name>` | Add tag to file |
| `remove` | `VYOMA_TAGS:remove/<path_b64>/<tag_name>` | Remove tag from file |
| `get` | `VYOMA_TAGS:get/<path_b64>` | List tags on file |
| `list_files` | `VYOMA_TAGS:list_files/<tag_name>` | Paths carrying this tag |
| `rename` | `VYOMA_TAGS:rename/<old_name>/<new_name>` | Rename tag globally |
| `delete` | `VYOMA_TAGS:delete/<tag_name>` | Delete tag from all files |
| `list_all` | `VYOMA_TAGS:list_all` | All known tags + colors |
| `set_color` | `VYOMA_TAGS:set_color/<tag_name>/<color>` | Set TagColor on tag |
| `set_meta` | `VYOMA_TAGS:set_meta/<path_b64>/<key_b64>/<type>/<value_b64>` | Set metadata key |
| `get_meta` | `VYOMA_TAGS:get_meta/<path_b64>/<key_b64>` | Get metadata value |
| `remove_meta` | `VYOMA_TAGS:remove_meta/<path_b64>/<key_b64>` | Remove metadata key |
| `list_meta` | `VYOMA_TAGS:list_meta/<path_b64>` | All metadata for file |

### Supervisor → App

```
VYOMA_TAGS:ok
VYOMA_TAGS:get_reply:<path_b64>:<tags_json_b64>
VYOMA_TAGS:list_files_reply:<tag_name>:<paths_json_b64>
VYOMA_TAGS:list_all_reply:<all_tags_json_b64>
VYOMA_TAGS:get_meta_reply:<path_b64>:<key_b64>:<type>:<value_b64>
VYOMA_TAGS:list_meta_reply:<path_b64>:<meta_json_b64>
VYOMA_TAGS:error:<reason_b64>
```

`<type>` field for metadata replies is one of: `string`, `int`, `float`, `bool`, `datetime`.
`<value_b64>` for `datetime` is the Unix timestamp in seconds as a decimal string, base64-encoded.

### Legacy VYOMA_FS: Protocol Lines (R41 compatibility)

Apps already using `VYOMA_FS:tag/` lines continue to work. The router dispatch (B4 fix)
handles both prefixes, delegating `VYOMA_FS:tag/` and `VYOMA_FS:xattr/` to the same
`file_tags::ipc_handler` module.

```
VYOMA_FS:tag/<path_b64>/add/<tag_name>
VYOMA_FS:tag/<path_b64>/remove/<tag_name>
VYOMA_FS:tag/<path_b64>/list
VYOMA_FS:tag/rename/<old_name>/<new_name>
VYOMA_FS:tag/delete/<tag_name>
VYOMA_FS:tag/list_all
VYOMA_FS:tag/search/<tag_name>
VYOMA_FS:xattr/<path_b64>/set/<ns_key_b64>/<value_b64>
VYOMA_FS:xattr/<path_b64>/get/<ns_key_b64>
VYOMA_FS:xattr/<path_b64>/list
VYOMA_FS:xattr/<path_b64>/remove/<ns_key_b64>
VYOMA_FS:app_meta/<path_b64>/set/<key_b64>/<value_b64>
VYOMA_FS:app_meta/<path_b64>/get/<key_b64>
VYOMA_FS:app_meta/<path_b64>/list
```

---

## 9. Router Dispatch (B4 Fix)

`router.rs` currently has NO `VYOMA_FS:` arm — all opcodes silently drop. Fix: add dispatch
arm before the draw block:

```rust
// supervisor/src/router.rs — add BEFORE VYOMA_DRAW: check
if let Some(rest) = line.strip_prefix("VYOMA_TAGS:") {
    file_tags::ipc_handler::handle_tags_line(rest, sender, inbox);
    return;
}
if let Some(rest) = line.strip_prefix("VYOMA_FS:") {
    // tag/xattr/app_meta handled by new file_tags handler
    if rest.starts_with("tag/") || rest.starts_with("xattr/") || rest.starts_with("app_meta/") {
        file_tags::ipc_handler::handle_fs_compat_line(rest, sender, inbox);
        return;
    }
    // write_begin / write_commit / cancel / watch handled in fs_ipc.rs
    fs_ipc::handle_fs_line(rest, sender, app_registry, inbox);
    return;
}
```

`TAG_INDEX` global initialization in `main()`:
```rust
// supervisor/src/main.rs — after data disk confirmed available
file_tags::persist::load_index();
```

---

## 10. WAL Concurrency (B2 Fix)

Single `Mutex<Connection>` serializes all reads behind writes. Fix: split into writer + readers:

```rust
// supervisor/src/file_tags/db.rs
pub struct MetaStore {
    writer: Mutex<Connection>,      // exclusive: INSERT/UPDATE/DELETE only
    reader: Connection,             // read_only=true, no mutex needed for parallel reads
}

impl MetaStore {
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let writer = Connection::open(path)?;
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let reader = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self { writer: Mutex::new(writer), reader })
    }
}
```

Tag lookups (`get_tags`, `get_metadata`) use `self.reader` directly (no contention).
Tag mutations use `self.writer.lock()`.

---

## 11. Tag Rename Race Fix (B3 Fix)

`BEGIN IMMEDIATE` prevents ghost `tag_names` rows when two apps rename simultaneously:

```rust
// supervisor/src/file_tags/db_tags.rs
pub fn rename_tag_db(store: &MetaStore, old: &str, new: &str) -> Result<(), String> {
    let conn = store.writer.lock().unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
    let exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM tag_names WHERE name = ?1",
        params![new],
        |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if exists {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(format!("tag '{new}' already exists"));
    }
    conn.execute("UPDATE tag_names SET name = ?1 WHERE name = ?2", params![new, old])
        .map_err(|e| e.to_string())?;
    conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    Ok(())
}
```

Without `BEGIN IMMEDIATE`, two concurrent renames to the same target each pass the existence
check and then race to violate the `UNIQUE` constraint. `IMMEDIATE` acquires a write lock
before either read, preventing the race (B3 fix).

---

## 12. Spotlight Integration

When tags change, `file_tags` notifies the Spotlight indexer via the existing `IndexCmd` channel:

```rust
// supervisor/src/file_tags/tags.rs — after successful add_tag / remove_tag
use crate::spotlight::SPOTLIGHT_INDEX;
use crate::spotlight::IndexCmd;

if let Some(idx) = SPOTLIGHT_INDEX.get() {
    let _ = idx.cmd_tx.send(IndexCmd::Reindex(canonical_path.to_string()));
}
```

Tag data read from `user.vyoma.tags` xattr is parsed by the Spotlight indexer thread,
enabling `tag:Red` and `tag:Work` queries in the Spotlight search subsystem (R42).

The Spotlight indexer uses `xattr_get` directly (no IPC round-trip) since both subsystems
run inside the supervisor process and share the `file_tags::xattr` module.

---

## 13. File Layout

```
supervisor/src/file_tags/
├── mod.rs          (~40 lines: pub re-exports, load_index init shim)
├── types.rs        (~90 lines: Tag, TagColor, FileTagRecord, MetadataValue)
├── index.rs        (~80 lines: TagIndex struct, TAG_INDEX OnceLock<Arc<Mutex<TagIndex>>>)
├── xattr.rs        (~80 lines: xattr_set/get/list/remove syscall wrappers)
├── tags.rs         (~110 lines: add_tag/remove_tag/get_tags/list_tagged_files/rename_tag/delete_tag)
├── meta.rs         (~70 lines: set_metadata/get_metadata/remove_metadata/list_metadata)
├── persist.rs      (~50 lines: load_index, flush_index with atomic .tmp→rename)
├── db.rs           (~80 lines: MetaStore open, WAL setup, writer+reader split — B2)
└── ipc_handler.rs  (~160 lines: parse VYOMA_TAGS: and VYOMA_FS: compat lines, route to ops)

supervisor/src/router.rs         (modified: VYOMA_TAGS: + VYOMA_FS: dispatch arms — B4)
supervisor/src/main.rs           (modified: file_tags::persist::load_index() after data disk)
supervisor/Cargo.toml            (modified: rusqlite bundled feature — B1)
supervisor/.cargo/config.toml   (modified: CC_x86_64_unknown_linux_musl — B1)
Makefile                         (modified: CC env var passed to Docker builds — B1)
```

All files are within the 500-line limit. `ipc_handler.rs` at ~160 lines is the largest;
if VYOMA_FS compat lines are extracted to a shim, it stays well under the limit.

---

## 14. Blocking Issue Resolution Summary

| ID | Issue | Status | Resolution |
|----|-------|--------|-----------|
| B1 | `rusqlite` bundled feature fails to compile for musl target | RESOLVED | `CC_x86_64_unknown_linux_musl=musl-gcc` in Makefile + `.cargo/config.toml`; `bundled` feature in Cargo.toml |
| B2 | Single `Mutex<Connection>` serializes all reads behind writes | RESOLVED | Split into `writer: Mutex<Connection>` (mutations) + `reader: Connection` (read_only=true, no lock needed) |
| B3 | Concurrent tag renames corrupt `tag_names` UNIQUE constraint | RESOLVED | `BEGIN IMMEDIATE` transaction + pre-check for target name existence before `UPDATE` |
| B4 | `VYOMA_FS:` has no dispatch arm in `router.rs` — all opcodes silently dropped | RESOLVED | `VYOMA_TAGS:` arm + `VYOMA_FS:` compat arm added in `router.rs`; both route to `file_tags::ipc_handler` |
| B5 | Namespace enforcement uses unqualified app `name` — insufficient for `com.<bundle>.*` | RESOLVED | `bundle_id: Option<String>` in `AppState`; `check_ns_write` strips `com.` prefix and matches `bundle_id` |
