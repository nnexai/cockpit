import { useEffect, useRef, useState } from "react";
import { CockpitClientError } from "../../client/CockpitClient";
import type { OrchestrationSnapshot, LiaisonLookupResponse, Run, TaskView } from "../../protocol/generated/v1";
import type { WorkbenchProps } from "../shell/Workbench";
import { plainError } from "../shell/plainErrors";
import { useSupervisor, type LiaisonAction, type MutationRequestScope } from "./useSupervisor";

type ConfirmationAttachment = { sessionId: string; stateStoreId: string; rootId: string; attachmentId: string; epoch: number };
type RecoveryState = ConfirmationAttachment & { message: string | null; retry: boolean; unavailable: boolean };
type RetainedOperation = { action: LiaisonAction; scope: MutationRequestScope; confirmation?: ConfirmationAttachment };
export interface ConfirmedRecovery {
  busy: boolean; unavailable: boolean; retry: boolean; message: string | null; error: boolean; stopConfirmable: boolean; freshAvailable: boolean; pendingAction: "resume" | "fresh" | null;
  confirm(): Promise<void>; retryResume(): Promise<void>; startNew(): Promise<void>; close(): void;
}
export interface LiaisonController {
  snapshot: OrchestrationSnapshot | null; root: Run | null; tasks: TaskView[];
  waiting: boolean; fresh: boolean; connected: boolean; status: string | null; actionLabel: string | null; actionEnabled: boolean; actionReason: string | null; startAvailable: boolean; error: string | null;
  paneId: string | null; tabId: string | null; start(): Promise<void>; activate(): Promise<void>; recovery: ConfirmedRecovery | null;
}
export function useLiaison(props: WorkbenchProps, openConversation: () => void, navigate: (paneId: string, run: Run, snapshot: OrchestrationSnapshot) => Promise<void>, conversationVisible: boolean): LiaisonController {
  const { client, state, selection, onReconnect } = props;
  const [project, setProject] = useState<{ sessionId: string; lookup: LiaisonLookupResponse; token: number; preferredRootId: string | null } | null>(null);
  const [rootSelection, setRootSelection] = useState<{ sessionId: string; rootId: string; token: number } | null>(null);
  const [lookupToken, setLookupToken] = useState(0);
  const lookupGeneration = useRef(0);
  const [navigationError, setNavigationError] = useState<string | null>(null);
  useEffect(() => { setNavigationError(null); }, [client, state.sessionId, selection.spaceId]);
  const retained = useRef<RetainedOperation | null>(null);
  const [recovery, setRecovery] = useState<RecoveryState | null>(null);
  const confirming = useRef(false);
  const focusRoot = useRef<{ sessionId: string; rootId: string; sourcePaneId: string | null; epoch?: number | null } | null>(null);
  const navigating = useRef(false);
  const lookup = project?.sessionId === state.sessionId ? project.lookup : null;
  const selected = rootSelection?.sessionId === state.sessionId ? rootSelection.rootId : null;
  // Keep a just-started root selected until the lookup following that selection arrives.
  const selectedValid = selected && (lookup?.status !== "resolved" || lookup.roots.some(item => item.root_id === selected) || project!.token < rootSelection!.token);
  const preferredRootId = project?.sessionId === state.sessionId ? project.preferredRootId : null;
  const rootId = selectedValid ? selected : preferredRootId;
  const observer = useSupervisor(client, state.sessionId ?? "", rootId, !!state.sessionId, state.sync === "live");
  useEffect(() => {
    let cancelled = false, timer = 0;
    if (!state.sessionId || state.sync !== "live") return;
    const sessionId = state.sessionId;
    const load = async () => {
      try {
        const next = await client.orchestrationLiaisonLookup({ session_id: sessionId, selected_root_id: null });
        if (!cancelled) {
          setProject(previous => ({ sessionId, lookup: next, token: lookupToken, preferredRootId: next.status === "resolved" ? next.preferred_root_id : previous?.sessionId === sessionId ? previous.preferredRootId : null }));
          if (next.status === "unavailable") timer = window.setTimeout(() => void load(), 1500);
        }
      } catch { if (!cancelled) timer = window.setTimeout(() => void load(), 1500); }
    };
    void load();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [client, state.sessionId, state.sync, lookupToken, observer.snapshot]);
  const snapshot = observer.snapshot;
  const root = rootId ? snapshot?.runs.find(run => run.run_id === rootId && run.root_id === rootId) ?? null : null;
  const candidate = lookup?.status === "resolved" ? lookup.roots.find(item => item.root_id === rootId) : undefined;
  const fresh = observer.connected && state.sync === "live" && snapshot?.runtime.status === "fresh";
  const observation = fresh && snapshot?.runtime.status === "fresh" && root ? snapshot.runtime.runs.find(item => item.run_id === root.run_id) : undefined;
  const waiting = !!root && root.stage !== "closed" && !!snapshot?.questions.some(question => question.run_id === root.run_id && question.receipt.status === "unresolved");
  const retainedScopeMatches = !retained.current || retained.current.scope.sessionId === state.sessionId && retained.current.scope.stateStoreId === snapshot?.state_store_id;
  const actionable = fresh && !observer.busy && !state.focusPending && !state.focusError && lookup?.status === "resolved" && retainedScopeMatches;
  const live = root && root.stage !== "closed" && root.stage !== "reported";
  const initialStarting = !!root && (root.stage === "queued" || root.stage === "starting") && !root.resume;
  const retainedConfirmation = retained.current?.confirmation;
  const kind = lookup?.status !== "resolved" ? rootId ? "resume" : "start"
    : !rootId ? "start"
    : !root || !candidate ? null
    : !live ? "start"
    : retainedConfirmation && retainedScopeMatches ? "confirm"
    : candidate.connection === "connected" ? null
    : candidate.resume === "in_progress" || candidate.resume === "blocked" && root.resume?.phase !== "blocked" || initialStarting ? "pending"
    : candidate.stop_confirmable ? "confirm"
    : candidate.resume === "available" || candidate.resume === "blocked" ? "resume" : "pending";
  const pendingReason = kind !== "pending" ? null : candidate?.resume === "in_progress" ? "Resuming…" : candidate?.cause?.message ?? (initialStarting ? "Starting…" : "Supervisor unavailable");
  const actionLabel = kind === "start" || kind === "pending" && initialStarting && candidate?.resume === "available" ? "Start supervisor" : kind === "confirm" || kind === "resume" || kind === "pending" ? "Resume supervisor" : null;
  const actionEnabled = actionable && (kind === "start" || kind === "resume" || kind === "confirm");
  const startAvailable = actionEnabled || actionable && observation?.presence === "present" && !!observation.pane_id;
  const tasks = root && snapshot?.board?.root_id === root.run_id ? snapshot.board.tasks : [];
  const status = !fresh ? "Offline" : observer.busy ? root ? "Resuming…" : "Starting…" : candidate?.resume === "blocked" && candidate.cause ? plainError(candidate.cause.code, candidate.cause.message).text : root?.resume && ["reconciling", "launching"].includes(root.resume.phase) ? "Resuming…" : root?.stage === "starting" || root?.stage === "queued" ? "Starting…" : null;
  const actionReason = actionEnabled || !actionLabel ? null : state.sync !== "live" ? "Offline" : lookup?.status !== "resolved" ? lookup?.message ?? "Checking supervisor…" : !fresh || observer.busy ? status : kind === "pending" ? pendingReason : retainedScopeMatches ? "Supervisor unavailable" : "The original request is kept.";
  const perform = async (operation: RetainedOperation) => {
    const result = await observer.mutateResult(operation.action, operation.scope);
    if (result?.result === "supervisor_started" && result.start.operation_id === operation.action.operation_id) {
      retained.current = null; focusRoot.current = { sessionId: operation.scope.sessionId, rootId: result.start.root_id, sourcePaneId: selection.paneId };
      const token = ++lookupGeneration.current;
      setRootSelection({ sessionId: operation.scope.sessionId, rootId: result.start.root_id, token });
      setLookupToken(token); onReconnect();
    } else if (result?.result === "run_resumed" && result.resume.resume.operation_id === operation.action.operation_id) {
      retained.current = null;
      if (result.resume.resume.phase !== "blocked") { focusRoot.current = { sessionId: operation.scope.sessionId, rootId: result.resume.run_id, sourcePaneId: selection.paneId, epoch: result.resume.resume.replacement_attachment_epoch }; onReconnect(); }
      setLookupToken(++lookupGeneration.current);
    }
    return result;
  };
  const start = async () => {
    if (!startAvailable || !state.sessionId || !snapshot || lookup?.status !== "resolved" || retained.current?.confirmation || kind === "confirm" && !retained.current) return;
    openConversation();
    setNavigationError(null);
    const acting = kind === "resume" || !!retained.current;
    if (root && !acting && kind !== "start" && observation?.presence === "present" && observation.pane_id) {
      try { await navigate(observation.pane_id, root, snapshot); }
      catch (failure) { setNavigationError(failure instanceof Error ? failure.message : "The conversation could not be opened."); }
      return;
    }
    if (root && !acting && kind !== "start") return;
    let operation = retained.current;
    if (operation && (operation.scope.sessionId !== state.sessionId || operation.scope.stateStoreId !== snapshot.state_store_id)) return;
    // Unknown outcomes must replay the original operation before changing kinds.
    if (!operation) {
      const action: LiaisonAction = root && kind !== "start" ? { action: "run_resume", run_id: root.run_id, operation_id: crypto.randomUUID(), expected_attachment_epoch: root.attachment?.epoch ?? null }
        : { action: "supervisor_start", operation_id: crypto.randomUUID(), target: selection.spaceId ? { target: "existing_space", workspace_id: selection.spaceId } : null, label: null, selected_root_id: live ? rootId : null };
      const scope = observer.captureMutationScope(action);
      if (!scope) return;
      operation = { action, scope }; retained.current = operation;
    }
    await perform(operation);
  };
  const activate = async () => {
    if (retained.current && !retained.current.confirmation || kind !== "confirm" && candidate?.resume !== "blocked") { await start(); return; }
    if (!actionEnabled || !state.sessionId || !snapshot) return;
    const captured = retained.current?.confirmation ?? (root?.attachment ? {
      sessionId: state.sessionId, stateStoreId: snapshot.state_store_id,
      rootId: root.run_id, attachmentId: root.attachment.attachment_id, epoch: root.attachment.epoch,
    } : null);
    if (captured) setRecovery({ ...captured, retry: !!retained.current, unavailable: false, message: retained.current ? "No answer yet. Retrying sends the same request." : candidate?.cause ? plainError(candidate.cause.code, candidate.cause.message).text : null });
  };
  const recoveryScopeMatches = !!recovery && recovery.sessionId === state.sessionId && recovery.stateStoreId === snapshot?.state_store_id && recovery.rootId === root?.run_id;
  useEffect(() => {
    if (recovery && recovery.sessionId !== state.sessionId && !confirming.current) setRecovery(null);
  }, [recovery, state.sessionId]);
  const recover = async (freshConversation: boolean, confirmedStopped = false) => {
    if (!recovery || recovery.unavailable || !recoveryScopeMatches || !actionable || confirming.current || !snapshot) return;
    if (retained.current && (retained.current.action.action === "run_start_fresh_supervisor") !== freshConversation || freshConversation && !retained.current && !candidate?.fresh_available) return;
    confirming.current = true;
    openConversation();
    setNavigationError(null);
    let operation = retained.current;
    if (!operation) {
      const action: LiaisonAction = freshConversation
        ? { action: "run_start_fresh_supervisor", run_id: recovery.rootId, operation_id: crypto.randomUUID(), expected_attachment_epoch: recovery.epoch }
        : { action: "run_resume", run_id: recovery.rootId, operation_id: crypto.randomUUID(), expected_attachment_epoch: recovery.epoch, ...(confirmedStopped ? { confirmed_stopped_attachment_id: recovery.attachmentId } : {}) };
      const scope = observer.captureMutationScope(action);
      if (!scope) { confirming.current = false; return; }
      operation = { action, scope, confirmation: recovery };
      retained.current = operation;
    }
    try {
      const result = await perform(operation);
      if (result?.result === "run_resumed" && result.resume.resume.operation_id === operation.action.operation_id) {
        if (result.resume.resume.phase === "blocked") {
          const cause = result.resume.resume.cause;
          setRecovery(previous => previous ? { ...previous, retry: false, unavailable: false, message: cause ? plainError(cause.code, cause.message).text : "The supervisor could not be started." } : null);
        } else setRecovery(null);
        return;
      }
      const failure = observer.mutationFailure.current;
      const code = failure instanceof CockpitClientError ? failure.operationCode : undefined;
      const definitive = code === "attachment_conflict" || code === "root_stop_confirmation_unavailable" || code === "fresh_supervisor_unavailable" || code === "fresh_supervisor_unsupported";
      if (definitive) retained.current = null;
      setRecovery(previous => previous ? { ...previous, retry: !definitive, unavailable: false,
        message: definitive && failure instanceof Error ? plainError(code, failure.message).text : "No answer yet. Retrying sends the same request.",
      } : null);
    } finally { confirming.current = false; }
  };
  useEffect(() => {
    if (!focusRoot.current || navigating.current || !root || !snapshot || focusRoot.current.sessionId !== state.sessionId || focusRoot.current.rootId !== root.run_id || focusRoot.current.epoch != null && root.attachment?.epoch !== focusRoot.current.epoch || observation?.presence !== "present" || !observation.pane_id || !state.snapshot?.panes.some(pane => pane.id === observation.pane_id)) return;
    if (!conversationVisible || selection.paneId !== focusRoot.current.sourcePaneId && selection.paneId !== observation.pane_id) { focusRoot.current = null; return; }
    navigating.current = true;
    void navigate(observation.pane_id, root, snapshot).then(() => { focusRoot.current = null; }).catch(failure => {
      focusRoot.current = null; setNavigationError(failure instanceof Error ? failure.message : "The conversation could not be opened.");
    }).finally(() => { navigating.current = false; });
  }, [root, snapshot, observation, state.sessionId, state.snapshot, selection.paneId, conversationVisible, navigate]);
  const savedLocation = root?.session_id === state.sessionId ? root.attachment?.location : null;
  const recoveryMessage = observer.busy ? "Working…" : !actionable ? actionReason : recovery?.message ?? null;
  return { snapshot, root, tasks, waiting, fresh, connected: !!live && candidate?.connection === "connected", status, actionLabel, actionEnabled, actionReason, startAvailable, start, activate,
    recovery: recovery && recoveryScopeMatches ? { busy: observer.busy, unavailable: recovery.unavailable || !actionable, retry: recovery.retry, message: recoveryMessage, error: !observer.busy && actionable && !!recovery.message, stopConfirmable: !!candidate?.stop_confirmable, freshAvailable: !!candidate?.fresh_available, pendingAction: recovery.retry ? retained.current?.action.action === "run_start_fresh_supervisor" ? "fresh" : "resume" : null, confirm: () => recover(false, true), retryResume: () => recover(false), startNew: () => recover(true), close: () => { if (!observer.busy && !confirming.current) setRecovery(null); } } : null,
    error: observer.error ?? navigationError ?? (fresh && candidate?.resume === "blocked" && candidate.cause ? plainError(candidate.cause.code, candidate.cause.message).text : null), paneId: observation?.presence === "present" ? observation.pane_id : savedLocation?.pane_id ?? null, tabId: observation?.presence === "present" ? observation.tab_id : savedLocation?.tab_id ?? null };
}
