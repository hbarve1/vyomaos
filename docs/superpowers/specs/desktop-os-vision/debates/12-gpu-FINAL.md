# Round 12 — GPU Acceleration Layer — FINAL

**macOS equivalent:** Metal / IOAcceleratorFamily
**Status:** FINAL (synthesis)
**Depends on:** R2 (SharedBuffer), R5 (QoS), R6 (DeviceManager), R9 (HAL), R10 (CallbackQueue), R11 (Compositor)
**Provides for:** R14 (Image pipeline), R16 (Animation), R17 (VYOMA_DRAW v3), R20 (HiDPI), R67 (audio mix), R68 (video decode)

---

## 1. Design Pillars

1. **WASM binaries never link wgpu.** GPU calls cross the sandbox boundary as WIT hostcalls. The supervisor owns the only wgpu instances on the system. This preserves the capability-secure invariant: an app with no `gpu` capability has no GPU hostcalls imported, therefore no GPU access at all.
2. **Per-app `wgpu::Device` isolation.** Each app with `gpu_level >= 2` gets its own `wgpu::Device` + `wgpu::Queue`. The compositor's `Device` is separate and is **never** handed to an app. A GPU hang in app A cannot freeze app B or the compositor.
3. **Compositor never blocks on the GPU.** A dedicated `FencePoller` thread tracks submitted frames. If the GPU frame isn't ready by vsync, the compositor falls back to the CPU pass for that frame (already canonical from R11). The compositor SCHED_FIFO thread NEVER calls a blocking GPU fence wait.
4. **Three capability levels** (Level 0 / 1 / 2) selected per platform profile. The MCU profile has no GPU code path at all; the embedded profile only allows compositor-side GPU; only desktop-full/native-mobile expose the per-app GPU.
5. **Defense in depth against GPU DoS.** Pre-validation of shaders, per-app submission quotas, GPU watchdog with Device destroy on hang, VRAM budget with eviction policy.
6. **Staging copies in v1.** All GPU ↔ WASM data traffic copies through staging buffers (host RAM → wgpu staging → GPU). Zero-copy (`HostCoherent`, `DmaBuf`) is explicitly deferred to v2.

The eight Fix resolutions (B1–B8) below address concerns raised in critique.

---

## 2. Layer Diagram

```
┌────────────────────────────────────────────────────────────────────┐
│  WASM app (wasm32-wasip2)                                          │
│    imports vyoma:gpu/graphics  (handles are u32, never raw ptrs)   │
│    imports vyoma:gpu/compute                                       │
└──────────────────────────────┬─────────────────────────────────────┘
                               │ WIT hostcall (Wasmtime trampoline)
┌──────────────────────────────▼─────────────────────────────────────┐
│  Supervisor: GpuRegistry (single struct, owned by main)            │
│    ├── AppGpuContext per iid {device, queue, resource_table,       │
│    │       vram_used, submission_count}                            │
│    ├── CompositorDevice (separate Device + Queue, never lent)      │
│    ├── ShaderCompiler (worker thread, naga, disk cache)            │
│    ├── FencePoller    (NORMAL prio thread, polls SubmissionIndex)  │
│    ├── VramBudget     (atomic accounting + eviction policy)        │
│    └── Platform       {backend, level} chosen at boot              │
└──────────────────────────────┬─────────────────────────────────────┘
                               │ wgpu-hal
┌──────────────────────────────▼─────────────────────────────────────┐
│  wgpu-hal → Vulkan / GLES / Software                               │
│    Backend = VulkanNative | VulkanVirgl | Gles3Virgl | Lavapipe |  │
│              Software                                              │
└──────────────────────────────┬─────────────────────────────────────┘
                               │
                       /dev/dri/{card0, renderD128}   (DRM/KMS)
```

---

## 3. Fix B1 — WIT hostcalls, never wgpu inside WASM

A WASM-linked wgpu would balloon binary size, exfiltrate driver pointers, and bypass the supervisor's quota/watchdog/eviction logic. Instead, the WASM app sees an **opaque handle ABI** mediated by Wasmtime hostcalls. All GPU resources are addressed by `u32` IDs scoped per app; an app can only refer to handles in its own `ResourceTable`.

### 3.1 WIT package

```wit
package vyoma:gpu@1.0.0;

interface graphics {
  record render-pipeline-desc {
    vertex-shader: u32,
    fragment-shader: u32,
    color-format: u8,            // 0=Bgra8 1=Rgba8 2=Rgb565
  }
  record render-pass-desc {
    color-attachments: list<u32>,
    clear-color: tuple<f32, f32, f32, f32>,
  }

  // Resource creation — returns opaque u32 handle, scoped to the calling iid.
  create-buffer:          func(size: u32, usage: u32)        -> result<u32, string>;
  create-texture:         func(w: u32, h: u32, fmt: u8)      -> result<u32, string>;
  create-shader:          func(wgsl: string)                  -> result<u32, string>;
  create-render-pipeline: func(desc: render-pipeline-desc)    -> result<u32, string>;

  // Data path — always staging copy in v1.
  write-buffer:  func(handle: u32, offset: u32, data: list<u8>)              -> result<_, string>;
  read-buffer:   func(handle: u32, offset: u32, len: u32)                    -> result<list<u8>, string>;

  // Command recording.
  begin-frame:       func()                                         -> result<u32, string>;
  render-pass:       func(encoder: u32, desc: render-pass-desc)     -> result<u32, string>;
  draw:              func(pass: u32, vertices: u32, instances: u32);
  end-render-pass:   func(pass: u32);
  submit:            func(encoder: u32)                             -> result<_, string>;
  present:           func()                                         -> result<_, string>;

  destroy-resource:  func(handle: u32);
}

interface compute {
  create-compute-pipeline: func(shader: u32, entry: string)         -> result<u32, string>;
  dispatch:                func(encoder: u32, x: u32, y: u32, z: u32);
}

world gpu-app {
  import graphics;
  import compute;
}
```

### 3.2 Binding policy

- A WASM app is allowed to import `vyoma:gpu/graphics` **only if** its manifest declares `gpu = true` (capability gate). The hostcalls aren't present in the import set otherwise.
- Compute is gated by an additional `gpu_compute = true`; on platforms with `GpuLevel::CompositorOnly`, the supervisor links a stub that returns `error("gpu not available")` for every call.
- Handles returned to WASM are `u32` allocated from a per-app monotonically increasing counter. Re-use is allowed after `destroy-resource` to keep the space small (16-bit generation prefix planned for v2 if exhaustion becomes a concern).

### 3.3 Manifest extension

```toml
# apps/my-gpu-app/vyoma.toml
[capabilities]
gpu = true              # imports vyoma:gpu/graphics
gpu_compute = false     # leave compute off if not needed
gpu_vram_kb = 16384     # voluntary cap; supervisor enforces hard cap from VramBudget
```

---

## 4. Fix B2 — Per-app `wgpu::Device` isolation

The supervisor maintains one `AppGpuContext` per iid and one `CompositorDevice`. The compositor `Device` is **never** referenced from app contexts, and app `Device`s are **never** shared with each other.

### 4.1 Types

```rust
// supervisor/src/gpu/device.rs

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64};

pub struct AppGpuContext {
    pub iid: IidKey,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub resources: ResourceTable,
    pub vram_used: AtomicU64,
    pub submission_count: AtomicU32, // reset each second by SecurityTimer
}

pub struct ResourceTable {
    pub buffers:   HashMap<u32, wgpu::Buffer>,
    pub textures:  HashMap<u32, wgpu::Texture>,
    pub shaders:   HashMap<u32, wgpu::ShaderModule>,
    pub pipelines: HashMap<u32, wgpu::RenderPipeline>,
    pub encoders:  HashMap<u32, wgpu::CommandEncoder>,
    pub next_id: u32,
}

impl ResourceTable {
    pub fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1); // 0 reserved
        id
    }
}
```

### 4.2 GpuRegistry

```rust
// supervisor/src/gpu/mod.rs

pub struct GpuRegistry {
    pub compositor_device: wgpu::Device,
    pub compositor_queue:  wgpu::Queue,
    pub apps:    Mutex<HashMap<IidKey, AppGpuContext>>,
    pub backend: GpuBackend,
    pub level:   GpuLevel,
    pub vram_budget: VramBudget,
    pub shader_compiler: ShaderCompiler,
    pub fence_poller: Arc<FencePoller>,
}

impl GpuRegistry {
    pub fn create_for_app(&self, iid: IidKey) -> Result<(), GpuError> {
        let mut apps = self.apps.lock().unwrap();
        if apps.len() >= MAX_GPU_APPS {                    // 4 on desktop-full
            return Err(GpuError::TooManyGpuApps);
        }
        let (device, queue) = self.new_app_device()?;     // separate VkDevice per app
        apps.insert(iid, AppGpuContext::new(iid, device, queue));
        Ok(())
    }

    pub fn destroy_for_app(&self, iid: IidKey) {
        if let Some(ctx) = self.apps.lock().unwrap().remove(&iid) {
            self.vram_budget.refund(ctx.vram_used.load(Ordering::SeqCst));
            // wgpu::Device drop triggers full resource cleanup.
        }
    }
}

const MAX_GPU_APPS: usize = 4; // desktop-full
```

### 4.3 Invariants

- `apps` is a `Mutex<HashMap>` only at registry granularity. Per-app contexts contain interior `wgpu::Device` (already thread-safe) and lock-free atomics; no per-app mutex is held across hostcalls.
- The compositor reads its own `compositor_device`/`compositor_queue` fields without ever touching the apps map.
- If a `wgpu::Device` is destroyed (watchdog, app exit), every `Buffer`/`Texture` derived from it becomes invalid; subsequent app submissions trap to the app via `error("device lost")`.
- App devices are created with `wgpu::Features::empty()` plus an allowlist (e.g. no `MAPPABLE_PRIMARY_BUFFERS` — host pointer leak risk).

---

## 5. Fix B3 — Submission quotas + GPU watchdog

A malicious app can attempt to queue billions of draw calls per second or write an infinite-loop fragment shader. Two layered defenses run before/around submit:

### 5.1 Submission rate limit

```rust
// supervisor/src/gpu/security.rs
pub const MAX_SUBMISSIONS_PER_SEC: u32 = 120;
pub const GPU_WATCHDOG_MS: u64 = 500;

pub fn check_submission(ctx: &AppGpuContext) -> Result<(), GpuError> {
    let count = ctx.submission_count.fetch_add(1, Ordering::Relaxed);
    if count >= MAX_SUBMISSIONS_PER_SEC {
        return Err(GpuError::SubmissionRateExceeded);
    }
    Ok(())
}

// A 1-Hz timer resets every app's submission_count → 0. Implemented as a
// CallbackQueue timer on the SecurityTimer slot (R10).
```

### 5.2 GPU watchdog

```rust
pub fn submit_with_watchdog(
    ctx: &AppGpuContext,
    encoder: wgpu::CommandBuffer,
    registry: Arc<GpuRegistry>,
    iid: IidKey,
) -> Result<wgpu::SubmissionIndex, GpuError> {
    check_submission(ctx)?;
    let idx = ctx.queue.submit(std::iter::once(encoder));

    // Hand to FencePoller; if poller doesn't see the fence signal within
    // GPU_WATCHDOG_MS, it tears the Device down.
    registry.fence_poller.watch(iid, idx, Duration::from_millis(GPU_WATCHDOG_MS));
    Ok(idx)
}

// In FencePoller (Fix B8): if deadline passes without signal,
// → log GpuWatchdogTriggered{iid, idx}
// → GpuRegistry::destroy_for_app(iid)        (drops VkDevice; aborts inflight commands)
// → notify app via PendingMessage::DeviceLost so its WIT hostcalls start returning errors
// → app may exit voluntarily (most will); supervisor does NOT kill the process
//   because the WASM logic may still be useful (e.g. CPU fallback rendering).
```

### 5.3 Side benefits

- `destroy-resource` is reference-counted so resources outliving the watchdog destroy gracefully.
- The same watchdog catches benign cases (compute shader with bad loop bound) without crashing the kernel module.

---

## 6. Fix B4 — Shader pre-validation + dedicated worker thread

Naga validation can take seconds on adversarial WGSL (deep AST nesting, monstrous identifiers). It must not run on the WIT thread — that thread is rate-limited and shared by other hostcalls. A dedicated worker compiles, validates, and caches.

### 6.1 Limits

```rust
// supervisor/src/gpu/shader_compiler.rs
pub const MAX_SHADER_BYTES: usize = 65_536;
pub const MAX_AST_NODES:    usize = 100_000;
pub const COMPILE_TIMEOUT_MS: u64 = 2_000;
```

### 6.2 Worker protocol

```rust
use crossbeam::channel::{Sender, Receiver};
use tokio::sync::oneshot;

pub struct ShaderCompiler {
    pub tx: Sender<CompileRequest>,
}

pub struct CompileRequest {
    pub iid:  IidKey,
    pub wgsl: String,
    pub reply: oneshot::Sender<Result<CompiledShader, String>>,
}

pub struct CompiledShader {
    pub spirv: Vec<u8>,
    pub sha256: [u8; 32],
    pub module: wgpu::ShaderModule, // baked against the requesting app's Device
}

impl ShaderCompiler {
    pub fn spawn(cache_dir: PathBuf) -> Self {
        let (tx, rx) = crossbeam::channel::bounded::<CompileRequest>(32);
        std::thread::Builder::new()
            .name("vyoma-shader-compile".into())
            .spawn(move || Self::worker_loop(rx, cache_dir))
            .expect("spawn shader worker");
        ShaderCompiler { tx }
    }

    fn worker_loop(rx: Receiver<CompileRequest>, cache_dir: PathBuf) {
        while let Ok(req) = rx.recv() {
            let res = Self::compile_one(&req, &cache_dir);
            let _ = req.reply.send(res);
        }
    }

    fn compile_one(req: &CompileRequest, cache: &Path) -> Result<CompiledShader, String> {
        // 1) Size + AST-node prevalidation (cheap, ms-level).
        if req.wgsl.len() > MAX_SHADER_BYTES {
            return Err(format!("shader exceeds {} bytes", MAX_SHADER_BYTES));
        }
        let ast = naga::front::wgsl::parse_str(&req.wgsl).map_err(|e| e.to_string())?;
        if ast.expressions.len() + ast.functions.len() * 8 > MAX_AST_NODES {
            return Err("AST node count exceeds limit".into());
        }

        // 2) Cache lookup by SHA-256 of WGSL.
        let sha = sha256_bytes(req.wgsl.as_bytes());
        let cache_path = cache.join(format!("{}/{}.spv", hex(req.iid.0), hex(sha)));
        if let Ok(spirv) = std::fs::read(&cache_path) {
            return Ok(CompiledShader { spirv, sha256: sha, module: bake_module(&spirv) });
        }

        // 3) Full naga validation + SPIR-V emission (with timeout).
        let spirv = compile_to_spirv_with_timeout(&ast, COMPILE_TIMEOUT_MS)?;
        write_cache_atomic(&cache_path, &spirv)?;
        write_meta(&cache_path.with_extension("meta"), &sha)?;
        Ok(CompiledShader { spirv: spirv.clone(), sha256: sha, module: bake_module(&spirv) })
    }
}
```

### 6.3 WIT integration

`create-shader(wgsl)` enqueues a `CompileRequest`, **awaits** the `oneshot::Receiver` with a 2 s ceiling, then registers the result in `ResourceTable.shaders`. The await is on the WIT call's host future (R10 CallbackQueue), not a blocking thread call — the app is `Yielded` while waiting and other hostcalls keep flowing.

### 6.4 Disk cache layout

```
/data/shaders/<iid-hex>/<sha256>.spv     # SPIR-V binary
/data/shaders/<iid-hex>/<sha256>.meta    # {wgsl_sha, naga_version, ts}
/data/shaders/cache_version              # bumped on supervisor upgrade
```

When supervisor boots and detects `cache_version` mismatch, it `unlink`s the whole `/data/shaders/` subtree. Pre-installed apps may ship a precompiled `.spv` in their bundle; the supervisor will accept it if the `.meta` matches a known-good naga version.

---

## 7. Fix B5 — Backend matrix + platform assignment

### 7.1 Types

```rust
// supervisor/src/gpu/platform.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBackend {
    VulkanNative,  // real GPU on native hardware
    VulkanVirgl,   // QEMU virtio-gpu + virgl (host Vulkan)
    Gles3Virgl,    // QEMU virtio-gpu + GLES3 (host OpenGL)
    Lavapipe,      // Mesa software Vulkan — QEMU dev default
    Software,      // CPU only; no wgpu::Device created
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuLevel {
    None            = 0, // mcu-minimal / robotics-rt / server-headless default
    CompositorOnly  = 1, // iot-edge / mobile v1
    Full            = 2, // desktop-full / native-mobile
}
```

### 7.2 V1 platform assignments

| Platform        | GpuLevel        | Default backend | Notes                                       |
|-----------------|-----------------|-----------------|---------------------------------------------|
| `desktop-full`  | `Full`          | `Lavapipe`      | CI gate 30 fps @ 1280×800                  |
| `mobile`        | `CompositorOnly`| `VulkanVirgl`   | Compositor + screen blit only; apps L0 calls return errors |
| `iot-edge`      | `CompositorOnly`| `Gles3Virgl`    | If KMS plane available                      |
| `robotics-rt`   | `None`          | `Software`      | Headless; framebuffer text only             |
| `server-headless`| `None`         | `Software`      | No display device at all                   |
| `mcu-minimal`   | `None`          | `Software`      | No wgpu crate compiled in                   |

Profile TOML field that controls this:

```toml
# supervisor/src/profile/profiles/desktop-full.toml
[display]
gpu_level = 2
gpu_backend = "lavapipe"   # override at boot via VYOMA_GPU_BACKEND=vulkan
```

### 7.3 Detection

```rust
pub fn detect_backend(env_override: Option<&str>) -> GpuBackend {
    if let Some(s) = env_override { return parse_backend(s); }
    if Path::new("/dev/dri/renderD128").exists() {
        if probe_vulkan_native() { return GpuBackend::VulkanNative; }
        if probe_virgl()        { return GpuBackend::VulkanVirgl;  }
        if probe_gles3()        { return GpuBackend::Gles3Virgl;   }
    }
    if probe_lavapipe() { return GpuBackend::Lavapipe; }
    GpuBackend::Software
}
```

`detect_backend` runs once at boot inside `GpuRegistry::init`. The choice is logged via the heartbeat emitter (R8/observability).

---

## 8. Fix B6 — Staging-buffer data path (no zero-copy v1)

Zero-copy paths require either Vulkan host-coherent memory exposed to WASM (security review) or `dma-buf` plumbing through the kernel (currently absent in our stripped allnoconfig kernel). v1 takes the simple, secure path: every GPU↔WASM byte goes through a staging buffer.

### 8.1 Write path (CPU → GPU)

```rust
pub fn host_write_buffer(
    ctx: &mut AppGpuContext,
    handle: u32,
    offset: u32,
    data: &[u8],
) -> Result<(), GpuError> {
    let buf = ctx.resources.buffers.get(&handle).ok_or(GpuError::BadHandle)?;
    if (offset as usize) + data.len() > buf.size() as usize {
        return Err(GpuError::OutOfBounds);
    }
    // queue.write_buffer internally allocates a staging slice and inserts a
    // copy command on the next submit. Single host copy. Bounded by the
    // size limit enforced at create-buffer time.
    ctx.queue.write_buffer(buf, offset as u64, data);
    Ok(())
}
```

### 8.2 Texture write path

```rust
pub fn host_write_texture(
    ctx: &mut AppGpuContext,
    handle: u32,
    data: &[u8],
    bytes_per_row: u32,
    extent: wgpu::Extent3d,
) -> Result<(), GpuError> {
    let tex = ctx.resources.textures.get(&handle).ok_or(GpuError::BadHandle)?;
    ctx.queue.write_texture(
        tex.as_image_copy(),
        data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(extent.height),
        },
        extent,
    );
    Ok(())
}
```

### 8.3 Read path (GPU → CPU)

```rust
pub fn host_read_buffer(
    ctx: &mut AppGpuContext,
    handle: u32,
    offset: u32,
    len: u32,
) -> Result<Vec<u8>, GpuError> {
    let buf = ctx.resources.buffers.get(&handle).ok_or(GpuError::BadHandle)?;
    let slice = buf.slice(offset as u64 .. (offset + len) as u64);

    // Async map; we drive the wgpu poll loop from the FencePoller.
    let (tx, rx) = oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| { let _ = tx.send(res); });
    // The hostcall yields the iid; FencePoller wakes it when ready.
    let _ = futures::executor::block_on(rx).map_err(|_| GpuError::DeviceLost)??;

    let view = slice.get_mapped_range();
    let out  = view.to_vec();   // one copy into WASM-bound Vec<u8>
    drop(view);
    buf.unmap();
    Ok(out)
}
```

### 8.4 Deferred to v2

```rust
pub enum SharedBuffer {
    Cpu(Vec<u8>),              // v1 — already shipped in R2
    HostCoherent(*mut u8),     // v2 — requires hardened mmap policy
    DmaBuf(RawFd),             // v2 — requires kernel CONFIG_DMA_BUF
}
```

`HostCoherent` and `DmaBuf` are explicitly **not** wired through any v1 hostcall.

---

## 9. Fix B7 — Three GPU capability levels

### 9.1 Per-level behaviour

| Level | Compositor uses GPU? | Apps see WIT GPU hostcalls? | Apps see compute? |
|-------|----------------------|-----------------------------|--------------------|
| 0     | No (CPU only)        | No                          | No                 |
| 1     | Yes                  | Yes — but every call returns `error("gpu not available")` | No |
| 2     | Yes                  | Yes — real per-app `Device` | Yes if `gpu_compute=true` |

Rationale for Level 1's stub-error mode: app code can be written once against the WIT and gracefully fall back at run time (e.g. switch to CPU rasterizer). That's exactly the macOS pattern when a Metal device returns nil.

### 9.2 Compile gate

The `gpu` module is compiled unconditionally (it's tiny), but the Wasmtime import surface differs:

```rust
// supervisor/src/gpu/wit_handlers.rs
pub fn add_to_linker(
    linker: &mut wasmtime::component::Linker<HostState>,
    level: GpuLevel,
    manifest_has_gpu: bool,
) -> wasmtime::Result<()> {
    if !manifest_has_gpu { return Ok(()); }   // capability gate

    match level {
        GpuLevel::None => Ok(()),             // hostcalls absent; app fails to instantiate
        GpuLevel::CompositorOnly => add_stub_handlers(linker),
        GpuLevel::Full           => add_full_handlers(linker),
    }
}
```

If `manifest_has_gpu=true` but `level==None`, instantiation fails with a clear error: "platform does not provide vyoma:gpu". That keeps the same WASM binary deployable across platforms without runtime branching.

---

## 10. Fix B8 — Non-blocking compositor ↔ GPU coupling

The compositor thread runs at SCHED_FIFO 40 (R11). It must NEVER call `device.poll(Maintain::Wait)` or wait on a `SubmissionIndex` — that would surrender the scheduling priority and risk a vsync miss. Instead, a separate `FencePoller` thread tracks all in-flight frames; the compositor only ever does `try_recv()`.

### 10.1 Types

```rust
// supervisor/src/gpu/fence_poller.rs

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub struct FencePoller {
    pending: Mutex<Vec<PendingFrame>>,
    condvar: Condvar,
    registry: Weak<GpuRegistry>,
}

pub struct PendingFrame {
    pub iid:  IidKey,
    pub fence: wgpu::SubmissionIndex,
    pub reply: oneshot::Sender<FrameResult>,
    pub deadline: Instant,
    pub purpose: FencePurpose,
}

pub enum FencePurpose {
    CompositorFrame,         // poll signals "ready to scan out"
    AppSubmission,           // poll signals "device still alive"
    BufferMap,               // poll wakes the read-buffer hostcall
}

pub enum FrameResult {
    Ready,
    Timeout,
    DeviceLost,
}
```

### 10.2 Poller loop

```rust
impl FencePoller {
    pub fn run(self: Arc<Self>) {
        // NORMAL priority thread (R10), poll cadence = 1 ms.
        loop {
            let now = Instant::now();
            let mut still_pending = Vec::new();
            for frame in self.take_pending() {
                if let Some(reg) = self.registry.upgrade() {
                    if reg.is_signaled(&frame) {
                        let _ = frame.reply.send(FrameResult::Ready);
                        continue;
                    }
                }
                if now >= frame.deadline {
                    // Watchdog: destroy this Device.
                    if let Some(reg) = self.registry.upgrade() {
                        if matches!(frame.purpose, FencePurpose::AppSubmission) {
                            reg.destroy_for_app(frame.iid);
                        }
                    }
                    let _ = frame.reply.send(FrameResult::Timeout);
                    continue;
                }
                still_pending.push(frame);
            }
            self.put_pending(still_pending);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn watch(&self, iid: IidKey, idx: wgpu::SubmissionIndex, timeout: Duration) -> oneshot::Receiver<FrameResult> {
        let (tx, rx) = oneshot::channel();
        let mut q = self.pending.lock().unwrap();
        q.push(PendingFrame {
            iid, fence: idx, reply: tx,
            deadline: Instant::now() + timeout,
            purpose: FencePurpose::AppSubmission,
        });
        self.condvar.notify_one();
        rx
    }
}
```

### 10.3 Compositor integration

```rust
// supervisor/src/display/compositor.rs (additions)

fn gpu_compositor_pass(state: &mut CompositorState, registry: &GpuRegistry) -> CompositorOutcome {
    // 1. Drain ready GPU frames non-blockingly.
    if let Ok(FrameResult::Ready) = state.last_gpu_frame.try_recv() {
        gpu_blit_to_framebuffer(state, registry);
        return CompositorOutcome::GpuPresented;
    }

    // 2. Optimistically submit next frame.
    let rx = submit_next_gpu_frame(state, registry);
    state.last_gpu_frame = rx;

    // 3. This vsync still owes a scan-out — use CPU path for THIS frame.
    cpu_compositor_pass(state)
}

pub enum CompositorOutcome {
    GpuPresented,
    CpuPresented(FallbackReason),
}

pub enum FallbackReason {
    GpuNotReady,
    GpuLevelTooLow,
    SceneClassifiedTrivial,
    GpuDeviceLost,
}
```

The CPU pass is canonical (R11) — the GPU is opt-in acceleration, never the only path to pixels.

---

## 11. GPU compositor pass + scene classification

```rust
// supervisor/src/gpu/compositor_gpu.rs

pub enum SceneComplexity {
    Trivial,
    Moderate,
    Heavy,
}

pub fn classify_scene(registry: &WindowRegistry) -> SceneComplexity {
    let window_count  = registry.visible_count();
    let alpha_windows = registry.alpha_composited_count();
    match (window_count, alpha_windows) {
        (0..=2, 0)         => SceneComplexity::Trivial,    // CPU is faster
        (3..=6, 0..=2)     => SceneComplexity::Moderate,   // GPU if available
        _                   => SceneComplexity::Heavy,      // GPU required
    }
}

pub fn choose_pass(level: GpuLevel, complexity: SceneComplexity) -> PassChoice {
    use {GpuLevel::*, SceneComplexity::*};
    match (level, complexity) {
        (None, _)                            => PassChoice::Cpu,
        (_, Trivial)                          => PassChoice::Cpu,
        (CompositorOnly | Full, Moderate)    => PassChoice::Gpu,
        (CompositorOnly | Full, Heavy)       => PassChoice::Gpu,
    }
}
```

The GPU pass renders each window surface as a textured quad with the per-window damage rect as the scissor. CPU-resident `Surface` buffers (from R11) are uploaded to a single shared `wgpu::Texture` per app (`SurfaceTexture`), refreshed only when the per-window damage flag is set.

```rust
pub struct SurfaceTexture {
    pub iid: IidKey,
    pub texture: wgpu::Texture,
    pub state: SurfaceTextureState,
    pub last_used_frame: u64,
    pub bytes: u64,
}

pub enum SurfaceTextureState {
    Hot,      // used this frame
    Warm,     // used within last 4 frames
    Cached,   // not visible but kept resident
    Evicted,  // CPU-only copy retained
}
```

---

## 12. VRAM management

```rust
// supervisor/src/gpu/vram.rs

pub struct VramBudget {
    pub total: u64,
    pub compositor_reserved: u64,   // 20% of total, never lent
    pub app_pool: AtomicU64,         // remaining; shared across apps
}

#[derive(Debug)]
pub enum VramError {
    OutOfMemory,
    BudgetExceeded { requested: u64, available: u64 },
}

pub struct VramGuard {
    bytes: u64,
    pool: Arc<AtomicU64>,
}

impl Drop for VramGuard {
    fn drop(&mut self) {
        self.pool.fetch_add(self.bytes, Ordering::SeqCst);
    }
}

impl VramBudget {
    pub fn try_charge(&self, bytes: u64) -> Result<VramGuard, VramError> {
        let mut cur = self.app_pool.load(Ordering::SeqCst);
        loop {
            if cur < bytes { return Err(VramError::OutOfMemory); }
            match self.app_pool.compare_exchange(
                cur, cur - bytes, Ordering::SeqCst, Ordering::SeqCst,
            ) {
                Ok(_)   => return Ok(VramGuard { bytes, pool: self.app_pool.clone() }),
                Err(c) => cur = c,
            }
        }
    }
}
```

### 12.1 Eviction policy

Charge order on OOM:

1. Drop all `Evicted` slots (already CPU-only, instant).
2. Demote `Cached` → `Evicted` until `bytes` are free.
3. Demote `Warm` → `Cached`, then evict.
4. Demote `Hot` → `Warm` only after a 1-frame grace period (so we don't flicker the window the user is interacting with).
5. If still OOM, **the compositor falls back to CPU for this single frame** (`FallbackReason::GpuDeviceLost` is not used here; we use `GpuNotReady`). The user sees no glitch because the CPU path is bit-for-bit equivalent.

### 12.2 Defaults

```rust
pub fn default_budget(backend: GpuBackend) -> VramBudget {
    let total = match backend {
        GpuBackend::VulkanNative   => 1024 * 1024 * 1024, // 1 GiB
        GpuBackend::VulkanVirgl    => 256  * 1024 * 1024,
        GpuBackend::Gles3Virgl     => 128  * 1024 * 1024,
        GpuBackend::Lavapipe       => 256  * 1024 * 1024,
        GpuBackend::Software       => 0,
    };
    VramBudget {
        total,
        compositor_reserved: total / 5,
        app_pool: AtomicU64::new(total - total / 5),
    }
}
```

---

## 13. Texture zero-init (security)

Newly created `wgpu::Texture` must be zero-initialised before the app can sample it, otherwise driver garbage (potentially containing pixels from another tenant) becomes visible. We use `wgpu::Features::CLEAR_TEXTURE` when available; otherwise issue a 1-call `queue.write_texture` of a zero slab. This runs inside `create-texture` before the handle is returned to the app.

```rust
pub fn host_create_texture(ctx: &mut AppGpuContext, w: u32, h: u32, fmt: u8)
    -> Result<u32, GpuError>
{
    let _g = ctx.vram_charge((w * h * 4) as u64)?;   // hard budget check
    let texture = ctx.device.create_texture(&desc(w, h, fmt));
    zero_initialise(&ctx.queue, &texture);            // mandatory
    let id = ctx.resources.alloc_id();
    ctx.resources.textures.insert(id, texture);
    Ok(id)
}
```

The same rule applies to buffers (`COPY_DST + MAP_WRITE`-creatable ones).

---

## 14. HAL integration (R9)

```rust
// supervisor/src/gpu/hal_gpu.rs

pub trait HalGpuDevice: Send + Sync {
    fn name(&self) -> &str;
    fn backend(&self) -> GpuBackend;
    fn open(&self) -> Result<wgpu::Adapter, HalError>;
}

pub struct GpuHal {
    pub primary: Arc<dyn HalGpuDevice>,
    pub backends_available: Vec<GpuBackend>,
}

// Registered in R6 DeviceManager under DeviceClass::Gpu at boot.
// HAL ownership: GpuRegistry borrows a HalGpuDevice via DeviceManager::lookup.
```

`HalGpuDevice::open()` returns the `wgpu::Adapter` from which the supervisor requests the compositor `Device` and (lazily) each app `Device`. Different backends supply different concrete adapters; the registry is backend-agnostic above this trait.

---

## 15. Prior-round integration recap

| Round | Component                | Integration                                                                 |
|-------|--------------------------|------------------------------------------------------------------------------|
| R2    | `SharedBuffer`           | v1 path is `SharedBuffer::Cpu` only; `HostCoherent`/`DmaBuf` deferred to v2. |
| R5    | QoS classes              | `CompositorDevice` runs at `UserInteractive`; each app `Device` runs at the QoS declared in the app manifest. |
| R6    | `DeviceManager`          | Registers `GpuHal` for each detected GPU under `DeviceClass::Gpu`.           |
| R9    | HAL                      | `HalGpuDevice::open()` is the canonical adapter source.                      |
| R10   | CallbackQueue            | `FencePoller` thread is NORMAL priority; map-async waits ride the IID's HPQ as `PendingMessage::GpuReady`. |
| R11   | Compositor               | `compositor.rs` gains `gpu_compositor_pass()` fast lane. `cpu_compositor_pass()` remains canonical.  |
| R8    | Boot / Observability     | Boot phase `GpuInit` after `DisplayInit`; heartbeat emits `{gpu_backend, gpu_level, vram_used}` every 5 s. |

---

## 16. Error taxonomy

```rust
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("gpu disabled on this platform")]                NotAvailable,
    #[error("too many gpu apps")]                            TooManyGpuApps,
    #[error("vram exhausted: {0:?}")]                        Vram(#[from] VramError),
    #[error("device lost")]                                  DeviceLost,
    #[error("submission rate exceeded")]                     SubmissionRateExceeded,
    #[error("shader: {0}")]                                  Shader(String),
    #[error("bad handle")]                                   BadHandle,
    #[error("out of bounds")]                                OutOfBounds,
    #[error("compile timeout")]                              CompileTimeout,
    #[error("watchdog triggered, device destroyed")]         WatchdogTriggered,
}

impl From<GpuError> for String {
    fn from(e: GpuError) -> Self { e.to_string() }  // for WIT result<_, string>
}
```

---

## 17. File layout (under `supervisor/src/gpu/`)

All files <500 LOC per project rule:

| File                    | Purpose                                                                              |
|-------------------------|--------------------------------------------------------------------------------------|
| `mod.rs`                | re-exports; `GpuRegistry` struct and lifecycle                                      |
| `device.rs`             | `AppGpuContext`, per-app device lifecycle, `ResourceTable`                          |
| `compositor_gpu.rs`     | `gpu_compositor_pass`, `SceneComplexity`, `choose_pass`                             |
| `wit_handlers.rs`       | WIT import wiring for `vyoma:gpu/graphics` + `vyoma:gpu/compute`                    |
| `resource_table.rs`     | per-app buffer/texture/shader/pipeline/encoder tables                               |
| `vram.rs`               | `VramBudget`, `SurfaceTextureState`, eviction policy                                 |
| `shader_compiler.rs`    | worker thread, naga pre-validation, disk cache, `CompiledShader`                    |
| `fence_poller.rs`       | `FencePoller`, `PendingFrame`, async GPU timeline                                    |
| `drm_event.rs`          | DRM atomic non-blocking commits + KMS page-flip event handling                      |
| `platform.rs`           | `GpuBackend`, `GpuLevel`, boot-time detection, profile parsing                      |
| `security.rs`           | submission quota, GPU watchdog, texture zero-init                                    |
| `hal_gpu.rs`            | `GpuHal` + `HalGpuDevice` trait; R9 registration                                     |

---

## 18. Test plan

Unit (`supervisor/tests/gpu_*.rs`):

- `gpu_capability_gate`: app without `gpu = true` cannot import `vyoma:gpu/graphics`.
- `gpu_per_app_isolation`: destroying app A's device leaves app B unaffected.
- `gpu_watchdog`: forge a fence that never signals; assert `destroy_for_app` runs ≤500 ms.
- `gpu_submission_quota`: 121 submits in 1 s → `SubmissionRateExceeded`.
- `gpu_shader_oversize`: 70 KB WGSL rejected without entering naga.
- `gpu_shader_cache`: same WGSL twice → second hit returns from disk in <1 ms.
- `gpu_vram_budget`: charge until `OutOfMemory`; refund on `VramGuard::drop`.
- `gpu_zero_init`: read-back of brand-new texture must be all-zero.
- `gpu_scene_classifier`: 2 windows / 0 alpha → `Trivial`; 7 windows → `Heavy`.
- `gpu_level_stub`: `Level::CompositorOnly` app gets `error("gpu not available")`.

Integration (`make gpu-bench`):

- Headless QEMU + Lavapipe; 1280×800 window with animated quad; CI gate ≥30 fps for 10 s.
- Watchdog smoke: launch app that runs `while(true);` in fragment shader; supervisor must remain responsive and surface watchdog log within 500 ms.

---

## 19. V1 / Deferred / Never

**Critical v1:** WIT interface, per-app `wgpu::Device`, shader worker + disk cache, GPU watchdog (B3), CPU compositor fallback (B8), VRAM budget (B7/12), texture zero-init.

**Deferred to v2:** `HostCoherent` / `DmaBuf` zero-copy data path, `GpuLevel::Full` on mobile, headless compute (server-headless), ray tracing, multi-GPU enumeration, Metal Performance Shaders analogues, persistent shader-cache shared across apps, hardware video decode integration (deferred to R68).

**Never:** wgpu compiled into the WASM binary, a single `wgpu::Device` shared across multiple apps, a blocking GPU fence wait inside the compositor SCHED_FIFO thread, exposing raw VkInstance pointers to WASM, allowing apps to choose their own GPU backend at runtime.

---
