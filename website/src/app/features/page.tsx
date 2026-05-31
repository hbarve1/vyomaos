import type { Metadata } from "next";
import FeaturesContent from "./FeaturesContent";

export const metadata: Metadata = {
  title: "Features",
  description: "Explore the key features of VyomaOS -- capability-secure WASM, multi-platform, and more.",
};

export default function FeaturesPage() {
  return <FeaturesContent />;
}
