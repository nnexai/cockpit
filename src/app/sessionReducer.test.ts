import { describe, expect, it } from "vitest";
import type { SessionSnapshotResponse, TerminalStreamMessage } from "../protocol/generated/v1";
import { initialSessionState, sessionReducer } from "./session/sessionStore";

function snapshot(sessionId: string, pane = "pane-1", focused = pane): SessionSnapshotResponse {
  return {
    session_id: sessionId,
    version: "0.8.2",
    protocol: 20,
    focused_space_id: "space-1",
    focused_tab_id: "tab-1",
    focused_pane_id: focused,
    spaces: [{ id: "space-1", label: "Space", number: 1, tab_count: 1, pane_count: 1, focused: true, agent_status: "idle", git: null }],
    tabs: [{ id: "tab-1", space_id: "space-1", label: "Tab", number: 1, pane_count: 1, focused: true }],
    panes: [{ id: pane, terminal_id: "term-1", space_id: "space-1", tab_id: "tab-1", title: null, focused: true, agent: null, agent_status: "idle", revision: 1 }],
    layouts: [],
    agents: [],
  };
}

function stream(sessionId: string, generation: number, sequence: number, value = snapshot(sessionId)) {
  return { type: "snapshot", session_id: sessionId, generation, sequence, snapshot: value } as const;
}
function terminalFrame(seq: string, full = true): TerminalStreamMessage {
  return {
    type: "frame",
    session_id: "one",
    pane_id: "pane-1",
    stream_id: "stream-1",
    seq,
    encoding: "ansi",
    width: 80,
    height: 24,
    full,
    bytes: "",
  };
}

function ready(sessionId: string) {
  let state = sessionReducer(initialSessionState, { type: "switch", sessionId });
  state = sessionReducer(state, { type: "snapshot/received", epoch: state.epoch, sessionId, snapshot: snapshot(sessionId) });
  return sessionReducer(state, { type: "stream/message", epoch: state.epoch, sessionId, message: stream(sessionId, 1, 1) });
}

describe("sessionReducer", () => {
  it("switches transactionally and rejects a late prior epoch", () => {
    const first = ready("one");
    const second = sessionReducer(first, { type: "switch", sessionId: "two" });
    const late = sessionReducer(second, { type: "stream/message", epoch: first.epoch, sessionId: "one", message: stream("one", 1, 2) });
    expect(second.snapshot).toBeNull();
    expect(late).toBe(second);
  });

  it("rejects old generations and marks sequence gaps stale", () => {
    let state = ready("one");
    const old = sessionReducer(state, { type: "stream/message", epoch: state.epoch, sessionId: "one", message: stream("one", 0, 2) });
    expect(old).toBe(state);
    state = sessionReducer(state, { type: "stream/message", epoch: state.epoch, sessionId: "one", message: stream("one", 1, 3) });
    expect(state.sync).toBe("stale");
    expect(state.syncError?.code).toBe("stream_sequence");
  });
  it("marks invalid future generation transitions stale instead of staying live", () => {
    for (const [generation, sequence, code] of [[2, 5, "stream_sequence"], [3, 1, "stream_generation"]] as const) {
      const state = ready("one");
      const rejected = sessionReducer(state, {
        type: "stream/message",
        epoch: state.epoch,
        sessionId: "one",
        message: stream("one", generation, sequence),
      });
      expect(rejected.sync).toBe("stale");
      expect(rejected.syncError?.code).toBe(code);
      expect(rejected.generation).toBe(1);
      expect(rejected.sequence).toBe(1);
    }
  });

  it("keeps the recovery error visible while a resync is loading", () => {
    let state = ready("one");
    state = sessionReducer(state, { type: "stream/error", epoch: state.epoch, sessionId: "one", code: "malformed_response", message: "Bad snapshot" });
    const loading = sessionReducer(state, { type: "snapshot/request", epoch: state.epoch, sessionId: "one" });
    expect(loading.sync).toBe("loading");
    expect(loading.syncError).toEqual(state.syncError);
  });


  it("does not adopt an unsequenced snapshot over a live stream", () => {
    const state = ready("one");
    const late = sessionReducer(state, {
      type: "snapshot/received",
      epoch: state.epoch,
      sessionId: "one",
      snapshot: snapshot("one", "pane-2"),
    });
    expect(late).toBe(state);
    expect(late.snapshot?.focused_pane_id).toBe("pane-1");
    expect(late.generation).toBe(1);
    expect(late.sequence).toBe(1);
    expect(late.sync).toBe("live");
  });

  it("loads a fresh snapshot and accepts its ordered first stream frame", () => {
    let state = ready("one");
    state = sessionReducer(state, { type: "stream/error", epoch: state.epoch, sessionId: "one", code: "stream_sequence", message: "gap" });
    state = sessionReducer(state, { type: "snapshot/request", epoch: state.epoch, sessionId: "one" });
    state = sessionReducer(state, { type: "snapshot/received", epoch: state.epoch, sessionId: "one", snapshot: snapshot("one", "pane-2") });
    expect(state.sync).toBe("loading");
    state = sessionReducer(state, { type: "stream/message", epoch: state.epoch, sessionId: "one", message: stream("one", 2, 1, snapshot("one", "pane-2")) });
    expect(state.sync).toBe("live");
    expect(state.generation).toBe(2);
    expect(state.sequence).toBe(1);
  });
  it("clears pending focus only after an ordered stream snapshot", () => {
    let state = ready("one");
    const request = { kind: "pane", target_id: "pane-2" } as const;
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request });
    expect(state.focusPending).toEqual(request);
    state = sessionReducer(state, {
      type: "stream/message",
      epoch: state.epoch,
      sessionId: "one",
      message: stream("one", 1, 2, snapshot("one", "pane-2")),
    });
    expect(state.focusPending).toBeNull();
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request });
    state = sessionReducer(state, {
      type: "stream/message",
      epoch: state.epoch,
      sessionId: "one",
      message: stream("one", 1, 3, snapshot("one", "pane-1")),
    });
    expect(state.focusPending).toEqual(request);
  });

  it("keeps the newest focus intent through a delayed older confirmation", () => {
    let state = ready("one");
    const first = { kind: "tab", target_id: "tab-2" } as const;
    const newest = { kind: "tab", target_id: "tab-3" } as const;
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request: first, token: 1 });
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request: newest, token: 2 });
    const olderConfirmation = sessionReducer(state, {
      type: "stream/message",
      epoch: state.epoch,
      sessionId: "one",
      message: stream("one", 1, 2, snapshot("one", "pane-1")),
    });
    expect(olderConfirmation.focusPending).toEqual(newest);
    const newestConfirmation = sessionReducer(olderConfirmation, {
      type: "stream/message",
      epoch: state.epoch,
      sessionId: "one",
      message: stream("one", 1, 3, { ...snapshot("one", "pane-3"), focused_tab_id: "tab-3" }),
    });
    expect(newestConfirmation.focusPending).toBeNull();
  });

  it("clears a stale focus error after the retried focus is authoritatively confirmed", () => {
    let state = ready("one");
    const request = { kind: "pane", target_id: "pane-2" } as const;
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request, token: 1 });
    state = sessionReducer(state, { type: "focus/error", epoch: state.epoch, sessionId: "one", token: 1, code: "focus_error", message: "failed" });
    expect(state.focusError?.code).toBe("focus_error");
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request, token: 2 });
    state = sessionReducer(state, { type: "stream/message", epoch: state.epoch, sessionId: "one", message: stream("one", 1, 2, snapshot("one", "pane-2")) });
    expect(state.focusPending).toBeNull();
    expect(state.focusError).toBeNull();
  });
  it("rejects duplicate and skipped terminal full frames", () => {
    for (const invalidSequence of ["1", "3"]) {
      let state = ready("one");
      state = sessionReducer(state, {
        type: "attachment/opened",
        epoch: state.epoch,
        sessionId: "one",
        paneId: "pane-1",
        streamId: "stream-1",
        mode: "observe",
      });
      state = sessionReducer(state, {
        type: "attachment/message",
        epoch: state.epoch,
        sessionId: "one",
        paneId: "pane-1",
        message: terminalFrame("1"),
      });
      const rejected = sessionReducer(state, {
        type: "attachment/message",
        epoch: state.epoch,
        sessionId: "one",
        paneId: "pane-1",
        message: terminalFrame(invalidSequence),
      });
      expect(rejected.attachments["pane-1"].terminalSequence).toBe(1n);
      expect(rejected.attachments["pane-1"].error).toEqual({
        code: "terminal_sequence",
        message: "Terminal output sequence is not consecutive",
      });
    }
  });


  it("tracks disconnect and ownership, and ignores disposed late attachment messages", () => {
    let state = ready("one");
    state = sessionReducer(state, { type: "stream/error", epoch: state.epoch, sessionId: "one", code: "closed", message: "closed" });
    expect(state.sync).toBe("disconnected");
    state = sessionReducer(state, { type: "attachment/opened", epoch: state.epoch, sessionId: "one", paneId: "pane-1", streamId: "stream-1", mode: "observe" });
    state = sessionReducer(state, { type: "attachment/message", epoch: state.epoch, sessionId: "one", paneId: "pane-1", message: { type: "ownership", session_id: "one", pane_id: "pane-1", stream_id: "stream-1", state: "observing", message: null } });
    expect(state.attachments["pane-1"].ownership).toBe("observing");
    state = sessionReducer(state, { type: "attachment/dispose", epoch: state.epoch, sessionId: "one", paneId: "pane-1", streamId: "stream-1" });
    const late = sessionReducer(state, { type: "attachment/message", epoch: state.epoch, sessionId: "one", paneId: "pane-1", message: { type: "ownership", session_id: "one", pane_id: "pane-1", stream_id: "stream-1", state: "owned", message: null } });
    expect(late.attachments["pane-1"].ownership).toBe("observing");
  });

  it("keeps a replacement attachment when delayed errors or disposal name the retired stream", () => {
    let state = ready("one");
    state = sessionReducer(state, { type: "attachment/opened", epoch: state.epoch, sessionId: "one", paneId: "pane-1", streamId: "stream-a", mode: "observe" });
    state = sessionReducer(state, { type: "attachment/opened", epoch: state.epoch, sessionId: "one", paneId: "pane-1", streamId: "stream-b", mode: "control" });
    const delayedError = sessionReducer(state, { type: "attachment/error", epoch: state.epoch, sessionId: "one", paneId: "pane-1", streamId: "stream-a", code: "terminal_disconnected", message: "old" });
    const delayedDispose = sessionReducer(delayedError, { type: "attachment/dispose", epoch: state.epoch, sessionId: "one", paneId: "pane-1", streamId: "stream-a" });
    expect(delayedDispose.attachments["pane-1"]).toMatchObject({ streamId: "stream-b", disposed: false, error: null });
  });

  it("records terminal.closed without removing the authoritative pane or disconnecting the session", () => {
    let state = ready("one");
    const authoritative = state.snapshot;
    state = sessionReducer(state, { type: "attachment/opened", epoch: state.epoch, sessionId: "one", paneId: "pane-1", streamId: "stream-1", mode: "control" });
    state = sessionReducer(state, { type: "attachment/message", epoch: state.epoch, sessionId: "one", paneId: "pane-1", message: { type: "closed", session_id: "one", pane_id: "pane-1", stream_id: "stream-1", reason: "process exited" } });
    expect(state.snapshot).toBe(authoritative);
    expect(state.snapshot?.panes.some((pane) => pane.id === "pane-1")).toBe(true);
    expect(state.sync).toBe("live");
    expect(state.attachments["pane-1"].error).toEqual({ code: "terminal_closed", message: "process exited" });
  });
});
