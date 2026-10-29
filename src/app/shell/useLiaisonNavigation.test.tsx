// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { SessionSnapshotResponse } from "../../protocol/generated/v1";
import { initialSessionState, sessionReducer, type SessionState } from "../session/sessionStore";
import type { WorkbenchProps } from "./Workbench";
import { useLiaisonNavigation, type LiaisonNavigation } from "./useLiaisonNavigation";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const roots: Root[] = [];
afterEach(async () => { for (const root of roots.splice(0)) await act(async () => root.unmount()); });

const snapshot: SessionSnapshotResponse = {
  session_id: "session", server_instance: "server", version: "1", protocol: 22,
  focused_space_id: "space", focused_tab_id: "tab", focused_pane_id: "origin",
  spaces: [], agents: [],
  tabs: [{ id: "tab", space_id: "space", label: "tab", number: 1, pane_count: 2, focused: true, focused_pane_id: "origin" }],
  panes: ["origin", "liaison"].map(id => ({ id, terminal_id: id, space_id: "space", tab_id: "tab", title: null, focused: id === "origin", agent: null, agent_status: "idle", revision: 1 })),
};

function fixture() {
  let state: SessionState = { ...initialSessionState, epoch: 1, sessionId: "session", snapshot, sync: "live" };
  let navigation!: LiaisonNavigation;
  let selectedPaneId = "origin";
  const onFocus = vi.fn((request: { kind: "pane"; target_id: string }) => {
    selectedPaneId = request.target_id;
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "session", token: state.focusToken + 1, request });
  });
  function Probe() {
    navigation = useLiaisonNavigation({ state, selection: { paneId: selectedPaneId }, onFocus } as unknown as WorkbenchProps, () => {});
    return null;
  }
  const root = createRoot(document.createElement("div")); roots.push(root);
  return {
    get navigation() { return navigation; }, onFocus,
    async render() { await act(async () => root.render(createElement(Probe))); },
    async update(next: (state: SessionState) => SessionState) { state = next(state); await act(async () => root.render(createElement(Probe))); },
  };
}

it("releases a superseded navigation so a later Go to liaison can focus and complete", async () => {
  const f = fixture(); await f.render();
  let first!: Promise<void>;
  act(() => { first = f.navigation.supervisorTerminal("liaison"); });
  const firstResult = first.then(() => "resolved", error => error.message as string);
  await f.render();
  // Selecting a local viewer invokes focusCoordinator.supersedeSelection(), which
  // clears the pending request and advances its token without focusing a pane.
  await f.update(state => sessionReducer(state, { type: "focus/clear", epoch: state.epoch, sessionId: "session", token: state.focusToken + 1 }));
  expect(await firstResult).toBe("Terminal focus was superseded by another selection.");

  let retry!: Promise<void>;
  act(() => { retry = f.navigation.supervisorTerminal("liaison"); });
  await f.render();
  await f.update(state => ({ ...state, focusPending: null, snapshot: { ...snapshot, focused_pane_id: "liaison" } }));
  await expect(retry).resolves.toBeUndefined();
  expect(f.onFocus).toHaveBeenCalledTimes(2);
});

it("does not mistake another focus intent or a delayed target snapshot for its acknowledgement", async () => {
  const f = fixture(); await f.render();
  let first!: Promise<void>;
  act(() => { first = f.navigation.supervisorTerminal("liaison"); });
  const firstResult = first.then(() => "resolved", error => error.message as string);
  await f.render();
  await f.update(state => sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "session", token: state.focusToken + 1, request: { kind: "pane", target_id: "origin" } }));
  expect(await firstResult).toBe("Terminal focus was superseded by another selection.");
  await f.update(state => ({ ...state, snapshot: { ...snapshot, focused_pane_id: "liaison" } }));
  let retry!: Promise<void>;
  act(() => { retry = f.navigation.supervisorTerminal("liaison"); });
  await f.render();
  await f.update(state => ({ ...state, focusPending: null, snapshot: { ...snapshot, focused_pane_id: "liaison" } }));
  await expect(retry).resolves.toBeUndefined();
});

it("keeps waiting when a same-tab viewer transition reissues the same pane focus", async () => {
  const f = fixture(); await f.render();
  let navigation!: Promise<void>;
  act(() => { navigation = f.navigation.supervisorTerminal("liaison"); });
  await f.render();
  await f.update(state => sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "session", token: state.focusToken + 1, request: { kind: "pane", target_id: "liaison" } }));
  await f.update(state => ({ ...state, focusPending: null, snapshot: { ...snapshot, focused_pane_id: "liaison" } }));
  await expect(navigation).resolves.toBeUndefined();
  expect(f.navigation.supervisorNavigationError).toBeNull();
});
