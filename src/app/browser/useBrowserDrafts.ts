import { useCallback, useEffect, useRef, useState } from "react";
import type { BrowserDraftRecoveryAction, BrowserViewDraftState } from "../../protocol/generated/v1";
import type { CockpitClient } from "../../client/CockpitClient";
import { context, errorMessage, retiredDocumentKey, retiredDraft, type DraftAssociationOwner } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserViewCommandApi } from "./useBrowserViewCommand";

export function useBrowserDrafts(state: BrowserPaneState, view: BrowserViewCommandApi, client: CockpitClient): BrowserDrafts {
  const { associationOwner, associationOwnerRef, workScope, draftRequestRef, snapshotRef, draftRef, feedbackPanelRef, pendingCaptureRef, pendingDeliveryIdsRef, captureInFlightRef, editorDirtyRef, editorRevisionRef, noteId, noteValue, notesOpen, applyDraft, setPendingCaptureState, setDraft, setSelectedId, setNoteId, setNoteValue, setMessage, setAnnotationNotice } = state;
  const { command } = view;
  const discardInFlightRef = useRef(false);
  const editorPersistRef = useRef<Promise<void> | null>(null);
  const retryRetiredDraftsRef = useRef<((targetId: string, documentGeneration: number | null) => void) | null>(null);
  const openDraft = useCallback(async (): Promise<void> => {
    const request = ++draftRequestRef.current;
    await associationOwner.mutationTail;
    if (request !== draftRequestRef.current) return;
    const current = snapshotRef.current; const documentContext = current ? context(current) : null;
    if (!documentContext || !current?.document || current.document.target_id !== current.displayed_target_id) return;
    const key = retiredDocumentKey(documentContext.target_id, documentContext.document_generation);
    let currentDocumentRetired = false;
    const targetPrefix = `${documentContext.target_id}:`;
    for (const retirementKey of associationOwner.retiredDocumentKeys) {
      if (!retirementKey.startsWith(targetPrefix)) continue;
      const suffix = retirementKey.slice(targetPrefix.length);
      if (suffix === "*") {
        retryRetiredDraftsRef.current?.(documentContext.target_id, null);
        currentDocumentRetired = true;
      } else {
        const retiredGeneration = Number(suffix);
        if (!Number.isSafeInteger(retiredGeneration)) continue;
        retryRetiredDraftsRef.current?.(documentContext.target_id, retiredGeneration);
        if (retirementKey === key) currentDocumentRetired = true;
      }
    }
    if (currentDocumentRetired) return;
    const retained = draftRef.current;
    if (retained && (retained.target_id !== documentContext.target_id || retained.document_generation !== documentContext.document_generation)) {
      draftRef.current = null;
      associationOwner.draft = null;
      associationOwner.localDraftRevision = null;
      setDraft(null);
      setSelectedId(null);
      setNoteId(null);
      setNoteValue("");
    }
    const listed = await command({ type: "draft", context: documentContext, draft_id: null, expected_revision: null, command: { type: "list" } });
    if (request !== draftRequestRef.current || !listed || listed.type !== "draft_inventory") return;
    const latest = snapshotRef.current;
    const latestContext = latest ? context(latest) : null;
    if (!latestContext || latestContext.target_id !== documentContext.target_id || latestContext.document_generation !== documentContext.document_generation) return;
    const currentDraft = listed.inventory.drafts.find((candidate) => candidate.target_id === documentContext.target_id && candidate.document_generation === documentContext.document_generation && !retiredDraft(associationOwner, candidate));
    setPendingCaptureState(listed.inventory.pending_capture);
    const draftId = currentDraft?.draft_id ?? null;
    await command({ type: "draft", context: latestContext, draft_id: draftId, expected_revision: null, command: { type: "open", draft_id: draftId } });
  }, [associationOwner, command, setPendingCaptureState]);
  const refreshBrowserFeedback = useCallback(async (owner = associationOwner): Promise<void> => {
    if (associationOwnerRef.current === owner && !owner.sealed) await feedbackPanelRef.current?.refresh();
  }, [associationOwner]);
  const queueDraftMutation = useCallback(<T,>(run: () => Promise<T>): Promise<T> => {
    const next = associationOwner.mutationTail.catch(() => undefined).then(run);
    associationOwner.mutationTail = next.then(() => undefined, () => undefined);
    return next;
  }, [associationOwner]);
  const retireDraftsFor = useCallback((targetId: string, documentGeneration: number | null): void => {
    const owner = associationOwner;
    const retirementKey = documentGeneration === null ? `${targetId}:*` : retiredDocumentKey(targetId, documentGeneration);
    owner.retiredDocumentKeys.add(retirementKey);
    if (owner.draft?.target_id === targetId
      && (documentGeneration === null || owner.draft.document_generation === documentGeneration)) {
      const draftId = owner.draft.draft_id;
      owner.retiredDraftIds.add(draftId);
      owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((mutation) => mutation.draftId !== draftId);
    }
    void queueDraftMutation(async () => {
      const listed = await client.browserDraftRecovery({ scope: workScope, action: { type: "list" } });
      if (listed.type !== "draft_inventory") return;
      if (associationOwnerRef.current === owner && !owner.sealed) setPendingCaptureState(listed.inventory.pending_capture);
      const drafts = listed.inventory.drafts.filter((draft) => draft.target_id === targetId
        && (documentGeneration === null || draft.document_generation === documentGeneration));
      const ids = new Set(drafts.map((draft) => draft.draft_id));
      for (const id of ids) owner.retiredDraftIds.add(id);
      owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((mutation) => !ids.has(mutation.draftId));
      let complete = true;
      for (const draft of drafts) {
        try {
          await client.browserDraftRecovery({ scope: workScope, action: { type: "discard_draft", draft_id: draft.draft_id, expected_revision: draft.revision } });
        } catch {
          complete = false;
        }
      }
      if (complete) owner.retiredDocumentKeys.delete(retirementKey);
    }).catch(() => undefined);
  }, [associationOwner, client, queueDraftMutation, setPendingCaptureState]);
  retryRetiredDraftsRef.current = retireDraftsFor;
  const persistEditor = useCallback(async (requestedGeneration = associationOwner.editorGeneration): Promise<void> => {
    const owner = associationOwner;
    let savedNoteId: string | null = null;
    let savedNoteText = "";
    let savedNotesOpen = false;
    const result = await queueDraftMutation(async () => {
      const currentDraft = owner.draft;
      if (!currentDraft) throw new Error("The browser draft association changed before editor save completed.");
      savedNoteId = owner.noteId;
      savedNoteText = owner.noteText.slice(0, 4000);
      savedNotesOpen = owner.notesOpen;
      const editor: BrowserViewDraftState["editor"] = {
        selected_annotation_id: savedNoteId,
        notes_open: savedNotesOpen,
        note_annotation_id: savedNoteId,
        note_text: savedNoteText,
      };
      const action: BrowserDraftRecoveryAction = {
        type: "set_editor",
        draft_id: currentDraft.draft_id,
        expected_revision: currentDraft.revision,
        editor,
      };
      const response = await client.browserDraftRecovery({ scope: workScope, action });
      if (response.type === "draft" && !retiredDraft(owner, response.draft)) {
        owner.draft = response.draft;
        owner.localDraftRevision = response.draft.revision;
        if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(response.draft);
      }
      return response;
    });
    if (result.type !== "draft" || !owner.draft || result.draft.draft_id !== owner.draft.draft_id) throw new Error("The browser draft editor save was not acknowledged; retry saving.");
    owner.draft = result.draft;
    owner.localDraftRevision = result.draft.revision;
    const acknowledged = result.draft.editor;
    if ((acknowledged.note_annotation_id ?? acknowledged.selected_annotation_id) !== savedNoteId || (acknowledged.note_text ?? "") !== savedNoteText || Boolean(acknowledged.notes_open) !== savedNotesOpen) throw new Error("The browser draft editor save is still pending; retry saving.");
    if (owner.editorGeneration === requestedGeneration && owner.noteId === savedNoteId && owner.noteText.slice(0, 4000) === savedNoteText && owner.notesOpen === savedNotesOpen) {
      editorDirtyRef.current = false;
    } else {
      editorDirtyRef.current = true;
    }
  }, [applyDraft, associationOwner, client, queueDraftMutation]);
  const [editorTick, setEditorTick] = useState(0);
  useEffect(() => {
    if (!editorDirtyRef.current || !draftRef.current) return;
    const generation = editorRevisionRef.current;
    const timer = window.setTimeout(() => {
      if (editorPersistRef.current) return;
      editorPersistRef.current = persistEditor(generation).catch(() => undefined).finally(() => {
        editorPersistRef.current = null;
        if (editorDirtyRef.current && editorRevisionRef.current !== generation) setEditorTick((tick) => tick + 1);
      });
    }, 120);
    return () => window.clearTimeout(timer);
  }, [editorTick, noteId, noteValue, notesOpen, persistEditor]);
  const discardDraft = async (): Promise<void> => {
    const current = snapshotRef.current;
    const owner = associationOwner;
    const draftAtIntent = owner.draft;
    const documentContext = current ? context(current) : null;
    if (discardInFlightRef.current || captureInFlightRef.current || pendingCaptureRef.current || pendingDeliveryIdsRef.current
      || owner.pendingAnnotationMutations.length > 0 || !draftAtIntent || !documentContext) {
      const notice = "Cannot discard the draft while capture, feedback delivery, or annotation recovery is pending.";
      setMessage(notice);
      setAnnotationNotice(notice);
      return;
    }
    if (draftAtIntent.target_id !== documentContext.target_id
      || draftAtIntent.document_generation !== documentContext.document_generation) {
      const notice = "The browser draft changed; wait for the current document draft before discarding.";
      setMessage(notice);
      setAnnotationNotice(notice);
      return;
    }
    discardInFlightRef.current = true;
    setAnnotationNotice(null);
    try {
      const inventory = await queueDraftMutation(async () => {
        const latestDraft = owner.draft;
        if (!latestDraft || latestDraft.draft_id !== draftAtIntent.draft_id
          || latestDraft.target_id !== documentContext.target_id
          || latestDraft.document_generation !== documentContext.document_generation) {
          throw new Error("The browser draft changed before discard; the current draft was retained.");
        }
        await client.browserDraftRecovery({
          scope: workScope,
          action: { type: "discard_draft", draft_id: draftAtIntent.draft_id, expected_revision: latestDraft.revision },
        });
        const listed = await client.browserDraftRecovery({ scope: workScope, action: { type: "list" } });
        if (listed.type === "draft_inventory" && !listed.inventory.drafts.some((candidate) => candidate.draft_id === draftAtIntent.draft_id)) {
          // Block late, already accepted draft-open responses before the next queued mutation.
          owner.retiredDraftIds.add(draftAtIntent.draft_id);
          draftRequestRef.current += 1;
        }
        return listed;
      });
      if (inventory.type !== "draft_inventory" || inventory.inventory.drafts.some((candidate) => candidate.draft_id === draftAtIntent.draft_id)) {
        const notice = "The draft discard could not be confirmed; the current draft was retained.";
        setMessage(notice);
        setAnnotationNotice(notice);
        return;
      }
      if (associationOwnerRef.current !== owner || snapshotRef.current?.displayed_target_id !== documentContext.target_id
        || snapshotRef.current?.document?.document_generation !== documentContext.document_generation) return;
      owner.draft = null;
      owner.localDraftRevision = null;
      draftRef.current = null;
      setDraft(null);
      setSelectedId(null);
      setNoteId(null);
      setNoteValue("");
      editorDirtyRef.current = false;
      await openDraft();
      setMessage("Discarded the current document draft and opened a clean draft.");
    } catch (error) {
      const notice = `Could not discard the current draft: ${errorMessage(error)}`;
      setMessage(notice);
      setAnnotationNotice(notice);
    } finally {
      discardInFlightRef.current = false;
    }
  };
  return { openDraft, refreshBrowserFeedback, queueDraftMutation, retireDraftsFor, persistEditor, discardDraft };
}

export interface BrowserDrafts {
  openDraft(): Promise<void>; refreshBrowserFeedback(owner?: DraftAssociationOwner): Promise<void>;
  queueDraftMutation<T>(run: () => Promise<T>): Promise<T>; retireDraftsFor(targetId: string, documentGeneration: number | null): void;
  persistEditor(requestedGeneration?: number): Promise<void>; discardDraft(): Promise<void>;
}
