import { UiIcon } from "../UiIcon";

export function TodosSourceButton({ folder }: { folder: string }) {
  return <details><summary aria-label="Markdown source todos.md" title="Markdown source"><UiIcon name="code" /><span className="notes-source-label">todos.md</span></summary><div className="notes-source-popover"><p>Todos and board share this ordinary file. Open it in your editor; Cockpit preserves headings, prose and task boundaries.</p><input aria-label="Todos Markdown file" readOnly value={`${folder}/todos.md`} onClick={event => event.currentTarget.select()} /></div></details>;
}
