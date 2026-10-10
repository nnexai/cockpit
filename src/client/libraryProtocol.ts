import type { ContextDirectory, ContextDocument, ContextMedia, ContextFileIndex, LibraryFileIndexRequest, LibraryAddRequest, LibraryAttachmentRequest, LibraryConfluenceSpacesRequest, LibraryDirectoryRequest, LibraryDocumentRequest, LibraryListing, LibraryMediaRequest, LibraryOperation, LibraryRefreshRequest, LibraryRemoveRequest, LibraryReplaceRequest, LibraryResolution, LibraryResolveRequest, SpaceTarget, SpaceContextRequest, SpaceContextListing, SpaceAddRequest, SpaceRepositoriesRequest, SpaceRemoveRequest } from "../protocol/generated/v1";
import { wireLibraryFileIndexRequest, wireLibraryAddRequest, wireLibraryAttachmentRequest, wireLibraryConfluenceSpacesRequest, wireLibraryDirectoryRequest, wireLibraryDocumentRequest, wireLibraryListing, wireLibraryMediaRequest, wireLibraryOperation, wireLibraryRefreshRequest, wireLibraryRemoveRequest, wireLibraryReplaceRequest, wireLibraryResolution, wireLibraryResolveRequest, wireSpaceContextRequest, wireSpaceContextListing, wireSpaceAddRequest, wireSpaceRepositoriesRequest, wireSpaceRemoveRequest, type WireFields } from "../protocol/generated/validate";
import { CockpitClientError, validateSessionId, validateResourceId } from "./CockpitClient";
import { parseContextDirectory, parseContextDocument, parseContextFileIndex } from "./contextProtocol";
import { parseContextMedia } from "./contextMediaProtocol";
import { constantMessage, definePolicy, parseWire, parseWireList } from "./wire";

const fail = (): never => { throw new CockpitClientError("malformed_response", "Invalid library request or response"); };
const text = (v: string, max = 4096): boolean => v.length <= max && !v.includes("\0");
const id = (v: string): boolean => text(v, 512) && v.length > 0 && !/[\x00-\x1f\x7f]/.test(v);
const path = (v: string, empty = false): boolean => text(v) && (empty || v.length > 0) && !v.startsWith("/") && !/^[A-Za-z]:/.test(v) && !v.includes("\\") && (v.length === 0 || v.split("/").every(p => p !== "" && p !== "." && p !== ".."));
const absolutePath = (v: string): boolean => text(v) && v.startsWith("/") && !/[\x00-\x1f\x7f]/.test(v) && !v.split("/").some(p => p === "." || p === "..");
function fields<K extends keyof WireFields>(names: readonly K[], check: (v: WireFields[K]) => boolean) {
  const unary = (v: WireFields[K]): boolean => check(v);
  return Object.fromEntries(names.map(name => [name, unary])) as Record<K, (v: WireFields[K]) => boolean>;
}
const POLICY = definePolicy({
  message: constantMessage("Invalid library request or response"),
  wire: {
    exact: new Set(["SpaceAddRequest", "SpaceRemoveRequest", "SpaceRepositoriesRequest"]),
    absent: new Set(["LibraryAddRequest.follow_mode"]),
    nullish: new Set([
      "LibraryAttachment.media_type", "LibraryAttachment.version", "LibraryItemSummary.provider_id", "LibraryItemSummary.provider_instance", "LibraryItemSummary.resource_type", "LibraryItemSummary.canonical_id", "LibraryItemSummary.parent_item_id", "LibraryItemSummary.source_url", "LibraryItemSummary.original_url", "LibraryItemSummary.source_revision", "LibraryItemSummary.fetched_at", "LibraryItemSummary.checked_at", "LibraryItemSummary.purge_after", "LibraryFollowSummary.last_refreshed_at", "LibraryResolveRequest.provider_id", "LibraryResolution.provider_id", "LibraryResolution.provider_instance", "LibraryResolution.canonical_id", "LibraryResolution.container_label", "LibraryResolution.existing_item_id", "LibraryResolution.existing_follow_id", "LibraryAddRequest.provider_id", "LibraryAddRequest.label", "LibraryPhase.message", "LibraryReportRow.reason",
      "LibraryItemSummary.reference_depth", "LibraryItemSummary.included_by", "LibraryFollowSummary.reference_depth", "LibraryResolution.reference_depth", "LibraryAddRequest.reference_depth",
    ]),
    order: {
      SpaceTarget: ["session_id", "space_id"],
      LibraryAddRequest: ["target", "input", "provider_id", "reference_depth", "follow", "follow_mode", "download_attachments", "refresh_existing", "label"],
      LibraryOperation: ["phases", "report", "space", "target", "operation_id", "kind", "item_ids", "cancel_requested", "finished", "created_at", "updated_at"],
      SpaceContextRequest: ["target"], SpaceAddRequest: ["keys", "target", "item_ids"], SpaceRemoveRequest: ["keys", "target", "item_ids"],
      SpaceRepositoriesRequest: ["keys", "target", "repository_paths"],
      SpaceContextListing: ["target", "space_label", "library_root", "checkout_path", "items", "repository_paths", "diagnostics"],
    },
    lengths: {
      "LibraryItemSummary.ancestors": { max: 5000 }, "LibraryItemSummary.conflict": { max: 5000 }, "LibraryItemSummary.refs": { max: 5000 }, "LibraryItemSummary.attachments": { max: 5000 }, "LibraryItemSummary.diagnostics": { max: 256 }, "LibraryItemSummary.included_by": { max: 64 },
      "LibraryFollowSummary.excluded_ids": { max: 5000 }, "LibraryListing.items": { max: 5000 }, "LibraryListing.follows": { max: 5000 }, "LibraryListing.diagnostics": { max: 256 }, "LibraryResolution.diagnostics": { max: 256 },
      "LibraryRefreshRequest[items].item_ids": { max: 5000 }, "LibraryAttachmentRequest.attachment_ids": { min: 1, max: 256 }, "LibraryReplaceRequest.confirmed": { max: 5000 }, "LibraryOperation.phases": { max: 256 }, "LibraryOperation.item_ids": { max: 1_000_000 }, "LibraryRefreshReport.rows": { max: 256 }, "SpacePhaseResult.item_ids": { max: 1_000_000 },
      "SpaceAddRequest.item_ids": { max: 5000 }, "SpaceRemoveRequest.item_ids": { max: 5000 }, "SpaceRepositoriesRequest.repository_paths": { max: 64 }, "SpaceContextListing.items": { max: 1_000_000 }, "SpaceContextListing.repository_paths": { max: 64 }, "SpaceContextListing.diagnostics": { max: 256 },
    },
    fields: {
      ...fields(["ProjectDiagnostic.code", "LibraryConflictFile.current_hash", "LibraryPartial.unit", "LibraryAttachment.attachment_id", "LibraryContainer.container_id", "LibraryAncestor.id", "LibraryItemSummary.item_id", "LibraryItemSummary.logical_id", "LibraryItemSummary.revision", "LibraryInclusionHolder[item].item_id", "LibraryInclusionHolder[follow].follow_id", "LibraryItemRef[follow].follow_id", "LibraryItemRef[space].space_context_id", "LibraryFollowSource[confluence_space].space_key", "LibraryFollowSummary.follow_id", "LibraryFollowSummary.provider_id", "LibraryFollowSummary.provider_instance", "LibraryListing.generation", "LibraryRefreshRequest[follow].follow_id", "LibraryRefreshRequest[container].provider_instance", "LibraryRefreshRequest[container].container_id", "LibraryAttachmentRequest.item_id", "LibraryReplaceRequest.item_id", "LibraryRemoveRequest[item].item_id", "LibraryRemoveRequest[item].expected_revision", "LibraryRemoveRequest[stop_following].follow_id", "LibraryRemoveRequest[follow].follow_id", "LibraryOperation.operation_id", "ErrorResponse.code", "SpacePhaseResult.space_id"], id),
      ...fields(["ProjectDiagnostic.message", "LibraryPartial.reason", "LibraryAttachment.original_name", "LibraryAttachment.stored_name", "LibraryContainer.label", "LibraryFolderInfo.origin_path", "LibraryAncestor.title", "LibraryItemSummary.title", "LibraryFollowSource[confluence_space].space_name", "ContextRoot.label", "ContextRoot.path", "ContextRoot.checkout_path", "LibraryResolution.title", "ErrorResponse.message", "LibraryReportRow.title", "LibraryOperation.created_at", "LibraryOperation.updated_at", "SpaceContextListing.space_label"], text),
      ...fields(["ProjectDiagnostic.path", "LibraryAttachment.media_type", "LibraryAttachment.version", "LibraryItemSummary.provider_id", "LibraryItemSummary.provider_instance", "LibraryItemSummary.resource_type", "LibraryItemSummary.canonical_id", "LibraryItemSummary.parent_item_id", "LibraryItemSummary.source_url", "LibraryItemSummary.original_url", "LibraryItemSummary.source_revision", "LibraryItemSummary.fetched_at", "LibraryItemSummary.checked_at", "LibraryItemSummary.purge_after", "LibraryFollowSummary.last_refreshed_at", "LibraryResolveRequest.provider_id", "LibraryResolution.provider_id", "LibraryResolution.provider_instance", "LibraryResolution.canonical_id", "LibraryResolution.container_label", "LibraryResolution.existing_item_id", "LibraryResolution.existing_follow_id", "LibraryAddRequest.provider_id", "LibraryAddRequest.label", "LibraryPhase.message", "LibraryReportRow.reason"], v => v === null || text(v)),
      ...fields(["LibraryConflictFile.path", "LibraryItemSummary.item_path", "LibraryDocumentRequest.path", "LibraryMediaRequest.path"], path),
      ...fields(["LibraryAttachment.relative_path", "LibraryItemSummary.document_path"], v => v === null || path(v)),
      ...fields(["LibraryInclusion.from_item_id", "LibraryReportRow.item_id", "LibraryReportRow.follow_id", "LibraryDirectoryRequest.revision", "LibraryDocumentRequest.expected_revision", "LibraryMediaRequest.expected_revision"], v => v === null || id(v)),
      ...fields(["LibraryFollowSummary.excluded_ids", "LibraryRefreshRequest[items].item_ids", "LibraryAttachmentRequest.attachment_ids", "LibraryOperation.item_ids", "SpacePhaseResult.item_ids", "SpaceAddRequest.item_ids", "SpaceRemoveRequest.item_ids"], v => v.every(id)),
      ...fields(["LibraryItemSummary.reference_depth", "LibraryFollowSummary.reference_depth", "LibraryResolution.reference_depth"], v => v === undefined || v <= 5),
      ...fields(["LibraryResolveRequest.input", "LibraryAddRequest.input"], v => text(v, 16 * 1024)),
      ...fields(["LibraryIssueMeta.updated"], v => text(v, 128)), "LibraryIssueMeta.fetched_updated": v => v === null || text(v, 128),
      ...fields(["LibraryIssueMeta.status", "LibraryIssueMeta.issue_type", "LibraryInclusion.from_label"], v => text(v, 512)), "LibraryIssueMeta.assignee": v => v === null || text(v, 512),
      "LibraryInclusion.relation": v => text(v, 128), "LibraryInclusion.depth": v => v <= 5, "LibraryAddRequest.reference_depth": v => v <= 5,
      "LibraryFollowSource[jira_query].jql": v => text(v, 2048), "LibraryConfluenceSpacesRequest.provider_id": v => id(v) && v.length <= 128,
      "ContextRoot.root_id": v => v.startsWith("library:") && id(v), "ContextRoot.kind": v => v === "library", "ContextRoot.repository_id": id,
      "LibraryDirectoryRequest.path": v => path(v, true),
      "SpaceTarget.session_id": v => id(v) && !!validateSessionId(v), "SpaceTarget.space_id": v => id(v) && !!validateResourceId(v),
      "SpaceContextListing.library_root": absolutePath, "SpaceContextListing.checkout_path": v => v === null || absolutePath(v),
      ...fields(["SpaceContextListing.repository_paths", "SpaceRepositoriesRequest.repository_paths"], v => v.every(absolutePath)),
    },
    checks: {
      LibraryListing: { unique: v => new Set(v.items.map(x => x.item_id)).size === v.items.length && new Set(v.follows.map(x => x.follow_id)).size === v.follows.length },
      LibraryAttachmentRequest: { unique: v => new Set(v.attachment_ids).size === v.attachment_ids.length },
    },
  },
});
export function parseLibraryListing(value: unknown): LibraryListing { return parseWire(value, wireLibraryListing, POLICY); }
export function parseLibraryResolveRequest(value: unknown): LibraryResolveRequest { return parseWire(value, wireLibraryResolveRequest, POLICY); }
export function parseLibraryResolution(value: unknown): LibraryResolution { return parseWire(value, wireLibraryResolution, POLICY); }
export function parseLibraryConfluenceSpacesRequest(value: unknown): LibraryConfluenceSpacesRequest { return parseWire(value, wireLibraryConfluenceSpacesRequest, POLICY); }
export function parseLibraryConfluenceSpaces(value: unknown, request: LibraryConfluenceSpacesRequest): LibraryResolution[] {
  return parseWireList(value, wireLibraryResolution, POLICY, { max: 10_000 }).map(v => v.kind === "confluence_space" && v.provider_id === request.provider_id && v.canonical_id !== null ? v : fail());
}
export function parseLibraryAddRequest(value: unknown): LibraryAddRequest { return parseWire(value, wireLibraryAddRequest, POLICY); }
export function parseLibraryRefreshRequest(value: unknown): LibraryRefreshRequest { return parseWire(value, wireLibraryRefreshRequest, POLICY); }
export function parseLibraryAttachmentRequest(value: unknown): LibraryAttachmentRequest { return parseWire(value, wireLibraryAttachmentRequest, POLICY); }
export function parseLibraryReplaceRequest(value: unknown): LibraryReplaceRequest { return parseWire(value, wireLibraryReplaceRequest, POLICY); }
export function parseLibraryRemoveRequest(value: unknown): LibraryRemoveRequest { return parseWire(value, wireLibraryRemoveRequest, POLICY); }
export function parseLibraryOperation(value: unknown): LibraryOperation { return parseWire(value, wireLibraryOperation, POLICY); }
export function matchLibraryOperation(value: LibraryOperation, operationId: string): LibraryOperation { return value.operation_id === operationId ? value : fail(); }
export function matchLibraryAttachmentsOperation(value: LibraryOperation, request: LibraryAttachmentRequest): LibraryOperation {
  return value.kind === "attachments" && value.item_ids.length <= 1 && (value.item_ids.length === 0 || value.item_ids[0] === request.item_id) ? value : fail();
}
export function parseLibraryOperationId(value: unknown): string { return typeof value === "string" && id(value) ? value : fail(); }
export function parseLibraryDirectoryRequest(value: unknown): LibraryDirectoryRequest { return parseWire(value, wireLibraryDirectoryRequest, POLICY); }
export function parseLibraryDocumentRequest(value: unknown): LibraryDocumentRequest { return parseWire(value, wireLibraryDocumentRequest, POLICY); }
export function parseLibraryMediaRequest(value: unknown): LibraryMediaRequest { return parseWire(value, wireLibraryMediaRequest, POLICY); }
export function parseLibraryFileIndexRequest(value: unknown): LibraryFileIndexRequest { return parseWire(value, wireLibraryFileIndexRequest, POLICY); }
export function parseLibraryFileIndex(value: unknown): ContextFileIndex {
  const v = parseContextFileIndex(value);
  return v.binding_id === "library" && v.root_id.startsWith("library:") ? v : fail();
}
export function parseLibraryDirectory(value: unknown): ContextDirectory {
  const v = parseContextDirectory(value);
  return v.binding_id === "library" && v.root_id.startsWith("library:") && v.entries.length <= 10_000 ? v : fail();
}
export function matchLibraryDirectory(value: ContextDirectory, request: LibraryDirectoryRequest): ContextDirectory {
  return value.binding_id === "library" && value.root_id.startsWith("library:") && value.path === request.path && (request.revision === null || value.revision === request.revision) ? value : fail();
}
export function parseLibraryDocument(value: unknown): ContextDocument {
  const v = parseContextDocument(value);
  return v.binding_id === "library" && v.root_id.startsWith("library:") ? v : fail();
}
export function matchLibraryDocument(value: ContextDocument, request: LibraryDocumentRequest): ContextDocument {
  return value.binding_id === "library" && value.root_id.startsWith("library:") && value.path === request.path && (request.expected_revision === null || value.revision === request.expected_revision) && (request.offset === null || value.offset === request.offset) ? value : fail();
}
export function parseLibraryMedia(value: unknown): ContextMedia {
  const v = parseContextMedia(value);
  return v.binding_id === "library" && v.root_id.startsWith("library:") ? v : fail();
}
export function matchLibraryMedia(value: ContextMedia, request: LibraryMediaRequest): ContextMedia {
  return value.binding_id === "library" && value.root_id.startsWith("library:") && value.path === request.path && (request.expected_revision === null || value.revision === request.expected_revision) ? value : fail();
}
function sameTarget(left: SpaceTarget, right: SpaceTarget): boolean { return left.session_id === right.session_id && left.space_id === right.space_id; }
export function parseSpaceContextRequest(value: unknown): SpaceContextRequest { return parseWire(value, wireSpaceContextRequest, POLICY); }
export function parseSpaceAddRequest(value: unknown): SpaceAddRequest { return parseWire(value, wireSpaceAddRequest, POLICY); }
export function parseSpaceRemoveRequest(value: unknown): SpaceRemoveRequest { return parseWire(value, wireSpaceRemoveRequest, POLICY); }
export function parseSpaceRepositoriesRequest(value: unknown): SpaceRepositoriesRequest { return parseWire(value, wireSpaceRepositoriesRequest, POLICY); }
export function parseSpaceContextListing(value: unknown): SpaceContextListing { return parseWire(value, wireSpaceContextListing, POLICY); }
export function matchSpaceContextListing(value: SpaceContextListing, request: SpaceContextRequest): SpaceContextListing { return sameTarget(value.target, request.target) ? value : fail(); }
export function matchSpaceOperation(value: LibraryOperation, target: SpaceTarget): LibraryOperation {
  return value.target && sameTarget(value.target, target) && (value.space === null || value.space.space_id === target.space_id) ? value : fail();
}
