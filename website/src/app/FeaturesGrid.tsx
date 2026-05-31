"use client";

import FeatureCard from "@/components/FeatureCard";
import ScrollReveal from "@/components/ScrollReveal";

const FEATURES = [
  {
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <rect x="3" y="3" width="14" height="14" rx="2" />
        <path d="M3 7h14M7 7v10" />
      </svg>
    ),
    title: "Capability-Secure",
    description: "Every app is sandboxed by default. Undeclared capabilities are never wired up -- no syscall filtering needed.",
  },
  {
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <circle cx="10" cy="10" r="7" />
        <path d="M7 10l2 2 4-4" />
      </svg>
    ),
    title: "WASM Runtime",
    description: "All apps compile to wasm32-wasip2 and run on Wasmtime. Deterministic, byte-identical binaries across all hosts.",
  },
  {
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <rect x="2" y="4" width="16" height="12" rx="1" />
        <path d="M6 8l3 3-3 3M11 14h4" />
      </svg>
    ),
    title: "62+ Supervisor Modules",
    description: "From compositor to firewall, OTA updates to HAL. The Rust supervisor manages everything above the kernel.",
  },
  {
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <rect x="3" y="3" width="5" height="5" rx="1" />
        <rect x="12" y="3" width="5" height="5" rx="1" />
        <rect x="3" y="12" width="5" height="5" rx="1" />
        <rect x="12" y="12" width="5" height="5" rx="1" />
      </svg>
    ),
    title: "207+ Apps",
    description: "Full desktop environment with file manager, text editor, browser, terminal, app store, and more.",
  },
  {
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M3 10a7 7 0 0114 0M6 10a4 4 0 018 0" />
        <circle cx="10" cy="10" r="1.5" fill="currentColor" />
      </svg>
    ),
    title: "Multi-Platform",
    description: "One codebase, six profiles: desktop, mobile, IoT, MCU, robotics, and server. From 128 KB to 1 GB RAM.",
  },
  {
    icon: (
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M10 3v6l3 3" />
        <circle cx="10" cy="10" r="7" />
      </svg>
    ),
    title: "<2s Boot",
    description: "Minimal Linux kernel (2.3 MB) plus a static Rust supervisor. Boots in QEMU in under 2 seconds.",
  },
];

export default function FeaturesGrid() {
  return (
    <section className="border-t border-vyoma-border/30 py-20">
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <ScrollReveal>
          <div className="mb-12 text-center">
            <h2 className="text-3xl font-bold text-vyoma-text">
              Built for the WASM era
            </h2>
            <p className="mt-3 text-vyoma-dim">
              A ground-up rethink of the OS for capability-secure WebAssembly workloads.
            </p>
          </div>
        </ScrollReveal>

        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {FEATURES.map((feature, i) => (
            <FeatureCard key={feature.title} {...feature} index={i} />
          ))}
        </div>
      </div>
    </section>
  );
}
