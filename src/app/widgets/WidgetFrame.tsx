import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { widgetDocument } from "./widgetDocument";

const MAX_SELECTION_BYTES = 16 * 1024;
const SHORTCUT_KEYS = /^(?:[^\x00-\x1f\x7f-\x9f]|Dead|Escape|Tab|Enter|ArrowUp|ArrowDown|ArrowLeft|ArrowRight)$/u;
const SHORTCUT_CODES = /^(?:Key[A-Z]|Digit[0-9]|Escape|Tab|Enter|ArrowUp|ArrowDown|ArrowLeft|ArrowRight|Space|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Backquote|Comma|Period|Slash)$/;

export type WidgetFrameProps = {
  document: string;
  revision: number;
  currentRevision: number;
  widgetKey: string;
  selection?: unknown;
  hasSelection?: boolean;
  agent: string;
  inputBlocked: boolean;
  onSelect(valueJson: string): void;
  onFocusRetired(): void;
  onUserFocus?(): void;
};

export function WidgetFrame({ document: html, revision, currentRevision, widgetKey, selection = null, hasSelection = false, agent, inputBlocked, onSelect, onFocusRetired, onUserFocus }: WidgetFrameProps) {
  // Selection is a mount/replacement snapshot, not a reason to reload a running page.
  const retained = useRef({ selection, hasSelection });
  retained.current = { selection, hasSelection };
  const next = useMemo(() => {
    const nonce = crypto.randomUUID();
    return { revision, widgetKey, document: html, nonce, srcDoc: widgetDocument(html, { nonce, revision, ...retained.current }) };
  }, [html, revision, widgetKey]);
  const [displayed, setDisplayed] = useState(next);
  const current = useRef(next);
  current.current = next;
  const displayedFrame = useRef<HTMLIFrameElement | null>(null);
  const pendingFrame = useRef<HTMLIFrameElement | null>(null);
  const replacing = displayed !== next;
  const frames = replacing ? [displayed, next] : [displayed];
  const live = useRef({ displayed, next, currentRevision, widgetKey, inputBlocked, onSelect, onUserFocus });
  live.current = { displayed, next, currentRevision, widgetKey, inputBlocked, onSelect, onUserFocus };
  const selectionTimes = useRef<{ nonce: string; times: number[] }>({ nonce: next.nonce, times: [] });
  const queuedSelection = useRef<{ nonce: string; valueJson: string } | null>(null);
  const shortcutTimes = useRef<number[]>([]);
  const prefixUntil = useRef(0);
  const inputIntent = useRef(-Infinity);
  const admittedFocus = useRef<string | null>(null);
  const hoveredFrame = useRef<string | null>(null);
  const admitFocus = () => {
    const state = live.current;
    if (window.document.activeElement !== displayedFrame.current || state.inputBlocked || admittedFocus.current === state.displayed.nonce) return;
    admittedFocus.current = state.displayed.nonce;
    state.onUserFocus?.();
  };
  const priorFocus = useRef<Element | null>(window.document.activeElement);
  const focusCheck = useRef<number | undefined>(undefined);
  const guardFocus = () => {
    window.clearTimeout(focusCheck.current);
    focusCheck.current = window.setTimeout(() => {
      const focused = window.document.activeElement;
      if (focused !== displayedFrame.current && focused !== pendingFrame.current) return;
      if (focused === displayedFrame.current && !live.current.inputBlocked
        && (admittedFocus.current === live.current.displayed.nonce || hoveredFrame.current === live.current.displayed.nonce
          || focused?.matches(":hover") || performance.now() - inputIntent.current < 500)) {
        admitFocus();
        return;
      }
      const prior = priorFocus.current;
      if (prior instanceof HTMLElement && prior !== window.document.body && prior.isConnected && !prior.closest("[inert]")) prior.focus({ preventScroll: true });
      else if (focused instanceof HTMLElement) focused.blur();
    // Child trusted-input postMessage is a separate renderer task. Give it a
    // frame to arrive; real pointer hover also permits entry without a timer race.
    }, 32);
  };
  useLayoutEffect(() => {
    const rememberFocus = (event: FocusEvent) => {
      if (event.target instanceof Element && event.target !== displayedFrame.current && event.target !== pendingFrame.current) {
        priorFocus.current = event.target;
        admittedFocus.current = null;
      }
    };
    const navigationIntent = (event: KeyboardEvent) => {
      if (!event.isTrusted || event.isComposing || live.current.inputBlocked) return;
      const dock = displayedFrame.current?.closest("[data-widget-tab]");
      if (event.key === "Tab" || (event.key === "Enter" && event.target instanceof Element && dock?.contains(event.target))) inputIntent.current = performance.now();
    };
    window.document.addEventListener("focusin", rememberFocus, true);
    window.document.addEventListener("keydown", navigationIntent, true);
    window.addEventListener("blur", guardFocus);
    return () => {
      window.document.removeEventListener("focusin", rememberFocus, true);
      window.document.removeEventListener("keydown", navigationIntent, true);
      window.removeEventListener("blur", guardFocus);
      window.clearTimeout(focusCheck.current);
    };
  }, []);
  useEffect(() => {
    shortcutTimes.current = []; prefixUntil.current = 0;
  }, [next.nonce]);
  useEffect(() => { inputIntent.current = -Infinity; admittedFocus.current = null; }, [displayed.nonce]);
  useEffect(() => {
    const queued = queuedSelection.current;
    if (!queued) return;
    if (queued.nonce !== next.nonce || currentRevision !== next.revision || inputBlocked) { queuedSelection.current = null; return; }
    if (displayed === next) { queuedSelection.current = null; onSelect(queued.valueJson); }
  }, [displayed, next, currentRevision, inputBlocked, onSelect]);
  useLayoutEffect(() => {
    const message = (event: MessageEvent) => {
      const state = live.current, activeFrame = displayedFrame.current;
      const isPending = state.displayed !== state.next && event.source === pendingFrame.current?.contentWindow;
      const frame = isPending ? pendingFrame.current : activeFrame;
      const incarnation = isPending ? state.next : state.displayed;
      const data: unknown = event.data;
      if (!frame || event.source !== frame.contentWindow || incarnation.widgetKey !== state.widgetKey
        || state.inputBlocked || window.document.body.classList.contains("is-pane-dragging")
        || typeof data !== "object" || data === null || Array.isArray(data)) return;
      const payload = data as Record<string, unknown>;
      if (payload.nonce !== incarnation.nonce || payload.revision !== incarnation.revision) return;
      if (payload.type === "cockpit.widget.intent") {
        if (!isPending) {
          inputIntent.current = performance.now();
          admitFocus();
        }
        return;
      }
      if (incarnation !== state.next || incarnation.revision !== state.currentRevision) return;
      if (payload.type === "cockpit.widget.select") {
        if (typeof payload.value_json !== "string" || payload.value_json.length > MAX_SELECTION_BYTES
          || new TextEncoder().encode(payload.value_json).byteLength > MAX_SELECTION_BYTES) return;
        const now = performance.now();
        if (selectionTimes.current.nonce !== incarnation.nonce) selectionTimes.current = { nonce: incarnation.nonce, times: [] };
        selectionTimes.current.times = selectionTimes.current.times.filter(time => now - time < 1000);
        if (selectionTimes.current.times.length >= 4) return;
        selectionTimes.current.times.push(now);
        if (isPending) queuedSelection.current = { nonce: incarnation.nonce, valueJson: payload.value_json };
        else state.onSelect(payload.value_json);
      } else if (payload.type === "cockpit.widget.shortcut") {
        if (isPending || window.document.activeElement !== frame || typeof payload.key !== "string" || payload.key.length > 10 || !SHORTCUT_KEYS.test(payload.key)
          || typeof payload.code !== "string" || payload.code.length > 12 || (payload.code !== "" && !SHORTCUT_CODES.test(payload.code))
          || ["ctrlKey", "altKey", "shiftKey", "metaKey", "repeat"].some(key => typeof payload[key] !== "boolean")
          || payload.metaKey) return;
        const now = performance.now();
        const prefix = payload.ctrlKey && !payload.altKey && !payload.shiftKey && payload.key.toLowerCase() === "b";
        if (!payload.ctrlKey && !payload.altKey && now >= prefixUntil.current) return;
        shortcutTimes.current = shortcutTimes.current.filter(time => now - time < 1000);
        if (shortcutTimes.current.length >= 16) return;
        shortcutTimes.current.push(now);
        prefixUntil.current = prefix ? now + 2000 : 0;
        // This dedicated channel goes directly to the host shortcut router. It is
        // never a synthetic keydown and cannot inject typing or terminal paste.
        window.dispatchEvent(new CustomEvent("cockpit-widget-shortcut", { detail: {
          key: payload.key, code: payload.code, ctrlKey: payload.ctrlKey, altKey: payload.altKey,
          shiftKey: payload.shiftKey, metaKey: false, repeat: payload.repeat, target: frame,
        } }));
      }
    };
    const prefixState = (event: Event) => {
      if (!(event instanceof CustomEvent) || typeof event.detail !== "object" || event.detail === null) return;
      const state = live.current, frame = displayedFrame.current;
      const { target, active } = event.detail as { target?: unknown; active?: unknown };
      if (!frame || target !== frame || typeof active !== "boolean" || window.document.activeElement !== frame
        || state.displayed !== state.next || state.displayed.revision !== state.currentRevision
        || state.displayed.widgetKey !== state.widgetKey || state.inputBlocked) return;
      prefixUntil.current = active ? performance.now() + 2000 : 0;
      frame.contentWindow?.postMessage({ type: "cockpit.widget.prefix", nonce: state.displayed.nonce,
        revision: state.currentRevision, active }, "*");
    };
    window.addEventListener("message", message);
    window.addEventListener("cockpit-widget-prefix", prefixState);
    return () => {
      window.removeEventListener("message", message);
      window.removeEventListener("cockpit-widget-prefix", prefixState);
    };
  }, []);

  return <div className="widget-frame-stack" data-input-blocked={inputBlocked || undefined}
    onPointerDownCapture={event => { if (event.nativeEvent.isTrusted && !inputBlocked) inputIntent.current = performance.now(); }}>
    {frames.map((frame) => {
      const pending = replacing && frame === next;
      return <iframe key={frame.nonce} ref={pending ? pendingFrame : displayedFrame} onFocus={guardFocus}
        onPointerEnter={event => { if (event.nativeEvent.isTrusted) hoveredFrame.current = frame.nonce; }}
        onPointerLeave={event => { if (event.nativeEvent.isTrusted && hoveredFrame.current === frame.nonce) hoveredFrame.current = null; }}
        className="widget-frame" data-pending={pending || undefined} data-revision={frame.revision}
        aria-hidden={pending || undefined} tabIndex={pending || inputBlocked ? -1 : 0}
        title={`Widget content from ${agent}`} sandbox="allow-scripts" allow="" referrerPolicy="no-referrer"
        srcDoc={frame.srcDoc} onLoad={() => {
          if (!pending || current.current !== frame) return;
          if (window.document.activeElement === displayedFrame.current) onFocusRetired();
          setDisplayed(frame);
        }} />;
    })}
  </div>;
}
