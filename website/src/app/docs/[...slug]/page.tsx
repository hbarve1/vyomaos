import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getDocBySlug, getDocSlugs, getAllDocs } from "@/lib/docs";
import MDXContent from "@/components/MDXContent";
import DocsSidebar from "@/components/DocsSidebar";

export async function generateStaticParams() {
  const slugs = getDocSlugs();
  return slugs.map((slug) => ({ slug }));
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string[] }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const doc = getDocBySlug(slug);
  if (!doc) return { title: "Not Found" };
  return {
    title: doc.meta.title,
    description: doc.meta.description,
  };
}

export default async function DocPage({
  params,
}: {
  params: Promise<{ slug: string[] }>;
}) {
  const { slug } = await params;
  const doc = getDocBySlug(slug);
  if (!doc) notFound();

  const sidebarItems = getAllDocs();

  return (
    <div className="mx-auto flex max-w-6xl px-4 sm:px-6">
      <DocsSidebar items={sidebarItems} />
      <article className="min-w-0 flex-1 py-8 lg:pl-8">
        <MDXContent source={doc.content} />
      </article>
    </div>
  );
}
