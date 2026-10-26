import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { QuickAskAvailability, QuickAskHarness, QuickAskLocation, QuickAskStatus } from "../../protocol/generated/v1";
import { quickAskPromptValidation, quickAskTerminal } from "./quickAsk";

type Destination = { sessionId: string; serverInstance: string; spaceId: string };

export interface UseQuickAskOptions {
  client: CockpitClient;
  open: boolean;
  setOpen: (open: boolean) => void;
  sessionId: string | null;
  serverInstance: string | null;
  spaceId: string | null;
  initialCwd: string | null;
  live: boolean;
  spaceIds: readonly string[];
  mutationBusy: boolean;
  onOpenTab: (location: QuickAskLocation) => void;
}

export interface QuickAskState {
  open: boolean;
  setOpen: (open: boolean) => void;
  harness: QuickAskHarness;
  setHarness: (harness: QuickAskHarness) => void;
  question: string;
  setQuestion: (question: string) => void;
  cwd: string;
  setCwd: (cwd: string) => void;
  availability: QuickAskAvailability | null;
  status: QuickAskStatus | null;
  error: string | null;
  validation: string | null;
  switchToTab: boolean;
  setSwitchToTab: (value: boolean) => void;
  unknownAcknowledged: boolean;
  inspected: boolean;
  directLaunch: boolean;
  manualPaste: boolean;
  busy: boolean;
  start: () => void;
  cancel: () => void;
  send: () => void;
  openTab: () => void;
  copyQuestion: () => void;
  dismissUnknown: () => void;
  newQuestion: () => void;
}

/** Window-local state: closing the composer never cancels an operation or discards a draft. */
export function useQuickAsk(options: UseQuickAskOptions): QuickAskState {
  const { client, open, setOpen, sessionId, serverInstance, spaceId, initialCwd, live, spaceIds, mutationBusy, onOpenTab } = options;
  const [harness, setHarness] = useState<QuickAskHarness>("omp");
  const [question, setQuestion] = useState("");
  const [cwd, setCwd] = useState("");
  const [availability, setAvailability] = useState<QuickAskAvailability | null>(null);
  const [status, setStatus] = useState<QuickAskStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [switchToTab, setSwitchToTab] = useState(true);
  const [unknownAcknowledged, setUnknownAcknowledged] = useState(false);
  const [inspected, setInspected] = useState(false);
  const [busy, setBusy] = useState(false);
  const destination = useRef<Destination | null>(null);
  const operation = useRef<{ sessionId: string; requestId: string } | null>(null);
  const requestMultiline = useRef(false);
  const statusRef = useRef<QuickAskStatus | null>(null);
  const actionBusy = useRef(false);
  const actionInFlight = useRef(false);
  const wasOpen = useRef(false);
  const uncertain = useRef(false);
  const uncertainSend = useRef(false);
  const switched = useRef<string | null>(null);

  useEffect(() => {
    if (open && !wasOpen.current) {
      if (!operation.current || (statusRef.current && quickAskTerminal(statusRef.current.phase) && statusRef.current.phase !== "outcome_unknown")) {
        if (destination.current?.spaceId !== spaceId || destination.current?.sessionId !== sessionId || destination.current?.serverInstance !== serverInstance) {
          setCwd(initialCwd ?? "");
        }
        destination.current = sessionId && serverInstance && spaceId ? { sessionId, serverInstance, spaceId } : null;
      }
    }
    wasOpen.current = open;
  }, [open, sessionId, serverInstance, spaceId, initialCwd]);

  useEffect(() => {
    if (open) {
      setAvailability(null);
      let active = true;
      void client.quickAskAvailability().then(value => {
        if (!active) return;
        setAvailability(value);
        setHarness(current => value.agents.some(agent => agent.harness === current && agent.found)
          ? current : value.agents.find(agent => agent.found)?.harness ?? current);
      }, () => {
        if (active) setError("Could not check agent availability. Your question is kept.");
      });
      return () => { active = false; };
    }
  }, [open, client]);

  const applyStatus = useCallback((next: QuickAskStatus) => {
    const current = operation.current;
    if (!current || current.sessionId !== next.session_id || current.requestId !== next.request_id) return;
    if (uncertainSend.current && next.phase === "ready_to_send") return;
    if (statusRef.current && next.updated_at_ms < statusRef.current.updated_at_ms) return;
    uncertain.current = false;
    statusRef.current = next;
    setStatus(next);
    setError(null);
    if (next.phase === "submitted") setQuestion("");
  }, []);

  const markUnknown = useCallback((message: string) => {
    uncertain.current = true;
    setError(message);
    const current = statusRef.current;
    if (current) {
      const unknown: QuickAskStatus = { ...current, phase: "outcome_unknown", delivery: "unknown", cancellable: false };
      statusRef.current = unknown;
      setStatus(unknown);
    }
  }, []);

  useEffect(() => {
    const current = operation.current;
    if (!current || (!uncertain.current && status && quickAskTerminal(status.phase))) return;
    let stopped = false;
    let timer: number;
    const poll = async () => {
      try {
        if (!actionInFlight.current) {
          const next = await client.quickAskStatus(current.sessionId, current.requestId);
          if (!stopped && !actionInFlight.current) applyStatus(next);
        }
      } catch {
        if (!stopped) markUnknown("Cannot confirm the question's outcome. Check the tab before asking again; it may have been sent.");
      }
      if (!stopped && operation.current === current && (uncertain.current || !statusRef.current || !quickAskTerminal(statusRef.current.phase))) {
        timer = window.setTimeout(poll, 400);
      }
    };
    timer = window.setTimeout(poll, 400);
    return () => { stopped = true; clearTimeout(timer); };
  }, [client, status, applyStatus, markUnknown]);

  useEffect(() => {
    if (open && (status?.phase === "submitted" || status?.phase === "initial_question_supplied") && status.location && switchToTab && switched.current !== status.request_id) {
      switched.current = status.request_id;
      setOpen(false);
      onOpenTab(status.location);
    }
  }, [open, status, switchToTab, onOpenTab, setOpen]);

  const validation = quickAskPromptValidation(question)
    ?? (!cwd.startsWith("/") ? "Enter an absolute folder path." : null)
    ?? (!live ? "Herdr is not live. Your question is kept." : null)
    ?? (!destination.current || !spaceIds.includes(destination.current.spaceId) || destination.current.sessionId !== sessionId || destination.current.serverInstance !== serverInstance ? "The original Space is no longer available." : null)
    ?? (mutationBusy ? "Another Herdr action is pending." : null)
    ?? (!availability ? "Checking agent availability." : null)
    ?? (availability && !availability.agents.some(agent => agent.harness === harness && agent.found) ? "This agent was not found on PATH." : null);

  const start = useCallback(() => {
    if (actionBusy.current || validation || !destination.current) return;
    const previous = statusRef.current;
    if (operation.current && (!previous || !quickAskTerminal(previous.phase) || previous.phase === "outcome_unknown")) return;
    requestMultiline.current = question.includes("\n");
    const target = destination.current;
    const requestId = crypto.randomUUID();
    operation.current = { sessionId: target.sessionId, requestId };
    const pending: QuickAskStatus = { request_id: requestId, session_id: target.sessionId, server_instance: target.serverInstance, harness, phase: "creating_tab", delivery: "not_sent", cancellable: false, location: null, error: null, updated_at_ms: 0 };
    statusRef.current = pending;
    uncertainSend.current = false;
    uncertain.current = false;
    switched.current = null;
    setStatus(pending);
    setInspected(false);
    setUnknownAcknowledged(false);
    setError(null);
    actionBusy.current = true;
    setBusy(true);
    void client.quickAskStart(target.sessionId, { request_id: requestId, server_instance: target.serverInstance, space_id: target.spaceId, cwd, harness, prompt: question })
      .then(applyStatus, () => markUnknown("Cannot confirm the request's outcome. Check the tab before asking again; it may have been sent."))
      .finally(() => { actionBusy.current = false; setBusy(false); });
  }, [client, cwd, harness, question, validation, applyStatus, markUnknown]);

  const action = useCallback((kind: "cancel" | "send") => {
    const current = operation.current;
    const previous = statusRef.current;
    if (!current || !previous || actionBusy.current || uncertain.current || (kind === "send" ? previous.phase !== "ready_to_send" || !inspected || requestMultiline.current : !previous.cancellable)) return;
    actionBusy.current = true;
    actionInFlight.current = true;
    setBusy(true);
    void client.quickAskAction(current.sessionId, { request_id: current.requestId, action: kind })
      .then(applyStatus, () => {
        if (kind === "send") uncertainSend.current = true;
        markUnknown("Cannot confirm the request's outcome. Check the tab before asking again; it may have been sent.");
      })
      .finally(() => { actionInFlight.current = false; actionBusy.current = false; setBusy(false); });
  }, [client, inspected, applyStatus, markUnknown]);

  const openTab = useCallback(() => {
    if (statusRef.current?.location) {
      if (statusRef.current.phase === "outcome_unknown") setUnknownAcknowledged(true);
      if (statusRef.current.phase === "waiting_for_agent" || statusRef.current.phase === "ready_to_send") setInspected(true);
      setOpen(false);
      onOpenTab(statusRef.current.location);
    }
  }, [onOpenTab, setOpen]);
  const copyQuestion = useCallback(() => {
    void navigator.clipboard.writeText(question).then(() => {
      if (statusRef.current?.phase === "outcome_unknown") setUnknownAcknowledged(true);
    }, () => setError("Could not copy the question."));
  }, [question]);
  const dismissUnknown = useCallback(() => {
    if (!unknownAcknowledged || statusRef.current?.phase !== "outcome_unknown") return;
    operation.current = null;
    statusRef.current = null;
    uncertain.current = false;
    uncertainSend.current = false;
    setStatus(null);
    setError(null);
    setUnknownAcknowledged(false);
  }, [unknownAcknowledged]);
  const newQuestion = useCallback(() => {
    if (actionBusy.current || (statusRef.current?.phase !== "submitted" && statusRef.current?.phase !== "initial_question_supplied")) return;
    operation.current = null;
    statusRef.current = null;
    uncertain.current = false;
    uncertainSend.current = false;
    destination.current = sessionId && serverInstance && spaceId ? { sessionId, serverInstance, spaceId } : null;
    setCwd(initialCwd ?? "");
    setStatus(null);
    setQuestion("");
    setError(null);
    setInspected(false);
    setUnknownAcknowledged(false);
  }, [sessionId, serverInstance, spaceId, initialCwd]);

  const directLaunch = harness === "copilot" || harness === "opencode" || (/[ \n]/u.test(question) && question[0] !== "@" && question[0] !== "-");
  const manualPaste = status?.phase === "ready_to_send" && requestMultiline.current;
  return { open, setOpen, harness, setHarness, question, setQuestion, cwd, setCwd, availability, status, error, validation, switchToTab, setSwitchToTab, unknownAcknowledged, inspected, directLaunch, manualPaste, busy, start, cancel: () => action("cancel"), send: () => action("send"), openTab, copyQuestion, dismissUnknown, newQuestion };
}
