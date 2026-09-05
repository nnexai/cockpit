import type {
  CommentAnchor, CommentAttachment, CommentBatch, CommentBatchList, CommentBatchMutation,
  CommentBatchRequest, CommentBatchSummary, CommentCapture, CommentDraft, CommentFileRef,
  CommentLocation, CommentOwner, CommentPreview, CommentPreviewRequest, CommentRemoveRequest,
  CommentRequestScope, CommentSourceState, CommentUpsertRequest,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

type CommentRecord = Record<string, unknown>;
const encoder = new TextEncoder();
const MAX_COMMENT_ANCHOR_LINES = 20_000;
const COMMENT_PREVIEW_LIMIT_BYTES = 64 * 1024;
const COMMENT_PREVIEW_FRAMING_BYTES = 12;
const MAX_COMMENT_PREVIEW_PAYLOAD_BYTES = 4 * 1024 * 1024;
const invalid = (label: string): never => {
  throw new CockpitClientError("malformed_response", `Malformed comments ${label}`);
};
const record = (value: unknown): value is CommentRecord =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const identity = (value: unknown): value is string => text(value) && value.length > 0 && value.length <= 4096 && !value.includes("\0");
const integer = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= 0xffffffff;
const keys = (value: CommentRecord, allowed: readonly string[]): boolean =>
  Object.keys(value).every((key) => allowed.includes(key));
const relativePath = (value: unknown): value is string =>
  identity(value) && !value.startsWith("/") && !value.includes("\\")
    && value.split("/").every((part) => part !== "" && part !== "." && part !== "..");

function parseScope(value: unknown): CommentRequestScope {
  if (!record(value) || !keys(value, ["binding_id", "client_id"])
    || !identity(value.binding_id) || !identity(value.client_id)) return invalid("scope");
  return { binding_id: value.binding_id, client_id: value.client_id };
}

function parseOwner(value: unknown): CommentOwner {
  if (!record(value) || !keys(value, ["session_id", "pane_id", "terminal_id", "source_kind", "source_id"])
    || !identity(value.session_id) || !identity(value.pane_id) || !identity(value.terminal_id)
    || (value.source_kind !== "context" && value.source_kind !== "review") || !identity(value.source_id)) {
    return invalid("owner");
  }
  return {
    session_id: value.session_id,
    pane_id: value.pane_id,
    terminal_id: value.terminal_id,
    source_kind: value.source_kind,
    source_id: value.source_id,
  };
}

function parseLocation(value: unknown): CommentLocation {
  if (!record(value) || !keys(value, ["workspace_id", "tab_id"])
    || !identity(value.workspace_id) || !identity(value.tab_id)) return invalid("location");
  return { workspace_id: value.workspace_id, tab_id: value.tab_id };
}

function parseAttachment(value: unknown): CommentAttachment {
  if (!record(value) || !keys(value, ["owner", "location", "binding_id", "client_id"])
    || !identity(value.binding_id) || !identity(value.client_id)) return invalid("attachment");
  return {
    owner: parseOwner(value.owner),
    location: parseLocation(value.location),
    binding_id: value.binding_id,
    client_id: value.client_id,
  };
}

function parseCapture(value: unknown): CommentCapture {
  if (!record(value) || !keys(value, ["root_id", "path", "expected_revision", "start_line", "end_line"])
    || !identity(value.root_id) || !relativePath(value.path) || !identity(value.expected_revision)) {
    return invalid("capture");
  }
  const startLine = value.start_line;
  const endLine = value.end_line;
  const wholeFile = startLine === null && endLine === null;
  const lines = integer(startLine) && startLine > 0 && integer(endLine) && endLine >= startLine;
  if (!wholeFile && !lines) return invalid("capture");
  return {
    root_id: value.root_id,
    path: value.path,
    expected_revision: value.expected_revision,
    start_line: wholeFile ? null : startLine,
    end_line: wholeFile ? null : endLine,
  };
}

function parseAnchor(value: unknown): CommentAnchor {
  if (!record(value) || typeof value.kind !== "string") return invalid("anchor");
  if (value.kind === "whole_file") return { kind: "whole_file" };
  if (value.kind !== "lines" || !integer(value.start_line) || value.start_line === 0
    || !integer(value.end_line) || value.end_line < value.start_line || !Array.isArray(value.selected_lines)
    || value.selected_lines.length !== value.end_line - value.start_line + 1 || value.selected_lines.length > MAX_COMMENT_ANCHOR_LINES) {
    return invalid("anchor");
  }
  const selectedLines = value.selected_lines.map((line) => {
    if (!text(line)) return invalid("anchor");
    return line;
  });
  return {
    kind: "lines",
    start_line: value.start_line,
    end_line: value.end_line,
    selected_lines: selectedLines,
  };
}

function parseFileRef(value: unknown): CommentFileRef {
  if (!record(value) || !keys(value, ["root_id", "path", "absolute_path", "revision", "content_hash"])
    || !identity(value.root_id) || !relativePath(value.path) || !identity(value.absolute_path)
    || !identity(value.revision) || !(value.content_hash === null || identity(value.content_hash))) {
    return invalid("file reference");
  }
  return {
    root_id: value.root_id,
    path: value.path,
    absolute_path: value.absolute_path,
    revision: value.revision,
    content_hash: value.content_hash,
  };
}

function parseSourceState(value: unknown): CommentSourceState {
  if (value === "current" || value === "changed" || value === "missing" || value === "unavailable") return value;
  return invalid("source state");
}

function parseDraft(value: unknown): CommentDraft {
  if (!record(value) || !keys(value, ["draft_id", "file_ref", "anchor", "comment_text", "source_state", "updated_at"])
    || !identity(value.draft_id) || !text(value.comment_text)
    || encoder.encode(value.comment_text).byteLength > 8 * 1024 || !identity(value.updated_at)) {
    return invalid("draft");
  }
  return {
    draft_id: value.draft_id,
    file_ref: parseFileRef(value.file_ref),
    anchor: parseAnchor(value.anchor),
    comment_text: value.comment_text,
    source_state: parseSourceState(value.source_state),
    updated_at: value.updated_at,
  };
}

function parseMutation(value: unknown): CommentBatchMutation {
  if (!record(value) || !keys(value, ["scope", "batch_id", "expected_generation"])
    || !identity(value.batch_id) || !integer(value.expected_generation)) return invalid("mutation");
  return {
    scope: parseScope(value.scope),
    batch_id: value.batch_id,
    expected_generation: value.expected_generation,
  };
}

function validateBatchSize(value: unknown): void {
  let encoded: string | undefined;
  try { encoded = JSON.stringify(value); } catch { return invalid("batch size"); }
  if (encoded === undefined || encoder.encode(encoded).byteLength > 4 * 1024 * 1024) return invalid("batch size");
}

export function parseCommentScope(value: unknown): CommentRequestScope {
  return parseScope(value);
}

export function parseCommentBatchRequest(value: unknown): CommentBatchRequest {
  if (!record(value) || !keys(value, ["scope", "batch_id"])
    || !(value.batch_id === null || identity(value.batch_id))) return invalid("batch request");
  return { scope: parseScope(value.scope), batch_id: value.batch_id };
}

export function parseCommentMutation(value: unknown): CommentBatchMutation {
  return parseMutation(value);
}

export function parseCommentUpsert(value: unknown): CommentUpsertRequest {
  if (!record(value) || !keys(value, ["batch", "draft_id", "capture", "comment_text"])
    || !(value.draft_id === null || identity(value.draft_id)) || !text(value.comment_text)
    || encoder.encode(value.comment_text).byteLength > 8 * 1024) return invalid("upsert request");
  if (value.draft_id === null && value.capture === null) return invalid("upsert request");
  if (value.draft_id !== null && value.capture !== null) return invalid("upsert request");
  return {
    batch: parseMutation(value.batch),
    draft_id: value.draft_id,
    capture: value.capture === null ? null : parseCapture(value.capture),
    comment_text: value.comment_text,
  };
}

export function parseCommentRemove(value: unknown): CommentRemoveRequest {
  if (!record(value) || !keys(value, ["batch", "draft_id"]) || !identity(value.draft_id)) return invalid("remove request");
  return { batch: parseMutation(value.batch), draft_id: value.draft_id };
}

export function parseCommentPreviewRequest(value: unknown): CommentPreviewRequest {
  if (!record(value) || !keys(value, ["batch", "retain_stale_excerpts"])
    || typeof value.retain_stale_excerpts !== "boolean") return invalid("preview request");
  return { batch: parseMutation(value.batch), retain_stale_excerpts: value.retain_stale_excerpts };
}

export function parseCommentBatch(value: unknown): CommentBatch {
  if (!record(value) || !identity(value.batch_id) || !integer(value.generation)
    || !(value.live_attachment === null || record(value.live_attachment))
    || !Array.isArray(value.drafts) || value.drafts.length > 64 || !identity(value.updated_at)) return invalid("batch");
  const drafts = value.drafts.map(parseDraft);
  if (new Set(drafts.map((draft) => draft.draft_id)).size !== drafts.length) return invalid("duplicate draft identity");
  validateBatchSize(value);
  return {
    batch_id: value.batch_id,
    generation: value.generation,
    owner: parseOwner(value.owner),
    last_known_location: parseLocation(value.last_known_location),
    live_attachment: value.live_attachment === null ? null : parseAttachment(value.live_attachment),
    drafts,
    updated_at: value.updated_at,
  };
}

function parseBatchSummary(value: unknown): CommentBatchSummary {
  if (!record(value) || !keys(value, ["batch_id", "generation", "owner", "last_known_location", "draft_count", "updated_at"])
    || !identity(value.batch_id) || !integer(value.generation) || !integer(value.draft_count)
    || value.draft_count > 64 || !identity(value.updated_at)) return invalid("batch summary");
  return {
    batch_id: value.batch_id,
    generation: value.generation,
    owner: parseOwner(value.owner),
    last_known_location: parseLocation(value.last_known_location),
    draft_count: value.draft_count,
    updated_at: value.updated_at,
  };
}

export function parseCommentBatchList(value: unknown): CommentBatchList {
  if (!record(value) || !Array.isArray(value.batches) || value.batches.length > 256
    || typeof value.truncated !== "boolean") return invalid("batch list");
  const batches = value.batches.map(parseBatchSummary);
  if (new Set(batches.map((batch) => batch.batch_id)).size !== batches.length) return invalid("duplicate batch identity");
  return { attachment: parseAttachment(value.attachment), batches, truncated: value.truncated };
}

export function parseCommentPreview(value: unknown): CommentPreview {
  if (!record(value) || !identity(value.batch_id) || !integer(value.generation) || !text(value.payload)
    || !integer(value.payload_bytes) || !integer(value.framed_bytes) || !integer(value.limit_bytes)
    || !integer(value.sanitized_controls) || typeof value.exportable !== "boolean"
    || !(value.reason === null || text(value.reason)) || !Array.isArray(value.stale_draft_ids)
    || value.stale_draft_ids.length > 64) return invalid("preview");
  const staleDraftIds = value.stale_draft_ids.map((draftId) => {
    if (!identity(draftId)) return invalid("preview");
    return draftId;
  });
  const payloadBytes = encoder.encode(value.payload).byteLength;
  if (value.limit_bytes !== COMMENT_PREVIEW_LIMIT_BYTES || payloadBytes > MAX_COMMENT_PREVIEW_PAYLOAD_BYTES
    || payloadBytes !== value.payload_bytes || value.framed_bytes !== payloadBytes + COMMENT_PREVIEW_FRAMING_BYTES
    || new Set(staleDraftIds).size !== staleDraftIds.length
    || (value.exportable && (value.framed_bytes > value.limit_bytes || value.reason !== null))
    || (!value.exportable && value.reason === null)) return invalid("preview byte count");
  return {
    batch_id: value.batch_id,
    generation: value.generation,
    payload: value.payload,
    payload_bytes: value.payload_bytes,
    framed_bytes: value.framed_bytes,
    limit_bytes: value.limit_bytes,
    sanitized_controls: value.sanitized_controls,
    stale_draft_ids: staleDraftIds,
    exportable: value.exportable,
    reason: value.reason,
  };
}

export function matchCommentAttachment(
  value: CommentAttachment,
  sessionId: string,
  paneId: string,
  request: CommentRequestScope,
): void {
  if (value.owner.session_id !== sessionId || value.owner.pane_id !== paneId
    || value.binding_id !== request.binding_id || value.client_id !== request.client_id) invalid("attachment identity");
}

export function matchCommentBatch(
  value: CommentBatch,
  sessionId: string,
  paneId: string,
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
    matchCommentAttachment(live, sessionId, paneId, request);
    if (value.owner.session_id !== live.owner.session_id || value.owner.pane_id !== live.owner.pane_id
      || value.owner.terminal_id !== live.owner.terminal_id || value.owner.source_kind !== live.owner.source_kind
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
