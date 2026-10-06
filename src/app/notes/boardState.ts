import type { NotesColumn, NotesOperation, NotesTodo } from "../../protocol/generated/v1";
import { todoIdentity, todoSelector } from "./useNotes"

/** Keyboard lane choice advances from the last key, not a rendered collision. */
export function kanbanKeyboardColumn(current: NotesColumn, direction: "ArrowLeft" | "ArrowRight"): NotesColumn {
  return direction === "ArrowRight" ? current === "backlog" ? "doing" : "done" : current === "done" ? "doing" : "backlog";
}

/** A gesture never silently upgrades its captured CAS revision or invents a rank. */
export function kanbanDropOperation(captured: NotesTodo, current: NotesTodo | undefined, collisionColumn: NotesColumn | null, keyboardColumn: NotesColumn | null = null): NotesOperation | null {
  // Keyboard intent is synchronous; pointer drops still require a real collision.
  const destination = keyboardColumn ?? collisionColumn;
  if (!destination || !current || current.lane === null || captured.revision !== current.revision || (todoIdentity(captured)) !== (todoIdentity(current))) return null;
  if ((captured.done ? "done" : captured.lane) === destination) return null;
  return { op: "kanban_move", todo: todoSelector(captured), to: destination };
}
