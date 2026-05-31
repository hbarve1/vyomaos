"use client";

import ScrollReveal from "@/components/ScrollReveal";

const DETAILED_FEATURES = [
  {
    title: "Capability-Secure by Default",
    description:
      "The supervisor only wires up WASI imports that are declared in each app's vyoma.toml manifest. If an app does not declare network access, there is no network interface to syscall. No seccomp, AppArmor, or SELinux needed.",
    details: ["Manifest-declared capabilities", "Zero-trust app model", "No filtering layer overhead"],
    accent: "from-vyoma-accent to-blue-400",
  },
  {
    title: "WASM Runtime (wasm32-wasip2)",
    description:
      "Every application compiles to WebAssembly using WASI Preview 2. Binaries are deterministic and byte-identical across all build hosts and architectures. The Wasmtime runtime executes apps with minimal overhead.",
    details: ["Deterministic builds", "Cross-platform binaries", "Wasmtime 43.0.0 runtime"],
    accent: "from-vyoma-green to-emerald-400",
  },
  {
    title: "Rust Supervisor (PID 1)",
    description:
      "A static musl-linked Rust binary serves as PID 1. It handles app lifecycle, IPC brokering, display compositing, input routing, and process management. 62+ subsystem modules cover everything from GPIO to OTA updates.",
    details: ["~2.9 MB static binary", "62+ subsystem modules", "Concurrent app scheduler"],
    accent: "from-vyoma-orange to-amber-400",
  },
  {
    title: "Multi-Platform Profiles",
    description:
      "Six platform profiles target different hardware: MCU (128 KB RAM, wasm3), IoT/Robotics (ARM64, WAMR AOT), Mobile (ARM64, Wasmtime JIT), Desktop (x86-64, Wasmtime JIT), and Server (headless, Wasmtime JIT).",
    details: ["MCU to server coverage", "Profile-specific runtimes", "Hardware abstraction layer"],
    accent: "from-purple-400 to-pink-400",
  },
  {
    title: "IPC & Windowing",
    description:
      "Apps communicate through a supervisor-brokered IPC system using @app: message format. The VYOMA_DRAW protocol enables framebuffer rendering with fill_rect, draw_text, draw_glyph, and draw_image commands.",
    details: ["Supervisor-brokered routing", "VYOMA_DRAW protocol v2", "Composited window manager"],
    accent: "from-cyan-400 to-vyoma-accent",
  },
  {
    title: "Minimal Linux Kernel",
    description:
      "The kernel is compiled with allnoconfig plus only the drivers VyomaOS needs: virtio, 9P, DRM, fbcon. No networking stack, no USB, no excess filesystem drivers. At 2.3 MB it boots in under 2 seconds.",
    details: ["2.3 MB kernel image", "allnoconfig baseline", "<2s boot to supervisor"],
    accent: "from-red-400 to-vyoma-orange",
  },
];

export default function FeaturesContent() {
  return (
    <div className="mx-auto max-w-6xl px-4 py-16 sm:px-6">
      <ScrollReveal>
        <div className="mb-16 text-center">
          <h1 className="text-4xl font-bold text-vyoma-text">Features</h1>
          <p className="mt-4 text-lg text-vyoma-dim">
            A ground-up OS built on WebAssembly, Rust, and a minimal kernel.
          </p>
        </div>
      </ScrollReveal>

      <div className="space-y-8">
        {DETAILED_FEATURES.map((feature, i) => (
          <ScrollReveal key={feature.title} delay={i * 0.08}>
            <div className="rounded-xl border border-vyoma-border/60 bg-vyoma-surface/30 p-6 sm:p-8">
              <div className="flex flex-col gap-6 sm:flex-row sm:items-start">
                <div className={`h-1.5 w-16 shrink-0 rounded-full bg-gradient-to-r ${feature.accent} sm:mt-2`} />
                <div className="flex-1">
                  <h2 className="text-xl font-semibold text-vyoma-text">{feature.title}</h2>
                  <p className="mt-2 leading-relaxed text-vyoma-dim">{feature.description}</p>
                  <ul className="mt-4 flex flex-wrap gap-2">
                    {feature.details.map((d) => (
                      <li
                        key={d}
                        className="rounded-md border border-vyoma-border/50 bg-vyoma-bg/50 px-3 py-1 text-xs text-vyoma-dim"
                      >
                        {d}
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            </div>
          </ScrollReveal>
        ))}
      </div>
    </div>
  );
}
