# Round 12 — GPU Acceleration Layer

**Role:** Architect
**Date:** 2026-05-29
**Status:** Proposal (awaiting Critic review)

---

## 0. Executive summary

VyomaOS is a WASM-first operating system: every user-facing application is a
`wasm32-wasip2` binary executed inside Wasmtime under the Rust PID-2 supervisor.
Round 11 established a software compositor that blits per-window `Surface`s into
a single framebuffer using Porter–Duff over and SIMD-friendly AVX2 loops on the
CPU. That is acceptable for the eight-window Phase-17 demo but it does not
scale to a macOS-class desktop where forty translucent surfaces, three full-bleed
animations, blurred toolbars, and shader-driven shaders for screensavers may
all need to run at 120 Hz simultaneously.

This round designs the GPU Acceleration Layer. The design is anchored on five
non-negotiables that fall out of Rounds 1–11:

1. **Sandbox preservation.** A WASM app is, by construction, forbidden from
   issuing raw Vulkan / DRM ioctls. Every GPU primitive an app touches must be
   mediated by the supervisor through a typed WIT interface — no
   capability-leaking pointer can cross the wasm/native boundary.
2. **wgpu-as-WIT.** The canonical GPU API for WASM is WebGPU. VyomaOS exposes a
   WIT-modeled WebGPU surface (`vyoma:gpu/webgpu@0.2.0`) and lets the
   in-supervisor `wgpu_core` crate execute on behalf of the app against a real
   Vulkan / virtio-gpu / Lavapipe backend. The wgpu Rust crate inside the app
   compiles against an `wgpu-wit` adapter shim that forwards the WebGPU object
   graph over a Wasmtime resource handle table; no protocol invention required.
3. **GPU as accelerator, never as oracle.** R11's software compositor remains
   the source-of-truth implementation. The GPU path is a fast lane: when GPU is
   available *and* the scene meets a complexity threshold, the supervisor
   switches to the GPU compositor. If VRAM is exhausted, a shader fails to
   compile, or the kernel driver crashes, the supervisor must transparently
   downgrade to CPU compositing within one frame.
4. **VRAM is a budget, not a resource pool.** The supervisor publishes a VRAM
   budget per QoS class (R5) and refuses allocations that would exceed it.
   Compositor textures always preempt app shader workloads. There is no
   page-faulting GPU swap (Apple's "purgeable" semantics are not implementable
   on Lima/Panfrost in 2026).
5. **No GPU compute capability without explicit declaration.** A new manifest
   field `capabilities.gpu` carries a struct, not a bool: it splits *graphics*
   (rasterization, presents) from *compute* (dispatch, storage buffers) because
   the timing channel attack surface of compute dispatch is materially higher.
   `gpu_compute = true` is gated behind the same review process as
   `network = true`.

The deliverable is a 13-file `supervisor/src/gpu/` module (each < 500 LOC) plus
a new WIT package, two new manifest fields, three new HAL traits, and a
documented degradation ladder. Six rounds of v1-skips are listed explicitly so
the Critic can hold us to them.

---

## 1. Design philosophy

### 1.1 Why WebGPU and not OpenGL ES / Vulkan / a custom protocol

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| Raw Vulkan over WIT | Fastest possible; mature drivers | 800-call API; reams of UB; descriptor management leaks pointers; impossible to sandbox without effectively re-inventing WebGPU | **Reject** |
| OpenGL ES 3.2 over WIT | Mali / PowerVR have first-class GLES drivers; well-understood | Stateful API is awful to sandbox; deprecated trajectory upstream; no compute on most embedded GLES | **Reject** |
| Custom display-list protocol (à la Skia) | Tiny attack surface; easy to ship | Forces every app to ship its own raster engine; can't accelerate games or shader art; misses the macOS Metal target | **Reject for v1** (kept as fallback in §4.4) |
| **WebGPU via wgpu** | Designed for sandboxes; multi-backend (Vulkan / Metal / D3D12 / virtio-gpu); Rust-native via `wgpu_core`; WGSL is a sane shader language; compiles to wasm32-wasip2 already | Younger spec; some platform extensions (mesh shaders, ray tracing) missing | **Accept** |

WebGPU was *designed* for the exact threat model we have: untrusted code that
must touch a GPU through an arms-length adapter. The wgpu Rust ecosystem (which
already publishes a `wgpu-types` crate compatible with `no_std + alloc`) lets
us:

- ship a thin `wgpu-wit` crate that apps depend on,
- run `wgpu_core` inside the supervisor against `wgpu-hal`'s Vulkan or
  virtio-gpu (gfx-rs) backend,
- get DXC-quality WGSL → SPIR-V compilation from the `naga` crate without
  shelling out to glslc or DXC.

### 1.2 What the WASM app sees

A WASM app linked against `wgpu-wit` (a drop-in replacement for the `wgpu`
crate, identical surface API) calls:

```rust
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
let adapter  = instance.request_adapter(&Default::default()).await?;
let (device, queue) = adapter.request_device(&Default::default(), None).await?;
let pipeline = device.create_render_pipeline(&desc);
// ... build a command buffer ...
queue.submit(Some(encoder.finish()));
```

Under the hood every call is a host function call into
`vyoma:gpu/webgpu/Instance.request-adapter`, returning a `resource adapter` —
a Wasmtime-managed integer handle bound to a row in the supervisor's
`wgpu_core::Adapter` table. The app *cannot* dereference the handle, copy it
between processes, or forge one: handles are unforgeable in the WIT canonical
ABI by construction.

### 1.3 What the supervisor sees

The supervisor owns:

- a single `wgpu_core::Instance` per boot,
- one `Device` per (physical GPU × QoS class), so a `Background` app cannot
  starve a `UserInteractive` queue,
- a per-app `ResourceTable<WgpuObject>` keeping handle → wgpu-core ID maps,
- a `VramBudget` accountant per QoS class with O(1) charge / refund.

### 1.4 When does the compositor use GPU vs CPU?

The compositor switches modes based on scene complexity, measured per-frame:

```
mode = match (scene_complexity, gpu_available, vram_pressure) {
    (Trivial,   _,     _)        => Cpu,
    (Moderate,  false, _)        => Cpu,
    (Moderate,  true,  Low)      => Gpu,
    (Heavy,     true,  Low|Mid)  => Gpu,
    (Heavy,     true,  High)     => CpuFallback,
    (Heavy,     false, _)        => CpuFallback,
};
```

`Trivial` ≤ 2 opaque surfaces; `Moderate` 3–8 surfaces with ≤ 2 alpha layers;
`Heavy` everything else. Detailed thresholds in §4.

The choice is made at the start of each compositor frame; mid-frame switching
is forbidden (introduces tearing).

---

## 2. GPU abstraction layers

### 2.1 The three-layer cake (mapped from Metal)

| macOS layer | VyomaOS equivalent | Files |
|---|---|---|
| Metal Shading Language + MTLDevice/Queue/CommandBuffer | `vyoma:gpu/webgpu` WIT package + `wgpu-wit` adapter in apps | `gpu/wit.rs`, `gpu/host_impl.rs` |
| MetalKit (compositor-side helpers, drawables) | `GpuCompositor` (R11 GPU path) | `gpu/compositor.rs`, `gpu/present.rs` |
| IOSurface / IOAcceleratorFamily (kernel) | `wgpu-hal` + virtio-gpu / Lima / Panfrost DRM | `hal/gpu.rs`, `gpu/backend.rs` |

### 2.2 wgpu Device management policy

```rust
// supervisor/src/gpu/devices.rs   (< 400 LOC target)

use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;

use crate::scheduler::QosClass;
use crate::gpu::vram::VramBudget;

/// One physical GPU adapter on the host.
///
/// On `desktop-full` under QEMU this is virtio-gpu; on Mali ARM SBCs it is the
/// Panfrost-backed Mali. On `mcu-minimal` / `server-headless` no `Gpu` exists.
pub struct Gpu {
    pub id:          GpuId,
    pub name:        String,
    pub vendor:      GpuVendor,
    pub limits:      wgpu_types::Limits,
    pub features:    wgpu_types::Features,
    pub vram_bytes:  u64,
    pub adapter:     Arc<wgpu_core::id::AdapterId>,
    /// One logical Device per QoS class so a Background compute job
    /// cannot drown a UserInteractive present.
    devices:         RwLock<HashMap<QosClass, Arc<LogicalDevice>>>,
    pub budget:      Arc<VramBudget>,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct GpuId(pub u16);

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum GpuVendor {
    Virtio,
    Mali,
    PowerVR,
    Adreno,
    Nvidia,
    Amd,
    Intel,
    SoftwareLavapipe,
}

pub struct LogicalDevice {
    pub qos:        QosClass,
    pub device_id:  wgpu_core::id::DeviceId,
    pub queue_id:   wgpu_core::id::QueueId,
    /// Per-device texture/buffer tables. Keyed by (app_id, handle).
    pub objects:    RwLock<ResourceTable>,
}
```

#### Rationale: one Device per (Gpu × QoS), not per app

Per-app Devices would waste descriptor heap budget (Mali only supports 8
concurrent Vulkan devices on some kernels). Per-QoS gives us:

- preemption boundary (cancel all Background work to land a UserInteractive
  present),
- separate command queues so OS chrome (UserInteractive) is never blocked by a
  game (UserInitiated),
- minimum number of Vulkan logical devices on resource-constrained drivers.

The `ResourceTable` per `LogicalDevice` namespaces objects by `app_id`, so
buffer 7 in app A is not the same handle as buffer 7 in app B even though they
share a Device. This matches the WebGPU spec's tab-isolation model.

### 2.3 Adapter selection algorithm

```rust
pub enum AdapterPolicy {
    /// Caller wants the best-performance discrete GPU.
    HighPerformance,
    /// Caller wants the lowest-power integrated GPU.
    LowPower,
    /// Caller wants a software adapter (Lavapipe) explicitly, e.g. for tests.
    SoftwareOnly,
}

impl GpuRegistry {
    pub fn select(&self, policy: AdapterPolicy, qos: QosClass)
        -> Result<Arc<LogicalDevice>, GpuError>
    {
        let candidates: Vec<&Gpu> = self.gpus.iter()
            .filter(|g| g.matches_policy(policy))
            .collect();
        let best = candidates.into_iter()
            .min_by_key(|g| g.budget.pressure_score(qos))
            .ok_or(GpuError::NoSuitableAdapter)?;
        Ok(best.logical_device_for(qos))
    }
}
```

On `desktop-full` there is only one GPU (virtio-gpu) so `select` is trivial; on
real mobile hardware with a Mali + a small NPU we will need to handle two
adapters (NPU is excluded from the GPU layer entirely; it lives in a future
"Accel" round).

### 2.4 Why we do not expose a per-app Device

We could in principle give each WASM app its own `wgpu::Device`, but this
forces a copy from the per-app uploader to the compositor's texture pool every
frame. Instead, both the compositor and the WASM app use the *same Device*
indexed by the compositor's QoS (`UserInteractive`); a WASM app's render target
is the compositor's source texture, with zero copy. The supervisor only needs
to flip a sub-resource between Read (compositor sample) and Write (app render)
at command-buffer boundaries via wgpu's resource state tracker.

---

## 3. WIT interface for GPU command submission

### 3.1 Package layout

```
wit/vyoma/gpu.wit
├── interface webgpu              # the main WebGPU surface
├── interface present             # compositor presentation primitives
├── interface compute              # gated subset; requires gpu_compute cap
└── world  gpu-app                  # default world apps link against
```

### 3.2 The WIT surface (excerpt)

The full WIT file is ~700 LOC; the essential resources are reproduced here so
the Critic can review the shape. Conventions follow the [W3C WebGPU IDL] 1:1
but expressed in WIT's resource/record/variant grammar.

```wit
package vyoma:gpu@0.2.0;

interface webgpu {
    use vyoma:display/types@0.2.0.{surface-id};

    /// Top-level access point. One Instance per app, created lazily on first call.
    resource instance {
        constructor();
        request-adapter:    func(opts: adapter-options) -> result<adapter, gpu-error>;
        enumerate-adapters: func() -> list<adapter-info>;
    }

    record adapter-options {
        power-preference:        power-preference,
        force-fallback-adapter:  bool,
        compatible-surface:      option<surface-id>,
    }

    enum power-preference { low-power, high-performance, fallback }

    record adapter-info {
        vendor:   string,
        device:   string,
        backend:  backend-kind,
        limits:   limits,
        features: features,
    }

    enum backend-kind { vulkan, virtio-gpu, panfrost, lavapipe-cpu }

    resource adapter {
        request-device: func(desc: device-descriptor)
            -> result<tuple<device, queue>, gpu-error>;
        get-info:       func() -> adapter-info;
    }

    record device-descriptor {
        label:           option<string>,
        required-features: features,
        required-limits:   limits,
    }

    resource device {
        create-buffer:           func(desc: buffer-descriptor)        -> result<buffer, gpu-error>;
        create-texture:          func(desc: texture-descriptor)       -> result<texture, gpu-error>;
        create-sampler:          func(desc: sampler-descriptor)       -> result<sampler, gpu-error>;
        create-shader-module:    func(desc: shader-module-descriptor) -> result<shader-module, gpu-error>;
        create-render-pipeline:  func(desc: render-pipeline-descriptor)
            -> result<render-pipeline, gpu-error>;
        create-compute-pipeline: func(desc: compute-pipeline-descriptor)
            -> result<compute-pipeline, gpu-error>;
        create-bind-group:       func(desc: bind-group-descriptor)    -> result<bind-group, gpu-error>;
        create-command-encoder:  func(desc: command-encoder-descriptor)
            -> result<command-encoder, gpu-error>;
        on-uncaptured-error:     func(cb: func(e: gpu-error));
        destroy:                 func();
    }

    resource queue {
        write-buffer:  func(b: borrow<buffer>, offset: u64, data: list<u8>) -> result<_, gpu-error>;
        write-texture: func(dest: image-copy-texture, data: list<u8>,
                            layout: image-data-layout, size: extent-3d) -> result<_, gpu-error>;
        submit:        func(cmds: list<command-buffer>);
        on-submitted-work-done: func(cb: func());
    }

    resource buffer {
        size:            func() -> u64,
        usage:           func() -> buffer-usage,
        map-async:       func(mode: map-mode, offset: u64, size: u64)
            -> result<_, gpu-error>;
        /// Zero-copy slice into the app's linear memory.  See §3.4.
        get-mapped-range: func(offset: u64, size: u64)
            -> result<mapped-slice, gpu-error>;
        unmap:           func();
        destroy:         func();
    }

    /// A borrowed pointer + length into the app's WASM linear memory.
    /// The host validates that [ptr, ptr+len) is inside the app's memory at
    /// submit time; the app's wgpu-wit adapter materializes this as &mut [u8].
    record mapped-slice {
        ptr:  u32,    // offset in the app's linear memory
        len:  u32,
    }
}

interface present {
    use vyoma:display/types@0.2.0.{surface-id};
    use webgpu.{texture, queue};

    resource swap-chain {
        get-current-texture: func() -> result<texture, gpu-error>;
        present:             func();
        configure:           func(desc: swap-chain-descriptor)
            -> result<_, gpu-error>;
    }

    record swap-chain-descriptor {
        format:        texture-format,
        usage:         texture-usage,
        width:         u32,
        height:        u32,
        present-mode:  present-mode,
        alpha-mode:    alpha-mode,
    }

    enum present-mode    { fifo, fifo-relaxed, mailbox, immediate }
    enum alpha-mode      { opaque, premultiplied, postmultiplied }

    /// The supervisor calls into the app via this to allow rendering a frame.
    resource frame-callback {
        invoke: func(t: timestamp-ns);
    }
}

interface compute {
    use webgpu.{device, command-encoder, compute-pipeline, bind-group, queue};

    /// Gated: only available if vyoma.toml declares gpu_compute = true.
    resource compute-pass-encoder {
        set-pipeline: func(p: borrow<compute-pipeline>);
        set-bind-group: func(idx: u32, g: borrow<bind-group>, dyn-offsets: list<u32>);
        dispatch-workgroups: func(x: u32, y: u32, z: u32);
        dispatch-workgroups-indirect: func(b: borrow<webgpu.buffer>, offset: u64);
        end: func();
    }
}

world gpu-app {
    import vyoma:gpu/webgpu@0.2.0;
    import vyoma:gpu/present@0.2.0;
    import vyoma:gpu/compute@0.2.0;  // host-side fails if cap not granted
}
```

### 3.3 Resource handle table

The Wasmtime canonical ABI gives us free unforgeable handles. The supervisor
maintains:

```rust
// supervisor/src/gpu/resource_table.rs   (< 350 LOC)

use slab::Slab;
use parking_lot::Mutex;

use wgpu_core::id::{
    AdapterId, DeviceId, QueueId, BufferId, TextureId, SamplerId,
    ShaderModuleId, RenderPipelineId, ComputePipelineId,
    BindGroupId, BindGroupLayoutId, CommandEncoderId, CommandBufferId,
    RenderPassEncoderId, ComputePassEncoderId,
};

#[derive(Clone)]
pub enum WgpuObject {
    Adapter(AdapterId),
    Device(DeviceId),
    Queue(QueueId),
    Buffer(BufferId),
    Texture(TextureId),
    Sampler(SamplerId),
    ShaderModule(ShaderModuleId),
    RenderPipeline(RenderPipelineId),
    ComputePipeline(ComputePipelineId),
    BindGroup(BindGroupId),
    BindGroupLayout(BindGroupLayoutId),
    CommandEncoder(CommandEncoderId),
    CommandBuffer(CommandBufferId),
    RenderPassEncoder(RenderPassEncoderId),
    ComputePassEncoder(ComputePassEncoderId),
}

pub struct ResourceTable {
    inner: Mutex<Slab<WgpuObject>>,
    /// Charge against the owning app's VRAM budget for objects in this table.
    /// On drop, all rows are refunded.
    charges: Mutex<Vec<(u32, u64)>>,
}

impl ResourceTable {
    pub fn insert(&self, obj: WgpuObject, bytes: u64) -> Handle {
        let key = self.inner.lock().insert(obj);
        if bytes > 0 {
            self.charges.lock().push((key as u32, bytes));
        }
        Handle(key as u32)
    }
    pub fn get(&self, h: Handle) -> Option<WgpuObject> {
        self.inner.lock().get(h.0 as usize).cloned()
    }
    pub fn remove(&self, h: Handle) -> Option<(WgpuObject, u64)> {
        let mut slab = self.inner.lock();
        let obj = slab.try_remove(h.0 as usize)?;
        let bytes = {
            let mut cs = self.charges.lock();
            let pos = cs.iter().position(|(k,_)| *k == h.0);
            pos.map(|p| cs.swap_remove(p).1).unwrap_or(0)
        };
        Some((obj, bytes))
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct Handle(pub u32);
```

### 3.4 Zero-copy mapped buffers

WebGPU's `mapAsync`/`getMappedRange` semantics are perfect for our SharedBuffer
story (R2). When an app calls `getMappedRange`, the supervisor:

1. Verifies that the app's mapped buffer was created with `MAP_READ` or
   `MAP_WRITE`.
2. Computes the linear-memory offset where the buffer lives. Two cases:
   - **CPU-backed (default).** A `Vec<u8>` in the supervisor staging arena.
     The host copies (memcpy) the staging into the app's linear memory at the
     offset returned by `mapped-slice`. Bidirectional sync at `unmap`.
   - **SharedBuffer-backed.** The buffer was created with
     `usage: SHARED_BUFFER`. The compositor allocated the buffer using
     `SurfaceMemoryCreator` (R11/R2), so the buffer's backing pages are *the
     same physical pages* as a slice of the app's linear memory. The
     `mapped-slice.ptr` returned is just the offset of those pages inside the
     app's linear memory. **No copy.**

The SharedBuffer case is the same mechanism as the R11 SharedBuffer surface
back-buffer, generalised from "compositor framebuffer" to "any GPU buffer".

### 3.5 Command buffer validation

The supervisor *cannot trust* a command buffer constructed by the app. Wgpu's
validator already covers most attack vectors (out-of-bounds binds, type
mismatches, unfinished encoders), but VyomaOS adds:

```rust
pub struct CommandBufferAudit {
    pub max_draw_calls:       u32,
    pub max_dispatch_groups:  u32,
    pub max_indirect_offset:  u64,
    pub max_bind_groups_used: u32,
    /// Wall-clock budget for replay on the queue. Submit fails if estimated.
    pub max_duration_ns:      u64,
}
```

The supervisor computes a static cost estimate (draw count × bound-pipeline
cost, no actual GPU timing) against the audit; over-budget submits are
rejected with `GpuError::QuotaExceeded`. Estimation lives in
`gpu/cost_model.rs`.

---

## 4. GPU-accelerated compositor path

### 4.1 Scene-complexity scoring

```rust
// supervisor/src/gpu/compositor.rs   (< 500 LOC; split into compositor.rs + pass.rs)

pub struct SceneStats {
    pub surface_count:        u16,
    pub alpha_surfaces:       u16,
    pub avg_surface_pixels:   u32,
    pub max_surface_pixels:   u32,
    pub blurred_surfaces:     u8,
    pub animating_surfaces:   u8,
    pub damaged_pixels:       u32,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum SceneComplexity { Trivial, Moderate, Heavy }

impl SceneStats {
    pub fn classify(&self) -> SceneComplexity {
        if self.surface_count <= 2 && self.alpha_surfaces == 0 {
            return SceneComplexity::Trivial;
        }
        if self.surface_count <= 8
            && self.alpha_surfaces <= 2
            && self.blurred_surfaces == 0
            && self.animating_surfaces <= 1
        {
            return SceneComplexity::Moderate;
        }
        SceneComplexity::Heavy
    }
}
```

### 4.2 The compositor mode machine

```rust
pub enum CompositorMode {
    /// R11 software path: per-window CPU blit with Porter–Duff.
    Cpu,
    /// GPU path: every Surface is a texture, composited via one render pass.
    Gpu,
    /// GPU is preferred but unavailable this frame; degrade to CPU and log.
    CpuFallback(FallbackReason),
}

#[derive(Copy, Clone, Debug)]
pub enum FallbackReason {
    NoGpuAdapter,
    VramExhausted,
    DriverError,
    ShaderCompileFailed,
    HardOomKill,
    PlatformDisabled,
}

pub struct Compositor {
    pub mode:      AtomicCompositorMode,
    pub gpu:       Option<Arc<GpuCompositor>>,
    pub cpu:       Arc<CpuCompositor>,        // R11
    pub vram:      Arc<VramBudget>,
    pub fallbacks: Mutex<RollingCount<8>>,    // last 8 fallback events
}
```

### 4.3 GPU compositor pass

The GPU path is one render pass per output:

```rust
pub struct GpuCompositor {
    device:        Arc<LogicalDevice>,
    pipeline:      RenderPipelineId,            // textured-quad with Porter-Duff
    blur_pipeline: Option<ComputePipelineId>,   // background blur, gpu_compute opt
    sampler:       SamplerId,
    swapchain:     SwapchainId,
    quad_vb:       BufferId,
    instance_vb:   BufferId,                    // per-surface instance data
    /// Per-app Surface → GPU texture mapping. Refcounted with the Surface.
    surface_tex:   RwLock<HashMap<SurfaceId, Arc<SurfaceTexture>>>,
}

pub struct SurfaceTexture {
    pub texture:   TextureId,
    pub view:      TextureViewId,
    pub bytes:     u64,
    pub mode:      SurfaceTextureMode,
}

pub enum SurfaceTextureMode {
    /// Texture is allocated as host-coherent so the app's CPU writes are
    /// immediately visible to the GPU (SharedBuffer path).
    SharedMapped,
    /// Texture is device-local; updates use queue.write_texture to upload.
    DeviceLocal,
    /// Surface is sampled directly from a virtio-gpu blob resource owned by
    /// the kernel — no upload needed (DRM dmabuf import). Best case.
    Dmabuf,
}
```

The render pass body is the canonical "draw N textured quads in z-order"
program; the shader is shared across all platforms (WGSL):

```wgsl
// supervisor/src/gpu/shaders/composite.wgsl  (< 80 LOC)

struct Instance {
    dst_rect:   vec4<f32>,    // x, y, w, h in output px
    src_rect:   vec4<f32>,    // u, v, w, h in [0,1] UV
    opacity:    f32,
    radius:     f32,          // corner radius in px
    shadow:     f32,          // shadow intensity 0..1
    _pad:       u32,
};

@group(0) @binding(0) var<storage,read> insts: array<Instance>;
@group(0) @binding(1) var          out_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       uv:   vec2<f32>,
    @location(1)       inst: u32,
};

@vertex fn vs_main(@location(0) corner: vec2<f32>,
                   @builtin(instance_index) iid: u32) -> VsOut {
    let i = insts[iid];
    let pos = i.dst_rect.xy + i.dst_rect.zw * corner;
    var o: VsOut;
    o.clip = vec4<f32>(pos * 2.0 - 1.0, 0.0, 1.0);
    o.uv   = i.src_rect.xy + i.src_rect.zw * corner;
    o.inst = iid;
    return o;
}

@group(1) @binding(0) var atlas: texture_2d<f32>;

@fragment fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let i = insts[in.inst];
    let c = textureSample(atlas, out_sampler, in.uv);
    let alpha = c.a * i.opacity * corner_mask(in.uv, i.radius);
    return vec4<f32>(c.rgb * alpha, alpha);
}
```

Each `Surface` is one instance; an entire scene draws in one draw call with
`instance_count = surface_count`. This is dramatically faster than R11's
per-window memcpy because the GPU rasterises in parallel and never touches the
CPU cache lines holding the surfaces.

### 4.4 DRM/KMS plane assignment (overlay planes)

Some GPUs expose hardware overlay planes (Mali Bifrost: 4 planes; virtio-gpu:
2 planes via the `multiplane` extension). When a surface fully covers the
output OR is a fullscreen video plane, the supervisor can hand it directly to
KMS as an overlay plane, costing zero GPU cycles for that surface:

```rust
pub struct PlaneAssignment {
    pub primary:  Option<SurfaceId>,
    pub overlay:  ArrayVec<SurfaceId, 4>,
    pub cursor:   Option<SurfaceId>,
}

impl GpuCompositor {
    pub fn maybe_assign_planes(&self, scene: &Scene) -> Option<PlaneAssignment> {
        let drm = self.hal.drm()?;
        if !drm.supports_atomic_modeset() { return None; }
        // Pick the bottom-most opaque surface as primary; fullscreen video as
        // an overlay; cursor on its own plane.
        let primary = scene.bottom_opaque_covering_output()?;
        let cursor  = scene.cursor_surface();
        let overlays = scene.fullscreen_videos().take(drm.overlay_planes() as usize - 1);
        Some(PlaneAssignment { primary: Some(primary), overlay: overlays, cursor })
    }
}
```

If `maybe_assign_planes` returns Some, the compositor render pass skips those
surfaces; KMS does the work in hardware. This is the same trick macOS uses for
fullscreen video on Apple Silicon and is a 30-50% power win.

### 4.5 Damage tracking integration

R11's damage tracking (per-surface dirty rects) carries over: the GPU path
uses `dst_rect = damage_rect ∩ surface_bounds` so a partial-update animation
re-renders only its damaged sub-region. The fragment shader is gated by a
scissor rect; the vertex shader culls the instance entirely if the damage rect
is empty.

---

## 5. GPU memory management

### 5.1 VRAM budget

```rust
// supervisor/src/gpu/vram.rs   (< 350 LOC)

use std::sync::atomic::{AtomicU64, Ordering};

pub struct VramBudget {
    pub total_bytes:   u64,
    /// Reserved exclusively for compositor textures. Cannot be drawn from by
    /// apps. Default: max(64 MiB, 25% of total).
    pub composite_res: u64,
    /// Reserved for the OS chrome (status bar, menus). Default: 16 MiB.
    pub chrome_res:    u64,
    /// Sum of in-use allocations per QoS class.
    pub used_ui:       AtomicU64,    // UserInteractive
    pub used_init:     AtomicU64,    // UserInitiated
    pub used_util:     AtomicU64,    // Utility
    pub used_bg:       AtomicU64,    // Background
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum VramPressure { Low, Mid, High }

impl VramBudget {
    pub fn pressure(&self) -> VramPressure {
        let used = self.used_ui.load(Ordering::Relaxed)
                 + self.used_init.load(Ordering::Relaxed)
                 + self.used_util.load(Ordering::Relaxed)
                 + self.used_bg.load(Ordering::Relaxed);
        let ratio = used as f64 / self.total_bytes as f64;
        match ratio {
            r if r < 0.65 => VramPressure::Low,
            r if r < 0.85 => VramPressure::Mid,
            _             => VramPressure::High,
        }
    }

    pub fn try_charge(&self, qos: QosClass, bytes: u64) -> Result<(), VramError> {
        let cell = self.cell(qos);
        let mut cur = cell.load(Ordering::Relaxed);
        loop {
            let new = cur + bytes;
            if new + self.composite_res + self.chrome_res > self.total_bytes {
                return Err(VramError::WouldExceedBudget);
            }
            match cell.compare_exchange_weak(cur, new,
                Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_)  => return Ok(()),
                Err(e) => cur = e,
            }
        }
    }

    pub fn refund(&self, qos: QosClass, bytes: u64) {
        self.cell(qos).fetch_sub(bytes, Ordering::AcqRel);
    }

    fn cell(&self, qos: QosClass) -> &AtomicU64 {
        match qos {
            QosClass::UserInteractive => &self.used_ui,
            QosClass::UserInitiated   => &self.used_init,
            QosClass::Utility         => &self.used_util,
            QosClass::Background      => &self.used_bg,
            QosClass::Maintenance     => &self.used_bg,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VramError {
    #[error("would exceed VRAM budget")]
    WouldExceedBudget,
    #[error("driver returned ENOSPC")]
    DriverOom,
}
```

### 5.2 OOM handling

When `try_charge` returns `WouldExceedBudget`:

1. The supervisor calls `Compositor.evict()` to drop *cached* (not in-use)
   surface textures (e.g. textures for minimised windows).
2. Retry `try_charge`. If still fails, return `GpuError::OutOfMemory` to the
   app.
3. If a *compositor* texture allocation fails, the compositor records a
   `FallbackReason::VramExhausted` and degrades to `CpuFallback` for the next
   frame. The CPU compositor never allocates VRAM.
4. If the kernel driver returns ENOSPC despite our budget passing, we treat
   this as a hard fault: kill the offending app (the one whose allocation
   triggered the call) and degrade compositor mode.

### 5.3 Surface texture lifecycle

```rust
pub enum SurfaceTextureState {
    /// Surface is composited every frame; texture stays in VRAM.
    Hot,
    /// Surface composited but partially obscured; texture stays.
    Warm,
    /// Surface is minimised; texture can be reclaimed at any time.
    Cached,
    /// Surface has been evicted; will be re-uploaded on next show.
    Evicted,
}
```

Eviction picks `Cached` first, then LRU `Warm`, never `Hot`. The compositor
never evicts a texture mid-frame.

### 5.4 SharedBuffer GPU path

R11 introduced SharedBuffer surfaces backed by `SurfaceMemoryCreator`. With
GPU we need three variants:

| Variant | Backing | Update path |
|---|---|---|
| `SharedBuffer::Cpu` (R11 default) | WASM linear memory page = CPU `Vec<u8>` slice | CPU compositor reads directly; GPU compositor `queue.write_texture` per damage rect |
| `SharedBuffer::HostCoherent` (new) | DMA-coherent VRAM page mapped to WASM linear memory via `SurfaceMemoryCreator::with_gpu(...)`. Requires a `host-coherent` heap (Mali yes, Panfrost yes, virtio-gpu via blob resources) | App writes → GPU reads, *no upload*. Single mmap. |
| `SharedBuffer::Dmabuf` (new) | dmabuf import from `/dev/dri/cardN` | Available only for video decode etc., not the app's writable memory |

Selection at Surface creation:

```rust
impl Surface {
    pub fn new_shared(qos: QosClass, fmt: SurfaceFormat, sz: (u32,u32))
        -> Result<Self, SurfaceError>
    {
        let want_gpu = compositor_mode_likely() == CompositorMode::Gpu;
        let host_coherent_ok = gpu::registry().host_coherent_supported();
        match (want_gpu, host_coherent_ok) {
            (true, true)  => Self::new_shared_host_coherent(qos, fmt, sz),
            (true, false) => Self::new_shared_cpu_with_upload_hint(qos, fmt, sz),
            (false, _)    => Self::new_shared_cpu(qos, fmt, sz),  // R11 path
        }
    }
}
```

---

## 6. Platform support matrix and feature flags

### 6.1 Per-platform GPU policy

| Platform | GPU adapter | Backend | Default compositor | Notes |
|---|---|---|---|---|
| `mcu-minimal` | None | None | CPU only (Mono1bpp) | `gpu` cap is rejected at manifest load |
| `iot-edge` | Mali G31 (or none) | Panfrost / Lima → wgpu-hal vulkan | CPU; opt-in GPU if Vulkan probe succeeds | Mid-pressure VRAM budget: 64 MiB total |
| `robotics-rt` | None or minimal DRM | None | CPU only | Hard real-time; GPU latency unpredictable |
| `mobile` | Mali / PowerVR | Panfrost / Imagination | GPU | Power management critical (§6.4) |
| `desktop-full` (QEMU) | virtio-gpu | virgl / Lavapipe / vulkan | GPU | Default development target |
| `server-headless` | Optional | None or Lavapipe | CPU; GPU only if `gpu_compute` apps installed | No display; compute-only path |

### 6.2 Profile TOML extension

```toml
# supervisor/src/profile/profiles/desktop-full.toml

[gpu]
enabled = true
backend = "auto"                  # "auto" | "vulkan" | "virtio" | "panfrost" | "lavapipe" | "off"
vram_budget_mb = 512              # total budget
composite_reserve_mb = 128
chrome_reserve_mb = 16
allow_gpu_compute = true          # if false, gpu_compute capability is denied at manifest load
overlay_planes = 2
host_coherent_buffers = true
```

```toml
# supervisor/src/profile/profiles/iot-edge.toml

[gpu]
enabled = true
backend = "panfrost"
vram_budget_mb = 64
composite_reserve_mb = 16
chrome_reserve_mb = 4
allow_gpu_compute = false
overlay_planes = 0
host_coherent_buffers = true
```

```toml
# mcu-minimal.toml
[gpu]
enabled = false
```

### 6.3 Compile-time feature flags

```toml
# supervisor/Cargo.toml

[features]
default            = ["gpu", "gpu-vulkan", "gpu-virtio"]
gpu                = ["dep:wgpu-core", "dep:wgpu-types", "dep:wgpu-hal", "dep:naga"]
gpu-vulkan         = ["wgpu-hal/vulkan"]
gpu-virtio         = ["dep:virtio-gpu", "wgpu-hal/gles"]   # gles over virgl
gpu-panfrost       = ["wgpu-hal/vulkan", "dep:panfrost-sys"]
gpu-lavapipe       = ["wgpu-hal/vulkan", "dep:lavapipe-bundled"]
gpu-compute        = []                                     # gated arch-wide kill switch
```

For `mcu-minimal` we build with `--no-default-features`, omitting the entire
`gpu/` module. The compositor compiles down to just the R11 software path.

### 6.4 Power management (mobile)

Each `LogicalDevice` registers itself with the R7 PowerManager and can be
clock-gated when idle for > 100 ms. The compositor explicitly *un-gates* the
GPU before submitting a present, and a watchdog re-gates 50 ms after the last
submission completes. App-side GPU work outside the compositor follows the
same rules but is rejected entirely while the device is in `Background QoS`
deep-sleep state (R7 `DeepSleep`).

---

## 7. Shader compilation pipeline

### 7.1 Pipeline overview

```
App source     ──► WGSL text  ──► (in supervisor) naga ──► SPIR-V ──► driver
or WGSL text                        ▲    ▲     ▲
                                    │    │     │
                            validate │ optimize│ assemble
```

WGSL is the only shader source language we accept. GLSL → WGSL conversion is
*not* provided by the supervisor; if an app ships GLSL it must convert at app
build time using `naga` linked into its own binary. This minimises attack
surface — naga is already > 30 KLOC and we do not want to add SPIRV-Cross or a
GLSL frontend.

### 7.2 Compilation flow

`device.create_shader_module(desc)` takes:

```rust
pub enum ShaderSource {
    Wgsl(String),
    SpirV(Vec<u32>),     // only with feature `passthrough_spirv` + `gpu_compute` cap
    NagaIr(Vec<u8>),     // for the precompile cache
}
```

The supervisor:

1. Parses WGSL → naga IR (`naga::front::wgsl::parse`).
2. Validates with `naga::valid::Validator::new(ValidationFlags::all(),
   Capabilities::default())`.
3. Lowers to backend IR (SPIR-V via `naga::back::spv` for Vulkan, MSL only on
   future macOS host port, GLSL for virgl GLES backend).
4. Hands SPIR-V to wgpu-hal which forwards to the driver.

Compilation runs on the **compile thread pool** (a `tokio::task::spawn_blocking`
on a 2-thread pool, QoS = `Utility`), not on the present thread. The app's
`createShaderModule` await blocks until compilation completes; the typical
shader takes 8–40 ms.

### 7.3 Caching

Two cache layers:

**Process-local cache.** Keyed by `Blake3(WGSL source)` → `Arc<NagaIr>` →
`(BackendKind, SpirVBytes)`. Bounded by an LRU with 64 entries per app.

**Disk cache.** `/data/shaders/<app-id>/<blake3>.spv`. Pre-warmed during app
install (the package manager from Phase 14 runs shaders through the
compile pipeline at `install_app` time when `[shaders]` is present in the
manifest).

```toml
# vyoma.toml (extension)
[capabilities]
gpu = true

[shaders]
# Optional precompile manifest; install-time compilation.
sources = [
  "shaders/main.wgsl",
  "shaders/blur.wgsl",
]
```

### 7.4 Compile budget

Shader compilation is bounded:

```rust
pub struct ShaderQuota {
    /// Max compilations per app per minute.
    pub per_minute:  u16,
    /// Max parallel compilations across the system.
    pub parallel:    u8,
    /// Max single compilation duration before timeout.
    pub max_ms:      u32,
}

impl Default for ShaderQuota {
    fn default() -> Self {
        Self { per_minute: 60, parallel: 4, max_ms: 500 }
    }
}
```

Apps that hit the rate limit get `GpuError::ShaderQuotaExceeded`. This stops a
malicious app from DoSing the compile thread pool by submitting one shader per
frame.

### 7.5 Hot reload

Developer mode (`VYOMA_DEV=1`) bypasses the disk cache and forces
recompilation on every `createShaderModule`. Not for production builds.

---

## 8. GPU app capabilities

### 8.1 Manifest schema additions

```toml
# vyoma.toml

[app]
name    = "game-of-life"
version = "0.1.0"
wasm    = "game-of-life.wasm"

[capabilities]
stdio        = true
display      = true

# NEW: GPU is a struct, not a bool. Defaults: all false.
[capabilities.gpu]
graphics     = true     # may create render pipelines, swap chains, present
compute      = false    # may create compute pipelines, dispatch (gated)
preferred_adapter = "auto"   # "auto" | "low-power" | "high-performance"
max_vram_mb  = 64       # per-app cap; the supervisor may grant less

# Optional: precompile shaders at install time
[shaders]
sources = ["shaders/render.wgsl"]
```

### 8.2 Capability matrix

| Capability | What it unlocks |
|---|---|
| `gpu.graphics = true` | `webgpu` Instance/Adapter/Device; render pipelines; `present` interface; surface backing storage as GPU texture |
| `gpu.compute = true`  | `compute` interface (compute pipelines, dispatch); storage buffers with read/write usage; indirect dispatch; required for *any* `STORAGE` buffer usage |
| `gpu.preferred_adapter` | Hint passed to `request-adapter`; the supervisor may override based on QoS |
| `gpu.max_vram_mb` | Per-app VRAM cap. The supervisor refuses allocations that would exceed this even if global budget allows it |

Apps without `gpu.graphics` cannot import `vyoma:gpu/webgpu`; the world
linkage fails at component instantiation time. This is enforced in the
manifest loader (see `manifest.rs`):

```rust
fn build_wit_imports(caps: &Capabilities) -> Vec<WitImport> {
    let mut v = vec![];
    if caps.stdio   { v.push(WitImport::Stdio); }
    if caps.display { v.push(WitImport::Display); }
    if caps.gpu.graphics {
        v.push(WitImport::GpuWebgpu);
        v.push(WitImport::GpuPresent);
    }
    if caps.gpu.compute {
        // gpu.compute requires gpu.graphics; checked in validator.
        v.push(WitImport::GpuCompute);
    }
    v
}
```

### 8.3 Manifest validation rules

- `gpu.compute = true` ⇒ `gpu.graphics = true` (a compute-only path will be
  added in a later round if needed).
- `gpu.compute = true` is rejected if the platform profile sets
  `allow_gpu_compute = false`.
- `gpu.max_vram_mb` must be ≤ profile `vram_budget_mb`.
- `gpu` is rejected on `mcu-minimal` and `robotics-rt`.
- `shaders.sources` paths must resolve under the app's wasm directory; cannot
  escape it.

### 8.4 Wasmtime linking

```rust
// supervisor/src/runtime/wasmtime.rs (extended)

pub fn link_gpu_imports(
    linker: &mut Linker<HostState>,
    caps:   &Capabilities,
) -> anyhow::Result<()> {
    if !caps.gpu.graphics { return Ok(()); }
    crate::gpu::wit::add_to_linker_webgpu(linker, |s| &mut s.gpu_webgpu)?;
    crate::gpu::wit::add_to_linker_present(linker, |s| &mut s.gpu_present)?;
    if caps.gpu.compute {
        crate::gpu::wit::add_to_linker_compute(linker, |s| &mut s.gpu_compute)?;
    }
    Ok(())
}
```

---

## 9. Security model

### 9.1 Threat model

A WASM app is *untrusted*. It may attempt:

1. **Sandbox escape** — write outside its linear memory via GPU pointers.
2. **VRAM exhaustion** — DoS by allocating until OOM.
3. **Compile-time DoS** — submit pathological WGSL until the compile pool is
   saturated.
4. **Cross-app information leak** — read another app's textures via:
   - Re-using a buffer that was not zero-initialised after another app freed
     it,
   - Indirect timing attacks on shared GPU caches,
   - GPU memory aliasing (e.g. virtio-gpu blob resources shared accidentally).
5. **Privilege escalation against the supervisor** — submit a command buffer
   that overwrites a compositor-owned texture.
6. **Driver / kernel exploit** — submit a SPIR-V binary that triggers a
   Panfrost CVE.

### 9.2 Mitigations

| Threat | Mitigation |
|---|---|
| Sandbox escape via pointer | All buffer / texture handles are unforgeable WIT resources; raw pointers are never crossed; `mapped-slice` is validated against the app's linear-memory bounds at call time |
| VRAM exhaustion | Per-app `max_vram_mb`; global VramBudget; QoS class budgets; OOM returns error, never kills |
| Compile DoS | ShaderQuota (rate-limit, parallelism cap, per-shader timeout); compile thread pool isolated |
| Cross-app texture leak | Every newly-allocated buffer/texture is zero-initialised by the supervisor before the app sees it (wgpu's `mapped_at_creation=true` path is wrapped to memset 0 if app didn't write); explicit `usage` flag enforcement |
| Cross-app timing channel | One Device per QoS class — apps in same QoS share but `gpu.compute` apps are forbidden from `UserInteractive`; compositor never shares Device with any app's Device unless QoS matches |
| Command-buffer attack on compositor | Compositor's textures are created in a private ResourceTable; app's command buffers cannot reference them. Texture handles never cross the WIT boundary into apps |
| Driver exploit via SPIR-V | Only naga-emitted SPIR-V reaches the driver; `passthrough_spirv` is gated behind `gpu.compute = true` and the platform profile (off by default) |
| Storage buffer side-channel | `gpu.compute` apps cannot run while a `gpu.graphics`-only app is presenting on `UserInteractive`; the scheduler serialises them |

### 9.3 Texture initialisation policy

```rust
pub struct TextureInitPolicy {
    pub zero_on_create:          bool,
    pub zero_on_reuse_from_pool: bool,
    pub require_initial_write:   bool,
}

impl Default for TextureInitPolicy {
    fn default() -> Self {
        Self {
            zero_on_create: true,
            zero_on_reuse_from_pool: true,
            require_initial_write: false,
        }
    }
}
```

`zero_on_create` makes texture creation 1.5× slower but eliminates a class of
information leaks. We accept the cost; the GPU does the clear via a render
pass with `LoadOp::Clear` so it parallelises with the next frame's setup.

### 9.4 Compute-graphics quarantine

This is the hardest problem. GPU compute caches share with rasterisation on
every commodity GPU. To minimise timing channels we enforce:

- A `gpu.compute = true` app may *not* run concurrently with the compositor.
  The compositor flushes its present, then the compute queue runs; only when
  compute finishes (or hits a budget) does the next compositor frame begin.
  This costs frames per second but is the only way to prevent compositor-side
  cache observation by a compute app.
- Compute apps are scheduled at `Utility` or lower QoS; `UserInteractive`
  apps may *not* declare `gpu.compute`.

This is a v1 stance. A more nuanced model (rate-limit compute dispatch
between presents) is deferred to v2.

### 9.5 Audit logging

Every GPU error and every fallback event is logged to the structured
observability sink (R10):

```rust
pub struct GpuAuditEvent {
    pub ts:        u64,
    pub app_id:    AppId,
    pub kind:      GpuAuditKind,
    pub detail:    String,
}

pub enum GpuAuditKind {
    AllocCharge,
    AllocRefund,
    AllocDenied,
    ShaderCompiled,
    ShaderFailed,
    SubmitAccepted,
    SubmitRejected,
    DeviceLost,
    Fallback(FallbackReason),
}
```

---

## 10. Implementation files and line budgets

The new module:

```
supervisor/src/gpu/
├── mod.rs                  // 200 LOC — module root, public re-exports, init()
├── registry.rs             // 380 LOC — Gpu / GpuRegistry / probe & enumerate
├── devices.rs              // 400 LOC — LogicalDevice / ResourceTable per QoS
├── resource_table.rs       // 320 LOC — Handle / Slab / charge tracking
├── wit.rs                  // 480 LOC — WIT bindgen glue + host_state extension
├── host_impl.rs            // 480 LOC — host-side methods for every WIT resource
├── compositor.rs           // 480 LOC — GpuCompositor, render pass orchestration
├── present.rs              // 320 LOC — swap chain, vsync coupling to R10 TimerLoop
├── vram.rs                 // 320 LOC — VramBudget, pressure, OOM
├── shader_cache.rs         // 380 LOC — naga front+validate, disk cache, quota
├── cost_model.rs           // 260 LOC — command-buffer audit cost estimator
├── backend.rs              // 240 LOC — wgpu-hal selection (vulkan / virtio / lavapipe)
├── shaders/
│   ├── composite.wgsl      //  80 LOC — textured-quad compositor
│   ├── blur.wgsl           // 110 LOC — separable Gaussian (gpu_compute opt-in)
│   └── cursor.wgsl         //  60 LOC — hardware cursor fallback fragment
└── tests/
    ├── budget.rs           // 280 LOC — VramBudget unit
    ├── handles.rs          // 240 LOC — ResourceTable charge/refund
    ├── compositor.rs       // 360 LOC — scene-mode classification + fallback
    └── shader_quota.rs     // 220 LOC — rate limiting & compile cache
```

Plus the small modifications elsewhere:

| File | Change | Lines added |
|---|---|---|
| `supervisor/src/manifest.rs` | parse `[capabilities.gpu]`, `[shaders]`, validate | ~ 80 |
| `supervisor/src/profile/mod.rs` | parse `[gpu]` block per profile | ~ 60 |
| `supervisor/src/profile/profiles/*.toml` | per-profile `[gpu]` block | ~ 12 each |
| `supervisor/src/runtime/wasmtime.rs` | `link_gpu_imports` | ~ 30 |
| `supervisor/src/hal/mod.rs` | `GpuDevice`, `DmabufImporter` traits | ~ 80 |
| `supervisor/src/hal/gpu.rs` (new) | platform glue for Mali/virtio/Lavapipe | ~ 380 |
| `supervisor/src/display/compositor.rs` | call out to `gpu::compositor` | ~ 60 |
| `supervisor/src/display/surface.rs` | `SharedBuffer::HostCoherent` variant | ~ 90 |
| `supervisor/src/lib.rs` | `pub mod gpu;` | 1 |
| `wit/vyoma/gpu.wit` (new) | 700 LOC; the WIT package | 700 |

Every Rust file stays under the 500-LOC repo rule.

### 10.1 Build/feature gating

When `--no-default-features`:

- `supervisor/src/gpu/` is gated by `#[cfg(feature = "gpu")]` in `lib.rs`.
- `supervisor/src/runtime/wasmtime.rs::link_gpu_imports` becomes a no-op stub.
- The manifest validator rejects `gpu = true` early with a clear error.

### 10.2 Trait definitions

```rust
// supervisor/src/hal/gpu.rs (excerpt)

use crate::hal::error::HalError;

pub trait GpuHal: Send + Sync {
    fn probe(&self) -> Result<Vec<HalGpuInfo>, HalError>;
    fn open_device(&self, id: HalGpuId) -> Result<Box<dyn HalGpuDevice>, HalError>;
}

pub struct HalGpuInfo {
    pub id:        HalGpuId,
    pub vendor:    String,
    pub backend:   HalBackend,
    pub vram:      u64,
    pub host_coherent: bool,
    pub overlay_planes: u8,
}

pub enum HalBackend { Vulkan, VirtioGpu, Panfrost, LavapipeCpu, PowerVR, Adreno }

pub trait HalGpuDevice: Send + Sync {
    fn submit_present(&self, frame: &PresentFrame) -> Result<(), HalError>;
    fn import_dmabuf(&self, fd: i32, layout: DmabufLayout)
        -> Result<HalTextureId, HalError>;
    fn set_overlay_planes(&self, assign: &PlaneAssignment) -> Result<(), HalError>;
    fn power_state(&self) -> HalPowerState;
    fn set_power_state(&self, s: HalPowerState) -> Result<(), HalError>;
}
```

---

## 11. Interaction with prior rounds

| Round | Touchpoint |
|---|---|
| R2 SharedBuffer | `SharedBuffer::HostCoherent` extends R2 model so a Surface backed by app linear memory is *also* a GPU texture. `SurfaceMemoryCreator` is extended to allocate from a host-coherent GPU heap. |
| R5 QoS | One `LogicalDevice` per QoS; VramBudget per QoS; compute apps capped at Utility QoS |
| R6 DeviceManager | The `GpuHal` is registered as a R6 device; probing happens in R6 enumeration phase |
| R7 PowerManager | `LogicalDevice` is a power-managed entity; clock-gated when idle |
| R9 HAL | New `GpuHal` trait alongside `DisplayDevice`, with per-platform implementations |
| R10 TimerLoop | The GPU present-callback is the vsync source for the compositor when GPU mode is active; falls back to TimerLoop tick otherwise |
| R11 Display Server & Compositor | The R11 software compositor remains the fallback; `GpuCompositor` is its accelerated sibling. R11's `Surface`, `SurfaceFormat`, `Scene`, and damage tracking are reused unchanged |

### 11.1 Surface compatibility

R11 `SurfaceFormat`:

| R11 format | GPU texture format | Notes |
|---|---|---|
| `Bgra32` | `BGRA8_UNORM_SRGB` | Default; matches every desktop GPU |
| `Rgb565` | `RGB565_UNORM` (Vulkan ext) | Mobile only; fallback to RGBA8 if unsupported |
| `Mono1bpp` | `R8_UNORM` | Expanded by shader; only for mcu/minimal — but mcu has no GPU, so unreachable in practice |

### 11.2 Compositor frame timing

R10 owns the master timer. The GPU compositor registers a vsync callback:

```rust
impl GpuCompositor {
    pub fn install_vsync(&self, timer: &Arc<TimerLoop>) {
        let comp = Arc::clone(self);
        timer.on_vsync(Box::new(move |t: VsyncEvent| {
            comp.try_present_frame(t);
        }));
    }
}
```

If `try_present_frame` overruns the vsync budget (e.g. shader compile blocks
present), we miss one vsync and the R10 TimerLoop forces the CPU fallback for
the next 4 frames (a small hysteresis to avoid thrashing).

---

## 12. Testing strategy

### 12.1 Layered tests

| Layer | Test | Tooling |
|---|---|---|
| Unit (pure logic) | `VramBudget` charge/refund races, OOM, multi-QoS | `cargo test --lib gpu::vram` |
| Unit (resource table) | Handle insert / get / remove / charge accounting | `cargo test --lib gpu::resource_table` |
| Unit (compositor mode) | Scene classification, fallback hysteresis | `cargo test --lib gpu::compositor` |
| Unit (shader cache) | Compile, cache hit, quota rate-limit, timeout | `cargo test --lib gpu::shader_cache` |
| Integration (WIT) | Boot an app linked against `wgpu-wit`; create buffer, write, map, read back | `make smoke-gpu` |
| Integration (compositor switching) | Mock GPU; assert frame-by-frame mode is correct as scene complexity changes | `make smoke-gpu-modes` |
| End-to-end (QEMU + virtio-gpu) | Boot `desktop-full`, run `triangle.wasm` app, screenshot, hash | `make smoke-gpu-qemu` |
| Stress | Allocate until VRAM exhaustion; assert compositor degrades, app gets `OutOfMemory` | `cargo test --lib gpu::vram -- stress --ignored` |

### 12.2 Critical invariants

```rust
#[test]
fn vram_budget_never_negative() { /* ... */ }

#[test]
fn compositor_never_panics_on_oom() { /* ... */ }

#[test]
fn shader_quota_rejects_after_burst() { /* ... */ }

#[test]
fn texture_zero_initialised_on_create() { /* ... */ }

#[test]
fn dropped_resource_table_refunds_all_charges() { /* ... */ }

#[test]
fn cpu_fallback_within_one_frame_of_gpu_error() { /* ... */ }

#[test]
fn compute_app_rejected_at_userinteractive_qos() { /* ... */ }

#[test]
fn handles_are_per_logical_device() {
    // App A's handle 7 must not be valid in App B's table.
}

#[test]
fn mapped_slice_validates_linear_memory_bounds() {
    // Crafted out-of-bounds offset must be rejected.
}
```

### 12.3 Benchmarks

`cargo bench --bench gpu_compositor` measures, per scene complexity:

- frames/sec on a 1920×1080 output,
- VRAM in use,
- CPU usage (compositor thread),
- GPU usage (via wgpu's timestamp query if available).

Targets for `desktop-full` (virtio-gpu, Lavapipe baseline):

| Scene | Min FPS | Max VRAM |
|---|---|---|
| Trivial (2 surfaces) | 120 | 4 MiB |
| Moderate (8 surfaces, 2 alpha) | 60 | 24 MiB |
| Heavy (32 surfaces, blur, animation) | 60 | 96 MiB |

### 12.4 Fuzzing

- Fuzz the WGSL parser via naga's existing fuzz target (we just enable it).
- Fuzz the command-buffer cost estimator with property tests:
  cost(empty) = 0; cost(buf + extra_draw) > cost(buf); cost(buf) ≤ MAX_COST.

### 12.5 CI matrix

```
matrix:
  os: [ubuntu-22.04]
  platform: [desktop-full, iot-edge, server-headless, mcu-minimal]
  features: [default, no-gpu]
  exclude:
    - platform: mcu-minimal
      features: default                  # mcu cannot build with gpu
```

---

## 13. Open questions for the Critic

1. **WGSL-only policy.** Should the supervisor also accept naga IR as a
   precompiled cache format? Apps that ship shaders precompiled at install
   time save 8–40 ms per shader, but the IR format is unstable and binds us
   to a specific naga version. *Proposal:* accept naga IR only when the
   format major version matches the supervisor's; reject otherwise and force
   recompile.

2. **Per-QoS Devices vs per-app Devices.** Per-QoS limits descriptor-heap
   pressure but conflates the "fairness" boundary across apps. Is the
   Critic comfortable with this, or do we need per-app Devices with a
   driver-level scheduler? *Proposal:* per-QoS in v1, revisit if Panfrost
   benchmarks reveal head-of-line blocking.

3. **`gpu.compute` quarantining the compositor.** This costs visible FPS
   when compute jobs are running. Should we instead rate-limit compute
   between presents (Apple's approach: 4 ms of compute per 16 ms frame) and
   accept a small timing-channel risk? *Proposal:* full quarantine in v1,
   relax to time-slicing in v2 once we have hardware preemption confidence.

4. **Texture zero-init cost.** 1.5× slower allocation is a lot for shader-
   heavy apps. Should this be opt-out via a `gpu.fast_alloc = true` cap
   that is itself gated behind `gpu.compute`? *Proposal:* opt-out is fine
   if reviewed; default stays on.

5. **DMABuf import.** Should *apps* be allowed to import dmabufs (for video
   playback), or only the compositor? Apps can't currently obtain a fd, so
   the question is whether we add a `vyoma:gpu/dmabuf` interface. *Proposal:*
   defer to v2; video apps use a supervisor-side decoder that hands a
   pre-uploaded Surface.

6. **virtio-gpu blob resources vs Mesa virgl.** We default to virgl in QEMU,
   but blob resources (with the `crosvm`-style mechanism) give zero-copy
   host↔guest memory. Worth the kernel patch? *Proposal:* virgl in v1;
   blob-resource path is a stretch goal documented in the open backlog.

7. **Overlay plane assignment.** Modern macOS uses up to 12 overlay planes;
   our budget of 4 is conservative. Bump to 8? *Proposal:* keep 4; expose
   as profile knob.

8. **GPU on `robotics-rt`.** Some industrial robot controllers (NVIDIA
   Jetson Orin) do have GPUs for ML inference. We disabled GPU on this
   profile for latency reasons. Should there be a `robotics-rt-gpu`
   sub-profile? *Proposal:* yes, but in a later round.

9. **Compositor Device sharing.** The compositor and apps share the
   `UserInteractive` Device. If an app submits a buggy pipeline that takes
   200 ms to execute, it stalls the compositor. Do we need a per-Device
   timeslicer? *Proposal:* enforce via the command-buffer cost model
   (§3.5); a buggy app is rejected at submit time.

10. **Compute on `server-headless`.** Servers often want compute (ML
    inference). Our design allows it but disables the compositor. Should
    we ship a "compute-only" world that doesn't import `present`?
    *Proposal:* yes — `world compute-app` excluded from `webgpu/present`.

---

## 14. Summary

The GPU Acceleration Layer for VyomaOS introduces:

1. A WIT-modeled WebGPU interface (`vyoma:gpu/webgpu`, `present`, `compute`)
   that lets WASM apps program the GPU through the standard `wgpu` API
   without any sandbox escape.
2. A 13-file `supervisor/src/gpu/` module, every file ≤ 500 LOC, structured
   into registry, devices, resource table, WIT host, compositor, present,
   VRAM budget, shader cache, cost model, backend, and three shaders.
3. A two-mode compositor — R11 software path remains canonical; a new GPU
   path activates when scene complexity warrants it and gracefully falls
   back on any error within one frame.
4. A per-QoS VRAM budget with O(1) charge/refund and a four-state texture
   lifecycle (Hot, Warm, Cached, Evicted) for eviction order.
5. A platform support matrix that disables GPU entirely on `mcu-minimal`
   and `robotics-rt`, uses Panfrost on ARM SBCs, virtio-gpu in QEMU, and
   real Vulkan when present.
6. A naga-based WGSL→SPIR-V pipeline with rate-limited compile quotas, a
   disk cache under `/data/shaders/`, and install-time precompilation via
   the package manager.
7. A new `[capabilities.gpu]` struct in `vyoma.toml` splitting graphics
   from compute, plus shader precompile hints in `[shaders]`.
8. A security model that zero-initialises all newly visible GPU memory,
   quarantines compute apps from `UserInteractive` presents, and gates
   raw SPIR-V passthrough behind a manifest opt-in.
9. Reuse of every prior round: R2 SharedBuffer extends with
   `HostCoherent`, R5 QoS scopes Devices, R6 registers `GpuHal`, R7 power-
   manages devices, R9 owns the trait, R10 drives vsync, R11's compositor
   gains a fast lane.
10. Explicit v1-skips: no Metal Performance Shaders, no ray tracing, no
    Heap/argument buffers, no multi-GPU, no eGPU, no headless-compute
    server, no app dmabuf import. These are documented and deferred.

The next round of debate may now focus on networking, audio, or the I/O
scheduler — the GPU surface area is contained, exhaustively typed, and
sand-boxed by construction.

---
