import type {
  WorkspaceTeardownAction,
  WorkspaceTeardownExecuteRequest,
  WorkspaceTeardownOutcome,
  WorkspaceTeardownPreview,
  WorkspaceTeardownPreviewRequest,
  WorkspaceTeardownRecovery,
  WorkspaceTeardownRecoveryList,
  WorkspaceTeardownResult,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(text);
const nullableText = (value: unknown): value is string | null => value === null || text(value);
const workspaceId = (value: unknown): value is string =>
  text(value) && /^[A-Za-z0-9:_-]{1,128}$/.test(value);
const operationId = (value: unknown): value is string =>
  text(value) && /^[A-Za-z0-9_-]{1,96}$/.test(value);
const action = (value: unknown): value is WorkspaceTeardownAction =>
  value === "close_space" || value === "remove_owned_worktree" || value === "reconcile_remove_outcome"
    || value === "remove_orphaned_companion" || value === "forget_association";
const outcome = (value: unknown): value is WorkspaceTeardownOutcome =>
  value === "completed" || value === "outcome_unknown" || value === "orphaned_companion" || value === "retained";

function malformed(label: string): never {
  throw new CockpitClientError("malformed_response", `Invalid ${label}`);
}

export function parseWorkspaceTeardownPreviewRequest(value: unknown): WorkspaceTeardownPreviewRequest {
  if (!record(value) || !workspaceId(value.workspace_id)) malformed("workspace teardown preview request");
  return value as WorkspaceTeardownPreviewRequest;
}

export function parseWorkspaceTeardownExecuteRequest(value: unknown): WorkspaceTeardownExecuteRequest {
  if (!record(value) || !operationId(value.operation_id) || !workspaceId(value.workspace_id)
    || !text(value.expected_endpoint_identity) || value.expected_endpoint_identity.length === 0
    || !text(value.expected_checkout_path) || value.expected_checkout_path.length === 0
    || !action(value.action) || !text(value.confirmation)) {
    malformed("workspace teardown execution request");
  }
  return value as WorkspaceTeardownExecuteRequest;
}

export function parseWorkspaceTeardownPreview(value: unknown): WorkspaceTeardownPreview {
  if (!record(value) || !operationId(value.operation_id) || !workspaceId(value.workspace_id)
    || !text(value.endpoint_identity) || !text(value.repository_key) || !text(value.repository_root)
    || !text(value.checkout_path) || !["owned_created", "borrowed_opened", "foreign", "unknown"].includes(String(value.ownership))
    || !["live", "missing", "ambiguous"].includes(String(value.workspace_state))
    || !["owned", "missing", "foreign", "ambiguous"].includes(String(value.companion_state))
    || typeof value.is_linked_worktree !== "boolean"
    || !["clean", "dirty", "unknown"].includes(String(value.dirty_state))
    || !nullableText(value.companion_path) || !Array.isArray(value.allowed_actions) || !value.allowed_actions.every(action)
    || !texts(value.blockers) || !texts(value.warnings) || !nullableText(value.required_confirmation)) {
    malformed("workspace teardown preview");
  }
  return value as WorkspaceTeardownPreview;
}

export function parseWorkspaceTeardownResult(value: unknown): WorkspaceTeardownResult {
  if (!record(value) || !operationId(value.operation_id) || !workspaceId(value.workspace_id)
    || !action(value.action) || !outcome(value.outcome) || !text(value.message)) {
    malformed("workspace teardown result");
  }
  return value as WorkspaceTeardownResult;
}

function recovery(value: unknown): value is WorkspaceTeardownRecovery {
  return record(value) && operationId(value.operation_id) && workspaceId(value.workspace_id)
    && text(value.checkout_path)
    && (value.state === "pending" || value.state === "outcome_unknown" || value.state === "orphaned_companion");
}

export function parseWorkspaceTeardownRecoveryList(value: unknown): WorkspaceTeardownRecoveryList {
  if (!record(value) || !Array.isArray(value.recoveries) || !value.recoveries.every(recovery)) {
    malformed("workspace teardown recoveries");
  }
  const operationIds = value.recoveries.map((entry) => entry.operation_id);
  if (new Set(operationIds).size !== operationIds.length) malformed("duplicate workspace teardown recovery");
  return value as WorkspaceTeardownRecoveryList;
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
