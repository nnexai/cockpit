import { describe, expect, it } from "vitest";
import type { NotesTodo } from "../../protocol/generated/v1";
import { acknowledgeDraft, changedDraft, reconcileDraft } from "./drafts";
import { kanbanDropOperation, kanbanKeyboardColumn } from "./boardState";
import { notesMutationError, todoIdentity } from "./useNotes";

const todo: NotesTodo = { id: "task1", ref: "L1@file1", text: "Review schema", done: false, lane: "doing", revision: "item1", line: 1, depth: 0, problems: [] };
describe("Notes draft conflict preservation", () => {
  it("keeps typing that arrives during a confirmed save without inventing an external conflict", () => {
    const typedDuringSave = changedDraft("submitted plus more typing", "old saved text", "rev1");
    const acknowledged = acknowledgeDraft(typedDuringSave, "submitted", "rev2", "submitted");
    expect(acknowledged).toEqual({ value: "submitted plus more typing", base: "submitted", revision: "rev2", conflict: false });
    expect(reconcileDraft(acknowledged, "submitted", "rev2")).toEqual(acknowledged);
  });
  it("accepts confirmed writer normalization without inventing a new unsaved edit", () => {
    const local = changedDraft("submitted", "old saved text", "rev1");
    expect(acknowledgeDraft(local, "submitted\n", "rev2", "submitted")).toEqual(changedDraft("submitted\n", "submitted\n", "rev2"));
  });
  it("keeps unsaved text and its original CAS token when the saved document changes", () => {
    const local = { ...changedDraft("saved", "saved", "rev1"), value: "my unsaved text" };
    expect(reconcileDraft(local, "other editor's text", "rev2")).toEqual({ value: "my unsaved text", base: "saved", revision: "rev1", conflict: true });
  });
  it("does not mark an unchanged base as conflicting or discard a dirty draft", () => {
    const local = { ...changedDraft("saved", "saved", "rev1"), value: "typed" };
    expect(reconcileDraft(local, "saved", "rev1")).toEqual(local);
  });
  it("refreshes clean documents while retaining an unresolved dirty conflict across repeated polls", () => {
    expect(reconcileDraft(changedDraft("saved", "saved", "rev1"), "new saved text", "rev2")).toEqual(changedDraft("new saved text", "new saved text", "rev2"));
    const local = { ...changedDraft("typed", "saved", "rev1"), conflict: true };
    expect(reconcileDraft(local, "saved", "rev1").conflict).toBe(true);
  });
});
describe("Notes captured Kanban gesture safety", () => {
  it("refuses a stale move instead of upgrading to an externally changed revision", () => {
    expect(kanbanDropOperation(todo, { ...todo, revision: "item2", done: true }, "backlog")).toBeNull();
  });
  it("refuses source removal, lost membership, changed identity and drops outside a column", () => {
    expect(kanbanDropOperation(todo, undefined, "backlog")).toBeNull();
    expect(kanbanDropOperation(todo, { ...todo, lane: null }, "backlog")).toBeNull();
    expect(kanbanDropOperation(todo, { ...todo, id: "different-task" }, "backlog")).toBeNull();
    expect(kanbanDropOperation(todo, todo, null)).toBeNull();
  });
  it("does not issue same-column moves or introduce a rank even for a checked task", () => {
    expect(kanbanDropOperation(todo, todo, "doing")).toBeNull();
    const completed = { ...todo, done: true };
    expect(kanbanDropOperation(completed, completed, "done")).toBeNull();
  });
  it("drops into the latest keyboard lane before the collision render catches up", () => {
    const destination = kanbanKeyboardColumn("doing", "ArrowRight");
    const expected = { op: "kanban_move", todo: { by: "id", id: todo.id, expected_revision: todo.revision }, to: "done" };
    expect(kanbanDropOperation(todo, todo, "doing", destination)).toEqual(expected);
    expect(kanbanDropOperation(todo, todo, null, destination)).toEqual(expected);
  });
  it("advances repeated rapid keys from their captured intent rather than an old collision", () => {
    const source = { ...todo, lane: "backlog" as const };
    let destination = kanbanKeyboardColumn("backlog", "ArrowRight");
    destination = kanbanKeyboardColumn(destination, "ArrowRight");
    expect(kanbanDropOperation(source, source, "backlog", destination)).toEqual({
      op: "kanban_move", todo: { by: "id", id: source.id, expected_revision: source.revision }, to: "done",
    });
    destination = kanbanKeyboardColumn(destination, "ArrowLeft");
    expect(kanbanDropOperation(source, source, "done", destination)).toEqual({
      op: "kanban_move", todo: { by: "id", id: source.id, expected_revision: source.revision }, to: "doing",
    });
    destination = kanbanKeyboardColumn(destination, "ArrowLeft");
    expect(kanbanDropOperation(source, source, "done", destination)).toBeNull();
  });
  it("clamps keyboard choices at both edges without wrapping or sending same-lane moves", () => {
    const backlog = { ...todo, lane: "backlog" as const };
    const completed = { ...todo, done: true };
    const left = kanbanKeyboardColumn(kanbanKeyboardColumn("backlog", "ArrowLeft"), "ArrowLeft");
    const right = kanbanKeyboardColumn(kanbanKeyboardColumn("done", "ArrowRight"), "ArrowRight");
    expect(left).toBe("backlog");
    expect(right).toBe("done");
    expect(kanbanDropOperation(backlog, backlog, "doing", left)).toBeNull();
    expect(kanbanDropOperation(completed, completed, "doing", right)).toBeNull();
    expect(kanbanDropOperation(todo, todo, "backlog", "doing")).toBeNull();
  });
  it("keeps pointer destinations collision-owned when there is no captured keyboard intent", () => {
    expect(kanbanDropOperation(todo, todo, "backlog", null)).toEqual({
      op: "kanban_move", todo: { by: "id", id: todo.id, expected_revision: todo.revision }, to: "backlog",
    });
    expect(kanbanDropOperation(todo, todo, null, null)).toBeNull();
  });
  it("does not let fresh keyboard intent bypass the captured source safeguards", () => {
    const destination = kanbanKeyboardColumn("doing", "ArrowRight");
    expect(kanbanDropOperation(todo, { ...todo, revision: "item2" }, "doing", destination)).toBeNull();
    expect(kanbanDropOperation(todo, { ...todo, lane: null }, "doing", destination)).toBeNull();
    expect(kanbanDropOperation(todo, undefined, "doing", destination)).toBeNull();
    expect(kanbanDropOperation(todo, { ...todo, id: "different-task" }, "doing", destination)).toBeNull();
  });
});

describe("Notes unconfirmed mutation boundary", () => {
  it("treats a missing reply and transport-level errors as an unknown write outcome", () => {
    expect(notesMutationError(new TypeError("Failed to fetch")).code).toBe("notes_outcome_unknown");
    expect(notesMutationError(Object.assign(new Error("Connection reset after send"), { code: "ECONNRESET" })).code).toBe("notes_outcome_unknown");
  });
  it("keeps an authoritative conflict distinct from an unconfirmed write", () => {
    expect(notesMutationError(Object.assign(new Error("Changed elsewhere"), { operationCode: "notes_conflict" })).code).toBe("notes_conflict");
  });
  it("does not alias duplicate IDs when selecting drafts or a captured gesture", () => {
    const duplicate = { ...todo, problems: ["duplicate_id" as const] };
    expect(todoIdentity(duplicate)).toBe(todo.ref);
    expect(kanbanDropOperation(duplicate, { ...duplicate, ref: "L2@file1", line: 2 }, "done")).toBeNull();
  });
});
