import type { NotesRequest, NotesResponse, NotesTodo } from "../protocol/generated/v1";
import { wireNotesRequest, wireNotesResponse, type TypedWirePolicy } from "../protocol/generated/validate";
import { definePolicy, parseWire, constantMessage } from "./wire";
import { CockpitClientError, validateSessionId, validateResourceId } from "./CockpitClient";

const KiB = 1024, MiB = 1024 * KiB;
const utf8 = new TextEncoder();
const fail = (): never => { throw new CockpitClientError("malformed_response", "Invalid Notes request or response"); };
/** Count UTF-8 bytes without silently replacing lone UTF-16 surrogates. */
function text(value: string, max = 4096): boolean {
  if (value.length > max) return false;
  let bytes = 0;
  for (let i = 0; i < value.length; i++) {
    const c = value.charCodeAt(i);
    if (c >= 0xd800 && c <= 0xdbff) {
      const next = value.charCodeAt(++i);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
      bytes += 4;
    } else if (c >= 0xdc00 && c <= 0xdfff) return false;
    else bytes += c < 0x80 ? 1 : c < 0x800 ? 2 : 3;
    if (bytes > max) return false;
  }
  return true;
}
const bound = (max: number) => (value: string) => text(value, max);
const nullable = (check: (value: string) => boolean) => (value: string | null) => value === null || check(value);
const uuid = (value: string) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value);
const todoId = (value: string) => /^[A-Za-z0-9_-]{1,64}$/.test(value);
const decisionId = (value: string) => /^[A-Za-z0-9_-]{1,128}$/.test(value);
const hash = (value: string) => /^sha256:[0-9a-f]{64}$/.test(value);
const revision = (value: string) => value === "absent" || hash(value);
const changeToken = (value: string) => value.length > 0 && text(value, 256);
const sessionId = (value: string) => text(value, 96) && Boolean(validateSessionId(value));
const resourceId = (value: string) => text(value, 128) && Boolean(validateResourceId(value));
const reference = (value: string) => {
  const match = /^L([1-9][0-9]*)@(sha256:[0-9a-f]{64})$/.exec(value);
  return text(value, 96) && match !== null && Number(match[1]) <= 0xffffffff;
};
const absolutePath = (value: string) => text(value) && value.length > 0 && !/[\x00-\x1f\x7f]/.test(value)
  && (value.startsWith("/") || /^[A-Za-z]:[\\/]/.test(value)) && !value.split(/[\\/]/).some(part => part === "." || part === "..");
const todoText = (value: string) => text(value, 2 * KiB) && !value.includes("<!--") && !value.includes("-->");
const author = (value: string) => text(value, 128) && !/[\x00-\x1f\x7f-\x9f]/.test(value);
const title = (value: string) => text(value, 512) && value.trim().length > 0 && !/[\x00-\x1f\x7f-\x9f]/.test(value);
function decided(value: string): boolean {
  const date = /^(\d{4})-(\d{2})-(\d{2})(?:$|T)/.exec(value);
  if (!text(value, 128) || !date || (!/^\d{4}-\d{2}-\d{2}$/.test(value) && !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(value))) return false;
  const year = Number(date[1]), month = Number(date[2]), day = Number(date[3]);
  const days = [31, year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return month >= 1 && month <= 12 && day >= 1 && day <= days[month - 1]! && (value.length <= 10 || (Number.isFinite(Date.parse(value)) && Number(value.slice(11, 13)) <= 23 && Number(value.slice(14, 16)) <= 59 && Number(value.slice(17, 19)) <= 59));
}
const unique = <T>(values: readonly T[], key: (value: T) => string) => new Set(values.map(key)).size === values.length;
const totalBytes = <T>(values: readonly T[], content: (value: T) => string) => values.reduce((size, value) => size + utf8.encode(content(value)).byteLength, 0);
const todos = (values: readonly NotesTodo[], rev: string) => unique(values, t => t.ref)
  && values.every(t => t.ref === `L${t.line}@${rev}`) && totalBytes(values, t => t.text) <= MiB;

const wire: TypedWirePolicy = {
  complete: new Set(["NotesRequest", "NotesResponse", "NotesTarget", "NotesTodoSelector", "NotesOperation", "NotesResult", "NotesCatalogEntry", "NotesSpaceInfo", "NotesChangeTokens", "NotesTargetInfo", "NotesDocument", "NotesTodo", "NotesBoard", "NotesDecisionSummary", "NotesDecision", "NotesComment"]),
  order: {
    NotesRequest: ["keys", "target", "operation"], NotesResponse: ["keys", "notes_id", "changed", "result"],
    NotesTargetInfo: ["keys", "change_tokens:keys", "notes_id", "folder", "space", "change_tokens"],
    NotesSpaceInfo: ["keys", "session_id", "space_id", "label"],
    "NotesTarget[space]": ["keys", "session_id", "space_id"],
  },
  lengths: {
    "NotesTodo.problems": { max: 4 }, "NotesDecisionSummary.replaced_by": { max: 4096 }, "NotesDecisionSummary.problems": { max: 256 },
    "NotesResult[catalog].entries": { max: 4096 }, "NotesResult[todos].todos": { max: 5000 }, "NotesResult[decisions].decisions": { max: 4096 }, "NotesResult[comments].comments": { max: 1000 },
    "NotesBoard.backlog": { max: 5000 }, "NotesBoard.doing": { max: 5000 }, "NotesBoard.done": { max: 5000 },
  },
  fields: {
    "NotesTarget[notes].notes_id": uuid, "NotesTarget[space].session_id": sessionId, "NotesTarget[space].space_id": resourceId,
    "NotesTodoSelector[id].id": todoId, "NotesTodoSelector[id].expected_revision": hash, "NotesTodoSelector[ref].ref": reference,
    "NotesOperation[target_attach].notes_id": uuid, "NotesOperation[scratchpad_append].text": bound(MiB), "NotesOperation[scratchpad_append].expected_revision": nullable(revision),
    "NotesOperation[scratchpad_replace].content": bound(MiB), "NotesOperation[scratchpad_replace].expected_revision": revision,
    "NotesOperation[todo_add].text": todoText, "NotesOperation[todo_update].text": nullable(todoText), "NotesOperation[decision_list].query": nullable(text),
    "NotesOperation[decision_get].decision_id": decisionId,
    "NotesOperation[decision_create].title": title, "NotesOperation[decision_create].body": bound(256 * KiB), "NotesOperation[decision_create].decided": nullable(decided),
    "NotesOperation[decision_update].decision_id": decisionId, "NotesOperation[decision_update].expected_revision": hash, "NotesOperation[decision_update].title": nullable(title), "NotesOperation[decision_update].body": nullable(bound(256 * KiB)),
    "NotesOperation[decision_replace].decision_id": decisionId, "NotesOperation[decision_replace].expected_revision": hash, "NotesOperation[decision_replace].title": title, "NotesOperation[decision_replace].body": bound(256 * KiB), "NotesOperation[decision_replace].decided": nullable(decided),
    "NotesOperation[comment_list].todo_id": todoId, "NotesOperation[comment_get].todo_id": todoId, "NotesOperation[comment_get].comment_id": uuid,
    "NotesOperation[comment_add].todo_id": todoId, "NotesOperation[comment_add].body": bound(64 * KiB), "NotesOperation[comment_add].author": nullable(author),
    "NotesOperation[comment_update].todo_id": todoId, "NotesOperation[comment_update].comment_id": uuid, "NotesOperation[comment_update].expected_revision": hash, "NotesOperation[comment_update].body": bound(64 * KiB),
    "NotesOperation[comment_remove].todo_id": todoId, "NotesOperation[comment_remove].comment_id": uuid, "NotesOperation[comment_remove].expected_revision": hash,
    "NotesCatalogEntry.notes_id": uuid, "NotesCatalogEntry.label": nullable(text), "NotesCatalogEntry.created": nullable(text),
    "NotesSpaceInfo.session_id": sessionId, "NotesSpaceInfo.space_id": resourceId, "NotesSpaceInfo.label": value => text(value),
    "NotesChangeTokens.scratchpad": changeToken, "NotesChangeTokens.todos": changeToken, "NotesChangeTokens.decisions": changeToken, "NotesChangeTokens.comments": changeToken,
    "NotesTargetInfo.notes_id": uuid, "NotesTargetInfo.folder": absolutePath, "NotesDocument.content": bound(MiB), "NotesDocument.revision": revision,
    "NotesTodo.id": nullable(todoId), "NotesTodo.ref": reference, "NotesTodo.text": bound(2 * KiB), "NotesTodo.revision": hash, "NotesTodo.line": value => value >= 1,
    "NotesDecisionSummary.decision_id": decisionId, "NotesDecisionSummary.title": bound(512), "NotesDecisionSummary.recorded": nullable(text), "NotesDecisionSummary.decided": nullable(text),
    "NotesDecisionSummary.replaces": nullable(decisionId), "NotesDecisionSummary.replaced_by": values => values.every(decisionId), "NotesDecisionSummary.revision": hash, "NotesDecisionSummary.problems": values => values.every(value => text(value)),
    "NotesDecision.body": bound(256 * KiB), "NotesDecision.relative_path": bound(256), "NotesDecision.path": absolutePath,
    "NotesComment.todo_id": todoId, "NotesComment.comment_id": uuid, "NotesComment.created": nullable(text), "NotesComment.author": nullable(author), "NotesComment.body": bound(64 * KiB), "NotesComment.revision": hash,
    "NotesResult[todos].revision": revision, "NotesResult[todo].revision": hash, "NotesResult[todo_removed].revision": hash, "NotesResult[board].revision": revision,
    "NotesResult[comments].todo_id": todoId, "NotesResult[comment_removed].todo_id": todoId, "NotesResult[comment_removed].comment_id": uuid, "NotesResponse.notes_id": nullable(uuid),
  },
  checks: {
    NotesTodo: { consistency: t => t.ref.startsWith(`L${t.line}@`) && new Set(t.problems).size === t.problems.length },
    NotesDocument: { absent: d => d.revision !== "absent" || d.content === "" },
    NotesDecision: { document: d => d.relative_path === `decisions/${d.summary.decision_id}.md` && utf8.encode(d.summary.title).byteLength + utf8.encode(d.body).byteLength <= 256 * KiB },
    "NotesResult[catalog]": { unique: r => unique(r.entries, e => e.notes_id) },
    "NotesResult[todos]": { consistency: r => todos(r.todos, r.revision) },
    "NotesResult[todo]": { consistency: r => r.todo.ref === `L${r.todo.line}@${r.revision}` },
    "NotesResult[board]": { consistency: r => {
      const { backlog, doing, done } = r.columns, all = [...backlog, ...doing, ...done];
      return all.length <= 5000 && todos(all, r.revision) && backlog.every(t => !t.done && t.lane === "backlog") && doing.every(t => !t.done && t.lane === "doing") && done.every(t => t.done && t.lane !== null);
    } },
    "NotesResult[decisions]": { unique: r => unique(r.decisions, d => d.decision_id) },
    "NotesResult[comments]": { consistency: r => unique(r.comments, c => c.comment_id) && r.comments.every(c => c.todo_id === r.todo_id) && totalBytes(r.comments, c => c.body) <= 16 * MiB },
  },
};
const policy = definePolicy({ wire, message: constantMessage("Invalid Notes request or response") });
export function parseNotesRequest(value: unknown): NotesRequest {
  const request = parseWire(value, wireNotesRequest, policy), op = request.operation.op;
  if (op === "catalog_list" ? request.target.kind !== "root"
    : op === "target_create" || op === "target_attach" ? request.target.kind !== "space"
    : op === "target_resolve" ? request.target.kind === "root" : request.target.kind !== "notes") return fail();
  if (!text(JSON.stringify(request), 4 * MiB)) return fail();
  return request;
}
export function parseNotesResponse(value: unknown): NotesResponse {
  const response = parseWire(value, wireNotesResponse, policy);
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
