import type { ReviewFileDiff, ReviewFileRequest, ReviewSnapshot, ReviewSnapshotRequest } from "../protocol/generated/v1";
import { wireReviewSnapshotRequest, wireReviewFileRequest, wireReviewSnapshot, wireReviewFileDiff, type TypedWirePolicy } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { constantMessage, definePolicy, parseWire } from "./wire";

const fail = (): never => { throw new CockpitClientError("malformed_response", "Malformed local review response"); };
const text = (v: string | null): boolean => v === null || v.length <= 4096;
const id = (v: string): boolean => v.length > 0 && v.length <= 4096 && !/[\x00-\x1f\x7f]/.test(v);
const review = definePolicy({
  message: constantMessage("Malformed local review response"),
  wire: {
    // source_revision remains required nullable; only the fields accepted by the old defaults are nullish.
    nullish: new Set(["ReviewFileRequest.source_side", "ReviewFileRequest.source_offset", "ReviewFileDiff.old_source_offset", "ReviewFileDiff.new_source_offset"]),
    lengths: { "ReviewSnapshot.files": { max: 100_000 }, "ReviewSnapshot.diagnostics": { max: 1024 },
      "ReviewFileDiff.hunks": { max: 2048 }, "ReviewHunk.lines": { max: 100_000 }, "ReviewFileDiff.diagnostics": { max: 1024 } },
    fields: {
      "ReviewSnapshotRequest.binding_id": id, "ReviewSnapshotRequest.repository_id": id, "ReviewSnapshotRequest.base_ref": text,
      "ReviewFileRequest.binding_id": id, "ReviewFileRequest.review_id": id, "ReviewFileRequest.file_id": id,
      "ReviewFileRequest.source_revision": text, "ReviewChangedFile.file_id": id,
      "ReviewChangedFile.old_path": text, "ReviewChangedFile.new_path": text, "ReviewChangedFile.summary": text,
      "ReviewChangedFile.old_revision": text, "ReviewChangedFile.new_revision": text,
      "ReviewSnapshot.binding_id": id, "ReviewSnapshot.session_id": id, "ReviewSnapshot.viewer_id": id,
      "ReviewSnapshot.review_id": id, "ReviewSnapshot.repository_id": id, "ReviewSnapshot.checkout_path": text,
      "ReviewSnapshot.source_id": id, "ReviewSnapshot.base_revision": text, "ReviewSnapshot.head_revision": text,
      "ReviewSnapshot.index_revision": id, "ReviewSnapshot.worktree_revision": id,
      "ProjectDiagnostic.code": id, "ProjectDiagnostic.message": text, "ProjectDiagnostic.path": text,
      "ReviewHunk.old_path": text, "ReviewHunk.new_path": text, "ReviewDiffLine.text": v => v.length <= 2 * 1024 * 1024,
      "ReviewFileDiff.binding_id": id, "ReviewFileDiff.session_id": id, "ReviewFileDiff.viewer_id": id,
      "ReviewFileDiff.review_id": id, "ReviewFileDiff.old_source": v => v === null || v.length <= 512 * 1024,
      "ReviewFileDiff.new_source": v => v === null || v.length <= 512 * 1024,
      "ReviewFileDiff.old_source_hash": text, "ReviewFileDiff.new_source_hash": text,
    },
    checks: { ReviewFileDiff: { total_lines: v => {
      let total = 0;
      for (const hunk of v.hunks) { total += hunk.lines.length; if (total > 100_000) return false; }
      return true;
    } } },
  } satisfies TypedWirePolicy,
});
export const parseReviewSnapshotRequest = (v: unknown): ReviewSnapshotRequest => parseWire(v, wireReviewSnapshotRequest, review);
export const parseReviewFileRequest = (v: unknown): ReviewFileRequest => parseWire(v, wireReviewFileRequest, review);
export const parseReviewSnapshot = (v: unknown): ReviewSnapshot => parseWire(v, wireReviewSnapshot, review);
export const parseReviewFile = (v: unknown): ReviewFileDiff => parseWire(v, wireReviewFileDiff, review);
export function matchReviewSnapshot(value: ReviewSnapshot, session: string, viewer: string, request: ReviewSnapshotRequest): ReviewSnapshot { if (value.session_id !== session || value.viewer_id !== viewer || value.binding_id !== request.binding_id || value.repository_id !== request.repository_id || value.comparison !== request.comparison) return fail(); return value; }
export function matchReviewFile(value: ReviewFileDiff, session: string, viewer: string, request: ReviewFileRequest): ReviewFileDiff { if (value.session_id !== session || value.viewer_id !== viewer || value.binding_id !== request.binding_id || value.review_id !== request.review_id || value.generation !== request.generation || value.file.file_id !== request.file_id) return fail(); if (request.source_revision !== null) { const revision = request.source_side === "old" ? value.file.old_revision : value.file.new_revision; if (revision !== request.source_revision) return fail(); } return value; }

