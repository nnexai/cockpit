import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ContextRoot, ProjectDiagnostic, LibraryItemSummary } from "../../protocol/generated/v1";
import type { ContextViewState, ContextViewerProps } from "../context/ContextViewer";
import type { DocumentState } from "../context/useDocumentLoader";
import type { Dispatch, SetStateAction } from "react";
import { LIBRARY_ROOT_ID, libraryReader, viewerReader, type ContextReader } from "../context/contextSource";
import { keyFor } from "../context/viewerState";
import { useLibraryListing, type LibraryListingState } from "./useLibraryOperation";
import { LIBRARY_TREE, VIEWER_TREE, type TreeLayout } from "../viewer/ViewerLayout";
export interface LibraryViewerSource {
  rootId: string | null;
  setRootId: Dispatch<SetStateAction<string | null>>;
  library: LibraryListingState;
  libraryRoot: ContextRoot;
  roots: ContextRoot[];
  root: ContextRoot;
  isLibrary: boolean;
  boundLibrary: boolean;
  reader: ContextReader | null;
  bindingId: string;
  identityKey: string;
  globalLibrary: boolean;
  layout: TreeLayout;
  directoryEnabled: boolean;
  commentsEnabled: boolean;
  latestRevisionOnly: boolean;
  allowRaster: boolean;
  rootDiagnostics: ProjectDiagnostic[];
}
export interface LibraryPrimarySelection {
  requestedPath: string | null;
  selectedPath: string | null;
  selectedSnapshotRevision: string | null;
  primarySelectionRef: { current: { identity: string; path: string; itemId: string } | null };
}
/** Provider items own their document and `_files/*`, not child items in their directory. */
function libraryItemHolds(item: LibraryItemSummary, path: string): boolean {
  const itemRoot = item.item_path.replace(/\/+$/, "");
  const attachmentPrefix = `${itemRoot}/_files/`;
  const relativeAttachment = path.startsWith(attachmentPrefix) ? path.slice(attachmentPrefix.length) : "";
  return item.document_path === path
    || (item.folder !== null && path.startsWith(`${itemRoot}/`))
    || (item.folder === null && relativeAttachment !== "" && !relativeAttachment.includes("/"));
}

export function useLibraryViewerSource({ client, context, value, onViewerError, library: viewLibrary }: ContextViewerProps) {
  const [rootId, setRootId] = useState<string | null>(value.rootId ?? (context ? context.default_root_id ?? context.roots[0]?.root_id ?? null : LIBRARY_ROOT_ID));
  const mountedRef = useRef(true);
  useEffect(() => { mountedRef.current = true; return () => { mountedRef.current = false; }; }, []);
  // Viewer-issued Library roots keep their same-tab authority; only the global Library is unbound.
  const libraryChosen = context === null || rootId === LIBRARY_ROOT_ID || (context.roots.find((candidate) => candidate.root_id === rootId) ?? context.roots[0])?.kind === "library";
  const needsLibraryLookup = libraryChosen;
  const viewerLibrary = useLibraryListing(client, viewLibrary === undefined && needsLibraryLookup);
  const library = viewLibrary ?? viewerLibrary;
  const serverLibraryRoot = library.listing?.root ?? null;
  const issuedLibraryRoot = context?.roots.find((candidate) => candidate.kind === "library") ?? null;
  const libraryPath = serverLibraryRoot?.path ?? "";
  const libraryRoot = useMemo<ContextRoot>(() => issuedLibraryRoot ?? ({ root_id: LIBRARY_ROOT_ID, kind: "library", label: "Library", path: libraryPath, repository_id: "", checkout_path: "" }), [issuedLibraryRoot, libraryPath]);
  const roots = useMemo(() => context ? [...context.roots.filter((candidate) => candidate.kind !== "library"), libraryRoot] : [libraryRoot], [libraryRoot, context]);
  const root: ContextRoot = libraryChosen || !context ? libraryRoot : context.roots.find((candidate) => candidate.root_id === rootId) ?? context.roots[0];
  const isLibrary = root?.kind === "library";
  const boundLibrary = Boolean(isLibrary && context?.roots.some((candidate) => candidate.kind === "library" && candidate.root_id === root.root_id));
  const serverLibraryRootId = serverLibraryRoot?.root_id ?? null;
  const sessionId = context?.session_id ?? null;
  const viewerId = context?.viewer_id ?? null;
  const viewerBindingId = context?.binding_id ?? null;
  const viewerErrorRef = useRef({ sessionId, viewerId, viewerBindingId, onViewerError });
  viewerErrorRef.current = { sessionId, viewerId, viewerBindingId, onViewerError };
  // Readers follow authority identity, not each refreshed listing or context object.
  const reader = useMemo<ContextReader | null>(() => {
    if (isLibrary && !boundLibrary) return serverLibraryRoot ? libraryReader(client, serverLibraryRoot) : null;
    return context ? viewerReader(client, context, (error) => {
      const current = viewerErrorRef.current;
      if (mountedRef.current && current.sessionId === sessionId && current.viewerId === viewerId && current.viewerBindingId === viewerBindingId) current.onViewerError?.(error);
    }) : null;
  }, [client, isLibrary, boundLibrary, serverLibraryRootId, sessionId, viewerId, viewerBindingId]);
  const bindingId = isLibrary && !boundLibrary ? "library" : viewerBindingId ?? "";
  const activeRootId = root?.root_id ?? "";
  const identityKey = `${reader?.identity ?? "pending"}\u0000${activeRootId}`;
  return { rootId, setRootId, library, libraryRoot, roots, root, isLibrary, boundLibrary,
    reader, bindingId, identityKey, globalLibrary: context === null && isLibrary,
    layout: isLibrary ? LIBRARY_TREE : VIEWER_TREE,
    directoryEnabled: !isLibrary, commentsEnabled: Boolean(context && root && (!isLibrary || boundLibrary)),
    latestRevisionOnly: isLibrary, allowRaster: root?.kind === "folder" || isLibrary,
    rootDiagnostics: isLibrary ? library.listing?.diagnostics ?? [] : context?.diagnostics ?? [] };
}

export function useLibraryPrimarySelection(source: LibraryViewerSource, value: ContextViewState) {
  const { root, identityKey, globalLibrary, library } = source;
  const requestedPath = value.rootId === root?.root_id ? value.path : null;
  const primarySelectionRef = useRef<{ identity: string; path: string; itemId: string } | null>(null);
  const previousPrimary = primarySelectionRef.current;
  const primaryLibraryItem = globalLibrary && requestedPath
    ? previousPrimary?.identity === identityKey && previousPrimary.path === requestedPath
      ? library.listing?.items.find((item) => item.item_id === previousPrimary.itemId)
      : library.listing?.items.find((item) => item.document_path === requestedPath)
    : undefined;
  // Only a primary document follows its stable item identity. Arbitrary files and
  // attachments retain their path identity, including the existing removal rules.
  const trackingPrimary = globalLibrary && previousPrimary?.identity === identityKey && previousPrimary.path === requestedPath;
  const selectedPath = trackingPrimary && library.listing
    ? primaryLibraryItem?.document_path ?? null
    : requestedPath;
  const selectedSnapshotRevision = primaryLibraryItem?.revision ?? null;
  useLayoutEffect(() => {
    primarySelectionRef.current = globalLibrary && requestedPath && primaryLibraryItem
      ? { identity: identityKey, path: requestedPath, itemId: primaryLibraryItem.item_id }
      : null;
  }, [globalLibrary, identityKey, primaryLibraryItem, requestedPath]);
  return { requestedPath, selectedPath, selectedSnapshotRevision, primarySelectionRef };
}

export function useLibrarySelectionFreshness(source: LibraryViewerSource, selection: LibraryPrimarySelection, value: ContextViewState,
  onChange: (next: ContextViewState) => void, setDocuments: Dispatch<SetStateAction<Record<string, DocumentState>>>) {
  const { root, globalLibrary, library } = source;
  const { requestedPath, selectedPath } = selection;
  useEffect(() => {
    if (!globalLibrary || library.status !== "ready" || requestedPath === selectedPath) return;
    const files = { ...value.files };
    if (root && requestedPath) {
      const oldKey = keyFor(root.root_id, requestedPath);
      setDocuments((current) => {
        const next = { ...current };
        delete next[oldKey];
        return next;
      });
    }
    if (root && requestedPath && selectedPath) {
      const oldKey = keyFor(root.root_id, requestedPath);
      const newKey = keyFor(root.root_id, selectedPath);
      const previous = files[oldKey];
      delete files[oldKey];
      if (previous) files[newKey] = { ...previous, path: selectedPath };
    }
    onChange({ ...value, path: selectedPath, files });
  }, [globalLibrary, library.status, onChange, requestedPath, root, selectedPath, value]);
}

export function useLibraryFileRetention(source: LibraryViewerSource, selectedPath: string | null, value: ContextViewState,
  onChange: (next: ContextViewState) => void, setDocuments: Dispatch<SetStateAction<Record<string, DocumentState>>>) {
  const { root, isLibrary, boundLibrary, library } = source;
  useEffect(() => {
    if (!isLibrary || boundLibrary || library.status !== "ready" || !library.listing || !selectedPath) return;
    if (library.listing.items.some((item) => libraryItemHolds(item, selectedPath))) return;
    const key = keyFor(root.root_id, selectedPath);
    setDocuments((current) => {
      const next = { ...current };
      delete next[key];
      return next;
    });
    onChange({ ...value, path: null });
  }, [isLibrary, boundLibrary, root, library.listing, library.status, onChange, selectedPath, value]);
}
