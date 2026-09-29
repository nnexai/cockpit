import { describe, expect, it } from "vitest";
import type { LibraryFollowSummary, LibraryItemState, LibraryItemSummary, ProjectProvider, ProviderCredentialStatus } from "../../protocol/generated/v1";
import { attachmentSummary, attachmentTreeMeta, confluencePageInput, confluenceSpaceInput, formatAgo, formatDateTime, itemKindLabel, itemTreeLabel, jiraAttachmentAccess, jiraQueryInput, jiraQueryPresets, jiraQueryProject, libraryFreshness, libraryInputUrl, libraryStateChip, libraryTree, lookupFailure, nestUnderParents, parseLibraryTime, relativeTime, sourceEditPhrase, timeDetail } from "./libraryState";

const providers: ProjectProvider[] = [
  { id: "gitlab", base_url: "https://gitlab.test", executable: "/usr/bin/glab" },
  { id: "github", base_url: "https://github.com", executable: "gh" },
  { id: "jira", base_url: "https://jira.test/jira/", executable: "jira" },
];

function item(overrides: Partial<LibraryItemSummary>): LibraryItemSummary {
  return {
    item_id: "source:x", logical_id: "source:x", kind: "provider_snapshot", provider_id: "gitlab", provider_instance: "https://gitlab.test", resource_type: "issue",
    canonical_id: "platform/api#1", container: null, parent_item_id: null, ancestors: [], order: null, title: "Title",
    document_path: "gitlab/gitlab.test/platform/api/issues/1/Title.md", item_path: "gitlab/gitlab.test/platform/api/issues/1", source_url: null, original_url: null, source_revision: null, revision: "r",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, attachments: [], folder: null, diagnostics: [],
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
    expect(libraryStateChip("conflict")).toMatchObject({ shape: "edit", word: "Edited in Library" });
  });

  it("turns a bare Jira key into that site's browse link and leaves links unchanged", () => {
    expect(libraryInputUrl(" OPS-311 ", providers[2])).toBe("https://jira.test/jira/browse/OPS-311");
    expect(libraryInputUrl("https://gitlab.test/platform/api/-/merge_requests/482", providers[2])).toBe("https://gitlab.test/platform/api/-/merge_requests/482");
    expect(libraryInputUrl("OPS-311", undefined)).toBe("OPS-311");
  });
});

// The Library index writes epoch milliseconds; provider frontmatter writes ISO. Both must display.
describe("Library timestamps", () => {
  const EPOCH_MS = "1790517466502";
  const ISO = "2026-09-27T11:34:44.043Z";
  const now = Date.parse("2026-09-27T14:11:00Z");

  it("parses epoch-millisecond strings and ISO, and nothing else", () => {
    expect(parseLibraryTime(EPOCH_MS)).toBe(1790517466502);
    expect(parseLibraryTime(ISO)).toBe(1790508884043);
    expect(parseLibraryTime("nope")).toBeNull();
    expect(parseLibraryTime("")).toBeNull();
    expect(parseLibraryTime(null)).toBeNull();
    expect(Date.parse(EPOCH_MS)).toBeNaN();
  });

  it("formats an epoch-ms time like the same instant as ISO, inline and in details", () => {
    const instant = new Date(1790517466502).toISOString();
    expect(relativeTime(EPOCH_MS, now)).toBe(relativeTime(instant, now));
    expect(relativeTime(EPOCH_MS, now)).toBe("13 min ago");
    const detail = timeDetail(EPOCH_MS, now)!;
    expect(detail).toEqual({ text: formatDateTime(1790517466502), ago: "13 min ago", iso: instant });
    expect(detail.text).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    expect(timeDetail(ISO, now)).toMatchObject({ iso: ISO, ago: "3 h ago" });
    expect(timeDetail("nope", now)).toBeNull();
  });

  it("reads recent times relatively and older ones as dates, never in the future", () => {
    const at = (offsetMs: number) => formatAgo(now - offsetMs, now);
    const minute = 60_000;
    const day = 24 * 60 * minute;
    expect(at(-5 * minute)).toBe("just now");
    expect(at(30_000)).toBe("just now");
    expect(at(59 * minute)).toBe("59 min ago");
    expect(at(5 * 60 * minute)).toBe("5 h ago");
    expect(at(13 * day)).toBe("13 d ago");
    expect(at(15 * day)).toBe(new Date(now - 15 * day).toLocaleDateString(undefined, { day: "numeric", month: "short" }));
    const lastYear = Date.parse("2025-03-05T12:00:00Z");
    expect(formatAgo(lastYear, now)).toBe(new Date(lastYear).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }));
    expect(formatAgo(lastYear, now)).toContain("2025");
  });

  it("shows the Cockpit clock as a time, or omits it, but never bare", () => {
    const base = item({ state: "fresh" });
    expect(libraryFreshness({ ...base, checked_at: EPOCH_MS }, now).phrase).toBe("Checked 13 min ago");
    expect(libraryFreshness({ ...base, checked_at: null, fetched_at: EPOCH_MS }, now).phrase).toBe("Checked 13 min ago");
    expect(libraryFreshness({ ...base, checked_at: "garbage" }, now).phrase).toBe("");
    expect(libraryFreshness({ ...item({ state: "removed_at_source" }), checked_at: EPOCH_MS }, now).notice).toMatch(/^Not found at source on .+\. The Library copy is kept\.$/);
  });

  it("keeps the source clock apart from the Cockpit clock", () => {
    expect(sourceEditPhrase({ version: "v3", editedAt: ISO, by: "Konni Hartmann" }, now)).toBe("v3 edited 3 h ago by Konni Hartmann");
    expect(sourceEditPhrase({ version: "v3", editedAt: "nope", by: null }, now)).toBe("v3");
    expect(sourceEditPhrase({ version: null, editedAt: null, by: null }, now)).toBeNull();
  });
});

describe("Attachment summaries", () => {
  const attachments = (...states: LibraryItemSummary["attachments"][number]["state"][]) => ({
    attachments: states.map((state, index) => ({ attachment_id: `a${index}`, original_name: "f", stored_name: "f", media_type: null, bytes: null, version: "1", state, relative_path: null })),
  });

  it("reads the header control and the tree meta from the same counts", () => {
    expect(attachmentSummary(attachments("not_downloaded", "not_downloaded"))).toBe("2 attachments");
    expect(attachmentSummary(attachments("downloaded"))).toBe("1 attachment · downloaded");
    expect(attachmentSummary(attachments("downloaded", "downloaded"))).toBe("2 attachments · all downloaded");
    expect(attachmentSummary(attachments("downloaded", "failed", "over_limit"))).toBe("3 attachments · 1 downloaded");
    expect(attachmentTreeMeta(attachments("not_downloaded"))).toBe("not downloaded");
    expect(attachmentTreeMeta(attachments("downloaded"))).toBe("1 downloaded");
    expect(attachmentTreeMeta(attachments("downloaded", "failed", "over_limit"))).toBe("1 of 3 downloaded");
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
    follow_id: "follow:sd", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", source: { kind: "confluence_space", space_key: "SD", space_name: "Software Development" },
    include_attachments: false, item_count: 0, partial: null, excluded_ids: [], last_refreshed_at: null, state: "fresh", ...overrides,
  });

  it("puts a follow's pages under its space container and lists a follow that holds no pages yet", () => {
    const page = item({ item_id: "source:h", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", resource_type: "page", canonical_id: "1", title: "Home", container: { container_id: "SD", label: "SD · Software Development" }, refs: [{ kind: "follow", follow_id: "follow:sd" }] });
    const [instance] = libraryTree([page], confluence, [follow({ item_count: 1 }), follow({ follow_id: "follow:ops", source: { kind: "confluence_space", space_key: "OPS", space_name: "Operations" } })]);
    expect(instance!.label).toBe("Confluence · nnexai.atlassian.net");
    expect(instance!.containers.map((container) => [container.label, container.follow?.follow_id, container.items.map((entry) => entry.item_id)]))
      .toEqual([["OPS · Operations", "follow:ops", []], ["SD · Software Development", "follow:sd", ["source:h"]]]);
  });

  it("puts a Jira issue under the first query in follows order that holds it, and under its project when none does", () => {
    const query = (id: string, jql: string): LibraryFollowSummary => follow({ follow_id: id, provider_id: "jira", provider_instance: "https://jira.test/jira", source: { kind: "jira_query", jql, mode: "live" } });
    const issue = (key: string, refs: LibraryItemSummary["refs"]) => item({ item_id: `source:${key}`, provider_id: "jira", provider_instance: "https://jira.test/jira", resource_type: "issue", canonical_id: key, container: { container_id: "OPS", label: "OPS" }, refs });
    const [instance] = libraryTree(
      [issue("OPS-1", [{ kind: "follow", follow_id: "follow:b" }, { kind: "follow", follow_id: "follow:a" }]), issue("OPS-2", [{ kind: "manual" }])],
      providers, [query("follow:a", "project = OPS"), query("follow:b", "assignee = currentUser()")],
    );
    expect(instance!.containers.map((container) => [container.label, container.follow?.follow_id ?? null, container.items.map((entry) => entry.item_id)]))
      .toEqual([["assignee = currentUser()", "follow:b", []], ["OPS", null, ["source:OPS-2"]], ["project = OPS", "follow:a", ["source:OPS-1"]]]);
  });
});

describe("Jira query input", () => {
  it("reads a bare project key or JQL operators, but not links, issue keys or folder paths, and needs a Jira provider", () => {
    expect(jiraQueryInput("OPS", providers)).toMatchObject({ jql: "OPS", bare: true });
    expect(jiraQueryInput("project = OPS AND updated >= -7d", providers)).toMatchObject({ bare: false });
    expect(jiraQueryInput("status in (Open, Blocked)", providers)?.providers.map((provider) => provider.id)).toEqual(["jira"]);
    for (const input of ["OPS-311", "https://jira.test/jira/browse/OPS-311", "~/notes", "/work/a=b", "ops", "", "just words"]) expect(jiraQueryInput(input, providers)).toBeNull();
    expect(jiraQueryInput("OPS", providers.filter((provider) => provider.id !== "jira"))).toBeNull();
  });

  it("offers project presets for a bare key or a project query, and only the personal one otherwise", () => {
    expect(jiraQueryPresets("OPS").map((preset) => preset.jql)).toEqual(["project = OPS", "project = OPS AND statusCategory != Done", "project = OPS AND updated >= -14d", "assignee = currentUser() AND resolution = Unresolved"]);
    expect(jiraQueryPresets(null).map((preset) => preset.jql)).toEqual(["assignee = currentUser() AND resolution = Unresolved"]);
    expect([jiraQueryProject("OPS"), jiraQueryProject("project = OPS AND updated >= -14d"), jiraQueryProject("assignee = x")]).toEqual(["OPS", "OPS", null]);
  });
});

describe("provider token entry points", () => {
  const jiraIssue = item({ provider_id: "jira", provider_instance: "https://jira.test/jira", resource_type: "issue", canonical_id: "OPS-1" });
  const status = (state: ProviderCredentialStatus["state"]): ProviderCredentialStatus => ({ provider_id: "jira", state, kind: state === "stored" ? "bearer" : null, supported_kinds: ["bearer", "basic"] });

  it("gates a Jira issue's downloads on its provider's stored token, and leaves other items alone", () => {
    expect(jiraAttachmentAccess(jiraIssue, providers, null)).toBe("loading");
    expect(jiraAttachmentAccess(jiraIssue, providers, [status("stored")])).toBe("stored");
    for (const state of ["not_stored", "vault_unavailable", "unsupported"] as const) expect(jiraAttachmentAccess(jiraIssue, providers, [status(state)])).toBe("needs_token");
    expect(jiraAttachmentAccess(jiraIssue, providers, [])).toBe("needs_token");
    expect(jiraAttachmentAccess(item({}), providers, null)).toBeNull();
    expect(jiraAttachmentAccess({ ...jiraIssue, resource_type: "page" }, providers, null)).toBeNull();
    expect(jiraAttachmentAccess({ ...jiraIssue, kind: "folder_copy" }, providers, null)).toBeNull();
  });

  it("offers the token dialog for Jira and Confluence credential failures only", () => {
    const confluence: ProjectProvider = { id: "wiki", base_url: "https://wiki.test", executable: "confluence", login: "default" };
    const all = [...providers, confluence];
    const jira = providers[2]!;
    const rejected = (code: string) => Object.assign(new Error("Refused by the host."), { code });
    expect(lookupFailure(rejected("source_credential_required"), "OPS-1", all, jira)).toMatchObject({ title: "✕ A token is needed", detail: "Refused by the host.", credentialProviderId: "jira" });
    expect(lookupFailure(rejected("source_auth_required"), "OPS-1", all, jira)).toMatchObject({ title: "✕ Jira sign-in required", credentialProviderId: "jira" });
    expect(lookupFailure(rejected("source_auth_failed"), "12345", all, confluence)).toMatchObject({ title: "✕ Confluence sign-in failed", credentialProviderId: "wiki" });
    // GitLab has no stored token: the failure points at the CLI and offers no dialog.
    const gitlab = lookupFailure(rejected("source_auth_failed"), "https://gitlab.test/platform/api/-/issues/1", all);
    expect(gitlab.credentialProviderId).toBeUndefined();
    expect(gitlab.detail).toBe("gitlab.test rejected the glab CLI's credentials. Sign in with the CLI, then retry.");
  });
});

describe("Jira and Confluence on one host", () => {
  const site = "https://nnexai.atlassian.net";
  const shared: ProjectProvider[] = [
    { id: "confluence", base_url: `${site}/wiki`, executable: "confluence", login: "default" },
    { id: "jira", base_url: site, executable: "jira" },
  ];
  const query: LibraryFollowSummary = {
    follow_id: "follow:jql", provider_id: "jira", provider_instance: site, source: { kind: "jira_query", jql: "resolution = Unresolved", mode: "live" },
    include_attachments: false, item_count: 2, partial: null, excluded_ids: [], last_refreshed_at: null, state: "fresh", reference_depth: 1,
  } as LibraryFollowSummary;
  const issue = (key: string) => item({ item_id: `source:${key}`, provider_id: "jira", provider_instance: site, resource_type: "issue", canonical_id: key, container: { container_id: "SCRUM", label: "SCRUM" }, refs: [{ kind: "follow", follow_id: query.follow_id }] });
  // A page the query pulled in through a followed reference: it holds the query's follow reference too.
  const page = item({ item_id: "source:page", provider_id: "confluence", provider_instance: `${site}/wiki`, resource_type: "page", canonical_id: "2621441", title: "Reference page", container: { container_id: "SD", label: "SD · Software Development" }, refs: [{ kind: "follow", follow_id: query.follow_id }] });

  it("keeps a Jira follow under its own provider and never under the Confluence instance on the same host", () => {
    const tree = libraryTree([issue("SCRUM-1"), page, issue("SCRUM-2")], shared, [query]);
    expect(tree.map((instance) => [instance.label, instance.providerId, instance.containers.map((container) => [container.label, container.follow?.follow_id ?? null, container.items.map((entry) => entry.item_id)])])).toEqual([
      ["Confluence · nnexai.atlassian.net", "confluence", [["SD · Software Development", null, ["source:page"]]]],
      ["Jira · nnexai.atlassian.net", "jira", [["resolution = Unresolved", "follow:jql", ["source:SCRUM-2", "source:SCRUM-1"]]]],
    ]);
  });

  it("names the provider by path when a typed link's host serves both", () => {
    const failure = (input: string) => lookupFailure({ code: "source_auth_failed", message: "no" }, input, shared);
    expect(failure(`${site}/browse/SCRUM-1`).credentialProviderId).toBe("jira");
    expect(failure(`${site}/wiki/spaces/SD/pages/2621441/Page`).credentialProviderId).toBe("confluence");
  });
});

describe("nesting items under their parent", () => {
  const node = (id: string, parent: string | null) => item({ item_id: id, parent_item_id: parent, title: id });
  const shape = (nodes: ReturnType<typeof nestUnderParents>): unknown[] => nodes.map((entry) => entry.children.length ? [entry.item.item_id, shape(entry.children)] : entry.item.item_id);

  it("nests children under their parent in list order, and keeps a child whose parent is elsewhere top-level", () => {
    expect(shape(nestUnderParents([node("p", null), node("c2", "p"), node("grandchild", "c2"), node("c1", "p"), node("orphan", "not-in-list"), node("solo", null)])))
      .toEqual([["p", [["c2", ["grandchild"]], "c1"]], "orphan", "solo"]);
  });

  it("lists a parent's children oldest key first while top-level issues stay newest first", () => {
    const keyed = (key: string, parent: string | null) => item({ item_id: key, canonical_id: key, parent_item_id: parent, title: key, provider_id: "jira" });
    // The list is in tree order already: newest key first.
    expect(shape(nestUnderParents([keyed("OPS-9", null), keyed("OPS-7", "OPS-2"), keyed("OPS-4", "OPS-2"), keyed("OPS-10", "OPS-2"), keyed("OPS-2", null), keyed("OPS-1", null)], providers)))
      .toEqual(["OPS-9", ["OPS-2", ["OPS-4", "OPS-7", "OPS-10"]], "OPS-1"]);
  });

  it("keeps other providers' child order, including numeric Confluence page ids", () => {
    const configured = [...providers, { id: "confluence", base_url: "https://wiki.test", executable: "confluence" }];
    for (const [provider_id, resource_type] of [["gitlab", "issue"], ["confluence", "page"]]) {
      const keyed = (key: string, parent: string | null) => item({ item_id: key, canonical_id: key, parent_item_id: parent, provider_id, resource_type });
      expect(shape(nestUnderParents([keyed("2", null), keyed("10", "2"), keyed("4", "2")], configured)))
        .toEqual([["2", ["10", "4"]]]);
    }
  });

  it("recognizes Jira through its executable when the provider has a custom id", () => {
    const keyed = (key: string, parent: string | null) => item({ item_id: key, canonical_id: key, parent_item_id: parent, provider_id: "company" });
    expect(shape(nestUnderParents([keyed("OPS-2", null), keyed("OPS-10", "OPS-2"), keyed("OPS-4", "OPS-2")],
      [{ id: "company", base_url: "https://jira.test", executable: "/usr/bin/jira" }])))
      .toEqual([["OPS-2", ["OPS-4", "OPS-10"]]]);
  });

  it("does not lose items in a parent loop or a self-parent", () => {
    expect(shape(nestUnderParents([node("a", "b"), node("b", "a"), node("self", "self"), node("tail", "a")]))).toEqual([["a", ["tail"]], "b", "self"]);
  });
});
