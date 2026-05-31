import type { Metadata } from "next";
import { getDocBySlug, getAllDocs } from "@/lib/docs";
import MDXContent from "@/components/MDXContent";
import DocsSidebar from "@/components/DocsSidebar";

export const metadata: Metadata = {
  title: "Documentation",
  description: "VyomaOS documentation -- getting started, guides, and API reference.",
};

export default async function DocsIndexPage() {
  const doc = getDocBySlug([]);
  const sidebarItems = getAllDocs();

  return (
    <div className="mx-auto flex max-w-6xl px-4 sm:px-6">
      <DocsSidebar items={sidebarItems} />
      <article className="min-w-0 flex-1 py-8 lg:pl-8">
        {doc ? (
          <MDXContent source={doc.content} />
        ) : (
          <div className="docs-content">
            <h1>Documentation</h1>
            <p>Welcome to the VyomaOS documentation.</p>
          </div>
        )}
      </article>
    </div>
  );
}
