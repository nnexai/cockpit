import type {
  NotesRequest, NotesResponse, NotesTarget, NotesOperation, NotesTodoSelector,
  NotesTodo, NotesDecisionSummary, NotesDecision, NotesComment, NotesTargetInfo, NotesResult,
} from "../protocol/generated/v1";
import { CockpitClientError, validateSessionId, validateResourceId } from "./CockpitClient";

const KiB = 1024;
const MiB = 1024 * KiB;
const utf8 = new TextEncoder();
const fail = (): never => { throw new CockpitClientError("malformed_response", "Invalid Notes request or response"); };
function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return fail();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || keys.some(key => !Object.hasOwn(result, key))) return fail();
  return result;
}
/** Count UTF-8 bytes without silently replacing lone UTF-16 surrogates. */
function text(value: unknown, max = 4096): string {
  if (typeof value !== "string" || value.length > max) return fail();
  let bytes = 0;
  for (let i = 0; i < value.length; i++) {
    const c = value.charCodeAt(i);
    if (c >= 0xd800 && c <= 0xdbff) {
      const next = value.charCodeAt(++i);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return fail();
      bytes += 4;
    } else if (c >= 0xdc00 && c <= 0xdfff) return fail();
    else bytes += c < 0x80 ? 1 : c < 0x800 ? 2 : 3;
    if (bytes > max) return fail();
  }
  return value;
}
const bool = (value: unknown): boolean => typeof value === "boolean" ? value : fail();
const nullable = <T>(value: unknown, parse: (value: unknown) => T): T | null => value === null ? null : parse(value);
const array = <T>(value: unknown, max: number, parse: (value: unknown) => T): T[] => Array.isArray(value) && value.length <= max ? value.map(item => parse(item)) : fail();
const oneOf = <T extends string>(value: unknown, options: readonly T[]): T => typeof value === "string" && options.includes(value as T) ? value as T : fail();
const uint = (value: unknown, min = 0): number => Number.isInteger(value) && (value as number) >= min && (value as number) <= 0xffffffff ? value as number : fail();
const uuid = (value: unknown): string => typeof value === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value) ? value : fail();
const todoId = (value: unknown): string => typeof value === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(value) ? value : fail();
const decisionId = (value: unknown): string => typeof value === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(value) ? value : fail();
const hash = (value: unknown): string => typeof value === "string" && /^sha256:[0-9a-f]{64}$/.test(value) ? value : fail();
const revision = (value: unknown): string => value === "absent" ? value : hash(value);
/** Metadata change tokens are compared for equality, never used as CAS revisions. */
const changeToken = (value: unknown): string => {
  const token = text(value, 256);
  return token.length > 0 ? token : fail();
};
const reference = (value: unknown): string => {
  const v = text(value, 96);
  const match = /^L([1-9][0-9]*)@(sha256:[0-9a-f]{64})$/.exec(v);
  return match && Number(match[1]) <= 0xffffffff ? v : fail();
};
const lane = (value: unknown) => oneOf(value, ["backlog", "doing"] as const);
function absolutePath(value: unknown): string {
  const v = text(value);
  if (!v || /[\x00-\x1f\x7f]/.test(v) || !(v.startsWith("/") || /^[A-Za-z]:[\\/]/.test(v)) || v.split(/[\\/]/).some(part => part === "." || part === "..")) return fail();
  return v;
}
function todoText(value: unknown): string {
  const v = text(value, 2 * KiB);
  return v.includes("<!--") || v.includes("-->") ? fail() : v;
}
function author(value: unknown): string {
  const v = text(value, 128);
  return /[\x00-\x1f\x7f-\x9f]/.test(v) ? fail() : v;
}
function title(value: unknown): string {
  const v = text(value, 512);
  return !v.trim() || /[\x00-\x1f\x7f-\x9f]/.test(v) ? fail() : v;
}
function decided(value: unknown): string {
  const v = text(value, 128);
  const date = /^(\d{4})-(\d{2})-(\d{2})(?:$|T)/.exec(v);
  if (!date || (!/^\d{4}-\d{2}-\d{2}$/.test(v) && !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(v))) return fail();
  const year = Number(date[1]), month = Number(date[2]), day = Number(date[3]);
  const days = [31, year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (month < 1 || month > 12 || day < 1 || day > days[month - 1]! || (v.length > 10 && (!Number.isFinite(Date.parse(v)) || Number(v.slice(11, 13)) > 23 || Number(v.slice(14, 16)) > 59 || Number(v.slice(17, 19)) > 59))) return fail();
  return v;
}
function target(value: unknown): NotesTarget {
  const kind = (value as Record<string, unknown> | null)?.kind;
  switch (kind) {
    case "root": record(value, ["kind"]); return { kind };
    case "notes": { const r = record(value, ["kind", "notes_id"]); return { kind, notes_id: uuid(r.notes_id) }; }
    case "space": { const r = record(value, ["kind", "session_id", "space_id"]); return { kind, session_id: validateSessionId(text(r.session_id, 96)), space_id: validateResourceId(text(r.space_id, 128)) }; }
    default: return fail();
  }
}
function selector(value: unknown): NotesTodoSelector {
  const by = (value as Record<string, unknown> | null)?.by;
  if (by === "id") { const r = record(value, ["by", "id", "expected_revision"]); return { by, id: todoId(r.id), expected_revision: hash(r.expected_revision) }; }
  if (by === "ref") { const r = record(value, ["by", "ref"]); return { by, ref: reference(r.ref) }; }
  return fail();
}
function operation(value: unknown): NotesOperation {
  const op = (value as Record<string, unknown> | null)?.op;
  switch (op) {
    case "catalog_list": case "target_resolve": case "target_create": case "scratchpad_read": case "kanban_list":
      record(value, ["op"]); return { op };
    case "target_attach": { const r = record(value, ["op", "notes_id"]); return { op, notes_id: uuid(r.notes_id) }; }
    case "scratchpad_append": { const r = record(value, ["op", "text", "expected_revision"]); return { op, text: text(r.text, MiB), expected_revision: nullable(r.expected_revision, revision) }; }
    case "scratchpad_replace": { const r = record(value, ["op", "content", "expected_revision"]); return { op, content: text(r.content, MiB), expected_revision: revision(r.expected_revision) }; }
    case "todo_list": { const r = record(value, ["op", "filter"]); return { op, filter: oneOf(r.filter, ["all", "open", "done"] as const) }; }
    case "todo_add": { const r = record(value, ["op", "text", "lane"]); return { op, text: todoText(r.text), lane: nullable(r.lane, lane) }; }
    case "todo_update": { const r = record(value, ["op", "todo", "text"]); return { op, todo: selector(r.todo), text: nullable(r.text, todoText) }; }
    case "todo_set_done": { const r = record(value, ["op", "todo", "done"]); return { op, todo: selector(r.todo), done: bool(r.done) }; }
    case "todo_remove": case "kanban_promote": case "kanban_unboard": { const r = record(value, ["op", "todo"]); return { op, todo: selector(r.todo) }; }
    case "kanban_move": { const r = record(value, ["op", "todo", "to"]); return { op, todo: selector(r.todo), to: oneOf(r.to, ["backlog", "doing", "done"] as const) }; }
    case "decision_list": { const r = record(value, ["op", "status", "query"]); return { op, status: oneOf(r.status, ["current", "history", "all"] as const), query: nullable(r.query, text) }; }
    case "decision_get": { const r = record(value, ["op", "decision_id"]); return { op, decision_id: decisionId(r.decision_id) }; }
    case "decision_create": { const r = record(value, ["op", "title", "body", "decided"]); return { op, title: title(r.title), body: text(r.body, 256 * KiB), decided: nullable(r.decided, decided) }; }
    case "decision_update": { const r = record(value, ["op", "decision_id", "expected_revision", "title", "body"]); return { op, decision_id: decisionId(r.decision_id), expected_revision: hash(r.expected_revision), title: nullable(r.title, title), body: nullable(r.body, v => text(v, 256 * KiB)) }; }
    case "decision_replace": { const r = record(value, ["op", "decision_id", "expected_revision", "title", "body", "decided"]); return { op, decision_id: decisionId(r.decision_id), expected_revision: hash(r.expected_revision), title: title(r.title), body: text(r.body, 256 * KiB), decided: nullable(r.decided, decided) }; }
    case "comment_list": { const r = record(value, ["op", "todo_id"]); return { op, todo_id: todoId(r.todo_id) }; }
    case "comment_get": { const r = record(value, ["op", "todo_id", "comment_id"]); return { op, todo_id: todoId(r.todo_id), comment_id: uuid(r.comment_id) }; }
    case "comment_add": { const r = record(value, ["op", "todo_id", "body", "author"]); return { op, todo_id: todoId(r.todo_id), body: text(r.body, 64 * KiB), author: nullable(r.author, author) }; }
    case "comment_update": { const r = record(value, ["op", "todo_id", "comment_id", "expected_revision", "body"]); return { op, todo_id: todoId(r.todo_id), comment_id: uuid(r.comment_id), expected_revision: hash(r.expected_revision), body: text(r.body, 64 * KiB) }; }
    case "comment_remove": { const r = record(value, ["op", "todo_id", "comment_id", "expected_revision"]); return { op, todo_id: todoId(r.todo_id), comment_id: uuid(r.comment_id), expected_revision: hash(r.expected_revision) }; }
    default: return fail();
  }
}
export function parseNotesRequest(value: unknown): NotesRequest {
  const r = record(value, ["target", "operation"]);
  const request = { target: target(r.target), operation: operation(r.operation) };
  const op = request.operation.op;
  if (op === "catalog_list" ? request.target.kind !== "root"
    : op === "target_create" || op === "target_attach" ? request.target.kind !== "space"
    : op === "target_resolve" ? request.target.kind === "root" : request.target.kind !== "notes") return fail();
  text(JSON.stringify(request), 4 * MiB);
  return request;
}
function todo(value: unknown): NotesTodo {
  const r = record(value, ["id", "ref", "text", "done", "lane", "revision", "line", "depth", "problems"]);
  const result: NotesTodo = { id: nullable(r.id, todoId), ref: reference(r.ref), text: text(r.text, 2 * KiB), done: bool(r.done), lane: nullable(r.lane, lane), revision: hash(r.revision), line: uint(r.line, 1), depth: uint(r.depth), problems: array(r.problems, 4, v => oneOf(v, ["duplicate_id", "unknown_lane", "metadata_malformed", "lazy_continuation"] as const)) };
  if (!result.ref.startsWith(`L${result.line}@`) || new Set(result.problems).size !== result.problems.length) return fail();
  return result;
}
function summary(value: unknown): NotesDecisionSummary {
  const r = record(value, ["decision_id", "title", "recorded", "decided", "replaces", "replaced_by", "status", "revision", "problems"]);
  return { decision_id: decisionId(r.decision_id), title: text(r.title, 512), recorded: nullable(r.recorded, text), decided: nullable(r.decided, text), replaces: nullable(r.replaces, decisionId), replaced_by: array(r.replaced_by, 4096, decisionId), status: oneOf(r.status, ["current", "replaced"] as const), revision: hash(r.revision), problems: array(r.problems, 256, text) };
}
function decision(value: unknown): NotesDecision {
  const r = record(value, ["summary", "body", "relative_path", "path"]);
  const s = summary(r.summary), relative_path = text(r.relative_path, 256), body = text(r.body, 256 * KiB);
  if (relative_path !== `decisions/${s.decision_id}.md` || utf8.encode(s.title).byteLength + utf8.encode(body).byteLength > 256 * KiB) return fail();
  return { summary: s, body, relative_path, path: absolutePath(r.path) };
}
function comment(value: unknown): NotesComment {
  const r = record(value, ["todo_id", "comment_id", "created", "author", "body", "revision"]);
  return { todo_id: todoId(r.todo_id), comment_id: uuid(r.comment_id), created: nullable(r.created, text), author: nullable(r.author, author), body: text(r.body, 64 * KiB), revision: hash(r.revision) };
}
function info(value: unknown): NotesTargetInfo {
  const r = record(value, ["notes_id", "folder", "space", "change_tokens"]);
  const tokens = record(r.change_tokens, ["scratchpad", "todos", "decisions", "comments"]);
  return { notes_id: uuid(r.notes_id), folder: absolutePath(r.folder), space: nullable(r.space, v => { const s = record(v, ["session_id", "space_id", "label"]); return { session_id: validateSessionId(text(s.session_id, 96)), space_id: validateResourceId(text(s.space_id, 128)), label: text(s.label) }; }), change_tokens: { scratchpad: changeToken(tokens.scratchpad), todos: changeToken(tokens.todos), decisions: changeToken(tokens.decisions), comments: changeToken(tokens.comments) } };
}
function unique<T>(values: T[], key: (value: T) => string): T[] {
  return new Set(values.map(key)).size === values.length ? values : fail();
}
function todos(value: unknown, fileRevision: string): NotesTodo[] {
  const result = array(value, 5000, todo);
  if (result.some(item => item.ref !== `L${item.line}@${fileRevision}`) || result.reduce((size, item) => size + utf8.encode(item.text).byteLength, 0) > MiB) return fail();
  return unique(result, item => item.ref);
}
function result(value: unknown): NotesResult {
  const kind = (value as Record<string, unknown> | null)?.kind;
  switch (kind) {
    case "catalog": { const r = record(value, ["kind", "entries"]); return { kind, entries: unique(array(r.entries, 4096, v => { const e = record(v, ["notes_id", "label", "created", "bound"]); return { notes_id: uuid(e.notes_id), label: nullable(e.label, text), created: nullable(e.created, text), bound: bool(e.bound) }; }), e => e.notes_id) }; }
    case "target": { const r = record(value, ["kind", "info"]); return { kind, info: info(r.info) }; }
    case "scratchpad": { const r = record(value, ["kind", "document"]), d = record(r.document, ["content", "revision"]); const document = { content: text(d.content, MiB), revision: revision(d.revision) }; if (document.revision === "absent" && document.content !== "") return fail(); return { kind, document }; }
    case "todos": { const r = record(value, ["kind", "revision", "todos"]), rev = revision(r.revision); return { kind, revision: rev, todos: todos(r.todos, rev) }; }
    case "todo": { const r = record(value, ["kind", "revision", "todo"]), rev = hash(r.revision), item = todo(r.todo); if (item.ref !== `L${item.line}@${rev}`) return fail(); return { kind, revision: rev, todo: item }; }
    case "todo_removed": { const r = record(value, ["kind", "revision"]); return { kind, revision: hash(r.revision) }; }
    case "board": {
      const r = record(value, ["kind", "revision", "columns"]), rev = revision(r.revision), c = record(r.columns, ["backlog", "doing", "done"]);
      const columns = { backlog: todos(c.backlog, rev), doing: todos(c.doing, rev), done: todos(c.done, rev) };
      const all = [...columns.backlog, ...columns.doing, ...columns.done];
      if (all.length > 5000 || all.reduce((size, t) => size + utf8.encode(t.text).byteLength, 0) > MiB || columns.backlog.some(t => t.done || t.lane !== "backlog") || columns.doing.some(t => t.done || t.lane !== "doing") || columns.done.some(t => !t.done || t.lane === null)) return fail();
      unique(all, t => t.ref);
      return { kind, revision: rev, columns };
    }
    case "decisions": { const r = record(value, ["kind", "decisions"]); return { kind, decisions: unique(array(r.decisions, 4096, summary), d => d.decision_id) }; }
    case "decision": { const r = record(value, ["kind", "decision"]); return { kind, decision: decision(r.decision) }; }
    case "comments": { const r = record(value, ["kind", "todo_id", "comments"]), todo_id = todoId(r.todo_id), comments = unique(array(r.comments, 1000, comment), c => c.comment_id); if (comments.some(c => c.todo_id !== todo_id) || comments.reduce((size, c) => size + utf8.encode(c.body).byteLength, 0) > 16 * MiB) return fail(); return { kind, todo_id, comments }; }
    case "comment": { const r = record(value, ["kind", "comment"]); return { kind, comment: comment(r.comment) }; }
    case "comment_removed": { const r = record(value, ["kind", "todo_id", "comment_id"]); return { kind, todo_id: todoId(r.todo_id), comment_id: uuid(r.comment_id) }; }
    default: return fail();
  }
}
export function parseNotesResponse(value: unknown): NotesResponse {
  const r = record(value, ["notes_id", "changed", "result"]);
  const response = { notes_id: nullable(r.notes_id, uuid), changed: bool(r.changed), result: result(r.result) };
  if (response.result.kind === "catalog" ? response.notes_id !== null || response.changed
    : response.notes_id === null || (response.result.kind === "target" && response.result.info.notes_id !== response.notes_id)) return fail();
  return response;
}
export function matchNotesResponse(value: NotesResponse, request: NotesRequest): NotesResponse {
  const { target, operation: op } = request, r = value.result;
  if (target.kind === "notes" && value.notes_id !== target.notes_id) return fail();
  switch (op.op) {
    case "catalog_list": if (r.kind !== "catalog") return fail(); break;
    case "target_resolve": case "target_create": case "target_attach":
      if (r.kind !== "target" || (target.kind === "space" && (r.info.space?.session_id !== target.session_id || r.info.space.space_id !== target.space_id)) || (op.op === "target_attach" && value.notes_id !== op.notes_id)) return fail(); break;
    case "scratchpad_read": case "scratchpad_append": case "scratchpad_replace": if (r.kind !== "scratchpad") return fail(); break;
    case "todo_list": if (r.kind !== "todos" || r.todos.some(t => op.filter === "open" ? t.done : op.filter === "done" ? !t.done : false)) return fail(); break;
    case "todo_add": if (op.text.trim() === "" ? r.kind !== "todos" || value.changed : r.kind !== "todo") return fail(); break;
    case "todo_remove": if (r.kind !== "todo_removed") return fail(); break;
    case "todo_update": case "todo_set_done": case "kanban_promote": case "kanban_move": case "kanban_unboard":
      if (r.kind !== "todo" || (op.todo.by === "id" ? r.todo.id !== op.todo.id : r.todo.line !== Number(op.todo.ref.slice(1, op.todo.ref.indexOf("@"))))) return fail();
      if (op.op === "todo_set_done" && r.todo.done !== op.done) return fail();
      if (op.op === "kanban_promote" && r.todo.lane === null) return fail();
      if (op.op === "kanban_unboard" && r.todo.lane !== null) return fail();
      if (op.op === "kanban_move" && (op.to === "done" ? !r.todo.done || r.todo.lane === null : r.todo.done || r.todo.lane !== op.to)) return fail();
      break;
    case "kanban_list": if (r.kind !== "board") return fail(); break;
    case "decision_list": if (r.kind !== "decisions" || r.decisions.some(d => op.status === "current" ? d.status !== "current" : op.status === "history" ? d.status !== "replaced" : false)) return fail(); break;
    case "decision_get": case "decision_update": if (r.kind !== "decision" || r.decision.summary.decision_id !== op.decision_id) return fail(); break;
    case "decision_create": if (r.kind !== "decision") return fail(); break;
    case "decision_replace": if (r.kind !== "decision" || r.decision.summary.replaces !== op.decision_id || r.decision.summary.decision_id === op.decision_id) return fail(); break;
    case "comment_list": if (r.kind !== "comments" || r.todo_id !== op.todo_id) return fail(); break;
    case "comment_get": case "comment_add": case "comment_update": if (r.kind !== "comment" || r.comment.todo_id !== op.todo_id || (op.op !== "comment_add" && r.comment.comment_id !== op.comment_id)) return fail(); break;
    case "comment_remove": if (r.kind !== "comment_removed" || r.todo_id !== op.todo_id || r.comment_id !== op.comment_id) return fail(); break;
  }
  if (["catalog_list", "target_resolve", "scratchpad_read", "todo_list", "kanban_list", "decision_list", "decision_get", "comment_list", "comment_get"].includes(op.op) && value.changed) return fail();
  return value;
}
