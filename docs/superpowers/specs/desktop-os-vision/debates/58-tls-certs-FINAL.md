# FINAL Spec: TLS & Certificate Store (Round 58)

**Subsystem**: TLS & Certificate Store  
**macOS Analogue**: `Security.framework` / Keychain cert store  
**Depends on**: R51 (networking), R52 (DNS), R59 (capability gate), R60 (Keychain for private keys)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Supervisor acts as the TLS broker. WASM apps cannot link native TLS libs. All certificate operations are pure-Rust.

```
supervisor/src/tls/
├── mod.rs        (~120 lines: init, IPC dispatch, capability gate (B2))
├── store.rs      (~180 lines: cert CRUD, R41 transactional writes, B1 scope validation)
├── validate.rs   (~140 lines: chain validation via webpki, SPKI pinning, CRL check (B4))
├── pinning.rs    (~80 lines: per-app pin registry, fingerprint match)
├── selfgen.rs    (~120 lines: rcgen self-signed cert generation, no key in reply (B3))
└── revocation.rs (~100 lines: CRL DER cache, is_revoked(), R41 writes)

/data/.vyoma/certs/
  roots/      — user-installed CA PEMs
  user/       — user leaf/intermediate certs
  mtls/       — per-app client cert + key pairs (keys never sent over IPC — B3)
  pins/       — per-app SPKI pin files
  crl/        — cached CRL DER files
```

No separate system root bundle file needed — `webpki-roots` compiled into binary (B5 fix).

---

## 2. Manifest Extension

```toml
[capabilities]
network = true
tls = true

[capabilities.tls]
pins = ["sha256//AAAA...base64...="]   # SPKI SHA-256 pins
mtls_cert = "my-app-client"            # name in /data/.vyoma/certs/mtls/
min_tls_version = "1.2"
```

```rust
// supervisor/src/manifest.rs — additions
#[serde(default)]
pub tls: bool,
#[serde(default)]
pub tls_config: Option<TlsCapability>,

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TlsCapability {
    #[serde(default)]
    pub pins: Vec<String>,
    #[serde(default)]
    pub mtls_cert: Option<String>,
    #[serde(default = "default_min_tls")]
    pub min_tls_version: String,
}
fn default_min_tls() -> String { "1.2".to_string() }
```

---

## 3. Store — R41 Transactional Write + Path Validation (B1 Fix)

```rust
// supervisor/src/tls/store.rs
const VALID_SCOPES: &[&str] = &["roots", "user", "mtls", "pins", "crl"];

fn validate_scope(scope: &str) -> Result<(), String> {
    if !VALID_SCOPES.contains(&scope) {
        return Err(format!("invalid scope '{scope}': must be one of {VALID_SCOPES:?}"));
    }
    Ok(())
}

fn validate_alias(alias: &str) -> Result<(), String> {
    if alias.is_empty() || alias.len() > 128 {
        return Err("alias must be 1–128 chars".into());
    }
    if alias.chars().any(|c| matches!(c, '/' | '\\' | '\0')) || alias.starts_with("..") {
        return Err("alias contains illegal characters".into());
    }
    Ok(())
}

pub fn write_atomic(&self, scope: &str, alias: &str, data: &[u8]) -> Result<(), String> {
    validate_scope(scope)?;
    validate_alias(alias)?;
    let dest = self.base.join(scope).join(alias);
    // Canonical check: dest must stay inside base
    let canonical_base = self.base.canonicalize()
        .map_err(|e| format!("canonicalize base: {e}"))?;
    let canonical_parent = dest.parent()
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_else(|| dest.parent().unwrap().to_path_buf());
    if !canonical_parent.starts_with(&canonical_base) {
        return Err(format!("path traversal detected: {} escapes cert store", dest.display()));
    }
    let tmp = dest.with_extension("tmp");
    fs::write(&tmp, data).map_err(|e| format!("write tmp: {e}"))?;
    {
        let f = fs::File::open(&tmp).map_err(|e| format!("open tmp: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync: {e}"))?;
    }
    fs::rename(&tmp, &dest).map_err(|e| format!("rename: {e}"))
}
```

---

## 4. Chain Validation + CRL Integration (B4 Fix)

```rust
// supervisor/src/tls/validate.rs

pub fn verify_chain(
    leaf_der:      &[u8],
    intermediates: &[Vec<u8>],
    store:         &CertStore,
    app_pins:      Option<&[String]>,
    crl_cache:     &CrlCache,   // B4: always passed, never skipped
) -> VerifyResult {
    let fp = spki_fingerprint(leaf_der);

    // Pin check first — fail fast
    if let Some(pins) = app_pins {
        if !pins.is_empty() && !pins.iter().any(|p| pin_matches(p, &fp)) {
            return VerifyResult { ok: false, fingerprint: fp,
                error: Some("pin mismatch".into()) };
        }
    }

    // B5 fix: use compiled-in webpki-roots; never zero trust anchors
    let anchors = build_trust_anchors(store);
    if anchors.is_empty() {
        return VerifyResult { ok: false, fingerprint: fp,
            error: Some("no trust anchors".into()) };
    }

    let inter_slices: Vec<&[u8]> = intermediates.iter().map(|v| v.as_slice()).collect();
    let cert = match EndEntityCert::try_from(leaf_der) {
        Ok(c) => c,
        Err(e) => return VerifyResult { ok: false, fingerprint: fp,
            error: Some(format!("parse: {e:?}")) },
    };
    let now = webpki::Time::try_from(std::time::SystemTime::now())
        .unwrap_or_else(|_| Time::from_seconds_since_unix_epoch(1_700_000_000));
    match cert.verify_is_valid_tls_server_cert(
        webpki::ALL_VERIFICATION_ALGS,
        &webpki::TlsServerTrustAnchors(&anchors),
        &inter_slices, now,
    ) {
        Ok(()) => {
            // B4 fix: check CRL AFTER chain validation succeeds
            let serial = extract_serial_hex(leaf_der).unwrap_or_default();
            if crl_cache.is_revoked(&serial) {
                return VerifyResult { ok: false, fingerprint: fp,
                    error: Some(format!("cert serial {serial} revoked (CRL)")) };
            }
            VerifyResult { ok: true, fingerprint: fp, error: None }
        }
        Err(e) => VerifyResult { ok: false, fingerprint: fp,
            error: Some(format!("{e:?}")) },
    }
}
```

```rust
// B5 fix: compiled-in Mozilla bundle as authoritative baseline
use webpki_roots::TLS_SERVER_ROOTS;

pub fn build_trust_anchors(store: &CertStore) -> Vec<webpki::TrustAnchor<'static>> {
    // Always start from compiled-in bundle — never zero anchors
    let mut anchors: Vec<webpki::TrustAnchor<'static>> =
        TLS_SERVER_ROOTS.iter().cloned().collect();
    // Augment with user-installed roots
    for der in store.load_user_roots() {
        let leaked: &'static [u8] = Box::leak(der.into_boxed_slice());
        if let Ok(anchor) = webpki::TrustAnchor::try_from_cert_der(leaked) {
            anchors.push(anchor);
        }
    }
    anchors
}
```

---

## 5. Self-Signed Cert — Key Never Leaves Supervisor (B3 Fix)

```rust
// supervisor/src/tls/selfgen.rs
pub fn generate_and_store(store: &CertStore, common_name: &str, sans: &[&str])
    -> Result<String, String>   // returns ONLY cert PEM — no key
{
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    params.distinguished_name = dn;
    params.subject_alt_names = sans.iter().map(|san| {
        if san.parse::<std::net::IpAddr>().is_ok() {
            SanType::IpAddress(san.parse().unwrap())
        } else {
            SanType::DnsName(san.to_string().try_into().unwrap())
        }
    }).collect();
    let key_pair = KeyPair::generate().map_err(|e| format!("keygen: {e}"))?;
    let cert = params.self_signed(&key_pair).map_err(|e| format!("sign: {e}"))?;

    // Store BOTH cert and key inside supervisor — key NEVER returned over IPC
    store.write_atomic("mtls", &format!("{common_name}.pem"), cert.pem().as_bytes())?;
    store.write_atomic("mtls", &format!("{common_name}.key"), key_pair.serialize_pem().as_bytes())?;

    Ok(cert.pem())  // return only cert (public material)
}
```

mTLS connections are performed supervisor-side via `tls-connect-mtls <host:port> <cert-name>`:
- Supervisor loads key from store
- Performs `rustls::ClientConnection` with the client cert
- Returns a `conn_id` to the app
- App uses `tls-send <id> <data>` / `tls-recv <id>` (like tcp-connect model)
- Raw private key bytes never cross the IPC boundary

---

## 6. Capability Gate (B2 Fix)

```rust
// AppState — add:
pub has_tls: bool,
pub tls_config: Option<TlsCapability>,

// In spawn_app:
has_tls: manifest.capabilities.tls,
tls_config: manifest.capabilities.tls_config.clone(),

// In ipc_handlers.rs dispatch — gate before calling tls::handle_tls_command:
"tls-verify-cert" | "tls-status" | "cert-add" | "cert-remove"
| "cert-list" | "cert-selfgen" | "cert-crl-refresh" | "cert-get-mtls"
| "tls-connect-mtls" => {
    let has_tls = {
        let reg = app_registry.lock().unwrap();
        reg.get(sender).map(|st| st.lock().unwrap().has_tls).unwrap_or(false)
    }; // lock dropped — ABBA safe
    if !has_tls {
        send_reply(sender, "REPLY:error: tls capability not declared in manifest", inbox);
        return true;
    }
    let tls_cap = /* snapshot from AppState */ None;
    // Also snapshot CRL cache before calling verify_chain (ABBA prevention)
    crate::tls::handle_tls_command(verb, parts, sender, tls_cap, inbox);
}
```

---

## 7. Protocol

```
@supervisor: tls-verify-cert <base64-leaf-der> [<base64-inter-der>...]
→ REPLY:tls-verify-cert ok <fingerprint-hex>
→ REPLY:tls-verify-cert error <reason>

@supervisor: cert-add <scope> <alias> <base64-pem>
→ REPLY:cert-add ok <alias>

@supervisor: cert-remove <scope> <alias>
→ REPLY:cert-remove ok

@supervisor: cert-list <scope>
→ REPLY:cert-list <alias>|<subject>|<expiry> || ...

@supervisor: cert-selfgen <common-name> [<san1,san2,...>]
→ REPLY:cert-selfgen ok <base64-cert-pem>    (no key — stored supervisor-side only)

@supervisor: tls-connect-mtls <host:port> <cert-name>
→ REPLY:tls-connect-mtls <conn_id>

@supervisor: cert-crl-refresh <issuer-alias>
→ REPLY:cert-crl-refresh queued

@supervisor: tls-status
→ REPLY:tls-status roots=<n> user=<n> pins=<n> crl_cached=<n>
```

---

## 8. Cargo Additions

```toml
webpki          = { version = "0.22", default-features = false }
webpki-roots    = "0.26"
rustls-pemfile  = "2"
x509-parser     = { version = "0.16", default-features = false, features = ["verify"] }
rcgen           = { version = "0.13", default-features = false }
sha2            = "0.10"
hex             = { version = "0.4", default-features = false }
base64          = { version = "0.22", default-features = false, features = ["alloc"] }
```

All pure-Rust, no C dependencies, musl-compatible.

---

## 9. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `cert-add` scope parameter is attacker-controlled path traversal — silent rogue CA injection | `validate_scope()` allowlist `["roots","user","mtls","pins","crl"]`; `validate_alias()` rejects separators; canonical path check in `write_atomic` |
| B2: `tls` capability bit never enforced at runtime — any app can call `tls-verify-cert` | Add `has_tls: bool` to `AppState`; gate all `tls-*`/`cert-*` commands before dispatch |
| B3: `cert-selfgen` returns private key over IPC pipe — key appears in app logs and log subscriber tails | `selfgen` stores key in supervisor's `mtls/` store, returns only cert PEM; mTLS done supervisor-side via `tls-connect-mtls` |
| B4: `CrlCache` populated but `verify_chain` never calls `is_revoked` — revoked certs always return ok | Pass `crl_cache` into `verify_chain`, check after chain validation succeeds; snapshot CRL under lock before I/O |
| B5: `load_trust_anchors` silently returns empty on missing system bundle — every cert fails as "no trust anchors" | Use `webpki-roots` compiled-in Mozilla bundle as authoritative baseline; user store is additive only; system PEM file removed from rootfs |
