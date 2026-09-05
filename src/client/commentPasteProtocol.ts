import type { CommentPastePrepareRequest, CommentPastePrepareResponse, CommentPasteReceipt, CommentPasteMarkPastedRequest, CommentPasteSendRequest, CommentPasteTarget } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
import { parseCommentMutation, parseCommentPreviewRequest } from "./commentProtocol";

const fail = (): never => { throw new CockpitClientError("malformed_response", "Malformed comment paste response"); };
const record = (v: unknown): Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v) ? v as Record<string, unknown> : fail();
const text = (v: unknown): string => typeof v === "string" && v.length > 0 && v.length <= 4096 && !/[\x00-\x1f\x7f]/.test(v) ? v : fail();
const number = (v: unknown): number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= 0xffffffff ? v : fail();
const message = (v: unknown): string | null => v === null ? null : text(v);
export const parseCommentPastePrepareRequest = (v: unknown): CommentPastePrepareRequest => parseCommentPreviewRequest(v);
export function parseCommentPasteTarget(v: unknown): CommentPasteTarget {
  const r = record(v);
  return { endpoint_identity: text(r.endpoint_identity), session_id: text(r.session_id), workspace_id: text(r.workspace_id), tab_id: text(r.tab_id), pane_id: text(r.pane_id), terminal_id: text(r.terminal_id), agent_fingerprint: text(r.agent_fingerprint), agent_label: text(r.agent_label) };
}
export function parseCommentPasteReceipt(v: unknown): CommentPasteReceipt {
  const r = record(v);
  if (!["pending", "accepted", "rejected", "outcome_unknown"].includes(String(r.state)) || !Array.isArray(r.sent_draft_ids) || r.sent_draft_ids.length > 64) return fail();
  if (typeof r.user_confirmed !== "boolean") return fail();
  return { user_confirmed: r.user_confirmed, operation_id: text(r.operation_id), request_id: text(r.request_id), batch_id: text(r.batch_id), batch_generation: number(r.batch_generation), payload_hash: text(r.payload_hash), target: parseCommentPasteTarget(r.target), state: r.state as CommentPasteReceipt["state"], sent_draft_ids: r.sent_draft_ids.map(text), created_at: text(r.created_at), completed_at: message(r.completed_at), message: message(r.message) };
}
export function parseCommentPastePrepare(v: unknown): CommentPastePrepareResponse {
  const r = record(v);
  if (!Array.isArray(r.targets) || r.targets.length > 256 || !Array.isArray(r.receipts) || r.receipts.length > 256 || typeof r.paste_available !== "boolean") return fail();
  const response = { batch_id: text(r.batch_id), generation: number(r.generation), payload_hash: text(r.payload_hash), payload_bytes: number(r.payload_bytes), framed_bytes: number(r.framed_bytes), limit_bytes: number(r.limit_bytes), targets: r.targets.map(parseCommentPasteTarget), receipts: r.receipts.map(parseCommentPasteReceipt), paste_available: r.paste_available, reason: message(r.reason) };
  if (response.framed_bytes !== response.payload_bytes + 12 || response.limit_bytes !== 65536) return fail();
  return response;
}
export function parseCommentPasteSendRequest(v: unknown): CommentPasteSendRequest {
  const r = record(v);
  if (typeof r.retain_stale_excerpts !== "boolean" || typeof r.acknowledge_duplicate_risk !== "boolean") return fail();
  return { batch: parseCommentMutation(r.batch), target: parseCommentPasteTarget(r.target), expected_payload_hash: text(r.expected_payload_hash), retain_stale_excerpts: r.retain_stale_excerpts, operation_id: text(r.operation_id), request_id: text(r.request_id), acknowledge_duplicate_risk: r.acknowledge_duplicate_risk };
}
export function matchPastePrepare(response: CommentPastePrepareResponse, session: string, request: CommentPastePrepareRequest): CommentPastePrepareResponse {
  if (response.batch_id !== request.batch.batch_id || response.generation !== request.batch.expected_generation || response.targets.some(t => t.session_id !== session) || response.receipts.some(r => r.batch_id !== response.batch_id)) return fail();
  return response;
}
export function matchPasteReceipt(response: CommentPasteReceipt, request: CommentPasteSendRequest): CommentPasteReceipt {
  if (response.operation_id !== request.operation_id || response.request_id !== request.request_id || response.batch_id !== request.batch.batch_id || response.batch_generation !== request.batch.expected_generation || response.payload_hash !== request.expected_payload_hash || JSON.stringify(response.target) !== JSON.stringify(request.target)) return fail();
  return response;
}

export function parseCommentPasteMarkPastedRequest(v: unknown): CommentPasteMarkPastedRequest {
  const r = record(v);
  return { batch: parseCommentMutation(r.batch), operation_id: text(r.operation_id) };
}
export function matchMarkedReceipt(response: CommentPasteReceipt, request: CommentPasteMarkPastedRequest): CommentPasteReceipt {
  if (response.operation_id !== request.operation_id || response.batch_id !== request.batch.batch_id || !response.user_confirmed || response.state !== "accepted") return fail();
  return response;
}
