// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { AgentAttentionChangeRequest, AgentAttentionResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import { deriveAgentQueue } from "../sidebar/agentAttentionModel";
import { initialSessionState } from "./sessionStore";
import { historicalAttentionKey, useAgentAttention, type AgentAttentionState } from "./agentAttention";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const roots: Root[] = [];
afterEach(async () => { for (const root of roots.splice(0)) await act(async () => root.unmount()); vi.useRealTimers(); vi.restoreAllMocks(); });
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
const snapshot: SessionSnapshotResponse = { session_id: "s", server_instance: "server", version: "1", protocol: 22, focused_space_id: null, focused_tab_id: null, focused_pane_id: null, spaces: [], tabs: [], panes: [{ id: "p", terminal_id: "t", space_id: "space", tab_id: "tab", title: null, focused: false, agent: "agent", agent_status: "idle", revision: 1 }], agents: [{ pane_id: "p", space_id: "space", tab_id: "tab", name: "agent", status: "idle", title: null, focused: false, state_change_seq: 4 }] };
const response: AgentAttentionResponse = { session_id: "s", server_instance: "server", epoch: 1, refused: null, records: [{ historical_id: null, pane_id: "p", terminal_id: "t", agent: "agent", last: { epoch: 1, seq: 4, status: "idle", observed_at: "now" }, completion: { epoch: 1, seq: 3, at: "then", inferred: true }, acked: null, dismissed: null }] };
function fixture() {
  const read = vi.fn((_session: string, _signal?: AbortSignal): Promise<AgentAttentionResponse> => Promise.resolve(response));
  const write = vi.fn((_session: string, request: AgentAttentionChangeRequest): Promise<AgentAttentionResponse> => Promise.resolve({ ...response, server_instance: request.server_instance }));
  const client = { agentAttention: read, changeAgentAttention: write } as unknown as CockpitClient;
  const props = { state: { ...initialSessionState, sessionId: "s", snapshot, sync: "live" as const }, live: true };
  let attention!: AgentAttentionState;
  function Probe() { attention = useAgentAttention(client, props.state, props.live); return null; }
  const root = createRoot(document.createElement("div")); roots.push(root);
  return { props, read, write, get attention() { return attention; }, async render() { await act(async () => root.render(createElement(Probe))); } };
}

it("loads only matching responses and discards a previous instance's late read", async () => {
  const f = fixture(); const first = deferred<AgentAttentionResponse>();
  f.read.mockReturnValueOnce(first.promise);
  await f.render(); expect(f.attention.availability).toBe("loading");
  f.props.state = { ...f.props.state, snapshot: { ...snapshot, server_instance: "new" } };
  f.read.mockResolvedValueOnce({ ...response, server_instance: "new" });
  await f.render(); expect(f.attention.response?.server_instance).toBe("new");
  await act(async () => first.resolve(response));
  expect(f.attention.response?.server_instance).toBe("new");
});

it("acks the captured completion only after the exact focus request is accepted and fulfilled", async () => {
  const f = fixture(); await f.render(); f.write.mockClear();
  const row = deriveAgentQueue(snapshot, response, "ready").visible[0]!;
  act(() => f.attention.activate(row));
  f.props.state = { ...f.props.state, focusToken: 1, focusPending: null, snapshot: { ...snapshot, focused_pane_id: "p" } };
  await f.render(); expect(f.write).not.toHaveBeenCalled();
  f.props.state = { ...f.props.state, focusAcceptedToken: 1 };
  await f.render();
  expect(f.write).toHaveBeenCalledWith("s", expect.objectContaining({ change: { kind: "seen", pane_id: "p", terminal_id: "t", agent: "agent", transition: { epoch: 1, seq: 3 } } }));
});

it("does not ack failed or superseded focus, but a selected row acknowledges immediately", async () => {
  const f = fixture(); await f.render(); f.write.mockClear();
  const row = deriveAgentQueue(snapshot, response, "ready").visible[0]!;
  act(() => f.attention.activate(row));
  f.props.state = { ...f.props.state, focusToken: 1, focusError: { code: "focus_rejected", message: "rejected" } };
  await f.render(); expect(f.write).not.toHaveBeenCalled();
  f.props.state = { ...f.props.state, focusError: null, snapshot: { ...snapshot, focused_pane_id: "p" } };
  await f.render(); await act(async () => { f.attention.activate(row); });
  expect(f.write).toHaveBeenCalledWith("s", expect.objectContaining({ change: expect.objectContaining({ kind: "seen", transition: { epoch: 1, seq: 3 } }) }));
});

it("refetches on focus and visible events without polling and exposes refusals", async () => {
  vi.useFakeTimers(); vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const f = fixture(); await f.render(); const count = f.read.mock.calls.length;
  await act(async () => { vi.advanceTimersByTime(60000); }); expect(f.read).toHaveBeenCalledTimes(count);
  await act(async () => window.dispatchEvent(new Event("focus")));
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(f.read).toHaveBeenCalledTimes(count + 2);
  f.write.mockResolvedValueOnce({ ...response, refused: { code: "agent_attention_target_changed", message: "Changed" } });
  const row = deriveAgentQueue(snapshot, response, "ready").visible[0]!;
  let result!: boolean;
  await act(async () => { result = await f.attention.dismiss(row); });
  expect(result).toBe(false); expect(f.attention.notes.get("p")).toBe("Changed");
});

it("sends observations in batches of at most 256", async () => {
  const f = fixture();
  f.props.state = { ...f.props.state, snapshot: {
    ...snapshot,
    panes: Array.from({ length: 300 }, (_, i) => ({ ...snapshot.panes[0]!, id: `p${i}`, terminal_id: `t${i}` })),
    agents: Array.from({ length: 300 }, (_, i) => ({ ...snapshot.agents[0]!, pane_id: `p${i}` })),
  } };
  await f.render();
  expect(f.write.mock.calls.map(([, request]) => request.observations.length)).toEqual([256, 44]);
});

it("preserves completion and inference evidence when one agent accumulates more than one batch", async () => {
  const f = fixture();
  const initial = deferred<AgentAttentionResponse>();
  f.read.mockReturnValueOnce(initial.promise);
  await f.render();
  for (let i = 0; i < 260; i++) {
    const status = i === 0 ? "done" : i % 2 ? "working" : "idle";
    f.props.state = { ...f.props.state, snapshot: { ...snapshot, agents: [{ ...snapshot.agents[0]!, status, state_change_seq: i + 5 }] } };
    await f.render();
  }
  await act(async () => initial.resolve(response));
  const observations = f.write.mock.calls.flatMap(([, request]) => request.observations);
  expect(observations).toHaveLength(261);
  expect(observations.slice(1, 4).map(item => item.status)).toEqual(["done", "working", "idle"]);
  expect(f.write.mock.calls.every(([, request]) => request.observations.length <= 256)).toBe(true);
});

it("acknowledges a historical identity explicitly without focusing its replacement", async () => {
  const f = fixture(); await f.render(); f.write.mockClear();
  const record = { ...response.records[0]!, historical_id: "history-id" };
  const pending = deferred<AgentAttentionResponse>();
  f.write.mockReturnValueOnce(pending.promise);
  let result!: Promise<boolean>;
  act(() => { result = f.attention.seenHistorical(record); });
  expect(f.attention.pending.has(historicalAttentionKey(record))).toBe(true);
  expect(f.write).toHaveBeenCalledWith("s", expect.objectContaining({ change: { kind: "seen_historical", historical_id: "history-id" } }));
  await act(async () => pending.resolve({ ...response, refused: { code: "agent_attention_target_changed", message: "History changed" } }));
  expect(await result).toBe(false);
  expect(f.attention.notes.get(historicalAttentionKey(record))).toBe("History changed");
  await expect(f.attention.seenHistorical(response.records[0]!)).resolves.toBe(false);
});
