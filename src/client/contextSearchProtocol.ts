import type {
  ContextInvalidation,
  ContextInvalidationRequest,
  ContextInvalidationResponse,
  ContextInvalidationState,
  ContextKnownRevision,
  ContextSearchRequest,
  ContextSearchResponse,
  ContextSearchResult,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

const MAX_QUERY_BYTES = 256;
const MAX_RESULTS = 100;
const MAX_SCANNED_FILES = 256;
const MAX_EXCERPT_BYTES = 512;
const MAX_KNOWN_REVISIONS = 128;
const encoder = new TextEncoder();
const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const identity = (value: unknown): value is string => text(value) && value.length > 0 && value.length <= 4096 && !value.includes("\0");
const positiveU32 = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value > 0 && value <= 0xffffffff;
const relativePath = (value: unknown): value is string =>
  text(value) && value.length > 0 && value.length <= 4096 && !value.includes("\0")
    && value.split("/").every((part) => part !== "" && part !== "." && part !== "..");

function malformed(label: string): never {
  throw new CockpitClientError("malformed_response", `Invalid Context ${label}`);
}

function parseKnown(value: unknown): ContextKnownRevision {
  if (!record(value) || !relativePath(value.path) || !identity(value.revision)) malformed("known revision");
  return { path: value.path, revision: value.revision };
}

export function parseContextSearchRequest(value: unknown): ContextSearchRequest {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id)
    || !text(value.query) || value.query.length === 0 || encoder.encode(value.query).byteLength > MAX_QUERY_BYTES
    || /[\u0000-\u001f\u007f-\u009f]/.test(value.query) || !positiveU32(value.request_generation)) {
    return malformed("search request");
  }
  return {
    binding_id: value.binding_id,
    root_id: value.root_id,
    query: value.query,
    request_generation: value.request_generation,
  };
}

export function parseContextInvalidationRequest(value: unknown): ContextInvalidationRequest {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id)
    || !positiveU32(value.request_generation) || !Array.isArray(value.known)
    || value.known.length > MAX_KNOWN_REVISIONS) return malformed("invalidation request");
  const known = value.known.map(parseKnown);
  if (new Set(known.map((entry) => entry.path)).size !== known.length) malformed("duplicate known path");
  return {
    binding_id: value.binding_id,
    root_id: value.root_id,
    request_generation: value.request_generation,
    known,
  };
}

function parseResult(value: unknown): ContextSearchResult {
  if (!record(value) || !relativePath(value.path) || !positiveU32(value.line)
    || !text(value.excerpt) || encoder.encode(value.excerpt).byteLength > MAX_EXCERPT_BYTES
    || !identity(value.revision)) malformed("search result");
  return { path: value.path, line: value.line, excerpt: value.excerpt, revision: value.revision };
}

function parseInvalidation(value: unknown): ContextInvalidation {
  if (!record(value) || !relativePath(value.path)
    || (value.state !== "changed" && value.state !== "missing" && value.state !== "unavailable")
    || !(value.revision === null || identity(value.revision))) malformed("invalidation");
  if (value.state === "changed" && value.revision === null) malformed("changed invalidation");
  return {
    path: value.path,
    state: value.state as ContextInvalidationState,
    revision: value.revision,
  };
}

export function parseContextSearchResponse(value: unknown): ContextSearchResponse {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !text(value.query)
    || !positiveU32(value.request_generation) || !Array.isArray(value.results) || value.results.length > MAX_RESULTS
    || !Number.isSafeInteger(value.scanned_files) || (value.scanned_files as number) < 0
    || (value.scanned_files as number) > MAX_SCANNED_FILES || typeof value.truncated !== "boolean") {
    return malformed("search response");
  }
  const results = value.results.map(parseResult);
  return {
    binding_id: value.binding_id,
    root_id: value.root_id,
    query: value.query,
    request_generation: value.request_generation,
    results,
    scanned_files: value.scanned_files as number,
    truncated: value.truncated,
  };
}

export function parseContextInvalidationResponse(value: unknown): ContextInvalidationResponse {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id)
    || !positiveU32(value.request_generation) || !Array.isArray(value.invalidations)
    || value.invalidations.length > MAX_KNOWN_REVISIONS || typeof value.truncated !== "boolean") {
    return malformed("invalidation response");
  }
  const invalidations = value.invalidations.map(parseInvalidation);
  if (new Set(invalidations.map((entry) => entry.path)).size !== invalidations.length) malformed("duplicate invalidation path");
  return {
    binding_id: value.binding_id,
    root_id: value.root_id,
    request_generation: value.request_generation,
    invalidations,
    truncated: value.truncated,
  };
}

export function matchContextSearchResponse(
  value: ContextSearchResponse,
  request: ContextSearchRequest,
): ContextSearchResponse {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id
    || value.query !== request.query || value.request_generation !== request.request_generation) {
    return malformed("search response identity");
  }
  return value;
}

export function matchContextInvalidationResponse(
  value: ContextInvalidationResponse,
  request: ContextInvalidationRequest,
): ContextInvalidationResponse {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id
    || value.request_generation !== request.request_generation) return malformed("invalidation response identity");
  return value;
}
