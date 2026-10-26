import { useEffect, useMemo, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { AgentAttentionChange, AgentAttentionObservation, AgentAttentionRecord, AgentAttentionResponse } from "../../protocol/generated/v1";
import { collectObservations, deriveAgentQueue, type AttentionAvailability, type AttentionRow } from "../sidebar/agentAttentionModel";
import type { SessionState } from "./sessionStore";

export type AgentAttentionState = {
  availability: AttentionAvailability;
  response: AgentAttentionResponse | null;
  pending: ReadonlySet<string>;
  notes: ReadonlyMap<string, string>;
  announcement: string;
  recoveryIncomplete: boolean;
  acknowledgeRecoveryWarning(): void;
  activate(row: AttentionRow): void;
  seen(row: AttentionRow): Promise<boolean>;
  seenHistorical(record: AgentAttentionRecord): Promise<boolean>;
  dismiss(row: AttentionRow): Promise<boolean>;
  restore(row: AttentionRow): Promise<boolean>;
  restoreAll(): Promise<boolean>;
  retry(): void;
  clearNote(paneId: string): void;
};

type Action = { change: AgentAttentionChange; key: string; resolve: (ok: boolean) => void };
type Intent = { row: AttentionRow; token: number };

// Bound outage memory while retaining the most recent completion, inferred
// working-to-idle transition, and current state of each pane incarnation.
const MAX_QUEUED_OBSERVATIONS = 1024;
const OBSERVATION_BATCH_SIZE = 256;
function compactObservations(observations: AgentAttentionObservation[]): { observations: AgentAttentionObservation[]; overflow: boolean } {
  if (observations.length <= MAX_QUEUED_OBSERVATIONS) return { observations, overflow: false };
  type Segment = { identity: string; last: number; done?: number; beforeIdle?: number; idle?: number };
  const active = new Map<string, Segment>();
  const segments: Segment[] = [];
  observations.forEach((observation, index) => {
    const identity = JSON.stringify([observation.terminal_id, observation.agent]);
    let segment = active.get(observation.pane_id);
    if (!segment || segment.identity !== identity) {
      segment = { identity, last: index };
      active.set(observation.pane_id, segment);
      segments.push(segment);
    } else {
      const previous = observations[segment.last]!;
      if (observation.status === "idle" && observation.seq > previous.seq && (previous.status === "working" || previous.status === "blocked")) {
        segment.beforeIdle = segment.last;
        segment.idle = index;
      }
      segment.last = index;
    }
    if (observation.status === "done") segment.done = index;
  });
  // Prefer segments with completion evidence, then newer segments. Retain whole
  // segments so an inferred transition pair is never cut in half.
  const retained: number[] = [];
  let overflow = false;
  segments.sort((a, b) => Number(b.done !== undefined || b.idle !== undefined) - Number(a.done !== undefined || a.idle !== undefined) || b.last - a.last);
  for (const segment of segments) {
    const indices = [...new Set([segment.last, segment.done, segment.beforeIdle, segment.idle].filter((index): index is number => index !== undefined))];
    if (retained.length + indices.length <= MAX_QUEUED_OBSERVATIONS) retained.push(...indices);
    else overflow = true;
  }
  retained.sort((a, b) => a - b);
  return { observations: retained.map(index => observations[index]!), overflow };
}

function takeObservationBatch(observations: AgentAttentionObservation[]): AgentAttentionObservation[] {
  // Different panes are independent. Keep each pane's observations together so
  // the server can see an inference pair in the same request.
  const panes = new Map<string, AgentAttentionObservation[]>();
  for (const observation of observations) {
    const group = panes.get(observation.pane_id) ?? [];
    group.push(observation);
    panes.set(observation.pane_id, group);
  }
  observations.splice(0, observations.length, ...Array.from(panes.values()).flat());
  let count = Math.min(OBSERVATION_BATCH_SIZE, observations.length);
  const before = observations[count - 1], after = observations[count];
  if (before && after && before.pane_id === after.pane_id && before.terminal_id === after.terminal_id && before.agent === after.agent &&
      (before.status === "working" || before.status === "blocked") && after.status === "idle" && before.seq < after.seq) count--;
  return observations.splice(0, count);
}

export function historicalAttentionKey(record: AgentAttentionRecord): string {
  return `historical:${record.historical_id}`;
}

export function useAgentAttention(client: CockpitClient, state: SessionState, live: boolean): AgentAttentionState {
  const sessionId = state.sessionId;
  const snapshot = state.snapshot?.session_id === sessionId ? state.snapshot : null;
  const instance = snapshot?.server_instance ?? null;
  const [, render] = useState(0);
  const recoveryIncomplete = useRef(false);
  const binding = useMemo(() => ({
    availability: "loading" as AttentionAvailability, response: null as AgentAttentionResponse | null,
    pending: new Set<string>(), notes: new Map<string, string>(), announcement: "",
    observations: [] as AgentAttentionObservation[], previous: new Map<string, string>(), actions: [] as Action[],
    busy: false, disposed: false, generation: 0, sequence: 0, lastApplied: 0, refresh: false,
    initial: true, announced: new Set<string>(), intent: null as Intent | null,
    pump: null as (() => void) | null,
  }), [client, sessionId, instance]);
  const current = useRef({ binding, state, live, snapshot });
  current.current = { binding, state, live, snapshot };
  const update = () => { if (current.current.binding === binding && !binding.disposed) render(n => n + 1); };

  useEffect(() => {
    if (!live || !sessionId || !instance) return;
    binding.disposed = false;
    const generation = ++binding.generation;
    let controller: AbortController | null = null;
    const valid = () => !binding.disposed && binding.generation === generation && current.current.binding === binding;
    const apply = (response: AgentAttentionResponse, sequence: number, announce: boolean) => {
      if (!valid() || sequence < binding.lastApplied) return;
      if (response.server_instance !== instance) {
        binding.availability = "unavailable"; binding.response = null; update(); return;
      }
      binding.lastApplied = sequence;
      binding.response = response;
      binding.availability = "ready";
      const snap = current.current.snapshot;
      if (snap) {
        const rows = deriveAgentQueue(snap, response, "ready").visible;
        const keys = new Set(rows.filter(row => row.unseen || row.display === "blocked").map(row => `${row.key}:${row.unseen ? "done" : "blocked"}`));
        if (!binding.initial && announce) {
          const added = rows.find(row => (row.unseen || row.display === "blocked") && !binding.announced.has(`${row.key}:${row.unseen ? "done" : "blocked"}`));
          if (added) binding.announcement = `${added.agent.name}: ${added.display === "blocked" ? "blocked" : "result not seen"}`;
        }
        binding.announced = keys;
      }
      binding.initial = false;
      update();
    };
    const pump = () => {
      if (!valid() || binding.busy || !current.current.live) return;
      if (binding.refresh) {
        binding.refresh = false; binding.busy = true;
        const sequence = ++binding.sequence;
        controller = new AbortController();
        client.agentAttention(sessionId, controller.signal).then(response => apply(response, sequence, false)).catch(() => {
          if (valid()) {
            binding.availability = "unavailable"; binding.response = null;
            for (const action of binding.actions.splice(0)) {
              binding.pending.delete(action.key);
              binding.notes.set(action.key, "Agent attention unavailable");
              action.resolve(false);
            }
            update();
          }
        }).finally(() => { if (valid()) { binding.busy = false; pump(); } });
        return;
      }
      if (binding.availability !== "ready" || (!binding.observations.length && !binding.actions.length)) return;
      const observations = takeObservationBatch(binding.observations);
      const action = binding.observations.length ? undefined : binding.actions.shift();
      binding.busy = true;
      const sequence = ++binding.sequence;
      client.changeAgentAttention(sessionId, { server_instance: instance, observations, change: action?.change ?? null }).then(response => {
        if (!valid()) { action?.resolve(false); return; }
        apply(response, sequence, true);
        if (action) {
          binding.pending.delete(action.key);
          if (response.refused) binding.notes.set(action.key, response.refused.message);
          else binding.notes.delete(action.key);
          action.resolve(!response.refused);
          update();
        }
      }).catch(error => {
        if (!valid()) { action?.resolve(false); return; }
        const compacted = compactObservations([...observations, ...binding.observations]);
        binding.observations = compacted.observations;
        recoveryIncomplete.current ||= compacted.overflow;
        if (action) {
          binding.pending.delete(action.key);
          binding.notes.set(action.key, error instanceof Error ? error.message : "Agent attention change failed");
          action.resolve(false);
        }
        for (const queued of binding.actions.splice(0)) {
          binding.pending.delete(queued.key);
          binding.notes.set(queued.key, "Agent attention unavailable");
          queued.resolve(false);
        }
        binding.availability = "unavailable"; binding.response = null; update();
      }).finally(() => { if (valid()) { binding.busy = false; pump(); } });
    };
    binding.pump = pump;
    const refresh = () => { binding.refresh = true; pump(); };
    const visibility = () => { if (document.visibilityState === "visible") refresh(); };
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", visibility);
    refresh();
    return () => {
      binding.disposed = true; ++binding.generation; controller?.abort(); binding.pump = null;
      binding.busy = false; binding.intent = null;
      for (const action of binding.actions.splice(0)) action.resolve(false);
      binding.pending.clear();
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [binding, client, sessionId, instance, live]);

  useEffect(() => {
    if (!live || !snapshot || !instance) return;
    const collected = collectObservations(binding.previous, snapshot);
    binding.previous = collected.next;
    const compacted = compactObservations([...binding.observations, ...collected.observations]);
    binding.observations = compacted.observations;
    if (compacted.overflow && !recoveryIncomplete.current) {
      recoveryIncomplete.current = true;
      update();
    }
    binding.pump?.();
  }, [binding, snapshot, live, instance]);

  const change = (key: string, value: AgentAttentionChange): Promise<boolean> => {
    if (current.current.binding !== binding || binding.disposed || !live || binding.availability !== "ready" || binding.pending.has(key)) return Promise.resolve(false);
    binding.pending.add(key); binding.notes.delete(key); update();
    // The frontend targets ES2022, before Promise.withResolvers.
    let resolve!: (ok: boolean) => void;
    const promise = new Promise<boolean>(done => { resolve = done; });
    binding.actions.push({ change: value, key, resolve }); binding.pump?.();
    return promise;
  };
  const seen = (row: AttentionRow) => row.terminalId && row.completion
    ? change(row.agent.pane_id, { kind: "seen", pane_id: row.agent.pane_id, terminal_id: row.terminalId, agent: row.agent.name, transition: { epoch: row.completion.epoch, seq: row.completion.seq } })
    : Promise.resolve(false);
  useEffect(() => {
    const intent = binding.intent;
    if (!intent) return;
    if (!live || state.focusError || state.focusToken > intent.token || (state.focusToken === intent.token && state.focusPending && (state.focusPending.kind !== "agent" || state.focusPending.target_id !== intent.row.agent.pane_id))) {
      binding.intent = null; return;
    }
    if (state.focusToken === intent.token && state.focusAcceptedToken === intent.token && !state.focusPending) {
      binding.intent = null;
      if (snapshot?.focused_pane_id === intent.row.agent.pane_id) void seen(intent.row);
    }
  }, [binding, state.focusToken, state.focusAcceptedToken, state.focusPending, state.focusError, snapshot, live]);

  return {
    availability: binding.availability, response: binding.response, pending: binding.pending, notes: binding.notes, announcement: binding.announcement,
    recoveryIncomplete: recoveryIncomplete.current,
    acknowledgeRecoveryWarning: () => { recoveryIncomplete.current = false; update(); },
    activate(row) {
      if (!live || !row.canSeen) return;
      if (!state.focusPending && snapshot?.focused_pane_id === row.agent.pane_id) { void seen(row); return; }
      binding.intent = { row, token: state.focusToken + 1 };
    },
    seen,
    seenHistorical: record => record.historical_id
      ? change(historicalAttentionKey(record), { kind: "seen_historical", historical_id: record.historical_id })
      : Promise.resolve(false),
    dismiss: row => row.terminalId && row.transition ? change(row.agent.pane_id, { kind: "dismiss", pane_id: row.agent.pane_id, terminal_id: row.terminalId, agent: row.agent.name, transition: row.transition }) : Promise.resolve(false),
    restore: row => change(row.agent.pane_id, { kind: "restore", pane_id: row.agent.pane_id, agent: row.agent.name }),
    restoreAll: () => change("*", { kind: "restore_all" }),
    retry: () => { binding.refresh = true; binding.pump?.(); },
    clearNote: paneId => { binding.notes.delete(paneId); update(); },
  };
}
