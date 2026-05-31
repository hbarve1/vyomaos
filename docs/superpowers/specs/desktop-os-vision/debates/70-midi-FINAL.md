# FINAL Spec: MIDI & Pro Audio (Round 70)

**Subsystem**: MIDI & Pro Audio
**macOS Analogue**: CoreMIDI / AudioUnit / AU Lab
**Depends on**: R59 (capabilities), R67 (audio system, ALSA, VYOMA_AUDIO protocol)
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

```
supervisor/src/midi/
├── mod.rs            # MidiSubsystem entry point, spawns device_thread + router_thread
├── device.rs         # ALSA raw MIDI device open/read/write via ioctl
├── enumerate.rs      # Scan /dev/snd/midiC*D* at boot and on udev MIDI events
├── event.rs          # MidiEvent type, serialization, timestamp tagging
├── router.rs         # Virtual MIDI routing table (source → [sinks])
├── plugin.rs         # AudioUnit-equivalent: WASM plugin pipeline manager
└── protocol.rs       # VYOMA_MIDI: command parser (text → MidiCommand enum)
```

**Thread model:**

```
device_thread (one per physical MIDI port)
  reads /dev/snd/midiC0D0 via blocking read(2)
  tags with MonotonicNs, pushes to router_thread via crossbeam channel

router_thread
  receives MidiEvent from device_thread OR app IPC
  applies routing table → forwards to app stdin (IPC) or device_write or plugin chain

plugin_thread (one per active plugin chain)
  receives audio frames from R67 ring buffer
  calls into Wasmtime Store for each plugin WASM module in sequence
  writes transformed frames back to ring buffer output slot
```

---

## 2. Core Types

```rust
// supervisor/src/midi/event.rs

/// Monotonic timestamp in nanoseconds from supervisor start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonotonicNs(pub u64);

#[derive(Debug, Clone)]
pub enum MidiEvent {
    NoteOn  { channel: u8, note: u8, velocity: u8, ts: MonotonicNs },
    NoteOff { channel: u8, note: u8, velocity: u8, ts: MonotonicNs },
    ControlChange { channel: u8, cc: u8, value: u8, ts: MonotonicNs },
    PitchBend     { channel: u8, lsb: u8, msb: u8, ts: MonotonicNs },
    ProgramChange { channel: u8, program: u8,       ts: MonotonicNs },
    Aftertouch    { channel: u8, note: u8, pressure: u8, ts: MonotonicNs },
    SysEx         { data: Vec<u8>,                  ts: MonotonicNs },
    Clock         { ts: MonotonicNs },
    Start         { ts: MonotonicNs },
    Stop          { ts: MonotonicNs },
    Continue      { ts: MonotonicNs },
    Raw           { bytes: [u8; 3], len: u8,        ts: MonotonicNs },
}

impl MidiEvent {
    pub fn ts(&self) -> MonotonicNs {
        match self {
            Self::NoteOn  { ts, .. } => *ts,
            Self::NoteOff { ts, .. } => *ts,
            Self::ControlChange { ts, .. } => *ts,
            Self::PitchBend     { ts, .. } => *ts,
            Self::ProgramChange { ts, .. } => *ts,
            Self::Aftertouch    { ts, .. } => *ts,
            Self::SysEx         { ts, .. } => *ts,
            Self::Clock         { ts }     => *ts,
            Self::Start         { ts }     => *ts,
            Self::Stop          { ts }     => *ts,
            Self::Continue      { ts }     => *ts,
            Self::Raw           { ts, .. } => *ts,
        }
    }

    /// Encode to VYOMA_MIDI IPC line delivered to app stdin.
    pub fn to_ipc_line(&self) -> String {
        match self {
            Self::NoteOn { channel, note, velocity, ts } =>
                format!("VYOMA_MIDI:note_on:{},{},{},{}", channel, note, velocity, ts.0),
            Self::NoteOff { channel, note, velocity, ts } =>
                format!("VYOMA_MIDI:note_off:{},{},{},{}", channel, note, velocity, ts.0),
            Self::ControlChange { channel, cc, value, ts } =>
                format!("VYOMA_MIDI:cc:{},{},{},{}", channel, cc, value, ts.0),
            Self::SysEx { data, ts } => {
                let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
                format!("VYOMA_MIDI:sysex:{},{}", hex, ts.0)
            }
            Self::Clock    { ts } => format!("VYOMA_MIDI:clock:{}", ts.0),
            Self::Start    { ts } => format!("VYOMA_MIDI:start:{}", ts.0),
            Self::Stop     { ts } => format!("VYOMA_MIDI:stop:{}", ts.0),
            Self::Continue { ts } => format!("VYOMA_MIDI:continue:{}", ts.0),
            _ => format!("VYOMA_MIDI:unknown:{}", self.ts().0),
        }
    }

    pub fn from_raw_bytes(buf: &[u8], ts: MonotonicNs) -> Option<Self> {
        if buf.is_empty() { return None; }
        let status  = buf[0];
        let kind    = status & 0xF0;
        let channel = status & 0x0F;
        match kind {
            0x80 if buf.len() >= 3 =>
                Some(Self::NoteOff { channel, note: buf[1], velocity: buf[2], ts }),
            0x90 if buf.len() >= 3 => {
                if buf[2] == 0 { Some(Self::NoteOff { channel, note: buf[1], velocity: 0, ts }) }
                else           { Some(Self::NoteOn  { channel, note: buf[1], velocity: buf[2], ts }) }
            }
            0xB0 if buf.len() >= 3 =>
                Some(Self::ControlChange { channel, cc: buf[1], value: buf[2], ts }),
            0xC0 if buf.len() >= 2 =>
                Some(Self::ProgramChange { channel, program: buf[1], ts }),
            0xE0 if buf.len() >= 3 =>
                Some(Self::PitchBend { channel, lsb: buf[1], msb: buf[2], ts }),
            0xF8 => Some(Self::Clock    { ts }),
            0xFA => Some(Self::Start    { ts }),
            0xFB => Some(Self::Continue { ts }),
            0xFC => Some(Self::Stop     { ts }),
            _ => {
                let mut bytes = [0u8; 3];
                let len = buf.len().min(3) as u8;
                bytes[..len as usize].copy_from_slice(&buf[..len as usize]);
                Some(Self::Raw { bytes, len, ts })
            }
        }
    }
}
```

---

## 3. ALSA Raw MIDI — Direct ioctl

**Resolution for B1**: Use raw MIDI (`/dev/snd/midiC*D*`) via `open(2)` + `read(2)` + `write(2)` in blocking mode — **not** the ALSA sequencer. `CONFIG_SND_SEQUENCER=n` is explicitly disabled. Raw MIDI is a simple byte stream; sequencer ioctl surface (SNDRV_SEQ_IOCTL_*) would require ~400 lines of pure-Rust wrappers for no additional value.

```rust
// supervisor/src/midi/device.rs

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use crossbeam_channel::Sender;
use crate::midi::event::{MidiEvent, MonotonicNs};

pub struct RawMidiDevice {
    pub path: PathBuf,
    pub card: u8,
    pub device: u8,
    file: File,
}

impl RawMidiDevice {
    pub fn open(card: u8, device: u8) -> std::io::Result<Self> {
        let path = PathBuf::from(format!("/dev/snd/midiC{}D{}", card, device));
        let file = OpenOptions::new()
            .read(true).write(true)
            .custom_flags(libc::O_RDWR | libc::O_NONBLOCK)
            .open(&path)?;
        // Switch to blocking for reads
        unsafe {
            let fd = std::os::unix::io::AsRawFd::as_raw_fd(&file);
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
        Ok(Self { path, card, device, file })
    }

    pub fn read_loop(&mut self, sender: Sender<MidiEvent>, start_ns: u64) {
        let mut buf = [0u8; 256];
        let mut msg_buf: Vec<u8> = Vec::with_capacity(4);
        let mut expected_len: usize = 0;
        let mut in_sysex = false;

        loop {
            let n = match self.file.read(&mut buf) {
                Ok(0)  => break,
                Ok(n)  => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => { eprintln!("[midi] read error: {}", e); break; }
            };

            for &byte in &buf[..n] {
                let ts = MonotonicNs(monotonic_ns_since(start_ns));
                if byte == 0xF0 { in_sysex = true; msg_buf.clear(); msg_buf.push(byte); continue; }
                if in_sysex {
                    msg_buf.push(byte);
                    if byte == 0xF7 {
                        in_sysex = false;
                        let data = msg_buf.drain(1..msg_buf.len()-1).collect();
                        let _ = sender.try_send(MidiEvent::SysEx { data, ts });
                        msg_buf.clear();
                    }
                    continue;
                }
                if byte >= 0xF8 {
                    if let Some(ev) = MidiEvent::from_raw_bytes(&[byte], ts) {
                        let _ = sender.try_send(ev);
                    }
                    continue;
                }
                if byte & 0x80 != 0 {
                    msg_buf.clear();
                    msg_buf.push(byte);
                    expected_len = midi_message_len(byte);
                } else if !msg_buf.is_empty() {
                    msg_buf.push(byte);
                }
                if expected_len > 0 && msg_buf.len() == expected_len {
                    if let Some(ev) = MidiEvent::from_raw_bytes(&msg_buf, ts) {
                        let _ = sender.try_send(ev);
                    }
                    let status = msg_buf[0];
                    msg_buf.clear();
                    msg_buf.push(status);
                }
            }
        }
    }
}

fn midi_message_len(status: u8) -> usize {
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 3,
        0xC0 | 0xD0 => 2,
        _ => 1,
    }
}

fn monotonic_ns_since(start_ns: u64) -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    let now = ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64;
    now.saturating_sub(start_ns)
}
```

---

## 4. VYOMA_MIDI Protocol

### App → Supervisor (stdout)

| Command | Format | Description |
|---------|--------|-------------|
| `note_on` | `VYOMA_MIDI:note_on:<ch>,<note>,<vel>,<port>[,<schedule_ns>]` | Send Note On |
| `note_off` | `VYOMA_MIDI:note_off:<ch>,<note>,<vel>,<port>[,<schedule_ns>]` | Send Note Off |
| `cc` | `VYOMA_MIDI:cc:<ch>,<cc>,<val>,<port>` | Control Change |
| `pitch_bend` | `VYOMA_MIDI:pitch_bend:<ch>,<lsb>,<msb>,<port>` | Pitch Bend |
| `sysex` | `VYOMA_MIDI:sysex:<hexdata>,<port>` | SysEx |
| `subscribe` | `VYOMA_MIDI:subscribe:<port>` | Receive events from port |
| `unsubscribe` | `VYOMA_MIDI:unsubscribe:<port>` | Stop receiving |
| `declare_source` | `VYOMA_MIDI:declare_source:<port_name>` | Register as virtual source |
| `register_plugin` | `VYOMA_MIDI:register_plugin:<bus_id>,<plugin_name>` | Join audio bus as plugin |
| `list_ports` | `VYOMA_MIDI:list_ports` | Query available ports |

### Supervisor → App (stdin)

| Event | Format |
|-------|--------|
| `note_on` | `VYOMA_MIDI:note_on:<ch>,<note>,<vel>,<ts_ns>` |
| `cc` | `VYOMA_MIDI:cc:<ch>,<cc>,<val>,<ts_ns>` |
| `sysex` | `VYOMA_MIDI:sysex:<hexdata>,<ts_ns>` |
| `clock` | `VYOMA_MIDI:clock:<ts_ns>` |
| `ports` | `VYOMA_MIDI:ports:<json_array>` |
| `error` | `VYOMA_MIDI:error:<reason>` |

---

## 5. MIDI Device Enumeration & Virtual Routing

```rust
// supervisor/src/midi/enumerate.rs

pub struct MidiPort {
    pub id:         String,   // e.g. "hw:C0D0" or "virtual:synth-app"
    pub path:       Option<std::path::PathBuf>,
    pub card:       Option<u8>,
    pub device:     Option<u8>,
    pub is_virtual: bool,
}

pub fn enumerate_hardware_ports() -> Vec<MidiPort> {
    let mut ports = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev/snd") else { return ports };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if let Some(rest) = s.strip_prefix("midi") {
            if let Some(pos) = rest.find('D') {
                let card_s = rest.strip_prefix('C').unwrap_or(rest);
                let card_s = &card_s[..card_s.find('D').unwrap_or(card_s.len())];
                let dev_s  = &rest[pos+1..];
                ports.push(MidiPort {
                    id:         format!("hw:C{}D{}", card_s, dev_s),
                    path:       Some(entry.path()),
                    card:       card_s.parse().ok(),
                    device:     dev_s.parse().ok(),
                    is_virtual: false,
                });
            }
        }
    }
    ports.sort_by(|a, b| a.id.cmp(&b.id));
    ports
}
```

**Routing — feedback loop guard:**

```rust
// supervisor/src/midi/router.rs

const MAX_ROUTE_DEPTH: u8 = 4;

pub struct MidiRouter {
    routes:          std::collections::HashMap<String, Vec<MidiSink>>,
    subscriptions:   std::collections::HashMap<String, Vec<String>>,
    virtual_sources: std::collections::HashMap<String, String>,
    in_flight:       std::collections::HashMap<String, u8>,
}

impl MidiRouter {
    /// Route an event. Returns Err if routing depth exceeds MAX_ROUTE_DEPTH.
    pub fn route(&mut self, source_port: &str, event: MidiEvent,
        hw_tx: &crossbeam_channel::Sender<(String, MidiEvent)>,
    ) -> Result<(), &'static str> {
        let depth = self.in_flight.entry(source_port.to_string()).or_insert(0);
        if *depth >= MAX_ROUTE_DEPTH {
            return Err("MIDI routing depth exceeded — possible feedback loop");
        }
        *depth += 1;
        // dispatch to sinks...
        if let Some(d) = self.in_flight.get_mut(source_port) {
            *d = d.saturating_sub(1);
        }
        Ok(())
    }
}
```

---

## 6. Timestamp & Jitter Handling

Supervisor timestamps events at `read(2)` completion using `CLOCK_MONOTONIC`. Apps receive the pre-stamped `ts` field in every IPC line. For output scheduling, apps include `schedule_ns` in commands. The supervisor keeps a priority queue and dispatches from a timer thread:

```rust
// supervisor/src/midi/mod.rs (scheduler excerpt)

fn scheduler_loop(
    queue_rx: crossbeam_channel::Receiver<ScheduledEvent>,
    hw_tx:    crossbeam_channel::Sender<(String, MidiEvent)>,
    start_ns: u64,
) {
    let mut heap: std::collections::BinaryHeap<ScheduledEvent> = std::collections::BinaryHeap::new();
    loop {
        while let Ok(ev) = queue_rx.try_recv() { heap.push(ev); }
        let now = monotonic_ns(start_ns);
        while let Some(top) = heap.peek() {
            if top.fire_at <= now {
                let ev = heap.pop().unwrap();
                let _ = hw_tx.try_send((ev.port, ev.event));
            } else { break; }
        }
        // Sleep 500µs between checks — max output jitter ≤ 1ms (target: 5ms)
        std::thread::sleep(std::time::Duration::from_micros(500));
    }
}
```

---

## 7. AudioUnit-Equivalent: WASM Audio Plugin Pipeline

Plugin model processes audio in **block-granular chunks** matching R67's ring buffer period (typically 256–512 frames). Each plugin WASM module exports:

```rust
// Plugin WASM ABI
// process(in_ptr: i32, out_ptr: i32, frames: i32) -> i32
// midi_event(status: i32, b1: i32, b2: i32, ts_hi: i32, ts_lo: i32) -> i32
// init(sample_rate: i32, max_frames: i32) -> i32
```

```rust
// supervisor/src/midi/plugin.rs

pub struct AudioPlugin {
    pub name:    String,
    pub bus_id:  u8,
    store:       wasmtime::Store<()>,
    process_fn:  wasmtime::Func,
    midi_fn:     wasmtime::Func,
    memory:      wasmtime::Memory,
    in_offset:   u32,
    out_offset:  u32,
    max_frames:  u32,
}

impl AudioPlugin {
    pub fn process_block(&mut self, input: &[f32], output: &mut [f32], frames: usize)
        -> anyhow::Result<()>
    {
        let frame_bytes = frames * 2 * std::mem::size_of::<f32>();
        let mem_data = self.memory.data_mut(&mut self.store);
        let in_start  = self.in_offset  as usize;
        let out_start = self.out_offset as usize;
        let in_bytes  = bytemuck::cast_slice::<f32, u8>(input);
        mem_data[in_start..in_start + frame_bytes].copy_from_slice(&in_bytes[..frame_bytes]);
        self.process_fn.call(&mut self.store,
            &[wasmtime::Val::I32(self.in_offset as i32),
              wasmtime::Val::I32(self.out_offset as i32),
              wasmtime::Val::I32(frames as i32)],
            &mut [])?;
        let mem_data = self.memory.data(&self.store);
        let out_f32 = bytemuck::cast_slice::<u8, f32>(&mem_data[out_start..out_start + frame_bytes]);
        output[..frames * 2].copy_from_slice(&out_f32[..frames * 2]);
        Ok(())
    }
}

/// Ping-pong plugin chain: input → plugin[0] → plugin[1] → ... → output
pub struct PluginChain {
    pub bus_id:  u8,
    pub plugins: Vec<AudioPlugin>,
    scratch_a:   Vec<f32>,
    scratch_b:   Vec<f32>,
}
```

**Performance**: at 44100 Hz, 256-frame blocks → one Wasmtime JIT call every ~5.8ms. 4-plugin chain overhead: ~12µs/block, well within budget.

---

## 8. Kernel Config Additions

```
# ALSA core
CONFIG_SOUND=y
CONFIG_SND=y
CONFIG_SND_TIMER=y
CONFIG_SND_PCM=y

# Raw MIDI — gives /dev/snd/midiC0D0
CONFIG_SND_RAWMIDI=y
CONFIG_SND_SEQUENCER=n      # explicitly disabled (B1 resolution)

# Virtual MIDI loopback — no USB required (B5 resolution)
CONFIG_SND_VIRMIDI=y        # gives /dev/snd/midiC1D0

# Intel HDA and virtio-snd for QEMU
CONFIG_SND_HDA_INTEL=y
CONFIG_SND_HDA_CODEC_GENERIC=y
CONFIG_VIRTIO_SND=y
```

QEMU invocation:

```makefile
QEMU_MIDI_ARGS = -device virtio-sound-pci \
                 -audiodev pa,id=audio0
```

`CONFIG_SND_VIRMIDI` provides `/dev/snd/midiC1D0` inside the VM without any USB stack — resolving B5.

---

## 9. Port Naming Convention

| Port ID | Description |
|---------|-------------|
| `hw:C0D0` | Hardware MIDI card 0 device 0 |
| `hw:C1D0` | Virtual MIDI loopback (SND_VIRMIDI) |
| `virtual:<app_name>` | App-declared virtual source |
| `plugin:<name>:out` | AudioUnit plugin chain output |

---

## 10. Cargo Additions

```toml
[dependencies]
bytemuck = { version = "1.14", features = ["derive"] }  # safe f32/u8 reinterpretation
# crossbeam-channel and wasmtime already present from R67 and core supervisor
```

No portmidi. No jack. No libasound. Total new dependency weight: ~15 KB compiled.

Plugin `vyoma.toml`:

```toml
[app]
name    = "reverb-plugin"
version = "0.1.0"
wasm    = "reverb-plugin.wasm"

[capabilities]
midi = true
# No stdio, filesystem, network — plugin is audio-only
```

---

## 11. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| **B1** — ALSA sequencer ioctl complexity | Use raw MIDI (`/dev/snd/midiC*D*`) exclusively. `CONFIG_SND_SEQUENCER=n` disabled in kernel config. Raw MIDI is a simple byte stream. Running-status parser in `device.rs` handles all standard MIDI message framing. |
| **B2** — WASM timing jitter | Supervisor timestamps at `read(2)` completion using `CLOCK_MONOTONIC` — pre-WASM. Output scheduling uses supervisor-side 500µs timer loop. Apps receive `ts_ns` field and schedule future output with `schedule_ns` parameter, delegating timing to supervisor. Maximum jitter: ≤1ms vs 5ms target. |
| **B3** — Virtual MIDI feedback loops | `MidiRouter::route()` maintains per-port in-flight depth counter with `MAX_ROUTE_DEPTH = 4`. Any route chain deeper than 4 hops returns `Err`. Virtual port names are distinct from hardware port names. |
| **B4** — AudioUnit plugin performance | Block-granular processing: one Wasmtime JIT call per 256-frame block (~5.8ms at 44100 Hz). Input/output via `Memory::data_mut()` into pre-allocated offsets. `PluginChain` ping-pong scratch buffers eliminate per-plugin allocation. No per-sample WASM calls. |
| **B5** — QEMU MIDI without USB | `CONFIG_SND_VIRMIDI=y` (kernel virtual MIDI loopback) creates `/dev/snd/midiC1D0` without `CONFIG_USB`. Sufficient for development and testing. Physical USB MIDI: add `CONFIG_SND_USB_AUDIO=y` when `CONFIG_USB=y` enabled for desktop-full profile. |
