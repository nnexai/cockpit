// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest, FocusResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import { sessionReducer, type SessionAction, type SessionState } from "./sessionStore";
import { FOCUS_FALLBACK_MS, FOCUS_PREPARE_MS, useFocusCoordinator, type FocusCoordinator, type FocusCoordinatorOptions, type FocusLocation } from "./focusCoordinator";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

type Deferred<T> = {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(reason: unknown): void;
};

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function liveState(sessionId = "session-1", epoch = 1): SessionState {
  return {
    epoch,
    sessionId,
    snapshot: null,
    generation: null,
    sequence: 0,
    sync: "live",
    syncError: null,
    focusPending: null,
    focusToken: 0,
    focusError: null,
    attachments: {},
  };
}

function snapshot(paneId = "pane-a"): SessionSnapshotResponse {
  return {
    session_id: "session-1", server_instance: "server-1", version: "1", protocol: 22,
    focused_space_id: "space-1", focused_tab_id: "tab-a", focused_pane_id: paneId,
    spaces: [], tabs: [], panes: [], agents: [],
  };
}

function request(id: string): FocusRequest {
  return { kind: "tab", target_id: id };
}

function accepted(id: string): FocusResponse {
  return { session_id: "session-1", kind: "tab", target_id: id, accepted: true };
}

const location: FocusLocation = { spaceId: "space-1", tabId: null, paneId: null };

type MountedCoordinator = Pick<FocusCoordinator, "focus" | "panePrepared" | "reset" | "supersedeSelection" | "getEchoes" | "consumeEcho" | "reconcile" | "retryFocus"> & {
  setState(state: SessionState): void;
  getState(): SessionState;
  unmount(): Promise<void>;
};
function mountCoordinator(client: CockpitClient, dispatch: (action: SessionAction) => void, callbacks: Partial<Pick<FocusCoordinatorOptions, "onIntent" | "onTimeout">> = {}): MountedCoordinator {
  const stateRef = { current: liveState() };
  const mountedRef = { current: true };
  let coordinator: FocusCoordinator | null = null;
  function Harness() {
    coordinator = useFocusCoordinator({
      client,
      stateRef,
      mountedRef,
      dispatch(action) {
        stateRef.current = sessionReducer(stateRef.current, action);
        dispatch(action);
      },
      describeError: (error, fallback) => ({ code: "focus_error", message: error instanceof Error ? error.message : fallback }),
      onIntent: callbacks.onIntent ?? (() => undefined),
      onTimeout: callbacks.onTimeout ?? (() => undefined),
    });
    return null;
  }
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const root = createRoot(host);
  act(() => { root.render(<Harness />); });
  return {
    focus(request, nextLocation, prepare) { coordinator!.focus(request, nextLocation, prepare); },
    panePrepared(paneId) { coordinator!.panePrepared(paneId); },
    reset() { coordinator!.reset(); },
    supersedeSelection() { coordinator!.supersedeSelection(); },
    getEchoes() { return coordinator!.getEchoes(); },
    consumeEcho(token) { coordinator!.consumeEcho(token); },
    reconcile(state, selection, controlPaneId) { return coordinator!.reconcile(state, selection, controlPaneId); },
    retryFocus() { coordinator!.retryFocus(); },
    setState(state) { stateRef.current = state; },
    getState() { return stateRef.current; },
    async unmount() {
      await act(async () => { mountedRef.current = false; coordinator!.reset(); root.unmount(); });
      host.remove();
    },
  };
}

async function settlePromises(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

describe("useFocusCoordinator", () => {
  const mounted: MountedCoordinator[] = [];

  afterEach(async () => {
    await Promise.all(mounted.splice(0).map((coordinator) => coordinator.unmount()));
  });

  it("waits for the first focus request before sending the newest queued intent", async () => {
    const first = deferred<FocusResponse>();
    const latest = deferred<FocusResponse>();
    const unexpected = deferred<FocusResponse>();
    const focus = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(latest.promise).mockReturnValue(unexpected.promise);
    const dispatch = vi.fn();
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, dispatch);
    mounted.push(coordinator);

    await act(async () => {
      coordinator.focus(request("tab-a"), location);
      coordinator.focus(request("tab-b"), location);
      coordinator.focus(request("tab-c"), location);
    });
    expect(focus).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenLastCalledWith("session-1", request("tab-a"));

    await act(async () => {
      first.resolve(accepted("tab-a"));
      await settlePromises();
    });
    expect(focus).toHaveBeenCalledTimes(2);
    expect(focus).toHaveBeenLastCalledWith("session-1", request("tab-c"));

    await act(async () => {
      latest.resolve(accepted("tab-c"));
      await settlePromises();
    });
  });

  it("does not send an intent queued before reset", async () => {
    const first = deferred<FocusResponse>();
    const focus = vi.fn().mockReturnValue(first.promise);
    const dispatch = vi.fn();
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, dispatch);
    mounted.push(coordinator);

    await act(async () => {
      coordinator.focus(request("tab-a"), location);
      coordinator.focus(request("tab-b"), location);
      coordinator.reset();
      first.resolve(accepted("tab-a"));
      await settlePromises();
    });
    expect(focus).toHaveBeenCalledTimes(1);
  });

  it("keeps a new intent queued across reset until the earlier request settles", async () => {
    const first = deferred<FocusResponse>();
    const next = deferred<FocusResponse>();
    const focus = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(next.promise);
    const dispatch = vi.fn();
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, dispatch);
    mounted.push(coordinator);

    await act(async () => {
      coordinator.focus(request("tab-a"), location);
      coordinator.reset();
      coordinator.focus(request("tab-b"), location);
    });
    expect(focus).toHaveBeenCalledTimes(1);

    await act(async () => {
      first.resolve(accepted("tab-a"));
      await settlePromises();
    });
    expect(focus).toHaveBeenCalledTimes(2);
    expect(focus).toHaveBeenLastCalledWith("session-1", request("tab-b"));

    await act(async () => {
      next.resolve(accepted("tab-b"));
      await settlePromises();
    });
  });

  it("does not let a hung old-session request block a new session, but waits when returning", async () => {
    const oldSession = deferred<FocusResponse>();
    const newSession = deferred<FocusResponse>();
    const returnedSession = deferred<FocusResponse>();
    const focus = vi.fn().mockReturnValueOnce(oldSession.promise).mockReturnValueOnce(newSession.promise).mockReturnValueOnce(returnedSession.promise);
    const dispatch = vi.fn();
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, dispatch);
    mounted.push(coordinator);

    await act(async () => {
      coordinator.focus(request("tab-old"), location);
      coordinator.reset();
      coordinator.setState(liveState("session-2", 2));
      coordinator.focus(request("tab-new"), location);
    });
    expect(focus).toHaveBeenCalledTimes(2);
    expect(focus).toHaveBeenLastCalledWith("session-2", request("tab-new"));

    await act(async () => {
      coordinator.reset();
      coordinator.setState(liveState("session-1", 3));
      coordinator.focus(request("tab-returned"), location);
    });
    expect(focus).toHaveBeenCalledTimes(2);

    await act(async () => {
      oldSession.resolve(accepted("tab-old"));
      await settlePromises();
    });
    expect(focus).toHaveBeenCalledTimes(3);
    expect(focus).toHaveBeenLastCalledWith("session-1", request("tab-returned"));

    await act(async () => {
      returnedSession.resolve(accepted("tab-returned"));
      await settlePromises();
    });
  });

  it("keeps a single rejected request retryable without automatically retrying it", async () => {
    const first = deferred<FocusResponse>();
    const focus = vi.fn().mockReturnValue(first.promise);
    const dispatch = vi.fn();
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, dispatch);
    mounted.push(coordinator);

    await act(async () => {
      coordinator.focus(request("tab-a"), location);
      first.reject(new Error("offline"));
      await settlePromises();
    });
    expect(focus).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenLastCalledWith(expect.objectContaining({
      type: "focus/error",
      code: "focus_error",
      message: "offline",
    }));
  });


  it.each(["pane", "agent", "tab", "space"] as const)("skips confirmed %s focus without creating input pending or an echo", async (kind) => {
    const focus = vi.fn();
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
    mounted.push(coordinator);
    const target_id = kind === "space" ? "space-1" : kind === "tab" ? "tab-a" : "pane-a";
    coordinator.setState({ ...liveState(), snapshot: snapshot() });
    await act(async () => { coordinator.focus({ kind, target_id }, location); });
    expect(focus).not.toHaveBeenCalled();
    expect(coordinator.getState().focusPending).toBeNull();
    expect(coordinator.getEchoes()).toEqual([]);
  });

  it("does not treat a stale snapshot as focus confirmation", async () => {
    const response = deferred<FocusResponse>();
    const focus = vi.fn().mockReturnValue(response.promise);
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
    mounted.push(coordinator);
    coordinator.setState({ ...liveState(), sync: "stale", snapshot: snapshot() });
    await act(async () => { coordinator.focus({ kind: "pane", target_id: "pane-a" }, location); });
    expect(focus).toHaveBeenCalledWith("session-1", { kind: "pane", target_id: "pane-a" });
    expect(coordinator.getState().focusPending).toEqual({ kind: "pane", target_id: "pane-a" });
  });

  it("supersedes input intent and unsent work without losing an issued echo when its acknowledgement arrives", async () => {
    vi.useFakeTimers();
    try {
      const response = deferred<FocusResponse>();
      const focus = vi.fn().mockReturnValue(response.promise);
      const onTimeout = vi.fn();
      const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn(), { onTimeout });
      mounted.push(coordinator);
      await act(async () => {
        coordinator.focus(request("tab-a"), location);
        coordinator.focus(request("tab-b"), location);
        coordinator.supersedeSelection();
        response.resolve(accepted("tab-a"));
        await settlePromises();
        await vi.advanceTimersByTimeAsync(FOCUS_FALLBACK_MS);
      });
      expect(focus).toHaveBeenCalledTimes(1);
      expect(onTimeout).not.toHaveBeenCalled();
      expect(coordinator.getState().focusPending).toBeNull();
      expect(coordinator.getState().focusError).toBeNull();
      expect(coordinator.getEchoes()).toEqual([{ token: 1, kind: "tab", targetId: "tab-a" }]);
      coordinator.consumeEcho(1);
      expect(coordinator.getEchoes()).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("retains an acknowledged echo after input confirmation is reconciled", async () => {
    const focus = vi.fn().mockResolvedValue(accepted("tab-a"));
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
    mounted.push(coordinator);
    await act(async () => { coordinator.focus(request("tab-a"), location); await settlePromises(); });
    const confirmed = { ...coordinator.getState(), snapshot: snapshot(), focusPending: null };
    coordinator.setState(confirmed);
    await act(async () => { coordinator.reconcile(confirmed, { ...location, paneId: "pane-a" }, null); });
    expect(coordinator.getEchoes()).toEqual([{ token: 1, kind: "tab", targetId: "tab-a" }]);
  });

  it("does not time out an acknowledged request after a viewer supersedes its input intent", async () => {
    vi.useFakeTimers();
    try {
      const onTimeout = vi.fn();
      const coordinator = mountCoordinator({ focus: vi.fn().mockResolvedValue(accepted("tab-a")) } as unknown as CockpitClient, vi.fn(), { onTimeout });
      mounted.push(coordinator);
      await act(async () => {
        coordinator.focus(request("tab-a"), location);
        await settlePromises();
        coordinator.supersedeSelection();
        await vi.advanceTimersByTimeAsync(FOCUS_FALLBACK_MS);
      });
      expect(onTimeout).not.toHaveBeenCalled();
      expect(coordinator.getState().focusError).toBeNull();
      expect(coordinator.getEchoes()).toEqual([{ token: 1, kind: "tab", targetId: "tab-a" }]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not resurrect an echo consumed by a stream snapshot before its HTTP acknowledgement", async () => {
    const response = deferred<FocusResponse>();
    const coordinator = mountCoordinator({ focus: vi.fn().mockReturnValue(response.promise) } as unknown as CockpitClient, vi.fn());
    mounted.push(coordinator);
    await act(async () => {
      coordinator.focus(request("tab-a"), location);
      coordinator.consumeEcho(1);
      response.resolve(accepted("tab-a"));
      await settlePromises();
    });
    expect(coordinator.getEchoes()).toEqual([]);
  });

  it("does not expose unissued prepare work as an echo and cancels it when a viewer is selected", async () => {
    vi.useFakeTimers();
    try {
      const focus = vi.fn();
      const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
      mounted.push(coordinator);
      await act(async () => {
        coordinator.focus(request("tab-a"), location, { paneId: "pane-a" });
        expect(coordinator.getEchoes()).toEqual([]);
        coordinator.supersedeSelection();
        coordinator.panePrepared("pane-a");
        await vi.advanceTimersByTimeAsync(FOCUS_PREPARE_MS);
      });
      expect(focus).not.toHaveBeenCalled();
      expect(coordinator.getState().focusPending).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("removes a rejected old request from echo candidates even after a newer selection", async () => {
    const response = deferred<FocusResponse>();
    const focus = vi.fn().mockReturnValue(response.promise);
    const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
    mounted.push(coordinator);
    await act(async () => {
      coordinator.focus(request("tab-a"), location);
      coordinator.supersedeSelection();
      response.resolve({ ...accepted("tab-a"), accepted: false });
      await settlePromises();
    });
    expect(coordinator.getEchoes()).toEqual([]);
    expect(coordinator.getState().focusError).toBeNull();
  });
  describe("preparing the target pane before Herdr focuses its tab", () => {
    it("sends focus once the pane reports its first frame, not before", async () => {
      const focus = vi.fn().mockResolvedValue(accepted("tab-a"));
      const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
      mounted.push(coordinator);
      await act(async () => { coordinator.focus(request("tab-a"), location, { paneId: "pane-a" }); });
      expect(focus).not.toHaveBeenCalled();
      await act(async () => { coordinator.panePrepared("other-pane"); });
      expect(focus).not.toHaveBeenCalled();
      await act(async () => { coordinator.panePrepared("pane-a"); await settlePromises(); });
      expect(focus).toHaveBeenCalledTimes(1);
    });

    it("focuses anyway when the pane never attaches", async () => {
      vi.useFakeTimers();
      try {
        const focus = vi.fn().mockResolvedValue(accepted("tab-a"));
        const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
        mounted.push(coordinator);
        await act(async () => { coordinator.focus(request("tab-a"), location, { paneId: "pane-a" }); });
        await act(async () => { await vi.advanceTimersByTimeAsync(FOCUS_PREPARE_MS - 1); });
        expect(focus).not.toHaveBeenCalled();
        await act(async () => { await vi.advanceTimersByTimeAsync(1); });
        expect(focus).toHaveBeenCalledTimes(1);
      } finally {
        vi.useRealTimers();
      }
    });

    it("drops a waiting request when a newer focus supersedes it", async () => {
      const focus = vi.fn().mockResolvedValue(accepted("tab-b"));
      const coordinator = mountCoordinator({ focus } as unknown as CockpitClient, vi.fn());
      mounted.push(coordinator);
      await act(async () => {
        coordinator.focus(request("tab-a"), location, { paneId: "pane-a" });
        coordinator.focus(request("tab-b"), location);
        await settlePromises();
      });
      expect(focus).toHaveBeenCalledTimes(1);
      expect(focus).toHaveBeenLastCalledWith("session-1", request("tab-b"));
      await act(async () => { coordinator.panePrepared("pane-a"); await settlePromises(); });
      expect(focus).toHaveBeenCalledTimes(1);
    });
  });
});
