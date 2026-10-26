import { useLayoutEffect, useRef, useState } from "react";
import type { SessionSnapshotResponse } from "../../protocol/generated/v1";
import { historicalAttentionKey, type AgentAttentionState } from "../session/agentAttention";
import { deriveAgentQueue, type AttentionRow } from "./agentAttentionModel";
import { StateGlyph } from "./StateGlyph";
import { SidebarSkeleton } from "./SidebarSkeleton";
import { spaceStatus, type Space } from "./spaceTree";
import { useRovingList } from "./useRovingList";

type Tab = SessionSnapshotResponse["tabs"][number];
const VIEW_KEY = "cockpit.sidebar.agentsView.v1";

export function Agents({ snapshot, attention, live, spaces, tabs, selectedPaneId, pendingPaneId, loading, hasSession, onSelect, focusRef }: {
  snapshot: SessionSnapshotResponse | null; attention: AgentAttentionState; live: boolean; spaces: Space[]; tabs: Tab[];
  selectedPaneId: string | null; pendingPaneId: string | null; loading: boolean; hasSession: boolean;
  onSelect: (row: AttentionRow) => void; focusRef: { current: (() => void) | null };
}) {
  const [view, setView] = useState<"queue" | "all">(() => { try { return localStorage.getItem(VIEW_KEY) === "all" ? "all" : "queue"; } catch { return "queue"; } });
  const chooseView = (next: "queue" | "all") => { setView(next); try { localStorage.setItem(VIEW_KEY, next); } catch { /* Storage is optional. */ } };
  const queue = snapshot ? deriveAgentQueue(snapshot, attention.response, attention.availability) : { visible: [], hidden: [] };
  const historical = attention.availability === "ready" && snapshot && attention.response && attention.response.server_instance === snapshot.server_instance && attention.response.session_id === snapshot.session_id ? attention.response.records.filter(record => record.historical_id !== null && record.completion) : [];
  const rows = view === "all" ? [...queue.visible, ...queue.hidden] : queue.visible;
  const pointer = useRef<string | null>(null);
  const handoff = useRef<{ key: string; index: number } | null>(null);
  const rowIds = rows.map(row => row.key);
  const roving = useRovingList({ rowIds, selectedId: rows.find(row => row.agent.pane_id === selectedPaneId && row.agent.pane_id !== pendingPaneId)?.key ?? null,
    onKey: (event, id) => {
      const row = rows.find(item => item.key === id);
      if (!row) return false;
      if (event.key === "Delete" || event.key === "Backspace") { if (live && (row.hidden || row.canDismiss)) { handoff.current = { key: id, index: rowIds.indexOf(id) }; void (row.hidden ? attention.restore(row) : attention.dismiss(row)).then(ok => { if (!ok && handoff.current?.key === id) handoff.current = null; }); } return true; }
      if (event.key === "Enter" && event.shiftKey) { if (live && row.canSeen) void attention.seen(row); return true; }
      return false;
    },
    onEscape: () => { const active = document.activeElement as HTMLElement | null; const row = rows.find(item => item.key === active?.dataset.rowId); if (!row || !attention.notes.has(row.agent.pane_id)) return false; attention.clearNote(row.agent.pane_id); return true; },
  });
  focusRef.current = roving.focusTarget;
  useLayoutEffect(() => {
    const pending = handoff.current;
    if (pending && !rowIds.includes(pending.key)) { handoff.current = null; const next = rowIds[Math.min(pending.index, rowIds.length - 1)]; if (next) roving.focusRow(next); else roving.listRef.current?.closest("section")?.querySelector<HTMLButtonElement>(".agent-hidden-footer button, .agent-view-switch button")?.focus(); }
  });
  const renderRow = (row: AttentionRow) => {
    const agent = row.agent;
    const spaceLabel = spaces.find(space => space.id === agent.space_id)?.label;
    const tabLabel = tabs.find(tab => tab.id === agent.tab_id)?.label;
    const location = [spaceLabel, tabLabel].filter(Boolean).join(" · ");
    const status = spaceStatus(row.display);
    const pending = agent.pane_id === pendingPaneId;
    const selected = agent.pane_id === selectedPaneId && !pending;
    const flag = row.flag === "unseen" ? "Earlier result not seen" : row.flag === "hidden" ? "Hidden" : row.flag === "done" ? "Done" : row.flag === "blocked" ? "Blocked" : null;
    const description = [location, agent.name, status.word, row.unseen ? row.display === "working" ? "earlier result not seen" : "not seen" : null, row.hidden ? "hidden" : null].filter(Boolean).join(", ");
    const title = [description, row.unseen && row.display === "done" && agent.status !== "done" ? "Completion retained until seen" : null, row.completionAt ? `Completed ${row.completionAt}${row.inferred ? " (inferred)" : ""}` : null].filter(Boolean).join(" · ");
    const disabled = !live || attention.pending.has(agent.pane_id);
    const note = attention.notes.get(agent.pane_id);
    return <div className="agent-row-group" key={row.key}>
      <button type="button" data-row-id={row.key} tabIndex={roving.tabIndexFor(row.key)} className={`agent-row state-${status.className}${selected ? " is-selected" : ""}${pending ? " is-pending" : ""}`} onPointerDown={() => { pointer.current = row.key; }} onClick={event => { if (event.detail !== 0 && pointer.current !== row.key) return; pointer.current = null; onSelect(row); }} title={title} aria-label={description} aria-current={selected ? "true" : undefined} aria-busy={pending || undefined}>
        <StateGlyph shape={status.shape} /><span className="agent-details"><span className="agent-line">{location ? <span className="agent-location">{location}</span> : <span className="agent-sub">{status.word} · {agent.name}</span>}{flag && !selected ? <span className="agent-flag">{flag}</span> : null}</span>{location && selected ? <span className="agent-sub">{status.word} · {agent.name}{flag ? ` · ${flag}` : ""}</span> : null}</span>
      </button>
      {(row.hidden || row.canDismiss) && attention.availability === "ready" ? <button type="button" tabIndex={-1} className="agent-row-action" aria-label={`${row.hidden ? "Restore" : "Dismiss"} ${agent.name}`} aria-disabled={disabled || undefined} title={!live ? "Herdr is not live" : undefined} onClick={() => { if (disabled) return; handoff.current = { key: row.key, index: rowIds.indexOf(row.key) }; void (row.hidden ? attention.restore(row) : attention.dismiss(row)).then(ok => { if (!ok && handoff.current?.key === row.key) handoff.current = null; }); }}>{row.hidden ? "Restore" : "Dismiss"}</button> : null}
      {note ? <p className="agent-row-note" role="status">{note}</p> : null}
    </div>;
  };
  const waiting = loading || (hasSession && snapshot !== null && (attention.availability === "loading" || (attention.availability === "ready" && attention.response?.server_instance !== snapshot.server_instance)));
  return <section className="sidebar-section agents-section" aria-labelledby="agents-heading">
    <div className="sidebar-section-heading"><h2 id="agents-heading">Agents</h2><span className="section-count">{queue.visible.length}</span><div className="agent-view-switch" aria-label="Agents view"><button type="button" aria-pressed={view === "queue"} onClick={() => chooseView("queue")}>Queue</button><button type="button" aria-pressed={view === "all"} onClick={() => chooseView("all")}>All</button></div></div>
    {attention.availability === "unavailable" && hasSession ? <p className="agent-notice">Attention unavailable. <button type="button" onClick={attention.retry}>Retry</button></p> : null}
    {attention.recoveryIncomplete ? <p className="agent-notice" role="status">Some agent activity could not be retained while attention was unavailable. Results may be missing. <button type="button" onClick={attention.acknowledgeRecoveryWarning}>Acknowledge</button></p> : null}
    <div className="agent-list" role="group" aria-label="Agents" ref={roving.listRef} {...roving.listProps}>
      {waiting ? <SidebarSkeleton rows={2} /> : !hasSession ? <p className="empty-row">No session selected</p> : rows.length === 0 && (view !== "all" || historical.length === 0) ? <p className="empty-row">No agents detected</p> : <>{queue.visible.map(renderRow)}{view === "all" && queue.hidden.length > 0 ? <><div className="agent-hidden-heading">Hidden <button type="button" aria-disabled={!live || undefined} title={!live ? "Herdr is not live" : undefined} onClick={() => { if (live) void attention.restoreAll(); }}>Restore all</button></div>{queue.hidden.map(renderRow)}</> : null}{view === "all" && historical.length > 0 ? <><div className="agent-hidden-heading">Past results not seen</div>{historical.map(record => {
        const key = historicalAttentionKey(record);
        const disabled = !live || attention.pending.has(key);
        return <div className="agent-historical" key={key}><span>{record.agent} · Past result not seen · {record.completion?.at}</span><button type="button" aria-label={`Mark past result from ${record.agent} seen`} aria-disabled={disabled || undefined} title={!live ? "Herdr is not live" : undefined} onClick={() => { if (!disabled) void attention.seenHistorical(record); }}>Mark seen</button>{attention.notes.get(key) ? <p className="agent-row-note" role="status">{attention.notes.get(key)}</p> : null}</div>;
      })}</> : null}</>}
    </div>
    {!waiting && view === "queue" && (queue.hidden.length > 0 || historical.length > 0) ? <div className="agent-hidden-footer">{queue.hidden.length > 0 ? `${queue.hidden.length} hidden` : ""}{queue.hidden.length > 0 && historical.length > 0 ? " · " : ""}{historical.length > 0 ? `${historical.length} past ${historical.length === 1 ? "result" : "results"} not seen` : ""} · <button type="button" onClick={() => chooseView("all")}>Show all</button></div> : null}
    <span className="sr-only" role="status" aria-live="polite">{attention.announcement}</span>
  </section>;
}
