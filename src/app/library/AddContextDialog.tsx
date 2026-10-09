import { useEffect, useId, useRef } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { createPortal } from "react-dom";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";
import { trapDialogKeys, useRestoreFocus } from "./LibraryConfirmDialog";
import type { LibrarySpace } from "./libraryState";
import { useAddContextOperation } from "./AddContextOperation";
import { useAddContextSource } from "./AddContextSource";
import { AddContextSourceStep } from "./AddContextSourceStep";
import { AddContextOptionsStep } from "./AddContextOptionsStep";
import "../projects/setup.css";
import "../projects/taskSetup.css";
import "./library.css";


/**
 * Add context (design §4.5): one field for a forge issue, MR or PR link, a
 * Jira key, a Confluence page link or id, a Confluence space link or key, or a
 * local folder path, plus `Browse Confluence spaces`. It always saves to the
 * Library first, then selects the saved items for a live target Space.
 */
export function AddContextDialog({ client, onClose, onOpenItem, space = null, defaultDestination = "space" }: {
  client: CockpitClient;
  onClose: () => void;
  onOpenItem?: (itemId: string) => void;
  /** The live Space that can select the saved Library items. */
  space?: LibrarySpace | null;
  defaultDestination?: "library" | "space";
}) {
  const titleId = useId();
  const fieldId = useId();
  const failureId = useId();
  const destinationId = useId();
  const followChoiceId = useId();
  const followModeId = useId();
  const browseId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const closeActionRef = useRef<HTMLButtonElement>(null);
  const primaryActionRef = useRef<HTMLButtonElement>(null);
  const submitRef = useRef<HTMLButtonElement>(null);
  useRestoreFocus();
  const lifecycle = useAddContextOperation(client, inputRef);
  const source = useAddContextSource(client, space, defaultDestination);
  const { add, begin, retryAll, operation, operationSpace, operationUnit, savedItemId, shownSpace,
    progress, spaceStep, finished, startError, starting, spaceFailed, libraryIncomplete, footerMessage,
    focusPhase, canRetry } = lifecycle;
  const { resolution, existing, refreshExisting, destinationSpace, spaceAddable, existingInSpace, existingFollow,
    follow, jiraFollow, existingItemIds, requestProviderId, requestUrl, depthOffered, referenceDepth,
    followMode, attachmentsOffered, downloadAttachments, folder, label, existingItem, credentials, pickedFocus } = source;
  const canAdd = resolution !== null && !add.starting && (!existing || refreshExisting || (destinationSpace !== null && spaceAddable));
  const primaryLabel = existing
    ? refreshExisting ? (destinationSpace ? `Refresh and add to ${destinationSpace.label}` : "Refresh from source") : destinationSpace ? existingInSpace ? "Already in Space" : "Add to Space" : existingFollow ? "Already following" : "Already in Library"
    : destinationSpace ? `Add to Library and ${destinationSpace.label}`
    : follow ? jiraFollow ? "Follow query" : "Follow space" : "Add to Library";
  const submit = () => {
    if (!canAdd || !resolution) return;
    const target = destinationSpace?.target ?? null;
    if (existing && target && !refreshExisting) {
      // Select the existing Library items without asking the provider again.
      begin(() => client.librarySpaceAdd({ target, item_ids: existingItemIds }), destinationSpace, existingItemIds[0] ?? null, "items");
      return;
    }
    const providerId = requestProviderId ?? resolution.provider_id;
    begin(() => client.libraryAdd({
      input: requestUrl,
      provider_id: providerId,
      reference_depth: depthOffered ? referenceDepth : 0,
      follow,
      follow_mode: followMode,
      download_attachments: attachmentsOffered && downloadAttachments,
      refresh_existing: Boolean(existing) && refreshExisting,
      label: folder ? label?.trim() || resolution.title : null,
      target,
    }), destinationSpace, null, follow ? jiraFollow ? (referenceDepth > 0 ? "items" : "issues") : "pages" : "items");
  };
  const addAnother = () => {
    lifecycle.reset();
    source.reset();
    window.requestAnimationFrame(() => inputRef.current?.focus());
  };
  // A space chosen in the browser moves focus to the action it enables, or back to the field.
  useEffect(() => {
    if (pickedFocus > 0) (submitRef.current && !submitRef.current.disabled ? submitRef.current : inputRef.current)?.focus();
  }, [pickedFocus]);
  const openedItemId = operation?.item_ids[0] ?? operation?.space?.item_ids[0] ?? savedItemId ?? existingItem;
  // Progress polls and failures must not steal focus; successful completion can offer the result.
  useEffect(() => {
    if (focusPhase === "finished") (primaryActionRef.current ?? closeActionRef.current)?.focus();
    else if (focusPhase === "running") closeActionRef.current?.focus();
    else if (focusPhase === "failed" && !closeActionRef.current?.closest("[role='dialog']")?.contains(document.activeElement)) closeActionRef.current?.focus();
  }, [focusPhase]);
  // On the body: a Context viewer is a size container and would clip a fixed overlay to its pane.
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-add" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={(event) => {
      if (event.key === "Enter" && !operation && event.target instanceof HTMLInputElement && event.target.type === "text" && !event.nativeEvent.isComposing) {
        event.preventDefault();
        submit();
        return;
      }
      trapDialogKeys(event, onClose);
    }}>
      <header className="task-setup-header"><h2 id={titleId}>Add context</h2><button type="button" className="task-setup-close" onClick={onClose} aria-label="Close Add context"><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        {operation || starting ? <div className="library-progress" aria-live="polite">
          <ol className="library-progress-steps">
            <li className={`library-progress-step is-${progress?.tone ?? "running"}`}>
              <span>{progress?.tone === "failed" ? "Library" : progress?.text ?? (savedItemId ? `Adding to ${shownSpace}…` : operationUnit === "pages" ? "Following the space…" : operationUnit === "issues" ? "Following the query…" : "Saving to Library…")}</span>
              {operation && !finished && !operation.cancel_requested && operation.kind !== "space_add" && !progress?.saved ? <button type="button" onClick={add.cancel}>Cancel</button> : null}
            </li>
            {operationSpace || spaceStep ? <li className={`library-progress-step is-${spaceStep?.tone ?? "running"}`}>
              <span>{spaceFailed ? shownSpace : spaceStep?.text ?? `Waiting to add to ${shownSpace}…`}</span>
            </li> : null}
          </ol>
        </div> : <>
          <AddContextSourceStep client={client} source={source} fieldId={fieldId} failureId={failureId} browseId={browseId} inputRef={inputRef} />
          <AddContextOptionsStep source={source} space={space} fieldId={fieldId} followChoiceId={followChoiceId} followModeId={followModeId} destinationId={destinationId} />
        </>}
      </div>
      <footer className="task-setup-footer">
        <ErrorSlot placement="dialog" message={footerMessage} />
        <div className="library-footer-actions">
        <button ref={closeActionRef} type="button" onClick={onClose}>{operation || starting ? "Close" : "Cancel"}</button>
        {!operation && !starting ? <>
          {startError && canRetry ? <>
            <button type="button" onClick={addAnother}>Add another</button>
            <button ref={primaryActionRef} type="button" className="setup-primary" onClick={retryAll}>Retry</button>
          </> : null}
          {existingItem && onOpenItem ? <button type="button" onClick={() => { onOpenItem(existingItem); onClose(); }}>Open in Library</button> : null}
          {!startError ? <button ref={submitRef} type="button" className="setup-primary" onClick={submit} disabled={!canAdd}>{primaryLabel}</button> : null}
        </> : !finished || !progress ? null : <>
          <button type="button" onClick={addAnother}>Add another</button>
          {/* The same source again: primary when nothing was saved, beside the result otherwise. */}
          {libraryIncomplete ? <button ref={progress.saved ? undefined : primaryActionRef} type="button" className={progress.saved ? undefined : "setup-primary"} onClick={retryAll}>Retry</button> : null}
          {spaceFailed && finished ? <>
            {onOpenItem && openedItemId ? <button type="button" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
          </> : null}
          {!progress.saved || spaceFailed ? null : onOpenItem && openedItemId ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
        </>}
        </div>
      </footer>
    </section>
    {credentials.dialog}
  </div>, document.body);
}
