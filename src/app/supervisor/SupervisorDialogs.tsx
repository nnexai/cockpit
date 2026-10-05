import { useCallback, useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { CockpitClient } from "../../client/CockpitClient";
import type { DispatchTarget, OrchestrationAction, OrchestrationSnapshot, RepositoryListResponse, SpaceSummary, TaskView } from "../../protocol/generated/v1";
import { useRestoreFocus } from "../library/LibraryConfirmDialog";
import { UiIcon } from "../UiIcon";
import "../projects/setup.css";
import "../projects/taskSetup.css";

export type SupervisorDialogsProps = {
  mode: "start" | "create" | "edit" | "propose";
  snapshot: OrchestrationSnapshot;
  task: TaskView | null;
  client: Pick<CockpitClient, "repositories">;
  spaces: SpaceSummary[];
  busy: boolean;
  error: string | null;
  mutate: (action: OrchestrationAction) => Promise<boolean>;
  onClose: () => void;
};

type TargetChoice = "isolated" | "worktree" | "directory" | "existingSpace";

function SupervisorDialogForm({ mode, snapshot, task, client, spaces, busy, error, mutate, onClose }: SupervisorDialogsProps) {
  const titleId = useId();
  const formId = useId();
  const dialogRef = useRef<HTMLElement>(null);
  const firstRef = useRef<HTMLInputElement>(null);
  const inFlight = useRef(false);
  const [opened] = useState(() => ({
    task: task?.task ?? null,
    rootId: snapshot.board?.root_id ?? snapshot.roots[0]?.root_id ?? "",
    currentRunId: task?.current_run_id ?? null,
  }));
  const [rootId, setRootId] = useState(opened.rootId);
  const [title, setTitle] = useState(mode === "edit" ? opened.task?.title ?? "" : "");
  const [body, setBody] = useState(mode === "edit" ? opened.task?.body ?? "" : "");
  const [label, setLabel] = useState("");
  const [targetChoice, setTargetChoice] = useState<TargetChoice>(mode === "start" ? "isolated" : "worktree");
  const [repositoryId, setRepositoryId] = useState("");
  const [branch, setBranch] = useState("");
  const [base, setBase] = useState("");
  const [checkout, setCheckout] = useState("");
  const [directory, setDirectory] = useState("");
  const [spaceId, setSpaceId] = useState("");
  const [parentRunId, setParentRunId] = useState(opened.rootId);
  const [brief, setBrief] = useState("");
  const [supersede, setSupersede] = useState(false);
  const [catalog, setCatalog] = useState<RepositoryListResponse | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogPending, setCatalogPending] = useState(false);
  const [catalogRefresh, setCatalogRefresh] = useState(0);
  const [localError, setLocalError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const locked = busy || pending;
  useRestoreFocus();
  useEffect(() => { firstRef.current?.focus({ preventScroll: true }); }, []);
  useEffect(() => {
    if (mode !== "propose") return;
    let active = true;
    setCatalogPending(true);
    setCatalogError(null);
    void client.repositories().then((result) => {
      if (active) setCatalog(result);
    }).catch((cause: unknown) => {
      if (active) setCatalogError(cause instanceof Error ? cause.message : "Could not load configured repositories.");
    }).finally(() => { if (active) setCatalogPending(false); });
    return () => { active = false; };
  }, [client, mode, catalogRefresh]);
  const liveTask = opened.task ? snapshot.board?.tasks.find((item) => item.task.task_id === opened.task?.task_id) : null;
  const taskChanged = !!liveTask && liveTask.task.task_revision !== opened.task?.task_revision;
  const runChanged = mode === "propose" && !!liveTask && liveTask.current_run_id !== opened.currentRunId;
  const currentRun = snapshot.runs.find((run) => run.run_id === opened.currentRunId && run.stage !== "closed");
  const parentChoices = snapshot.runs.filter((run) => run.root_id === opened.rootId && run.stage !== "closed");
  const chosenParent = parentChoices.find((run) => run.run_id === parentRunId);
  const selectedRepository = catalog?.repositories.find((repository) => repository.repository_id === repositoryId);
  const close = useCallback(() => { if (!busy && !inFlight.current) onClose(); }, [busy, onClose]);
  const trapKeys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (event.key !== "Tab") return;
    event.stopPropagation();
    const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>("button, input, textarea, select, a[href], summary, [tabindex]:not([tabindex='-1'])")]
      .filter((element) => !element.matches(":disabled, [hidden], [aria-hidden='true']") && !element.closest("[hidden], [inert]") && element.getClientRects().length > 0);
    if (!focusable.length) { event.preventDefault(); dialogRef.current?.focus(); return; }
    const index = focusable.indexOf(document.activeElement as HTMLElement);
    if (event.shiftKey && index <= 0) { event.preventDefault(); focusable.at(-1)?.focus(); }
    else if (!event.shiftKey && (index < 0 || index === focusable.length - 1)) { event.preventDefault(); focusable[0]?.focus(); }
  };
  const submit = async () => {
    if (locked || inFlight.current) return;
    setLocalError(null);
    let action: OrchestrationAction;
    if (mode === "create" || mode === "edit") {
      if (!rootId || !snapshot.roots.some((root) => root.root_id === rootId)) { setLocalError("Choose an available supervisor root for this task."); return; }
      if (!title.trim()) { setLocalError("A task title is required."); return; }
      if (mode === "edit") {
        if (!opened.task) { setLocalError("No task was selected for editing."); return; }
        action = { action: "task_update", root_id: opened.rootId, task_id: opened.task.task_id, expected_task_revision: opened.task.task_revision, title: title.trim(), body };
      } else action = { action: "task_create", root_id: rootId, title: title.trim(), body };
    } else {
      let target: DispatchTarget | null = null;
      if (targetChoice === "existingSpace") {
        if (!spaces.some((space) => space.id === spaceId)) { setLocalError("Choose an available existing Space."); return; }
        target = { target: "existing_space", workspace_id: spaceId };
      } else if (targetChoice === "directory") {
        if (!directory.trim().startsWith("/")) { setLocalError("Enter an explicit absolute directory path."); return; }
        target = { target: "setup", request: { operation: "open", path: directory.trim(), label: label.trim() || null, task_name: opened.task?.title ?? null, focus: false } };
      } else if (targetChoice === "worktree") {
        if (!selectedRepository) { setLocalError("Choose a configured repository for the new worktree."); return; }
        target = { target: "setup", request: { operation: "create", repository_id: selectedRepository.repository_id, branch: branch.trim() || null, base_ref: base.trim() || null, checkout_path: checkout.trim() || null, label: label.trim() || null, task_name: opened.task?.title ?? null, artifact_url: null, linked_artifact_urls: [], focus: false } };
      }
      if (mode === "start") action = { action: "supervisor_start", target, label: label.trim() || null };
      else {
        if (!opened.task || !target) { setLocalError("A selected task and a worker target are required."); return; }
        if (runChanged) { setLocalError("The task's current run changed after this dialog opened. Close and reopen the proposal to review the new run before superseding it."); return; }
        if (!chosenParent) { setLocalError("Choose an active parent run under this task's supervisor root."); return; }
        if (!brief.trim()) { setLocalError("Provide the bounded preparation and read-only initialization brief."); return; }
        if (currentRun && !supersede) { setLocalError("This task has an open run. Explicitly authorize superseding that run, or cancel this proposal."); return; }
        action = { action: "run_propose", task_id: opened.task.task_id, parent_run_id: chosenParent.run_id, label: label.trim() || null, target, prepare_brief: brief, supersedes_run_id: supersede && currentRun ? currentRun.run_id : null };
      }
    }
    inFlight.current = true;
    setPending(true);
    try { if (await mutate(action)) onClose(); }
    catch (cause) { setLocalError(cause instanceof Error ? cause.message : "Could not save the requested change. Your draft is preserved."); }
    finally { inFlight.current = false; setPending(false); }
  };
  const dialogTitle = mode === "start" ? "Start supervisor" : mode === "create" ? "Create Markdown task" : mode === "edit" ? "Edit Markdown task" : "Propose worker run";
  const submitTitle = mode === "start" ? "Start supervisor" : mode === "create" ? "Create task" : mode === "edit" ? "Save task" : "Propose run";
  return createPortal(<div className="setup-overlay" role="presentation">
    <section ref={dialogRef} tabIndex={-1} className="setup-dialog task-setup supervisor-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-busy={locked} onKeyDown={trapKeys}>
      <header className="task-setup-header"><h2 id={titleId}>{dialogTitle}</h2><button type="button" className="task-setup-close" aria-label={`Close ${dialogTitle}`} disabled={locked} onClick={close}><UiIcon name="close" /></button></header>
      <form id={formId} className="task-setup-body" onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <fieldset disabled={locked}>
          {mode === "create" || mode === "edit" ? <>
            {mode === "create" ? <label>Supervisor root<select value={rootId} onChange={(event) => setRootId(event.target.value)}><option value="">Choose a supervisor</option>{snapshot.roots.map((root) => <option key={root.root_id} value={root.root_id}>{root.label}</option>)}</select></label> : <p>Revision at open: <code>{opened.task?.task_revision ?? "No selected task"}</code></p>}
            <label>Task title<input ref={firstRef} value={title} onChange={(event) => setTitle(event.target.value)} autoComplete="off" /></label>
            <label>Markdown body<textarea rows={10} value={body} onChange={(event) => setBody(event.target.value)} /></label>
            <p>The canonical task is Markdown. Body formatting is saved as entered; no Space-local copy is created.</p>
            {mode === "edit" && taskChanged ? <p className="supervisor-error" role="alert">The canonical task changed after this dialog opened. Saving uses the original revision and will reject a conflicting edit; your draft will remain intact.</p> : null}
          </> : <>
            {mode === "propose" ? <p>Task: {opened.task?.title ?? "No selected task"}</p> : null}
            <label>{mode === "start" ? "Supervisor name (optional)" : "Worker name (optional)"}<input ref={firstRef} value={label} onChange={(event) => setLabel(event.target.value)} autoComplete="off" /></label>
            {mode === "propose" ? <label>Parent run<select value={parentRunId} onChange={(event) => setParentRunId(event.target.value)}><option value="">Choose an active parent</option>{parentChoices.map((run) => <option key={run.run_id} value={run.run_id}>{run.label} · {run.kind} · {run.stage}</option>)}</select></label> : null}
            <label>Target<select value={targetChoice} onChange={(event) => setTargetChoice(event.target.value as TargetChoice)}>
              {mode === "start" ? <option value="isolated">None · default isolated supervisor directory</option> : <option value="worktree">New repository worktree</option>}
              <option value="directory">Open an explicit directory</option><option value="existingSpace">Existing Space</option>
            </select></label>
            {targetChoice === "isolated" ? <p>Cockpit opens an isolated directory under its orchestration state root. It does not infer a target from the current Space or working directory.</p> : null}
            {targetChoice === "worktree" ? <>
              <label>Repository<select value={repositoryId} disabled={catalogPending || locked} onChange={(event) => setRepositoryId(event.target.value)}><option value="">{catalogPending ? "Loading configured repositories…" : "Choose a repository"}</option>{catalog?.repositories.map((repository) => <option key={repository.repository_id} value={repository.repository_id}>{repository.name} · {repository.checkout_path}</option>)}</select></label>
              {catalogError ? <p className="supervisor-error" role="alert">{catalogError} <button type="button" onClick={() => setCatalogRefresh((value) => value + 1)}>Reload repositories</button></p> : null}
              {catalog && !catalog.repositories.length ? <p>No configured repositories are available. Choose an explicit directory or existing Space instead.</p> : null}
              {catalog?.diagnostics.map((diagnostic, index) => <p key={index}>{diagnostic.message}{diagnostic.path ? ` · ${diagnostic.path}` : ""}</p>)}
              <label>Branch (optional)<input value={branch} onChange={(event) => setBranch(event.target.value)} autoComplete="off" spellCheck={false} /></label>
              <label>Base reference (optional)<input value={base} onChange={(event) => setBase(event.target.value)} autoComplete="off" spellCheck={false} /></label>
              <label>Checkout destination (optional)<input value={checkout} onChange={(event) => setCheckout(event.target.value)} autoComplete="off" spellCheck={false} /></label>
              <p>Blank fields use repository setup defaults. The exact setup plan must be reviewed and granted separately; proposing does not create the worktree or launch the worker.</p>
            </> : null}
            {targetChoice === "directory" ? <label>Absolute directory<input value={directory} onChange={(event) => setDirectory(event.target.value)} autoComplete="off" spellCheck={false} /></label> : null}
            {targetChoice === "existingSpace" ? <label>Space<select value={spaceId} onChange={(event) => setSpaceId(event.target.value)}><option value="">Choose an existing Space</option>{spaces.map((space) => <option key={space.id} value={space.id}>{space.label}</option>)}</select></label> : null}
            {mode === "propose" ? <>
              <label>Preparation and read-only initialization brief<textarea rows={8} value={brief} onChange={(event) => setBrief(event.target.value)} /></label>
              <p>The brief is delivered through the worker inbox. Prepare and Execute need separate operator grants bound to exact plans. Read-only is policy, not an OS sandbox.</p>
              {currentRun ? <label><input type="checkbox" checked={supersede} onChange={(event) => setSupersede(event.target.checked)} /> Explicitly supersede {currentRun.label} (attempt {currentRun.attempt}, {currentRun.stage}) when the new run receives Prepare authorization. Its Space and checkout are not torn down.</label> : <p>No open current run will be superseded.</p>}
              {runChanged ? <p className="supervisor-error" role="alert">The task's current run changed. Close and reopen this proposal to review the new run; no stale supersede action will be sent.</p> : null}
            </> : <p>The supervisor opens without switching your current terminal focus.</p>}
          </>}
        </fieldset>
        {localError || error ? <p className="supervisor-error" role="alert">{localError ?? error}</p> : null}
      </form>
      <footer className="task-setup-footer"><button type="button" disabled={locked} onClick={close}>Cancel</button><button type="submit" className="setup-primary" form={formId} disabled={locked || runChanged}>{pending ? "Saving…" : submitTitle}</button></footer>
    </section>
  </div>, document.body);
}

export function SupervisorDialogs(props: SupervisorDialogsProps) {
  return <SupervisorDialogForm key={`${props.mode}:${props.task?.task.task_id ?? "none"}`} {...props} />;
}
