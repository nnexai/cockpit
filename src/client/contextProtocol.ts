import type { ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest, ContextFileIndex, ContextFileIndexRequest, ViewerSourceOptions, ViewerSourceSelector, ViewerOpenRequest, ViewerContext } from "../protocol/generated/v1";
import { wireContextDirectory, wireContextDirectoryRequest, wireContextDocument, wireContextDocumentRequest, wireContextFileIndex, wireContextFileIndexRequest, wireViewerSourceOptions, wireViewerSourceSelector, wireViewerOpenRequest, wireViewerContext, type WireFields } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, messageTable, parseWire } from "./wire";

const identity = (v: string): boolean => v.length > 0 && v.length <= 512 && !v.includes("\0");
const relativePath = (v: string, empty = false): boolean => v.length <= 4096 && !v.includes("\0") && ((empty && v === "") || (v.length > 0 && v.split("/").every(p => p !== "" && p !== "." && p !== "..")));
function fields<K extends keyof WireFields>(names: readonly K[], check: (v: WireFields[K]) => boolean) {
  const unary = (v: WireFields[K]): boolean => check(v);
  return Object.fromEntries(names.map(name => [name, unary])) as Record<K, (v: WireFields[K]) => boolean>;
}
function malformed(label: string): never { throw new CockpitClientError("malformed_response", `Invalid ${label}`); }
const POLICY = definePolicy({
  message: messageTable({
    ViewerSourceOptions: f => `Invalid viewer source ${f.refinement === "identity" ? "root identity" : "options"}`, ViewerSourceSelector: "Invalid viewer source selector", ViewerOpenRequest: "Invalid viewer open request", ViewerContext: f => `Invalid viewer context${f.refinement === "identity" ? " root identity" : ""}`,
    ContextDirectoryRequest: f => `Invalid Context directory ${f.path.find(p => p.type === "ContextDirectoryRequest")?.field === "offset" ? "continuation" : f.path.find(p => p.type === "ContextDirectoryRequest")?.field === "revision" ? "revision" : "request"}`,
    ContextFileIndexRequest: "Invalid Context file-index request", ContextFileIndex: "Invalid Context file index", ContextDocumentRequest: "Invalid Context document request",
    ContextDirectory: f => `Invalid ${f.refinement === "unique" ? "duplicate Context entry identities" : ({ revision: "Context directory revision", next_offset: "Context directory continuation", total_entries: "Context directory count" } as Record<string, string>)[f.path.find(p => p.type === "ContextDirectory")?.field ?? ""] ?? "Context directory"}`,
    ContextDocument: f => `Invalid ${({ offset: "Context document offset", next_offset: "Context document next_offset", line_offset: "Context document line_offset", total_bytes: "Context document total bytes" } as Record<string, string>)[f.path.find(p => p.type === "ContextDocument")?.field ?? ""] ?? "Context document"}`,
  }, "Invalid Context document"),
  wire: {
    raw: new Set(["ViewerSourceOptions", "ViewerContext", "ContextFileIndexRequest", "ContextFileIndex", "ContextDirectory", "ContextDocument"]),
    nullish: new Set(["ContextDirectoryRequest.offset", "ContextDirectoryRequest.revision", "ContextDocumentRequest.offset", "ContextDirectory.revision", "ContextDirectory.next_offset", "ContextDirectory.total_entries", "ContextDocument.offset", "ContextDocument.next_offset", "ContextDocument.line_offset", "ContextDocument.total_bytes"]),
    order: {
      ViewerOpenRequest: ["tab_id", "source_pane_id", "client_id", "kind", "source"],
      ContextDirectoryRequest: ["binding_id", "root_id", "path", "offset", "revision"],
      ContextDirectory: ["binding_id", "root_id", "path", "entries", "truncated", "diagnostics", "check:unique", "revision", "next_offset", "total_entries"],
      ContextDocument: ["binding_id", "root_id", "path", "revision", "content_hash", "bytes", "media_type", "text", "truncated", "diagnostics", "check:content", "offset", "next_offset", "line_offset", "total_bytes"],
    },
    emit: { ViewerOpenRequest: ["tab_id", "source_pane_id", "client_id", "kind", "source"] },
    lengths: { "ContextFileIndex.files": { max: 50_000 }, "ContextDirectory.entries": { max: 10_000 } },
    fields: {
      ...fields(["ContextRoot.root_id", "ContextRoot.repository_id", "ViewerSourceOptions.session_id", "ViewerSourceOptions.pane_id", "ViewerSourceOptions.tab_id", "ViewerSourceOptions.space_id", "ViewerSourceSelector[files_repository].root_id", "ViewerSourceSelector[review].repository_id", "ViewerOpenRequest.tab_id", "ViewerOpenRequest.source_pane_id", "ViewerOpenRequest.client_id", "ViewerContext.session_id", "ViewerContext.viewer_id", "ViewerContext.binding_id", "ViewerContext.tab_id", "ViewerContext.space_id", "ViewerContext.source_id", "ContextDirectoryRequest.binding_id", "ContextDirectoryRequest.root_id", "ContextFileIndexRequest.binding_id", "ContextFileIndexRequest.root_id", "ContextFileIndex.binding_id", "ContextFileIndex.root_id", "ContextDirectory.binding_id", "ContextDirectory.root_id", "ContextDocumentRequest.binding_id", "ContextDocumentRequest.root_id", "ContextDocument.binding_id", "ContextDocument.root_id", "ContextDocument.revision", "ContextEntry.entry_id"], identity),
      ...fields(["ContextIndexedFile.path", "ContextDocumentRequest.path", "ContextDocument.path"], relativePath),
      ...fields(["ContextDirectoryRequest.path", "ContextDirectory.path"], v => relativePath(v, true)),
      "ViewerSourceOptions.review_repository_ids": v => v.every(identity), "ContextEntry.path": v => v === null || relativePath(v),
      "ContextDirectoryRequest.offset": v => v === undefined || v <= 100_000, "ContextDirectoryRequest.revision": v => v === undefined || identity(v),
      "ContextDirectory.next_offset": v => v === undefined || v <= 100_000,
    },
    checks: {
      ContextDirectory: { unique: v => new Set(v.entries.map(x => x.entry_id)).size === v.entries.length },
      ContextDocument: { content: v => (!v.truncated || v.content_hash === null) && (v.text === null || v.text.length <= 8 * 1024 * 1024) },
      ViewerSourceOptions: {
        identity: v => new Set(v.roots.map(x => x.root_id)).size === v.roots.length && new Set(v.review_repository_ids).size === v.review_repository_ids.length
          && (v.files_context_root_id === null || v.roots.some(x => x.root_id === v.files_context_root_id && x.kind === "library"))
          && (v.files_folder_root_id === null || v.roots.some(x => x.root_id === v.files_folder_root_id && x.kind === "folder"))
          && v.review_repository_ids.every(id => v.roots.some(x => x.repository_id === id && x.kind === "repository")),
      },
      ViewerContext: {
        source: v => (v.kind === "review") === (v.source_kind === "review"),
        identity: v => {
          const ids = v.roots.map(x => x.root_id);
          return new Set(ids).size === ids.length && (v.default_root_id === null || ids.includes(v.default_root_id));
        },
      },
    },
  },
});
export function parseViewerSourceOptions(value: unknown): ViewerSourceOptions { return parseWire(value, wireViewerSourceOptions, POLICY); }
export function matchViewerSourceOptions(value: ViewerSourceOptions, sessionId: string, paneId: string): ViewerSourceOptions {
  if (value.session_id !== sessionId || value.pane_id !== paneId) malformed("viewer source options identity");
  return value;
}
export function parseViewerSourceSelector(value: unknown): ViewerSourceSelector { return parseWire(value, wireViewerSourceSelector, POLICY); }
export function parseViewerOpenRequest(value: unknown): ViewerOpenRequest {
  const v = parseWire(value, wireViewerOpenRequest, POLICY);
  if ((v.kind === "review") !== (v.source.kind === "review")) malformed("viewer source kind");
  return v;
}
export function parseViewerContext(value: unknown): ViewerContext { return parseWire(value, wireViewerContext, POLICY); }
export function matchViewerContext(value: ViewerContext, sessionId: string, request: ViewerOpenRequest): ViewerContext {
  if (value.session_id !== sessionId || value.tab_id !== request.tab_id || value.kind !== request.kind) malformed("viewer context identity");
  const source = request.source;
  if (source.kind === "review" && !value.roots.some(x => x.kind === "repository" && x.repository_id === source.repository_id)) malformed("viewer context repository identity");
  if (source.kind === "files_repository" && !value.roots.some(x => x.kind === "repository" && x.root_id === source.root_id)) malformed("viewer context selected repository identity");
  return value;
}
export function parseContextDirectoryRequest(value: unknown): ContextDirectoryRequest {
  const v = parseWire(value, wireContextDirectoryRequest, POLICY);
  return { binding_id: v.binding_id, root_id: v.root_id, path: v.path, offset: v.offset, revision: v.revision };
}
export function parseContextFileIndexRequest(value: unknown): ContextFileIndexRequest { return parseWire(value, wireContextFileIndexRequest, POLICY); }
export function parseContextFileIndex(value: unknown): ContextFileIndex { return parseWire(value, wireContextFileIndex, POLICY); }
export function parseContextDocumentRequest(value: unknown): ContextDocumentRequest {
  const directory = parseContextDirectoryRequest(value);
  if (!relativePath(directory.path)) malformed("Context document request");
  const v = parseWire(value, wireContextDocumentRequest, POLICY);
  return { ...v, offset: v.offset };
}
export function parseContextDirectory(value: unknown): ContextDirectory {
  const v = parseWire(value, wireContextDirectory, POLICY);
  return { ...v, revision: v.revision ?? undefined, next_offset: v.next_offset ?? undefined, total_entries: v.total_entries ?? undefined };
}
export function parseContextDocument(value: unknown): ContextDocument {
  const v = parseWire(value, wireContextDocument, POLICY);
  return { ...v, offset: v.offset ?? undefined, next_offset: v.next_offset ?? undefined, line_offset: v.line_offset ?? undefined, total_bytes: v.total_bytes ?? undefined };
}
export function matchContextResponse<T extends { binding_id: string; root_id: string; path: string }>(value: T, request: ContextDirectoryRequest): T {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id || value.path !== request.path) malformed("Context response identity");
  return value;
}
