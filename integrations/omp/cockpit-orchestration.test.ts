import { describe, expect, it } from "vitest";
import { decodeControl, emptyWakeState, lifecycleStatus, mayAcknowledge, observeWake, parseWakeSummary, prepareToolAllowed, recoverWake, reportIdentity } from "./cockpit-orchestration";

describe("durable inbox wake bookkeeping", () => {
  it("coalesces arrivals without implying a read or acknowledgement", () => {
    const first = observeWake(emptyWakeState(), 7, 2);
    const queued = { ...first, queued: true, queuedThrough: 7, pendingThrough: 0, pendingCount: 0 };
    const second = observeWake(queued, 9, 2);
    expect(second).toMatchObject({ queued: true, queuedThrough: 7, pendingThrough: 9, pendingCount: 2, readThrough: 0, ackedThrough: 0 });
    expect(observeWake(second, 9, 2)).toBe(second);
    expect(mayAcknowledge(second, 9)).toBe(false);
  });

  it("uses numeric kind counts, not the CLI pending boolean, and rejects malformed numeric boundaries", () => {
    const envelope = { pending: true, through_seq: 7, counts: [{ kind: "report", count: 2 }, { kind: "instruction", count: 1 }] };
    const summary = parseWakeSummary(envelope);
    expect(observeWake(emptyWakeState(), summary.through, summary.count).pendingCount).toBe(3);
    for (const invalid of [
      { ...envelope, pending: 1 },
      { ...envelope, through_seq: Number.MAX_SAFE_INTEGER + 1 },
      { ...envelope, counts: [{ kind: "report", count: true }] },
      { ...envelope, counts: [{ kind: "report", count: -1 }] },
      { ...envelope, counts: [{ kind: "report", count: Number.MAX_SAFE_INTEGER }, { kind: "instruction", count: 1 }] },
      { ...envelope, pending: false },
    ]) expect(() => parseWakeSummary(invalid)).toThrow();
    expect(parseWakeSummary({ pending: false, through_seq: 7, counts: [] })).toEqual({ through: 7, count: 0 });
  });

  it("re-notifies durable unacked mail after every crash point, independent of queued markers", () => {
    for (const queued of [false, true]) {
      const restored = recoverWake([{ type: "custom", customType: "cockpit-orchestration-wake-v1", data: { ...emptyWakeState(), seen: 9, queued, queuedThrough: 9, readThrough: 7, ackedThrough: 4 } }]);
      expect(restored).toMatchObject({ seen: 0, queued: false, readThrough: 7, ackedThrough: 4 });
      expect(observeWake(restored, 9, 3).pendingThrough).toBe(9);
    }
  });

  it("does not treat read or wake as processed and forbids acknowledging unseen sequences", () => {
    const state = { ...emptyWakeState(), seen: 20, queuedThrough: 20, readThrough: 7 };
    expect(mayAcknowledge(state, 7)).toBe(true);
    expect(mayAcknowledge(state, 8)).toBe(false);
    expect(mayAcknowledge(state, 0)).toBe(false);
    expect(mayAcknowledge(state, 3.5)).toBe(false);
  });

  it("ignores unrelated/custom malformed entries", () => {
    expect(recoverWake([{ type: "custom", customType: "another-extension", data: { readThrough: 100, ackedThrough: 100 } }, { type: "custom", customType: "cockpit-orchestration-wake-v1", data: { readThrough: "7", ackedThrough: 4 } }])).toEqual(emptyWakeState());
  });
});

describe("native identity and bounded preparation", () => {
  it("distinguishes subagents by kind, never by depth or inherited run env", () => {
    expect(reportIdentity({ kind: "sub", id: "clone-at-depth-zero" }, "progress")).toEqual(["--agent-kind", "subagent", "--subagent-id", "clone-at-depth-zero"]);
    expect(() => reportIdentity({ kind: "sub", id: "worker" }, "ready")).toThrow("bound main");
    expect(() => reportIdentity({ kind: "sub", id: "worker" }, "result")).toThrow("bound main");
    expect(reportIdentity({ kind: "main", id: "Main" }, "result")).toEqual(["--agent-kind", "main"]);
  });

  it("fails closed for mutation, shell execution, nested dispatch and unknown tools", () => {
    for (const name of ["write", "edit", "bash", "eval", "task", "cockpit_delegate", "cockpit_task", "mcp__provider__write", "toString"]) expect(prepareToolAllowed(name)).toBe(false);
    for (const name of ["read", "find", "grep", "glob", "cockpit_report"]) expect(prepareToolAllowed(name)).toBe(true);
    expect(prepareToolAllowed("cockpit_inbox", "list")).toBe(true);
    expect(prepareToolAllowed("cockpit_inbox", "ack")).toBe(true);
    expect(prepareToolAllowed("cockpit_inbox", "send")).toBe(false);
    expect(prepareToolAllowed("cockpit_message", "show")).toBe(true);
    expect(prepareToolAllowed("cockpit_task", "list")).toBe(true);
    for (const operation of ["message", "annotate", "list", undefined]) expect(prepareToolAllowed("cockpit_message", operation)).toBe(false);
    for (const operation of ["create", "update", "show", undefined]) expect(prepareToolAllowed("cockpit_task", operation)).toBe(false);
  });
});

describe("exact-subagent controls and terminal telemetry", () => {
  it("decodes only the addressed agent and preserves actual send content", () => {
    expect(decodeControl(JSON.stringify({ subagent_id: "nested-2", op: { op: "send", text: "Use the new brief." } }), "nested-2")).toEqual({ op: "send", text: "Use the new brief." });
    expect(decodeControl(JSON.stringify({ subagent_id: "nested-2", op: { op: "cancel" } }), "nested-2")).toEqual({ op: "cancel" });
    expect(() => decodeControl(JSON.stringify({ subagent_id: "sibling", op: { op: "cancel" } }), "nested-2")).toThrow("another subagent");
    expect(() => decodeControl(JSON.stringify({ subagent_id: "nested-2", op: { op: "send", text: " " } }), "nested-2")).toThrow("Invalid");
  });

  it("keeps real cancellation separate from successful termination", () => {
    expect(lifecycleStatus([{ role: "assistant", stopReason: "stop" }], true)).toBe("cancelled");
    expect(lifecycleStatus([{ role: "assistant", stopReason: "aborted" }], false)).toBe("cancelled");
    expect(lifecycleStatus([{ role: "assistant", stopReason: "error" }], false)).toBe("failed");
    expect(lifecycleStatus([{ role: "assistant", stopReason: "error" }, { role: "assistant", stopReason: "stop" }], false)).toBe("done");
    expect(lifecycleStatus([], false)).toBe("failed");
  });
});
