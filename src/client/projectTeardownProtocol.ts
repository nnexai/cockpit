import type {
  WorkspaceTeardownExecuteRequest, WorkspaceTeardownPreview, WorkspaceTeardownPreviewRequest,
  WorkspaceTeardownRecoveryList, WorkspaceTeardownResult,
} from "../protocol/generated/v1";
import {
  wireWorkspaceTeardownExecuteRequest, wireWorkspaceTeardownPreview, wireWorkspaceTeardownPreviewRequest,
  wireWorkspaceTeardownRecoveryList, wireWorkspaceTeardownResult, type TypedWirePolicy,
} from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, parseWire, constantMessage, type ProtocolPolicy } from "./wire";

const workspaceId = (value: string) => /^[A-Za-z0-9:_-]{1,128}$/.test(value);
const operationId = (value: string) => /^[A-Za-z0-9_-]{1,96}$/.test(value);
const discardFingerprint = (value: string) => /^[a-f0-9]{64}$/.test(value);
const discardPath = (value: string) => value.length > 0 && !value.startsWith("/") &&
  !/^[A-Za-z]:/.test(value) && !value.includes("\0") &&
  !value.split(/[\\/]/).some((part) => part === "..") && !value.startsWith("\\");
function malformed(label: string): never { throw new CockpitClientError("malformed_response", `Invalid ${label}`); }
const RULES = {
  raw: new Set(["WorkspaceTeardownPreviewRequest", "WorkspaceTeardownExecuteRequest", "WorkspaceTeardownPreview",
    "WorkspaceTeardownResult", "WorkspaceTeardownRecoveryList"]),
  fields: {
    "WorkspaceTeardownPreviewRequest.workspace_id": workspaceId,
    "WorkspaceTeardownExecuteRequest.operation_id": operationId, "WorkspaceTeardownExecuteRequest.workspace_id": workspaceId,
    "WorkspaceTeardownExecuteRequest.expected_endpoint_identity": (value) => value.length > 0,
    "WorkspaceTeardownExecuteRequest.discard_fingerprint": (value) => value === undefined || discardFingerprint(value),
    "WorkspaceTeardownPreview.operation_id": operationId, "WorkspaceTeardownPreview.workspace_id": workspaceId,
    "WorkspaceTeardownPreview.discard_count": (value) => Number.isInteger(value) && value >= 0 && value <= 0xffffffff,
    "WorkspaceTeardownPreview.discard_paths": (value) => value.length <= 50 && value.every(discardPath),
    "WorkspaceTeardownPreview.discard_fingerprint": (value) => value === null || discardFingerprint(value),
    "WorkspaceTeardownResult.operation_id": operationId, "WorkspaceTeardownResult.workspace_id": workspaceId,
    "WorkspaceTeardownRecovery.operation_id": operationId, "WorkspaceTeardownRecovery.workspace_id": workspaceId,
  },
} satisfies TypedWirePolicy;
const policy = (label: string): ProtocolPolicy => definePolicy({ wire: RULES, message: constantMessage(`Invalid ${label}`) });
const PREVIEW_REQUEST = policy("workspace teardown preview request"), EXECUTE_REQUEST = policy("workspace teardown execution request");
const PREVIEW = policy("workspace teardown preview"), RESULT = policy("workspace teardown result"), RECOVERIES = policy("workspace teardown recoveries");
export function parseWorkspaceTeardownPreviewRequest(value: unknown): WorkspaceTeardownPreviewRequest { return parseWire(value, wireWorkspaceTeardownPreviewRequest, PREVIEW_REQUEST); }
export function parseWorkspaceTeardownExecuteRequest(value: unknown): WorkspaceTeardownExecuteRequest {
  const result = parseWire(value, wireWorkspaceTeardownExecuteRequest, EXECUTE_REQUEST);
  if (result.action !== "close_space" && result.expected_checkout_path.length === 0) {
    malformed("workspace teardown checkout path");
  }
  return result;
}
export function parseWorkspaceTeardownPreview(value: unknown): WorkspaceTeardownPreview {
  const result = parseWire(value, wireWorkspaceTeardownPreview, PREVIEW);
  if (result.discard_paths.length > result.discard_count) malformed("workspace teardown discard paths");
  return result;
}
export function parseWorkspaceTeardownResult(value: unknown): WorkspaceTeardownResult { return parseWire(value, wireWorkspaceTeardownResult, RESULT); }
export function parseWorkspaceTeardownRecoveryList(value: unknown): WorkspaceTeardownRecoveryList {
  const result = parseWire(value, wireWorkspaceTeardownRecoveryList, RECOVERIES);
  if (new Set(result.recoveries.map((entry) => entry.operation_id)).size !== result.recoveries.length) malformed("duplicate workspace teardown recovery");
  return result;
}

export function matchWorkspaceTeardownPreview(
  value: WorkspaceTeardownPreview,
  request: WorkspaceTeardownPreviewRequest,
): WorkspaceTeardownPreview {
  if (value.workspace_id !== request.workspace_id) malformed("workspace teardown preview identity");
  return value;
}

export function matchWorkspaceTeardownResult(
  value: WorkspaceTeardownResult,
  request: WorkspaceTeardownExecuteRequest,
): WorkspaceTeardownResult {
  if (value.operation_id !== request.operation_id || value.workspace_id !== request.workspace_id || value.action !== request.action) {
    malformed("workspace teardown result identity");
  }
  return value;
}
