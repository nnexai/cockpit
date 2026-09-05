import type { ContextSnapshotRequest, ContextSnapshotResponse, ProjectDiagnostic } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
const fail = (): never => { throw new CockpitClientError("malformed_response", "Malformed local snapshot response"); };
const record = (v: unknown): Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v) ? v as Record<string, unknown> : fail();
const text = (v: unknown): string => typeof v === "string" && v.length > 0 && v.length <= 4096 && !/[\x00-\x1f\x7f]/.test(v) ? v : fail();
const integer = (v: unknown, max: number): number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= max ? v : fail();
export function parseContextSnapshotRequest(value: unknown): ContextSnapshotRequest {
  const r = record(value); if (r.mode !== "working_tree") return fail();
  return { binding_id: text(r.binding_id), root_id: text(r.root_id), repository_id: text(r.repository_id), mode: r.mode };
}
export function parseContextSnapshotResponse(value: unknown): ContextSnapshotResponse {
  const r = record(value); const identity = parseContextSnapshotRequest(r);
  const path = text(r.snapshot_path);
  if (path.startsWith("/") || path.includes("\\") || path.split("/").some(part => !part || part === "." || part === "..") || !path.startsWith("repos/")
    || !["copy", "reflink", "mixed"].includes(String(r.copy_mode)) || typeof r.dirty !== "boolean" || !Array.isArray(r.diagnostics) || r.diagnostics.length > 1024) return fail();
  const diagnostics: ProjectDiagnostic[] = r.diagnostics.map(item => { const d = record(item); return { code: text(d.code), message: text(d.message), path: d.path === null ? null : text(d.path) }; });
  return { ...identity, snapshot_path: path, generation: text(r.generation), copy_mode: r.copy_mode as ContextSnapshotResponse["copy_mode"], files: integer(r.files, 512), bytes: integer(r.bytes, 32 * 1024 * 1024), source_head: r.source_head === null ? null : text(r.source_head), dirty: r.dirty, diagnostics };
}
export function matchContextSnapshot(response: ContextSnapshotResponse, request: ContextSnapshotRequest): ContextSnapshotResponse {
  if (response.binding_id !== request.binding_id || response.root_id !== request.root_id || response.repository_id !== request.repository_id || response.mode !== request.mode) return fail();
  return response;
}
