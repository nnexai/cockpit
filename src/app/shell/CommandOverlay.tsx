import { useEffect, useRef, useState, type ReactNode } from "react";
import { UiIcon, type UiIconName } from "../UiIcon";
import { SHORTCUT_SEPARATOR } from "../input/shortcuts";
import { trapModalTab, useModalFocus } from "../input/modal";
import { rankFuzzyMatches } from "../input/fileNavigation";
import type { CommandAction } from "./commands";

export const commandGroupIcons: Record<CommandAction["group"], UiIconName> = { Herdr: "terminal", Navigate: "forward", Space: "grid", Tab: "browser", Pane: "terminal", Browser: "browser", Library: "library" };

export function commandIcon(action: CommandAction): UiIconName {
  if (action.icon) return action.icon;
  if (action.id.endsWith("-left") || action.id.endsWith(":previous-tab")) return "back";
  if (action.id.endsWith("-right") || action.id.endsWith(":next-tab")) return "forward";
  if (action.id.endsWith("-up")) return "up";
  if (action.id.endsWith("-down")) return "down";
  if (action.id.includes("rename")) return "edit";
  if (action.id.includes("close")) return "close";
  if (action.id.includes("new-") || action.id.endsWith(":add")) return "plus";
  if (action.id.includes("cleanup") || action.id.includes("refresh")) return "refresh";
  if (action.id.endsWith(":zoom-pane")) return "expand";
  if (action.id.endsWith(":open-file-picker")) return "search";
  if (action.id.endsWith(":toggle-sidebar")) return "sidebar";
  return commandGroupIcons[action.group];
}

/** A shortcut as `kbd` chips: `Ctrl+B` `i` for a sequence, one chip for a chord, alternatives separated by "or". */
export function ShortcutKeys({ text }: { text: string }) {
  return <span className="command-keys">{text.split(SHORTCUT_SEPARATOR).map((form, index) => <span className="command-key-form" key={form}>
    {index > 0 ? <span className="command-key-or">or</span> : null}
    {(form.startsWith("Ctrl+B ") ? ["Ctrl+B", form.slice("Ctrl+B ".length)] : [form]).map((chip) => <kbd key={chip}>{chip}</kbd>)}
  </span>)}</span>;
}

export function CommandOverlay({ actions, statusContent, onSwitchSession, onDismiss }: { actions: CommandAction[]; statusContent?: ReactNode; onSwitchSession: () => void; onDismiss: () => void }) {
  const ref = useModalFocus<HTMLElement>(onDismiss);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const activeRowRef = useRef<HTMLButtonElement | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [showAll, setShowAll] = useState(false);
  const normalized = query.trim().toLocaleLowerCase();
  const ranked = normalized
    ? rankFuzzyMatches(query, actions, (action) => `${action.label} ${action.shortcut ?? ""} ${action.group} ${action.reasonDetail ?? ""}`)
    : actions.map((action, index) => ({ ...action, score: index, matchedIndices: [] as number[] }));
  const groups = ["Herdr", "Navigate", "Space", "Tab", "Pane", "Browser", "Library"] as const;
  // Rows render grouped, so keep `filtered` in that order for the highlight and arrow keys. With a query, groups follow their best match.
  const groupOrder: readonly CommandAction["group"][] = normalized ? [...new Set(ranked.map((action) => action.group))] : groups;
  const filtered = groupOrder.flatMap((group) => ranked.filter((action) => action.group === group && (normalized || showAll || group === "Herdr" || action.primary)));
  useEffect(() => setActive((current) => Math.min(current, Math.max(0, filtered.length - 1))), [filtered.length]);
  useEffect(() => { searchRef.current?.focus(); }, []);
  useEffect(() => { activeRowRef.current?.scrollIntoView?.({ block: "nearest" }); }, [active, normalized]);
  const runActive = () => {
    const action = filtered[active];
    if (action && !action.disabled) action.run();
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><section ref={ref} className="command-overlay" role="dialog" aria-modal="true" aria-labelledby="commands-title" onKeyDown={(event) => {
    const listStep = event.key === "ArrowDown" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "n") ? 1 : event.key === "ArrowUp" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "p") ? -1 : 0;
    if (listStep !== 0) { event.preventDefault(); setActive((current) => filtered.length === 0 ? 0 : (current + (listStep === 1 ? 1 : filtered.length - 1)) % filtered.length); return; }
    if (event.key === "Enter" && document.activeElement instanceof HTMLInputElement) { event.preventDefault(); runActive(); return; }
    trapModalTab(event, ref.current);
  }}><header><h2 id="commands-title">Commands</h2><button type="button" onClick={onDismiss} aria-label="Close commands"><UiIcon name="close" /></button></header><div className="command-search-box"><UiIcon name="search" /><input ref={searchRef} className="command-search" aria-label="Find a command" placeholder="Find a command…" autoComplete="off" value={query} onChange={(event) => { setQuery(event.target.value); setActive(0); }} /></div>{statusContent ? <div className="command-status">{statusContent}</div> : null}<div className="command-list" role="listbox" aria-label="Available commands">{filtered.length === 0 ? <p className="command-empty">No matching commands.</p> : groupOrder.map((group) => {
    const groupActions = filtered.filter((action) => action.group === group);
    if (groupActions.length === 0) return null;
    return <section className="command-group" key={group}><h3>{group}</h3>{groupActions.map((action) => {
      const index = filtered.indexOf(action);
      return <button ref={index === active ? activeRowRef : null} type="button" role="option" aria-selected={index === active} className={`command-row${index === active ? " is-active" : ""}`} key={action.id} disabled={action.disabled} onMouseMove={() => { if (index !== active) setActive(index); }} onClick={() => action.run()}><UiIcon name={commandIcon(action)} /><span className="command-row-label"><span>{Array.from(action.label, (character, characterIndex) => action.matchedIndices.includes(characterIndex) ? <mark key={characterIndex}>{character}</mark> : character)}</span>{action.reason ? <small title={action.reasonDetail ?? action.reason}>{action.reason}</small> : null}</span>{action.shortcut ? <ShortcutKeys text={action.shortcut} /> : null}</button>;
    })}</section>;
  })}</div><footer className="command-footer"><span>↑↓ or Ctrl+N/P navigate · Enter choose · Esc close · type a name or a key</span><button type="button" onClick={() => { setShowAll((value) => !value); setActive(0); }}>{showAll ? "Quick commands" : "All commands"}</button></footer></section></div>;
}
