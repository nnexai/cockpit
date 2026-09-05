import type { ContextMedia, ContextMediaRequest } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

const MAX_MEDIA_BYTES = 8 * 1024 * 1024;
const MAX_BASE64_BYTES = Math.ceil(MAX_MEDIA_BYTES / 3) * 4;
const MAX_PIXELS = 16_000_000;
const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const identity = (value: unknown): value is string =>
  text(value) && value.length > 0 && value.length <= 4096 && !value.includes("\0");
const relativePath = (value: unknown): value is string =>
  text(value) && value.length > 0 && value.length <= 4096 && !value.includes("\0")
    && value.split("/").every((part) => part !== "" && part !== "." && part !== "..");
const safeU32 = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value > 0 && value <= 0xffffffff;
const safeByteCount = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= MAX_MEDIA_BYTES;

function malformed(label: string): never {
  throw new CockpitClientError("malformed_response", `Invalid Context media ${label}`);
}

function base64ByteLength(value: string): number {
  if (value.length === 0 || value.length > MAX_BASE64_BYTES || value.length % 4 !== 0
    || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
    return -1;
  }
  const padding = value.endsWith("==") ? 2 : value.endsWith("=") ? 1 : 0;
  return (value.length / 4) * 3 - padding;
}

export function parseContextMediaRequest(value: unknown): ContextMediaRequest {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !relativePath(value.path)
    || !(value.expected_revision === null || identity(value.expected_revision))) {
    return malformed("request");
  }
  return {
    binding_id: value.binding_id,
    root_id: value.root_id,
    path: value.path,
    expected_revision: value.expected_revision,
  };
}

export function parseContextMedia(value: unknown): ContextMedia {
  if (!record(value) || !identity(value.binding_id) || !identity(value.root_id) || !relativePath(value.path)
    || !identity(value.revision) || !identity(value.content_hash) || !safeByteCount(value.bytes)
    || (value.mime_type !== "image/png" && value.mime_type !== "image/jpeg")
    || !safeU32(value.width) || !safeU32(value.height)
    || (value.width as number) * (value.height as number) > MAX_PIXELS
    || !text(value.data_base64) || base64ByteLength(value.data_base64) !== value.bytes) {
    return malformed("response");
  }
  return {
    binding_id: value.binding_id,
    root_id: value.root_id,
    path: value.path,
    revision: value.revision,
    content_hash: value.content_hash,
    bytes: value.bytes,
    mime_type: value.mime_type,
    width: value.width,
    height: value.height,
    data_base64: value.data_base64,
  };
}

export function matchContextMedia(value: ContextMedia, request: ContextMediaRequest): ContextMedia {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id || value.path !== request.path
    || (request.expected_revision !== null && value.revision !== request.expected_revision)) {
    return malformed("response identity");
  }
  return value;
}
