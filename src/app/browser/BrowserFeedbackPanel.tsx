import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { BrowserFeedbackCapture, BrowserFeedbackLookup, BrowserFeedbackSendResponse, BrowserViewCommandOutcome, BrowserViewDraftInventory, BrowserViewPendingCapture, BrowserWorkScope } from "../../protocol/generated/v1";
import { SavedDeliveryList } from "./BrowserChrome";
import { deliveryOperationId, errorMessage, newId, type SavedDelivery } from "./browserPaneModel";

export interface BrowserFeedbackPanelHandle {
  refresh(): Promise<void>;
  sendCapture(captureId: string, ids: string[]): Promise<boolean>;
  recoverPending(action: "retry_pending" | "discard_pending"): Promise<BrowserViewCommandOutcome | null>;
}
export interface BrowserFeedbackPanelProps {
  client: CockpitClient;
  scope: BrowserWorkScope;
  refreshKey?: number;
  /** A live leaf can supply a capture immediately, before its next feedback lookup. */
  pendingCapture?: BrowserViewPendingCapture | null;
  beforeRecovery?: () => Promise<void>;
  onRecovered?: (outcome: BrowserViewCommandOutcome, pending: BrowserViewPendingCapture | null) => void | Promise<void>;
  onPendingCaptureChange?: (pending: BrowserViewPendingCapture | null) => void;
  onPendingFeedbackChange?: (ids: string[] | null) => void;
  onMessage?: (message: string) => void;
  /** Tab delivery may use the leaf's existing focused-agent delivery hook. */
  onSend?: (ids: string[], operationId: string, acknowledgeDuplicateRisk: boolean) => Promise<BrowserFeedbackSendResponse>;
}

const sameIds = (left: string[], right: string[]): boolean => left.length === right.length && left.every((id, index) => id === right[index]);

export const BrowserFeedbackPanel = forwardRef<BrowserFeedbackPanelHandle, BrowserFeedbackPanelProps>(function BrowserFeedbackPanel(props, ref) {
  const { client, scope, refreshKey = 0 } = props;
  const scopeKey = JSON.stringify(scope);
  const owner = useRef({ key: scopeKey, client });
  if (owner.current.key !== scopeKey || owner.current.client !== client) {
    owner.current = { key: scopeKey, client };
  }
  const scopeOwner = owner.current;
  const callbacks = useRef(props);
  callbacks.current = props;
  const mounted = useRef(true);
  const lookupSequence = useRef(0);
  const busyRef = useRef<typeof scopeOwner | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [deliveries, setDeliveries] = useState<SavedDelivery[]>([]);
  const deliveriesRef = useRef<SavedDelivery[]>([]);
  const [captures, setCaptures] = useState<BrowserFeedbackCapture[]>([]);
  const [inventory, setInventory] = useState<BrowserViewDraftInventory | null>(null);
  const pendingRef = useRef<BrowserViewPendingCapture | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [duplicateRisk, setDuplicateRisk] = useState(false);
  const [preview, setPreview] = useState<{ captureId: string; src: string } | null>(null);
  const active = (token: typeof owner.current): boolean => mounted.current && owner.current === token;
  const report = (message: string): void => { setNotice(message); callbacks.current.onMessage?.(message); };
  const publish = (next: SavedDelivery[]): void => {
    deliveriesRef.current = next;
    setDeliveries(next);
    const pending = next.flatMap((item) => item.ids);
    callbacks.current.onPendingFeedbackChange?.(pending.length ? pending : null);
  };
  const update = (item: SavedDelivery): void => publish([...deliveriesRef.current.filter((candidate) => candidate.capture_id !== item.capture_id), item].sort((a, b) => a.capture_id.localeCompare(b.capture_id)));
  const applyLookup = (lookup: BrowserFeedbackLookup): void => {
    const pending = new Set(lookup.feedback.captures.flatMap((capture) => capture.pending_ids));
    const next = lookup.feedback.captures.filter((capture) => capture.pending_ids.length > 0).map((capture): SavedDelivery => {
      const receipt = lookup.deliveries.find((item) => item.capture_id === capture.id);
      const local = deliveriesRef.current.find((item) => item.capture_id === capture.id);
      // A missing receipt is not evidence that a transport-unknown paste never happened.
      if (!receipt && local?.hasReceipt) return { ...local, blocked: true, message: "The operation receipt is unavailable; no paste can be replayed." };
      const ids = receipt?.selected_ids ?? capture.pending_ids;
      const blocked = ids.length === 0 || ids.some((id) => !pending.has(id));
      return { capture_id: capture.id, ids: [...ids], operation_id: receipt?.operation_id ?? deliveryOperationId(capture.id), state: receipt?.state ?? "pending", message: blocked ? "Receipt IDs no longer match pending feedback; do not retry this operation." : receipt?.message ?? "Current-run feedback is waiting for an explicit retry.", blocked, hasReceipt: Boolean(receipt) };
    }).sort((a, b) => a.capture_id.localeCompare(b.capture_id));
    publish(next);
    setDuplicateRisk(false);
    setCaptures(lookup.feedback.captures);
    setSelected((id) => next.some((item) => item.capture_id === id) ? id : next[0]?.capture_id ?? null);
    if (lookup.drafts) {
      setInventory(lookup.drafts);
      pendingRef.current = lookup.drafts.pending_capture;
      callbacks.current.onPendingCaptureChange?.(lookup.drafts.pending_capture);
    }
  };
  const refresh = useCallback(async (): Promise<void> => {
    const token = scopeOwner;
    if (!active(token)) return;
    const sequence = ++lookupSequence.current;
    try {
      const lookup = await client.browserFeedback({ scope });
      if (!active(token) || sequence !== lookupSequence.current) return;
      applyLookup(lookup);
    } catch (error) {
      if (active(token) && sequence === lookupSequence.current) report(`Could not refresh browser feedback: ${errorMessage(error)}`);
    }
  }, [client, scopeKey]);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; ++lookupSequence.current; };
  }, []);
  useEffect(() => {
    publish([]);
    setCaptures([]); setInventory(null); pendingRef.current = null;
    busyRef.current = null; setBusy(false);
    setSelected(null); setDuplicateRisk(false); setNotice(null); setPreview(null);
    void refresh();
  }, [scopeKey, client]);
  useEffect(() => { if (refreshKey > 0) void refresh(); }, [refreshKey, refresh]);
  useEffect(() => {
    if (props.pendingCapture === undefined) return;
    pendingRef.current = props.pendingCapture;
    setInventory((current) => ({ drafts: current?.drafts ?? [], active_draft_limit: current?.active_draft_limit ?? 0, pending_capture: props.pendingCapture ?? null }));
  }, [props.pendingCapture, scopeKey]);

  const run = async <T,>(work: () => Promise<T>, fallback: T): Promise<T> => {
    if (busyRef.current) return fallback;
    const token = scopeOwner;
    if (!active(token)) return fallback;
    busyRef.current = token; setBusy(true);
    try { return await work(); }
    catch (error) { if (active(token)) report(errorMessage(error)); return fallback; }
    finally {
      if (busyRef.current === token) busyRef.current = null;
      if (active(token)) setBusy(false);
    }
  };
  const acknowledge = async (item: SavedDelivery): Promise<void> => {
    const token = scopeOwner;
    await client.acknowledgeBrowserFeedback({ scope, ids: item.ids });
    if (!active(token)) return;
    await callbacks.current.onRecovered?.({ type: "none" }, null);
    await refresh();
  };
  const send = async (item: SavedDelivery, acknowledgeDuplicateRisk: boolean): Promise<boolean> => {
    const token = owner.current;
    ++lookupSequence.current;
    update({ ...item, state: "pending", message: "Feedback delivery is pending.", hasReceipt: true });
    setDuplicateRisk(false);
    let response: BrowserFeedbackSendResponse;
    try {
      response = callbacks.current.onSend
        ? await callbacks.current.onSend(item.ids, item.operation_id, acknowledgeDuplicateRisk)
        : await client.sendBrowserFeedback({ scope, ids: item.ids, operation_id: item.operation_id, acknowledge_duplicate_risk: acknowledgeDuplicateRisk });
    } catch (error) {
      if (active(token)) {
        ++lookupSequence.current;
        const message = `Could not confirm annotation delivery; the original operation was retained: ${errorMessage(error)}`;
        update({ ...item, state: "outcome_unknown", message, hasReceipt: true }); report(message);
      }
      return false;
    }
    if (!active(token)) return false;
    ++lookupSequence.current;
    if (response.operation_id !== item.operation_id) {
      const message = "Feedback delivery returned a different operation identity; inspect the receipt before retrying.";
      update({ ...item, state: "outcome_unknown", message, hasReceipt: true }); report(message);
      return false;
    }
    const state = response.state === "pending" ? "outcome_unknown" : response.state;
    update({ ...item, state, message: response.message, hasReceipt: true });
    if (state !== "accepted") { report(response.message); return false; }
    report(`Pasted to ${response.target?.agent_label ?? "selected agent"} · Enter not sent`);
    try { await acknowledge({ ...item, state: "accepted" }); }
    catch (error) { if (active(token)) report(`Feedback was accepted, but its receipt could not be acknowledged: ${errorMessage(error)}`); }
    return true;
  };
  const reconcile = async (saved: SavedDelivery): Promise<{ item: SavedDelivery; receipt: BrowserFeedbackLookup["deliveries"][number] | undefined } | null> => {
    const token = owner.current;
    const lookup = await client.browserFeedback({ scope });
    if (!active(token)) return null;
    const capture = lookup.feedback.captures.find((item) => item.id === saved.capture_id);
    const receipt = lookup.deliveries.find((item) => item.capture_id === saved.capture_id);
    const pending = new Set(lookup.feedback.captures.flatMap((item) => item.pending_ids));
    if (!capture || saved.ids.length === 0 || saved.ids.some((id) => !pending.has(id))
      || (receipt && (receipt.operation_id !== saved.operation_id || !sameIds(receipt.selected_ids, saved.ids))) || (!receipt && saved.hasReceipt)) {
      applyLookup(lookup);
      report("Receipt identity changed or its selected IDs are no longer pending; no retry was sent.");
      return null;
    }
    return { item: saved, receipt };
  };
  const retry = async (captureId: string, resolveUnknown = false): Promise<boolean> => run(async () => {
    const saved = deliveriesRef.current.find((item) => item.capture_id === captureId);
    if (!saved || saved.blocked) return false;
    const original = await reconcile(saved);
    if (!original) return false;
    const receipt = original.receipt;
    if (resolveUnknown) {
      if (!duplicateRisk || receipt?.state !== "outcome_unknown") { report("The operation is still unresolved. No paste was replayed."); return false; }
      return send({ ...saved, operation_id: newId("browser-feedback"), hasReceipt: false }, true);
    }
    if (receipt?.state === "accepted") { await acknowledge(saved); return true; }
    if (receipt?.state === "pending" || receipt?.state === "outcome_unknown") {
      update({ ...saved, state: receipt.state, message: receipt.message, hasReceipt: true });
      report("The operation is still unresolved. No paste was replayed; review the receipt or explicitly acknowledge duplicate risk."); return false;
    }
    return send(receipt ? { ...saved, operation_id: newId("browser-feedback"), hasReceipt: false } : saved, false);
  }, false);
  const sendCapture = (captureId: string, ids: string[]): Promise<boolean> => {
    const saved: SavedDelivery = { capture_id: captureId, ids: [...ids], operation_id: deliveryOperationId(captureId), state: "pending", message: "Current-run feedback is ready for explicit delivery.", blocked: false, hasReceipt: false };
    if (!deliveriesRef.current.some((item) => item.capture_id === captureId)) update(saved);
    setSelected(captureId);
    return retry(captureId);
  };
  const recoverPending = (action: "retry_pending" | "discard_pending"): Promise<BrowserViewCommandOutcome | null> => run(async () => {
    const token = owner.current;
    const pending = pendingRef.current;
    await callbacks.current.beforeRecovery?.();
    if (!active(token)) return null;
    const outcome = await client.browserDraftRecovery({ scope, action: { type: action } });
    if (!active(token)) return null;
    if (outcome.type === "capture") {
      const next = outcome.capture.state === "pending" ? outcome.capture.pending : null;
      pendingRef.current = next;
      callbacks.current.onPendingCaptureChange?.(next);
      if (outcome.capture.state === "pending") report(next?.last_error ?? "Could not save the pending capture; retry or discard it.");
      else await callbacks.current.onRecovered?.(outcome, pending);
    }
    await refresh();
    return outcome;
  }, null);
  useImperativeHandle(ref, () => ({ refresh, sendCapture, recoverPending }));
  const canSend = !busy;
  return <div className="browser-feedback-panel" aria-label="Browser feedback">
    {notice ? <span role="status">{notice}</span> : null}
    {inventory?.pending_capture ? <div className="browser-capture-pending" role="status">
      Pending capture {inventory.pending_capture.capture_id} · {inventory.pending_capture.last_error ?? "Save failed; the capture is retained."}
      <button type="button" disabled={busy} onClick={() => void recoverPending("retry_pending").then(async (outcome) => {
        if (active(scopeOwner) && outcome?.type === "capture" && outcome.capture.state === "saved") {
          await sendCapture(outcome.capture.saved.capture_id, outcome.capture.saved.annotation_ids);
        }
      })}>Retry pending capture</button>
      <button type="button" disabled={busy} aria-label="Discard pending capture" onClick={() => void recoverPending("discard_pending")}>Discard pending capture</button>
    </div> : null}
    {deliveries.length ? <SavedDeliveryList deliveries={deliveries} selectedCaptureId={selected} duplicateRisk={duplicateRisk} sendDisabled={!canSend} busy={busy}
      onSelect={(id) => { setSelected(id); setDuplicateRisk(false); }} onDuplicateRiskChange={setDuplicateRisk}
      onResolveDuplicateRisk={() => { if (selected) void retry(selected, true); }}
      onRetry={(id) => void retry(id)} onAcknowledge={(id) => void run(async () => {
        const saved = deliveriesRef.current.find((item) => item.capture_id === id);
        if (!saved || saved.blocked) return;
        const original = await reconcile(saved);
        if (original?.receipt?.state === "accepted") await acknowledge(saved);
      }, undefined)} /> : null}
    {captures.map((capture, index) => <details key={capture.id}>
      <summary>Current-run image {index + 1} · {capture.page.title || capture.page.url || capture.id}</summary>
      <p>{capture.page.url} · {capture.page.captured_at}</p>
      {capture.annotations.map((annotation) => <p key={annotation.id}>{annotation.comment}</p>)}
      <button type="button" disabled={busy} onClick={() => void run(async () => {
        const token = owner.current;
        const image = await client.browserFeedbackImage({ scope, capture_id: capture.id });
        if (active(token)) setPreview({ captureId: capture.id, src: `data:${image.mime_type};base64,${image.data_base64}` });
      }, undefined)}>View current-run image</button>
      {preview?.captureId === capture.id ? <img src={preview.src} alt={`Current-run browser capture ${index + 1}`} style={{ maxWidth: "100%" }} /> : null}
    </details>)}
    {inventory?.drafts.map((draft) => <details key={draft.draft_id}>
      <summary>Current-run draft {draft.draft_id} · revision {draft.revision} · {draft.annotations.length} annotations</summary>
      <p>Original document {draft.target_id} · generation {draft.document_generation} · {draft.freshness}</p>
      {draft.annotations.map((annotation) => <p key={annotation.id}>{annotation.comment || `${annotation.kind} annotation`}</p>)}
      {draft.editor.note_text ? <p>Editor note: {draft.editor.note_text}</p> : null}
      <button type="button" disabled={busy || Boolean(inventory.pending_capture) || deliveries.length > 0} onClick={() => void run(async () => {
        const token = owner.current;
        await callbacks.current.beforeRecovery?.();
        if (!active(token)) return;
        // Read the revision after preserving live editor work; never discard a newer write.
        const listed = await client.browserDraftRecovery({ scope, action: { type: "list" } });
        if (!active(token)) return;
        const latest = listed.type === "draft_inventory" ? listed.inventory.drafts.find((item) => item.draft_id === draft.draft_id) : null;
        if (!latest || latest.revision !== draft.revision) { report("The draft changed; refresh and review it before discarding."); await refresh(); return; }
        const outcome = await client.browserDraftRecovery({ scope, action: { type: "discard_draft", draft_id: draft.draft_id, expected_revision: draft.revision } });
        if (!active(token)) return;
        await callbacks.current.onRecovered?.(outcome, null);
        await refresh();
      }, undefined)}>Discard current-run draft</button>
    </details>)}
  </div>;
});
