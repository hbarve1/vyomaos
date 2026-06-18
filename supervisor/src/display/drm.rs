// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! DRM/KMS dumb-buffer backend with double-buffering and page flip.
//!
//! Opens `/dev/dri/card0`, finds a connected connector with a preferred mode,
//! creates two dumb buffers, and page-flips between them on each frame.
//! Returns `None` from `DrmDisplay::open()` if DRM is unavailable, so the
//! caller can fall back to the fbdev path.

use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

// ── DRM ioctl numbers (x86_64 Linux) ─────────────────────────────────────────

const DRM_IOCTL_SET_MASTER: libc::c_ulong = 0x0000_641E;
const DRM_IOCTL_MODE_GETRESOURCES: libc::c_ulong = 0xC038_64A0;
#[allow(dead_code)]
const DRM_IOCTL_MODE_GETCRTC: libc::c_ulong = 0xC068_64A1;
const DRM_IOCTL_MODE_SETCRTC: libc::c_ulong = 0xC068_64A2;
const DRM_IOCTL_MODE_GETENCODER: libc::c_ulong = 0xC014_64A6;
const DRM_IOCTL_MODE_GETCONNECTOR: libc::c_ulong = 0xC050_64A7;
const DRM_IOCTL_MODE_ADDFB: libc::c_ulong = 0xC01C_64AE;
const DRM_IOCTL_MODE_PAGE_FLIP: libc::c_ulong = 0xC018_64B0;
const DRM_IOCTL_MODE_CREATE_DUMB: libc::c_ulong = 0xC020_64B2;
const DRM_IOCTL_MODE_MAP_DUMB: libc::c_ulong = 0xC010_64B3;

const DRM_MODE_CONNECTED: u32 = 1;
const DRM_MODE_TYPE_PREFERRED: u32 = 1 << 3;
const DRM_MODE_PAGE_FLIP_EVENT: u32 = 0x01;

/// Wrapper for libc::ioctl — handles type mismatch between glibc (c_ulong) and musl (c_int).
#[inline]
unsafe fn drm_ioctl(fd: i32, request: libc::c_ulong, arg: *mut libc::c_void) -> libc::c_int {
    libc::ioctl(fd, request as libc::Ioctl, arg)
}
#[inline]
unsafe fn drm_ioctl0(fd: i32, request: libc::c_ulong) -> libc::c_int {
    libc::ioctl(fd, request as libc::Ioctl)
}

// ── DRM ioctl structures ─────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrmModeModeinfo {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,
    pub vrefresh: u32,
    pub flags: u32,
    pub type_: u32,
    pub name: [u8; 32],
}

#[repr(C)]
struct DrmModeCardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
struct DrmModeGetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    _pad: u32,
}

#[repr(C)]
struct DrmModeGetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
struct DrmModeCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: DrmModeModeinfo,
}

#[repr(C)]
struct DrmModeCreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
struct DrmModeMapDumb {
    handle: u32,
    _pad: u32,
    offset: u64,
}

#[repr(C)]
struct DrmModeFbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

#[repr(C)]
struct DrmModeCrtcPageFlip {
    crtc_id: u32,
    fb_id: u32,
    flags: u32,
    _reserved: u32,
    user_data: u64,
}

// ── DRM event structs for reading page-flip completion ───────────────────────

#[repr(C)]
struct DrmEvent {
    type_: u32,
    length: u32,
}

#[repr(C)]
#[allow(dead_code)]
struct DrmEventVblank {
    base: DrmEvent,
    user_data: u64,
    tv_sec: u32,
    tv_usec: u32,
    sequence: u32,
    crtc_id: u32,
}

// ── DrmDisplay ───────────────────────────────────────────────────────────────

pub struct DrmDisplay {
    fd: i32,
    crtc_id: u32,
    connector_id: u32,
    mode: DrmModeModeinfo,
    fb_ids: [u32; 2],
    buf_ptrs: [*mut u8; 2],
    buf_len: usize,
    front: usize, // index of the buffer currently on screen (0 or 1)
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

// All access is serialised through the Framebuffer Mutex in mod.rs.
unsafe impl Send for DrmDisplay {}

impl DrmDisplay {
    /// Try to open `/dev/dri/card0` and set up DRM with double-buffered dumb
    /// buffers. Returns `None` if DRM is unavailable or setup fails.
    pub fn open() -> Option<Self> {
        let file = OpenOptions::new().read(true).write(true).open("/dev/dri/card0").ok()?;
        let fd = file.as_raw_fd();
        // Keep the File alive by leaking it (fd stays valid for process lifetime).
        std::mem::forget(file);

        // Try to become DRM master (not fatal if it fails).
        unsafe { drm_ioctl0(fd, DRM_IOCTL_SET_MASTER); }

        let (crtc_id, connector_id, mode) = find_connected_output(fd)?;
        let width = mode.hdisplay as u32;
        let height = mode.vdisplay as u32;

        // Create two dumb buffers.
        let buf0 = create_dumb_buffer(fd, width, height)?;
        let buf1 = create_dumb_buffer(fd, width, height)?;

        let stride = buf0.2;
        let buf_len = buf0.3;

        // Set the CRTC to display buffer 0.
        set_crtc(fd, crtc_id, buf0.0, connector_id, &mode)?;

        eprintln!(
            "vyoma-display: DRM {}x{} stride={} ({} KiB x2 dumb buffers)",
            width, height, stride, buf_len / 1024,
        );

        Some(DrmDisplay {
            fd,
            crtc_id,
            connector_id,
            mode,
            fb_ids: [buf0.0, buf1.0],
            buf_ptrs: [buf0.1, buf1.1],
            buf_len,
            front: 0,
            width,
            height,
            stride,
        })
    }

    /// Returns a mutable slice to the back buffer (the one NOT currently on screen).
    #[allow(dead_code)]
    pub fn back_buffer(&mut self) -> &mut [u8] {
        let idx = 1 - self.front;
        unsafe { std::slice::from_raw_parts_mut(self.buf_ptrs[idx], self.buf_len) }
    }

    /// Copy dirty rows from `src` into the DRM back buffer, then page flip.
    pub fn flip_with_dirty(&mut self, src: &[u8], row_top: u32, row_bot: u32) {
        if row_top >= row_bot { return; }
        let back_idx = 1 - self.front;
        let start = (row_top * self.stride) as usize;
        let end = ((row_bot * self.stride) as usize).min(self.buf_len);
        if start < end {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    src.as_ptr().add(start),
                    self.buf_ptrs[back_idx].add(start),
                    end - start,
                );
            }
        }
        self.page_flip(back_idx);
    }

    /// Issue a page flip ioctl, then wait for the flip event.
    fn page_flip(&mut self, buf_idx: usize) {
        let mut flip = DrmModeCrtcPageFlip {
            crtc_id: self.crtc_id,
            fb_id: self.fb_ids[buf_idx],
            flags: DRM_MODE_PAGE_FLIP_EVENT,
            _reserved: 0,
            user_data: 0,
        };
        let ret = unsafe {
            drm_ioctl(
                self.fd,
                DRM_IOCTL_MODE_PAGE_FLIP,
                &mut flip as *mut _ as *mut libc::c_void,
            )
        };
        if ret < 0 {
            // Page flip failed — fall back to SetCrtc (synchronous).
            set_crtc(
                self.fd,
                self.crtc_id,
                self.fb_ids[buf_idx],
                self.connector_id,
                &self.mode,
            );
            self.front = buf_idx;
            return;
        }
        // Wait for flip completion by reading the event.
        self.wait_flip_event();
        self.front = buf_idx;
    }

    /// Block until a page-flip-complete event arrives on the DRM fd.
    fn wait_flip_event(&self) {
        let mut buf = [0u8; 256];
        let n = unsafe {
            libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
        };
        if n < 0 {
            // Read failed — not critical, next flip will still work.
            return;
        }
        // We consumed the event; no further parsing needed.
    }
}

// ── Helper functions ─────────────────────────────────────────────────────────

/// Find a connected connector, its encoder, and the preferred mode.
/// Returns (crtc_id, connector_id, mode).
fn find_connected_output(fd: i32) -> Option<(u32, u32, DrmModeModeinfo)> {
    // First call: get counts.
    let mut res: DrmModeCardRes = unsafe { std::mem::zeroed() };
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }

    if res.count_connectors == 0 || res.count_crtcs == 0 {
        return None;
    }

    // Allocate arrays for IDs.
    let mut connector_ids = vec![0u32; res.count_connectors as usize];
    let mut crtc_ids = vec![0u32; res.count_crtcs as usize];
    let mut encoder_ids = vec![0u32; res.count_encoders as usize];

    res.connector_id_ptr = connector_ids.as_mut_ptr() as u64;
    res.crtc_id_ptr = crtc_ids.as_mut_ptr() as u64;
    res.encoder_id_ptr = encoder_ids.as_mut_ptr() as u64;

    // Second call: fill arrays.
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }

    // Find a connected connector with modes.
    for &conn_id in &connector_ids {
        if let Some(result) = try_connector(fd, conn_id, &crtc_ids) {
            return Some(result);
        }
    }
    None
}

/// Try a single connector: if connected and has modes, find crtc via encoder.
fn try_connector(
    fd: i32,
    conn_id: u32,
    crtc_ids: &[u32],
) -> Option<(u32, u32, DrmModeModeinfo)> {
    // First call to get counts.
    let mut conn: DrmModeGetConnector = unsafe { std::mem::zeroed() };
    conn.connector_id = conn_id;
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }

    if conn.connection != DRM_MODE_CONNECTED || conn.count_modes == 0 {
        return None;
    }

    // Allocate mode array and second call.
    let mut modes = vec![unsafe { std::mem::zeroed::<DrmModeModeinfo>() }; conn.count_modes as usize];
    let mut encoder_ids = vec![0u32; conn.count_encoders as usize];
    conn.modes_ptr = modes.as_mut_ptr() as u64;
    conn.encoders_ptr = encoder_ids.as_mut_ptr() as u64;

    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }

    // Pick the preferred mode, or the first one if none is marked preferred.
    let mode = modes
        .iter()
        .find(|m| m.type_ & DRM_MODE_TYPE_PREFERRED != 0)
        .or(modes.first())?;

    // Find CRTC via the current encoder, or pick the first available CRTC.
    let crtc_id = if conn.encoder_id != 0 {
        get_encoder_crtc(fd, conn.encoder_id).or_else(|| crtc_ids.first().copied())
    } else {
        crtc_ids.first().copied()
    }?;

    Some((crtc_id, conn_id, *mode))
}

/// Get the CRTC ID from an encoder.
fn get_encoder_crtc(fd: i32, encoder_id: u32) -> Option<u32> {
    let mut enc: DrmModeGetEncoder = unsafe { std::mem::zeroed() };
    enc.encoder_id = encoder_id;
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &mut enc as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }
    if enc.crtc_id != 0 { Some(enc.crtc_id) } else { None }
}

/// Create a dumb buffer and return (fb_id, mmap_ptr, pitch, size).
fn create_dumb_buffer(fd: i32, width: u32, height: u32) -> Option<(u32, *mut u8, u32, usize)> {
    let mut create = DrmModeCreateDumb {
        height,
        width,
        bpp: 32,
        flags: 0,
        handle: 0,
        pitch: 0,
        size: 0,
    };
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &mut create as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }

    // Add as framebuffer.
    let mut fb_cmd = DrmModeFbCmd {
        fb_id: 0,
        width,
        height,
        pitch: create.pitch,
        bpp: 32,
        depth: 24,
        handle: create.handle,
    };
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_ADDFB, &mut fb_cmd as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }

    // Map to userspace.
    let mut map = DrmModeMapDumb {
        handle: create.handle,
        _pad: 0,
        offset: 0,
    };
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mut map as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }

    let size = create.size as usize;
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            map.offset as libc::off_t,
        )
    };
    if ptr == libc::MAP_FAILED {
        return None;
    }

    Some((fb_cmd.fb_id, ptr as *mut u8, create.pitch, size))
}

/// Set the CRTC to display a given framebuffer.
fn set_crtc(
    fd: i32,
    crtc_id: u32,
    fb_id: u32,
    connector_id: u32,
    mode: &DrmModeModeinfo,
) -> Option<()> {
    let mut conn_ids = [connector_id];
    let mut crtc = DrmModeCrtc {
        set_connectors_ptr: conn_ids.as_mut_ptr() as u64,
        count_connectors: 1,
        crtc_id,
        fb_id,
        x: 0,
        y: 0,
        gamma_size: 0,
        mode_valid: 1,
        mode: *mode,
    };
    if unsafe { drm_ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &mut crtc as *mut _ as *mut libc::c_void) } < 0 {
        return None;
    }
    Some(())
}
