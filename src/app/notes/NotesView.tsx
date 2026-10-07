import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { NotesCatalogEntry, NotesTargetInfo, NotesTodo } from "../../protocol/generated/v1";
import type { LibrarySpace } from "../library/libraryState";
import { UiIcon } from "../UiIcon";
import { MarkdownEditor, MarkdownPreview, NotesEditorScope } from "./MarkdownEditor";
import { acknowledgeDraft, changedDraft } from "./drafts"
import { todoIdentity, useNotes, notesError, todoSelector } from "./useNotes"
import { TodoTitle } from "./TodoTitle";
import { Kanban } from "./Kanban";
import { TaskDetail } from "./TaskDetail";
import { Decisions } from "./Decisions";
import { AgentAccess } from "./AgentAccess";
import { RetainedDrafts } from "./RetainedDrafts";
import "./notes.css";

type ViewTab = "scratchpad" | "todos" | "kanban" | "decisions";
type ViewState = { tab: ViewTab; selectedTodo: string | null; selectedDecision: string | null; hideCompleted: boolean; detailOpen: boolean };
type NotesProblem = { code: string; message: string };
type CatalogState = { status: "idle" | "loading" } | { status: "ready"; entries: NotesCatalogEntry[] } | { status: "error"; problem: NotesProblem };
const tabs: ViewTab[] = ["scratchpad", "todos", "kanban", "decisions"];
const tabLabels: Record<ViewTab, string> = { scratchpad: "Scratchpad", todos: "Todos", kanban: "Kanban", decisions: "Decisions" };
const surfaceStates = new Map<string, ViewState>();
function readSurface(notesId: string): ViewState {
  const memory = surfaceStates.get(notesId);
  if (memory) return memory;
  const fallback: ViewState = { tab: "kanban", selectedTodo: null, selectedDecision: null, hideCompleted: false, detailOpen: false };
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(`cockpit.notes.surface.v1:${notesId}`) ?? "null");
    if (!raw || typeof raw !== "object" || !("tab" in raw) || !("selectedTodo" in raw) || !("selectedDecision" in raw) || !("hideCompleted" in raw)) return fallback;
    if ((raw.tab === "scratchpad" || raw.tab === "todos" || raw.tab === "kanban" || raw.tab === "decisions") && (raw.selectedTodo === null || typeof raw.selectedTodo === "string") && (raw.selectedDecision === null || typeof raw.selectedDecision === "string") && typeof raw.hideCompleted === "boolean") return { tab: raw.tab, selectedTodo: raw.selectedTodo, selectedDecision: raw.selectedDecision, hideCompleted: raw.hideCompleted, detailOpen: "detailOpen" in raw && raw.detailOpen === true };
  } catch { /* Session state remains available when persistent browser storage is disabled. */ }
  return fallback;
}
function NotesContent({ client, info }: { client: CockpitClient; info: NotesTargetInfo }) {
  const model = useNotes(client, info);
  const [surface, setSurface] = useState(() => readSurface(info.notes_id));
  const [counts, setCounts] = useState<Record<string, number>>({});
  const [showDetail, setShowDetail] = useState(surface.detailOpen);
  const detailInvoker = useRef<HTMLElement | null>(null);
  const [savingMine, setSavingMine] = useState(false);
  const contentMounted = useRef(false);
  const detailVisible = useRef(showDetail); detailVisible.current = showDetail;
  const [checkedUnknown, setCheckedUnknown] = useState<string | null>(null);
  const [checkingSaved, setCheckingSaved] = useState(false);
  const checkEpoch = useRef(0);
  useLayoutEffect(() => { contentMounted.current = true; return () => { contentMounted.current = false; }; }, []);
  useEffect(() => {
    surfaceStates.set(info.notes_id, surface);
    try { localStorage.setItem(`cockpit.notes.surface.v1:${info.notes_id}`, JSON.stringify(surface)); } catch { /* Draft storage has a separate visible warning. */ }
  }, [surface, info.notes_id]);
  useEffect(() => { setSurface(current => current.detailOpen === showDetail ? current : { ...current, detailOpen: showDetail }); }, [showDetail]);
  const unknown = model.error?.code === "notes_outcome_unknown";
  const countedIds = Object.keys(counts).sort().join(",");
  useEffect(() => {
    let active = true;
    const refreshCounts = async () => {
      for (const id of countedIds ? countedIds.split(",") : []) {
        if (showDetail && id === surface.selectedTodo) continue;
        try {
          const response = await model.request({ op: "comment_list", todo_id: id });
          if (!active) return;
          if (response.result.kind === "comments") { const count = response.result.comments.length; setCounts(current => current[id] === count ? current : { ...current, [id]: count }); }
        } catch { /* Do not invent a zero count after a failed read. */ }
      }
    };
    void refreshCounts();
    return () => { active = false; };
  }, [model.request, info.change_tokens.comments, countedIds, showDetail, surface.selectedTodo]);
  const checkSavedState = async () => {
    const problem = model.error;
    const epoch = ++checkEpoch.current;
    const incident = model.drafts.unknownOutcome ?? problem?.message ?? null;
    setCheckedUnknown(null); setCheckingSaved(true);
    try {
      await model.refresh();
      if (surface.selectedTodo) await model.request({ op: "comment_list", todo_id: surface.selectedTodo });
      if (!contentMounted.current || epoch !== checkEpoch.current) return;
      if (problem?.code === "notes_outcome_unknown") setCheckedUnknown(incident);
      else model.setError(null);
    } catch (failure) {
      const issue = notesError(failure);
      if (contentMounted.current && epoch === checkEpoch.current) model.setError(problem?.code === "notes_outcome_unknown" ? { code: "notes_outcome_unknown", message: `Still unable to check saved state: ${issue.message}` } : issue);
    } finally { if (contentMounted.current && epoch === checkEpoch.current) setCheckingSaved(false); }
  };
  const reviewedUnknown = unknown && checkedUnknown !== null && checkedUnknown === (model.drafts.unknownOutcome ?? model.error?.message);
  const acknowledgeUnknown = () => {
    if (!reviewedUnknown || checkingSaved || model.busy) return;
    ++checkEpoch.current; setCheckedUnknown(null); model.setError(null);
  };
  const selectedTodo = model.todos.find(todo => todo.id === surface.selectedTodo) ?? null;
  const openDetails = async (todo: NotesTodo, opener: HTMLElement) => {
    detailInvoker.current = opener;
    if (!todo.id || todo.problems.includes("duplicate_id")) {
      await model.mutate({ op: "todo_update", todo: { by: "ref", ref: todo.ref }, text: null }, response => {
        if (response.result.kind === "todo" && response.result.todo.id) { setSurface(current => ({ ...current, selectedTodo: response.result.kind === "todo" ? response.result.todo.id : null })); setShowDetail(true); }
      });
      return;
    }
    setSurface(current => ({ ...current, selectedTodo: todo.id })); setShowDetail(true);
  };
  const closeDetails = () => {
    const closingDetail = document.querySelector(".notes-task-detail");
    setShowDetail(false);
    requestAnimationFrame(() => {
      if (!contentMounted.current || detailVisible.current) return;
      const focused = document.activeElement;
      if (focused !== document.body && !closingDetail?.contains(focused)) return;
      if (detailInvoker.current?.isConnected) detailInvoker.current.focus({ preventScroll: true });
      else document.getElementById(surface.tab === "todos" ? "todoAddInput" : "kanbanAddInput")?.focus({ preventScroll: true });
    });
  };
  const saveScratchpad = async (keepMine = false) => {
    const draft = model.drafts.scratchpad;
    if (!draft || !model.scratchpad || unknown || model.busy || draft.value === draft.base || (!keepMine && draft.conflict)) return;
    const submitted = draft.value;
    if (keepMine) setSavingMine(true);
    try {
      await model.mutate({ op: "scratchpad_replace", content: submitted, expected_revision: keepMine ? model.scratchpad.revision : draft.revision }, response => {
        if (response.result.kind !== "scratchpad") return;
        const document = response.result.document;
        model.updateDrafts(current => ({ ...current, scratchpad: current.scratchpad ? acknowledgeDraft(current.scratchpad, document.content, document.revision, submitted) : current.scratchpad }));
      });
    } finally { setSavingMine(false); }
  };
  const scratch = model.drafts.scratchpad;
  const activeDetail = showDetail && surface.selectedTodo && (surface.tab === "kanban" || surface.tab === "todos");
  const visibleTodos = model.todos.filter(todo => !surface.hideCompleted || !todo.done);
  return <NotesEditorScope value={info.notes_id}>
    <div className="notes-tabs" role="tablist" aria-label="Notes views">{tabs.map(tab => {
      const n = tab === "todos" ? model.todos.filter(todo => !todo.done).length : tab === "kanban" ? model.todos.filter(todo => todo.lane !== null).length : model.decisions.length;
      const name = tab === "todos" ? `Todos — ${n} open` : tab === "kanban" ? `Kanban — ${n} on board` : tab === "decisions" ? `Decisions — ${n} ${n === 1 ? "record" : "records"} (Current and History)` : tabLabels[tab];
      return <button type="button" role="tab" id={`f-${tab}`} key={tab} aria-label={name} title={name} aria-selected={surface.tab === tab} aria-controls={`notes-${tab}-panel`} tabIndex={surface.tab === tab ? 0 : -1} onClick={() => setSurface(current => ({ ...current, tab }))} onKeyDown={event => {
      if (!event.ctrlKey && !event.metaKey && (event.key === "ArrowRight" || event.key === "ArrowLeft" || event.key === "Home" || event.key === "End")) {
        event.preventDefault();
        const next = event.key === "Home" ? tabs[0] : event.key === "End" ? tabs[3] : tabs[(tabs.indexOf(tab) + (event.key === "ArrowRight" ? 1 : 3)) % 4];
        setSurface(current => ({ ...current, tab: next })); document.getElementById(`f-${next}`)?.focus();
      }
    }}><UiIcon name={tab === "scratchpad" ? "file" : tab === "todos" ? "check" : tab === "kanban" ? "grid" : "library"} /><span>{tabLabels[tab]}</span>{tab !== "scratchpad" ? <span className="notes-tab-count" aria-hidden="true">{tab === "todos" ? model.todos.filter(todo => !todo.done).length : tab === "kanban" ? model.todos.filter(todo => todo.lane !== null).length : model.decisions.length}</span> : null}</button>;
    })}</div>
    <div className={`notes-panels${activeDetail ? " has-detail" : ""}`}>
      <section id="notes-scratchpad-panel" className="notes-scratchpad" role="tabpanel" aria-labelledby="f-scratchpad" hidden={surface.tab !== "scratchpad"}>
        {scratch ? <><MarkdownEditor label="Scratchpad Markdown" value={scratch.value} onChange={value => model.updateDrafts(current => ({ ...current, scratchpad: { ...scratch, value } }))} onSave={() => void saveScratchpad()} /><div className="notes-save-status"><span className={`notes-save-state${model.busy ? " is-saving" : scratch.value === scratch.base ? " is-saved" : " is-draft"}`} role="status"><span className="notes-save-dot" aria-hidden="true" />{model.busy ? "Saving Notes…" : scratch.value === scratch.base ? "Saved · scratchpad.md" : "Draft kept · not yet saved"}</span>{scratch.conflict ? <div className="notes-conflict" role="alert"><p>Scratchpad changed elsewhere. Current saved version:</p><details><summary>Show saved Markdown</summary><small className="notes-conflict-saved-label">Saved version (read-only)</small><MarkdownPreview content={model.scratchpad?.content ?? ""} /></details><button type="button" disabled={model.busy || unknown || savingMine} onClick={() => void saveScratchpad(true)}>Keep mine and save</button><button type="button" onClick={() => { if (model.scratchpad) model.updateDrafts(current => ({ ...current, scratchpad: changedDraft(model.scratchpad!.content, model.scratchpad!.content, model.scratchpad!.revision) })); }}>Reload (discard my draft)</button><p className="notes-conflict-consequence">Keep mine overwrites the saved version. Reload discards your draft.</p></div> : <button className="notes-primary" type="button" disabled={model.busy || unknown || scratch.value === scratch.base} onClick={() => void saveScratchpad()}>Save scratchpad</button>}</div></> : <p>Loading scratchpad…</p>}
      </section>
      <section id="notes-todos-panel" className="notes-todos notes-task-main" role="tabpanel" aria-labelledby="f-todos" hidden={surface.tab !== "todos"}>
        <form className="notes-add-form" onSubmit={event => { event.preventDefault(); const text = model.drafts.addTodo; if (text.trim()) void model.mutate({ op: "todo_add", text, lane: null }, () => model.updateDrafts(current => current.addTodo === text ? { ...current, addTodo: "" } : current)); }}><UiIcon name="plus" /><input id="todoAddInput" aria-label="Add a todo" placeholder="Add a todo…" value={model.drafts.addTodo} onChange={event => model.updateDrafts(current => ({ ...current, addTodo: event.target.value }))} /><button className="notes-primary" id="todoAddBtn" type="submit" disabled={model.busy || unknown || !model.drafts.addTodo.trim()}>Add</button></form>
        <div className="notes-todo-toolbar"><button className="notes-toggle-chip" id="toggleCompletedBtn" type="button" aria-pressed={surface.hideCompleted} onClick={() => setSurface(current => ({ ...current, hideCompleted: !current.hideCompleted }))}><UiIcon name="check" />Hide completed</button><details><summary aria-label="Markdown source todos.md" title="Markdown source"><UiIcon name="code" /><span className="notes-source-label">todos.md</span></summary><div className="notes-source-popover"><p>Todos and board share this ordinary file. Open it in your editor; Cockpit preserves headings, prose and task boundaries.</p><input aria-label="Todos Markdown file" readOnly value={`${info.folder}/todos.md`} onClick={event => event.currentTarget.select()} /></div></details></div>
        <ul id="todoList" className="notes-todo-list">{visibleTodos.map(todo => {
          const adopt = !todo.id || todo.problems.includes("duplicate_id");
          return <li className={`todo-row${surface.selectedTodo === todo.id ? " is-selected" : ""}`} key={todoIdentity(todo)} data-todo-id={todoIdentity(todo)} data-depth={Math.min(todo.depth, 6)} style={{ paddingInlineStart: `${12 + Math.min(todo.depth, 6) * 12}px` }}>
          <label className="notes-check"><input type="checkbox" aria-label={`Complete ${todo.text}`} checked={todo.done} disabled={model.busy || unknown} onChange={() => void model.mutate({ op: "todo_set_done", todo: todoSelector(todo), done: !todo.done })} /></label><TodoTitle model={model} todo={todo} />
          <div className="notes-row-actions"><button className={`notes-todo-comments${adopt ? " is-adopt" : ""}`} type="button" title={adopt ? "Adopt task & comments — adds an id to this task in todos.md" : undefined} aria-label={!todo.id || todo.problems.includes("duplicate_id") ? "Adopt task & comments" : counts[todo.id] === undefined ? "Comments" : `Comments ${counts[todo.id]}`} onClick={event => void openDetails(todo, event.currentTarget)} disabled={model.busy || unknown || todo.problems.includes("metadata_malformed") || todo.problems.includes("lazy_continuation")}>{adopt ? <><UiIcon name="plus" /><span className="notes-adopt-label">Adopt</span></> : <UiIcon name="comment" />}{todo.id && (counts[todo.id] ?? 0) > 0 ? <span className="notes-comment-count">{counts[todo.id]}</span> : null}</button>{todo.lane ? <button className="todo-board-chip notes-todo-secondary notes-icon-button" type="button" aria-label={`On board · ${todo.done ? "Done" : todo.lane === "doing" ? "Doing" : "Backlog"}`} title={`On board · ${todo.done ? "Done" : todo.lane === "doing" ? "Doing" : "Backlog"}`} onClick={() => { setSurface(current => ({ ...current, tab: "kanban" })); setShowDetail(false); requestAnimationFrame(() => { const cards = document.querySelectorAll<HTMLElement>(".kanban-card"); const card = Array.from(cards).find(node => node.dataset.todoId === (todoIdentity(todo))); card?.scrollIntoView({ block: "nearest", inline: "nearest" }); card?.querySelector<HTMLElement>("textarea")?.focus({ preventScroll: true }); }); }}><span className="todo-board-lane" aria-hidden="true">{todo.done ? "Dn" : todo.lane === "doing" ? "Do" : "B"}</span></button> : <button className="todo-promote-btn notes-todo-secondary notes-icon-button" type="button" aria-label="+ Board" title="+ Board" disabled={model.busy || unknown} onClick={() => void model.mutate({ op: "kanban_promote", todo: todoSelector(todo) })}><UiIcon name="grid" /></button>}</div>
        </li>;
        })}</ul>
        {model.todos.length && !visibleTodos.length ? <p className="notes-empty notes-todos-filtered-empty">All tasks are completed and hidden.</p> : null}
        {!model.todos.length ? <div className="notes-empty notes-empty-actionable"><span className="notes-empty-icon"><UiIcon name="check" /></span><p>No todos yet. Add one above.</p><button type="button" onClick={() => document.getElementById("todoAddInput")?.focus()}>Add a todo</button></div> : null}
      </section>
      <section id="notes-kanban-panel" className="notes-task-main" role="tabpanel" aria-labelledby="f-kanban" hidden={surface.tab !== "kanban"}>{surface.tab === "kanban" ? <Kanban model={model} onDetails={(todo, opener) => void openDetails(todo, opener)} counts={counts} selectedId={surface.selectedTodo} /> : null}</section>
      <section id="notes-decisions-panel" role="tabpanel" aria-labelledby="f-decisions" hidden={surface.tab !== "decisions"}><Decisions model={model} selectedId={surface.selectedDecision} onSelect={selectedDecision => setSurface(current => ({ ...current, selectedDecision }))} /></section>
      {activeDetail ? <TaskDetail key={surface.selectedTodo} model={model} todo={selectedTodo} todoId={surface.selectedTodo!} onClose={closeDetails} onCount={(id, count) => setCounts(current => current[id] === count ? current : { ...current, [id]: count })} /> : null}
    </div>
    <div className="notes-problem-slot">
      <div className="notes-problem-content">
      {model.error ? <div className={unknown ? "notes-error notes-unknown" : "notes-error"} role="alert"><UiIcon name="info" />{unknown ? <ol className="notes-unknown-steps">
        <li><span>Could not confirm whether the change was saved. Your drafts are kept. Check saved state before choosing any next write; posting again may duplicate.</span> <code>{model.error.code}</code></li>
        <li><button type="button" disabled={model.busy || checkingSaved} onClick={() => void checkSavedState()}>{checkingSaved ? "Checking saved state…" : "Check saved state"}</button></li>
        {reviewedUnknown ? <li><span>Saved state was read. Review it first: the next write may duplicate the unconfirmed change.</span><button type="button" disabled={model.busy || checkingSaved} onClick={acknowledgeUnknown}>I checked saved state; allow next write</button></li> : null}
      </ol> : <><span>{unknown ? "Could not confirm whether the change was saved. Your drafts are kept. Check saved state before choosing any next write; posting again may duplicate." : model.error.code === "notes_conflict" ? "Changed elsewhere. Your text is kept; use the editor's Reload or Keep mine controls." : model.error.message}</span><code>{model.error.code}</code><button type="button" disabled={model.busy || checkingSaved} onClick={() => void checkSavedState()}>{checkingSaved ? "Checking saved state…" : "Check saved state"}</button>{reviewedUnknown ? <><span>Saved state was read. Review it first: the next write may duplicate the unconfirmed change.</span><button type="button" disabled={model.busy || checkingSaved} onClick={acknowledgeUnknown}>I checked saved state; allow next write</button></> : null}</>}</div> : null}
      {model.storageError ? <p className="notes-error" role="alert"><UiIcon name="info" /><span>{model.storageError}</span></p> : null}
      </div>
    </div>
    <RetainedDrafts model={model} />
    <AgentAccess info={info} model={model} todo={selectedTodo} decisionId={surface.selectedDecision} />
  </NotesEditorScope>;
}

export function NotesView({ client, space, onClose }: { client: CockpitClient; space: LibrarySpace | null; onClose(): void }) {
  const [info, setInfo] = useState<NotesTargetInfo | null>(null);
  const [error, setError] = useState<{ code: string; message: string } | null>(null);
  const [loading, setLoading] = useState(true);
  const [picker, setPicker] = useState(false);
  const [catalog, setCatalog] = useState<CatalogState>({ status: "idle" });
  const [selected, setSelected] = useState<string | null>(null);
  const [confirmTransfer, setConfirmTransfer] = useState(false);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const root = useRef<HTMLElement>(null);
  const invoker = useRef<HTMLElement | null>(null);
  const pickerInvoker = useRef<HTMLElement | null>(null);
  const attachOpener = useRef<HTMLButtonElement>(null);
  const attachAction = useRef<HTMLButtonElement>(null);
  const keepAssociation = useRef<HTMLButtonElement>(null);
  const catalogRetry = useRef<HTMLButtonElement>(null);
  const pickerCancel = useRef<HTMLButtonElement>(null);
  const pickerVisible = useRef(picker); pickerVisible.current = picker;
  const resolveRef = useRef<() => Promise<void>>(async () => {});
  const target = space?.target;
  useEffect(() => {
    invoker.current = document.activeElement instanceof HTMLElement && !root.current?.contains(document.activeElement) ? document.activeElement : null;
    root.current?.focus({ preventScroll: true });
  }, []);
  useLayoutEffect(() => {
    const element = root.current;
    return () => {
      if (element?.contains(document.activeElement) || document.activeElement === document.body) {
        const fallback = invoker.current?.isConnected ? invoker.current : document.querySelector<HTMLElement>('[aria-controls="cockpit-notes"]');
        fallback?.focus({ preventScroll: true });
      }
    };
  }, []);
  useLayoutEffect(() => {
    if (!picker) return;
    const focused = document.activeElement;
    if ((focused !== document.body && focused !== root.current) || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
    const initial = catalog.status === "ready"
      ? root.current?.querySelector<HTMLInputElement>('input[name="notes-attach"]:checked') ?? root.current?.querySelector<HTMLInputElement>('input[name="notes-attach"]') ?? pickerCancel.current
      : catalog.status === "error" ? catalogRetry.current : root.current;
    initial?.focus({ preventScroll: true });
  }, [picker, catalog.status]);
  useLayoutEffect(() => {
    if (!confirmTransfer || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
    const focused = document.activeElement;
    if (focused === document.body || focused === root.current) keepAssociation.current?.focus({ preventScroll: true });
  }, [confirmTransfer]);
  useEffect(() => {
    let active = true;
    setInfo(null); setError(null); setLoading(true); setPicker(false); setSelected(null); setConfirmTransfer(false);
    const resolve = async () => {
      if (!target) { if (active) setLoading(false); return; }
      if (!space?.live) { if (active) { setError({ code: "notes_space_unavailable", message: "The Space association needs a live Herdr connection. Drafts are kept; reconnect before resolving it." }); setLoading(false); } return; }
      try {
        const response = await client.notes({ target: { kind: "space", ...target }, operation: { op: "target_resolve" } });
        if (active && response.result.kind === "target") { setInfo(response.result.info); setError(null); }
      } catch (failure) { if (active) { setError(notesError(failure)); setInfo(null); } }
      finally { if (active) setLoading(false); }
    };
    resolveRef.current = resolve;
    void resolve();
    const onFocus = () => { if (!pending.current && !pickerVisible.current && document.visibilityState !== "hidden") void resolve(); };
    const timer = window.setInterval(onFocus, 5000);
    window.addEventListener("focus", onFocus);
    return () => { active = false; window.clearInterval(timer); window.removeEventListener("focus", onFocus); };
  }, [client, target?.session_id, target?.space_id, space?.live]);
  const bind = async (notesId?: string) => {
    if (!target || pending.current) return;
    pending.current = true; setBusy(true); setError(null);
    try {
      const response = await client.notes({ target: { kind: "space", ...target }, operation: notesId ? { op: "target_attach", notes_id: notesId } : { op: "target_create" } });
      if (response.result.kind === "target") { setInfo(response.result.info); setPicker(false); }
    } catch (failure) { setError(notesError(failure)); }
    finally { pending.current = false; setBusy(false); }
  };
  const loadCatalog = async () => {
    if (!pickerVisible.current) pickerInvoker.current = document.activeElement instanceof HTMLElement ? document.activeElement : attachOpener.current;
    setPicker(true); setCatalog({ status: "loading" }); setConfirmTransfer(false);
    try {
      const response = await client.notes({ target: { kind: "root" }, operation: { op: "catalog_list" } });
      if (response.result.kind !== "catalog") throw new Error("Notes returned an unexpected catalog result");
      setCatalog({ status: "ready", entries: response.result.entries });
    } catch (failure) { setCatalog({ status: "error", problem: notesError(failure) }); }
  };
  const closePicker = () => {
    const closingControl = document.activeElement;
    setPicker(false); setConfirmTransfer(false);
    requestAnimationFrame(() => {
      if (!root.current?.isConnected || pending.current || (document.activeElement !== document.body && document.activeElement !== closingControl)) return;
      const opener = pickerInvoker.current?.isConnected ? pickerInvoker.current : attachOpener.current;
      (opener ?? root.current)?.focus({ preventScroll: true });
    });
  };
  const cancelTransfer = () => {
    const closingControl = document.activeElement;
    setConfirmTransfer(false);
    requestAnimationFrame(() => {
      if (!root.current?.isConnected || pending.current || (document.activeElement !== document.body && document.activeElement !== closingControl)) return;
      attachAction.current?.focus({ preventScroll: true });
    });
  };
  const candidate = catalog.status === "ready" ? catalog.entries.find(entry => entry.notes_id === selected) : undefined;
  return <section id="cockpit-notes" className="notes-view" ref={root} tabIndex={-1} aria-label="Notes" aria-busy={busy || loading || (picker && catalog.status === "loading")} onKeyDown={event => { if (event.key === "Escape" && !event.defaultPrevented && !event.nativeEvent.isComposing) { event.preventDefault(); if (confirmTransfer) cancelTransfer(); else if (picker) closePicker(); else onClose(); } }}>
    <header className="notes-view-header"><div className="notes-view-context"><UiIcon name="file" /><h2>Notes</h2><span className="notes-space-label">{space?.label ?? "Select a Space"}</span></div><button className="notes-icon-button" type="button" aria-label="Close Notes" onClick={onClose}><UiIcon name="close" /></button></header>
    {info ? <NotesContent key={info.notes_id} client={client} info={info} /> : <div className={`notes-binding${picker ? " notes-binding-picker" : " notes-binding-empty"}`}>
      {!target ? <p>Select a Space to open its notes.</p> : picker ? <>
        <h3>Attach existing notes</h3><p>Choose the exact Notes ID. Files stay outside the repository.</p>
        {catalog.status === "loading" ? <p>Finding existing notes…</p> : catalog.status === "error" ? <div className="notes-error" role="alert"><span>{catalog.problem.message} <code>{catalog.problem.code}</code></span><button ref={catalogRetry} type="button" onClick={() => void loadCatalog()}>Retry reading catalog</button></div> : catalog.status === "ready" ? catalog.entries.length ? <div className="notes-catalog">{catalog.entries.map(entry => <label key={entry.notes_id}><input type="radio" name="notes-attach" checked={selected === entry.notes_id} onChange={() => { setSelected(entry.notes_id); setConfirmTransfer(false); }} /><span>{entry.label ? `Last called ${entry.label}` : "Untitled notes"}<code>{entry.notes_id}</code><small>{entry.bound ? "Attached elsewhere" : "Not attached"}</small></span></label>)}</div> : <p>No existing notes found.</p> : null}
        {catalog.status === "ready" && selected && !candidate ? <p className="notes-catalog-missing">The selected notes are no longer in the catalog.</p> : null}
        {confirmTransfer ? <div className="notes-conflict" role="group" aria-label="Confirm transferring Notes association"><p>Attach here? The other Space will become unbound. Files are not moved or deleted.</p><button ref={keepAssociation} type="button" onClick={cancelTransfer}>Keep current association</button><button className="notes-transfer-attach" type="button" disabled={busy || !candidate} onClick={() => { if (candidate) void bind(candidate.notes_id); }}>Attach here</button></div> : null}
        <div className="notes-picker-actions">{confirmTransfer ? null : <button ref={attachAction} type="button" disabled={!candidate || busy} onClick={() => { if (!candidate) return; if (candidate.bound) setConfirmTransfer(true); else void bind(candidate.notes_id); }}>Attach</button>}<button ref={pickerCancel} type="button" disabled={busy} onClick={closePicker}>Cancel</button></div>
      </> : loading ? <p>Resolving this Space's notes…</p> : error?.code === "notes_unbound" ? <>
        <span className="notes-empty-icon"><UiIcon name="file" /></span><h3>No notes for this Space</h3><p>Notes live outside the repository. Create notes for this Space, or attach notes you already have.</p><div className="notes-binding-actions"><button className="notes-primary" type="button" id="createNotesBtn" disabled={busy || !space?.live} onClick={() => void bind()}><UiIcon name="plus" />Create notes</button><button ref={attachOpener} type="button" id="attachNotesBtn" disabled={busy || !space?.live} onClick={() => void loadCatalog()}><UiIcon name="clip" />Attach existing notes…</button></div>
      </> : <><p>Could not resolve this Space's notes.</p><button type="button" disabled={busy} onClick={() => void resolveRef.current()}>Retry</button></>}
      {error && error.code !== "notes_unbound" ? <p className="notes-error" role="alert">{error.message} <code>{error.code}</code></p> : null}
    </div>}
  </section>;
}
