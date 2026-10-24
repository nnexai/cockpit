import { useEffect, useRef, useState } from "react";
import type { NotesDecision, NotesDecisionSummary } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { acknowledgeDraft, changedDraft, reconcileDraft } from "./drafts"
import { MarkdownEditor, MarkdownPreview } from "./MarkdownEditor";
import { notesError, type NotesModel } from "./useNotes";

export function Decisions({ model, selectedId, onSelect }: { model: NotesModel; selectedId: string | null; onSelect(id: string | null): void }) {
  const [filter, setFilter] = useState<"current" | "history">("current");
  const [query, setQuery] = useState("");
  const [oldest, setOldest] = useState(false);
  const [records, setRecords] = useState<NotesDecisionSummary[]>([]);
  const [otherMatches, setOtherMatches] = useState(0);
  const [record, setRecord] = useState<NotesDecision | null>(null);
  const [showNew, setShowNew] = useState(Boolean(model.drafts.newDecision));
  const [showList, setShowList] = useState(!selectedId && !model.drafts.newDecision);
  const [editing, setEditing] = useState(false);
  const [loading, setLoading] = useState(false);
  const creating = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [copyStatus, setCopyStatus] = useState("");
  const recordDetails = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const closeOutside = (event: PointerEvent) => {
      const node = recordDetails.current;
      if (node?.open && event.target instanceof Node && !node.contains(event.target)) node.open = false;
    };
    document.addEventListener("pointerdown", closeOutside);
    return () => document.removeEventListener("pointerdown", closeOutside);
  }, []);
  useEffect(() => { setCopyStatus(""); }, [selectedId]);
  const copyReference = async (label: string, value: string) => {
    setCopyStatus("");
    try { await navigator.clipboard.writeText(value); setCopyStatus(`Copied ${label}.`); }
    catch { setCopyStatus(`Could not copy ${label}. Select the path and copy it manually.`); }
  };
  const revision = model.decisions.find(item => item.decision_id === selectedId)?.revision;
  const newDraft = model.drafts.newDecision;
  const edit = selectedId ? model.drafts.decisionEdits[selectedId] : undefined;
  const dirty = edit && (edit.title.value !== edit.title.base || edit.body.value !== edit.body.base);
  const unknown = model.error?.code === "notes_outcome_unknown";
  const replacementSource = newDraft?.replaces ? model.decisions.find(item => item.decision_id === newDraft.replaces) : undefined;
  const replacementBlocked = Boolean(newDraft?.replaces && model.scratchpad && (!replacementSource || replacementSource.status === "replaced" || replacementSource.revision !== newDraft.revision));
  useEffect(() => {
    let active = true;
    const timer = window.setTimeout(() => {
      setLoading(true);
      void model.request({ op: "decision_list", status: filter, query: query || null }).then(async response => {
        if (!active || response.result.kind !== "decisions") return;
        setRecords(response.result.decisions); setError(null);
        if (!response.result.decisions.length && query) {
          const other = await model.request({ op: "decision_list", status: filter === "current" ? "history" : "current", query });
          if (active && other.result.kind === "decisions") setOtherMatches(other.result.decisions.length);
        } else setOtherMatches(0);
      }).catch(failure => { if (active) setError(notesError(failure).message); }).finally(() => { if (active) setLoading(false); });
    }, 160);
    return () => { active = false; window.clearTimeout(timer); };
  }, [model.request, model.decisions, filter, query]);
  useEffect(() => {
    if (!selectedId) { setRecord(null); return; }
    let active = true;
    void model.request({ op: "decision_get", decision_id: selectedId }).then(response => {
      if (!active || response.result.kind !== "decision") return;
      const next = response.result.decision;
      setRecord(next); setError(null);
      model.updateDrafts(current => {
        const previous = current.decisionEdits[selectedId];
        if (!previous) return current;
        return { ...current, decisionEdits: { ...current.decisionEdits, [selectedId]: { title: reconcileDraft(previous.title, next.summary.title, next.summary.revision), body: reconcileDraft(previous.body, next.body, next.summary.revision) } } };
      });
    }).catch(failure => { if (active) { setError(notesError(failure).message); setRecord(null); } });
    return () => { active = false; };
  }, [model.request, model.updateDrafts, selectedId, revision]);
  const beginNew = () => {
    model.updateDrafts(current => current.newDecision ? current : { ...current, newDecision: { title: "", body: "", decided: "", replaces: null, revision: null } });
    setShowNew(true); setShowList(false); setEditing(false);
  };
  const saveNew = async () => {
    if (!newDraft?.title.trim() || creating.current || unknown || replacementBlocked) return;
    creating.current = true;
    const submitted = { ...newDraft };
    try {
      await model.mutate(submitted.replaces && submitted.revision ? { op: "decision_replace", decision_id: submitted.replaces, expected_revision: submitted.revision, title: submitted.title, body: submitted.body, decided: submitted.decided || null } : { op: "decision_create", title: submitted.title, body: submitted.body, decided: submitted.decided || null }, response => {
        if (response.result.kind !== "decision") return;
        onSelect(response.result.decision.summary.decision_id); setRecord(response.result.decision); setShowNew(false); setShowList(false);
        model.updateDrafts(current => {
          if (current.newDecision?.title !== submitted.title || current.newDecision?.body !== submitted.body || current.newDecision?.decided !== submitted.decided) return current;
          const next = { ...current }; delete next.newDecision; return next;
        });
      });
    } finally { creating.current = false; }
  };
  const saveEdit = async (keepMine = false) => {
    if (!record || !edit || !dirty || unknown || (!keepMine && (edit.title.conflict || edit.body.conflict))) return;
    const title = edit.title.value; const body = edit.body.value; const id = record.summary.decision_id;
    await model.mutate({ op: "decision_update", decision_id: id, expected_revision: keepMine ? record.summary.revision : edit.body.value !== edit.body.base ? edit.body.revision : edit.title.revision, title, body }, response => {
      if (response.result.kind !== "decision") return;
      const saved = response.result.decision;
      setRecord(saved);
      let retainedTyping = false;
      model.updateDrafts(current => {
        const edits = { ...current.decisionEdits };
        const previous = edits[id];
        if (previous) {
          const next = { title: acknowledgeDraft(previous.title, saved.summary.title, saved.summary.revision, title), body: acknowledgeDraft(previous.body, saved.body, saved.summary.revision, body) };
          retainedTyping = next.title.value !== next.title.base || next.body.value !== next.body.base;
          if (retainedTyping) edits[id] = next; else delete edits[id];
        }
        return { ...current, decisionEdits: edits };
      });
      if (!retainedTyping) setEditing(false);
    });
  };
  const ordered = [...records].sort((a, b) => {
    const left = a.recorded ? Date.parse(a.recorded) : NaN; const right = b.recorded ? Date.parse(b.recorded) : NaN;
    if (Number.isNaN(left)) return Number.isNaN(right) ? a.decision_id.localeCompare(b.decision_id) : 1;
    if (Number.isNaN(right)) return -1;
    return (oldest ? left - right : right - left) || a.decision_id.localeCompare(b.decision_id);
  });
  const beginEdit = () => {
    if (!record) return;
    model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: current.decisionEdits[record.summary.decision_id] ?? { title: changedDraft(record.summary.title, record.summary.title, record.summary.revision), body: changedDraft(record.body, record.body, record.summary.revision) } } }));
    setEditing(true);
  };
  const beginReplacement = () => {
    if (!record) return;
    model.updateDrafts(current => current.newDecision ? current : { ...current, newDecision: { title: record.summary.title, body: "", decided: "", replaces: record.summary.decision_id, revision: record.summary.revision } });
    setShowNew(true); setEditing(false);
  };
  return <div className={`notes-decisions${showList ? " is-list" : ""}`}>
    <section className="notes-decision-collection">
      <div className="notes-decision-controls">
        <div className="notes-decision-control-row">
          <div className="notes-decision-search"><UiIcon name="search" /><input id="decisionSearch" aria-label="Search decision title and body" placeholder="Search decisions" value={query} onChange={event => setQuery(event.target.value)} /></div>
          <button type="button" className="notes-primary notes-icon-button" id="newDecisionBtn" title={newDraft ? "Resume draft" : "New decision"} aria-label={newDraft ? "Resume draft" : "New decision"} onClick={beginNew}><UiIcon name={newDraft ? "edit" : "plus"} /></button>
        </div>
        <div className="notes-decision-filters">
          <div className="notes-decision-status" role="group" aria-label="Decision status"><button type="button" aria-pressed={filter === "current"} onClick={() => setFilter("current")}>Current <span>{model.decisions.filter(item => item.status === "current").length}</span></button><button type="button" aria-pressed={filter === "history"} onClick={() => setFilter("history")}>History <span>{model.decisions.filter(item => item.status === "replaced").length}</span></button></div>
          <button type="button" id="sortToggle" title={oldest ? "Oldest first" : "Newest first"} aria-label={oldest ? "Sort order: Oldest first. Activate to show newest first." : "Sort order: Newest first. Activate to show oldest first."} onClick={() => setOldest(!oldest)}><UiIcon name="down" /></button>
        </div>
      </div>
      <div id="decisionList" className="notes-decision-list" aria-busy={loading}>
        {ordered.map(item => <button type="button" key={item.decision_id} className={`notes-decision-row${selectedId === item.decision_id && !showNew ? " is-selected" : ""}`} onClick={() => { onSelect(item.decision_id); setShowNew(false); setEditing(false); setShowList(false); }}><strong className="notes-decision-title">{item.title}</strong><span className="notes-decision-meta"><small className="notes-decision-date">{item.recorded && !Number.isNaN(Date.parse(item.recorded)) ? `Recorded ${new Date(item.recorded).toLocaleDateString()}` : "Recorded: unknown"}{item.status === "replaced" ? " · Replaced" : ""}</small></span></button>)}
        {!loading && !records.length ? <div className="notes-empty notes-decision-index-empty">
          <p>{query ? `No ${filter} decisions match “${query}”.` : `No ${filter} decisions yet.`}</p>
          {query ? <button type="button" onClick={() => setQuery("")}>Clear search</button> : null}
          {otherMatches ? <p>{otherMatches} matching {filter === "current" ? "History" : "Current"} {otherMatches === 1 ? "record" : "records"}. <button type="button" onClick={() => setFilter(filter === "current" ? "history" : "current")}>Show {filter === "current" ? "History" : "Current"}</button></p> : null}
        </div> : null}
      </div>
    </section>
    <section className="notes-decision-detail">
      <div className="notes-decision-toolbar">
        <button type="button" className="notes-decision-back" id="decisionBackBtn" aria-label="Back to decisions" onClick={() => setShowList(true)}><UiIcon name="back" />Decisions</button>
        <div className="notes-decision-toolbar-actions">
          {showNew && newDraft ? <>
            <span className="notes-decision-draft" role="status"><span className="notes-draft-dot" aria-hidden="true" />Draft kept</span>
            <button type="button" id="cancelDraftBtn" aria-label="Cancel (discard draft)" disabled={model.busy} onClick={() => { model.updateDrafts(current => { const next = { ...current }; delete next.newDecision; return next; }); setShowNew(false); }}>Discard draft</button>
            <button type="button" className="notes-primary" id="recordDecisionBtn" disabled={model.busy || !newDraft.title.trim() || unknown || replacementBlocked} onClick={() => void saveNew()}>{model.busy ? "Recording…" : "Record decision"}</button>
          </> : record && editing && edit ? <>
            {dirty ? <span className="notes-decision-draft" role="status"><span className="notes-draft-dot" aria-hidden="true" />Draft kept</span> : null}
            <button type="button" aria-label="Back / keep edit" onClick={() => setEditing(false)}>Back</button>
            <button type="button" id="discardEditsBtn" aria-label="Discard edits" onClick={() => { model.updateDrafts(current => { const edits = { ...current.decisionEdits }; delete edits[record.summary.decision_id]; return { ...current, decisionEdits: edits }; }); setEditing(false); }}>Discard</button>
            {!edit.title.conflict && !edit.body.conflict ? <button type="button" className="notes-primary" id="saveEditsBtn" aria-label="Save edits" title="Save edits (Ctrl+S)" disabled={!dirty || model.busy || unknown} onClick={() => void saveEdit()}>Save</button> : null}
          </> : record ? <>
            <button type="button" className="notes-decision-edit" aria-label={dirty ? "Resume edit · draft kept" : "Edit"} title={dirty ? "Resume edit · draft kept" : "Edit"} onClick={beginEdit}><UiIcon name="edit" /><span className="notes-reader-action-label">Edit</span>{dirty ? <span className="notes-draft-dot" aria-hidden="true" /> : null}</button>
            <button type="button" id="replaceBtn" aria-label="Replace with a new decision" title={record.summary.status !== "current" ? "Already replaced. Follow the successor instead." : newDraft ? "Finish or cancel your existing draft before replacing." : "Replace with a new decision"} disabled={record.summary.status !== "current" || model.busy || Boolean(newDraft)} onClick={beginReplacement}>Replace…</button>
            <details className="notes-decision-more" ref={recordDetails} onKeyDown={event => {
              if (event.key !== "Escape" || !event.currentTarget.open) return;
              event.preventDefault(); event.stopPropagation(); event.currentTarget.open = false;
              event.currentTarget.querySelector<HTMLElement>("summary")?.focus();
            }}>
              <summary aria-label="Record details" title="Record details"><UiIcon name="info" /></summary>
              <div>
                <span>Recorded (immutable): {record.summary.recorded ?? "unknown"}</span>
                <span>Decided: {record.summary.decided ?? "unknown"}</span>
                <label>Reference<span className="notes-decision-reference"><input id="moreReference" readOnly value={record.relative_path} title={record.relative_path} onClick={event => event.currentTarget.select()} /><button className="notes-icon-button" type="button" aria-label="Copy decision reference" title="Copy decision reference" onClick={() => void copyReference("decision reference", record.relative_path)}><UiIcon name="copy" /></button></span></label>
                <label>File<span className="notes-decision-reference"><input readOnly value={record.path} title={record.path} onClick={event => event.currentTarget.select()} /><button className="notes-icon-button" type="button" aria-label="Copy decision file path" title="Copy decision file path" onClick={() => void copyReference("decision file path", record.path)}><UiIcon name="copy" /></button></span></label>
                <p role="status" aria-atomic="true">{copyStatus}</p>
              </div>
            </details>
          </> : null}
        </div>
      </div>
      <div className={`notes-error-row${error ? " has-error" : ""}`}>{error ? <p role="alert"><UiIcon name="info" /><span>{error}</span></p> : null}</div>
      {record && !showNew && selectedId && !loading && !records.some(item => item.decision_id === selectedId) ? <p className="notes-decision-context">This record isn't in the current list filter.</p> : null}
      {showNew && newDraft ? <div className="notes-decision-write">
        <label className="notes-decision-title-field"><span className="sr-only">Title</span><input id="newTitleInput" aria-label="New decision title" placeholder="Decision title" value={newDraft.title} maxLength={512} onChange={event => model.updateDrafts(current => ({ ...current, newDecision: { ...newDraft, title: event.target.value } }))} /></label>
        <div className="notes-decision-new-meta">
          <span>{newDraft.replaces ? "Replacement — the old file stays unchanged" : "New decision"}</span>
          <label>Decided <input aria-label="Decision historical date" placeholder="YYYY-MM-DD or RFC3339" value={newDraft.decided} onChange={event => model.updateDrafts(current => ({ ...current, newDecision: { ...newDraft, decided: event.target.value } }))} /></label>
          <span>Recorded date is set on save</span>
        </div>
      {showNew && newDraft?.replaces && replacementBlocked ? <div className="notes-conflict" role="alert"><p><UiIcon name="info" /><span>{!replacementSource ? "The source decision is no longer present. Your replacement draft is kept." : replacementSource.status === "replaced" ? "This decision has already been replaced. Your draft is kept; review its successor rather than recording another replacement." : `The source decision changed elsewhere: ${replacementSource.title}. Review it before replacing this version.`}</span></p>{replacementSource ? <button type="button" onClick={() => { onSelect(replacementSource.decision_id); setShowNew(false); setEditing(false); }}>Review source decision</button> : null}{replacementSource?.status === "current" ? <button type="button" onClick={() => model.updateDrafts(current => current.newDecision ? { ...current, newDecision: { ...current.newDecision, revision: replacementSource.revision } } : current)}>Use current source revision</button> : null}</div> : null}
        <MarkdownEditor label="New decision Markdown" value={newDraft.body} onChange={body => model.updateDrafts(current => ({ ...current, newDecision: { ...newDraft, body } }))} onSave={() => void saveNew()} />
      </div> : record ? <>
        {!editing ? <header className="notes-decision-heading"><h3>{record.summary.title}</h3></header> : null}
        {!editing ? <div className="notes-decision-footer">
          <div className="notes-decision-footer-meta">
            <span>{record.summary.recorded && !Number.isNaN(Date.parse(record.summary.recorded)) ? `Recorded ${new Date(record.summary.recorded).toLocaleDateString()}` : "Recorded: unknown"}</span>
            {record.summary.decided ? <span>Decided {record.summary.decided}</span> : null}
            {record.summary.replaces ? <button type="button" onClick={() => { onSelect(record.summary.replaces); setShowNew(false); setEditing(false); }}>Replaces {model.decisions.find(item => item.decision_id === record.summary.replaces)?.title ?? `${record.summary.replaces} (missing record)`}</button> : null}
            {record.summary.replaced_by.map(id => <button type="button" key={id} onClick={() => { onSelect(id); setShowNew(false); setEditing(false); }}>Replaced by {model.decisions.find(item => item.decision_id === id)?.title ?? id}</button>)}
          </div>
          {record.summary.problems.length ? <p className="notes-decision-source-issues"><UiIcon name="info" /><span>Source issues: {record.summary.problems.join(", ")}</span></p> : null}
        </div> : null}
        {editing && edit ? <div className="notes-decision-write">
          <label className="notes-decision-title-field"><span className="sr-only">Title</span><input id="editTitleInput" aria-label="Edit decision title" value={edit.title.value} onChange={event => model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: { ...edit, title: { ...edit.title, value: event.target.value } } } }))} /></label>
          <p className="notes-decision-write-hint">Editing · Decided and recorded dates are kept</p>
          {edit.title.conflict || edit.body.conflict ? <div className="notes-conflict" role="alert">
            <p><UiIcon name="info" /><span>This decision changed elsewhere. Current saved version:</span></p>
            <strong>{record.summary.title}</strong>
            <MarkdownPreview content={record.body} />
            <button type="button" disabled={model.busy || unknown} onClick={() => void saveEdit(true)}>Keep mine and save</button>
            <button type="button" onClick={() => model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: { title: changedDraft(record.summary.title, record.summary.title, record.summary.revision), body: changedDraft(record.body, record.body, record.summary.revision) } } }))}>Reload (discard my edits)</button>
          </div> : null}
          <MarkdownEditor label="Edit decision Markdown" draftKey={`decision:${record.summary.decision_id}`} value={edit.body.value} onChange={value => model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: { ...edit, body: { ...edit.body, value } } } }))} onSave={() => void saveEdit()} />
        </div> : <div className="notes-decision-body"><MarkdownPreview content={record.body} /></div>}
      </> : <div className="notes-empty notes-decision-empty"><span className="notes-empty-icon"><UiIcon name="file" /></span><p className="notes-empty-title">Select a decision</p></div>}
    </section>
  </div>;
}
