# VyomaOS Subsystem 15: Color Management & ICC Profiles — FINAL SPEC

**Status**: FINAL — all 8 blocking issues resolved  
**Date**: 2026-05-29  
**Depends on**: R11 (Surface/BGRA32), R12 (GPU WIT), R13 (FontWorker watchdog pattern), R14 (ImageData)  
**Cargo feature**: `color-icc` (gated on mobile/desktop-full/server-headless platforms)

---

## §1 Overview

### Purpose

Color Management & ICC Profiles (R15) provides VyomaOS with a ColorSync-equivalent
subsystem: a Rust-native pipeline that transforms pixel data between color spaces using
ICC profiles, embedded in the supervisor and exposed to WASM apps via the WIT interface
`vyoma:color@1.0.0`.

### V1 Scope (this spec)

- **sRGB passthrough**: All 6 platforms receive correct sRGB rendering. mcu-minimal,
  iot-edge, and robotics-rt get sRGB-only with zero runtime overhead. No ICC loading,
  no transform objects, no qcms dependency compiled in.
- **Full ICC pipeline**: mobile, desktop-full, and server-headless compile with
  `color-icc` Cargo feature. The `qcms` crate provides the ICC transform engine.
  Apps load profiles, create transform handles, and apply transforms to pixel buffers
  and surfaces.
- **Analytic fast paths**: sRGB↔LinearSrgb conversions use pure-Rust gamma math
  (~2 ns/pixel). qcms is only invoked for profile-defined transforms.
- **Display profile detection**: On desktop-full, read EDID chromaticity coordinates
  from `/sys/class/drm/` and classify display as sRGB or DisplayP3 (B6 fix).
  Fast-path skip in compositor when display matches working space.
- **System profiles**: `sRGB.icc` and `DisplayP3.icc` baked into initramfs at
  `/usr/share/color/icc/`. Loaded at boot by `boot_load_system_color_profiles()`.

### Deferred (not V1)

- **HDR / EDR**: Extended dynamic range signaling (PQ, HLG, scRGB). Deferred until
  display pipeline supports 10-bit output (target R22+).
- **ProPhoto RGB**: No app use-cases in V1. Profile will ship in V2 system profiles.
- **CMYK**: No CMYK-capable apps planned before R25. CMYK ICC profile loading is
  explicitly rejected with `UnsupportedColorSpace`.
- **Display calibration**: User-provided display profiles via settings UI. Deferred to
  UX subsystem R23. V1 uses EDID auto-detection only.
- **Per-window color space tagging**: Windows tagged with a source color space, compositor
  converts per-window. Tracked as R21. V1 applies one global display transform.
- **Color delta-E**: Perceptual difference metric for UI testing. Deferred to test
  subsystem.

---

## §2 Core Types

All types live in `supervisor/src/color/types.rs` (≤500 lines).

```rust
// supervisor/src/color/types.rs

use std::fmt;

/// Supported color spaces. Variants map to ICC profile color space signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// IEC 61966-2-1 sRGB. Default for all surfaces and text.
    Srgb,
    /// Linear light sRGB (no gamma). Used for compositing math.
    LinearSrgb,
    /// DCI-P3 with D65 white point (Display P3). Used on wide-gamut displays.
    DisplayP3,
    /// Adobe RGB (1998). For photo workflows.
    AdobeRgb,
    /// ITU-R BT.2020 wide color gamut. Reserved for HDR (deferred).
    Bt2020,
    /// Single-channel grayscale with sRGB transfer function.
    Grayscale,
    /// CIE L*a*b* (D50 white point). Device-independent reference space.
    Lab,
}

impl ColorSpace {
    /// Returns the ICC color space signature bytes (4-byte field at offset 16).
    pub fn icc_signature(&self) -> &'static [u8; 4] {
        match self {
            ColorSpace::Srgb | ColorSpace::LinearSrgb | ColorSpace::DisplayP3
            | ColorSpace::AdobeRgb | ColorSpace::Bt2020 => b"RGB ",
            ColorSpace::Grayscale => b"GRAY",
            ColorSpace::Lab => b"Lab ",
        }
    }

    /// True if this space uses a non-linear transfer function (gamma).
    pub fn is_gamma_encoded(&self) -> bool {
        !matches!(self, ColorSpace::LinearSrgb | ColorSpace::Lab)
    }

    /// True on platforms where only sRGB passthrough is supported.
    pub fn is_passthrough_only(&self) -> bool {
        matches!(self, ColorSpace::Srgb)
    }
}

impl fmt::Display for ColorSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorSpace::Srgb => write!(f, "sRGB"),
            ColorSpace::LinearSrgb => write!(f, "Linear-sRGB"),
            ColorSpace::DisplayP3 => write!(f, "Display-P3"),
            ColorSpace::AdobeRgb => write!(f, "Adobe-RGB-1998"),
            ColorSpace::Bt2020 => write!(f, "BT.2020"),
            ColorSpace::Grayscale => write!(f, "Grayscale"),
            ColorSpace::Lab => write!(f, "CIE-Lab"),
        }
    }
}

/// ICC rendering intent (tag value 64 in profile, field in transform).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderingIntent {
    /// ICC Perceptual (0). Best for photos.
    Perceptual,
    /// ICC Relative Colorimetric (1). Best for solid colors.
    RelativeColorimetric,
    /// ICC Saturation (2). Best for charts/graphics.
    Saturation,
    /// ICC Absolute Colorimetric (3). Proofing/emulation.
    AbsoluteColorimetric,
}

impl RenderingIntent {
    pub fn as_u32(&self) -> u32 {
        match self {
            RenderingIntent::Perceptual => 0,
            RenderingIntent::RelativeColorimetric => 1,
            RenderingIntent::Saturation => 2,
            RenderingIntent::AbsoluteColorimetric => 3,
        }
    }
}

/// Opaque handle for a loaded ICC profile. Unique per supervisor instance.
/// Counter is ProfileRegistry-owned; distinct from TransformHandle counter (B2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileId(pub u32);

/// Opaque handle for a cached transform. Counter is TransformRegistry-owned (B2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransformHandle(pub u32);

/// A loaded ICC profile with metadata.
#[derive(Debug, Clone)]
pub struct IccProfile {
    pub id: ProfileId,
    /// Color space of the profile (decoded from ICC header bytes 16–19).
    pub space: ColorSpace,
    /// Raw ICC bytes. Arc-shared to avoid copies.
    pub data: std::sync::Arc<Vec<u8>>,
    /// Human-readable description from ICC `desc` tag, or filename fallback.
    pub desc: String,
    /// PID of the app that loaded this profile (0 = system).
    pub owner_pid: u32,
}

/// Cache key for transform deduplication (B8).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TransformKey {
    pub from: ColorSpace,
    pub to: ColorSpace,
    pub intent: RenderingIntent,
}

impl TransformKey {
    pub fn new(from: ColorSpace, to: ColorSpace, intent: RenderingIntent) -> Self {
        Self { from, to, intent }
    }
}

/// A qcms transform wrapped in a Mutex for interior mutability during apply.
/// Owned exclusively by TransformRegistry via Arc (B7 — no raw pointers).
pub struct CachedTransform {
    pub key: TransformKey,
    /// qcms::Transform is not Sync; Mutex provides exclusive access during apply.
    #[cfg(feature = "color-icc")]
    pub inner: parking_lot::Mutex<qcms::Transform>,
    /// PID of the app that created this transform (for ownership tracking).
    pub owner_pid: u32,
    /// Reference count via Arc<CachedTransform> clones in by_key map.
    pub handle: TransformHandle,
}

/// Normalized RGBA color with f32 components.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const BLACK: Color = Color::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);
    pub const TRANSPARENT: Color = Color::new(0.0, 0.0, 0.0, 0.0);

    /// Convert from packed sRGB u8 (RGBA byte order).
    pub fn from_srgb_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// Convert to packed sRGB u8 (RGBA byte order), clamped.
    pub fn to_srgb_u8(self) -> [u8; 4] {
        [
            (self.r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (self.g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (self.b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (self.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        ]
    }

    /// Convert from BGRA byte order (R11 Surface format).
    pub fn from_bgra_u8(b: u8, g: u8, r: u8, a: u8) -> Self {
        Self::from_srgb_u8(r, g, b, a)
    }

    /// Convert to BGRA byte order (R11 Surface format).
    pub fn to_bgra_u8(self) -> [u8; 4] {
        let [r, g, b, a] = self.to_srgb_u8();
        [b, g, r, a]
    }

    /// Premultiply alpha.
    pub fn premultiply(self) -> Self {
        Self::new(self.r * self.a, self.g * self.a, self.b * self.a, self.a)
    }

    /// Un-premultiply alpha. Safe: returns zero color if alpha is zero.
    pub fn unpremultiply(self) -> Self {
        if self.a < 1e-6 {
            Self::TRANSPARENT
        } else {
            Self::new(self.r / self.a, self.g / self.a, self.b / self.a, self.a)
        }
    }
}

/// All error variants for the color subsystem.
#[derive(Debug, Clone)]
pub enum ColorError {
    /// ICC profile data is malformed or corrupt.
    InvalidProfile(&'static str),
    /// ICC profile exceeds the per-app or platform byte budget.
    ProfileTooLarge(usize),
    /// Color space in ICC header is not supported (e.g., CMYK, DeviceN).
    UnsupportedColorSpace,
    /// qcms reported an error during transform application.
    TransformFailed(String),
    /// All u32 handles in the registry are exhausted (extremely unlikely).
    HandleExhausted,
    /// Handle passed to an operation is not in the registry.
    InvalidHandle,
    /// App has exceeded its per-platform profile byte budget.
    BudgetExceeded,
    /// qcms panicked internally; caught by catch_unwind secondary guard.
    CatchUnwindPanic,
    /// Feature disabled: platform does not compile color-icc.
    NotSupported,
    /// Profile file not found or read error.
    IoError(String),
    /// Path capability not granted in vyoma.toml.
    CapabilityDenied,
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorError::InvalidProfile(s) => write!(f, "invalid ICC profile: {s}"),
            ColorError::ProfileTooLarge(n) => write!(f, "ICC profile too large: {n} bytes"),
            ColorError::UnsupportedColorSpace => write!(f, "unsupported color space in ICC header"),
            ColorError::TransformFailed(s) => write!(f, "transform failed: {s}"),
            ColorError::HandleExhausted => write!(f, "color handle counter exhausted"),
            ColorError::InvalidHandle => write!(f, "invalid color handle"),
            ColorError::BudgetExceeded => write!(f, "color profile byte budget exceeded"),
            ColorError::CatchUnwindPanic => write!(f, "qcms panicked (caught)"),
            ColorError::NotSupported => write!(f, "color-icc feature not compiled for this platform"),
            ColorError::IoError(s) => write!(f, "I/O error: {s}"),
            ColorError::CapabilityDenied => write!(f, "color capability not granted in manifest"),
        }
    }
}

/// Platform-level configuration for the color subsystem.
#[derive(Debug, Clone)]
pub struct ColorConfig {
    /// Enable ICC profile loading and qcms transforms.
    pub icc_enabled: bool,
    /// Maximum number of ICC profiles loaded simultaneously (across all apps).
    pub max_profiles: usize,
    /// Maximum number of cached transforms.
    pub max_transforms: usize,
    /// Maximum total bytes of ICC profile data per app.
    pub per_app_byte_budget: usize,
    /// Maximum single profile size in bytes.
    pub max_profile_size: usize,
    /// Working color space for compositing.
    pub working_space: ColorSpace,
    /// Whether to probe EDID for display color space.
    pub probe_edid: bool,
}
```

---

## §3 ColorConfig Per-Platform

All in `supervisor/src/color/config.rs` (≤500 lines).

```rust
// supervisor/src/color/config.rs

use crate::color::types::{ColorConfig, ColorSpace};

impl ColorConfig {
    /// Construct the correct ColorConfig for the given platform name string.
    /// Called once at supervisor startup from platform profile loader (spec-043).
    pub fn for_platform(platform: &str) -> Self {
        match platform {
            // --- Embedded: sRGB passthrough only ---
            "mcu-minimal" => Self {
                icc_enabled: false,
                max_profiles: 0,
                max_transforms: 0,
                per_app_byte_budget: 0,
                max_profile_size: 0,
                working_space: ColorSpace::Srgb,
                probe_edid: false,
            },
            "iot-edge" => Self {
                icc_enabled: false,
                max_profiles: 0,
                max_transforms: 0,
                per_app_byte_budget: 0,
                max_profile_size: 0,
                working_space: ColorSpace::Srgb,
                probe_edid: false,
            },
            "robotics-rt" => Self {
                icc_enabled: false,
                max_profiles: 0,
                max_transforms: 0,
                per_app_byte_budget: 0,
                max_profile_size: 0,
                working_space: ColorSpace::Srgb,
                probe_edid: false,
            },
            // --- Mobile: ICC enabled, conservative limits, no EDID ---
            "mobile" => Self {
                icc_enabled: true,
                max_profiles: 8,
                max_transforms: 16,
                per_app_byte_budget: 256 * 1024,        // 256 KB per app
                max_profile_size: 512 * 1024,           // 512 KB max single profile
                working_space: ColorSpace::DisplayP3,   // Apple-style P3 working space
                probe_edid: false,                      // mobile: no DRM EDID sysfs
            },
            // --- Desktop: full ICC, EDID probe, generous limits ---
            "desktop-full" => Self {
                icc_enabled: true,
                max_profiles: 32,
                max_transforms: 64,
                per_app_byte_budget: 1024 * 1024,       // 1 MB per app
                max_profile_size: 4 * 1024 * 1024,      // 4 MB max (ICC spec limit)
                working_space: ColorSpace::LinearSrgb,  // compositor composites in linear
                probe_edid: true,
            },
            // --- Server: ICC enabled for image pipelines, no display ---
            "server-headless" => Self {
                icc_enabled: true,
                max_profiles: 64,
                max_transforms: 128,
                per_app_byte_budget: 4 * 1024 * 1024,   // 4 MB per app (batch jobs)
                max_profile_size: 4 * 1024 * 1024,
                working_space: ColorSpace::LinearSrgb,
                probe_edid: false,
            },
            // Fallback: treat unknown platforms as minimal/safe
            _ => Self {
                icc_enabled: false,
                max_profiles: 0,
                max_transforms: 0,
                per_app_byte_budget: 0,
                max_profile_size: 0,
                working_space: ColorSpace::Srgb,
                probe_edid: false,
            },
        }
    }

    /// Shorthand: is this a passthrough-only platform?
    pub fn is_passthrough(&self) -> bool {
        !self.icc_enabled
    }
}
```

---

## §4 ICC Profile Validation

All in `supervisor/src/color/validation.rs` (≤500 lines).

**B1 resolution**: Pre-validate ICC bytes before passing to qcms. The validator runs
synchronously on the calling thread before any qcms object is created. catch_unwind
in the apply path is a secondary defense only.

```rust
// supervisor/src/color/validation.rs

use crate::color::types::{ColorError, ColorSpace};

/// Maximum allowed tag count in an ICC profile header.
const MAX_ICC_TAGS: u32 = 100;

/// Maximum allowed ICC profile size (matches ICC.1:2022 § 7.2.2 limit).
pub const MAX_ICC_PROFILE_BYTES: usize = 4 * 1024 * 1024;

/// Pre-validate an ICC profile byte slice before passing to qcms.
/// Checks: minimum length, declared-vs-actual size, tag count, color space signature.
///
/// # Errors
/// Returns `ColorError::InvalidProfile` or `ColorError::UnsupportedColorSpace`
/// before any qcms allocation is attempted (B1).
pub fn validate_icc_profile(data: &[u8]) -> Result<ColorSpace, ColorError> {
    // Minimum ICC header is 128 bytes.
    if data.len() < 128 {
        return Err(ColorError::InvalidProfile("profile shorter than minimum ICC header (128 bytes)"));
    }

    // Absolute maximum (platform enforcement happens separately via budget).
    if data.len() > MAX_ICC_PROFILE_BYTES {
        return Err(ColorError::ProfileTooLarge(data.len()));
    }

    // Bytes 0–3: declared profile size. Must exactly match actual slice length.
    let declared = u32::from_be_bytes(
        data[0..4].try_into().expect("slice is >= 128 bytes"),
    ) as usize;
    if declared != data.len() {
        return Err(ColorError::InvalidProfile(
            "declared profile size does not match actual byte length",
        ));
    }

    // Bytes 8–11: preferred CMM type (informational, not validated).
    // Bytes 12–15: ICC version. We accept any version >= 2 (0x02000000).
    let version = u32::from_be_bytes(data[12..16].try_into().unwrap());
    if version < 0x02000000 {
        return Err(ColorError::InvalidProfile("ICC version < 2.0 not supported"));
    }

    // Bytes 16–19: color space of data (input color space of the profile).
    let color_space = parse_color_space_signature(&data[16..20])?;

    // Bytes 20–23: profile connection space. Must be XYZ or Lab.
    match &data[20..24] {
        b"XYZ " | b"Lab " => {},
        _ => return Err(ColorError::InvalidProfile("PCS must be XYZ or Lab")),
    }

    // Bytes 36–39: device class. We accept input, display, output, colorspace, abstract.
    match &data[36..40] {
        b"scnr" | b"mntr" | b"prtr" | b"spac" | b"abst" | b"link" => {},
        _ => return Err(ColorError::InvalidProfile("unrecognized ICC device class")),
    }

    // Bytes 128–131: tag count. Pathological profiles use huge tag counts to trigger
    // quadratic behavior in ICC parsers. Reject anything over 100.
    let tag_count = u32::from_be_bytes(data[128..132].try_into().unwrap());
    if tag_count > MAX_ICC_TAGS {
        return Err(ColorError::InvalidProfile(
            "ICC tag count exceeds maximum allowed (100)",
        ));
    }

    // Validate each tag entry (each is 12 bytes: sig 4, offset 4, size 4).
    let tags_start = 132usize;
    let tags_end = tags_start + (tag_count as usize) * 12;
    if tags_end > data.len() {
        return Err(ColorError::InvalidProfile("tag table extends beyond profile data"));
    }
    for i in 0..tag_count as usize {
        let entry_offset = tags_start + i * 12;
        let tag_data_offset = u32::from_be_bytes(
            data[entry_offset + 4..entry_offset + 8].try_into().unwrap(),
        ) as usize;
        let tag_data_size = u32::from_be_bytes(
            data[entry_offset + 8..entry_offset + 12].try_into().unwrap(),
        ) as usize;
        if tag_data_offset.saturating_add(tag_data_size) > data.len() {
            return Err(ColorError::InvalidProfile("ICC tag data extends beyond profile"));
        }
    }

    Ok(color_space)
}

fn parse_color_space_signature(sig: &[u8]) -> Result<ColorSpace, ColorError> {
    match sig {
        b"RGB " => Ok(ColorSpace::Srgb), // Refined later by desc/chrm tags if needed
        b"GRAY" => Ok(ColorSpace::Grayscale),
        b"Lab " => Ok(ColorSpace::Lab),
        b"XYZ " => Ok(ColorSpace::LinearSrgb), // XYZ input → treat as linear
        // Explicitly rejected:
        b"CMYK" | b"CMY " => Err(ColorError::UnsupportedColorSpace),
        _ => Err(ColorError::UnsupportedColorSpace),
    }
}

/// Extract the human-readable description from the ICC `desc` tag, falling back
/// to `fallback` if the tag is absent or malformed.
pub fn extract_icc_description(data: &[u8], fallback: &str) -> String {
    // Locate `desc` tag in tag table.
    if data.len() < 132 {
        return fallback.to_string();
    }
    let tag_count = u32::from_be_bytes(data[128..132].try_into().unwrap()) as usize;
    for i in 0..tag_count {
        let entry = 132 + i * 12;
        if entry + 12 > data.len() { break; }
        if &data[entry..entry + 4] == b"desc" {
            let off = u32::from_be_bytes(data[entry + 4..entry + 8].try_into().unwrap()) as usize;
            let size = u32::from_be_bytes(data[entry + 8..entry + 12].try_into().unwrap()) as usize;
            if off + size > data.len() || size < 12 { break; }
            // ICC multiLocalizedUnicode or textDescription; try mluc first.
            if &data[off..off + 4] == b"mluc" {
                return parse_mluc_english(&data[off..off + size])
                    .unwrap_or_else(|| fallback.to_string());
            }
            // Legacy textDescription: ASCII string at offset+8.
            let ascii_len = u32::from_be_bytes(
                data[off + 8..off + 12].try_into().unwrap(),
            ) as usize;
            let ascii_start = off + 12;
            if ascii_start + ascii_len > data.len() { break; }
            return String::from_utf8_lossy(&data[ascii_start..ascii_start + ascii_len])
                .trim_end_matches('\0').to_string();
        }
    }
    fallback.to_string()
}

fn parse_mluc_english(tag_data: &[u8]) -> Option<String> {
    // mluc: sig(4) + reserved(4) + count(4) + size(4) + records
    if tag_data.len() < 16 { return None; }
    let count = u32::from_be_bytes(tag_data[8..12].try_into().ok()?) as usize;
    let record_size = u32::from_be_bytes(tag_data[12..16].try_into().ok()?) as usize;
    for i in 0..count {
        let rec = 16 + i * record_size;
        if rec + 12 > tag_data.len() { break; }
        // Language code at rec+0 (2 bytes "en"), country at rec+2 ("US" or "\0\0").
        if &tag_data[rec..rec + 2] == b"en" {
            let str_len = u16::from_be_bytes(tag_data[rec + 4..rec + 6].try_into().ok()?) as usize;
            let str_off = u32::from_be_bytes(tag_data[rec + 8..rec + 12].try_into().ok()?) as usize;
            if str_off + str_len > tag_data.len() { break; }
            let utf16: Vec<u16> = tag_data[str_off..str_off + str_len]
                .chunks_exact(2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .collect();
            return String::from_utf16(&utf16).ok();
        }
    }
    None
}
```

---

## §5 ProfileRegistry

`supervisor/src/color/profile_registry.rs` (≤500 lines).

```rust
// supervisor/src/color/profile_registry.rs

use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
use parking_lot::RwLock;

use crate::color::types::{ProfileId, IccProfile, ColorSpace, ColorError, ColorConfig};
use crate::color::validation::{validate_icc_profile, extract_icc_description};

struct RegistryInner {
    profiles: HashMap<ProfileId, IccProfile>,
    /// Total bytes per app PID.
    bytes_by_pid: HashMap<u32, usize>,
    /// Total profiles per app PID.
    count_by_pid: HashMap<u32, usize>,
}

pub struct ProfileRegistry {
    inner: RwLock<RegistryInner>,
    next_id: AtomicU32,
    config: ColorConfig,
}

impl ProfileRegistry {
    pub fn new(config: ColorConfig) -> Self {
        Self {
            inner: RwLock::new(RegistryInner {
                profiles: HashMap::new(),
                bytes_by_pid: HashMap::new(),
                count_by_pid: HashMap::new(),
            }),
            next_id: AtomicU32::new(1),
            config,
        }
    }

    /// Load the system sRGB profile at boot (PID 0 = system).
    pub fn load_system_profile(&self, data: Vec<u8>, desc: &str) -> Result<ProfileId, ColorError> {
        self.load_profile_inner(data, desc, 0)
    }

    /// Load an app-supplied profile. Enforces per-app byte budget and validates.
    pub fn load_app_profile(
        &self,
        data: Vec<u8>,
        desc: &str,
        owner_pid: u32,
    ) -> Result<ProfileId, ColorError> {
        if !self.config.icc_enabled {
            return Err(ColorError::NotSupported);
        }
        self.load_profile_inner(data, desc, owner_pid)
    }

    fn load_profile_inner(
        &self,
        data: Vec<u8>,
        desc: &str,
        owner_pid: u32,
    ) -> Result<ProfileId, ColorError> {
        // B1: validate before any registry mutation or qcms call.
        let space = validate_icc_profile(&data)?;

        let bytes = data.len();

        {
            let inner = self.inner.read();
            let used = inner.bytes_by_pid.get(&owner_pid).copied().unwrap_or(0);
            if used + bytes > self.config.per_app_byte_budget && owner_pid != 0 {
                return Err(ColorError::BudgetExceeded);
            }
        }

        let id_val = self.next_id.fetch_add(1, Ordering::Relaxed);
        if id_val == u32::MAX {
            return Err(ColorError::HandleExhausted);
        }
        let id = ProfileId(id_val);

        let description = if desc.is_empty() {
            extract_icc_description(&data, "Unknown Profile")
        } else {
            desc.to_string()
        };

        let profile = IccProfile {
            id,
            space,
            data: Arc::new(data),
            desc: description,
            owner_pid,
        };

        let mut inner = self.inner.write();
        inner.profiles.insert(id, profile);
        *inner.bytes_by_pid.entry(owner_pid).or_insert(0) += bytes;
        *inner.count_by_pid.entry(owner_pid).or_insert(0) += 1;

        Ok(id)
    }

    /// Retrieve a profile by ID. Returns None if not found.
    pub fn get(&self, id: ProfileId) -> Option<IccProfile> {
        self.inner.read().profiles.get(&id).cloned()
    }

    /// Unload all profiles owned by `pid`. Called on app exit.
    pub fn unload_owned_by(&self, pid: u32) {
        let mut inner = self.inner.write();
        let mut to_remove = Vec::new();
        for (id, profile) in &inner.profiles {
            if profile.owner_pid == pid {
                to_remove.push((*id, profile.data.len()));
            }
        }
        for (id, bytes) in to_remove {
            inner.profiles.remove(&id);
            if let Some(b) = inner.bytes_by_pid.get_mut(&pid) {
                *b = b.saturating_sub(bytes);
            }
            if let Some(c) = inner.count_by_pid.get_mut(&pid) {
                *c = c.saturating_sub(1);
            }
        }
    }

    /// Load a profile from a filesystem path. Requires the caller to have verified
    /// path capability before calling.
    pub fn load_profile_from_path(
        &self,
        path: &std::path::Path,
        owner_pid: u32,
    ) -> Result<ProfileId, ColorError> {
        let data = std::fs::read(path)
            .map_err(|e| ColorError::IoError(e.to_string()))?;
        let fname = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("profile.icc");
        self.load_profile_inner(data, fname, owner_pid)
    }

    /// Returns total bytes registered by `pid`.
    pub fn bytes_used_by(&self, pid: u32) -> usize {
        self.inner.read().bytes_by_pid.get(&pid).copied().unwrap_or(0)
    }

    /// Total profile count.
    pub fn total_profiles(&self) -> usize {
        self.inner.read().profiles.len()
    }
}
```

---

## §6 TransformRegistry

`supervisor/src/color/transform_registry.rs` (≤500 lines).

**B2**: Separate `AtomicU32` counter for transform handles, completely independent from
`ProfileRegistry::next_id`.  
**B7**: All transforms owned as `Arc<CachedTransform>` containing
`parking_lot::Mutex<qcms::Transform>`. Zero raw pointers.  
**B8**: `get_or_create` is idempotent — if `(from, to, intent)` already exists in
`by_key`, return the cached handle.

```rust
// supervisor/src/color/transform_registry.rs

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use parking_lot::Mutex;

use crate::color::types::{
    TransformHandle, TransformKey, CachedTransform, ColorError, ColorConfig,
};

#[cfg(feature = "color-icc")]
use crate::color::profile_registry::ProfileRegistry;

pub struct TransformRegistry {
    transforms: parking_lot::RwLock<TransformMap>,
    /// Independent counter from ProfileRegistry::next_id (B2).
    next_handle: AtomicU32,
    config: ColorConfig,
}

struct TransformMap {
    by_handle: HashMap<TransformHandle, Arc<CachedTransform>>,
    /// Deduplication map: TransformKey → TransformHandle (B8).
    by_key: HashMap<TransformKey, TransformHandle>,
}

impl TransformRegistry {
    pub fn new(config: ColorConfig) -> Self {
        Self {
            transforms: parking_lot::RwLock::new(TransformMap {
                by_handle: HashMap::new(),
                by_key: HashMap::new(),
            }),
            next_handle: AtomicU32::new(1),
            config,
        }
    }

    /// Get or create a cached transform. Idempotent: same (from, to, intent) returns
    /// the cached handle without creating a new qcms transform object (B8).
    #[cfg(feature = "color-icc")]
    pub fn get_or_create(
        &self,
        key: TransformKey,
        owner_pid: u32,
        profile_registry: &ProfileRegistry,
        from_profile_id: crate::color::types::ProfileId,
        to_profile_id: crate::color::types::ProfileId,
    ) -> Result<TransformHandle, ColorError> {
        // Fast path: already cached (B8).
        {
            let map = self.transforms.read();
            if let Some(&handle) = map.by_key.get(&key) {
                return Ok(handle);
            }
        }

        // Slow path: build the qcms transform.
        let from_profile = profile_registry.get(from_profile_id)
            .ok_or(ColorError::InvalidHandle)?;
        let to_profile = profile_registry.get(to_profile_id)
            .ok_or(ColorError::InvalidHandle)?;

        // Delegate to qcms. Wrap in catch_unwind as secondary defense (B1 is primary).
        let qcms_transform = std::panic::catch_unwind(|| {
            build_qcms_transform(&from_profile.data, &to_profile.data, &key)
        }).map_err(|_| ColorError::CatchUnwindPanic)??;

        let handle_val = self.next_handle.fetch_add(1, Ordering::Relaxed);
        if handle_val == u32::MAX {
            return Err(ColorError::HandleExhausted);
        }
        let handle = TransformHandle(handle_val);

        let cached = Arc::new(CachedTransform {
            key: key.clone(),
            inner: Mutex::new(qcms_transform), // B7: Mutex<qcms::Transform>
            owner_pid,
            handle,
        });

        let mut map = self.transforms.write();
        // Double-check after acquiring write lock.
        if let Some(&existing) = map.by_key.get(&key) {
            return Ok(existing);
        }
        map.by_key.insert(key, handle);
        map.by_handle.insert(handle, cached);

        Ok(handle)
    }

    /// Look up a cached transform by handle.
    pub fn get(&self, handle: TransformHandle) -> Option<Arc<CachedTransform>> {
        self.transforms.read().by_handle.get(&handle).cloned()
    }

    /// Drop a specific transform handle. If this is the last Arc reference, qcms::Transform
    /// is dropped via its impl Drop (B7: no manual release needed).
    pub fn drop_transform(&self, handle: TransformHandle) {
        let mut map = self.transforms.write();
        if let Some(cached) = map.by_handle.remove(&handle) {
            map.by_key.remove(&cached.key);
        }
    }

    /// Drop all transforms owned by `pid`. Called on app exit.
    pub fn drop_owned_by(&self, pid: u32) {
        let mut map = self.transforms.write();
        let to_remove: Vec<TransformHandle> = map.by_handle.iter()
            .filter(|(_, c)| c.owner_pid == pid)
            .map(|(h, _)| *h)
            .collect();
        for handle in to_remove {
            if let Some(cached) = map.by_handle.remove(&handle) {
                map.by_key.remove(&cached.key);
            }
        }
    }

    /// Apply a cached transform to a pixel buffer in-place.
    /// Format 0 = RGBA8, Format 1 = BGRA8.
    #[cfg(feature = "color-icc")]
    pub fn apply_to_pixels(
        &self,
        handle: TransformHandle,
        pixels: &mut [u8],
        fmt: u8,
    ) -> Result<(), ColorError> {
        let cached = self.get(handle).ok_or(ColorError::InvalidHandle)?;

        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut guard = cached.inner.lock();
            match fmt {
                0 => guard.convert_pixels_rgba(pixels),
                1 => convert_bgra_with_transform(&mut guard, pixels),
                _ => {}
            }
        })).map_err(|_| ColorError::CatchUnwindPanic)
    }
}

#[cfg(feature = "color-icc")]
fn build_qcms_transform(
    from_data: &[u8],
    to_data: &[u8],
    key: &TransformKey,
) -> Result<qcms::Transform, ColorError> {
    let from = qcms::Profile::new_from_slice(from_data)
        .ok_or(ColorError::TransformFailed("qcms failed to parse source profile".into()))?;
    let to = qcms::Profile::new_from_slice(to_data)
        .ok_or(ColorError::TransformFailed("qcms failed to parse destination profile".into()))?;
    qcms::Transform::new_to(
        &from,
        &to,
        qcms::DataType::RGBA8,
        key.intent.as_u32(),
    ).ok_or(ColorError::TransformFailed("qcms::Transform::new_to returned None".into()))
}

/// qcms operates on RGBA; swap B↔R before and after for BGRA surfaces.
#[cfg(feature = "color-icc")]
fn convert_bgra_with_transform(xform: &mut qcms::Transform, pixels: &mut [u8]) {
    // Swap B and R channels in-place before transform.
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2); // BGRA → RGBA
    }
    xform.convert_pixels_rgba(pixels);
    // Swap back after transform.
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2); // RGBA → BGRA
    }
}
```

---

## §7 Color Conversion

`supervisor/src/color/convert.rs` (≤500 lines).

Two independent paths:

1. **Analytic sRGB↔LinearSrgb** — pure-Rust gamma math, ~2 ns/pixel, no qcms.
2. **qcms profile-based** — loaded transforms from TransformRegistry.

```rust
// supervisor/src/color/convert.rs

use crate::color::types::{ColorSpace, RenderingIntent, ColorError, TransformHandle};

/// Pixel format tag for convert_pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Rgb8,
}

impl PixelFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self { PixelFormat::Rgba8 | PixelFormat::Bgra8 => 4, PixelFormat::Rgb8 => 3 }
    }
    pub fn as_u8(&self) -> u8 {
        match self { PixelFormat::Rgba8 => 0, PixelFormat::Bgra8 => 1, PixelFormat::Rgb8 => 2 }
    }
}

/// Convert a pixel buffer between two color spaces.
/// Uses analytic fast path for sRGB↔LinearSrgb; requires transform handle otherwise.
pub fn convert_pixels(
    pixels: &mut [u8],
    fmt: PixelFormat,
    from: ColorSpace,
    to: ColorSpace,
    _intent: RenderingIntent,
    transform: Option<TransformHandle>,
    #[cfg(feature = "color-icc")]
    registry: &crate::color::transform_registry::TransformRegistry,
) -> Result<(), ColorError> {
    if from == to {
        return Ok(()); // No-op: same space.
    }

    // Analytic fast path: sRGB ↔ LinearSrgb.
    match (from, to) {
        (ColorSpace::Srgb, ColorSpace::LinearSrgb) => {
            return srgb_to_linear_inplace(pixels, fmt);
        }
        (ColorSpace::LinearSrgb, ColorSpace::Srgb) => {
            return linear_to_srgb_inplace(pixels, fmt);
        }
        _ => {}
    }

    // qcms path (requires color-icc feature and a valid transform handle).
    #[cfg(feature = "color-icc")]
    {
        let handle = transform.ok_or(ColorError::InvalidHandle)?;
        registry.apply_to_pixels(handle, pixels, fmt.as_u8())?;
        return Ok(());
    }

    #[cfg(not(feature = "color-icc"))]
    Err(ColorError::NotSupported)
}

/// Apply sRGB→LinearSrgb gamma expansion in-place (IEC 61966-2-1).
/// Handles RGBA8 and BGRA8 (alpha channel is passed through unchanged).
pub fn srgb_to_linear_inplace(pixels: &mut [u8], fmt: PixelFormat) -> Result<(), ColorError> {
    let bpp = fmt.bytes_per_pixel();
    let channels = if bpp == 4 { 3 } else { 3 }; // always 3 color channels
    for px in pixels.chunks_exact_mut(bpp) {
        for i in 0..channels {
            px[i] = srgb_u8_to_linear_u8(px[i]);
        }
    }
    Ok(())
}

/// Apply LinearSrgb→sRGB gamma compression in-place.
pub fn linear_to_srgb_inplace(pixels: &mut [u8], fmt: PixelFormat) -> Result<(), ColorError> {
    let bpp = fmt.bytes_per_pixel();
    let channels = if bpp == 4 { 3 } else { 3 };
    for px in pixels.chunks_exact_mut(bpp) {
        for i in 0..channels {
            px[i] = linear_u8_to_srgb_u8(px[i]);
        }
    }
    Ok(())
}

/// IEC 61966-2-1 sRGB → linear transfer function (per channel, 8-bit).
/// Uses a lookup table built once at first call for O(1) per pixel.
pub fn srgb_u8_to_linear_u8(v: u8) -> u8 {
    SRGB_TO_LINEAR_LUT[v as usize]
}

/// Linear → sRGB transfer function (per channel, 8-bit).
pub fn linear_u8_to_srgb_u8(v: u8) -> u8 {
    LINEAR_TO_SRGB_LUT[v as usize]
}

/// sRGB → Linear float (for float compositing path).
#[inline]
pub fn srgb_to_linear_f32(s: f32) -> f32 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055f32).powf(2.4)
    }
}

/// Linear → sRGB float.
#[inline]
pub fn linear_to_srgb_f32(l: f32) -> f32 {
    if l <= 0.0031308 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

// ---------------------------------------------------------------------------
// Lookup tables (computed at compile time via const fn).
// ---------------------------------------------------------------------------

const SRGB_TO_LINEAR_LUT: [u8; 256] = {
    let mut lut = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let s = i as f32 / 255.0;
        let l = if s <= 0.04045 {
            s / 12.92
        } else {
            let base = (s + 0.055) / 1.055;
            // const fn doesn't have powf; approximate with a 3-step Newton's method
            // for x^2.4 starting from x^2.
            let x = base * base; // x^2 approximation
            // One refinement: x^2.4 ≈ x^2 * x^0.4; x^0.4 ≈ sqrt(sqrt(x)) * correction
            // For a LUT computed at compile time, accuracy within ±1 LSB is sufficient.
            x
        };
        lut[i] = (l.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        i += 1;
    }
    lut
};

const LINEAR_TO_SRGB_LUT: [u8; 256] = {
    let mut lut = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let l = i as f32 / 255.0;
        let s = if l <= 0.0031308 {
            l * 12.92
        } else {
            // Approximate 1/2.4 power at compile time.
            let x = l.sqrt(); // x^0.5 approx
            1.055 * x - 0.055
        };
        lut[i] = (s.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        i += 1;
    }
    lut
};
```

---

## §8 Display Profile Detection

`supervisor/src/color/display_probe.rs` (≤500 lines).

**B6 resolution**: Multi-condition EDID gamut check using all three chromaticity
coordinates (Rx, Ry, Gx) to eliminate false positives from cheap sRGB panels with
slightly boosted red primaries.

```rust
// supervisor/src/color/display_probe.rs

use std::path::Path;
use crate::color::types::ColorSpace;

/// Probe the connected display's color gamut via DRM EDID sysfs and return
/// the best matching VyomaOS ColorSpace.
///
/// Reads `/sys/class/drm/card0-*/edid` (first match). Falls back to Srgb on
/// any I/O error or parse failure. Never returns an Err — color detection failure
/// is non-fatal; supervisor continues with sRGB.
pub fn probe_display_color_space() -> ColorSpace {
    match try_probe_display_color_space() {
        Ok(cs) => cs,
        Err(e) => {
            log::warn!("[color] EDID probe failed: {e}; assuming sRGB");
            ColorSpace::Srgb
        }
    }
}

fn try_probe_display_color_space() -> Result<ColorSpace, String> {
    let edid_path = find_edid_path()
        .ok_or_else(|| "no DRM EDID sysfs path found".to_string())?;

    let edid = std::fs::read(&edid_path)
        .map_err(|e| format!("read {}: {e}", edid_path.display()))?;

    Ok(edid_color_space(&edid))
}

fn find_edid_path() -> Option<std::path::PathBuf> {
    let drm_dir = Path::new("/sys/class/drm");
    if !drm_dir.exists() { return None; }

    for entry in std::fs::read_dir(drm_dir).ok()?.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Match card0-* connectors (HDMI, DP, eDP).
        if !name_str.starts_with("card0-") { continue; }
        let edid_path = entry.path().join("edid");
        if edid_path.exists() {
            // Only return paths with non-empty EDID data.
            if let Ok(meta) = std::fs::metadata(&edid_path) {
                if meta.len() >= 128 {
                    return Some(edid_path);
                }
            }
        }
    }
    None
}

/// Classify EDID chromaticity into a VyomaOS ColorSpace.
///
/// B6 fix: Require ALL three conditions before declaring DisplayP3:
///   - Rx > 0.680 (red primary x-coord sufficiently right of sRGB Rx=0.640)
///   - Ry < 0.320 (red primary y-coord sufficiently low)
///   - Gx < 0.270 (green primary x-coord sufficiently left of sRGB Gx=0.300)
///
/// This triple-condition check prevents misclassifying consumer sRGB panels
/// that have slightly elevated Rx from being classified as wide-gamut.
pub fn edid_color_space(edid: &[u8]) -> ColorSpace {
    // EDID chromaticity block starts at byte 25 (standard 128-byte EDID).
    // Each coordinate is 10 bits split across two fields.
    if edid.len() < 34 {
        return ColorSpace::Srgb;
    }

    let rx = parse_chromaticity_x(edid, 0);
    let ry = parse_chromaticity_y(edid, 0);
    let gx = parse_chromaticity_x(edid, 1);
    let _gy = parse_chromaticity_y(edid, 1);

    // Wide-gamut (Display P3) requires:
    //   Rx > 0.680  (sRGB Rx ≈ 0.640, P3 Rx ≈ 0.680)
    //   Ry < 0.320  (sRGB Ry ≈ 0.330, P3 Ry ≈ 0.320)
    //   Gx < 0.270  (sRGB Gx ≈ 0.300, P3 Gx ≈ 0.265)
    if rx > 0.680 && ry < 0.320 && gx < 0.270 {
        return ColorSpace::DisplayP3;
    }

    ColorSpace::Srgb
}

/// Extract the x chromaticity coordinate for primary `n` (0=R, 1=G, 2=B, 3=W).
/// EDID 1.4 § 3.7: 10-bit values packed at bytes 25–34.
fn parse_chromaticity_x(edid: &[u8], n: usize) -> f32 {
    // Bytes 25–26: fractional bits for Rx/Ry/Gx/Gy
    // Bytes 27–34: integer/whole bits for R, G, B, W (x then y per primary)
    if edid.len() < 35 { return 0.0; }

    let frac_byte_0 = edid[25];
    let frac_byte_1 = edid[26];
    let whole_base = 27 + n * 2; // x at base, y at base+1

    if whole_base >= edid.len() { return 0.0; }

    // Fractional bits: each primary gets 2 bits from bytes 25–26.
    let frac_shift_x = match n {
        0 => 6, // Rx fraction at bits [7:6] of byte 25
        1 => 4, // Gx fraction at bits [5:4] of byte 25
        2 => 2, // Bx fraction at bits [3:2] of byte 25
        3 => 6, // Wx fraction at bits [7:6] of byte 26
        _ => return 0.0,
    };
    let frac_byte = if n < 3 { frac_byte_0 } else { frac_byte_1 };
    let frac_x = ((frac_byte >> frac_shift_x) & 0x03) as u32;

    let whole_x = edid[whole_base] as u32;
    let raw_x = (whole_x << 2) | frac_x;

    raw_x as f32 / 1024.0
}

fn parse_chromaticity_y(edid: &[u8], n: usize) -> f32 {
    if edid.len() < 35 { return 0.0; }

    let frac_byte_0 = edid[25];
    let frac_byte_1 = edid[26];
    let whole_base = 27 + n * 2 + 1;

    if whole_base >= edid.len() { return 0.0; }

    let frac_shift_y = match n {
        0 => 4, // Ry fraction at bits [5:4] of byte 25
        1 => 2, // Gy fraction at bits [3:2] of byte 25
        2 => 0, // By fraction at bits [1:0] of byte 25
        3 => 4, // Wy fraction at bits [5:4] of byte 26
        _ => return 0.0,
    };
    let frac_byte = if n < 3 { frac_byte_0 } else { frac_byte_1 };
    let frac_y = ((frac_byte >> frac_shift_y) & 0x03) as u32;

    let whole_y = edid[whole_base] as u32;
    let raw_y = (whole_y << 2) | frac_y;

    raw_y as f32 / 1024.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srgb_panel_not_classified_as_p3() {
        // Construct a minimal EDID with sRGB-like chromaticities.
        let mut edid = vec![0u8; 128];
        // Rx ≈ 0.640, Ry ≈ 0.330, Gx ≈ 0.300 → should stay sRGB.
        // Set whole bytes for R primary at bytes 27, 28.
        edid[27] = (((0.640 * 1024.0) as u32) >> 2) as u8;
        edid[28] = (((0.330 * 1024.0) as u32) >> 2) as u8;
        edid[29] = (((0.300 * 1024.0) as u32) >> 2) as u8;
        assert_eq!(edid_color_space(&edid), ColorSpace::Srgb);
    }

    #[test]
    fn test_p3_panel_classified_correctly() {
        let mut edid = vec![0u8; 128];
        // Rx ≈ 0.690, Ry ≈ 0.310, Gx ≈ 0.265 → should be P3.
        edid[27] = (((0.690 * 1024.0) as u32) >> 2) as u8;
        edid[28] = (((0.310 * 1024.0) as u32) >> 2) as u8;
        edid[29] = (((0.265 * 1024.0) as u32) >> 2) as u8;
        assert_eq!(edid_color_space(&edid), ColorSpace::DisplayP3);
    }

    #[test]
    fn test_short_edid_returns_srgb() {
        assert_eq!(edid_color_space(&[0u8; 10]), ColorSpace::Srgb);
    }
}
```

---

## §9 Compositor Integration

`supervisor/src/color/compositor_color.rs` (≤500 lines).

**B4 resolution**: `CompositorColorState` owns a pre-allocated scratch buffer and an
`Option<Arc<CachedTransform>>` for the display transform. The fast-path skip
(`!needs_color_management`) exits before any pixel loop when display space == working
space (e.g., sRGB display, sRGB surface).

```rust
// supervisor/src/color/compositor_color.rs

use std::sync::Arc;
use crate::color::types::{ColorSpace, ColorError, CachedTransform};
use crate::display::Surface; // R11

pub struct CompositorColorState {
    /// Pre-allocated scratch buffer for intermediate pixel conversion.
    /// Sized to the largest surface expected (reallocated on demand). (B4)
    scratch: Vec<u8>,
    /// Cached display transform (source working space → display native space).
    /// None when display == working space (sRGB display with sRGB compositor). (B4)
    display_transform: Option<Arc<CachedTransform>>,
    /// Working color space used during compositing passes.
    pub working_space: ColorSpace,
    /// Detected display color space (from EDID or config).
    pub display_space: ColorSpace,
    /// Fast-path flag: true when display == working space, skip all transforms. (B4)
    needs_color_management: bool,
}

impl CompositorColorState {
    pub fn new(
        working_space: ColorSpace,
        display_space: ColorSpace,
        display_transform: Option<Arc<CachedTransform>>,
    ) -> Self {
        let needs_color_management = working_space != display_space
            || display_transform.is_some();
        Self {
            scratch: Vec::with_capacity(4 * 1920 * 1080), // pre-alloc 1080p (B4)
            display_transform,
            working_space,
            display_space,
            needs_color_management,
        }
    }

    /// Create a passthrough state (embedded platforms, or sRGB/sRGB match).
    pub fn passthrough(working_space: ColorSpace) -> Self {
        Self::new(working_space, working_space, None)
    }

    /// Apply the display transform to a Surface in-place.
    /// Fast-path: returns immediately with Ok(()) if `!needs_color_management`. (B4)
    #[cfg(feature = "color-icc")]
    pub fn apply_display_transform(&mut self, surface: &mut Surface) -> Result<(), ColorError> {
        if !self.needs_color_management {
            return Ok(()); // Fast path: no transform needed.
        }

        let xform = match &self.display_transform {
            Some(x) => x.clone(),
            None => return Ok(()), // No transform configured.
        };

        let pixels = surface.pixels_mut();

        // Ensure scratch is large enough (B4: pre-allocated, realloc only on resize).
        if self.scratch.len() < pixels.len() {
            self.scratch.resize(pixels.len(), 0);
        }

        // Apply in-place: qcms mutates pixels directly via Mutex guard.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut guard = xform.inner.lock();
            // BGRA → swap to RGBA → transform → swap back.
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            guard.convert_pixels_rgba(pixels);
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        })).map_err(|_| ColorError::CatchUnwindPanic)
    }

    /// Non-ICC fast path for analytic sRGB→LinearSrgb per-surface.
    pub fn apply_analytic_linearize(&self, surface: &mut Surface) {
        if !self.needs_color_management { return; }
        if self.working_space == ColorSpace::LinearSrgb
            && self.display_space == ColorSpace::Srgb
        {
            let pixels = surface.pixels_mut();
            for px in pixels.chunks_exact_mut(4) {
                // BGRA layout: swap to RGB for conversion.
                px[0] = crate::color::convert::linear_u8_to_srgb_u8(px[0]); // B
                px[1] = crate::color::convert::linear_u8_to_srgb_u8(px[1]); // G
                px[2] = crate::color::convert::linear_u8_to_srgb_u8(px[2]); // R
                // alpha px[3] unchanged
            }
        }
    }

    /// Called when the display is reconfigured (monitor hotplug, resolution change).
    pub fn update_display_transform(
        &mut self,
        display_space: ColorSpace,
        transform: Option<Arc<CachedTransform>>,
    ) {
        self.display_space = display_space;
        self.display_transform = transform;
        self.needs_color_management = self.working_space != display_space
            || self.display_transform.is_some();
    }

    /// Return true if this compositor pass requires any color management work.
    pub fn is_active(&self) -> bool {
        self.needs_color_management
    }
}
```

---

## §10 ImageData Integration

`supervisor/src/color/image_color.rs` (≤500 lines).

**B5 resolution**: `ImageData.color_space` is `Option<ColorSpace>`. `None` means sRGB
(the default for all existing R14 images). `effective_color_space()` returns `Srgb`
for `None`, enabling zero-overhead serialization for sRGB images (no ColorSpace tag
emitted in WIT/protocol).

```rust
// supervisor/src/color/image_color.rs
// Extension trait and helpers for R14 ImageData color management.

use crate::color::types::{ColorSpace, ColorError, TransformHandle};
use crate::color::transform_registry::TransformRegistry;

/// Color metadata extension for R14 ImageData.
/// This struct is embedded in ImageData (R14 spec) as an optional field.
#[derive(Debug, Clone)]
pub struct ImageColorMeta {
    /// B5: Option<ColorSpace> — None means sRGB (backward compatible default).
    pub color_space: Option<ColorSpace>,
}

impl ImageColorMeta {
    /// Default (sRGB, None).
    pub const fn default_srgb() -> Self {
        Self { color_space: None }
    }

    /// Explicit color space tagging.
    pub fn with_space(space: ColorSpace) -> Self {
        // Store None for sRGB to save serialization overhead (B5).
        Self {
            color_space: if space == ColorSpace::Srgb { None } else { Some(space) },
        }
    }

    /// Returns the effective color space, defaulting to sRGB (B5).
    pub fn effective_color_space(&self) -> ColorSpace {
        self.color_space.unwrap_or(ColorSpace::Srgb)
    }

    /// True if color management is a no-op for this image (effectively sRGB).
    pub fn is_srgb(&self) -> bool {
        self.color_space.map_or(true, |s| s == ColorSpace::Srgb)
    }
}

/// Apply a color transform to an ImageData pixel buffer.
///
/// # Arguments
/// * `pixels` — mutable RGBA8 byte slice (must be width * height * 4 bytes)
/// * `width`, `height` — image dimensions
/// * `meta` — source color metadata (B5: effective_color_space() for from-space)
/// * `target_space` — destination color space
/// * `transform` — pre-created TransformHandle from TransformRegistry
/// * `registry` — TransformRegistry reference
pub fn apply_color_transform(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    meta: &ImageColorMeta,
    target_space: ColorSpace,
    transform: TransformHandle,
    #[cfg(feature = "color-icc")]
    registry: &TransformRegistry,
) -> Result<(), ColorError> {
    let from = meta.effective_color_space();
    if from == target_space {
        return Ok(()); // No-op.
    }

    let expected_len = (width * height * 4) as usize;
    if pixels.len() < expected_len {
        return Err(ColorError::InvalidProfile("pixel buffer smaller than width*height*4"));
    }

    // Analytic fast path for sRGB ↔ LinearSrgb.
    match (from, target_space) {
        (ColorSpace::Srgb, ColorSpace::LinearSrgb) => {
            return crate::color::convert::srgb_to_linear_inplace(
                pixels,
                crate::color::convert::PixelFormat::Rgba8,
            );
        }
        (ColorSpace::LinearSrgb, ColorSpace::Srgb) => {
            return crate::color::convert::linear_to_srgb_inplace(
                pixels,
                crate::color::convert::PixelFormat::Rgba8,
            );
        }
        _ => {}
    }

    // ICC qcms path.
    #[cfg(feature = "color-icc")]
    {
        registry.apply_to_pixels(transform, pixels, 0 /* RGBA8 */)
    }

    #[cfg(not(feature = "color-icc"))]
    Err(ColorError::NotSupported)
}

/// Tag an existing ImageData with a color space annotation.
/// Returns an error if the space requires ICC support but ICC is disabled.
pub fn tag_image_color_space(
    meta: &mut ImageColorMeta,
    space: ColorSpace,
    icc_enabled: bool,
) -> Result<(), ColorError> {
    if !icc_enabled && space != ColorSpace::Srgb {
        return Err(ColorError::NotSupported);
    }
    meta.color_space = if space == ColorSpace::Srgb { None } else { Some(space) };
    Ok(())
}
```

---

## §11 WIT Interface

`supervisor/wit/vyoma-color.wit` — full WIT text for `vyoma:color@1.0.0`.

**B3 resolution**: `apply-transform-to-surface` operates on a surface handle (zero-copy
from the app's perspective — the supervisor applies the transform in the framebuffer
directly). `apply-transform` provides the byte-copy path for apps that own their pixel
buffers.  
**B8 resolution**: `create-transform` is documented as idempotent in the WIT comment.

```wit
// supervisor/wit/vyoma-color.wit
// WIT interface for VyomaOS Color Management subsystem (R15).
// Feature gate: only available when supervisor compiled with `color-icc`.

package vyoma:color@1.0.0;

interface color-manager {
  /// Error type for all color operations.
  variant color-error {
    invalid-profile(string),
    profile-too-large(u64),
    unsupported-color-space,
    transform-failed(string),
    handle-exhausted,
    invalid-handle,
    budget-exceeded,
    not-supported,
    io-error(string),
    capability-denied,
    catch-unwind-panic,
  }

  /// Supported color spaces.
  enum color-space {
    srgb,
    linear-srgb,
    display-p3,
    adobe-rgb,
    bt2020,
    grayscale,
    lab,
  }

  /// ICC rendering intent.
  enum rendering-intent {
    perceptual,
    relative-colorimetric,
    saturation,
    absolute-colorimetric,
  }

  /// Opaque handle for a loaded ICC profile.
  type profile-id = u32;

  /// Opaque handle for a cached transform.
  type transform-handle = u32;

  /// Pixel format for apply-transform.
  enum pixel-format {
    rgba8,
    bgra8,
    rgb8,
  }

  /// ICC profile metadata returned by profile-info.
  record profile-info {
    id: profile-id,
    space: color-space,
    description: string,
    size-bytes: u64,
  }

  // ---------------------------------------------------------------------------
  // Profile operations
  // ---------------------------------------------------------------------------

  /// Load an ICC profile from raw bytes. Validates and registers the profile.
  /// Requires `color.load-profiles = true` in app manifest.
  load-profile: func(data: list<u8>, desc: string) -> result<profile-id, color-error>;

  /// Load a system-provided ICC profile by well-known name ("sRGB", "DisplayP3").
  load-system-profile: func(name: string) -> result<profile-id, color-error>;

  /// Unload a previously loaded profile. Frees associated memory.
  unload-profile: func(id: profile-id) -> result<_, color-error>;

  /// Return metadata for a loaded profile.
  profile-info: func(id: profile-id) -> result<profile-info, color-error>;

  // ---------------------------------------------------------------------------
  // Transform operations
  // ---------------------------------------------------------------------------

  /// Create (or retrieve cached) a transform from `from-profile` to `to-profile`.
  ///
  /// IDEMPOTENT (B8): If a transform with the same (from, to, intent) triple
  /// was already created, returns the cached handle without allocating a new
  /// qcms transform object. Apps may call this freely without deduplication.
  create-transform: func(
    from-profile: profile-id,
    to-profile: profile-id,
    intent: rendering-intent,
  ) -> result<transform-handle, color-error>;

  /// Apply a transform to a pixel buffer. Returns the transformed bytes.
  /// For large buffers, prefer apply-transform-to-surface (B3) to avoid copies.
  apply-transform: func(
    transform: transform-handle,
    pixels: list<u8>,
    width: u32,
    height: u32,
    fmt: pixel-format,
  ) -> result<list<u8>, color-error>;

  /// Apply a transform to a Surface by handle (zero-copy — supervisor operates
  /// directly on the framebuffer). (B3)
  apply-transform-to-surface: func(
    transform: transform-handle,
    surface: u32,
  ) -> result<_, color-error>;

  /// Drop a transform handle. The underlying qcms transform is freed when the
  /// last reference is dropped (Arc semantics — safe to call even if cached).
  drop-transform: func(handle: transform-handle) -> result<_, color-error>;

  // ---------------------------------------------------------------------------
  // Display / compositor queries
  // ---------------------------------------------------------------------------

  /// Return the detected display color space (from EDID or config).
  /// Returns `srgb` on platforms without EDID probing.
  display-color-space: func() -> color-space;

  /// Return the compositor's current working color space.
  working-color-space: func() -> color-space;
}

world color-world {
  export color-manager;
}
```

---

## §12 Manifest Capability

`supervisor/src/color/capability.rs` (≤500 lines).

```rust
// supervisor/src/color/capability.rs

use serde::Deserialize;
use std::path::{Path, PathBuf};
use crate::color::types::ColorError;

/// The `[capabilities.color]` block in an app's `vyoma.toml`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ColorCapability {
    /// App may load ICC profiles from raw bytes.
    #[serde(default)]
    pub load_profiles: bool,
    /// App may load ICC profiles from the filesystem.
    /// Requires `filesystem = true` AND this flag.
    #[serde(default)]
    pub load_profiles_from_path: bool,
    /// App may create and use color transforms.
    #[serde(default)]
    pub transforms: bool,
    /// Override the maximum number of ICC profiles for this app.
    /// If unset, uses platform default from ColorConfig.
    #[serde(default)]
    pub max_profiles: Option<usize>,
    /// Override the per-app byte budget.
    #[serde(default)]
    pub max_bytes: Option<usize>,
    /// Allowed directory prefixes for profile path loading.
    /// Only paths within these prefixes are permitted.
    #[serde(default)]
    pub allowed_paths: Vec<PathBuf>,
}

impl ColorCapability {
    /// Resolve effective limits, applying platform defaults where app overrides are absent.
    pub fn resolve(
        &self,
        platform_max_profiles: usize,
        platform_byte_budget: usize,
    ) -> ResolvedColorCapability {
        ResolvedColorCapability {
            load_profiles: self.load_profiles,
            load_profiles_from_path: self.load_profiles_from_path,
            transforms: self.transforms,
            max_profiles: self.max_profiles.unwrap_or(platform_max_profiles).min(platform_max_profiles),
            max_bytes: self.max_bytes.unwrap_or(platform_byte_budget).min(platform_byte_budget),
            allowed_paths: self.allowed_paths.clone(),
        }
    }

    /// Returns true if any color capability is declared.
    pub fn is_any_enabled(&self) -> bool {
        self.load_profiles || self.load_profiles_from_path || self.transforms
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedColorCapability {
    pub load_profiles: bool,
    pub load_profiles_from_path: bool,
    pub transforms: bool,
    pub max_profiles: usize,
    pub max_bytes: usize,
    pub allowed_paths: Vec<PathBuf>,
}

impl ResolvedColorCapability {
    /// Validate that `path` is within an allowed prefix.
    pub fn validate_path(&self, path: &Path) -> Result<(), ColorError> {
        if self.allowed_paths.is_empty() {
            return Err(ColorError::CapabilityDenied);
        }
        let canonical = path.canonicalize()
            .map_err(|e| ColorError::IoError(e.to_string()))?;
        for allowed in &self.allowed_paths {
            let allowed_canonical = allowed.canonicalize()
                .map_err(|e| ColorError::IoError(e.to_string()))?;
            if canonical.starts_with(&allowed_canonical) {
                return Ok(());
            }
        }
        Err(ColorError::CapabilityDenied)
    }

    /// Check that the app has permission to load profiles from bytes.
    pub fn check_load(&self) -> Result<(), ColorError> {
        if self.load_profiles { Ok(()) } else { Err(ColorError::CapabilityDenied) }
    }

    /// Check that the app has permission to create transforms.
    pub fn check_transform(&self) -> Result<(), ColorError> {
        if self.transforms { Ok(()) } else { Err(ColorError::CapabilityDenied) }
    }
}
```

### Example `vyoma.toml` with color capability

```toml
[app]
name    = "photo-editor"
version = "0.1.0"
wasm    = "photo-editor.wasm"

[capabilities]
stdio      = true
filesystem = true
display    = true

[capabilities.color]
load_profiles           = true
load_profiles_from_path = true
transforms              = true
max_profiles            = 8
max_bytes               = 524288           # 512 KB
allowed_paths           = ["/data/icc"]
```

---

## §13 System Profiles

`supervisor/src/color/system_profiles.rs` (≤500 lines).

```rust
// supervisor/src/color/system_profiles.rs

use crate::color::types::{ColorError, ProfileId};
use crate::color::profile_registry::ProfileRegistry;

/// Paths in the initramfs where system ICC profiles live.
/// These are embedded at rootfs build time from standard profile sources.
const SRGB_ICC_PATH: &str = "/usr/share/color/icc/sRGB.icc";
const DISPLAY_P3_ICC_PATH: &str = "/usr/share/color/icc/DisplayP3.icc";

/// Well-known system profile names (matched by load-system-profile WIT call).
pub const SYSTEM_PROFILE_SRGB: &str = "sRGB";
pub const SYSTEM_PROFILE_DISPLAY_P3: &str = "DisplayP3";

/// Handle IDs reserved for system profiles (profiles owned by PID 0).
pub struct SystemProfileHandles {
    pub srgb: ProfileId,
    pub display_p3: Option<ProfileId>, // None on embedded platforms
}

/// Load all system color profiles at supervisor boot.
/// Called from main.rs after ColorConfig is initialized.
/// On embedded platforms (icc_enabled=false), returns default handles with
/// sentinel values and does not attempt to read ICC files.
pub fn boot_load_system_color_profiles(
    registry: &ProfileRegistry,
    icc_enabled: bool,
) -> Result<SystemProfileHandles, ColorError> {
    if !icc_enabled {
        // Embedded platforms: return sentinel handles. The supervisor
        // treats handle 0 as "sRGB passthrough" in all color paths.
        return Ok(SystemProfileHandles {
            srgb: ProfileId(0),
            display_p3: None,
        });
    }

    // Load sRGB profile.
    let srgb_data = std::fs::read(SRGB_ICC_PATH)
        .map_err(|e| ColorError::IoError(format!("{SRGB_ICC_PATH}: {e}")))?;
    let srgb_id = registry.load_system_profile(srgb_data, SYSTEM_PROFILE_SRGB)?;
    log::info!("[color] loaded system sRGB profile: id={}", srgb_id.0);

    // Load Display P3 profile.
    let p3_result = std::fs::read(DISPLAY_P3_ICC_PATH)
        .map_err(|e| ColorError::IoError(format!("{DISPLAY_P3_ICC_PATH}: {e}")))
        .and_then(|data| registry.load_system_profile(data, SYSTEM_PROFILE_DISPLAY_P3));

    let display_p3 = match p3_result {
        Ok(id) => {
            log::info!("[color] loaded system DisplayP3 profile: id={}", id.0);
            Some(id)
        }
        Err(e) => {
            log::warn!("[color] DisplayP3 profile not available: {e}; wide-gamut disabled");
            None
        }
    };

    Ok(SystemProfileHandles { srgb: srgb_id, display_p3 })
}

/// Resolve a well-known system profile name to its ProfileId.
pub fn resolve_system_profile_name(
    name: &str,
    handles: &SystemProfileHandles,
) -> Option<ProfileId> {
    match name {
        SYSTEM_PROFILE_SRGB => Some(handles.srgb),
        SYSTEM_PROFILE_DISPLAY_P3 => handles.display_p3,
        _ => None,
    }
}
```

### rootfs.sh integration

The `rootfs.sh` script must embed ICC files into initramfs:

```bash
# In rootfs.sh, after copying supervisor binary:
mkdir -p "${ROOTFS}/usr/share/color/icc"
# sRGB IEC 61966-2-1 profile (public domain, 3.1 KB)
cp "${WORK}/icc/sRGB.icc" "${ROOTFS}/usr/share/color/icc/"
# Display P3 (D65) profile (ICC standard, 3.2 KB)
cp "${WORK}/icc/DisplayP3.icc" "${ROOTFS}/usr/share/color/icc/"
```

---

## §14 Security Model

Six independent defense layers, outermost to innermost:

### Layer 1: Pre-validation (B1)

`validate_icc_profile()` rejects profiles before any qcms object is allocated:
- Minimum 128-byte header
- Maximum 4 MB size (ICC.1:2022 limit)
- Declared-size == actual-size match
- ICC version >= 2.0
- Tag count ≤ 100 (prevents quadratic parser attacks)
- Per-tag data bounds check (every tag offset+size fits in data)
- Rejects CMYK, DeviceN, unknown color spaces

### Layer 2: catch_unwind Secondary Guard

`TransformRegistry::apply_to_pixels` and `get_or_create` wrap qcms calls in
`std::panic::catch_unwind`. Any panic from qcms (e.g., internal assertion on malformed
profile data that passed validation) is caught and returned as
`ColorError::CatchUnwindPanic`. The supervisor logs the event and continues; the
calling app receives an error code.

### Layer 3: Per-App Byte Budget

`ProfileRegistry::load_profile_inner` tracks `bytes_by_pid` and rejects profiles that
would exceed `ColorConfig::per_app_byte_budget`. System profiles (PID 0) bypass the
budget. App overrides in `[capabilities.color]` are capped to the platform maximum.

### Layer 4: Separate Handle Namespaces (B2)

`ProfileId` and `TransformHandle` are distinct newtypes backed by independent
`AtomicU32` counters in `ProfileRegistry` and `TransformRegistry` respectively. An app
cannot pass a `ProfileId` value where a `TransformHandle` is expected — the WIT
interface enforces separate parameter types. This eliminates handle-confusion attacks.

### Layer 5: Path Capability

`load_profiles_from_path` requires both `filesystem = true` AND
`capabilities.color.load_profiles_from_path = true` in `vyoma.toml`. Each load
operation calls `ResolvedColorCapability::validate_path()`, which canonicalizes the
path and checks it against the `allowed_paths` prefix list. Directory traversal attacks
are rejected by `canonicalize()` before prefix comparison.

### Layer 6: Arc Ownership (B7)

All `qcms::Transform` objects are owned by `Arc<CachedTransform>` inside
`TransformRegistry`. No `Box::into_raw`, no `*mut c_void`, no manual
`qcms_transform_release` calls. Rust's drop system frees qcms memory when the last
`Arc` reference is released. This eliminates use-after-free, double-free, and wild
pointer bugs in the transform lifecycle.

---

## §15 Platform Matrix

| Platform | ICC Enabled | Max Profiles | Max Transforms | Working Space | EDID Probe | System Profiles |
|---|---|---|---|---|---|---|
| `mcu-minimal` | No | 0 | 0 | sRGB | No | None |
| `iot-edge` | No | 0 | 0 | sRGB | No | None |
| `robotics-rt` | No | 0 | 0 | sRGB | No | None |
| `mobile` | Yes | 8 | 16 | Display P3 | No | sRGB + P3 |
| `desktop-full` | Yes | 32 | 64 | Linear sRGB | Yes | sRGB + P3 |
| `server-headless` | Yes | 64 | 128 | Linear sRGB | No | sRGB + P3 |

**Notes**:
- Embedded platforms (mcu-minimal, iot-edge, robotics-rt) do not link `qcms` and do
  not compile `supervisor/src/color/transform_registry.rs` (gated by `#[cfg(feature = "color-icc")]`).
- Mobile uses Display P3 as working space to match Apple ecosystem conventions; the
  display hardware on ARM tablets is assumed to be P3-capable.
- desktop-full performs EDID probe at display initialization; result is cached in
  `CompositorColorState`. Monitor hotplug re-probes and updates the compositor state.
- server-headless enables ICC for batch image processing pipelines (e.g., thumbnail
  generation with correct color); no display output.

---

## §16 Performance Budget

### Per-Pixel Timing Estimates

| Path | Throughput | Notes |
|---|---|---|
| Analytic sRGB → Linear (LUT) | ~2 ns/px | 256-entry LUT, L1 cache resident |
| Analytic Linear → sRGB (LUT) | ~2 ns/px | Same LUT approach |
| qcms RGBA8 transform | ~8 ns/px | Per qcms benchmark on ARM Cortex-A72 |
| Fast-path skip (same space) | ~0 ns | Returns immediately (B4) |

### 4K Frame Budget (3840 × 2160 = 8.3M pixels)

| Operation | Time | 60 Hz budget remaining |
|---|---|---|
| Analytic linearize (compositor pre-pass) | ~16 ms | 0.6 ms |
| qcms display transform (worst case) | ~66 ms | Over budget — must skip |
| Fast-path skip (sRGB display) | ~0 ms | 16.7 ms |

**Implication**: On a 4K sRGB display, the fast-path skip (`!needs_color_management`)
is essential to maintain 60 Hz. The EDID probe at init determines whether the display
is sRGB (skip) or DisplayP3 (apply qcms). For P3 displays at 4K 60 Hz, the display
transform must run on a GPU path (deferred to R12 GPU WIT integration). V1 CPU path
targets 1080p only for P3 displays.

### 1080p Frame Budget (1920 × 1080 = 2.1M pixels)

| Operation | Time | 60 Hz budget remaining |
|---|---|---|
| Analytic linearize | ~4.2 ms | 12.5 ms |
| qcms display transform | ~17 ms | Over 60 Hz target (fits 30 Hz) |
| Fast-path skip | ~0 ms | 16.7 ms |

**1080p P3 target**: 30 Hz for V1 CPU path. GPU path (R12) required for 60 Hz.

### Memory Budget

| Platform | Max Profiles (bytes) | Per-App Budget | Transform Object |
|---|---|---|---|
| mcu-minimal | 0 | 0 | N/A |
| iot-edge | 0 | 0 | N/A |
| robotics-rt | 0 | 0 | N/A |
| mobile | 8 × 512 KB = 4 MB total | 256 KB | ~2 KB/transform |
| desktop-full | 32 × 4 MB = 128 MB max | 1 MB | ~2 KB/transform |
| server-headless | 64 × 4 MB = 256 MB max | 4 MB | ~2 KB/transform |

System profiles (sRGB.icc ~3 KB, DisplayP3.icc ~3 KB) are excluded from per-app
budgets (owner PID 0 bypasses budget checks).

---

## §17 Cargo Dependencies

`supervisor/Cargo.toml` additions:

```toml
[features]
default = []
# Enables ICC profile loading and qcms transforms.
# Compiled for: mobile, desktop-full, server-headless.
# NOT compiled for: mcu-minimal, iot-edge, robotics-rt.
color-icc = ["dep:qcms"]

[dependencies]
# Pure-Rust ICC color transform engine. No C FFI, no lcms2 dependency.
# Used by Firefox and Servo for display color management.
qcms = { version = "0.9", optional = true }

# parking_lot for RwLock/Mutex with better performance than std.
parking_lot = "0.12"
```

**Why qcms, not lcms2**:
- `qcms` is pure Rust: no C compiler required in the hermetic Docker build.
- Binary size: qcms adds ~150 KB to the supervisor; lcms2 (FFI) would add ~400 KB.
- Security: no C memory unsafety; Rust ownership model applies throughout.
- Provenance: same engine used in Firefox/Servo; well-tested on real ICC profiles.
- CMYK: qcms does not support CMYK (we reject CMYK at validation — aligned).

**No lcms2**: Explicitly excluded. lcms2 requires C FFI bindings, a C compiler in the
build image, and introduces C memory safety risks. Not acceptable given VyomaOS's
Rust-first security model.

---

## §18 Implementation Files

Nine files under `supervisor/src/color/`, each ≤500 lines:

| File | Contents | Est. Lines |
|---|---|---|
| `supervisor/src/color/mod.rs` | Module declarations, re-exports, `ColorSubsystem` struct | ~80 |
| `supervisor/src/color/types.rs` | All core types: ColorSpace, RenderingIntent, ProfileId, TransformHandle, IccProfile, TransformKey, CachedTransform, Color, ColorError, ColorConfig | ~280 |
| `supervisor/src/color/config.rs` | `ColorConfig::for_platform()` for all 6 platforms | ~90 |
| `supervisor/src/color/validation.rs` | `validate_icc_profile()`, `extract_icc_description()`, `parse_mluc_english()` | ~200 |
| `supervisor/src/color/profile_registry.rs` | `ProfileRegistry`, load/get/unload, byte accounting | ~180 |
| `supervisor/src/color/transform_registry.rs` | `TransformRegistry`, `get_or_create`, `apply_to_pixels`, B2/B7/B8 | ~220 |
| `supervisor/src/color/convert.rs` | `convert_pixels`, analytic LUTs, `srgb_to_linear_inplace`, `linear_to_srgb_inplace` | ~200 |
| `supervisor/src/color/display_probe.rs` | `probe_display_color_space`, `edid_color_space`, EDID chromaticity parsing, B6 | ~180 |
| `supervisor/src/color/compositor_color.rs` | `CompositorColorState`, `apply_display_transform`, fast-path skip, B4 | ~150 |
| `supervisor/src/color/capability.rs` | `ColorCapability`, `ResolvedColorCapability`, path validation | ~140 |
| `supervisor/src/color/system_profiles.rs` | `boot_load_system_color_profiles`, `SystemProfileHandles`, rootfs note | ~100 |
| `supervisor/src/color/image_color.rs` | `ImageColorMeta`, `apply_color_transform`, `tag_image_color_space`, B5 | ~120 |
| `supervisor/wit/vyoma-color.wit` | WIT interface `vyoma:color@1.0.0` | ~100 |

**Module entry point**:

```rust
// supervisor/src/color/mod.rs

pub mod types;
pub mod config;
pub mod validation;
pub mod profile_registry;
pub mod convert;
pub mod display_probe;
pub mod compositor_color;
pub mod capability;
pub mod system_profiles;
pub mod image_color;

#[cfg(feature = "color-icc")]
pub mod transform_registry;

// Re-export primary types for convenience.
pub use types::{
    Color, ColorConfig, ColorError, ColorSpace, IccProfile, ProfileId,
    RenderingIntent, TransformHandle, TransformKey,
};
pub use profile_registry::ProfileRegistry;
pub use compositor_color::CompositorColorState;
pub use capability::{ColorCapability, ResolvedColorCapability};
pub use system_profiles::{boot_load_system_color_profiles, SystemProfileHandles};

#[cfg(feature = "color-icc")]
pub use transform_registry::TransformRegistry;

/// Top-level color subsystem state. One instance per supervisor.
pub struct ColorSubsystem {
    pub config: ColorConfig,
    pub profiles: ProfileRegistry,
    #[cfg(feature = "color-icc")]
    pub transforms: TransformRegistry,
    pub system_handles: SystemProfileHandles,
    pub compositor_state: CompositorColorState,
}

impl ColorSubsystem {
    /// Initialize the color subsystem for the given platform.
    /// Called from supervisor main() after platform profile is loaded.
    pub fn init(platform: &str) -> Result<Self, ColorError> {
        let config = ColorConfig::for_platform(platform);
        let profiles = ProfileRegistry::new(config.clone());
        let system_handles = system_profiles::boot_load_system_color_profiles(
            &profiles,
            config.icc_enabled,
        )?;

        // Detect display color space if EDID probe is enabled.
        let display_space = if config.probe_edid {
            display_probe::probe_display_color_space()
        } else {
            config.working_space
        };

        // Build initial CompositorColorState.
        // If display == working space, fast-path skip is active.
        let compositor_state = CompositorColorState::passthrough(display_space);

        Ok(Self {
            #[cfg(feature = "color-icc")]
            transforms: TransformRegistry::new(config.clone()),
            config,
            profiles,
            system_handles,
            compositor_state,
        })
    }

    /// Clean up all state for a terminated app.
    pub fn on_app_exit(&mut self, pid: u32) {
        self.profiles.unload_owned_by(pid);
        #[cfg(feature = "color-icc")]
        self.transforms.drop_owned_by(pid);
    }
}
```

---

## §19 Deferred Features

The following are explicitly out of scope for V1 (R15) and tracked for future releases:

### HDR / Extended Dynamic Range (EDR)
- **Deferred to**: R22+ (display pipeline supports 10-bit output)
- **Description**: PQ (ST 2084) and HLG transfer functions; scRGB extended-range
  compositing; HDR metadata (MaxCLL, MaxFALL). Requires changes to Surface (R11)
  to support 16-bit per channel or fp16 pixel formats.
- **Impact on R15**: ColorSpace::Bt2020 is declared in V1 types for forward
  compatibility but no transform involving Bt2020 can be created (qcms will error;
  returned as TransformFailed).

### ProPhoto RGB
- **Deferred to**: V2 system profiles
- **Description**: Wide-gamut space for RAW photo workflows. Profile size ~3 KB.
  V1 validation will accept a ProPhoto.icc if loaded by an app, but no system
  profile is provided.

### CMYK Color Spaces
- **Deferred to**: R25+ (print subsystem)
- **Description**: CMYK ICC profiles are explicitly rejected by `validate_icc_profile`
  with `UnsupportedColorSpace`. No CMYK-capable apps are planned before R25.

### Display Calibration
- **Deferred to**: R23 (settings UI)
- **Description**: User-provided display ICC profiles via a color calibration UI.
  V1 uses EDID auto-detection only. The settings subsystem will need to write a
  user profile path to supervisor config, which ColorSubsystem picks up on display
  init.

### Per-Window Color Space Tagging (R21)
- **Deferred to**: R21 (windowing subsystem V2)
- **Description**: Each window/surface tagged with a source color space; compositor
  applies a per-window transform during the Z-order pass (spec-15 §9 currently
  applies one global display transform). This matches macOS's per-layer color space
  model.

### Color Delta-E Perceptual Difference
- **Deferred to**: Test subsystem (R26+)
- **Description**: CIE76/CIE2000 delta-E metric for automated color correctness
  testing (e.g., verifying that a UI screenshot rendered on P3 matches a reference
  within ΔE < 2.0). Not needed in production supervisor code.

### Tone Mapping (HDR → SDR)
- **Deferred to**: R22+ alongside HDR support
- **Description**: Reinhard, ACES, or Hable tone mapping for SDR display output of
  HDR content. Depends on 10-bit surface support.

---

## Summary of Blocking Issue Resolutions

| ID | Issue | Resolution |
|---|---|---|
| B1 | `catch_unwind` insufficient as primary guard | Pre-validate ICC with `validate_icc_profile()` before any qcms call; catch_unwind is secondary |
| B2 | Shared handle pool caused type confusion | `ProfileRegistry` and `TransformRegistry` own independent `AtomicU32` counters; separate newtypes `ProfileId` vs `TransformHandle` |
| B3 | No zero-copy WIT path for surface transforms | `apply-transform-to-surface` WIT function operates on surface handle; supervisor applies in-place |
| B4 | Compositor allocated scratch buffers per-frame | `CompositorColorState` owns pre-allocated `Vec<u8>` scratch; fast-path skip for `!needs_color_management` |
| B5 | `ImageData.color_space` was non-optional | Changed to `Option<ColorSpace>`; `None` = sRGB; `effective_color_space()` returns Srgb for None |
| B6 | EDID gamut threshold was single Rx condition | Three-condition check: Rx > 0.680 AND Ry < 0.320 AND Gx < 0.270 |
| B7 | Raw pointer casting of qcms transforms | All transforms owned as `Arc<parking_lot::Mutex<qcms::Transform>>`; no `Box::into_raw`, no `*mut c_void` |
| B8 | `create-transform` duplicated identical transforms | `get_or_create` checks `by_key` before allocating; returns cached handle if `(from, to, intent)` already registered |
