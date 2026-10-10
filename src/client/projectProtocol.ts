import type {
  ProjectConfiguration, RepositoryListResponse, WorkspaceDefaults, WorkspaceDefaultsRequest, WorkspaceOperation,
  WorkspaceOperationRequest, WorkspaceSetupPlan, WorkspaceSetupRequest, WorkspaceReconcileRequest,
} from "../protocol/generated/v1";
import {
  wireProjectConfiguration, wireRepositoryListResponse, wireWorkspaceDefaults, wireWorkspaceDefaultsRequest,
  wireWorkspaceOperation, wireWorkspaceOperationRequest, wireWorkspaceSetupPlan, wireWorkspaceSetupRequest,
  wireWorkspaceReconcileRequest, type TypedWirePolicy,
} from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, parseWire, constantMessage, rootMessage, type ProtocolPolicy } from "./wire";

function malformed(label: string): never { throw new CockpitClientError("malformed_response", `Invalid ${label}`); }
const nonempty = (value: string) => value.length > 0;
const operationId = (value: string) => /^[A-Za-z0-9_-]{1,96}$/.test(value);
const RULES = {
  absent: new Set(["ProjectConfiguration.orchestration"]), // projects.rs:100: #[serde(default)], still validates supplied values.
  exact: new Set(["WorkspaceSetupRequest[open]", "WorkspaceSetupRequest[create]"]),
  raw: new Set(["ProjectConfiguration", "RepositoryListResponse", "WorkspaceOperationRequest", "WorkspaceReconcileRequest",
    "WorkspaceSetupPlan", "WorkspaceOperation", "WorkspaceDefaults.artifact", "WorkspaceDefaults.repositories", "WorkspaceDefaults.linked_artifacts",
    "WorkspaceSetupRequest[create].linked_artifact_urls"]),
  fields: {
    "RepositoryCandidate.repository_id": nonempty, "RepositoryCandidate.provenance": nonempty,
    "ProjectArtifact.kind": (value) => ["issue", "review", "wiki", "custom"].includes(value),
    "ProjectProvider.executable": (value) => value !== null,
    "ProjectProvider.deployment": (value) => value !== null,
    "ProjectProvider.login": (value) => value !== undefined && value !== null && value.length > 0 && value.length <= 256,
    "WorkspaceDefaultsRequest.artifact_url": (value) => !!value.trim() && value.length <= 2048,
    "WorkspaceSetupRequest[open].path": (value) => !!value.trim(),
    "WorkspaceSetupRequest[create].repository_id": nonempty,
    "WorkspaceOperationRequest.operation_id": operationId, "WorkspaceReconcileRequest.operation_id": operationId,
    "WorkspaceSetupPlan.endpoint_identity": nonempty,
  },
  lengths: { "WorkspaceSetupRequest[create].linked_artifact_urls": { max: 4 } },
  order: {
    WorkspaceSetupRequest: ["label", "task_name", "focus", "tag", "tag:known"],
    "WorkspaceSetupRequest[open]": ["path", "keys"],
    "WorkspaceSetupRequest[create]": ["repository_id", "branch", "base_ref", "checkout_path", "artifact_url", "linked_artifact_urls", "keys"],
    WorkspaceOperation: ["operation_id", "generation", "sequence", "session_id", "state", "step", "workspace_id", "tab_id",
      "pane_id", "resume_allowed", "cancel_requested", "updated_at", "error", "owned_resources", "plan"],
  },
  emit: {
    "WorkspaceSetupRequest[open]": ["tag", "path", "label", "task_name", "focus"],
    "WorkspaceSetupRequest[create]": ["tag", "repository_id", "branch", "base_ref", "checkout_path", "label", "task_name", "artifact_url", "linked_artifact_urls", "focus"],
  },
} satisfies TypedWirePolicy;
const policy = (label: string): ProtocolPolicy => definePolicy({ wire: RULES, message: constantMessage(`Invalid ${label}`) });
const CONFIGURATION = policy("project configuration"), CATALOG = policy("repository catalog");
const DEFAULTS_REQUEST = policy("workspace defaults request"), DEFAULTS = policy("workspace defaults");
const OPERATION_REQUEST = policy("workspace operation request"), RECONCILE = policy("workspace reconciliation request");
const PLAN = policy("workspace setup plan");
const OPERATION = definePolicy({ wire: RULES, message: rootMessage("Invalid", {
  WorkspaceOperation: "workspace operation", WorkspaceSetupPlan: "workspace setup plan",
}) });
const SETUP = definePolicy({ wire: RULES, message: (failure) => {
  const owner = failure.path.find((frame) => frame.type === "WorkspaceSetupRequest");
  if (failure.reason === "shape" && !owner?.field && !owner?.variant) return "Invalid workspace setup request";
  if (owner?.field === "label" || owner?.field === "task_name" || owner?.field === "focus") return "Invalid workspace setup request";
  return owner?.variant === "open" ? "Invalid directory setup request" : "Invalid worktree setup request";
} });

export function parseProjectConfiguration(value: unknown): ProjectConfiguration { return parseWire(value, wireProjectConfiguration, CONFIGURATION); }
export function parseRepositoryList(value: unknown): RepositoryListResponse {
  const result = parseWire(value, wireRepositoryListResponse, CATALOG);
  if (new Set(result.repositories.map((repository) => repository.repository_id)).size !== result.repositories.length) malformed("duplicate repository identities");
  return result;
}
export function parseWorkspaceDefaultsRequest(value: unknown): WorkspaceDefaultsRequest { return parseWire(value, wireWorkspaceDefaultsRequest, DEFAULTS_REQUEST); }
export function parseWorkspaceDefaults(value: unknown): WorkspaceDefaults {
  const result = parseWire(value, wireWorkspaceDefaults, DEFAULTS);
  if (result.repository_id !== null && !result.repositories.some((repository) => repository.repository_id === result.repository_id)) malformed("workspace defaults repository identity");
  return result;
}
export function parseWorkspaceSetupRequest(value: unknown): WorkspaceSetupRequest { return parseWire(value, wireWorkspaceSetupRequest, SETUP); }
export function parseWorkspaceOperationRequest(value: unknown): WorkspaceOperationRequest { return parseWire(value, wireWorkspaceOperationRequest, OPERATION_REQUEST); }
export function parseWorkspaceReconcileRequest(value: unknown): WorkspaceReconcileRequest {
  parseWorkspaceOperationRequest(value);
  return parseWire(value, wireWorkspaceReconcileRequest, RECONCILE);
}
export function parseWorkspaceSetupPlan(value: unknown): WorkspaceSetupPlan { return parseWire(value, wireWorkspaceSetupPlan, PLAN); }
export function parseWorkspaceOperation(value: unknown): WorkspaceOperation {
  const result = parseWire(value, wireWorkspaceOperation, OPERATION);
  if (result.plan.operation_id !== result.operation_id || result.plan.session_id !== result.session_id) malformed("workspace operation identity");
  return result;
}

export function matchProjectSession<T extends { session_id: string; operation_id: string }>(
  value: T, sessionId: string, operationId?: string,
): T {
  if (value.session_id !== sessionId || (operationId !== undefined && value.operation_id !== operationId)) {
    malformed("workspace response identity");
  }
  return value;
}

export function validateProjectOperationId(value: string): void {
  if (!/^[A-Za-z0-9_-]{1,96}$/.test(value)) malformed("workspace operation identity");
}
