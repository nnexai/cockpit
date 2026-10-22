// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SpaceServiceStatus, SpaceSummary } from "../../protocol/generated/v1";
import { Spaces } from "./Spaces";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
beforeEach(() => {
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => { stored.set(key, value); },
    removeItem: (key: string) => { stored.delete(key); },
    clear: () => stored.clear(),
  });
});
afterEach(() => vi.unstubAllGlobals());
const space = (id: string): SpaceSummary => ({ id, label: id, number: 1, tab_count: 1, pane_count: 1, focused: false, agent_status: "idle", git: null });
const service: SpaceServiceStatus = { key: "dev", space_id: "app", pane_id: "pane", tab_id: null, title: "dev server", status: "starting", port: 5173 };
function props(): ComponentProps<typeof Spaces> {
  return {
    spaces: [space("app"), space("other")], services: new Map([["app", [service]]]), pendingServiceKey: null, onSelectService: vi.fn(),
    gitStatus: new Map(), gitEntry: () => undefined, onGitAction: vi.fn(), onDismissGitProblem: vi.fn(), onRetryGitStatus: vi.fn(),
    selectedSpaceId: "app", pendingSpaceId: null, editingId: null, busy: false, loading: false, hasSession: true, notes: [],
    tags: { ready: true, failed: false, editBlocked: null, catalog: [], bySpace: new Map(), pending: new Set(), change: vi.fn(async () => true) }, tagEditorRequest: null,
    onEdit: vi.fn(), onSelect: vi.fn(), onContext: vi.fn(), onSetup: vi.fn(), setupEnabled: true, mutate: vi.fn(() => true), focusRef: { current: null },
  };
}

it("puts service buttons in the existing roving sequence and activates only the service focus callback", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const current = props();
  const key = async (element: HTMLElement, value: string) => { await act(async () => { element.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true })); }); };
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    const rows = [...host.querySelectorAll<HTMLButtonElement>("[data-row-id]")];
    expect(rows.map(row => row.dataset.rowId)).toEqual(["app", "service:app:dev", "other"]);
    expect(rows.filter(row => row.tabIndex === 0)).toEqual([rows[0]]);
    await act(async () => rows[0].focus());
    await key(rows[0], "ArrowDown");
    expect(document.activeElement).toBe(rows[1]);
    expect(current.onSelect).not.toHaveBeenCalled();
    expect(current.onSelectService).not.toHaveBeenCalled();
    expect(rows[1].getAttribute("aria-label")).toBe("dev server, starting, port 5173");
    expect(rows[1].title).toBe("dev server, starting, port 5173. Focus pane");
    for (const value of ["ArrowLeft", "ArrowRight", "ContextMenu", "F2", "Delete"]) await key(rows[1], value);
    expect(current.onContext).not.toHaveBeenCalled();
    expect(current.onEdit).not.toHaveBeenCalled();
    expect(current.mutate).not.toHaveBeenCalled();
    expect(rows[1].draggable).toBe(false);
    await act(async () => rows[1].click());
    expect(current.onSelectService).toHaveBeenCalledWith(service);
    expect(current.onSelect).not.toHaveBeenCalled();
    await key(rows[1], "ArrowDown");
    expect(document.activeElement).toBe(rows[2]);
    await key(rows[2], "Home");
    expect(document.activeElement).toBe(rows[0]);
    await key(rows[0], "End");
    expect(document.activeElement).toBe(rows[2]);
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("returns keyboard focus to the Space when its focused service disappears, with no empty service wrapper", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const current = props();
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    const button = host.querySelector<HTMLButtonElement>('[data-row-id="service:app:dev"]')!;
    await act(async () => button.focus());
    await act(async () => root.render(createElement(Spaces, { ...current, services: new Map() })));
    expect(document.activeElement).toBe(host.querySelector('[data-row-id="app"]'));
    expect(host.querySelector(".space-service-list")).toBeNull();
    expect(host.querySelectorAll(".space-row-group")).toHaveLength(2);
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("keeps parent services visible when worktrees collapse and never promotes child services to the parent", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const parent = { ...space("app"), git: { repository_key: "repo", repository: "repo", branch: "main", checkout_path: "/repo", is_linked_worktree: false } };
  const child = { ...space("child"), git: { ...parent.git, branch: "topic", checkout_path: "/child", is_linked_worktree: true } };
  const childService = { ...service, key: "child-dev", space_id: "child", pane_id: "child-pane", title: "child server" };
  const current = { ...props(), spaces: [parent, child], services: new Map([["app", [service]], ["child", [childService]]]) };
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    expect(host.querySelectorAll(".space-service-row")).toHaveLength(2);
    await act(async () => host.querySelector<HTMLButtonElement>(".space-chevron")!.click());
    expect(host.querySelectorAll(".space-service-row")).toHaveLength(1);
    expect(host.querySelector(".space-service-name")?.textContent).toBe("dev server");
    expect(host.querySelector('[data-space-id="child"]')).toBeNull();
  } finally { await act(async () => root.unmount()); }
});

it("announces tab targets, omits absent ports, and preserves focusable busy rows without extra actions", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const tabService = { ...service, tab_id: "tab", port: null, status: "unhealthy" as const };
  const current = { ...props(), services: new Map([["app", [tabService]]]), pendingServiceKey: "dev", busy: true };
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    const button = host.querySelector<HTMLButtonElement>(".space-service-row")!;
    expect(button.getAttribute("aria-label")).toBe("dev server, unhealthy");
    expect(button.title).toBe("dev server, unhealthy. Focus tab");
    expect(button.getAttribute("aria-busy")).toBe("true");
    expect(button.disabled).toBe(false);
    expect(button.querySelector(".space-service-port")).toBeNull();
    await act(async () => button.click());
    expect(current.onSelectService).not.toHaveBeenCalled();
    expect(button.querySelectorAll("button")).toHaveLength(0);
  } finally { await act(async () => root.unmount()); }
});

it("groups duplicate Space references with unique roving identities and services on the first visible occurrence", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const current = props();
  current.tags = { ...current.tags, catalog: ["My check", "Needs testing", "Empty"], bySpace: new Map([["app", ["My check", "Needs testing"]]]) };
  const key = async (element: HTMLElement, value: string) => { await act(async () => { element.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true })); }); };
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    expect(host.querySelector(".space-tag-chip")?.textContent).toBe("My check+1");
    const switchButton = [...host.querySelectorAll<HTMLButtonElement>(".spaces-view-switch button")].find(button => button.textContent === "Tags")!;
    await act(async () => switchButton.click());
    expect(document.activeElement).toBe(switchButton);
    expect([...host.querySelectorAll(".space-tag-group-name")].map(element => element.textContent)).toEqual(["My check", "Needs testing", "Untagged"]);
    const identities = [...host.querySelectorAll<HTMLElement>("[data-row-id]")].map(element => element.dataset.rowId);
    expect(new Set(identities).size).toBe(identities.length);
    expect(host.querySelectorAll('[data-space-id="app"]')).toHaveLength(2);
    expect(host.querySelectorAll('[aria-current="true"]')).toHaveLength(1);
    expect(host.querySelectorAll(".space-service-list")).toHaveLength(1);
    expect([...host.querySelectorAll<HTMLDivElement>(".space-tree-row")].every(element => !element.draggable)).toBe(true);
    const occurrences = [...host.querySelectorAll<HTMLButtonElement>('[data-space-id="app"] .resource-select')];
    await act(async () => occurrences[1].click());
    expect(current.onSelect).toHaveBeenCalledWith(current.spaces[0]);
    await key(occurrences[1], "ContextMenu");
    expect(current.onContext).toHaveBeenLastCalledWith(expect.anything(), { kind: "space", id: "app" });
    const group = host.querySelector<HTMLButtonElement>(".space-tag-group")!;
    await act(async () => group.focus());
    await key(group, "ArrowLeft");
    expect(group.getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelectorAll('[data-space-id="app"]')).toHaveLength(1);
    expect(host.querySelectorAll(".space-service-list")).toHaveLength(1);
    expect(host.querySelector('[data-space-id="app"] .resource-select')?.getAttribute("aria-current")).toBe("true");
    expect(current.mutate).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("never projects unknown tag data as Untagged and keeps cached offline chips noneditable", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const current = props();
  window.localStorage.setItem("cockpit.sidebar.spacesView.v1", "tags");
  try {
    await act(async () => root.render(createElement(Spaces, { ...current, tags: { ...current.tags, ready: false, failed: true, editBlocked: "Tags unavailable" } })));
    expect(host.textContent).toContain("Tags unavailable");
    expect(host.querySelector(".space-tag-group")).toBeNull();
    await act(async () => root.render(createElement(Spaces, { ...current, tags: { ...current.tags, editBlocked: "Herdr is not live", catalog: ["My check"], bySpace: new Map([["app", ["My check"]]]) } })));
    expect(host.textContent).toContain("My check");
    await act(async () => host.querySelectorAll<HTMLButtonElement>(".spaces-view-switch button")[0].click());
    const chip = host.querySelector<HTMLButtonElement>(".space-tag-chip")!;
    expect(chip.getAttribute("aria-disabled")).toBe("true");
    await act(async () => chip.click());
    expect(document.querySelector(".space-tag-popover")).toBeNull();
    expect(current.tags.change).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); }
});

it("opens exact requested Space tags, restores Escape focus, and preserves the input when membership moves", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const current = props();
  window.localStorage.setItem("cockpit.sidebar.spacesView.v1", "tags");
  try {
    await act(async () => root.render(createElement(Spaces, { ...current, tagEditorRequest: { spaceId: "other", token: 1 } })));
    const input = document.querySelector<HTMLInputElement>(".space-tag-input")!;
    expect(input).not.toBeNull();
    expect(document.activeElement).toBe(input);
    expect(document.querySelector('[role="dialog"]')?.getAttribute("aria-label")).toBe("Tags for other");
    await act(async () => root.render(createElement(Spaces, { ...current, tagEditorRequest: { spaceId: "other", token: 1 }, tags: { ...current.tags, catalog: ["New tag"], bySpace: new Map([["other", ["New tag"]]]) } })));
    expect(document.querySelector(".space-tag-input")).toBe(input);
    expect(document.activeElement).toBe(input);
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.querySelector(".space-tag-popover")).toBeNull();
    expect(document.activeElement).toBe(host.querySelector('[data-space-id="other"] .resource-select'));
    expect(current.onSelect).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("uses the small no-agent dot only for Space rows and remembers the chosen view", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  const current = { ...props(), spaces: [{ ...space("app"), agent_status: "unknown" }] };
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    expect(host.querySelector(".sb-glyph.is-none circle")?.getAttribute("r")).toBe("2.75");
    expect(host.querySelector(".resource-select")?.getAttribute("aria-label")).toContain("No agent");
    await act(async () => host.querySelectorAll<HTMLButtonElement>(".spaces-view-switch button")[1].click());
    expect(window.localStorage.getItem("cockpit.sidebar.spacesView.v1")).toBe("tags");
  } finally { await act(async () => root.unmount()); }
  const second = createRoot(host);
  try {
    await act(async () => second.render(createElement(Spaces, current)));
    expect(host.querySelectorAll(".space-tag-group")).toHaveLength(1);
    expect(host.querySelectorAll(".spaces-view-switch button")[1].getAttribute("aria-pressed")).toBe("true");
  } finally { await act(async () => second.unmount()); }
});

it("keeps tag failures local after the picker closes and dismisses the row note with Escape", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const current = props();
  let finish!: (value: boolean) => void;
  current.tags = { ...current.tags, catalog: ["Review"], change: vi.fn(() => new Promise<boolean>(resolve => { finish = resolve; })) };
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    const trigger = host.querySelector<HTMLButtonElement>(".space-tag-add")!;
    await act(async () => trigger.click());
    expect(current.onSelect).not.toHaveBeenCalled();
    const checkbox = document.querySelector<HTMLInputElement>('.space-tag-popover input[type="checkbox"]')!;
    await act(async () => checkbox.click());
    expect(current.tags.change).toHaveBeenCalledExactlyOnceWith("app", "Review", true);
    const input = document.querySelector<HTMLInputElement>(".space-tag-input")!;
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(document.activeElement).toBe(trigger);
    await act(async () => finish(false));
    expect(host.querySelector(".row-note")?.getAttribute("role")).toBe("status");
    const row = host.querySelector<HTMLButtonElement>('[data-row-id="app"]')!;
    await act(async () => { row.focus(); row.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(host.querySelector(".row-note")).toBeNull();
    expect(document.activeElement).toBe(row);
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("expands a collapsed tag group for an exact Space editor request", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const current = props();
  current.tags = { ...current.tags, catalog: ["Review"], bySpace: new Map([["app", ["Review"]]]) };
  window.localStorage.setItem("cockpit.sidebar.spacesView.v1", "tags");
  try {
    await act(async () => root.render(createElement(Spaces, current)));
    await act(async () => host.querySelector<HTMLButtonElement>(".space-tag-group")!.click());
    expect(host.querySelector('[data-space-id="app"]')).toBeNull();
    await act(async () => root.render(createElement(Spaces, { ...current, tagEditorRequest: { spaceId: "app", token: 1 } })));
    expect(host.querySelector('[data-space-id="app"]')).not.toBeNull();
    expect(document.querySelector('[role="dialog"]')?.getAttribute("aria-label")).toBe("Tags for app");
    expect(document.activeElement).toBe(document.querySelector(".space-tag-input"));
  } finally { await act(async () => root.unmount()); host.remove(); }
});
