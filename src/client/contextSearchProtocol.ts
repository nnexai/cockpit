import type { ContextInvalidationRequest, ContextInvalidationResponse, ContextSearchRequest, ContextSearchResponse } from "../protocol/generated/v1";
import { wireContextInvalidationRequest, wireContextInvalidationResponse, wireContextSearchRequest, wireContextSearchResponse } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, messageTable, parseWire } from "./wire";

const encoder = new TextEncoder();
const identity = (v: string): boolean => v.length > 0 && v.length <= 4096 && !v.includes("\0");
const relativePath = (v: string): boolean => identity(v) && v.split("/").every(p => p !== "" && p !== "." && p !== "..");
const POLICY = definePolicy({
  message: messageTable({
    ContextSearchRequest: f => `Invalid Context ${f.path.find(p => p.type === "ContextSearchRequest")?.field === "offset" ? "search continuation" : f.path.find(p => p.type === "ContextSearchRequest")?.field === "revision" ? "search revision" : "search request"}`,
    ContextSearchResponse: f => `Invalid Context ${({ revision: "search revision", next_offset: "search continuation", partial_reason: "search partial reason" } as Record<string, string>)[f.path.find(p => p.type === "ContextSearchResponse")?.field ?? ""] ?? "search response"}`,
    ContextSearchResult: "Invalid Context search result", ContextKnownRevision: "Invalid Context known revision",
    ContextInvalidation: f => `Invalid Context ${f.refinement === "changed" ? "changed invalidation" : "invalidation"}`,
    ContextInvalidationRequest: f => `Invalid Context ${f.refinement === "unique" ? "duplicate known path" : "invalidation request"}`,
    ContextInvalidationResponse: f => `Invalid Context ${f.refinement === "unique" ? "duplicate invalidation path" : "invalidation response"}`,
  }, "Invalid Context search response"),
  wire: {
    nullish: new Set(["ContextSearchRequest.revision", "ContextSearchResponse.revision", "ContextSearchResponse.next_offset", "ContextSearchResponse.partial_reason"]),
    order: {
      ContextSearchRequest: ["binding_id", "root_id", "query", "request_generation", "offset", "revision"],
      ContextSearchResponse: ["binding_id", "root_id", "query", "request_generation", "results:shallow", "scanned_files", "truncated", "results", "revision", "next_offset", "partial_reason"],
      ContextInvalidationRequest: ["binding_id", "root_id", "request_generation", "known:shallow", "known"],
      ContextInvalidationResponse: ["binding_id", "root_id", "request_generation", "invalidations:shallow", "truncated", "invalidations"],
    },
    lengths: { "ContextSearchResponse.results": { max: 1000 }, "ContextInvalidationRequest.known": { max: 128 }, "ContextInvalidationResponse.invalidations": { max: 128 } },
    fields: {
      "ContextSearchRequest.binding_id": identity, "ContextSearchRequest.root_id": identity,
      "ContextSearchRequest.query": v => v.length > 0 && encoder.encode(v).byteLength <= 256 && !/[\u0000-\u001f\u007f-\u009f]/.test(v),
      "ContextSearchRequest.request_generation": v => v > 0, "ContextSearchRequest.offset": v => v === undefined || (v !== null && v <= 100_000),
      "ContextSearchRequest.revision": v => v === undefined || identity(v),
      "ContextSearchResponse.binding_id": identity, "ContextSearchResponse.root_id": identity,
      "ContextSearchResponse.request_generation": v => v > 0, "ContextSearchResponse.scanned_files": v => v <= 100_000,
      "ContextSearchResponse.revision": v => v === undefined || identity(v), "ContextSearchResponse.next_offset": v => v === undefined || v <= 100_000,
      "ContextSearchResult.path": relativePath, "ContextSearchResult.line": v => v > 0,
      "ContextSearchResult.excerpt": v => encoder.encode(v).byteLength <= 512, "ContextSearchResult.revision": identity,
      "ContextKnownRevision.path": relativePath, "ContextKnownRevision.revision": identity,
      "ContextInvalidation.path": relativePath, "ContextInvalidation.revision": v => v === null || identity(v),
      "ContextInvalidationRequest.binding_id": identity, "ContextInvalidationRequest.root_id": identity, "ContextInvalidationRequest.request_generation": v => v > 0,
      "ContextInvalidationResponse.binding_id": identity, "ContextInvalidationResponse.root_id": identity, "ContextInvalidationResponse.request_generation": v => v > 0,
    },
    checks: {
      ContextInvalidation: { changed: v => v.state !== "changed" || v.revision !== null },
      ContextInvalidationRequest: { unique: v => new Set(v.known.map(x => x.path)).size === v.known.length },
      ContextInvalidationResponse: { unique: v => new Set(v.invalidations.map(x => x.path)).size === v.invalidations.length },
    },
  },
});
export function parseContextSearchRequest(value: unknown): ContextSearchRequest {
  const v = parseWire(value, wireContextSearchRequest, POLICY);
  return { binding_id: v.binding_id, root_id: v.root_id, query: v.query, request_generation: v.request_generation, offset: v.offset, revision: v.revision };
}
export function parseContextSearchResponse(value: unknown): ContextSearchResponse {
  const v = parseWire(value, wireContextSearchResponse, POLICY);
  return { binding_id: v.binding_id, root_id: v.root_id, query: v.query, request_generation: v.request_generation, results: v.results, scanned_files: v.scanned_files, truncated: v.truncated, revision: v.revision, next_offset: v.next_offset, partial_reason: v.partial_reason };
}
export function parseContextInvalidationRequest(value: unknown): ContextInvalidationRequest { return parseWire(value, wireContextInvalidationRequest, POLICY); }
export function parseContextInvalidationResponse(value: unknown): ContextInvalidationResponse { return parseWire(value, wireContextInvalidationResponse, POLICY); }
export function matchContextSearchResponse(value: ContextSearchResponse, request: ContextSearchRequest): ContextSearchResponse {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id || value.query !== request.query || value.request_generation !== request.request_generation) {
    throw new CockpitClientError("malformed_response", "Invalid Context search response identity");
  }
  return value;
}
export function matchContextInvalidationResponse(value: ContextInvalidationResponse, request: ContextInvalidationRequest): ContextInvalidationResponse {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id || value.request_generation !== request.request_generation) {
    throw new CockpitClientError("malformed_response", "Invalid Context invalidation response identity");
  }
  return value;
}
