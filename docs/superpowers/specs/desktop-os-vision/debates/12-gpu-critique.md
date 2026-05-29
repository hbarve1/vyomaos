# Round 12 Critique — GPU Acceleration Layer

**Role:** Critic
**Date:** 2026-05-29
**Verdict:** REJECT AND REDESIGN

---

## 1. Verdict summary

The proposal reads as a confident port of WebGPU semantics into VyomaOS, but at
least four of its load-bearing claims are mechanically false on
`wasm32-wasip2` + Wasmtime, and three more are wishful thinking about Linux
GPU drivers in 2026. The architecture-shaped object you have built is sound;
the engineering substrate it sits on is not.

The proposal must be reworked along five axes before it is mergeable:

1. **Drop the "drop-in `wgpu` crate in the app" framing.** It is not a drop-in.
   `wgpu` does not produce a working WebGPU client on `wasm32-wasip2` outside
   a browser. You will ship a hand-written `wgpu-wit` shim and you must say so
   in §1.2 and §3 explicitly. The cost is your "no protocol invention" claim
   in §1 line 2.
2. **Replace per-QoS Device sharing across mutually-distrusting apps with
   per-app `wgpu_core::Device` instances**, or accept that VyomaOS provides
   no GPU memory isolation and document that as a limitation. The current
   §2.4 ("compositor and app share the UserInteractive Device") gives an app
   *direct* access to the same Vulkan `VkDevice` that holds compositor
   textures, which is a memory-isolation hole no `ResourceTable` namespace
   fixes.
3. **Make the QEMU GPU story concrete.** virtio-gpu + virgl supports OpenGL
   ES 3.1; it does *not* meaningfully support Vulkan (Venus is still
   experimental in QEMU 9.x in 2026). `wgpu-hal/vulkan` over virgl does not
   work. Either the development backend is `wgpu-hal/gles3` (which kills
   compute), or it is Lavapipe (which is software and defeats the round's
   purpose), or it is host passthrough. Pick one explicitly; the §6.3
   `gpu-virtio = [..., "wgpu-hal/gles"]` line tacitly admits this but the
   prose §1.1 sells Vulkan.
4. **Confront the WASM↔GPU memory-model gap.** §3.4 claims `mapped-slice`
   gives "zero-copy" with VRAM mapped into the app's linear memory. This is
   not implementable on virtio-gpu, Panfrost, Mali, or Lavapipe today.
   `SharedBuffer::HostCoherent` as described requires either dmabuf import
   into WASM linear memory (not a thing) or a Wasmtime custom-memory creator
   that backs WASM pages with GPU device memory (a research project, not a
   v1 deliverable). Demote `HostCoherent` from §1.4's headline to v2.
5. **Take the shader-compilation DoS surface seriously.** ShaderQuota
   (§7.4) rate-limits the number of compile *calls*, not the cost of any one
   compile. A 9.9 MiB WGSL file shipped within rate limit can pin a compile
   thread for tens of seconds. Add a *byte-size* and *AST-node-count* cap
   measured before `naga::valid::Validator` is invoked.

Below: blocking issues in detail (mostly the attack vectors enumerated in the
brief), then non-blocking concerns, then what the architect got right, then
synthesis questions.

---

## 2. BLOCKING issues

### B1. The "wgpu inside the WASM app" path is fictional

§1.1 sells WebGPU on the strength of "compiles to wasm32-wasip2 already"
(line 71) and §1.2 shows a `wgpu::Instance::new(...)` call inside the app.
This conflates two completely different `wgpu` targets:

- `wgpu` for `wasm32-unknown-unknown` produces a binary that talks to the
  *browser host's* `navigator.gpu` JavaScript API. That code path is JS
  interop, not WIT, not WASI.
- `wgpu` for `wasm32-wasip2` *does not exist as a published configuration*.
  The crate has no Wasmtime/WASI host bindings, no WIT bindgen output, and
  no canonical-ABI shim. Its `wgpu_hal` Vulkan backend is gated on `cfg(unix)`
  and requires `libloading` to dlopen `libvulkan.so` — neither available in
  a WASM sandbox.

What you are actually proposing is to write a *new* crate — call it
`wgpu-wit` as you do, but be honest that it is not a drop-in replacement and
it does not exist today. Every WIT call has to round-trip through Wasmtime's
resource table, every record needs canonical-ABI encoding, and every
async-returning method (`request_adapter`, `request_device`, `map_async`,
`on_submitted_work_done`) is asynchronous *across the WASM boundary*, which
WIT 0.2 components handle by returning futures resources — a feature that as
of mid-2026 is still warming up in wasmtime trunk.

The §1.2 example `instance.request_adapter(&Default::default()).await` would
need:

- a Wasmtime async task wakeup,
- a host future bound to a wgpu-core poll,
- a way for the WASM app to *await* a future across a component boundary.

The component-model `future<T>` resource is in wasmtime 25+ but is not
stabilised; the apps' linker code that wraps WIT futures into Rust `Future`s
is hand-rolled per host. **You are inventing the protocol you claim not to be
inventing.** Worse, you are inventing it on a moving target.

Required changes:
- §1.1 line 71 ("compiles to wasm32-wasip2 already") must be deleted or
  changed to "we ship a new crate `wgpu-wit` that re-implements the wgpu
  surface API as a WIT client".
- §1.2 must show a `wgpu-wit::Instance`, not `wgpu::Instance`, and the
  prose must say *this is not the upstream wgpu crate; source compatibility
  is a goal, ABI compatibility is not*.
- §13 needs an open question: "what is our policy when upstream wgpu's
  trait surface changes between releases?" This is your CADT/treadmill
  exposure.
- Scope the v1 deliverable to a *subset* of the WebGPU API (no
  RenderBundleEncoder, no QuerySets, no DeviceLostInfo callback) and
  enumerate the subset in §3.

### B2. One Device shared across mutually distrusting apps = no isolation

§2.2 declares "one logical Device per QoS class". §2.4 ("we do not expose a
per-app Device") and §9.2 ("compositor never shares Device with any app's
Device *unless QoS matches*", emphasis added) make explicit that the
compositor and any `UserInteractive` app share one `wgpu_core::DeviceId`,
which translates to one `VkDevice` on the Vulkan backend.

This is a structural memory-isolation hole, for three independent reasons:

**B2.1 VkDevice is a shared address space.** Vulkan allocations on the same
device share descriptor heaps, sampler indices, and (depending on driver)
GPU virtual address space. A SPIR-V shader executing under a render pipeline
in app A can issue a descriptor index that, before the driver bounds-checks,
indexes into a descriptor created by the compositor. The architect leans
heavily on "wgpu's validator already covers most attack vectors" (§3.5 line
508). It does cover the *legal* attacks; it does not cover *driver bugs*.
The history of GPU CVEs (CVE-2023-21400, CVE-2024-32487, the entire
Panfrost CVE list) is overwhelmingly "driver mishandles out-of-range
descriptor". Sharing a `VkDevice` between the supervisor's compositor
textures and an arbitrary WASM app collapses any defense in depth here.

**B2.2 Per-app `ResourceTable` namespacing does not address this.** §2.2
asserts "buffer 7 in app A is not the same handle as buffer 7 in app B even
though they share a Device". True at the WIT level, irrelevant at the
Vulkan level. The handle table is a *host-side* check that the app cannot
forge a numeric handle to another app's buffer. It does not prevent a
*shader* — which executes on the GPU with no handle-table check — from
referencing memory by GPU virtual address. wgpu's bind-group validator
checks descriptor *bindings* on the CPU side, not what the shader *does
once running*.

**B2.3 Compute cache side channels.** §9.2 line "gpu.compute apps cannot
run while a gpu.graphics-only app is presenting on UserInteractive" assumes
the QoS taxonomy prevents this. But the more important case is two
`UserInteractive` graphics apps on the same Device. They share L2, share
the texture cache, share GMMU TLBs. The §9.4 "compute-graphics quarantine"
does not protect graphics-vs-graphics.

Required changes:
- §2.4 must be rewritten. Either:
  - **Option A (preferred):** one `wgpu_core::Device` per app. Yes, this
    costs descriptor heap. Yes, Mali drivers limit concurrent devices; cap
    GPU-using apps at N (say, 8) and queue the rest. The compositor gets
    its own Device. Cross-Device texture sharing happens via *explicit*
    submit-time copy or dmabuf import; no `mapped_slice` zero-copy from app
    into the compositor's texture pool unless mediated.
  - **Option B (documented limitation):** keep the shared Device, mark
    VyomaOS as "no GPU memory isolation between apps in the same QoS class
    on a given GPU; do not run mutually distrusting apps with `gpu = true`
    on the same hardware". Update the security model in §9.1–9.2 to admit
    this.
- The "zero copy from app render target to compositor source texture"
  optimisation (§2.4 last paragraph) is incompatible with Option A and must
  be removed or moved to v2 under a "trusted apps only" flag.

### B3. Priority inversion / GPU DoS

A direct corollary of B2 but worth its own item. Even if memory isolation
were not a concern, sharing one Vulkan queue (the §2.2 `LogicalDevice.queue_id`)
across the compositor and apps means a 100 ms-budget command buffer from a
poorly-written app blocks the compositor.

§13 open question 9 ("compositor Device sharing… do we need a per-Device
timeslicer? *Proposal:* enforce via the command-buffer cost model") gives
the wrong answer. The §3.5 `CommandBufferAudit` is a *static* cost estimate
("draw count × bound-pipeline cost, no actual GPU timing", line 521). This
estimate is open-loop and uncalibrated:

- A `dispatch_workgroups(1024, 1024, 1024)` is one indirect call but trillions
  of GPU cycles.
- A 4-vertex draw with a 50 KB fragment shader is one draw call but unbounded
  ALU time.
- An indirect draw reads its count from a buffer the audit cannot inspect.

GPU runtime DoS on Linux has no good solution. NVIDIA's GSP-RM has a
soft-timeout (TDR-equivalent); AMD does too; Mali has none and Panfrost has
TDR partially upstream as of mid-2025. Counting on per-driver TDR is fine,
but you must say so:

> The supervisor relies on the kernel GPU driver's timeout/reset (TDR) to
> bound any single submission's wallclock cost. On drivers without TDR
> (Panfrost on kernels <6.6, virtio-gpu virgl path, Lavapipe), a misbehaving
> GPU app may hang the GPU pipeline for the duration of its work. Apps with
> `gpu.compute = true` are gated for this reason.

The proposal does say something like this in §9.4 for compute but not for
graphics. Make it explicit.

Required changes:
- §3.5: replace "static cost estimate accepted/rejected at submit time" with
  "static cost estimate used as a heuristic *plus* runtime fence-with-timeout
  *plus* driver TDR. None alone is sufficient."
- §6.1: per-platform table must list "TDR available? (kernel ≥ X)" so the
  ops team can see where graphics-side DoS is mitigated and where it is not.
- Compositor design (§4.2): if a present misses its vsync deadline by more
  than one frame and the cause is GPU stall, the offending app's pipeline
  must be cancelled (`Device::lost`) and the app killed or restarted. R7
  PowerManager and R10 TimerLoop both need a hook for this.

### B4. Shader compilation as a DoS vector

This is partially addressed in §7.4 but the mitigations are mis-targeted.

`naga::valid::Validator::new(ValidationFlags::all(), Capabilities::default())`
(§7.2 step 2) is O(IR size) per pass and has well-known pathological cases:

- Deeply nested struct types blow up the type-equivalence walker.
- Long `switch` chains in the entry-point analyzer.
- Composite array constructors of constant size 10^9 (legal in WGSL grammar,
  rejected at validate, but the rejection itself walks the array literal).

The §7.4 `ShaderQuota { per_minute: 60, parallel: 4, max_ms: 500 }` defends
against many compile calls but not one giant call. Setting `max_ms: 500`
implies you will let a 500 ms compile run before timing it out — and to
implement that timeout you have to either (a) wrap naga in a thread you can
kill (Rust threads cannot be safely killed), or (b) check elapsed time
inside naga (requires patching naga), or (c) put naga in its own process
(huge engineering cost we have not budgeted).

Required changes:
- Add **input gating** before naga is called:
  - WGSL source: ≤ 256 KiB. Apps shipping bigger should split or precompile.
  - SPIR-V passthrough (§7.2 `ShaderSource::SpirV`): ≤ 1 MiB.
  - AST node count: ≤ 64 K (estimated as `source_size / 4` before parsing).
- Move shader compilation into a *separate process* (a "naga sandbox
  worker") that the supervisor talks to over a pipe. This is the only way
  to safely apply a wallclock timeout to a long compile: kill the worker
  pid. Yes, this costs 5–15 ms of IPC overhead per compile, dominated by
  the compile itself.
- The disk shader cache (§7.3) currently lives at
  `/data/shaders/<app-id>/<blake3>.spv`. Add a per-app size cap (e.g. 32
  MiB) and an LRU eviction policy — otherwise a malicious app fills `/data`
  with compiled shaders and bricks the system. R4 storage round has
  per-app quotas; reference them here.
- Document the threat model: a `gpu.graphics = true` app can degrade the
  compositor's responsiveness during its own shader compile bursts. This
  is acceptable for now but should be on the v2 backlog ("isolate
  compositor's compile pool from app compile pool").

### B5. virtio-gpu / virgl reality check

The proposal's headline says the desktop-full development environment uses
virtio-gpu with the Vulkan backend (§6.3 `gpu-vulkan = ["wgpu-hal/vulkan"]`
listed in defaults). The §6.3 fine print includes
`gpu-virtio = [..., "wgpu-hal/gles"]` which suggests the architect knows
virgl exposes OpenGL ES, not Vulkan.

State of QEMU GPU virtualization in mid-2026:

- **virgl** (in-kernel and Mesa side) is mature for OpenGL ES 3.1, partial
  for OpenGL 4.5, no Vulkan.
- **Venus** (Vulkan-on-Vulkan via virtio-gpu) is upstream in Mesa 23+ but
  requires guest Linux kernel ≥ 6.4 with `CONFIG_DRM_VIRTIO_GPU_VULKAN`,
  QEMU 8+ with `-device virtio-gpu-rutabaga`, and a host with working
  Vulkan. It works for triangle.c demos and breaks unpredictably on
  anything more complex.
- **Lavapipe** is CPU-only Vulkan. Running the GPU acceleration layer on
  Lavapipe means doing all the wgpu/SPIR-V/validation work for a compute
  cost that is *worse* than the R11 software compositor.

The architect needs to pick one and own it:

- If the dev backend is **virgl + GLES3** (`wgpu-hal/gles`): no compute
  shaders, no storage buffers, no MSAA > 4× — large parts of §3 (especially
  `compute`) are non-functional in the QEMU dev environment and you must
  test them in CI on a different backend (real hardware? Lavapipe?).
- If the dev backend is **Venus**: state-of-the-art unstable; do not
  promise this in v1.
- If the dev backend is **Lavapipe**: GPU layer adds nothing over R11
  software compositor; you have built an abstraction tax with zero perf.
- If the dev backend is **host passthrough** (vfio-pci): only works on
  specific hardware, breaks CI.

Required changes:
- §1.1 verdict table: revise "WebGPU via wgpu" pros to be honest about
  backends ("Vulkan on real HW; GLES3 in QEMU dev path; Venus optional").
- §6.1 platform support matrix: per-backend feature delta. "GLES3 backend
  has no compute pipelines" must appear.
- §6.3 features: `gpu-virtio` cannot pull in both `wgpu-hal/vulkan` and
  `wgpu-hal/gles`. Split into `gpu-virtio-gles` and `gpu-virtio-venus`.
- Add a v1 statement: "The QEMU desktop-full development image uses
  Lavapipe by default. Real-GPU testing happens on physical hardware in
  the CI matrix nightly". Yes, this means desktop-full does not gain
  anything from R12 in the dev loop. Own it.
- The §12.5 CI matrix is missing this dimension entirely.

### B6. GPU + WASM linear-memory incompatibility

§3.4 "Zero-copy mapped buffers" promises that
`mapped-slice.ptr` is "the offset of those pages inside the app's linear
memory" for the `SharedBuffer::HostCoherent` case, with "No copy."

This is mechanically false on the platforms VyomaOS targets, for layered
reasons:

**B6.1 WASM linear memory is virtually contiguous, GPU memory is not.**
Wasmtime allocates WASM linear memory via either `mmap(MAP_PRIVATE |
MAP_ANONYMOUS)` or a pooling allocator. GPU device memory is allocated by
the kernel DRM driver via GEM/dmabuf. For these two regions to share *physical*
pages, you need:

- a *single* allocator that returns memory mappable into both,
- a way to tell Wasmtime to use that allocator for *part* of the linear
  memory (Wasmtime's `MemoryCreator` is whole-memory, not subregion),
- a GPU driver that exposes a host-coherent VRAM heap that is also CPU
  cacheable (true on integrated GPUs, partially true on Mali, false on
  most discrete GPUs).

The R11 work introduced `SurfaceMemoryCreator` for SharedBuffer (per
R11-FINAL), but that is system RAM shared between the supervisor and the
WASM app via memory-creator hooks. Extending it to GPU memory requires that
the GPU's host-coherent heap is *the same* heap. On virtio-gpu in QEMU,
the guest cannot allocate host-coherent VRAM at all (the host's GPU memory
is not directly mappable into the guest's address space without blob
resources, which the architect punts on in §13.6). On Lavapipe, "VRAM" is
host memory and host-coherency is free, but Lavapipe is CPU rendering so
"GPU buffer" is just an allocation in the supervisor's address space — not
mappable into the *app's* address space without a copy.

**B6.2 The mapped-slice handle returned to the WASM app gives a u32 offset
inside the app's linear memory.** This only works if the GPU memory has
already been mapped into the app's linear memory *before* the WIT call
returns. That requires either:

- Wasmtime growing linear memory at runtime to absorb the buffer (`memory.grow`
  semantics, but as part of a host call — not a thing today),
- the buffer having been pre-allocated as part of the linear memory at app
  start (works only for fixed-size, pre-known buffers — a non-starter for
  general GPU work),
- the supervisor reaching into the app's `MemoryCreator`'s backing
  allocation and remapping a page (technically possible but the WASM app's
  store does not expect this; concurrent access requires Wasmtime
  guarantees we have not negotiated).

**B6.3 Cache coherency.** Even if you solve B6.1 and B6.2, on Mali and
many embedded GPUs, "host-coherent" memory is uncached on the CPU side.
The app reading 4 KB of just-rendered GPU output via WASM `load` instructions
will run at ~50 MB/s instead of ~10 GB/s. This often dominates the time
saved by avoiding a copy.

Required changes:
- §1.4 line "VRAM is a budget, not a resource pool" is fine; but the
  §3.4 SharedBuffer-backed zero-copy claim must move to a documented v2
  feature with a clear pre-requisites list (Wasmtime feature N, kernel
  feature M, GPU host-coherent heap, etc.).
- The §5.4 `SharedBuffer::HostCoherent` row in the table must be marked
  "v2; requires blob-resource virtio-gpu and Wasmtime partial-mmap memory
  creator". The `Surface::new_shared` selection in §5.4 reduces to
  `(want_gpu, _) → Self::new_shared_cpu_with_upload_hint`, i.e. CPU
  staging + `queue.write_texture` in v1.
- §3.4 case 1 (CPU-backed) becomes the only v1 path. The `mapped-slice`
  return value is then *not* a zero-copy offset into linear memory; it is
  a synthetic offset where the supervisor will memcpy data on `unmap`.
  Document this in the WIT comment.

### B7. Platform matrix scope creep

§6.1 claims `iot-edge` runs Panfrost on Mali G31 with a 64 MiB VRAM
budget and `mobile` runs Panfrost or Imagination PowerVR. Each of these
is a multi-year integration effort:

- **Mali G31 + Panfrost**: works on kernels ≥ 5.10, has Vulkan 1.1 since
  2022, but Vulkan is not feature-complete (no inheritedQueries, partial
  storageImageMultisample, etc.). Many wgpu features map to unsupported
  Vulkan features, returning runtime errors the proposal does not handle
  beyond "fallback to CPU."
- **PowerVR**: Imagination has an open-source Vulkan driver as of 2023, but
  it covers only the BXM/BXS series. Older PowerVR SGX (typical of cheap
  mobile chips that ship "mobile" devices in 2026) has no open driver and
  the proprietary blob is not redistributable. Saying "PowerVR backend"
  without saying "Imagination's open driver only" is misleading.
- **virtio-gpu**: see B5.

The architect needs to be explicit per platform what "supported" means.
There are three different engineering efforts conflated:

1. **The compositor uses GPU** for its own work — supervisor only.
2. **Apps can request the GPU** but the API surface is constrained.
3. **GPU driver is integrated** into the build (probe, init, error
   recovery).

`iot-edge` and `mobile` in v1 should probably only be (1), or even none.
The proposal as written implies (3) on every non-mcu platform.

Required changes:
- §6.1: split each platform row into three columns: "compositor uses GPU
  / apps can use graphics / apps can use compute / driver integrated in
  v1". Be honest. The expected outcome:
  - `desktop-full`: all four, via Lavapipe.
  - `iot-edge`: maybe compositor-only; nothing else in v1.
  - `mobile`: same, deferred.
  - `mcu-minimal` / `robotics-rt`: none.
  - `server-headless`: compute only via Lavapipe; no compositor.
- Move §6.4 power management to a stub for v1; integrate properly when
  `mobile` actually has a working GPU driver.

### B8. GPU timeline / supervisor coupling

§11.2 says the compositor "registers a vsync callback" via TimerLoop and
"If `try_present_frame` overruns the vsync budget (e.g. shader compile
blocks present), we miss one vsync and the R10 TimerLoop forces the CPU
fallback for the next 4 frames."

Two problems:

**B8.1 GPU submit/fence is async; the present thread must not poll.**
`wgpu::Queue::submit` returns a fence; reading the framebuffer requires
either:

- blocking on the fence on the present thread (stalls the compositor),
- polling the fence from a different thread and signalling the present
  thread (cross-thread sync, plus framebuffer ownership transfer).

The R11 compositor runs at SCHED_FIFO 40. If the GPU compositor blocks on
a fence on the same SCHED_FIFO thread for 8 ms, *every other SCHED_FIFO
process loses CPU for 8 ms*, including audio (R13?), input, and any
lifeline timer. R11's design assumed compositor work is bounded and CPU-
side; GPU work breaks that assumption.

The proposal does not address how the GPU compositor integrates with
SCHED_FIFO 40. It needs to.

**B8.2 Reading the framebuffer back to KMS.** When the GPU finishes
rendering, who copies the result to the framebuffer that KMS atomic flip
uses? On Linux with virtio-gpu + virgl, the typical path is:

1. wgpu render into a GPU texture.
2. The render target *is* the KMS scanout buffer (via dmabuf).
3. `drmModeAtomicCommit` flips the buffer in.

Step 2 requires the GPU texture to be allocated as a dmabuf importable into
KMS. wgpu doesn't expose this directly; you reach down through wgpu-hal's
Vulkan instance to allocate a `VkImage` with `VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT`
and import it into KMS via `drmPrimeFDToHandle`. This is a real engineering
effort and is not mentioned in the proposal. §4.4 ("overlay planes") only
addresses *direct scanout* of an app surface, not the compositor's main
render target.

Required changes:
- §4.3 needs a "Frame pipeline" subsection that maps out the threading:
  - which thread does `queue.submit()`?
  - which thread polls the fence?
  - which thread does `drmModeAtomicCommit`?
  - what is the SCHED_FIFO priority of each?
- §4.3 / §11.2: the compositor cannot run on the same SCHED_FIFO 40
  thread as R11's CPU compositor when in GPU mode. Either:
  - the compositor thread does `queue.submit` then `drmModeAtomicCommit`
    with `DRM_MODE_PAGE_FLIP_ASYNC` and never polls the fence (KMS waits),
    or
  - the compositor thread submits, polls fence with a timeout (e.g. half
    the vsync interval), and if the fence isn't ready, falls back to CPU
    *for that frame*. The fallback table (§4.2 `FallbackReason`) is
    missing a `GpuTimeout` variant.
- §11.2's "miss one vsync, fall back for 4 frames" needs an explanation
  of *who detects the miss*. Right now there is no GPU watchdog thread
  described.

---

## 3. NON-BLOCKING concerns

### N1. WGSL-only policy is the right answer but the rationale is wrong

§7.1 says we accept only WGSL because GLSL conversion would add SPIRV-Cross
which is "30 KLOC and we do not want to add" it. Fine, but the *real* reason
is that SPIRV-Cross has a long CVE history and we get the same expressive
power with naga. State that. The "we don't want more code" line undersells
the security argument.

### N2. The cost-model claim of "static estimate" needs benchmarks

§3.5's cost model assumes draw-call counts and bound pipelines are
sufficient to estimate GPU cost. In practice the bound pipeline's fragment
shader complexity dominates by 100×–10000×. The cost model needs a per-
pipeline `cost_factor` derived at pipeline-creation time (count ALU ops,
texture samples, branches in the WGSL AST) and used as a multiplier in the
audit. Mention this in §3.5 or punt explicitly to a future round.

### N3. `gpu.max_vram_mb` is per-app but VRAM is global

§8.1 says `max_vram_mb = 64` is "per-app cap." §5.1 has per-QoS-class
budgets but no per-app accountancy. The §8.2 capability table says "the
supervisor refuses allocations that would exceed it" — meaning per-app
accounting must exist. But the implementation files in §10.0 don't have a
per-app charge table. Add one explicitly, probably as a HashMap inside
`VramBudget`.

### N4. Texture zero-init via `LoadOp::Clear` "parallelises with next
frame's setup"

§9.3 line "the GPU does the clear via a render pass with `LoadOp::Clear`
so it parallelises with the next frame's setup" assumes the clear and the
next frame's render commands run concurrently on the GPU. They do not
unless they are on different queues; on a shared queue they serialise.
And §2.2 has one queue per Device. So the clear runs before the next
frame's setup. Cost 1.5× becomes more like 2×. Either find a separate
queue or be honest about the cost.

### N5. ShaderSource::SpirV path is gated on `gpu.compute` but used by
`gpu.graphics`

§7.2: "`SpirV(Vec<u32>)` only with feature `passthrough_spirv` +
`gpu_compute` cap." But the more common reason to use SPIR-V passthrough
is a precompiled graphics shader (shipped to avoid the 8–40 ms compile
cost on every device). Gating it behind `gpu.compute` is wrong. Split the
gate: `passthrough_spirv` should be its own capability or behind a
verified-publisher flag.

### N6. ResourceTable charges every object including handles for which
bytes = 0

§3.3 `ResourceTable::insert(obj, bytes)` skips the charge if `bytes == 0`.
But it still adds a slab entry. Apps can spam `create_bind_group` (bytes
mostly 0; the resource is a handle to a layout) until the slab grows
unbounded. Add a *count* quota separate from the *bytes* quota. The
§7.4 ShaderQuota model is the template.

### N7. Compute-graphics quarantine cost (§9.4)

"Compute queue runs; only when compute finishes does the next compositor
frame begin" makes the compositor non-deterministic in frame time when any
compute app is active. For a desktop OS this is fine; for a Mac-fidelity
OS that brags about 120 Hz buttery scroll, it is unacceptable. Either:
- ban compute on `desktop-full` for v1 too (not just mobile / iot), or
- accept that 120 Hz is best-effort and compositor frame time is bounded
  only at, say, 60 Hz when compute is in flight.

State the choice; today §9.4 reads like an aspiration.

### N8. The `gpu-app` world imports compute unconditionally

§3.2 line 408: `world gpu-app { import vyoma:gpu/compute@0.2.0; }`. Then
§3.2 comment: "host-side fails if cap not granted." A WIT *world* mismatch
fails at component instantiation time *with a hard error* — there's no
"soft fail" on import. If an app's manifest doesn't grant compute, the
app's `gpu-app` world has no compute import in its component-type, so the
component-as-built does not link. You need two worlds:

```wit
world gpu-app          { import webgpu; import present; }
world gpu-compute-app  { import webgpu; import present; import compute; }
```

And the supervisor picks which world to link based on the manifest. Fix §3.2.

### N9. The Cargo features in §6.3 are inconsistent

`gpu` enables `dep:wgpu-core`, `dep:wgpu-types`, `dep:wgpu-hal`, `dep:naga`.
But `gpu-virtio` includes `wgpu-hal/gles` and `gpu-vulkan` includes
`wgpu-hal/vulkan` — these are wgpu-hal sub-features, which only work if
wgpu-hal is already a dependency. The feature graph forces `gpu` to be
on whenever `gpu-virtio` is on; document this with `default-features =
false, features = [...]` examples. Also: `gpu-compute = []` is described
as "gated arch-wide kill switch" but an empty feature does nothing. It
needs `#[cfg(feature = "gpu-compute")]` gates at every compute touchpoint,
or it should be removed.

### N10. The §13 question #4 "fast_alloc opt-out" is a footgun

§13.4 asks if `gpu.fast_alloc = true` should skip zero-init. The §9.3
TextureInitPolicy explicitly enumerates this. The risk: an app developer
sets `fast_alloc = true` for perf, then logs into a different app context,
and the previous app's data leaks. The default-on stance is right; the
opt-out should *only* be permitted for compute apps and *only* for
storage buffers they themselves wrote to in the same submission.

### N11. The `wgpu-wit` crate is a fork/clone treadmill

Upstream wgpu releases every 6 weeks and breaks the surface API regularly.
Maintaining `wgpu-wit` as a source-compatible shim means tracking those
breakages forever, or pinning to a wgpu version and shipping a stale API
to apps. Either is a long-term tax. Add a §13 question: "what is the
versioning policy for wgpu-wit vs upstream wgpu?" and propose: pin to
wgpu N.M, document an API freeze, accept that VyomaOS apps cannot use new
wgpu features until we bump.

### N12. Overlay planes (§4.4) are over-promised

The `multiplane` virtio-gpu extension is not stable upstream. Mali Bifrost
has 4 planes per output *on paper* but the Panfrost KMS implementation
only wires up the primary plane and (sometimes) the cursor. v1 overlay
plane support is realistically "primary plane + hardware cursor" — i.e.
exactly what KMS already does. Demote §4.4 to a v2 feature; keep the API
shape but expect 0 overlay planes on the v1 platforms.

### N13. Cost-model multiplied by GPU clock speed?

§3.5's `max_duration_ns: u64` is wallclock, but the cost model produces a
unitless "estimated cost." How is that unit-converted to ns? You either
need a calibration step at boot (run a synthetic benchmark, derive
nanoseconds per cost unit) or you punt on `max_duration_ns` and use a unit-
less budget. Pick one; the proposal is fuzzy.

### N14. R11 SchedFifo coupling not addressed

R11-FINAL established the compositor runs at SCHED_FIFO 40. R12 needs to
say what priority the GPU compositor thread runs at, what priority the
GPU fence-polling thread runs at, and what priority the shader compile
workers run at. The §7.2 line "tokio::task::spawn_blocking on a 2-thread
pool, QoS = Utility" is missing the actual scheduling-class mapping. R5
QoS classes are an abstraction; what `sched_setattr(SCHED_OTHER, nice=10)`
or similar do they translate to here?

### N15. Manifest backwards compatibility

`[capabilities.gpu]` is now a struct. Apps shipping `gpu = true` (bool)
will fail manifest parsing. The migration path is not addressed. Either
the parser accepts both forms with a deprecation warning, or you cut a
clean break and document it in §8.

### N16. `host-coherent` is spelled inconsistently

`host_coherent_buffers` in TOML (§6.2) vs `host_coherent_supported()` in
Rust (§5.4) vs `HostCoherent` enum variant. Minor but worth fixing for
grep.

### N17. The structured `[shaders]` block doesn't include WGSL options

The §7.3 `[shaders]` section just lists source files. It should also
indicate target backends (vulkan / gles3 / metal) so the install-time
precompiler can produce per-backend SPIR-V/GLSL. Otherwise the precompile
is platform-specific and breaks the "build once, deploy anywhere" WASM
story.

### N18. R10 vsync coupling assumes external GPU clock

§11.2 `timer.on_vsync(...)` assumes R10 TimerLoop has a real vsync signal.
In QEMU with virtio-gpu, the timer just ticks at 60 Hz; in real KMS, it
comes from `DRM_EVENT_VBLANK`. The proposal must say how R10 sources its
vsync, and what happens when the GPU compositor wants 120 Hz on hardware
that supports it but R10 still ticks at 60 Hz.

---

## 4. What the architect got right

Despite the structural problems above, large parts of this proposal are
solid and should survive a redesign:

1. **WIT-modeled WebGPU is the right north star.** Whatever its current
   implementation gaps (B1), targeting WebGPU is correct: it's the only
   GPU API designed for sandboxed clients, it's the only one with a sane
   shader language (WGSL), and the validation model is mature. Don't
   abandon this just because the wgpu-wit shim is more work than implied.

2. **Splitting `gpu.graphics` from `gpu.compute` as separate capability
   fields.** This is the right call. Compute's attack surface is materially
   higher (timing channels, longer submissions, storage buffers) and apps
   that need only render-on-canvas should not opt into compute.

3. **Two-mode compositor with R11 software as the canonical implementation.**
   "GPU as accelerator, never as oracle" (§1 line 3) is exactly right.
   The R11 compositor remains source-of-truth; the GPU path is a fast lane.
   This is the right defense for the brittle Linux GPU driver landscape.

4. **Per-QoS-class VRAM budgets with O(1) charge/refund and explicit
   pressure thresholds.** §5.1 is well thought out. The compositor and
   chrome reserves are mandatory and protect responsiveness from app
   memory pressure.

5. **The Texture lifecycle states (Hot/Warm/Cached/Evicted) with LRU
   eviction.** §5.3 is clean and matches the well-established cache-state
   machine pattern from macOS IOSurface purgeable policy. The "never evict
   mid-frame" invariant is correctly stated.

6. **`zero_on_create` defaulting to true.** §9.3 makes the right call on
   the perf-vs-info-leak tradeoff. A class of cross-app leaks is closed by
   default.

7. **WGSL-only input policy.** §7.1's rejection of GLSL ingest in the
   supervisor is correct. Apps that ship GLSL precompile to WGSL or naga IR
   at app-build time. Smaller attack surface, simpler validator.

8. **Documented v1-skips.** §14 item 10 enumerates Metal Performance
   Shaders, ray tracing, Heap/argument buffers, multi-GPU, eGPU, headless
   compute server, and app dmabuf import. That discipline is rare and
   appreciated; the open question backlog (§13) is similarly thorough.

9. **Reuse of prior rounds.** §11 explicitly threads the design through
   R2, R5, R6, R7, R9, R10, R11. The compositor frame timing diagram and
   the SharedBuffer extension are *almost right* (B6 caveats notwithstanding).

10. **The implementation file layout respects the 500-line rule.** §10's
    13-file split is plausible and aligns with the supervisor convention.
    The cost model and shader cache as separate files in particular are
    sensible.

11. **Honest accounting of degradation paths.** §4.2 `FallbackReason`
    enumerates real failure modes (NoGpuAdapter, VramExhausted, DriverError,
    ShaderCompileFailed, HardOomKill, PlatformDisabled). The hysteresis
    around fallback (§11.2 "next 4 frames") is the right shape, even if
    the trigger mechanism needs work (B8).

---

## 5. Questions for synthesis

The architect's §13 already enumerates ten open questions. The Critic
adds the following, ordered by blocking severity:

**S1.** What is the v1 dev-loop story? With B5 / B6 unresolved, the
`desktop-full` QEMU image either runs Lavapipe (no real GPU acceleration)
or virgl GLES3 (no compute, partial graphics). Pick one explicitly. The
implication for §12.3 benchmarks: targets at "120 FPS trivial scene" need
to be benchmarked against the chosen backend, not a hypothetical Vulkan.

**S2.** Per-app Device vs shared Device (B2). Choose: (A) per-app Device
+ accept descriptor-heap pressure and force submit-time copies between
app and compositor; (B) shared Device + document "no GPU memory isolation
between apps in the same QoS class on the same GPU" as a security
limitation. Architect proposed (B) without acknowledging the security
gap; Critic prefers (A) but needs a concrete answer.

**S3.** What does `mapped-slice` actually return (B6)? Either it's a
synthetic offset that requires a host-side memcpy on map/unmap (v1, safe),
or it's a real linear-memory offset backed by GPU memory (v2, requires
research). Lock down v1 to the former.

**S4.** Where does shader compilation actually run (B4)? On the
supervisor's main thread (blocks the whole supervisor on a 5-second naga
pathological case), in a thread pool (rust threads can't be safely killed
on timeout), or in a separate process (real engineering cost but
correct)? Pick.

**S5.** How does the compositor present without blocking SCHED_FIFO 40
(B8)? Either a separate fence-polling thread with cross-thread
framebuffer ownership transfer, or KMS-driven async page flip that lets
the kernel wait on the fence. The §11 R10 vsync integration is
incomplete until this is decided.

**S6.** When wgpu upstream breaks the API, what do we do (N11)? Pin and
freeze (apps can't use new wgpu features), or track-and-update (perpetual
maintenance tax). The answer affects the WIT package versioning policy
(currently `vyoma:gpu@0.2.0` — is that the WebGPU spec version, the
wgpu version, or VyomaOS's own?).

**S7.** What is the compute timing-channel posture for `desktop-full`
(N7)? Either ban compute for v1 (small scope, safe), or accept that
compute apps degrade the compositor to ~60 Hz best-effort. The latter
breaks the 120 Hz promise but is much more useful.

**S8.** Does `gpu = bool` continue to parse alongside `[capabilities.gpu]`
struct (N15)? If yes, write the migration; if no, write the breaking-change
note and bump a manifest schema version.

**S9.** Does R10 publish a real vsync source or a 60 Hz tick (N18)? And
on hardware that supports 120 Hz, what's the path to wire up KMS VBLANK
events for the GPU compositor?

**S10.** What is the actual claim about `iot-edge` and `mobile` GPU
support in v1 (B7)? Compositor only? Apps too? Driver integrated? None
of the above? The §6.1 table promises all three and probably means none.

---

## Closing

The proposal is admirable in its breadth. The author has done the work
to map Metal to WebGPU, integrate with R2 / R5 / R6 / R7 / R9 / R10 / R11,
think about VRAM budgets, design a shader cache, and enumerate v1-skips.
This is a senior architect's document.

It is also wrong about the things it cannot afford to be wrong about. The
two non-negotiables are:

- **The wgpu-as-WIT story doesn't exist yet.** Stop selling it as a
  drop-in. Ship it as a new crate we own, scope it down to a subset, and
  treat upstream wgpu as a moving target.
- **One Device shared across apps is not isolation.** Pick per-app Devices
  or document the limitation. There is no middle ground that the
  `ResourceTable` per-app namespacing provides.

If both are addressed, plus the QEMU / virtio-gpu backend story (B5), the
mapped-buffer reality check (B6), the shader-compilation DoS (B4), the
GPU-fence threading (B8), and the realistic scope on iot/mobile (B7), the
result is a v1 GPU layer the Critic can sign off on. As written, it is a
v0.5 GPU layer with v2 ambitions painted over the gaps.

**Verdict: REJECT AND REDESIGN.** Resubmit with the above changes; the
core direction is correct.
