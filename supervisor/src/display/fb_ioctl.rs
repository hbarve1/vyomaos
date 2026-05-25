// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

// Linux framebuffer ioctl definitions used by open_fb.

pub const FBIOGET_VSCREENINFO: libc::Ioctl = 0x4600;

// fb_var_screeninfo — purely u32 fields (+ bitfield sub-structs of u32),
// so no cross-platform alignment surprises on x86_64.
#[repr(C)]
pub struct FbBitfield {
    pub offset:    u32,
    pub length:    u32,
    pub msb_right: u32,
}

#[repr(C)]
pub struct FbVarScreeninfo {
    pub xres:          u32,
    pub yres:          u32,
    pub xres_virtual:  u32,
    pub yres_virtual:  u32,
    pub xoffset:       u32,
    pub yoffset:       u32,
    pub bits_per_pixel: u32,
    pub grayscale:     u32,
    pub red:           FbBitfield,
    pub green:         FbBitfield,
    pub blue:          FbBitfield,
    pub transp:        FbBitfield,
    pub nonstd:        u32,
    pub activate:      u32,
    pub height:        u32, // physical mm — not pixel height
    pub width:         u32, // physical mm — not pixel width
    pub accel_flags:   u32,
    pub pixclock:      u32,
    pub left_margin:   u32,
    pub right_margin:  u32,
    pub upper_margin:  u32,
    pub lower_margin:  u32,
    pub hsync_len:     u32,
    pub vsync_len:     u32,
    pub sync:          u32,
    pub vmode:         u32,
    pub rotate:        u32,
    pub colorspace:    u32,
    pub reserved:      [u32; 4],
}
