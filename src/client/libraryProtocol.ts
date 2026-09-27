import type {
  ContextDirectory, ContextDocument, ContextMedia,
  LibraryAddRequest, LibraryAttachmentRequest, LibraryConfluenceSpacesRequest, LibraryConflictFile, LibraryDirectoryRequest, LibraryDocumentRequest,
  LibraryFollowSummary, LibraryItemSummary, LibraryListing, LibraryMediaRequest, LibraryOperation, LibraryRefreshRequest,
  LibraryRemoveRequest, LibraryReplaceRequest, LibraryResolution, LibraryResolveRequest, ProjectDiagnostic,
  SpaceTarget, SpaceContextRequest, SpaceContextListing, SpaceAddRequest, SpaceAttemptsDismissRequest, SpaceUpdateRequest, SpaceRemoveRequest,
} from "../protocol/generated/v1";
import { CockpitClientError, validateSessionId, validateResourceId } from "./CockpitClient";
import { parseContextDirectory, parseContextDocument } from "./contextProtocol";
import { parseContextMedia } from "./contextMediaProtocol";

const fail = (): never => { throw new CockpitClientError("malformed_response", "Invalid library request or response"); };
const record = (value: unknown): Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : fail();
const text = (value: unknown, max = 4096): string => typeof value === "string" && value.length <= max && !value.includes("\0") ? value : fail();
const id = (value: unknown): string => { const v = text(value, 512); return v.length > 0 && !/[\x00-\x1f\x7f]/.test(v) ? v : fail(); };
const nullable = <T>(value: unknown, parse: (value: unknown) => T): T | null => value === null ? null : parse(value);
const bool = (value: unknown): boolean => typeof value === "boolean" ? value : fail();
const integer = (value: unknown, max = Number.MAX_SAFE_INTEGER): number => Number.isSafeInteger(value) && (value as number) >= 0 && (value as number) <= max ? value as number : fail();
const array = <T>(value: unknown, max: number, parse: (value: unknown) => T): T[] => Array.isArray(value) && value.length <= max ? value.map(parse) : fail();
const oneOf = <T extends string>(value: unknown, options: readonly T[]): T => typeof value === "string" && options.includes(value as T) ? value as T : fail();
const optionalText = (value: unknown): string | null => value === undefined || value === null ? null : text(value);
function path(value: unknown, allowEmpty = false): string {
  const result = text(value, 4096);
  if ((result.length === 0 && !allowEmpty) || result.startsWith("/") || /^[A-Za-z]:/.test(result) || result.includes("\\") || (result.length > 0 && result.split("/").some(part => part === "" || part === "." || part === ".."))) return fail();
  return result;
}
function diagnostics(value: unknown): ProjectDiagnostic[] {
  return array(value, 256, v => { const r = record(v); return { code: id(r.code), message: text(r.message), path: nullable(r.path, text) } as ProjectDiagnostic; });
}
function conflict(value: unknown): LibraryConflictFile {
  const r = record(value); return { path: path(r.path), current_hash: id(r.current_hash) };
}
function partial(value: unknown) {
  if (value === null) return null;
  const r = record(value); return { unit: id(r.unit), have: integer(r.have), total: nullable(r.total, integer), reason: text(r.reason) };
}
function attachment(value: unknown) {
  const r = record(value);
  const relative_path = nullable(r.relative_path, path);
  return { attachment_id: id(r.attachment_id), original_name: text(r.original_name), stored_name: text(r.stored_name), media_type: optionalText(r.media_type), bytes: nullable(r.bytes, integer), version: optionalText(r.version), state: oneOf(r.state, ["not_downloaded", "downloaded", "over_limit", "failed"] as const), relative_path };
}
function item(value: unknown): LibraryItemSummary {
  const r = record(value);
  const container = nullable(r.container, v => { const c = record(v); return { container_id: id(c.container_id), label: text(c.label) }; });
  const folder = nullable(r.folder, v => { const f = record(v); return { origin_path: text(f.origin_path), git_working_tree: bool(f.git_working_tree), files: integer(f.files), bytes: integer(f.bytes), skipped_symlinks: integer(f.skipped_symlinks, 0xffffffff), skipped_special: integer(f.skipped_special, 0xffffffff), skipped_ignored: integer(f.skipped_ignored, 0xffffffff), skipped_other: integer(f.skipped_other, 0xffffffff) }; });
  return {
    item_id: id(r.item_id), logical_id: id(r.logical_id), kind: oneOf(r.kind, ["provider_snapshot", "folder_copy"] as const),
    provider_id: optionalText(r.provider_id), provider_instance: optionalText(r.provider_instance), resource_type: optionalText(r.resource_type), canonical_id: optionalText(r.canonical_id),
    container, parent_item_id: optionalText(r.parent_item_id), ancestors: array(r.ancestors, 5000, v => { const a = record(v); return { id: id(a.id), title: text(a.title) }; }),
    order: nullable(r.order, v => integer(v, 0xffffffff)), title: text(r.title), document_path: nullable(r.document_path, v => path(v)), item_path: path(r.item_path),
    source_url: optionalText(r.source_url), original_url: optionalText(r.original_url), source_revision: optionalText(r.source_revision), revision: id(r.revision),
    state: oneOf(r.state, ["fresh", "changed", "unknown", "removed_at_source", "conflict", "failed", "partial"] as const), partial: partial(r.partial),
    conflict: array(r.conflict, 5000, conflict), fetched_at: optionalText(r.fetched_at), checked_at: optionalText(r.checked_at), follow_id: optionalText(r.follow_id),
    attachments: array(r.attachments, 5000, attachment), folder, diagnostics: diagnostics(r.diagnostics),
  };
}
function follow(value: unknown): LibraryFollowSummary {
  const r = record(value); return { follow_id: id(r.follow_id), provider_id: id(r.provider_id), provider_instance: id(r.provider_instance), space_key: id(r.space_key), space_name: text(r.space_name), include_attachments: bool(r.include_attachments), page_count: integer(r.page_count, 0xffffffff), partial: partial(r.partial), excluded_page_ids: array(r.excluded_page_ids, 5000, id), last_refreshed_at: optionalText(r.last_refreshed_at), state: oneOf(r.state, ["fresh", "changed", "unknown", "removed_at_source", "conflict", "failed", "partial"] as const) };
}
function libraryRoot(value: unknown) {
  const r = record(value);
  if (r.kind !== "library" || typeof r.root_id !== "string" || !r.root_id.startsWith("library:")) return fail();
  return { root_id: id(r.root_id), kind: "library" as const, label: text(r.label), path: text(r.path), repository_id: id(r.repository_id), checkout_path: text(r.checkout_path), companion_id: nullable(r.companion_id, id) };
}
export function parseLibraryListing(value: unknown): LibraryListing {
  const r = record(value); const items = array(r.items, 5000, item); const follows = array(r.follows, 5000, follow);
  if (new Set(items.map(x => x.item_id)).size !== items.length || new Set(follows.map(x => x.follow_id)).size !== follows.length) return fail();
  return { root: libraryRoot(r.root), generation: id(r.generation), items, follows, next_offset: nullable(r.next_offset, v => integer(v, 0xffffffff)), diagnostics: diagnostics(r.diagnostics) };
}
export function parseLibraryResolveRequest(value: unknown): LibraryResolveRequest { const r = record(value); return { input: text(r.input, 16 * 1024), provider_id: optionalText(r.provider_id) }; }
export function parseLibraryResolution(value: unknown): LibraryResolution {
  const r = record(value); return { kind: oneOf(r.kind, ["artifact", "confluence_page", "confluence_space", "folder"] as const), provider_id: optionalText(r.provider_id), provider_instance: optionalText(r.provider_instance), title: text(r.title), canonical_id: optionalText(r.canonical_id), container_label: optionalText(r.container_label), existing_item_id: optionalText(r.existing_item_id), existing_follow_id: optionalText(r.existing_follow_id), page_count: nullable(r.page_count, v => integer(v, 0xffffffff)), git_working_tree: nullable(r.git_working_tree, bool), file_count: nullable(r.file_count, integer), diagnostics: diagnostics(r.diagnostics) };
}
export function parseLibraryConfluenceSpacesRequest(value: unknown): LibraryConfluenceSpacesRequest {
  const r = record(value); const provider_id = id(r.provider_id);
  return provider_id.length <= 128 ? { provider_id } : fail();
}
/** Every browsed space belongs to the requested provider and is a Confluence space. */
export function parseLibraryConfluenceSpaces(value: unknown, request: LibraryConfluenceSpacesRequest): LibraryResolution[] {
  return array(value, 10_000, parseLibraryResolution).map(space => space.kind === "confluence_space" && space.provider_id === request.provider_id && space.canonical_id !== null ? space : fail());
}
export function parseLibraryAddRequest(value: unknown): LibraryAddRequest {
  const r = record(value); const target = nullable(r.target, parseSpaceTarget);
  return { input: text(r.input, 16 * 1024), provider_id: optionalText(r.provider_id), hydrate_references: bool(r.hydrate_references), follow_space: bool(r.follow_space), download_attachments: bool(r.download_attachments), refresh_existing: bool(r.refresh_existing), label: optionalText(r.label), target };
}
export function parseLibraryRefreshRequest(value: unknown): LibraryRefreshRequest {
  const r = record(value);
  switch (r.scope) {
    case "items": return { scope: "items", item_ids: array(r.item_ids, 5000, id) };
    case "follow": return { scope: "follow", follow_id: id(r.follow_id) };
    case "container": return { scope: "container", provider_instance: id(r.provider_instance), container_id: id(r.container_id) };
    case "all": return { scope: "all" };
    default: return fail();
  }
}

export function parseLibraryAttachmentRequest(value: unknown): LibraryAttachmentRequest {
  const r = record(value);
  const attachmentIds = array(r.attachment_ids, 256, id);
  if (attachmentIds.length === 0 || new Set(attachmentIds).size !== attachmentIds.length) return fail();
  return {
    item_id: id(r.item_id),
    attachment_ids: attachmentIds,
    action: oneOf(r.action, ["download", "remove_downloaded"] as const),
  };
}
export function parseLibraryReplaceRequest(value: unknown): LibraryReplaceRequest { const r = record(value); return { item_id: id(r.item_id), confirmed: array(r.confirmed, 5000, conflict) }; }
export function parseLibraryRemoveRequest(value: unknown): LibraryRemoveRequest {
  const r = record(value);
  switch (r.mode) {
    case "item": return { mode: "item", item_id: id(r.item_id), expected_revision: id(r.expected_revision) };
    case "stop_following": return { mode: "stop_following", follow_id: id(r.follow_id) };
    case "follow": return { mode: "follow", follow_id: id(r.follow_id) };
    default: return fail();
  }
}
const opKinds = ["add", "refresh", "space_add", "space_update", "attachments"] as const;
const phaseStates = ["pending", "running", "done", "partial", "failed", "cancelled"] as const;
export function parseLibraryOperation(value: unknown): LibraryOperation {
  const r = record(value);
  const phases = array(r.phases, 256, v => { const p = record(v); return { phase: oneOf(p.phase, ["library", "space"] as const), state: oneOf(p.state, phaseStates), done: integer(p.done), total: nullable(p.total, integer), message: optionalText(p.message), error: nullable(p.error, e => { const x = record(e); return { code: id(x.code), message: text(x.message) }; }) }; });
  const report = nullable(r.report, v => { const x = record(v); return { new: integer(x.new), updated: integer(x.updated), unchanged: integer(x.unchanged), removed_at_source: integer(x.removed_at_source), partial: integer(x.partial), failed: integer(x.failed), conflict: integer(x.conflict), rows: array(x.rows, 256, row => { const a = record(row); return { item_id: nullable(a.item_id, id), follow_id: nullable(a.follow_id, id), title: text(a.title), outcome: oneOf(a.outcome, ["new", "updated", "unchanged", "removed_at_source", "partial", "failed", "conflict"] as const), reason: optionalText(a.reason) }; }), truncated_rows: bool(x.truncated_rows) }; });
  const space = nullable(r.space, v => { const x = record(v); return { space_id: id(x.space_id), copy_mode: nullable(x.copy_mode, m => oneOf(m, ["reflink", "copy", "mixed"] as const)), written: array(x.written, 5000, path), skipped_edited: array(x.skipped_edited, 5000, path), companion_root_id: optionalText(x.companion_root_id) }; });
  const target = nullable(r.target, parseSpaceTarget);
  return { operation_id: id(r.operation_id), kind: oneOf(r.kind, opKinds), phases, item_ids: array(r.item_ids, 1_000_000, id), report, space, target, cancel_requested: bool(r.cancel_requested), finished: bool(r.finished), created_at: text(r.created_at), updated_at: text(r.updated_at) };
}
export function matchLibraryOperation(value: LibraryOperation, operationId: string): LibraryOperation { return value.operation_id === operationId ? value : fail(); }
export function matchLibraryAttachmentsOperation(value: LibraryOperation, request: LibraryAttachmentRequest): LibraryOperation {
  return value.kind === "attachments" && value.item_ids.length <= 1
    && (value.item_ids.length === 0 || value.item_ids[0] === request.item_id) ? value : fail();
}
export function parseLibraryOperationId(value: unknown): string { return id(value); }
export function parseLibraryDirectoryRequest(value: unknown): LibraryDirectoryRequest { const r = record(value); return { path: path(r.path, true), offset: nullable(r.offset, v => integer(v, 0xffffffff)), revision: nullable(r.revision, id) }; }
export function parseLibraryDocumentRequest(value: unknown): LibraryDocumentRequest { const r = record(value); return { path: path(r.path), expected_revision: nullable(r.expected_revision, id), offset: nullable(r.offset, integer) }; }
export function parseLibraryMediaRequest(value: unknown): LibraryMediaRequest { const r = record(value); return { path: path(r.path), expected_revision: nullable(r.expected_revision, id) }; }
export function parseLibraryDirectory(value: unknown): ContextDirectory {
  const parsed = parseContextDirectory(value);
  if (parsed.binding_id !== "library" || !parsed.root_id.startsWith("library:") || parsed.entries.length > 10_000) return fail();
  return parsed;
}
export function matchLibraryDirectory(value: ContextDirectory, request: LibraryDirectoryRequest): ContextDirectory { return value.binding_id === "library" && value.root_id.startsWith("library:") && value.path === request.path && (request.revision === null || value.revision === request.revision) ? value : fail(); }
export function parseLibraryDocument(value: unknown): ContextDocument { const parsed = parseContextDocument(value); if (parsed.binding_id !== "library" || !parsed.root_id.startsWith("library:")) return fail(); return parsed; }
export function matchLibraryDocument(value: ContextDocument, request: LibraryDocumentRequest): ContextDocument { return value.binding_id === "library" && value.root_id.startsWith("library:") && value.path === request.path && (request.expected_revision === null || value.revision === request.expected_revision) && (request.offset === null || value.offset === request.offset) ? value : fail(); }
export function parseLibraryMedia(value: unknown): ContextMedia { const parsed = parseContextMedia(value); if (parsed.binding_id !== "library" || !parsed.root_id.startsWith("library:")) return fail(); return parsed; }
export function matchLibraryMedia(value: ContextMedia, request: LibraryMediaRequest): ContextMedia { return value.binding_id === "library" && value.root_id.startsWith("library:") && value.path === request.path && (request.expected_revision === null || value.revision === request.expected_revision) ? value : fail(); }

function parseSpaceTarget(value: unknown): SpaceTarget {
  const r = record(value);
  return { session_id: validateSessionId(id(r.session_id)), space_id: validateResourceId(id(r.space_id)) };
}
function sameTarget(left: SpaceTarget, right: SpaceTarget): boolean {
  return left.session_id === right.session_id && left.space_id === right.space_id;
}
function errorResponse(value: unknown) {
  const r = record(value);
  return { code: id(r.code), message: text(r.message) };
}
export function parseSpaceContextRequest(value: unknown): SpaceContextRequest {
  return { target: parseSpaceTarget(record(value).target) };
}
export function parseSpaceAddRequest(value: unknown): SpaceAddRequest {
  const r = record(value);
  const item_ids = array(r.item_ids, 5000, id);
  const follow_ids = array(r.follow_ids, 5000, id);
  if (item_ids.length + follow_ids.length > 5000) return fail();
  return { target: parseSpaceTarget(r.target), item_ids, follow_ids };
}
export function parseSpaceAttemptsDismissRequest(value: unknown): SpaceAttemptsDismissRequest {
  return parseSpaceAddRequest(value);
}
export function parseSpaceUpdateRequest(value: unknown): SpaceUpdateRequest {
  const r = record(value);
  const s = record(r.scope);
  const scope = s.scope === "all"
    ? { scope: "all" as const }
    : s.scope === "selection"
      ? { scope: "selection" as const, item_ids: array(s.item_ids, 5000, id), follow_ids: array(s.follow_ids, 5000, id) }
      : fail();
  if (scope.scope === "selection" && scope.item_ids.length + scope.follow_ids.length > 5000) return fail();
  return { target: parseSpaceTarget(r.target), scope, replace_edited: array(r.replace_edited, 5000, conflict) };
}
export function parseSpaceRemoveRequest(value: unknown): SpaceRemoveRequest {
  const r = record(value);
  return { target: parseSpaceTarget(r.target), logical_id: id(r.logical_id), confirmed: array(r.confirmed, 5000, conflict) };
}
export function parseSpaceContextListing(value: unknown): SpaceContextListing {
  const r = record(value);
  const target = parseSpaceTarget(r.target);
  const c = record(r.companion);
  const companion = c.status === "available"
    ? { status: "available" as const, companion_root_id: id(c.companion_root_id), companion_label: text(c.companion_label) }
    : c.status === "unavailable"
      ? { status: "unavailable" as const, error: errorResponse(c.error) }
      : fail();
  const attempts = array(r.attempts, 256, value => {
    const a = record(value);
    const attemptTarget = parseSpaceTarget(a.target);
    if (!sameTarget(attemptTarget, target)) return fail();
    if ((a.item_id === null) === (a.follow_id === null)) return fail();
    return {
      target: attemptTarget, space_label: optionalText(a.space_label),
      item_id: nullable(a.item_id, id), follow_id: nullable(a.follow_id, id),
      title: text(a.title), state: oneOf(a.state, ["pending", "failed"] as const),
      error: nullable(a.error, errorResponse), operation_id: id(a.operation_id), updated_at: text(a.updated_at),
    };
  });
  const rows = array(r.rows, 20000, value => {
    const a = record(value);
    return {
      item_id: nullable(a.item_id, id), logical_id: id(a.logical_id), title: text(a.title),
      provider_id: nullable(a.provider_id, id), resource_type: nullable(a.resource_type, id),
      kind: oneOf(a.kind, ["provider_snapshot", "folder_copy"] as const),
      state: oneOf(a.state, ["up_to_date", "library_newer", "edited_in_space", "removed_at_source", "missing_in_space", "not_in_library", "not_linked"] as const),
      library_newer: bool(a.library_newer), paths: array(a.paths, 5000, value => path(value)),
      edited: array(a.edited, 5000, conflict),
      copy_mode: nullable(a.copy_mode, v => oneOf(v, ["reflink", "copy", "mixed"] as const)),
      library_revision_copied: nullable(a.library_revision_copied, id),
      current_library_revision: nullable(a.current_library_revision, id),
      follow: nullable(a.follow, value => {
        const f = record(value);
        return { follow_id: id(f.follow_id), space_key: id(f.space_key), page_count: integer(f.page_count), new_pages: integer(f.new_pages), changed_pages: integer(f.changed_pages), edited_pages: integer(f.edited_pages), removed_at_source_pages: integer(f.removed_at_source_pages) };
      }),
    };
  });
  return { target, companion, attempts, rows, behind: integer(r.behind), diagnostics: diagnostics(r.diagnostics) };
}
export function matchSpaceContextListing(value: SpaceContextListing, request: SpaceContextRequest): SpaceContextListing {
  return sameTarget(value.target, request.target) ? value : fail();
}
export function matchSpaceOperation(value: LibraryOperation, target: SpaceTarget): LibraryOperation {
  return value.target && sameTarget(value.target, target) ? value : fail();
}
export function parseSpaceAttemptsDismissed(value: unknown): void {
  if (value !== null) fail();
}
