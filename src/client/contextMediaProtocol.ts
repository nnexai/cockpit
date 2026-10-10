import type { ContextMedia, ContextMediaRequest } from "../protocol/generated/v1";
import { wireContextMedia, wireContextMediaRequest } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, parseWire, rootMessage } from "./wire";

const MAX_MEDIA_BYTES = 8 * 1024 * 1024;
const identity = (v: string): boolean => v.length > 0 && v.length <= 4096 && !v.includes("\0");
const relativePath = (v: string): boolean => identity(v) && v.split("/").every(p => p !== "" && p !== "." && p !== "..");
function base64ByteLength(value: string): number {
  if (value.length === 0 || value.length > Math.ceil(MAX_MEDIA_BYTES / 3) * 4 || value.length % 4 !== 0
    || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) return -1;
  return value.length / 4 * 3 - (value.endsWith("==") ? 2 : value.endsWith("=") ? 1 : 0);
}
const POLICY = definePolicy({
  message: rootMessage("Invalid Context media", { ContextMediaRequest: "request", ContextMedia: "response" }),
  wire: {
    fields: {
      "ContextMediaRequest.binding_id": identity, "ContextMediaRequest.root_id": identity, "ContextMediaRequest.path": relativePath,
      "ContextMediaRequest.expected_revision": v => v === null || identity(v),
      "ContextMedia.binding_id": identity, "ContextMedia.root_id": identity, "ContextMedia.path": relativePath,
      "ContextMedia.revision": identity, "ContextMedia.content_hash": identity, "ContextMedia.bytes": v => v <= MAX_MEDIA_BYTES,
      "ContextMedia.mime_type": v => v === "image/png" || v === "image/jpeg", "ContextMedia.width": v => v > 0, "ContextMedia.height": v => v > 0,
    },
    checks: { ContextMedia: { payload: v => v.width * v.height <= 16_000_000 && base64ByteLength(v.data_base64) === v.bytes } },
  },
});
export function parseContextMediaRequest(value: unknown): ContextMediaRequest { return parseWire(value, wireContextMediaRequest, POLICY); }
export function parseContextMedia(value: unknown): ContextMedia { return parseWire(value, wireContextMedia, POLICY); }
export function matchContextMedia(value: ContextMedia, request: ContextMediaRequest): ContextMedia {
  if (value.binding_id !== request.binding_id || value.root_id !== request.root_id || value.path !== request.path
    || (request.expected_revision !== null && value.revision !== request.expected_revision)) {
    throw new CockpitClientError("malformed_response", "Invalid Context media response identity");
  }
  return value;
}
