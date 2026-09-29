import type { SessionSnapshotResponse } from "../../protocol/generated/v1";
import { StateGlyph } from "./StateGlyph";
import { SidebarSkeleton } from "./SidebarSkeleton";
import { orderAgentsByHerdrPriority, spaceStatus, type Agent, type Space } from "./spaceTree";
import { useRovingList } from "./useRovingList";

type Tab = SessionSnapshotResponse["tabs"][number];

export function Agents({ agents, spaces, tabs, selectedPaneId, pendingPaneId, loading, hasSession, onSelect, focusRef }: {
  agents: Agent[];
  spaces: Space[];
  tabs: Tab[];
  /** The pane Herdr confirmed as focused. */
  selectedPaneId: string | null;
  /** The pane a focus request is in flight for; Herdr has not confirmed it yet. */
  pendingPaneId: string | null;
  loading: boolean;
  hasSession: boolean;
  onSelect: (agent: Agent) => void;
  /** Set to a function that focuses the selected row, else the first. */
  focusRef: { current: (() => void) | null };
}) {
  const orderedAgents = orderAgentsByHerdrPriority(agents);
  const rowKey = (agent: Agent) => `${agent.pane_id}:${agent.name}`;
  const selectedAgent = orderedAgents.find((agent) => agent.pane_id === selectedPaneId && agent.pane_id !== pendingPaneId);
  const roving = useRovingList({ rowIds: orderedAgents.map(rowKey), selectedId: selectedAgent ? rowKey(selectedAgent) : null });
  focusRef.current = roving.focusTarget;
  return <section className="sidebar-section agents-section" aria-labelledby="agents-heading">
    <div className="sidebar-section-heading"><h2 id="agents-heading">Agents</h2><span className="section-count">{orderedAgents.length}</span></div>
    <div className="agent-list" role="group" aria-label="Agents" ref={roving.listRef} {...roving.listProps}>
      {loading ? <SidebarSkeleton rows={2} />
        : !hasSession ? <p className="empty-row">No session selected</p>
        : orderedAgents.length === 0 ? <p className="empty-row">No agents detected</p>
        : orderedAgents.map((agent) => {
          const spaceLabel = spaces.find((space) => space.id === agent.space_id)?.label;
          const tabLabel = tabs.find((tab) => tab.id === agent.tab_id)?.label;
          const location = [spaceLabel, tabLabel].filter(Boolean).join(" · ");
          const status = spaceStatus(agent.status || "unknown");
          const pending = agent.pane_id === pendingPaneId;
          const selected = agent.pane_id === selectedPaneId && !pending;
          const key = rowKey(agent);
          const description = [location, agent.name, agent.status || "unknown"].filter(Boolean);
          return <button type="button" key={key} data-row-id={key} tabIndex={roving.tabIndexFor(key)} className={`agent-row state-${status.className}${selected ? " is-selected" : ""}${pending ? " is-pending" : ""}`}
            onClick={() => onSelect(agent)} title={description.join(" · ")} aria-label={description.join(", ")} aria-current={selected ? "true" : undefined} aria-busy={pending || undefined}>
            <StateGlyph shape={status.shape} />
            <span className="agent-details">
              {location ? <span className="agent-location">{spaceLabel ? <span className="agent-space">{spaceLabel}</span> : null}{tabLabel ? <span className="agent-tab">{spaceLabel ? " · " : ""}{tabLabel}</span> : null}</span> : null}
              <span className="agent-sub"><span className="agent-word">{status.word}</span><span className="agent-name">{agent.name ? ` · ${agent.name}` : ""}</span></span>
            </span>
          </button>;
        })}
    </div>
  </section>;
}
