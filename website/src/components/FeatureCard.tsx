"use client";

import ScrollReveal from "./ScrollReveal";

interface FeatureCardProps {
  icon: React.ReactNode;
  title: string;
  description: string;
  index: number;
}

export default function FeatureCard({ icon, title, description, index }: FeatureCardProps) {
  return (
    <ScrollReveal delay={index * 0.1}>
      <div className="group rounded-xl border border-vyoma-border/60 bg-vyoma-surface/50 p-6 transition-all hover:border-vyoma-accent/40 hover:bg-vyoma-surface">
        <div className="mb-4 flex h-10 w-10 items-center justify-center rounded-lg bg-vyoma-accent/10 text-vyoma-accent">
          {icon}
        </div>
        <h3 className="mb-2 text-lg font-semibold text-vyoma-text">{title}</h3>
        <p className="text-sm leading-relaxed text-vyoma-dim">{description}</p>
      </div>
    </ScrollReveal>
  );
}
