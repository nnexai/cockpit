import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, CommentPastePrepareResponse, CommentPasteReceipt, CommentRequestScope, CommentPreview } from "../../protocol/generated/v1";

function operationCode(error: unknown): string | null {
  if (typeof error !== "object" || error === null || !("operationCode" in error)) return null;
  const value = error.operationCode;
  return typeof value === "string" && value.length > 0 ? value : null;
}

function errorText(error: unknown): string {
  const message = error instanceof Error && error.message ? error.message : "The paste request failed.";
  const code = operationCode(error);
  return code ? `${code}: ${message}` : message;
}

const PROVEN_PRE_DISPATCH_ERRORS = new Set([
  "request_not_dispatched",
  "comments_paste_input_bounded",
  "comments_paste_framing",
]);

function mayHaveDispatched(error: unknown): boolean {
  // A host operation code is not, by itself, proof that no bytes were sent:
  // receipt persistence and reconciliation can fail after Herdr accepted them.
  const code = operationCode(error);
  return code === null || !PROVEN_PRE_DISPATCH_ERRORS.has(code);
}

export function CommentPasteControls({ client, sessionId, paneId, scope, batch, retainStale, preview, onBatchChanged }: {
  client: CockpitClient; sessionId: string; paneId: string; scope: CommentRequestScope;
  batch: CommentBatch; retainStale: boolean; preview: CommentPreview | null; onBatchChanged: () => void;
}) {
  const [prepared, setPrepared] = useState<CommentPastePrepareResponse | null>(null);
  const [targetPane, setTargetPane] = useState("");
  const [receipt, setReceipt] = useState<CommentPasteReceipt | null>(null);

  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [duplicateRisk, setDuplicateRisk] = useState(false);
  const identity = `${sessionId}\0${paneId}\0${scope.binding_id}\0${batch.batch_id}\0${batch.generation}`;
  const identityRef = useRef(identity); identityRef.current = identity;
  const onBatchChangedRef = useRef(onBatchChanged); onBatchChangedRef.current = onBatchChanged;
  const requestSequence = useRef(0);
  const prepare = useCallback(async () => {
    const sequence = ++requestSequence.current;
    setError(null); setPrepared(null); setDuplicateRisk(false);
    try {
      const result = await client.commentPastePrepare(sessionId, paneId, { batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, retain_stale_excerpts: retainStale });
      if (identityRef.current !== identity || sequence !== requestSequence.current) return;
      setPrepared(result);
    } catch (reason) {
      if (identityRef.current !== identity || sequence !== requestSequence.current) return;
      if (operationCode(reason) === "stale_generation") {
        // A send whose response was lost may have archived the sent drafts.
        // Only the reloaded batch can show what remains to paste.
        setError("Comments changed since this view loaded. Reloading the current comments…");
        onBatchChangedRef.current();
        return;
      }
      setError(reason instanceof Error ? reason.message : "Could not prepare paste.");
    }
  }, [batch.batch_id, batch.generation, client, identity, paneId, retainStale, scope, sessionId]);
  useEffect(() => {
    setReceipt(null); void prepare();
    return () => { requestSequence.current += 1; };
  }, [prepare]);
  useEffect(() => {
    if (!prepared) return;
    setTargetPane((current) => prepared.targets.some((candidate) => candidate.pane_id === current) ? current : (prepared.targets[0]?.pane_id ?? ""));
  }, [prepared]);
  const target = prepared?.targets.find(candidate => candidate.pane_id === targetPane);
  const previousUnknown = prepared?.receipts.some(item => !item.user_confirmed && (item.state === "outcome_unknown" || item.state === "pending")) ?? false;
  const send = async () => {
    if (!prepared || !target || pending || (previousUnknown && !duplicateRisk)) return;
    setPending(true); setError(null); setReceipt(null);
    const operationId = crypto.randomUUID();
    try {
      const result = await client.commentPasteSend(sessionId, paneId, {
        batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, target,
        expected_payload_hash: prepared.payload_hash, retain_stale_excerpts: retainStale,
        operation_id: operationId, request_id: operationId, acknowledge_duplicate_risk: duplicateRisk,
      });
      if (identityRef.current !== identity) return;
      setReceipt(result);
      setPrepared(null); setTargetPane("");
      if (result.state === "accepted") onBatchChanged();
    } catch (reason) {
      if (identityRef.current === identity) {
        if (mayHaveDispatched(reason)) {
          setError(`Paste outcome is unconfirmed. Your comments are retained. Refresh the receipt before any retry. ${errorText(reason)}`);
          setPrepared(null); setTargetPane("");
        } else {
          setError(`Paste failed before dispatch. Your comments are retained. ${errorText(reason)}`);
        }
      }
    } finally { setPending(false); }
  };
  const markPasted = async (operationId: string) => {
    if (pending || !duplicateRisk) return;
    setPending(true); setError(null);
    try {
      const result = await client.commentPasteMarkPasted(sessionId, paneId, {
        batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, operation_id: operationId,
      });
      if (identityRef.current !== identity) return;
      setReceipt(result); setPrepared(null); setTargetPane(""); onBatchChanged();
    } catch (reason) {
      if (identityRef.current === identity) setError(reason instanceof Error ? reason.message : "Could not resolve the paste receipt.");
    } finally { setPending(false); }
  };
  const receipts = receipt ? [receipt] : prepared?.receipts ?? [];
  const needsResolution = receipts.filter(item => !item.user_confirmed && (item.state === "outcome_unknown" || item.state === "pending" || (item.state === "accepted" && item.message?.startsWith("reconciliation required:"))));
  return <section className="comment-paste" aria-label="Paste comments to an agent">
    <h3>Paste to agent</h3>
    <p>Paste fills the selected agent input without submitting.</p>
    {error ? <p role="alert">{error}</p> : null}
    {receipts.map(item => <p role="status" key={item.operation_id}>Paste {item.state === "outcome_unknown" ? "outcome unknown" : item.state}{item.message ? `: ${item.message}` : ""}</p>)}
    {prepared?.reason ? <p role="status">{prepared.reason}</p> : null}
    {prepared?.paste_available && prepared.targets.length === 0 ? <p>No eligible agent in this tab.</p> : null}
    {prepared && prepared.targets.length > 0 ? <label>Agent in this tab<select aria-label="Paste target agent" value={targetPane} onChange={event => setTargetPane(event.target.value)} disabled={pending}>{prepared.targets.map(item => <option key={item.pane_id} value={item.pane_id}>{item.agent_label} · {item.pane_id}</option>)}</select></label> : null}
    {needsResolution.length > 0 ? <label><input type="checkbox" checked={duplicateRisk} onChange={event => setDuplicateRisk(event.target.checked)} /> I checked the agent input. Marking pasted resolves this delivery; retrying may duplicate it.</label> : null}
    {needsResolution.map(item => <button type="button" key={item.operation_id} disabled={pending || !duplicateRisk} onClick={() => void markPasted(item.operation_id)}>Mark pasted · {item.operation_id.slice(0, 8)}</button>)}
    <div className="comment-draft-actions"><button type="button" onClick={() => void prepare()} disabled={pending}>Refresh targets</button><button type="button" onClick={() => void send()} disabled={pending || !target || !prepared?.paste_available || (previousUnknown && !duplicateRisk)}>{pending ? "Pasting…" : "Paste to agent"}</button></div>
  </section>;
}
