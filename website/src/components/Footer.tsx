import Link from "next/link";

const FOOTER_SECTIONS = [
  {
    title: "Project",
    links: [
      { label: "Features", href: "/features" },
      { label: "Documentation", href: "/docs" },
      { label: "Architecture", href: "/docs/architecture/overview" },
    ],
  },
  {
    title: "Resources",
    links: [
      { label: "Quick Start", href: "/docs/guides/quick-start" },
      { label: "Manifest Reference", href: "/docs/reference/manifest" },
      { label: "Display Protocol", href: "/docs/reference/display-protocol" },
    ],
  },
  {
    title: "Community",
    links: [
      { label: "GitHub", href: "https://github.com/hbarve1/vyomaos", external: true },
      { label: "Issues", href: "https://github.com/hbarve1/vyomaos/issues", external: true },
      { label: "Discussions", href: "https://github.com/hbarve1/vyomaos/discussions", external: true },
    ],
  },
];

export default function Footer() {
  return (
    <footer className="border-t border-vyoma-border/50 bg-vyoma-bg">
      <div className="mx-auto max-w-6xl px-4 py-12 sm:px-6">
        <div className="grid grid-cols-2 gap-8 md:grid-cols-4">
          {/* Brand */}
          <div className="col-span-2 md:col-span-1">
            <div className="flex items-center gap-2 text-lg font-bold text-vyoma-text">
              <span className="inline-block h-7 w-7 rounded-lg bg-vyoma-accent text-center text-sm font-black leading-7 text-vyoma-bg">
                V
              </span>
              VyomaOS
            </div>
            <p className="mt-3 text-sm text-vyoma-dim">
              The WASM-first operating system. Capability-secure by default.
            </p>
          </div>

          {/* Link sections */}
          {FOOTER_SECTIONS.map((section) => (
            <div key={section.title}>
              <h3 className="mb-3 text-sm font-semibold text-vyoma-text">{section.title}</h3>
              <ul className="space-y-2">
                {section.links.map((link) => (
                  <li key={link.href}>
                    <Link
                      href={link.href}
                      target={"external" in link ? "_blank" : undefined}
                      rel={"external" in link ? "noopener noreferrer" : undefined}
                      className="text-sm text-vyoma-dim transition-colors hover:text-vyoma-text"
                    >
                      {link.label}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <div className="mt-10 border-t border-vyoma-border/50 pt-6 text-center text-sm text-vyoma-dim">
          Built with Rust, WASM, and a minimal Linux kernel.
        </div>
      </div>
    </footer>
  );
}
