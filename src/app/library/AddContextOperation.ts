import { useEffect, useRef, useState, type RefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation } from "../../protocol/generated/v1";
import { errorText, issueCount, pageCount, type LibrarySpace } from "./libraryState";
import { useLibraryOperation } from "./useLibraryOperation";

type Tone = "running" | "done" | "failed";

/**
 * Phase 1 (design §4.7): the Library step. `saved` follows the items the
 * operation actually saved, so a cancel or failure after some were published
 * still offers them; `complete` is false when part of the source wasn't added.
 */
function libraryPhase(operation: LibraryOperation, unit: "items" | "pages" | "issues"): { text: string; tone: Tone; saved: boolean; complete: boolean } {
  if (operation.kind === "space_add") return { text: "✓ Saved to Library", tone: "done", saved: true, complete: true };
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  const partialReason = operation.report?.rows.find((row) => row.outcome === "partial")?.reason;
  const unchanged = operation.report?.rows.find((row) => row.outcome === "unchanged");
  const count = operation.item_ids.length;
  // A followed space counts its pages (`✓ Saved to Library · 38 pages`), a followed query its issues, or its items once references add other kinds.
  const summary = unit === "pages" ? ` · ${pageCount(count)}` : unit === "issues" ? ` · ${issueCount(count)}` : count > 1 ? ` · ${count} items` : "";
  switch (phase?.state ?? "pending") {
    case "pending":
    case "running":
      if (operation.cancel_requested) return { text: "Cancelling…", tone: "running", saved: false, complete: false };
      return { text: `Saving to Library…${phase?.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running", saved: false, complete: false };
    case "done":
      if (operation.cancel_requested) return { text: "Already saved to Library.", tone: "done", saved: true, complete: true };
      if (count === 0 && unchanged) return { text: `✓ ${unchanged.reason ?? "Already saved in Library"}`, tone: "done", saved: true, complete: true };
      return { text: `✓ Saved to Library${summary}`, tone: "done", saved: true, complete: true };
    case "partial":
      return { text: `◐ Saved to Library, partial${partialReason ? `: ${partialReason}` : ""}${summary}`, tone: "done", saved: true, complete: true };
    case "failed": {
      const reason = (phase?.error?.message ?? phase?.message ?? "The source could not be saved").replace(/\.$/, "");
      if (count > 0) return { text: `◐ Saved to Library${summary}, then stopped: ${reason}. Anything not yet saved wasn't added.`, tone: "failed", saved: true, complete: false };
      return { text: `✕ Not saved. ${reason}. Nothing was added.`, tone: "failed", saved: false, complete: false };
    }
    case "cancelled":
      if (count > 0) return { text: `◐ Saved to Library${summary}, then cancelled. Anything not yet saved wasn't added.`, tone: "done", saved: true, complete: false };
      return { text: "Cancelled. Nothing was added.", tone: "failed", saved: false, complete: false };
  }
}

/** Selecting saved Library items never changes their paths or contents. */
function spacePhase(operation: LibraryOperation, space: string): { text: string; tone: Tone } | null {
  const phase = operation.phases.find((candidate) => candidate.phase === "space");
  if (!phase) return null;
  const reason = (phase.error?.message ?? phase.message ?? "The selection could not be saved").replace(/\.$/, "");
  switch (phase.state) {
    case "pending":
      return operation.finished ? { text: `Saved to Library, but couldn't select for ${space}. ${reason}.`, tone: "failed" } : null;
    case "running":
      return { text: `Adding to ${space}…${phase.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running" };
    case "done":
    case "partial":
      return { text: `✓ Added to ${space}`, tone: "done" };
    case "failed":
    case "cancelled":
      return { text: `Saved to Library, but couldn't select for ${space}. ${phase.state === "cancelled" ? "Selection was cancelled" : reason}.`, tone: "failed" };
  }
}

/**
 * The dialog's accepted request outlives the dialog (design §4.7: closing does
 * not cancel). Reopening shows a request still starting, a running add, an
 * outcome that finished while closed, until `Add another` or a new add.
 */
type AcceptedAdd = {
  begin: () => Promise<LibraryOperation>;
  savedItemId: string | null;
  unit: "pages" | "issues" | "items";
  space: LibrarySpace | null;
  /** Settles after `operation` or `error` is recorded. */
  pending: Promise<LibraryOperation>;
  operation: LibraryOperation | null;
  error: string | null;
  seen: boolean;
};
let accepted: AcceptedAdd | null = null;

export function useAddContextOperation(client: CockpitClient, inputRef: RefObject<HTMLInputElement | null>) {
  const restoredRef = useRef(accepted);
  const lastBeginRef = useRef<(() => Promise<LibraryOperation>) | null>(restoredRef.current?.begin ?? null);
  const [operationSpace, setOperationSpace] = useState<LibrarySpace | null>(restoredRef.current?.space ?? null);
  const [savedItemId, setSavedItemId] = useState<string | null>(restoredRef.current?.savedItemId ?? null);
  const [operationUnit, setOperationUnit] = useState<"pages" | "issues" | "items">(restoredRef.current?.unit ?? "items");
  const [restoredError, setRestoredError] = useState<string | null>(restoredRef.current && !restoredRef.current.operation ? restoredRef.current.error : null);
  // Closed before the request was accepted: follow it until it settles.
  const [awaitingStart, setAwaitingStart] = useState(Boolean(restoredRef.current && !restoredRef.current.operation && !restoredRef.current.error));
  const add = useLibraryOperation(client);
  const resume = add.resume;
  useEffect(() => {
    const restored = restoredRef.current;
    if (restored?.operation) { resume(restored.operation); return; }
    if (!restored || restored.error) { if (!restored) inputRef.current?.focus(); return; }
    let current = true;
    // A newer request replaces this one; its late result must not overwrite the dialog.
    restored.pending.then((next) => {
      if (!current || accepted !== restored) return;
      setAwaitingStart(false);
      resume(next);
    }, () => {
      if (!current || accepted !== restored) return;
      setAwaitingStart(false);
      setRestoredError(restored.error);
    });
    return () => { current = false; };
  }, [resume]);
  const operation = add.operation;
  const shownSpace = operationSpace?.label ?? "the Space";
  const progress = operation ? libraryPhase(operation, operationUnit) : null;
  const spaceStep = operation && progress?.saved ? spacePhase(operation, shownSpace) : null;
  const finished = operation?.finished ?? false;
  const startError = add.error ?? restoredError;
  const operationRef = useRef(operation);
  operationRef.current = operation;
  // A displayed, complete success is done with; anything else stays for the next opening.
  useEffect(() => () => {
    const latest = operationRef.current;
    if (!accepted?.seen || !latest?.finished || accepted.operation?.operation_id !== latest.operation_id) return;
    const phase = libraryPhase(latest, accepted.unit);
    if (phase.saved && phase.complete && spacePhase(latest, "")?.tone !== "failed") accepted = null;
  }, []);
  useEffect(() => {
    if (finished && operation && accepted?.operation?.operation_id === operation.operation_id) accepted.seen = true;
  }, [finished, operation]);
  const begin = (request: () => Promise<LibraryOperation>, requestSpace: LibrarySpace | null, requestSavedItemId: string | null, unit: "pages" | "issues" | "items") => {
    const pending = request();
    const entry: AcceptedAdd = { begin: request, savedItemId: requestSavedItemId, unit, space: requestSpace, operation: null, error: null, seen: false, pending };
    // Record the result on this request only; a newer request owns `accepted`.
    entry.pending = pending.then((next) => { entry.operation = next; return next; }, (cause: unknown) => {
      entry.error = errorText(cause, "The Library operation could not start.");
      throw cause;
    });
    accepted = entry;
    lastBeginRef.current = request;
    setOperationSpace(requestSpace);
    setSavedItemId(requestSavedItemId);
    setOperationUnit(unit);
    setRestoredError(null);
    setAwaitingStart(false);
    void add.start(() => entry.pending);
  };
  // Reset only on an explicit Add another, never on close or remount.
  const reset = () => {
    accepted = null;
    add.reset();
    setRestoredError(null);
    setAwaitingStart(false);
  };
  // The same source again, whether the request never started or stopped part way.
  const retryAll = () => { if (lastBeginRef.current) begin(lastBeginRef.current, operationSpace, savedItemId, operationUnit); };
  const starting = add.starting || awaitingStart;
  const spaceFailed = spaceStep?.tone === "failed";
  const libraryIncomplete = finished && progress !== null && !progress.complete && lastBeginRef.current !== null;
  const actionFailed = Boolean(startError || spaceFailed || progress?.tone === "failed");
  const footerMessage = startError ?? (spaceFailed ? spaceStep?.text : progress?.tone === "failed" ? progress.text : null);
  const focusPhase = finished ? actionFailed ? "failed" : "finished" : starting || operation ? "running" : "idle";
  return { add, begin, reset, retryAll, operation, operationSpace, operationUnit, savedItemId, shownSpace,
    progress, spaceStep, finished, startError, starting, spaceFailed, libraryIncomplete, footerMessage, focusPhase,
    canRetry: lastBeginRef.current !== null };
}
