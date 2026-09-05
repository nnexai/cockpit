import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, CommentPastePrepareResponse, CommentPasteReceipt, CommentRequestScope, CommentPreview } from "../../protocol/generated/v1";

export function CommentPasteControls({ client, sessionId, paneId, scope, batch, retainStale, preview, onAccepted }: {
  client: CockpitClient; sessionId: string; paneId: string; scope: CommentRequestScope;
  batch: CommentBatch; retainStale: boolean; preview: CommentPreview | null; onAccepted: () => void;
}) {
  const [prepared, setPrepared] = useState<CommentPastePrepareResponse | null>(null);
  const [targetPane, setTargetPane] = useState("");
  const [receipt, setReceipt] = useState<CommentPasteReceipt | null>(null);
  const [reviewedHash, setReviewedHash] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [duplicateRisk, setDuplicateRisk] = useState(false);
  const identity = `${sessionId}\0${paneId}\0${scope.binding_id}\0${batch.batch_id}\0${batch.generation}`;
  const identityRef = useRef(identity); identityRef.current = identity;
  const requestSequence = useRef(0);
  const prepare = useCallback(async () => {
    const sequence = ++requestSequence.current;
    setError(null); setPrepared(null); setDuplicateRisk(false);
    try {
      const result = await client.commentPastePrepare(sessionId, paneId, { batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, retain_stale_excerpts: retainStale });
      if (identityRef.current !== identity || sequence !== requestSequence.current) return;
      setPrepared(result);
    } catch (reason) {
      if (identityRef.current === identity && sequence === requestSequence.current) setError(reason instanceof Error ? reason.message : "Could not prepare paste.");
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
  useEffect(() => {
    let active = true;
    setReviewedHash(null);
    if (preview?.exportable && preview.batch_id === batch.batch_id && preview.generation === batch.generation) {
      void crypto.subtle.digest("SHA-256", new TextEncoder().encode(preview.payload)).then(hash => {
        if (active) setReviewedHash(`sha256:${Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, "0")).join("")}`);
      }).catch(() => { if (active) setError("This window could not verify the preview bytes for paste."); });
    }
    return () => { active = false; };
  }, [batch.batch_id, batch.generation, preview]);
  const previewMatches = reviewedHash !== null && reviewedHash === prepared?.payload_hash;
  const target = prepared?.targets.find(candidate => candidate.pane_id === targetPane);
  const previousUnknown = prepared?.receipts.some(item => !item.user_confirmed && (item.state === "outcome_unknown" || item.state === "pending")) ?? false;
  const send = async () => {
    if (!prepared || !target || !previewMatches || pending || (previousUnknown && !duplicateRisk)) return;
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
      if (result.state === "accepted") onAccepted();
    } catch (reason) {
      if (identityRef.current === identity) {
        setError(`Paste outcome is unconfirmed. Your comments are retained. Refresh the receipt before any retry. ${reason instanceof Error ? reason.message : ""}`);
        setPrepared(null); setTargetPane("");
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
      setReceipt(result); setPrepared(null); setTargetPane(""); onAccepted();
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
    {!previewMatches ? <p className="comment-paste-notice">Refresh the preview before pasting.</p> : null}
    {prepared?.reason ? <p role="status">{prepared.reason}</p> : null}
    {prepared?.paste_available && prepared.targets.length === 0 ? <p>No eligible agent in this tab.</p> : null}
    {prepared && prepared.targets.length > 0 ? <label>Agent in this tab<select aria-label="Paste target agent" value={targetPane} onChange={event => setTargetPane(event.target.value)} disabled={pending}>{prepared.targets.map(item => <option key={item.pane_id} value={item.pane_id}>{item.agent_label} · {item.pane_id}</option>)}</select></label> : null}
    {needsResolution.length > 0 ? <label><input type="checkbox" checked={duplicateRisk} onChange={event => setDuplicateRisk(event.target.checked)} /> I checked the agent input. Marking pasted resolves this delivery; retrying may duplicate it.</label> : null}
    {needsResolution.map(item => <button type="button" key={item.operation_id} disabled={pending || !duplicateRisk} onClick={() => void markPasted(item.operation_id)}>Mark pasted · {item.operation_id.slice(0, 8)}</button>)}
    <div className="comment-draft-actions"><button type="button" onClick={() => void prepare()} disabled={pending}>Refresh targets</button><button type="button" onClick={() => void send()} disabled={pending || !target || !previewMatches || !prepared?.paste_available || (previousUnknown && !duplicateRisk)}>{pending ? "Pasting…" : "Paste to agent"}</button></div>
  </section>;
}
