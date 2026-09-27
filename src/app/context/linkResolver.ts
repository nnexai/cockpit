import type { LibraryItemSummary } from "../../protocol/generated/v1";

export type ContextLinkResolution =
  | { kind: "relative"; path: string; fragment: string | null }
  | { kind: "library"; item: LibraryItemSummary }
  | { kind: "external"; href: string }
  | { kind: "refused"; reason: "root_escape" | "absolute_path" | "unsupported_scheme" | "invalid" }
  | { kind: "inert" };
function normalizedUrl(value: string): URL | null {
  try {
    const url = new URL(value);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    url.hash = "";
    url.pathname = url.pathname.replace(/\/+$/, "") || "/";
    return url;
  } catch {
    return null;
  }
}

function confluencePageId(url: URL): string | null {
  const queryId = url.searchParams.get("pageId");
  if (queryId && /^\d+$/.test(queryId)) return queryId;
  const match = /\/pages\/(\d+)(?:\/|$)/.exec(url.pathname);
  return match?.[1] ?? null;
}

function jiraIssueKey(url: URL): string | null {
  return /^\/browse\/([A-Z][A-Z0-9_]*-\d+)\/?$/i.exec(url.pathname)?.[1]?.toUpperCase() ?? null;
}

function sameSourceUrl(item: LibraryItemSummary, target: URL): boolean {
  for (const value of [item.source_url, item.original_url]) {
    if (!value) continue;
    const candidate = normalizedUrl(value);
    if (candidate && candidate.origin === target.origin && candidate.pathname === target.pathname && candidate.search === target.search) return true;
  }
  const itemUrls = [item.source_url, item.original_url].filter((value): value is string => Boolean(value)).map(normalizedUrl).filter((url): url is URL => url !== null);
  const targetPageId = confluencePageId(target);
  if (targetPageId && itemUrls.some((url) => url.host === target.host && confluencePageId(url) === targetPageId)) return true;
  const targetIssue = jiraIssueKey(target);
  if (targetIssue && itemUrls.some((url) => url.host === target.host && jiraIssueKey(url) === targetIssue)) return true;
  return false;
}

function sourceItemFor(href: string, items: readonly LibraryItemSummary[]): LibraryItemSummary | null {
  const target = normalizedUrl(href);
  if (!target) return null;
  return items.find((item) => sameSourceUrl(item, target)) ?? null;
}

/** Resolves a Markdown link without granting it authority outside the active root. */
export function resolveContextLink(
  href: string | undefined,
  documentPath: string,
  items: readonly LibraryItemSummary[] = [],
): ContextLinkResolution {
  if (!href) return { kind: "inert" };
  const external = normalizedUrl(href);
  if (external) {
    const item = sourceItemFor(href, items);
    return item ? { kind: "library", item } : { kind: "external", href: external.href };
  }
  if (href.startsWith("//")) return { kind: "refused", reason: "unsupported_scheme" };
  if (href.startsWith("/")) return { kind: "refused", reason: "absolute_path" };
  if (/^[a-z][a-z\d+.-]*:/i.test(href)) return { kind: "refused", reason: "unsupported_scheme" };
  const hashIndex = href.indexOf("#");
  const fragment = hashIndex >= 0 ? href.slice(hashIndex + 1) : null;
  const pathAndQuery = hashIndex >= 0 ? href.slice(0, hashIndex) : href;
  const pathValue = pathAndQuery.split("?", 1)[0];
  let decoded: string;
  try { decoded = decodeURIComponent(pathValue); } catch { return { kind: "refused", reason: "invalid" }; }
  if (!decoded) return { kind: "inert" };
  const segments = documentPath.split("/").slice(0, -1).filter(Boolean);
  for (const segment of decoded.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      if (!segments.length) return { kind: "refused", reason: "root_escape" };
      segments.pop();
    } else {
      segments.push(segment);
    }
  }
  if (segments.some((segment) => segment === "." || segment === ".." || segment.includes("\\") || segment.includes("\0"))) return { kind: "refused", reason: "invalid" };
  return segments.length ? { kind: "relative", path: segments.join("/"), fragment } : { kind: "inert" };
}
