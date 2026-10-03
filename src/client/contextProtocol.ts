import type {
  ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
  ContextEntry, ContextFileIndex, ContextFileIndexRequest,
  ContextRoot, ViewerSourceOptions, ViewerSourceSelector, ViewerOpenRequest, ViewerContext,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const nullableText = (value: unknown): value is string | null => value === null || text(value);
const identity = (value: unknown): value is string => text(value) && value.length > 0 && value.length <= 512 && !value.includes("\0");
const bytes = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const boundedOffset = (value: unknown): value is number => bytes(value) && value <= 100_000;
const diagnostics = (value: unknown): boolean => Array.isArray(value) && value.every((item) => record(item) && text(item.code) && text(item.message) && nullableText(item.path));

function malformed(label: string): never {
  throw new CockpitClientError("malformed_response", `Invalid ${label}`);
}

function relativePath(value: unknown, allowEmpty: boolean): value is string {
  return text(value) && value.length <= 4096 && !value.includes("\0")
    && ((allowEmpty && value === "") || (value.length > 0 && value.split("/").every((part) => part !== "" && part !== "." && part !== "..")));
}

function root(value: unknown): value is ContextRoot {
  return record(value) && identity(value.root_id) && (value.kind === "repository" || value.kind === "library" || value.kind === "folder")
    && text(value.label) && text(value.path) && identity(value.repository_id)
    && text(value.checkout_path);
}

export function parseViewerSourceOptions(value: unknown): ViewerSourceOptions {
  if (!record(value) || !identity(value.session_id) || !identity(value.pane_id) || !identity(value.tab_id)
    || !identity(value.space_id) || !nullableText(value.files_context_root_id) || !nullableText(value.files_folder_root_id)
    || !Array.isArray(value.review_repository_ids) || !value.review_repository_ids.every(identity)
    || !Array.isArray(value.roots) || !value.roots.every(root) || !text(value.reason)
    || !diagnostics(value.diagnostics)) malformed("viewer source options");
  const roots = value.roots;
  if (new Set(roots.map((item) => item.root_id)).size !== roots.length
    || new Set(value.review_repository_ids).size !== value.review_repository_ids.length
    || (value.files_context_root_id !== null && !roots.some((item) => item.root_id === value.files_context_root_id && item.kind === "library"))
    || (value.files_folder_root_id !== null && !roots.some((item) => item.root_id === value.files_folder_root_id && item.kind === "folder"))
    || value.review_repository_ids.some((id) => !roots.some((item) => item.repository_id === id && item.kind === "repository"))) {
    malformed("viewer source root identity");
  }
  return value as unknown as ViewerSourceOptions;
}

export function matchViewerSourceOptions(value: ViewerSourceOptions, sessionId: string, paneId: string): ViewerSourceOptions {
  if (value.session_id !== sessionId || value.pane_id !== paneId) malformed("viewer source options identity");
  return value;
}

export function parseViewerSourceSelector(value: unknown): ViewerSourceSelector {
  if (!record(value)) malformed("viewer source selector");
  if (value.kind === "files_context" || value.kind === "files_folder") return { kind: value.kind };
  if (value.kind === "files_repository" && identity(value.root_id)) return { kind: "files_repository", root_id: value.root_id };
  if (value.kind === "review" && identity(value.repository_id)) return { kind: "review", repository_id: value.repository_id };
  return malformed("viewer source selector");
}

export function parseViewerOpenRequest(value: unknown): ViewerOpenRequest {
  if (!record(value) || !identity(value.tab_id) || !identity(value.source_pane_id)
    || !identity(value.client_id) || (value.kind !== "files" && value.kind !== "review")) malformed("viewer open request");
  const source = parseViewerSourceSelector(value.source);
  if ((value.kind === "review") !== (source.kind === "review")) malformed("viewer source kind");
  return { tab_id: value.tab_id, source_pane_id: value.source_pane_id, client_id: value.client_id, kind: value.kind, source };
}

export function parseViewerContext(value: unknown): ViewerContext {
  if (!record(value) || !identity(value.session_id) || !identity(value.viewer_id) || !identity(value.binding_id)
    || !identity(value.tab_id) || !identity(value.space_id) || !identity(value.source_id)
    || (value.kind !== "files" && value.kind !== "review")
    || (value.source_kind !== "context" && value.source_kind !== "review")
    || ((value.kind === "review") !== (value.source_kind === "review"))
    || !Array.isArray(value.roots) || !value.roots.every(root)
    || !nullableText(value.default_root_id) || !diagnostics(value.diagnostics)) malformed("viewer context");
  const ids = value.roots.map((item) => item.root_id);
  if (new Set(ids).size !== ids.length || (value.default_root_id !== null && !ids.includes(value.default_root_id))) {
    malformed("viewer context root identity");
  }
  return value as unknown as ViewerContext;
}

export function matchViewerContext(value: ViewerContext, sessionId: string, request: ViewerOpenRequest): ViewerContext {
  if (value.session_id !== sessionId || value.tab_id !== request.tab_id || value.kind !== request.kind) malformed("viewer context identity");
  const source = request.source;
  if (source.kind === "review" && !value.roots.some((item) => item.kind === "repository" && item.repository_id === source.repository_id)) {
    malformed("viewer context repository identity");
  }
  if (source.kind === "files_repository" && !value.roots.some((item) => item.kind === "repository" && item.root_id === source.root_id)) {
    malformed("viewer context selected repository identity");
  }
  return value;
}

export function parseContextDirectoryRequest(value: unknown): ContextDirectoryRequest {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !relativePath(value.path, true)) malformed("Context directory request");
  if (value.offset !== undefined && value.offset !== null && !boundedOffset(value.offset)) malformed("Context directory continuation");
  if (value.revision !== undefined && value.revision !== null && !identity(value.revision)) malformed("Context directory revision");
  return {
    binding_id: value.binding_id as string,
    root_id: value.root_id as string,
    path: value.path as string,
    offset: value.offset === undefined || value.offset === null ? undefined : value.offset,
    revision: value.revision === undefined || value.revision === null ? undefined : value.revision,
  };
}

export function parseContextFileIndexRequest(value: unknown): ContextFileIndexRequest {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id)
    || (value.mode !== "cached" && value.mode !== "fresh")) malformed("Context file-index request");
  return value as unknown as ContextFileIndexRequest;
}

export function parseContextFileIndex(value: unknown): ContextFileIndex {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id)
    || !Array.isArray(value.files) || value.files.length > 50_000
    || !value.files.every((file) => record(file) && relativePath(file.path, false) && (file.bytes === null || bytes(file.bytes)))
    || typeof value.truncated !== "boolean" || !["git", "walk"].includes(String(value.source))
    || !["fresh", "cached", "miss"].includes(String(value.state)) || !diagnostics(value.diagnostics)) malformed("Context file index");
  return value as unknown as ContextFileIndex;
}

export function parseContextDocumentRequest(value: unknown): ContextDocumentRequest {
  const request = parseContextDirectoryRequest(value);
  if (!relativePath(request.path, false) || !record(value) || !nullableText(value.expected_revision)) malformed("Context document request");
  if (value.offset !== undefined && value.offset !== null && !bytes(value.offset)) malformed("Context document continuation");
  return {
    binding_id: value.binding_id as string,
    root_id: value.root_id as string,
    path: value.path as string,
    expected_revision: value.expected_revision as string | null,
    offset: value.offset === undefined || value.offset === null ? undefined : value.offset,
  };
}

function entry(value: unknown): value is ContextEntry {
  return record(value) && identity(value.entry_id) && text(value.name)
    && (value.path === null || relativePath(value.path, false))
    && text(value.kind) && ["directory", "file", "symlink", "other"].includes(value.kind)
    && (value.bytes === null || bytes(value.bytes)) && text(value.revision) && nullableText(value.refusal);
}

export function parseContextDirectory(value: unknown): ContextDirectory {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !relativePath(value.path, true)
    || !Array.isArray(value.entries) || value.entries.length > 10_000 || !value.entries.every(entry)
    || typeof value.truncated !== "boolean" || !diagnostics(value.diagnostics)) malformed("Context directory");
  const ids = value.entries.map((item) => item.entry_id);
  if (new Set(ids).size !== ids.length) malformed("duplicate Context entry identities");
  if (value.revision !== undefined && !nullableText(value.revision)) malformed("Context directory revision");
  if (value.next_offset !== undefined && value.next_offset !== null && !boundedOffset(value.next_offset)) malformed("Context directory continuation");
  if (value.total_entries !== undefined && value.total_entries !== null && !bytes(value.total_entries)) malformed("Context directory count");
  return {
    ...value,
    revision: value.revision === undefined || value.revision === null ? undefined : value.revision,
    next_offset: value.next_offset === undefined || value.next_offset === null ? undefined : value.next_offset,
    total_entries: value.total_entries === undefined || value.total_entries === null ? undefined : value.total_entries,
  } as unknown as ContextDirectory;
}

export function parseContextDocument(value: unknown): ContextDocument {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !relativePath(value.path, false)
    || !identity(value.revision) || !nullableText(value.content_hash) || !bytes(value.bytes)
    || !text(value.media_type) || !nullableText(value.text) || typeof value.truncated !== "boolean"
    || !diagnostics(value.diagnostics) || (value.truncated && value.content_hash !== null)
    || (text(value.text) && value.text.length > 8 * 1024 * 1024)) malformed("Context document");
  for (const [name, item] of [["offset", value.offset], ["next_offset", value.next_offset], ["line_offset", value.line_offset]] as const) {
    if (item !== undefined && item !== null && !bytes(item)) malformed(`Context document ${name}`);
  }
  if (value.total_bytes !== undefined && value.total_bytes !== null && !bytes(value.total_bytes)) malformed("Context document total bytes");
  return {
    ...value,
    offset: value.offset === undefined || value.offset === null ? undefined : value.offset,
    next_offset: value.next_offset === undefined || value.next_offset === null ? undefined : value.next_offset,
    line_offset: value.line_offset === undefined || value.line_offset === null ? undefined : value.line_offset,
    total_bytes: value.total_bytes === null ? undefined : value.total_bytes,
  } as unknown as ContextDocument;
}

export function matchContextResponse<T extends { binding_id: string; root_id: string; path: string }>(value: T, request: ContextDirectoryRequest): T {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id || value.path !== request.path) malformed("Context response identity");
  return value;
}

