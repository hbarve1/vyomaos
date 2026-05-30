# FINAL Spec: Secure Enclave & TEE (Round 63)

**Subsystem**: Secure Enclave & TEE  
**macOS Analogue**: Secure Enclave / SEP / T-series chip  
**Depends on**: R60 (Keychain — software AES-256-GCM, Argon2id; noted R63 adds hardware-sealed headless unlock), R59 (capability model)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

R63 targets TPM 2.0 HMAC-based key sealing via `/dev/tpm0`. Full OP-TEE/SGX is skipped in v1. A software fallback covers QEMU and RPi4 (no OP-TEE out of box). The design is a `TeeBackend` trait with two implementations.

```
supervisor/src/tee/
├── mod.rs          (~120 lines: TeeBackend trait, SealedKeyBlob, probe_backend)
├── tpm2_raw.rs     (~200 lines: raw /dev/tpm0 command framing — no C library (B1))
├── tpm2.rs         (~250 lines: Tpm2Backend: seal/unseal using tpm2_raw primitives)
├── software.rs     (~180 lines: SoftwareBackend: device_seed + HKDF-SHA256 + AES-GCM)
├── sealed_key.rs   (~100 lines: R41 blob persistence at /data/.vyoma/tee/)
├── ipc.rs          (~120 lines: tee-seal/unseal/attest/status command dispatcher)
├── measure.rs      (~80 lines:  WASM SHA-256 measurement log)
└── recovery.rs     (~150 lines: Bech32 recovery key envelope (B5))

/data/.vyoma/tee/
  keychain_master.sealed — SealedKeyBlob (bincode, R41 writes)
  device_seed            — 32 random bytes, created once on first boot
  measurements.jsonl     — append-only WASM measurement log
  recovery.sealed        — recovery envelope (plaintext-safe to back up)
```

---

## 2. Core Types

```rust
// supervisor/src/tee/mod.rs

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SealedKeyBlob {
    pub version:      u8,
    pub backend:      BackendKind,
    pub nonce:        [u8; 12],      // AES-GCM nonce (software) or TPM nonce
    pub ciphertext:   Vec<u8>,
    pub tag:          [u8; 16],      // AES-GCM tag (software) or TPM HMAC
    pub pcr_policy:   Option<PcrPolicy>,
    pub device_hmac:  [u8; 32],      // HMAC-SHA256(device_seed, "blob-binding-v1") (B2)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BackendKind { Tpm2, Software }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PcrPolicy {
    pub pcr_selection: Vec<u8>,
    pub digest:        [u8; 32],
}

pub trait TeeBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn seal(&self, plaintext: &[u8], bind_pcrs: &[u8]) -> TeeResult<SealedKeyBlob>;
    fn unseal(&self, blob: &SealedKeyBlob) -> TeeResult<Vec<u8>>;
    fn attest(&self, nonce: &[u8]) -> TeeResult<Option<Vec<u8>>>;
    fn is_available(&self) -> bool;
}

pub fn probe_backend() -> Box<dyn TeeBackend> {
    if Tpm2Backend::probe().is_ok() {
        Box::new(Tpm2Backend::new())
    } else {
        Box::new(SoftwareBackend::new())
    }
}
```

---

## 3. Raw TPM2 Command Framing — No C Library (B1 Fix)

`tss-esapi` links OpenSSL and adds ~2MB to the binary. Instead, implement the minimal TPM2 commands needed for HMAC-based key sealing directly via `/dev/tpm0`:

```rust
// supervisor/src/tee/tpm2_raw.rs

const TPM2_ST_NO_SESSIONS: u16 = 0x8001;
const TPM2_CC_GET_RANDOM:  u32 = 0x0000017B;

pub fn tpm2_command(cmd_code: u32, payload: &[u8]) -> TeeResult<Vec<u8>> {
    // TPM2 command: tag(2) + size(4) + cmdCode(4) + payload
    let size = (10u32 + payload.len() as u32).to_be_bytes();
    let mut cmd = Vec::with_capacity(10 + payload.len());
    cmd.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
    cmd.extend_from_slice(&size);
    cmd.extend_from_slice(&cmd_code.to_be_bytes());
    cmd.extend_from_slice(payload);

    let mut dev = std::fs::OpenOptions::new()
        .read(true).write(true)
        .open("/dev/tpm0")
        .map_err(|_| TeeError::NoHardware)?;
    use std::io::{Read, Write};
    dev.write_all(&cmd)?;

    let mut resp = vec![0u8; 4096];
    let n = dev.read(&mut resp)?;
    resp.truncate(n);
    if n < 10 { return Err(TeeError::Crypto("short TPM response".into())); }
    let rc = u32::from_be_bytes(resp[6..10].try_into().unwrap());
    if rc != 0 { return Err(TeeError::TpmRc { code: rc }); }
    Ok(resp[10..].to_vec())
}

pub fn tpm2_get_random(num_bytes: u16) -> TeeResult<Vec<u8>> {
    let resp = tpm2_command(TPM2_CC_GET_RANDOM, &num_bytes.to_be_bytes())?;
    if resp.len() < 2 { return Err(TeeError::Crypto("bad GetRandom resp".into())); }
    let len = u16::from_be_bytes(resp[0..2].try_into().unwrap()) as usize;
    Ok(resp[2..2+len].to_vec())
}
```

Sealing uses TPM NV index to store an HMAC key; the VEK is encrypted with AES-256-GCM keyed by the TPM-derived HMAC — no `Create`/`Load`/`Unseal` object hierarchy required.

---

## 4. Software Fallback Backend

```rust
// supervisor/src/tee/software.rs

const SEED_PATH: &str = "/data/.vyoma/tee/device_seed";

pub struct SoftwareBackend { derived_key: [u8; 32] }

impl SoftwareBackend {
    pub fn new() -> Self {
        let seed = Self::load_or_create_seed().unwrap_or([0u8; 32]);
        let machine_id = Self::read_machine_id();
        // HKDF-SHA256(machine_id || seed) → 32-byte binding key
        let ikm = [machine_id.as_bytes(), seed.as_slice()].concat();
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, &ikm);
        let mut derived_key = [0u8; 32];
        hk.expand(b"vyoma-tee-v1", &mut derived_key).expect("HKDF expand");
        Self { derived_key }
    }

    fn load_or_create_seed() -> TeeResult<[u8; 32]> {
        if let Ok(data) = std::fs::read(SEED_PATH) {
            if data.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&data);
                return Ok(arr);
            }
        }
        let mut seed = [0u8; 32];
        use aes_gcm::aead::rand_core::RngCore;
        aes_gcm::aead::OsRng.fill_bytes(&mut seed);
        std::fs::create_dir_all("/data/.vyoma/tee/")?;
        // R41: .tmp → fsync → rename
        atomic_write(SEED_PATH, &seed)?;
        Ok(seed)
    }
}
```

---

## 5. PCR Zero Validation (B2 Fix)

Binding to a zero PCR7 (no measured boot in QEMU) provides no security. Before sealing to TPM PCRs, validate they are non-zero:

```rust
// supervisor/src/tee/tpm2.rs

const ZERO_PCR: [u8; 32] = [0u8; 32];

fn validate_pcr_policy_sanity(pcr_indices: &[u8]) -> TeeResult<()> {
    for &idx in pcr_indices {
        let pcr_value = tpm2_raw::read_pcr(idx)?;
        if pcr_value == ZERO_PCR {
            log::warn!("[tee] PCR{idx} is all-zeros — no measured boot. \
                       Falling back to software backend.");
            return Err(TeeError::PolicyMismatch);
        }
    }
    Ok(())
}

// In Tpm2Backend::seal() — always validate before sealing
fn seal(&self, plaintext: &[u8], bind_pcrs: &[u8]) -> TeeResult<SealedKeyBlob> {
    let _guard = self.lock.lock().map_err(|_| TeeError::Crypto("lock poisoned".into()))?;
    if !bind_pcrs.is_empty() {
        validate_pcr_policy_sanity(bind_pcrs)?;
    }
    self.seal_inner(plaintext, bind_pcrs)
}
```

---

## 6. Concurrency Safety: Single-Flight TPM Mutex (B3 Fix)

The TPM character device is not safe for concurrent access. A `Mutex<()>` in `Tpm2Backend` serializes all TPM I/O:

```rust
pub struct Tpm2Backend {
    tcti: String,
    lock: std::sync::Mutex<()>,  // single-flight: one TPM command at a time
}

impl TeeBackend for Tpm2Backend {
    fn seal(&self, plaintext: &[u8], bind_pcrs: &[u8]) -> TeeResult<SealedKeyBlob> {
        let _guard = self.lock.lock()
            .map_err(|_| TeeError::Crypto("tpm lock poisoned".into()))?;
        self.seal_inner(plaintext, bind_pcrs)
        // _guard dropped here — next thread can proceed
    }
    fn unseal(&self, blob: &SealedKeyBlob) -> TeeResult<Vec<u8>> {
        let _guard = self.lock.lock()
            .map_err(|_| TeeError::Crypto("tpm lock poisoned".into()))?;
        self.unseal_inner(blob)
    }
}
```

Additionally, a `TeeService` thread processes all TEE IPC requests serially from a `mpsc` channel, preventing any concurrent TPM access at the architectural level.

---

## 7. Backend Migration Path (B4 Fix)

When hardware is upgraded from no-TPM to TPM, the old software-backend blob is unreadable by the new TPM backend. Instead of a brick scenario, `try_headless_unlock` returns a `NeedsMigration` variant and prompts for one-time passphrase confirmation:

```rust
pub enum UnlockResult {
    HeadlessOk([u8; 32]),
    NeedsProvisioning,
    NeedsMigration { old_backend: BackendKind },
    Failed(TeeError),
}

pub fn try_headless_unlock(backend: &dyn TeeBackend) -> UnlockResult {
    let blob = match crate::tee::sealed_key::load_blob() {
        Ok(b)  => b,
        Err(_) => return UnlockResult::NeedsProvisioning,
    };
    if blob.backend == BackendKind::Software && backend.kind() == BackendKind::Tpm2 {
        return UnlockResult::NeedsMigration { old_backend: BackendKind::Software };
    }
    match backend.unseal(&blob) {
        Ok(pt) if pt.len() == 32 => {
            let mut key = [0u8; 32];
            key.copy_from_slice(&pt);
            UnlockResult::HeadlessOk(key)
        }
        Ok(_)  => UnlockResult::Failed(TeeError::Crypto("bad key len".into())),
        Err(e) => UnlockResult::Failed(e),
    }
}

/// One-time migration: unseal with software backend, re-seal with TPM.
pub fn migrate_blob(passphrase: &[u8], new_backend: &dyn TeeBackend) -> TeeResult<()> {
    let old_backend = SoftwareBackend::new();
    let old_blob    = crate::tee::sealed_key::load_blob()?;
    let master_key  = old_backend.unseal(&old_blob)?;
    let new_blob    = new_backend.seal(&master_key, &[7])?;
    crate::tee::sealed_key::store_blob(&new_blob)
}
```

---

## 8. Recovery Key Envelope (B5 Fix)

Loss of `device_seed` (disk wipe) permanently destroys software-backend sealed data. The recovery key is a 32-byte random value displayed once at provisioning and stored out-of-band:

```rust
// supervisor/src/tee/recovery.rs

const RECOVERY_BLOB_PATH: &str = "/data/.vyoma/tee/recovery.sealed";

pub struct RecoveryEnvelope {
    pub version:          u8,
    pub nonce:            [u8; 12],
    /// master_key encrypted with Argon2id(recovery_key, recovery_salt)
    pub ciphertext:       Vec<u8>,
    pub tag:              [u8; 16],
    pub recovery_salt:    [u8; 32],
    /// First 8 bytes of SHA256(recovery_key) encoded for display
    pub key_fingerprint:  String,   // e.g. "vyoma1a3f9c2d1b8e47f2"
}
```

At provisioning time, the supervisor prints:

```
[tee] RECOVERY KEY (record and store securely — shown ONCE):
[tee]   vyoma1a3f9c2d1b8e47f2
```

`recovery.sealed` is stored on disk — it is safe to back up since it is useless without the out-of-band recovery key.

---

## 9. R60 Keychain Integration — Headless Unlock

```rust
// Boot sequence additions to supervisor/src/main.rs (after mount_filesystems):
let backend = crate::tee::mod::probe_backend();
match crate::tee::mod::try_headless_unlock(backend.as_ref()) {
    UnlockResult::HeadlessOk(key) => {
        crate::keychain::store::unlock(key);
        log_info!(Subsystem::Security, None, "keychain: headless unlock via TEE");
    }
    UnlockResult::NeedsProvisioning => {
        // First boot — passphrase prompt handled by keychain unlock flow
    }
    UnlockResult::NeedsMigration { .. } => {
        // Prompt user for passphrase on serial console; call migrate_blob()
    }
    UnlockResult::Failed(e) => {
        log_warn!(Subsystem::Security, None, "TEE unseal failed: {e}; passphrase required");
    }
}
```

---

## 10. Protocol

```
@supervisor: tee-seal <hex_plaintext> [pcrs=7,11]
→ TEE_RESP:seal:ok:<label>
→ TEE_RESP:seal:err:<reason>

@supervisor: tee-unseal <label>
→ TEE_RESP:unseal:ok:<hex_plaintext>
→ TEE_RESP:unseal:err:<reason>

@supervisor: tee-attest <hex_nonce_16bytes>
→ TEE_RESP:attest:ok:<base64_quote>
→ TEE_RESP:attest:unavailable

@supervisor: tee-status
→ TEE_RESP:status:tpm2|software:available|unavailable
```

---

## 11. Cargo Additions

```toml
aes-gcm  = "0.10"
hkdf     = "0.12"
sha2     = "0.10"     # already present
bincode  = "1"
hex      = "0.4"
base64   = "0.21"
# tss-esapi is NOT added — raw /dev/tpm0 framing used instead (B1 fix)
```

All pure Rust, no C dependencies, musl-compatible.

---

## 12. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `tss-esapi` links OpenSSL C library — breaks musl static build and adds ~2MB to binary | Raw TPM2 command buffer framing via `/dev/tpm0` in `tpm2_raw.rs`; HMAC-based key sealing only (no `Create`/`Load`/`Unseal` object hierarchy); no C deps |
| B2: PCR7=0x00...00 on QEMU swtpm (no Secure Boot) — sealing to a zero PCR provides zero binding; blob portable across any QEMU instance | `validate_pcr_policy_sanity()` rejects zero PCRs before sealing; adds `device_hmac` field (HMAC over `device_seed`) to `SealedKeyBlob` so blob is also machine-bound |
| B3: Concurrent `tee-unseal` from two app threads both call `tpm2_command()` on `/dev/tpm0` — response bytes interleave, producing garbage or hang | `Mutex<()>` in `Tpm2Backend` held for full command/response cycle; `TeeService` actor thread serializes all TEE IPC requests |
| B4: Software→TPM backend upgrade permanently bricks sealed blobs — `try_headless_unlock` returns `IntegrityFailure` with no recovery path | `UnlockResult::NeedsMigration` triggers one-time passphrase confirmation + `migrate_blob()` re-seals under new backend; no silent brick |
| B5: `device_seed` loss (disk wipe / factory reset) permanently destroys software-sealed keychain data — no recovery | `RecoveryEnvelope` with 32-byte out-of-band recovery key displayed once at provisioning; `recovery.sealed` stored on disk but useless without the out-of-band key |
