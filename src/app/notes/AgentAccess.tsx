import { useState } from "react";
import { UiIcon } from "../UiIcon";
import type { NotesTargetInfo, NotesTodo } from "../../protocol/generated/v1";
import type { NotesModel } from "./useNotes";

export function AgentAccess({ info, model, todo, decisionId }: { info: NotesTargetInfo; model: NotesModel; todo: NotesTodo | null; decisionId: string | null }) {
  const [copyStatus, setCopyStatus] = useState("");
  const copyCommand = async (label: string, value: string) => {
    setCopyStatus("");
    try {
      await navigator.clipboard.writeText(value);
      setCopyStatus(`Copied ${label}.`);
    } catch {
      setCopyStatus(`Could not copy ${label}. Select the command and copy it manually.`);
    }
  };
  // The producer folder is <root>/<UUID>; retain its separator for filesystem roots.
  const root = info.folder.slice(0, -info.notes_id.length);
  const command = `COCKPIT_NOTES_ROOT='${root.replaceAll("'", "'\"'\"'")}' cockpit-cli notes`;
  const base = `${command} --notes ${info.notes_id}`;
  const selector = todo ? todo.id && !todo.problems.includes("duplicate_id") ? `--id ${todo.id} --expected-revision '${todo.revision}'` : `--ref '${todo.ref}'` : "--id <todo-id> --expected-revision '<item-revision>'";
  const decision = model.decisions.find(item => item.decision_id === decisionId);
  const decisionSelector = decision ? `--id ${decision.decision_id} --expected-revision '${decision.revision}'` : "--id <decision-id> --expected-revision '<record-revision>'";
  const commands = [
    ["Resolve once, then pin the Notes UUID", `${command} --current target`],
    ["Scratchpad read", `${base} scratchpad read`],
    ["Scratchpad append", `${base} scratchpad append --text 'Research notes'`],
    ["Scratchpad replace (stdin)", `${base} scratchpad replace --stdin --expected-revision '${model.scratchpad?.revision ?? "<file-revision>"}'`],
    ["Todos read", `${base} todo list --open`],
    ["Todos add", `${base} todo add --text 'Review API'`],
    ["Todos edit", `${base} todo update ${selector} --text 'Review API contract'`],
    ["Todos check / reopen", `${base} todo ${todo?.done ? "reopen" : "complete"} ${selector}`],
    ["Board read", `${base} kanban list`],
    ["Board add", `${base} kanban add --text 'Review schema'`],
    ["Board promote", `${base} kanban promote ${selector}`],
    ["Board move", `${base} kanban move ${selector} --to doing`],
    ["Board remove (retains todo and comments)", `${base} kanban unboard ${selector}`],
    ["Decisions search title and body", `${base} decision list --status current --query 'schema'`],
    ["Decisions create (Markdown stdin)", `${base} decision create --title 'Keep Markdown' --stdin`],
    ["Decisions read", `${base} decision get --id ${decision?.decision_id ?? "<decision-id>"}`],
    ["Decisions edit", `${base} decision update ${decisionSelector} --title 'Keep ordinary Markdown' --stdin`],
    ["Decisions replace (old file unchanged)", `${base} decision replace ${decisionSelector} --title 'New approach' --stdin`],
    ["Comments read", `${base} comment list --todo ${todo?.id ?? "<todo-id>"}`],
    ["Comments post", `${base} comment add --todo ${todo?.id ?? "<todo-id>"} --text 'Blocked on API' --author 'Agent'`],
    ["Comments read one", `${base} comment get --todo ${todo?.id ?? "<todo-id>"} --comment <comment-id>`],
    ["Comments edit", `${base} comment update --todo ${todo?.id ?? "<todo-id>"} --comment <comment-id> --expected-revision '<comment-revision>' --stdin`],
    ["Comments remove", `${base} comment remove --todo ${todo?.id ?? "<todo-id>"} --comment <comment-id> --expected-revision '<comment-revision>'`],
  ];
  return <details className="notes-agent-access" id="agentAccessDetails">
    <summary className="notes-agent-summary">
      <UiIcon name="terminal" />
      <span>Agent access</span>
      <span className="notes-agent-summary-meta">cockpit-cli</span>
      <span className="notes-disclosure-chevron"><UiIcon name="down" /></span>
    </summary>
    <div className="notes-agent-body notes-agent-overlay">
      <div className="notes-agent-scroll">
        <p>Real commands use the same Markdown files and revision checks as these controls. Resolve once and pin this Notes ID; a Space change will not retarget it. Replace placeholders with the IDs/revisions returned by reads. Never retry an unconfirmed write automatically.</p>
        <label>Notes UUID<input readOnly value={info.notes_id} onClick={event => event.currentTarget.select()} /></label>
        <label>Folder outside the repository<input readOnly value={info.folder} onClick={event => event.currentTarget.select()} /></label>
        <div className="notes-agent-commands">
          {commands.map(([label, command]) => <div className="notes-agent-command" key={label}>
            <label><span>{label}</span><input readOnly value={command} onClick={event => event.currentTarget.select()} /></label>
            <button type="button" className="notes-agent-copy" aria-label={`Copy ${label}`} title={`Copy ${label}`} onClick={() => void copyCommand(label, command)}><UiIcon name="copy" /></button>
          </div>)}
        </div>
        <p>All four surfaces plus comments support real read/write. Additional verbs: todo remove, catalog, target --create / --attach. Use cockpit-cli notes --help for flags. Imported todos use --ref until an explicit edit adopts a stable ID.</p>
      </div>
      <div className="notes-agent-status" role="status" aria-atomic="true">{copyStatus}</div>
    </div>
  </details>;
}
