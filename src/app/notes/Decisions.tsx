import { useEffect, useRef, useState } from "react";
import type { NotesDecision, NotesDecisionSummary } from "../../protocol/generated/v1";
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
  return <div className={`notes-decisions${showList ? " is-list" : ""}`}>
    <section className="notes-decision-collection"><div className="notes-decision-controls"><input id="decisionSearch" aria-label="Search decision title and body" placeholder="Search title and body…" value={query} onChange={event => setQuery(event.target.value)} /><button type="button" id="newDecisionBtn" onClick={beginNew}>{newDraft ? "Resume draft" : "New decision"}</button><label>Status <select aria-label="Decision status" value={filter} onChange={event => setFilter(event.target.value === "history" ? "history" : "current")}><option value="current">Current</option><option value="history">History</option></select></label><button type="button" id="sortToggle" aria-label="Decision sort order" onClick={() => setOldest(!oldest)}>{oldest ? "Oldest" : "Newest"}</button></div>
      <div id="decisionList" className="notes-decision-list" aria-busy={loading}>{ordered.map(item => <button type="button" key={item.decision_id} className={`notes-decision-row${selectedId === item.decision_id && !showNew ? " is-selected" : ""}`} onClick={() => { onSelect(item.decision_id); setShowNew(false); setEditing(false); setShowList(false); }}><strong>{item.title}</strong><small>{item.recorded && !Number.isNaN(Date.parse(item.recorded)) ? new Date(item.recorded).toLocaleDateString() : "Recorded: unknown"}{item.status === "replaced" ? " · Replaced" : ""}</small></button>)}{!loading && !records.length ? <div className="notes-empty"><p>{query ? `No ${filter} decisions match “${query}”.` : `No ${filter} decisions yet.`}</p>{query ? <button type="button" onClick={() => setQuery("")}>Clear search</button> : <button type="button" onClick={beginNew}>New decision</button>}{otherMatches ? <p>{otherMatches} matching {filter === "current" ? "History" : "Current"} {otherMatches === 1 ? "record" : "records"}. <button type="button" onClick={() => setFilter(filter === "current" ? "history" : "current")}>Show {filter === "current" ? "History" : "Current"}</button></p> : null}</div> : null}</div>
    </section>
    <section className="notes-decision-detail"><button type="button" className="notes-decision-back" id="decisionBackBtn" onClick={() => setShowList(true)}>Back to decisions</button>{error ? <p role="alert">{error}</p> : null}
      {showNew && newDraft?.replaces && replacementBlocked ? <div className="notes-conflict" role="alert"><p>{!replacementSource ? "The source decision is no longer present. Your replacement draft is kept." : replacementSource.status === "replaced" ? "This decision has already been replaced. Your draft is kept; review its successor rather than recording another replacement." : `The source decision changed elsewhere: ${replacementSource.title}. Review it before replacing this version.`}</p>{replacementSource ? <button type="button" onClick={() => { onSelect(replacementSource.decision_id); setShowNew(false); setEditing(false); }}>Review source decision</button> : null}{replacementSource?.status === "current" ? <button type="button" onClick={() => model.updateDrafts(current => current.newDecision ? { ...current, newDecision: { ...current.newDecision, revision: replacementSource.revision } } : current)}>Use current source revision</button> : null}</div> : null}
      {showNew && newDraft ? <div className="notes-decision-write"><h3>{newDraft.replaces ? "Replacement decision" : "New decision"}</h3>{newDraft.replaces ? <p>Replaces {newDraft.replaces}. The old Markdown file stays untouched.</p> : <p>Recorded date is set only when this record is saved.</p>}<label>Title<input id="newTitleInput" aria-label="New decision title" value={newDraft.title} maxLength={512} onChange={event => model.updateDrafts(current => ({ ...current, newDecision: { ...newDraft, title: event.target.value } }))} /></label><label>Decided date (optional)<input aria-label="Decision historical date" placeholder="YYYY-MM-DD or RFC3339" value={newDraft.decided} onChange={event => model.updateDrafts(current => ({ ...current, newDecision: { ...newDraft, decided: event.target.value } }))} /></label><MarkdownEditor label="New decision Markdown" value={newDraft.body} onChange={body => model.updateDrafts(current => ({ ...current, newDecision: { ...newDraft, body } }))} onSave={() => void saveNew()} /><div className="notes-row-actions"><small>Draft kept</small><button type="button" id="cancelDraftBtn" disabled={model.busy} onClick={() => { model.updateDrafts(current => { const next = { ...current }; delete next.newDecision; return next; }); setShowNew(false); }}>Cancel (discard draft)</button><button type="button" id="recordDecisionBtn" disabled={model.busy || !newDraft.title.trim() || unknown || replacementBlocked} onClick={() => void saveNew()}>{model.busy ? "Recording…" : "Record decision"}</button></div></div> : record ? <>
        <header className="notes-decision-heading"><h3>{record.summary.title}</h3><span>{record.summary.recorded && !Number.isNaN(Date.parse(record.summary.recorded)) ? `Recorded ${new Date(record.summary.recorded).toLocaleString()}` : "Recorded: unknown"}</span><button type="button" onClick={() => { model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: current.decisionEdits[record.summary.decision_id] ?? { title: changedDraft(record.summary.title, record.summary.title, record.summary.revision), body: changedDraft(record.body, record.body, record.summary.revision) } } })); setEditing(true); }}>{dirty ? "Resume edit · draft kept" : "Edit"}</button></header>
        {editing && edit ? <div className="notes-decision-write"><label>Title<input id="editTitleInput" aria-label="Edit decision title" value={edit.title.value} onChange={event => model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: { ...edit, title: { ...edit.title, value: event.target.value } } } }))} /></label><MarkdownEditor label="Edit decision Markdown" draftKey={`decision:${record.summary.decision_id}`} value={edit.body.value} onChange={value => model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: { ...edit, body: { ...edit.body, value } } } }))} onSave={() => void saveEdit()} />{edit.title.conflict || edit.body.conflict ? <div className="notes-conflict" role="alert"><p>This decision changed elsewhere. Current saved version:</p><strong>{record.summary.title}</strong><MarkdownPreview content={record.body} /><button type="button" disabled={model.busy || unknown} onClick={() => void saveEdit(true)}>Keep mine and save</button><button type="button" onClick={() => model.updateDrafts(current => ({ ...current, decisionEdits: { ...current.decisionEdits, [record.summary.decision_id]: { title: changedDraft(record.summary.title, record.summary.title, record.summary.revision), body: changedDraft(record.body, record.body, record.summary.revision) } } }))}>Reload (discard my edits)</button></div> : <button type="button" id="saveEditsBtn" disabled={!dirty || model.busy || unknown} onClick={() => void saveEdit()}>Save edits</button>}<button type="button" onClick={() => setEditing(false)}>Back / keep edit</button><button type="button" id="discardEditsBtn" onClick={() => { model.updateDrafts(current => { const edits = { ...current.decisionEdits }; delete edits[record.summary.decision_id]; return { ...current, decisionEdits: edits }; }); setEditing(false); }}>Discard edits</button></div> : <div className="notes-decision-body"><MarkdownPreview content={record.body} /></div>}
        <details className="notes-decision-more"><summary>More · date, reference, replacement</summary><div><span>Recorded (immutable): {record.summary.recorded ?? "unknown"}</span><span>Decided: {record.summary.decided ?? "unknown"}</span><label>Reference<input id="moreReference" readOnly value={record.relative_path} onClick={event => event.currentTarget.select()} /></label><label>File<input readOnly value={record.path} onClick={event => event.currentTarget.select()} /></label>{record.summary.replaces ? <button type="button" onClick={() => { onSelect(record.summary.replaces); setShowNew(false); setEditing(false); }}>Replaces {record.summary.replaces}{model.decisions.some(item => item.decision_id === record.summary.replaces) ? "" : " (missing record)"}</button> : null}{record.summary.replaced_by.map(id => <button type="button" key={id} onClick={() => { onSelect(id); setShowNew(false); setEditing(false); }}>Replaced by {model.decisions.find(item => item.decision_id === id)?.title ?? id}</button>)}{record.summary.problems.length ? <p>Source issues: {record.summary.problems.join(", ")}</p> : null}<button type="button" id="replaceBtn" disabled={record.summary.status !== "current" || model.busy || Boolean(newDraft)} onClick={() => { model.updateDrafts(current => current.newDecision ? current : { ...current, newDecision: { title: record.summary.title, body: "", decided: "", replaces: record.summary.decision_id, revision: record.summary.revision } }); setShowNew(true); setEditing(false); }}>Replace with a new decision</button>{record.summary.status !== "current" ? <small>Already replaced. Follow the successor instead.</small> : newDraft ? <small>Finish or cancel your existing draft before replacing.</small> : null}</div></details>
      </> : <p className="notes-empty">Select a decision or start a new one.</p>}
    </section>
  </div>;
}
