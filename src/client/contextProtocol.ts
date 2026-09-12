import type {
  ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
  ContextEntry, ContextLaunchRequest, ContextRoot, PanePresentation,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const nullableText = (value: unknown): value is string | null => value === null || text(value);
const identity = (value: unknown): value is string => text(value) && value.length > 0 && value.length <= 512 && !value.includes("\0");
const bytes = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const diagnostics = (value: unknown): boolean => Array.isArray(value) && value.every((item) => record(item) && text(item.code) && text(item.message) && nullableText(item.path));

function malformed(label: string): never {
  throw new CockpitClientError("malformed_response", `Invalid ${label}`);
}

function relativePath(value: unknown, allowEmpty: boolean): value is string {
  return text(value) && value.length <= 4096 && !value.includes("\0")
    && ((allowEmpty && value === "") || (value.length > 0 && value.split("/").every((part) => part !== "" && part !== "." && part !== "..")));
}

function root(value: unknown): value is ContextRoot {
  return record(value) && identity(value.root_id) && (value.kind === "repository" || value.kind === "companion" || value.kind === "folder")
    && text(value.label) && text(value.path) && identity(value.repository_id)
    && text(value.checkout_path) && nullableText(value.companion_id)
    && (value.kind === "companion" ? identity(value.companion_id) : value.companion_id === null);
}

export function parsePanePresentation(value: unknown): PanePresentation {
  if (!record(value) || !identity(value.session_id) || !identity(value.pane_id) || !identity(value.terminal_id)
    || !identity(value.binding_id) || !(value.extension === null || value.extension === "context" || value.extension === "review")
    || !(value.renderer === null || (value.renderer === value.extension && (value.renderer === "context" || value.renderer === "review")))
    || !text(value.confidence) || !["verified_launch", "verified_process", "candidate", "none", "unsupported"].includes(value.confidence)
    || !text(value.reason) || !Array.isArray(value.roots) || !value.roots.every(root)
    || !nullableText(value.default_root_id) || typeof value.can_open_context !== "boolean"
    || typeof value.can_open_files !== "boolean" || !nullableText(value.files_root_id)
    || typeof value.can_open_review !== "boolean" || !diagnostics(value.diagnostics)) malformed("pane presentation");
  const ids = value.roots.map((item) => item.root_id);
  const filesRoot = value.files_root_id === null
    ? undefined
    : value.roots.find((item) => item.root_id === value.files_root_id);
  if (new Set(ids).size !== ids.length || (value.default_root_id !== null && !ids.includes(value.default_root_id))
    || (value.can_open_files && filesRoot?.kind !== "folder")
    || (!value.can_open_files && value.files_root_id !== null)) malformed("Context root identity");
  return value as unknown as PanePresentation;
}

export function matchPanePresentation(value: PanePresentation, sessionId: string, paneId?: string): PanePresentation {
  if (value.session_id !== sessionId || (paneId !== undefined && value.pane_id !== paneId)) malformed("pane presentation identity");
  return value;
}

export function parseContextDirectoryRequest(value: unknown): ContextDirectoryRequest {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !relativePath(value.path, true)) malformed("Context directory request");
  return value as unknown as ContextDirectoryRequest;
}

export function parseContextDocumentRequest(value: unknown): ContextDocumentRequest {
  const request = parseContextDirectoryRequest(value);
  if (!relativePath(request.path, false) || !record(value) || !nullableText(value.expected_revision)) malformed("Context document request");
  return value as unknown as ContextDocumentRequest;
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
  if (value.next_offset !== undefined && !bytes(value.next_offset)) malformed("Context directory continuation");
  if (value.total_entries !== undefined && value.total_entries !== null && !bytes(value.total_entries)) malformed("Context directory count");
  return value as unknown as ContextDirectory;
}

export function parseContextDocument(value: unknown): ContextDocument {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !relativePath(value.path, false)
    || !identity(value.revision) || !nullableText(value.content_hash) || !bytes(value.bytes)
    || !text(value.media_type) || !nullableText(value.text) || typeof value.truncated !== "boolean"
    || !diagnostics(value.diagnostics) || (value.truncated && value.content_hash !== null)
    || (text(value.text) && value.text.length > 8 * 1024 * 1024)) malformed("Context document");
  for (const [name, item] of [["offset", value.offset], ["next_offset", value.next_offset], ["line_offset", value.line_offset]] as const) {
    if (item !== undefined && !bytes(item)) malformed(`Context document ${name}`);
  }
  if (value.total_bytes !== undefined && value.total_bytes !== null && !bytes(value.total_bytes)) malformed("Context document total bytes");
  return value as unknown as ContextDocument;
}

export function matchContextResponse<T extends { binding_id: string; root_id: string; path: string }>(value: T, request: ContextDirectoryRequest): T {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id || value.path !== request.path) malformed("Context response identity");
  return value;
}

export function parseContextLaunchRequest(value: unknown): ContextLaunchRequest {
  if (!record(value) || !identity(value.pane_id) || !identity(value.binding_id) || !identity(value.root_id)
    || (value.direction !== "right" && value.direction !== "down")) malformed("Context launch request");
  return value as unknown as ContextLaunchRequest;
}
