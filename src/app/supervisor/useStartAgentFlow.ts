import { useEffect, useRef, useState } from "react";
import type { Run } from "../../protocol/generated/v1";
import type { SupervisorLifecycleDrafts } from "./useSupervisorDrafts";
import type { SupervisorViewInputs, SupervisorViewModel, SupervisorViewNavigation, StartAgentFlowState, SupervisorViewActions } from "./supervisorViewTypes";

export function useStartAgentFlowState(sessionId: string, lifecycle: SupervisorLifecycleDrafts, changed: () => void): StartAgentFlowState {
  const [startPending, setStartPending] = useState(false);
  const [, renderUnknown] = useState(false);
  const startLock = useRef(false);
  const resumeOperations = useRef(lifecycle.resumes);
  const startDraft = useRef(lifecycle.start);
  resumeOperations.current = lifecycle.resumes; startDraft.current = lifecycle.start;
  const setStartUnknown: StartAgentFlowState["setStartUnknown"] = value => {
    lifecycle.startUnknown = typeof value === "function" ? value(lifecycle.startUnknown) : value;
    changed(); renderUnknown(lifecycle.startUnknown);
  };
  const lastSession = useRef(sessionId);
  const seenStart = useRef(0);
  return { startPending, setStartPending, startUnknown: lifecycle.startUnknown, setStartUnknown, startLock, resumeOperations, startDraft, lastSession, seenStart };
}

export function useStartAgentSessionReset(context: Pick<SupervisorViewInputs, "lastSession" | "sessionId" | "setRootId" | "setDialog" | "setTerminalError" | "setNotice" | "setAttentionOpen">) {
  const { lastSession, sessionId, setRootId, setDialog, setTerminalError, setNotice, setAttentionOpen } = context;
  useEffect(() => {
    if (lastSession.current === sessionId) return;
    lastSession.current = sessionId; setRootId(null); setDialog(null); setTerminalError(null); setNotice(null); setAttentionOpen(false);
  }, [sessionId]);
}

export function useStartAgentFlow(context: Pick<SupervisorViewInputs,
  "snapshot" | "connected" | "busy" | "startLock" | "startUnknown" | "setDialog" | "startFocus" | "setStartPending" | "setNotice" | "mutateResult" | "captureMutationScope" | "setRootId" | "setStartUnknown" | "startToken" | "startIntent" | "seenStart" | "active" | "runtimeLive" | "resumeOperations" | "startDraft" | "changed" | "sessionId"
> & Pick<SupervisorViewModel, "rootState" | "destination" | "root"> & Pick<SupervisorViewNavigation, "focusTaskOrStart">): Pick<SupervisorViewActions, "started" | "start" | "restart"> {
  const { snapshot, connected, busy, startLock, startUnknown, rootState, destination, setDialog, startFocus, setStartPending, setNotice, mutateResult, captureMutationScope, setRootId, setStartUnknown, startToken, startIntent, seenStart, active, root, focusTaskOrStart, runtimeLive, resumeOperations, startDraft, changed, sessionId } = context;
  const fresh = snapshot?.session_id === sessionId && connected && runtimeLive && snapshot.runtime.status === "fresh";
  const started = (runId: string) => { delete startDraft.current.operation; delete startDraft.current.requestScope; changed(); if (startFocus.current) startFocus.current.runId = runId; setRootId(runId); setStartUnknown(false); };
  const start = async (intent: "reuse_or_resume" | "another" = "reuse_or_resume") => {
    if (!snapshot || !fresh || busy || startLock.current || startUnknown && (!startDraft.current.operation || !startDraft.current.requestScope) || rootState?.kind === "starting") return;
    if (intent === "another" && startDraft.current.operation) { setNotice("The previous start is not confirmed. Your original request is kept; another agent will not be started."); return; }
    if (intent === "another") {
      startDraft.current.intent = "another";
      if (!startDraft.current.spaceId && destination) startDraft.current.spaceId = destination.id;
      changed(); setDialog({ mode: "start" }); return;
    }
    if (!destination && !startDraft.current.operation) { startDraft.current.intent = "reuse_or_resume"; changed(); setDialog({ mode: "start" }); return; }
    startFocus.current = { runId: null, invoker: document.activeElement };
    startLock.current = true; setStartPending(true); setNotice("Starting…");
    try {
      if (!startDraft.current.operation) {
        Object.assign(startDraft.current, { label: "", location: "existing", spaceId: destination!.id, directory: "", intent: "reuse_or_resume" });
        startDraft.current.operation = { action: "supervisor_start", operation_id: crypto.randomUUID(), mode: "reuse_or_resume", target: { target: "existing_space", workspace_id: destination!.id }, label: null, selected_root_id: root?.run_id ?? null, expected_project_key: null };
        startDraft.current.requestScope = captureMutationScope(startDraft.current.operation) ?? undefined;
        changed();
      }
      const action = startDraft.current.operation!;
      if (!startDraft.current.requestScope) { setStartUnknown(true); setNotice("The original start scope is unavailable. Your original request is kept."); return; }
      const result = await mutateResult(action, startDraft.current.requestScope);
      if (result?.result === "supervisor_started" && result.start.operation_id === action.operation_id) {
        started(result.start.root_id);
        setNotice(result.start.outcome === "reused" ? "Supervisor already running" : result.start.outcome === "resuming" ? "Resuming…" : "Starting…");
      } else { setStartUnknown(true); setNotice("The start was not confirmed. Your original request is kept."); }
    } finally { startLock.current = false; setStartPending(false); }
  };
  useEffect(() => { if (startToken > seenStart.current && active && snapshot && fresh) { seenStart.current = startToken; void start(startIntent); } }, [startToken, active, snapshot, fresh]);
  useEffect(() => {
    if (!active || !rootState?.verified || !root || startFocus.current?.runId !== root.run_id) return;
    setNotice(current => current === "Starting…" || current === "Resuming…" ? "Supervisor connected" : current);
    if (document.activeElement === startFocus.current.invoker) focusTaskOrStart(); startFocus.current = null;
  }, [active, root, rootState?.verified]);
  const restart = async (run: Run) => {
    if (!snapshot || !fresh || busy || startLock.current || run.session_id !== sessionId || run.stage === "closed" || run.stage === "reported") return;
    if (run.resume?.phase === "blocked" && run.resume.cause?.class === "decision" && run.resume.decision_id) {
      const retained = resumeOperations.current.get(run.run_id);
      setDialog({ mode: "replace", run, ...(retained?.action.mode === "replace_existing" ? { operation: retained.action, requestScope: retained.requestScope } : {}) }); return;
    }
    if (run.dispatch?.step === "setup_unknown") { setDialog({ mode: "setup_recovery", run }); return; }
    let retained = resumeOperations.current.get(run.run_id);
    if (retained && run.resume?.operation_id === retained.action.operation_id && ["connected", "reattached"].includes(run.resume.phase)) { resumeOperations.current.delete(run.run_id); retained = undefined; }
    if (!retained) {
      const action = { action: "run_resume" as const, run_id: run.run_id, operation_id: crypto.randomUUID(), expected_attachment_epoch: run.attachment?.epoch ?? null, mode: "reattach_or_restart" as const, decision_id: null };
      const requestScope = captureMutationScope(action);
      if (!requestScope) return;
      retained = { action, requestScope }; resumeOperations.current.set(run.run_id, retained); changed();
    }
    startFocus.current = { runId: run.run_id, invoker: document.activeElement };
    startLock.current = true; setStartPending(true); setNotice("Resuming…");
    try {
      const result = await mutateResult(retained.action, retained.requestScope);
      if (result?.result !== "run_resumed" || result.resume.run_id !== run.run_id || result.resume.resume.operation_id !== retained.action.operation_id) { setNotice("Resume was not confirmed. Your original request is kept; retry Resume to check that same operation."); return; }
      const resumed = result.resume.resume;
      if (resumed.phase === "blocked") setNotice(`Can't resume · ${resumed.cause?.message ?? "the existing agent is kept"}`);
      else if (resumed.phase === "connected" || resumed.phase === "reattached") { resumeOperations.current.delete(run.run_id); changed(); setNotice("Supervisor connected"); }
      else setNotice("Resuming…");
    } finally { startLock.current = false; setStartPending(false); }
  };
  return { started, start, restart };
}
