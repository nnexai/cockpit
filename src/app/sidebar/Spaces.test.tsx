// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { SpaceServiceStatus, SpaceSummary } from "../../protocol/generated/v1";
import { Spaces } from "./Spaces";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const space = (id: string): SpaceSummary => ({ id, label: id, number: 1, tab_count: 1, pane_count: 1, focused: false, agent_status: "idle", git: null });
const service: SpaceServiceStatus = { key: "dev", space_id: "app", pane_id: "pane", tab_id: null, title: "dev server", status: "starting", port: 5173 };
function props(): ComponentProps<typeof Spaces> {
  return {
    spaces: [space("app"), space("other")], services: new Map([["app", [service]]]), pendingServiceKey: null, onSelectService: vi.fn(),
    gitStatus: new Map(), gitEntry: () => undefined, onGitAction: vi.fn(), onDismissGitProblem: vi.fn(), onRetryGitStatus: vi.fn(),
    selectedSpaceId: "app", pendingSpaceId: null, editingId: null, busy: false, loading: false, hasSession: true, notes: [],
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
