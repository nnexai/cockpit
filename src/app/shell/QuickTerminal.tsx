import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { TerminalPane } from "../TerminalPane";
import { chordMatches, formatShortcut, shortcutEntry } from "../input/shortcuts";
import { UiIcon } from "../UiIcon";

type Props = {
  client: CockpitClient; sessionId: string; paneId: string; focusEpoch: number;
  returnFocus: HTMLElement | null; returnSelection?: { start: number; end: number; direction: "forward" | "backward" | "none" } | null; suspended: boolean; cwd?: string | null; onDismiss(): void;
};

/** A throwaway shell floating over the work area. Closing ends the shell immediately. */
export function QuickTerminal({ client, sessionId, paneId, focusEpoch, returnFocus, returnSelection, suspended, cwd, onDismiss }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const terminate = useRef<(() => boolean) | null>(null);
  const suspendedRef = useRef(suspended);
  suspendedRef.current = suspended;
  const [viewport, setViewport] = useState(() => ({ left: 0, top: 0, width: window.innerWidth, height: window.innerHeight }));
  useLayoutEffect(() => {
    const visual = window.visualViewport;
    const update = () => setViewport(visual
      ? { left: visual.offsetLeft, top: visual.offsetTop, width: visual.width, height: visual.height }
      : { left: 0, top: 0, width: window.innerWidth, height: window.innerHeight });
    update();
    window.addEventListener("resize", update);
    visual?.addEventListener("resize", update);
    visual?.addEventListener("scroll", update);
    return () => { window.removeEventListener("resize", update); visual?.removeEventListener("resize", update); visual?.removeEventListener("scroll", update); };
  }, []);
  useEffect(() => {
    container.current?.focus();
    const focus = (event: FocusEvent) => {
      if (!suspendedRef.current && container.current && event.target instanceof Node && !container.current.contains(event.target)) container.current.querySelector<HTMLElement>(".xterm-helper-textarea, button")?.focus();
    };
    document.addEventListener("focusin", focus);
    return () => { document.removeEventListener("focusin", focus); queueMicrotask(() => {
      if (!returnFocus?.isConnected) return;
      returnFocus.focus({ preventScroll: true });
      if (returnSelection && (returnFocus instanceof HTMLTextAreaElement || returnFocus instanceof HTMLInputElement) && returnFocus.selectionStart !== null) {
        returnFocus.setSelectionRange(returnSelection.start, returnSelection.end, returnSelection.direction);
      }
    }); };
  }, [returnFocus, returnSelection]);
  // Unmounting releases the stream, which also ends the shell; terminating first just makes it prompt.
  const close = () => { terminate.current?.(); onDismiss(); };
  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (shortcutEntry("quick-terminal").chords!.some(chord => chordMatches(chord, event))) {
      event.preventDefault();
      event.stopPropagation();
      if (!event.repeat) close();
      return;
    }
    if (event.key === "Tab" && event.target instanceof HTMLButtonElement && !event.ctrlKey && !event.altKey && !event.metaKey) {
      event.preventDefault();
      container.current?.querySelector<HTMLElement>(".xterm-helper-textarea")?.focus();
    }
  };
  const shortcut = formatShortcut("quick-terminal");
  // Use the visible viewport, including when a mobile keyboard changes its bounds.
  const width = Math.max(0, Math.min(viewport.width - 16, viewport.width * 0.92, 1440));
  const height = Math.max(0, Math.min(viewport.height - 16, viewport.height * 0.86, 960));
  return <div className="quick-terminal-backdrop" inert={suspended}>
    <div ref={container} tabIndex={-1} className="quick-terminal" role="dialog" aria-modal="true" aria-label="Quick terminal" onKeyDownCapture={keyDown}
      style={{ left: viewport.left + (viewport.width - width) / 2, top: viewport.top + (viewport.height - height) / 2, width, height }}>
      <header className="quick-terminal-header">
        <UiIcon name="terminal" />
        <span className="quick-terminal-cwd" title={cwd ?? undefined}><bdi>{cwd ?? "Quick terminal"}</bdi></span>
        {shortcut ? <kbd className="quick-terminal-key" aria-hidden="true">{shortcut}</kbd> : null}
        <button type="button" className="quick-terminal-close" aria-label="Close quick terminal" title={shortcut ? `Close (${shortcut})` : "Close"} onClick={close}><UiIcon name="close" /></button>
      </header>
      <div className="quick-terminal-body"><TerminalPane client={client} request={{ session_id: sessionId, pane_id: paneId, target_kind: "scratch" }} selected presented controlAllowed controlPending={false} focusEpoch={focusEpoch} focusToken={1} terminalMouseInput terminateRef={terminate} accessibleLabel="Quick terminal shell" onClosed={onDismiss} /></div>
    </div>
  </div>;
}
