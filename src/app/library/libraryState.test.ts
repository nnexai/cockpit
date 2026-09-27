import { describe, expect, it } from "vitest";
import type { LibraryFollowSummary, LibraryItemState, LibraryItemSummary, ProjectProvider } from "../../protocol/generated/v1";
import { confluencePageInput, confluenceSpaceInput, itemKindLabel, itemTreeLabel, libraryInputUrl, libraryStateChip, libraryTree } from "./libraryState";

const providers: ProjectProvider[] = [
  { id: "gitlab", base_url: "https://gitlab.test", executable: "/usr/bin/glab" },
  { id: "github", base_url: "https://github.com", executable: "gh" },
  { id: "jira", base_url: "https://jira.test/jira/", executable: "jira" },
];

function item(overrides: Partial<LibraryItemSummary>): LibraryItemSummary {
  return {
    item_id: "source:x", logical_id: "source:x", kind: "provider_snapshot", provider_id: "gitlab", provider_instance: "https://gitlab.test", resource_type: "issue",
    canonical_id: "platform/api#1", container: null, parent_item_id: null, ancestors: [], order: null, title: "Title",
    document_path: "x/document.md", item_path: "x", source_url: null, original_url: null, source_revision: null, revision: "r",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, follow_id: null, attachments: [], folder: null, diagnostics: [],
    ...overrides,
  };
}

describe("Library tree ordering", () => {
  it("orders instances alphabetically with Folders last, containers alphabetically, and items newest id first", () => {
    const tree = libraryTree([
      item({ item_id: "a", canonical_id: "platform/api#9", container: { container_id: "platform/api", label: "platform/api" } }),
      item({ item_id: "folder", kind: "folder_copy", provider_id: null, provider_instance: null, canonical_id: null, title: "design-notes" }),
      item({ item_id: "b", canonical_id: "platform/api!1290", resource_type: "review", container: { container_id: "platform/api", label: "platform/api" } }),
      item({ item_id: "c", canonical_id: "aaa/web#3", container: { container_id: "aaa/web", label: "aaa/web" } }),
      item({ item_id: "j", provider_id: "jira", provider_instance: "https://jira.test/jira", resource_type: "issue", canonical_id: "OPS-311", container: { container_id: "OPS", label: "OPS" } }),
      item({ item_id: "g", provider_id: "github", provider_instance: "https://github.com", resource_type: "review", canonical_id: "other/repo!7", container: { container_id: "other/repo", label: "other/repo" } }),
    ], providers);
    expect(tree.map((instance) => instance.label)).toEqual(["GitHub · github.com", "GitLab · gitlab.test", "Jira · jira.test/jira", "Folders"]);
    const gitlab = tree[1]!;
    expect(gitlab.containers.map((container) => container.label)).toEqual(["aaa/web", "platform/api"]);
    expect(gitlab.containers[1]!.items.map((entry) => entry.item_id)).toEqual(["b", "a"]);
  });
});

describe("Library vocabulary", () => {
  it("labels kinds per provider, including GitHub pull requests, and uses each forge's id sigil", () => {
    const pr = item({ provider_id: "github", resource_type: "review", canonical_id: "other/repo!7", title: "Fork fix" });
    const mr = item({ resource_type: "review", canonical_id: "platform/api!482", title: "Fix token refresh race" });
    expect(itemKindLabel(pr, providers)).toBe("GitHub PR");
    expect(itemTreeLabel(pr, providers)).toBe("#7 Fork fix");
    expect(itemKindLabel(mr, providers)).toBe("GitLab MR");
    expect(itemTreeLabel(mr, providers)).toBe("!482 Fix token refresh race");
    expect(itemKindLabel(item({ provider_id: "jira", canonical_id: "OPS-311" }), providers)).toBe("Jira issue");
  });

  it("reads Up to date only for fresh items", () => {
    const states: LibraryItemState[] = ["fresh", "changed", "unknown", "removed_at_source", "conflict", "failed", "partial"];
    expect(states.filter((state) => libraryStateChip(state).word === "Up to date")).toEqual(["fresh"]);
    expect(libraryStateChip("conflict")).toMatchObject({ glyph: "✎", word: "Edited in Library" });
  });

  it("turns a bare Jira key into that site's browse link and leaves links unchanged", () => {
    expect(libraryInputUrl(" OPS-311 ", providers[2])).toBe("https://jira.test/jira/browse/OPS-311");
    expect(libraryInputUrl("https://gitlab.test/platform/api/-/merge_requests/482", providers[2])).toBe("https://gitlab.test/platform/api/-/merge_requests/482");
    expect(libraryInputUrl("OPS-311", undefined)).toBe("OPS-311");
  });
});

describe("Confluence page recognition", () => {
  const withConfluence: ProjectProvider[] = [
    ...providers,
    { id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "/opt/homebrew/bin/confluence", login: "default" },
    { id: "dc", base_url: "https://confluence.example.com/confluence/", executable: "confluence", login: "dc" },
  ];
  const recognized = (input: string) => {
    const page = confluencePageInput(input, withConfluence);
    return page && { ...page, providers: page.providers.map((provider) => provider.id) };
  };

  it("reads Cloud, Data Center display and page-id links against the configured instance that contains them", () => {
    expect(recognized("https://nnexai.atlassian.net/wiki/spaces/SD/pages/98765/Release+checklist")).toEqual({ pageId: "98765", spaceKey: "SD", title: null, host: "nnexai.atlassian.net", providers: ["cloud"] });
    expect(recognized("https://confluence.example.com/confluence/display/ENG/Release+Checklist")).toEqual({ pageId: null, spaceKey: "ENG", title: "Release Checklist", host: "confluence.example.com", providers: ["dc"] });
    expect(recognized("https://confluence.example.com/confluence/pages/viewpage.action?pageId=4242")).toEqual({ pageId: "4242", spaceKey: null, title: null, host: "confluence.example.com", providers: ["dc"] });
    // Outside the configured path prefix, or on another host, no instance can read it.
    expect(recognized("https://confluence.example.com/display/ENG/Release+Checklist")?.providers).toEqual([]);
    expect(recognized("http://nnexai.atlassian.net/wiki/spaces/SD/pages/98765")?.providers).toEqual([]);
  });

  it("offers every Confluence instance for a bare id and leaves other providers' links, keys and paths alone", () => {
    expect(recognized(" 98765 ")?.providers).toEqual(["cloud", "dc"]);
    expect(recognized("https://gitlab.test/display/group/project")).toBeNull();
    expect(recognized("OPS-311")).toBeNull();
    expect(recognized("~/notes")).toBeNull();
    expect(recognized("https://nnexai.atlassian.net/wiki/spaces/SD/overview")).toBeNull();
  });
});

describe("Confluence space recognition", () => {
  const withConfluence: ProjectProvider[] = [
    ...providers,
    { id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "confluence", login: "default" },
    { id: "dc", base_url: "https://confluence.example.com/confluence/", executable: "confluence", login: "dc" },
  ];
  const recognized = (input: string, configured = withConfluence) => {
    const space = confluenceSpaceInput(input, configured);
    return space && { ...space, providers: space.providers.map((provider) => provider.id) };
  };

  it("reads space links and keys against the configured instance, leaving folders, pages and unconfigured keys alone", () => {
    expect(recognized("https://nnexai.atlassian.net/wiki/spaces/SD/overview")).toEqual({ spaceKey: "SD", host: "nnexai.atlassian.net", providers: ["cloud"] });
    expect(recognized("https://confluence.example.com/confluence/display/ENG/")).toEqual({ spaceKey: "ENG", host: "confluence.example.com", providers: ["dc"] });
    expect(recognized(" ~jdoe ")).toEqual({ spaceKey: "~jdoe", host: null, providers: ["cloud", "dc"] });
    // A bare key needs a configured Confluence provider; the home folder and a page link are not spaces.
    expect(recognized("SD", providers)).toBeNull();
    expect(recognized("~")).toBeNull();
    expect(recognized("~/notes")).toBeNull();
    expect(recognized("https://nnexai.atlassian.net/wiki/spaces/SD/pages/98765/Release")).toBeNull();
    expect(recognized("https://gitlab.test/display/group")).toBeNull();
  });
});

describe("Followed spaces in the tree", () => {
  const confluence: ProjectProvider[] = [{ id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "confluence" }];
  const follow = (overrides: Partial<LibraryFollowSummary>): LibraryFollowSummary => ({
    follow_id: "follow:sd", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", space_key: "SD", space_name: "Software Development",
    include_attachments: false, page_count: 0, partial: null, excluded_page_ids: [], last_refreshed_at: null, state: "fresh", ...overrides,
  });

  it("puts a follow's pages under its space container and lists a follow that holds no pages yet", () => {
    const page = item({ item_id: "source:h", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", resource_type: "page", canonical_id: "1", title: "Home", container: { container_id: "SD", label: "SD · Software Development" }, follow_id: "follow:sd" });
    const [instance] = libraryTree([page], confluence, [follow({ page_count: 1 }), follow({ follow_id: "follow:ops", space_key: "OPS", space_name: "Operations" })]);
    expect(instance!.label).toBe("Confluence · nnexai.atlassian.net");
    expect(instance!.containers.map((container) => [container.label, container.follow?.follow_id, container.items.map((entry) => entry.item_id)]))
      .toEqual([["OPS · Operations", "follow:ops", []], ["SD · Software Development", "follow:sd", ["source:h"]]]);
  });
});
