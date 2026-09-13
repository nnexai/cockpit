import { UiIcon } from "../UiIcon";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { rankFuzzyMatches } from "../input/fileNavigation";
import type { CockpitClient } from "../../client/CockpitClient";
import type {
  ProjectConfiguration,
  RepositoryCandidate,
  RepositoryListResponse,
  WorkspaceDefaults,
  WorkspaceOperation,
  WorkspaceOperationState,
  WorkspaceRecoveryAction,
  WorkspaceSetupMode,
  WorkspaceSetupPlan,
  WorkspaceSetupRequest,
} from "../../protocol/generated/v1";
import "./setup.css";

export type SetupClient = Pick<CockpitClient,
  "projectConfiguration" | "repositories" | "resolveWorkspaceDefaults" | "planWorkspace" | "startWorkspace" | "workspaceOperation" | "cancelWorkspace" | "resumeWorkspace" | "reconcileWorkspace"
>;

export type SetupDialogProps = {
  client: SetupClient;
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
  mode: WorkspaceSetupMode;
  branch: string;
  base: string;
  checkoutPath: string;
  openPath: string;
  label: string;
  artifactUrl: string;
  focus: boolean;
};

type TextField = "repositoryId" | "branch" | "base" | "checkoutPath" | "openPath" | "label" | "artifactUrl";
type LoadState = "loading" | "ready" | "empty" | "error";
type PlanState = { plan: WorkspaceSetupPlan | null; error: string | null; pending: boolean };
type SourceState = { defaults: WorkspaceDefaults | null; error: string | null; pending: boolean };
type ExplicitFields = { repository: boolean; branch: boolean; checkoutPath: boolean; label: boolean };

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

function operationCanReset(state: WorkspaceOperationState): boolean {
  return state === "completed" || state === "cancelled";
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
  return fallback;
}

function modeLabel(mode: WorkspaceSetupMode): string {
  return mode === "create" ? "Create Space" : "Open Space";
}

function directoryName(path: string): string {
  return path.trim().split("/").filter(Boolean).at(-1) ?? "";
}

function optional(value: string): string | null {
  return value.trim() || null;
}

function requestLabel(form: FormState): string | null {
  return optional(form.label) ?? (form.mode === "create" ? optional(form.branch) : optional(directoryName(form.openPath)));
}

export function makeRequest(form: FormState): WorkspaceSetupRequest {
  const label = requestLabel(form);
  if (form.mode === "open") {
    return { operation: "open", path: form.openPath.trim(), label, task_name: null, focus: form.focus };
  }
  return {
    operation: "create",
    repository_id: form.repositoryId,
    branch: optional(form.branch),
    base_ref: optional(form.base),
    checkout_path: optional(form.checkoutPath),
    label,
    task_name: null,
    artifact_url: optional(form.artifactUrl),
    focus: form.focus,
  };
}

export function operationStatusMessage(operation: Pick<WorkspaceOperation, "state" | "step" | "error">): string {
  const sourceFailure = operation.error?.code.startsWith("source_") ?? false;
  if (operation.state === "partial" && sourceFailure) return "Source import is partial. Retry the source step to finish Context.";
  if (operation.state === "partial") return "Workspace setup is partial. Review retained resources and retry the failed step.";
  if (operation.state === "needs_review") return "Workspace setup needs review. Inspect retained resources before recovery.";
  switch (operation.step) {
    case "context_preparing": return "Preparing Context.";
    case "context_ready": return "Context is ready. Preparing the terminal.";
    case "completed": return "Space and terminal are ready.";
    case "environment_requested": return "Preparing the terminal.";
    default: return `Setup is at ${operation.step.replaceAll("_", " ")}.`;
  }
}

function operationStepLabel(step: WorkspaceOperation["step"]): string {
  switch (step) {
    case "context_preparing": return "Preparing Context";
    case "context_ready": return "Context ready";
    case "environment_requested": return "Preparing terminal";
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
  return { repositoryId: "", mode: "create", branch: "", base: "", checkoutPath: "", openPath: "", label: "", artifactUrl: "", focus: true };
}

function RepositoryPicker({ repositories, selectedId, loading, disabled, onChoose }: {
  repositories: readonly RepositoryCandidate[];
  selectedId: string;
  loading: boolean;
  disabled: boolean;
  onChoose: (repository: RepositoryCandidate) => void;
}) {
  const pickerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [selection, setSelection] = useState<{ query: string; id: string | null }>({ query: "", id: selectedId || null });
  const previousSelectedId = useRef(selectedId);
  const matches = useMemo(() => rankFuzzyMatches(query, repositories, (repository) => repository.name), [query, repositories]);
  const selectionId = query === "" && previousSelectedId.current !== selectedId ? selectedId : selection.id;
  const active = selection.query === query ? Math.max(0, matches.findIndex((repository) => repository.repository_id === selectionId)) : 0;
  const activeId = matches[active]?.repository_id ?? null;
  const selectIndex = (index: number) => setSelection({ query, id: matches[index]?.repository_id ?? null });

  useEffect(() => {
    setSelection((current) => current.query === query && current.id === activeId ? current : { query, id: activeId });
  }, [activeId, query]);
  useEffect(() => {
    if (query === "" && previousSelectedId.current !== selectedId) {
      previousSelectedId.current = selectedId;
      setSelection({ query, id: selectedId || null });
      return;
    }
    previousSelectedId.current = selectedId;
  }, [query, selectedId]);
  useEffect(() => {
    pickerRef.current?.querySelector<HTMLElement>(`[data-setup-repository-result-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [active, activeId, pickerOpen]);

  const choose = (repository = matches[active]) => {
    if (disabled) return;
    if (repository) {
      setSelection({ query, id: repository.repository_id });
      onChoose(repository);
    }
  };
  const onBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setPickerOpen(false);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      setQuery("");
      inputRef.current?.focus();
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      choose();
      return;
    }
    if (event.key === "ArrowDown" || (event.ctrlKey && event.key.toLowerCase() === "n")) {
      event.preventDefault();
      selectIndex(Math.min(Math.max(0, matches.length - 1), active + 1));
      return;
    }
    if (event.key === "ArrowUp" || (event.ctrlKey && event.key.toLowerCase() === "p")) {
      event.preventDefault();
      selectIndex(Math.max(0, active - 1));
    }
  };

  return <div ref={pickerRef} className="setup-repository-picker" onFocus={() => setPickerOpen(true)} onBlur={onBlur} onKeyDown={onKeyDown}>
    <div className="setup-repository-control">
      <input ref={inputRef} id="setup-repository" type="search" aria-label="Find a repository" role="combobox" aria-expanded={pickerOpen} aria-controls={pickerOpen ? "setup-repository-results" : undefined} aria-autocomplete="list" aria-activedescendant={pickerOpen && activeId ? `setup-repository-result-${active}` : undefined} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Type to find a repository" autoComplete="off" disabled={disabled} />
      <p className="setup-picker-status" aria-live="polite">{loading ? "Loading local repositories…" : `${repositories.length} repositories`}</p>
      {pickerOpen ? <div id="setup-repository-results" className="setup-repository-list" role="listbox" aria-label="Matching repositories">
        {matches.map((repository, index) => <button key={repository.repository_id} id={`setup-repository-result-${index}`} data-setup-repository-result-index={index} type="button" role="option" aria-selected={index === active} className={`setup-repository${repository.repository_id === selectedId ? " is-selected" : ""}${index === active ? " is-active" : ""}`} onFocus={() => selectIndex(index)} onMouseMove={() => selectIndex(index)} onClick={() => choose(repository)} disabled={disabled}>
          <span className="setup-repository-title"><strong>{Array.from(repository.name, (character, characterIndex) => matches[index].matchedIndices.includes(characterIndex) ? <mark key={characterIndex}>{character}</mark> : character)}</strong></span>
          <code>{repository.root}</code>
          {repository.branch ? <span className="setup-repository-meta">{repository.branch}{repository.is_detached ? " · detached" : ""}</span> : null}
        </button>)}
        {!loading && matches.length === 0 ? <p>No matching repositories.</p> : null}
      </div> : null}
    </div>
    <p className="setup-picker-help">↑↓ or Ctrl+N/P to choose · Enter select · Esc clear</p>
  </div>;
}

function Field({ label, hint, children, htmlFor }: { label: string; hint?: string; children: ReactNode; htmlFor?: string }) {
  return <label className="setup-field" htmlFor={htmlFor}>
    <span className="setup-label">{label}{hint ? <span className="setup-hint"> · {hint}</span> : null}</span>
    {children}
  </label>;
}

function DiagnosticList({ diagnostics }: { diagnostics: Array<{ code: string; message: string; path?: string | null }> }) {
  if (diagnostics.length === 0) return null;
  return <div className="setup-diagnostics" role="status">
    <strong>Configuration diagnostics</strong>
    <ul>{diagnostics.map((diagnostic, index) => <li key={`${diagnostic.code}-${diagnostic.path ?? ""}-${index}`}>
      <code>{diagnostic.code}</code> {diagnostic.message}
      {diagnostic.path ? <code className="setup-diagnostic-path">{diagnostic.path}</code> : null}
    </li>)}</ul>
  </div>;
}

function PlanDetails({ plan }: { plan: WorkspaceSetupPlan }) {
  return <details className="setup-disclosure">
    <summary>Operation details</summary>
    <div className="setup-plan-summary">
      <div className="setup-summary-row"><span>Operation</span><strong>{modeLabel(plan.mode)}</strong></div>
      <div className="setup-summary-row"><span>Ownership</span><span>{plan.ownership.replaceAll("_", " ")}</span></div>
      <div className="setup-summary-row"><span>Checkout</span><code>{plan.checkout_path}</code></div>
      <div className="setup-summary-row"><span>Companion</span><code>{plan.companion_path}</code></div>
      {plan.repository ? <div className="setup-summary-row"><span>Repository</span><code>{plan.repository.root}</code></div> : null}
      {plan.branch ? <div className="setup-summary-row"><span>Branch</span><code>{plan.branch}{plan.base ? ` ← ${plan.base}` : ""}</code></div> : null}
      {plan.artifact ? <div className="setup-summary-row"><span>Source</span><span>{plan.artifact.kind} · {plan.artifact.canonical_id}</span></div> : null}
      <div className="setup-effects"><strong>Effects</strong><ul>{plan.effects.map((effect, index) => <li key={`${effect}-${index}`}>{effect}</li>)}</ul></div>
      {plan.warnings.length > 0 ? <div className="setup-warnings"><strong>Warnings</strong><ul>{plan.warnings.map((warning, index) => <li key={`${warning}-${index}`}>{warning}</li>)}</ul></div> : null}
    </div>
  </details>;
}

function Progress({ operation, readError, busy, onCancel, onResume, onReview }: {
  operation: WorkspaceOperation;
  readError: string | null;
  busy: boolean;
  onCancel: () => void;
  onResume: () => void;
  onReview: (action: WorkspaceRecoveryAction) => void;
}) {
  const failed = operation.state === "partial" || operation.state === "needs_review";
  const recoveryAction = recoveryActionFor(operation);
  const retrySource = sourceRetry(operation);
  const recoveryLabel = recoveryAction === "accept_existing_worktree" ? "Recover existing checkout" : recoveryAction === "retry_environment" ? "Retry environment" : null;
  return <section className="setup-progress" aria-live="polite" aria-busy={busy} aria-label="Workspace setup progress">
    <div className="setup-progress-heading"><h3>{operationStepLabel(operation.step)}</h3><span className={`setup-state setup-state-${operation.state}`}>{operation.state.replaceAll("_", " ")}</span></div>
    <div className="setup-progress-track"><span style={{ width: `${operation.step === "completed" ? 100 : Math.min(94, Math.max(8, (operation.sequence + 1) * 10))}%` }} /></div>
    <p className="setup-progress-message" role="status">{operationStatusMessage(operation)}</p>
    {readError ? <p className="setup-transient" role="status">Could not read the latest operation status. Showing the last confirmed state.</p> : null}
    {operation.error && operation.state !== "running" ? <p className="setup-error" role="alert">{operation.error.message}</p> : null}
    {operation.owned_resources.length > 0 ? <details className="setup-disclosure"><summary>Retained resources</summary><div className="setup-owned"><ul>{operation.owned_resources.map((resource, index) => <li key={`${resource.kind}-${resource.path}-${index}`}><code>{resource.path}</code> · {resource.created_by_operation ? "created by this operation" : "existing resource"}</li>)}</ul></div></details> : null}
    <div className="setup-progress-actions">
      {operation.state === "running" || operation.state === "planned" ? <button type="button" onClick={onCancel} disabled={busy}>Cancel operation</button> : null}
      {failed && operation.resume_allowed ? <button type="button" className="setup-primary" onClick={onResume} disabled={busy}>{retrySource ? "Retry source import" : "Resume failed step"}</button> : null}
      {recoveryAction && recoveryLabel ? <button type="button" className="setup-primary" onClick={() => onReview(recoveryAction)} disabled={busy}>{recoveryLabel}</button> : null}
    </div>
  </section>;
}

export function SetupDialog({ client, sessionId, open, selectedParent = null, onClose, onCompleted }: SetupDialogProps) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const loadRequestToken = useRef(0);
  const defaultsRequestToken = useRef(0);
  const planRequestToken = useRef(0);
  const pollToken = useRef(0);
  const completedOperation = useRef<string | null>(null);
  const operationRef = useRef<WorkspaceOperation | null>(null);
  const dispatchRef = useRef(false);
  const pollCount = useRef(0);
  const loadedSession = useRef<string | null>(null);
  const openRef = useRef(open);
  const explicit = useRef<ExplicitFields>({ repository: false, branch: false, checkoutPath: false, label: false });
  const [form, setForm] = useState<FormState>(initialForm);
  const [configuration, setConfiguration] = useState<ProjectConfiguration | null>(null);
  const [repositories, setRepositories] = useState<RepositoryCandidate[]>([]);
  const [diagnostics, setDiagnostics] = useState<RepositoryListResponse["diagnostics"]>([]);
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [sourceState, setSourceState] = useState<SourceState>({ defaults: null, error: null, pending: false });
  const [planState, setPlanState] = useState<PlanState>({ plan: null, error: null, pending: false });
  const [operation, setOperation] = useState<WorkspaceOperation | null>(null);
  const [operationReadError, setOperationReadError] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [pathError, setPathError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const [requestRefresh, setRequestRefresh] = useState(0);
  const [lookupRevision, setLookupRevision] = useState(0);

  operationRef.current = operation;
  const onCompletedRef = useRef(onCompleted);
  onCompletedRef.current = onCompleted;

  const invalidatePlan = useCallback(() => {
    if (dispatchRef.current) return;
    planRequestToken.current += 1;
    setPlanState({ plan: null, error: null, pending: false });
  }, []);

  const updateText = useCallback((key: TextField, value: string, manuallyEdited = false) => {
    if (dispatchRef.current) return;
    if (manuallyEdited) {
      if (key === "repositoryId") explicit.current.repository = true;
      if (key === "branch") explicit.current.branch = true;
      if (key === "checkoutPath") explicit.current.checkoutPath = true;
      if (key === "label") explicit.current.label = true;
    }
    if (key === "artifactUrl" || key === "repositoryId") {
      defaultsRequestToken.current += 1;
      if (key === "artifactUrl") setSourceState({ defaults: null, error: null, pending: false });
    }
    if (key === "repositoryId" && manuallyEdited) setLookupRevision((current) => current + 1);
    setForm((current) => {
      if (key !== "artifactUrl") return { ...current, [key]: value };
      return {
        ...current,
        artifactUrl: value,
        repositoryId: explicit.current.repository ? current.repositoryId : "",
        branch: explicit.current.branch ? current.branch : "",
        checkoutPath: explicit.current.checkoutPath ? current.checkoutPath : "",
        label: explicit.current.label ? current.label : "",
      };
    });
    if (key === "checkoutPath" || key === "openPath") setPathError(null);
    invalidatePlan();
  }, [invalidatePlan]);

  const updateFocus = useCallback((focus: boolean) => {
    if (dispatchRef.current) return;
    setForm((current) => ({ ...current, focus }));
    invalidatePlan();
  }, [invalidatePlan]);

  const setMode = useCallback((mode: WorkspaceSetupMode) => {
    if (dispatchRef.current) return;
    defaultsRequestToken.current += 1;
    setSourceState({ defaults: null, error: null, pending: false });
    setForm((current) => ({ ...current, mode }));
    setPathError(null);
    invalidatePlan();
  }, [invalidatePlan]);

  const resetCompletedOperation = useCallback(() => {
    dispatchRef.current = false;
    completedOperation.current = null;
    explicit.current = { repository: false, branch: false, checkoutPath: false, label: false };
    setForm(initialForm());
    setPlanState({ plan: null, error: null, pending: false });
    setOperation(null);
    setOperationError(null);
    setOperationReadError(null);
  }, []);

  const handleClose = useCallback(() => {
    defaultsRequestToken.current += 1;
    if (!dispatchRef.current) invalidatePlan();
    pollToken.current += 1;
    onClose();
  }, [invalidatePlan, onClose]);

  useEffect(() => {
    const wasOpen = openRef.current;
    openRef.current = open;
    defaultsRequestToken.current += 1;
    if (open && !wasOpen && operationRef.current && operationCanReset(operationRef.current.state)) resetCompletedOperation();
    if (!open) pollToken.current += 1;
  }, [open, resetCompletedOperation]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    const sessionChanged = loadedSession.current !== null && loadedSession.current !== sessionId;
    loadedSession.current = sessionId;
    const token = ++loadRequestToken.current;
    setLoadState("loading");
    setLoadError(null);
    setConfiguration(null);
    setRepositories([]);
    setDiagnostics([]);
    if (sessionChanged && !dispatchRef.current) resetCompletedOperation();
    void Promise.all([client.projectConfiguration(), client.repositories()]).then(([nextConfiguration, response]) => {
      if (!active || token !== loadRequestToken.current) return;
      setConfiguration(nextConfiguration);
      setRepositories(response.repositories);
      setDiagnostics(response.diagnostics);
      setLoadState(response.repositories.length === 0 ? "empty" : "ready");
      const parentRepository = selectedParent ? resolveParentRepository(response.repositories, selectedParent) : null;
      if (!dispatchRef.current) setForm((current) => ({ ...current, repositoryId: current.repositoryId || parentRepository?.repository_id || (selectedParent ? "" : response.repositories[0]?.repository_id || "") }));
    }).catch((error: unknown) => {
      if (!active || token !== loadRequestToken.current) return;
      setLoadState("error");
      setLoadError(errorMessage(error, "Could not load project setup."));
    });
    return () => { active = false; };
  }, [client, open, requestRefresh, resetCompletedOperation, selectedParent?.checkoutPath, selectedParent?.repositoryKey, sessionId]);

  useEffect(() => {
    if (!open || form.mode !== "create" || dispatchRef.current) return;
    const artifactUrl = form.artifactUrl.trim();
    if (!artifactUrl) {
      defaultsRequestToken.current += 1;
      setSourceState({ defaults: null, error: null, pending: false });
      return;
    }
    const token = ++defaultsRequestToken.current;
    setSourceState({ defaults: null, error: null, pending: true });
    const timeout = window.setTimeout(() => {
      void client.resolveWorkspaceDefaults({
        artifact_url: artifactUrl,
        repository_id: explicit.current.repository ? form.repositoryId || null : null,
      }).then((defaults) => {
        if (token !== defaultsRequestToken.current || dispatchRef.current) return;
        setSourceState({ defaults, error: null, pending: false });
        invalidatePlan();
        setForm((current) => ({
          ...current,
          repositoryId: explicit.current.repository ? current.repositoryId : defaults.repository_id ?? current.repositoryId,
          branch: explicit.current.branch ? current.branch : defaults.branch ?? current.branch,
          checkoutPath: explicit.current.checkoutPath ? current.checkoutPath : defaults.checkout_path ?? current.checkoutPath,
          label: explicit.current.label ? current.label : defaults.label ?? current.label,
        }));
      }).catch((error: unknown) => {
        if (token !== defaultsRequestToken.current || dispatchRef.current) return;
        setSourceState({ defaults: null, error: errorMessage(error, "Could not resolve this source. Enter repository and branch manually."), pending: false });
      });
    }, 300);
    return () => window.clearTimeout(timeout);
  }, [client, form.artifactUrl, form.mode, invalidatePlan, lookupRevision, open]);

  useEffect(() => {
    if (!open) return;
    dialogRef.current?.querySelector<HTMLElement>("input, select, button")?.focus();
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
      const current = focusable.findIndex((item) => item === document.activeElement);
      const next = event.shiftKey ? (current <= 0 ? focusable.length - 1 : current - 1) : (current + 1) % focusable.length;
      if (current < 0 || next !== current + (event.shiftKey ? -1 : 1)) {
        event.preventDefault();
        focusable[next]?.focus();
      }
    };
    root.addEventListener("keydown", onKeyDown);
    return () => root.removeEventListener("keydown", onKeyDown);
  }, [handleClose, open]);

  const acceptOperation = useCallback((next: WorkspaceOperation) => {
    const current = operationRef.current;
    if (current && (current.operation_id !== next.operation_id || !operationSnapshotIsNewer(current, next))) return;
    operationRef.current = next;
    setOperation(next);
    setOperationError(null);
    if (next.state === "completed" && completedOperation.current !== next.operation_id) {
      completedOperation.current = next.operation_id;
      onCompletedRef.current(next);
    }
  }, []);

  const inspectReceipt = useCallback(async () => {
    const plan = planState.plan;
    if (!plan || actionPending) return;
    setActionPending(true);
    setOperationError(null);
    try {
      const next = await client.workspaceOperation(plan.session_id, plan.operation_id);
      if (next.operation_id !== plan.operation_id || next.generation < plan.generation) {
        setOperationError("The operation status did not match the retained setup receipt.");
        return;
      }
      acceptOperation(next);
      setOperationReadError(null);
    } catch (error: unknown) {
      setOperationError(errorMessage(error, "Could not read the retained operation. Its outcome is still unknown."));
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, client, planState.plan, sessionId]);

  const submit = useCallback(async () => {
    if (planState.pending || actionPending || dispatchRef.current) return;
    if (form.mode === "open" && !form.openPath.trim()) {
      setPathError("Enter the directory to open.");
      return;
    }
    if (form.mode === "create" && !form.repositoryId) {
      setPlanState({ plan: null, error: "Choose a local repository.", pending: false });
      return;
    }
    const token = ++planRequestToken.current;
    setPlanState({ plan: null, error: null, pending: true });
    setOperationError(null);
    try {
      const plan = await client.planWorkspace(sessionId, makeRequest(form));
      if (token !== planRequestToken.current || dispatchRef.current) return;
      if (plan.session_id !== sessionId || plan.mode !== form.mode || (form.mode === "create" && plan.repository?.repository_id !== form.repositoryId)) {
        setPlanState({ plan: null, error: "The setup plan did not match the current form. Check the fields and try again.", pending: false });
        return;
      }
      dispatchRef.current = true;
      setPlanState({ plan, error: null, pending: false });
      setActionPending(true);
      try {
        const next = await client.startWorkspace(sessionId, { operation_id: plan.operation_id, expected_generation: plan.generation });
        if (next.operation_id !== plan.operation_id || next.generation < plan.generation) {
          setOperationError("The workspace operation response did not match the retained setup receipt.");
          return;
        }
        acceptOperation(next);
      } catch (error: unknown) {
        setOperationError(errorMessage(error, "Start request outcome is unknown. Cockpit retained the operation receipt and is checking its status."));
        try {
          const next = await client.workspaceOperation(sessionId, plan.operation_id);
          if (next.operation_id !== plan.operation_id || next.generation < plan.generation) {
            setOperationError("Start request outcome is unknown. The returned operation did not match the retained receipt.");
            return;
          }
          acceptOperation(next);
        } catch (inspectionError: unknown) {
          setOperationError(`${errorMessage(error, "Start request outcome is unknown.")} ${errorMessage(inspectionError, "The operation receipt is retained. Use Check operation before any further action.")}`);
        }
      }
    } catch (error: unknown) {
      if (token !== planRequestToken.current || dispatchRef.current) return;
      const message = errorMessage(error, "Could not set up this workspace.");
      if (form.mode === "open") {
        setPathError(message);
        setPlanState((current) => ({ ...current, pending: false }));
      } else {
        setPlanState((current) => ({ ...current, error: message, pending: false }));
      }
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, client, form, planState.pending, sessionId]);

  const pollOperation = useCallback(async (operationId: string, generation: number) => {
    const token = ++pollToken.current;
    pollCount.current = 0;
    let last = operationRef.current;
    while (pollToken.current === token && pollCount.current < MAX_POLL_REQUESTS && last && !operationIsTerminal(last.state)) {
      pollCount.current += 1;
      try {
        const next = await client.workspaceOperation(last.session_id, operationId);
        if (pollToken.current !== token) return;
        if (next.generation >= generation && operationSnapshotIsNewer(last, next)) {
          last = next;
          acceptOperation(next);
          setOperationReadError(null);
        }
      } catch (error: unknown) {
        if (pollToken.current !== token) return;
        setOperationReadError(errorMessage(error, "Operation status unavailable."));
      }
      if (pollToken.current !== token || !last || operationIsTerminal(last.state)) return;
      await new Promise<void>((resolve) => window.setTimeout(resolve, POLL_INTERVAL_MS));
    }
  }, [acceptOperation, client, sessionId]);

  useEffect(() => {
    if (!open || !operation || operationIsTerminal(operation.state)) return;
    void pollOperation(operation.operation_id, operation.generation);
    return () => { pollToken.current += 1; };
  }, [open, operation?.operation_id, operation?.generation, operation?.state, pollOperation]);

  const cancel = useCallback(async () => {
    if (!operation || (operation.state !== "running" && operation.state !== "planned") || actionPending) return;
    setActionPending(true);
    try { acceptOperation(await client.cancelWorkspace(operation.session_id, { operation_id: operation.operation_id, expected_generation: operation.generation })); }
    catch (error: unknown) { setOperationError(errorMessage(error, "Could not cancel the operation.")); }
    finally { setActionPending(false); }
  }, [acceptOperation, actionPending, client, operation, sessionId]);

  const resume = useCallback(async () => {
    if (!operation || !operation.resume_allowed || actionPending) return;
    setActionPending(true);
    try { acceptOperation(await client.resumeWorkspace(operation.session_id, { operation_id: operation.operation_id, expected_generation: operation.generation })); }
    catch (error: unknown) { setOperationError(errorMessage(error, "Could not resume the operation.")); }
    finally { setActionPending(false); }
  }, [acceptOperation, actionPending, client, operation, sessionId]);

  const reconcile = useCallback(async (action: WorkspaceRecoveryAction) => {
    if (!operation || recoveryActionFor(operation) !== action || actionPending) return;
    setActionPending(true);
    try { acceptOperation(await client.reconcileWorkspace(operation.session_id, { operation_id: operation.operation_id, expected_generation: operation.generation, action })); }
    catch (error: unknown) { setOperationError(errorMessage(error, "Could not reconcile the operation.")); }
    finally { setActionPending(false); }
  }, [acceptOperation, actionPending, client, operation, sessionId]);

  const onTextField = (key: TextField, manuallyEdited = false) => (event: ChangeEvent<HTMLInputElement | HTMLSelectElement>) => updateText(key, event.currentTarget.value, manuallyEdited);
  const onFocusChange = (event: ChangeEvent<HTMLInputElement>) => updateFocus(event.currentTarget.checked);
  const selectedRepository = repositories.find((repository) => repository.repository_id === form.repositoryId);
  const sourceRepositories = sourceState.defaults?.repositories ?? repositories;
  const diagnosticsWithLoad = loadError ? [{ code: "configuration_unavailable", message: loadError, path: null }] : diagnostics;
  const editingLocked = dispatchRef.current;

  if (!open) return null;
  return <div className="setup-overlay" role="presentation">
    <section className="setup-dialog setup-dialog-compact" ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby={titleId}>
      <header className="setup-header"><h2 id={titleId}>New Space</h2><span className="setup-session">{sessionId}</span><button type="button" className="setup-close" onClick={handleClose} aria-label="Close setup dialog"><UiIcon name="close" /></button></header>
      <main className="setup-body"><section className="setup-step-content setup-compact-form">
        {form.mode === "create" ? <>
          <Field label="Issue / MR URL" htmlFor="setup-artifact-url"><input id="setup-artifact-url" type="url" value={form.artifactUrl} onChange={onTextField("artifactUrl")} placeholder="Optional URL" disabled={editingLocked} /></Field>
          {sourceState.pending ? <p className="setup-inline-status">Resolving source defaults…</p> : null}
          {sourceState.error ? <p className="setup-error" role="alert">{sourceState.error}</p> : null}
          {sourceState.defaults?.artifact ? <p className="setup-inline-status is-valid">Resolved {sourceState.defaults.artifact.kind} · {sourceState.defaults.artifact.canonical_id}</p> : null}
          {loadState !== "error" && loadState !== "empty" ? <div className="setup-field"><label className="setup-label" htmlFor="setup-repository">Repository</label><RepositoryPicker repositories={sourceRepositories} selectedId={form.repositoryId} loading={loadState === "loading"} disabled={editingLocked} onChoose={(repository) => updateText("repositoryId", repository.repository_id, true)} /></div> : null}
        </> : null}
        <div className="setup-field"><span className="setup-label" id="setup-operation-label">Operation</span><div className="viewer-segmented setup-operation-choice" role="group" aria-labelledby="setup-operation-label">
          <button type="button" aria-pressed={form.mode === "create"} onClick={() => setMode("create")} disabled={editingLocked}>New worktree</button>
          <button type="button" aria-pressed={form.mode === "open"} onClick={() => setMode("open")} disabled={editingLocked}>Existing directory</button>
        </div></div>
        {form.mode === "open" ? <>
          <Field label="Path" htmlFor="setup-checkout"><input id="setup-checkout" value={form.openPath} onChange={onTextField("openPath", true)} placeholder="/absolute/path/to/directory" aria-invalid={Boolean(pathError)} aria-describedby={pathError ? "setup-path-error" : undefined} disabled={editingLocked} /></Field>
          {pathError ? <p id="setup-path-error" className="setup-error" role="alert">{pathError}</p> : null}
          <Field label="Space name" htmlFor="setup-label"><input id="setup-label" value={form.label} onChange={onTextField("label", true)} placeholder="Defaults to directory name" disabled={editingLocked} /></Field>

        </> : <>
          {loadState === "error" ? <div className="setup-empty setup-empty-error"><strong>Could not load project setup</strong><p>{loadError}</p><button type="button" onClick={() => setRequestRefresh((value) => value + 1)} disabled={editingLocked}>Reload</button></div> : null}
          {loadState === "empty" ? <p className="setup-empty">No configured local repositories are available.</p> : null}
          <Field label="Branch" htmlFor="setup-branch"><input id="setup-branch" value={form.branch} onChange={onTextField("branch", true)} placeholder="Configured default" disabled={editingLocked} /></Field>
          <Field label="Space name" htmlFor="setup-label"><input id="setup-label" value={form.label} onChange={onTextField("label", true)} placeholder={form.branch || "Defaults to branch"} disabled={editingLocked} /></Field>
          <details className="setup-disclosure"><summary>Advanced</summary><Field label="Base" htmlFor="setup-base"><input id="setup-base" value={form.base} onChange={onTextField("base")} placeholder="Configured default" disabled={editingLocked} /></Field><Field label="Destination" htmlFor="setup-destination"><input id="setup-destination" value={form.checkoutPath} onChange={onTextField("checkoutPath", true)} placeholder="Automatic backend default" disabled={editingLocked} /></Field></details>
        </>}
        <details className="setup-disclosure"><summary>Operation details</summary>
          <p className="setup-operation-summary">{form.mode === "create" ? "Create a linked worktree, a Herdr Space, and companion context. Configured repository actions run automatically." : "Open this directory in a Herdr Space. The directory and its files remain yours."}</p>
          {selectedRepository ? <p className="setup-selected-note"><code>{selectedRepository.root}</code></p> : null}
          <label className="setup-focus"><input type="checkbox" checked={form.focus} onChange={onFocusChange} disabled={editingLocked} /> Focus the resulting Space and terminal</label>
          {configuration ? <div className="setup-config">Worktrees <code>{configuration.worktree_root}</code></div> : null}
          <DiagnosticList diagnostics={diagnosticsWithLoad} />
        </details>
        {planState.error ? <p className="setup-error" role="alert">{planState.error}</p> : null}
        {planState.plan ? <PlanDetails plan={planState.plan} /> : null}
        {operation ? <Progress operation={operation} readError={operationReadError} busy={actionPending} onCancel={cancel} onResume={resume} onReview={reconcile} /> : null}
        {!operation && planState.plan ? <div className="setup-actions"><button type="button" onClick={handleClose}>Close</button><button type="button" className="setup-primary" onClick={() => void inspectReceipt()} disabled={actionPending}>{actionPending ? "Checking…" : "Check operation"}</button></div> : null}
        {!operation && !planState.plan ? <div className="setup-actions"><button type="button" onClick={handleClose}>Cancel</button><button type="button" className="setup-primary" disabled={planState.pending || actionPending || (form.mode === "create" && (!form.repositoryId || loadState !== "ready"))} onClick={() => void submit()}>{planState.pending || actionPending ? "Preparing…" : modeLabel(form.mode)}</button></div> : null}
        {operationError ? <p className="setup-error" role="alert">{operationError}</p> : null}
      </section></main>
    </section>
  </div>;
}
