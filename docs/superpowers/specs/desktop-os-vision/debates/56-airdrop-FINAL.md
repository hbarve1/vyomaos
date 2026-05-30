# FINAL Spec: AirDrop & P2P File Transfer (Round 56)

**Subsystem**: AirDrop & P2P File Transfer  
**macOS Analogue**: `AirDrop` / `AWDL`  
**Depends on**: R51 (networking), R52 (mDNS), R49 (sandbox FS container dirs)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

Four new files under `supervisor/src/airdrop/` (all under 500 lines):

```
supervisor/src/airdrop/
├── mod.rs        (~80 lines: AirdropState, statics, init(), capability guard)
├── discovery.rs  (~120 lines: mDNS _vyoma-drop._tcp.local. advertise + peer scan)
├── transfer.rs   (~200 lines: TCP acceptor, framing, chunked send/recv, R41 writes)
├── auth.rs       (~80 lines: Ed25519 identity keypair, TOFU trust store (B5))
└── prompt.rs     (~80 lines: per-transfer-id accept/reject channel map (B1))
```

New capability bit in `supervisor/src/manifest.rs`:
```rust
#[serde(default)]
pub airdrop: bool,
```

Supervisor owns all sockets — WASM apps never see raw file descriptors.

---

## 2. Discovery — mDNS Integration (R52)

Supervisor advertises `_vyoma-drop._tcp.local.` on port 7474 and listens for peers announcing the same service. Runs in a dedicated background thread.

```rust
// supervisor/src/airdrop/discovery.rs
pub const AIRDROP_TCP_PORT: u16 = 7474;
pub const AIRDROP_SERVICE:  &str = "_vyoma-drop._tcp.local.";

#[derive(Clone, Debug)]
pub struct Peer {
    pub hostname:  String,
    pub addr:      String,   // "192.168.x.y:7474"
    pub last_seen: Instant,
}

static PEERS: OnceLock<Mutex<HashMap<String, Peer>>> = OnceLock::new();
pub fn peers() -> &'static Mutex<HashMap<String, Peer>> {
    PEERS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn start(our_hostname: &str) {
    let hostname = our_hostname.to_string();
    thread::spawn(move || {
        let sock = match UdpSocket::bind("0.0.0.0:5353") {
            Ok(s) => s,
            Err(e) => { eprintln!("[airdrop] mDNS bind: {e}"); return; }
        };
        let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
        let mc_addr: Ipv4Addr = "224.0.0.251".parse().unwrap();
        let _ = sock.join_multicast_v4(&mc_addr, &Ipv4Addr::UNSPECIFIED);

        let mut last_announce = Instant::now();
        let mut buf = [0u8; 1500];
        loop {
            if last_announce.elapsed() > Duration::from_secs(30) {
                let pkt = build_announce_packet(&hostname, AIRDROP_TCP_PORT);
                let _ = sock.send_to(&pkt, "224.0.0.251:5353");
                last_announce = Instant::now();
            }
            if let Ok((n, src)) = sock.recv_from(&mut buf) {
                if let Some(peer) = parse_announce_packet(&buf[..n], src) {
                    peers().lock().unwrap().insert(peer.hostname.clone(), peer);
                }
            }
            // Expire peers older than 90 s
            peers().lock().unwrap().retain(|_, p| p.last_seen.elapsed() < Duration::from_secs(90));
        }
    });
}
```

---

## 3. Wire Protocol

TCP framing on port 7474:

```
[ MAGIC 5B "YDROP" ] [ version 1B=0x01 ]
--- Auth handshake (B5) ---
[ their_pubkey 32B ] → [ our_pubkey 32B ] [ challenge 32B ]
← [ their_signature 64B ]
--- Transfer header ---
[ sender_name_len 1B ] [ sender_name ]
[ filename_len 2B LE ] [ filename UTF-8 ]
[ file_size 8B LE ]
--- ACK from receiver: 1B (0x01=accept, 0x00=reject) ---
[ chunks: 4B LE len + N bytes payload ]
[ SHA-256 checksum 32B ]
```

Hard cap: `MAX_FILE_SIZE = 512 MB`. Max concurrent transfers: 4 (semaphore — B2).

---

## 4. Concurrent Transfer Semaphore (B2 Fix)

```rust
// airdrop/mod.rs
pub static TRANSFER_SEM: OnceLock<Arc<Mutex<u8>>> = OnceLock::new();
const MAX_CONCURRENT_TRANSFERS: u8 = 4;

// In handle_incoming — acquire slot or reject immediately:
{
    let mut count = TRANSFER_SEM.get().unwrap().lock().unwrap();
    if *count >= MAX_CONCURRENT_TRANSFERS {
        let _ = stream.write_all(&[0x00]); // reject: server busy
        return;
    }
    *count += 1;
}
struct TransferGuard;
impl Drop for TransferGuard {
    fn drop(&mut self) {
        if let Some(sem) = TRANSFER_SEM.get() { *sem.lock().unwrap() -= 1; }
    }
}
let _guard = TransferGuard;
```

---

## 5. Safe Destination Path (B3 Fix)

```rust
// airdrop/transfer.rs
fn safe_dest_path(filename: &str) -> Option<PathBuf> {
    // Take only basename — strips all directory components
    let base = Path::new(filename).file_name()?.to_str()?;
    // Strip leading dots (hidden file prevention)
    let base = base.trim_start_matches('.');
    let base = if base.is_empty() { "file" } else { base };
    // Allow only ASCII alphanumeric + safe punctuation
    let base: String = base.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' })
        .collect();
    let dest_dir = PathBuf::from("/data/.vyoma/airdrop/");
    fs::create_dir_all(&dest_dir).ok()?;
    let candidate = dest_dir.join(&base);
    // Verify parent is exactly the landing zone
    if candidate.parent() != Some(dest_dir.as_path()) { return None; }
    Some(candidate)
}
```

Receive flow:
```rust
let dest_path = match safe_dest_path(&filename) {
    Some(p) => p,
    None => return, // path traversal rejected
};
let tmp_path = dest_path.with_extension("tmp");
// ... receive to tmp_path, fsync, R41 rename to dest_path
```

---

## 6. Chunked Send — No Full-File Buffering (B4 Fix)

```rust
// airdrop/transfer.rs
pub fn send_file(peer_addr: &str, our_name: &str, filepath: &Path, inbox: &Arc<Inbox>) -> Result<(), String> {
    let file_size = fs::metadata(filepath).map_err(|e| e.to_string())?.len();
    if file_size > MAX_FILE_SIZE {
        return Err(format!("file too large ({file_size} > {MAX_FILE_SIZE})"));
    }
    let mut file   = fs::File::open(filepath).map_err(|e| e.to_string())?;
    let mut stream = TcpStream::connect(peer_addr).map_err(|e| e.to_string())?;
    // ... send header, wait ACK ...
    // Stream in 64 KB chunks — NEVER buffer entire file
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];  // 64 KB
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        let chunk = &buf[..n];
        hasher.update(chunk);
        stream.write_all(&(n as u32).to_le_bytes()).map_err(|e| e.to_string())?;
        stream.write_all(chunk).map_err(|e| e.to_string())?;
    }
    let hash: [u8; 32] = hasher.finalize().into();
    stream.write_all(&hash).map_err(|e| e.to_string())?;
    Ok(())
}
```

Peak heap: 64 KB regardless of file size.

---

## 7. Per-Transfer-ID Accept/Reject (B1 Fix)

```rust
// airdrop/prompt.rs
type TransferId = u64;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PENDING: OnceLock<Mutex<HashMap<TransferId, mpsc::Sender<bool>>>> = OnceLock::new();

pub fn ask_user_accept(from: &str, filename: &str, size: u64, inbox: &Arc<Inbox>) -> bool {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel::<bool>();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
        .lock().unwrap().insert(id, tx);
    // UI message includes transfer-id for keyed accept/reject
    send_reply("airdrop-ui", &format!("AIRDROP:incoming:{}|{}|{}|{}", id, from, filename, size), inbox);
    let result = rx.recv_timeout(Duration::from_secs(30)).unwrap_or(false);
    PENDING.get().unwrap().lock().unwrap().remove(&id);
    result
}

pub fn handle_user_response(id: TransferId, accepted: bool) {
    if let Some(tx) = PENDING.get().unwrap().lock().unwrap().remove(&id) {
        let _ = tx.send(accepted);
    }
}
```

IPC verbs: `airdrop-accept <id>` and `airdrop-reject <id>` — UI app echoes the id back.

---

## 8. TOFU Authentication (B5 Fix)

Ed25519 keypair at `/data/.vyoma/airdrop/identity.key`. Trust store at `/data/.vyoma/airdrop/trusted_peers.toml`.

```rust
// airdrop/auth.rs — load or generate keypair
pub fn load_or_generate_keypair() -> ([u8; 64], [u8; 32]) {
    // Try to load from disk (R41 written on first use)
    if let (Ok(sk), Ok(vk)) = (fs::read(KEY_PATH), fs::read(PUBKEY_PATH)) {
        if sk.len() == 64 && vk.len() == 32 {
            let mut kb = ([0u8; 64], [0u8; 32]);
            kb.0.copy_from_slice(&sk); kb.1.copy_from_slice(&vk);
            return kb;
        }
    }
    // Generate from /dev/urandom entropy
    let mut seed = [0u8; 32];
    let _ = fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut seed));
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let vk: ed25519_dalek::VerifyingKey = (&sk).into();
    let sk_bytes = sk.to_keypair_bytes();
    let vk_bytes = vk.to_bytes();
    // R41 transactional write
    let _ = fs::write(format!("{KEY_PATH}.tmp"), &sk_bytes)
        .and_then(|_| fs::rename(format!("{KEY_PATH}.tmp"), KEY_PATH));
    let _ = fs::write(format!("{PUBKEY_PATH}.tmp"), &vk_bytes)
        .and_then(|_| fs::rename(format!("{PUBKEY_PATH}.tmp"), PUBKEY_PATH));
    (sk_bytes, vk_bytes)
}
```

Handshake in `handle_incoming`: exchange pubkeys → receiver sends 32-byte challenge → sender signs → receiver verifies. Unknown pubkey → UI shows 8-byte hex fingerprint for user confirmation. Trusted pubkeys skip re-prompt on subsequent transfers.

```toml
# /data/.vyoma/airdrop/trusted_peers.toml
[[peer]]
pubkey_hex = "a1b2c3d4e5f6a7b8"
label      = "Alice's VyomaOS"
```

---

## 9. Protocol

```
@supervisor: airdrop-send <peer_hostname> <filepath>
→ REPLY:airdrop-send:queued  (immediate)
→ REPLY:airdrop-send:ok  OR  REPLY:airdrop-send:error:<reason>  (async)

@supervisor: airdrop-peers
→ REPLY:airdrop-peers:<hostname1>,<hostname2>,...

@supervisor: airdrop-accept <id>
@supervisor: airdrop-reject <id>

AIRDROP:incoming:<id>|<from>|<filename>|<size_bytes>
AIRDROP:received:/data/.vyoma/airdrop/<filename>
AIRDROP:error:<reason>
```

Capability gate: `airdrop: bool` in `Capabilities`. File path for `airdrop-send` must be within `/data/` (canonicalize check).

---

## 10. Cargo.toml Additions

```toml
[dependencies]
ed25519-dalek = { version = "2", features = ["rand_core"] }
sha2          = "0.10"
hex           = "0.4"
```

---

## 11. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Single `PENDING` slot silently drops concurrent incoming transfers when two arrive simultaneously | `TransferId`-keyed `HashMap<u64, mpsc::Sender<bool>>`; UI message includes id; `airdrop-accept <id>` keyed response |
| B2: Unbounded `thread::spawn` per connection exhausts thread stack memory (DoS) | Semaphore cap of 4 concurrent transfers; immediate TCP reject with `0x00` when full |
| B3: `sanitize_filename` strips `/` but not unicode look-alikes; embedded traversal sequences after strip could escape landing zone | Use `Path::file_name()` to extract basename only, then allow-list chars; verify `candidate.parent() == dest_dir` |
| B4: `send_file` could buffer 512 MB into heap Vec, triggering OOM-killer on PID 1 | Stream in 64 KB chunks via `file.read(&mut buf)` loop; peak heap bounded to `CHUNK_SIZE` |
| B5: No authentication — any LAN device can impersonate any peer name | Ed25519 TOFU: per-device keypair at `/data/.vyoma/airdrop/identity.key`; challenge-response in handshake; trusted_peers.toml |
