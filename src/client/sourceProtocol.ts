import type { SourceImportRequest, SourceRefreshRequest, SourceImportResponse, SourceEntry } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
const fail = (): never => { throw new CockpitClientError("malformed_response", "Invalid source response"); };
const object = (value: unknown): Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : fail();
const text = (value: unknown, max = 4096): string => typeof value === "string" && value.length <= max && !value.includes("\0") ? value : fail();
const id = (value: unknown): string => { const result = text(value, 512); return result.length && !/[\x00-\x1f\x7f]/.test(result) ? result : fail(); };
const hydration = (value: unknown): boolean => value === undefined ? false : typeof value === "boolean" ? value : fail();
const optional = (value: unknown): string | null => value === null ? null : text(value);
const values = (value: unknown, choices: string[]): string => typeof value === "string" && choices.includes(value) ? value : fail();
export function parseSourceScope(value: unknown): { binding_id: string; root_id: string } { const r = object(value); return { binding_id: id(r.binding_id), root_id: id(r.root_id) }; }
export function parseSourceImport(value: unknown): SourceImportRequest { const r = object(value); return { ...parseSourceScope(r), provider_id: id(r.provider_id), artifact_url: text(r.artifact_url), hydrate_references: hydration(r.hydrate_references) }; }
export function parseSourceRefresh(value: unknown): SourceRefreshRequest { const r = object(value); return { ...parseSourceScope(r), source_id: id(r.source_id), hydrate_references: hydration(r.hydrate_references) }; }
export function parseSourceResponse(value: unknown): SourceImportResponse {
  const r = object(value);
  if (!Array.isArray(r.entries) || r.entries.length > 256 || !Array.isArray(r.diagnostics) || r.diagnostics.length > 256) return fail();
  const entries: SourceEntry[] = r.entries.map(value => {
    const e = object(value); const path = optional(e.relative_path);
    if (path !== null && (path.startsWith("/") || path.split("/").some(part => !part || part === "." || part === ".."))) return fail();
    return { source_id: id(e.source_id), provider_id: id(e.provider_id), provider_instance: text(e.provider_instance), resource_type: id(e.resource_type), canonical_id: text(e.canonical_id), title: text(e.title), source_url: optional(e.source_url), original_url: optional(e.original_url), source_revision: optional(e.source_revision), content_hash: id(e.content_hash), freshness: values(e.freshness, ["fresh", "changed", "unknown", "unavailable", "conflict"]) as SourceEntry["freshness"], status: values(e.status, ["materialized", "unchanged", "conflict", "unsupported", "failed"]) as SourceEntry["status"], relative_path: path };
  });
  if (new Set(entries.map(entry => entry.source_id)).size !== entries.length) return fail();
  return { ...parseSourceScope(r), entries, diagnostics: r.diagnostics.map(value => { const d = object(value); return { code: id(d.code), message: text(d.message), path: optional(d.path) }; }) };
}
export function matchSourceResponse(value: SourceImportResponse, request: { binding_id: string; root_id: string }): SourceImportResponse { return value.binding_id === request.binding_id && value.root_id === request.root_id ? value : fail(); }
