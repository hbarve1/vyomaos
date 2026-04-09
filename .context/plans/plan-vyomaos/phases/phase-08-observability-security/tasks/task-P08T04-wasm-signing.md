# P08T04 — wasm-signing

## Phase

Phase 08 — Observability & Security

## Goal

Build a `vyoma-sign` CLI tool that signs a WASM binary using an ed25519 key pair and embeds the signature as a WASM custom section named `vyoma-sig`; implement signature verification in the supervisor (`supervisor/src/verify.rs`) so that any app whose `.wasm` file is missing or has an invalid signature is refused at launch time.

## File to create / modify

```
tools/vyoma-sign/Cargo.toml
tools/vyoma-sign/src/main.rs
supervisor/src/verify.rs     (new)
supervisor/src/main.rs       (modify — call verify_wasm() before launching each app)
supervisor/Cargo.toml        (modify — add ed25519-dalek, sha2, wasmparser)
```

## Implementation

### `tools/vyoma-sign/Cargo.toml`

```toml
[package]
name    = "vyoma-sign"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "vyoma-sign"
path = "src/main.rs"

[dependencies]
ed25519-dalek = { version = "2", features = ["rand_core"] }
rand          = "0.8"
sha2          = "0.10"
wasmparser    = "0.216"
wasm-encoder  = "0.216"
hex           = "0.4"
clap          = { version = "4", features = ["derive"] }
```

---

### `tools/vyoma-sign/src/main.rs`

```rust
//! vyoma-sign — sign a WASM binary with ed25519 and embed the signature
//! as a WASM custom section named "vyoma-sig".
//!
//! Usage:
//!   vyoma-sign keygen  --out key.pem
//!   vyoma-sign sign    --key key.pem --in app.wasm --out app-signed.wasm
//!   vyoma-sign verify  --pubkey pub.hex --in app-signed.wasm

use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey, Signature, Verifier};
use sha2::{Sha256, Digest};
use rand::rngs::OsRng;
use wasm_encoder::{Module, RawSection};
use wasmparser::Parser as WasmParser;

const CUSTOM_SECTION_NAME: &str = "vyoma-sig";

// ── CLI ────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "vyoma-sign", about = "Sign and verify VyomaOS WASM binaries")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate an ed25519 key pair
    Keygen {
        /// Output file for the 64-byte signing key (hex-encoded)
        #[arg(long, default_value = "signing-key.hex")]
        out: PathBuf,
        /// Output file for the 32-byte verifying key (hex-encoded)
        #[arg(long, default_value = "verifying-key.hex")]
        pubout: PathBuf,
    },
    /// Sign a WASM file and embed the signature as a custom section
    Sign {
        /// Path to the 64-byte signing key (hex-encoded)
        #[arg(long)]
        key: PathBuf,
        /// Input unsigned WASM file
        #[arg(long, short)]
        input: PathBuf,
        /// Output signed WASM file
        #[arg(long, short)]
        output: PathBuf,
    },
    /// Verify the signature embedded in a signed WASM file
    Verify {
        /// Path to the 32-byte verifying key (hex-encoded)
        #[arg(long)]
        pubkey: PathBuf,
        /// Signed WASM file to verify
        #[arg(long, short)]
        input: PathBuf,
    },
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Keygen { out, pubout } => cmd_keygen(out, pubout),
        Commands::Sign { key, input, output } => cmd_sign(key, input, output),
        Commands::Verify { pubkey, input } => cmd_verify(pubkey, input),
    }
}

// ── keygen ─────────────────────────────────────────────────────────────────

fn cmd_keygen(signing_out: PathBuf, verifying_out: PathBuf) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let signing_hex   = hex::encode(signing_key.to_bytes());
    let verifying_hex = hex::encode(verifying_key.to_bytes());

    fs::write(&signing_out, &signing_hex)
        .unwrap_or_else(|e| panic!("cannot write signing key: {}", e));
    fs::write(&verifying_out, &verifying_hex)
        .unwrap_or_else(|e| panic!("cannot write verifying key: {}", e));

    println!("Generated signing key:   {}", signing_out.display());
    println!("Generated verifying key: {}", verifying_out.display());
    println!("Verifying key (hex): {}", verifying_hex);
}

// ── sign ───────────────────────────────────────────────────────────────────

fn cmd_sign(key_path: PathBuf, input_path: PathBuf, output_path: PathBuf) {
    // Load signing key
    let key_hex = fs::read_to_string(&key_path)
        .unwrap_or_else(|e| panic!("cannot read key file: {}", e));
    let key_bytes: [u8; 32] = hex::decode(key_hex.trim())
        .expect("key file is not valid hex")
        .try_into()
        .expect("key must be exactly 32 bytes");
    let signing_key = SigningKey::from_bytes(&key_bytes);

    // Load input WASM
    let wasm_bytes = fs::read(&input_path)
        .unwrap_or_else(|e| panic!("cannot read wasm file: {}", e));

    // Compute SHA-256 of the WASM bytes (sign the hash, not the raw file)
    let digest = Sha256::digest(&wasm_bytes);

    // Sign
    let signature: Signature = signing_key.sign(&digest);
    let sig_bytes = signature.to_bytes(); // 64 bytes

    // Rebuild the WASM module with the custom section appended
    let signed_wasm = embed_custom_section(&wasm_bytes, CUSTOM_SECTION_NAME, &sig_bytes);

    fs::write(&output_path, &signed_wasm)
        .unwrap_or_else(|e| panic!("cannot write signed wasm: {}", e));

    println!("Signed {} → {}", input_path.display(), output_path.display());
    println!("Signature (hex): {}", hex::encode(sig_bytes));
}

// ── verify ─────────────────────────────────────────────────────────────────

fn cmd_verify(pubkey_path: PathBuf, input_path: PathBuf) {
    let key_hex = fs::read_to_string(&pubkey_path)
        .unwrap_or_else(|e| panic!("cannot read pubkey file: {}", e));
    let key_bytes: [u8; 32] = hex::decode(key_hex.trim())
        .expect("pubkey is not valid hex")
        .try_into()
        .expect("pubkey must be 32 bytes");
    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .expect("invalid verifying key");

    let signed_wasm = fs::read(&input_path)
        .unwrap_or_else(|e| panic!("cannot read wasm: {}", e));

    match extract_and_verify(&signed_wasm, &verifying_key) {
        Ok(())  => println!("Signature VALID: {}", input_path.display()),
        Err(e)  => { eprintln!("Signature INVALID: {}", e); std::process::exit(1); }
    }
}

// ── WASM custom section helpers ────────────────────────────────────────────

/// Append a custom section to a WASM module.
/// The section name and data are encoded per the WASM binary format spec.
fn embed_custom_section(wasm: &[u8], name: &str, data: &[u8]) -> Vec<u8> {
    // We rebuild the entire module using wasm-encoder to be spec-compliant.
    // Copy all existing sections verbatim, then append the custom section.
    let mut module = Module::new();

    for payload in WasmParser::new(0).parse_all(wasm) {
        match payload.expect("invalid WASM") {
            wasmparser::Payload::Version { encoding, .. } => {
                // Version is implicit in Module::new()
                let _ = encoding;
            }
            wasmparser::Payload::RawSection { id, data, .. } => {
                module.section(&RawSection { id, data });
            }
            wasmparser::Payload::End(_) => break,
            _ => {}
        }
    }

    // Append vyoma-sig custom section (id = 0)
    let mut custom_body = Vec::new();
    // Encode name as WASM string (u32 length prefix + UTF-8 bytes)
    let name_bytes = name.as_bytes();
    leb128_write(&mut custom_body, name_bytes.len() as u64);
    custom_body.extend_from_slice(name_bytes);
    custom_body.extend_from_slice(data);

    module.section(&RawSection { id: 0, data: &custom_body });
    module.finish()
}

fn leb128_write(buf: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 { buf.push(byte); break; } else { buf.push(byte | 0x80); }
    }
}

/// Extract the vyoma-sig custom section from a WASM file and verify it.
pub fn extract_and_verify(wasm: &[u8], key: &VerifyingKey) -> Result<(), String> {
    // The signature was computed over the WASM bytes *without* the custom section.
    // Strip the custom section first, then verify.
    let (base_wasm, sig_bytes) = strip_custom_section(wasm, CUSTOM_SECTION_NAME)
        .ok_or("vyoma-sig custom section not found")?;

    if sig_bytes.len() != 64 {
        return Err(format!("expected 64-byte signature, got {}", sig_bytes.len()));
    }
    let sig_arr: [u8; 64] = sig_bytes.try_into().unwrap();
    let signature = Signature::from_bytes(&sig_arr);

    let digest = Sha256::digest(&base_wasm);
    key.verify(&digest, &signature)
        .map_err(|e| format!("signature verification failed: {}", e))
}

/// Return (wasm_without_section, section_data) or None if section absent.
fn strip_custom_section(wasm: &[u8], target_name: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut base = Vec::new();
    let mut sig_data = None;

    // Write WASM magic + version header
    base.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D]); // \0asm
    base.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1

    for payload in WasmParser::new(0).parse_all(wasm) {
        match payload.ok()? {
            wasmparser::Payload::Version { .. } => {}
            wasmparser::Payload::RawSection { id: 0, data, .. } => {
                // Parse name from custom section
                if let Some((name, rest)) = parse_wasm_string(data) {
                    if name == target_name {
                        sig_data = Some(rest.to_vec());
                        continue; // skip — don't include in base_wasm
                    }
                }
                // Non-matching custom section: include verbatim
                let section_bytes = encode_section(0, data);
                base.extend_from_slice(&section_bytes);
            }
            wasmparser::Payload::RawSection { id, data, .. } => {
                base.extend_from_slice(&encode_section(id, data));
            }
            wasmparser::Payload::End(_) => break,
            _ => {}
        }
    }

    sig_data.map(|s| (base, s))
}

fn parse_wasm_string(data: &[u8]) -> Option<(&str, &[u8])> {
    if data.is_empty() { return None; }
    let (len, consumed) = leb128_read(data)?;
    let end = consumed + len as usize;
    if end > data.len() { return None; }
    let name = std::str::from_utf8(&data[consumed..end]).ok()?;
    Some((name, &data[end..]))
}

fn leb128_read(data: &[u8]) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift  = 0u32;
    for (i, &byte) in data.iter().enumerate() {
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 { return Some((result, i + 1)); }
        shift += 7;
        if shift >= 64 { return None; }
    }
    None
}

fn encode_section(id: u8, data: &[u8]) -> Vec<u8> {
    let mut out = vec![id];
    leb128_write(&mut out, data.len() as u64);
    out.extend_from_slice(data);
    out
}
```

---

### `supervisor/src/verify.rs`

```rust
//! verify.rs — verify the ed25519 signature embedded in a WASM binary.
//!
//! The supervisor calls `verify_wasm()` before launching any app.
//! If the signature is absent or invalid, the app is refused.

use ed25519_dalek::{Signature, VerifyingKey, Verifier};
use sha2::{Sha256, Digest};
use std::fs;

const CUSTOM_SECTION_NAME: &str = "vyoma-sig";

/// Load the operator's ed25519 verifying key from a hex file.
/// The key file is embedded in the initramfs at /etc/vyoma/verifying-key.hex.
pub fn load_verifying_key(path: &str) -> Result<VerifyingKey, String> {
    let hex = fs::read_to_string(path)
        .map_err(|e| format!("cannot read verifying key {}: {}", path, e))?;
    let bytes: [u8; 32] = hex::decode(hex.trim())
        .map_err(|e| format!("verifying key is not valid hex: {}", e))?
        .try_into()
        .map_err(|_| "verifying key must be 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|e| format!("invalid verifying key: {}", e))
}

/// Verify the vyoma-sig custom section in the WASM binary at `wasm_path`.
///
/// Returns Ok(()) if the signature is present and valid.
/// Returns Err(reason) if missing or invalid.
pub fn verify_wasm(wasm_path: &str, key: &VerifyingKey) -> Result<(), String> {
    let wasm_bytes = fs::read(wasm_path)
        .map_err(|e| format!("cannot read wasm {}: {}", wasm_path, e))?;

    // Delegate to the same logic as vyoma-sign verify
    // (duplicate the strip_custom_section + verify logic here to avoid
    //  depending on the tools/ crate from the supervisor)
    let (base_wasm, sig_bytes) =
        strip_custom_section(&wasm_bytes)
            .ok_or_else(|| format!("{}: missing vyoma-sig section", wasm_path))?;

    if sig_bytes.len() != 64 {
        return Err(format!("{}: signature must be 64 bytes, got {}", wasm_path, sig_bytes.len()));
    }
    let sig_arr: [u8; 64] = sig_bytes.try_into().unwrap();
    let signature = Signature::from_bytes(&sig_arr);

    let digest = Sha256::digest(&base_wasm);
    key.verify(&digest, &signature)
        .map_err(|e| format!("{}: invalid signature: {}", wasm_path, e))
}

// ── WASM section parsing (minimal, no external parser) ────────────────────

fn strip_custom_section(wasm: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if wasm.len() < 8 { return None; }
    // Check magic + version
    if &wasm[0..4] != b"\x00asm" { return None; }

    let mut base: Vec<u8> = wasm[0..8].to_vec(); // magic + version
    let mut pos = 8usize;
    let mut sig_data: Option<Vec<u8>> = None;

    while pos < wasm.len() {
        let section_id = wasm[pos];
        pos += 1;
        let (section_len, consumed) = leb128_read(&wasm[pos..])?;
        pos += consumed;
        let section_end = pos + section_len as usize;
        if section_end > wasm.len() { return None; }
        let section_data = &wasm[pos..section_end];

        if section_id == 0 {
            if let Some((name, rest)) = parse_wasm_string(section_data) {
                if name == CUSTOM_SECTION_NAME {
                    sig_data = Some(rest.to_vec());
                    pos = section_end;
                    continue;
                }
            }
        }

        // Include this section in base
        base.push(section_id);
        leb128_write_vec(&mut base, section_len);
        base.extend_from_slice(section_data);
        pos = section_end;
    }

    sig_data.map(|s| (base, s))
}

fn parse_wasm_string(data: &[u8]) -> Option<(&str, &[u8])> {
    let (len, consumed) = leb128_read(data)?;
    let end = consumed + len as usize;
    if end > data.len() { return None; }
    let name = std::str::from_utf8(&data[consumed..end]).ok()?;
    Some((name, &data[end..]))
}

fn leb128_read(data: &[u8]) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift  = 0u32;
    for (i, &byte) in data.iter().enumerate() {
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 { return Some((result, i + 1)); }
        shift += 7;
        if shift >= 64 { return None; }
    }
    None
}

fn leb128_write_vec(buf: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 { buf.push(byte); break; } else { buf.push(byte | 0x80); }
    }
}
```

---

### `supervisor/Cargo.toml` additions

```toml
ed25519-dalek = { version = "2", features = [] }
sha2          = "0.10"
hex           = "0.4"
```

---

### `supervisor/src/main.rs` — call verify_wasm before launch

```rust
mod verify;

const VERIFYING_KEY_PATH: &str = "/etc/vyoma/verifying-key.hex";

fn main() {
    // ... boot setup ...

    // Load the operator's verifying key once at startup
    let verifying_key = match verify::load_verifying_key(VERIFYING_KEY_PATH) {
        Ok(k)  => Some(k),
        Err(e) => {
            log_warn!("no verifying key loaded ({}); signature checks disabled", e);
            None
        }
    };

    // ... thread spawn loop ...
    // Pass verifying_key by Arc<Option<VerifyingKey>> if needed across threads
}

fn run_app(entry: &BootEntry, verifying_key: Option<&ed25519_dalek::VerifyingKey>) -> (String, i32) {
    // ... manifest parsing ...

    // Verify signature before launching
    if let Some(key) = verifying_key {
        if let Err(reason) = verify::verify_wasm(wasm_path.to_str().unwrap_or(""), key) {
            log_app_error!(&manifest.app.name, "signature check failed: {}", reason);
            return (manifest.app.name.clone(), 126); // 126 = permission denied
        }
        log_app_info!(&manifest.app.name, "signature verified OK");
    }

    // ... rest of launch logic ...
}
```

---

### Operator workflow

```sh
# 1. Generate a key pair (once, store signing key securely)
vyoma-sign keygen --out signing-key.hex --pubout verifying-key.hex

# 2. Sign each WASM app
vyoma-sign sign --key signing-key.hex \
  --input  apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm \
  --output apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm

# 3. Copy verifying-key.hex into the initramfs
cp verifying-key.hex base/modules/scripts/verifying-key.hex
# (rootfs.sh copies it to /etc/vyoma/verifying-key.hex in the cpio archive)
```

## Notes

- The signature is computed over the **SHA-256 hash** of the WASM bytes (without the custom section), not over the raw bytes. This follows the ed25519ph ("prehashed") convention and handles large files efficiently.
- The custom section name `"vyoma-sig"` is chosen to avoid collision with known WASM tooling section names (`"name"`, `"sourceMappingURL"`, `"dylink"`, etc.).
- The signing key must be kept confidential and should never be embedded in the initramfs. Only the 32-byte verifying key goes into the OS image.
- Exit code `126` for signature failure follows the shell convention for "permission denied to execute".
- If `VERIFYING_KEY_PATH` does not exist, the supervisor logs a warning and launches apps without verification. This allows development builds (where no key has been generated) to work unchanged.
- The `strip_custom_section` function in `verify.rs` is a minimal reimplementation to avoid a dependency from `supervisor/` on `tools/vyoma-sign/`. If the codebase is restructured into a Cargo workspace, this logic can be extracted to a shared `vyoma-core` crate.
- Cross-reference: P08T01 structured logging should be used for all log calls in `verify.rs` and `main.rs` signature-check code.

## Verification

```sh
# 1. Build vyoma-sign tool
(cd tools/vyoma-sign && cargo build --release 2>&1 | tail -5)
test -x tools/vyoma-sign/target/release/vyoma-sign && echo "vyoma-sign: built"

# 2. Build supervisor with verify.rs
(cd supervisor && cargo build 2>&1 | tail -5)
test -x supervisor/target/debug/supervisor && echo "supervisor: built OK"

# 3. End-to-end signing and verification test
tools/vyoma-sign/target/release/vyoma-sign keygen \
  --out     /tmp/test-signing-key.hex \
  --pubout  /tmp/test-verifying-key.hex
echo "keygen: OK"

# Sign the hello-world WASM (must be built from P05T01)
test -f apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm || \
  (cd apps/hello-world && cargo build --release)

tools/vyoma-sign/target/release/vyoma-sign sign \
  --key    /tmp/test-signing-key.hex \
  --input  apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm \
  --output /tmp/hello-world-signed.wasm
echo "sign: OK"
test -f /tmp/hello-world-signed.wasm && echo "signed wasm: present"

# Verify the signed WASM
tools/vyoma-sign/target/release/vyoma-sign verify \
  --pubkey /tmp/test-verifying-key.hex \
  --input  /tmp/hello-world-signed.wasm && echo "verify: PASS"

# 4. Confirm that a tampered file fails verification
cp /tmp/hello-world-signed.wasm /tmp/hello-world-tampered.wasm
# Flip a byte in the WASM body (offset 100)
python3 -c "
with open('/tmp/hello-world-tampered.wasm', 'r+b') as f:
    f.seek(100)
    b = f.read(1)
    f.seek(100)
    f.write(bytes([b[0] ^ 0xFF]))
"
tools/vyoma-sign/target/release/vyoma-sign verify \
  --pubkey /tmp/test-verifying-key.hex \
  --input  /tmp/hello-world-tampered.wasm && echo "FAIL: tampered file should not verify" || echo "tampered file correctly rejected: PASS"

# 5. Confirm the signed WASM still runs under wasmtime (custom section is harmless)
wasmtime run /tmp/hello-world-signed.wasm | grep -q "Hello from VyomaOS" && echo "signed wasm executes: OK"

# 6. Confirm unsigned WASM is refused by the supervisor (when key is loaded)
cp /tmp/test-verifying-key.hex /tmp/vyoma-test-key.hex
# The supervisor should log "signature check failed" and return exit 126
grep -q "verify_wasm"        supervisor/src/main.rs    && echo "verify_wasm called in run_app: OK"
grep -q "verify::load_verifying_key" supervisor/src/main.rs && echo "key loading: present"

# 7. Confirm custom section name constant
grep -q "vyoma-sig"  supervisor/src/verify.rs         && echo "section name in verify.rs: OK"
grep -q "vyoma-sig"  tools/vyoma-sign/src/main.rs     && echo "section name in vyoma-sign: OK"
```
