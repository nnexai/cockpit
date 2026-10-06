import { Fragment, useCallback, useContext, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { DndContext, DragOverlay, useSensor, useSensors, useDraggable, useDroppable, pointerWithin, rectIntersection, type KeyboardCoordinateGetter, type CollisionDetection } from "@dnd-kit/core";
import type { NotesColumn, NotesOperation, NotesTodo } from "../../protocol/generated/v1";
import { TodoTitle } from "./TodoTitle";
import { todoIdentity, todoSelector, type NotesModel } from "./useNotes"
import { kanbanDropOperation, kanbanKeyboardColumn } from "./boardState";
import { NotesEditorScope } from "./MarkdownEditor";
import { NotesKeyboardSensor, NotesMouseSensor, NotesTouchSensor } from "./notesSensors";
import { UiIcon } from "../UiIcon";
const boardPositions = new Map<string, { left: number; columns: Record<NotesColumn, number> }>();

const columns: NotesColumn[] = ["backlog", "doing", "done"];
const labels: Record<NotesColumn, string> = { backlog: "Backlog", doing: "Doing", done: "Done" };
function columnFor(todo: NotesTodo): NotesColumn { return todo.done ? "done" : todo.lane ?? "backlog"; }
const collision: CollisionDetection = args => args.pointerCoordinates ? pointerWithin(args) : rectIntersection(args);
// Notes owns the accurate polite drag status; dnd-kit's collision announcements
// can lag a keyboard choice and must not contradict that existing live region.
const noDragAnnouncement = () => undefined;
const dragAnnouncements = { onDragStart: noDragAnnouncement, onDragOver: noDragAnnouncement, onDragEnd: noDragAnnouncement, onDragCancel: noDragAnnouncement };
function DropZone({ column, chip = false, destination, children }: { column: NotesColumn; chip?: boolean; destination?: NotesColumn | null; children: ReactNode }) {
  const { setNodeRef, isOver } = useDroppable({ id: `${chip ? "chip" : "column"}:${column}`, data: { column } });
  const highlighted = destination === undefined ? isOver : destination === column;
  return <div ref={setNodeRef} className={`${chip ? "kanban-drop-chip" : "kanban-column"}${highlighted ? " is-drop-target" : ""}`} data-col={column} id={chip ? undefined : `col-${column}`}>{children}</div>;
}
function Card({ todo, model, onDetails, count, selected, dragBusy }: { todo: NotesTodo; model: NotesModel; onDetails(todo: NotesTodo, opener: HTMLElement): void; count?: number; selected: boolean; dragBusy: boolean }) {
  const id = todoIdentity(todo);
  const disabled = model.busy || model.error?.code === "notes_outcome_unknown" || todo.problems.includes("metadata_malformed") || todo.problems.includes("lazy_continuation");
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, isDragging } = useDraggable({ id, data: { todo, column: columnFor(todo) }, disabled });
  const apply = (operation: NotesOperation, control: string) => {
    const root = document.querySelector<HTMLElement>(".notes-view");
    const invoker = document.activeElement;
    let updatedId = id;
    void model.mutate(operation, response => { if (response.result.kind === "todo") updatedId = todoIdentity(response.result.todo); }).then(success => {
      if (!success) return;
      requestAnimationFrame(() => {
        if (!root?.isConnected || (document.activeElement !== invoker && document.activeElement !== document.body)) return;
        const card = Array.from(root.querySelectorAll<HTMLElement>(".kanban-card")).find(node => node.dataset.todoId === updatedId);
        const target = card?.querySelector<HTMLElement>(control) ?? (operation.op === "kanban_unboard" ? root.querySelector<HTMLElement>("#kanbanAddInput") : null);
        if (!target || target.closest("[hidden]")) return;
        card?.scrollIntoView({ block: "nearest", inline: "nearest" }); target.focus({ preventScroll: true });
      });
    });
  };
  return <li ref={setNodeRef} className={`kanban-card${selected ? " is-selected" : ""}${isDragging ? " is-dragging" : ""}`} data-todo-id={id} onClick={event => {
    if (dragBusy || (event.target instanceof Element && event.target.closest("button,input,textarea,select,a,label,summary,details"))) return;
    const opener = event.currentTarget.querySelector<HTMLElement>(".kanban-card-comments");
    if (opener) onDetails(todo, opener);
  }}>
    <div className="kanban-card-main"><button type="button" ref={setActivatorNodeRef} className="kanban-card-handle" {...attributes} {...listeners} onKeyDown={event => {
      if (event.nativeEvent.isComposing || event.repeat) { event.stopPropagation(); return; }
      if (event.key === "Tab" && dragBusy) {
        // dnd-kit consumes its cancel key; reproduce the ordinary next tab stop after cancellation.
        const root = event.currentTarget.closest(".notes-view");
        const stops = Array.from(root?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), summary, [tabindex="0"], .cm-content') ?? []).filter(node => node.getClientRects().length > 0 && !node.closest("[hidden], [inert]"));
        const index = stops.indexOf(event.currentTarget);
        const next = stops[index + (event.shiftKey ? -1 : 1)];
        if (next) requestAnimationFrame(() => requestAnimationFrame(() => next.focus()));
      }
      listeners?.onKeyDown?.(event);
    }} aria-label={`Move card: ${todo.text}`} aria-describedby="notes-drag-instructions" disabled={disabled}><UiIcon name="more" /></button><input className="kanban-card-check" type="checkbox" aria-label={`Complete ${todo.text}`} checked={todo.done} disabled={disabled} onChange={() => void apply({ op: "todo_set_done", todo: todoSelector(todo), done: !todo.done }, ".kanban-card-check")} /><TodoTitle model={model} todo={todo} className="kanban-card-title" /></div>
    <div className="kanban-card-actions"><button className="kanban-card-comments" type="button" aria-label={`${!todo.id || todo.problems.includes("duplicate_id") ? "Adopt task & comments" : count === undefined ? "Comments" : count ? `Comments ${count}` : "Comment"}${todo.id && (model.drafts.commentNew[todo.id]?.body || Object.entries(model.drafts.comments).some(([key, draft]) => key.startsWith(`${todo.id}/`) && draft.value !== draft.base)) ? " · Draft" : ""}`} onClick={event => onDetails(todo, event.currentTarget)}><UiIcon name="comment" />{count !== undefined && count > 0 ? <span className="notes-comment-count">{count}</span> : null}{todo.id && (model.drafts.commentNew[todo.id]?.body || Object.entries(model.drafts.comments).some(([key, draft]) => key.startsWith(`${todo.id}/`) && draft.value !== draft.base)) ? <span className="notes-draft-dot" title="Draft kept" /> : null}</button><details className="kanban-card-menu"><summary aria-label={`Card actions: ${todo.text}`} title="Card actions"><UiIcon name="more" /></summary><div className="kanban-card-menu-body"><label className="notes-move-label">Move<select className="kanban-card-move" aria-label={`Move ${todo.text} to column`} value={columnFor(todo)} disabled={disabled} onChange={event => { const to = event.target.value; if (to === "backlog" || to === "doing" || to === "done") void apply({ op: "kanban_move", todo: todoSelector(todo), to }, ".kanban-card-move"); }}>{columns.map(column => <option value={column} key={column}>{labels[column]}</option>)}</select></label><button className="kanban-card-unboard" type="button" disabled={disabled} title="Keeps the task and its comments" onClick={() => void apply({ op: "kanban_unboard", todo: todoSelector(todo) }, ".kanban-card-unboard")}><UiIcon name="back" />Remove from board</button></div></details></div>
  </li>;
}
export function Kanban({ model, onDetails, counts, selectedId, hidden = false }: { model: NotesModel; onDetails(todo: NotesTodo, opener: HTMLElement): void; counts: Record<string, number>; selectedId: string | null; hidden?: boolean }) {
  const [active, setActive] = useState<NotesTodo | null>(null);
  const dragSnapshot = useRef<NotesTodo | null>(null);
  const keyboardDestination = useRef<NotesColumn | null>(null);
  const [destination, setDestination] = useState<NotesColumn | null>(null);
  const [announcement, announce] = useState("");
  const board = useRef<HTMLDivElement>(null);
  const dragSuppress = useRef(false);
  const scope = useContext(NotesEditorScope);
  const restored = useRef(false);
  useLayoutEffect(() => {
    if (!model.scratchpad || restored.current || !board.current) return;
    restored.current = true;
    const saved = boardPositions.get(scope);
    if (!saved) return;
    board.current.scrollLeft = saved.left;
    for (const column of columns) { const list = board.current.querySelector<HTMLElement>(`[data-col="${column}"] .kanban-card-list`); if (list) list.scrollTop = saved.columns[column]; }
  }, [scope, model.scratchpad]);
  const cancelSensor = useRef<(() => void) | null>(null);
  const registerCancellation = useCallback((cancel: () => void) => {
    cancelSensor.current = cancel;
    return () => { if (cancelSensor.current === cancel) cancelSensor.current = null; };
  }, []);
  const cancelGesture = useCallback(() => { cancelSensor.current?.(); dragSnapshot.current = null; keyboardDestination.current = null; }, []);
  useLayoutEffect(() => {
    window.addEventListener("blur", cancelGesture);
    return () => { cancelGesture(); window.removeEventListener("blur", cancelGesture); };
  }, [cancelGesture]);
  useLayoutEffect(() => { if (hidden) cancelGesture(); }, [hidden, cancelGesture]);
  useLayoutEffect(() => {
    if (!active) return;
    const current = model.todos.find(todo => todoIdentity(todo) === todoIdentity(active));
    if (!current || current.revision !== active.revision || current.lane === null) cancelGesture();
  }, [active, model.todos, cancelGesture]);
  const coordinates = useCallback<KeyboardCoordinateGetter>((event, { context, currentCoordinates }) => {
    if ((event.code !== "ArrowLeft" && event.code !== "ArrowRight") || !keyboardDestination.current) return undefined;
    event.preventDefault();
    // Intent was already captured by the board before the sensor sees the key.
    const target = context.droppableContainers.get(`column:${keyboardDestination.current}`);
    const rect = target ? context.droppableRects.get(target.id) : null;
    return rect ? { x: rect.left + rect.width / 2, y: rect.top + Math.min(80, rect.height / 2) } : currentCoordinates;
  }, []);
  const sensors = useSensors(useSensor(NotesMouseSensor, { registerCancellation, activationConstraint: { distance: 5 } }), useSensor(NotesTouchSensor, { registerCancellation, activationConstraint: { delay: 200, tolerance: 8 } }), useSensor(NotesKeyboardSensor, { registerCancellation, coordinateGetter: coordinates, keyboardCodes: { start: ["Space", "Enter"], cancel: ["Escape", "Tab"], end: ["Space", "Enter"] } }));
  const tasks = model.todos.filter(todo => todo.lane !== null);
  const restore = (id: string) => requestAnimationFrame(() => {
    if (!board.current || !document.hasFocus() || board.current.closest("[inert]") || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && focused !== document.body && (!focused.classList.contains("kanban-card-handle") || focused.closest<HTMLElement>("[data-todo-id]")?.dataset.todoId !== id)) return;
    const cards = board.current?.querySelectorAll<HTMLElement>("[data-todo-id]") ?? [];
    const card = Array.from(cards).find(node => node.dataset.todoId === id);
    card?.scrollIntoView({ block: "nearest", inline: "nearest" }); card?.querySelector<HTMLElement>(".kanban-card-handle")?.focus({ preventScroll: true });
  });
  return <div className="notes-kanban" hidden={hidden} onKeyDownCapture={event => {
    if (!keyboardDestination.current || event.nativeEvent.isComposing || event.repeat || (event.code !== "ArrowLeft" && event.code !== "ArrowRight")) return;
    event.preventDefault();
    // The pinned sensor installs its document listener asynchronously and may
    // miss the first arrow. Record intent on the existing board event path,
    // before either that startup gap or a collision/render lag can lose it.
    const next = kanbanKeyboardColumn(keyboardDestination.current, event.code);
    keyboardDestination.current = next;
    setDestination(next);
    const captured = dragSnapshot.current;
    if (captured) announce(`${labels[next]} — ${next === "done" ? "checks the task" : captured.done ? "reopens the task" : "keeps the task open"}. File order is preserved.`);
  }} onKeyDown={event => {
    // Keep Escape inside Notes, but let the real sensor see it and detach.
    if (dragSnapshot.current && event.key === "Escape") event.preventDefault();
  }} onBlurCapture={event => {
    if (event.target instanceof HTMLElement && event.target.classList.contains("kanban-card-handle")) cancelGesture();
  }}>
    <form className="notes-add-form" id="kanbanAddForm" onSubmit={event => { event.preventDefault(); const text = model.drafts.addCard; if (text.trim()) void model.mutate({ op: "todo_add", text, lane: "backlog" }, () => model.updateDrafts(current => current.addCard === text ? { ...current, addCard: "" } : current)); }}><UiIcon name="plus" /><input id="kanbanAddInput" aria-label="Add a card to Backlog" placeholder="Add a card to Backlog…" value={model.drafts.addCard} onChange={event => model.updateDrafts(current => ({ ...current, addCard: event.target.value }))} /><button id="kanbanAddBtn" className="notes-primary" type="submit" disabled={model.busy || !model.drafts.addCard.trim() || model.error?.code === "notes_outcome_unknown"}>Add</button></form>
    <span id="notes-drag-instructions" className="sr-only">Space or Enter picks up. Left and Right choose a column. Space or Enter drops. Escape or Tab cancels. Order follows todos.md, not drop position.</span>
    <DndContext sensors={sensors} collisionDetection={collision} autoScroll={{ enabled: true }} accessibility={{ restoreFocus: false, announcements: dragAnnouncements, screenReaderInstructions: { draggable: "Use Space or Enter to pick up, Left or Right to choose a column, Space or Enter to drop, Escape or Tab to cancel. Tasks stay in file order." } }} cancelDrop={({ active: source }) => {
      const captured = dragSnapshot.current;
      const current = model.todos.find(todo => (todoIdentity(todo)) === source.id);
      return !captured || !current || captured.revision !== current.revision;
    }} onDragStart={event => {
      const task = event.active.data.current?.todo as NotesTodo | undefined;
      if (!task) return;
      keyboardDestination.current = event.activatorEvent.type === "keydown" ? columnFor(task) : null;
      dragSnapshot.current = task; setActive(task); setDestination(columnFor(task)); dragSuppress.current = true;
      announce(`Picked up ${task.text}. Choose Backlog, Doing or Done. Order stays as in todos.md.`);
    }} onDragOver={event => {
      const next = keyboardDestination.current ?? event.over?.data.current?.column as NotesColumn | undefined;
      setDestination(next ?? null);
      if (next && active) announce(`${labels[next]} — ${next === "done" ? "checks the task" : active.done ? "reopens the task" : "keeps the task open"}. File order is preserved.`);
    }} onDragMove={event => {
      const rect = board.current?.getBoundingClientRect();
      const source = event.active.rect.current.translated;
      if (!rect || !source || !board.current) return;
      if (source.right > rect.right - 36) board.current.scrollLeft += 16;
      else if (source.left < rect.left + 36) board.current.scrollLeft -= 16;
    }} onDragCancel={() => { const captured = dragSnapshot.current; dragSnapshot.current = null; keyboardDestination.current = null; if (captured) restore(todoIdentity(captured)); setActive(null); setDestination(null); announce("Move cancelled. Task unchanged."); window.setTimeout(() => { dragSuppress.current = false; }, 0); }} onDragEnd={event => {
      const captured = dragSnapshot.current;
      const collisionColumn = event.over?.data.current?.column as NotesColumn | undefined;
      const keyboardColumn = keyboardDestination.current;
      const to = keyboardColumn ?? collisionColumn;
      dragSnapshot.current = null; keyboardDestination.current = null; setActive(null); setDestination(null);
      window.setTimeout(() => { dragSuppress.current = false; }, 0);
      if (!captured || !board.current?.isConnected || hidden) return;
      const current = model.todos.find(todo => (todoIdentity(todo)) === (todoIdentity(captured)));
      const operation = kanbanDropOperation(captured, current, collisionColumn ?? null, keyboardColumn);
      if (!operation || !to) { announce("Move cancelled or unchanged. File order kept."); restore(todoIdentity(captured)); return; }
      let movedId = todoIdentity(captured);
      announce(`Moving ${captured.text} to ${labels[to]}. File order kept.`);
      void model.mutate(operation, response => { if (response.result.kind === "todo") movedId = todoIdentity(response.result.todo); }).then(success => { announce(success ? `${captured.text} moved to ${labels[to]}. File order kept.` : "Move not confirmed. Current location refreshed; no retry was sent."); restore(movedId); });
    }}>
      <div className="kanban-chips" role="group" aria-label="Jump to column">{columns.map(column => <DropZone column={column} chip key={column} destination={active ? destination : undefined}><button type="button" className="kanban-chip" data-col={column} aria-label={`Jump to ${labels[column]} column`} onClick={() => board.current?.querySelector(`[data-col="${column}"]`)?.scrollIntoView({ block: "nearest", inline: "start" })}>{labels[column]} <span>{tasks.filter(todo => columnFor(todo) === column).length}</span></button></DropZone>)}</div>
      {!tasks.length ? <div className="notes-empty notes-board-empty" id="kanbanEmptyHint"><UiIcon name="grid" /><p>No cards. Add one above or put a todo on the board.</p><button type="button" className="notes-primary" onClick={() => document.getElementById("kanbanAddInput")?.focus()}><UiIcon name="plus" />Add a card</button></div> : null}
      <div className="kanban-board" id="kanbanBoard" ref={board} onScrollCapture={event => {
        const saved = boardPositions.get(scope) ?? { left: 0, columns: { backlog: 0, doing: 0, done: 0 } };
        if (event.target === event.currentTarget) saved.left = event.currentTarget.scrollLeft;
        else if (event.target instanceof HTMLElement && event.target.classList.contains("kanban-card-list")) {
          const column = event.target.closest<HTMLElement>("[data-col]")?.dataset.col;
          if (column === "backlog" || column === "doing" || column === "done") saved.columns[column] = event.target.scrollTop;
        }
        boardPositions.set(scope, saved);
        if (boardPositions.size > 256) { const oldest = boardPositions.keys().next().value; if (oldest !== undefined) boardPositions.delete(oldest); }
      }}>{columns.map(column => {
        const members = tasks.filter(todo => columnFor(todo) === column);
        const before = active ? members.find(todo => todo.line > active.line) : undefined;
        const marker = active && destination === column ? <li className="kanban-insertion">{labels[column]} · {column === "done" ? "Check task" : active.done ? "Reopen task" : "Keep open"} · file order</li> : null;
        return <DropZone column={column} key={column} destination={active ? destination : undefined}><h3><span className="kanban-lane-dot" aria-hidden="true" />{labels[column]} <span className="notes-count">{members.length}</span><button type="button" className="notes-icon-button kanban-lane-add" aria-label={`Add a card from ${labels[column]}`} title="Add a card to Backlog" onClick={() => document.getElementById("kanbanAddInput")?.focus()}><UiIcon name="plus" /></button></h3><ul className="kanban-card-list">{members.map(todo => <Fragment key={todoIdentity(todo)}>{before === todo ? marker : null}<Card todo={todo} model={model} onDetails={(task, opener) => { if (!dragSuppress.current) onDetails(task, opener); }} count={todo.id ? counts[todo.id] : undefined} selected={selectedId === todo.id} dragBusy={Boolean(active)} /></Fragment>)}{!before ? marker : null}{!members.length ? <li className="notes-empty">No cards.</li> : null}</ul></DropZone>;
      })}</div>
      <DragOverlay>{active ? <div className="kanban-drag-ghost">{active.text}<small>{destination ? labels[destination] : "Choose a column"} · file order</small></div> : null}</DragOverlay>
    </DndContext>
    <p className="notes-live-status" role="status">{announcement}</p>
  </div>;
}
