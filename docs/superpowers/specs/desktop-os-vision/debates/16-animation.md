# Round 16 — Animation Engine
## VyomaOS Subsystem 16: Animation Engine (macOS equiv: Core Animation / CALayer)

**Date**: 2026-05-29  
**Status**: Architect Draft  
**Prior rounds**: R11 (Surface/VSync), R12 (GPU WIT), R13 (Font/watchdog), R14 (ImageData), R15 (ColorSpace)

---

## 1. Core Data Model

The animation engine is built around the `Layer` as the fundamental animatable unit. Each layer owns its geometric and visual properties, carries content (surface, image, or GPU texture), and participates in a tree rooted at a per-app `RootLayer`.

```rust
// supervisor/src/animation/mod.rs

use crate::display::SurfaceId;
use crate::images::ImageHandle;

pub type LayerId = u32;
pub type TransactionHandle = u32;
pub type AnimationKey = String;

/// Fundamental animatable unit. All fields represent the MODEL layer
/// (what the app declared). The PRESENTATION layer is maintained separately
/// in PresentationLayer for the currently interpolated state.
#[derive(Debug, Clone)]
pub struct Layer {
    pub id: LayerId,
    pub parent: Option<LayerId>,
    pub children: Vec<LayerId>,

    // --- Geometric properties (all in parent coordinate space unless noted) ---

    /// Center point in parent coordinate space. Default: (0, 0).
    pub position: [f32; 2],

    /// [x, y, w, h] in the layer's own coordinate space.
    /// x/y are typically 0,0 for non-scrolling layers.
    pub bounds: [f32; 4],

    /// The layer's origin point for transforms, expressed as a fraction of bounds.
    /// [0.5, 0.5] = center (default). [0.0, 0.0] = top-left.
    pub anchor_point: [f32; 2],

    /// Column-major 4x4 transform applied around anchor_point.
    pub transform: Transform3D,

    /// Additional sublayer transform applied to all children.
    pub sublayer_transform: Transform3D,

    // --- Visual properties ---

    /// 0.0 = fully transparent, 1.0 = fully opaque.
    pub opacity: f32,
    pub hidden: bool,
    pub corner_radius: f32,
    pub background_color: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub shadow: Option<LayerShadow>,
    pub mask_to_bounds: bool,

    // --- Z-ordering ---

    /// Stacking index among siblings. Higher = on top.
    pub z_position: f32,

    // --- Content ---
    pub contents: Option<LayerContents>,

    /// Whether implicit animations are enabled for this layer specifically.
    pub implicit_animations_enabled: bool,
}

impl Layer {
    pub fn new(id: LayerId) -> Self {
        Layer {
            id,
            parent: None,
            children: Vec::new(),
            position: [0.0, 0.0],
            bounds: [0.0, 0.0, 0.0, 0.0],
            anchor_point: [0.5, 0.5],
            transform: Transform3D::identity(),
            sublayer_transform: Transform3D::identity(),
            opacity: 1.0,
            hidden: false,
            corner_radius: 0.0,
            background_color: Color::TRANSPARENT,
            border_color: Color::TRANSPARENT,
            border_width: 0.0,
            shadow: None,
            mask_to_bounds: false,
            z_position: 0.0,
            contents: None,
            implicit_animations_enabled: true,
        }
    }
}

/// Layer content source: one of three origins from prior subsystems.
#[derive(Debug, Clone)]
pub enum LayerContents {
    /// Software-rendered surface from R11.
    Surface(SurfaceId),
    /// Decoded image from R14 (already in LinearLight RGBA32F per R15).
    Image(ImageHandle),
    /// Raw GPU texture handle from R12 wgpu::Texture.
    GpuTexture(u32),
}

/// Column-major 4x4 homogeneous transform matrix.
/// m[col][row], so m[3][0..2] is the translation column.
#[derive(Debug, Clone, Copy)]
pub struct Transform3D {
    pub m: [[f32; 4]; 4],
}

impl Transform3D {
    pub fn identity() -> Self {
        Transform3D {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Returns true if any element is NaN or Inf.
    pub fn is_valid(&self) -> bool {
        self.m.iter().flatten().all(|v| v.is_finite())
    }

    /// Sanitize: replace any NaN or Inf with identity element.
    pub fn sanitize(mut self) -> Self {
        let identity = Self::identity();
        for col in 0..4 {
            for row in 0..4 {
                if !self.m[col][row].is_finite() {
                    self.m[col][row] = identity.m[col][row];
                }
            }
        }
        self
    }

    pub fn translation(tx: f32, ty: f32) -> Self {
        let mut m = Self::identity();
        m.m[3][0] = tx;
        m.m[3][1] = ty;
        m
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        let mut m = Self::identity();
        m.m[0][0] = sx;
        m.m[1][1] = sy;
        m
    }

    pub fn rotation_z(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        let mut m = Self::identity();
        m.m[0][0] = c;
        m.m[0][1] = s;
        m.m[1][0] = -s;
        m.m[1][1] = c;
        m
    }

    /// Column-major matrix multiplication: self * rhs.
    pub fn mul(&self, rhs: &Transform3D) -> Transform3D {
        let mut out = [[0.0f32; 4]; 4];
        for col in 0..4 {
            for row in 0..4 {
                out[col][row] = (0..4).map(|k| self.m[k][row] * rhs.m[col][k]).sum();
            }
        }
        Transform3D { m: out }
    }
}

/// RGBA color in linear light (as per R15 ColorSpace subsystem).
/// All interpolation operates in this space.
#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
}

#[derive(Debug, Clone)]
pub struct LayerShadow {
    pub color: Color,
    pub opacity: f32,
    pub radius: f32,
    pub offset: [f32; 2],
}

/// Which animatable property a keyframe animation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimatableProperty {
    Position,
    Bounds,
    Opacity,
    Transform,
    CornerRadius,
    BackgroundColor,
    BorderColor,
    BorderWidth,
    ShadowOpacity,
    ZPosition,
}

/// A single interpolatable value for an AnimatableProperty.
#[derive(Debug, Clone)]
pub enum AnimatableValue {
    Float(f32),
    Point([f32; 2]),
    Rect([f32; 4]),
    Color(Color),
    Transform(Transform3D),
}

/// Timing function for an animation segment.
#[derive(Debug, Clone)]
pub enum AnimationTimingFunction {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Custom cubic Bezier control points (P1 and P2; P0=0,0 P3=1,1 implied).
    CubicBezier { cx1: f32, cy1: f32, cx2: f32, cy2: f32 },
    /// Physically-based spring. Supervisor determines criticality from params.
    Spring {
        mass: f32,
        stiffness: f32,
        damping: f32,
        /// Initial velocity to inherit (e.g. from gesture velocity).
        initial_velocity: f32,
    },
}

impl AnimationTimingFunction {
    /// id encoding used in stdout protocol and WIT u8 parameter.
    pub const LINEAR: u8 = 0;
    pub const EASE_IN: u8 = 1;
    pub const EASE_OUT: u8 = 2;
    pub const EASE_IN_OUT: u8 = 3;
    pub const SPRING_DEFAULT: u8 = 4;
    pub const CUBIC_BEZIER: u8 = 5;
}

/// How the animation affects the layer after it ends or before it begins.
#[derive(Debug, Clone, Copy)]
pub enum FillMode {
    /// On end: animated value is removed, property snaps to model value.
    Removed,
    /// On end: animated value is retained (presentation stays at final keyframe).
    Forwards,
    /// On begin: animated value is applied immediately (before begin_time_ms).
    Backwards,
    /// Combines Forwards + Backwards.
    Both,
}

/// A single keyframe in a KeyframeAnimation.
#[derive(Debug, Clone)]
pub struct Keyframe {
    /// Normalized time in [0.0, 1.0]. First must be 0.0, last must be 1.0.
    pub time: f32,
    pub value: AnimatableValue,
    /// Per-segment timing override. None = use animation's root timing function.
    pub timing: Option<AnimationTimingFunction>,
}

/// A complete keyframe animation specification attached to a layer.
#[derive(Debug, Clone)]
pub struct KeyframeAnimation {
    pub property: AnimatableProperty,
    pub keyframes: Vec<Keyframe>,
    /// Total duration of one cycle in milliseconds. Max 30,000 (30s).
    pub duration_ms: u32,
    /// Number of repetitions. f32::INFINITY = loop forever.
    pub repeat_count: f32,
    pub autoreverse: bool,
    pub fill_mode: FillMode,
    pub timing: AnimationTimingFunction,
    /// Offset from transaction begin_time. Negative = animation started in past.
    pub begin_time_ms: i64,
}

/// An implicit or transaction-driven property change group.
#[derive(Debug, Clone)]
pub struct AnimationTransaction {
    pub handle: TransactionHandle,
    /// Owning app PID.
    pub pid: u32,
    /// Duration in ms. 0 = immediate (no interpolation, jump to value).
    pub duration_ms: u32,
    pub timing: AnimationTimingFunction,
    pub changes: Vec<LayerChange>,
    /// If set, supervisor sends VYOMA_ANIM_COMPLETE:<token> to app stdin when done.
    pub completion_token: Option<u32>,
    /// Wall-clock ms at which the transaction was committed. Set by supervisor.
    pub committed_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct LayerChange {
    pub layer_id: LayerId,
    pub property: AnimatableProperty,
    pub new_value: AnimatableValue,
}
```

---

## 2. Animation Engine — Core Execution Model

### 2.1 Two-Layer Model

```rust
// supervisor/src/animation/presentation.rs

/// The model layer holds what the app declared via transactions.
/// It is updated atomically when a transaction commits.
pub type ModelLayer = Layer;

/// The presentation layer holds the current rendered state at the
/// current vsync tick, after all animations have been advanced.
/// Apps can query presentation values via WIT hostcalls.
#[derive(Debug, Clone)]
pub struct PresentationLayer {
    pub id: LayerId,
    pub position: [f32; 2],
    pub bounds: [f32; 4],
    pub anchor_point: [f32; 2],
    pub transform: Transform3D,
    pub opacity: f32,
    pub hidden: bool,
    pub corner_radius: f32,
    pub background_color: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub shadow_opacity: f32,
    pub z_position: f32,
    pub contents: Option<LayerContents>,
    /// True if any animation touched this layer this tick.
    pub dirty: bool,
}

impl PresentationLayer {
    /// Initialize presentation layer from the model layer (no animation yet).
    pub fn from_model(model: &ModelLayer) -> Self {
        PresentationLayer {
            id: model.id,
            position: model.position,
            bounds: model.bounds,
            anchor_point: model.anchor_point,
            transform: model.transform,
            opacity: model.opacity,
            hidden: model.hidden,
            corner_radius: model.corner_radius,
            background_color: model.background_color,
            border_color: model.border_color,
            border_width: model.border_width,
            shadow_opacity: model.shadow.as_ref().map(|s| s.opacity).unwrap_or(0.0),
            z_position: model.z_position,
            contents: model.contents.clone(),
            dirty: false,
        }
    }

    /// Apply an interpolated value to the appropriate presentation field.
    pub fn apply(&mut self, property: AnimatableProperty, value: &AnimatableValue) {
        self.dirty = true;
        match (property, value) {
            (AnimatableProperty::Position, AnimatableValue::Point(p)) => self.position = *p,
            (AnimatableProperty::Bounds, AnimatableValue::Rect(r)) => self.bounds = *r,
            (AnimatableProperty::Opacity, AnimatableValue::Float(v)) => {
                self.opacity = v.clamp(0.0, 1.0)
            }
            (AnimatableProperty::Transform, AnimatableValue::Transform(t)) => {
                self.transform = t.sanitize()
            }
            (AnimatableProperty::CornerRadius, AnimatableValue::Float(v)) => {
                self.corner_radius = v.max(0.0)
            }
            (AnimatableProperty::BackgroundColor, AnimatableValue::Color(c)) => {
                self.background_color = *c
            }
            (AnimatableProperty::BorderColor, AnimatableValue::Color(c)) => {
                self.border_color = *c
            }
            (AnimatableProperty::BorderWidth, AnimatableValue::Float(v)) => {
                self.border_width = v.max(0.0)
            }
            (AnimatableProperty::ShadowOpacity, AnimatableValue::Float(v)) => {
                self.shadow_opacity = v.clamp(0.0, 1.0)
            }
            (AnimatableProperty::ZPosition, AnimatableValue::Float(v)) => {
                self.z_position = if v.is_finite() { *v } else { 0.0 }
            }
            _ => {} // type mismatch: silently ignored, property stays unchanged
        }
    }
}
```

### 2.2 Layer Tree

```rust
// supervisor/src/animation/layer_tree.rs

use std::collections::HashMap;
use crate::animation::mod::{Layer, LayerId, PresentationLayer};

pub struct LayerTree {
    /// Model layers. Source of truth for app-declared state.
    pub model: HashMap<LayerId, Layer>,
    /// Presentation layers. Updated every vsync.
    pub presentation: HashMap<LayerId, PresentationLayer>,
    /// Root layer IDs per app PID (each app has one implicit root).
    pub roots: HashMap<u32, LayerId>,
    /// Next available layer ID (monotonically increasing, per-tree).
    next_id: LayerId,
    /// Flat sorted list cache: (z_effective, LayerId), rebuilt when dirty.
    z_order_cache: Vec<(f32, LayerId)>,
    z_order_dirty: bool,
}

impl LayerTree {
    pub fn new() -> Self {
        LayerTree {
            model: HashMap::new(),
            presentation: HashMap::new(),
            roots: HashMap::new(),
            next_id: 1,
            z_order_cache: Vec::new(),
            z_order_dirty: true,
        }
    }

    pub fn create_layer(&mut self, pid: u32) -> LayerId {
        let id = self.next_id;
        self.next_id += 1;
        let layer = Layer::new(id);
        self.presentation.insert(id, PresentationLayer::from_model(&layer));
        self.model.insert(id, layer);
        self.z_order_dirty = true;
        id
    }

    pub fn drop_layer(&mut self, id: LayerId) {
        if let Some(layer) = self.model.remove(&id) {
            // Reparent children to the dropped layer's parent (orphan prevention)
            let orphan_parent = layer.parent;
            for child_id in &layer.children {
                if let Some(child) = self.model.get_mut(child_id) {
                    child.parent = orphan_parent;
                }
                if let Some(parent_id) = orphan_parent {
                    if let Some(parent) = self.model.get_mut(&parent_id) {
                        parent.children.push(*child_id);
                    }
                }
            }
            // Remove from parent's children list
            if let Some(parent_id) = layer.parent {
                if let Some(parent) = self.model.get_mut(&parent_id) {
                    parent.children.retain(|c| *c != id);
                }
            }
        }
        self.presentation.remove(&id);
        self.z_order_dirty = true;
    }

    /// Add child to parent in model tree.
    pub fn add_child(&mut self, parent: LayerId, child: LayerId) {
        if let Some(child_layer) = self.model.get_mut(&child) {
            child_layer.parent = Some(parent);
        }
        if let Some(parent_layer) = self.model.get_mut(&parent) {
            if !parent_layer.children.contains(&child) {
                parent_layer.children.push(child);
            }
        }
        self.z_order_dirty = true;
    }

    /// Flatten the entire tree into a depth-first, z-sorted list of
    /// (accumulated_z, LayerId) for compositor use. Result is ascending z order
    /// (paint back-to-front). Rebuilds from scratch only when z_order_dirty.
    pub fn flatten(&mut self) -> &[(f32, LayerId)] {
        if !self.z_order_dirty {
            return &self.z_order_cache;
        }
        self.z_order_cache.clear();
        // Collect all root IDs
        let roots: Vec<LayerId> = self.roots.values().copied().collect();
        for root_id in roots {
            self.flatten_subtree(root_id, 0.0);
        }
        // Stable sort by accumulated z. std sort is O(n log n), ~stable across frames.
        self.z_order_cache.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        self.z_order_dirty = false;
        &self.z_order_cache
    }

    fn flatten_subtree(&mut self, id: LayerId, parent_z: f32) {
        let layer_z = self.model.get(&id).map(|l| l.z_position).unwrap_or(0.0);
        let effective_z = parent_z + layer_z;
        self.z_order_cache.push((effective_z, id));
        let children: Vec<LayerId> = self.model.get(&id)
            .map(|l| l.children.clone())
            .unwrap_or_default();
        for child_id in children {
            self.flatten_subtree(child_id, effective_z);
        }
    }

    pub fn mark_z_dirty(&mut self) {
        self.z_order_dirty = true;
    }
}
```

### 2.3 Running Animations

```rust
// supervisor/src/animation/runner.rs

use crate::animation::mod::{
    KeyframeAnimation, AnimationTimingFunction, AnimatableValue, FillMode, LayerId,
};
use crate::animation::timing::evaluate_timing;
use crate::animation::interpolate::interpolate_value;

pub struct RunningAnimation {
    pub layer_id: LayerId,
    pub key: String,
    pub spec: KeyframeAnimation,
    /// Elapsed time in ms since the animation's effective begin.
    pub elapsed_ms: f64,
    pub finished: bool,
}

impl RunningAnimation {
    /// Advance animation by `delta_ms`. Returns the current interpolated value,
    /// or None if the animation has ended and should be removed.
    pub fn advance(&mut self, delta_ms: f64) -> Option<AnimatableValue> {
        if self.finished {
            return None;
        }

        self.elapsed_ms += delta_ms;
        let duration = self.spec.duration_ms as f64;

        if duration == 0.0 {
            // Zero-duration: snap immediately to final keyframe value.
            self.finished = true;
            return self.spec.keyframes.last().map(|k| k.value.clone());
        }

        // Handle repeat: compute cycle position.
        let cycle_elapsed = self.elapsed_ms / duration;
        let total_repeat = if self.spec.repeat_count.is_infinite() {
            f64::INFINITY
        } else {
            self.spec.repeat_count as f64
        };

        if cycle_elapsed >= total_repeat && !total_repeat.is_infinite() {
            self.finished = true;
            // FillMode::Forwards or Both: return final value, don't snap.
            return match self.spec.fill_mode {
                FillMode::Forwards | FillMode::Both => {
                    self.spec.keyframes.last().map(|k| k.value.clone())
                }
                FillMode::Removed => None,
                FillMode::Backwards => None,
            };
        }

        // Within a cycle, compute normalized t in [0, 1].
        let cycle_frac = cycle_elapsed.fract();
        let t_raw = if self.spec.autoreverse {
            // Ping-pong: even cycles forward, odd cycles backward.
            let cycle_index = cycle_elapsed.floor() as u64;
            if cycle_index % 2 == 0 { cycle_frac } else { 1.0 - cycle_frac }
        } else {
            cycle_frac
        };

        // Apply timing function to get eased t.
        let t_eased = evaluate_timing(&self.spec.timing, t_raw as f32) as f64;

        // Interpolate between keyframes.
        Some(interpolate_keyframes(&self.spec, t_eased as f32))
    }
}

fn interpolate_keyframes(spec: &KeyframeAnimation, t: f32) -> AnimatableValue {
    let kf = &spec.keyframes;
    if kf.is_empty() {
        // Should never happen; caller validates on add-animation.
        return AnimatableValue::Float(0.0);
    }
    if kf.len() == 1 {
        return kf[0].value.clone();
    }

    // Find the surrounding keyframe pair.
    let mut lo = 0;
    for (i, k) in kf.iter().enumerate() {
        if k.time <= t { lo = i; }
    }
    let hi = (lo + 1).min(kf.len() - 1);

    if lo == hi {
        return kf[lo].value.clone();
    }

    let t_lo = kf[lo].time;
    let t_hi = kf[hi].time;
    let segment_t = if (t_hi - t_lo).abs() < 1e-6 {
        0.0
    } else {
        (t - t_lo) / (t_hi - t_lo)
    };

    // Per-keyframe timing override.
    let timing = kf[lo].timing.as_ref().unwrap_or(&spec.timing);
    let segment_t_eased = evaluate_timing(timing, segment_t);

    interpolate_value(&kf[lo].value, &kf[hi].value, segment_t_eased)
}
```

### 2.4 Timing Functions

```rust
// supervisor/src/animation/timing.rs

use crate::animation::mod::AnimationTimingFunction;

/// Evaluate a timing function at normalized input t in [0.0, 1.0].
/// Returns eased t in [0.0, 1.0] (may exceed range for spring).
pub fn evaluate_timing(f: &AnimationTimingFunction, t: f32) -> f32 {
    match f {
        AnimationTimingFunction::Linear => t,
        AnimationTimingFunction::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        AnimationTimingFunction::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        AnimationTimingFunction::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
        AnimationTimingFunction::CubicBezier { cx1, cy1, cx2, cy2 } => {
            cubic_bezier(*cx1, *cy1, *cx2, *cy2, t)
        }
        AnimationTimingFunction::Spring { mass, stiffness, damping, initial_velocity } => {
            evaluate_spring(*mass, *stiffness, *damping, *initial_velocity, t)
        }
    }
}

/// Cubic Bezier evaluation via Newton's method (same as WebKit/CA implementation).
/// Control points: P0=(0,0), P1=(cx1,cy1), P2=(cx2,cy2), P3=(1,1).
fn cubic_bezier(cx1: f32, cy1: f32, cx2: f32, cy2: f32, t: f32) -> f32 {
    // Solve for x-parameter then evaluate y.
    let ax = 1.0 - 3.0*cx2 + 3.0*cx1;
    let bx = 3.0*cx2 - 6.0*cx1;
    let cx = 3.0*cx1;
    let ay = 1.0 - 3.0*cy2 + 3.0*cy1;
    let by = 3.0*cy2 - 6.0*cy1;
    let cy = 3.0*cy1;

    // Binary search for u such that bezier_x(u) = t.
    let mut u = t;
    for _ in 0..8 {
        let x = ((ax*u + bx)*u + cx)*u;
        let dx = (3.0*ax*u + 2.0*bx)*u + cx;
        if dx.abs() < 1e-6 { break; }
        u -= (x - t) / dx;
        u = u.clamp(0.0, 1.0);
    }
    ((ay*u + by)*u + cy)*u
}

/// Spring physics evaluator. The timing function is evaluated against
/// normalized animation progress t ∈ [0,1] mapped to time_s in [0, duration_hint_s].
///
/// We use the general damped harmonic oscillator solution:
///
/// Let ω₀ = sqrt(stiffness/mass), ζ = damping / (2 * sqrt(stiffness * mass))
///
/// Three regimes:
///   ζ < 1 (underdamped): oscillates, x(t) = e^(-ζω₀t)[A·cos(ωd·t) + B·sin(ωd·t)] + 1
///   ζ = 1 (critical):    x(t) = 1 - (1 + ω₀t)·e^(-ω₀t)   (no overshoot)
///   ζ > 1 (overdamped):  x(t) = 1 - (A·e^(r₁t) + B·e^(r₂t))
///
/// duration_hint_s: total animation duration in seconds (to map t → real time).
pub fn evaluate_spring(
    mass: f32, stiffness: f32, damping: f32,
    initial_velocity: f32, t: f32
) -> f32 {
    // Clamp to prevent numerical explosion from malformed params.
    let mass = mass.clamp(0.01, 100.0);
    let stiffness = stiffness.clamp(0.1, 10000.0);
    let damping = damping.clamp(0.0, 1000.0);

    let omega0 = (stiffness / mass).sqrt();
    let critical_damping = 2.0 * (stiffness * mass).sqrt();
    let zeta = damping / critical_damping;

    // Map t ∈ [0,1] to time. Use a fixed 1-second window; the animation
    // runner maps the spring's natural settling time to the declared duration_ms.
    // We evaluate at t_s = t * settle_time where settle_time is computed from
    // the spring's natural frequency.
    let settle_time = settle_time_estimate(omega0, zeta);
    let t_s = (t * settle_time).max(0.0);

    let x = if (zeta - 1.0).abs() < 0.001 {
        // Critical damping.
        1.0 - (1.0 + omega0 * t_s) * (-omega0 * t_s).exp()
            + initial_velocity * t_s * (-omega0 * t_s).exp()
    } else if zeta < 1.0 {
        // Underdamped.
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let env = (-(zeta * omega0 * t_s)).exp();
        let a = (initial_velocity + zeta * omega0) / omega_d;
        1.0 - env * (t_s * omega_d).cos() - env * a * (t_s * omega_d).sin()
    } else {
        // Overdamped.
        let q = (zeta * zeta - 1.0).sqrt();
        let r1 = omega0 * (-zeta + q);
        let r2 = omega0 * (-zeta - q);
        let a = (initial_velocity - r2) / (r1 - r2);
        let b = 1.0 - a;
        1.0 - a * (r1 * t_s).exp() - b * (r2 * t_s).exp()
    };

    x.clamp(-2.0, 2.0) // allow mild overshoot, prevent explosion
}

fn settle_time_estimate(omega0: f32, zeta: f32) -> f32 {
    // Time to settle within 0.1% of target.
    if zeta >= 1.0 {
        10.0 / omega0
    } else {
        // -ln(0.001) / (zeta * omega0)
        6.908 / (zeta * omega0).max(0.01)
    }
}
```

### 2.5 Interpolation

```rust
// supervisor/src/animation/interpolate.rs

use crate::animation::mod::{AnimatableValue, Color, Transform3D};

/// Linear interpolation between two AnimatableValues.
/// All color interpolation happens in linear light (R15).
pub fn interpolate_value(from: &AnimatableValue, to: &AnimatableValue, t: f32) -> AnimatableValue {
    match (from, to) {
        (AnimatableValue::Float(a), AnimatableValue::Float(b)) => {
            AnimatableValue::Float(lerp_f32(*a, *b, t))
        }
        (AnimatableValue::Point(a), AnimatableValue::Point(b)) => {
            AnimatableValue::Point([lerp_f32(a[0], b[0], t), lerp_f32(a[1], b[1], t)])
        }
        (AnimatableValue::Rect(a), AnimatableValue::Rect(b)) => {
            AnimatableValue::Rect([
                lerp_f32(a[0], b[0], t),
                lerp_f32(a[1], b[1], t),
                lerp_f32(a[2], b[2], t),
                lerp_f32(a[3], b[3], t),
            ])
        }
        (AnimatableValue::Color(a), AnimatableValue::Color(b)) => {
            // Linear-light interpolation (no gamma correction needed; R15 stores linear).
            AnimatableValue::Color(Color {
                r: lerp_f32(a.r, b.r, t),
                g: lerp_f32(a.g, b.g, t),
                b: lerp_f32(a.b, b.b, t),
                a: lerp_f32(a.a, b.a, t),
            })
        }
        (AnimatableValue::Transform(a), AnimatableValue::Transform(b)) => {
            // Decompose, slerp rotation, lerp translation/scale, recompose.
            AnimatableValue::Transform(lerp_transform(a, b, t))
        }
        _ => {
            // Type mismatch: discrete jump at t >= 0.5.
            if t >= 0.5 { to.clone() } else { from.clone() }
        }
    }
}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Decompose-interpolate-recompose for Transform3D.
/// For pure 2D transforms (translation + rotation + scale), extract components,
/// slerp the rotation angle, lerp translation and scale.
fn lerp_transform(a: &Transform3D, b: &Transform3D, t: f32) -> Transform3D {
    // Extract 2D components from column-major 4x4.
    let (ax, ay, as_, ar) = decompose_2d(a);
    let (bx, by, bs_, br) = decompose_2d(b);

    // Slerp rotation (shortest arc).
    let ar_norm = normalize_angle(ar);
    let br_norm = normalize_angle(br);
    let mut da = br_norm - ar_norm;
    while da > std::f32::consts::PI { da -= 2.0 * std::f32::consts::PI; }
    while da < -std::f32::consts::PI { da += 2.0 * std::f32::consts::PI; }
    let r = ar_norm + da * t;

    let tx = lerp_f32(ax, bx, t);
    let ty = lerp_f32(ay, by, t);
    let scale = lerp_f32(as_, bs_, t);

    recompose_2d(tx, ty, scale, r)
}

fn decompose_2d(m: &Transform3D) -> (f32, f32, f32, f32) {
    let tx = m.m[3][0];
    let ty = m.m[3][1];
    let scale = (m.m[0][0] * m.m[0][0] + m.m[0][1] * m.m[0][1]).sqrt();
    let rotation = m.m[0][1].atan2(m.m[0][0]);
    (tx, ty, scale, rotation)
}

fn recompose_2d(tx: f32, ty: f32, scale: f32, rotation: f32) -> Transform3D {
    let (s, c) = rotation.sin_cos();
    Transform3D {
        m: [
            [c * scale, s * scale, 0.0, 0.0],
            [-s * scale, c * scale, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [tx, ty, 0.0, 1.0],
        ],
    }
}

fn normalize_angle(a: f32) -> f32 {
    let pi2 = 2.0 * std::f32::consts::PI;
    ((a % pi2) + pi2) % pi2
}
```

### 2.6 Transaction Queue

```rust
// supervisor/src/animation/transaction.rs

use std::collections::{HashMap, VecDeque};
use crate::animation::mod::{AnimationTransaction, TransactionHandle, LayerChange,
                              AnimationTimingFunction};

/// Per-app pending (not yet committed) transactions.
pub struct OpenTransaction {
    pub handle: TransactionHandle,
    pub pid: u32,
    pub duration_ms: u32,
    pub timing: AnimationTimingFunction,
    pub changes: Vec<LayerChange>,
    pub completion_token: Option<u32>,
}

/// Committed transactions waiting to be processed by the animation runner.
pub struct TransactionQueue {
    /// One open (uncommitted) transaction per app at a time.
    open: HashMap<u32, OpenTransaction>,
    /// FIFO of committed transactions ready for the animation engine.
    /// Bounded to prevent unbounded memory growth from malicious/buggy apps.
    committed: VecDeque<AnimationTransaction>,
    /// Per-app committed count cap.
    per_app_pending: HashMap<u32, u32>,
    next_handle: TransactionHandle,
}

impl TransactionQueue {
    const MAX_COMMITTED_TOTAL: usize = 512;
    const MAX_PER_APP_PENDING: u32 = 16;

    pub fn new() -> Self {
        TransactionQueue {
            open: HashMap::new(),
            committed: VecDeque::with_capacity(64),
            per_app_pending: HashMap::new(),
            next_handle: 1,
        }
    }

    pub fn begin(&mut self, pid: u32, duration_ms: u32,
                 timing: AnimationTimingFunction) -> TransactionHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.open.insert(pid, OpenTransaction {
            handle, pid, duration_ms, timing,
            changes: Vec::new(),
            completion_token: None,
        });
        handle
    }

    pub fn push_change(&mut self, pid: u32, change: LayerChange) -> Result<(), &'static str> {
        let tx = self.open.get_mut(&pid).ok_or("no open transaction")?;
        // Per-transaction change cap to prevent huge allocations.
        if tx.changes.len() >= 4096 {
            return Err("transaction change limit exceeded");
        }
        tx.changes.push(change);
        Ok(())
    }

    pub fn set_completion_token(&mut self, pid: u32, token: u32) -> Result<(), &'static str> {
        let tx = self.open.get_mut(&pid).ok_or("no open transaction")?;
        tx.completion_token = Some(token);
        Ok(())
    }

    pub fn commit(&mut self, pid: u32, now_ms: u64) -> Result<TransactionHandle, &'static str> {
        let tx = self.open.remove(&pid).ok_or("no open transaction")?;
        let handle = tx.handle;

        // Rate-limit per app.
        let pending = self.per_app_pending.entry(pid).or_insert(0);
        if *pending >= Self::MAX_PER_APP_PENDING {
            return Err("per-app transaction queue full");
        }
        // Global cap.
        if self.committed.len() >= Self::MAX_COMMITTED_TOTAL {
            return Err("global transaction queue full");
        }

        *pending += 1;
        self.committed.push_back(AnimationTransaction {
            handle,
            pid,
            duration_ms: tx.duration_ms,
            timing: tx.timing,
            changes: tx.changes,
            completion_token: tx.completion_token,
            committed_at_ms: now_ms,
        });
        Ok(handle)
    }

    pub fn cancel(&mut self, pid: u32) {
        self.open.remove(&pid);
    }

    /// Drain all committed transactions for processing by the animation engine.
    pub fn drain_committed(&mut self) -> impl Iterator<Item = AnimationTransaction> + '_ {
        // Return and clear per-app counts.
        let items: Vec<_> = self.committed.drain(..).collect();
        self.per_app_pending.clear();
        items.into_iter()
    }
}
```

---

## 3. WIT Interface

```wit
// supervisor/src/animation/vyoma_animation.wit

package vyoma:animation@1.0.0;

/// Layer management and animation for WASM apps.
interface layers {

    // ── Layer lifecycle ─────────────────────────────────────────────────

    /// Allocate a new layer. Returns an opaque layer handle.
    /// Fails (returns 0) if the app's max_layers quota is exceeded.
    create-layer: func() -> u32;

    /// Release a layer. Children are re-parented to the dropped layer's parent.
    drop-layer: func(layer: u32);

    /// Set parent-child relationship. Child is appended to parent's children.
    add-sublayer: func(parent: u32, child: u32) -> result<_, string>;

    /// Remove child from parent.
    remove-sublayer: func(parent: u32, child: u32) -> result<_, string>;

    // ── Transactions ────────────────────────────────────────────────────

    /// Begin a new animation transaction.
    ///
    /// duration-ms: animation duration. 0 = immediate (no interpolation).
    /// timing: timing function ID (0=linear, 1=ease-in, 2=ease-out,
    ///         3=ease-in-out, 4=spring-default, 5=custom-bezier)
    ///
    /// Returns transaction handle. Only one open transaction per app at a time.
    begin-transaction: func(duration-ms: u32, timing: u8) -> result<u32, string>;

    /// Register a spring timing function on the currently open transaction.
    /// mass/stiffness/damping/initial-velocity are all f32.
    set-transaction-spring: func(tx: u32, mass: f32, stiffness: f32,
                                 damping: f32, initial-velocity: f32) -> result<_, string>;

    /// Associate a completion token. Supervisor writes
    /// VYOMA_ANIM_COMPLETE:<token> to app stdin when the animation ends.
    set-completion-token: func(tx: u32, token: u32) -> result<_, string>;

    /// Commit the open transaction to the animation engine.
    commit-transaction: func(tx: u32) -> result<_, string>;

    /// Discard the open transaction without applying any changes.
    cancel-transaction: func(tx: u32);

    // ── Property setters ────────────────────────────────────────────────
    // All setters enqueue a LayerChange into the open transaction (if any),
    // or apply immediately as an implicit animation if none is open.

    set-position: func(layer: u32, x: f32, y: f32);
    set-bounds: func(layer: u32, x: f32, y: f32, w: f32, h: f32);
    set-opacity: func(layer: u32, opacity: f32);

    /// Column-major 4x4 matrix. Must be exactly 16 floats; error if not.
    set-transform: func(layer: u32, m: list<f32>) -> result<_, string>;

    set-corner-radius: func(layer: u32, r: f32);
    set-background-color: func(layer: u32, r: f32, g: f32, b: f32, a: f32);
    set-border-color: func(layer: u32, r: f32, g: f32, b: f32, a: f32);
    set-border-width: func(layer: u32, w: f32);
    set-shadow: func(layer: u32, r: f32, g: f32, b: f32, a: f32,
                     opacity: f32, radius: f32, dx: f32, dy: f32);
    set-hidden: func(layer: u32, hidden: bool);
    set-z-position: func(layer: u32, z: f32);
    set-anchor-point: func(layer: u32, x: f32, y: f32);

    /// Attach a Surface (R11) as layer content.
    set-contents-surface: func(layer: u32, surface: u32);

    /// Attach a decoded Image (R14) as layer content.
    set-contents-image: func(layer: u32, image: u32);

    /// Attach a GPU texture (R12 wgpu handle) as layer content.
    set-contents-gpu-texture: func(layer: u32, texture: u32);

    // ── Explicit keyframe animations ────────────────────────────────────

    /// Add a named keyframe animation to a layer.
    /// anim-data is a msgpack-encoded AnimSpec (see spec §3.1).
    add-animation: func(layer: u32, key: string,
                        anim-data: list<u8>) -> result<_, string>;

    /// Remove a named animation by key. Layer presentation snaps to model value.
    remove-animation: func(layer: u32, key: string);

    /// Remove all animations from a layer.
    remove-all-animations: func(layer: u32);

    // ── Presentation layer queries ───────────────────────────────────────

    /// Query the current interpolated position of a layer.
    presentation-position: func(layer: u32) -> tuple<f32, f32>;

    /// Query the current interpolated opacity.
    presentation-opacity: func(layer: u32) -> f32;

    /// Query the current interpolated bounds.
    presentation-bounds: func(layer: u32) -> tuple<f32, f32, f32, f32>;

    /// Query the current transform as 16 column-major floats.
    presentation-transform: func(layer: u32) -> list<f32>;

    // ── Implicit animation control ───────────────────────────────────────

    /// Disable implicit animations for the calling app's entire layer tree.
    set-implicit-animations-enabled: func(enabled: bool);
}

/// AnimSpec msgpack schema (§3.1):
/// {
///   "property": u8,         // AnimatableProperty ordinal
///   "keyframes": [
///     { "time": f32, "value": <typed>, "timing": u8? }
///   ],
///   "duration_ms": u32,
///   "repeat_count": f32,
///   "autoreverse": bool,
///   "fill_mode": u8,
///   "timing": u8,
///   "begin_time_ms": i64
/// }

world vyoma-animation-app {
    import layers;
}
```

---

## 4. VYOMA_ANIM Stdout Protocol (v2)

The stdout protocol provides a zero-dependency path for WASM apps that cannot or do not use WIT hostcalls. The supervisor's display parser thread reads lines from each app's stdout pipe and routes `VYOMA_ANIM:` lines to the animation subsystem via an internal channel.

```
# Open a transaction. duration_ms and timing_id are mandatory.
VYOMA_ANIM:begin:<duration_ms>,<timing_id>

# For spring timing (timing_id=4), optionally override spring params:
VYOMA_ANIM:spring:<mass>,<stiffness>,<damping>,<initial_velocity>

# Property change setters (enqueue into open transaction):
VYOMA_ANIM:set_pos:<layer_id>,<x>,<y>
VYOMA_ANIM:set_bounds:<layer_id>,<x>,<y>,<w>,<h>
VYOMA_ANIM:set_opacity:<layer_id>,<opacity>
VYOMA_ANIM:set_transform:<layer_id>,<m00>,<m10>,...,<m33>   (16 floats, column-major)
VYOMA_ANIM:set_corner_radius:<layer_id>,<r>
VYOMA_ANIM:set_color:<layer_id>,<r>,<g>,<b>,<a>
VYOMA_ANIM:set_border_color:<layer_id>,<r>,<g>,<b>,<a>
VYOMA_ANIM:set_border_width:<layer_id>,<w>
VYOMA_ANIM:set_hidden:<layer_id>,<0|1>
VYOMA_ANIM:set_z:<layer_id>,<z>

# Layer management:
VYOMA_ANIM:create_layer                                      → supervisor replies VYOMA_ANIM_LAYER:<id>
VYOMA_ANIM:drop_layer:<layer_id>
VYOMA_ANIM:add_child:<parent_id>,<child_id>

# Commit opens transaction to animation engine:
VYOMA_ANIM:commit[:<completion_token>]

# Cancel discards open transaction:
VYOMA_ANIM:cancel

# Explicit keyframe animation (single-shot; no open transaction required):
VYOMA_ANIM:add_keyframe:<layer_id>,<key>,<prop_id>,<duration_ms>,<repeat>,<autoreverse>,<timing_id>,<t0>,<v0>[,...,<tN>,<vN>]

VYOMA_ANIM:remove_keyframe:<layer_id>,<key>
VYOMA_ANIM:remove_all_keyframes:<layer_id>
```

**Supervisor responses written to app stdin:**
```
VYOMA_ANIM_LAYER:<id>\n          → response to create_layer
VYOMA_ANIM_COMPLETE:<token>\n    → animation completion callback
VYOMA_ANIM_ERROR:<msg>\n         → parse or quota error
```

**Synchronization model**: The display parser thread reads stdout lines and sends `AnimCommand` messages to the animation engine via a bounded `crossbeam_channel::bounded(256)` channel. The animation engine processes commands in batch at the beginning of each vsync tick, before advancing running animations and compositing. This means the maximum latency between an app writing `VYOMA_ANIM:commit` and the animation starting is one vsync frame (16.6ms at 60Hz). Flood protection: the channel is bounded; if it fills, new commands are dropped and `VYOMA_ANIM_ERROR:rate_limit` is sent to the app.

---

## 5. Manifest Capability

```toml
# apps/my-app/vyoma.toml
[capabilities.animation]
layers = true           # Enable layer tree + transactions (required)
max_layers = 64         # Per-app layer cap. Range: 1..=512. Default: 64.
spring_physics = true   # Allow spring timing function. Default: false.
                        # Spring is CPU-intensive; must be explicitly opt-in.
```

**Platform defaults injected by `AnimationConfig`:**

| Platform | layers | max_layers | spring_physics |
|---|---|---|---|
| mcu-minimal | false | 0 | false |
| iot-edge | false | 8 | false |
| robotics-rt | false | 8 | false |
| mobile | true | 128 | true |
| desktop-full | true | 512 | true |
| server-headless | false | 0 | false |

```rust
// supervisor/src/animation/config.rs

use crate::profile::Platform;

#[derive(Debug, Clone)]
pub struct AnimationConfig {
    /// Whether the animation subsystem is active on this platform.
    pub enabled: bool,
    /// Maximum layers per app (enforced at create_layer time).
    pub max_layers_per_app: u32,
    /// Whether spring timing functions are permitted.
    pub spring_physics_enabled: bool,
    /// Maximum animation duration in milliseconds (DoS prevention).
    pub max_duration_ms: u32,
    /// Maximum keyframes per animation.
    pub max_keyframes: usize,
    /// Whether the GPU compositor path is active.
    pub gpu_compositor: bool,
    /// Maximum total layers across all apps.
    pub global_layer_cap: u32,
}

impl AnimationConfig {
    pub fn for_platform(platform: Platform) -> Self {
        match platform {
            Platform::McuMinimal | Platform::IotEdge | Platform::RoboticsRt
            | Platform::ServerHeadless => AnimationConfig {
                enabled: false,
                max_layers_per_app: 0,
                spring_physics_enabled: false,
                max_duration_ms: 0,
                max_keyframes: 0,
                gpu_compositor: false,
                global_layer_cap: 0,
            },
            Platform::Mobile => AnimationConfig {
                enabled: true,
                max_layers_per_app: 128,
                spring_physics_enabled: true,
                max_duration_ms: 30_000,
                max_keyframes: 64,
                gpu_compositor: true,
                global_layer_cap: 2048,
            },
            Platform::DesktopFull => AnimationConfig {
                enabled: true,
                max_layers_per_app: 512,
                spring_physics_enabled: true,
                max_duration_ms: 30_000,
                max_keyframes: 128,
                gpu_compositor: true,
                global_layer_cap: 8192,
            },
        }
    }
}
```

---

## 6. Integration With Prior Subsystems

### 6.1 R11: VSync Loop + Framebuffer

```rust
// supervisor/src/animation/compositor.rs (fragment)

use crate::display::{Framebuffer, blit_surface};
use crate::animation::layer_tree::LayerTree;
use crate::animation::presentation::PresentationLayer;
use crate::images::ImageHandle;

pub struct AnimationCompositor {
    pub layer_tree: LayerTree,
}

impl AnimationCompositor {
    /// Called once per vsync tick BEFORE the framebuffer is sent to the display.
    /// Walks the z-sorted layer list and blits each visible layer onto `fb`.
    pub fn composite_frame(&mut self, fb: &mut Framebuffer) {
        let flat = self.layer_tree.flatten().to_vec();
        for (_z, layer_id) in &flat {
            let Some(layer) = self.layer_tree.presentation.get(layer_id) else { continue };
            if layer.hidden || layer.opacity <= 0.0 {
                continue;
            }
            self.blit_layer(fb, layer);
        }
    }

    fn blit_layer(&self, fb: &mut Framebuffer, layer: &PresentationLayer) {
        // Apply the layer's computed transform to get a screen-space quad.
        let screen_rect = self.compute_screen_rect(layer);

        // Fill background color.
        if layer.background_color.a > 0.0 {
            fb.fill_rect_rgba(screen_rect, layer.background_color, layer.opacity);
        }

        // Blit content.
        match &layer.contents {
            Some(LayerContents::Surface(sid)) => {
                blit_surface(fb, *sid, screen_rect, layer.opacity,
                             layer.corner_radius, layer.mask_to_bounds);
            }
            Some(LayerContents::Image(img)) => {
                blit_image(fb, img, screen_rect, layer.opacity,
                           layer.corner_radius);
            }
            Some(LayerContents::GpuTexture(_tex)) => {
                // GPU path: handed off to R12 compositor. Software compositor skips.
            }
            None => {}
        }

        // Border overlay.
        if layer.border_width > 0.0 && layer.border_color.a > 0.0 {
            fb.draw_border(screen_rect, layer.border_width,
                           layer.border_color, layer.opacity);
        }
    }

    fn compute_screen_rect(&self, layer: &PresentationLayer) -> [f32; 4] {
        // For the 2D software compositor, apply transform translation only.
        // Full matrix projection is the GPU path's responsibility (R12).
        let tx = layer.transform.m[3][0];
        let ty = layer.transform.m[3][1];
        let px = layer.position[0] + tx;
        let py = layer.position[1] + ty;
        let w = layer.bounds[2];
        let h = layer.bounds[3];
        let ax = layer.anchor_point[0];
        let ay = layer.anchor_point[1];
        [px - ax * w, py - ay * h, w, h]
    }
}
```

### 6.2 R12: GPU Compositor Path

When `AnimationConfig::gpu_compositor` is true and scene complexity is `Heavy` (per R12's `SceneComplexity` enum), the animation compositor delegates rendering to the GPU path:

```rust
// In the vsync loop (supervisor/src/display.rs):
if config.gpu_compositor && scene_complexity == SceneComplexity::Heavy {
    let flat = compositor.layer_tree.flatten().to_vec();
    let layer_uniforms: Vec<LayerUniform> = flat.iter()
        .filter_map(|(_, id)| compositor.layer_tree.presentation.get(id))
        .map(|l| LayerUniform {
            transform: l.transform.m,
            opacity: l.opacity,
            corner_radius: l.corner_radius,
            bounds: l.bounds,
        })
        .collect();
    gpu_compositor.submit_layer_batch(&layer_uniforms, fb_texture);
} else {
    compositor.composite_frame(fb);
}
```

### 6.3 R13: Font Layer Animations

Text animations (slide-in, opacity fade) use layers whose content is a pre-rendered font Surface from R13:

```rust
// Fade-in a text surface over 300ms:
let layer = layer_tree.create_layer(pid);
let model = layer_tree.model.get_mut(&layer).unwrap();
model.contents = Some(LayerContents::Surface(text_surface_id));
model.opacity = 0.0;
model.position = [100.0, 200.0];
model.bounds = [0.0, 0.0, 400.0, 32.0];

// Open a transaction for the fade:
let tx = tx_queue.begin(pid, 300, AnimationTimingFunction::EaseOut);
tx_queue.push_change(pid, LayerChange {
    layer_id: layer,
    property: AnimatableProperty::Opacity,
    new_value: AnimatableValue::Float(1.0),
});
tx_queue.commit(pid, now_ms)?;
```

### 6.4 R14: Image Layer Crossfade

Cross-dissolve between two images:

```rust
// Crossfade from image_a to image_b over 200ms:
// Layer A: fade out
let tx_a = tx_queue.begin(pid, 200, AnimationTimingFunction::Linear);
tx_queue.push_change(pid, LayerChange {
    layer_id: layer_a, property: AnimatableProperty::Opacity,
    new_value: AnimatableValue::Float(0.0),
});
tx_queue.commit(pid, now_ms)?;

// Layer B: already at opacity=0, fade in
let tx_b = tx_queue.begin(pid, 200, AnimationTimingFunction::Linear);
tx_queue.push_change(pid, LayerChange {
    layer_id: layer_b, property: AnimatableProperty::Opacity,
    new_value: AnimatableValue::Float(1.0),
});
tx_queue.commit(pid, now_ms)?;
```

### 6.5 R15: Color Interpolation in Linear Light

All `AnimatableValue::Color` interpolation is performed in the linear-light colorspace that R15 established. When a layer's `background_color` is set from sRGB app input, it is converted to linear-light on ingestion (in `wit_handlers.rs`), stored as `Color { r, g, b, a }` in linear-light, and converted back to sRGB only at blit time in the compositor.

```rust
// supervisor/src/animation/wit_handlers.rs (fragment)
fn set_background_color_handler(
    store: &mut Store, layer: u32, r: f32, g: f32, b: f32, a: f32
) {
    // Convert from sRGB (app-provided) to linear light (R15).
    let linear = Color {
        r: srgb_to_linear(r),
        g: srgb_to_linear(g),
        b: srgb_to_linear(b),
        a, // alpha is always linear
    };
    enqueue_change(store, layer, AnimatableProperty::BackgroundColor,
                   AnimatableValue::Color(linear));
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 }
    else { ((c + 0.055) / 1.055).powf(2.4) }
}
```

---

## 7. Platform Matrix

| Feature | mcu-minimal | iot-edge | robotics-rt | mobile | desktop-full | server-headless |
|---|---|---|---|---|---|---|
| Layer tree | no | basic | basic | yes | yes | no |
| Max layers/app | 0 | 8 | 8 | 128 | 512 | 0 |
| Implicit animations | no | no | no | yes | yes | no |
| Spring physics | no | no | no | yes | yes | no |
| GPU compositor path | no | no | no | yes | yes | no |
| Keyframe animations | no | no | no | yes | yes | no |
| Stdout protocol | no | no | no | yes | yes | no |
| WIT hostcalls | no | no | no | yes | yes | no |
| Completion callbacks | no | no | no | yes | yes | no |
| Color interpolation | n/a | n/a | n/a | linear | linear | n/a |
| Max animation duration | n/a | n/a | n/a | 30s | 30s | n/a |

**iot-edge / robotics-rt "basic" layer support**: No animation, no transaction — the LayerTree exists as a static compositing structure only. Apps on these platforms use static layer positions set once at startup. No `VYOMA_ANIM:` protocol is parsed; the subsystem is compiled with `#[cfg(feature = "animation")]` guard.

---

## 8. Security Model

### 8.1 Layer Handle Isolation

Each WASM app receives opaque `LayerId` values that are local to a per-app `LayerHandleTable`. The supervisor maintains:

```rust
// supervisor/src/animation/wit_handlers.rs

pub struct LayerHandleTable {
    pid: u32,
    /// Map from app-visible handle → global LayerId in the LayerTree.
    handles: HashMap<u32, LayerId>,
    /// Reverse map for cleanup on app exit.
    reverse: HashMap<LayerId, u32>,
    /// Count of currently allocated layers.
    count: u32,
    /// Limit from AnimationConfig.
    max: u32,
}

impl LayerHandleTable {
    pub fn resolve(&self, app_handle: u32) -> Result<LayerId, &'static str> {
        self.handles.get(&app_handle).copied().ok_or("invalid layer handle")
    }

    pub fn allocate(&mut self, global_id: LayerId) -> Result<u32, &'static str> {
        if self.count >= self.max {
            return Err("layer quota exceeded");
        }
        let handle = global_id; // opaque but predictable; fine since isolated
        self.handles.insert(handle, global_id);
        self.reverse.insert(global_id, handle);
        self.count += 1;
        Ok(handle)
    }

    pub fn release(&mut self, app_handle: u32) -> Option<LayerId> {
        let global_id = self.handles.remove(&app_handle)?;
        self.reverse.remove(&global_id);
        self.count -= 1;
        Some(global_id)
    }
}
```

App A cannot construct a valid handle for App B's layers because handle-to-LayerId resolution goes through App A's own `LayerHandleTable`. Cross-app layer access requires explicit supervisor mediation (not implemented in v1).

### 8.2 Animation Duration Cap

```rust
fn validate_animation_spec(spec: &KeyframeAnimation, config: &AnimationConfig)
    -> Result<(), &'static str>
{
    if spec.duration_ms > config.max_duration_ms {
        return Err("animation duration exceeds platform maximum");
    }
    if spec.keyframes.len() > config.max_keyframes {
        return Err("too many keyframes");
    }
    if spec.keyframes.len() < 2 {
        return Err("animation requires at least 2 keyframes");
    }
    if spec.keyframes.first().map(|k| k.time).unwrap_or(1.0) != 0.0 {
        return Err("first keyframe time must be 0.0");
    }
    if spec.keyframes.last().map(|k| k.time).unwrap_or(0.0) != 1.0 {
        return Err("last keyframe time must be 1.0");
    }
    Ok(())
}
```

Infinite-loop animations (`repeat_count = f32::INFINITY`) are permitted but the CPU cost per tick is constant (one interpolation step), so they do not cause unbounded computation growth.

### 8.3 Transform Sanitization

All `set-transform` calls sanitize before storage:
```rust
let transform = Transform3D { m: ... }.sanitize(); // clamps NaN/Inf to identity elements
```

The compositor's `compute_screen_rect` uses only translation components from the transform matrix; the full 4x4 matrix projection (which could generate near-zero w, causing division by zero) is handled exclusively by the GPU path, which clips all geometry against the viewport before rasterization.

### 8.4 Completion Callback Backpressure

`VYOMA_ANIM_COMPLETE:<token>` is written to the app's stdin pipe. If the stdin pipe is full (app has not drained its input), the supervisor uses a non-blocking write:

```rust
use std::io::ErrorKind;

fn send_completion(app_stdin: &mut PipeWriter, token: u32) {
    let msg = format!("VYOMA_ANIM_COMPLETE:{}\n", token);
    match app_stdin.write_nonblocking(msg.as_bytes()) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            // Stdin pipe full. Log and drop. App is responsible for draining.
            log::warn!("anim_complete drop: pid stdin full, token={}", token);
        }
        Err(e) => {
            log::error!("anim_complete write error: {}", e);
        }
    }
}
```

### 8.5 Layer Churn Rate Limiting

Rapid create/drop cycling is throttled:
```rust
const MAX_CREATES_PER_SECOND: u32 = 60;
// Per-app create counter reset every second in the vsync tick housekeeping.
```

### 8.6 Spring Physics Opt-In

Spring physics is a compile-time feature gate (`#[cfg(feature = "spring_physics")]`) AND a runtime manifest check. Even if an app declares `spring_physics = true` but the platform's `AnimationConfig::spring_physics_enabled = false`, the supervisor substitutes `EaseInOut` silently.

---

## 9. AnimationEngine — Top-Level Orchestrator

```rust
// supervisor/src/animation/runner.rs (AnimationEngine portion)

pub struct AnimationEngine {
    pub config: AnimationConfig,
    pub layer_tree: LayerTree,
    pub tx_queue: TransactionQueue,
    /// All currently-running animations.
    running: Vec<RunningAnimation>,
    /// Implicit animation default duration.
    implicit_duration_ms: u32,
    /// Per-app handle tables.
    handle_tables: HashMap<u32, LayerHandleTable>,
}

impl AnimationEngine {
    pub const IMPLICIT_DURATION_MS: u32 = 250;

    /// Called once per vsync tick. delta_ms is elapsed since last tick.
    pub fn tick(&mut self, delta_ms: f64, now_ms: u64) {
        // 1. Drain and apply committed transactions.
        let txns: Vec<_> = self.tx_queue.drain_committed().collect();
        for txn in txns {
            self.apply_transaction(txn, now_ms);
        }

        // 2. Advance all running animations.
        let mut finished_tokens: Vec<(u32, u32)> = Vec::new(); // (pid, token)
        let mut to_remove: Vec<usize> = Vec::new();

        for (i, anim) in self.running.iter_mut().enumerate() {
            if let Some(value) = anim.advance(delta_ms) {
                if let Some(layer) = self.layer_tree.presentation.get_mut(&anim.layer_id) {
                    layer.apply(anim.spec.property, &value);
                }
            } else {
                to_remove.push(i);
            }
        }

        // 3. Collect completion tokens and remove finished animations.
        for i in to_remove.iter().rev() {
            let anim = self.running.remove(*i);
            if let Some((pid, token)) = anim.completion_token {
                finished_tokens.push((pid, token));
            }
        }

        // 4. Send completion callbacks.
        for (pid, token) in finished_tokens {
            self.send_completion_callback(pid, token);
        }

        // 5. Sync model-to-presentation for non-animated layers.
        self.sync_static_layers();
    }

    fn apply_transaction(&mut self, txn: AnimationTransaction, now_ms: u64) {
        for change in &txn.changes {
            // Update the model layer unconditionally.
            if let Some(model) = self.layer_tree.model.get_mut(&change.layer_id) {
                apply_change_to_model(model, change);
            }

            if txn.duration_ms == 0 {
                // Immediate: snap presentation layer too.
                if let Some(pres) = self.layer_tree.presentation.get_mut(&change.layer_id) {
                    pres.apply(change.property, &change.new_value);
                }
            } else {
                // Animate from current presentation value to new model value.
                let from_value = self.current_presentation_value(
                    change.layer_id, change.property
                );
                let anim = RunningAnimation {
                    layer_id: change.layer_id,
                    key: format!("__tx_{}", txn.handle),
                    spec: KeyframeAnimation {
                        property: change.property,
                        keyframes: vec![
                            Keyframe { time: 0.0, value: from_value, timing: None },
                            Keyframe { time: 1.0, value: change.new_value.clone(), timing: None },
                        ],
                        duration_ms: txn.duration_ms,
                        repeat_count: 1.0,
                        autoreverse: false,
                        fill_mode: FillMode::Forwards,
                        timing: txn.timing.clone(),
                        begin_time_ms: 0,
                    },
                    elapsed_ms: 0.0,
                    finished: false,
                    completion_token: txn.completion_token.map(|t| (txn.pid, t)),
                };
                // Remove any existing animation on the same property/layer.
                self.running.retain(|r| {
                    !(r.layer_id == change.layer_id && r.spec.property == change.property)
                });
                self.running.push(anim);
            }
        }
    }

    fn sync_static_layers(&mut self) {
        // For layers with no running animation, keep presentation = model.
        let animated: std::collections::HashSet<LayerId> = self.running.iter()
            .map(|r| r.layer_id).collect();
        for (id, model) in &self.layer_tree.model {
            if !animated.contains(id) {
                if let Some(pres) = self.layer_tree.presentation.get_mut(id) {
                    if pres.dirty { pres.dirty = false; }
                    // Only sync if presentation diverged from model for non-animated props.
                    pres.hidden = model.hidden;
                    pres.z_position = model.z_position;
                    pres.contents = model.contents.clone();
                }
            }
        }
    }
}
```

---

## 10. Implementation File Map

| File | Purpose | ≤500 LOC |
|---|---|---|
| `supervisor/src/animation/mod.rs` | Public types: Layer, Transform3D, Color, AnimationTimingFunction, KeyframeAnimation, AnimationTransaction, LayerChange, AnimatableProperty/Value, FillMode | yes |
| `supervisor/src/animation/config.rs` | AnimationConfig, per-platform defaults | yes |
| `supervisor/src/animation/layer_tree.rs` | LayerTree: parent/child management, z-order cache, flatten() | yes |
| `supervisor/src/animation/presentation.rs` | PresentationLayer, from_model(), apply() | yes |
| `supervisor/src/animation/transaction.rs` | TransactionQueue, OpenTransaction, rate-limiting | yes |
| `supervisor/src/animation/runner.rs` | AnimationEngine, RunningAnimation, tick(), advance() | yes |
| `supervisor/src/animation/timing.rs` | cubic_bezier(), evaluate_spring(), settle_time_estimate() | yes |
| `supervisor/src/animation/interpolate.rs` | interpolate_value(), lerp_transform(), slerp rotation | yes |
| `supervisor/src/animation/compositor.rs` | composite_frame(), blit_layer(), compute_screen_rect() | yes |
| `supervisor/src/animation/wit_handlers.rs` | Wasmtime linker registration for vyoma:animation/layers | yes |
| `supervisor/src/animation/protocol.rs` | VYOMA_ANIM: stdout line parser, AnimCommand enum | yes |

Total: 11 files × ≤500 LOC = ≤5,500 lines. Each file is independently testable.

**Unit test coverage targets** (in `supervisor/tests/animation_*.rs`):
- `timing_spring_critical`: verify `evaluate_spring` with ζ=1 matches analytic formula
- `timing_spring_underdamped`: verify oscillation and eventual convergence
- `interpolate_color_linear`: verify midpoint between two colors in linear light
- `interpolate_transform_rotation`: verify slerp takes shortest arc
- `transaction_rate_limit`: verify `MAX_PER_APP_PENDING` enforced
- `layer_quota`: verify `max_layers_per_app` enforced, correct error returned
- `layer_handle_isolation`: verify App A handle cannot resolve to App B's global LayerId
- `nan_in_transform`: verify sanitize() replaces NaN with identity elements
- `completion_callback_full_pipe`: verify non-blocking write drops gracefully
- `z_order_flatten`: verify depth-first z-sort produces correct paint order
- `implicit_animation_default`: verify 250ms EaseInOut is applied when no tx is open

---

## Summary

This spec defines VyomaOS's Animation Engine as a faithful WASM-first reimplementation of macOS Core Animation semantics:

- **Two-layer model** (Model vs Presentation) with full transaction batching
- **Physically correct spring physics** with all three damping regimes (under/critical/over)
- **Explicit keyframe animations** with per-segment timing overrides
- **Implicit animations** auto-generated for property changes outside transactions
- **Zero-dependency stdout protocol** (`VYOMA_ANIM:`) for apps without WIT
- **WIT hostcall interface** (`vyoma:animation@1.0.0`) for type-safe access
- **Full R11–R15 integration**: VSync loop, GPU compositor, font surfaces, image crossfade, linear-light color interpolation
- **Platform-stratified config**: 4 platforms get no animation (mcu/iot/robotics/server); mobile and desktop-full get full feature set
- **Hardened security**: layer handle isolation, duration cap, NaN sanitization, completion callback backpressure, churn rate limiting
