import type {
  ProjectArtifact, ProjectConfiguration, RepositoryCandidate, RepositoryListResponse, WorkspaceDefaults, WorkspaceDefaultsRequest,
  WorkspaceOperation, WorkspaceOperationRequest, WorkspaceSetupPlan, WorkspaceSetupRequest,
  WorkspaceReconcileRequest,
} from "../protocol/generated/v1";
import { CockpitClientError, parseErrorEnvelope } from "./CockpitClient";

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const nullableText = (value: unknown): value is string | null => value === null || text(value);
const bool = (value: unknown): value is boolean => typeof value === "boolean";
const u32 = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 0xffffffff;
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(text);
const mode = (value: unknown) => value === "create" || value === "open";

function malformed(label: string): never {
  throw new CockpitClientError("malformed_response", `Invalid ${label}`);
}

function isRepository(value: unknown): value is RepositoryCandidate {
  return record(value) && text(value.repository_id) && value.repository_id.length > 0
    && text(value.name) && text(value.root) && text(value.checkout_path) && text(value.common_dir)
    && nullableText(value.branch) && bool(value.is_linked_worktree) && bool(value.is_detached)
    && text(value.provenance) && value.provenance.length > 0;
}

function isArtifact(value: unknown): value is ProjectArtifact {
  return record(value) && text(value.provider_id) && text(value.kind)
    && ["issue", "review", "wiki", "custom"].includes(value.kind)
    && text(value.canonical_id) && text(value.original_url) && text(value.canonical_url);
}

export function parseProjectConfiguration(value: unknown): ProjectConfiguration {
  if (!record(value) || !u32(value.version) || !texts(value.repository_roots)
    || !text(value.worktree_root) || !text(value.companion_root) || !text(value.state_root)
    || !text(value.branch_template) || !text(value.checkout_template)
    || !record(value.limits) || !u32(value.limits.catalog_depth) || !u32(value.limits.catalog_entries)
    || !u32(value.limits.git_timeout_ms) || !u32(value.limits.git_output_bytes) || !u32(value.limits.operation_timeout_ms)
    || !u32(value.limits.context_preview_bytes) || !u32(value.limits.context_preview_lines)
    || !u32(value.limits.context_directory_entries) || !u32(value.limits.context_tree_depth)
    || !record(value.origins) || !Object.values(value.origins).every(text)
    || !Array.isArray(value.providers) || !value.providers.every((provider) => record(provider)
      && text(provider.id) && text(provider.base_url) && text(provider.executable) && (provider.login === undefined || (text(provider.login) && provider.login.length > 0 && provider.login.length <= 256)))) {
    malformed("project configuration");
  }
  return value as unknown as ProjectConfiguration;
}

export function parseRepositoryList(value: unknown): RepositoryListResponse {
  if (!record(value) || !Array.isArray(value.repositories) || !value.repositories.every(isRepository)
    || !Array.isArray(value.diagnostics) || !value.diagnostics.every((diagnostic) => record(diagnostic)
      && text(diagnostic.code) && text(diagnostic.message) && nullableText(diagnostic.path))) {
    malformed("repository catalog");
  }
  const ids = value.repositories.map((repository) => repository.repository_id);
  if (new Set(ids).size !== ids.length) malformed("duplicate repository identities");
  return value as unknown as RepositoryListResponse;
}

export function parseWorkspaceDefaultsRequest(value: unknown): WorkspaceDefaultsRequest {
  if (!record(value) || !text(value.artifact_url) || !value.artifact_url.trim() || value.artifact_url.length > 2048 || !nullableText(value.repository_id)) malformed("workspace defaults request");
  return { artifact_url: value.artifact_url, repository_id: value.repository_id };
}

export function parseWorkspaceDefaults(value: unknown): WorkspaceDefaults {
  if (!record(value) || !isArtifact(value.artifact) || !Array.isArray(value.repositories) || !value.repositories.every(isRepository)
    || !nullableText(value.repository_id) || !nullableText(value.branch) || !nullableText(value.label) || !nullableText(value.checkout_path)) malformed("workspace defaults");
  if (value.repository_id !== null && !value.repositories.some(repository => repository.repository_id === value.repository_id)) malformed("workspace defaults repository identity");
  return { artifact: value.artifact, repositories: value.repositories, repository_id: value.repository_id, branch: value.branch, label: value.label, checkout_path: value.checkout_path };
}

export function parseWorkspaceSetupRequest(value: unknown): WorkspaceSetupRequest {
  if (!record(value) || !nullableText(value.label) || !nullableText(value.task_name) || !bool(value.focus)) malformed("workspace setup request");
  if (value.operation === "open") {
    if (!text(value.path) || !value.path.trim() || Object.keys(value).some(key => !["operation", "path", "label", "task_name", "focus"].includes(key))) malformed("directory setup request");
    return { operation: "open", path: value.path, label: value.label, task_name: value.task_name, focus: value.focus };
  }
  if (value.operation !== "create" || !text(value.repository_id) || !value.repository_id || !nullableText(value.branch) || !nullableText(value.base_ref)
    || !nullableText(value.checkout_path) || !nullableText(value.artifact_url)
    || Object.keys(value).some(key => !["operation", "repository_id", "branch", "base_ref", "checkout_path", "label", "task_name", "artifact_url", "focus"].includes(key))) malformed("worktree setup request");
  return { operation: "create", repository_id: value.repository_id, branch: value.branch, base_ref: value.base_ref, checkout_path: value.checkout_path, label: value.label, task_name: value.task_name, artifact_url: value.artifact_url, focus: value.focus };
}

export function parseWorkspaceOperationRequest(value: unknown): WorkspaceOperationRequest {
  if (!record(value) || !text(value.operation_id) || !/^[A-Za-z0-9_-]{1,96}$/.test(value.operation_id)
    || !u32(value.expected_generation)) malformed("workspace operation request");
  return value as unknown as WorkspaceOperationRequest;
}

export function parseWorkspaceReconcileRequest(value: unknown): WorkspaceReconcileRequest {
  const request = parseWorkspaceOperationRequest(value) as WorkspaceReconcileRequest;
  if (request.action !== "accept_existing_worktree" && request.action !== "retry_environment") {
    malformed("workspace reconciliation request");
  }
  return request;
}

export function parseWorkspaceSetupPlan(value: unknown): WorkspaceSetupPlan {
  if (!record(value) || !text(value.operation_id) || !u32(value.generation) || !text(value.session_id)
    || !text(value.endpoint_identity) || value.endpoint_identity.length === 0
    || !(value.repository === null || isRepository(value.repository)) || !mode(value.mode) || !nullableText(value.branch) || !nullableText(value.base)
    || !text(value.checkout_path) || !text(value.companion_path) || !text(value.label)
    || !text(value.companion_id) || value.companion_id.length === 0 || !bool(value.companion_created_by_operation)
    || !bool(value.focus) || !["owned_worktree", "borrowed_directory"].includes(String(value.ownership))
    || !(value.artifact === null || isArtifact(value.artifact)) || !texts(value.effects) || !texts(value.warnings)) {
    malformed("workspace setup plan");
  }
  return value as unknown as WorkspaceSetupPlan;
}

export function parseWorkspaceOperation(value: unknown): WorkspaceOperation {
  if (!record(value) || !text(value.operation_id) || !u32(value.generation) || !u32(value.sequence)
    || !text(value.session_id) || !text(value.state)
    || !["planned", "running", "completed", "partial", "outcome_unknown", "cancelled", "needs_review"].includes(value.state)
    || !text(value.step) || !["planned", "validated", "herdr_requested", "herdr_observed", "worktree_ready", "workspace_verified",
      "companion_ready", "context_preparing", "context_ready", "environment_requested", "environment_ready", "completed"].includes(value.step)
    || !nullableText(value.workspace_id) || !nullableText(value.tab_id) || !nullableText(value.pane_id)
    || !nullableText(value.companion_id) || !bool(value.resume_allowed) || !bool(value.cancel_requested)
    || !text(value.updated_at) || !(value.error === null || parseErrorEnvelope(value.error) !== undefined)
    || !Array.isArray(value.owned_resources) || !value.owned_resources.every((resource) => record(resource)
      && text(resource.kind) && text(resource.path) && bool(resource.created_by_operation))) {
    malformed("workspace operation");
  }
  const plan = parseWorkspaceSetupPlan(value.plan);
  if (plan.operation_id !== value.operation_id || plan.session_id !== value.session_id
    || (value.companion_id !== null && value.companion_id !== plan.companion_id)) {
    malformed("workspace operation identity");
  }
  return value as unknown as WorkspaceOperation;
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
