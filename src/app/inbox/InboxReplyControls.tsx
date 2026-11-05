import { useEffect, useRef, type KeyboardEvent } from "react";
import { UiIcon, type UiIconName } from "../UiIcon";

export type ReplyTarget = { key: string; label: string; icon: UiIconName; checked: boolean; hasDraft: boolean };

/** `Reply to`: one radio per open question plus Follow up. Moving with the arrow keys selects but never sends or enters the field. */
export function ReplyTargets({ targets, disabled, onPick }: { targets: ReplyTarget[]; disabled: boolean; onPick(key: string, pointer: boolean): void }) {
  const anyChecked = targets.some(target => target.checked);
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const forward = event.key === "ArrowRight" || event.key === "ArrowDown", backward = event.key === "ArrowLeft" || event.key === "ArrowUp";
    if (!forward && !backward) return;
    const radios = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]'));
    const from = radios.indexOf(event.target as HTMLButtonElement);
    if (from < 0 || disabled) return;
    event.preventDefault();
    const next = (from + (forward ? 1 : radios.length - 1)) % radios.length;
    radios[next]!.focus();
    onPick(targets[next]!.key, false);
  };
  return <div className="inbox-targets" role="radiogroup" aria-label="Reply to" onKeyDown={move}>
    {targets.map((target, index) => <button key={target.key || "follow-up"} type="button" role="radio" className="inbox-chip" aria-checked={target.checked} tabIndex={target.checked || (!anyChecked && index === 0) ? 0 : -1} disabled={disabled} onClick={event => onPick(target.key, event.detail > 0)}>
      <UiIcon name={target.icon} /><span>{target.label}</span>{target.hasDraft && <><i className="inbox-chip-dot" aria-hidden="true" /><span className="inbox-sr"> has a draft</span></>}
    </button>)}
  </div>;
}

/** Shown when a choice would replace different typed text. Focus lands on Replace; Escape keeps the typed text. */
export function ReplaceChoice({ choice, onReplace, onAdd, onKeep }: { choice: string; onReplace(): void; onAdd(): void; onKeep(): void }) {
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => { first.current?.focus(); }, []);
  return <div className="inbox-note inbox-replace" role="alertdialog" aria-label="Replace draft?" onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onKeep(); } }}>
    <UiIcon name="edit" />
    <div>
      <p>Replace your draft with “{choice}”?</p>
      <div className="inbox-replace-acts">
        <button ref={first} type="button" className="inbox-btn" onClick={onReplace}>Replace</button>
        <button type="button" className="inbox-btn" onClick={onAdd}>Add below</button>
        <button type="button" className="inbox-btn" onClick={onKeep}>Keep mine</button>
      </div>
    </div>
  </div>;
}
