import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { TerminalPane } from "../TerminalPane";
import { herdrShadowsPrefix } from "../input/herdrBindings";

type Props = {
  client: CockpitClient; sessionId: string; paneId: string; focusEpoch: number;
  returnFocus: HTMLElement | null; suspended: boolean; onDismiss(): void;
};

export function QuickTerminal({ client, sessionId, paneId, focusEpoch, returnFocus, suspended, onDismiss }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const terminate = useRef<(() => boolean) | null>(null);
  const closePending = useRef(false);
  const prefix = useRef(false);
  const suspendedRef = useRef(suspended);
  suspendedRef.current = suspended;
  const [rect, setRect] = useState<DOMRect | null>(null);
  const [closeState, setCloseState] = useState<"idle" | "pending" | "unconfirmed">("idle");
  const onTerminalStateChange = useCallback((state: { ready: boolean; error: string | null }) => {
    if (state.error && closePending.current) {
      closePending.current = false;
      setCloseState("unconfirmed");
    }
  }, []);
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
  const close = () => {
    if (closePending.current) return;
    if (closeState === "unconfirmed") {
      if (window.confirm("The terminal's closure could not be confirmed. Dismiss it anyway? The shell or detached processes may still be running.")) onDismiss();
      return;
    }
    if (!window.confirm("Close the quick terminal? This will terminate its shell. Detached processes may continue running.")) return;
    closePending.current = true;
    setCloseState("pending");
    if (!terminate.current?.()) {
      closePending.current = false;
      setCloseState("unconfirmed");
    }
  };
  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Tab" && event.target instanceof HTMLButtonElement && !event.ctrlKey && !event.altKey && !event.metaKey) {
      const elements = Array.from(container.current?.querySelectorAll<HTMLElement>("button:not(:disabled), .xterm-helper-textarea") ?? []);
      if (elements.length) {
        event.preventDefault();
        const index = elements.indexOf(document.activeElement as HTMLElement);
        elements[(index + (event.shiftKey ? -1 : 1) + elements.length) % elements.length]?.focus();
      }
      return;
    }
    if (!herdrShadowsPrefix("u", false) && event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey && event.key.toLowerCase() === "b") {
      if (prefix.current) { prefix.current = false; return; }
      prefix.current = true; event.preventDefault(); event.stopPropagation();
      return;
    }
    if (prefix.current && !["Control", "Shift", "Alt", "Meta"].includes(event.key)) {
      prefix.current = false;
      if (event.key.toLowerCase() === "u" && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) { event.preventDefault(); event.stopPropagation(); close(); }
    }
  };
  return <div className="quick-terminal-backdrop" inert={suspended}>
    <div ref={container} tabIndex={-1} className="quick-terminal" role="dialog" aria-modal="true" aria-label="Quick terminal" onKeyDownCapture={keyDown}
      style={rect ? { left: rect.left + rect.width * 0.1, top: rect.top + rect.height * 0.1, width: rect.width * 0.8, height: rect.height * 0.8 } : undefined}>
      <header className="quick-terminal-header"><span>Quick terminal</span><button type="button" aria-label="Close quick terminal" disabled={closeState === "pending"} onClick={close}>{closeState === "pending" ? "Closing…" : closeState === "unconfirmed" ? "Dismiss…" : "Close"}</button></header>
      {closeState === "pending" ? <div role="status">Waiting for the terminal to close…</div> : closeState === "unconfirmed" ? <div role="alert">Terminal closure could not be confirmed. The shell or detached processes may still be running.</div> : null}
      <div className="quick-terminal-body"><TerminalPane client={client} request={{ session_id: sessionId, pane_id: paneId, target_kind: "scratch" }} selected presented controlAllowed controlPending={false} focusEpoch={focusEpoch} focusToken={1} terminalMouseInput terminateRef={terminate} accessibleLabel="Quick terminal shell" onStateChange={onTerminalStateChange} onClosed={onDismiss} /></div>
    </div>
  </div>;
}
