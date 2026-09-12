import { useCallback, useEffect, useId, useRef, useState, type ChangeEvent, type ReactNode } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type {
  ProjectConfiguration,
  RepositoryCandidate,
  RepositoryListResponse,
  WorkspaceOperation,
  WorkspaceOperationRequest,
  WorkspaceOperationState,
  WorkspaceRecoveryAction,
  WorkspaceSetupMode,
  WorkspaceSetupPlan,
  WorkspaceSetupRequest,
} from "../../protocol/generated/v1";
import "./setup.css";

export type SetupDialogProps = {
  client: CockpitClient;
  sessionId: string;
  open: boolean;
  selectedParent?: SetupParent | null;
  onClose: () => void;
  onCompleted: (operation: WorkspaceOperation) => void;
};

export type SetupParent = {
  label: string;
  repositoryKey: string;
  checkoutPath: string;
};

type FormState = {
  repositoryId: string;
  query: string;
  mode: WorkspaceSetupMode;
  openTarget: "branch" | "path";
  branch: string;
  base: string;
  checkoutPath: string;
  label: string;
  taskName: string;
  artifactUrl: string;
  focus: boolean;
  repositoryConsent: boolean;
};

type LoadState = "loading" | "ready" | "empty" | "error";
type DialogStep = 1 | 2 | 3 | 4;
type PlanState = { plan: WorkspaceSetupPlan | null; error: string | null; pending: boolean };

const POLL_INTERVAL_MS = 700;
const MAX_POLL_REQUESTS = 180;

export function operationSnapshotIsNewer(current: WorkspaceOperation | null, next: WorkspaceOperation): boolean {
  if (!current) return true;
  if (next.generation !== current.generation) return next.generation > current.generation;
  return next.sequence > current.sequence;
}

function operationIsTerminal(state: WorkspaceOperationState): boolean {
  return state === "completed" || state === "cancelled" || state === "partial" || state === "outcome_unknown" || state === "needs_review";
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
  return fallback;
}

function modeLabel(mode: WorkspaceSetupMode): string {
  return mode === "create" ? "Create linked worktree" : "Open existing checkout";
}

function issueUrlPlaceholder(configuration: ProjectConfiguration | null): string {
  const github = configuration?.providers.find((provider) => provider.id === "github" && provider.base_url === "https://github.com");
  return github ? "https://github.com/owner/repository/issues/number" : "https://provider.example/owner/repository/issues/number";
}

export function operationStatusMessage(operation: Pick<WorkspaceOperation, "state" | "step" | "error">): string {
  const sourceFailure = operation.error?.code.startsWith("source_") ?? false;
  if (operation.state === "partial" && sourceFailure) {
    return "Source import is partial. Retry the source step to finish Context; the reviewed workspace and companion remain available for review.";
  }
  if (operation.state === "partial") {
    return "Workspace setup is partial. Review the retained effects and retry the recorded failed step.";
  }
  if (operation.state === "needs_review") {
    return "Workspace setup needs review. Inspect the retained effects before choosing a recovery action.";
  }
  switch (operation.step) {
    case "context_preparing":
      return "Preparing the Context companion and validating source availability.";
    case "context_ready":
      return "Context is ready. Preparing the context-aware terminal.";
    case "completed":
      return "Workspace, Context, and the context-aware terminal are ready.";
    default:
      return `Setup is at ${operation.step.replaceAll("_", " ")}.`;
  }
}

function operationStepLabel(step: WorkspaceOperation["step"]): string {
  switch (step) {
    case "context_preparing": return "Preparing Context";
    case "context_ready": return "Context ready";
    default: return step.replaceAll("_", " ");
  }
}

function sourceRetry(operation: WorkspaceOperation): boolean {
  return operation.error?.code.startsWith("source_") ?? false;
}

function resolveParentRepository(repositories: RepositoryCandidate[], parent: SetupParent): RepositoryCandidate | null {
  const checkoutMatches = repositories.filter((repository) => repository.checkout_path === parent.checkoutPath || repository.root === parent.checkoutPath);
  if (checkoutMatches.length === 1) return checkoutMatches[0];
  if (checkoutMatches.length > 1) return null;
  const commonMatches = repositories.filter((repository) => repository.common_dir === parent.repositoryKey);
  return commonMatches.length === 1 ? commonMatches[0] : null;
}

function recoveryActionFor(operation: WorkspaceOperation): WorkspaceRecoveryAction | null {
  if (operation.state !== "outcome_unknown") return null;
  if (operation.step === "herdr_requested") return "accept_existing_worktree";
  if (operation.step === "environment_requested") return "retry_environment";
  return null;
}


function initialForm(): FormState {
  return {
    repositoryId: "",
    query: "",
    mode: "create",
    openTarget: "branch",
    branch: "",
    base: "",
    checkoutPath: "",
    label: "",
    taskName: "",
    artifactUrl: "",
    focus: true,
    repositoryConsent: false,
  };
}

function makeRequest(form: FormState): WorkspaceSetupRequest {
  const optional = (value: string): string | null => value.trim() || null;
  const branch = optional(form.branch);
  const checkoutPath = optional(form.checkoutPath);
  return {
    repository_id: form.repositoryId,
    mode: form.mode,
    branch: form.mode === "open" && form.openTarget === "path" ? null : branch,
    base: form.mode === "open" ? null : optional(form.base),
    checkout_path: form.mode === "open" && form.openTarget === "branch" ? null : checkoutPath,
    label: optional(form.label),
    task_name: optional(form.taskName),
    artifact_url: optional(form.artifactUrl),
    focus: form.focus,
    trust_repository: form.repositoryConsent,
  };
}

function StepButton({ step, current, label, onSelect }: { step: DialogStep; current: DialogStep; label: string; onSelect: (step: DialogStep) => void }) {
  return (
    <button
      className={`setup-step${current === step ? " is-current" : ""}`}
      type="button"
      aria-current={current === step ? "step" : undefined}
      onClick={() => onSelect(step)}
    >
      <span className="setup-step-number">{step}</span>
      <span>{label}</span>
    </button>
  );
}

function Field({ label, hint, children, htmlFor }: { label: string; hint?: string; children: ReactNode; htmlFor?: string }) {
  return (
    <label className="setup-field" htmlFor={htmlFor}>
      <span className="setup-label">{label}{hint ? <span className="setup-hint"> · {hint}</span> : null}</span>
      {children}
    </label>
  );
}

function DiagnosticList({ diagnostics }: { diagnostics: Array<{ code: string; message: string; path?: string | null }> }) {
  if (diagnostics.length === 0) return null;
  return (
    <div className="setup-diagnostics" role="status">
      <strong>Configuration diagnostics</strong>
      <ul>
        {diagnostics.map((diagnostic, index) => (
          <li key={`${diagnostic.code}-${diagnostic.path ?? ""}-${index}`}>
            <code>{diagnostic.code}</code> {diagnostic.message}{diagnostic.path ? <code className="setup-diagnostic-path">{diagnostic.path}</code> : null}
          </li>
        ))}
      </ul>
    </div>
  );
}

function RepositoryChoice({ repository, selected, onSelect }: { repository: RepositoryCandidate; selected: boolean; onSelect: () => void }) {
  return (
    <button className={`setup-repository${selected ? " is-selected" : ""}`} type="button" role="option" aria-selected={selected} onClick={onSelect}>
      <span className="setup-repository-title"><strong>{repository.name}</strong>{repository.is_detached ? <span className="setup-badge">detached</span> : null}{repository.is_linked_worktree ? <span className="setup-badge">linked worktree</span> : null}</span>
      <code>{repository.root}</code>
      <span className="setup-repository-meta">{repository.branch ? `branch ${repository.branch}` : "detached HEAD"} · {repository.repository_id}</span>
    </button>
  );
}

function PlanSummary({ plan }: { plan: WorkspaceSetupPlan }) {
  return (
    <div className="setup-plan-summary">
      <div className="setup-summary-row"><span>Repository</span><code>{plan.repository.root}</code></div>
      <div className="setup-summary-row"><span>Operation</span><strong>{modeLabel(plan.mode)}</strong></div>
      <div className="setup-summary-row"><span>Checkout</span><code>{plan.checkout_path}</code></div>
      <div className="setup-summary-row"><span>Companion</span><code>{plan.companion_path}</code></div>
      <div className="setup-summary-row"><span>Branch / base</span><code>{plan.branch ?? "(existing)"}{plan.base ? ` ← ${plan.base}` : ""}</code></div>
      <div className="setup-summary-row"><span>Label</span><span>{plan.label}</span></div>
      <div className="setup-summary-row"><span>Artifact</span><span>{plan.artifact ? `${plan.artifact.kind} · ${plan.artifact.canonical_id}` : "No artifact selected"}</span></div>
      <div className="setup-summary-row"><span>Focus / consent</span><span>{plan.focus ? "Focus new context terminal" : "Keep current focus"} · {plan.trust_repository ? "Consent recorded" : "Consent not recorded"}</span></div>
      <div className="setup-effects"><strong>Exact effects</strong><ul>{plan.effects.map((effect, index) => <li key={`${effect}-${index}`}>{effect}</li>)}</ul></div>
      {plan.warnings.length > 0 ? <div className="setup-warnings"><strong>Warnings</strong><ul>{plan.warnings.map((warning, index) => <li key={`${warning}-${index}`}>{warning}</li>)}</ul></div> : null}
      <p className="setup-terminal-boundary"><strong>Terminal boundary:</strong> the Herdr-created root pane cannot receive environment values retroactively. Cockpit creates a new context-aware terminal for new processes only; start an agent manually there. The root pane remains unchanged.</p>
    </div>
  );
}

function Progress({ operation, readError, busy, onCancel, onResume, onReview }: { operation: WorkspaceOperation; readError: string | null; busy: boolean; onCancel: () => void; onResume: () => void; onReview: (action: WorkspaceRecoveryAction) => void }) {
  const failed = operation.state === "partial" || operation.state === "needs_review";
  const recoveryAction = recoveryActionFor(operation);
  const recoveryLabel = recoveryAction === "accept_existing_worktree" ? "Recover existing checkout" : recoveryAction === "retry_environment" ? "Retry environment" : null;
  const retrySource = sourceRetry(operation);
  return (
    <section className="setup-progress" aria-live="polite" aria-busy={busy} aria-label="Workspace setup progress">
      <div className="setup-progress-heading"><div><span className="setup-eyebrow">Durable operation</span><h3>{operationStepLabel(operation.step)}</h3></div><span className={`setup-state setup-state-${operation.state}`}>{operation.state.replaceAll("_", " ")}</span></div>
      <div className="setup-progress-track"><span style={{ width: `${operation.step === "completed" ? 100 : Math.min(94, Math.max(8, (operation.sequence + 1) * 10))}%` }} /></div>
      <p className="setup-progress-meta">Generation {operation.generation} · update {operation.sequence} · last updated {operation.updated_at}</p>
      <p className="setup-progress-message" role="status">{operationStatusMessage(operation)}</p>
      {readError ? <p className="setup-transient" role="status">Could not read the latest operation snapshot. Showing the last confirmed progress; no mutation was retried.</p> : null}
      {operation.error ? <p className="setup-error" role="alert"><strong>{operation.error.code}</strong> {operation.error.message}</p> : null}
      {operation.owned_resources.length > 0 ? <div className="setup-owned"><strong>Resources retained</strong><ul>{operation.owned_resources.map((resource, index) => <li key={`${resource.kind}-${resource.path}-${index}`}><code>{resource.path}</code> · {resource.created_by_operation ? "created by this operation" : "existing resource"}</li>)}</ul></div> : null}
      <div className="setup-progress-actions">
        {operation.state === "running" || operation.state === "planned" ? <button type="button" onClick={onCancel} disabled={busy}>Cancel operation</button> : null}
        {failed && operation.resume_allowed ? <button type="button" className="setup-primary" onClick={onResume} disabled={busy}>{retrySource ? "Retry source import" : "Resume failed step"}</button> : null}
        {recoveryAction && recoveryLabel ? <button type="button" className="setup-primary" onClick={() => onReview(recoveryAction)} disabled={busy}>{recoveryLabel}</button> : null}
      </div>
      {failed ? <p className="setup-recovery-note">Completed effects remain in place. {retrySource ? "Retry addresses only the source import." : "Resume addresses only the recorded failed step."}</p> : null}
      {recoveryAction === "accept_existing_worktree" ? <p className="setup-recovery-note">The worktree outcome is uncertain. Cockpit validates the exact checkout and, only when Herdr has no workspace open for it, explicitly opens that checkout once. It never redispatches Create. Review the resulting state, then explicitly resume if offered.</p> : null}
      {recoveryAction === "retry_environment" ? <p className="setup-recovery-note">The environment outcome is uncertain. A previous request may have opened a tab. Retrying may create a new environment tab; any uncertain old tab or pane remains untouched. Reconcile does not dispatch Herdr mutations. Review the resulting state, then explicitly resume if offered.</p> : null}
    </section>
  );
}
export function SetupDialog({ client, sessionId, open, selectedParent = null, onClose, onCompleted }: SetupDialogProps) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const loadRequestToken = useRef(0);
  const planRequestToken = useRef(0);
  const pollToken = useRef(0);
  const completedOperation = useRef<string | null>(null);
  const operationRef = useRef<WorkspaceOperation | null>(null);
  const pollCount = useRef(0);
  const loadedSession = useRef<string | null>(null);
  const openRef = useRef(open);
  const [step, setStep] = useState<DialogStep>(1);
  const [form, setForm] = useState<FormState>(initialForm);
  const [configuration, setConfiguration] = useState<ProjectConfiguration | null>(null);
  const [repositories, setRepositories] = useState<RepositoryCandidate[]>([]);
  const [diagnostics, setDiagnostics] = useState<RepositoryListResponse["diagnostics"]>([]);
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [planState, setPlanState] = useState<PlanState>({ plan: null, error: null, pending: false });
  const [operation, setOperation] = useState<WorkspaceOperation | null>(null);
  const [operationReadError, setOperationReadError] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const [requestRefresh, setRequestRefresh] = useState(0);

  operationRef.current = operation;
  const onCompletedRef = useRef(onCompleted);
  onCompletedRef.current = onCompleted;

  const updateForm = useCallback(<K extends keyof FormState>(key: K, value: FormState[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
    planRequestToken.current += 1;
    setPlanState({ plan: null, error: null, pending: false });
  }, []);
  const updateOpenTarget = useCallback((target: "branch" | "path", value: string) => {
    setForm((current) => ({
      ...current,
      openTarget: target,
      branch: target === "branch" ? value : "",
      checkoutPath: target === "path" ? value : "",
    }));
    planRequestToken.current += 1;
    setPlanState({ plan: null, error: null, pending: false });
  }, []);

  const setWorktreeMode = useCallback((mode: WorkspaceSetupMode) => {
    setForm((current) => ({
      ...current,
      mode,
      base: mode === "open" ? "" : current.base,
      branch: mode === "open" && current.openTarget === "path" ? "" : current.branch,
      checkoutPath: mode === "open" && current.openTarget === "branch" ? "" : current.checkoutPath,
    }));
    planRequestToken.current += 1;
    setPlanState({ plan: null, error: null, pending: false });
  }, []);

  const handleClose = useCallback(() => {
    planRequestToken.current += 1;
    pollToken.current += 1;
    setPlanState({ plan: null, error: null, pending: false });
    setActionPending(false);
    onClose();
  }, [onClose]);

  useEffect(() => {
    const wasOpen = openRef.current;
    openRef.current = open;
    planRequestToken.current += 1;
    if (open) {
      if (!wasOpen && operationRef.current && (operationRef.current.state === "completed" || operationRef.current.state === "cancelled")) {
        operationRef.current = null;
        completedOperation.current = null;
        setOperation(null);
        setOperationError(null);
        setOperationReadError(null);
        setForm(initialForm());
        setStep(1);
      }
      return;
    }
    pollToken.current += 1;
    setPlanState({ plan: null, error: null, pending: false });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    const sessionChanged = loadedSession.current !== null && loadedSession.current !== sessionId;
    loadedSession.current = sessionId;
    const token = ++loadRequestToken.current;
    planRequestToken.current += 1;
    setLoadState("loading");
    setLoadError(null);
    setConfiguration(null);
    setRepositories([]);
    setDiagnostics([]);
    if (sessionChanged) {
      pollToken.current += 1;
      setForm(initialForm());
      setStep(1);
      setPlanState({ plan: null, error: null, pending: false });
      setOperation(null);
      setOperationError(null);
      setOperationReadError(null);
    }
    void Promise.all([client.projectConfiguration(), client.repositories()]).then(([nextConfiguration, response]) => {
      if (!active || token !== loadRequestToken.current) return;
      setConfiguration(nextConfiguration);
      setRepositories(response.repositories);
      setDiagnostics(response.diagnostics);
      setLoadState(response.repositories.length === 0 ? "empty" : "ready");
      const parentRepository = selectedParent ? resolveParentRepository(response.repositories, selectedParent) : null;
      setForm((current) => ({
        ...current,
        repositoryId: current.repositoryId || parentRepository?.repository_id || (selectedParent ? "" : response.repositories[0]?.repository_id || ""),
      }));
    }).catch((error: unknown) => {
      if (!active || token !== loadRequestToken.current) return;
      setLoadState("error");
      setLoadError(errorMessage(error, "Could not load project configuration and repositories."));
    });
    return () => { active = false; };
  }, [client, open, requestRefresh, sessionId, selectedParent?.checkoutPath, selectedParent?.repositoryKey]);


  useEffect(() => {
    if (!open) return;
    const root = dialogRef.current;
    if (!root) return;
    const first = root.querySelector<HTMLElement>("input, select, button");
    first?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const root = dialogRef.current;
    if (!root) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        handleClose();
      }
      if (event.key !== "Tab") return;
      const focusable = [...root.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled])")];
      if (focusable.length === 0) return;
      const current = focusable.indexOf(document.activeElement as HTMLElement);
      const next = event.shiftKey ? (current <= 0 ? focusable.length - 1 : current - 1) : (current + 1) % focusable.length;
      if (current < 0 || next !== current + (event.shiftKey ? -1 : 1)) {
        event.preventDefault();
        focusable[next]?.focus();
      }
    };
    root.addEventListener("keydown", onKeyDown);
    return () => root.removeEventListener("keydown", onKeyDown);
  }, [handleClose, open]);

  const visibleRepositories = repositories.filter((repository) => {
    const query = form.query.trim().toLowerCase();
    return !query || `${repository.name} ${repository.root} ${repository.repository_id}`.toLowerCase().includes(query);
  });
  const selectedRepository = repositories.find((repository) => repository.repository_id === form.repositoryId) ?? null;
  const parentRepository = selectedParent ? resolveParentRepository(repositories, selectedParent) : null;
  const parentRepositoryMatched = parentRepository !== null;

  const createPlan = useCallback(async () => {
    if (!form.repositoryId) {
      setPlanState({ plan: null, error: "Choose a local repository before continuing.", pending: false });
      setStep(1);
      return;
    }
    if (!form.repositoryConsent) {
      setPlanState({ plan: null, error: "Before Review, explicitly consent to the configured repository actions. Stock Herdr cannot suppress those actions.", pending: false });
      return;
    }
    if (form.mode === "open") {
      const branch = form.branch.trim();
      const checkoutPath = form.checkoutPath.trim();
      if (form.base.trim()) {
        setPlanState({ plan: null, error: "Base ref is only available when creating a linked worktree. Remove it before opening an existing checkout.", pending: false });
        return;
      }
      if ((branch.length > 0) === (checkoutPath.length > 0)) {
        setPlanState({ plan: null, error: "Open requires exactly one target: enter a branch or an existing checkout path, not both.", pending: false });
        return;
      }
    }
    const token = ++planRequestToken.current;
    setPlanState({ plan: null, error: null, pending: true });
    try {
      const plan = await client.planWorkspace(sessionId, makeRequest(form));
      if (token !== planRequestToken.current) return;
      if (plan.session_id !== sessionId || plan.repository.repository_id !== form.repositoryId) {
        setPlanState({ plan: null, error: "The setup plan did not match the selected session or repository. Review your selection and try again.", pending: false });
        return;
      }
      setPlanState({ plan, error: null, pending: false });
      setStep(4);
    } catch (error: unknown) {
      if (token !== planRequestToken.current) return;
      setPlanState({ plan: null, error: errorMessage(error, "Could not create a setup plan."), pending: false });
    }
  }, [form, client, sessionId]);

  const acceptOperation = useCallback((next: WorkspaceOperation) => {
    const current = operationRef.current;
    if (current && (current.operation_id !== next.operation_id || !operationSnapshotIsNewer(current, next))) return;
    operationRef.current = next;
    setOperation(next);
    setOperationError(null);
    if (next.state === "completed" && completedOperation.current !== next.operation_id) {
      completedOperation.current = next.operation_id;
      onCompleted(next);
    }
  }, [onCompleted]);
  const start = useCallback(async () => {
    const plan = planState.plan;
    if (!plan || planState.pending || actionPending) return;
    setOperationError(null);
    setActionPending(true);
    try {
      const next = await client.startWorkspace(sessionId, { operation_id: plan.operation_id, expected_generation: plan.generation });
      if (next.operation_id !== plan.operation_id || next.generation < plan.generation) {
        setOperationError("The workspace operation response did not match the reviewed plan. No result was adopted.");
        return;
      }
      acceptOperation(next);
    } catch (error: unknown) {
      setOperationError(errorMessage(error, "Could not start the workspace operation."));
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, planState, client, sessionId]);

  const pollOperation = useCallback(async (operationId: string, generation: number) => {
    const token = ++pollToken.current;
    pollCount.current = 0;
    let last = operationRef.current;
    while (pollToken.current === token && pollCount.current < MAX_POLL_REQUESTS && last && !operationIsTerminal(last.state)) {
      pollCount.current += 1;
      try {
        const next = await client.workspaceOperation(sessionId, operationId);
        if (pollToken.current !== token) return;
        if (next.generation < generation || (last && !operationSnapshotIsNewer(last, next))) {
          await new Promise<void>((resolve) => window.setTimeout(resolve, POLL_INTERVAL_MS));
          continue;
        }
        last = next;
        setOperation((current) => current && operationSnapshotIsNewer(current, next) ? next : current);
        setOperationReadError(null);
        if (next.state === "completed" && completedOperation.current !== next.operation_id) {
          completedOperation.current = next.operation_id;
          onCompletedRef.current(next);
        }
      } catch (error: unknown) {
        if (pollToken.current !== token) return;
        setOperationReadError(errorMessage(error, "Operation snapshot unavailable."));
      }
      if (pollToken.current !== token || !last || operationIsTerminal(last.state)) return;
      await new Promise<void>((resolve) => window.setTimeout(resolve, POLL_INTERVAL_MS));
    }
  }, [client, sessionId]);

  useEffect(() => {
    if (!open || !operation || operationIsTerminal(operation.state)) return;
    void pollOperation(operation.operation_id, operation.generation);
    return () => { pollToken.current += 1; };
  }, [open, operation?.operation_id, operation?.generation, operation?.state, pollOperation]);

  const cancel = useCallback(async () => {
    if (!operation || (operation.state !== "running" && operation.state !== "planned") || actionPending) return;
    setActionPending(true);
    try {
      const next = await client.cancelWorkspace(sessionId, { operation_id: operation.operation_id, expected_generation: operation.generation });
      acceptOperation(next);
    } catch (error: unknown) {
      setOperationError(errorMessage(error, "Could not cancel the operation."));
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, operation, client, sessionId]);

  const resume = useCallback(async () => {
    if (!operation || !operation.resume_allowed || actionPending) return;
    setOperationError(null);
    setActionPending(true);
    try {
      const next = await client.resumeWorkspace(sessionId, { operation_id: operation.operation_id, expected_generation: operation.generation });
      acceptOperation(next);
    } catch (error: unknown) {
      setOperationError(errorMessage(error, "Could not resume the operation."));
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, operation, client, sessionId]);
  const reconcile = useCallback(async (action: WorkspaceRecoveryAction) => {
    if (!operation || recoveryActionFor(operation) !== action || actionPending) return;
    setStep(4);
    setOperationError(null);
    setActionPending(true);
    try {
      const next = await client.reconcileWorkspace(sessionId, {
        operation_id: operation.operation_id,
        expected_generation: operation.generation,
        action,
      });
      acceptOperation(next);
      setOperationReadError(null);
    } catch (error: unknown) {
      setOperationError(errorMessage(error, "Could not reconcile the uncertain operation."));
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, client, operation, sessionId]);


  const selectStep = (next: DialogStep) => {
    if (next === 4 && !planState.plan) {
      void createPlan();
      return;
    }
    setStep(next);
  };

  const onField = (key: keyof FormState) => (event: ChangeEvent<HTMLInputElement | HTMLSelectElement>) => {
    const value = event.target.type === "checkbox" ? (event.target as HTMLInputElement).checked : event.target.value;
    updateForm(key, value as never);
  };

  const diagnosticsWithLoad = loadError ? [{ code: "configuration_unavailable", message: loadError, path: null }] : diagnostics;
  const operationDone = operation !== null && operation.state === "completed";
  if (!open) return null;

  return (
    <div className="setup-overlay" role="presentation">
      <section className="setup-dialog" ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <header className="setup-header"><div><span className="setup-eyebrow">New task Space</span><h2 id={titleId}>Set up a workspace</h2><p>Choose a configured local repository, review exact effects, then create or open through Herdr.</p></div><button type="button" className="setup-close" onClick={handleClose} aria-label="Close setup dialog">Close</button></header>
        <nav className="setup-stepper" aria-label="Setup steps">
          <StepButton step={1} current={step} label="Repository" onSelect={selectStep} />
          <StepButton step={2} current={step} label="Worktree" onSelect={selectStep} />
          <StepButton step={3} current={step} label="Context" onSelect={selectStep} />
          <StepButton step={4} current={step} label="Review" onSelect={selectStep} />
        </nav>
        {operation ? <Progress operation={operation} readError={operationReadError} busy={actionPending} onCancel={cancel} onResume={resume} onReview={reconcile} /> : null}
        {operationError ? <p className="setup-error setup-dialog-error" role="alert">{operationError}</p> : null}
        {!operationDone ? <main className="setup-body">
          {step === 1 ? <section className="setup-step-content" aria-labelledby="repository-heading"><div className="setup-section-heading"><div><h3 id="repository-heading">Local repository</h3><p>Repository selection is required. An artifact URL never clones or discovers a remote repository.</p></div>{configuration ? <span className="setup-config-badge">Config v{configuration.version}</span> : null}</div>
            {selectedParent ? <p className={`setup-selected-note${loadState === "ready" && !parentRepositoryMatched ? " setup-parent-warning" : ""}`} role={loadState === "ready" && !parentRepositoryMatched ? "alert" : "status"}>{loadState === "loading" ? <>Opened from <strong>{selectedParent.label}</strong>; checking its checkout against configured repositories.</> : parentRepositoryMatched ? <>Opened from <strong>{selectedParent.label}</strong>; matched <code>{parentRepository?.root}</code>. Change the selection below only if you intend to use another repository.</> : <>Opened from <strong>{selectedParent.label}</strong>, but no unambiguous configured repository matches its checkout. Choose one explicitly; setup will not switch repositories for you.</>}</p> : null}
            {loadState === "loading" ? <p className="setup-empty">Loading configured repositories…</p> : null}
            {loadState === "error" ? <div className="setup-empty setup-empty-error"><strong>Could not load project setup</strong><p>{loadError}</p><button type="button" onClick={() => setRequestRefresh((value) => value + 1)}>Reload repositories</button></div> : null}
            {loadState === "empty" ? <div className="setup-empty"><strong>No configured local repositories</strong><p>Add a repository root to the startup configuration, then reload. No Herdr workspace was created.</p></div> : null}
            {loadState === "ready" ? <><Field label="Search repositories" htmlFor="setup-repository-search"><input id="setup-repository-search" type="search" value={form.query} onChange={onField("query")} placeholder="Name, path, or repository ID" autoComplete="off" /></Field><div className="setup-repository-list" role="listbox" aria-label="Configured local repositories">{visibleRepositories.length > 0 ? visibleRepositories.map((repository) => <RepositoryChoice key={repository.repository_id} repository={repository} selected={repository.repository_id === form.repositoryId} onSelect={() => updateForm("repositoryId", repository.repository_id)} />) : <p className="setup-empty">No repositories match this search.</p>}</div>{selectedRepository ? <p className="setup-selected-note">Selected <code>{selectedRepository.root}</code>{selectedRepository.is_detached ? " · detached source requires review" : ""}</p> : null}</> : null}
            {configuration ? <div className="setup-config"><strong>Configured roots</strong><ul><li>Repositories: <code>{configuration.repository_roots.length > 0 ? configuration.repository_roots.join(", ") : "(none)"}</code></li><li>Worktrees: <code>{configuration.worktree_root}</code></li><li>Companions: <code>{configuration.companion_root}</code></li><li>State: <code>{configuration.state_root}</code></li></ul></div> : null}
            <DiagnosticList diagnostics={diagnosticsWithLoad} />
            <Field label="Task name" hint="optional" htmlFor="setup-task-name"><input id="setup-task-name" value={form.taskName} onChange={onField("taskName")} placeholder="Short task description" /></Field>
            <Field label="Issue or review URL" hint="optional" htmlFor="setup-artifact-url"><input id="setup-artifact-url" type="url" value={form.artifactUrl} onChange={onField("artifactUrl")} placeholder={issueUrlPlaceholder(configuration)} /></Field>
            <div className="setup-actions"><button type="button" className="setup-primary" disabled={!form.repositoryId || loadState !== "ready"} onClick={() => setStep(2)}>Continue to worktree</button></div>
          </section> : null}
          {step === 2 ? <section className="setup-step-content" aria-labelledby="worktree-heading"><div className="setup-section-heading"><div><h3 id="worktree-heading">Worktree</h3><p>Choose create or open. Branch and destination values are reviewed before any Herdr mutation.</p></div></div><div className="setup-choice-grid"><button type="button" className={form.mode === "create" ? "is-selected" : ""} aria-pressed={form.mode === "create"} onClick={() => setWorktreeMode("create")}><strong>Create linked worktree</strong><span>New checkout below the configured worktree root.</span></button><button type="button" className={form.mode === "open" ? "is-selected" : ""} aria-pressed={form.mode === "open"} onClick={() => setWorktreeMode("open")}><strong>Open existing checkout</strong><span>Use an already-discovered local checkout; it remains borrowed.</span></button></div>{form.mode === "open" ? <><p className="setup-label">Open target</p><div className="setup-choice-grid"><button type="button" className={form.openTarget === "branch" ? "is-selected" : ""} aria-pressed={form.openTarget === "branch"} onClick={() => updateOpenTarget("branch", form.branch)}><strong>Branch</strong><span>Resolve one existing checkout by branch.</span></button><button type="button" className={form.openTarget === "path" ? "is-selected" : ""} aria-pressed={form.openTarget === "path"} onClick={() => updateOpenTarget("path", form.checkoutPath)}><strong>Checkout path</strong><span>Use one exact existing checkout path.</span></button></div>{form.openTarget === "branch" ? <Field label="Branch" hint="required for Open" htmlFor="setup-branch"><input id="setup-branch" value={form.branch} onChange={(event) => updateOpenTarget("branch", event.target.value)} placeholder="feature/task-name" /></Field> : <Field label="Checkout path" hint="required for Open" htmlFor="setup-checkout"><input id="setup-checkout" value={form.checkoutPath} onChange={(event) => updateOpenTarget("path", event.target.value)} placeholder="/absolute/path/to/checkout" /></Field>}</> : <><Field label="Branch" hint="required by policy" htmlFor="setup-branch"><input id="setup-branch" value={form.branch} onChange={onField("branch")} placeholder="feature/task-name" /></Field><Field label="Base ref" hint="optional" htmlFor="setup-base"><input id="setup-base" value={form.base} onChange={onField("base")} placeholder="main" /></Field><Field label="Destination" hint="optional" htmlFor="setup-checkout"><input id="setup-checkout" value={form.checkoutPath} onChange={onField("checkoutPath")} placeholder="Configured worktree root default" /></Field></>}<Field label="Space label" hint="optional" htmlFor="setup-label"><input id="setup-label" value={form.label} onChange={onField("label")} placeholder={form.taskName || "Task workspace"} /></Field><div className="setup-checkboxes"><label><input type="checkbox" checked={form.focus} onChange={onField("focus")} /> Focus the resulting Space and context terminal</label></div><p className="setup-info">Open supplies exactly one branch or checkout path. Base refs are rejected for Open. Create validates branch names and destinations against Git and configured roots; collisions stop before mutation.</p><div className="setup-actions"><button type="button" onClick={() => setStep(1)}>Back</button><button type="button" className="setup-primary" onClick={() => setStep(3)}>Continue to context</button></div></section> : null}
          {step === 3 ? <section className="setup-step-content" aria-labelledby="context-heading"><div className="setup-section-heading"><div><h3 id="context-heading">Context</h3><p>Review the companion association and terminal environment. Start agents manually in the new terminal.</p></div></div><div className="setup-context-card"><strong>Companion association</strong><p>A Cockpit-owned companion is created at the reviewed path. The manifest records the Herdr session, returned workspace, repository provenance, and optional artifact.</p><span className="setup-badge">Selected after planning</span></div><div className="setup-context-card"><strong>Sources</strong><p>{form.artifactUrl.trim() ? "The supplied artifact URL is retained for typed validation; provider downloads remain explicit and bounded." : "No artifact selected. Repository-only setup creates no remote source."}</p></div><div className="setup-context-card"><strong>Installed Context viewer</strong><p>Opening a Context pane requires the configured file-viewer support to be installed and available at the Herdr endpoint. This setup does not install or enable viewer support; Context reports its availability when opened, and source failures remain partial with an explicit retry.</p></div><div className="setup-context-card"><strong>Terminal handoff</strong><p>A new context-aware terminal receives allowlisted <code>COCKPIT_*</code> values for processes Cockpit creates. The original Herdr root pane is unchanged; start an agent manually.</p></div><div className="setup-consent"><strong>Per-operation repository consent</strong><p>Stock Herdr cannot suppress configured repository actions. This consent applies only to this reviewed operation; it is not a Herdr trust flag.</p><label><input type="checkbox" checked={form.repositoryConsent} onChange={onField("repositoryConsent")} /> I understand and consent to the configured repository actions for this operation.</label></div><div className="setup-actions"><button type="button" onClick={() => setStep(2)}>Back</button><button type="button" className="setup-primary" onClick={() => void createPlan()} disabled={planState.pending || !form.repositoryId || !form.repositoryConsent}>{planState.pending ? "Planning…" : "Review exact effects"}</button></div>{planState.error ? <p className="setup-error" role="alert">{planState.error}</p> : null}</section> : null}
          {step === 4 ? <section className="setup-step-content" aria-labelledby="review-heading"><div className="setup-section-heading"><div><h3 id="review-heading">Review before starting</h3><p>Nothing is created until you explicitly start this reviewed operation.</p></div></div>{planState.pending ? <p className="setup-empty">Preparing exact paths and effects…</p> : null}{planState.error ? <p className="setup-error" role="alert">{planState.error}</p> : null}{planState.plan ? <PlanSummary plan={planState.plan} /> : null}<div className="setup-actions"><button type="button" onClick={() => setStep(3)}>Back</button>{planState.plan ? <button type="button" className="setup-primary" onClick={() => void start()} disabled={Boolean(operation) || actionPending || !planState.plan.trust_repository}>Start {modeLabel(planState.plan.mode)}</button> : <button type="button" className="setup-primary" onClick={() => void createPlan()} disabled={planState.pending || !form.repositoryConsent}>Create review plan</button>}</div></section> : null}
        </main> : null}
        {operationDone ? <section className="setup-complete" aria-live="polite"><h3>Workspace setup completed</h3><p>The Herdr workspace, companion association, and context-aware terminal are ready. Start an agent manually in the new terminal; the original root pane was left unchanged.</p><PlanSummary plan={operation.plan} /><div className="setup-actions"><button type="button" className="setup-primary" onClick={handleClose}>Done</button></div></section> : null}
        <footer className="setup-footer"><span>Session <code>{sessionId}</code></span><span>Closing hides this dialog; it does not cancel or remove resources.</span></footer>
      </section>
    </div>
  );
}
