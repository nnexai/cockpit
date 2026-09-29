import { describe, expect, it } from "vitest";
import type { LibraryItemSummary } from "../../protocol/generated/v1";
import { resolveContextLink } from "./linkResolver";

const item = (overrides: Partial<LibraryItemSummary>): LibraryItemSummary => ({
  item_id: "page:98765", logical_id: "page:98765", kind: "provider_snapshot", provider_id: "confluence", provider_instance: "https://wiki.test/wiki", resource_type: "page", canonical_id: "98765",
  container: null, parent_item_id: null, ancestors: [], order: null, title: "Page", document_path: "confluence/wiki.test/Space/Page/Page.md", item_path: "confluence/wiki.test/Space/Page",
  source_url: "https://wiki.test/wiki/spaces/KEY/pages/98765/Slug", original_url: null, source_revision: "1", revision: "r1", state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, folder: null, diagnostics: [], attachments: [], ...overrides,
});

describe("resolveContextLink", () => {
  it("resolves encoded relative paths and strips optional fragments", () => {
    expect(resolveContextLink("../Parent/Parent%20Page.md#section", "Space/Child/Child.md")).toEqual({ kind: "relative", path: "Space/Parent/Parent Page.md", fragment: "section" });
    expect(resolveContextLink("_files/x.png", "Space/Child.md")).toEqual({ kind: "relative", path: "Space/_files/x.png", fragment: null });
  });

  it("refuses paths that escape the current root", () => {
    expect(resolveContextLink("../../outside.md", "only.md")).toEqual({ kind: "refused", reason: "root_escape" });
  });

  it("matches Confluence URL variants by host and page id", () => {
    const page = item({});
    expect(resolveContextLink("https://wiki.test/pages/viewpage.action?pageId=98765#history", "index.md", [page])).toEqual({ kind: "library", item: page });
    expect(resolveContextLink("https://wiki.test/wiki/spaces/KEY/pages/98765/Other-title/", "index.md", [page])).toEqual({ kind: "library", item: page });
  });

  it("refuses absolute filesystem paths instead of treating them as links", () => {
    expect(resolveContextLink("/etc/passwd", "docs/index.md")).toEqual({ kind: "refused", reason: "absolute_path" });
  });
  it("matches Jira browse variants by host and issue key", () => {
    const issue = item({ item_id: "jira:1", canonical_id: "PROJ-1", provider_id: "jira", source_url: "https://jira.test/browse/PROJ-1", original_url: null });
    expect(resolveContextLink("https://jira.test/browse/proj-1/#activity", "index.md", [issue])).toEqual({ kind: "library", item: issue });
  });

  it("normalizes generic source links and leaves unmatched web links external", () => {
    const source = item({ source_url: "https://wiki.test/wiki/a/b/" });
    expect(resolveContextLink("https://wiki.test/wiki/a/b#heading", "index.md", [source])).toEqual({ kind: "library", item: source });
    expect(resolveContextLink("https://elsewhere.test/page", "index.md", [source])).toEqual({ kind: "external", href: "https://elsewhere.test/page" });
  });
});
