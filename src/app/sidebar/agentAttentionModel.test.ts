import { expect, it } from "vitest";
import type { AgentAttentionResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import { collectObservations, deriveAgentQueue } from "./agentAttentionModel";

const snapshot = { session_id: "s", server_instance: "server", panes: [{ id: "p", terminal_id: "t" }], agents: [{ pane_id: "p", name: "agent", status: "idle", state_change_seq: 4, focused: false }] } as SessionSnapshotResponse;
const response: AgentAttentionResponse = { session_id: "s", server_instance: "server", epoch: 1, refused: null, records: [{ historical_id: null, pane_id: "p", terminal_id: "t", agent: "agent", last: { epoch: 1, seq: 4, status: "idle", observed_at: "now" }, completion: { epoch: 1, seq: 3, at: "then", inferred: true }, acked: null }] };

it("retains completion through idle and working, and fences the acknowledgement mark", () => {
  const idle = deriveAgentQueue(snapshot, response, "ready").visible[0]!;
  expect(idle.display).toBe("done");
  expect(idle.completion).toEqual({ epoch: 1, seq: 3, at: "then", inferred: true });
  const working = { ...snapshot, agents: [{ ...snapshot.agents[0]!, status: "working", state_change_seq: 5 }] };
  expect(deriveAgentQueue(working, response, "ready").visible[0]).toMatchObject({ display: "working", flag: "unseen", completion: { epoch: 1, seq: 3 } });
});

it("automatically excludes ordinary idle but keeps actual done after acknowledgement", () => {
  const record = { ...response.records[0]!, completion: null };
  expect(deriveAgentQueue(snapshot, { ...response, records: [record] }, "ready").visible).toHaveLength(0);
  expect(deriveAgentQueue(snapshot, null, "unavailable").visible).toHaveLength(0);
  const acked = { ...response.records[0]!, acked: { epoch: 1, seq: 3, at: "now" } };
  expect(deriveAgentQueue(snapshot, { ...response, records: [acked] }, "ready").visible).toHaveLength(0);
  const done = { ...snapshot, agents: [{ ...snapshot.agents[0]!, status: "done", state_change_seq: 3 }] };
  expect(deriveAgentQueue(done, { ...response, records: [acked] }, "ready").visible).toHaveLength(1);
  expect(deriveAgentQueue(done, null, "unavailable").visible).toHaveLength(1);
});

it("keeps active and unseen agents", () => {
  expect(deriveAgentQueue(snapshot, response, "ready").visible[0]).toMatchObject({ unseen: true, display: "done" });
  for (const status of ["working", "blocked"]) {
    const active = { ...snapshot, agents: [{ ...snapshot.agents[0]!, status }] };
    expect(deriveAgentQueue(active, null, "ready").visible).toHaveLength(1);
  }
});

it("diffs lifecycle observations but ignores titles", () => {
  const first = collectObservations(new Map(), snapshot);
  expect(first.observations).toHaveLength(1);
  expect(collectObservations(first.next, { ...snapshot, agents: [{ ...snapshot.agents[0]!, title: "renamed" }] }).observations).toHaveLength(0);
});
