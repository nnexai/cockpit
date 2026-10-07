import { useRef, useState } from "react";
import type { TaskLane } from "../../protocol/generated/v1";
import type { ScrollOffset } from "./reveal";

export type SupervisorViewDrafts = {
  mode: "tasks" | "graph";
  offsets: { tasks: ScrollOffset | null; graph: ScrollOffset | null };
  laneScroll: Partial<Record<TaskLane, number>>;
  collapsedLanes: TaskLane[];
  detailWidth: number | null;
  sheetHeight: number | null;
  queueOpenRow: string | null;
};

export type TextDraft = { text: string; operation: { id: string; text: string } | null; notice: string | null; error: string | null };
export type EditDraft = { title: string; body: string; revision: string };
export type ScopeDrafts = {
  messages: Map<string, TextDraft>;
  edits: Map<string, EditDraft>;
  selectedTask: string | null;
  selectedRun: string | null;
  selectedSubagent: string | null;
  showSubagents: boolean;
  attentionOnly: boolean;
  spaceFilter: string;
  disclosures: Record<"completed" | "history" | "diagnostics" | "archive", boolean>;
  view: SupervisorViewDrafts;
};
export function newTextDraft(): TextDraft { return { text: "", operation: null, notice: null, error: null }; }
export function newScopeDrafts(): ScopeDrafts {
  return { messages: new Map(), edits: new Map(), selectedTask: null, selectedRun: null, selectedSubagent: null, showSubagents: true, attentionOnly: false, spaceFilter: "", disclosures: { completed: false, history: false, diagnostics: false, archive: false }, view: { mode: "tasks", offsets: { tasks: null, graph: null }, laneScroll: {}, collapsedLanes: [], detailWidth: null, sheetHeight: null, queueOpenRow: null } };
}
export function messageDraft(scope: ScopeDrafts, key: string): TextDraft {
  let draft = scope.messages.get(key);
  if (!draft) { draft = newTextDraft(); scope.messages.set(key, draft); }
  return draft;
}
/** Memory belongs to the mounted workarea, not its current root, selected row or snapshot. */
export function useSupervisorDrafts(sessionId: string) {
  const registry = useRef({ sessionId, scopes: new Map<string, ScopeDrafts>() });
  const [, render] = useState(0);
  if (registry.current.sessionId !== sessionId) registry.current = { sessionId, scopes: new Map() };
  return {
    scope: (rootId: string | null) => {
      const key = rootId ?? "unselected";
      let scope = registry.current.scopes.get(key);
      if (!scope) { scope = newScopeDrafts(); registry.current.scopes.set(key, scope); }
      return scope;
    },
    changed: () => render(value => value + 1),
  };
}
