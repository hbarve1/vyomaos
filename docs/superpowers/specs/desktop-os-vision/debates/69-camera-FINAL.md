# FINAL Spec: Camera & Capture Pipeline (Round 69)

**Subsystem**: Camera & Capture Pipeline
**macOS Analogue**: AVCaptureSession / IIDCFamily
**Depends on**: R59 (capabilities), R68 (video/shm patterns), R11 (compositor overlay for permission)
**Status**: FINAL — all blocking issues resolved

---

## 1. Architecture

```
supervisor/src/camera/
├── mod.rs              # CameraSubsystem entry, IPC verb dispatch, capability gate
├── v4l2.rs             # Raw V4L2 ioctl wrappers; device open/configure/stream/close
├── frame_shm.rs        # memfd-backed 4-slot frame ring; write_volatile + fence handoff
├── permission.rs       # Permission gate: deferred reply, overlay, 30-second timeout
├── permission_store.rs # /data/.vyoma/camera/permissions.json — R41 persistent grants
└── indicator.rs        # Green "camera active" disc in menu bar; CAM_ACTIVE_COLOR
```

**Thread model:**

```
cam_device_thread (one per active camera session):
  opens /dev/video0 via V4L2
  enqueues NUM_BUFS=4 kernel mmap buffers (VIDIOC_REQBUFS + VIDIOC_QBUF)
  blocking VIDIOC_DQBUF loop → copies frame to FrameShm slot → notifies app

IPC thread:
  parses VYOMA_CAMERA:open / close / list
  calls handle_cam_open() → deferred REPLY:cam-pending req_id=N
  spawns permission gate thread

indicator_thread (global singleton):
  watches CAM_ACTIVE atomic bool
  posts FlushCmd::DrawCamIndicator to compositor when state changes
```

---

## 2. V4L2 Ioctl Constants and Wrappers

```rust
// supervisor/src/camera/v4l2.rs

// V4L2 ioctl numbers from <linux/videodev2.h>
const VIDIOC_QUERYCAP:  u64 = 0x8068_5600;
const VIDIOC_S_FMT:     u64 = 0xC0D0_5605;
const VIDIOC_REQBUFS:   u64 = 0xC014_5608;
const VIDIOC_QUERYBUF:  u64 = 0xC058_5609;
const VIDIOC_QBUF:      u64 = 0xC058_560F;
const VIDIOC_DQBUF:     u64 = 0xC058_5611;
const VIDIOC_STREAMON:  u64 = 0x4004_5612;
const VIDIOC_STREAMOFF: u64 = 0x4004_5613;

pub const NUM_BUFS: u32 = 4;

#[repr(C)]
pub struct V4l2Capability {
    pub driver:    [u8; 16],
    pub card:      [u8; 32],
    pub bus_info:  [u8; 32],
    pub version:   u32,
    pub capabilities: u32,
    pub device_caps:  u32,
    pub reserved:  [u32; 3],
}

#[repr(C)]
pub struct V4l2Format { /* 208 bytes; includes V4l2PixFormat union */ }

#[repr(C)]
pub struct V4l2RequestBuffers {
    pub count:  u32,
    pub type_:  u32,  // V4L2_BUF_TYPE_VIDEO_CAPTURE = 1
    pub memory: u32,  // V4L2_MEMORY_MMAP = 1
    pub reserved: [u32; 2],
}

#[repr(C)]
pub struct V4l2Buffer {
    pub index:    u32,
    pub type_:    u32,
    pub bytesused: u32,
    pub flags:    u32,
    pub field:    u32,
    pub timestamp: libc::timeval,
    pub timecode:  V4l2Timecode,
    pub sequence: u32,
    pub memory:   u32,
    pub m_offset: u32,  // mmap offset (union — use first field)
    pub length:   u32,
    pub reserved2: u32,
    pub reserved:  u32,
}

pub enum PixelFormat { Yuyv, Mjpeg }

impl PixelFormat {
    pub fn fourcc(&self) -> u32 {
        match self {
            Self::Yuyv => u32::from_le_bytes(*b"YUYV"),
            Self::Mjpeg => u32::from_le_bytes(*b"MJPG"),
        }
    }
    pub fn name(&self) -> &'static str {
        match self { Self::Yuyv => "yuyv", Self::Mjpeg => "mjpeg" }
    }
}

pub fn v4l2_open(path: &str) -> Result<RawFd, CamError> {
    let c_path = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 { return Err(CamError::Open(errno())); }
    Ok(fd)
}

pub fn v4l2_query_cap(fd: RawFd) -> Result<V4l2Capability, CamError> {
    let mut cap = unsafe { std::mem::zeroed::<V4l2Capability>() };
    let rc = unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) };
    if rc < 0 { return Err(CamError::QueryCap(errno())); }
    Ok(cap)
}

pub fn v4l2_set_fmt(fd: RawFd, w: u32, h: u32, fmt: &PixelFormat) -> Result<(), CamError> {
    let mut format = unsafe { std::mem::zeroed::<V4l2Format>() };
    format.set_capture(w, h, fmt.fourcc());
    let rc = unsafe { libc::ioctl(fd, VIDIOC_S_FMT, &mut format) };
    if rc < 0 { return Err(CamError::SetFmt(errno())); }
    Ok(())
}

pub fn v4l2_request_bufs(fd: RawFd) -> Result<u32, CamError> {
    let mut req = V4l2RequestBuffers {
        count: NUM_BUFS, type_: 1, memory: 1, reserved: [0; 2],
    };
    let rc = unsafe { libc::ioctl(fd, VIDIOC_REQBUFS, &mut req) };
    if rc < 0 { return Err(CamError::ReqBufs(errno())); }
    Ok(req.count)
}

pub struct KernelBuf { pub ptr: *mut u8, pub len: usize }

pub fn v4l2_mmap_bufs(fd: RawFd, count: u32) -> Result<Vec<KernelBuf>, CamError> {
    let mut bufs = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mut buf = unsafe { std::mem::zeroed::<V4l2Buffer>() };
        buf.index = i; buf.type_ = 1; buf.memory = 1;
        let rc = unsafe { libc::ioctl(fd, VIDIOC_QUERYBUF, &mut buf) };
        if rc < 0 { return Err(CamError::QueryBuf(errno())); }
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), buf.length as usize,
                libc::PROT_READ, libc::MAP_SHARED, fd, buf.m_offset as i64)
        };
        if ptr == libc::MAP_FAILED { return Err(CamError::Mmap(errno())); }
        bufs.push(KernelBuf { ptr: ptr as *mut u8, len: buf.length as usize });
    }
    Ok(bufs)
}

pub fn v4l2_stream_on(fd: RawFd) -> Result<(), CamError> {
    let type_: u32 = 1;
    let rc = unsafe { libc::ioctl(fd, VIDIOC_STREAMON, &type_) };
    if rc < 0 { return Err(CamError::StreamOn(errno())); }
    Ok(())
}
```

---

## 3. Shared Memory Frame Ring (frame_shm.rs)

```rust
// supervisor/src/camera/frame_shm.rs

/// 4-slot lock-free frame ring backed by memfd.
/// No MFD_CLOEXEC — fd must be inheritable by WASM app process.
pub const FRAME_SLOTS: usize = 4;
const SLOT_ALIGN: usize = 64;

pub struct FrameShm {
    pub fd:        RawFd,
    pub ptr:       *mut u8,
    pub slot_size: usize,   // bytes per frame slot (width * height * bytes_per_pixel)
    pub total:     usize,   // 4 * slot_size, rounded up to page
    pub seq:       AtomicU64, // low 2 bits = write slot index, rest = frame count
}

impl FrameShm {
    pub fn create(session_id: u64, width: u32, height: u32, fmt: &PixelFormat) -> Result<Arc<Self>, CamError> {
        let bpp = match fmt { PixelFormat::Yuyv => 2, PixelFormat::Mjpeg => 3 };
        let frame_bytes = (width * height * bpp) as usize;
        let slot_size = (frame_bytes + SLOT_ALIGN - 1) & !(SLOT_ALIGN - 1);
        let total = (slot_size * FRAME_SLOTS + 4095) & !4095;

        let name = CString::new(format!("vyoma_cam_{session_id}")).unwrap();
        // No MFD_CLOEXEC — fd must be inheritable
        let fd = unsafe { libc::syscall(libc::SYS_memfd_create, name.as_ptr(), 0u32) } as RawFd;
        if fd < 0 { return Err(CamError::MemfdCreate(errno())); }
        unsafe { libc::ftruncate(fd, total as i64) };

        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), total,
                libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if ptr == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return Err(CamError::Mmap(errno()));
        }

        Ok(Arc::new(Self { fd, ptr: ptr as *mut u8, slot_size, total, seq: AtomicU64::new(0) }))
    }

    /// Write a frame to the next slot. Called by cam_device_thread.
    /// Uses write_volatile + Release fence — no lock required.
    pub fn write_frame(&self, data: &[u8]) {
        let old_seq = self.seq.load(Relaxed);
        let slot = (old_seq as usize + 1) & (FRAME_SLOTS - 1);
        let base = slot * self.slot_size;
        let len = data.len().min(self.slot_size);

        // Mark in-progress: odd seq
        self.seq.store(old_seq | 1, Release);

        unsafe {
            for (i, &b) in data[..len].iter().enumerate() {
                (self.ptr.add(base + i) as *mut u8).write_volatile(b);
            }
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);

        // Mark complete: even seq, incremented
        self.seq.store((old_seq + 2) & !1, Release);
    }
}
```

---

## 4. IPC Protocol

```
VYOMA_CAMERA:open:<req_id>,<width>,<height>,<fps>,<fmt>
VYOMA_CAMERA:close:
VYOMA_CAMERA:list:
```

Supervisor responses to app stdin:
```
REPLY:cam-pending req_id=N            # deferred; permission gate started
PERM_RESP:N:allowed                   # permission granted
PERM_RESP:N:denied                    # permission denied or timed out
VYOMA_SYSTEM:cam-shm-fd:<fd> w=<w> h=<h> fmt=<fmt>   # fd number for mmapping
VYOMA_CAMERA:frame:<seq>:<w>:<h>:<fmt>               # new frame notification
VYOMA_CAMERA:list-result:<json>                       # list of available devices
```

Pixel data never appears in-band. App mmaps the shared fd for pixel access:

```rust
// App-side (WASM) example
let fd = cam_shm_fd;  // received via VYOMA_SYSTEM:cam-shm-fd line
let ptr = unsafe { libc::mmap(null_mut(), total, PROT_READ, MAP_SHARED, fd, 0) };
// On "VYOMA_CAMERA:frame:<seq>:..." notification, read from ptr at slot = seq % 4
```

---

## 5. Permission Gate

```rust
// supervisor/src/camera/permission.rs

pub struct PendingOpenParams {
    pub app_name: String,
    pub width:    u32,
    pub height:   u32,
    pub fps:      u32,
    pub fmt:      PixelFormat,
}

static PENDING_OPEN_PARAMS: Mutex<HashMap<u64, PendingOpenParams>> = Mutex::new(HashMap::new());

// Overlay hit-test rects (compositor coords)
static PERM_OVERLAY: OnceLock<PermOverlay> = OnceLock::new();

pub struct PermOverlay {
    pub deny_rect:  Rect,
    pub allow_rect: Rect,
}

pub fn handle_cam_open(app_name: &str, req_id: u64, params: PendingOpenParams, ctx: &AppContext) {
    if !ctx.caps.camera {
        send_reply(&ctx.inbox, "REPLY:cam-denied capability-not-declared");
        return;
    }

    // Check persistent grant
    if permission_store::is_granted(app_name) {
        send_reply(&ctx.inbox, &format!("REPLY:pending req_id={req_id}"));
        spawn_camera_session(app_name, req_id, params, ctx.inbox.clone());
        return;
    }

    // Deferred reply
    send_reply(&ctx.inbox, &format!("REPLY:cam-pending req_id={req_id}"));
    PENDING_OPEN_PARAMS.lock().unwrap().insert(req_id, params);

    let app_inbox = ctx.inbox.clone();
    let app_name = app_name.to_string();
    std::thread::Builder::new()
        .name(format!("cam-perm-{req_id}"))
        .spawn(move || {
            show_camera_permission_overlay(&app_name, req_id);
            let result = wait_for_perm_response(req_id, Duration::from_secs(30));
            let params = PENDING_OPEN_PARAMS.lock().unwrap().remove(&req_id);

            match result {
                PermResult::Allowed => {
                    let _ = app_inbox.send(format!("PERM_RESP:{req_id}:allowed"));
                    if let Some(p) = params {
                        spawn_camera_session(&app_name, req_id, p, app_inbox);
                    }
                }
                _ => {
                    let _ = app_inbox.send(format!("PERM_RESP:{req_id}:denied"));
                }
            }
        })
        .expect("spawn cam perm thread");
}
```

---

## 6. Camera-Active Indicator

```rust
// supervisor/src/camera/indicator.rs

pub const CAM_ACTIVE_COLOR: u32 = 0x30D158FF; // Apple green

static CAM_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_camera_active(active: bool) {
    let was = CAM_ACTIVE.swap(active, SeqCst);
    if was != active {
        // Post draw command to compositor flush queue
        FLUSH_TX.get().unwrap().send(FlushCmd::DrawCamIndicator { active }).ok();
    }
}

// Compositor handles FlushCmd::DrawCamIndicator:
// Draws green disc (radius 5px) + "REC" label at menu bar right region
// when active=true; erases to menu bar background when active=false
```

---

## 7. Manifest Capability Extension

```rust
// supervisor/src/manifest.rs

pub struct AppCapabilities {
    // … existing fields …
    pub camera: bool,
}
```

`vyoma.toml` usage:
```toml
[capabilities]
camera = true
```

---

## 8. Kernel Configuration

Add to `base/kernel.config`:
```
CONFIG_MEDIA_SUPPORT=y
CONFIG_MEDIA_CAMERA_SUPPORT=y
CONFIG_VIDEO_DEV=y
CONFIG_VIDEO_V4L2=y
CONFIG_USB_VIDEO_CLASS=y
CONFIG_USB_EHCI_HCD=y
CONFIG_USB_OHCI_HCD=y
```

**QEMU camera target** (`Makefile`):

```makefile
run-cam:
	$(QEMU) $(QEMU_FLAGS) \
		-device usb-ehci,id=ehci0 \
		-device usb-webcam,bus=ehci0.0 \
		$(SERIAL_FLAGS)
```

Graceful fallback: `v4l2_open("/dev/video0")` returns `Err(CamError::Open(ENOENT))` → supervisor logs `[camera] no camera device found` → `VYOMA_CAMERA:list-result:[]` sent to app; no crash.

---

## 9. Blocking Issues (B1–B5)

**B1 — memfd_create must NOT use MFD_CLOEXEC**
The frame shm fd must be inherited by the WASM app's Wasmtime process. `MFD_CLOEXEC` would close it on exec. Supervisor passes raw fd number in `VYOMA_SYSTEM:cam-shm-fd:<fd>` line; app mmaps it for read-only access. Supervisor owns the write side.

**B2 — write_volatile + Release fence for lock-free frame handoff**
Frame copy uses `write_volatile` per byte (prevents compiler reordering), followed by `std::sync::atomic::fence(Release)`. Reader uses `fence(Acquire)` before reading. The `seq` atomic transitions: odd = write in progress (reader skips), even = frame complete. No mutex required in hot path.

**B3 — V4L2 struct sizes must match kernel ABI exactly**
`V4l2Buffer` is 88 bytes on 64-bit Linux (including timeval as two i64). Validate with `const _: () = assert!(size_of::<V4l2Buffer>() == 88)`. Source from `include/uapi/linux/videodev2.h` in Linux 5.10 tree.

**B4 — PENDING_OPEN_PARAMS must be cleaned up on deny/timeout**
`wait_for_perm_response` returns `PermResult::Denied` on 30s timeout. The perm thread removes the entry from `PENDING_OPEN_PARAMS` and sends `PERM_RESP:N:denied` in both deny and timeout cases. Memory does not accumulate for unanswered requests.

**B5 — No camera in standard QEMU — use run-cam target with graceful fallback**
Standard `make run-gui` has no USB camera device. Add `make run-cam` target with `-device usb-ehci,id=ehci0 -device usb-webcam,bus=ehci0.0`. In production hardware, `/dev/video0` exists. If `v4l2_open` returns `ENOENT`, supervisor logs the absence, responds to `VYOMA_CAMERA:list:` with empty list, and does not crash. Apps must handle empty camera list gracefully.
