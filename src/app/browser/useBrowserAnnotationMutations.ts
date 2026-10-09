import { useCallback } from "react";
import type { BrowserDraftRecoveryAction, BrowserViewDraftAnnotation } from "../../protocol/generated/v1";
import type { CockpitClient } from "../../client/CockpitClient";
import { cloneDraftAnnotation, context, errorMessage, newId, retiredDraft, sameDraftAnnotation } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserViewCommandApi } from "./useBrowserViewCommand";
import type { BrowserDrafts } from "./useBrowserDrafts";

export function useBrowserAnnotationMutations(state: BrowserPaneState, view: BrowserViewCommandApi, drafts: BrowserDrafts, client: CockpitClient): BrowserAnnotationMutations {
  const { associationOwner, associationOwnerRef, workScope, snapshotRef, editorDirtyRef, noteIdRef, noteValueRef, pendingDeliveryIdsRef, applyDraft, setMessage, setSelectedId, setNoteId, setNoteValue, setNoteEditorDismissed, markEditorDirty } = state;
  const { command } = view;
  const { queueDraftMutation } = drafts;
  const persist = async (annotation: BrowserViewDraftAnnotation): Promise<boolean> => {
    const owner = associationOwner;
    const editorGenerationAtIntent = owner.editorGeneration;
    const noteIdAtIntent = owner.noteId;
    const noteTextAtIntent = owner.noteText.slice(0, 4000);
    const notesOpenAtIntent = owner.notesOpen;
    const current = snapshotRef.current; const documentContext = current ? context(current) : null;
    const draftAtIntent = owner.draft;
    if (!documentContext || !draftAtIntent) { setMessage("The browser draft is still recovering."); return false; }
    const mutationKey = newId("annotation-intent");
    owner.pendingAnnotationMutations.push({ key: mutationKey, kind: "upsert", draftId: draftAtIntent.draft_id, expectedRevision: draftAtIntent.revision, annotation: cloneDraftAnnotation(annotation), annotationId: annotation.id });
    try {
      let expectedRevision = draftAtIntent.revision;
      const expectedDraft = () => {
        const candidate = owner.draft;
        if (!candidate || candidate.draft_id !== draftAtIntent.draft_id) throw new Error("The browser draft association changed before annotation save completed.");
        if (candidate.revision !== draftAtIntent.revision && owner.localDraftRevision !== candidate.revision) {
          throw new Error("The browser draft changed externally before annotation save completed.");
        }
        expectedRevision = candidate.revision;
        return candidate;
      };
      let acknowledgedResult = await queueDraftMutation(async () => {
        expectedDraft();
        return command({ type: "draft", context: documentContext, draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, command: { type: "upsert_annotation", annotation } });
      });
      if (acknowledgedResult?.type === "draft" && !retiredDraft(owner, acknowledgedResult.draft)) {
        owner.draft = acknowledgedResult.draft;
        owner.localDraftRevision = acknowledgedResult.draft.revision;
      }
      let latest = snapshotRef.current; let sameContext = Boolean(latest && JSON.stringify(context(latest)) === JSON.stringify(documentContext));
      const mutation = owner.pendingAnnotationMutations.find((candidate) => candidate.key === mutationKey);
      if (mutation) mutation.expectedRevision = expectedRevision;
      let accepted = sameContext && acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => sameDraftAnnotation(candidate, annotation));
      if (!accepted && !owner.retiredDraftIds.has(draftAtIntent.draft_id)) {
        const recovery = await queueDraftMutation(() => client.browserDraftRecovery({ scope: workScope, action: { type: "upsert_annotation", draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, annotation } }));
        acknowledgedResult = recovery.type === "draft" ? recovery : null;
        if (acknowledgedResult?.type === "draft" && !retiredDraft(owner, acknowledgedResult.draft)) {
          owner.draft = acknowledgedResult.draft;
          owner.localDraftRevision = acknowledgedResult.draft.revision;
          if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(acknowledgedResult.draft);
        }
        accepted = Boolean(acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => sameDraftAnnotation(candidate, annotation)));
      }
      if (owner.retiredDraftIds.has(draftAtIntent.draft_id)) {
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutationKey);
        return false;
      }
      latest = snapshotRef.current; sameContext = Boolean(latest && JSON.stringify(context(latest)) === JSON.stringify(documentContext));
      if (accepted) {
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((mutation) => mutation.key !== mutationKey);
        if (associationOwnerRef.current === owner && !owner.sealed) {
          const editorUnchanged = owner.editorGeneration === editorGenerationAtIntent
            && owner.noteId === noteIdAtIntent
            && owner.noteText.slice(0, 4000) === noteTextAtIntent
            && owner.notesOpen === notesOpenAtIntent;
          if (editorUnchanged) {
            setSelectedId(annotation.id);
            setNoteId(annotation.id);
            setNoteValue((annotation.comment ?? "").slice(0, 4000));
            setNoteEditorDismissed(false);
            editorDirtyRef.current = false;
          } else {
            setMessage("Annotation saved; newer editor work was retained.");
          }
        }
      } else setMessage("Annotation save was not acknowledged; retry saving.");
      return accepted;
    } catch (error) {
      if (!owner.retiredDraftIds.has(draftAtIntent.draft_id)) setMessage(`Annotation save failed; retry saving: ${errorMessage(error)}`);
      else owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutationKey);
      return false;
    }
  };
  const removeAnnotation = (annotationId: string): void => {
    const owner = associationOwner;
    const current = snapshotRef.current; const documentContext = current ? context(current) : null;
    const draftAtIntent = owner.draft;
    if (!documentContext || !draftAtIntent) { setMessage("The browser draft is still recovering."); return; }
    const mutationKey = newId("annotation-intent");
    const previousAnnotation = draftAtIntent.annotations.find((annotation) => annotation.id === annotationId) ?? null;
    owner.pendingAnnotationMutations.push({ key: mutationKey, kind: "remove", draftId: draftAtIntent.draft_id, expectedRevision: draftAtIntent.revision, annotation: previousAnnotation ? cloneDraftAnnotation(previousAnnotation) : null, annotationId });
    void (async () => {
      let expectedRevision = draftAtIntent.revision;
      const expectedDraft = () => {
        const candidate = owner.draft;
        if (!candidate || candidate.draft_id !== draftAtIntent.draft_id) throw new Error("The browser draft association changed before annotation removal completed.");
        if (candidate.revision !== draftAtIntent.revision && owner.localDraftRevision !== candidate.revision) {
          throw new Error("The browser draft changed externally before annotation removal completed.");
        }
        expectedRevision = candidate.revision;
      };
      let acknowledgedResult = await queueDraftMutation(async () => {
        if (owner.retiredDraftIds.has(draftAtIntent.draft_id)) return null;
        expectedDraft();
        return command({ type: "draft", context: documentContext, draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, command: { type: "remove_annotation", annotation_id: annotationId } });
      });
      if (owner.retiredDraftIds.has(draftAtIntent.draft_id)) {
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutationKey);
        return;
      }
      if (acknowledgedResult?.type === "draft" && !retiredDraft(owner, acknowledgedResult.draft)) {
        owner.draft = acknowledgedResult.draft;
        owner.localDraftRevision = acknowledgedResult.draft.revision;
      }
      const mutation = owner.pendingAnnotationMutations.find((candidate) => candidate.key === mutationKey);
      if (mutation) mutation.expectedRevision = expectedRevision;
      const latest = snapshotRef.current;
      let acknowledged = Boolean(latest && JSON.stringify(context(latest)) === JSON.stringify(documentContext) && acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && !acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => candidate.id === annotationId));
      if (!acknowledged) {
        const recovery = await queueDraftMutation(() => client.browserDraftRecovery({ scope: workScope, action: { type: "remove_annotation", draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, annotation_id: annotationId } }));
        acknowledgedResult = recovery.type === "draft" ? recovery : null;
        if (acknowledgedResult?.type === "draft" && !retiredDraft(owner, acknowledgedResult.draft)) {
          owner.draft = acknowledgedResult.draft;
          owner.localDraftRevision = acknowledgedResult.draft.revision;
          if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(acknowledgedResult.draft);
        }
        if (owner.retiredDraftIds.has(draftAtIntent.draft_id)) {
          owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutationKey);
          return;
        }
        acknowledged = Boolean(acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && !acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => candidate.id === annotationId));
      }
      if (acknowledged) {
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((mutation) => mutation.key !== mutationKey);
      }
      else setMessage("Annotation removal was not acknowledged; retry removing it.");
    })().catch((error) => {
      if (!owner.retiredDraftIds.has(draftAtIntent.draft_id)) setMessage(`Annotation removal failed; retry removing it: ${errorMessage(error)}`);
      else owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutationKey);
    });
    if (noteIdRef.current === annotationId) {
      noteIdRef.current = null;
      noteValueRef.current = "";
      owner.noteId = null;
      owner.noteText = "";
      setNoteId(null);
      setNoteValue("");
      setNoteEditorDismissed(true);
      markEditorDirty();
    }
    setSelectedId((selected) => selected === annotationId ? null : selected);
  };
  const retryAnnotationMutations = useCallback(async (): Promise<void> => {
    const owner = associationOwner;
    for (const mutation of [...owner.pendingAnnotationMutations]) {
      if (owner.retiredDraftIds.has(mutation.draftId)) {
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutation.key);
        continue;
      }
      try {
        const resolution = await queueDraftMutation(async () => {
          if (owner.retiredDraftIds.has(mutation.draftId)) throw new Error("Browser draft was retired during navigation.");
          const inventory = await client.browserDraftRecovery({ scope: workScope, action: { type: "list" } });
          if (owner.retiredDraftIds.has(mutation.draftId)) throw new Error("Browser draft was retired during navigation.");
          if (inventory.type !== "draft_inventory") throw new Error("The retained annotation inventory could not be read.");
          const current = inventory.inventory.drafts.find((candidate) => candidate.draft_id === mutation.draftId);
          if (!current) throw new Error("The retained annotation draft no longer exists.");
          const alreadyApplied = mutation.kind === "upsert"
            ? current.annotations.some((candidate) => sameDraftAnnotation(candidate, mutation.annotation!))
            : !current.annotations.some((candidate) => candidate.id === mutation.annotationId);
          let expectedRevision = mutation.expectedRevision;
          if (current.revision !== expectedRevision) {
            if (alreadyApplied) return { draft: current, acknowledged: true, local: false };
            if (owner.localDraftRevision === current.revision && current.revision > expectedRevision) {
              expectedRevision = current.revision;
              mutation.expectedRevision = expectedRevision;
            } else {
              throw new Error("The retained annotation action conflicts with a newer draft revision.");
            }
          }
          const action: BrowserDraftRecoveryAction = mutation.kind === "upsert"
            ? { type: "upsert_annotation", draft_id: mutation.draftId, expected_revision: expectedRevision, annotation: mutation.annotation! }
            : { type: "remove_annotation", draft_id: mutation.draftId, expected_revision: expectedRevision, annotation_id: mutation.annotationId };
          const response = await client.browserDraftRecovery({ scope: workScope, action });
          const nextDraft = response.type === "draft" ? response.draft : null;
          const acknowledged = Boolean(nextDraft && (mutation.kind === "upsert"
            ? nextDraft.annotations.some((candidate) => sameDraftAnnotation(candidate, mutation.annotation!))
            : !nextDraft.annotations.some((candidate) => candidate.id === mutation.annotationId)));
          return { draft: nextDraft, acknowledged, local: true };
        });
        if (owner.retiredDraftIds.has(mutation.draftId)) {
          owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutation.key);
          continue;
        }
        if (!resolution.draft || !resolution.acknowledged) throw new Error("The retained annotation action was not acknowledged.");
        owner.draft = resolution.draft;
        if (resolution.local) owner.localDraftRevision = resolution.draft.revision;
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutation.key);
        if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(resolution.draft);
      } catch (error) {
        if (owner.retiredDraftIds.has(mutation.draftId)) {
          owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutation.key);
          continue;
        }
        setMessage(`Could not reconcile retained annotation work: ${errorMessage(error)}`);
        return;
      }
    }
    setMessage("Retained annotation work was acknowledged.");
  }, [applyDraft, associationOwner, client, queueDraftMutation]);
  const discardAnnotationMutations = useCallback(async (): Promise<void> => {
    const owner = associationOwner;
    const discarded = owner.pendingAnnotationMutations.length;
    const hadUnknownDelivery = Boolean(pendingDeliveryIdsRef.current);
    owner.pendingAnnotationMutations = [];
    if (hadUnknownDelivery) {
      pendingDeliveryIdsRef.current = null;
    }
    if (discarded === 0 && !hadUnknownDelivery) return;
    setMessage(hadUnknownDelivery
      ? "Discarded local retry intents. Feedback may already exist remotely; no undo or duplicate send was attempted."
      : "Discarded local annotation retry intents. A prior unknown write may still exist remotely; no undo was attempted.");
  }, [associationOwner]);
  return { persist, removeAnnotation, retryAnnotationMutations, discardAnnotationMutations };
}

export interface BrowserAnnotationMutations {
  persist(annotation: BrowserViewDraftAnnotation): Promise<boolean>; removeAnnotation(annotationId: string): void;
  retryAnnotationMutations(): Promise<void>; discardAnnotationMutations(): Promise<void>;
}
