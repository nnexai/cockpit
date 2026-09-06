// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest, FocusResponse } from "../../protocol/generated/v1";
import type { SessionAction, SessionState } from "./sessionStore";
import { useFocusCoordinator, type FocusLocation } from "./focusCoordinator";

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

function request(id: string): FocusRequest {
  return { kind: "tab", target_id: id };
}

function accepted(id: string): FocusResponse {
  return { session_id: "session-1", kind: "tab", target_id: id, accepted: true };
}

const location: FocusLocation = { spaceId: "space-1", tabId: null, paneId: null };

type MountedCoordinator = {
  focus(request: FocusRequest, location: FocusLocation): void;
  reset(): void;
  setState(state: SessionState): void;
  unmount(): Promise<void>;
};

function mountCoordinator(client: CockpitClient, dispatch: (action: SessionAction) => void): MountedCoordinator {
  const stateRef = { current: liveState() };
  const mountedRef = { current: true };
  let coordinator: ReturnType<typeof useFocusCoordinator> | null = null;
  function Harness() {
    coordinator = useFocusCoordinator({
      client,
      stateRef,
      mountedRef,
      dispatch,
      describeError: (error, fallback) => ({ code: "focus_error", message: error instanceof Error ? error.message : fallback }),
      onTimeout: () => undefined,
    });
    return null;
  }
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const root = createRoot(host);
  act(() => { root.render(<Harness />); });
  return {
    focus(request, nextLocation) { coordinator!.focus(request, nextLocation); },
    reset() { coordinator!.reset(); },
    setState(state) { stateRef.current = state; },
    async unmount() { await act(async () => { root.unmount(); }); host.remove(); },
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
    expect(dispatch.mock.calls.map(([action]) => action.type)).toEqual(["focus/request", "focus/request"]);
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
});
