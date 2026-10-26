import type { AgentAttentionObservation, AgentAttentionRecord, AgentAttentionResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import { stateClass, type Agent, type StatusClass } from "./spaceTree";

export type AttentionAvailability = "loading" | "ready" | "unavailable";
type Mark = { epoch: number; seq: number };
export type AttentionRow = { agent: Agent; key: string; terminalId: string | null; transition: Mark | null; completion: Mark | null; display: StatusClass; flag: "blocked" | "done" | "unseen" | "hidden" | null; unseen: boolean; hidden: boolean; completionAt: string | null; inferred: boolean; canDismiss: boolean; canSeen: boolean };
const compare = (a: Mark | null | undefined, b: Mark | null | undefined) => !a ? (!b ? 0 : -1) : !b ? 1 : a.epoch - b.epoch || a.seq - b.seq;

export function deriveAgentQueue(snapshot: SessionSnapshotResponse, attention: AgentAttentionResponse | null, availability: AttentionAvailability): { visible: AttentionRow[]; hidden: AttentionRow[] } {
  const usable = availability === "ready" && attention?.server_instance === snapshot.server_instance && attention.session_id === snapshot.session_id ? attention : null;
  const terminals = new Map(snapshot.panes.map(pane => [pane.id, pane.terminal_id]));
  const records = new Map((usable?.records ?? []).filter(record => record.historical_id === null).map(record => [record.pane_id, record]));
  const rows = snapshot.agents.map(agent => {
    const terminalId = terminals.get(agent.pane_id) ?? null;
    let record: AgentAttentionRecord | undefined = records.get(agent.pane_id);
    if (record && (record.agent !== agent.name || (record.last.epoch === usable?.epoch && record.terminal_id !== terminalId))) record = undefined;
    const status = stateClass(agent.status || "unknown");
    const transition = usable ? { epoch: usable.epoch, seq: agent.state_change_seq } : null;
    const completion = status === "done" && transition && compare(transition, record?.completion) > 0 ? transition : record?.completion;
    const unseen = availability !== "unavailable" && (completion ? compare(completion, record?.acked) > 0 : status === "done");
    const hidden = Boolean(transition && record?.last.epoch === transition.epoch && !unseen && (status === "idle" || status === "done") && compare(record.dismissed, transition) === 0);
    const display = availability === "unavailable" ? status : status === "blocked" || status === "working" ? status : unseen ? "done" : status === "done" ? "idle" : status;
    const flag = availability === "unavailable" ? null : status === "blocked" ? "blocked" : unseen ? status === "working" ? "unseen" : "done" : hidden ? "hidden" : null;
    const tier = status === "blocked" ? 0 : unseen ? 1 : status === "working" ? 2 : display === "idle" ? 3 : 4;
    return { agent, key: `${agent.pane_id}:${agent.name}`, terminalId, transition, completion: completion ?? null, display, flag, unseen, hidden, completionAt: record?.completion?.at ?? null, inferred: record?.completion?.inferred ?? false, canDismiss: Boolean(transition && terminalId && (display === "idle" || display === "done")), canSeen: Boolean(transition && terminalId && unseen), tier } satisfies AttentionRow & { tier: number };
  });
  rows.sort((a, b) => a.tier - b.tier || compare(b.transition, a.transition) || Number(b.agent.focused) - Number(a.agent.focused) || a.agent.pane_id.localeCompare(b.agent.pane_id));
  return { visible: rows.filter(row => !row.hidden), hidden: rows.filter(row => row.hidden) };
}

export function collectObservations(previous: ReadonlyMap<string, string>, snapshot: SessionSnapshotResponse): { next: Map<string, string>; observations: AgentAttentionObservation[] } {
  const terminals = new Map(snapshot.panes.map(pane => [pane.id, pane.terminal_id]));
  const next = new Map<string, string>();
  const observations: AgentAttentionObservation[] = [];
  for (const agent of snapshot.agents) {
    const terminal_id = terminals.get(agent.pane_id);
    if (!terminal_id) continue;
    const key = JSON.stringify([terminal_id, agent.name, agent.status, agent.state_change_seq]);
    next.set(agent.pane_id, key);
    if (previous.get(agent.pane_id) !== key) observations.push({ pane_id: agent.pane_id, terminal_id, agent: agent.name, status: agent.status, seq: agent.state_change_seq });
  }
  return { next, observations };
}
