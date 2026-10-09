import { useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { trapModalTab, useModalFocus } from "../input/modal";
import type { Space } from "../sidebar/spaceTree";
import type { Mutate, Pane, Tab } from "./model";

export type PaneDialog =
  | { kind: "rename"; paneId: string }
  | { kind: "swap"; paneId: string }
  | { kind: "move"; paneId: string };

export function moveDestinationLabel(tab: Tab, spaces: Space[]): string {
  const space = spaces.find((candidate) => candidate.id === tab.space_id);
  return `${space?.label ?? `Space ${space?.number ?? "?"}`} / ${tab.label || `Tab ${tab.number}`}`;
}

export function PaneDialogOverlay({ dialog, panes, tabs, spaces, busy, onDismiss, mutate, leafChoices, onSwap, confirmMove }: { dialog: PaneDialog; panes: Pane[]; tabs: Tab[]; spaces: Space[]; busy: boolean; onDismiss: () => void; mutate: Mutate; leafChoices: { id: string; title: string }[]; onSwap(a: string, b: string): void; confirmMove(pane: Pane): boolean }) {
  const pane = panes.find((candidate) => candidate.id === dialog.paneId);
  const [value, setValue] = useState(dialog.kind === "rename" ? pane?.title ?? "" : "");
  const ref = useModalFocus<HTMLFormElement>(onDismiss);
  if (!pane && dialog.kind !== "swap") return null;
  const submit = () => {
    const key = `pane:${dialog.paneId}`;
    let accepted = false;
    if (dialog.kind === "rename" && pane) accepted = mutate(key, { type: "pane_rename", pane_id: pane.id, label: value.trim() || null });
    if (dialog.kind === "swap" && value) { onSwap(dialog.paneId, value); accepted = true; }
    if (dialog.kind === "move" && pane && value && confirmMove(pane)) {
      if (value === "new-tab") accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "new_tab", space_id: pane.space_id, label: null } }, true);
      if (value === "new-space") accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "new_space", label: null, tab_label: null } }, true);
      if (value.startsWith("tab:")) accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "existing_tab", tab_id: value.slice(4), direction: "right", target_pane_id: null, ratio: null } }, true);
    }
    if (accepted) onDismiss();
  };
  const onChooserKeyDown = (event: ReactKeyboardEvent<HTMLFormElement>) => {
    if (event.target instanceof HTMLSelectElement && event.ctrlKey && !event.altKey && !event.metaKey
      && (event.key.toLowerCase() === "n" || event.key.toLowerCase() === "p")) {
      const choices = [...event.target.options].filter((option) => option.value !== "");
      if (choices.length > 0) {
        event.preventDefault();
        const current = choices.findIndex((option) => option.value === value);
        const direction = event.key.toLowerCase() === "n" ? 1 : -1;
        const next = current < 0 ? (direction > 0 ? 0 : choices.length - 1) : (current + direction + choices.length) % choices.length;
        setValue(choices[next].value);
      }
      return;
    }
    trapModalTab(event, ref.current);
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay" role="dialog" aria-modal="true" aria-labelledby="chooser-title" onSubmit={(event) => { event.preventDefault(); submit(); }} onKeyDown={onChooserKeyDown}>
    <h2 id="chooser-title">{dialog.kind} pane</h2>
    {dialog.kind === "rename" ? <input aria-label="Pane name" value={value} onChange={(event) => setValue(event.target.value)} /> : <select aria-label={dialog.kind === "swap" ? "Swap target" : "Move destination"} value={value} onChange={(event) => setValue(event.target.value)}><option value="">Choose...</option>{dialog.kind === "swap" ? leafChoices.filter(candidate => candidate.id !== dialog.paneId).map(candidate => <option key={candidate.id} value={candidate.id}>{candidate.title}</option>) : <><option value="new-tab">New tab in this space</option><option value="new-space">New space</option>{tabs.filter(tab => tab.id !== pane?.tab_id).map(tab => <option key={tab.id} value={`tab:${tab.id}`}>{moveDestinationLabel(tab, spaces)}</option>)}</>}</select>}
    <footer><button type="button" onClick={onDismiss}>Cancel</button><button type="submit" disabled={busy || (dialog.kind !== "rename" && !value)}>{dialog.kind}</button></footer>
  </form></div>;
}
