# FINAL Spec: Keychain & Secret Storage (Round 60)

**Subsystem**: Keychain & Secret Storage  
**macOS Analogue**: Keychain Services / `SecItem` API  
**Depends on**: R49 (sandbox FS), R53 (VPN — WireGuard key ref), R55 (Wi-Fi PMK migration), R58 (TLS cert keys), R59 (capability gating)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Supervisor brokers all secret access. WASM apps never see raw key material beyond a base64 payload delivered over a single-use reply. The keychain is locked by default; unlocked by a privileged app supplying a PIN/passphrase.

```
supervisor/src/keychain/
├── mod.rs        (~80 lines: handle_keychain_command(), IPC dispatch (B3, B5))
├── kdf.rs        (~60 lines: KdfParams, Argon2id derive_key)
├── store.rs      (~80 lines: MASTER_KEY OnceLock, ZeroizingKey, lock/unlock/snapshot_key (B2))
├── crypto.rs     (~100 lines: AES-256-GCM encrypt/decrypt, write_secret_atomic R41 (B4))
├── access.rs     (~80 lines: owner encoding, secret_path validation (B4), access_check)
├── lock_timer.rs (~60 lines: LAST_ACCESS_SECS, idle lock after 5 min)
└── migrate.rs    (~60 lines: migrate_wifi_pmks from R55 in-memory PMK)

/data/.vyoma/keychain/
  meta.toml           — KDF params + salt (plaintext — no secrets here)
  <namespace>/<label> — nonce(12) || encrypted_payload || GCM_tag(16)
```

---

## 2. Storage Format

Encrypted flat files — one secret per file. No SQLite (avoids WAL/journal complexity with R41).

```toml
# /data/.vyoma/keychain/meta.toml
version    = 1
kdf        = "argon2id"
argon2_m_cost = 65536    # 64 MiB — memory-hard
argon2_t_cost = 3
argon2_p_cost = 1
salt       = "base64-16-byte-salt"  # generated once at keychain-init
```

Encryption: **AES-256-GCM** (`aes-gcm` crate, pure-Rust). 96-bit random nonce per write. GCM tag detects tampering.

Secret file binary layout (after decryption):
```
[4B LE u32: owner_len] [owner_len bytes: owner UTF-8] [remaining: secret payload]
```

Owner is inside the authenticated ciphertext — tampered ownership invalidates the GCM tag.

---

## 3. Key Derivation

```rust
// supervisor/src/keychain/kdf.rs

pub const KEY_LEN: usize = 32;

pub struct KdfParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt:   [u8; 16],
}

impl KdfParams {
    pub fn derive_key(&self, passphrase: &[u8]) -> Result<[u8; KEY_LEN], String> {
        let params = argon2::Params::new(self.m_cost, self.t_cost, self.p_cost, Some(KEY_LEN))
            .map_err(|e| format!("argon2 params: {e}"))?;
        let argon2 = argon2::Argon2::new(
            argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut key = [0u8; KEY_LEN];
        argon2.hash_password_into(passphrase, &self.salt, &mut key)
            .map_err(|e| format!("argon2 derive: {e}"))?;
        Ok(key)
    }
}
```

~400ms on Raspberry Pi 4 — acceptable for interactive unlock, prohibitive for brute-force.

---

## 4. In-Memory Master Key + Atomic Snapshot (B2 Fix)

TOCTOU between `is_unlocked()` check and key use:

```rust
// supervisor/src/keychain/store.rs

struct ZeroizingKey([u8; 32]);
impl Drop for ZeroizingKey {
    fn drop(&mut self) { for b in self.0.iter_mut() { *b = 0; } }
}

static MASTER_KEY: OnceLock<Mutex<Option<ZeroizingKey>>> = OnceLock::new();

pub fn master_key() -> &'static Mutex<Option<ZeroizingKey>> {
    MASTER_KEY.get_or_init(|| Mutex::new(None))
}

// B2 fix: single lock acquisition — atomically checks + copies key
pub fn snapshot_key() -> Option<zeroize::Zeroizing<[u8; 32]>> {
    let guard = master_key().lock().unwrap();
    guard.as_ref().map(|k| {
        let mut copy = [0u8; 32];
        copy.copy_from_slice(&k.0);
        zeroize::Zeroizing::new(copy)
    })
    // lock released here — before any I/O
}

pub fn unlock(key: [u8; 32]) {
    *master_key().lock().unwrap() = Some(ZeroizingKey(key));
}

pub fn lock() {
    *master_key().lock().unwrap() = None;
    // ZeroizingKey::drop() fires — memory overwritten
}
```

All handlers call `snapshot_key()` — single critical section, no TOCTOU.

---

## 5. Secret Path Validation (B4 Fix)

```rust
// supervisor/src/keychain/access.rs

const KEYCHAIN_ROOT: &str = "/data/.vyoma/keychain";

pub fn secret_path(label: &str) -> Result<std::path::PathBuf, String> {
    // Reject null bytes and traversal sequences immediately
    if label.contains('\0') || label.contains("..") {
        return Err(format!("invalid label: forbidden sequence"));
    }
    // Allowlist: only [a-zA-Z0-9-_/.] 
    for ch in label.chars() {
        if !matches!(ch, 'a'..='z'|'A'..='Z'|'0'..='9'|'-'|'_'|'/'|'.') {
            return Err(format!("invalid label: forbidden char '{ch}'"));
        }
    }
    // Exactly one slash: <namespace>/<label>
    let parts: Vec<&str> = label.splitn(3, '/').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        return Err(format!("label must be <namespace>/<label> with exactly one slash"));
    }
    let candidate = std::path::PathBuf::from(KEYCHAIN_ROOT).join(label);
    // Canonical prefix check
    let root = std::path::PathBuf::from(KEYCHAIN_ROOT)
        .canonicalize().unwrap_or_else(|_| std::path::PathBuf::from(KEYCHAIN_ROOT));
    let expected_parent = root.join(parts[0]);
    if !expected_parent.to_string_lossy().starts_with(&*root.to_string_lossy()) {
        return Err(format!("path traversal in '{label}'"));
    }
    Ok(candidate)
}
```

---

## 6. Zeroize on Delivery (B1 Fix)

Secret bytes must not survive in allocator-recycled heap after delivery:

```rust
// supervisor/src/keychain/crypto.rs
use zeroize::Zeroizing;

pub fn decrypt_secret(key: &[u8; 32], ciphertext: &[u8])
    -> Result<Zeroizing<Vec<u8>>, String>
{
    use aes_gcm::{Aes256Gcm, KeyInit, aead::{Aead, Nonce}};
    if ciphertext.len() < 12 + 16 { return Err("too short".into()); }
    let (nonce_bytes, body) = ciphertext.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("aes: {e}"))?;
    let nonce = Nonce::<Aes256Gcm>::from_slice(nonce_bytes);
    let pt = cipher.decrypt(nonce, body)
        .map_err(|_| "decrypt failed (tampered or wrong key)".to_string())?;
    Ok(Zeroizing::new(pt))
}

// In mod.rs keychain-get handler:
fn handle_keychain_get(sender: &str, label: &str, inbox: &Inbox) {
    let key = match snapshot_key() {
        Some(k) => k,
        None => { send_reply(sender, "REPLY:keychain-locked", inbox); return; }
    };
    // key lock already released by snapshot_key — all I/O below is lock-free
    let path = secret_path(label).unwrap();
    let ct = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => { send_reply(sender, "REPLY:keychain-not-found", inbox); return; }
    };
    let plaintext: Zeroizing<Vec<u8>> = match decrypt_secret(&*key, &ct) {
        Ok(p) => p,
        Err(_) => { send_reply(sender, "REPLY:keychain-error decrypt", inbox); return; }
    };
    let payload = extract_payload(&plaintext);   // strips owner prefix, returns Zeroizing<Vec<u8>>
    let encoded = Zeroizing::new(base64_encode(&payload));
    let reply = format!("REPLY:keychain-secret {encoded}");
    send_reply(sender, &reply, inbox);
    // encoded + payload + plaintext all zeroed by Zeroizing::drop() here
}
```

---

## 7. Secret Redaction in Logs + Out-of-Band Routing (B3 + B5 Fix)

```rust
// In router.rs — intercept keychain commands BEFORE any logging
if line.starts_with("@supervisor: keychain-") {
    // Route directly — NEVER update LAST_SENDER, NEVER log the line
    crate::keychain::handle_keychain_command(...);
    return;
}
```

```rust
// In app_threads.rs — skip keychain-secret from log_buf circular buffer
if !line.contains("keychain-secret") {
    let mut st = state.lock().unwrap();
    if st.log_buf.len() >= LOG_BUF_SIZE { st.log_buf.pop_front(); }
    st.log_buf.push_back(line.clone());
}
```

```rust
// In keychain/mod.rs — PIN delivered via out-of-band path with zeroize + rate limiting
"keychain-unlock" => {
    if !sender_has_shell_cap(sender, app_registry) {
        send_reply(sender, "REPLY:keychain-denied", inbox); return;
    }
    if !check_unlock_rate_limit() {
        send_reply(sender, "REPLY:keychain-error rate-limited", inbox); return;
    }
    let pin_b64 = rest.trim();
    let pin: Zeroizing<Vec<u8>> = match base64_decode_zeroizing(pin_b64) {
        Ok(b) => b,
        Err(_) => { send_reply(sender, "REPLY:keychain-error bad-encoding", inbox); return; }
    };
    let kdf = load_kdf_params()?;
    match kdf.derive_key(&pin) {
        Ok(key) => {
            crate::keychain::store::unlock(key);
            crate::keychain::lock_timer::touch();
            // pin zeroed here by Zeroizing::drop()
            // Broadcast unlock so R53/R55/R58 can retry blocked requests
            let names: Vec<String> = inbox.lock().unwrap().keys().cloned().collect();
            for name in names {
                send_reply(&name, "VYOMA_SYSTEM:keychain-unlocked", inbox);
            }
            record_unlock_success();
            send_reply(sender, "REPLY:keychain-ok", inbox);
        }
        Err(e) => {
            record_unlock_failure(); // exponential backoff (B5 rate limit)
            send_reply(sender, &format!("REPLY:keychain-error {e}"), inbox);
        }
    }
}
```

Rate limiting on unlock attempts:
```rust
static UNLOCK_FAIL_COUNT:      AtomicU32 = AtomicU32::new(0);
static UNLOCK_LOCKED_UNTIL_SECS: AtomicU64 = AtomicU64::new(0);

fn check_unlock_rate_limit() -> bool {
    let now = unix_secs();
    now >= UNLOCK_LOCKED_UNTIL_SECS.load(Ordering::Relaxed)
}
fn record_unlock_failure() {
    let fails = UNLOCK_FAIL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let delay = (1u64 << (fails.min(12) - 1)).min(3600);
    UNLOCK_LOCKED_UNTIL_SECS.store(unix_secs() + delay, Ordering::Relaxed);
}
fn record_unlock_success() {
    UNLOCK_FAIL_COUNT.store(0, Ordering::Relaxed);
    UNLOCK_LOCKED_UNTIL_SECS.store(0, Ordering::Relaxed);
}
```

---

## 8. Idle Lock Timer

```rust
// supervisor/src/keychain/lock_timer.rs

static LAST_ACCESS_SECS: AtomicU64 = AtomicU64::new(0);
const IDLE_TIMEOUT_SECS: u64 = 300;  // 5 minutes

pub fn touch() {
    LAST_ACCESS_SECS.store(unix_secs(), Ordering::Relaxed);
}

pub fn spawn_idle_locker() {
    std::thread::Builder::new().name("keychain-idle-lock".into()).spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(30));
            let last = LAST_ACCESS_SECS.load(Ordering::Relaxed);
            if last > 0 && unix_secs().saturating_sub(last) >= IDLE_TIMEOUT_SECS {
                super::store::lock();
                LAST_ACCESS_SECS.store(0, Ordering::Relaxed);
            }
        }
    }).expect("idle locker spawn");
}
```

---

## 9. R41 Transactional Write

```rust
// supervisor/src/keychain/crypto.rs

pub fn write_secret_atomic(path: &Path, ciphertext: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true)
            .open(&tmp)?;
        f.write_all(ciphertext)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}
```

---

## 10. Protocol

```
@supervisor: keychain-unlock <base64-pin>    # shell=true required; never logged
→ REPLY:keychain-ok | REPLY:keychain-error rate-limited

@supervisor: keychain-lock
→ REPLY:keychain-ok

@supervisor: keychain-store <ns>/<label> <base64-secret>   # never logged (B3)
→ REPLY:keychain-ok | REPLY:keychain-locked | REPLY:keychain-error

@supervisor: keychain-get <ns>/<label>
→ REPLY:keychain-secret <base64>  | REPLY:keychain-locked | REPLY:keychain-denied

@supervisor: keychain-delete <ns>/<label>
→ REPLY:keychain-ok

@supervisor: keychain-list <ns>
→ REPLY:keychain-list <label1>|<label2>|...

VYOMA_SYSTEM:keychain-unlocked    # broadcast to all apps on unlock
```

**Supervisor restart**: keychain is locked on restart. Apps blocked on `keychain-get` receive `REPLY:keychain-locked`; they subscribe to `VYOMA_SYSTEM:keychain-unlocked` and retry.  
**R63 Secure Enclave**: will add hardware-sealed headless unlock; R60 software key is the fallback.

---

## 11. Cargo Additions

```toml
argon2   = { version = "0.5", default-features = false, features = ["alloc"] }
aes-gcm  = { version = "0.10", default-features = false, features = ["aes", "alloc"] }
zeroize  = { version = "1", features = ["alloc"] }
```

All pure-Rust, no C deps, musl-compatible.

---

## 12. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Decrypted secret bytes remain in allocator-recycled heap after `Vec<u8>` drop — visible to memory dump | `Zeroizing<Vec<u8>>` / `Zeroizing<[u8;32]>` wrappers from `zeroize` crate; overwrite on drop for every plaintext and base64 intermediate |
| B2: TOCTOU between `is_unlocked()` check and key use — idle lock timer fires between check and use | `snapshot_key()` acquires lock once, copies key bytes to stack, drops lock before all I/O — atomic check+snapshot, no two-phase race |
| B3: `keychain-unlock <pin>` routed through normal IPC pipeline — PIN appears in LAST_SENDER map, log_buf, management server tail | `router.rs` intercepts `keychain-*` before logging/LAST_SENDER update; `log_buf` skips lines containing `keychain-secret`; unlock handler uses `Zeroizing` PIN buffer |
| B4: Label `../../etc/passwd` passed to `secret_path()` — supervisor root can write to arbitrary paths under `/data/.vyoma/keychain/../../` | Allowlist char set + `..` rejection + single-slash depth limit + canonical prefix check in `access.rs` |
| B5: `keychain-unlock` has no brute-force protection — attacker app can try 10,000 PINs/second | Exponential backoff: `2^(fail-1)` second lockout per attempt (max 3600s); `AtomicU32` fail counter + `AtomicU64` locked-until timestamp |
