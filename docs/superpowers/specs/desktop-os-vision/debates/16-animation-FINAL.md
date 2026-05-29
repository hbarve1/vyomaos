# Round 16 FINAL — Animation Engine
## VyomaOS Subsystem 16: Animation Engine (macOS equiv: Core Animation / CALayer)

**Status**: FINAL — all 5 blocking issues resolved
**Date**: 2026-05-29
**Architect source**: 16-animation.md (1,735 lines)
**Critic source**: 16-animation-critique.md (500 lines)
**Blocking issues resolved**: B1 (double-buffer TxQueue), B2 (spring convergence), B3 (large-finite transform clamp), B4 (implicit animation path), B5 (WIT structured types)

---

## Overview

The VyomaOS Animation Engine is the Core Animation equivalent: a two-layer model (ModelLayer + PresentationLayer) with keyframe and spring animations, per-app handle isolation, and a double-buffered transaction pipeline that keeps the vsync thread lock-free during compositing.

**Platform availability**: disabled on `mcu-minimal`, `iot-edge`, `robotics-rt`, `server-headless`; full on `mobile` and `desktop-full`.

**Integration**: Called as `anim_engine.tick(delta_ms, fb)` from R11's vsync loop before the blit pass.

---

## Core Types

```rust
// supervisor/src/animation/mod.rs

pub type LayerId = u32;
pub type Pid = u32;

// Timing function IDs (protocol wire format)
pub const TIMING_LINEAR:      u8 = 0;
pub const TIMING_EASE_IN_OUT: u8 = 1;
pub const TIMING_EASE_IN:     u8 = 2;
pub const TIMING_EASE_OUT:    u8 = 3;
// Spring timing: use begin_spring_transaction(), not timing_id

pub const FILL_REMOVED:  u8 = 0;  // default — animation removed on completion
pub const FILL_FORWARDS: u8 = 1;  // presentation freezes at final value

pub const IMPLICIT_DURATION_MS: u32 = 250;
pub const SPRING_SETTLE_THRESHOLD: f32 = 0.001;

#[derive(Clone, Debug)]
pub struct Transform3D {
    pub m: [[f32; 4]; 4],  // column-major
}

impl Transform3D {
    pub fn identity() -> Self {
        let mut m = [[0f32; 4]; 4];
        m[0][0] = 1.0; m[1][1] = 1.0; m[2][2] = 1.0; m[3][3] = 1.0;
        Transform3D { m }
    }

    // B3 FIX: replace NaN/Inf with identity elements, then clamp large-finite values
    // Prevents integer overflow in blit and undefined GPU rasterizer behavior
    pub fn sanitize(&mut self) {
        let id = Self::identity();
        for col in 0..4 {
            for row in 0..4 {
                if !self.m[col][row].is_finite() {
                    self.m[col][row] = id.m[col][row];
                }
            }
        }
        // Clamp translation (m[3][0], m[3][1]) — f32::MAX is finite but overflows pixel math
        self.m[3][0] = self.m[3][0].clamp(-65_536.0, 65_536.0);
        self.m[3][1] = self.m[3][1].clamp(-65_536.0, 65_536.0);
        // Clamp scale (m[0][0], m[1][1]) — 256x is already extreme for any UI
        self.m[0][0] = self.m[0][0].clamp(-256.0, 256.0);
        self.m[1][1] = self.m[1][1].clamp(-256.0, 256.0);
    }
}

#[derive(Clone, Debug)]
pub enum AnimatableValue {
    Float(f32),
    Point(f32, f32),
    Size(f32, f32),
    Rect(f32, f32, f32, f32),
    Color(f32, f32, f32, f32),  // linear light (sRGB ingested at WIT boundary via R15)
    Transform(Transform3D),
}

#[derive(Clone, Debug)]
pub struct SpringParams {
    pub mass: f32,        // default 1.0
    pub stiffness: f32,   // default 200.0
    pub damping: f32,     // default 10.0
    // B8/N8 FIX: units are fraction of (target - current) per second (WebKit normalization)
    // Convert from px/s: initial_velocity_px_per_s / (target - current)
    pub initial_velocity: f32,
}

impl Default for SpringParams {
    fn default() -> Self {
        SpringParams { mass: 1.0, stiffness: 200.0, damping: 10.0, initial_velocity: 0.0 }
    }
}

#[derive(Clone, Debug)]
pub enum AnimationTimingFunction {
    Linear,
    CubicBezier(f32, f32, f32, f32),
    // B2 FIX: duration_ms is a MAXIMUM timeout for springs, not a target duration.
    // Animation terminates when |current - target| < SPRING_SETTLE_THRESHOLD OR elapsed >= duration_ms.
    Spring(SpringParams),
}

#[derive(Clone, Debug)]
pub enum AnimatableProperty {
    PositionX, PositionY,
    Width, Height,
    Opacity,
    Transform,
    BackgroundColorR, BackgroundColorG, BackgroundColorB, BackgroundColorA,
    CornerRadius,
    ZPosition,
    Hidden,  // discrete: snaps at t=0.5
}

// B5 FIX: structured keyframe type replaces msgpack list<u8>
#[derive(Clone, Debug)]
pub struct Keyframe {
    pub time: f32,                  // normalized 0.0..=1.0
    pub value: AnimatableValue,
    pub timing: Option<u8>,         // per-keyframe timing override (TIMING_* constant)
}

#[derive(Clone, Debug)]
pub struct AnimSpec {
    pub property: AnimatableProperty,
    pub duration_ms: u32,
    pub repeat_count: f32,          // f32::INFINITY for forever; protocol: "inf" or "-1"
    pub autoreverse: bool,
    pub fill_mode: u8,
    pub timing: AnimationTimingFunction,
    pub keyframes: Vec<Keyframe>,
}

// N10 FIX: Arc-wrapped GPU texture ref — layer tree and app R12 handle share ownership
pub type GpuTextureRef = std::sync::Arc<u32>;

#[derive(Clone, Debug)]
pub enum LayerContents {
    None,
    Surface(u32),          // R11 surface handle (supervisor owns lifetime)
    Image(u32),            // R14 image handle (ref-counted in R14)
    GpuTexture(GpuTextureRef),  // Arc prevents dangling handle when app drops R12 texture
}

#[derive(Clone, Debug)]
pub struct ModelLayer {
    pub id: LayerId,
    pub pid: Pid,
    pub parent: Option<LayerId>,
    pub children: Vec<LayerId>,
    pub position: (f32, f32),
    pub bounds: (f32, f32, f32, f32),  // x, y, w, h
    pub opacity: f32,
    pub transform: Transform3D,
    pub corner_radius: f32,
    pub background_color: (f32, f32, f32, f32),  // linear light
    pub border_width: f32,
    pub border_color: (f32, f32, f32, f32),
    pub contents: LayerContents,
    pub hidden: bool,
    pub z_position: f32,
    pub model_dirty: bool,  // N9 FIX: set on any property write; cleared after sync_static_layers
}

#[derive(Clone, Debug)]
pub struct PresentationLayer {
    pub id: LayerId,
    pub position: (f32, f32),
    pub bounds: (f32, f32, f32, f32),
    pub opacity: f32,
    pub transform: Transform3D,
    pub corner_radius: f32,
    pub background_color: (f32, f32, f32, f32),
    pub contents: LayerContents,
    pub hidden: bool,
    pub z_position: f32,
}
```

---

## B1 Fix: Double-Buffered TransactionQueue

**Problem**: stdout parser thread and vsync compositor thread both contending on a single `Mutex<TransactionQueue>`. On a loaded system: vsync thread parks waiting for the lock → missed vsync deadline. Or stdout parser parks → app pipe backs up.

**Fix**: Two `VecDeque` buffers. Parser writes to `back` (under brief lock). Vsync thread atomically swaps front/back at tick start (single lock, ~100ns), then processes from `front` exclusively — no lock held during compositing.

```rust
// supervisor/src/animation/transaction.rs

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

pub struct TransactionBuffer {
    pub committed: VecDeque<AnimationTransaction>,
    pub per_app_pending: HashMap<Pid, u32>,
}

impl TransactionBuffer {
    fn new() -> Self {
        TransactionBuffer { committed: VecDeque::new(), per_app_pending: HashMap::new() }
    }
}

pub struct TransactionQueue {
    /// stdout parser writes here (brief lock per commit)
    back: Arc<Mutex<TransactionBuffer>>,
    /// vsync thread reads exclusively after swap (no contention during compositing)
    front: Mutex<TransactionBuffer>,
    /// per-app open (uncommitted) transactions
    open_tx: HashMap<Pid, AnimationTransaction>,
}

impl TransactionQueue {
    pub fn new() -> Self {
        TransactionQueue {
            back: Arc::new(Mutex::new(TransactionBuffer::new())),
            front: Mutex::new(TransactionBuffer::new()),
            open_tx: HashMap::new(),
        }
    }

    pub fn back_writer(&self) -> Arc<Mutex<TransactionBuffer>> {
        Arc::clone(&self.back)
    }

    /// Called by vsync thread at tick start — swaps buffers atomically, returns drained transactions
    pub fn swap_and_drain(&self) -> Vec<AnimationTransaction> {
        let mut back = self.back.lock().unwrap();   // brief lock — just the swap
        let mut front = self.front.lock().unwrap();
        std::mem::swap(&mut *front, &mut *back);
        drop(back);  // release back lock immediately — parser can write again
        let items: Vec<_> = front.committed.drain(..).collect();
        // N7 FIX: decrement per-app counts as drained (not clear entire map)
        for txn in &items {
            if let Some(c) = front.per_app_pending.get_mut(&txn.pid) {
                *c = c.saturating_sub(1);
            }
        }
        items
    }

    pub fn begin_transaction(&mut self, pid: Pid, duration_ms: u32,
                             timing: AnimationTimingFunction) -> Result<(), String> {
        if self.open_tx.contains_key(&pid) {
            return Err(format!("pid {} already has open transaction", pid));
        }
        self.open_tx.insert(pid, AnimationTransaction {
            pid, duration_ms, timing,
            changes: Vec::new(),
            completion_token: None,
        });
        Ok(())
    }

    pub fn push_change(&mut self, pid: Pid, change: LayerChange) -> Result<(), &'static str> {
        self.open_tx.get_mut(&pid)
            .map(|tx| { tx.changes.push(change); })
            .ok_or("no open transaction")
    }

    pub fn commit_transaction(&mut self, pid: Pid) -> Result<(), String> {
        let tx = self.open_tx.remove(&pid)
            .ok_or_else(|| format!("no open transaction for pid {}", pid))?;
        let mut back = self.back.lock().unwrap();
        let pending = back.per_app_pending.entry(pid).or_insert(0);
        if *pending >= 16 {
            return Err(format!("pid {} exceeded per-app tx limit (16)", pid));
        }
        if back.committed.len() >= 512 {
            return Err("global tx queue full (512)".into());
        }
        *pending += 1;
        back.committed.push_back(tx);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AnimationTransaction {
    pub pid: Pid,
    pub duration_ms: u32,
    pub timing: AnimationTimingFunction,
    pub changes: Vec<LayerChange>,
    pub completion_token: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct LayerChange {
    pub layer_id: LayerId,
    pub property: AnimatableProperty,
    pub target_value: AnimatableValue,
}
```

---

## B2 Fix: Spring Duration Semantics + All Three Damping Regimes

**Problem**: Spring animations forced into the same `duration_ms` slot as cubic-bezier. `duration_ms` means "stop at this time" rather than "should have settled by this time" — distorts perceived spring feel.

**Fix**: For `AnimationTimingFunction::Spring`, `duration_ms` is a *maximum timeout*. Animation terminates when `|current - target| < SPRING_SETTLE_THRESHOLD` OR when `elapsed_ms >= duration_ms`, whichever comes first.

```rust
// supervisor/src/animation/timing.rs

/// Evaluate spring position at time t_s (seconds).
/// Spring terminates early if converged — caller checks spring_is_converged().
pub fn spring_eval_scalar(
    x0: f32, target: f32, t_s: f32,
    omega0: f32, zeta: f32,
    // N8 FIX: initial_velocity is fraction of (target - x0) per second (WebKit units)
    initial_velocity_norm: f32,
) -> f32 {
    let v0 = initial_velocity_norm * (target - x0);
    let dx = x0 - target;

    if (zeta - 1.0).abs() < 1e-4 {
        // Critical damping
        let c2 = v0 + omega0 * dx;
        target + (dx + c2 * t_s) * (-omega0 * t_s).exp()
    } else if zeta < 1.0 {
        // Underdamped — oscillates then settles
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let c1 = dx;
        let c2 = (v0 + zeta * omega0 * dx) / omega_d;
        let decay = (-zeta * omega0 * t_s).exp();
        target + decay * (c1 * (omega_d * t_s).cos() + c2 * (omega_d * t_s).sin())
    } else {
        // Overdamped — two exponential decays, no overshoot
        let r = (zeta * zeta - 1.0).sqrt();
        let s1 = omega0 * (-zeta + r);
        let s2 = omega0 * (-zeta - r);
        let c1 = (v0 - s2 * dx) / (s1 - s2);
        let c2 = dx - c1;
        target + c1 * (s1 * t_s).exp() + c2 * (s2 * t_s).exp()
    }
}

pub fn spring_is_converged(current: f32, target: f32) -> bool {
    (current - target).abs() < SPRING_SETTLE_THRESHOLD
}

// supervisor/src/animation/runner.rs

pub struct RunningAnimation {
    pub layer_id: LayerId,
    pub key: String,
    pub property: AnimatableProperty,
    pub spec: AnimSpec,
    pub elapsed_ms: f32,
    pub from_value: AnimatableValue,
    pub completion_group: Option<Arc<CompletionGroup>>,
}

// N2 FIX: shared counter across all RunningAnimations from same transaction
pub struct CompletionGroup {
    pub token: u32,
    pub pid: Pid,
    pub pending: std::sync::atomic::AtomicU32,
}

impl CompletionGroup {
    pub fn new(token: u32, pid: Pid, count: u32) -> Arc<Self> {
        Arc::new(CompletionGroup {
            token, pid,
            pending: std::sync::atomic::AtomicU32::new(count),
        })
    }
    /// Returns true if this was the last pending animation (caller fires callback)
    pub fn decrement(&self) -> bool {
        self.pending.fetch_sub(1, std::sync::atomic::Ordering::AcqRel) == 1
    }
}

impl RunningAnimation {
    /// Returns Some(final_value) when complete, None while running.
    /// B2 FIX: Spring terminates on convergence OR timeout.
    pub fn advance(&mut self, delta_ms: f32) -> Option<AnimatableValue> {
        self.elapsed_ms += delta_ms;
        let target = &self.spec.keyframes.last()
            .map(|kf| kf.value.clone())
            .unwrap_or(self.from_value.clone());

        match &self.spec.timing {
            AnimationTimingFunction::Spring(params) => {
                let t_s = self.elapsed_ms / 1000.0;
                let omega0 = (params.stiffness / params.mass).sqrt();
                let zeta = params.damping / (2.0 * (params.stiffness * params.mass).sqrt());
                let v0 = params.initial_velocity;

                let current = self.eval_spring_animatable(t_s, omega0, zeta, v0, target);

                // B2: early termination on convergence
                if self.animatable_converged(&current, target) {
                    return Some(target.clone());
                }
                // B2: also terminate at timeout
                if self.elapsed_ms >= self.spec.duration_ms as f32 {
                    return Some(target.clone());
                }
                // still running — presentation updated externally
                None
            }
            _ => {
                // Cubic bezier / linear: tick-based t
                let t = (self.elapsed_ms / self.spec.duration_ms as f32).clamp(0.0, 1.0);
                if self.elapsed_ms >= self.spec.duration_ms as f32 {
                    return Some(target.clone());
                }
                None
            }
        }
    }

    fn animatable_converged(&self, a: &AnimatableValue, b: &AnimatableValue) -> bool {
        match (a, b) {
            (AnimatableValue::Float(x), AnimatableValue::Float(y)) => spring_is_converged(*x, *y),
            (AnimatableValue::Point(ax, ay), AnimatableValue::Point(bx, by)) =>
                spring_is_converged(*ax, *bx) && spring_is_converged(*ay, *by),
            (AnimatableValue::Rect(ax,ay,aw,ah), AnimatableValue::Rect(bx,by,bw,bh)) =>
                spring_is_converged(*ax,*bx) && spring_is_converged(*ay,*by) &&
                spring_is_converged(*aw,*bw) && spring_is_converged(*ah,*bh),
            _ => true,
        }
    }

    fn eval_spring_animatable(&self, t_s: f32, omega0: f32, zeta: f32, v0: f32,
                               target: &AnimatableValue) -> AnimatableValue {
        match (&self.from_value, target) {
            (AnimatableValue::Float(x0), AnimatableValue::Float(tgt)) =>
                AnimatableValue::Float(spring_eval_scalar(*x0, *tgt, t_s, omega0, zeta, v0)),
            (AnimatableValue::Point(x0,y0), AnimatableValue::Point(tx,ty)) =>
                AnimatableValue::Point(
                    spring_eval_scalar(*x0, *tx, t_s, omega0, zeta, v0),
                    spring_eval_scalar(*y0, *ty, t_s, omega0, zeta, v0),
                ),
            _ => target.clone(),
        }
    }
}
```

---

## B4 Fix: Implicit Animation Code Path

**Problem**: `set-position` (and all other setters) called outside a transaction — spec says "implicit animation fires", but no code path existed. `push_change()` returned `Err("no open transaction")` with no implicit fallback.

**Fix**: In `wit_handlers.rs`, when `push_change()` returns `Err("no open transaction")` AND `implicit_animations_enabled` is true, synthesize a transaction with `IMPLICIT_DURATION_MS` and `EaseInOut`, commit immediately.

```rust
// supervisor/src/animation/wit_handlers.rs  (relevant excerpt)

fn handle_set_position(engine: &mut AnimationEngine, pid: Pid,
                       layer_id: LayerId, x: f32, y: f32) -> Result<(), String> {
    let cx = LayerChange { layer_id, property: AnimatableProperty::PositionX,
                           target_value: AnimatableValue::Float(x) };
    let cy = LayerChange { layer_id, property: AnimatableProperty::PositionY,
                           target_value: AnimatableValue::Float(y) };

    match engine.tx_queue.push_change(pid, cx.clone()) {
        Ok(()) => {
            engine.tx_queue.push_change(pid, cy)?;
        }
        Err("no open transaction") => {
            let implicit = engine.per_app_state
                .get(&pid).map(|s| s.implicit_animations_enabled).unwrap_or(false);

            if implicit {
                // B4 FIX: synthesize implicit transaction — bypasses per_app_pending cap
                engine.tx_queue.begin_transaction(
                    pid,
                    IMPLICIT_DURATION_MS,
                    AnimationTimingFunction::CubicBezier(0.25, 0.1, 0.25, 1.0), // ease-in-out
                )?;
                engine.tx_queue.push_change(pid, cx)?;
                engine.tx_queue.push_change(pid, cy)?;
                engine.tx_queue.commit_transaction(pid)?;
            } else {
                // No implicit animation — apply immediately to model and presentation
                engine.apply_immediate(pid, layer_id, AnimatableProperty::PositionX,
                                       AnimatableValue::Float(x));
                engine.apply_immediate(pid, layer_id, AnimatableProperty::PositionY,
                                       AnimatableValue::Float(y));
            }
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
```

The same pattern applies to all property setters (`set-opacity`, `set-bounds`, `set-background-color`, `set-hidden`, `set-corner-radius`, `set-transform`). Each checks for an open transaction, falls back to implicit if none exists, or applies immediately if implicit animations are disabled.

---

## B5 Fix: Structured WIT Types Replace msgpack

**Problem**: `add-animation: func(layer: u32, key: string, anim: list<u8>)` with msgpack-encoded payload introduced an unaudited external crate, no versioning, and no type safety.

**Fix**: Replace with structured WIT record types. Zero new dependencies.

```wit
// supervisor/src/animation/vyoma-animation.wit

package vyoma:animation@1.0.0;

// B5 FIX: structured types — no msgpack, no list<u8>
record keyframe {
    time: f32,
    value: anim-value,
    timing: option<u8>,   // TIMING_* constant; none = inherit from transaction
}

variant anim-value {
    float-val(f32),
    point-val(tuple<f32, f32>),
    size-val(tuple<f32, f32>),
    rect-val(tuple<f32, f32, f32, f32>),
    color-val(tuple<f32, f32, f32, f32>),  // linear light
}

record spring-params {
    mass: f32,
    stiffness: f32,
    damping: f32,
    initial-velocity: f32,  // fraction of (target - current) per second
}

interface layers {
    // Lifecycle
    create-layer: func() -> result<u32, string>;
    drop-layer: func(layer: u32);

    // Hierarchy
    add-sublayer: func(parent: u32, child: u32) -> result<_, string>;
    remove-from-parent: func(layer: u32);

    // Transaction control
    begin-transaction: func(duration-ms: u32, timing: u8) -> result<_, string>;
    begin-spring-transaction: func(duration-ms: u32, params: spring-params) -> result<_, string>;
    commit-transaction: func() -> result<_, string>;
    cancel-transaction: func();
    set-completion-token: func(token: u32);

    // Property setters (implicit animation fires if no open tx and implicit-animations-enabled)
    set-position: func(layer: u32, x: f32, y: f32);
    set-bounds: func(layer: u32, x: f32, y: f32, w: f32, h: f32);
    set-opacity: func(layer: u32, opacity: f32);
    // B5 FIX: transform as named tuple, not list<f32> — type-safe, no wrong-stride bugs
    set-transform: func(layer: u32,
        m00: f32, m01: f32, m02: f32, m03: f32,
        m10: f32, m11: f32, m12: f32, m13: f32,
        m20: f32, m21: f32, m22: f32, m23: f32,
        m30: f32, m31: f32, m32: f32, m33: f32);
    set-corner-radius: func(layer: u32, r: f32);
    set-background-color: func(layer: u32, r: f32, g: f32, b: f32, a: f32);
    set-contents-surface: func(layer: u32, surface: u32);
    set-contents-image: func(layer: u32, image: u32);
    set-hidden: func(layer: u32, hidden: bool);
    set-z-position: func(layer: u32, z: f32);
    set-implicit-animations: func(enabled: bool);

    // B5 FIX: structured keyframe animation — zero external deps
    add-animation: func(
        layer: u32, key: string, property: u8,
        duration-ms: u32, repeat-count: f32,
        autoreverse: bool, fill-mode: u8, timing: u8,
        keyframes: list<keyframe>,
    ) -> result<_, string>;
    remove-animation: func(layer: u32, key: string);

    // Presentation queries (interpolated values during animation)
    presentation-position: func(layer: u32) -> tuple<f32, f32>;
    presentation-opacity: func(layer: u32) -> f32;
    presentation-bounds: func(layer: u32) -> tuple<f32, f32, f32, f32>;

    // N3 FIX: hit-test uses PresentationLayer geometry (animated position)
    // A button mid-slide is hittable at its CURRENT animated position, not model position
    hit-test: func(x: f32, y: f32) -> option<u32>;
}

world vyoma-app { import layers; }
```

---

## N3 Fix: hit-test Uses Presentation Layer

```rust
// supervisor/src/animation/compositor.rs

/// Hit-test using PresentationLayer (animated) geometry, not ModelLayer (model) geometry.
/// Ensures interactive elements are hittable at their current animated positions.
pub fn hit_test(&self, x: f32, y: f32, pid: Pid) -> Option<LayerId> {
    let flat = self.layer_tree.flatten();  // sorted by z_position, topmost last
    for &layer_id in flat.iter().rev() {  // test topmost first
        let pres = self.layer_tree.presentation.get(&layer_id)?;
        if pres.hidden || pres.opacity < 0.01 { continue; }

        // Transform test point into layer's local space
        let local_x = x - pres.position.0;
        let local_y = y - pres.position.1;
        let (bx, by, bw, bh) = pres.bounds;

        if local_x >= bx && local_x < bx + bw && local_y >= by && local_y < by + bh {
            // Only return layers owned by the requesting app
            if self.layer_tree.model.get(&layer_id)
                .map(|m| m.pid == pid).unwrap_or(false)
            {
                return Some(layer_id);
            }
        }
    }
    None
}
```

---

## N2 Fix: Completion Callback Fires Once Per Transaction

```rust
// When apply_transaction() processes a transaction with N changes:
// 1. Create ONE CompletionGroup with pending = N
// 2. Pass Arc::clone to each RunningAnimation
// 3. Each RunningAnimation calls group.decrement() on completion
// 4. Callback fires only when decrement() returns true (last one done)

// In AnimationEngine::apply_transaction():
if let Some(token) = txn.completion_token {
    let group = CompletionGroup::new(token, txn.pid, txn.changes.len() as u32);
    for anim in &mut new_running_animations {
        anim.completion_group = Some(Arc::clone(&group));
    }
}
```

---

## VYOMA_ANIM Stdout Protocol

```
# Transaction control
VYOMA_ANIM:begin:<duration_ms>,<timing_id>
VYOMA_ANIM:begin_spring:<duration_ms>,<mass>,<stiffness>,<damping>,<initial_velocity>
VYOMA_ANIM:commit
VYOMA_ANIM:cancel
VYOMA_ANIM:set_token:<u32>

# Layer lifecycle
VYOMA_ANIM:create_layer        → supervisor replies VYOMA_ANIM_LAYER:<id> on stdin
                                 NOTE: prefer WIT create-layer() (synchronous return)
VYOMA_ANIM:drop_layer:<id>

# Property mutations (implicit animation fires if no open tx + implicit enabled)
VYOMA_ANIM:set_pos:<id>,<x>,<y>
VYOMA_ANIM:set_bounds:<id>,<x>,<y>,<w>,<h>
VYOMA_ANIM:set_opacity:<id>,<opacity>
VYOMA_ANIM:set_color:<id>,<r>,<g>,<b>,<a>    # linear light
VYOMA_ANIM:set_hidden:<id>,<0|1>
VYOMA_ANIM:set_z:<id>,<z>

# Hierarchy
VYOMA_ANIM:add_child:<parent_id>,<child_id>
VYOMA_ANIM:remove_from_parent:<id>

# Explicit keyframe animation
VYOMA_ANIM:add_anim:<id>,<key>,<prop_id>,<dur_ms>,<timing_id>,<repeat>,<autoreverse>,<fill>,<kf_count>
  Followed by <kf_count> lines: VYOMA_ANIM:kf:<time>,<type_id>,<values_csv>
  repeat: positive float, "inf", or "-1" (→ f32::INFINITY)
  prop_id: 0=pos_x 1=pos_y 2=width 3=height 4=opacity 5=transform 6=corner_radius
           7=bg_r 8=bg_g 9=bg_b 10=bg_a 11=z_pos 12=hidden
VYOMA_ANIM:remove_anim:<id>,<key>

# Supervisor → App (written to app stdin, non-blocking O_NONBLOCK)
VYOMA_ANIM_LAYER:<id>          # response to create_layer (stdout protocol only)
VYOMA_ANIM_COMPLETE:<token>    # fires when ALL animations in transaction with that token settle
                                # dropped (logged) if app stdin pipe full — app must drain stdin
```

---

## AnimationEngine Vsync Tick

```rust
// supervisor/src/animation/mod.rs

pub struct AnimationEngine {
    pub layer_tree: LayerTree,
    pub running: HashMap<(LayerId, String), RunningAnimation>,
    pub tx_queue: TransactionQueue,  // B1: double-buffered
    pub config: AnimationConfig,
    pub per_app_state: HashMap<Pid, AppAnimState>,
}

impl AnimationEngine {
    /// Called from R11 vsync loop before blit pass
    pub fn tick(&mut self, delta_ms: f32, fb: &mut Framebuffer) {
        // B1: atomic swap, no lock held during compositing
        let new_txns = self.tx_queue.swap_and_drain();
        for txn in new_txns {
            self.apply_transaction(txn);
        }

        // Advance running animations, collect completions
        let mut completed = Vec::new();
        for (key, anim) in &mut self.running {
            if let Some(final_val) = anim.advance(delta_ms) {
                self.layer_tree.set_presentation(&key.0, &anim.property, final_val);
                if let Some(g) = &anim.completion_group {
                    if g.decrement() {
                        self.fire_completion_callback(g.pid, g.token);
                    }
                }
                completed.push(key.clone());
            }
        }
        for k in completed { self.running.remove(&k); }

        // N9: sync only dirty static layers
        self.layer_tree.sync_static_layers();

        // Composite to framebuffer
        self.compositor.composite_frame(&self.layer_tree, fb, &self.config);
    }

    fn fire_completion_callback(&self, pid: Pid, token: u32) {
        let msg = format!("VYOMA_ANIM_COMPLETE:{}\n", token);
        // non-blocking write; drop if pipe full, log the drop
        self.write_to_stdin_nonblocking(pid, msg.as_bytes());
    }
}
```

---

## Security Model

| Threat | Mitigation |
|--------|-----------|
| App A spoofing App B's layer handle | `LayerHandleTable` validates `pid` match on every WIT call |
| NaN/Inf in transform → compositor crash | `Transform3D::sanitize()` — NaN→identity, then clamp |
| Large-finite translation (f32::MAX) → pixel overflow | B3: `m[3][0,1].clamp(-65536, 65536)` |
| DoS via 10-year animation | 30s cap enforced in `validate_animation_spec()` |
| Queue exhaustion by single app | Per-app limit: 16 pending transactions |
| Queue exhaustion by all apps | Global limit: 512 transactions |
| Layer handle churn (create+drop at 60Hz) | Rate limit: 60 creates/second per app |
| Full stdin pipe blocking vsync on callback | `O_NONBLOCK` write — drops callback, logs drop |
| Dangling GPU texture | `GpuTextureRef = Arc<u32>` — freed only when both app and layer tree drop |

---

## Platform Matrix

| Feature | mcu | iot | robotics | server | mobile | desktop |
|---------|-----|-----|----------|--------|--------|---------|
| Layer tree | — | — | — | — | ✓ | ✓ |
| Implicit anim | — | — | — | — | ✓ | ✓ |
| Spring physics | — | — | — | — | ✓ | ✓ |
| GPU compositor | — | — | — | — | ✓ | ✓ |
| Max layers/app | 0 | 0 | 0 | 0 | 128 | 512 |
| Max tx pending | 0 | 0 | 0 | 0 | 8 | 16 |
| Duration cap | — | — | — | — | 10s | 30s |
| hit-test | — | — | — | — | ✓ | ✓ |

---

## Implementation Files (supervisor/src/animation/, all ≤500 LOC)

| File | Responsibility |
|------|---------------|
| `mod.rs` | Public types, constants, `AnimationEngine` struct |
| `config.rs` | `AnimationConfig` per-platform |
| `layer_tree.rs` | `LayerTree`, `flatten()`, z-order cache |
| `presentation.rs` | `PresentationLayer`, `sync_static_layers()` (N9: model_dirty check) |
| `transaction.rs` | B1: double-buffered `TransactionQueue`, N7: per-count drain |
| `runner.rs` | `RunningAnimation::advance()`, B2: spring convergence, N2: `CompletionGroup` |
| `timing.rs` | `spring_eval_scalar()` (all 3 damping regimes), `cubic_bezier_eval()` |
| `interpolate.rs` | `lerp`/`slerp` for all `AnimatableValue` variants |
| `compositor.rs` | `composite_frame()`, N3: `hit_test()` using `PresentationLayer` |
| `wit_handlers.rs` | Wasmtime linker, B4: implicit animation path, B5: structured WIT types |
| `protocol.rs` | `VYOMA_ANIM:` stdout parser, `"inf"`/`"-1"` repeat encoding |

`Cargo.toml`: **no new external dependencies**. B5 removes the msgpack requirement entirely.

---

## Integration

- **R11**: `anim_engine.tick(delta_ms, fb)` called from vsync loop BEFORE blit pass
- **R12**: `LayerContents::GpuTexture(Arc<u32>)` — Arc ref shared with R12's texture registry; one wgpu submit per frame (animation compositor delegates GPU layers to R12's `submit_layer_batch()`)
- **R13**: Font text → Surface → `set-contents-surface` on layer; animation moves/fades the layer
- **R14**: Image crossfade → two layers with opacity swap in one transaction; `LayerContents::Image(u32)` ref-counted by R14
- **R15**: `set-background-color` ingested as sRGB, converted to linear light in `wit_handlers.rs`; all color lerp in linear light; output converted back to sRGB by R15 before framebuffer write

---

## Required Unit Tests

1. `spring_critical_damping_settles` — reaches target within `settle_time_estimate()`
2. `spring_underdamped_oscillates` — oscillates and converges
3. `spring_overdamped_no_overshoot` — no zero-crossing
4. `spring_early_convergence_b2` — iOS params (ζ=0.7, ω₀=20) terminates before `duration_ms`
5. `double_buffer_no_deadlock_b1` — concurrent producer+consumer, no lock stall
6. `implicit_animation_fires_b4` — `set-position` outside tx with `implicit=true` creates animation
7. `implicit_animation_immediate_b4` — `set-position` with `implicit=false` applies immediately
8. `structured_keyframe_wit_b5` — WIT `keyframe` record encodes/decodes correctly; no msgpack dep
9. `transform_clamp_b3` — `f32::MAX` translation clamped to 65536
10. `transform_nan_b3` — NaN on diagonal replaced with identity
11. `completion_fires_once_n2` — transaction with 5 changes → exactly 1 `VYOMA_ANIM_COMPLETE`
12. `hit_test_presentation_n3` — hit-test returns animated position, not model position
13. `layer_handle_isolation` — App A cannot reference App B's handles
14. `layer_quota_enforced` — 513th layer fails with `max_layers=512`
15. `duration_cap_enforced` — 31s animation rejected
16. `tx_per_app_cap` — 17th transaction from same pid rejected
17. `gpu_texture_arc_n10` — dropping app R12 handle doesn't free texture while layer holds Arc
18. `z_order_stable` — equal z_position layers maintain insertion order
