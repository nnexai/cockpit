// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient, CockpitClientError, LocalChangeEvent, LocalChangeScope } from "../../client/CockpitClient";
import type { SessionSnapshotResponse } from "../../protocol/generated/v1";
import { useSpaceStateStreams, type SpaceStateWatch } from "./spaceStateStreams";
import { useSpaceGitStatus } from "./spaceGitStatus";
import { useSpaceTags } from "./spaceTags";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
const snapshot = { session_id: "s", server_instance: "one", spaces: [] } as unknown as SessionSnapshotResponse;
type Stream = { scope: LocalChangeScope; event: (event: LocalChangeEvent) => Promise<void>; error: (error: CockpitClientError) => void; signal: AbortSignal };
const event = (kind: "ready" | "changed" | "unavailable") => ({ stream_id: "stream", sequence: 1, kind, topics: [] }) as LocalChangeEvent;

it("uses independent streams, refreshes invalidations, and polls only an unavailable topic", async () => {
  vi.useFakeTimers();
  const streams: Stream[] = [];
  const subscribeLocalChanges = vi.fn(async (scope: LocalChangeScope, onEvent: Stream["event"], error: Stream["error"], signal: AbortSignal) => {
    streams.push({ scope, event: onEvent, error, signal }); return { close: vi.fn() };
  });
  const spaceGitStatus = vi.fn(async () => ({ session_id: "s", server_instance: "one", spaces: [] }));
  const spaceTags = vi.fn(async () => ({ session_id: "s", server_instance: "one", catalog: [], spaces: [] }));
  const client = { subscribeLocalChanges, spaceGitStatus, spaceTags } as unknown as CockpitClient;
  let current!: SpaceStateWatch;
  let refresh = () => {};
  function Probe() {
    current = useSpaceStateStreams(client, "s", "one");
    refresh = useSpaceGitStatus(client, "s", "checkout", 15_000, current.git, "one").refresh;
    useSpaceTags(client, "s", snapshot, true, current.tags);
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    expect(streams.map(stream => stream.scope)).toEqual([
      { kind: "space_git", session_id: "s", server_instance: "one" },
      { kind: "space_tags", session_id: "s", server_instance: "one" },
    ]);
    await act(async () => { await streams[0].event(event("ready")); await streams[1].event(event("ready")); });
    expect(current.git.status).toBe("watching"); expect(current.tags.status).toBe("watching");
    const initialGit = spaceGitStatus.mock.calls.length, initialTags = spaceTags.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(120_000); });
    expect(subscribeLocalChanges).toHaveBeenCalledTimes(2);
    expect(spaceGitStatus).toHaveBeenCalledTimes(initialGit); expect(spaceTags).toHaveBeenCalledTimes(initialTags);
    await act(async () => { await streams[0].event(event("changed")); });
    expect(spaceGitStatus).toHaveBeenCalledTimes(initialGit + 1); expect(spaceTags).toHaveBeenCalledTimes(initialTags);
    await act(async () => { await streams[1].event(event("changed")); });
    expect(spaceTags).toHaveBeenCalledTimes(initialTags + 1);
    await act(async () => { await streams[0].event(event("unavailable")); });
    expect(current.git.status).toBe("unavailable"); expect(current.tags.status).toBe("watching");
    await act(async () => { await vi.advanceTimersByTimeAsync(15_000); });
    expect(spaceGitStatus).toHaveBeenCalledTimes(initialGit + 2); expect(spaceTags).toHaveBeenCalledTimes(initialTags + 1);
    await act(async () => refresh());
    expect(spaceGitStatus).toHaveBeenCalledTimes(initialGit + 3);
    await act(async () => { await streams[0].event(event("ready")); await streams[1].event(event("unavailable")); });
    expect(current.git.status).toBe("watching"); expect(current.tags.status).toBe("unavailable");
    const recoveredGit = spaceGitStatus.mock.calls.length, unavailableTags = spaceTags.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(15_000); });
    expect(spaceGitStatus).toHaveBeenCalledTimes(recoveredGit); expect(spaceTags).toHaveBeenCalledTimes(unavailableTags + 1);
  } finally { await act(async () => root.unmount()); }
});

it("fences old incarnations and reconnects visible streams with fresh baselines", async () => {
  let visibility: DocumentVisibilityState = "visible";
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
  const streams: Stream[] = [];
  const client = { subscribeLocalChanges: vi.fn(async (scope: LocalChangeScope, onEvent: Stream["event"], error: Stream["error"], signal: AbortSignal) => {
    streams.push({ scope, event: onEvent, error, signal }); return { close: vi.fn() };
  }) } as unknown as CockpitClient;
  let instance = "one";
  let current!: SpaceStateWatch;
  function Probe() { current = useSpaceStateStreams(client, "s", instance); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { await streams[0].event(event("ready")); await streams[1].event(event("ready")); });
    visibility = "hidden"; await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(streams[0].signal.aborted).toBe(true); expect(streams[1].signal.aborted).toBe(true);
    visibility = "visible"; await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(streams).toHaveLength(4);
    await act(async () => { await streams[2].event(event("ready")); });
    expect(current.git.counter).toBe(2);
    instance = "two"; await act(async () => root.render(createElement(Probe)));
    expect(streams[2].signal.aborted).toBe(true); expect(streams[3].signal.aborted).toBe(true);
    await act(async () => { await streams[2].event(event("changed")); });
    expect(current.git).toEqual({ counter: 0, status: "pending" });
    await act(async () => { await streams[4].event(event("ready")); });
    expect(current.git).toEqual({ counter: 1, status: "watching" });
  } finally { await act(async () => root.unmount()); }
});
