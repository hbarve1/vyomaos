"use client";

import Link from "next/link";
import { useState } from "react";

const NAV_LINKS = [
  { href: "/features", label: "Features" },
  { href: "/docs", label: "Docs" },
  { href: "https://github.com/hbarve1/vyomaos", label: "GitHub", external: true },
];

export default function Navbar() {
  const [mobileOpen, setMobileOpen] = useState(false);

  return (
    <nav className="sticky top-0 z-50 border-b border-vyoma-border/50 bg-vyoma-bg/80 backdrop-blur-xl">
      <div className="mx-auto flex h-16 max-w-6xl items-center justify-between px-4 sm:px-6">
        <Link href="/" className="flex items-center gap-2 text-lg font-bold text-vyoma-text">
          <span className="inline-block h-7 w-7 rounded-lg bg-vyoma-accent text-center text-sm font-black leading-7 text-vyoma-bg">
            V
          </span>
          VyomaOS
        </Link>

        {/* Desktop links */}
        <div className="hidden items-center gap-6 md:flex">
          {NAV_LINKS.map((link) => (
            <Link
              key={link.href}
              href={link.href}
              target={link.external ? "_blank" : undefined}
              rel={link.external ? "noopener noreferrer" : undefined}
              className="text-sm text-vyoma-dim transition-colors hover:text-vyoma-text"
            >
              {link.label}
            </Link>
          ))}
          <Link
            href="/docs"
            className="rounded-lg bg-vyoma-accent px-4 py-2 text-sm font-medium text-vyoma-bg transition-colors hover:bg-vyoma-accent/90"
          >
            Get Started
          </Link>
        </div>

        {/* Mobile toggle */}
        <button
          onClick={() => setMobileOpen(!mobileOpen)}
          className="flex h-10 w-10 items-center justify-center rounded-lg text-vyoma-dim hover:text-vyoma-text md:hidden"
          aria-label="Toggle menu"
        >
          {mobileOpen ? (
            <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M4 4l12 12M16 4L4 16" />
            </svg>
          ) : (
            <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M3 5h14M3 10h14M3 15h14" />
            </svg>
          )}
        </button>
      </div>

      {/* Mobile menu */}
      {mobileOpen && (
        <div className="border-t border-vyoma-border/50 bg-vyoma-bg/95 backdrop-blur-xl md:hidden">
          <div className="flex flex-col gap-2 px-4 py-4">
            {NAV_LINKS.map((link) => (
              <Link
                key={link.href}
                href={link.href}
                target={link.external ? "_blank" : undefined}
                onClick={() => setMobileOpen(false)}
                className="rounded-lg px-3 py-2 text-sm text-vyoma-dim transition-colors hover:bg-vyoma-surface hover:text-vyoma-text"
              >
                {link.label}
              </Link>
            ))}
          </div>
        </div>
      )}
    </nav>
  );
}
