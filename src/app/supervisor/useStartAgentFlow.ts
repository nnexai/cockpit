import { useEffect, useRef, useState } from "react";
import { agentState } from "./SupervisorActions";
import type { StartDraft } from "./SupervisorDialogs";
import type { Run } from "../../protocol/generated/v1";
import type { SupervisorViewInputs, SupervisorViewModel, StartAgentFlowState, SupervisorViewActions } from "./supervisorViewTypes";

export function useStartAgentFlowState(sessionId: string): StartAgentFlowState {
  const [startPending, setStartPending] = useState(false);
  const [startUnknown, setStartUnknown] = useState(false);
  const startLock = useRef(false);
  const [pendingRestart, setPendingRestart] = useState<string | null>(null);
  const startDraft = useRef<StartDraft>({ label: "", location: "existing", spaceId: "", directory: "" });
  const lastSession = useRef(sessionId);
  const seenStart = useRef(0);
  return { startPending, setStartPending, startUnknown, setStartUnknown, startLock, pendingRestart, setPendingRestart, startDraft, lastSession, seenStart };
}

export function useStartAgentSessionReset(context: SupervisorViewInputs) {
  const { lastSession, sessionId, setRootId, setDialog, setTerminalError, setNotice, setStartUnknown, setPendingRestart, setAttentionOpen, startDraft } = context;
  useEffect(() => {
    if (lastSession.current === sessionId) return;
    lastSession.current = sessionId; setRootId(null); setDialog(null); setTerminalError(null); setNotice(null); setStartUnknown(false); setPendingRestart(null); setAttentionOpen(false);
    startDraft.current = { label: "", location: "existing", spaceId: "", directory: "" };
  }, [sessionId]);
}

export function useStartAgentFlow(context: SupervisorViewInputs & SupervisorViewModel & { focusTaskOrStart(): void }): Pick<SupervisorViewActions, "started" | "start" | "restart"> {
  const { snapshot, connected, busy, startLock, startUnknown, rootState, destination, setDialog, startFocus, setStartPending, setNotice, mutateResult, setRootId, setStartUnknown, startToken, seenStart, active, root, focusTaskOrStart, runtimeLive, setPendingRestart, pendingRestart } = context;
  const started = (runId: string) => { if (startFocus.current) startFocus.current.runId = runId; setRootId(runId); setStartUnknown(false); setNotice(null); };
  const start = async () => {
    if (!snapshot || !connected || busy || startLock.current || startUnknown || rootState?.kind === "starting") return;
    if (!destination) { setDialog({ mode: "start" }); return; }
    startFocus.current = { runId: null, invoker: document.activeElement };
    startLock.current = true; setStartPending(true); setNotice(null);
    try {
      const result = await mutateResult({ action: "supervisor_start", target: { target: "existing_space", workspace_id: destination.id }, label: null });
      if (result?.result === "run") started(result.run_id);
      else { setStartUnknown(true); setNotice("The start was not confirmed. Check status before starting another agent; a terminal may already have opened."); }
    } finally { startLock.current = false; setStartPending(false); }
  };
  useEffect(() => { if (startToken > seenStart.current && active && snapshot && connected) { seenStart.current = startToken; void start(); } }, [startToken, active, snapshot, connected]);
  useEffect(() => {
    if (!active || !rootState?.verified || !root || startFocus.current?.runId !== root.run_id) return;
    if (document.activeElement === startFocus.current.invoker) focusTaskOrStart(); startFocus.current = null;
  }, [active, root, rootState?.verified]);
  const restart = async (run: Run) => {
    if (!connected || !runtimeLive || busy || !run.dispatch) return;
    if (run.dispatch.step === "setup_unknown") { setDialog({ mode: "setup_recovery", run }); return; }
    if (run.dispatch.step === "plan_failed") { await mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null }); return; }
    if (run.dispatch.step === "launch_unknown" || run.dispatch.step === "needs_review") { setDialog({ mode: "retry", run }); return; }
    setNotice("Checking the previous launch before restart…");
    if (await mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null })) setPendingRestart(run.run_id);
    else setNotice("The previous launch could not be checked. No new agent was requested; tasks and history are kept.");
  };
  useEffect(() => {
    if (!pendingRestart || !snapshot || !active) return;
    if (!connected || !runtimeLive || snapshot.runtime.status !== "fresh") { setPendingRestart(null); setNotice("The previous launch is unobserved. Check the connection before restarting. No new agent was requested."); return; }
    const run = snapshot.runs.find(item => item.run_id === pendingRestart);
    if (!run || run.stage === "closed") { setPendingRestart(null); return; }
    if (run.dispatch?.step === "launch_unknown" || run.dispatch?.step === "needs_review") { setPendingRestart(null); setNotice(null); setDialog({ mode: "retry", run }); }
    else if (agentState(snapshot, run, connected, runtimeLive).verified) { setPendingRestart(null); setNotice("The original OMP agent is still connected. No new launch was started."); }
  }, [pendingRestart, snapshot, active, connected, runtimeLive]);
  return { started, start, restart };
}
