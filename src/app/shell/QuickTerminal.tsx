import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { TerminalPane } from "../TerminalPane";
import { chordMatches, formatShortcut, shortcutEntry } from "../input/shortcuts";
import { UiIcon } from "../UiIcon";

type Props = {
  client: CockpitClient; sessionId: string; paneId: string; focusEpoch: number;
  returnFocus: HTMLElement | null; suspended: boolean; cwd?: string | null; onDismiss(): void;
};

/** A throwaway shell floating over the work area. Closing ends the shell immediately. */
export function QuickTerminal({ client, sessionId, paneId, focusEpoch, returnFocus, suspended, cwd, onDismiss }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const terminate = useRef<(() => boolean) | null>(null);
  const suspendedRef = useRef(suspended);
  suspendedRef.current = suspended;
  const [rect, setRect] = useState<DOMRect | null>(null);
  useLayoutEffect(() => {
    const target = document.querySelector<HTMLElement>(".workarea-content");
    if (!target) return;
    const update = () => setRect(target.getBoundingClientRect());
    update();
    const observer = new ResizeObserver(update);
    observer.observe(target);
    window.addEventListener("resize", update);
    return () => { observer.disconnect(); window.removeEventListener("resize", update); };
  }, []);
  useEffect(() => {
    container.current?.focus();
    const focus = (event: FocusEvent) => {
      if (!suspendedRef.current && container.current && event.target instanceof Node && !container.current.contains(event.target)) container.current.querySelector<HTMLElement>(".xterm-helper-textarea, button")?.focus();
    };
    document.addEventListener("focusin", focus);
    return () => { document.removeEventListener("focusin", focus); queueMicrotask(() => { if (returnFocus?.isConnected) returnFocus.focus(); }); };
  }, [returnFocus]);
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
  // Horizontally centred on the window (the sidebar must not shift it); vertically placed in the work area.
  const width = Math.min(window.innerWidth - 16, Math.max(320, Math.min(1040, window.innerWidth * 0.8)));
  const height = rect ? Math.min(rect.height - 16, Math.max(200, Math.min(680, rect.height * 0.78))) : 0;
  return <div className="quick-terminal-backdrop" inert={suspended}>
    <div ref={container} tabIndex={-1} className="quick-terminal" role="dialog" aria-modal="true" aria-label="Quick terminal" onKeyDownCapture={keyDown}
      style={rect ? { left: (window.innerWidth - width) / 2, top: rect.top + Math.min(48, (rect.height - height) / 2), width, height } : undefined}>
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
