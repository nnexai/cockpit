import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, CommentPreview, CommentRequestScope } from "../../protocol/generated/v1";

type CommentBatchOptions = {
  client: CockpitClient;
  sessionId: string;
  viewerId: string;
  scope: CommentRequestScope;
  identity: string;
  currentSourceId: string | null | undefined;
  invalidationGeneration: number;
  refreshGeneration: number;
  onCountChange?: (count: number) => void;
  onViewerError?: (error: unknown) => void;
};

export function commentErrorText(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
  return "The comments request could not be completed.";
}

// All batch requests share these guards, including editor and recovery mutations.
export function useCommentBatch({ client, sessionId, viewerId, scope, identity, currentSourceId, invalidationGeneration, refreshGeneration, onCountChange, onViewerError }: CommentBatchOptions) {
  const [batch, setBatch] = useState<CommentBatch | null>(null);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<CommentPreview | null>(null);
  const [retainedStale, setRetainedStale] = useState(false);
  const generationRef = useRef(0);
  const refreshedInvalidation = useRef(invalidationGeneration);
  const refreshedRefresh = useRef(refreshGeneration);
  const persistedBatchId = batch && batch.generation > 0 ? batch.batch_id : null;
  const identityRef = useRef(identity);
  identityRef.current = identity;
  const onViewerErrorRef = useRef(onViewerError);
  onViewerErrorRef.current = onViewerError;
  const reportError = useCallback((reason: unknown) => {
    setError(commentErrorText(reason));
    onViewerErrorRef.current?.(reason);
  }, []);

  const loadBatch = useCallback(async (batchId: string | null = null) => {
    const generation = ++generationRef.current;
    setLoading(true);
    setError(null);
    try {
      const next = await client.commentBatch(sessionId, viewerId, { scope, batch_id: batchId });
      if (generation !== generationRef.current || identityRef.current !== identity) return;
      setBatch(next);
      setPreview(null);
      onCountChange?.(next.drafts.length);
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) reportError(reason);
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setLoading(false);
    }
  }, [client, identity, onCountChange, viewerId, sessionId, reportError, scope]);

  useEffect(() => {
    if (pending || loading) return;
    const invalidated = refreshedInvalidation.current !== invalidationGeneration;
    const explicitlyRefreshed = refreshedRefresh.current !== refreshGeneration;
    if (!invalidated && !explicitlyRefreshed) return;
    refreshedInvalidation.current = invalidationGeneration;
    refreshedRefresh.current = refreshGeneration;
    void loadBatch(persistedBatchId);
  }, [persistedBatchId, invalidationGeneration, refreshGeneration, loadBatch, loading, pending]);

  const attach = async () => {
    if (!batch || pending || !currentSourceId) return;
    const generation = ++generationRef.current;
    setPending(true); setError(null);
    try {
      const next = await client.commentAttach(sessionId, viewerId, { scope, batch_id: batch.batch_id, expected_generation: batch.generation });
      if (generation === generationRef.current && identityRef.current === identity) { setBatch(next); setPreview(null); }
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) reportError(reason);
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setPending(false);
    }
  };

  const makePreview = async (retain: boolean) => {
    if (!batch || pending) return;
    const generation = ++generationRef.current;
    setPending(true); setError(null);
    try {
      const request = { batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, retain_stale_excerpts: retain };
      const next = await client.commentPreview(sessionId, viewerId, request);
      if (generation === generationRef.current && identityRef.current === identity) {
        setPreview(next);
        setRetainedStale(retain);
        if (next.stale_draft_ids.length > 0) {
          setBatch((current) => current ? {
            ...current,
            drafts: current.drafts.map((draft) => next.stale_draft_ids.includes(draft.draft_id) && draft.source_state === "current" ? { ...draft, source_state: "changed" } : draft),
          } : current);
        }
      }
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) reportError(reason);
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setPending(false);
    }
  };

  return {
    batch, setBatch, loading, pending, setPending, error, setError,
    preview, setPreview, retainedStale, setRetainedStale, persistedBatchId,
    generationRef, identityRef, onViewerErrorRef, reportError, loadBatch, attach, makePreview,
  };
}
