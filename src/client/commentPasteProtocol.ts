import type { CommentPastePrepareRequest, CommentPastePrepareResponse, CommentPasteReceipt, CommentPasteMarkPastedRequest, CommentPasteSendRequest, CommentPasteTarget } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
import { COMMENT_WIRE, parseCommentPreviewRequest } from "./commentProtocol";
import { wireCommentPasteTarget, wireCommentPasteReceipt, wireCommentPastePrepareResponse,
  wireCommentPasteSendRequest, wireCommentPasteMarkPastedRequest, type TypedWirePolicy } from "../protocol/generated/validate";
import { definePolicy, messageTable, parseWire } from "./wire";

const fail = (): never => { throw new CockpitClientError("malformed_response", "Malformed comment paste response"); };
const text = (v: string): boolean => v.length > 0 && v.length <= 4096 && !/[\x00-\x1f\x7f]/.test(v);
const paste = definePolicy({
  message: messageTable({ CommentBatchMutation: "Malformed comments mutation", CommentRequestScope: "Malformed comments scope" },
    "Malformed comment paste response"),
  wire: {
    ...COMMENT_WIRE,
    order: { ...COMMENT_WIRE.order,
      CommentPasteSendRequest: ["retain_stale_excerpts", "acknowledge_duplicate_risk", "batch", "target",
        "expected_payload_hash", "operation_id", "request_id"],
      CommentPasteMarkPastedRequest: ["batch", "operation_id"],
    },
    emit: { ...COMMENT_WIRE.emit,
      CommentPasteTarget: ["endpoint_identity", "session_id", "workspace_id", "tab_id", "pane_id", "terminal_id", "agent_fingerprint", "agent_label"],
      CommentPasteReceipt: ["user_confirmed", "operation_id", "request_id", "batch_id", "batch_generation", "payload_hash", "target", "state", "sent_draft_ids", "created_at", "completed_at", "message"],
    },
    lengths: { ...COMMENT_WIRE.lengths, "CommentPasteReceipt.sent_draft_ids": { max: 64 },
      "CommentPastePrepareResponse.targets": { max: 256 }, "CommentPastePrepareResponse.receipts": { max: 256 } },
    fields: { ...COMMENT_WIRE.fields,
      "CommentPasteTarget.endpoint_identity": text, "CommentPasteTarget.session_id": text,
      "CommentPasteTarget.workspace_id": text, "CommentPasteTarget.tab_id": text, "CommentPasteTarget.pane_id": text,
      "CommentPasteTarget.terminal_id": text, "CommentPasteTarget.agent_fingerprint": text, "CommentPasteTarget.agent_label": text,
      "CommentPasteReceipt.operation_id": text, "CommentPasteReceipt.request_id": text, "CommentPasteReceipt.batch_id": text,
      "CommentPasteReceipt.payload_hash": text, "CommentPasteReceipt.sent_draft_ids": v => v.every(text),
      "CommentPasteReceipt.created_at": text, "CommentPasteReceipt.completed_at": v => v === null || text(v),
      "CommentPasteReceipt.message": v => v === null || text(v), "CommentPastePrepareResponse.batch_id": text,
      "CommentPastePrepareResponse.payload_hash": text, "CommentPastePrepareResponse.reason": v => v === null || text(v),
      "CommentPasteSendRequest.expected_payload_hash": text, "CommentPasteSendRequest.operation_id": text,
      "CommentPasteSendRequest.request_id": text, "CommentPasteMarkPastedRequest.operation_id": text,
    },
    checks: { ...COMMENT_WIRE.checks, CommentPastePrepareResponse: { framing: v => v.framed_bytes === v.payload_bytes + 12 && v.limit_bytes === 65536 } },
  } satisfies TypedWirePolicy,
});
export const parseCommentPastePrepareRequest = (v: unknown): CommentPastePrepareRequest => parseCommentPreviewRequest(v);
export const parseCommentPasteTarget = (v: unknown): CommentPasteTarget => parseWire(v, wireCommentPasteTarget, paste);
export const parseCommentPasteReceipt = (v: unknown): CommentPasteReceipt => parseWire(v, wireCommentPasteReceipt, paste);
export const parseCommentPastePrepare = (v: unknown): CommentPastePrepareResponse => parseWire(v, wireCommentPastePrepareResponse, paste);
export const parseCommentPasteSendRequest = (v: unknown): CommentPasteSendRequest => parseWire(v, wireCommentPasteSendRequest, paste);
export function matchPastePrepare(response: CommentPastePrepareResponse, session: string, request: CommentPastePrepareRequest): CommentPastePrepareResponse {
  if (response.batch_id !== request.batch.batch_id || response.generation !== request.batch.expected_generation || response.targets.some(t => t.session_id !== session) || response.receipts.some(r => r.batch_id !== response.batch_id)) return fail();
  return response;
}
export function matchPasteReceipt(response: CommentPasteReceipt, request: CommentPasteSendRequest): CommentPasteReceipt {
  if (response.operation_id !== request.operation_id || response.request_id !== request.request_id || response.batch_id !== request.batch.batch_id || response.batch_generation !== request.batch.expected_generation || response.payload_hash !== request.expected_payload_hash || JSON.stringify(response.target) !== JSON.stringify(request.target)) return fail();
  return response;
}

export const parseCommentPasteMarkPastedRequest = (v: unknown): CommentPasteMarkPastedRequest => parseWire(v, wireCommentPasteMarkPastedRequest, paste);
export function matchMarkedReceipt(response: CommentPasteReceipt, request: CommentPasteMarkPastedRequest): CommentPasteReceipt {
  if (response.operation_id !== request.operation_id || response.batch_id !== request.batch.batch_id || !response.user_confirmed || response.state !== "accepted") return fail();
  return response;
}
