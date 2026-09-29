// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { LibraryFollowSummary, LibraryItemSummary, ProjectProvider } from "../../protocol/generated/v1";
import { LibraryTree } from "./LibraryTree";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const providers: ProjectProvider[] = [{ id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "confluence", login: "default" }];
const home = { id: "1", title: "Engineering home" };
const processPage = { id: "10", title: "Release process" };

function page(overrides: Partial<LibraryItemSummary>): LibraryItemSummary {
  return {
    item_id: "source:page", logical_id: "source:page", kind: "provider_snapshot", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", resource_type: "page",
    canonical_id: "0", container: { container_id: "SD", label: "SD · Software Development" }, parent_item_id: null, ancestors: [], order: null, title: "Page",
    document_path: "confluence/nnexai.atlassian.net/SD - Software Development/Page/Page.md", item_path: "confluence/nnexai.atlassian.net/SD - Software Development/Page", source_url: null, original_url: null, source_revision: "1", revision: "r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, attachments: [], folder: null, diagnostics: [],
    ...overrides,
  };
}

async function nextFrame(): Promise<void> {
  await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 40)); });
}

it("places pages under provider, space and ancestors, and gives a page with children separate expand and open targets", async () => {
  const parent = page({ item_id: "source:process", canonical_id: "10", title: "Release process", ancestors: [home], order: 2, document_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Release process/Release process.md", item_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Release process" });
  const child = page({ item_id: "source:checklist", canonical_id: "11", title: "Release checklist", ancestors: [home, processPage], order: 1, state: "changed", document_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Release process/Release checklist/Release checklist.md", item_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Release process/Release checklist" });
  // Page-tree order, not id order: a higher id sorts after its earlier sibling.
  const sibling = page({ item_id: "source:architecture", canonical_id: "50", title: "Architecture overview", ancestors: [home], order: 3, document_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Architecture overview/Architecture overview.md", item_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Architecture overview" });
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const rowLabels = () => [...host.querySelectorAll<HTMLButtonElement>("[data-library-row]")].map((row) => row.querySelector(".context-tree-name")?.textContent);
  const row = (key: string) => host.querySelector<HTMLButtonElement>(`[data-library-row="${key}"]`)!;
  const key = (target: HTMLElement, name: string) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true })); });
  // One indent formula: every row and its page chevron read `--depth` from their node.
  const depthOf = (row: HTMLElement) => row.closest<HTMLElement>(".context-tree-node")!.style.getPropertyValue("--depth");
  try {
    await act(async () => root.render(<LibraryTree items={[sibling, child, parent]} providers={providers} selectedItemId="source:checklist" pendingItemIds={new Set()} actions={actions} />));
    expect(rowLabels()).toEqual(["Confluence · nnexai.atlassian.net", "SD · Software Development", "Engineering home", "Release process", "Release checklist", "Architecture overview"]);
    expect([...host.querySelectorAll<HTMLButtonElement>("[data-library-row]")].map(depthOf)).toEqual(["0", "1", "2", "3", "4", "3"]);
    const space = [...host.querySelectorAll<HTMLButtonElement>("[data-library-row]")][1]!;
    expect(space.querySelector(".context-tree-meta")?.textContent).toBe("Pages");
    expect(row("source:checklist").getAttribute("aria-current")).toBe("true");
    expect(row("source:checklist").getAttribute("aria-label")).toBe("Release checklist, Confluence page, updated on last refresh");
    // An ancestor that isn't in the Library is a plain group; a leaf page has no chevron.
    const ancestor = [...host.querySelectorAll<HTMLButtonElement>("[data-library-row]")][2]!;
    expect(ancestor.getAttribute("aria-expanded")).toBe("true");
    expect(host.querySelectorAll(".library-page-disclosure")).toHaveLength(1);

    const chevron = host.querySelector<HTMLButtonElement>(".library-page-disclosure")!;
    expect(chevron.getAttribute("aria-label")).toBe("Expand Release process");
    expect(chevron.getAttribute("aria-expanded")).toBe("true");
    expect(chevron.tabIndex).toBe(-1);
    // The label opens the page and leaves it expanded.
    await act(async () => row("source:process").click());
    expect(actions.open).toHaveBeenCalledWith(parent);
    expect(rowLabels()).toContain("Release checklist");
    // The chevron collapses without opening, and keyboard focus lands on the page row.
    actions.open.mockClear();
    await act(async () => chevron.click());
    await nextFrame();
    expect(actions.open).not.toHaveBeenCalled();
    expect(host.querySelector(".library-page-disclosure")!.getAttribute("aria-expanded")).toBe("false");
    expect(rowLabels()).not.toContain("Release checklist");
    expect(document.activeElement).toBe(row("source:process"));

    // Keys: ArrowRight expands then enters, ArrowLeft returns and collapses, Enter opens a page node.
    await key(row("source:process"), "ArrowRight");
    expect(rowLabels()).toContain("Release checklist");
    await key(row("source:process"), "ArrowRight");
    await nextFrame();
    expect(document.activeElement).toBe(row("source:checklist"));
    await key(row("source:checklist"), "ArrowLeft");
    await nextFrame();
    expect(document.activeElement).toBe(row("source:process"));
    await key(row("source:process"), "ArrowLeft");
    expect(rowLabels()).not.toContain("Release checklist");
    await key(row("source:process"), "Enter");
    expect(actions.open).toHaveBeenCalledWith(parent);
    expect(rowLabels()).not.toContain("Release checklist");
    // Enter on a plain ancestor group toggles it instead.
    actions.open.mockClear();
    await key(ancestor, "Enter");
    expect(actions.open).not.toHaveBeenCalled();
    expect(rowLabels()).toEqual(["Confluence · nnexai.atlassian.net", "SD · Software Development", "Engineering home"]);

    // The page's menu is the item menu, and nothing was refreshed or removed along the way.
    await key(ancestor, "Enter");
    await act(async () => { row("source:process").dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true })); });
    expect([...document.body.querySelectorAll('[role="menuitem"]')].map((item) => item.textContent)).toEqual(["Open", "Refresh from source", "Copy source link", "Remove from Library…"]);
    expect(actions.refresh).not.toHaveBeenCalled();
    expect(actions.remove).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("makes a page with attachments expandable, lists attachment metadata read-only under Attachments, and still opens the page from its label", async () => {
  const attachment = (id: string, overrides: Partial<LibraryItemSummary["attachments"][number]>): LibraryItemSummary["attachments"][number] => ({
    attachment_id: id, original_name: "file", stored_name: "file", media_type: null, bytes: null, version: "1", state: "not_downloaded", relative_path: null, ...overrides,
  });
  // A page with a child page and an attachment, and a page with attachments only.
  const mixed = page({ item_id: "source:process", canonical_id: "10", title: "Release process", ancestors: [home], order: 2,
    attachments: [attachment("source:process#attachment:r1", { original_name: "runbook.pdf", stored_name: "runbook.pdf" })] });
  const child = page({ item_id: "source:checklist", canonical_id: "11", title: "Release checklist", ancestors: [home, processPage], order: 1 });
  const solo = page({ item_id: "source:architecture", canonical_id: "50", title: "Architecture overview", ancestors: [home], order: 3, document_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Architecture overview/Architecture overview.md", item_path: "confluence/nnexai.atlassian.net/SD - Software Development/Engineering home/Architecture overview",
    attachments: [
      attachment("source:architecture#attachment:a1", { original_name: "release-flow.png", stored_name: "release-flow.png", media_type: "image/png", bytes: 84_000 }),
      attachment("source:architecture#attachment:a2", { original_name: "Q3/plan?.pdf", stored_name: "Q3_plan_.pdf", media_type: "application/pdf", bytes: 1_200_000, version: "3" }),
    ] });
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const treeRows = () => [...host.querySelectorAll<HTMLElement>("[data-library-row]")];
  const rowLabels = () => treeRows().map((row) => row.querySelector(".context-tree-name")?.textContent);
  const named = (label: string) => treeRows().find((row) => row.querySelector(".context-tree-name")?.textContent === label)!;
  // The Attachments group of the last page; both pages have one, so it is not findable by name.
  const depthOf = (row: HTMLElement) => row.closest<HTMLElement>(".context-tree-node")!.style.getPropertyValue("--depth");
  const lastGroup = () => treeRows().filter((row) => row.classList.contains("is-attachments")).at(-1)!;
  const key = (target: HTMLElement, name: string, init: KeyboardEventInit = {}) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, ...init })); });
  try {
    await act(async () => root.render(<LibraryTree items={[solo, child, mixed]} providers={providers} selectedItemId={null} pendingItemIds={new Set()} actions={actions} />));
    // Child pages come first, then the page's Attachments group, folded until opened; page-tree order is kept.
    expect(rowLabels()).toEqual(["Confluence · nnexai.atlassian.net", "SD · Software Development", "Engineering home",
      "Release process", "Release checklist", "Attachments", "Architecture overview", "Attachments"]);
    const [processGroup, architectureGroup] = treeRows().filter((row) => row.classList.contains("is-attachments"));
    expect(processGroup!.getAttribute("aria-expanded")).toBe("false");
    await act(async () => processGroup!.click());
    await act(async () => architectureGroup!.click());
    expect(rowLabels()).toEqual(["Confluence · nnexai.atlassian.net", "SD · Software Development", "Engineering home",
      "Release process", "Release checklist", "Attachments", "runbook.pdf",
      "Architecture overview", "Attachments", "release-flow.png", "Q3_plan_.pdf"]);
    expect(treeRows().map(depthOf)).toEqual(["0", "1", "2", "3", "4", "4", "5", "3", "4", "5", "5"]);
    expect([...host.querySelectorAll(".library-page-disclosure")].map((chevron) => chevron.getAttribute("aria-label"))).toEqual(["Expand Release process", "Expand Architecture overview"]);

    const group = architectureGroup!;
    expect(group.getAttribute("aria-expanded")).toBe("true");
    expect(group.querySelector(".context-tree-meta")?.textContent).toBe("not downloaded");
    // Metadata only: stored name shown, original name as tooltip when it differs; size, type and state.
    const png = named("release-flow.png");
    const pdf = named("Q3_plan_.pdf");
    expect(png.querySelector(".context-tree-name")?.getAttribute("title")).toBeNull();
    expect(pdf.querySelector(".context-tree-name")?.getAttribute("title")).toBe("Q3/plan?.pdf");
    expect([png, pdf, named("runbook.pdf")].map((row) => row.querySelector(".context-tree-meta")?.textContent))
      .toEqual(["84 KB · not downloaded", "1.2 MB · not downloaded", "not downloaded"]);
    // The media type moved to the tooltip, and stays in the accessible name.
    expect(png.querySelector(".context-tree-meta")?.getAttribute("title")).toBe("image/png");
    // Read-only metadata, not a control: a named group that arrow keys reach but Tab skips, with no button or link in or around it.
    for (const row of [png, pdf, named("runbook.pdf")]) {
      expect(row.tagName).toBe("DIV");
      expect(row.getAttribute("role")).toBe("group");
      expect(row.tabIndex).toBe(-1);
      expect(row.closest("button, a")).toBeNull();
      expect(row.querySelector("button, a")).toBeNull();
    }
    expect(pdf.getAttribute("aria-label")).toBe("Q3_plan_.pdf, attachment, 1.2 MB · application/pdf · not downloaded");
    expect(pdf.hasAttribute("aria-expanded")).toBe(false);
    // No download or removal controls: the tree's only buttons are page, group and container rows and page chevrons.
    expect(host.querySelectorAll("button:not([data-library-row]):not(.library-page-disclosure), a")).toHaveLength(0);

    // The attachment-only page's label opens the page; its chevron collapses without opening.
    await act(async () => named("Architecture overview").click());
    expect(actions.open).toHaveBeenCalledWith(solo);
    actions.open.mockClear();
    const chevron = host.querySelectorAll<HTMLButtonElement>(".library-page-disclosure")[1]!;
    await act(async () => chevron.click());
    await nextFrame();
    expect(actions.open).not.toHaveBeenCalled();
    expect(rowLabels().filter((label) => label === "Attachments")).toHaveLength(1);
    expect(document.activeElement).toBe(named("Architecture overview"));

    // Keys: ArrowRight expands then enters the group, then its first attachment.
    await key(named("Architecture overview"), "ArrowRight");
    expect(rowLabels().filter((label) => label === "Attachments")).toHaveLength(2);
    await key(named("Architecture overview"), "ArrowRight");
    await nextFrame();
    expect(document.activeElement).toBe(lastGroup());
    await key(lastGroup(), "ArrowRight");
    await nextFrame();
    expect(document.activeElement).toBe(named("release-flow.png"));
    await key(named("release-flow.png"), "ArrowDown");
    await nextFrame();
    expect(document.activeElement).toBe(named("Q3_plan_.pdf"));
    // An attachment is a read-only leaf: Enter, Space, click, ArrowRight, the menu key and right-click do nothing.
    const before = rowLabels();
    await key(named("Q3_plan_.pdf"), "Enter");
    await key(named("Q3_plan_.pdf"), " ");
    await act(async () => named("Q3_plan_.pdf").click());
    await key(named("Q3_plan_.pdf"), "ArrowRight");
    await key(named("Q3_plan_.pdf"), "F10", { shiftKey: true });
    await act(async () => { named("Q3_plan_.pdf").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 20, clientY: 20 })); });
    await nextFrame();
    expect(document.activeElement).toBe(named("Q3_plan_.pdf"));
    expect(rowLabels()).toEqual(before);
    expect(document.body.querySelectorAll('[role="menuitem"]')).toHaveLength(0);
    expect(actions.open).not.toHaveBeenCalled();
    // Arrow keys still move between attachment rows.
    await key(named("Q3_plan_.pdf"), "ArrowUp");
    await nextFrame();
    expect(document.activeElement).toBe(named("release-flow.png"));
    await key(named("release-flow.png"), "ArrowDown");
    await nextFrame();
    expect(document.activeElement).toBe(named("Q3_plan_.pdf"));
    // ArrowLeft goes to the group, collapses it, and Enter reopens it.
    await key(named("Q3_plan_.pdf"), "ArrowLeft");
    await nextFrame();
    expect(document.activeElement).toBe(lastGroup());
    await key(lastGroup(), "ArrowLeft");
    expect(rowLabels()).not.toContain("release-flow.png");
    expect(lastGroup().getAttribute("aria-expanded")).toBe("false");
    await key(lastGroup(), "Enter");
    expect(rowLabels()).toContain("release-flow.png");
    await key(lastGroup(), "ArrowLeft");
    await key(lastGroup(), "ArrowLeft");
    await nextFrame();
    expect(document.activeElement).toBe(named("Architecture overview"));
    // Enter on the page opens it; its menu is the item menu, with no attachment actions yet.
    await key(named("Architecture overview"), "Enter");
    expect(actions.open).toHaveBeenCalledWith(solo);
    await key(named("Architecture overview"), "F10", { shiftKey: true });
    expect([...document.body.querySelectorAll('[role="menuitem"]')].map((item) => item.textContent)).toEqual(["Open", "Refresh from source", "Copy source link", "Remove from Library…"]);
    expect(actions.refresh).not.toHaveBeenCalled();
    expect(actions.remove).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("shows followed spaces with their partial count and folder ancestors, and refreshes, stops following and removes a space from its menu", async () => {
  const follow = (overrides: Partial<LibraryFollowSummary>): LibraryFollowSummary => ({
    follow_id: "follow:sd", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", source: { kind: "confluence_space", space_key: "SD", space_name: "Software Development" },
    include_attachments: false, item_count: 4, partial: null, excluded_ids: [], last_refreshed_at: null, state: "fresh", ...overrides,
  });
  const sd = follow({});
  // Partial: the page limit stopped enumeration at 3 of 5, so nothing is shown for this space yet.
  const ops = follow({ follow_id: "follow:ops", source: { kind: "confluence_space", space_key: "OPS", space_name: "Operations" }, item_count: 3, partial: { unit: "pages", have: 3, total: 5, reason: "page limit" } });
  const homePage = { id: "1", title: "Home" };
  const folder = { id: "900", title: "Release folder" };
  const team = { id: "30", title: "Team" };
  const followed = (overrides: Partial<LibraryItemSummary>) => page({ refs: [{ kind: "follow", follow_id: "follow:sd" }], ...overrides });
  const pages = [
    followed({ item_id: "source:home", canonical_id: "1", title: "Home", order: 1 }),
    followed({ item_id: "source:architecture", canonical_id: "20", title: "Architecture", ancestors: [homePage], order: 2 }),
    // A second top-level tree under a Cloud folder, which is not a page.
    followed({ item_id: "source:team", canonical_id: "30", title: "Team", ancestors: [folder], order: 3 }),
    followed({ item_id: "source:team-notes", canonical_id: "31", title: "Team notes", ancestors: [folder, team], order: 4 }),
  ];
  let stop!: () => void;
  const removeFollow = vi.fn((_follow: LibraryFollowSummary, mode: "stop_following" | "follow") => mode === "stop_following" ? new Promise<void>((resolve) => { stop = resolve; }) : Promise.resolve());
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false, removeFollow };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const treeRows = () => [...host.querySelectorAll<HTMLElement>("[data-library-row]")];
  const named = (label: string) => treeRows().find((row) => row.querySelector(".context-tree-name")?.textContent === label)!;
  const menuItems = () => [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
  const openMenu = (row: HTMLElement) => act(async () => { row.focus(); row.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true })); });
  const choose = (label: string) => act(async () => menuItems().find((item) => item.textContent === label)!.click());
  try {
    await act(async () => root.render(<LibraryTree items={pages} follows={[sd, ops]} providers={providers} selectedItemId={null} pendingItemIds={new Set()} actions={actions} />));
    expect(treeRows().map((row) => row.querySelector(".context-tree-name")?.textContent))
      .toEqual(["Confluence · nnexai.atlassian.net", "OPS · Operations", "SD · Software Development", "Home", "Architecture", "Release folder", "Team", "Team notes"]);
    const opsRow = named("OPS · Operations");
    expect([...opsRow.querySelectorAll(".context-tree-meta")].map((meta) => meta.textContent)).toEqual(["Following", "3 of 5"]);
    expect(opsRow.getAttribute("aria-label")).toBe("OPS · Operations, following, partial: 3 of 5 pages (page limit)");
    expect(named("SD · Software Development").querySelector(".context-tree-meta")?.textContent).toBe("Following");
    // The folder is a non-document group: it expands but never opens.
    const folderRow = named("Release folder");
    expect(folderRow.getAttribute("aria-label")).toBe("Release folder, folder");
    expect(folderRow.querySelector(".context-tree-meta")?.textContent).toBe("Folder");
    await act(async () => folderRow.click());
    expect(actions.open).not.toHaveBeenCalled();
    expect(treeRows().map((row) => row.querySelector(".context-tree-name")?.textContent)).not.toContain("Team");
    await act(async () => folderRow.click());

    await openMenu(named("SD · Software Development"));
    expect(menuItems().map((item) => item.textContent)).toEqual(["Refresh space", "Stop following", "Remove space and its items…"]);
    await choose("Refresh space");
    expect(actions.refresh).toHaveBeenCalledWith({ scope: "follow", follow_id: "follow:sd" }, ["source:home", "source:architecture", "source:team", "source:team-notes"]);

    // Stopping keeps the pages and says so once the Library accepted it.
    await openMenu(named("SD · Software Development"));
    await choose("Stop following");
    expect(removeFollow).toHaveBeenLastCalledWith(sd, "stop_following");
    expect(host.querySelector('[role="status"]')).toBeNull();
    await act(async () => stop());
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Stopped following SD. Its 4 pages stay in the Library; refresh no longer adds new pages.");

    // Removal asks first, with Cancel focused and `Stop following only` as the lesser choice.
    await openMenu(named("SD · Software Development"));
    await choose("Remove space and its items…");
    const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog.querySelector("h2")?.textContent).toBe("Remove SD · Software Development from the Library?");
    expect(dialog.textContent).toContain("Deletes the items only this space holds (up to 4 pages) from the Library and stops following it. Items you kept in the Library or that another follow holds stay.");
    expect([...dialog.querySelectorAll("footer button")].map((button) => button.textContent)).toEqual(["Cancel", "Stop following only", "Remove space"]);
    expect(document.activeElement?.textContent).toBe("Cancel");
    await act(async () => [...dialog.querySelectorAll<HTMLButtonElement>("footer button")].find((button) => button.textContent === "Remove space")!.click());
    expect(removeFollow).toHaveBeenLastCalledWith(sd, "follow");
    expect(document.body.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(named("SD · Software Development"));
    // The reread drops the space; focus moves to its provider row instead of the page body.
    await act(async () => root.render(<LibraryTree items={[]} follows={[ops]} providers={providers} selectedItemId={null} pendingItemIds={new Set()} actions={actions} />));
    expect(named("SD · Software Development")).toBeUndefined();
    expect(document.activeElement).toBe(named("Confluence · nnexai.atlassian.net"));
    expect(actions.remove).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("shows a followed Jira query as a container titled by its JQL, marks unreferenced issues, and offers Keep in Library only for issues without their own reference", async () => {
  const jira: ProjectProvider[] = [{ id: "jira", base_url: "https://jira.test", executable: "jira", login: "default" }];
  const issue = (key: string, overrides: Partial<LibraryItemSummary>) => page({
    item_id: `source:${key}`, provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue", canonical_id: key, container: { container_id: "OPS", label: "OPS" }, title: `Issue ${key}`,
    document_path: `jira/jira.test/OPS/${key}/Issue.md`, item_path: `jira/jira.test/OPS/${key}`, source_url: `https://jira.test/browse/${key}`, ...overrides,
  });
  const query: LibraryFollowSummary = {
    follow_id: "follow:q", provider_id: "jira", provider_instance: "https://jira.test", source: { kind: "jira_query", jql: "project = OPS AND updated >= -14d", mode: "accumulate" },
    include_attachments: false, item_count: 2, partial: null, excluded_ids: [], last_refreshed_at: null, state: "fresh",
  };
  const held = issue("OPS-1", { refs: [{ kind: "follow", follow_id: "follow:q" }] });
  const tombstoned = issue("OPS-2", { refs: [], purge_after: String(Date.now() + 86_400_000) });
  const own = issue("OPS-3", { refs: [{ kind: "manual" }] });
  const keep = vi.fn();
  const removeFollow = vi.fn(async () => undefined);
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false, removeFollow, keep };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const treeRows = () => [...host.querySelectorAll<HTMLElement>("[data-library-row]")];
  const named = (label: string) => treeRows().find((row) => row.querySelector(".context-tree-name")?.textContent === label)!;
  const menuItems = () => [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
  const openMenu = (row: HTMLElement) => act(async () => { row.focus(); row.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true })); });
  try {
    await act(async () => root.render(<LibraryTree items={[held, tombstoned, own]} follows={[query]} providers={jira} selectedItemId={null} pendingItemIds={new Set()} actions={actions} />));
    const names = treeRows().map((row) => row.querySelector(".context-tree-name")?.textContent);
    // The followed issue sits under the query; the unfollowed ones under their project.
    expect(names).toEqual(["Jira · jira.test", "OPS", "OPS-3 Issue OPS-3", "OPS-2 Issue OPS-2", "project = OPS AND updated >= -14d", "OPS-1 Issue OPS-1"]);
    const followRow = named("project = OPS AND updated >= -14d");
    expect([...followRow.querySelectorAll(".context-tree-meta")].map((meta) => meta.textContent)).toEqual(["Following", "2 issues · Accumulate"]);
    expect(named("OPS-2 Issue OPS-2").querySelector(".context-tree-meta")?.textContent).toBe("Unreferenced");
    expect(named("OPS-1 Issue OPS-1").querySelector(".context-tree-meta")).toBeNull();

    await openMenu(named("OPS-2 Issue OPS-2"));
    expect(menuItems().map((item) => item.textContent)).toContain("Keep in Library");
    await act(async () => menuItems().find((item) => item.textContent === "Keep in Library")!.click());
    expect(keep).toHaveBeenCalledWith(tombstoned);

    await openMenu(named("OPS-3 Issue OPS-3"));
    expect(menuItems().map((item) => item.textContent)).not.toContain("Keep in Library");
    await act(async () => menuItems().find((item) => item.textContent === "Refresh from source")!.click());

    await openMenu(followRow);
    expect(menuItems().map((item) => item.textContent)).toEqual(["Refresh query", "Stop following", "Remove query and its items…"]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("offers Provider token… on a Jira or Confluence instance row, opens it for that provider, and reads token states only when an issue's menu opens", async () => {
  const config: ProjectProvider[] = [
    { id: "jira", base_url: "https://team.atlassian.net", executable: "jira" },
    { id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "confluence", login: "default" },
    { id: "gitlab", base_url: "https://gitlab.test", executable: "glab" },
  ];
  const issue = page({
    item_id: "source:ops-1", provider_id: "jira", provider_instance: "https://team.atlassian.net", resource_type: "issue", canonical_id: "OPS-1", container: null, title: "Crash",
    attachments: [{ attachment_id: "1", original_name: "trace.log", stored_name: "trace.log", media_type: "text/plain", bytes: 10, version: null, state: "not_downloaded", relative_path: null }],
  });
  const wiki = page({ item_id: "source:wiki", canonical_id: "9", title: "Home" });
  const repo = page({ item_id: "source:gl", provider_id: "gitlab", provider_instance: "https://gitlab.test", resource_type: "issue", canonical_id: "acme/api#7", container: null, title: "Bug" });
  const credentials = { statuses: null, ensure: vi.fn(), open: vi.fn(), attachmentAccess: vi.fn((): "stored" | "needs_token" => "needs_token") };
  const attachments = { start: vi.fn(), open: vi.fn(), busy: false, active: null };
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false, credentials, attachments };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const rows = () => [...host.querySelectorAll<HTMLButtonElement>("[data-library-row]")];
  const rowNamed = (text: string) => rows().find((row) => row.querySelector(".context-tree-name")?.textContent === text)!;
  const menuItem = (text: string) => [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((item) => item.textContent === text);
  const menuItems = () => [...document.body.querySelectorAll('[role="menuitem"]')].map((item) => item.textContent);
  const rightClick = (row: HTMLElement) => act(async () => { row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 20, clientY: 20 })); });
  try {
    await act(async () => root.render(<LibraryTree items={[issue, wiki, repo]} providers={config} selectedItemId={null} pendingItemIds={new Set()} actions={actions} />));
    expect(credentials.ensure).not.toHaveBeenCalled();
    await rightClick(rowNamed("Jira · team.atlassian.net"));
    expect(menuItems()).toEqual(["Refresh all in Jira · team.atlassian.net", "Provider token…"]);
    await act(async () => menuItem("Provider token…")!.click());
    expect(credentials.open).toHaveBeenLastCalledWith("jira");
    await rightClick(rowNamed("Confluence · nnexai.atlassian.net"));
    await act(async () => menuItem("Provider token…")!.click());
    expect(credentials.open).toHaveBeenLastCalledWith("cloud");
    // A provider that can't store a token has no entry.
    await rightClick(rowNamed("GitLab · gitlab.test"));
    expect(menuItems()).toEqual(["Refresh all in GitLab · gitlab.test"]);
    await act(async () => { document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); });
    expect(credentials.ensure).not.toHaveBeenCalled();
    // A Jira issue's menu reads the token states, and without a token offers the dialog instead of a download.
    await rightClick(rows().find((row) => row.dataset.libraryRow === "source:ops-1")!);
    expect(credentials.ensure).toHaveBeenCalledTimes(1);
    expect(menuItems()).toContain("Store a token to download attachments…");
    expect(menuItems()).not.toContain("Download attachments");
    await act(async () => menuItem("Store a token to download attachments…")!.click());
    expect(credentials.open).toHaveBeenLastCalledWith("jira");
    expect(attachments.start).not.toHaveBeenCalled();
    // With a token stored the same menu downloads.
    credentials.attachmentAccess.mockReturnValue("stored");
    await rightClick(rows().find((row) => row.dataset.libraryRow === "source:ops-1")!);
    await act(async () => menuItem("Download attachments")!.click());
    expect(attachments.start).toHaveBeenCalledWith(issue, "download", ["1"]);
    // While the token states are still loading, the menu still opens (asking for them) with the download disabled.
    await act(async () => { document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); });
    credentials.attachmentAccess.mockReturnValue("loading" as never);
    credentials.ensure.mockClear();
    await rightClick(rows().find((row) => row.dataset.libraryRow === "source:ops-1")!);
    expect(credentials.ensure).toHaveBeenCalledTimes(1);
    expect(menuItem("Download attachments")?.disabled).toBe(true);
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("lists a Jira issue's attachments under an Attachments group, and leaves an issue without attachments a plain row", async () => {
  const jira: ProjectProvider[] = [{ id: "jira", base_url: "https://jira.test", executable: "jira" }];
  const issue = (key: string, attachments: LibraryItemSummary["attachments"]) => page({
    item_id: `issue:${key}`, provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue", canonical_id: key, container: { container_id: "OPS", label: "OPS" },
    title: key, document_path: `jira/jira.test/OPS/${key}/${key}.md`, item_path: `jira/jira.test/OPS/${key}`, attachments,
  });
  const file = { attachment_id: "1", original_name: "trace.log", stored_name: "trace.log", media_type: null, bytes: 2048, version: null, state: "not_downloaded" as const, relative_path: null };
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const labels = () => [...host.querySelectorAll<HTMLElement>("[data-library-row]")].map((row) => row.querySelector(".context-tree-name")?.textContent);
  try {
    await act(async () => root.render(<LibraryTree items={[issue("OPS-2", []), issue("OPS-1", [file])]} providers={jira} selectedItemId={null} pendingItemIds={new Set()} actions={actions} />));
    // The issue with attachments gets its group right after it; the other stays a leaf.
    expect(labels().slice(2)).toEqual(["OPS-2", "OPS-1", "Attachments"]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

const site = "https://nnexai.atlassian.net";
const jiraProviders: ProjectProvider[] = [
  { id: "confluence", base_url: `${site}/wiki`, executable: "confluence", login: "default" },
  { id: "jira", base_url: site, executable: "jira" },
];
const query: LibraryFollowSummary = {
  follow_id: "follow:jql", provider_id: "jira", provider_instance: site, source: { kind: "jira_query", jql: "project = SCRUM AND resolution = Unresolved ORDER BY created DESC", mode: "live" },
  include_attachments: false, item_count: 4, partial: null, excluded_ids: [], last_refreshed_at: null, state: "fresh",
} as LibraryFollowSummary;

function issue(number: number, overrides: Partial<LibraryItemSummary> = {}): LibraryItemSummary {
  return page({
    item_id: `source:SCRUM-${number}`, provider_id: "jira", provider_instance: site, resource_type: "issue", canonical_id: `SCRUM-${number}`, title: `Issue ${number}`,
    container: { container_id: "SCRUM", label: "SCRUM" }, refs: [{ kind: "follow", follow_id: query.follow_id }],
    document_path: `jira/nnexai.atlassian.net/SCRUM/SCRUM-${number}/Issue ${number}.md`, item_path: `jira/nnexai.atlassian.net/SCRUM/SCRUM-${number}`, ...overrides,
  });
}

/** SCRUM-2 has subtasks SCRUM-3 and SCRUM-4; SCRUM-1 is a plain issue; SCRUM-5 is a subtask of an issue outside the Library. */
const parent = issue(2);
const issues = [issue(1), parent, issue(3, { parent_item_id: parent.item_id }), issue(4, { parent_item_id: parent.item_id }), issue(5, { parent_item_id: "source:SCRUM-9" })];

async function renderTree(items: LibraryItemSummary[], selectedItemId: string | null = null) {
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(<LibraryTree items={items} follows={[query]} providers={jiraProviders} selectedItemId={selectedItemId} pendingItemIds={new Set()} actions={actions} />));
  const rows = () => [...host.querySelectorAll<HTMLElement>("[data-library-row]")];
  return {
    actions, host, rows,
    labels: () => rows().map((row) => row.querySelector(".context-tree-name")?.textContent),
    row: (name: string) => rows().find((row) => row.querySelector(".context-tree-name")?.textContent === name)!,
    key: (target: Element, name: string, init: KeyboardEventInit = {}) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...init })); }),
    stops: () => rows().filter((row) => row.tabIndex === 0).map((row) => row.querySelector(".context-tree-name")?.textContent),
    dispose: async () => { await act(async () => root.unmount()); host.remove(); },
  };
}

it("shows a Jira follow's query in full as its label with the state on its own line, and nests subtasks under their parent issue", async () => {
  const tree = await renderTree(issues);
  try {
    // Subtasks sit under their parent (newest first among siblings, like every issue list); an orphaned subtask stays a top-level row.
    const jql = "project = SCRUM AND resolution = Unresolved ORDER BY created DESC";
    expect(tree.labels()).toEqual(["Jira · nnexai.atlassian.net", jql, "SCRUM-5 Issue 5", "SCRUM-2 Issue 2", "SCRUM-4 Issue 4", "SCRUM-3 Issue 3", "SCRUM-1 Issue 1"]);
    const depths = tree.rows().map((row) => row.closest<HTMLElement>(".context-tree-node")!.style.getPropertyValue("--depth"));
    expect(depths).toEqual(["0", "1", "2", "2", "3", "3", "2"]);
    expect(tree.row("SCRUM-2 Issue 2").classList.contains("is-page")).toBe(true);
    expect(tree.row("SCRUM-3 Issue 3").classList.contains("is-item")).toBe(true);
    // The follow row keeps the whole query as its name, tooltip included; Following and the count are a separate line.
    const follow = tree.rows()[1]!;
    expect(follow.querySelector(".context-tree-name")?.getAttribute("title")).toBe(follow.querySelector(".context-tree-name")?.textContent);
    expect(follow.querySelector(".context-tree-name")?.textContent).toBe("project = SCRUM AND resolution = Unresolved ORDER BY created DESC");
    expect(follow.querySelector(".library-follow-line")?.textContent).toBe("Following4 issues · Live");
    // Collapsing the parent hides only its subtasks.
    await act(async () => tree.host.querySelector<HTMLButtonElement>(".library-page-disclosure")!.click());
    expect(tree.labels()).toEqual(["Jira · nnexai.atlassian.net", jql, "SCRUM-5 Issue 5", "SCRUM-2 Issue 2", "SCRUM-1 Issue 1"]);
  } finally {
    await tree.dispose();
  }
});

it("moves focus with the arrow, Home and End keys, and Left and Right walk to the parent and first child", async () => {
  const tree = await renderTree(issues);
  try {
    const focused = () => document.activeElement?.querySelector(".context-tree-name")?.textContent;
    tree.row("SCRUM-5 Issue 5").focus();
    await tree.key(tree.row("SCRUM-5 Issue 5"), "ArrowDown");
    expect(focused()).toBe("SCRUM-2 Issue 2");
    await tree.key(tree.row("SCRUM-2 Issue 2"), "ArrowDown");
    expect(focused()).toBe("SCRUM-4 Issue 4");
    await tree.key(tree.row("SCRUM-4 Issue 4"), "ArrowUp");
    expect(focused()).toBe("SCRUM-2 Issue 2");
    await tree.key(tree.row("SCRUM-2 Issue 2"), "End");
    expect(focused()).toBe("SCRUM-1 Issue 1");
    await tree.key(tree.row("SCRUM-1 Issue 1"), "ArrowDown");
    expect(focused()).toBe("SCRUM-1 Issue 1");
    await tree.key(tree.row("SCRUM-1 Issue 1"), "Home");
    expect(focused()).toBe("Jira · nnexai.atlassian.net");
    await tree.key(tree.row("SCRUM-1 Issue 1"), "ArrowUp");
    // Right on an open parent enters its first child; Left goes back to the parent, then collapses it, then walks up.
    await tree.key(tree.row("SCRUM-2 Issue 2"), "ArrowRight");
    expect(focused()).toBe("SCRUM-4 Issue 4");
    await tree.key(tree.row("SCRUM-4 Issue 4"), "ArrowLeft");
    expect(focused()).toBe("SCRUM-2 Issue 2");
    await tree.key(tree.row("SCRUM-2 Issue 2"), "ArrowLeft");
    expect(tree.labels()).not.toContain("SCRUM-4 Issue 4");
    await tree.key(tree.row("SCRUM-2 Issue 2"), "ArrowRight");
    expect(tree.labels()).toContain("SCRUM-4 Issue 4");
    await tree.key(tree.row("SCRUM-5 Issue 5"), "ArrowLeft");
    expect(focused()).toBe("project = SCRUM AND resolution = Unresolved ORDER BY created DESC");
    // Enter opens an issue but folds a group; Shift+F10 and the Menu key open the row menu with keyboard focus inside it.
    await tree.key(tree.row("SCRUM-2 Issue 2"), "Enter");
    expect(tree.actions.open).toHaveBeenCalledWith(parent);
    await tree.key(tree.row("SCRUM-2 Issue 2"), "F10", { shiftKey: true });
    expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
    expect(document.activeElement?.getAttribute("role")).toBe("menuitem");
    await act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
    await tree.key(tree.row("SCRUM-1 Issue 1"), "ContextMenu");
    expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
  } finally {
    await tree.dispose();
  }
});

it("has one tab stop: the row that held focus, else the selected row, else the first", async () => {
  const tree = await renderTree(issues, "source:SCRUM-3");
  try {
    expect(tree.stops()).toEqual(["SCRUM-3 Issue 3"]);
    tree.row("SCRUM-1 Issue 1").focus();
    await act(async () => undefined);
    expect(tree.stops()).toEqual(["SCRUM-1 Issue 1"]);
    // Folding away the row that was the tab stop (and the selected one) leaves the first row as the stop, never none.
    await act(async () => tree.row("Jira · nnexai.atlassian.net").click());
    expect(tree.stops()).toEqual(["Jira · nnexai.atlassian.net"]);
  } finally {
    await tree.dispose();
  }
  const bare = await renderTree(issues);
  try {
    expect(bare.stops()).toEqual(["Jira · nnexai.atlassian.net"]);
  } finally {
    await bare.dispose();
  }
});

it("takes a navigation key pressed with focus lost on the page back into the tree, and leaves other keys and focused elements alone", async () => {
  const tree = await renderTree(issues, "source:SCRUM-3");
  const outside = document.createElement("button");
  document.body.append(outside);
  try {
    const press = (target: EventTarget, name: string) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true })); });
    // jsdom has no layout, so the tree counts as shown only when its client rects say so.
    const layout = vi.spyOn(Element.prototype, "getClientRects").mockReturnValue([{}] as unknown as DOMRectList);
    (document.activeElement as HTMLElement | null)?.blur();
    await press(document.body, "a");
    expect(document.activeElement).toBe(document.body);
    await press(document.body, "ArrowDown");
    expect(document.activeElement).toBe(tree.row("SCRUM-3 Issue 3"));
    // Focus in another control keeps its own arrow keys.
    outside.focus();
    await press(outside, "ArrowDown");
    expect(document.activeElement).toBe(outside);
    // A tree that is not shown takes no keys.
    layout.mockReturnValue([] as unknown as DOMRectList);
    outside.blur();
    await press(document.body, "ArrowDown");
    expect(document.activeElement).toBe(document.body);
  } finally {
    vi.restoreAllMocks();
    outside.remove();
    await tree.dispose();
  }
});
