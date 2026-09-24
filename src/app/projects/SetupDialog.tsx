import { UiIcon } from "../UiIcon";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { rankFuzzyMatches } from "../input/fileNavigation";
import type { CockpitClient } from "../../client/CockpitClient";
import type {
  LinkedArtifact,
  PaneSummary,
  RepositoryCandidate,
  RepositoryListResponse,
  SpaceSummary,
  WorkspaceDefaults,
  WorkspaceOperation,
  WorkspaceOperationState,
  WorkspaceRecoveryAction,
  WorkspaceSetupMode,
  WorkspaceSetupPlan,
  WorkspaceSetupRequest,
} from "../../protocol/generated/v1";
import "./setup.css";
import "./taskSetup.css";

export type SetupClient = Pick<CockpitClient,
  "repositories" | "sessionSnapshot" | "resolveWorkspaceDefaults" | "planWorkspace" | "startWorkspace" | "workspaceOperation" | "cancelWorkspace" | "resumeWorkspace" | "reconcileWorkspace"
>;

export type SetupDialogProps = {
  client: SetupClient;
  sessionId: string;
  open: boolean;
  selectedParent?: SetupParent | null;
  /** The Space the dialog opened from; its pane folders are re-read on open. */
  parentSpaceId?: string | null;
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

type TextField = "branch" | "base" | "checkoutPath" | "openPath" | "label" | "artifactUrl";
type LoadState = "loading" | "ready" | "empty" | "error";
/** A plan belongs to the exact request it was prepared for. */
type PlanState = { key: string | null; plan: WorkspaceSetupPlan | null; error: string | null; pending: boolean };
type SourceState = { defaults: WorkspaceDefaults | null; error: string | null; pending: boolean };
type ExplicitFields = { repository: boolean; branch: boolean; checkoutPath: boolean; label: boolean };

const POLL_INTERVAL_MS = 700;
const MAX_POLL_REQUESTS = 180;
const PLAN_DELAY_MS = 350;
const SOURCE_DELAY_MS = 300;

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
function errorCode(error: unknown): string | null {
  if (typeof error === "object" && error !== null && "operationCode" in error && typeof error.operationCode === "string") return error.operationCode;
  if (typeof error === "object" && error !== null && "code" in error && typeof error.code === "string") return error.code;
  return null;
}

function requiresPlanReview(error: unknown): boolean {
  const code = errorCode(error);
  return code === "stale_plan"
    || code === "stale_identity"
    || code === "repository_identity_stale"
    || code?.startsWith("source_") === true;
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

export function makeRequest(form: FormState, linkedArtifactUrls: readonly string[] = []): WorkspaceSetupRequest {
  const label = requestLabel(form);
  if (form.mode === "open") {
    return { operation: "open", path: form.openPath.trim(), label, task_name: null, focus: form.focus };
  }
  const artifactUrl = optional(form.artifactUrl);
  return {
    operation: "create",
    repository_id: form.repositoryId,
    branch: optional(form.branch),
    base_ref: optional(form.base),
    checkout_path: optional(form.checkoutPath),
    label,
    task_name: null,
    artifact_url: artifactUrl,
    linked_artifact_urls: artifactUrl ? [...linkedArtifactUrls] : [],
    focus: form.focus,
  };
}

export function operationStatusMessage(operation: Pick<WorkspaceOperation, "state" | "step" | "error">): string {
  const sourceFailure = operation.error?.code.startsWith("source_") ?? false;
  if (operation.state === "partial" && sourceFailure) return "The Space is ready, but a source import failed. Retry the source step to finish Context.";
  if (operation.state === "partial") return "Setup stopped part way. Retry the failed step; nothing is created twice.";
  if (operation.state === "needs_review") return "Setup needs review. Check the retained resources before recovery.";
  if (operation.state === "outcome_unknown") return "Herdr did not confirm the last step. Recover it before trying again.";
  if (operation.state === "cancelled") return "Setup was cancelled.";
  switch (operation.step) {
    case "planned":
    case "validated":
    case "herdr_requested":
    case "herdr_observed":
      return "Creating the worktree and Space…";
    case "worktree_ready":
    case "workspace_verified":
    case "companion_ready":
      return "Preparing the Space…";
    case "environment_requested":
    case "environment_ready":
      return "Opening the terminal…";
    case "context_preparing": return "Preparing Context…";
    case "context_ready": return "Context is ready. Finishing…";
    case "completed": return "Space and terminal are ready.";
    default: return "Setting up…";
  }
}

function sourceRetry(operation: WorkspaceOperation): boolean {
  return operation.error?.code.startsWith("source_") ?? false;
}

/** The Space's own checkout, else the focused pane's folder (may be nested). */
export function setupParentFor(space: SpaceSummary | undefined, panes: PaneSummary[], focusedPaneId: string | null): SetupParent | null {
  if (!space) return null;
  if (space.git) return { label: space.label, repositoryKey: space.git.repository_key, checkoutPath: space.git.checkout_path };
  const spacePanes = panes.filter((pane) => pane.space_id === space.id);
  const pane = spacePanes.find((candidate) => candidate.id === focusedPaneId) ?? spacePanes.find((candidate) => candidate.focused) ?? spacePanes[0];
  return pane?.cwd ? { label: space.label, repositoryKey: "", checkoutPath: pane.cwd } : null;
}

function contains(folder: string, path: string): boolean {
  return path === folder || path.startsWith(folder.endsWith("/") ? folder : `${folder}/`);
}

export function resolveParentRepository(repositories: RepositoryCandidate[], parent: SetupParent): RepositoryCandidate | null {
  const checkoutMatches = repositories.filter((repository) => repository.checkout_path === parent.checkoutPath || repository.root === parent.checkoutPath);
  if (checkoutMatches.length === 1) return checkoutMatches[0];
  if (checkoutMatches.length > 1) return null;
  const commonMatches = parent.repositoryKey ? repositories.filter((repository) => repository.common_dir === parent.repositoryKey) : [];
  if (commonMatches.length === 1) return commonMatches[0];
  // A pane folder inside a checkout: the innermost containing checkout wins.
  const containing = repositories.filter((repository) => contains(repository.checkout_path, parent.checkoutPath));
  const depth = Math.max(-1, ...containing.map((repository) => repository.checkout_path.length));
  const innermost = containing.filter((repository) => repository.checkout_path.length === depth);
  return innermost.length === 1 ? innermost[0] : null;
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

function shortPath(path: string): string {
  const home = /^\/home\/[^/]+|^\/Users\/[^/]+/.exec(path)?.[0];
  return home ? `~${path.slice(home.length)}` : path;
}

function artifactLabel(defaults: WorkspaceDefaults): string {
  const { artifact } = defaults;
  const id = artifact.kind === "review" && artifact.canonical_id.includes("!") ? `!${artifact.canonical_id.split("!").at(-1)}`
    : artifact.kind === "issue" && artifact.canonical_id.includes("#") ? `#${artifact.canonical_id.split("#").at(-1)}`
      : artifact.canonical_id;
  const kind = artifact.kind === "review" ? "MR" : "Issue";
  return `${kind} ${id}${defaults.title ? ` · ${defaults.title}` : ""}`;
}

/** File-picker style repository choice: the field shows the chosen
 * repository; typing filters by fuzzy match, and Enter takes the highlighted
 * match (the best one unless the arrows moved). */
function RepositoryPicker({ repositories, selected, loading, disabled, onChoose }: {
  repositories: readonly RepositoryCandidate[];
  selected: RepositoryCandidate | null;
  loading: boolean;
  disabled: boolean;
  onChoose: (repository: RepositoryCandidate) => void;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const listOpen = query !== null;
  const matches = useMemo(() => rankFuzzyMatches(query ?? "", repositories, (repository) => repository.name).slice(0, 50), [query, repositories]);
  const activeIndex = Math.min(active, Math.max(0, matches.length - 1));

  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-setup-repository-result-index="${activeIndex}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex, listOpen]);

  const close = () => { setQuery(null); setActive(0); };
  const choose = (repository: RepositoryCandidate | undefined) => {
    if (!repository || disabled) return;
    onChoose(repository);
    close();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    const down = event.key === "ArrowDown" || (event.ctrlKey && event.key.toLowerCase() === "n");
    const up = event.key === "ArrowUp" || (event.ctrlKey && event.key.toLowerCase() === "p");
    if (!listOpen) {
      if (down) { event.preventDefault(); setQuery(""); setActive(0); }
      return;
    }
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (event.key === "Enter") { event.preventDefault(); event.stopPropagation(); choose(matches[activeIndex]); return; }
    if (event.key === "Tab") { if (query) choose(matches[activeIndex]); else close(); return; }
    if (down) { event.preventDefault(); setActive(Math.min(matches.length - 1, activeIndex + 1)); return; }
    if (up) { event.preventDefault(); setActive(Math.max(0, activeIndex - 1)); }
  };
  const status = loading ? "Loading repositories…" : repositories.length === 0 ? "No repositories found" : undefined;

  return <div className="task-setup-picker">
    <input ref={inputRef} id="setup-repository" type="text" role="combobox" aria-label="Repository" aria-expanded={listOpen} aria-controls={listOpen ? "setup-repository-results" : undefined} aria-autocomplete="list"
      aria-activedescendant={listOpen && matches[activeIndex] ? `setup-repository-result-${activeIndex}` : undefined}
      value={query ?? selected?.name ?? ""} placeholder={status ?? "Type to find a repository"} autoComplete="off" spellCheck={false} disabled={disabled}
      onChange={(event) => { setQuery(event.target.value); setActive(0); }}
      onFocus={(event) => event.currentTarget.select()}
      onBlur={() => close()}
      onKeyDown={onKeyDown} />
    {listOpen ? <div ref={listRef} id="setup-repository-results" className="task-setup-results" role="listbox" aria-label="Matching repositories">
      {matches.map((repository, index) => <div key={repository.repository_id} id={`setup-repository-result-${index}`} data-setup-repository-result-index={index} role="option" aria-selected={index === activeIndex}
        className={`task-setup-result${index === activeIndex ? " is-active" : ""}`}
        onMouseMove={() => setActive(index)} onMouseDown={(event) => { event.preventDefault(); choose(repository); }}>
        <strong>{Array.from(repository.name, (character, characterIndex) => repository.matchedIndices.includes(characterIndex) ? <mark key={characterIndex}>{character}</mark> : character)}</strong>
        <code>{shortPath(repository.root)}</code>
      </div>)}
      {!loading && matches.length === 0 ? <p>No matching repositories.</p> : null}
    </div> : null}
  </div>;
}

function Row({ label, htmlFor, children }: { label: string; htmlFor?: string; children: ReactNode }) {
  return <div className="task-setup-row"><label htmlFor={htmlFor}>{label}</label><div>{children}</div></div>;
}

function SourceStatus({ state, linkedChoice, onLinked, disabled }: {
  state: SourceState;
  linkedChoice: Record<string, boolean>;
  onLinked: (url: string, include: boolean) => void;
  disabled: boolean;
}) {
  if (state.pending) return <p className="task-setup-note">Looking up the link…</p>;
  if (state.error) return <p className="task-setup-note is-error" role="alert">{state.error}</p>;
  const defaults = state.defaults;
  if (!defaults) return null;
  return <>
    <p className="task-setup-note is-valid">✓ {artifactLabel(defaults)}</p>
    {defaults.linked_artifacts.map((linked: LinkedArtifact) => linked.error
      ? <p key={linked.artifact.canonical_url} className="task-setup-note is-error">Linked {linked.artifact.canonical_id} could not be read: {linked.error}</p>
      : <label key={linked.artifact.canonical_url} className="task-setup-note task-setup-linked">
        <input type="checkbox" checked={linkedChoice[linked.artifact.canonical_url] !== false} disabled={disabled} onChange={(event) => onLinked(linked.artifact.canonical_url, event.target.checked)} />
        Also import {linked.artifact.canonical_id}{linked.title ? ` · ${linked.title}` : ""}
      </label>)}
  </>;
}

function PlanSummary({ plan }: { plan: WorkspaceSetupPlan }) {
  const sources = [plan.artifact?.canonical_id, ...plan.linked_artifacts.map((artifact) => artifact.canonical_id)].filter(Boolean);
  return <div className="task-setup-summary">
    {plan.mode === "create"
      ? <p>New worktree <code>{plan.branch}</code>{plan.base ? <> from <code>{plan.base.slice(0, 10)}</code></> : null} in <code title={plan.checkout_path}>{shortPath(plan.checkout_path)}</code></p>
      : <p>Open <code title={plan.checkout_path}>{shortPath(plan.checkout_path)}</code> as it is; Cockpit never deletes it</p>}
    {sources.length > 0 ? <p>Imports {sources.join(", ")} into Context</p> : null}
    {plan.warnings.map((warning) => <p key={warning} className="is-warning">{warning}</p>)}
  </div>;
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
  const recoveryLabel = recoveryAction === "accept_existing_worktree" ? "Recover existing checkout" : recoveryAction === "retry_environment" ? "Retry terminal" : null;
  const running = operation.state === "running" || operation.state === "planned";
  return <section className={`task-setup-progress${running ? " is-running" : ""}`} aria-live="polite" aria-busy={busy} aria-label="Space setup progress">
    <p role="status">{operationStatusMessage(operation)}</p>
    {readError ? <p className="task-setup-note">Could not read the latest status; showing the last confirmed step.</p> : null}
    {operation.error && !running ? <p className="task-setup-note is-error" role="alert">{operation.error.message}</p> : null}
    {failed && operation.owned_resources.length > 0 ? <details><summary>Retained resources</summary><ul>{operation.owned_resources.map((resource, index) => <li key={`${resource.kind}-${resource.path}-${index}`}><code>{resource.path}</code> · {resource.created_by_operation ? "created by this setup" : "existing"}</li>)}</ul></details> : null}
    <div className="task-setup-progress-actions">
      {running ? <button type="button" onClick={onCancel} disabled={busy}>Cancel</button> : null}
      {failed && operation.resume_allowed ? <button type="button" className="setup-primary" onClick={onResume} disabled={busy}>{sourceRetry(operation) ? "Retry source import" : "Resume failed step"}</button> : null}
      {recoveryAction && recoveryLabel ? <button type="button" className="setup-primary" onClick={() => onReview(recoveryAction)} disabled={busy}>{recoveryLabel}</button> : null}
    </div>
  </section>;
}

export function SetupDialog({ client, sessionId, open, selectedParent = null, parentSpaceId = null, onClose, onCompleted }: SetupDialogProps) {
  const titleId = useId();
  const dialogRef = useRef<HTMLElement>(null);
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
  const parentChoice = useRef<string | null>(null);
  const explicit = useRef<ExplicitFields>({ repository: false, branch: false, checkoutPath: false, label: false });
  const [form, setForm] = useState<FormState>(initialForm);
  const [repositories, setRepositories] = useState<RepositoryCandidate[]>([]);
  const [diagnostics, setDiagnostics] = useState<RepositoryListResponse["diagnostics"]>([]);
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [sourceState, setSourceState] = useState<SourceState>({ defaults: null, error: null, pending: false });
  const [linkedChoice, setLinkedChoice] = useState<Record<string, boolean>>({});
  const [planState, setPlanState] = useState<PlanState>({ key: null, plan: null, error: null, pending: false });
  const [planRevision, setPlanRevision] = useState(0);
  const [startRequested, setStartRequested] = useState(false);
  const [operation, setOperation] = useState<WorkspaceOperation | null>(null);
  const [operationReadError, setOperationReadError] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const [dispatched, setDispatched] = useState(false);
  const [requestRefresh, setRequestRefresh] = useState(0);
  const [lookupRevision, setLookupRevision] = useState(0);

  operationRef.current = operation;
  const onCompletedRef = useRef(onCompleted);
  onCompletedRef.current = onCompleted;
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  const linkedUrls = useMemo(() => (sourceState.defaults?.linked_artifacts ?? [])
    .filter((linked) => !linked.error && linkedChoice[linked.artifact.canonical_url] !== false)
    .map((linked) => linked.artifact.canonical_url), [linkedChoice, sourceState.defaults]);
  // The request the form currently describes, or null while it is incomplete.
  const requestKey = useMemo(() => {
    if (form.mode === "open") return form.openPath.trim() ? JSON.stringify(makeRequest(form)) : null;
    if (!form.repositoryId || sourceState.pending) return null;
    return JSON.stringify(makeRequest(form, linkedUrls));
  }, [form, linkedUrls, sourceState.pending]);
  const currentPlan = planState.key === requestKey ? planState.plan : null;

  const setDispatch = (value: boolean) => { dispatchRef.current = value; setDispatched(value); };

  const updateText = useCallback((key: TextField, value: string, manuallyEdited = false) => {
    if (dispatchRef.current) return;
    if (manuallyEdited) {
      if (key === "branch") explicit.current.branch = true;
      if (key === "checkoutPath") explicit.current.checkoutPath = true;
      if (key === "label") explicit.current.label = true;
    }
    if (key === "artifactUrl") {
      defaultsRequestToken.current += 1;
      setSourceState({ defaults: null, error: null, pending: false });
      setLinkedChoice({});
    }
    setForm((current) => {
      if (key !== "artifactUrl") return { ...current, [key]: value };
      return {
        ...current,
        artifactUrl: value,
        branch: explicit.current.branch ? current.branch : "",
        checkoutPath: explicit.current.checkoutPath ? current.checkoutPath : "",
        label: explicit.current.label ? current.label : "",
      };
    });
    setFormError(null);
  }, []);

  const chooseRepository = useCallback((repository: RepositoryCandidate) => {
    if (dispatchRef.current) return;
    explicit.current.repository = true;
    parentChoice.current = null;
    defaultsRequestToken.current += 1;
    setLookupRevision((current) => current + 1);
    setForm((current) => ({ ...current, repositoryId: repository.repository_id }));
    setFormError(null);
  }, []);

  const setMode = useCallback((mode: WorkspaceSetupMode) => {
    if (dispatchRef.current) return;
    setForm((current) => ({ ...current, mode }));
    setFormError(null);
    setStartRequested(false);
    window.setTimeout(() => dialogRef.current?.querySelector<HTMLElement>(mode === "open" ? "#setup-checkout" : "#setup-artifact-url")?.focus(), 0);
  }, []);

  const resetCompletedOperation = useCallback(() => {
    setDispatch(false);
    completedOperation.current = null;
    parentChoice.current = null;
    explicit.current = { repository: false, branch: false, checkoutPath: false, label: false };
    setForm(initialForm());
    setSourceState({ defaults: null, error: null, pending: false });
    setLinkedChoice({});
    setPlanState({ key: null, plan: null, error: null, pending: false });
    setStartRequested(false);
    setOperation(null);
    setOperationError(null);
    setOperationReadError(null);
  }, []);

  const handleClose = useCallback(() => {
    defaultsRequestToken.current += 1;
    pollToken.current += 1;
    setStartRequested(false);
    onCloseRef.current();
  }, []);

  useEffect(() => {
    const wasOpen = openRef.current;
    openRef.current = open;
    if (open && !wasOpen && operationRef.current && operationCanReset(operationRef.current.state)) resetCompletedOperation();
    if (!open) { pollToken.current += 1; defaultsRequestToken.current += 1; }
  }, [open, resetCompletedOperation]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    const sessionChanged = loadedSession.current !== null && loadedSession.current !== sessionId;
    loadedSession.current = sessionId;
    const token = ++loadRequestToken.current;
    setLoadState("loading");
    setLoadError(null);
    if (sessionChanged && !dispatchRef.current) resetCompletedOperation();
    // Changing folders emits no Herdr event, so the live snapshot can lag.
    const freshParent = parentSpaceId
      ? client.sessionSnapshot(sessionId).then(
        (snapshot) => setupParentFor(snapshot.spaces.find((space) => space.id === parentSpaceId), snapshot.panes, snapshot.focused_pane_id),
        () => null,
      )
      : Promise.resolve(null);
    void Promise.all([client.repositories(), freshParent]).then(([response, fresh]) => {
      if (!active || token !== loadRequestToken.current) return;
      const parent = fresh ?? selectedParent;
      setRepositories(response.repositories);
      setDiagnostics(response.diagnostics);
      setLoadState(response.repositories.length === 0 ? "empty" : "ready");
      const parentRepository = parent ? resolveParentRepository(response.repositories, parent) : null;
      if (!dispatchRef.current && parentRepository) {
        // Follow the current Space while the repository is still our own
        // earlier guess; never replace a choice or a link's repository.
        setForm((current) => {
          if (current.repositoryId && current.repositoryId !== parentChoice.current) return current;
          parentChoice.current = parentRepository.repository_id;
          return { ...current, repositoryId: parentRepository.repository_id };
        });
      }
    }).catch((error: unknown) => {
      if (!active || token !== loadRequestToken.current) return;
      setLoadState("error");
      setLoadError(errorMessage(error, "Could not load repositories."));
    });
    return () => { active = false; };
  }, [client, open, parentSpaceId, requestRefresh, resetCompletedOperation, selectedParent?.checkoutPath, selectedParent?.repositoryKey, sessionId]);

  // Resolve a pasted link into repository, branch and name defaults.
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
        if (!explicit.current.repository && defaults.repository_id) parentChoice.current = null;
        setForm((current) => ({
          ...current,
          repositoryId: explicit.current.repository ? current.repositoryId : defaults.repository_id ?? current.repositoryId,
          branch: explicit.current.branch ? current.branch : defaults.branch ?? current.branch,
          checkoutPath: explicit.current.checkoutPath ? current.checkoutPath : defaults.checkout_path ?? current.checkoutPath,
          label: explicit.current.label ? current.label : defaults.label ?? current.label,
        }));
      }).catch((error: unknown) => {
        if (token !== defaultsRequestToken.current || dispatchRef.current) return;
        setSourceState({ defaults: null, error: errorMessage(error, "Could not read this link. Choose the repository and branch yourself."), pending: false });
      });
    }, SOURCE_DELAY_MS);
    return () => window.clearTimeout(timeout);
  }, [client, form.artifactUrl, form.mode, lookupRevision, open]);

  // Prepare the authoritative plan for the current form as the user types.
  useEffect(() => {
    if (!open || dispatchRef.current || operationRef.current) return;
    const token = ++planRequestToken.current;
    if (!requestKey) {
      setPlanState({ key: null, plan: null, error: null, pending: false });
      return;
    }
    setPlanState((current) => ({ key: requestKey, plan: null, error: current.key === requestKey ? current.error : null, pending: true }));
    const request = JSON.parse(requestKey) as WorkspaceSetupRequest;
    const timeout = window.setTimeout(() => {
      void client.planWorkspace(sessionId, request).then((plan) => {
        if (token !== planRequestToken.current || dispatchRef.current) return;
        const requestArtifactUrl = request.operation === "create" ? request.artifact_url : null;
        if (plan.session_id !== sessionId
          || plan.mode !== (request.operation === "create" ? "create" : "open")
          || (request.operation === "create" && plan.repository?.repository_id !== request.repository_id)
          || (request.operation === "open" && plan.checkout_path !== request.path)
          || (plan.artifact?.original_url ?? null) !== requestArtifactUrl) {
          setPlanState({ key: requestKey, plan: null, error: "The prepared setup did not match the form. Check the fields.", pending: false });
          return;
        }
        setPlanState({ key: requestKey, plan, error: null, pending: false });
      }).catch((error: unknown) => {
        if (token !== planRequestToken.current || dispatchRef.current) return;
        setPlanState({ key: requestKey, plan: null, error: errorMessage(error, "Could not prepare this setup."), pending: false });
        setStartRequested(false);
      });
    }, PLAN_DELAY_MS);
    return () => window.clearTimeout(timeout);
  }, [client, open, planRevision, requestKey, sessionId]);

  useEffect(() => {
    if (!open) return;
    const focusFirst = () => {
      if (dialogRef.current?.contains(document.activeElement)) return;
      dialogRef.current?.querySelector<HTMLElement>("#setup-artifact-url, #setup-checkout")?.focus();
    };
    focusFirst();
    // A closing sidebar drawer can restore its own focus after this render.
    const frame = window.requestAnimationFrame(focusFirst);
    // Escape still closes setup when focus is outside the dialog.
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || dialogRef.current?.contains(event.target as Node)) return;
      event.preventDefault();
      handleClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => { window.cancelAnimationFrame(frame); window.removeEventListener("keydown", onKeyDown); };
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
      // The new Space is focused by Herdr; nothing is left to decide here.
      pollToken.current += 1;
      onCloseRef.current();
    }
  }, []);

  const approve = useCallback(async (plan: WorkspaceSetupPlan) => {
    if (actionPending || dispatchRef.current) return;
    setStartRequested(false);
    setDispatch(true);
    setActionPending(true);
    setOperationError(null);
    try {
      const next = await client.startWorkspace(sessionId, { operation_id: plan.operation_id, expected_generation: plan.generation });
      if (next.operation_id !== plan.operation_id || next.generation < plan.generation) {
        setOperationError("The setup response did not match the prepared setup.");
        return;
      }
      acceptOperation(next);
    } catch (error: unknown) {
      if (requiresPlanReview(error)) {
        setDispatch(false);
        setOperationError(`${errorMessage(error, "The setup changed before it could start.")} Check the updated setup and press Create again.`);
        setPlanRevision((value) => value + 1);
        return;
      }
      setOperationError(errorMessage(error, "The start request outcome is unknown. Checking its status."));
      try {
        const next = await client.workspaceOperation(sessionId, plan.operation_id);
        if (next.operation_id !== plan.operation_id || next.generation < plan.generation) {
          setOperationError("The start request outcome is unknown; the returned operation did not match.");
          return;
        }
        acceptOperation(next);
      } catch (inspectionError: unknown) {
        setOperationError(`${errorMessage(error, "The start request outcome is unknown.")} ${errorMessage(inspectionError, "Use Check setup before trying again.")}`);
      }
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, client, sessionId]);

  const inspectReceipt = useCallback(async () => {
    const plan = planState.plan;
    if (!plan || actionPending) return;
    setActionPending(true);
    setOperationError(null);
    try {
      const next = await client.workspaceOperation(plan.session_id, plan.operation_id);
      if (next.operation_id !== plan.operation_id || next.generation < plan.generation) {
        setOperationError("The setup status did not match the started setup.");
        return;
      }
      acceptOperation(next);
      setOperationReadError(null);
    } catch (error: unknown) {
      setOperationError(errorMessage(error, "Could not read the started setup. Its outcome is still unknown."));
    } finally {
      setActionPending(false);
    }
  }, [acceptOperation, actionPending, client, planState.plan]);

  // Create waits for the plan of the current form, then starts exactly it.
  useEffect(() => {
    if (!startRequested || !currentPlan || planState.pending) return;
    void approve(currentPlan);
  }, [approve, currentPlan, planState.pending, startRequested]);

  const create = useCallback(() => {
    if (actionPending) return;
    if (dispatchRef.current) { void inspectReceipt(); return; }
    if (form.mode === "open" && !form.openPath.trim()) {
      setFormError("Enter the folder to open.");
      dialogRef.current?.querySelector<HTMLElement>("#setup-checkout")?.focus();
      return;
    }
    if (form.mode === "create" && !form.repositoryId) {
      setFormError(sourceState.pending ? null : "Choose a repository.");
      if (sourceState.pending) { setStartRequested(true); return; }
      dialogRef.current?.querySelector<HTMLElement>("#setup-repository")?.focus();
      return;
    }
    if (planState.error && planState.key === requestKey) {
      // Retry a failed preparation instead of starting nothing.
      setPlanRevision((value) => value + 1);
    }
    setOperationError(null);
    setStartRequested(true);
  }, [actionPending, form.mode, form.openPath, form.repositoryId, inspectReceipt, planState.error, planState.key, requestKey, sourceState.pending]);

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
        setOperationReadError(errorMessage(error, "Setup status unavailable."));
      }
      if (pollToken.current !== token || !last || operationIsTerminal(last.state)) return;
      await new Promise<void>((resolve) => window.setTimeout(resolve, POLL_INTERVAL_MS));
    }
  }, [acceptOperation, client]);

  useEffect(() => {
    if (!open || !operation || operationIsTerminal(operation.state)) return;
    void pollOperation(operation.operation_id, operation.generation);
    return () => { pollToken.current += 1; };
  }, [open, operation?.operation_id, operation?.generation, operation?.state, pollOperation]);

  const runAction = useCallback(async (action: () => Promise<WorkspaceOperation>, fallback: string) => {
    if (actionPending) return;
    setActionPending(true);
    try { acceptOperation(await action()); }
    catch (error: unknown) { setOperationError(errorMessage(error, fallback)); }
    finally { setActionPending(false); }
  }, [acceptOperation, actionPending]);
  const cancel = () => operation && void runAction(() => client.cancelWorkspace(operation.session_id, { operation_id: operation.operation_id, expected_generation: operation.generation }), "Could not cancel the setup.");
  const resume = () => operation?.resume_allowed && void runAction(() => client.resumeWorkspace(operation.session_id, { operation_id: operation.operation_id, expected_generation: operation.generation }), "Could not resume the setup.");
  const reconcile = (action: WorkspaceRecoveryAction) => operation && recoveryActionFor(operation) === action && void runAction(() => client.reconcileWorkspace(operation.session_id, { operation_id: operation.operation_id, expected_generation: operation.generation, action }), "Could not recover the setup.");

  const onDialogKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    // The repository list stops its own Enter and Escape from reaching here.
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      handleClose();
      return;
    }
    const target = event.target as HTMLElement;
    if (event.key === "Enter" && target instanceof HTMLInputElement && target.type !== "checkbox") {
      event.preventDefault();
      create();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled]), summary")];
    if (focusable.length === 0) return;
    const current = focusable.indexOf(document.activeElement as HTMLElement);
    if (event.shiftKey && current <= 0) { event.preventDefault(); focusable.at(-1)?.focus(); }
    else if (!event.shiftKey && current === focusable.length - 1) { event.preventDefault(); focusable[0]?.focus(); }
  };

  const onText = (key: TextField, manuallyEdited = false) => (event: ChangeEvent<HTMLInputElement>) => updateText(key, event.currentTarget.value, manuallyEdited);
  const sourceRepositories = sourceState.defaults?.repositories.length ? sourceState.defaults.repositories : repositories;
  const selectedRepository = repositories.find((repository) => repository.repository_id === form.repositoryId)
    ?? sourceRepositories.find((repository) => repository.repository_id === form.repositoryId) ?? null;
  const locked = dispatched;
  const working = startRequested || actionPending;
  const primaryLabel = dispatched && !operation ? (actionPending ? "Checking…" : "Check setup")
    : working ? (form.mode === "open" ? "Opening…" : "Creating…")
      : form.mode === "open" ? "Open Space" : "Create Space";
  const catalogNote = diagnostics[0]?.message;

  if (!open) return null;
  return <div className="setup-overlay" role="presentation">
    <section className="setup-dialog task-setup" ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={onDialogKeyDown}>
      <header className="task-setup-header"><h2 id={titleId}>{form.mode === "open" ? "Open a folder as a Space" : "New Space"}</h2><button type="button" className="task-setup-close" onClick={handleClose} aria-label="Close setup dialog"><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        {form.mode === "create" ? <>
          <Row label="Link" htmlFor="setup-artifact-url">
            <input id="setup-artifact-url" type="url" value={form.artifactUrl} onChange={onText("artifactUrl")} placeholder="GitLab MR, issue or Jira link (optional)" autoComplete="off" spellCheck={false} disabled={locked} />
            <SourceStatus state={sourceState} linkedChoice={linkedChoice} disabled={locked} onLinked={(url, include) => setLinkedChoice((current) => ({ ...current, [url]: include }))} />
          </Row>
          <Row label="Repository" htmlFor="setup-repository">
            {loadState === "error"
              ? <p className="task-setup-note is-error">{loadError} <button type="button" className="task-setup-link" onClick={() => setRequestRefresh((value) => value + 1)}>Reload</button></p>
              : <RepositoryPicker repositories={sourceRepositories} selected={selectedRepository} loading={loadState === "loading"} disabled={locked} onChoose={chooseRepository} />}
            {catalogNote ? <p className="task-setup-note">{catalogNote}</p> : null}
          </Row>
          <Row label="Branch" htmlFor="setup-branch"><input id="setup-branch" value={form.branch} onChange={onText("branch", true)} placeholder={currentPlan?.branch ?? "From the link or branch pattern"} autoComplete="off" spellCheck={false} disabled={locked} /></Row>
        </> : <Row label="Folder" htmlFor="setup-checkout">
          <input id="setup-checkout" value={form.openPath} onChange={onText("openPath", true)} placeholder="/path/to/folder" aria-invalid={Boolean(formError)} autoComplete="off" spellCheck={false} disabled={locked} />
        </Row>}
        <Row label="Name" htmlFor="setup-label"><input id="setup-label" value={form.label} onChange={onText("label", true)} placeholder={currentPlan?.label ?? (form.mode === "open" ? directoryName(form.openPath) || "Folder name" : form.branch || "Branch name")} autoComplete="off" spellCheck={false} disabled={locked} /></Row>
        <details className="task-setup-more">
          <summary>More</summary>
          {form.mode === "create" ? <>
            <Row label="Base" htmlFor="setup-base"><input id="setup-base" value={form.base} onChange={onText("base")} placeholder="Repository default" autoComplete="off" spellCheck={false} disabled={locked} /></Row>
            <Row label="Destination" htmlFor="setup-destination"><input id="setup-destination" value={form.checkoutPath} onChange={onText("checkoutPath", true)} placeholder="Automatic" autoComplete="off" spellCheck={false} disabled={locked} /></Row>
          </> : null}
          <label className="task-setup-check"><input type="checkbox" checked={form.focus} onChange={(event) => { if (!dispatchRef.current) setForm((current) => ({ ...current, focus: event.target.checked })); }} disabled={locked} /> Switch to the new Space</label>
          <button type="button" className="task-setup-link" onClick={() => setMode(form.mode === "open" ? "create" : "open")} disabled={locked}>{form.mode === "open" ? "Create a new worktree instead" : "Open an existing folder instead"}</button>
        </details>
        {operation ? <Progress operation={operation} readError={operationReadError} busy={actionPending} onCancel={cancel} onResume={resume} onReview={reconcile} />
          : <div className="task-setup-status" aria-live="polite">
            {formError ? <p className="task-setup-note is-error" role="alert">{formError}</p> : null}
            {planState.key === requestKey && planState.error ? <p className="task-setup-note is-error" role="alert">{planState.error}</p> : null}
            {currentPlan ? <PlanSummary plan={currentPlan} /> : planState.pending ? <p className="task-setup-note">Preparing…</p> : null}
          </div>}
        {operationError ? <p className="task-setup-note is-error" role="alert">{operationError}</p> : null}
      </div>
      {!operation || !operationIsTerminal(operation.state) || operation.state === "cancelled" ? <footer className="task-setup-footer">
        <button type="button" onClick={handleClose}>{operation ? "Close" : "Cancel"}</button>
        {!operation ? <button type="button" className="setup-primary" onClick={create} disabled={actionPending || (loadState === "loading" && form.mode === "create" && !form.repositoryId)}>{primaryLabel}</button> : null}
      </footer> : <footer className="task-setup-footer"><button type="button" onClick={handleClose}>Close</button></footer>}
    </section>
  </div>;
}
