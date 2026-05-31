"use client";

import Link from "next/link";
import ScrollReveal from "@/components/ScrollReveal";

const STACK_LAYERS = [
  { label: "WASM Apps", detail: "wasm32-wasip2 binaries, 1-10 KB each", color: "bg-vyoma-accent" },
  { label: "Wasmtime Runtime", detail: "WASI Preview 2, capability enforcement", color: "bg-vyoma-accent/80" },
  { label: "Rust Supervisor", detail: "PID 1, IPC broker, compositor, HAL", color: "bg-vyoma-green/80" },
  { label: "Linux Kernel", detail: "allnoconfig, 2.3 MB, hardware only", color: "bg-vyoma-orange/80" },
];

export default function ArchitectureSection() {
  return (
    <section className="border-t border-vyoma-border/30 py-20">
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <ScrollReveal>
          <div className="mb-12 text-center">
            <h2 className="text-3xl font-bold text-vyoma-text">System Architecture</h2>
            <p className="mt-3 text-vyoma-dim">
              Four layers. Every app sandboxed. The kernel handles hardware, nothing else.
            </p>
          </div>
        </ScrollReveal>

        <div className="grid items-start gap-10 lg:grid-cols-2">
          {/* Stack diagram */}
          <ScrollReveal delay={0.1}>
            <div className="space-y-3">
              {STACK_LAYERS.map((layer, i) => (
                <div
                  key={layer.label}
                  className="rounded-lg border border-vyoma-border/60 bg-vyoma-surface/50 p-4"
                >
                  <div className="flex items-center gap-3">
                    <div className={`h-3 w-3 rounded-full ${layer.color}`} />
                    <span className="font-semibold text-vyoma-text">{layer.label}</span>
                  </div>
                  <p className="mt-1 pl-6 text-sm text-vyoma-dim">{layer.detail}</p>
                  {i < STACK_LAYERS.length - 1 && (
                    <div className="mt-3 flex justify-center text-vyoma-dim">
                      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
                        <path d="M8 3v10M4 9l4 4 4-4" />
                      </svg>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </ScrollReveal>

          {/* Supervisor detail */}
          <ScrollReveal delay={0.2}>
            <div className="rounded-xl border border-vyoma-border/60 bg-vyoma-surface/30 p-6">
              <h3 className="mb-4 text-lg font-semibold text-vyoma-text">Supervisor Subsystems</h3>
              <div className="grid grid-cols-2 gap-2">
                {[
                  "Manifest Parser",
                  "Concurrent Scheduler",
                  "IPC Broker",
                  "Framebuffer Driver",
                  "TTY Input Router",
                  "Process Manager",
                  "Window Compositor",
                  "OTA Updater",
                  "HAL (GPIO/I2C/SPI)",
                  "Seccomp Enforcer",
                  "Package Manager",
                  "Session Manager",
                ].map((sub) => (
                  <div
                    key={sub}
                    className="rounded-md border border-vyoma-border/40 bg-vyoma-bg/50 px-3 py-2 text-xs text-vyoma-dim"
                  >
                    {sub}
                  </div>
                ))}
              </div>
              <div className="mt-6">
                <Link
                  href="/docs/architecture/overview"
                  className="text-sm font-medium text-vyoma-accent transition-colors hover:text-vyoma-accent/80"
                >
                  Read the full architecture docs &rarr;
                </Link>
              </div>
            </div>
          </ScrollReveal>
        </div>
      </div>
    </section>
  );
}
