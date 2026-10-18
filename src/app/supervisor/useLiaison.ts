import { useEffect, useRef, useState } from "react";
import type { OrchestrationSnapshot, ProjectRootLookupResponse, Run, TaskView } from "../../protocol/generated/v1";
import type { WorkbenchProps } from "../shell/Workbench";
import { useSupervisor, type LiaisonAction, type MutationRequestScope } from "./useSupervisor";

type RetainedOperation = { action: LiaisonAction; scope: MutationRequestScope; spaceId: string; confirmedBlocked: boolean };
export interface LiaisonController {
  snapshot: OrchestrationSnapshot | null; root: Run | null; tasks: TaskView[];
  waiting: boolean; fresh: boolean; status: string | null; actionLabel: string | null; startAvailable: boolean; error: string | null;
  paneId: string | null; tabId: string | null; start(): Promise<void>;
}
export function useLiaison(props: WorkbenchProps, openConversation: () => void, navigate: (paneId: string, run: Run, snapshot: OrchestrationSnapshot) => Promise<void>, conversationVisible: boolean): LiaisonController {
  const { client, state, selection, onReconnect } = props;
  const [project, setProject] = useState<{ sessionId: string; spaceId: string; lookup: ProjectRootLookupResponse } | null>(null);
  const [rootSelection, setRootSelection] = useState<{ sessionId: string; spaceId: string; rootId: string } | null>(null);
  const [lookupToken, setLookupToken] = useState(0);
  const [navigationError, setNavigationError] = useState<string | null>(null);
  useEffect(() => { setNavigationError(null); }, [client, state.sessionId, selection.spaceId]);
  const retained = useRef<RetainedOperation | null>(null);
  const focusRoot = useRef<{ sessionId: string; rootId: string; sourcePaneId: string | null } | null>(null);
  const navigating = useRef(false);
  const lookup = project?.sessionId === state.sessionId && project.spaceId === selection.spaceId ? project.lookup : null;
  const selected = rootSelection?.sessionId === state.sessionId && rootSelection.spaceId === selection.spaceId ? rootSelection.rootId : null;
  const rootId = selected ?? (lookup?.status === "resolved" ? lookup.preferred_root_id : null);
  const observer = useSupervisor(client, state.sessionId ?? "", rootId, !!state.sessionId, state.sync === "live");
  useEffect(() => {
    let cancelled = false, timer = 0;
    if (!state.sessionId || !selection.spaceId || state.sync !== "live") return;
    const sessionId = state.sessionId, spaceId = selection.spaceId;
    const load = async () => {
      try {
        const next = await client.orchestrationProjectRootLookup({ session_id: sessionId, workspace_id: spaceId, selected_root_id: null });
        if (!cancelled) {
          setProject({ sessionId, spaceId, lookup: next });
          if (next.status === "unavailable") timer = window.setTimeout(() => void load(), 1500);
        }
      } catch { if (!cancelled) timer = window.setTimeout(() => void load(), 1500); }
    };
    void load();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [client, state.sessionId, state.sync, selection.spaceId, lookupToken, observer.snapshot]);
  const snapshot = observer.snapshot;
  const root = rootId ? snapshot?.runs.find(run => run.run_id === rootId && run.root_id === rootId) ?? null : null;
  const candidate = lookup?.status === "resolved" ? lookup.roots.find(item => item.root_id === rootId) : undefined;
  const fresh = observer.connected && state.sync === "live" && snapshot?.runtime.status === "fresh";
  const observation = fresh && snapshot?.runtime.status === "fresh" && root ? snapshot.runtime.runs.find(item => item.run_id === root.run_id) : undefined;
  const waiting = !!root && root.stage !== "closed" && !!snapshot?.questions.some(question => question.run_id === root.run_id && question.receipt.status === "unresolved");
  const retainedScopeMatches = !retained.current || retained.current.spaceId === selection.spaceId && retained.current.scope.sessionId === state.sessionId && retained.current.scope.stateStoreId === snapshot?.state_store_id;
  const actionable = fresh && !observer.busy && !state.focusPending && !state.focusError && lookup?.status === "resolved" && !!selection.spaceId && selection.spaceId === state.snapshot?.focused_space_id && retainedScopeMatches;
  const reselect = actionable && !!root && candidate?.resume === "blocked" && candidate.cause?.code === "checkout_identity_changed" && root.stage !== "closed" && root.stage !== "reported";
  const actionLabel = actionable && !rootId ? "Start supervisor" : reselect ? "Resume in this folder" : actionable && root && candidate?.resume === "available" && candidate.connection === "disconnected" && root.stage !== "closed" && root.stage !== "reported" ? "Resume supervisor" : null;
  const startAvailable = actionable && (!!actionLabel || observation?.presence === "present" && !!observation.pane_id);
  const tasks = root && snapshot?.board?.root_id === root.run_id ? snapshot.board.tasks : [];
  const status = !fresh ? "Offline" : observer.busy ? root ? "Resuming…" : "Starting…" : candidate?.resume === "blocked" && candidate.cause ? candidate.cause.message : root?.resume && ["reconciling", "launching"].includes(root.resume.phase) ? "Resuming…" : root?.stage === "starting" || root?.stage === "queued" ? "Starting…" : null;
  const start = async () => {
    if (!startAvailable || !state.sessionId || !selection.spaceId || !snapshot || lookup?.status !== "resolved") return;
    openConversation();
    setNavigationError(null);
    const acting = actionLabel === "Resume supervisor" || actionLabel === "Resume in this folder";
    if (root && !acting && observation?.presence === "present" && observation.pane_id) {
      try { await navigate(observation.pane_id, root, snapshot); }
      catch (failure) { setNavigationError(failure instanceof Error ? failure.message : "The conversation could not be opened."); }
      return;
    }
    if (root && !acting) return;
    let operation = retained.current;
    if (operation && (operation.spaceId !== selection.spaceId || operation.scope.sessionId !== state.sessionId || operation.scope.stateStoreId !== snapshot.state_store_id)) return;
    // Re-selection needs a new Start, not a confirmed blocked plain Resume.
    // Unknown outcomes must replay the original operation before changing kinds.
    if (reselect && operation?.action.action === "run_resume" && operation.confirmedBlocked) {
      retained.current = null; operation = null;
    }
    if (!operation) {
      const action: LiaisonAction = root && !reselect ? { action: "run_resume", run_id: root.run_id, operation_id: crypto.randomUUID(), expected_attachment_epoch: root.attachment?.epoch ?? null }
        : { action: "supervisor_start", operation_id: crypto.randomUUID(), mode: "reuse_or_resume", target: { target: "existing_space", workspace_id: selection.spaceId }, label: null, selected_root_id: rootId, expected_project_key: lookup.project.key };
      const scope = observer.captureMutationScope(action);
      if (!scope) return;
      operation = { action, scope, spaceId: selection.spaceId, confirmedBlocked: false }; retained.current = operation;
    }
    const result = await observer.mutateResult(operation.action, operation.scope);
    if (result?.result === "supervisor_started" && result.start.operation_id === operation.action.operation_id) {
      retained.current = null; focusRoot.current = { sessionId: operation.scope.sessionId, rootId: result.start.root_id, sourcePaneId: selection.paneId };
      setRootSelection({ sessionId: operation.scope.sessionId, spaceId: operation.spaceId, rootId: result.start.root_id });
      setLookupToken(value => value + 1); onReconnect();
    } else if (result?.result === "run_resumed" && result.resume.resume.operation_id === operation.action.operation_id) {
      operation.confirmedBlocked = result.resume.resume.phase === "blocked";
      if (result.resume.resume.phase !== "blocked") { retained.current = null; focusRoot.current = { sessionId: operation.scope.sessionId, rootId: result.resume.run_id, sourcePaneId: selection.paneId }; onReconnect(); }
      setLookupToken(value => value + 1);
    }
  };
  useEffect(() => {
    if (!focusRoot.current || navigating.current || !root || !snapshot || focusRoot.current.sessionId !== state.sessionId || focusRoot.current.rootId !== root.run_id || observation?.presence !== "present" || !observation.pane_id || !state.snapshot?.panes.some(pane => pane.id === observation.pane_id)) return;
    if (!conversationVisible || selection.paneId !== focusRoot.current.sourcePaneId && selection.paneId !== observation.pane_id) { focusRoot.current = null; return; }
    navigating.current = true;
    void navigate(observation.pane_id, root, snapshot).then(() => { focusRoot.current = null; }).catch(failure => {
      focusRoot.current = null; setNavigationError(failure instanceof Error ? failure.message : "The conversation could not be opened.");
    }).finally(() => { navigating.current = false; });
  }, [root, snapshot, observation, state.sessionId, state.snapshot, selection.paneId, conversationVisible, navigate]);
  const savedLocation = root?.session_id === state.sessionId ? root.attachment?.location : null;
  return { snapshot, root, tasks, waiting, fresh, status, actionLabel, startAvailable, start, error: observer.error ?? navigationError ?? (fresh && candidate?.resume === "blocked" ? candidate.cause?.message ?? null : null), paneId: observation?.presence === "present" ? observation.pane_id : savedLocation?.pane_id ?? null, tabId: observation?.presence === "present" ? observation.tab_id : savedLocation?.tab_id ?? null };
}
