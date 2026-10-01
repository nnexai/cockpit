// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { SpaceGitActionRequest, SpaceGitActionResponse, SpaceGitStatus } from "../../protocol/generated/v1";
import { gitActionReason, gitActionRequest, gitFailureNote, gitOutcomeNote, useSpaceGitActions, type SpaceGitActions } from "./spaceGitActions";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

function tracked(space_id = "child", root = "/repo/worktree", ahead = 1, behind = 0): SpaceGitStatus {
  return { space_id, source: "herdr_checkout", checkout: { state: "branch", root, branch: "feature", upstream: { state: "tracked", name: "origin/main", ahead, behind } } };
}
const request: SpaceGitActionRequest = { space_id: "child", action: "push", expected_root: "/repo/worktree", expected_branch: "feature", expected_upstream: "origin/main" };

it("allows pulling cached ahead-only and level branches, and blocks unsafe cached positions", () => {
  expect(gitActionReason(tracked(), "pull")).toBeUndefined();
  expect(gitActionReason(tracked("child", "/repo/worktree", 0, 0), "pull")).toBeUndefined();
  expect(gitActionRequest(tracked(), "push")).toEqual(request);
  expect(gitActionRequest(tracked("child", "/repo/worktree", 1, 1), "pull")).toBeNull();
  expect(gitActionRequest(tracked("child", "/repo/worktree", 1, 1), "push")).toBeNull();
  expect(gitActionRequest(tracked("child", "/repo/worktree", 0, 1), "push")).toBeNull();
});

it("does not redirect a worktree action to its parent or invent an upstream", () => {
  const parent = tracked("parent", "/repo", 0, 1);
  expect(gitActionRequest(parent, "pull")?.expected_root).toBe("/repo");
  expect(gitActionRequest(tracked(), "pull")?.expected_root).toBe("/repo/worktree");
  for (const checkout of [
    { state: "detached" as const, root: "/repo" },
    { state: "unavailable" as const, root: null, code: "read_failed", message: "cannot read checkout" },
    ...([{ state: "none" }, { state: "gone", name: "origin/main" }, { state: "local", name: "main" }, { state: "unavailable", name: "origin/main", code: "read_failed", message: "failed" }] as const).map(upstream => ({ state: "branch" as const, root: "/repo", branch: "main", upstream })),
  ]) {
    const status: SpaceGitStatus = { space_id: "parent", source: "pane_folder", checkout };
    expect(gitActionRequest(status, "pull")).toBeNull();
    expect(gitActionRequest(status, "push")).toBeNull();
    expect(gitActionReason(status, "pull")).toBeTruthy();
  }
});

it.each(["space_git_target_changed", "space_git_action_ineligible", "space_git_action_in_progress", "space_git_not_run", "invalid_session_id"])("claims not-run only for proven %s", operationCode => {
  const note = gitFailureNote(new CockpitClientError("http_error", "target changed", { operationCode }), request);
  expect(note.message).toContain("was not run");
});

it.each([undefined, "malformed_response", "space_git_outcome_unknown", "some_backend_error"])("keeps %s errors uncertain", operationCode => {
  const note = gitFailureNote(new CockpitClientError("transport_error", "connection lost", { operationCode }), request);
  expect(note.message).toContain("may or may not");
  expect(note.message).not.toContain("was not run");
});

it("limits definitive pull refusal claims to branch and working files, not all Git state", () => {
  const pull = { ...request, action: "pull" as const };
  const refused = gitOutcomeNote({ result: "refused", reason: "local_changes", detail: "local changes" }, pull);
  expect(refused.message).toContain("branch and files are unchanged");
  expect(refused.message).not.toContain("nothing changed");
});

afterEach(() => { vi.useRealTimers(); });

describe("checkout-scoped write state", () => {
  it("single-flights a shared root while another checkout stays actionable, and settles independent notes", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    vi.useFakeTimers();
    const completions: ((response: SpaceGitActionResponse) => void)[] = [];
    const spaceGitAction = vi.fn(() => {
      const { promise, resolve } = deferred<SpaceGitActionResponse>();
      completions.push(resolve);
      return promise;
    });
    const client = { spaceGitAction } as unknown as CockpitClient;
    const statuses = new Map([tracked(), tracked("alias"), tracked("parent", "/repo")].map(status => [status.space_id, status]));
    const refresh = vi.fn();
    const onResult = vi.fn();
    let actions!: SpaceGitActions;
    function Probe() { actions = useSpaceGitActions(client, "session", statuses, refresh, onResult); return null; }
    const root = createRoot(document.createElement("div"));
    try {
      await act(async () => root.render(<Probe />));
      act(() => actions.run(request));
      act(() => actions.run({ ...request, space_id: "alias" }));
      act(() => actions.run({ ...request, space_id: "parent", expected_root: "/repo" }));
      expect(spaceGitAction).toHaveBeenCalledTimes(2);
      expect(actions.forStatus(statuses.get("alias"))?.pending).toBe(true);
      expect(actions.forStatus(statuses.get("parent"))?.pending).toBe(true);
      await act(async () => completions[0]({ session_id: "session", space_id: "child", action: "push", root: request.expected_root, branch: "feature", upstream: "origin/main", outcome: { result: "refused", reason: "remote_rejected", detail: "remote rejected push" } }));
      expect(actions.forStatus(statuses.get("alias"))?.pending).toBe(false);
      expect(actions.forStatus(statuses.get("alias"))?.note?.tone).toBe("error");
      expect(actions.forStatus(statuses.get("parent"))?.pending).toBe(true);
      act(() => actions.dismiss(request.expected_root));
      expect(actions.forStatus(statuses.get("child"))).toBeUndefined();
      expect(refresh).toHaveBeenCalledTimes(1);
    } finally { await act(async () => root.unmount()); }
  });

  it("does not attach a stale result to a retargeted Space or a different session", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    vi.useFakeTimers();
    let complete!: (response: SpaceGitActionResponse) => void;
    const client = { spaceGitAction: vi.fn(() => {
      const { promise, resolve } = deferred<SpaceGitActionResponse>();
      complete = resolve;
      return promise;
    }) } as unknown as CockpitClient;
    let statuses = new Map([["child", tracked()]]);
    let session = "session";
    const refresh = vi.fn();
    const onResult = vi.fn();
    let actions!: SpaceGitActions;
    function Probe() { actions = useSpaceGitActions(client, session, statuses, refresh, onResult); return null; }
    const root = createRoot(document.createElement("div"));
    try {
      await act(async () => root.render(<Probe />));
      act(() => actions.run(request));
      statuses = new Map([["child", tracked("child", "/other")]]);
      await act(async () => root.render(<Probe />));
      await act(async () => complete({ session_id: "session", space_id: "child", action: "push", root: request.expected_root, branch: "feature", upstream: "origin/main", outcome: { result: "updated", commits: 1 } }));
      expect(actions.forStatus(statuses.get("child"))).toBeUndefined();
      expect(onResult).not.toHaveBeenCalled();
      expect(refresh).toHaveBeenCalledTimes(1);
      statuses = new Map([["child", tracked()]]);
      await act(async () => root.render(<Probe />));
      act(() => actions.run(request));
      session = "other-session";
      await act(async () => root.render(<Probe />));
      await act(async () => complete({ session_id: "session", space_id: "child", action: "push", root: request.expected_root, branch: "feature", upstream: "origin/main", outcome: { result: "up_to_date" } }));
      expect(actions.forStatus(statuses.get("child"))).toBeUndefined();
      expect(onResult).not.toHaveBeenCalled();
      expect(refresh).toHaveBeenCalledTimes(1);
    } finally { await act(async () => root.unmount()); }
  });

  it("keeps a newer pending action through an older success timer, then records an uncertain failure without retrying", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    vi.useFakeTimers();
    const first = deferred<SpaceGitActionResponse>();
    const second = deferred<SpaceGitActionResponse>();
    const spaceGitAction = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const client = { spaceGitAction } as unknown as CockpitClient;
    const status = tracked();
    const statuses = new Map([[status.space_id, status]]);
    const refresh = vi.fn();
    let actions!: SpaceGitActions;
    function Probe() { actions = useSpaceGitActions(client, "session", statuses, refresh, () => undefined); return null; }
    const root = createRoot(document.createElement("div"));
    try {
      await act(async () => root.render(<Probe />));
      act(() => actions.run(request));
      await act(async () => first.resolve({ session_id: "session", space_id: "child", action: "push", root: "/repo/worktree", branch: "feature", upstream: "origin/main", outcome: { result: "up_to_date" } }));
      expect(actions.forStatus(status)?.pending).toBe(false);
      act(() => actions.run(request));
      await act(async () => vi.advanceTimersByTimeAsync(6000));
      expect(actions.forStatus(status)?.pending).toBe(true);
      await act(async () => second.reject(new CockpitClientError("transport_error", "disconnected")));
      expect(actions.forStatus(status)?.pending).toBe(false);
      expect(actions.forStatus(status)?.note?.message).toContain("may or may not");
      await act(async () => vi.advanceTimersByTimeAsync(60_000));
      expect(spaceGitAction).toHaveBeenCalledTimes(2);
      expect(refresh).toHaveBeenCalledTimes(2);
    } finally { await act(async () => root.unmount()); }
  });
});
