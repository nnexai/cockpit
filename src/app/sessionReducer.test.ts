import { describe, expect, it } from "vitest";
import type { SessionSnapshotResponse, TerminalStreamMessage } from "../protocol/generated/v1";
import { initialSessionState, sessionReducer } from "./sessionReducer";

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
    expect(state.syncError?.code).toBe("stream_gap");
  });

  it("keeps the recovery error visible while a resync is loading", () => {
    let state = ready("one");
    state = sessionReducer(state, { type: "stream/error", epoch: state.epoch, sessionId: "one", code: "malformed_response", message: "Bad snapshot" });
    const loading = sessionReducer(state, { type: "snapshot/request", epoch: state.epoch, sessionId: "one" });
    expect(loading.sync).toBe("loading");
    expect(loading.syncError).toEqual(state.syncError);
  });

  it("adopts an authoritative snapshot while preserving a live stream", () => {
    const state = ready("one");
    const authoritative = snapshot("one", "pane-2");
    const adopted = sessionReducer(state, {
      type: "snapshot/received",
      epoch: state.epoch,
      sessionId: "one",
      snapshot: authoritative,
      preserveStream: true,
    });
    expect(adopted.snapshot).toBe(authoritative);
    expect(adopted.generation).toBe(state.generation);
    expect(adopted.sequence).toBe(state.sequence);
    expect(adopted.sync).toBe("live");
    expect(adopted.syncError).toBeNull();
  });

  it("adopts an authoritative snapshot without clearing stale stream state", () => {
    let state = ready("one");
    state = sessionReducer(state, {
      type: "stream/message",
      epoch: state.epoch,
      sessionId: "one",
      message: stream("one", 1, 3),
    });
    const authoritative = snapshot("one", "pane-2");
    const syncError = state.syncError;
    const adopted = sessionReducer(state, {
      type: "snapshot/received",
      epoch: state.epoch,
      sessionId: "one",
      snapshot: authoritative,
      preserveStream: true,
    });
    expect(adopted.snapshot).toBe(authoritative);
    expect(adopted.generation).toBe(state.generation);
    expect(adopted.sequence).toBe(state.sequence);
    expect(adopted.sync).toBe("stale");
    expect(adopted.syncError).toBe(syncError);
  });

  it("adopts an authoritative snapshot without clearing disconnected stream state", () => {
    let state = ready("one");
    state = sessionReducer(state, {
      type: "stream/error",
      epoch: state.epoch,
      sessionId: "one",
      code: "closed",
      message: "closed",
    });
    const authoritative = snapshot("one", "pane-2");
    const syncError = state.syncError;
    const adopted = sessionReducer(state, {
      type: "snapshot/received",
      epoch: state.epoch,
      sessionId: "one",
      snapshot: authoritative,
      preserveStream: true,
    });
    expect(adopted.snapshot).toBe(authoritative);
    expect(adopted.generation).toBe(state.generation);
    expect(adopted.sequence).toBe(state.sequence);
    expect(adopted.sync).toBe("disconnected");
    expect(adopted.syncError).toBe(syncError);
  });

  it("clears pending focus from an authoritative HTTP snapshot", () => {
    let state = ready("one");
    const request = { kind: "pane", target_id: "pane-2" } as const;
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request });
    expect(state.focusPending).toEqual(request);
    state = sessionReducer(state, {
      type: "snapshot/received",
      epoch: state.epoch,
      sessionId: "one",
      snapshot: snapshot("one", "pane-2"),
      preserveStream: true,
    });
    expect(state.focusPending).toBeNull();
    state = sessionReducer(state, { type: "focus/request", epoch: state.epoch, sessionId: "one", request });
    state = sessionReducer(state, {
      type: "snapshot/received",
      epoch: state.epoch,
      sessionId: "one",
      snapshot: snapshot("one", "pane-1"),
      preserveStream: true,
    });
    expect(state.focusPending).toEqual(request);
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
    state = sessionReducer(state, { type: "attachment/dispose", epoch: state.epoch, sessionId: "one", paneId: "pane-1" });
    const late = sessionReducer(state, { type: "attachment/message", epoch: state.epoch, sessionId: "one", paneId: "pane-1", message: { type: "ownership", session_id: "one", pane_id: "pane-1", stream_id: "stream-1", state: "owned", message: null } });
    expect(late.attachments["pane-1"].ownership).toBe("observing");
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

