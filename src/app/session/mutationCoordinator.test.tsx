// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CreatedPane, ResourceMutationRequest, ResourceMutationResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import { initialSessionState, type SessionAction, type SessionState } from "./sessionStore";
import { useMutationCoordinator, type MutationCoordinator, type MutationCoordinatorOptions } from "./mutationCoordinator";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail; });
  return { promise, resolve, reject };
}

const created: CreatedPane = { pane_id: "pane-new", terminal_id: "terminal-new", space_id: "space-1", tab_id: "tab-1" };
const split: ResourceMutationRequest = { type: "pane_split", pane_id: "pane-source", direction: "right", ratio: null };

function snapshot(focusedPane = "pane-other"): SessionSnapshotResponse {
  return {
    session_id: "session-1", server_instance: "server-1", version: "1", protocol: 22,
    focused_space_id: "space-1", focused_tab_id: "tab-1", focused_pane_id: focusedPane,
    spaces: [], tabs: [], panes: [], agents: [],
  };
}

type MountedCoordinator = {
  mutate: MutationCoordinator["mutate"];
  reset(): void;
  state(): MutationCoordinator["state"];
  observe(snapshot: SessionSnapshotResponse): void;
  actions: SessionAction[];
  unmount(): Promise<void>;
};

function mountCoordinator(client: CockpitClient, options: Partial<Pick<MutationCoordinatorOptions, "onBegin" | "onSettled" | "onResync" | "mutationSnapshot">> & { onSnapshot?(snapshot: SessionSnapshotResponse): void } = {}): MountedCoordinator {
  const stateRef = { current: { ...initialSessionState, sessionId: "session-1", epoch: 1, generation: 1, sequence: 1, sync: "live", snapshot: snapshot("pane-source") } as SessionState };
  const mountedRef = { current: true };
  const sessionObservationRef = { current: 0 };
  const actions: SessionAction[] = [];
  let coordinator!: MutationCoordinator;
  function Harness() {
    coordinator = useMutationCoordinator({
      client, stateRef, mountedRef, sessionObservationRef,
      dispatchSession: (action) => {
        actions.push(action);
        if (action.type === "snapshot/authoritative") options.onSnapshot?.(action.snapshot);
      },
      describeError: (error, fallback) => ({ code: error instanceof Error ? error.message : undefined, message: error instanceof Error ? error.message : fallback }),
      mutationSnapshot: options.mutationSnapshot ?? ((_sessionId, response) => (response as ResourceMutationResponse).snapshot),
      onBegin: options.onBegin ?? (() => undefined),
      onSettled: options.onSettled ?? (() => undefined),
      onResync: options.onResync ?? (() => undefined),
    });
    return null;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  act(() => { root.render(<Harness />); });
  return {
    mutate(key, request, select) { return coordinator.mutate(key, request, select); },
    reset() { coordinator.reset(); },
    state() { return coordinator.state; },
    observe(next) {
      sessionObservationRef.current += 1;
      stateRef.current = { ...stateRef.current, snapshot: next, sequence: stateRef.current.sequence + 1 };
    },
    actions,
    async unmount() { mountedRef.current = false; await act(async () => { root.unmount(); }); host.remove(); },
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

describe("useMutationCoordinator creation coordination", () => {
  const mounted: MountedCoordinator[] = [];
  afterEach(async () => { await Promise.all(mounted.splice(0).map((coordinator) => coordinator.unmount())); });

  it("holds creation before the request can expose members and settles its receipt before applying the snapshot", async () => {
    const response = deferred<ResourceMutationResponse>();
    let held = false;
    let heldAtRequest = false;
    let heldAtSnapshot = false;
    let receiptAtSnapshot: CreatedPane | null = null;
    let attributed: CreatedPane | null = null;
    const mutate = vi.fn(() => { heldAtRequest = held; return response.promise; });
    const coordinator = mountCoordinator({ mutate } as unknown as CockpitClient, {
      onBegin() { held = true; },
      onSettled(_operation, receipt) { held = false; attributed = receipt; },
      onSnapshot() {
        heldAtSnapshot = held;
        receiptAtSnapshot = attributed;
      },
    });
    mounted.push(coordinator);
    let started = false;
    let overlapping = true;
    await act(async () => {
      started = coordinator.mutate("split", split, true);
      overlapping = coordinator.mutate("other", split, true);
      response.resolve({ session_id: "session-1", snapshot: snapshot(), created });
      await settle();
    });
    expect(started).toBe(true);
    expect(overlapping).toBe(false);
    expect(heldAtRequest).toBe(true);
    expect(heldAtSnapshot).toBe(false);
    expect(receiptAtSnapshot).toEqual(created);
    expect(attributed).toEqual(created);
    expect(held).toBe(false);
    expect(coordinator.state().pending).toBeNull();
    expect(coordinator.actions.find((action) => action.type === "snapshot/authoritative")).toEqual({ type: "snapshot/authoritative", epoch: 1, sessionId: "session-1", snapshot: snapshot() });
  });

  it("still attributes a validated created pane when the stream outruns a differently focused mutation response", async () => {
    const response = deferred<ResourceMutationResponse>();
    const mutate = vi.fn().mockReturnValue(response.promise);
    const settlements: Array<{ receipt: CreatedPane | null; current: boolean | undefined }> = [];
    const coordinator = mountCoordinator({ mutate } as unknown as CockpitClient, {
      onSettled(_operation, receipt, _snapshot, current) { settlements.push({ receipt, current }); },
    });
    mounted.push(coordinator);
    await act(async () => {
      coordinator.mutate("split", split, true);
      coordinator.observe(snapshot("pane-external"));
      response.resolve({ session_id: "session-1", snapshot: snapshot("pane-other"), created });
      await settle();
    });
    expect(settlements).toEqual([{ receipt: created, current: false }]);
    expect(coordinator.actions.some((action) => action.type === "snapshot/authoritative")).toBe(false);
    expect(coordinator.state().pending).toBeNull();
  });

  it("does not invent a created pane from snapshot focus when no receipt exists", async () => {
    const response = { session_id: "session-1", snapshot: snapshot("pane-new"), created: null };
    const settlements: Array<CreatedPane | null> = [];
    const coordinator = mountCoordinator({ mutate: vi.fn().mockResolvedValue(response) } as unknown as CockpitClient, {
      onSettled(_operation, receipt) { settlements.push(receipt); },
    });
    mounted.push(coordinator);
    await act(async () => { coordinator.mutate("split", split, true); await settle(); });
    expect(settlements).toEqual([null]);
  });

  it.each(["request_outcome_unknown", "mutation_applied_snapshot_failed", "offline"])("releases held members on %s without automatically issuing another mutation", async (code) => {
    const response = deferred<ResourceMutationResponse>();
    const mutate = vi.fn().mockReturnValue(response.promise);
    const resync = vi.fn();
    let held = false;
    const coordinator = mountCoordinator({ mutate } as unknown as CockpitClient, {
      onBegin() { held = true; },
      onSettled(_operation, receipt) { if (receipt === null) held = false; },
      onResync: resync,
    });
    mounted.push(coordinator);
    await act(async () => {
      coordinator.mutate("split", split, true);
      response.reject(new Error(code));
      await settle();
    });
    expect(held).toBe(false);
    expect(coordinator.state().errors.split.code).toBe(code);
    expect(mutate).toHaveBeenCalledTimes(1);
    expect(resync).toHaveBeenCalledTimes(code === "offline" ? 0 : 1);
    expect(coordinator.actions).toEqual([]);
  });

  it("settles malformed snapshots as failure so held members are released", async () => {
    const settlements: Array<CreatedPane | null> = [];
    const coordinator = mountCoordinator({ mutate: vi.fn().mockResolvedValue({ session_id: "session-1", snapshot: snapshot(), created }) } as unknown as CockpitClient, {
      mutationSnapshot() { throw new Error("malformed_response"); },
      onSettled(_operation, receipt) { settlements.push(receipt); },
    });
    mounted.push(coordinator);
    await act(async () => { coordinator.mutate("split", split, true); await settle(); });
    expect(settlements).toEqual([null]);
    expect(coordinator.state().errors.split.code).toBe("malformed_response");
    expect(coordinator.actions).toEqual([]);
  });

  it("settles an old failure without clearing or failing a new operation after reset", async () => {
    const first = deferred<ResourceMutationResponse>();
    const second = deferred<ResourceMutationResponse>();
    const settlements: Array<{ token: number; receipt: CreatedPane | null }> = [];
    const coordinator = mountCoordinator({ mutate: vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise) } as unknown as CockpitClient, {
      onSettled(operation, receipt) { settlements.push({ token: operation.token, receipt }); },
    });
    mounted.push(coordinator);
    await act(async () => {
      coordinator.mutate("first", split, true);
      coordinator.reset();
      coordinator.mutate("second", split, true);
      first.reject(new Error("offline"));
      await settle();
    });
    expect(settlements).toEqual([{ token: 1, receipt: null }]);
    expect(coordinator.state().pending?.key).toBe("second");
    expect(coordinator.state().errors).toEqual({});
    await act(async () => {
      second.resolve({ session_id: "session-1", snapshot: snapshot(), created });
      await settle();
    });
    expect(coordinator.state().pending).toBeNull();
    expect(settlements).toEqual([{ token: 1, receipt: null }, { token: 3, receipt: created }]);
  });
});
