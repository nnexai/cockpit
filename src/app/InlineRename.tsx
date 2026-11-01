import { useRef, useState } from "react";

/** Replaces a label with an input. Enter commits, Escape cancels. */
export function InlineRename({ label, ariaLabel, onCommit, onCancel, commitOnBlur = false }: { label: string; ariaLabel: string; onCommit: (label: string) => boolean; onCancel: () => void; commitOnBlur?: boolean }) {
  const [value, setValue] = useState(label);
  const finished = useRef(false);
  const cancel = () => {
    if (finished.current) return;
    finished.current = true;
    onCancel();
  };
  const commit = () => {
    if (finished.current) return;
    const next = value.trim();
    if (!next) { cancel(); return; }
    finished.current = true;
    if (!onCommit(next)) finished.current = false;
  };
  return <input className="inline-rename" aria-label={ariaLabel} autoFocus value={value} onChange={(event) => setValue(event.target.value)} onBlur={commitOnBlur ? commit : cancel} onKeyDown={(event) => {
    if (event.key === "Escape") { event.preventDefault(); cancel(); }
    if (event.key === "Enter") { event.preventDefault(); commit(); }
  }} />;
}
