import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResourceMutationRequest, ResourceMutationResponse, SessionSnapshotResponse, SessionSummary, SpaceGitSummary, SpaceSummary, TabLayout, TerminalCommand } from "../protocol/generated/v1";
import {
  authoritativeMutationSnapshot,
  authoritativeSelection,
  canSwitchSessions,
  contextMenuPosition,
  deriveResizeHandles,
  initialMutationCoordinatorState,
  mutationFailureCanRetry,
  moveDestinationLabel,
  mutationCoordinatorReducer,
  projectedPaneIds,
  nextModalFocusIndex,
  projectedPaneRect,
  resizeRequest,
  scheduleFocusFallback,
  reconcileSessionChoice,
  prefixCommandForKey,
  spaceDropBeforeId,
  tabDropInsertionIndex,
  projectSpaceTree,
  spaceStatus,
  tabLabelIsRedundant,
} from "./App";
import { appendPendingControlCommand, MAX_PENDING_CONTROL_COMMANDS, terminalCellPosition } from "./TerminalPane";

function snapshot(sessionId = "session-1", focusedPaneId = "pane-1"): SessionSnapshotResponse {
  return {
    session_id: sessionId,
    version: "0.8.2",
    protocol: 20,
    focused_space_id: "space-1",
    focused_tab_id: "tab-1",
    focused_pane_id: focusedPaneId,
    spaces: [{ id: "space-1", label: "Space", number: 1, tab_count: 1, pane_count: 2, focused: true, agent_status: "idle", git: null }],
    tabs: [{ id: "tab-1", space_id: "space-1", label: "Tab", number: 1, pane_count: 2, focused: true }],
    panes: ["pane-1", "pane-2"].map((id) => ({ id, terminal_id: `terminal-${id}`, space_id: "space-1", tab_id: "tab-1", title: null, focused: id === focusedPaneId, agent: null, agent_status: "idle", revision: 1 })),
    layouts: [],
    agents: [],
  };
}

function space(id: string, label: string, git: SpaceGitSummary | null = null, agentStatus = "idle"): SpaceSummary {
  return { id, label, number: 1, tab_count: 0, pane_count: 0, focused: false, agent_status: agentStatus, git };
}

function git(repositoryKey: string, branch: string, isLinkedWorktree: boolean): SpaceGitSummary {
  return { repository_key: repositoryKey, repository: "repo", branch, checkout_path: "/checkout", is_linked_worktree: isLinkedWorktree };
}

describe("Space tree projection", () => {
  it("leaves non-Git, singleton Git, and all-linked Spaces ungrouped in source order", () => {
    const spaces = [
      space("plain", "plain"),
      space("single", "single", git("single-repo", "main", false)),
      space("linked-1", "linked one", git("linked-repo", "worktree/one", true)),
      space("linked-2", "linked two", git("linked-repo", "worktree/two", true)),
    ];
    const rows = projectSpaceTree(spaces);
    expect(rows.map((row) => [row.kind, row.space.id])).toEqual([
      ["top-level", "plain"],
      ["top-level", "single"],
      ["top-level", "linked-1"],
      ["top-level", "linked-2"],
    ]);
  });

  it("emits the first non-linked parent followed by children in authoritative source order", () => {
    const spaces = [
      space("w1C", "worktree one", git("lilygo", "worktree/brave-forest-7518", true)),
      space("w18", "lilygo-t3", git("lilygo", "main", false)),
      space("w1D", "worktree two", git("lilygo", "worktree/brave-stone-13f0", true)),
      space("cockpit", "cockpit", git("cockpit", "main", false)),
    ];
    const rows = projectSpaceTree(spaces);
    expect(rows.map((row) => [row.kind, row.space.id, row.label, row.branch])).toEqual([
      ["parent", "w18", "lilygo-t3", "main"],
      ["child", "w1C", "brave-forest-7518", "worktree/brave-forest-7518"],
      ["child", "w1D", "brave-stone-13f0", "worktree/brave-stone-13f0"],
      ["top-level", "cockpit", "cockpit", "main"],
    ]);
  });

  it("strips only an exact worktree prefix from child branch labels", () => {
    const spaces = [
      space("parent", "repo", git("repo", "main", false)),
      space("exact", "exact fallback", git("repo", "worktree/topic", true)),
      space("embedded", "embedded fallback", git("repo", "feature/worktree/topic", true)),
    ];
    expect(projectSpaceTree(spaces).map((row) => row.label)).toEqual(["repo", "topic", "feature/worktree/topic"]);
  });

  it("retains only the selected child beneath a collapsed parent", () => {
    const spaces = [
      space("parent", "repo", git("repo", "main", false)),
      space("child-1", "one", git("repo", "worktree/one", true)),
      space("child-2", "two", git("repo", "worktree/two", true)),
    ];
    const rows = projectSpaceTree(spaces, new Set(["repo"]), "child-2");
    expect(rows.map((row) => [row.kind, row.space.id, row.expanded, row.connector])).toEqual([
      ["parent", "parent", false, null],
      ["child", "child-2", false, "└─"],
    ]);
  });

  it("never duplicates a grouped Space row", () => {
    const spaces = [
      space("child-before", "before", git("repo", "worktree/before", true)),
      space("parent", "repo", git("repo", "main", false)),
      space("second-parent", "repo clone", git("repo", "release", false)),
      space("child-after", "after", git("repo", "worktree/after", true)),
    ];
    const ids = projectSpaceTree(spaces).map((row) => row.space.id);
    expect(ids).toEqual(["parent", "child-before", "second-parent", "child-after"]);
    expect(new Set(ids).size).toBe(spaces.length);
  });

  it("maps Space agent status to its glyph and class", () => {
    expect(spaceStatus("blocked")).toEqual({ glyph: "●", className: "blocked" });
    expect(spaceStatus("running")).toEqual({ glyph: "◐", className: "working" });
    expect(spaceStatus("complete")).toEqual({ glyph: "●", className: "done" });
    expect(spaceStatus("idle")).toEqual({ glyph: "○", className: "idle" });
    expect(spaceStatus("unexpected")).toEqual({ glyph: "·", className: "unknown" });
  });
});

const firstOperation = { epoch: 1, token: 1, key: "pane:pane-1", request: { type: "pane_zoom", pane_id: "pane-1", mode: "toggle" } as const, focusFromSnapshot: false };

describe("mutation coordination", () => {
  it("keeps the first pending operation and rejects stale responses", () => {
    let state = mutationCoordinatorReducer(initialMutationCoordinatorState, { type: "begin", operation: firstOperation });
    const secondOperation = { ...firstOperation, token: 2, key: "pane:pane-2", request: { type: "pane_close", pane_id: "pane-2" } as const };
    const ignoredSecond = mutationCoordinatorReducer(state, { type: "begin", operation: secondOperation });
    expect(ignoredSecond).toBe(state);
    const staleToken = mutationCoordinatorReducer(state, { type: "succeed", epoch: 1, token: 2 });
    const staleEpoch = mutationCoordinatorReducer(state, { type: "fail", epoch: 0, token: 1, error: { message: "late" } });
    expect(staleToken).toBe(state);
    expect(staleEpoch).toBe(state);
    expect(state.pending).toEqual(firstOperation);
  });

  it("clears stale mutation failures after authoritative resync", () => {
    let state = mutationCoordinatorReducer(initialMutationCoordinatorState, { type: "begin", operation: firstOperation });
    state = mutationCoordinatorReducer(state, { type: "fail", epoch: 1, token: 1, error: { code: "transport_error", message: "uncertain" } });
    expect(Object.keys(state.errors)).toEqual(["pane:pane-1"]);
    expect(mutationCoordinatorReducer(state, { type: "reset" })).toEqual(initialMutationCoordinatorState);
  });

  it("adopts only a parser-validated authoritative mutation snapshot for the requested session", () => {
    const response: ResourceMutationResponse = { session_id: "session-1", snapshot: snapshot() };
    expect(authoritativeMutationSnapshot("session-1", response)).toEqual(response.snapshot);
    expect(() => authoritativeMutationSnapshot("other-session", response)).toThrow(/another session/);
    expect(() => authoritativeMutationSnapshot("session-1", { ...response, snapshot: { ...response.snapshot, panes: [{}] } as SessionSnapshotResponse })).toThrow();
  });
});

describe("resource drop boundaries", () => {
  it("uses Herdr pre-removal tab insertion boundaries without sending no-ops", () => {
    expect(tabDropInsertionIndex(0, 1, false)).toBeNull();
    expect(tabDropInsertionIndex(0, 1, true)).toBe(2);
    expect(tabDropInsertionIndex(0, 2, false)).toBe(2);
    expect(tabDropInsertionIndex(0, 2, true)).toBe(3);
    expect(tabDropInsertionIndex(2, 0, false)).toBe(0);
    expect(tabDropInsertionIndex(2, 0, true)).toBe(1);
    expect(tabDropInsertionIndex(1, 1, true)).toBeNull();
  });

  it("converts Space target halves to Herdr before anchors", () => {
    const spaces = ["a", "b", "c"].map((id) => space(id, id));
    expect(spaceDropBeforeId(spaces, "a", "b", false)).toBeUndefined();
    expect(spaceDropBeforeId(spaces, "a", "b", true)).toBe("c");
    expect(spaceDropBeforeId(spaces, "c", "a", false)).toBe("a");
    expect(spaceDropBeforeId(spaces, "c", "a", true)).toBe("b");
    expect(spaceDropBeforeId(spaces, "b", "b", true)).toBeUndefined();
  });
});

describe("desktop command routing", () => {
  it("clips fixed context menus to the viewport gutter", () => {
    expect(contextMenuPosition(790, 590, 800, 600, 208, 320)).toEqual({ x: 584, y: 272 });
    expect(contextMenuPosition(-20, -10, 800, 600, 208, 320)).toEqual({ x: 8, y: 8 });
  });

  it("omits Herdr's numeric automatic tab labels", () => {
    expect(tabLabelIsRedundant("1", 1)).toBe(true);
    expect(tabLabelIsRedundant(" 5 ", 4)).toBe(true);
    expect(tabLabelIsRedundant("shell", 1)).toBe(false);
  });

  it("maps wheel pointers to bounded zero-based terminal cells", () => {
    const bounds = { left: 100, top: 50, width: 800, height: 400 };
    expect(terminalCellPosition(500, 250, bounds, 80, 20)).toEqual({ column: 40, row: 10 });
    expect(terminalCellPosition(99, 49, bounds, 80, 20)).toEqual({ column: 0, row: 0 });
    expect(terminalCellPosition(999, 999, bounds, 80, 20)).toEqual({ column: 79, row: 19 });
  });


  it("offers session switching only when another session exists", () => {
    expect(canSwitchSessions(0)).toBe(false);
    expect(canSwitchSessions(1)).toBe(false);
    expect(canSwitchSessions(2)).toBe(true);
  });
  it("maps Herdr prefix keys without treating unmodified variants as destructive commands", () => {
    expect(prefixCommandForKey("N", true)).toBe("new-space");
    expect(prefixCommandForKey("W", true)).toBe("rename-space");
    expect(prefixCommandForKey("D", true)).toBe("close-space");
    expect(prefixCommandForKey("c", false)).toBe("new-tab");
    expect(prefixCommandForKey("T", true)).toBe("rename-tab");
    expect(prefixCommandForKey("X", true)).toBe("close-tab");
    expect(prefixCommandForKey("P", true)).toBe("rename-pane");
    expect(prefixCommandForKey("v", false)).toBe("split-right");
    expect(prefixCommandForKey("?", false)).toBe("help");
    expect(prefixCommandForKey("p", false)).toBe("previous-tab");
    expect(prefixCommandForKey("n", false)).toBe("next-tab");
    expect(prefixCommandForKey("z", false)).toBe("zoom-pane");
    expect(prefixCommandForKey("r", false)).toBe("resize");
    expect(prefixCommandForKey("-", false)).toBe("split-down");
    expect(prefixCommandForKey("x", false)).toBe("close-pane");
    expect(prefixCommandForKey("D", false)).toBeNull();
  });

  it("replays only idempotent absolute mutations", () => {
    const retryable: ResourceMutationRequest[] = [
      { type: "space_rename", space_id: "space-1", label: "Main" },
      { type: "space_move_block", space_ids: ["space-1"], before_space_id: null },
      { type: "tab_rename", tab_id: "tab-1", label: "Shell" },
      { type: "tab_move", tab_id: "tab-1", insert_index: 0 },
      { type: "pane_rename", pane_id: "pane-1", label: "Logs" },
      { type: "pane_zoom", pane_id: "pane-1", mode: "on" },
      { type: "pane_zoom", pane_id: "pane-1", mode: "off" },
    ];
    const ambiguous: ResourceMutationRequest[] = [
      { type: "space_create", cwd: null, label: null },
      { type: "tab_create", space_id: "space-1", label: null },
      { type: "pane_split", pane_id: "pane-1", direction: "right", ratio: null },
      { type: "pane_resize", pane_id: "pane-1", direction: "right", amount: 0.1 },
      { type: "pane_swap", source_pane_id: "pane-1", target_pane_id: "pane-2" },
      { type: "pane_move", pane_id: "pane-1", destination: { type: "new_space", label: null, tab_label: null } },
      { type: "pane_close", pane_id: "pane-1" },
      { type: "pane_zoom", pane_id: "pane-1", mode: "toggle" },
    ];
    retryable.forEach((request) => expect(mutationFailureCanRetry(request, "transport_error")).toBe(true));
    ambiguous.forEach((request) => expect(mutationFailureCanRetry(request, "transport_error")).toBe(false));
    retryable.forEach((request) => expect(mutationFailureCanRetry(request, "mutation_applied_snapshot_failed")).toBe(false));
  });

  it("reconciles removed sessions and exposes human move destinations", () => {
    const sessions: SessionSummary[] = [
      { id: "session-1", label: "One", is_default: true, running: true },
      { id: "session-2", label: "Two", is_default: false, running: true },
    ];
    expect(reconcileSessionChoice(sessions, "removed", "session-2")).toBe("session-2");
    expect(reconcileSessionChoice([], "removed", "session-2")).toBe("");
    const source = snapshot();
    source.spaces.push({ id: "space-2", label: "Ops", number: 2, tab_count: 1, pane_count: 1, focused: false, agent_status: "unknown", git: null });
    expect(moveDestinationLabel({ ...source.tabs[0], id: "internal-tab", space_id: "space-2", label: "Logs", number: 4 }, source.spaces)).toBe("Ops / Logs");
    expect(moveDestinationLabel({ ...source.tabs[0], id: "internal-tab", space_id: "space-2", label: "", number: 4 }, source.spaces)).toBe("Ops / Tab 4");
  });

  it("wraps modal tab focus in both directions", () => {
    expect(nextModalFocusIndex(2, 3, false)).toBe(0);
    expect(nextModalFocusIndex(0, 3, true)).toBe(2);
    expect(nextModalFocusIndex(-1, 3, false)).toBe(0);
  });

  it("bounds pending terminal input while retaining the newest commands", () => {
    let queue: TerminalCommand[] = [];
    for (let index = 0; index < MAX_PENDING_CONTROL_COMMANDS + 3; index += 1) {
      queue = appendPendingControlCommand(queue, { type: "terminal.input", text: String(index), bytes: null });
    }
    expect(queue).toHaveLength(MAX_PENDING_CONTROL_COMMANDS);
    expect(queue[0]).toMatchObject({ text: "3" });
    expect(queue.at(-1)).toMatchObject({ text: String(MAX_PENDING_CONTROL_COMMANDS + 2) });
  });
});

describe("pane interactions", () => {
  const layout: TabLayout = {
    space_id: "space-1",
    tab_id: "tab-1",
    area: { x: 0, y: 0, width: 80, height: 24 },
    focused_pane_id: "pane-1",
    zoomed: false,
    panes: [
      { pane_id: "pane-1", focused: true, rect: { x: 0, y: 0, width: 40, height: 24 } },
      { pane_id: "pane-2", focused: false, rect: { x: 40, y: 0, width: 40, height: 24 } },
      { pane_id: "pane-floating", focused: false, rect: { x: 100, y: 30, width: 10, height: 10 } },
    ],
  };

  it("creates handles only for shared edges and normalizes resize direction and amount", () => {
    const handles = deriveResizeHandles(layout);
    expect(handles).toHaveLength(1);
    expect(handles[0]).toMatchObject({ paneId: "pane-1", axis: "x", positiveDirection: "right" });
    expect(resizeRequest(handles[0], 100, 1000)).toEqual({ type: "pane_resize", pane_id: "pane-1", direction: "right", amount: 0.1 });
    expect(resizeRequest(handles[0], -50, 1000)).toEqual({ type: "pane_resize", pane_id: "pane-1", direction: "left", amount: 0.05 });
    expect(resizeRequest(handles[0], 0, 1000)).toBeNull();
  });

  it("projects exactly the focused pane over the full canvas while zoomed and restores every pane when unzoomed", () => {
    const paneIds = ["pane-1", "pane-2"];
    const zoomed = { ...layout, zoomed: true, focused_pane_id: "pane-2" };
    expect(projectedPaneIds(paneIds, zoomed, "pane-1")).toEqual(["pane-2"]);
    expect(projectedPaneRect(zoomed, "pane-2")).toEqual(layout.area);
    expect(projectedPaneIds(paneIds, layout, "pane-2")).toEqual(paneIds);
    expect(projectedPaneRect(layout, "pane-2")).toEqual(layout.panes[1].rect);
  });

  it("hands selection and input ownership to the pane focused by an authoritative split snapshot", () => {
    const afterSplit = snapshot("session-1", "pane-2");
    expect(authoritativeSelection(afterSplit)).toEqual({ spaceId: "space-1", tabId: "tab-1", paneId: "pane-2" });
    expect(authoritativeMutationSnapshot("session-1", { session_id: "session-1", snapshot: afterSplit }).focused_pane_id).toBe("pane-2");
  });
});

describe("focus fallback", () => {
  afterEach(() => vi.useRealTimers());

  it("waits 500ms before showing pending feedback and resyncing, and honors cancellation", () => {
    vi.useFakeTimers();
    const delayed = vi.fn();
    const resync = vi.fn();
    const cancel = scheduleFocusFallback(() => true, delayed, resync);
    vi.advanceTimersByTime(499);
    expect(delayed).not.toHaveBeenCalled();
    expect(resync).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(delayed).toHaveBeenCalledOnce();
    expect(resync).toHaveBeenCalledOnce();

    const cancelledDelayed = vi.fn();
    const cancelled = scheduleFocusFallback(() => true, cancelledDelayed, vi.fn());
    cancelled();
    vi.runAllTimers();
    expect(cancelledDelayed).not.toHaveBeenCalled();
    cancel();
  });
});
