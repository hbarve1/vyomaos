"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { SidebarItem } from "@/lib/docs";

interface DocsSidebarProps {
  items: SidebarItem[];
}

function SidebarLink({ item }: { item: SidebarItem }) {
  const pathname = usePathname();
  const isActive = pathname === item.href;

  return (
    <Link
      href={item.href}
      className={`block rounded-md px-3 py-1.5 text-sm transition-colors ${
        isActive
          ? "bg-vyoma-accent/10 font-medium text-vyoma-accent"
          : "text-vyoma-dim hover:bg-vyoma-surface hover:text-vyoma-text"
      }`}
    >
      {item.title}
    </Link>
  );
}

function SidebarSection({ item }: { item: SidebarItem }) {
  return (
    <div className="mb-4">
      {item.children ? (
        <>
          <h4 className="mb-1 px-3 text-xs font-semibold uppercase tracking-wider text-vyoma-dim">
            {item.title}
          </h4>
          <div className="space-y-0.5">
            {item.children.map((child) => (
              <SidebarLink key={child.href} item={child} />
            ))}
          </div>
        </>
      ) : (
        <SidebarLink item={item} />
      )}
    </div>
  );
}

export default function DocsSidebar({ items }: DocsSidebarProps) {
  return (
    <aside className="sticky top-20 hidden h-[calc(100vh-5rem)] w-64 shrink-0 overflow-y-auto pr-6 lg:block">
      <nav className="space-y-1 py-6">
        {items.map((item) => (
          <SidebarSection key={item.href} item={item} />
        ))}
      </nav>
    </aside>
  );
}
