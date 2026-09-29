import { useEffect, useRef, useState, type DragEvent, type PointerEvent } from "react";
import type { ResourceMutationRequest, SpaceGitStatus } from "../../protocol/generated/v1";
import { aheadBehindLabel } from "../session/spaceGitStatus";
import { withShortcut } from "../input/shortcuts";
import { InlineRename } from "../InlineRename";
import { UiIcon } from "../UiIcon";
import { SidebarSkeleton } from "./SidebarSkeleton";
import { StateGlyph } from "./StateGlyph";
import { projectSpaceTree, spaceDropBeforeId, spaceRawName, spaceRowStatus, spaceStatus, type Space } from "./spaceTree";
import { useRovingList } from "./useRovingList";

/** What the sidebar needs from a context-menu trigger: a point, from a pointer or from a row's box. */
export type ContextAnchor = { clientX: number; clientY: number; preventDefault(): void; stopPropagation(): void };
export type SpaceContextTarget = { kind: "space"; id: string };
type DragIntent = { sourceId: string; order: string[] };
type DropMark = { targetId: string; side: "before" | "after" } | null;
/** A rejected Space mutation, shown directly under the row it concerns. */
export type SpaceNote = { spaceId: string; id: string; message: string };

export function Spaces({ spaces, gitStatus, selectedSpaceId, pendingSpaceId, editingId, busy, loading, hasSession, notes, onEdit, onSelect, onContext, onSetup, setupEnabled, mutate, focusRef }: {
  spaces: Space[];
  gitStatus: ReadonlyMap<string, SpaceGitStatus>;
  /** The Space Herdr confirmed as focused. */
  selectedSpaceId: string | null;
  /** The Space a focus request is in flight for; Herdr has not confirmed it yet. */
  pendingSpaceId: string | null;
  editingId: string | null;
  /** A mutation is in flight: rows ignore clicks but keep keyboard focus. */
  busy: boolean;
  loading: boolean;
  hasSession: boolean;
  notes: readonly SpaceNote[];
  onEdit: (id: string | null) => void;
  onSelect: (space: Space) => void;
  onContext: (event: ContextAnchor, target: SpaceContextTarget) => void;
  onSetup: () => void;
  setupEnabled: boolean;
  mutate: (key: string, request: ResourceMutationRequest) => boolean;
  /** Set to a function that focuses the selected row, else the first. */
  focusRef: { current: (() => void) | null };
}) {
  const [collapsedRepositoryKeys, setCollapsedRepositoryKeys] = useState<Set<string>>(() => new Set());
  const [dragIntent, setDragIntent] = useState<DragIntent | null>(null);
  const [dropMark, setDropMark] = useState<DropMark>(null);
  const [staleDrag, setStaleDrag] = useState<{ spaceId: string; message: string } | null>(null);
  const [dismissedNotes, setDismissedNotes] = useState<ReadonlySet<string>>(() => new Set());
  const rows = projectSpaceTree(spaces, collapsedRepositoryKeys, selectedSpaceId);
  const visibleNotes = notes.filter((note) => !dismissedNotes.has(note.id));
  const noteFor = (spaceId: string): string | null => (staleDrag?.spaceId === spaceId ? staleDrag.message : null) ?? visibleNotes.find((note) => note.spaceId === spaceId)?.message ?? null;
  const toggleRepository = (repositoryKey: string) => {
    setCollapsedRepositoryKeys((current) => {
      const next = new Set(current);
      if (next.has(repositoryKey)) next.delete(repositoryKey);
      else next.add(repositoryKey);
      return next;
    });
  };
  const roving = useRovingList({
    rowIds: rows.map((row) => row.space.id),
    selectedId: selectedSpaceId,
    onEscape: () => {
      if (!staleDrag && visibleNotes.length === 0) return false;
      setStaleDrag(null);
      setDismissedNotes((current) => new Set([...current, ...visibleNotes.map((note) => note.id)]));
      return true;
    },
    onKey: (event, id) => {
      const index = rows.findIndex((row) => row.space.id === id);
      const row = rows[index];
      if (!row) return false;
      if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
        const box = event.currentTarget.getBoundingClientRect();
        onContext({ clientX: box.left + 24, clientY: box.bottom, preventDefault: () => event.preventDefault(), stopPropagation: () => event.stopPropagation() }, { kind: "space", id });
        return true;
      }
      if (event.shiftKey) return false;
      if (event.key === "ArrowRight" && row.kind === "parent" && row.repositoryKey) {
        if (!row.expanded) toggleRepository(row.repositoryKey);
        else roving.focusRow(rows[index + 1]?.kind === "child" ? rows[index + 1].space.id : undefined);
        return true;
      }
      if (event.key === "ArrowLeft") {
        if (row.kind === "parent" && row.expanded && row.repositoryKey) toggleRepository(row.repositoryKey);
        else if (row.kind === "child") roving.focusRow(rows.slice(0, index).reverse().find((candidate) => candidate.kind === "parent" && candidate.repositoryKey === row.repositoryKey)?.space.id);
        else return false;
        return true;
      }
      return false;
    },
  });
  focusRef.current = roving.focusTarget;

  // The rename input unmounts when a rename commits or cancels; give focus back to its row instead of dropping it to the page.
  const previousEditingId = useRef<string | null>(null);
  useEffect(() => {
    const renamed = previousEditingId.current;
    previousEditingId.current = editingId;
    if (renamed && !editingId && (!document.activeElement || document.activeElement === document.body)) roving.focusRow(renamed);
  }, [editingId, roving]);

  // A Space that no longer exists cannot keep a drag or its note alive.
  useEffect(() => {
    if (staleDrag && !spaces.some((space) => space.id === staleDrag.spaceId)) setStaleDrag(null);
  }, [spaces, staleDrag]);

  const startDrag = (event: DragEvent, space: Space) => {
    if (busy) return;
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("application/x-cockpit-space", space.id);
    event.dataTransfer.setData("text/plain", `space:${space.id}`);
    setDragIntent({ sourceId: space.id, order: spaces.map((candidate) => candidate.id) });
    setStaleDrag(null);
  };
  const dropSide = (event: PointerEvent | DragEvent) => {
    const box = event.currentTarget.getBoundingClientRect();
    return event.clientY >= box.top + box.height / 2 ? "after" as const : "before" as const;
  };

  return <section className="sidebar-section spaces-section" aria-labelledby="spaces-heading">
    <div className="sidebar-section-heading">
      <h2 id="spaces-heading">Spaces</h2>
      <span className="section-count">{spaces.length}</span>
      <button type="button" className="space-setup" aria-label="Set up a task Space" title={withShortcut("Set up a task Space", "setup-space")} disabled={busy || !setupEnabled} onClick={onSetup}><UiIcon name="plus" /></button>
    </div>
    <div className="space-list" role="group" aria-label="Spaces" ref={roving.listRef} {...roving.listProps}>
      {loading ? <SidebarSkeleton rows={6} />
        : !hasSession ? <p className="empty-row">No session selected</p>
        : spaces.length === 0 ? <p className="empty-row">No Spaces in this session</p>
        : rows.map((row, index) => {
          const space = row.space;
          const status = spaceStatus(spaceRowStatus(row, spaces));
          const ownStatus = spaceStatus(space.agent_status);
          const word = status.className === "unknown" ? "No agent" : status.word;
          const git = gitStatus.get(space.id);
          const branch = row.branch ?? git?.branch ?? null;
          const position = aheadBehindLabel(git);
          // Herdr shows the branch under every Space's name except a linked worktree's, which is named for its branch.
          const showBranch = Boolean(branch && !space.git?.is_linked_worktree && row.kind !== "child");
          const title = [
            `${row.kind === "child" ? spaceRawName(space) : space.label} · ${word}`,
            branch ? (git?.upstream ? `${branch} · ${position || "level"} vs ${git.upstream}` : branch) : null,
            space.git?.checkout_path ?? null,
          ].filter(Boolean).join("\n");
          const accessibleName = [
            row.label,
            word,
            showBranch ? `branch ${branch}` : null,
            showBranch && git?.upstream ? `${git.ahead ?? 0} ahead, ${git.behind ?? 0} behind` : null,
          ].filter(Boolean).join(", ");
          const pending = pendingSpaceId === space.id;
          const selected = selectedSpaceId === space.id && !pending;
          const side = dropMark?.targetId === space.id ? dropMark.side : null;
          const note = noteFor(space.id);
          const hiddenUrgency = row.kind === "parent" && !row.expanded && status.className !== ownStatus.className && status.className !== "unknown"
            ? ` (worktree ${status.word.toLowerCase()})` : "";
          return <div key={space.id} className="space-row-group">
            <div className={`space-tree-row space-tree-${row.kind}${showBranch ? " has-branch" : ""} state-${status.className}${selected ? " is-selected" : ""}${pending ? " is-pending" : ""}${side ? ` drop-${side}` : ""}`} draggable={!busy && editingId !== space.id}
              onDragStart={(event) => startDrag(event, space)}
              onDragEnd={() => { setDragIntent(null); setDropMark(null); }}
              onDragEnter={(event) => { if (!busy) event.preventDefault(); }}
              onDragOver={(event) => { if (!busy) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; if (dragIntent && dragIntent.sourceId !== space.id) setDropMark({ targetId: space.id, side: dropSide(event) }); } }}
              onDrop={(event) => {
                if (busy) return;
                event.preventDefault();
                const fallback = event.dataTransfer.getData("text/plain");
                const id = event.dataTransfer.getData("application/x-cockpit-space") || (fallback.startsWith("space:") ? fallback.slice(6) : "");
                const intent = dragIntent?.sourceId === id ? dragIntent : { sourceId: id, order: spaces.map((candidate) => candidate.id) };
                const unchanged = intent.order.length === spaces.length && intent.order.every((candidate, position) => candidate === spaces[position]?.id);
                const beforeSpaceId = unchanged ? spaceDropBeforeId(spaces, id, space.id, dropSide(event) === "after") : undefined;
                setDropMark(null);
                if (!unchanged) setStaleDrag({ spaceId: space.id, message: "Space order changed while dragging. Start again." });
                else if (beforeSpaceId !== undefined) mutate(`space:${id}`, { type: "space_move_block", space_ids: [id], before_space_id: beforeSpaceId });
              }}
              onPointerLeave={() => { if (dropMark?.targetId === space.id) setDropMark(null); }}
              onPointerMove={(event) => { if (dragIntent && dragIntent.sourceId !== space.id) setDropMark({ targetId: space.id, side: dropSide(event) }); }}
              onContextMenu={(event) => onContext(event, { kind: "space", id: space.id })}>
              {row.kind === "child" ? <span className={`space-connector${row.connector === "└─" ? " is-last" : ""}`} aria-hidden="true" /> : null}
              {editingId === space.id
                ? <InlineRename label={space.label} ariaLabel={`Rename Space ${space.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`space:${space.id}`, { type: "space_rename", space_id: space.id, label }); if (accepted) onEdit(null); return accepted; }} />
                : <button type="button" data-row-id={space.id} tabIndex={roving.tabIndexFor(space.id)} draggable={!busy} className="resource-select" title={title} aria-label={accessibleName} aria-disabled={busy || undefined} aria-current={selected ? "true" : undefined} aria-busy={pending || undefined}
                  onDragStart={(event) => startDrag(event, space)} onClick={() => { if (!busy) onSelect(space); }} onDoubleClick={() => onEdit(space.id)}>
                  <StateGlyph shape={status.shape} />
                  <span className="space-details">
                    <span className="space-label">{row.label}</span>
                    {showBranch ? <span className="space-branch"><span className="space-branch-name">{branch}</span>{position ? <span className="space-ahead-behind" aria-hidden="true">{position}</span> : null}</span> : null}
                  </span>
                </button>}
              {row.kind === "parent" && row.repositoryKey
                ? <button type="button" className="space-chevron" tabIndex={-1} aria-disabled={busy || undefined} aria-label={`${row.expanded ? "Collapse" : "Expand"} ${space.label}${hiddenUrgency}`} aria-expanded={row.expanded} onClick={() => { if (!busy) toggleRepository(row.repositoryKey!); }}><UiIcon name={row.expanded ? "down" : "right"} /></button>
                : null}
            </div>
            {note ? <p className="row-note" role="status">{note}</p> : null}
          </div>;
        })}
    </div>
  </section>;
}

/** Rejected Space mutations, as notes for the row they concern. */
export function spaceNotesFromFailures(failures: readonly { operation: { key: string; token: number; request: ResourceMutationRequest } }[]): SpaceNote[] {
  return failures.flatMap(({ operation }): SpaceNote[] => {
    const id = `${operation.key}:${operation.token}`;
    if (operation.request.type === "space_move_block") return [{ spaceId: operation.request.space_ids[0], id, message: "Move was not applied. Herdr kept the Space order." }];
    if (operation.request.type === "space_rename") return [{ spaceId: operation.request.space_id, id, message: "Rename was not applied. Herdr kept the old name." }];
    return [];
  });
}
