# FINAL Spec: Code Signing & Notarization (Round 62)

**Subsystem**: Code Signing & Notarization  
**macOS Analogue**: `codesign` / Gatekeeper / Notarization  
**Depends on**: R50 (package manager, `.vyomapkg`, Ed25519), R58 (TLS/cert store), R59 (capability model, TrustLevel), R60 (Keychain)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Signing happens at package creation time (host-side `tools/vyoma-sign`). Verification happens at install time (R50 package manager) and at spawn time (supervisor). The entire chain is offline-capable and integrates with the existing `sha2`-based integrity checks.

```
supervisor/src/codesign/
├── mod.rs          (~120 lines: TrustLevel, SigSidecar, OFFICIAL_KEYS, public API)
├── verifier.rs     (~180 lines: code_dir_hash_v2, verify_app, verify_and_seal)
├── revocation.rs   (~80 lines: is_revoked from /data/.vyoma/revoked_keys.txt)
├── ipc.rs          (~140 lines: codesign-verify/trust-add/revoke handlers)
└── gatekeeper.rs   (~80 lines: load/check policy from /data/.vyoma/gatekeeper.toml)

tools/vyoma-sign/   — host-side signing tool (not in supervisor binary)
```

---

## 2. Signing Format

The code directory — a deterministic hash-of-hashes — is what gets signed. It covers the WASM binary and a **canonical subset** of the manifest (B1 fix), not the raw manifest bytes.

```
code_dir_hash_v2 = SHA256(
    "VYOMA_CODE_DIR_V2\x00"    -- domain separator
    || SHA256(wasm_bytes)
    || SHA256(canonical_manifest_json)   -- [app] + [capabilities] only, JSON sorted keys
    || name_bytes || 0x00
)
```

The signature lives in a **sidecar file** `<app_logical_name>.sig` (99 bytes) inside the `.vyomapkg` archive alongside `<name>.wasm` and `vyoma.toml`. Package layout:

```
my-app-1.0.0.vyomapkg (tar+zst)
├── my-app.wasm          ← binary
├── vyoma.toml           ← manifest
└── my-app.sig           ← 99-byte Ed25519 sidecar
```

---

## 3. Core Types

```rust
// supervisor/src/codesign/mod.rs

/// Trust classification for a WASM app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    Official,    // signed by baked-in official VyomaOS key
    Developer,   // signed by a key in /data/.vyoma/trusted_keys/
    Unsigned,    // no .sig sidecar present
}

/// Compact 99-byte sidecar: magic(2) + version(1) + pubkey(32) + sig(64)
pub struct SigSidecar {
    pub version:   u8,          // must be 1
    pub pubkey:    [u8; 32],    // Ed25519 raw public key
    pub signature: [u8; 64],    // Ed25519 sig over code_dir_hash_v2
}
pub const SIG_MAGIC: [u8; 2] = [0x56, 0x53];  // "VS"
pub const SIG_SIZE:  usize   = 99;             // 2 + 1 + 32 + 64

/// Official VyomaOS signing keys — baked in, never removable.
/// Add new keys on rotation; use revocation.rs to retire old ones.
pub const OFFICIAL_KEYS: &[&[u8; 32]] = &[
    &[
        0x9f, 0x3a, 0x11, 0xc4, 0x87, 0xde, 0x56, 0x2b,
        0x4e, 0x71, 0xa0, 0x33, 0xf8, 0x9c, 0xbd, 0x22,
        0x6d, 0x48, 0xe5, 0x01, 0x77, 0xca, 0x3f, 0x9a,
        0x12, 0x80, 0xb5, 0x44, 0x6e, 0x22, 0xd1, 0x7f,
    ],
];
```

---

## 4. Canonical Manifest Subset for Signing (B1 Fix)

Only the `[app]` and `[capabilities]` sections are included in the code directory hash. The `[window]` section is excluded because the supervisor can mutate it at runtime (layout persistence), which would invalidate the signature.

```rust
// supervisor/src/codesign/verifier.rs

/// Build signing-stable bytes from raw manifest TOML.
/// Excludes [window], [menu_items], and any future runtime-mutable sections.
pub fn manifest_signing_bytes(raw_manifest: &str) -> Result<Vec<u8>, String> {
    let doc: toml::Value = toml::from_str(raw_manifest)
        .map_err(|e| format!("manifest parse: {e}"))?;

    let app  = doc.get("app").cloned()
        .unwrap_or(toml::Value::Table(Default::default()));
    let caps = doc.get("capabilities").cloned()
        .unwrap_or(toml::Value::Table(Default::default()));

    // Convert to serde_json for deterministic key-sorted serialization.
    let canonical = serde_json::json!({
        "app":          toml_val_to_sorted_json(app),
        "capabilities": toml_val_to_sorted_json(caps),
    });
    serde_json::to_vec(&canonical)
        .map_err(|e| format!("canonical json: {e}"))
}

pub fn code_dir_hash_v2(wasm_bytes: &[u8], manifest_raw: &str, name: &str)
    -> Result<[u8; 32], String>
{
    use sha2::{Digest, Sha256};
    let signing_bytes = manifest_signing_bytes(manifest_raw)?;
    let wasm_hash:     [u8; 32] = Sha256::digest(wasm_bytes).into();
    let manifest_hash: [u8; 32] = Sha256::digest(&signing_bytes).into();

    let mut h = Sha256::new();
    h.update(b"VYOMA_CODE_DIR_V2\x00");
    h.update(&wasm_hash);
    h.update(&manifest_hash);
    h.update(name.as_bytes());
    h.update(b"\x00");
    Ok(h.finalize().into())
}
```

---

## 5. Ed25519 Verification — Pure Rust, No C (B2 Fix)

```toml
# supervisor/Cargo.toml additions
ed25519-dalek = { version = "2", default-features = false, features = ["alloc", "digest"] }
getrandom     = { version = "0.2", features = ["custom"] }
```

```rust
// supervisor/src/codesign/mod.rs
// Supervisor only verifies — register a no-op custom getrandom to satisfy
// the linker without touching the seccomp-denied getrandom(2) syscall.
use getrandom::register_custom_getrandom;
fn supervisor_no_getrandom(_buf: &mut [u8]) -> Result<(), getrandom::Error> {
    panic!("getrandom called in supervisor — key generation must use vyoma-sign");
}
register_custom_getrandom!(supervisor_no_getrandom);
```

```rust
// supervisor/src/codesign/verifier.rs
use ed25519_dalek::{Signature, VerifyingKey};

pub fn verify_sig(pubkey: &[u8; 32], sig: &[u8; 64], msg_hash: &[u8; 32])
    -> Result<(), &'static str>
{
    let vk = VerifyingKey::from_bytes(pubkey)
        .map_err(|_| "invalid public key")?;
    let s = Signature::from_bytes(sig);
    vk.verify_strict(msg_hash, &s)
        .map_err(|_| "signature verification failed")
}
```

---

## 6. TOCTOU-Safe Spawn via `memfd_create` + Seals (B3 Fix)

```rust
// supervisor/src/codesign/verifier.rs

/// Verify the WASM binary and return a sealed memfd — the verified bytes
/// in an immutable anonymous mapping that cannot be swapped for a malicious
/// binary between verification and wasmtime exec.
pub fn verify_and_seal(
    wasm_path:     &std::path::Path,
    manifest_path: &std::path::Path,
    name:          &str,
    data_dir:      &str,
) -> Result<(TrustLevel, std::fs::File), String> {
    let wasm_bytes    = std::fs::read(wasm_path)
        .map_err(|e| format!("read wasm: {e}"))?;
    let manifest_raw  = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("read manifest: {e}"))?;

    let trust = verify_in_memory(&wasm_bytes, &manifest_raw, name, data_dir)?;

    // Create sealed memfd: write verified bytes, then seal against further writes.
    let name_c = std::ffi::CString::new(name).unwrap();
    let fd = unsafe {
        libc::syscall(
            libc::SYS_memfd_create,
            name_c.as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        ) as i32
    };
    if fd < 0 { return Err(format!("memfd_create: {}", std::io::Error::last_os_error())); }

    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    use std::io::{Seek, SeekFrom, Write};
    file.write_all(&wasm_bytes).map_err(|e| format!("memfd write: {e}"))?;

    let seal_ret = unsafe {
        libc::fcntl(fd, libc::F_ADD_SEALS,
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK)
    };
    if seal_ret < 0 { return Err("memfd seal failed".into()); }
    file.seek(SeekFrom::Start(0)).map_err(|e| format!("memfd seek: {e}"))?;

    Ok((trust, file))
}
```

In `spawn_app`, replace path-based wasmtime invocation:

```rust
let (trust, wasm_fd) = codesign::verifier::verify_and_seal(
    &wasm_path, &manifest_path, name, DATA_DIR)?;
let fd_path = format!("/proc/self/fd/{}", {
    use std::os::unix::io::AsRawFd;
    wasm_fd.as_raw_fd()
});
cmd.arg(&fd_path);
let child = cmd.spawn()?;
drop(wasm_fd);  // seal still holds in child; Drop on our side frees the fd
```

---

## 7. `codesign-trust-add` Authorization Gate (B4 Fix)

```rust
// supervisor/src/codesign/ipc.rs

pub fn handle_codesign_trust_add(
    hex_pubkey:   &str,
    label:        &str,
    sender:       &str,
    inbox:        &Inbox,
    app_registry: &AppRegistry,
    data_dir:     &str,
) {
    // Requires codesign_admin capability AND Official trust level.
    let (has_admin, trust) = {
        let reg = app_registry.lock().unwrap();
        reg.get(sender).map(|st| {
            let s = st.lock().unwrap();
            (s.codesign_admin, s.trust_level)
        }).unwrap_or((false, TrustLevel::Unsigned))
    };
    if !has_admin || trust != TrustLevel::Official {
        send_reply(sender,
            "REPLY:codesign-trust-add error: codesign_admin + Official trust required",
            inbox);
        return;
    }
    // R41 transactional write to /data/.vyoma/trusted_keys/<hex>.pub.toml
    write_dev_key_r41(hex_pubkey, label, data_dir)
        .map(|_| send_reply(sender,
            &format!("REPLY:codesign-trust-add ok {hex_pubkey}"), inbox))
        .unwrap_or_else(|e| send_reply(sender,
            &format!("REPLY:codesign-trust-add error {e}"), inbox));
}
```

`AppState` gains `codesign_admin: bool` and `trust_level: TrustLevel` fields set during `spawn_app` from the codesign verification result.

---

## 8. Sidecar Naming Normalization (B5 Fix)

The `.sig` sidecar is always named `<app.name>.sig` (logical app name from `vyoma.toml [app] name`), never derived from `AppMeta.wasm` filename. `verify_app` constructs:

```rust
let sig_path = wasm_path.parent()
    .unwrap_or(Path::new("."))
    .join(format!("{name}.sig"));  // `name` = manifest.app.name
```

The `update` IPC command is extended to accept an optional `<sig_url>` parameter. If Gatekeeper `allow_unsigned = false` and no `.sig_url` is provided, the update is rejected before any file download.

---

## 9. Gatekeeper Policy

```toml
# /data/.vyoma/gatekeeper.toml
[policy]
allow_unsigned  = false   # true = developer mode
allow_developer = true    # false = official-only (kiosk/production)
```

In `spawn_app` after `verify_and_seal`:

```rust
match (trust, gatekeeper_policy) {
    (TrustLevel::Unsigned, p) if !p.allow_unsigned => abort_spawn("unsigned app blocked"),
    (TrustLevel::Developer, p) if !p.allow_developer => abort_spawn("developer app blocked"),
    _ => { /* proceed */ }
}
```

---

## 10. Protocol

```
@supervisor: codesign-verify <app>
→ REPLY:codesign <app> trust=Official|Developer|Unsigned key=<hex>|none valid=true|false

@supervisor: codesign-trust-add <hex_pubkey_32_bytes> <label>
→ REPLY:codesign-trust-add ok <hex>  (requires codesign_admin + Official trust)

@supervisor: codesign-revoke <hex_pubkey_32_bytes>
→ REPLY:codesign-revoke ok
```

---

## 11. Cargo Additions

```toml
ed25519-dalek = { version = "2", default-features = false, features = ["alloc", "digest"] }
getrandom     = { version = "0.2", features = ["custom"] }
serde_json    = "1"   # already present
```

All pure Rust, no C dependencies, musl-compatible.

---

## 12. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Code directory hashes raw manifest bytes — `[window]` section mutated by supervisor at runtime invalidates the signature | Canonical subset: only `[app]` + `[capabilities]` in key-sorted JSON form; `manifest_signing_bytes()` excludes `[window]`; domain separator bumped to `V2` |
| B2: `ed25519-dalek` v2 pulls `getrandom` which calls the `getrandom(2)` syscall blocked by seccomp; adds ~100KB to binary | `features = ["alloc", "digest"]` (no key-gen); `register_custom_getrandom!` with a `panic!` stub satisfies the linker without a syscall |
| B3: TOCTOU race: `verify_app()` reads wasm at T1, wasmtime exec at T2 — attacker replaces wasm in `/data/apps/` between T1 and T2 | `verify_and_seal()` reads bytes into memory, writes to `memfd_create` + `F_SEAL_WRITE\|F_SEAL_GROW\|F_SEAL_SHRINK`, passes `/proc/self/fd/<N>` to wasmtime — immutable from verification to exec |
| B4: `codesign-trust-add` has no auth — any `shell=true` app can elevate arbitrary keys to developer trust | New `codesign_admin: bool` manifest capability; IPC handler additionally requires `AppState.trust_level == TrustLevel::Official`; double gate prevents escalation |
| B5: `.sig` path constructed from `AppMeta.wasm` filename; after `update` command renames binary to `<app_name>.wasm`, sidecar path mismatches and verification always fails | Sidecar always named `<manifest.app.name>.sig` (logical name, not wasm filename); `update` command requires `<sig_url>` parameter; `vyoma-sign` writes `<app.name>.sig` |
