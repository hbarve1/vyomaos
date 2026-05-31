import fs from "fs";
import path from "path";
import matter from "gray-matter";

const DOCS_DIR = path.join(/* turbopackIgnore: true */ process.cwd(), "src/content/docs");

export interface DocMeta {
  title: string;
  description: string;
  order?: number;
  slug: string[];
}

export interface Doc {
  meta: DocMeta;
  content: string;
}

function getFilesRecursive(dir: string, base: string = ""): string[] {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const relPath = base ? `${base}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      files.push(...getFilesRecursive(path.join(dir, entry.name), relPath));
    } else if (entry.name.endsWith(".md") || entry.name.endsWith(".mdx")) {
      files.push(relPath);
    }
  }

  return files;
}

function filePathToSlug(filePath: string): string[] {
  const withoutExt = filePath.replace(/\.(md|mdx)$/, "");
  if (withoutExt === "index") return [];
  const parts = withoutExt.split("/");
  if (parts[parts.length - 1] === "index") {
    parts.pop();
  }
  return parts;
}

export function getDocBySlug(slug: string[]): Doc | null {
  const slugPath = slug.length === 0 ? "index" : slug.join("/");

  const candidates = [
    path.join(DOCS_DIR, `${slugPath}.md`),
    path.join(DOCS_DIR, `${slugPath}.mdx`),
    path.join(DOCS_DIR, `${slugPath}/index.md`),
    path.join(DOCS_DIR, `${slugPath}/index.mdx`),
  ];

  for (const filePath of candidates) {
    if (fs.existsSync(filePath)) {
      const raw = fs.readFileSync(filePath, "utf-8");
      const { data, content } = matter(raw);
      return {
        meta: {
          title: (data.title as string) || slug[slug.length - 1] || "Docs",
          description: (data.description as string) || "",
          order: data.order as number | undefined,
          slug,
        },
        content,
      };
    }
  }

  return null;
}

export interface SidebarItem {
  title: string;
  slug: string[];
  href: string;
  children?: SidebarItem[];
}

export function getAllDocs(): SidebarItem[] {
  const files = getFilesRecursive(DOCS_DIR);
  const items: SidebarItem[] = [];
  const sections: Record<string, SidebarItem[]> = {};

  for (const file of files) {
    const slug = filePathToSlug(file);
    const filePath = path.join(DOCS_DIR, file);
    const raw = fs.readFileSync(filePath, "utf-8");
    const { data } = matter(raw);
    const title = (data.title as string) || slug[slug.length - 1] || "Overview";
    const href = slug.length === 0 ? "/docs" : `/docs/${slug.join("/")}`;

    if (slug.length === 0) {
      items.unshift({ title, slug, href });
    } else if (slug.length === 1) {
      items.push({ title, slug, href });
    } else {
      const section = slug[0];
      if (!sections[section]) sections[section] = [];
      sections[section].push({ title, slug, href });
    }
  }

  for (const [section, children] of Object.entries(sections)) {
    const existing = items.find((i) => i.slug[0] === section);
    if (existing) {
      existing.children = children;
    } else {
      items.push({
        title: section.charAt(0).toUpperCase() + section.slice(1),
        slug: [section],
        href: `/docs/${section}`,
        children,
      });
    }
  }

  return items;
}

export function getDocSlugs(): string[][] {
  const files = getFilesRecursive(DOCS_DIR);
  return files.map(filePathToSlug).filter((s) => s.length > 0);
}
