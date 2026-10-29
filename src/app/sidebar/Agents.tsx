import { useLayoutEffect, useRef, useState } from "react";
import type { SessionSnapshotResponse } from "../../protocol/generated/v1";
import { returnFocusFromSidebar } from "../input/shortcuts";
import type { AgentAttentionState } from "../session/agentAttention";
import { deriveAgentQueue, type AgentsView, type AttentionRow } from "./agentAttentionModel";
import { StateGlyph } from "./StateGlyph";
import { SidebarSkeleton } from "./SidebarSkeleton";
import { spaceStatus, type Space } from "./spaceTree";
import { useRovingList } from "./useRovingList";

type Tab = SessionSnapshotResponse["tabs"][number];

export function Agents({ snapshot, attention, live, spaces, tabs, selectedPaneId, pendingPaneId, loading, hasSession, onSelect, focusRef, view, onViewChange }: {
  snapshot: SessionSnapshotResponse | null; attention: AgentAttentionState; live: boolean; spaces: Space[]; tabs: Tab[];
  selectedPaneId: string | null; pendingPaneId: string | null; loading: boolean; hasSession: boolean;
  onSelect: (row: AttentionRow) => void; focusRef: { current: (() => void) | null };
  view: AgentsView; onViewChange: (view: AgentsView) => void;
}) {
  const queue = snapshot ? deriveAgentQueue(snapshot, attention.response, attention.availability, view === "all") : { visible: [], hiddenIdle: 0 };
  const rows = queue.visible;
  const [viewAnnouncement, setViewAnnouncement] = useState("");
  const pointer = useRef<string | null>(null);
  const focusedRow = useRef<{ key: string; index: number } | null>(null);
  const rowIds = rows.map(row => row.key);
  const roving = useRovingList({ rowIds, selectedId: rows.find(row => row.agent.pane_id === selectedPaneId && row.agent.pane_id !== pendingPaneId)?.key ?? null,
    onKey: (event, id) => {
      const row = rows.find(item => item.key === id);
      if (!row) return false;
      if (event.key === "Enter" && event.shiftKey) { if (live && row.canSeen) void attention.seen(row); return true; }
      return false;
    },
    onEscape: () => { const active = document.activeElement as HTMLElement | null; const row = rows.find(item => item.key === active?.dataset.rowId); if (!row || !attention.notes.has(row.agent.pane_id)) return false; attention.clearNote(row.agent.pane_id); return true; },
  });
  useLayoutEffect(() => {
    const previous = focusedRow.current;
    if (!previous) return;
    const index = rowIds.indexOf(previous.key);
    if (index !== -1) {
      previous.index = index;
      return;
    }
    focusedRow.current = null;
    if (document.activeElement !== document.body) return;
    const next = rowIds[Math.min(previous.index, rowIds.length - 1)];
    if (next !== undefined && roving.listRef.current?.querySelectorAll("[data-row-id]").length) roving.focusRow(next);
    else returnFocusFromSidebar();
  });
  focusRef.current = roving.focusTarget;
  const renderRow = (row: AttentionRow) => {
    const agent = row.agent;
    const spaceLabel = spaces.find(space => space.id === agent.space_id)?.label;
    const tabLabel = tabs.find(tab => tab.id === agent.tab_id)?.label;
    const location = [spaceLabel, tabLabel].filter(Boolean).join(" · ");
    const status = spaceStatus(row.display);
    const pending = agent.pane_id === pendingPaneId;
    const selected = agent.pane_id === selectedPaneId && !pending;
    const flag = row.flag === "unseen" ? "Unseen" : row.flag === "done" ? "Done" : row.flag === "blocked" ? "Blocked" : null;
    const purpose = agent.title?.trim();
    const purposeLine = purpose && ![agent.name, location, spaceLabel, tabLabel].some(value => value?.trim().toLocaleLowerCase() === purpose.toLocaleLowerCase()) ? purpose : null;
    const description = [location, agent.name, status.word, purposeLine, row.unseen ? row.display === "working" ? "earlier result not seen" : "not seen" : null].filter(Boolean).join(", ");
    const title = [description, row.unseen && row.display === "done" && agent.status !== "done" ? "Completion retained until seen" : null, row.completionAt ? `Completed ${row.completionAt}${row.inferred ? " (inferred)" : ""}` : null].filter(Boolean).join(" · ");
    const note = attention.notes.get(agent.pane_id);
    return <div className="agent-row-group" key={row.key}>
      <button type="button" data-row-id={row.key} tabIndex={roving.tabIndexFor(row.key)} className={`agent-row state-${status.className}${selected ? " is-selected" : ""}${pending ? " is-pending" : ""}`} onPointerDown={() => { pointer.current = row.key; }} onClick={event => { if (event.detail !== 0 && pointer.current !== row.key) return; pointer.current = null; onSelect(row); }} title={title} aria-label={description} aria-current={selected ? "true" : undefined} aria-busy={pending || undefined}>
        <StateGlyph shape={status.shape} /><span className="agent-details"><span className="agent-line">{location ? <span className="agent-location">{location}</span> : <span className="agent-sub">{status.word} · {agent.name}</span>}{flag && !selected ? <span className="agent-flag">{flag}</span> : null}</span>{location && selected ? <span className="agent-sub">{status.word} · {agent.name}{flag ? ` · ${flag}` : ""}</span> : null}{purposeLine ? <span className="agent-purpose">{purposeLine}</span> : null}</span>
      </button>
      {note ? <p className="agent-row-note" role="status">{note}</p> : null}
    </div>;
  };
  const waiting = loading || (hasSession && snapshot !== null && (attention.availability === "loading" || (attention.availability === "ready" && attention.response?.server_instance !== snapshot.server_instance)));
  return <section className="sidebar-section agents-section" aria-labelledby="agents-heading">
    <div className="sidebar-section-heading"><h2 id="agents-heading">Agents</h2><span className="section-count">{rows.length}</span>
      <button type="button" className="agents-view-toggle" aria-pressed={view === "all"} disabled={!hasSession}
        aria-label={view === "attention" && queue.hiddenIdle ? `Show all agents, ${queue.hiddenIdle} idle hidden` : "Show all agents"}
        title="Show all agents, including idle" onClick={event => {
          const next = view === "all" ? "attention" : "all";
          const count = next === "all" ? rows.length + queue.hiddenIdle : rows.length - queue.hiddenIdle;
          onViewChange(next);
          setViewAnnouncement(next === "all" ? `Showing all ${count} agents` : `Showing ${count} agents that need attention`);
          event.currentTarget.focus({ preventScroll: true });
        }}>All{view === "attention" && queue.hiddenIdle > 0 ? <span aria-hidden="true"> +{queue.hiddenIdle}</span> : null}</button>
    </div>
    {attention.availability === "unavailable" && hasSession ? <p className="agent-notice">Attention unavailable. <button type="button" onClick={attention.retry}>Retry</button></p> : null}
    <div className="agent-list" role="group" aria-label="Agents" ref={roving.listRef} {...roving.listProps}
      onFocus={event => {
        roving.listProps.onFocus(event);
        const key = (event.target as HTMLElement).dataset.rowId;
        const index = key === undefined ? -1 : rowIds.indexOf(key);
        focusedRow.current = key !== undefined && index !== -1 ? { key, index } : null;
      }}
      onBlur={event => {
        roving.listProps.onBlur(event);
        if (!event.currentTarget.contains(event.relatedTarget as Node | null) && (event.relatedTarget || (event.target as HTMLElement).isConnected)) focusedRow.current = null;
      }}>
      {waiting ? <SidebarSkeleton rows={2} /> : !hasSession ? <p className="empty-row">No session selected</p> : rows.length === 0 ? <p className="empty-row">{view === "all" ? "No agents in this session" : "No agents need attention"}</p> : rows.map(renderRow)}
    </div>
    <span className="sr-only" role="status" aria-live="polite">{attention.announcement}</span>
    <span className="sr-only" role="status" aria-live="polite">{viewAnnouncement}</span>
  </section>;
}
