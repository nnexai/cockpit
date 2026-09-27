// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { LibraryItemSummary, ProjectProvider } from "../../protocol/generated/v1";
import { LibraryTree } from "./LibraryTree";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const providers: ProjectProvider[] = [{ id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "confluence", login: "default" }];
const home = { id: "1", title: "Engineering home" };
const processPage = { id: "10", title: "Release process" };

function page(overrides: Partial<LibraryItemSummary>): LibraryItemSummary {
  return {
    item_id: "source:page", logical_id: "source:page", kind: "provider_snapshot", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", resource_type: "page",
    canonical_id: "0", container: { container_id: "SD", label: "SD · Software Development" }, parent_item_id: null, ancestors: [], order: null, title: "Page",
    document_path: "pages/page/document.md", item_path: "pages/page", source_url: null, original_url: null, source_revision: "1", revision: "r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, follow_id: null, attachments: [], folder: null, diagnostics: [],
    ...overrides,
  };
}

async function nextFrame(): Promise<void> {
  await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 40)); });
}

it("places pages under provider, space and ancestors, and gives a page with children separate expand and open targets", async () => {
  const parent = page({ item_id: "source:process", canonical_id: "10", title: "Release process", ancestors: [home], order: 2, document_path: "pages/process/document.md" });
  const child = page({ item_id: "source:checklist", canonical_id: "11", title: "Release checklist", ancestors: [home, processPage], order: 1, state: "changed", document_path: "pages/checklist/document.md" });
  // Page-tree order, not id order: a higher id sorts after its earlier sibling.
  const sibling = page({ item_id: "source:architecture", canonical_id: "50", title: "Architecture overview", ancestors: [home], order: 3, document_path: "pages/architecture/document.md" });
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const rowLabels = () => [...host.querySelectorAll<HTMLButtonElement>("[data-library-row]")].map((row) => row.querySelector(".context-tree-name")?.textContent);
  const row = (key: string) => host.querySelector<HTMLButtonElement>(`[data-library-row="${key}"]`)!;
  const key = (target: HTMLElement, name: string) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true })); });
  try {
    await act(async () => root.render(<LibraryTree items={[sibling, child, parent]} providers={providers} selectedItemId="source:checklist" pendingItemIds={new Set()} actions={actions} />));
    expect(rowLabels()).toEqual(["Confluence · nnexai.atlassian.net", "SD · Software Development", "Engineering home", "Release process", "Release checklist", "Architecture overview"]);
    expect([...host.querySelectorAll<HTMLButtonElement>("[data-library-row]")].map((button) => button.style.paddingLeft)).toEqual(["8px", "24px", "40px", "56px", "72px", "56px"]);
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

it("makes a page with attachments expandable, lists attachment metadata read-only under Attachments (N), and still opens the page from its label", async () => {
  const attachment = (id: string, overrides: Partial<LibraryItemSummary["attachments"][number]>): LibraryItemSummary["attachments"][number] => ({
    attachment_id: id, original_name: "file", stored_name: "file", media_type: null, bytes: null, version: "1", state: "not_downloaded", relative_path: null, ...overrides,
  });
  // A page with a child page and an attachment, and a page with attachments only.
  const mixed = page({ item_id: "source:process", canonical_id: "10", title: "Release process", ancestors: [home], order: 2,
    attachments: [attachment("source:process#attachment:r1", { original_name: "runbook.pdf", stored_name: "runbook.pdf" })] });
  const child = page({ item_id: "source:checklist", canonical_id: "11", title: "Release checklist", ancestors: [home, processPage], order: 1 });
  const solo = page({ item_id: "source:architecture", canonical_id: "50", title: "Architecture overview", ancestors: [home], order: 3, document_path: "pages/architecture/document.md",
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
  const key = (target: HTMLElement, name: string, init: KeyboardEventInit = {}) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, ...init })); });
  try {
    await act(async () => root.render(<LibraryTree items={[solo, child, mixed]} providers={providers} selectedItemId={null} pendingItemIds={new Set()} actions={actions} />));
    // Child pages come first, then the page's Attachments group; page-tree order is kept.
    expect(rowLabels()).toEqual(["Confluence · nnexai.atlassian.net", "SD · Software Development", "Engineering home",
      "Release process", "Release checklist", "Attachments (1)", "runbook.pdf",
      "Architecture overview", "Attachments (2)", "release-flow.png", "Q3_plan_.pdf"]);
    expect(treeRows().map((row) => row.style.paddingLeft)).toEqual(["8px", "24px", "40px", "56px", "72px", "72px", "88px", "56px", "72px", "88px", "88px"]);
    expect([...host.querySelectorAll(".library-page-disclosure")].map((chevron) => chevron.getAttribute("aria-label"))).toEqual(["Expand Release process", "Expand Architecture overview"]);

    const group = named("Attachments (2)");
    expect(group.getAttribute("aria-expanded")).toBe("true");
    expect(group.querySelector(".context-tree-meta")?.textContent).toBe("0 downloaded");
    // Metadata only: stored name shown, original name as tooltip when it differs; size, type and state.
    const png = named("release-flow.png");
    const pdf = named("Q3_plan_.pdf");
    expect(png.querySelector(".context-tree-name")?.getAttribute("title")).toBeNull();
    expect(pdf.querySelector(".context-tree-name")?.getAttribute("title")).toBe("Q3/plan?.pdf");
    expect([png, pdf, named("runbook.pdf")].map((row) => row.querySelector(".context-tree-meta")?.textContent))
      .toEqual(["84 KB · image/png · not downloaded", "1.2 MB · application/pdf · not downloaded", "not downloaded"]);
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
    expect(rowLabels()).not.toContain("Attachments (2)");
    expect(document.activeElement).toBe(named("Architecture overview"));

    // Keys: ArrowRight expands then enters the group, then its first attachment.
    await key(named("Architecture overview"), "ArrowRight");
    expect(rowLabels()).toContain("Attachments (2)");
    await key(named("Architecture overview"), "ArrowRight");
    await nextFrame();
    expect(document.activeElement).toBe(named("Attachments (2)"));
    await key(named("Attachments (2)"), "ArrowRight");
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
    expect(document.activeElement).toBe(named("Attachments (2)"));
    await key(named("Attachments (2)"), "ArrowLeft");
    expect(rowLabels()).not.toContain("release-flow.png");
    expect(named("Attachments (2)").getAttribute("aria-expanded")).toBe("false");
    await key(named("Attachments (2)"), "Enter");
    expect(rowLabels()).toContain("release-flow.png");
    await key(named("Attachments (2)"), "ArrowLeft");
    await key(named("Attachments (2)"), "ArrowLeft");
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
