import { useState } from "react";

/** Replaces a label with an input. Enter commits, Escape or blur cancels. */
export function InlineRename({ label, ariaLabel, onCommit, onCancel }: { label: string; ariaLabel: string; onCommit: (label: string) => boolean; onCancel: () => void }) {
  const [value, setValue] = useState(label);
  return <input className="inline-rename" aria-label={ariaLabel} autoFocus value={value} onChange={(event) => setValue(event.target.value)} onBlur={onCancel} onKeyDown={(event) => {
    if (event.key === "Escape") { event.preventDefault(); onCancel(); }
    if (event.key === "Enter") { event.preventDefault(); const next = value.trim(); if (!next) onCancel(); else onCommit(next); }
  }} />;
}
