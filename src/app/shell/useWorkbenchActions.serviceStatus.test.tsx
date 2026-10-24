// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { PaneSummary, SpaceServiceStatus, TabSummary } from "../../protocol/generated/v1";
import type { WorkbenchProps } from "./Workbench";
import type { WorkbenchState } from "./useWorkbenchState";
import { useWorkbenchActions, type WorkbenchActions } from "./useWorkbenchActions";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const pane: PaneSummary = { id: "p1", terminal_id: "terminal", space_id: "s1", tab_id: "t1", title: null, focused: false, agent: null, agent_status: "idle", revision: 0 };
const tab: TabSummary = { id: "t1", space_id: "s1", label: "service", number: 1, pane_count: 2, focused: false, focused_pane_id: "p2" };
const service: SpaceServiceStatus = { key: "dev", pane_id: "p1", tab_id: null, title: "dev server", status: "healthy", port: 5173 };

it("focuses the exact service pane for both pane and tab entries despite another remembered leaf", async () => {
  const onFocus = vi.fn();
  const leave = vi.fn();
  const drawerFocusTarget = { current: null };
  const props = {
    state: { sync: "live" }, onFocus,
    ctx: { getState: () => ({ tabs: { t1: { selectedLeafId: "remembered", zoomLeafId: null } } }) },
  } as unknown as WorkbenchProps;
  const runtime = {
    snapshot: { panes: [pane], tabs: [tab], spaces: [{ id: "s1" }], focused_tab_id: "elsewhere" }, allTabs: [tab], modalOpen: false, mutationBusy: false,
    narrowViewport: true, drawerFocusTarget, attachedPaneIds: { current: new Set(["p2"]) }, leave,
  } as unknown as WorkbenchState;
  let actions!: WorkbenchActions;
  function Probe() { actions = useWorkbenchActions(props, runtime); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    await act(async () => actions.focusService(service));
    expect(leave).toHaveBeenCalledWith("focus");
    expect(drawerFocusTarget.current).toEqual({ spaceId: "s1", paneId: "p1" });
    expect(onFocus).toHaveBeenLastCalledWith({ kind: "pane", target_id: "p1" }, { spaceId: "s1", tabId: "t1", paneId: "p1" });
    await act(async () => actions.focusService({ ...service, tab_id: "t1" }));
    expect(onFocus).toHaveBeenLastCalledWith({ kind: "pane", target_id: "p1" }, { spaceId: "s1", tabId: "t1", paneId: "p1" });
  } finally { await act(async () => root.unmount()); }
});

it("does not focus stale service identities or dispatch while modal, mutating, or disconnected", async () => {
  const onFocus = vi.fn();
  const leave = vi.fn();
  const props = { state: { sync: "live" }, onFocus } as unknown as WorkbenchProps;
  const runtime = { snapshot: { panes: [pane], tabs: [tab], spaces: [{ id: "s1" }] }, allTabs: [tab], modalOpen: false, mutationBusy: false, leave } as unknown as WorkbenchState;
  let actions!: WorkbenchActions;
  function Probe() { actions = useWorkbenchActions(props, runtime); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    for (const stale of [{ ...service, pane_id: "dead" }, { ...service, tab_id: "other" }]) {
      await act(async () => actions.focusService(stale));
    }
    runtime.snapshot = { ...runtime.snapshot!, tabs: [{ ...tab, space_id: "other" }] };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => actions.focusService(service));
    runtime.snapshot = { ...runtime.snapshot!, tabs: [tab], spaces: [] };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => actions.focusService(service));
    runtime.snapshot = { ...runtime.snapshot!, spaces: [{ id: "s1" }] } as WorkbenchState["snapshot"];
    runtime.modalOpen = true;
    await act(async () => root.render(createElement(Probe)));
    await act(async () => actions.focusService(service));
    runtime.modalOpen = false;
    runtime.mutationBusy = true;
    await act(async () => root.render(createElement(Probe)));
    await act(async () => actions.focusService(service));
    runtime.mutationBusy = false;
    props.state.sync = "disconnected";
    await act(async () => root.render(createElement(Probe)));
    await act(async () => actions.focusService(service));
    expect(onFocus).not.toHaveBeenCalled();
    expect(leave).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); }
});
