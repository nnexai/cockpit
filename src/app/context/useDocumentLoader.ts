import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { ContextRoot, ContextDocument, ContextKnownRevision } from "../../protocol/generated/v1";
import type { ContextReader, ContextDocumentRead } from "./contextSource";
import { readableError } from "./viewerState";
import { MAX_RETAINED_FILE_STATES } from "./viewerState";
export type DocumentState = {
  status: "loading" | "ready" | "error";
  document?: ContextDocument;
  error?: string;
};
const MAX_RETAINED_DOCUMENT_BYTES = 8 * 1024 * 1024;
function retainedDocumentBytes(state: DocumentState | undefined): number {
  const text = state?.document?.text;
  if (text === null || text === undefined) return 0;
  return Math.max(text.length, state?.document?.bytes ?? 0);
}
function retainDocumentState(current: Record<string, DocumentState>, key: string, state: DocumentState): Record<string, DocumentState> {
  const next = { ...current };
  delete next[key];
  next[key] = state;
  const keys = Object.keys(next);
  while (keys.length > MAX_RETAINED_FILE_STATES) {
    const oldest = keys.shift();
    if (oldest === undefined) break;
    delete next[oldest];
  }
  let bytes = Object.keys(next).reduce((total, candidate) => total + retainedDocumentBytes(next[candidate]), 0);
  if (bytes > MAX_RETAINED_DOCUMENT_BYTES) {
    for (const candidate of Object.keys(next)) {
      if (candidate === key) continue;
      bytes -= retainedDocumentBytes(next[candidate]);
      delete next[candidate];
      if (bytes <= MAX_RETAINED_DOCUMENT_BYTES) break;
    }
  }
  return next;
}
export interface DocumentCache {
  documents: Record<string, DocumentState>;
  setDocuments: Dispatch<SetStateAction<Record<string, DocumentState>>>;
}

export function useDocumentCache(): DocumentCache {
  const [documents, setDocuments] = useState<Record<string, DocumentState>>({});
  return { documents, setDocuments };
}

export function useDocumentLoader({ cache, reader, root, bindingId, identityKey, selectedPath, selectedKey, selectedRevision, selectedSnapshotRevision, refreshGeneration }: {
  reader: ContextReader | null; root: ContextRoot; bindingId: string; identityKey: string;
  selectedPath: string | null; selectedKey: string | null; selectedRevision: string | null;
  selectedSnapshotRevision: string | null; refreshGeneration: number;
  cache: DocumentCache;
}) {
  const { documents, setDocuments } = cache;
  const [documentPageLoading, setDocumentPageLoading] = useState<string | null>(null);
  const documentRequestSequence = useRef(0);
  const documentController = useRef<AbortController | null>(null);
  const mountedRef = useRef(true);
  const requestIdentityRef = useRef(identityKey);
  requestIdentityRef.current = identityKey;
  const currentBindingRef = useRef(bindingId);
  currentBindingRef.current = bindingId;
  const currentRootRef = useRef(root.root_id);
  currentRootRef.current = root.root_id;
  const activeRootId = root.root_id;
  useEffect(() => { mountedRef.current = true; return () => {
    mountedRef.current = false;
    documentController.current?.abort();
    documentController.current = null;
    documentRequestSequence.current += 1;
  }; }, []);
  useEffect(() => {
    documentController.current?.abort();
    documentController.current = null;
    setDocuments({});
    documentRequestSequence.current += 1;
  }, [identityKey]);
  const documentState = selectedKey ? documents[selectedKey] : undefined;
  const selectedDocumentIdentity = `${selectedKey ?? ""}\u0000${selectedSnapshotRevision ?? ""}`;
  const selectedDocumentIdentityRef = useRef(selectedDocumentIdentity);
  selectedDocumentIdentityRef.current = selectedDocumentIdentity;
  const document = documentState?.document;
  const loadDocumentPage = useCallback(async () => {
    if (!reader || !root || !selectedPath || !selectedKey || !document || document.next_offset === undefined) return;
    const expectedOffset = document.next_offset;
    const requestKey = `${selectedKey}\u0000${document.revision}\u0000${expectedOffset}`;
    const controller = new AbortController();
    documentController.current?.abort();
    documentController.current = controller;
    setDocumentPageLoading(requestKey);
    try {
      const data = await reader.document({
        root_id: root.root_id,
        path: selectedPath,
        expected_revision: document.revision,
        offset: expectedOffset,
      }, controller.signal);
      if (controller.signal.aborted || selectedDocumentIdentityRef.current !== selectedDocumentIdentity) return;
      const offset = data.offset ?? 0;
      if (data.revision !== document.revision || offset !== expectedOffset || data.text === null) throw new Error("Source changed while loading the next page; refresh to revalidate it.");
      setDocuments((current) => retainDocumentState(current, selectedKey, {
        status: "ready",
        document: {
          ...data,
          text: `${document.text ?? ""}${data.text}`,
          offset: 0,
          next_offset: data.next_offset,
          truncated: data.truncated,
          content_hash: data.truncated ? null : data.content_hash,
        },
      }));
    } catch (error) {
      if (!controller.signal.aborted && selectedDocumentIdentityRef.current === selectedDocumentIdentity) setDocuments((current) => retainDocumentState(current, selectedKey, { status: "error", document: current[selectedKey]?.document ?? document, error: readableError(error) }));
    } finally {
      if (documentController.current === controller) documentController.current = null;
      setDocumentPageLoading((current) => current === requestKey ? null : current);
    }
  }, [document, reader, root, selectedDocumentIdentity, selectedKey, selectedPath]);
  const knownRevisions = useMemo<ContextKnownRevision[]>(() => Object.values(documents)
    .map((state) => state.document)
    .filter((candidate): candidate is ContextDocument => candidate !== undefined && candidate.root_id === activeRootId)
    .map((candidate) => ({ path: candidate.path, revision: candidate.revision }))
    .sort((left, right) => left.path.localeCompare(right.path)), [activeRootId, documents]);
  useEffect(() => {
    if (!reader || !root || !selectedPath || !selectedKey) return;
    documentController.current?.abort();
    const requestId = ++documentRequestSequence.current;
    const controller = new AbortController();
    documentController.current = controller;
    const requestIdentity = identityKey;
    const requestBindingId = bindingId;
    const requestRootId = root.root_id;
    const requestDocumentIdentity = selectedDocumentIdentity;
    setDocumentPageLoading(null);
    setDocuments((current) => retainDocumentState(current, selectedKey, { status: "loading", document: current[selectedKey]?.document }));
    const request: ContextDocumentRead = { root_id: requestRootId, path: selectedPath, expected_revision: selectedRevision };
    void reader.document(request, controller.signal).then((data) => {
      if (!mountedRef.current || controller.signal.aborted || requestId !== documentRequestSequence.current
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId
        || selectedDocumentIdentityRef.current !== requestDocumentIdentity) return;
      setDocuments((current) => retainDocumentState(current, selectedKey, { status: "ready", document: data }));
    }).catch((error: unknown) => {
      if (!mountedRef.current || controller.signal.aborted || requestId !== documentRequestSequence.current
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId
        || selectedDocumentIdentityRef.current !== requestDocumentIdentity) return;
      setDocuments((current) => retainDocumentState(current, selectedKey, { status: "error", document: current[selectedKey]?.document, error: readableError(error) }));
    });
    return () => {
      controller.abort();
      if (documentController.current === controller) documentController.current = null;
      documentRequestSequence.current += 1;
    };
  }, [bindingId, identityKey, reader, refreshGeneration, selectedDocumentIdentity, selectedKey, selectedPath, selectedRevision, root?.root_id]);
  return { documents, setDocuments, documentState, document, documentPageLoading, loadDocumentPage, knownRevisions };
}
