"use client";

import Link from "next/link";
import dynamic from "next/dynamic";
import { motion } from "framer-motion";

const Scene3D = dynamic(() => import("./Scene3D"), { ssr: false });

const fadeUp = {
  initial: { opacity: 0, y: 20 },
  whileInView: { opacity: 1, y: 0 },
  viewport: { once: true, amount: 0.1 as const },
};

export default function Hero() {
  return (
    <section className="relative overflow-hidden">
      <div className="pointer-events-none absolute inset-0 bg-gradient-to-b from-vyoma-accent/5 via-transparent to-transparent" />

      <div className="mx-auto max-w-6xl px-4 pb-16 pt-12 sm:px-6 sm:pt-20">
        <div className="grid items-center gap-8 lg:grid-cols-2">
          <div>
            <motion.div {...fadeUp} transition={{ duration: 0.5 }}>
              <div className="mb-4 inline-block rounded-full border border-vyoma-border bg-vyoma-surface/80 px-4 py-1.5 text-xs font-medium text-vyoma-dim">
                112 phases complete &middot; 207+ apps &middot; 6 platforms
              </div>
            </motion.div>

            <motion.h1
              {...fadeUp}
              transition={{ duration: 0.5, delay: 0.1 }}
              className="text-4xl font-bold leading-tight tracking-tight text-vyoma-text sm:text-5xl lg:text-6xl"
            >
              The{" "}
              <span className="bg-gradient-to-r from-vyoma-accent to-vyoma-green bg-clip-text text-transparent">
                WASM-First
              </span>{" "}
              Operating System
            </motion.h1>

            <motion.p
              {...fadeUp}
              transition={{ duration: 0.5, delay: 0.2 }}
              className="mt-5 max-w-xl text-lg leading-relaxed text-vyoma-dim"
            >
              Every app is a capability-secure WebAssembly binary. A Rust supervisor
              on a minimal Linux kernel delivers sandboxing, IPC, and a full desktop
              — from MCU to server.
            </motion.p>

            <motion.div
              {...fadeUp}
              transition={{ duration: 0.5, delay: 0.3 }}
              className="mt-8 flex flex-wrap gap-3"
            >
              <Link
                href="/docs"
                className="rounded-lg bg-vyoma-accent px-6 py-3 text-sm font-semibold text-vyoma-bg transition-colors hover:bg-vyoma-accent/90"
              >
                Get Started
              </Link>
              <Link
                href="https://github.com/hbarve1/vyomaos"
                target="_blank"
                rel="noopener noreferrer"
                className="rounded-lg border border-vyoma-border bg-vyoma-surface px-6 py-3 text-sm font-semibold text-vyoma-text transition-colors hover:border-vyoma-dim"
              >
                View on GitHub
              </Link>
            </motion.div>
          </div>

          <motion.div
            initial={{ opacity: 0, scale: 0.9 }}
            whileInView={{ opacity: 1, scale: 1 }}
            viewport={{ once: true, amount: 0.1 }}
            transition={{ duration: 0.7, delay: 0.2 }}
          >
            <Scene3D />
          </motion.div>
        </div>
      </div>
    </section>
  );
}
