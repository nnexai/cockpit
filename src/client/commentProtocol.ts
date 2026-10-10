import type { CommentAttachment, CommentBatch, CommentBatchList, CommentBatchMutation, CommentBatchRequest,
  CommentPreview, CommentPreviewRequest, CommentRemoveRequest, CommentRequestScope, CommentUpsertRequest } from "../protocol/generated/v1";
import { wireCommentRequestScope, wireCommentBatchRequest, wireCommentBatchMutation, wireCommentUpsertRequest,
  wireCommentRemoveRequest, wireCommentPreviewRequest, wireCommentBatch, wireCommentBatchList,
  wireCommentPreview, type TypedWirePolicy } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, messageTable, parseWire } from "./wire";
const encoder = new TextEncoder();
const identity = (value: string): boolean => value.length > 0 && value.length <= 4096 && !value.includes("\0");
const relativePath = (value: string): boolean => identity(value) && !value.startsWith("/") && !value.includes("\\")
  && value.split("/").every((part) => part !== "" && part !== "." && part !== "..");
const invalid = (label: string): never => { throw new CockpitClientError("malformed_response", `Malformed comments ${label}`); };
const batchSize = (value: unknown): boolean => {
  try { const encoded = JSON.stringify(value); return encoded !== undefined && encoder.encode(encoded).byteLength <= 4 * 1024 * 1024; }
  catch { return false; }
};
// Names-only policies preserve the former guards before each nested parser and the projected key order.
export const COMMENT_WIRE = {
  exact: new Set(["CommentRequestScope", "CommentOwner[viewer]", "CommentLocation", "CommentAttachment",
    "CommentReviewRef", "CommentCapture", "CommentFileRef", "CommentDraft", "CommentBatchMutation",
    "CommentBatchRequest", "CommentUpsertRequest", "CommentRemoveRequest", "CommentPreviewRequest", "CommentBatchSummary"]),
  absent: new Set(["CommentCapture.review", "CommentFileRef.review"]),
  order: {
    CommentAttachment: ["keys", "binding_id", "client_id", "owner", "location", "check:live_viewer"],
    CommentCapture: ["keys", "root_id", "path", "expected_revision", "start_line", "end_line", "check:lines", "review"],
    CommentFileRef: ["keys", "root_id", "path", "absolute_path", "revision", "content_hash", "review"],
    CommentDraft: ["keys", "draft_id", "comment_text", "updated_at", "file_ref", "anchor", "source_state"],
    CommentBatchMutation: ["keys", "batch_id", "expected_generation", "scope"],
    CommentBatchRequest: ["keys", "batch_id", "scope"],
    CommentUpsertRequest: ["keys", "draft_id", "comment_text", "check:capture_choice", "batch", "capture"],
    CommentRemoveRequest: ["keys", "draft_id", "batch"],
    CommentPreviewRequest: ["keys", "retain_stale_excerpts", "batch"],
    CommentBatch: ["batch_id", "generation", "live_attachment:record", "drafts:shallow", "updated_at", "drafts",
      "check:duplicate_draft", "check:batch_size", "owner", "live_attachment", "last_known_location"],
    CommentBatchSummary: ["keys", "batch_id", "generation", "draft_count", "updated_at", "owner", "last_known_location"],
    CommentBatchList: ["batches:shallow", "truncated", "batches", "check:duplicate_batch", "attachment"],
  },
  emit: {
    "CommentOwner[viewer]": ["tag", "session_id", "server_instance", "tab_id", "source_kind", "source_id"],
    CommentCapture: ["review", "root_id", "path", "expected_revision", "start_line", "end_line"],
    CommentFileRef: ["review", "root_id", "path", "absolute_path", "revision", "content_hash"],
  },
  lengths: { "CommentAnchor[lines].selected_lines": { max: 20_000 }, "CommentBatch.drafts": { max: 64 },
    "CommentBatchList.batches": { max: 256 }, "CommentPreview.stale_draft_ids": { max: 64 } },
  fields: {
    "CommentRequestScope.binding_id": identity, "CommentRequestScope.client_id": identity, "CommentOwner[viewer].session_id": identity, "CommentOwner[viewer].source_id": identity,
    "CommentOwner[viewer].server_instance": v => /^[0-9a-f]{16}$/.test(v), "CommentOwner[viewer].tab_id": identity,
    "CommentLocation.workspace_id": identity, "CommentLocation.tab_id": identity, "CommentAttachment.binding_id": identity, "CommentAttachment.client_id": identity,
    "CommentReviewRef.review_id": identity, "CommentReviewRef.file_id": identity, "CommentReviewRef.generation": v => v >= 1,
    "CommentCapture.root_id": identity, "CommentCapture.path": relativePath, "CommentCapture.expected_revision": identity,
    "CommentFileRef.root_id": identity, "CommentFileRef.path": relativePath, "CommentFileRef.absolute_path": identity,
    "CommentFileRef.revision": identity, "CommentFileRef.content_hash": v => v === null || identity(v),
    "CommentDraft.draft_id": identity, "CommentDraft.comment_text": v => encoder.encode(v).byteLength <= 8 * 1024,
    "CommentDraft.updated_at": identity, "CommentBatchMutation.batch_id": identity, "CommentBatchRequest.batch_id": v => v === null || identity(v),
    "CommentUpsertRequest.draft_id": v => v === null || identity(v), "CommentUpsertRequest.comment_text": v => encoder.encode(v).byteLength <= 8 * 1024,
    "CommentRemoveRequest.draft_id": identity, "CommentBatch.batch_id": identity, "CommentBatch.updated_at": identity,
    "CommentBatchSummary.batch_id": identity, "CommentBatchSummary.draft_count": v => v <= 64,
    "CommentBatchSummary.updated_at": identity, "CommentPreview.batch_id": identity, "CommentPreview.stale_draft_ids": v => v.every(identity),
  },
  checks: {
    CommentAttachment: { live_viewer: v => v.owner.kind === "viewer" && v.owner.tab_id === v.location.tab_id },
    CommentCapture: { lines: v => (v.start_line === null && v.end_line === null)
      || (v.start_line !== null && v.start_line > 0 && v.end_line !== null && v.end_line >= v.start_line) },
    "CommentAnchor[lines]": { lines: v => v.start_line > 0 && v.end_line >= v.start_line
      && v.selected_lines.length === v.end_line - v.start_line + 1 },
    CommentUpsertRequest: { capture_choice: (_v, original) => (original.draft_id === null) !== (original.capture === null) },
    CommentBatch: { duplicate_draft: v => new Set(v.drafts.map(d => d.draft_id)).size === v.drafts.length,
      batch_size: (_v, original) => batchSize(original) },
    CommentBatchList: { duplicate_batch: v => new Set(v.batches.map(b => b.batch_id)).size === v.batches.length },
    CommentPreview: { byte_count: v => {
      const bytes = encoder.encode(v.payload).byteLength;
      return v.limit_bytes === 64 * 1024 && bytes <= 4 * 1024 * 1024 && bytes === v.payload_bytes
        && v.framed_bytes === bytes + 12 && new Set(v.stale_draft_ids).size === v.stale_draft_ids.length
        && (v.exportable ? v.framed_bytes <= v.limit_bytes && v.reason === null : v.reason !== null);
    } },
  },
} satisfies TypedWirePolicy;
const comments = definePolicy({
  wire: COMMENT_WIRE,
  message: messageTable({
    CommentRequestScope: "Malformed comments scope", CommentOwner: "Malformed comments owner", CommentLocation: "Malformed comments location",
    CommentAttachment: f => `Malformed comments ${f.refinement === "live_viewer" ? "live viewer attachment" : "attachment"}`,
    CommentReviewRef: "Malformed comments review anchor", CommentCapture: "Malformed comments capture",
    CommentAnchor: "Malformed comments anchor", CommentFileRef: "Malformed comments file reference",
    CommentSourceState: "Malformed comments source state", CommentDraft: "Malformed comments draft",
    CommentBatchMutation: "Malformed comments mutation", CommentBatchRequest: "Malformed comments batch request",
    CommentUpsertRequest: "Malformed comments upsert request", CommentRemoveRequest: "Malformed comments remove request", CommentPreviewRequest: "Malformed comments preview request",
    CommentBatch: f => `Malformed comments ${f.refinement === "duplicate_draft" ? "duplicate draft identity" : f.refinement === "batch_size" ? "batch size" : "batch"}`,
    CommentBatchSummary: "Malformed comments batch summary",
    CommentBatchList: f => `Malformed comments ${f.refinement === "duplicate_batch" ? "duplicate batch identity" : "batch list"}`,
    CommentPreview: f => `Malformed comments ${f.refinement === "byte_count" ? "preview byte count" : "preview"}`,
  }, "Malformed comments owner"),
});
export const parseCommentScope = (v: unknown): CommentRequestScope => parseWire(v, wireCommentRequestScope, comments);
export const parseCommentBatchRequest = (v: unknown): CommentBatchRequest => parseWire(v, wireCommentBatchRequest, comments);
export const parseCommentMutation = (v: unknown): CommentBatchMutation => parseWire(v, wireCommentBatchMutation, comments);
export const parseCommentUpsert = (v: unknown): CommentUpsertRequest => parseWire(v, wireCommentUpsertRequest, comments);
export const parseCommentRemove = (v: unknown): CommentRemoveRequest => parseWire(v, wireCommentRemoveRequest, comments);
export const parseCommentPreviewRequest = (v: unknown): CommentPreviewRequest => parseWire(v, wireCommentPreviewRequest, comments);
export const parseCommentBatch = (v: unknown): CommentBatch => parseWire(v, wireCommentBatch, comments);
export const parseCommentBatchList = (v: unknown): CommentBatchList => parseWire(v, wireCommentBatchList, comments);
export const parseCommentPreview = (v: unknown): CommentPreview => parseWire(v, wireCommentPreview, comments);

export function matchCommentAttachment(
  value: CommentAttachment,
  sessionId: string,
  request: CommentRequestScope,
): void {
  // binding_id pins the freshly authorized viewer; durable ownership is tab/source scoped.
  if (value.owner.kind !== "viewer" || value.owner.session_id !== sessionId
    || value.location.tab_id !== value.owner.tab_id
    || value.binding_id !== request.binding_id || value.client_id !== request.client_id) invalid("attachment identity");
}

export function matchCommentBatch(
  value: CommentBatch,
  sessionId: string,
  request: CommentRequestScope,
  batchId?: string | null,
  expectedGeneration?: number,
  requireAttachment = false,
): CommentBatch {
  if (batchId != null && value.batch_id !== batchId) invalid("batch identity");
  if (expectedGeneration !== undefined
    && (expectedGeneration === 0xffffffff || value.generation !== expectedGeneration + 1)) invalid("batch generation");
  if (value.live_attachment === null) {
    if (requireAttachment) invalid("missing attachment");
  } else {
    const live = value.live_attachment;
    matchCommentAttachment(live, sessionId, request);
    if (value.owner.kind !== "viewer" || live.owner.kind !== "viewer"
      || value.owner.session_id !== live.owner.session_id || value.owner.server_instance !== live.owner.server_instance
      || value.owner.tab_id !== live.owner.tab_id || value.owner.source_kind !== live.owner.source_kind
      || value.owner.source_id !== live.owner.source_id
      || value.last_known_location.workspace_id !== live.location.workspace_id
      || value.last_known_location.tab_id !== live.location.tab_id) invalid("batch attachment identity");
  }
  return value;
}

export function matchCommentPreview(value: CommentPreview, request: CommentBatchMutation): CommentPreview {
  if (value.batch_id !== request.batch_id || value.generation !== request.expected_generation) invalid("preview identity");
  return value;
}
