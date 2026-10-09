import { useState, type MouseEvent } from "react";
import { UiIcon } from "../UiIcon";
import { InlineRename } from "../InlineRename";
import { LibraryProblems } from "../library/LibraryProblems";
import { tabDropInsertionIndex } from "../layout/layoutProjection";
import { withShortcut } from "../input/shortcuts";
import type { ContextTarget } from "./ContextMenu";
import type { Mutate, Tab } from "./model";

export type DragIntent = { kind: "space" | "tab"; sourceId: string; order: string[] };
export type DropMark = { targetId: string; side: "before" | "after" } | null;

export function tabLabelIsRedundant(label: string, displayedNumber: number): boolean {
  const trimmed = label.trim();
  return trimmed === String(displayedNumber) || /^\d+$/.test(trimmed);
}

export function TabStrip({ tabs, selectedTabId, editingId, busy, browserOpen, browserDisabledReason, libraryOpen, supervisorOpen, onSupervisor, notesOpen, onNotesToggle, widgetDots, widgetsPending, onWidgets, onEdit, onSelect, onContext, onCreate, onBrowserToggle, onLibraryToggle, onCommands, sidebarOpen, onToggleSidebar, mutate }: {
  tabs: Tab[];
  selectedTabId: string | null;
  editingId: string | null;
  busy: boolean;
  browserOpen: boolean;
  browserDisabledReason: string | null;
  libraryOpen: boolean;
  supervisorOpen: boolean;
  onSupervisor(): void;
  notesOpen: boolean;
  onNotesToggle(): void;
  widgetDots: ReadonlySet<string>;
  widgetsPending: boolean;
  onWidgets(): void;
  onEdit: (id: string | null) => void;
  onSelect: (tab: Tab) => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onCreate: () => void;
  onBrowserToggle: () => void;
  onLibraryToggle: () => void;
  onCommands: () => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  mutate: Mutate;
}) {
  const [dragIntent, setDragIntent] = useState<DragIntent | null>(null);
  const [dropMark, setDropMark] = useState<DropMark>(null);
  const [dragMessage, setDragMessage] = useState<string | null>(null);
  return <nav className="tab-toolbar" aria-label="Tabs"><button type="button" className="tab-icon-button" aria-label={sidebarOpen ? "Hide sidebar" : "Show sidebar"} title={withShortcut(sidebarOpen ? "Hide sidebar" : "Show sidebar", "toggle-sidebar")} aria-expanded={sidebarOpen} aria-controls="cockpit-sidebar" onClick={onToggleSidebar}><UiIcon name={sidebarOpen ? "sidebar-open" : "sidebar"} /></button><div className="tab-strip" role="tablist">{tabs.map((tab, index) => {
    const displayedNumber = index + 1;
    const redundantLabel = tabLabelIsRedundant(tab.label, displayedNumber);
    const accessibleLabel = redundantLabel ? `Tab ${displayedNumber}` : `Tab ${displayedNumber}: ${tab.label}`;
    const side = dropMark?.targetId === tab.id ? dropMark.side : null;
    return <div className={`tab-item${tab.id === selectedTabId ? " is-selected" : ""}${side ? ` drop-${side}` : ""}`} key={tab.id} draggable={!busy && editingId !== tab.id}
      onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-tab", tab.id); event.dataTransfer.setData("text/plain", `tab:${tab.id}`); setDragIntent({ kind: "tab", sourceId: tab.id, order: tabs.map((candidate) => candidate.id) }); setDragMessage(null); } }}
      onDragEnd={() => { setDragIntent(null); setDropMark(null); }}
      onDragEnter={(event) => { if (!busy) event.preventDefault(); }}
      onDragOver={(event) => { if (!busy) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; if (dragIntent && dragIntent.sourceId !== tab.id) setDropMark({ targetId: tab.id, side: event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2 ? "after" : "before" }); } }}
      onDrop={(event) => {
        if (busy) return;
        event.preventDefault();
        const fallback = event.dataTransfer.getData("text/plain");
        const id = event.dataTransfer.getData("application/x-cockpit-tab") || (fallback.startsWith("tab:") ? fallback.slice(4) : "");
        if (!id || id === tab.id) return;
        const afterTarget = event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2;
        const intent = dragIntent?.sourceId === id ? dragIntent : { kind: "tab" as const, sourceId: id, order: tabs.map((candidate) => candidate.id) };
        const unchanged = intent.order.length === tabs.length && intent.order.every((candidate, position) => candidate === tabs[position]?.id);
        const insertion = unchanged ? tabDropInsertionIndex(tabs.findIndex((candidate) => candidate.id === id), index, afterTarget) : null;
        setDropMark(null);
        if (!unchanged) setDragMessage("Tab order changed while dragging. Start again.");
        else if (insertion !== null) mutate(`tab:${id}`, { type: "tab_move", tab_id: id, insert_index: insertion });
      }}
      onContextMenu={(event) => onContext(event, { kind: "tab", id: tab.id })}>
      {editingId === tab.id
        ? <InlineRename label={tab.label} ariaLabel={`Rename tab ${tab.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`tab:${tab.id}`, { type: "tab_rename", tab_id: tab.id, label }); if (accepted) onEdit(null); return accepted; }} />
        : <button type="button" disabled={busy} draggable={!busy} role="tab" aria-selected={tab.id === selectedTabId} aria-label={accessibleLabel} className="tab-button" title={displayedNumber <= 9 ? withShortcut(redundantLabel ? `Tab ${displayedNumber}` : tab.label, `select-tab-${displayedNumber as 1}`) : redundantLabel ? `Tab ${displayedNumber}` : tab.label} onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-tab", tab.id); event.dataTransfer.setData("text/plain", `tab:${tab.id}`); setDragIntent({ kind: "tab", sourceId: tab.id, order: tabs.map((candidate) => candidate.id) }); setDragMessage(null); } }} onClick={() => onSelect(tab)} onDoubleClick={() => onEdit(tab.id)}><span className="n">{displayedNumber}</span>{redundantLabel ? null : <span className="tab-label">{tab.label}</span>}{widgetDots.has(tab.id) ? <span className="widget-dot"><span className="sr-only">widget</span></span> : null}</button>}
    </div>;
  })}
    <button type="button" disabled={busy} className="tab-add" aria-label="Create tab" title={withShortcut("New tab", "new-tab")} onClick={onCreate}><UiIcon name="plus" /></button></div>{dragMessage ? <span className="resource-inline-status tab-drag-status" role="status">{dragMessage}</span> : null}<div className="tab-strip-actions"><span className="tab-strip-separator" aria-hidden="true" />{widgetsPending ? <button type="button" className="tab-strip-action" aria-label="Show widgets" disabled={busy} onClick={onWidgets}>Widgets <span className="widget-dot" aria-hidden="true" /></button> : null}<button type="button" className="tab-icon-button" disabled={busy || Boolean(browserDisabledReason)} aria-label="Browser" aria-pressed={browserOpen} title={browserDisabledReason ?? withShortcut(browserOpen ? "Close Browser (stops it and deletes its profile: cookies, logins, site data)" : "Open browser for tab", "toggle-browser")} onClick={onBrowserToggle}><UiIcon name="browser" /></button><button type="button" className="tab-icon-button" aria-label="Library" aria-pressed={libraryOpen} title={withShortcut(libraryOpen ? "Close Library" : "Open Library", "toggle-library")} onClick={onLibraryToggle}><UiIcon name="library" /></button><LibraryProblems /><button type="button" className="tab-strip-action" aria-label={notesOpen ? "Notes (open)" : "Open Notes"} aria-controls="cockpit-notes" aria-pressed={notesOpen} title={notesOpen ? "Close Notes" : "Open Notes"} onClick={onNotesToggle}>Notes</button><button type="button" className="tab-strip-action" title={withShortcut("Commands", "help")} onClick={onCommands}>Commands</button></div>
    <div className="tab-strip-actions supervisor-topbar-actions"><button type="button" className="tab-strip-action" aria-pressed={supervisorOpen} onClick={onSupervisor}>Supervisor</button></div>
  </nav>;
}
