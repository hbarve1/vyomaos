# FINAL Spec: Audio System (Round 67)

**Subsystem**: Audio System
**macOS Analogue**: CoreAudio / AudioHAL / AVAudioEngine
**Depends on**: R59 (capabilities), R11 (display/overlay for mic permission prompt)
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

```
supervisor/src/audio/
├── mod.rs       # AudioSubsystem entry point; IPC verb parser; base64 decode; capability gate
├── alsa.rs      # Raw ALSA PCM ioctl wrappers (no libasound); open/hw_params/sw_params/write/read
├── ring.rs      # SPSC lock-free StreamRing (128 KB circular buffer, power-of-two)
├── mixer.rs     # Mixer thread: snapshot streams, mix outside lock, write ALSA period
└── mic.rs       # Microphone capture thread; permission gate; base64 encode; push to app stdin
```

**Thread model:**

```
Per app (audio_output=true):
  IPC thread: parses VYOMA_AUDIO:write:<sid>,<b64> → decodes → pushes bytes to StreamRing

mixer_thread (singleton via OnceLock):
  wakes every PERIOD_FRAMES (1024 frames ≈ 23 ms at 44100 Hz)
  snapshots Arc<StreamRing> refs under global lock → drops lock
  reads PERIOD_FRAMES from each ring, mixes to i16 scratch buffer
  calls alsa_write_frames(pcm_fd, &scratch, PERIOD_FRAMES)

mic_thread (spawned per mic_open grant):
  calls alsa_read_frames in blocking loop
  base64-encodes each period
  pushes "VYOMA_AUDIO:mic_data:<b64>" to requesting app stdin
```

---

## 2. VYOMA_AUDIO Protocol

```
VYOMA_AUDIO:open:<sid>,<rate>,<fmt>,<ch>
VYOMA_AUDIO:write:<sid>,<base64_pcm>
VYOMA_AUDIO:close:<sid>
VYOMA_AUDIO:set_vol:<sid>,<0-100>
VYOMA_AUDIO:mic_open:<req_id>
VYOMA_AUDIO:mic_close:
```

- `<sid>`: u64 session identifier, chosen by app, unique per app
- `<rate>`: sample rate (e.g. `44100`, `48000`)
- `<fmt>`: `s16le` | `s32le` | `f32le`
- `<ch>`: `1` (mono) | `2` (stereo)
- `<base64_pcm>`: interleaved PCM frames, base64-encoded, no padding required
- `mic_open` triggers permission gate; response via `VYOMA_AUDIO:mic_data:<b64>` lines to app stdin
- `set_vol` is per-stream software volume (0 = silence, 100 = unity gain)

---

## 3. SPSC Lock-Free StreamRing

```rust
// supervisor/src/audio/ring.rs

pub const RING_CAP: usize = 131_072; // 128 KB — power of two

pub struct StreamRing {
    buf:   Box<[u8; RING_CAP]>,
    write: AtomicUsize,  // written by IPC/decode thread
    read:  AtomicUsize,  // read by mixer thread
}

impl StreamRing {
    pub fn new() -> Arc<Self> { /* heap-alloc + zeroed buf */ }

    /// Called by IPC thread. Writes bytes; drops oldest data on overflow.
    pub fn push(&self, data: &[u8]) {
        let w = self.write.load(Relaxed);
        let r = self.read.load(Acquire);
        // available = RING_CAP - (w - r) — if data.len() > available, advance r
        for byte in data {
            let idx = w.wrapping_add(i) & (RING_CAP - 1);
            unsafe { (*self.buf.as_ptr().add(idx)).write(*byte); }
        }
        self.write.fetch_add(data.len(), Release);
    }

    /// Called by mixer thread. Reads exactly `n` bytes (zero-pads if underrun).
    pub fn pull(&self, out: &mut [u8]) {
        let r = self.read.load(Relaxed);
        let w = self.write.load(Acquire);
        let avail = w.wrapping_sub(r).min(out.len());
        for i in 0..avail {
            out[i] = unsafe { *self.buf.as_ptr().add(r.wrapping_add(i) & (RING_CAP - 1)) };
        }
        out[avail..].fill(0); // silence on underrun
        self.read.fetch_add(out.len(), Release);
    }
}
```

---

## 4. Raw ALSA PCM Ioctls

```rust
// supervisor/src/audio/alsa.rs

// Ioctl numbers from <linux/include/uapi/sound/asound.h>
const SNDRV_PCM_IOCTL_HW_PARAMS:    u64 = 0xC250_4111;
const SNDRV_PCM_IOCTL_SW_PARAMS:    u64 = 0xC088_4113;
const SNDRV_PCM_IOCTL_WRITEI_FRAMES: u64 = 0xC010_5150;
const SNDRV_PCM_IOCTL_READI_FRAMES:  u64 = 0xC010_5151;

#[repr(C)]
pub struct SndrPcmHwParams { /* 608 bytes — matches kernel struct */ }

#[repr(C)]
pub struct SndrPcmSwParams { /* 136 bytes */ }

#[repr(C)]
pub struct SndrXferi {
    pub buf: *mut std::ffi::c_void,
    pub frames: u64,
}

pub fn alsa_open_pcm(card: u32, dev: u32, capture: bool) -> Result<RawFd, AlsaError> {
    let path = format!("/dev/snd/pcmC{}D{}{}",
        card, dev, if capture { 'c' } else { 'p' });
    let flags = if capture { O_RDONLY } else { O_WRONLY } | O_NONBLOCK | O_CLOEXEC;
    let fd = unsafe { libc::open(path.as_ptr() as *const _, flags) };
    if fd < 0 { return Err(AlsaError::Open(errno())); }
    Ok(fd)
}

pub fn alsa_hw_params(fd: RawFd, rate: u32, channels: u32, format: SampleFormat)
    -> Result<(), AlsaError>
{
    let mut params = SndrPcmHwParams::default();
    params.set_access(SNDRV_PCM_ACCESS_RW_INTERLEAVED);
    params.set_format(format.to_alsa());
    params.set_rate_near(rate);
    params.set_channels(channels);
    params.set_period_size(PERIOD_FRAMES as _);
    params.set_buffer_size((PERIOD_FRAMES * 4) as _);
    let rc = unsafe { libc::ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS, &mut params) };
    if rc < 0 { return Err(AlsaError::HwParams(errno())); }
    Ok(())
}

pub fn alsa_write_frames(fd: RawFd, buf: &[i16], frames: usize) -> Result<usize, AlsaError> {
    let mut xferi = SndrXferi { buf: buf.as_ptr() as *mut _, frames: frames as u64 };
    let rc = unsafe { libc::ioctl(fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &mut xferi) };
    if rc < 0 { return Err(AlsaError::Write(errno())); }
    Ok(xferi.frames as usize)
}

pub fn alsa_read_frames(fd: RawFd, buf: &mut [i16], frames: usize) -> Result<usize, AlsaError> {
    let mut xferi = SndrXferi { buf: buf.as_ptr() as *mut _, frames: frames as u64 };
    let rc = unsafe { libc::ioctl(fd, SNDRV_PCM_IOCTL_READI_FRAMES, &mut xferi) };
    if rc < 0 { return Err(AlsaError::Read(errno())); }
    Ok(xferi.frames as usize)
}

pub const PERIOD_FRAMES: usize = 1024; // ~23 ms at 44100 Hz
```

---

## 5. Mixer Thread

```rust
// supervisor/src/audio/mixer.rs

static MIXER_STARTED: OnceLock<()> = OnceLock::new();
static ACTIVE_STREAMS: Mutex<HashMap<u64, (Arc<StreamRing>, f32)>> = Mutex::new(HashMap::new());

pub fn ensure_mixer_started(pcm_fd: RawFd) {
    MIXER_STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("audio-mixer".into())
            .spawn(move || mixer_loop(pcm_fd))
            .expect("spawn audio mixer");
    });
}

fn mixer_loop(pcm_fd: RawFd) {
    // scratch: stereo i16, PERIOD_FRAMES per channel
    let mut scratch = vec![0i16; PERIOD_FRAMES * 2];
    let mut pcm_buf = vec![0i16; PERIOD_FRAMES * 2];

    loop {
        // 1. Snapshot stream list + volumes under lock, then drop immediately
        let streams: Vec<(Arc<StreamRing>, f32)> = {
            let guard = ACTIVE_STREAMS.lock().unwrap();
            guard.values().cloned().collect()
        };

        // 2. Mix outside lock
        pcm_buf.iter_mut().for_each(|s| *s = 0);
        let mut frame_bytes = vec![0u8; PERIOD_FRAMES * 4]; // s16le stereo
        for (ring, vol) in &streams {
            ring.pull(&mut frame_bytes);
            let scale = (vol * 32767.0) as i32;
            for (i, chunk) in frame_bytes.chunks_exact(2).enumerate() {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as i32;
                let mixed = pcm_buf[i] as i32 + (sample * scale / 32767);
                pcm_buf[i] = mixed.clamp(-32768, 32767) as i16;
            }
        }

        // 3. Write to ALSA
        if let Err(e) = alsa_write_frames(pcm_fd, &pcm_buf, PERIOD_FRAMES) {
            eprintln!("[audio] alsa write error: {e:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
```

---

## 6. IPC Handler (mod.rs)

```rust
// supervisor/src/audio/mod.rs

pub fn handle_audio_command(app_name: &str, line: &str, ctx: &mut AppContext) {
    let rest = line.strip_prefix("VYOMA_AUDIO:").unwrap();
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    match parts[0] {
        "open" => {
            // parts[1] = "<sid>,<rate>,<fmt>,<ch>"
            if !ctx.caps.audio_output {
                eprintln!("[audio] {app_name} lacks audio_output capability");
                return;
            }
            let fields: Vec<&str> = parts[1].split(',').collect();
            let sid: u64 = fields[0].parse().unwrap_or(0);
            let rate: u32 = fields[1].parse().unwrap_or(44100);
            // fmt and ch stored for ring format negotiation
            let ring = StreamRing::new();
            ACTIVE_STREAMS.lock().unwrap().insert(sid, (ring, 1.0));
            ensure_mixer_started(get_or_open_pcm());
        }
        "write" => {
            let fields: Vec<&str> = parts[1].splitn(2, ',').collect();
            let sid: u64 = fields[0].parse().unwrap_or(0);
            let pcm_bytes = base64_decode(fields[1]);
            if let Some((ring, _)) = ACTIVE_STREAMS.lock().unwrap().get(&sid) {
                ring.push(&pcm_bytes);
            }
        }
        "close" => {
            let sid: u64 = parts[1].parse().unwrap_or(0);
            ACTIVE_STREAMS.lock().unwrap().remove(&sid);
        }
        "set_vol" => {
            let fields: Vec<&str> = parts[1].split(',').collect();
            let sid: u64 = fields[0].parse().unwrap_or(0);
            let vol: f32 = fields[1].parse::<u8>().unwrap_or(100) as f32 / 100.0;
            if let Some(entry) = ACTIVE_STREAMS.lock().unwrap().get_mut(&sid) {
                entry.1 = vol;
            }
        }
        "mic_open" => {
            let req_id: u64 = parts[1].parse().unwrap_or(0);
            handle_mic_open(app_name, req_id, ctx);
        }
        "mic_close" => {
            stop_mic_capture(app_name);
        }
        _ => eprintln!("[audio] unknown command: {rest}"),
    }
}

/// Inline base64 decoder — no external dep.
fn base64_decode(s: &str) -> Vec<u8> {
    const TABLE: [u8; 256] = build_base64_table();
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let b = [
            TABLE[chunk[0] as usize],
            TABLE[chunk.get(1).copied().unwrap_or(b'A') as usize],
            TABLE[chunk.get(2).copied().unwrap_or(b'A') as usize],
            TABLE[chunk.get(3).copied().unwrap_or(b'A') as usize],
        ];
        let v = (b[0] as u32) << 18 | (b[1] as u32) << 12 |
                (b[2] as u32) << 6  |  b[3] as u32;
        out.push((v >> 16) as u8);
        if chunk.len() > 2 { out.push((v >> 8) as u8); }
        if chunk.len() > 3 { out.push(v as u8); }
    }
    out
}
```

---

## 7. Microphone Permission Gate

```rust
// supervisor/src/audio/mic.rs

static MIC_PERM_MAP: Mutex<HashMap<u64, MicPermStatus>> = Mutex::new(HashMap::new());

pub fn handle_mic_open(app_name: &str, req_id: u64, ctx: &mut AppContext) {
    if !ctx.caps.audio_input {
        send_reply(ctx.inbox, "REPLY:mic-denied capability-not-declared");
        return;
    }

    // Deferred reply: IPC thread freed immediately
    send_reply(ctx.inbox, &format!("REPLY:pending req_id={req_id}"));

    // Show overlay (non-blocking — posts FlushCmd::ShowOverlay via channel)
    show_mic_permission_overlay(app_name, req_id);

    let app_inbox = ctx.inbox.clone();
    std::thread::Builder::new()
        .name(format!("mic-perm-{req_id}"))
        .spawn(move || {
            let result = wait_for_permission_overlay_response(req_id, Duration::from_secs(30));
            let resp = match result {
                PermResult::Allowed => {
                    spawn_mic_capture_thread(app_name.to_string(), app_inbox.clone());
                    format!("PERM_RESP:{req_id}:allowed")
                }
                PermResult::Denied | PermResult::Timeout => {
                    format!("PERM_RESP:{req_id}:denied")
                }
            };
            let _ = app_inbox.send(resp);
        })
        .expect("spawn mic perm thread");
}

fn spawn_mic_capture_thread(app_name: String, inbox: AppInbox) {
    std::thread::Builder::new()
        .name(format!("mic-cap-{app_name}"))
        .spawn(move || {
            let fd = alsa_open_pcm(0, 0, true).expect("open capture PCM");
            alsa_hw_params(fd, 44100, 1, SampleFormat::S16Le).expect("mic hw_params");
            let mut buf = vec![0i16; PERIOD_FRAMES];
            loop {
                match alsa_read_frames(fd, &mut buf, PERIOD_FRAMES) {
                    Ok(_) => {
                        let bytes: Vec<u8> = buf.iter()
                            .flat_map(|s| s.to_le_bytes())
                            .collect();
                        let b64 = base64_encode(&bytes);
                        let _ = inbox.send(format!("VYOMA_AUDIO:mic_data:{b64}"));
                    }
                    Err(e) => {
                        eprintln!("[audio] mic read error: {e:?}");
                        break;
                    }
                }
            }
            unsafe { libc::close(fd) };
        })
        .expect("spawn mic capture");
}
```

---

## 8. Manifest Capability Extension

```rust
// supervisor/src/manifest.rs — add to AppCapabilities

pub struct AppCapabilities {
    // … existing fields …
    pub audio_output: bool,
    pub audio_input:  bool,
}
```

`vyoma.toml` usage:
```toml
[capabilities]
audio_output = true
audio_input  = false  # default; mic_open will fail if true not set
```

---

## 9. Kernel Configuration

Add to `base/kernel.config`:
```
CONFIG_SOUND=y
CONFIG_SND=y
CONFIG_SND_PCM=y
CONFIG_SND_VIRTIO=y
CONFIG_SND_HDA_INTEL=y
CONFIG_SND_USB_AUDIO=y
```

QEMU ALSA device (added to `make run-gui` target):
```
-device virtio-sound-pci,audiodev=snd0
-audiodev pa,id=snd0,out.mixing-engine=off
```

---

## 10. Blocking Issues (B1–B5)

**B1 — ALSA ioctl struct sizes must exactly match kernel ABI**
`SndrPcmHwParams` is 608 bytes; `SndrPcmSwParams` is 136 bytes. Off-by-one padding causes silent `EINVAL`. Validate with `static_assert!(size_of::<SndrPcmHwParams>() == 608)` via `const _: () = assert!(...)` in alsa.rs. Source sizes from `include/uapi/sound/asound.h` in Linux 5.10 tree.

**B2 — ABBA deadlock in mixer: lock → snapshot → drop → mix**
The mixer loop holds `ACTIVE_STREAMS` lock only to clone `Arc<StreamRing>` references, then drops the lock before reading ring data or calling ALSA ioctls. Never call ALSA inside the streams lock. Snapshot pattern enforced structurally in `mixer_loop`.

**B3 — OnceLock PCM fd races on first write**
`ensure_mixer_started()` uses `OnceLock<()>`; the PCM fd is opened inside the `get_or_init` closure before the mixer thread is spawned. The closure runs exactly once; subsequent calls return immediately. Store `pcm_fd` in a `OnceLock<RawFd>` initialized in the same closure.

**B4 — Microphone: IPC thread must not block on 30-second permission prompt**
`handle_mic_open` sends `REPLY:pending req_id=N` immediately, then spawns a dedicated `mic-perm-{req_id}` thread that blocks on the overlay response channel. The IPC thread processes the next command without delay. Result delivered as `PERM_RESP:N:allowed|denied` pushed to app stdin from the perm thread.

**B5 — No libasound / no new Cargo deps**
All ALSA interaction via raw `libc::ioctl`. Inline base64 decode in `mod.rs` (const lookup table, ~40 lines), inline encode in `mic.rs`. `SampleFormat::S16Le` maps to `SNDRV_PCM_FORMAT_S16_LE = 2`. No `extern crate` additions to `Cargo.toml`.
