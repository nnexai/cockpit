import { createContext, useContext, useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { NotesTodo } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { todoIdentity, todoSelector, type NotesModel } from "./useNotes";

type Surface = "todos" | "kanban" | "detail";
type Pending = { owner: string; revision: string; selector: ReturnType<typeof todoSelector>; text: string; opener: HTMLElement };
type DeleteContext = { model: NotesModel; pending: Pending | null; setPending(value: Pending | null): void; removed(text: string): void; alive: { current: boolean } };
const TodoDeletion = createContext<DeleteContext | null>(null);

export function TodoDeleteProvider({ model, surface, children }: { model: NotesModel; surface: string; children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const [announcement, announce] = useState("");
  const alive = useRef(false);
  useLayoutEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => { setPending(null); }, [surface]);
  return <TodoDeletion.Provider value={{ model, pending, setPending, removed: text => announce(`Deleted todo “${text}”. Comments kept.`), alive }}>{children}<p className="notes-live-status" role="status">{announcement}</p></TodoDeletion.Provider>;
}

export function useTodoDelete(todo: NotesTodo, surface: Surface) {
  const context = useContext(TodoDeletion);
  if (!context) throw new Error("Todo deletion needs its Notes surface");
  const { model, pending, setPending } = context;
  const owner = `${surface}/${todoIdentity(todo)}`;
  const open = pending?.owner === owner;
  const malformed = todo.problems.includes("metadata_malformed") || todo.problems.includes("lazy_continuation");
  const unknown = model.error?.code === "notes_outcome_unknown";
  const blocked = model.busy || unknown || malformed;
  const changed = open && (pending.revision !== todo.revision || (pending.selector.by === "ref" && pending.selector.ref !== todo.ref));
  const panelId = `notes-todo-delete-${surface}-${encodeURIComponent(todoIdentity(todo))}`;
  const cancel = () => {
    if (!open) return;
    const opener = pending.opener;
    const root = opener.closest(".notes-view");
    const origin = root?.querySelector<HTMLElement>(`[id="${panelId}"]`);
    setPending(null);
    requestAnimationFrame(() => {
      const focused = document.activeElement;
      if (!context.alive.current || !opener.isConnected || opener.closest("[hidden], [inert]") || (focused !== document.body && focused !== opener && !origin?.contains(focused))) return;
      opener.focus({ preventScroll: true });
    });
  };
  const begin = (opener: HTMLElement) => {
    if (blocked) return;
    if (open) { cancel(); return; }
    setPending({ owner, revision: todo.revision, selector: todoSelector(todo), text: todo.text, opener });
  };
  const confirm = async () => {
    if (!open || blocked || changed) return;
    const captured = pending;
    const root = captured.opener.closest<HTMLElement>(".notes-view");
    const origin = captured.opener.closest<HTMLElement>(surface === "detail" ? ".notes-task-summary" : "[data-todo-id]");
    const selector = surface === "kanban" ? ".kanban-card" : ".todo-row";
    const rows = Array.from(root?.querySelectorAll<HTMLElement>(selector) ?? []).filter(row => !row.closest("[hidden]"));
    const index = origin ? rows.indexOf(origin) : -1;
    const adjacent = [rows[index + 1], rows[index - 1]];
    const success = await model.mutate({ op: "todo_remove", todo: captured.selector });
    if (!context.alive.current) return;
    setPending(null);
    if (success) context.removed(captured.text);
    requestAnimationFrame(() => {
      const focused = document.activeElement;
      if (!context.alive.current || !root?.isConnected || (focused !== document.body && !origin?.contains(focused))) return;
      const target = success
        ? surface === "detail" ? root.querySelector<HTMLElement>("#notes-task-detail-title") : adjacent.find(row => row?.isConnected)?.querySelector<HTMLElement>('input[type="checkbox"]') ?? root.querySelector<HTMLElement>(surface === "kanban" ? "#kanbanAddInput" : "#todoAddInput")
        : captured.opener.isConnected ? captured.opener : root.querySelector<HTMLElement>(surface === "detail" ? "#notes-task-detail-title" : surface === "kanban" ? "#kanbanAddInput" : "#todoAddInput");
      if (!target || target.closest("[hidden], [inert]")) return;
      target.focus({ preventScroll: true });
    });
  };
  return { open, begin, cancel, confirm, changed, blocked, busy: model.busy, panelId, malformed, model };
}

export function TodoDeleteTrigger({ todo, surface, disabled = false }: { todo: NotesTodo; surface: Surface; disabled?: boolean }) {
  const state = useTodoDelete(todo, surface);
  return <button type="button" className={`notes-todo-delete${surface === "todos" ? " notes-icon-button" : ""}`} aria-label={surface === "todos" ? `Delete todo: ${todo.text}` : undefined} title={state.malformed ? "Repair this item in todos.md first" : "Delete todo…"} aria-expanded={state.open} aria-controls={state.panelId} disabled={state.blocked || disabled} onClick={event => state.begin(event.currentTarget)}><UiIcon name="trash" />{surface !== "todos" ? surface === "kanban" ? "Delete todo…" : "Delete todo" : null}</button>;
}

export function TodoDeletePanel({ todo, surface }: { todo: NotesTodo; surface: Surface }) {
  const state = useTodoDelete(todo, surface);
  const labelId = useId();
  const bodyId = useId();
  const draft = state.model.drafts.todos[todoIdentity(todo)];
  if (!state.open) return null;
  return <div id={state.panelId} className="notes-delete-confirm" role="group" aria-label="Delete this todo?" aria-labelledby={labelId} aria-describedby={bodyId} onClick={event => event.stopPropagation()} onKeyDown={event => {
    if (event.key !== "Escape" || event.nativeEvent.isComposing) return;
    event.preventDefault(); event.stopPropagation();
    if (!state.busy) state.cancel();
  }}>
    <strong id={labelId}>Delete this todo?</strong>
    <p className="notes-delete-title" title={todo.text}>“{todo.text}”</p>
    <p id={bodyId}>{state.changed ? "Changed elsewhere. Close this confirmation and reopen it to review the current todo before deleting." : "Removes the task from todos.md. Its comments stay saved. Cannot be undone from Cockpit."}</p>
    {draft && draft.value !== draft.base ? <p>Your unsaved title edit moves to Retained drafts.</p> : null}
    <div className="notes-delete-confirm-actions"><button type="button" autoFocus disabled={state.busy} onClick={state.cancel}>Keep</button><button type="button" className="notes-delete-danger" disabled={state.blocked || state.changed} onClick={() => void state.confirm()}>{state.busy ? "Deleting…" : "Delete todo"}</button></div>
  </div>;
}

export function TodoDeleteRow({ todo, selected, children }: { todo: NotesTodo; selected: boolean; children: ReactNode }) {
  const state = useTodoDelete(todo, "todos");
  return <li className={`todo-row${selected ? " is-selected" : ""}${state.open ? " is-confirming-delete" : ""}`} data-todo-id={todoIdentity(todo)} data-depth={Math.min(todo.depth, 6)} style={{ paddingInlineStart: `${8 + Math.min(todo.depth, 6) * 12}px` }}>{children}<TodoDeletePanel todo={todo} surface="todos" /></li>;
}
