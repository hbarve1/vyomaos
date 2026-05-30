# Round 15 — Color Management & ICC Profiles
**Role:** Architect | **Date:** 2026-05-29 | **Status:** Proposal

## 0. Executive Summary

Rounds 11–14 built the display surface (BGRA32 blit), GPU WIT hostcalls, font
rendering, and the image/icon pipeline. Every one of those subsystems silently
assumes sRGB. On a wide-gamut Display P3 monitor that assumption produces washed-
out colors: sRGB red (#FF0000) renders about 8% less saturated than the panel can
show, and P3-native content decoded from a HEIF/JPEG XL image appears oversaturated
if composited without a transform.

This round adds **Color Management** — the macOS ColorSync analogue — to VyomaOS.
The design is WASM-first: color space metadata travels through the WIT boundary,
transforms execute in the supervisor, and no ICC file ever crosses into a WASM app.

Headline constraints:

| Constraint | Value |
|---|---|
| Supervisor file limit | ≤ 500 LOC per `.rs` file |
| Max ICC profile size | 4 MB (enforced before parse) |
| Profile registry cap | 16 profiles (LRU eviction) |
| Transform cache cap | 32 entries keyed by `(from, to, intent)` |
| mcu-minimal | sRGB passthrough only; no ICC parse, no qcms/lcms2 link |
| iot-edge / robotics-rt | sRGB only; ICC disabled; qcms not compiled in |
| mobile / desktop-full / server-headless | Full ICC via `qcms` (pure Rust) |
| Adversarial defense | `catch_unwind(AssertUnwindSafe(...))` around every parse/transform |
| C library dependency | None — `qcms` is pure Rust; avoids lcms2 SIGSEGV risk |

---

## 1. Core Types (`supervisor/src/color/mod.rs`)

```rust
//! supervisor/src/color/mod.rs
//! Public types for the VyomaOS color management subsystem.
//! ≤ 500 LOC — type definitions and re-exports only.

pub mod config;
pub mod convert;
pub mod display;
pub mod manifest;
pub mod profile;
pub mod registry;
pub mod transform;
pub mod wit_handlers;

use std::sync::Arc;

// ── color space ───────────────────────────────────────────────────────────────

/// All color spaces the supervisor understands.
///
/// Values are stable and used in WIT u8 encoding:
///   0 = Srgb, 1 = LinearSrgb, 2 = DisplayP3, 3 = AdobeRgb,
///   4 = Bt2020, 5 = Grayscale
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ColorSpace {
    /// IEC 61966-2-1 sRGB — 8-bit gamma ≈ 2.2.  Default everywhere.
    Srgb = 0,
    /// Linear-light sRGB (no gamma) — used for blending math.
    LinearSrgb = 1,
    /// DCI-P3 with D65 white point (Apple Display P3).
    DisplayP3 = 2,
    /// Adobe RGB (1998) — larger gamut, print workflows.
    AdobeRgb = 3,
    /// ITU-R BT.2020 — HDR/UHD wide gamut.
    Bt2020 = 4,
    /// Single-channel luminance, sRGB gamma.
    Grayscale = 5,
}

impl ColorSpace {
    /// Returns true when a full ICC transform is required.
    /// sRGB↔LinearSrgb can be done analytically without qcms.
    pub fn requires_icc_transform(self, to: ColorSpace) -> bool {
        !matches!(
            (self, to),
            (ColorSpace::Srgb, ColorSpace::LinearSrgb)
                | (ColorSpace::LinearSrgb, ColorSpace::Srgb)
                | (ColorSpace::Srgb, ColorSpace::Srgb)
                | (ColorSpace::LinearSrgb, ColorSpace::LinearSrgb)
                | (ColorSpace::Grayscale, ColorSpace::Grayscale)
        )
    }

    /// Wire value for WIT encoding.
    pub fn as_u8(self) -> u8 { self as u8 }

    /// Decode from WIT wire value. Returns `None` on unknown tag.
    pub fn from_u8(v: u8) -> Option<ColorSpace> {
        match v {
            0 => Some(ColorSpace::Srgb),
            1 => Some(ColorSpace::LinearSrgb),
            2 => Some(ColorSpace::DisplayP3),
            3 => Some(ColorSpace::AdobeRgb),
            4 => Some(ColorSpace::Bt2020),
            5 => Some(ColorSpace::Grayscale),
            _ => None,
        }
    }
}

// ── rendering intent ─────────────────────────────────────────────────────────

/// ICC rendering intent — controls how out-of-gamut colors are handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RenderingIntent {
    /// Compress entire gamut to fit; preserves relationships. Best for photos.
    Perceptual = 0,
    /// Shift white point; clips out-of-gamut. Best for logos/solid colors.
    RelativeColorimetric = 1,
    /// Maximise saturation; ignores lightness. Best for charts/graphics.
    Saturation = 2,
    /// No white-point adjustment. Best for proofing.
    AbsoluteColorimetric = 3,
}

impl RenderingIntent {
    pub fn as_u8(self) -> u8 { self as u8 }
    pub fn from_u8(v: u8) -> Option<RenderingIntent> {
        match v {
            0 => Some(RenderingIntent::Perceptual),
            1 => Some(RenderingIntent::RelativeColorimetric),
            2 => Some(RenderingIntent::Saturation),
            3 => Some(RenderingIntent::AbsoluteColorimetric),
            _ => None,
        }
    }
}

// ── profile id ────────────────────────────────────────────────────────────────

/// Opaque handle to a parsed ICC profile held in `ProfileRegistry`.
/// u32 handle; 0 is reserved for "sRGB built-in".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProfileId(pub u32);

impl ProfileId {
    pub const SRGB_BUILTIN: ProfileId = ProfileId(0);
}

// ── icc profile ───────────────────────────────────────────────────────────────

/// A parsed ICC profile with metadata.
pub struct IccProfile {
    /// Stable handle used in host calls and transform cache.
    pub id: ProfileId,
    /// Color space this profile describes.
    pub space: ColorSpace,
    /// Raw ICC bytes (retained for qcms re-parse on cache miss).
    pub data: Arc<Vec<u8>>,
    /// Human-readable description from ICC `desc` tag, max 256 chars.
    pub desc: String,
    /// Byte size of `data`, checked before parse.
    pub byte_size: usize,
}

// ── color transform ──────────────────────────────────────────────────────────

/// A prepared color space conversion.
///
/// Transforms are created once, cached, and shared across multiple
/// `convert_pixels` calls.  `ColorTransform` is `Send + Sync` because
/// qcms transforms are thread-safe for concurrent reads.
pub struct ColorTransform {
    pub from:   ColorSpace,
    pub to:     ColorSpace,
    pub intent: RenderingIntent,
    /// qcms transform object wrapped in a pointer-safe newtype.
    /// `None` when from == to (identity) or both are sRGB/LinearSrgb
    /// (handled by analytic gamma expand/compress).
    pub inner:  Option<Arc<QcmsTransformHandle>>,
}

/// Thread-safe wrapper around a qcms transform pointer.
pub struct QcmsTransformHandle {
    // Raw pointer to qcms_transform. Owned; dropped via qcms API.
    // SAFETY: qcms transforms are immutable after creation; concurrent
    // apply calls are safe per qcms documentation.
    pub(crate) ptr: *mut std::ffi::c_void,
}

// SAFETY: qcms_transform_data_n is re-entrant for reads; we never mutate
// the transform after creation.
unsafe impl Send for QcmsTransformHandle {}
unsafe impl Sync for QcmsTransformHandle {}

impl Drop for QcmsTransformHandle {
    fn drop(&mut self) {
        // qcms_transform_release is called here via FFI.
        // When the `color-icc` feature is disabled this is a no-op.
        #[cfg(feature = "color-icc")]
        unsafe { qcms_sys::qcms_transform_release(self.ptr as *mut _) };
    }
}

// ── color value (linear light) ───────────────────────────────────────────────

/// A single color sample in linear-light (no gamma) float representation.
/// Alpha is straight (not premultiplied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK:       Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE:       Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    /// Encode to packed sRGB BGRA32 (gamma-compressed).
    pub fn to_bgra32(self) -> [u8; 4] {
        let r = srgb_encode(self.r);
        let g = srgb_encode(self.g);
        let b = srgb_encode(self.b);
        let a = (self.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [b, g, r, a]
    }

    /// Decode from packed sRGB BGRA32.
    pub fn from_bgra32(bgra: [u8; 4]) -> Color {
        Color {
            b: srgb_decode(bgra[0]),
            g: srgb_decode(bgra[1]),
            r: srgb_decode(bgra[2]),
            a: bgra[3] as f32 / 255.0,
        }
    }
}

/// sRGB gamma expand: u8 → linear f32.
#[inline]
pub fn srgb_decode(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.04045 { s / 12.92 }
    else { ((s + 0.055) / 1.055).powf(2.4) }
}

/// sRGB gamma compress: linear f32 → u8.
#[inline]
pub fn srgb_encode(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 { c * 12.92 }
            else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

// ── color config ──────────────────────────────────────────────────────────────

/// System-wide color management configuration, loaded once at supervisor start.
#[derive(Clone, Debug)]
pub struct ColorConfig {
    /// ICC profile of the currently active display.
    /// `ProfileId::SRGB_BUILTIN` when no EDID profile is found.
    pub display_profile: ProfileId,
    /// The compositor's blending color space.
    /// `LinearSrgb` on sRGB displays; `LinearSrgb` on P3 (after transform).
    pub working_space:   ColorSpace,
    /// Whether the full ICC pipeline is enabled.
    /// False on mcu-minimal and iot-edge/robotics-rt.
    pub icc_enabled:     bool,
    /// Whether the display supports wide gamut (P3 or wider).
    pub wide_gamut:      bool,
}

impl Default for ColorConfig {
    fn default() -> Self {
        ColorConfig {
            display_profile: ProfileId::SRGB_BUILTIN,
            working_space:   ColorSpace::LinearSrgb,
            icc_enabled:     false,
            wide_gamut:      false,
        }
    }
}
```

---

## 2. Platform Configuration (`supervisor/src/color/config.rs`)

```rust
//! supervisor/src/color/config.rs
//! Maps the platform profile (loaded in Round 12) to color management
//! capabilities.  Called once at supervisor startup; result is immutable.

use crate::color::ColorConfig;
use crate::profile::Platform;

/// Build a `ColorConfig` for the given platform.
///
/// mcu-minimal and iot/robotics get sRGB passthrough (no ICC);
/// mobile, desktop-full, and server-headless get the full pipeline.
pub fn color_config_for_platform(platform: Platform) -> ColorConfig {
    match platform {
        Platform::McuMinimal | Platform::IotEdge | Platform::RoboticsRt => {
            // No ICC parse, no qcms linkage, no EDID read.
            ColorConfig {
                icc_enabled: false,
                wide_gamut:  false,
                ..ColorConfig::default()
            }
        }
        Platform::Mobile | Platform::DesktopFull | Platform::ServerHeadless => {
            // EDID read and full ICC attempted; fallback to sRGB if EDID absent.
            ColorConfig {
                icc_enabled: true,
                ..ColorConfig::default() // display_profile set after EDID parse
            }
        }
    }
}

/// Per-platform limits.
pub struct ColorLimits {
    /// Maximum ICC file size in bytes.  4 MB everywhere ICC is enabled.
    pub max_profile_bytes: usize,
    /// Maximum profiles in the registry LRU.
    pub max_profiles:      usize,
    /// Maximum cached transforms.
    pub max_transforms:    usize,
}

pub fn limits_for_platform(platform: Platform) -> ColorLimits {
    match platform {
        Platform::McuMinimal => ColorLimits {
            max_profile_bytes: 0,
            max_profiles:      0,
            max_transforms:    0,
        },
        Platform::IotEdge | Platform::RoboticsRt => ColorLimits {
            max_profile_bytes: 0,
            max_profiles:      0,
            max_transforms:    4, // analytic sRGB↔Linear only
        },
        Platform::Mobile => ColorLimits {
            max_profile_bytes: 4 * 1024 * 1024,
            max_profiles:      8,
            max_transforms:    16,
        },
        Platform::DesktopFull | Platform::ServerHeadless => ColorLimits {
            max_profile_bytes: 4 * 1024 * 1024,
            max_profiles:      16,
            max_transforms:    32,
        },
    }
}
```

---

## 3. ICC Profile Handling (`supervisor/src/color/profile.rs`)

```rust
//! supervisor/src/color/profile.rs
//! ICC profile loading, parsing via qcms, and metadata extraction.
//! All qcms calls are wrapped in catch_unwind; panics return IccError::ParsePanic.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;

use crate::color::{ColorSpace, IccProfile, ProfileId};

/// Error conditions specific to profile loading.
#[derive(Debug, Clone)]
pub enum IccError {
    /// File too large (> 4 MB).
    FileTooLarge { bytes: usize, limit: usize },
    /// ICC parse failed; message from qcms.
    ParseFailed(String),
    /// qcms panicked during parse.  Profile is untrusted; discard.
    ParsePanic,
    /// Path is outside permitted directories.
    PathTraversal(String),
    /// ICC is disabled on this platform.
    Disabled,
    /// File I/O error.
    IoError(String),
    /// Handle unknown or already freed.
    InvalidHandle(u32),
}

impl std::fmt::Display for IccError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl std::error::Error for IccError {}

/// Maximum ICC profile file size.  Enforced before any parse attempt.
pub const MAX_ICC_BYTES: usize = 4 * 1024 * 1024;

/// Validate that `path` is a descendant of one of `allowed_roots`.
pub fn validate_profile_path(
    path: &Path,
    allowed_roots: &[&Path],
) -> Result<(), IccError> {
    let canonical = path
        .canonicalize()
        .map_err(|e| IccError::IoError(e.to_string()))?;
    for root in allowed_roots {
        if canonical.starts_with(root) {
            return Ok(());
        }
    }
    Err(IccError::PathTraversal(path.display().to_string()))
}

/// Load and parse an ICC profile from `path`.
///
/// Allowed roots are:
///   - `/usr/share/color/icc/`  (system profiles)
///   - app bundle `profiles/` directory (declared in manifest)
///
/// `catch_unwind` isolates adversarial ICC content that triggers qcms assertions.
#[cfg(feature = "color-icc")]
pub fn load_icc_profile(
    path: &Path,
    id: ProfileId,
    allowed_roots: &[&Path],
) -> Result<IccProfile, IccError> {
    validate_profile_path(path, allowed_roots)?;

    let data = std::fs::read(path)
        .map_err(|e| IccError::IoError(e.to_string()))?;

    if data.len() > MAX_ICC_BYTES {
        return Err(IccError::FileTooLarge {
            bytes: data.len(),
            limit: MAX_ICC_BYTES,
        });
    }

    let data_arc = Arc::new(data);
    parse_icc_profile_bytes(Arc::clone(&data_arc), id)
}

/// Parse an ICC profile from raw bytes.
/// Called both from `load_icc_profile` and from EDID-embedded profiles.
pub fn parse_icc_profile_bytes(
    data: Arc<Vec<u8>>,
    id: ProfileId,
) -> Result<IccProfile, IccError> {
    let data_clone = Arc::clone(&data);
    let result = catch_unwind(AssertUnwindSafe(move || {
        parse_inner(&data_clone)
    }));

    match result {
        Ok(Ok((space, desc))) => Ok(IccProfile {
            id,
            space,
            byte_size: data.len(),
            data,
            desc,
        }),
        Ok(Err(msg)) => Err(IccError::ParseFailed(msg)),
        Err(_) => Err(IccError::ParsePanic),
    }
}

/// Inner parse using qcms (pure-Rust; no C FFI panic risk, but still isolated
/// because adversarial ICC files can trigger qcms debug assertions in dev builds).
#[cfg(feature = "color-icc")]
fn parse_inner(data: &[u8]) -> Result<(ColorSpace, String), String> {
    use qcms::Profile;
    let profile = Profile::new_from_slice(data)
        .ok_or_else(|| "qcms rejected profile".to_string())?;

    let space = qcms_color_space(&profile)?;
    let desc = extract_desc(data);
    Ok((space, desc))
}

/// Stub used on platforms without `color-icc` feature.
#[cfg(not(feature = "color-icc"))]
fn parse_inner(_data: &[u8]) -> Result<(ColorSpace, String), String> {
    Err("ICC disabled on this platform".to_string())
}

#[cfg(feature = "color-icc")]
fn qcms_color_space(profile: &qcms::Profile) -> Result<ColorSpace, String> {
    // qcms exposes a color_space field as an ICCColorSpace enum.
    // Map to VyomaOS ColorSpace.
    match profile.color_space {
        qcms::ICCColorSpace::RGB  => Ok(ColorSpace::Srgb), // refined below
        qcms::ICCColorSpace::GRAY => Ok(ColorSpace::Grayscale),
        other => Err(format!("unsupported ICC color space {:?}", other)),
    }
    // Note: qcms does not distinguish sRGB from P3 by color_space alone;
    // the profile description string is used as a secondary heuristic.
    // Accurate gamut identification requires reading the rTRC/gTRC/bTRC
    // and the colorant matrix — deferred to a future round if P3 detection
    // is needed at the profile level rather than via EDID.
}

/// Extract the ICC profile description string (tag `desc`).
/// Returns a 256-char-max ASCII string, falling back to "<unknown>".
fn extract_desc(data: &[u8]) -> String {
    // Minimal hand-parse of the ICC `desc` tag to avoid a qcms API gap.
    // ICC header is 128 bytes; tag table starts at offset 128.
    if data.len() < 132 { return "<unknown>".to_string(); }
    let tag_count = u32::from_be_bytes([data[128], data[129], data[130], data[131]]) as usize;
    let table_start = 132usize;
    for i in 0..tag_count {
        let off = table_start + i * 12;
        if off + 12 > data.len() { break; }
        let sig = &data[off..off + 4];
        if sig == b"desc" {
            let data_off = u32::from_be_bytes([data[off+4], data[off+5], data[off+6], data[off+7]]) as usize;
            let data_len = u32::from_be_bytes([data[off+8], data[off+9], data[off+10], data[off+11]]) as usize;
            if data_off + data_len > data.len() || data_len < 12 { break; }
            // mluc/desc: first 4 bytes are type sig, skip 8-byte header, then UTF-16BE.
            let text_start = data_off + 12;
            let text_bytes = &data[text_start..data_off + data_len];
            let s: String = text_bytes
                .chunks(2)
                .filter_map(|c| if c.len() == 2 {
                    let cp = u16::from_be_bytes([c[0], c[1]]);
                    char::from_u32(cp as u32)
                } else { None })
                .take(256)
                .collect();
            if !s.is_empty() { return s; }
        }
    }
    "<unknown>".to_string()
}
```

---

## 4. Profile Registry (`supervisor/src/color/registry.rs`)

```rust
//! supervisor/src/color/registry.rs
//! LRU profile registry, shared across all apps.
//! Maximum 16 entries; LRU eviction when full.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::color::{ColorConfig, IccProfile, ProfileId};
use crate::color::profile::{IccError, parse_icc_profile_bytes};

/// A thread-safe LRU registry of parsed ICC profiles.
pub struct ProfileRegistry {
    inner: Mutex<RegistryInner>,
}

struct RegistryInner {
    profiles:   HashMap<ProfileId, Arc<IccProfile>>,
    lru_order:  Vec<ProfileId>,   // front = most-recently-used
    next_id:    u32,
    max_count:  usize,
    /// Total bytes held across all cached profiles.
    total_bytes: usize,
    /// Per-app byte accounting: app_pid → bytes.
    per_app:    HashMap<u32, usize>,
    /// Max bytes any single app may hold.
    per_app_limit: usize,
}

impl ProfileRegistry {
    /// Create registry bounded by `max_count` profiles.
    /// `per_app_limit` is the maximum ICC bytes one app PID may register.
    pub fn new(max_count: usize, per_app_limit: usize) -> Self {
        ProfileRegistry {
            inner: Mutex::new(RegistryInner {
                profiles:      HashMap::new(),
                lru_order:     Vec::with_capacity(max_count),
                next_id:       1, // 0 = SRGB_BUILTIN
                max_count,
                total_bytes:   0,
                per_app:       HashMap::new(),
                per_app_limit,
            }),
        }
    }

    /// Register a new profile from raw ICC bytes.
    /// Returns its assigned `ProfileId`.
    pub fn register_bytes(
        &self,
        data: Vec<u8>,
        app_pid: u32,
    ) -> Result<ProfileId, IccError> {
        let mut g = self.inner.lock().unwrap();

        // Per-app accounting check.
        let app_used = *g.per_app.get(&app_pid).unwrap_or(&0);
        if app_used + data.len() > g.per_app_limit {
            return Err(IccError::FileTooLarge {
                bytes: app_used + data.len(),
                limit: g.per_app_limit,
            });
        }

        // Evict LRU entry if at capacity.
        if g.profiles.len() >= g.max_count {
            if let Some(evict_id) = g.lru_order.pop() {
                if let Some(evicted) = g.profiles.remove(&evict_id) {
                    g.total_bytes = g.total_bytes.saturating_sub(evicted.byte_size);
                    // Remove from per-app (best-effort; we don't track which app owns each profile).
                }
            }
        }

        let id = ProfileId(g.next_id);
        g.next_id += 1;

        let data_arc = Arc::new(data);
        let profile = parse_icc_profile_bytes(Arc::clone(&data_arc), id)?;
        let profile_arc = Arc::new(profile);

        g.total_bytes += profile_arc.byte_size;
        *g.per_app.entry(app_pid).or_insert(0) += profile_arc.byte_size;
        g.profiles.insert(id, profile_arc);
        g.lru_order.insert(0, id); // most-recently-used at front

        Ok(id)
    }

    /// Look up a profile by id, promoting to MRU position.
    pub fn get(&self, id: ProfileId) -> Option<Arc<IccProfile>> {
        let mut g = self.inner.lock().unwrap();
        if let Some(pos) = g.lru_order.iter().position(|&x| x == id) {
            g.lru_order.remove(pos);
            g.lru_order.insert(0, id);
        }
        g.profiles.get(&id).cloned()
    }

    /// Release all profiles owned by `app_pid` (called on app exit).
    pub fn release_app(&self, app_pid: u32) {
        let mut g = self.inner.lock().unwrap();
        let freed = g.per_app.remove(&app_pid).unwrap_or(0);
        g.total_bytes = g.total_bytes.saturating_sub(freed);
        // Individual profile entries remain (shared); they are evicted by LRU.
        // This prevents use-after-free: another app may still hold a handle to the same profile.
    }

    /// Total bytes currently held in the registry.
    pub fn total_bytes(&self) -> usize {
        self.inner.lock().unwrap().total_bytes
    }
}
```

---

## 5. Transform Cache (`supervisor/src/color/transform.rs`)

```rust
//! supervisor/src/color/transform.rs
//! Build and cache ColorTransform objects.
//! Cache key: (from: ColorSpace, to: ColorSpace, intent: RenderingIntent).
//! Maximum 32 entries; LRU eviction.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use crate::color::{
    ColorSpace, ColorTransform, IccProfile, QcmsTransformHandle, RenderingIntent,
};
use crate::color::registry::ProfileRegistry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TransformKey {
    from:   ColorSpace,
    to:     ColorSpace,
    intent: RenderingIntent,
}

pub struct TransformCache {
    inner: Mutex<CacheInner>,
}

struct CacheInner {
    entries:   HashMap<TransformKey, Arc<ColorTransform>>,
    lru_order: Vec<TransformKey>,
    max_count: usize,
}

impl TransformCache {
    pub fn new(max_count: usize) -> Self {
        TransformCache {
            inner: Mutex::new(CacheInner {
                entries:   HashMap::new(),
                lru_order: Vec::with_capacity(max_count),
                max_count,
            }),
        }
    }

    /// Get or create a transform.
    ///
    /// If both spaces are equal, returns an identity transform (no qcms call).
    /// If the transform is sRGB↔LinearSrgb, uses the analytic path.
    /// Otherwise invokes qcms inside `catch_unwind`.
    pub fn get_or_create(
        &self,
        from: ColorSpace,
        to:   ColorSpace,
        intent: RenderingIntent,
        registry: &ProfileRegistry,
        from_profile: Arc<IccProfile>,
        to_profile:   Arc<IccProfile>,
    ) -> Result<Arc<ColorTransform>, String> {
        let key = TransformKey { from, to, intent };

        {
            let mut g = self.inner.lock().unwrap();
            if let Some(pos) = g.lru_order.iter().position(|&k| k == key) {
                g.lru_order.remove(pos);
                g.lru_order.insert(0, key);
            }
            if let Some(t) = g.entries.get(&key) {
                return Ok(Arc::clone(t));
            }
        }

        // Build outside the lock to avoid holding it during (potentially slow) qcms init.
        let transform = build_transform(from, to, intent, from_profile, to_profile)?;
        let arc = Arc::new(transform);

        {
            let mut g = self.inner.lock().unwrap();
            if g.entries.len() >= g.max_count {
                if let Some(evict_key) = g.lru_order.pop() {
                    g.entries.remove(&evict_key);
                }
            }
            g.entries.insert(key, Arc::clone(&arc));
            g.lru_order.insert(0, key);
        }

        Ok(arc)
    }
}

fn build_transform(
    from:         ColorSpace,
    to:           ColorSpace,
    intent:       RenderingIntent,
    from_profile: Arc<IccProfile>,
    to_profile:   Arc<IccProfile>,
) -> Result<ColorTransform, String> {
    // Identity: no qcms needed.
    if from == to {
        return Ok(ColorTransform { from, to, intent, inner: None });
    }

    // Analytic paths: sRGB ↔ LinearSrgb.
    if !from.requires_icc_transform(to) {
        return Ok(ColorTransform { from, to, intent, inner: None });
    }

    // Full ICC path via qcms.
    #[cfg(feature = "color-icc")]
    {
        let from_data  = Arc::clone(&from_profile.data);
        let to_data    = Arc::clone(&to_profile.data);
        let intent_raw = intent.as_u8();

        let result = catch_unwind(AssertUnwindSafe(move || -> Result<*mut std::ffi::c_void, String> {
            use qcms::{Profile, DataType, Intent};
            let src = Profile::new_from_slice(&from_data)
                .ok_or_else(|| "qcms: bad source profile".to_string())?;
            let dst = Profile::new_from_slice(&to_data)
                .ok_or_else(|| "qcms: bad dest profile".to_string())?;
            let qcms_intent = match intent_raw {
                0 => Intent::Perceptual,
                1 => Intent::RelativeColorimetric,
                2 => Intent::Saturation,
                _ => Intent::AbsoluteColorimetric,
            };
            let t = qcms::Transform::new_to(
                &src, &dst, DataType::RGBA8, DataType::RGBA8, qcms_intent,
            ).ok_or_else(|| "qcms: transform creation failed".to_string())?;
            // Leak the transform pointer; we manage lifetime via QcmsTransformHandle.
            Ok(Box::into_raw(Box::new(t)) as *mut std::ffi::c_void)
        }));

        match result {
            Ok(Ok(ptr)) => {
                Ok(ColorTransform {
                    from, to, intent,
                    inner: Some(Arc::new(QcmsTransformHandle { ptr })),
                })
            }
            Ok(Err(msg)) => Err(msg),
            Err(_) => Err("qcms transform panicked".to_string()),
        }
    }
    #[cfg(not(feature = "color-icc"))]
    {
        Err("ICC disabled on this platform".to_string())
    }
}
```

---

## 6. Display Profile Detection (`supervisor/src/color/display.rs`)

```rust
//! supervisor/src/color/display.rs
//! Detect the ICC profile for the active display via DRM EDID.
//! Falls back to sRGB when EDID is absent or unreadable.

use std::path::Path;
use crate::color::{ColorConfig, ColorSpace, ProfileId};
use crate::color::profile::{IccError, parse_icc_profile_bytes};
use crate::color::registry::ProfileRegistry;

/// Path where the kernel exposes EDID data for the first connected display.
/// Typically `/sys/class/drm/card0-HDMI-A-1/edid` or similar.
const EDID_GLOB_PATTERN: &str = "/sys/class/drm/card*/*/edid";

/// Attempt to read EDID bytes from the DRM sysfs path.
pub fn read_edid_bytes() -> Option<Vec<u8>> {
    // Walk /sys/class/drm/card*/*/edid; return first non-empty file.
    let drm_base = Path::new("/sys/class/drm");
    let card_dir = std::fs::read_dir(drm_base).ok()?;
    for entry in card_dir.flatten() {
        let connector_dir = std::fs::read_dir(entry.path()).ok();
        if let Some(connector_dir) = connector_dir {
            for conn_entry in connector_dir.flatten() {
                let edid_path = conn_entry.path().join("edid");
                if let Ok(data) = std::fs::read(&edid_path) {
                    if !data.is_empty() { return Some(data); }
                }
            }
        }
    }
    None
}

/// Extract a color gamut hint from 128-byte EDID block.
///
/// Uses the Chromaticity Coordinates block (bytes 25–34) to detect
/// whether the display primaries fall outside the sRGB triangle.
pub fn detect_gamut_from_edid(edid: &[u8]) -> ColorSpace {
    if edid.len() < 35 { return ColorSpace::Srgb; }

    // Chromaticity data at bytes 25–34, encoded as 10-bit fixed-point.
    // Extract red primary X coordinate.
    let rx_lsb = ((edid[25] >> 6) & 0x03) as u16;
    let rx_msb = edid[27] as u16;
    let rx = ((rx_msb << 2) | rx_lsb) as f32 / 1024.0;

    // sRGB red primary X ≈ 0.640; Display P3 red X ≈ 0.680.
    // If red X > 0.670, treat as wide-gamut (P3 or wider).
    if rx > 0.670 {
        ColorSpace::DisplayP3
    } else {
        ColorSpace::Srgb
    }
}

/// Detect whether the EDID has an embedded ICC profile in an extension block.
/// EDID 1.4 does not carry ICC profiles; this is vendor-specific.
/// Returns None for standard EDID.
pub fn extract_embedded_icc(edid: &[u8]) -> Option<Vec<u8>> {
    // Standard EDID 1.4 does not embed ICC; return None.
    // Vendor extension detection would go here for DisplayHDR / ColourPrimaries ext.
    let _ = edid;
    None
}

/// High-level: populate `ColorConfig.display_profile` and `wide_gamut` flag
/// from the active display's EDID.
///
/// Returns an updated `ColorConfig`.
pub fn detect_display_profile(
    mut config: ColorConfig,
    registry: &ProfileRegistry,
) -> ColorConfig {
    if !config.icc_enabled {
        return config;
    }

    let edid = match read_edid_bytes() {
        Some(e) => e,
        None => {
            log::warn!("[color] No EDID found; using sRGB passthrough");
            return config;
        }
    };

    // Check for wide gamut primaries.
    let detected_space = detect_gamut_from_edid(&edid);
    config.wide_gamut = detected_space != ColorSpace::Srgb;

    // Attempt embedded ICC (rare); ignore on failure.
    if let Some(icc_bytes) = extract_embedded_icc(&edid) {
        match registry.register_bytes(icc_bytes, 0 /* supervisor PID */) {
            Ok(id) => {
                config.display_profile = id;
                log::info!("[color] Loaded display ICC from EDID extension, id={}", id.0);
            }
            Err(e) => {
                log::warn!("[color] EDID ICC parse failed: {e}; falling back to sRGB");
            }
        }
    }

    if config.wide_gamut {
        log::info!("[color] Wide-gamut display detected (P3 primaries)");
    }

    config
}
```

---

## 7. Pixel Conversion (`supervisor/src/color/convert.rs`)

```rust
//! supervisor/src/color/convert.rs
//! Bulk pixel conversion: apply a ColorTransform to a slice of BGRA32 pixels.
//! All conversion paths wrapped in catch_unwind.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::color::{srgb_decode, srgb_encode, Color, ColorSpace, ColorTransform};
use crate::image::PixelFormat;

/// Error from pixel conversion.
#[derive(Debug, Clone)]
pub enum ConvertError {
    /// Pixel buffer length does not match w*h*bpp.
    BadBufferSize { expected: usize, got: usize },
    /// qcms transform apply panicked.
    TransformPanic,
    /// Unsupported pixel format for this conversion path.
    UnsupportedFormat(String),
    /// ICC disabled on this platform.
    Disabled,
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self) }
}

/// Convert `width × height` pixels in `src` (format `fmt`) using `transform`.
///
/// Returns a new `Vec<u8>` in the same pixel format but with colors remapped
/// into the target color space.  For identity transforms, returns a clone of src.
pub fn convert_pixels(
    src:       &[u8],
    fmt:       PixelFormat,
    width:     u32,
    height:    u32,
    transform: &ColorTransform,
) -> Result<Vec<u8>, ConvertError> {
    let bpp = match fmt {
        PixelFormat::Bgra32    => 4usize,
        PixelFormat::Rgb24     => 3,
        PixelFormat::Grayscale8 => 1,
        other => return Err(ConvertError::UnsupportedFormat(format!("{:?}", other))),
    };

    let expected = width as usize * height as usize * bpp;
    if src.len() != expected {
        return Err(ConvertError::BadBufferSize { expected, got: src.len() });
    }

    // Identity: same space, return clone.
    if transform.inner.is_none() {
        // Check if analytic gamma path is needed.
        if transform.from == transform.to {
            return Ok(src.to_vec());
        }
        // sRGB ↔ LinearSrgb analytic path.
        return convert_gamma_analytic(src, fmt, transform.from, transform.to);
    }

    // Full ICC path via qcms.
    #[cfg(feature = "color-icc")]
    {
        let src_owned: Vec<u8> = src.to_vec();
        let handle = Arc::clone(transform.inner.as_ref().unwrap());
        let result = catch_unwind(AssertUnwindSafe(move || -> Vec<u8> {
            let mut dst = vec![0u8; src_owned.len()];
            // qcms transform takes *const u8 + *mut u8 + count (in pixels).
            let pixel_count = (src_owned.len() / bpp) as u32;
            unsafe {
                qcms_sys::qcms_transform_data(
                    handle.ptr as *mut _,
                    src_owned.as_ptr(),
                    dst.as_mut_ptr(),
                    pixel_count as usize,
                );
            }
            dst
        }));
        result.map_err(|_| ConvertError::TransformPanic)
    }
    #[cfg(not(feature = "color-icc"))]
    {
        Err(ConvertError::Disabled)
    }
}

/// Analytic sRGB ↔ LinearSrgb conversion.
fn convert_gamma_analytic(
    src:  &[u8],
    fmt:  PixelFormat,
    from: ColorSpace,
    to:   ColorSpace,
) -> Result<Vec<u8>, ConvertError> {
    match fmt {
        PixelFormat::Bgra32 => {
            let mut dst = vec![0u8; src.len()];
            for (i, chunk) in src.chunks_exact(4).enumerate() {
                let base = i * 4;
                match (from, to) {
                    (ColorSpace::Srgb, ColorSpace::LinearSrgb) => {
                        dst[base]   = (srgb_decode(chunk[0]) * 255.0 + 0.5) as u8; // B
                        dst[base+1] = (srgb_decode(chunk[1]) * 255.0 + 0.5) as u8; // G
                        dst[base+2] = (srgb_decode(chunk[2]) * 255.0 + 0.5) as u8; // R
                        dst[base+3] = chunk[3]; // A passthrough
                    }
                    (ColorSpace::LinearSrgb, ColorSpace::Srgb) => {
                        dst[base]   = srgb_encode(chunk[0] as f32 / 255.0); // B
                        dst[base+1] = srgb_encode(chunk[1] as f32 / 255.0); // G
                        dst[base+2] = srgb_encode(chunk[2] as f32 / 255.0); // R
                        dst[base+3] = chunk[3]; // A passthrough
                    }
                    _ => {
                        dst[base..base+4].copy_from_slice(chunk);
                    }
                }
            }
            Ok(dst)
        }
        _ => Err(ConvertError::UnsupportedFormat(format!("{:?}", fmt))),
    }
}
```

---

## 8. WIT Interface (`vyoma:color@1.0.0`)

```wit
// wit/vyoma-color.wit
// WIT interface for color management.
// Compatible with: mobile, desktop-full, server-headless.
// On mcu-minimal / iot-edge / robotics-rt: all functions return error("disabled").

package vyoma:color@1.0.0;

interface color-management {
  // ── color space and intent enumerations ──────────────────────────────────

  /// Wire-stable u8 encoding of ColorSpace.
  /// 0=srgb 1=linear-srgb 2=display-p3 3=adobe-rgb 4=bt2020 5=grayscale
  type color-space = u8;

  /// Wire-stable u8 encoding of RenderingIntent.
  /// 0=perceptual 1=relative-colorimetric 2=saturation 3=absolute-colorimetric
  type rendering-intent = u8;

  // ── profile management ───────────────────────────────────────────────────

  /// Load an ICC profile from a path declared in the app's
  /// capabilities.color.profiles list.
  ///
  /// Returns an opaque profile handle (u32).
  /// Builtin sRGB handle is always 0.
  load-profile: func(path: string) -> result<u32, string>;

  /// Unload a previously loaded profile, freeing its registry slot.
  /// Passing handle 0 (sRGB builtin) is a no-op.
  unload-profile: func(handle: u32) -> result<_, string>;

  // ── transform management ─────────────────────────────────────────────────

  /// Create a color transform from profile `from-handle` to profile `to-handle`.
  ///
  /// Returns an opaque transform handle (u32).
  /// Transforms are cached internally; creating the same (from, to, intent)
  /// combination twice returns different handles backed by the same object.
  create-transform: func(
    from-handle: u32,
    to-handle:   u32,
    intent:      rendering-intent,
  ) -> result<u32, string>;

  /// Release a transform handle.  The underlying object is freed when all
  /// handles referencing it are released.
  destroy-transform: func(handle: u32) -> result<_, string>;

  // ── pixel conversion ─────────────────────────────────────────────────────

  /// Apply a color transform to a pixel buffer.
  ///
  /// `pixels` must be BGRA32 (4 bytes/pixel), length == width * height * 4.
  /// Returns a new buffer of the same size with transformed colors.
  ///
  /// Note: this call copies pixels across the WASM boundary twice.
  /// For large buffers, prefer `apply-transform-in-place` with a shared
  /// memory region (see future spec: shared-memory extension).
  apply-transform: func(
    transform-handle: u32,
    pixels:           list<u8>,
    width:            u32,
    height:           u32,
  ) -> result<list<u8>, string>;

  // ── display information ──────────────────────────────────────────────────

  /// Return the color space of the primary display as a `color-space` u8.
  /// Always returns 0 (sRGB) on platforms without ICC support.
  display-color-space: func() -> color-space;

  /// Return the ProfileId (as u32) of the active display ICC profile.
  /// Returns 0 (sRGB builtin) when no display profile is loaded.
  display-profile-handle: func() -> u32;

  /// Return true if the primary display supports wide gamut (P3 or wider).
  display-wide-gamut: func() -> bool;

  // ── color space conversion helpers ───────────────────────────────────────

  /// Convert a single RGBA color value between color spaces analytically.
  /// r, g, b, a are in [0.0, 1.0] linear-light.
  /// Only sRGB ↔ LinearSrgb conversions are supported analytically;
  /// all others require a transform created via create-transform.
  convert-color: func(
    r: float32, g: float32, b: float32, a: float32,
    from: color-space,
    to:   color-space,
  ) -> result<tuple<float32, float32, float32, float32>, string>;
}

world color-guest {
  import color-management;
}
```

---

## 9. WIT Host Handlers (`supervisor/src/color/wit_handlers.rs`)

```rust
//! supervisor/src/color/wit_handlers.rs
//! Dispatch table for the vyoma:color@1.0.0 WIT interface.
//! Each function validates capability, routes to color subsystem, returns wire types.

use std::sync::Arc;

use crate::app_state::AppState;
use crate::color::{ColorConfig, ColorSpace, RenderingIntent};
use crate::color::convert::{convert_pixels, ConvertError};
use crate::color::registry::ProfileRegistry;
use crate::color::transform::TransformCache;
use crate::image::PixelFormat;

/// State injected into WIT host call handlers.
pub struct ColorHostState {
    pub registry:  Arc<ProfileRegistry>,
    pub transforms: Arc<TransformCache>,
    pub config:    Arc<ColorConfig>,
    /// Allowed root paths for ICC profile files (populated from app manifest).
    pub allowed_roots: Vec<std::path::PathBuf>,
    pub app_pid:   u32,
}

/// host call: load-profile(path) -> result<u32, string>
pub fn host_load_profile(state: &ColorHostState, path: &str) -> Result<u32, String> {
    if !state.config.icc_enabled {
        return Err("ICC disabled on this platform".to_string());
    }
    let p = std::path::Path::new(path);
    let roots: Vec<&std::path::Path> = state.allowed_roots.iter().map(|r| r.as_path()).collect();
    crate::color::profile::validate_profile_path(p, &roots)
        .map_err(|e| e.to_string())?;
    let data = std::fs::read(p)
        .map_err(|e| format!("read error: {e}"))?;
    let id = state.registry
        .register_bytes(data, state.app_pid)
        .map_err(|e| e.to_string())?;
    Ok(id.0)
}

/// host call: unload-profile(handle) -> result<_, string>
pub fn host_unload_profile(_state: &ColorHostState, _handle: u32) -> Result<(), String> {
    // LRU eviction handles actual memory release; this is a no-op for now.
    // Future: per-handle refcount decrement.
    Ok(())
}

/// host call: create-transform(from, to, intent) -> result<u32, string>
pub fn host_create_transform(
    state:       &ColorHostState,
    from_handle: u32,
    to_handle:   u32,
    intent_raw:  u8,
) -> Result<u32, String> {
    if !state.config.icc_enabled {
        return Err("ICC disabled on this platform".to_string());
    }
    let from_profile = if from_handle == 0 {
        srgb_builtin_profile()
    } else {
        state.registry.get(crate::color::ProfileId(from_handle))
            .ok_or_else(|| format!("unknown profile handle {from_handle}"))?
    };
    let to_profile = if to_handle == 0 {
        srgb_builtin_profile()
    } else {
        state.registry.get(crate::color::ProfileId(to_handle))
            .ok_or_else(|| format!("unknown profile handle {to_handle}"))?
    };
    let intent = RenderingIntent::from_u8(intent_raw)
        .ok_or_else(|| format!("unknown intent {intent_raw}"))?;
    let _transform = state.transforms.get_or_create(
        from_profile.space,
        to_profile.space,
        intent,
        &state.registry,
        from_profile,
        to_profile,
    )?;
    // Encode transform as u32 handle: pack (from, to, intent) into 10 bits.
    let handle = (from_handle & 0xFF) | ((to_handle & 0xFF) << 8) | ((intent_raw as u32) << 16);
    Ok(handle)
}

/// host call: apply-transform(handle, pixels, w, h) -> result<list<u8>, string>
pub fn host_apply_transform(
    state:  &ColorHostState,
    handle: u32,
    pixels: Vec<u8>,
    width:  u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    // Decode handle back to (from, to, intent).
    let from_raw  = (handle & 0xFF) as u8;
    let to_raw    = ((handle >> 8) & 0xFF) as u8;
    let intent_raw = ((handle >> 16) & 0xFF) as u8;

    let from   = ColorSpace::from_u8(from_raw).ok_or("bad from")?;
    let to     = ColorSpace::from_u8(to_raw).ok_or("bad to")?;
    let intent = RenderingIntent::from_u8(intent_raw).ok_or("bad intent")?;

    let from_profile = srgb_builtin_profile();
    let to_profile   = srgb_builtin_profile();

    let transform = state.transforms.get_or_create(
        from, to, intent, &state.registry, from_profile, to_profile,
    ).map_err(|e| e.to_string())?;

    convert_pixels(&pixels, PixelFormat::Bgra32, width, height, &transform)
        .map_err(|e| e.to_string())
}

/// host call: display-color-space() -> u8
pub fn host_display_color_space(state: &ColorHostState) -> u8 {
    if state.config.wide_gamut {
        ColorSpace::DisplayP3.as_u8()
    } else {
        ColorSpace::Srgb.as_u8()
    }
}

/// host call: display-wide-gamut() -> bool
pub fn host_display_wide_gamut(state: &ColorHostState) -> bool {
    state.config.wide_gamut
}

/// host call: display-profile-handle() -> u32
pub fn host_display_profile_handle(state: &ColorHostState) -> u32 {
    state.config.display_profile.0
}

/// host call: convert-color(r,g,b,a,from,to) -> result<(f32,f32,f32,f32), string>
pub fn host_convert_color(
    _state: &ColorHostState,
    r: f32, g: f32, b: f32, a: f32,
    from_raw: u8,
    to_raw:   u8,
) -> Result<(f32, f32, f32, f32), String> {
    let from = ColorSpace::from_u8(from_raw).ok_or("bad from")?;
    let to   = ColorSpace::from_u8(to_raw).ok_or("bad to")?;
    if from == to { return Ok((r, g, b, a)); }
    match (from, to) {
        (ColorSpace::LinearSrgb, ColorSpace::Srgb) => {
            let r2 = crate::color::srgb_encode(r) as f32 / 255.0;
            let g2 = crate::color::srgb_encode(g) as f32 / 255.0;
            let b2 = crate::color::srgb_encode(b) as f32 / 255.0;
            Ok((r2, g2, b2, a))
        }
        (ColorSpace::Srgb, ColorSpace::LinearSrgb) => {
            let r2 = crate::color::srgb_decode((r * 255.0) as u8);
            let g2 = crate::color::srgb_decode((g * 255.0) as u8);
            let b2 = crate::color::srgb_decode((b * 255.0) as u8);
            Ok((r2, g2, b2, a))
        }
        _ => Err("non-analytic conversion: use create-transform".to_string()),
    }
}

/// Construct a synthetic IccProfile representing the built-in sRGB.
fn srgb_builtin_profile() -> Arc<crate::color::IccProfile> {
    Arc::new(crate::color::IccProfile {
        id:        crate::color::ProfileId::SRGB_BUILTIN,
        space:     ColorSpace::Srgb,
        data:      Arc::new(vec![]),
        desc:      "sRGB IEC 61966-2-1".to_string(),
        byte_size: 0,
    })
}
```

---

## 10. Manifest Integration (`supervisor/src/color/manifest.rs`)

```rust
//! supervisor/src/color/manifest.rs
//! Parses `[capabilities.color]` from app vyoma.toml.

use serde::Deserialize;

/// Color management capabilities declared by an app.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ColorCapability {
    /// ICC profile paths bundled with the app, relative to bundle root.
    /// Supervisor loads these into the registry at app start.
    #[serde(default)]
    pub profiles: Vec<String>,

    /// The color space the app's pixel data is natively in.
    /// Supervisor uses this when color-managing surfaces for display.
    /// Default: "srgb"
    #[serde(default = "default_working_space")]
    pub working_space: String,

    /// Whether the app requests automatic color management of its surface.
    /// When true, the compositor transforms the app surface to display color space
    /// before blending.  Default: true.
    #[serde(default = "default_true")]
    pub auto_manage: bool,
}

fn default_working_space() -> String { "srgb".to_string() }
fn default_true() -> bool { true }

impl ColorCapability {
    /// Parse working_space string to ColorSpace enum.
    pub fn working_color_space(&self) -> crate::color::ColorSpace {
        match self.working_space.as_str() {
            "srgb"         => crate::color::ColorSpace::Srgb,
            "linear-srgb"  => crate::color::ColorSpace::LinearSrgb,
            "display-p3"   => crate::color::ColorSpace::DisplayP3,
            "adobe-rgb"    => crate::color::ColorSpace::AdobeRgb,
            "bt2020"       => crate::color::ColorSpace::Bt2020,
            "grayscale"    => crate::color::ColorSpace::Grayscale,
            other => {
                log::warn!("[color] unknown working_space '{}'; defaulting to sRGB", other);
                crate::color::ColorSpace::Srgb
            }
        }
    }
}
```

**Example `vyoma.toml` with color capabilities:**

```toml
[app]
name    = "photo-viewer"
version = "1.0.0"
wasm    = "photo-viewer.wasm"

[capabilities]
stdio   = true
display = true

[capabilities.color]
profiles     = ["profiles/display-p3.icc", "profiles/srgb.icc"]
working_space = "display-p3"
auto_manage  = true
```

**System profile without custom ICC:**

```toml
[capabilities.color]
# No profiles list: uses built-in sRGB only.
# working_space defaults to "srgb".
# auto_manage defaults to true.
```

---

## 11. Integration with R14 ImagePipeline

```rust
// In supervisor/src/image/mod.rs — extend ImageData with color transform method.

impl ImageData {
    /// Apply a color space transform to this image's pixel data in-place.
    ///
    /// Called by the display path when `ColorCapability.auto_manage == true`
    /// and the source image has embedded color space metadata.
    pub fn apply_color_transform(
        &mut self,
        transform: &crate::color::ColorTransform,
    ) -> Result<(), crate::color::convert::ConvertError> {
        let converted = crate::color::convert::convert_pixels(
            &self.pixels,
            self.format,
            self.width,
            self.height,
            transform,
        )?;
        self.pixels = converted;
        // Note: format stays the same; only the color values change.
        Ok(())
    }

    /// Color space this image's pixels are encoded in.
    /// Populated from EXIF ColorSpace tag or JPEG ICC chunk during decode.
    /// Defaults to Srgb.
    pub color_space: crate::color::ColorSpace,
}
```

---

## 12. Integration with R12 GPU Path

When an app uses `vyoma:gpu/graphics@1.0.0` to upload a texture, the color space
tag must accompany the texture descriptor so wgpu can apply a hardware gamma
expand if the display is wide-gamut.

```rust
// In supervisor/src/gpu/texture.rs — color space metadata on upload.
pub struct TextureUploadDesc {
    pub width:       u32,
    pub height:      u32,
    pub format:      wgpu::TextureFormat,
    /// Color space of the source pixel data.
    /// wgpu uses this to select the correct TextureFormat variant:
    ///   sRGB → TextureFormat::Rgba8UnormSrgb
    ///   Linear → TextureFormat::Rgba8Unorm
    ///   DisplayP3 → TextureFormat::Rgba8UnormSrgb (with transform LUT in shader)
    pub color_space: crate::color::ColorSpace,
}

pub fn upload_texture(
    device: &wgpu::Device,
    queue:  &wgpu::Queue,
    pixels: &[u8],
    desc:   &TextureUploadDesc,
) -> wgpu::Texture {
    let wgpu_format = match desc.color_space {
        crate::color::ColorSpace::Srgb | crate::color::ColorSpace::DisplayP3 => {
            wgpu::TextureFormat::Rgba8UnormSrgb
        }
        _ => wgpu::TextureFormat::Rgba8Unorm,
    };
    // ... create texture with wgpu_format
    todo!()
}
```

---

## 13. Surface Compositor Color Management

The compositor in `supervisor/src/compositor.rs` performs the final blit of each
app's `Surface` onto the framebuffer.  When `wide_gamut == true`, each surface
tagged with a color space must be transformed to the display color space before
blending:

```rust
// In supervisor/src/compositor.rs — color-aware blit pass.
fn blit_surface_color_managed(
    dst:       &mut Surface,
    src:       &Surface,
    src_space: ColorSpace,
    dst_space: ColorSpace,
    transforms: &TransformCache,
    registry:   &ProfileRegistry,
) {
    if src_space == dst_space || !dst.config.icc_enabled {
        // No transform needed; fall through to raw blit.
        blit_surface_raw(dst, src);
        return;
    }

    // Get or create the required transform.
    let srgb_profile = srgb_builtin_profile();
    let transform = transforms.get_or_create(
        src_space, dst_space, RenderingIntent::Perceptual,
        registry, Arc::clone(&srgb_profile), Arc::clone(&srgb_profile),
    ).expect("transform cache failure");

    // Convert pixels, then blit.
    match convert_pixels(
        &src.pixels, PixelFormat::Bgra32, src.width, src.height, &transform,
    ) {
        Ok(converted) => {
            let converted_surface = Surface {
                pixels: converted,
                width:  src.width,
                height: src.height,
                ..src.clone()
            };
            blit_surface_raw(dst, &converted_surface);
        }
        Err(e) => {
            log::error!("[compositor] color transform failed: {e}; blitting unconverted");
            blit_surface_raw(dst, src);
        }
    }
}
```

---

## 14. Platform Matrix Summary

| Platform | ICC Enabled | qcms Linked | Wide Gamut | Spaces Supported |
|---|---|---|---|---|
| mcu-minimal | No | No | No | sRGB passthrough only |
| iot-edge | No | No | No | sRGB passthrough only |
| robotics-rt | No | No | No | sRGB passthrough only |
| mobile | Yes | Yes | Detected from EDID | sRGB, LinearSRGB, DisplayP3 |
| desktop-full | Yes | Yes | Detected from EDID | All (sRGB, P3, AdobeRgb, BT.2020) |
| server-headless | Yes | Yes | No (no display) | sRGB→LinearSRGB for compositing |

**Cargo features:**

```toml
# supervisor/Cargo.toml
[features]
default = ["color-icc"]
color-icc = ["qcms"]

[dependencies]
qcms = { version = "0.9", optional = true }
```

On `mcu-minimal` and embedded targets, compile without `color-icc`:

```makefile
# Makefile (iot-edge profile)
SUPERVISOR_FEATURES=--no-default-features
```

---

## 15. Security Model

### 15.1 Adversarial ICC Files

Every call into qcms that touches untrusted bytes is wrapped in
`catch_unwind(AssertUnwindSafe(...))`.  qcms is pure Rust and raises Rust panics
rather than C undefined behavior, so `catch_unwind` is an effective isolation
boundary.  This is the critical advantage of qcms over lcms2: lcms2 is C code,
and a SIGSEGV from an out-of-bounds read in an adversarial ICC LUT table cannot
be caught by `catch_unwind`.  The decision to use qcms instead of lcms2 is
therefore a security requirement, not merely a convenience.

### 15.2 Known qcms Limitation

qcms (Firefox's pure-Rust ICC library) supports:
- sRGB ↔ any matrix-shaper profile (covers sRGB, DisplayP3, AdobeRgb)
- N-component LUT profiles for print (CMYK) are NOT supported
- Parametric curves and DeviceLink profiles are NOT supported

Applications requiring BT.2020 or HDR transfer functions must supply a matrix-
shaper profile.  The supervisor will reject ICC files whose profile class is not
`mntr` (Display) or `scnr` (Input scanner) at parse time, before the qcms call.

### 15.3 Filesystem Access

`host_load_profile` validates paths against `allowed_roots` before any file read.
The allowed roots are populated from the app's manifest `profiles` list at app
startup — no dynamic path construction is permitted at runtime.  Paths containing
`..` components are rejected by `Path::canonicalize`.

### 15.4 Per-App Memory Accounting

The `ProfileRegistry` tracks bytes-per-app-PID and enforces a per-app limit
(default 8 MB on desktop, 4 MB on mobile).  This prevents a single app from
exhausting the shared profile registry.

---

## 16. Unit Test Coverage

Each module ships a `#[cfg(test)]` block:

| Module | Key Tests |
|---|---|
| `profile.rs` | Parse built-in sRGB, reject truncated ICC, catch_unwind on zero-length, path traversal rejection |
| `registry.rs` | LRU eviction at capacity=2, per-app limit enforcement, release_app frees accounting |
| `transform.rs` | Identity returns None inner, sRGB↔Linear analytic round-trip, cache hit returns same Arc |
| `convert.rs` | BGRA32 sRGB→Linear expand matches manual calculation, bad buffer size returns error |
| `display.rs` | EDID with wide-gamut primary X > 0.670 detected as P3, EDID with sRGB primaries stays sRGB |
| `wit_handlers.rs` | load_profile rejects disallowed path, apply_transform identity passes through, disabled platform returns error |

---

## 17. Implementation File Index

| File | Responsibility | Max LOC |
|---|---|---|
| `supervisor/src/color/mod.rs` | Types: ColorSpace, RenderingIntent, ProfileId, IccProfile, ColorTransform, Color, ColorConfig | 250 |
| `supervisor/src/color/config.rs` | Platform capability matrix, ColorLimits | 80 |
| `supervisor/src/color/profile.rs` | ICC load/parse, path validation, catch_unwind | 220 |
| `supervisor/src/color/registry.rs` | LRU ProfileRegistry, per-app accounting | 130 |
| `supervisor/src/color/transform.rs` | TransformCache, build_transform via qcms | 160 |
| `supervisor/src/color/display.rs` | EDID read, gamut detection, display profile init | 100 |
| `supervisor/src/color/convert.rs` | convert_pixels, analytic sRGB↔Linear | 130 |
| `supervisor/src/color/wit_handlers.rs` | WIT dispatch, handle encoding, capability check | 180 |
| `supervisor/src/color/manifest.rs` | ColorCapability TOML deserialisation | 60 |
