import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type PointerEvent } from "react";
import { createPortal } from "react-dom";
import type { ResourceMutationRequest, SpaceGitAction, SpaceGitStatus, SpaceServiceStatus } from "../../protocol/generated/v1";
import { aheadBehindLabel, SPACE_GIT_POLL_MS } from "../session/spaceGitStatus";
import { withShortcut } from "../input/shortcuts";
import { InlineRename } from "../InlineRename";
import { UiIcon } from "../UiIcon";
import { SidebarSkeleton } from "./SidebarSkeleton";
import { StateGlyph, spaceGlyphShape } from "./StateGlyph";
import { projectSpaceTree, spaceDropBeforeId, spaceRawName, spaceRowStatus, spaceStatus, type Space, type SpaceTreeRow } from "./spaceTree";
import { useRovingList } from "./useRovingList";
import { GitProblemNotice, SpaceGitAction as InlineGitAction } from "./SpaceGitAction";
import { gitReadProblem, type GitActionState } from "../session/spaceGitActions";
import type { SpaceTagsState } from "../session/spaceTags";
import { projectTagGroups, readSpacesView, writeSpacesView, type TagListRow, type SpacesView } from "./spaceTagView";
import { SpaceTagChip } from "./SpaceTagChip";
import { SpaceTagPopover } from "./SpaceTagPopover";

type DisplaySpaceRow = SpaceTreeRow & { rowId: string; first: boolean; grouped: boolean; exclude?: string };
type DisplayRow = DisplaySpaceRow | Extract<TagListRow, { kind: "group" }>;
const TAG_FAILURE = "Could not confirm the tag change. Check the tags before trying again.";

/** What the sidebar needs from a context-menu trigger: a point, from a pointer or from a row's box. */
export type ContextAnchor = { clientX: number; clientY: number; preventDefault(): void; stopPropagation(): void };
export type SpaceContextTarget = { kind: "space"; id: string };
type DragIntent = { sourceId: string; order: string[] };
type DropMark = { targetId: string; side: "before" | "after" } | null;
/** A rejected Space mutation, shown directly under the row it concerns. */
export type SpaceNote = { spaceId: string; id: string; message: string };

export function Spaces({ spaces, tags, tagEditorRequest, editorSuspended = false, services, pendingServiceKey, onSelectService, gitStatus, gitBlocked, gitEntry, gitStatusError, onRetryGitStatus, onGitAction, onDismissGitProblem, selectedSpaceId, pendingSpaceId, editingId, busy, loading, hasSession, notes, onEdit, onSelect, onContext, onSetup, setupEnabled, mutate, focusRef }: {
  spaces: Space[];
  tags: SpaceTagsState;
  tagEditorRequest: { spaceId: string; token: number } | null;
  editorSuspended?: boolean;
  gitStatus: ReadonlyMap<string, SpaceGitStatus>;
  services: ReadonlyMap<string, readonly SpaceServiceStatus[]>;
  pendingServiceKey: string | null;
  onSelectService(service: SpaceServiceStatus): void;
  gitBlocked?: string;
  gitEntry(status: SpaceGitStatus | undefined): GitActionState | undefined;
  onGitAction(spaceId: string, action: SpaceGitAction): void;
  onDismissGitProblem(root: string): void;
  gitStatusError?: string;
  onRetryGitStatus(): void;
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
  const [gitReadProblemId, setGitReadProblemId] = useState<string | null>(null);
  const focusedService = useRef<{ element: HTMLElement; spaceId: string } | null>(null);
  const focusedSpace = useRef<{ element: HTMLElement; spaceId: string } | null>(null);
  const [view, setView] = useState<SpacesView>(readSpacesView);
  const [collapsedTags, setCollapsedTags] = useState<Set<string>>(() => new Set());
  const [editor, setEditor] = useState<{ spaceId: string; rowId: string } | null>(null);
  const editorRef = useRef(editor);
  editorRef.current = editor;
  const editorTrigger = useRef<HTMLElement | null>(null);
  const [editorPosition, setEditorPosition] = useState<{ left: number; top: number; width: number } | null>(null);
  const [tagNotes, setTagNotes] = useState<SpaceNote[]>([]);
  const failureToken = useRef(0);
  const treeRows = projectSpaceTree(spaces, collapsedRepositoryKeys, selectedSpaceId);
  const rows: DisplayRow[] = view === "repository"
    ? treeRows.map(row => ({ ...row, rowId: row.space.id, first: true, grouped: false }))
    : !tags.ready ? [] : projectTagGroups(spaces, tags.bySpace, tags.catalog, collapsedTags).map(row => row.kind === "group" ? row : ({
      kind: "top-level", space: row.space, label: row.space.label, branch: row.space.git?.branch ?? null,
      repositoryKey: null, expanded: true, connector: null, rowId: row.rowId, first: row.first, grouped: true, exclude: row.exclude,
    }));
  const spaceRows = rows.filter((row): row is DisplaySpaceRow => row.kind !== "group");
  const rowsById = new Map(rows.map(row => [row.rowId, row]));
  const visibleNotes = [...notes, ...tagNotes].filter((note) => !dismissedNotes.has(note.id));
  const noteFor = (spaceId: string): string | null => (staleDrag?.spaceId === spaceId ? staleDrag.message : null) ?? visibleNotes.find((note) => note.spaceId === spaceId)?.message ?? null;
  const firstRow = (spaceId: string) => spaceRows.find(row => row.space.id === spaceId);
  const rowElement = (rowId: string) => [...roving.listRef.current?.querySelectorAll<HTMLButtonElement>("[data-row-id]") ?? []].find(element => element.dataset.rowId === rowId);
  const closeEditor = (reason: "escape" | "outside" | "toggle", outsideTarget?: Element) => {
    const current = editorRef.current;
    if (!current) return;
    setEditor(null);
    editorRef.current = null;
    const target = editorTrigger.current?.isConnected ? editorTrigger.current : rowElement(firstRow(current.spaceId)?.rowId ?? current.rowId);
    if (reason !== "outside") target?.focus({ preventScroll: true });
    else window.setTimeout(() => {
      if ((!outsideTarget?.closest("button, input, select, textarea, a[href], [tabindex]") || document.activeElement === document.body)
        && (!document.activeElement || document.activeElement === document.body)) (rowElement(firstRow(current.spaceId)?.rowId ?? current.rowId) ?? target)?.focus({ preventScroll: true });
    }, 0);
  };
  const openEditor = (spaceId: string, rowId: string, trigger: HTMLElement) => {
    if (tags.editBlocked || editingId || editorSuspended) return;
    if (editorRef.current?.rowId === rowId && editorTrigger.current === trigger) { closeEditor("toggle"); return; }
    editorTrigger.current = trigger;
    setEditor({ spaceId, rowId });
    window.setTimeout(() => {
      if (editorRef.current?.spaceId === spaceId) document.querySelector<HTMLInputElement>(".space-tag-input")?.focus({ preventScroll: true });
    }, 0);
  };
  const toggleTagGroup = (key: string) => setCollapsedTags(current => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });
  const toggleRepository = (repositoryKey: string) => {
    setCollapsedRepositoryKeys((current) => {
      const next = new Set(current);
      if (next.has(repositoryKey)) next.delete(repositoryKey);
      else next.add(repositoryKey);
      return next;
    });
  };
  const roving = useRovingList({
    rowIds: rows.flatMap(row => row.kind === "group" ? [row.rowId] : [row.rowId, ...(row.first ? services.get(row.space.id) ?? [] : []).map(service => `service:${row.space.id}:${service.key}`)]),
    selectedId: selectedSpaceId ? firstRow(selectedSpaceId)?.rowId ?? null : null,
    onEscape: () => {
      const focusedId = document.activeElement?.closest<HTMLElement>("[data-space-id]")?.dataset.spaceId;
      const git = focusedId ? gitStatus.get(focusedId) : undefined;
      const entry = gitEntry(git);
      if (entry?.result?.kind === "problem" || (gitReadProblemId === focusedId && !gitStatusError && gitReadProblem(git))) {
        if (git?.checkout.state !== "unavailable" && git?.checkout.root) onDismissGitProblem(git.checkout.root);
        setGitReadProblemId(null);
        return true;
      }
      if (!staleDrag && visibleNotes.length === 0) return false;
      setStaleDrag(null);
      setDismissedNotes((current) => new Set([...current, ...visibleNotes.map((note) => note.id)]));
      return true;
    },
    onKey: (event, id) => {
      if (id.startsWith("service:")) return false;
      const index = rows.findIndex(row => row.rowId === id);
      const row = rowsById.get(id);
      if (!row) return false;
      if (row.kind === "group") {
        if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
          if ((event.key === "ArrowRight") !== row.expanded) toggleTagGroup(row.groupKey);
          return true;
        }
        return false;
      }
      if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
        const box = event.target instanceof HTMLElement ? event.target.getBoundingClientRect() : event.currentTarget.getBoundingClientRect();
        onContext({ clientX: box.left + 24, clientY: box.bottom, preventDefault: () => event.preventDefault(), stopPropagation: () => event.stopPropagation() }, { kind: "space", id: row.space.id });
        return true;
      }
      if (event.shiftKey) return false;
      if (event.key === "ArrowRight" && row.kind === "parent" && row.repositoryKey) {
        if (!row.expanded) toggleRepository(row.repositoryKey);
        else { const next = rows[index + 1]; if (next?.kind === "child") roving.focusRow(next.rowId); }
        return true;
      }
      if (event.key === "ArrowLeft") {
        if (row.kind === "parent" && row.expanded && row.repositoryKey) toggleRepository(row.repositoryKey);
        else if (row.kind === "child") roving.focusRow(spaceRows.slice(0, spaceRows.indexOf(row)).reverse().find(candidate => candidate.kind === "parent" && candidate.repositoryKey === row.repositoryKey)?.rowId);
        else return false;
        return true;
      }
      return false;
    },
  });
  focusRef.current = roving.focusTarget;

  // A removed service must not strand keyboard focus on the page.
  useEffect(() => {
    const previous = focusedService.current;
    if (!previous || previous.element.isConnected) return;
    focusedService.current = null;
    if (!document.activeElement || document.activeElement === document.body) roving.focusRow(firstRow(previous.spaceId)?.rowId);
  });

  useLayoutEffect(() => {
    const previous = focusedSpace.current;
    if (!previous || previous.element.isConnected || (document.activeElement && document.activeElement !== document.body)) return;
    focusedSpace.current = null;
    const replacement = firstRow(previous.spaceId);
    if (replacement) roving.focusRow(replacement.rowId);
    else roving.focusTarget();
  });

  // The rename input unmounts when a rename commits or cancels; give focus back to its row instead of dropping it to the page.
  const previousEditingId = useRef<string | null>(null);
  useEffect(() => {
    const renamed = previousEditingId.current;
    previousEditingId.current = editingId;
    if (renamed && !editingId && (!document.activeElement || document.activeElement === document.body)) roving.focusRow(firstRow(renamed)?.rowId);
  }, [editingId, roving]);

  // A Space that no longer exists cannot keep a drag or its note alive.
  useEffect(() => {
    if (staleDrag && !spaces.some((space) => space.id === staleDrag.spaceId)) setStaleDrag(null);
  }, [spaces, staleDrag]);

  useEffect(() => {
    if (gitReadProblemId && !gitStatusError && !gitReadProblem(gitStatus.get(gitReadProblemId))) setGitReadProblemId(null);
  }, [gitReadProblemId, gitStatus, gitStatusError]);
  const handledEditorRequest = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (!tagEditorRequest || handledEditorRequest.current === tagEditorRequest.token) return;
    if (editingId || tags.editBlocked || editorSuspended) { handledEditorRequest.current = tagEditorRequest.token; return; }
    const space = spaces.find(candidate => candidate.id === tagEditorRequest.spaceId);
    if (!space) { handledEditorRequest.current = tagEditorRequest.token; return; }
    const target = firstRow(space.id);
    if (!target) {
      if (view === "tags") {
        const all = projectTagGroups(spaces, tags.bySpace, tags.catalog, new Set());
        const occurrence = all.find(row => row.kind === "space" && row.space.id === space.id);
        if (occurrence?.kind === "space") setCollapsedTags(current => new Set([...current].filter(key => key !== occurrence.groupKey)));
      } else if (space.git?.repository_key) setCollapsedRepositoryKeys(current => new Set([...current].filter(key => key !== space.git!.repository_key)));
      return;
    }
    const trigger = rowElement(target.rowId);
    if (!trigger) return;
    handledEditorRequest.current = tagEditorRequest.token;
    trigger.scrollIntoView?.({ block: "nearest" });
    openEditor(space.id, target.rowId, trigger);
  });

  // Keep one portal-mounted picker when tag membership moves its row between groups.
  useLayoutEffect(() => {
    if (!editor) return;
    if (editorSuspended) {
      setEditor(null);
      editorRef.current = null;
      return;
    }
    const row = spaceRows.find(candidate => candidate.rowId === editor.rowId) ?? firstRow(editor.spaceId);
    const anchor = row ? rowElement(row.rowId) : null;
    if (!anchor || anchor.closest("[hidden]")) {
      setEditor(null);
      editorRef.current = null;
      if (document.activeElement?.closest(".space-tag-popover") || document.activeElement === document.body) roving.focusTarget();
      return;
    }
    if (row!.rowId !== editor.rowId) {
      editorTrigger.current = anchor;
      setEditor({ spaceId: editor.spaceId, rowId: row!.rowId });
    }
    const position = () => {
      const box = anchor.closest(".space-tree-row")!.getBoundingClientRect();
      const width = Math.min(Math.max(0, box.width - 20), window.innerWidth - 20);
      const next = { left: Math.max(10, Math.min(box.left + 10, window.innerWidth - width - 10)), top: Math.max(8, Math.min(box.bottom, window.innerHeight - 250)), width };
      setEditorPosition(current => current && current.left === next.left && current.top === next.top && current.width === next.width ? current : next);
    };
    position();
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    return () => {
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
    };
  });
  useEffect(() => () => {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest(".space-tag-popover")) document.querySelector<HTMLElement>(".tab-button[aria-selected='true'], .drawer-toggle")?.focus({ preventScroll: true });
  }, []);

  const startDrag = (event: DragEvent, space: Space) => {
    if (busy) return;
    closeEditor("toggle");
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
      <div className="spaces-view-switch" role="group" aria-label="Spaces view">
        {(["repository", "tags"] as const).map(value => <button key={value} type="button" aria-pressed={view === value} onClick={event => {
          closeEditor("toggle");
          event.currentTarget.focus({ preventScroll: true });
          setView(value);
          writeSpacesView(value);
          setDragIntent(null);
          setDropMark(null);
        }}>{value === "repository" ? "Repository" : "Tags"}</button>)}
      </div>
      <button type="button" className="space-setup" aria-label="Set up a task Space" title={withShortcut("Set up a task Space", "setup-space")} disabled={busy || !setupEnabled} onClick={onSetup}><UiIcon name="plus" /></button>
    </div>
    {hasSession && gitStatusError ? <GitProblemNotice section problem={{
      kind: "problem", unknown: false, summary: "Git status unavailable.",
      details: [`Last-known status is retained. Pull and Push are off until a read succeeds. Cockpit reads again every ${SPACE_GIT_POLL_MS / 1000} s.`, gitStatusError],
    }} onRetry={onRetryGitStatus} /> : null}
    <div className="space-list" role="group" aria-label="Spaces" ref={roving.listRef} {...roving.listProps} onFocusCapture={event => {
      const element = event.target as HTMLElement;
      const spaceId = element.closest<HTMLElement>("[data-space-id]")?.dataset.spaceId;
      focusedService.current = element.dataset.rowId?.startsWith("service:") && spaceId ? { element, spaceId } : null;
      focusedSpace.current = spaceId ? { element, spaceId } : null;
    }}>
      {loading ? <SidebarSkeleton rows={6} />
        : !hasSession ? <p className="empty-row">No session selected</p>
        : spaces.length === 0 ? <p className="empty-row">No Spaces in this session</p>
        : view === "tags" && !tags.ready ? tags.failed ? <p className="empty-row">Tags unavailable</p> : <SidebarSkeleton rows={6} />
        : rows.map(row => {
          if (row.kind === "group") return <button key={row.rowId} type="button" className="space-tag-group" data-row-id={row.rowId} tabIndex={roving.tabIndexFor(row.rowId)} aria-expanded={row.expanded} aria-label={`${row.label}, ${row.count} Spaces, ${row.expanded ? "expanded" : "collapsed"}`} onClick={() => toggleTagGroup(row.groupKey)}>
            <UiIcon name={row.expanded ? "down" : "right"} /><span className="space-tag-group-name">{row.label}</span><span className="section-count">{row.count}</span>
          </button>;
          const space = row.space;
          const status = spaceStatus(spaceRowStatus(row, spaces));
          const ownStatus = spaceStatus(space.agent_status);
          const word = status.className === "unknown" ? "No agent" : status.word;
          const git = gitStatus.get(space.id);
          const checkout = git?.checkout;
          const branch = checkout ? (checkout.state === "branch" ? checkout.branch : null) : row.branch || null;
          const tracked = checkout?.state === "branch" && checkout.upstream.state === "tracked" ? checkout.upstream : undefined;
          const position = aheadBehindLabel(git);
          const child = space.git?.is_linked_worktree || row.kind === "child";
          const showBranch = !row.grouped && Boolean(!child && (branch || checkout || (gitStatusError && space.git)));
          const secondary = row.grouped ? [space.git?.repository, space.git?.branch].filter(Boolean).join(" · ") : "";
          const spaceTags = tags.bySpace.get(space.id) ?? [];
          const entry = gitEntry(git);
          const problem = entry?.result?.kind === "problem" ? entry.result : undefined;
          const readMessage = !gitStatusError ? gitReadProblem(git) : undefined;
          const readOpen = gitReadProblemId === space.id && Boolean(readMessage);
          const gitAction = <InlineGitAction status={git} child={Boolean(child)} entry={entry} blocked={gitBlocked} statusError={gitStatusError && (git || space.git) ? gitStatusError : undefined} onAction={action => { setGitReadProblemId(null); onGitAction(space.id, action); }} onReadProblem={readMessage ? () => setGitReadProblemId(space.id) : undefined} />;
          const title = [
            `${row.kind === "child" ? spaceRawName(space) : space.label} · ${word}`,
            branch ? (tracked ? `${branch} · ${position || "level"} vs ${tracked.name} (last fetch)` : branch) : checkout?.state === "detached" ? "Detached HEAD" : checkout?.state === "unavailable" ? checkout.message : null,
            checkout && checkout.state !== "unavailable" ? checkout.root : space.git?.checkout_path,
          ].filter(Boolean).join("\n");
          const accessibleName = [row.label, word, branch ? `branch ${branch}` : null,
            !row.grouped && tracked ? `${tracked.ahead} ahead, ${tracked.behind} behind ${tracked.name}` : null,
            spaceTags.length ? `tags ${spaceTags.join(", ")}` : null].filter(Boolean).join(", ");
          const pending = pendingSpaceId === space.id;
          const selected = selectedSpaceId === space.id && !pending;
          const side = !row.grouped && dropMark?.targetId === space.id ? dropMark.side : null;
          const note = noteFor(space.id);
          const hiddenUrgency = row.kind === "parent" && !row.expanded && status.className !== ownStatus.className && status.className !== "unknown"
            ? ` (worktree ${status.word.toLowerCase()})` : "";
          const hiddenGitProblem = row.kind === "parent" && !row.expanded && spaces.some(candidate => candidate.id !== space.id
            && candidate.git?.repository_key === row.repositoryKey && !spaceRows.some(visible => visible.space.id === candidate.id)
            && gitEntry(gitStatus.get(candidate.id))?.result?.kind === "problem");
          return <div key={row.rowId} className="space-row-group" data-space-id={space.id}>
            <div className={`space-tree-row space-tree-${row.kind}${row.grouped ? " space-tagged-row" : ""}${showBranch || secondary ? " has-branch" : ""} state-${status.className}${selected ? " is-selected" : ""}${pending ? " is-pending" : ""}${side ? ` drop-${side}` : ""}`} draggable={!row.grouped && !busy && editingId !== space.id}
              onDragStart={row.grouped ? undefined : (event) => startDrag(event, space)}
              onDragEnd={row.grouped ? undefined : () => { setDragIntent(null); setDropMark(null); }}
              onDragEnter={row.grouped ? undefined : (event) => { if (!busy) event.preventDefault(); }}
              onDragOver={row.grouped ? undefined : (event) => { if (!busy) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; if (dragIntent && dragIntent.sourceId !== space.id) setDropMark({ targetId: space.id, side: dropSide(event) }); } }}
              onDrop={row.grouped ? undefined : (event) => {
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
              onPointerLeave={row.grouped ? undefined : () => { if (dropMark?.targetId === space.id) setDropMark(null); }}
              onPointerMove={row.grouped ? undefined : (event) => { if (dragIntent && dragIntent.sourceId !== space.id) setDropMark({ targetId: space.id, side: dropSide(event) }); }}
              onContextMenu={(event) => onContext(event, { kind: "space", id: space.id })}>
              {row.kind === "child" ? <span className={`space-connector${row.connector === "└─" ? " is-last" : ""}`} aria-hidden="true" /> : null}
              {editingId === space.id && row.first
                ? <InlineRename label={space.label} ariaLabel={`Rename Space ${space.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`space:${space.id}`, { type: "space_rename", space_id: space.id, label }); if (accepted) onEdit(null); return accepted; }} />
                : <>
                  <button type="button" data-row-id={row.rowId} tabIndex={roving.tabIndexFor(row.rowId)} draggable={!row.grouped && !busy} className="resource-select" title={title} aria-label={accessibleName} aria-disabled={busy || undefined} aria-current={selected && row.first ? "true" : undefined} aria-busy={pending || undefined}
                    onDragStart={row.grouped ? undefined : (event) => startDrag(event, space)} onClick={() => { if (!busy) onSelect(space); }} onDoubleClick={() => onEdit(space.id)} />
                  <div className="space-row-content" aria-hidden={undefined}>
                    <StateGlyph shape={spaceGlyphShape(status)} />
                    <span className="space-details">
                      <span className="space-name-line"><span className="space-label">{row.label}</span>{!row.grouped && child ? gitAction : null}</span>
                      {showBranch ? <span className="space-branch">{branch ? <span className="space-branch-name">{branch}</span> : null}{gitAction}</span> : null}
                      {secondary ? <span className="space-tag-secondary">{secondary}</span> : null}
                    </span>
                    {tags.ready ? <SpaceTagChip spaceLabel={space.label} tags={spaceTags} exclude={row.exclude} showAdd={!row.exclude} expanded={editor?.rowId === row.rowId} disabled={tags.editBlocked} busy={tags.pending.has(space.id)} onToggle={trigger => openEditor(space.id, row.rowId, trigger)} /> : null}
                  </div>
                </>}
              {row.kind === "parent" && row.repositoryKey
                ? <button type="button" className={`space-chevron${hiddenGitProblem ? " has-git-problem" : ""}`} tabIndex={-1} aria-disabled={busy || undefined} aria-label={`${row.expanded ? "Collapse" : "Expand"} ${space.label}${hiddenUrgency}${hiddenGitProblem ? " (worktree Git problem)" : ""}`} aria-expanded={row.expanded} onClick={() => { if (!busy) toggleRepository(row.repositoryKey!); }}><UiIcon name={row.expanded ? "down" : "right"} /></button>
                : null}
            </div>
            {row.first && services.get(space.id)?.length ? <div className="space-service-list" role="group" aria-label={`Services in ${space.label}`}>
              {services.get(space.id)!.map(service => {
                const id = `service:${space.id}:${service.key}`;
                const label = [service.title, service.status, service.port === null ? null : `port ${service.port}`].filter(Boolean).join(", ");
                const servicePending = pendingServiceKey === service.key;
                return <button key={service.key} type="button" className="space-service-row" data-row-id={id} tabIndex={roving.tabIndexFor(id)}
                  title={`${label}. Focus ${service.tab_id === null ? "pane" : "tab"}`} aria-label={label} aria-disabled={busy || undefined} aria-busy={servicePending || undefined}
                  onClick={() => { if (!busy) onSelectService(service); }}>
                  <span className={`space-service-dot is-${service.status}`} aria-hidden="true" />
                  <span className="space-service-name">{service.title}</span>
                  {service.port === null ? null : <span className="space-service-port">:{service.port}</span>}
                </button>;
              })}
            </div> : null}
            {row.first ? <span className="sr-only" role="status">{entry?.pending ? `${entry.request.action === "push" ? "Pushing" : "Pulling"} ${space.label}: ${entry.request.expected_branch} ${entry.request.action === "push" ? "→" : "←"} ${entry.request.expected_upstream}` : entry?.result?.kind === "done" ? `${space.label}: ${entry.result.message}` : ""}</span> : null}
            {row.first && note ? <p className="row-note" role="status">{note}</p> : null}
            {row.first && problem ? <GitProblemNotice key={entry!.token} problem={problem} child={Boolean(child)} onDismiss={() => {
              if (checkout && checkout.state !== "unavailable") onDismissGitProblem(checkout.root);
              roving.focusRow(row.rowId);
            }} /> : row.first && readOpen ? <GitProblemNotice key="read" problem={{ kind: "problem", unknown: false, summary: "Git status unavailable.", details: [readMessage!] }} child={Boolean(child)} onDismiss={() => {
              setGitReadProblemId(null);
              roving.focusRow(row.rowId);
            }} /> : null}
          </div>;
        })}
    </div>
    {!editorSuspended && editor && editorPosition && spaces.some(space => space.id === editor.spaceId) ? createPortal(<div className="space-tag-layer" style={editorPosition}>
      <SpaceTagPopover key={editor.spaceId} spaceLabel={spaces.find(space => space.id === editor.spaceId)!.label} catalog={tags.catalog} tags={tags.bySpace.get(editor.spaceId) ?? []} busy={tags.pending.has(editor.spaceId)} blocked={tags.editBlocked} triggerRef={editorTrigger} onClose={closeEditor} onChange={async (tag, assigned) => {
        const spaceId = editor.spaceId;
        const applied = await tags.change(spaceId, tag, assigned);
        if (!applied && editorRef.current?.spaceId !== spaceId) setTagNotes(current => [...current.filter(note => note.spaceId !== spaceId), { spaceId, id: `tag:${spaceId}:${++failureToken.current}`, message: TAG_FAILURE }]);
        return applied;
      }} />
    </div>, document.body) : null}
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
