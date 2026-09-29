import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResourceMutationRequest, ResourceMutationResponse, SessionSnapshotResponse, SessionSummary, TerminalCommand } from "../protocol/generated/v1";
import {
  authoritativeMutationSnapshot,
  canSwitchSessions,
  contextMenuPosition,
  mutationFailureCanRetry,
  moveDestinationLabel,
  reconcileSessionChoice,
  rendererReasonFor,
  tabLabelIsRedundant,
} from "./App";
import { tabDropInsertionIndex } from "./layout/layoutProjection";
import { initialMutationCoordinatorState, mutationCoordinatorReducer } from "./session/mutationCoordinator";
import { initialSessionState, sessionReducer } from "./session/sessionStore";
import { scheduleFocusFallback } from "./session/focusCoordinator";
import { appendPendingControlCommand, createCockpitTerminal, forwardTerminalMouse, MAX_PENDING_CONTROL_COMMANDS, terminalCellPosition, terminalModifiedEnterInput, terminalMouseButton, terminalMouseCommand } from "./TerminalPane";

function snapshot(sessionId = "session-1", focusedPaneId = "pane-1"): SessionSnapshotResponse {
  return {
    session_id: sessionId,
    server_instance: "0123456789abcdef",
    version: "0.8.2",
    protocol: 20,
    focused_space_id: "space-1",
    focused_tab_id: "tab-1",
    focused_pane_id: focusedPaneId,
    spaces: [{ id: "space-1", label: "Space", number: 1, tab_count: 1, pane_count: 2, focused: true, agent_status: "idle", git: null }],
    tabs: [{ id: "tab-1", space_id: "space-1", label: "Tab", number: 1, pane_count: 2, focused: true, focused_pane_id: focusedPaneId }],
    panes: ["pane-1", "pane-2"].map((id) => ({ id, terminal_id: `terminal-${id}`, space_id: "space-1", tab_id: "tab-1", title: null, focused: id === focusedPaneId, agent: null, agent_status: "idle", revision: 1 })),
    agents: [],
  };
}
it("renders a Herdr mutation snapshot immediately while stream recovery begins", () => {
  const initial = { ...initialSessionState, epoch: 1, sessionId: "session-1", sync: "live" as const, snapshot: snapshot() };
  const updated = { ...snapshot(), panes: snapshot().panes.map((pane) => ({ ...pane, title: "Renamed terminal" })) };
  expect(sessionReducer(initial, { type: "snapshot/authoritative", epoch: 1, sessionId: "session-1", snapshot: updated }).snapshot).toEqual(updated);
});


  it("sends Shift+Enter as the bare line feed preserved by Herdr", () => {
    const event = { type: "keydown", key: "Enter", shiftKey: true, ctrlKey: false, altKey: false, metaKey: false };
    expect(terminalModifiedEnterInput(event)).toBe("\n");
    expect(terminalModifiedEnterInput({ ...event, shiftKey: false })).toBeNull();
    expect(terminalModifiedEnterInput({ ...event, ctrlKey: true })).toBeNull();
    expect(terminalModifiedEnterInput({ ...event, type: "keyup" })).toBeNull();
  });

  it("negotiates Kitty keyboard reporting with terminal applications", async () => {
    const terminal = createCockpitTerminal(16);
    const data = vi.fn();
    terminal.onData(data);

    await new Promise<void>((resolve) => terminal.write("\u001b[?u", resolve));

    expect(data).toHaveBeenCalledWith("\u001b[?0u");
    terminal.dispose();
  });

  it("maps browser pointer coordinates into pane-local terminal coordinates", () => {
    const event = { clientX: 110, clientY: 70, shiftKey: true, ctrlKey: false, altKey: true, metaKey: false };
    const command = terminalMouseCommand(
      "down",
      "left",
      event,
      { left: 10, top: 20, width: 800, height: 400 },
      80,
      40,
    );

    expect(command).toEqual({
      type: "terminal.mouse",
      kind: "down",
      button: "left",
      column: 10,
      row: 5,
      modifiers: 5,
    });
    expect(terminalMouseCommand("moved", null, { ...event, clientX: 10_000, clientY: 10_000 }, { left: 10, top: 20, width: 800, height: 400 }, 80, 40)).toMatchObject({ column: 79, row: 39 });
    expect(terminalMouseButton(0)).toBe("left");
    expect(terminalMouseButton(1)).toBe("middle");
    expect(terminalMouseButton(2)).toBe("right");
    expect(terminalMouseButton(3)).toBeNull();
  });

  it("gates every structured pointer command on host capability", () => {
    const send = vi.fn();
    const event = { clientX: 110, clientY: 70, shiftKey: false, ctrlKey: false, altKey: false, metaKey: false };
    const bounds = { left: 10, top: 20, width: 800, height: 400 };
    const events = [
      ["down", "left"],
      ["moved", null],
      ["drag", "left"],
      ["up", "left"],
    ] as const;

    for (const [kind, button] of events) {
      expect(forwardTerminalMouse(false, send, kind, button, event, bounds, 80, 40)).toBe(false);
    }
    expect(send).not.toHaveBeenCalled();

    for (const [kind, button] of events) {
      expect(forwardTerminalMouse(true, send, kind, button, event, bounds, 80, 40)).toBe(true);
    }
    expect(send.mock.calls.map(([command]) => command.kind)).toEqual(["down", "moved", "drag", "up"]);
  });

const firstOperation = { epoch: 1, token: 1, key: "pane:pane-1", request: { type: "pane_rename", pane_id: "pane-1", label: "Logs" } as const, focusFromSnapshot: false };

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
    const response: ResourceMutationResponse = { session_id: "session-1", snapshot: snapshot(), created: null };
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
  it("replays only idempotent absolute mutations", () => {
    const retryable: ResourceMutationRequest[] = [
      { type: "space_rename", space_id: "space-1", label: "Main" },
      { type: "space_move_block", space_ids: ["space-1"], before_space_id: null },
      { type: "tab_rename", tab_id: "tab-1", label: "Shell" },
      { type: "tab_move", tab_id: "tab-1", insert_index: 0 },
      { type: "pane_rename", pane_id: "pane-1", label: "Logs" },
    ];
    const ambiguous: ResourceMutationRequest[] = [
      { type: "space_create", cwd: null, label: null },
      { type: "tab_create", space_id: "space-1", label: null },
      { type: "pane_split", pane_id: "pane-1", direction: "right", ratio: null },
      { type: "pane_move", pane_id: "pane-1", destination: { type: "new_space", label: null, tab_label: null } },
      { type: "pane_close", pane_id: "pane-1" },
    ];
    retryable.forEach((request) => expect(mutationFailureCanRetry(request, "transport_error")).toBe(true));
    ambiguous.forEach((request) => expect(mutationFailureCanRetry(request, "transport_error")).toBe(false));
    retryable.forEach((request) => expect(mutationFailureCanRetry(request, "mutation_applied_snapshot_failed")).toBe(false));
    expect(mutationFailureCanRetry(retryable[0], "request_outcome_unknown")).toBe(false);
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

describe("rendererReasonFor", () => {
  const chain = "pane is not a supported extension; Open Context requires a source pane at a reviewed companion checkout; Open files requires a safe source pane directory";
  it("picks the clause for the requested action", () => {
    expect(rendererReasonFor("context", chain)).toBe("Requires a source pane at a reviewed companion checkout");
    expect(rendererReasonFor("files", chain)).toBe("Requires a safe source pane directory");
  });
  it("leaves unmatched chains to the caller's fallback", () => {
    expect(rendererReasonFor("review", chain)).toBeUndefined();
  });
});
