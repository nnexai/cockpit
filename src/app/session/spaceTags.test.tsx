// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionSnapshotResponse, SpaceTagChangeRequest, SpaceTagsResponse } from "../../protocol/generated/v1";
import { SPACE_TAGS_POLL_MS, useSpaceTags, type SpaceTagsState } from "./spaceTags";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const roots: Root[] = [];
beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
});
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function snapshot(sessionId = "session", serverInstance = "server"): SessionSnapshotResponse {
  return {
    session_id: sessionId, server_instance: serverInstance, version: "1", protocol: 1,
    focused_space_id: "s1", focused_tab_id: null, focused_pane_id: null,
    spaces: ["s1", "s2"].map((id, index) => ({ id, label: id, number: index + 1, tab_count: 0, pane_count: 0, focused: index === 0, agent_status: "idle", git: null })),
    tabs: [], panes: [], agents: [],
  };
}

function response(spaces: SpaceTagsResponse["spaces"] = [], catalog: string[] = ["My check", "Needs testing"]): SpaceTagsResponse {
  return { session_id: "session", server_instance: "server", catalog, spaces };
}

function fixture() {
  const read = vi.fn((_session: string, _signal?: AbortSignal): Promise<SpaceTagsResponse> => Promise.resolve(response()));
  const write = vi.fn((_session: string, _request: SpaceTagChangeRequest): Promise<SpaceTagsResponse> => Promise.resolve(response()));
  const props = { client: { spaceTags: read, changeSpaceTag: write } as unknown as CockpitClient, sessionId: "session" as string | null, snapshot: snapshot() as SessionSnapshotResponse | null, live: true };
  let state!: SpaceTagsState;
  function Probe() { state = useSpaceTags(props.client, props.sessionId, props.snapshot, props.live); return null; }
  const root = createRoot(document.createElement("div"));
  roots.push(root);
  return { props, read, write, get state() { return state; }, async render() { await act(async () => root.render(createElement(Probe))); } };
}

it("polls visible live instances every fifteen seconds without overlapping reads and refreshes on focus", async () => {
  const f = fixture();
  const initial = deferred<SpaceTagsResponse>();
  f.read.mockReturnValueOnce(initial.promise);
  await f.render();
  expect(f.state.editBlocked).toBe("Tags are loading");
  await act(async () => vi.advanceTimersByTime(SPACE_TAGS_POLL_MS * 3));
  expect(f.read).toHaveBeenCalledTimes(1);
  await act(async () => initial.resolve(response()));
  await act(async () => vi.advanceTimersByTime(SPACE_TAGS_POLL_MS - 1));
  expect(f.read).toHaveBeenCalledTimes(1);
  await act(async () => vi.advanceTimersByTime(1));
  expect(f.read).toHaveBeenCalledTimes(2);
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(f.read).toHaveBeenCalledTimes(3);
});

it("skips offline or mismatched snapshots and aborts hidden reads until visibility returns", async () => {
  const f = fixture();
  f.props.live = false;
  await f.render();
  expect(f.state.editBlocked).toBe("Herdr is not live");
  expect(await f.state.change("s1", "My check", true)).toBe(false);
  expect(f.read).not.toHaveBeenCalled();
  f.props.live = true;
  f.props.snapshot = snapshot("other");
  await f.render();
  expect(f.read).not.toHaveBeenCalled();
  f.props.snapshot = snapshot();
  const pending = deferred<SpaceTagsResponse>();
  f.read.mockReturnValueOnce(pending.promise);
  await f.render();
  const signal = f.read.mock.calls[0][1]!;
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(signal.aborted).toBe(true);
  await act(async () => { window.dispatchEvent(new Event("focus")); vi.advanceTimersByTime(SPACE_TAGS_POLL_MS * 2); });
  expect(f.read).toHaveBeenCalledTimes(1);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(f.read).toHaveBeenCalledTimes(2);
  expect(f.state.ready).toBe(true);
  f.props.live = false;
  await f.render();
  await act(async () => vi.advanceTimersByTime(SPACE_TAGS_POLL_MS * 2));
  expect(f.read).toHaveBeenCalledTimes(2);
});

it("rejects another instance's first response without enabling editing", async () => {
  const f = fixture();
  f.read.mockResolvedValue({ ...response([{ space_id: "s1", tags: ["My check"] }]), server_instance: "other" });
  await f.render();
  expect(f.state.ready).toBe(false);
  expect(f.state.failed).toBe(true);
  expect(f.state.editBlocked).toBe("Tags unavailable");
  expect(f.state.catalog).toEqual([]);
  expect(f.state.bySpace.size).toBe(0);
  expect(await f.state.change("s1", "My check", true)).toBe(false);
  expect(f.write).not.toHaveBeenCalled();
});

it.each(["client", "session", "instance"] as const)("isolates state, pending writes and retained consumers after %s changes", async identity => {
  const f = fixture();
  f.read.mockResolvedValue(response([{ space_id: "s1", tags: ["My check"] }]));
  await f.render();
  const oldChange = f.state.change;
  const write = deferred<SpaceTagsResponse>();
  f.write.mockReturnValueOnce(write.promise);
  let outcome!: Promise<boolean>;
  act(() => { outcome = f.state.change("s1", "Needs testing", true); });
  expect(f.state.pending.has("s1")).toBe(true);
  const nextRead = deferred<SpaceTagsResponse>();
  const read = vi.fn(() => nextRead.promise);
  if (identity === "client") f.props.client = { spaceTags: read, changeSpaceTag: f.write } as unknown as CockpitClient;
  else {
    f.read.mockReturnValue(nextRead.promise);
    f.props.snapshot = identity === "session" ? snapshot("other") : snapshot("session", "replacement");
    if (identity === "session") f.props.sessionId = "other";
  }
  await f.render();
  expect(f.state.ready).toBe(false);
  expect(f.state.pending.size).toBe(0);
  expect(f.state.bySpace.size).toBe(0);
  expect(f.state.catalog).toEqual([]);
  expect(await oldChange("s2", "My check", true)).toBe(false);
  await act(async () => write.resolve(response([{ space_id: "s1", tags: ["Needs testing"] }])));
  expect(await outcome).toBe(false);
  expect(f.state.ready).toBe(false);
  expect(f.state.pending.size).toBe(0);
  expect(f.state.bySpace.size).toBe(0);
});

it("aborts and ignores a late read from the previous server instance", async () => {
  const f = fixture();
  const old = deferred<SpaceTagsResponse>();
  const next = deferred<SpaceTagsResponse>();
  f.read.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
  await f.render();
  const signal = f.read.mock.calls[0][1]!;
  f.props.snapshot = snapshot("session", "replacement");
  await f.render();
  expect(signal.aborted).toBe(true);
  await act(async () => old.resolve(response([{ space_id: "s1", tags: ["My check"] }])));
  expect(f.state.ready).toBe(false);
  expect(f.state.bySpace.size).toBe(0);
  await act(async () => next.resolve({ ...response([{ space_id: "s2", tags: ["Needs testing"] }]), server_instance: "replacement" }));
  expect(f.state.ready).toBe(true);
  expect(f.state.bySpace.get("s2")).toEqual(["Needs testing"]);
  expect(f.state.bySpace.has("s1")).toBe(false);
});

it("does not let a read started before an acknowledged write undo it", async () => {
  const f = fixture();
  await f.render();
  const stale = deferred<SpaceTagsResponse>();
  const reconciliation = deferred<SpaceTagsResponse>();
  f.read.mockReturnValueOnce(stale.promise).mockReturnValueOnce(reconciliation.promise);
  await act(async () => window.dispatchEvent(new Event("focus")));
  f.write.mockResolvedValueOnce(response([{ space_id: "s1", tags: ["My check"] }]));
  let outcome!: Promise<boolean>;
  await act(async () => { outcome = f.state.change("s1", "My check", true); await outcome; });
  expect(await outcome).toBe(true);
  expect(f.state.bySpace.get("s1")).toEqual(["My check"]);
  await act(async () => stale.resolve(response()));
  expect(f.state.bySpace.get("s1")).toEqual(["My check"]);
  await act(async () => reconciliation.resolve(response([{ space_id: "s1", tags: ["My check"] }])));
  expect(f.state.bySpace.get("s1")).toEqual(["My check"]);
});

it("keeps two distinct concurrently acknowledged Spaces when full write responses arrive out of order", async () => {
  const f = fixture();
  await f.render();
  const first = deferred<SpaceTagsResponse>();
  const second = deferred<SpaceTagsResponse>();
  const reconciliation = deferred<SpaceTagsResponse>();
  f.write.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  f.read.mockReturnValueOnce(reconciliation.promise);
  let one!: Promise<boolean>;
  let two!: Promise<boolean>;
  act(() => {
    one = f.state.change("s1", "My check", true);
    two = f.state.change("s2", "Needs testing", true);
  });
  expect(f.write).toHaveBeenCalledTimes(2);
  expect([...f.state.pending]).toEqual(["s1", "s2"]);
  expect(await f.state.change("s1", "Needs testing", true)).toBe(false);
  await act(async () => second.resolve(response([{ space_id: "s2", tags: ["Needs testing"] }], ["Needs testing"])));
  expect(await two).toBe(true);
  expect(f.state.bySpace.get("s2")).toEqual(["Needs testing"]);
  expect([...f.state.pending]).toEqual(["s1"]);
  expect(f.read).toHaveBeenCalledTimes(1);
  await act(async () => first.resolve(response([{ space_id: "s1", tags: ["My check"] }], ["My check"])));
  expect(await one).toBe(true);
  expect(f.state.bySpace.get("s1")).toEqual(["My check"]);
  expect(f.state.bySpace.get("s2")).toEqual(["Needs testing"]);
  expect(f.state.pending.size).toBe(0);
  expect(f.read).toHaveBeenCalledTimes(2);
  await act(async () => reconciliation.resolve(response([
    { space_id: "s1", tags: ["My check"] }, { space_id: "s2", tags: ["Needs testing"] },
  ], ["Needs testing", "My check"])));
  expect(f.state.catalog).toEqual(["Needs testing", "My check"]);
  expect(f.state.bySpace.size).toBe(2);
});

it("preserves acknowledged data on read failures and refreshes unknown write outcomes without claiming success", async () => {
  const f = fixture();
  f.read.mockRejectedValueOnce(new Error("unavailable"));
  await f.render();
  expect(f.state.failed).toBe(true);
  expect(f.state.editBlocked).toBe("Tags unavailable");
  f.read.mockResolvedValueOnce(response([{ space_id: "s1", tags: ["My check"] }]));
  await act(async () => window.dispatchEvent(new Event("focus")));
  f.read.mockRejectedValueOnce(new Error("offline"));
  await act(async () => vi.advanceTimersByTime(SPACE_TAGS_POLL_MS));
  expect(f.state.ready).toBe(true);
  expect(f.state.failed).toBe(false);
  expect(f.state.bySpace.get("s1")).toEqual(["My check"]);
  const actual = deferred<SpaceTagsResponse>();
  f.read.mockReturnValueOnce(actual.promise);
  f.write.mockRejectedValueOnce(new Error("outcome unknown"));
  let outcome!: Promise<boolean>;
  await act(async () => { outcome = f.state.change("s1", "Needs testing", true); await outcome; });
  expect(await outcome).toBe(false);
  expect(f.state.pending.size).toBe(0);
  expect(f.state.bySpace.get("s1")).toEqual(["My check"]);
  await act(async () => actual.resolve(response([{ space_id: "s1", tags: ["My check", "Needs testing"] }])));
  expect(f.state.bySpace.get("s1")).toEqual(["My check", "Needs testing"]);
});

it("filters removed Spaces at the consumer boundary and normalizes submitted tags to NFC", async () => {
  const f = fixture();
  f.read.mockResolvedValue(response([{ space_id: "s1", tags: ["My check"] }, { space_id: "removed", tags: ["My check"] }]));
  await f.render();
  expect(f.state.bySpace.has("removed")).toBe(false);
  expect(await f.state.change("removed", "My check", true)).toBe(false);
  const retained = f.state.change;
  f.props.snapshot = { ...snapshot(), spaces: [snapshot().spaces[1]] };
  await f.render();
  expect(f.state.bySpace.has("s1")).toBe(false);
  expect(await retained("s1", "My check", true)).toBe(false);
  await act(async () => { await f.state.change("s2", "Cafe\u0301", true); });
  expect(f.write).toHaveBeenCalledExactlyOnceWith("session", { space_id: "s2", server_instance: "server", tag: "Café", assigned: true });
});

it("clears pending on offline transition and rejects acknowledgements from a previous live period", async () => {
  const f = fixture();
  await f.render();
  const write = deferred<SpaceTagsResponse>();
  f.write.mockReturnValueOnce(write.promise);
  let outcome!: Promise<boolean>;
  act(() => { outcome = f.state.change("s1", "My check", true); });
  f.props.live = false;
  await f.render();
  expect(f.state.pending.size).toBe(0);
  expect(f.state.editBlocked).toBe("Herdr is not live");
  f.props.live = true;
  await f.render();
  await act(async () => write.resolve(response([{ space_id: "s1", tags: ["My check"] }])));
  expect(await outcome).toBe(false);
  expect(f.state.pending.size).toBe(0);
  expect(f.state.bySpace.size).toBe(0);
});
