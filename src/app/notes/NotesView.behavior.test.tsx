// @vitest-environment jsdom
import { act } from "react";
import type * as ReactTypes from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { NotesCatalogEntry, NotesComment, NotesRequest, NotesResponse, NotesTargetInfo, NotesTodo } from "../../protocol/generated/v1";
import type { LibrarySpace } from "../library/libraryState";

// Keep the real NotesView, useNotes, TaskDetail and TodoTitle consumers. Only
// CodeMirror/Markdown rendering is replaced; no guard or mutation logic lives here.
vi.mock("./MarkdownEditor", async () => {
  const React = await vi.importActual<typeof ReactTypes>("react");
  return {
    NotesEditorScope: React.createContext(""),
    MarkdownPreview: ({ content }: { content: string }) => <div>{content}</div>,
    MarkdownEditor: ({ label, value, onChange, disabled }: {
      label: string; value: string; onChange(value: string): void; disabled?: boolean;
    }) => <textarea className="cm-content" aria-label={label} value={value} disabled={disabled} onChange={event => onChange(event.target.value)} />,
  };
});
import { NotesView } from "./NotesView";

let root: Root | null = null;
let host: HTMLDivElement | null = null;
let frames: FrameRequestCallback[] = [];
let fixtureSequence = 0;

function problem(code: string, message: string): CockpitClientError {
  return new CockpitClientError("http_error", message, { operationCode: code });
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
interface Fixture {
  client: CockpitClient;
  space: LibrarySpace;
  info: NotesTargetInfo;
  task: NotesTodo;
  notes: Mock<(request: NotesRequest) => Promise<NotesResponse>>;
  resolveTarget: Mock<() => Promise<NotesResponse>>;
  readCatalog: Mock<() => Promise<NotesResponse>>;
  readComments: Mock<(todoId: string) => Promise<NotesResponse>>;
  addComment: Mock<(operation: Extract<NotesRequest["operation"], { op: "comment_add" }>) => Promise<NotesResponse>>;
  addTodo: Mock<(operation: Extract<NotesRequest["operation"], { op: "todo_add" }>) => Promise<NotesResponse>>;
  response(result: NotesResponse["result"], changed?: boolean): NotesResponse;
  replaceTodos(todos: NotesTodo[]): void;
}
function fixture(): Fixture {
  const notesId = `10000000-0000-4000-8000-${(++fixtureSequence).toString(16).padStart(12, "0")}`;
  const space: LibrarySpace = { target: { session_id: "session", space_id: notesId }, label: "Review", live: true };
  const info: NotesTargetInfo = {
    notes_id: notesId, folder: `/data/notes/${notesId}`,
    space: { ...space.target, label: space.label },
    change_tokens: { scratchpad: "scratch-1", todos: "todos-1", decisions: "decisions-1", comments: "comments-1" },
  };
  const task: NotesTodo = { id: "task1", ref: "L1@file1", text: "Review schema", done: false, lane: "doing", revision: "item1", line: 1, depth: 0, problems: [] };
  let todos = [task];
  let comments: NotesComment[] = [];
  const response = (result: NotesResponse["result"], changed = false): NotesResponse => ({ notes_id: notesId, changed, result });
  const resolveTarget = vi.fn(async (): Promise<NotesResponse> => response({ kind: "target", info }));
  const readCatalog = vi.fn(async (): Promise<NotesResponse> => response({ kind: "catalog", entries: [] }));
  const readComments = vi.fn(async (todoId: string): Promise<NotesResponse> => response({ kind: "comments", todo_id: todoId, comments }));
  const addComment = vi.fn(async (operation: Extract<NotesRequest["operation"], { op: "comment_add" }>): Promise<NotesResponse> => {
    const comment: NotesComment = { todo_id: operation.todo_id, comment_id: `20000000-0000-4000-8000-${(comments.length + 1).toString(16).padStart(12, "0")}`, created: null, author: operation.author, body: operation.body, revision: "comment-1" };
    comments = [...comments, comment];
    return response({ kind: "comment", comment }, true);
  });
  const addTodo = vi.fn(async (operation: Extract<NotesRequest["operation"], { op: "todo_add" }>): Promise<NotesResponse> => {
    const added: NotesTodo = { ...task, id: "task2", ref: "L2@file2", line: 2, text: operation.text, lane: operation.lane };
    todos = [...todos, added];
    return response({ kind: "todo", revision: "todos-2", todo: added }, true);
  });
  const notes = vi.fn(async ({ operation }: NotesRequest): Promise<NotesResponse> => {
    switch (operation.op) {
      case "target_resolve": return resolveTarget();
      case "catalog_list": return readCatalog();
      case "scratchpad_read": return response({ kind: "scratchpad", document: { content: "Saved notes", revision: "scratch-1" } });
      case "todo_list": return response({ kind: "todos", revision: "todos-1", todos });
      case "decision_list": return response({ kind: "decisions", decisions: [] });
      case "comment_list": return readComments(operation.todo_id);
      case "comment_add": return addComment(operation);
      case "todo_add": return addTodo(operation);
      default: throw new Error(`Unexpected Notes operation: ${operation.op}`);
    }
  });
  return { client: { notes } as unknown as CockpitClient, space, info, task, notes, resolveTarget, readCatalog, readComments, addComment, addTodo, response, replaceTodos: (next: NotesTodo[]) => { todos = next; } };
}

async function settle(): Promise<void> {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
}
async function mount(current: Fixture): Promise<void> {
  frames = [];
  // jsdom has no layout observer; these tests exercise guards and focus ownership,
  // not title geometry. Match the app's existing test-only native API seam.
  vi.stubGlobal("ResizeObserver", class {
    readonly observe = vi.fn();
    readonly unobserve = vi.fn();
    readonly disconnect = vi.fn();
  });
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frames.push(callback); return frames.length; });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await render(current);
}
async function render(current: Fixture): Promise<void> {
  await act(async () => root!.render(<NotesView client={current.client} space={current.space} onClose={vi.fn()} />));
  await settle();
}
function query<T extends Element>(selector: string): T {
  const element = host!.querySelector<T>(selector);
  if (!element) throw new Error(`Missing element: ${selector}`);
  return element;
}
function findButton(label: string): HTMLButtonElement | undefined {
  return [...host!.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === label || button.getAttribute("aria-label") === label);
}
function button(label: string): HTMLButtonElement {
  const element = findButton(label);
  if (!element) throw new Error(`Missing button: ${label}`);
  return element;
}
async function click(element: HTMLElement): Promise<void> {
  await act(async () => { element.focus(); element.click(); });
  await settle();
}
async function type(selector: string, value: string): Promise<void> {
  const field = query<HTMLInputElement | HTMLTextAreaElement>(selector);
  await act(async () => {
    const prototype = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
function flushFrames(): void {
  act(() => frames.splice(0).forEach(callback => callback(0)));
}
async function openTask(current: Fixture): Promise<void> {
  await click(query<HTMLButtonElement>("#f-todos"));
  await type('#todoAddInput', "Next task draft");
  await click(query<HTMLButtonElement>(`#todoList [data-todo-id="${current.task.id}"] .notes-row-actions button`));
  await type('[aria-label="New comment Markdown"]', "Comment draft to preserve");
}
function expectGuarded(current: Fixture, writes: number): void {
  expect(query<HTMLButtonElement>("#todoAddBtn").disabled).toBe(true);
  expect(query<HTMLButtonElement>("#postCommentBtn").disabled).toBe(true);
  expect(query<HTMLInputElement>("#todoAddInput").value).toBe("Next task draft");
  expect(query<HTMLTextAreaElement>('[aria-label="New comment Markdown"]').value).toBe("Comment draft to preserve");
  expect(current.addComment).toHaveBeenCalledTimes(writes);
  expect(current.addTodo).not.toHaveBeenCalled();
}

afterEach(async () => {
  if (root) await act(async () => root?.unmount());
  root = null;
  host?.remove(); host = null;
  frames = [];
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("NotesView consumer safety", () => {
  it("requires explicit acknowledgment after saved-state reads without replaying an uncertain comment or enabling other writes", async () => {
    const current = fixture();
    current.addComment.mockRejectedValueOnce(new TypeError("Connection lost after sending comment"));
    await mount(current);
    await openTask(current);
    await click(button("Comment"));
    expectGuarded(current, 1);
    expect(findButton("I checked saved state; allow next write")).toBeUndefined();

    const readsBefore = current.readComments.mock.calls.length;
    await click(button("Check saved state"));
    expect(current.readComments.mock.calls.length).toBeGreaterThan(readsBefore);
    expectGuarded(current, 1);
    // Exercise the real submit and keyboard consumers too, not just disabled chrome.
    await act(async () => {
      query<HTMLFormElement>("#notes-todos-panel form").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      query<HTMLTextAreaElement>('[aria-label="New comment Markdown"]').dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
    });
    expectGuarded(current, 1);

    await click(button("I checked saved state; allow next write"));
    expect(query<HTMLButtonElement>("#todoAddBtn").disabled).toBe(false);
    expect(query<HTMLButtonElement>("#postCommentBtn").disabled).toBe(false);
    expect(current.addComment).toHaveBeenCalledTimes(1);
    await click(button("Comment"));
    expect(current.addComment).toHaveBeenCalledTimes(2);
    expect(query<HTMLTextAreaElement>('[aria-label="New comment Markdown"]').value).toBe("");
    expect(host!.querySelectorAll(".notes-comment")).toHaveLength(1);
  });

  it("does not carry read proof into a new uncertain mutation or a remounted consumer, and a failed check supplies no proof", async () => {
    const current = fixture();
    current.addComment.mockRejectedValueOnce(new TypeError("First reply lost"));
    await mount(current);
    await openTask(current);
    await click(button("Comment"));
    await click(button("Check saved state"));
    await click(button("I checked saved state; allow next write"));
    current.addComment.mockRejectedValueOnce(new TypeError("Second reply lost"));
    await click(button("Comment"));
    expectGuarded(current, 2);
    expect(findButton("I checked saved state; allow next write")).toBeUndefined();

    current.readComments.mockRejectedValueOnce(new TypeError("Cannot read saved comments"));
    await click(button("Check saved state"));
    expectGuarded(current, 2);
    expect(findButton("I checked saved state; allow next write")).toBeUndefined();
    await click(button("Check saved state"));
    expect(button("I checked saved state; allow next write").disabled).toBe(false);
    await act(async () => root!.render(null));
    await render(current);
    expectGuarded(current, 2);
    expect(findButton("I checked saved state; allow next write")).toBeUndefined();
    await click(button("Check saved state"));
    expectGuarded(current, 2);
    await click(button("I checked saved state; allow next write"));
    expect(query<HTMLButtonElement>("#postCommentBtn").disabled).toBe(false);
    expect(current.addComment).toHaveBeenCalledTimes(2);
  });

  it("keeps a catalog failure through elapsed time, focus and an already-started unbound Space resolve until an explicit catalog retry", async () => {
    vi.useFakeTimers();
    const current = fixture();
    current.resolveTarget.mockRejectedValue(problem("notes_unbound", "Space has no association"));
    current.readCatalog.mockRejectedValueOnce(problem("notes_io", "Catalog directory cannot be read"));
    await mount(current);
    const pendingResolve = deferred<NotesResponse>();
    current.resolveTarget.mockReturnValueOnce(pendingResolve.promise);
    await act(async () => window.dispatchEvent(new Event("focus")));
    await click(button("Attach existing notes…"));
    expect(button("Attach").disabled).toBe(true);

    await act(async () => {
      pendingResolve.reject(problem("notes_unbound", "Still no association"));
      await vi.advanceTimersByTimeAsync(15_000);
      window.dispatchEvent(new Event("focus"));
    });
    await settle();
    expect(button("Retry reading catalog").disabled).toBe(false);
    expect(button("Attach").disabled).toBe(true);
    expect(current.readCatalog).toHaveBeenCalledTimes(1);

    await click(button("Retry reading catalog"));
    expect(current.readCatalog).toHaveBeenCalledTimes(2);
    expect(host!.querySelector('[role="alert"]')).toBeNull();
  });

  it("refuses attachment when the selected catalog entry disappears on reopening", async () => {
    const current = fixture();
    current.resolveTarget.mockRejectedValue(problem("notes_unbound", "Space has no association"));
    const entry: NotesCatalogEntry = { notes_id: "30000000-0000-4000-8000-000000000002", label: "Available notes", created: null, bound: false };
    current.readCatalog.mockResolvedValueOnce(current.response({ kind: "catalog", entries: [entry] }));
    current.readCatalog.mockResolvedValue(current.response({ kind: "catalog", entries: [] }));
    await mount(current);
    await click(button("Attach existing notes…"));
    await click(query<HTMLInputElement>('input[name="notes-attach"]'));
    expect(button("Attach").disabled).toBe(false);
    await click(button("Cancel"));
    await click(button("Attach existing notes…"));
    expect(host!.querySelector('input[name="notes-attach"]')).toBeNull();
    expect(button("Attach").disabled).toBe(true);
    await click(button("Attach"));
    expect(current.notes.mock.calls.some(([request]) => request.operation.op === "target_attach")).toBe(false);
  });

  it("returns Keep to the current logical Attach action and Cancel to the picker opener without transferring Notes", async () => {
    const current = fixture();
    current.resolveTarget.mockRejectedValue(problem("notes_unbound", "Space has no association"));
    const entry: NotesCatalogEntry = { notes_id: "30000000-0000-4000-8000-000000000001", label: "Other Space", created: null, bound: true };
    current.readCatalog.mockResolvedValue(current.response({ kind: "catalog", entries: [entry] }));
    await mount(current);
    await click(button("Attach existing notes…"));
    await click(query<HTMLInputElement>('input[name="notes-attach"]'));
    await click(button("Attach"));
    await click(button("Keep current association"));
    flushFrames();
    expect(document.activeElement).toBe(button("Attach"));
    expect(button("Attach").disabled).toBe(false);
    await click(button("Cancel"));
    flushFrames();
    expect(document.activeElement).toBe(button("Attach existing notes…"));
    expect(current.notes.mock.calls.some(([request]) => request.operation.op === "target_attach")).toBe(false);
  });

  it("does not let a late detail-close frame steal a new task title's focus after the original row disappears", async () => {
    const current = fixture();
    await mount(current);
    await click(query<HTMLButtonElement>("#f-todos"));
    await click(query<HTMLButtonElement>('#todoList [data-todo-id="task1"] .notes-row-actions button'));
    flushFrames();
    await click(button("Close task details"));
    current.replaceTodos([{ ...current.task, id: "replacement-task", ref: "L1@file2", text: "Newly selected title", revision: "item2" }]);
    await act(async () => window.dispatchEvent(new Event("focus")));
    await settle();
    const title = query<HTMLTextAreaElement>('#todoList [aria-label="Task title: Newly selected title"]');
    title.focus();
    flushFrames();
    expect(document.activeElement).toBe(title);
    expect(query<HTMLInputElement>("#todoAddInput")).not.toBe(document.activeElement);
  });
});
