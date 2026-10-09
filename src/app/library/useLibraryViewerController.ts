import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject, type Dispatch, type SetStateAction } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextRoot, LibraryAttachment, LibraryItemSummary, LibraryRefreshRequest, LibraryListing } from "../../protocol/generated/v1";
import type { ContextViewState, LibraryCommand } from "../context/ContextViewer";
import { useLibraryFileRetention, type LibraryViewerSource } from "./useLibraryViewerSource";
import { useLibraryViewerOperations, type LibraryViewerOperations } from "./useLibraryViewerOperations";
import { useLibraryViewerSpace } from "./useLibraryViewerSpace";
import { useProviderCredentialActions, type ProviderCredentialActions } from "./useProviderCredentials";
import { announceLibraryChanged, LIBRARY_CHANGED_EVENT, type LibraryChangeDetail, type SpaceListingState } from "./useLibraryOperation";
import { attachmentPath, type LibraryAttachmentActions, type LibraryItemActions } from "./LibraryTree";
import type { ItemSpaceState } from "./LibraryItemHeader";
import type { LibrarySpace } from "./libraryState";
import { copyText } from "./clipboard";
import type { DocumentState } from "../context/useDocumentLoader";
export interface LibraryViewerController extends LibraryViewerOperations {
  providerCredentials: { actions: ProviderCredentialActions; dialog: ReactNode };
  spaceListing: SpaceListingState;
  displaySpace: LibrarySpace | null;
  itemSpace: (item: LibraryItemSummary) => ItemSpaceState | null;
  noticeItem: LibraryItemSummary | null | undefined;
  noticeAttachment: LibraryAttachment | undefined;
  selectedAttachmentId: string | null | undefined;
  selectedLibraryItem: LibraryItemSummary | null | undefined;
  libraryActions: LibraryItemActions;
  attachmentActions: LibraryAttachmentActions;
  refreshLibrary: () => void;
  startLibraryRefresh: (request: LibraryRefreshRequest, itemIds: string[]) => void;
  openLibraryItem: (item: LibraryItemSummary) => void;
  onLibraryLink: (item: LibraryItemSummary) => void;
  requestLibraryItem: (itemId: string) => void;
  compactToolbar: boolean;
  listing: LibraryListing | null;
  reload: (path: string, loadDirectory: (root: ContextRoot, path: string, force?: boolean) => Promise<void>) => void;
}
export function useLibraryViewerController({ client, source, selectedPath, value, onChange, space, libraryCommand, revalidate, openFile, chooseRoot, viewerRef, setDocuments }: {
  client: CockpitClient; source: LibraryViewerSource; selectedPath: string | null;
  value: ContextViewState; onChange: (next: ContextViewState) => void; space: LibrarySpace | null;
  libraryCommand: LibraryCommand | null; revalidate: () => void;
  openFile: (path: string, revision: string | null) => void; chooseRoot: (root: ContextRoot) => void;
  viewerRef: RefObject<HTMLElement | null>;
  setDocuments: Dispatch<SetStateAction<Record<string, DocumentState>>>;
}) {
  const { library, isLibrary, globalLibrary, libraryRoot } = source;
  const operations = useLibraryViewerOperations(client, library, revalidate);
  const { setLibraryReportVerb, setLibraryReportDismissed, setLibraryPendingIds, setLibraryConfirm,
    setLibraryTreeNotice, startLibraryOperation, libraryBusy, libraryItems, attachmentNotice,
    setAttachmentNotice, attachmentBusy, attachmentRequest, attachmentOperation,
    setBeforeAttachments, setAttachmentRequest } = operations;
  const { spaceListing, displaySpace, itemSpace, spaceRemoving, spaceAdd } = useLibraryViewerSpace(client, space);
  const providerCredentials = useProviderCredentialActions(client, library.providers);
  const [libraryOpenRequest, setLibraryOpenRequest] = useState<string | null>(null);
  const handledLibraryCommand = useRef<number | null>(null);
  const noticeItem = isLibrary ? libraryItems?.find((item) => item.item_id === attachmentNotice?.itemId) : null;
  const noticeAttachment = noticeItem?.attachments.find((attachment) => attachment.attachment_id === attachmentNotice?.attachmentId);
  const selectedAttachmentId = noticeAttachment?.attachment_id ?? (isLibrary && selectedPath ? libraryItems?.flatMap((item) => item.attachments.filter((attachment) => attachmentPath(item, attachment) === selectedPath)).at(0)?.attachment_id : null);
  const selectedLibraryItem = isLibrary && selectedPath && !noticeAttachment ? libraryItems?.find((item) => item.document_path === selectedPath) ?? null : null;
  useEffect(() => { setAttachmentNotice(null); }, [selectedPath, isLibrary]);
  useEffect(() => {
    if (!isLibrary) return;
    // Selecting saved Library items does not change their documents or replace action focus.
    const changed = (event: Event) => {
      const kind = (event as CustomEvent<LibraryChangeDetail>).detail?.kind;
      // A probe publishes the listing first; its selected snapshot drives the
      // global read. Legacy/manual events still force a disk revalidation.
      if (kind === "space_add" || globalLibrary && kind === "snapshot_update") return;
      revalidate();
    };
    window.addEventListener(LIBRARY_CHANGED_EVENT, changed);
    return () => window.removeEventListener(LIBRARY_CHANGED_EVENT, changed);
  }, [globalLibrary, isLibrary, revalidate]);
  useLibraryFileRetention(source, selectedPath, value, onChange, setDocuments);
  const startLibraryRefresh = useCallback((request: LibraryRefreshRequest, itemIds: string[]) => {
    setLibraryReportVerb("Refresh");
    setLibraryReportDismissed(false);
    setLibraryPendingIds(new Set(itemIds));
    void startLibraryOperation(() => client.libraryRefresh(request));
  }, [client, startLibraryOperation]);
  const refreshLibrary = useCallback(() => {
    startLibraryRefresh({ scope: "all" }, (libraryItems ?? []).map((item) => item.item_id));
  }, [libraryItems, startLibraryRefresh]);
  const openLibraryItem = (item: LibraryItemSummary) => {
    setAttachmentNotice(null);
    if (item.document_path) openFile(item.document_path, null);
  };
  const attachmentActions: LibraryAttachmentActions = {
    busy: libraryBusy,
    active: attachmentBusy ? attachmentRequest : null,
    start: (item, action, attachmentIds) => {
      if (libraryBusy || attachmentIds.length === 0) return;
      const request = { item_id: item.item_id, attachment_ids: attachmentIds, action };
      setBeforeAttachments(undefined);
      setAttachmentRequest(request);
      void attachmentOperation.start(() => client.libraryAttachments(request));
    },
    open: (item, attachment) => {
      const path = attachmentPath(item, attachment);
      if (path) { setAttachmentNotice(null); openFile(path, null); }
      else setAttachmentNotice({ itemId: item.item_id, attachmentId: attachment.attachment_id });
    },
  };
  const libraryActions: LibraryItemActions = {
    attachments: attachmentActions,
    credentials: providerCredentials.actions,
    open: openLibraryItem,
    refresh: startLibraryRefresh,
    remove: (item) => setLibraryConfirm({ kind: "remove", item }),
    copyLink: (item) => {
      const link = item.source_url ?? item.original_url;
      if (link) void copyText(link);
    },
    canCopyLink: typeof navigator !== "undefined" && Boolean(navigator.clipboard),
    copyLibraryPath: (item) => { if (item.document_path) void copyText(item.document_path); },
    canCopyLibraryPath: typeof navigator !== "undefined" && Boolean(navigator.clipboard),
    refreshBusy: libraryBusy,
    spaceEntries: (item) => {
      const state = itemSpace(item);
      if (!state) return [];
      return [{ label: state.selected ? "Remove from Space" : "Add to Space", onSelect: state.selected ? state.onRemove : state.onAdd, disabled: state.busy || spaceRemoving !== null || spaceAdd.starting || spaceAdd.running }];
    },
    removeFollow: async (follow, mode) => {
      await client.libraryRemove({ mode, follow_id: follow.follow_id });
      // The open item goes only when this follow was all that held it.
      const heldOnlyByFollow = selectedLibraryItem !== null && selectedLibraryItem !== undefined && selectedLibraryItem.refs.length > 0 && selectedLibraryItem.refs.every((ref) => ref.kind === "follow" && ref.follow_id === follow.follow_id);
      if (mode === "follow" && heldOnlyByFollow) onChange({ ...value, path: null });
      announceLibraryChanged();
    },
    // Saving the item's own link again takes the "Already saved" path, which adds the `manual` reference.
    keep: (item) => {
      if (!item.source_url) return;
      const input = item.source_url;
      setLibraryReportVerb("Keep");
      setLibraryReportDismissed(false);
      setLibraryPendingIds(new Set([item.item_id]));
      void startLibraryOperation(() => client.libraryAdd({ input, provider_id: item.provider_id, reference_depth: 0, follow: false, follow_mode: null, download_attachments: false, refresh_existing: false, label: null, target: null }));
    },
  };
  // Palette commands and `Open in Library` wait until the listing can serve them.
  useEffect(() => {
    // The token dialog needs no listing: it opens even while the Library is unavailable.
    if (isLibrary && libraryCommand?.kind === "tokens" && handledLibraryCommand.current !== libraryCommand.token) {
      handledLibraryCommand.current = libraryCommand.token;
      providerCredentials.actions.open("");
    }
    if (!isLibrary || !library.listing) return;
    if (libraryCommand && libraryCommand.kind !== "tokens" && handledLibraryCommand.current !== libraryCommand.token) {
      if (libraryCommand.kind === "refresh") {
        handledLibraryCommand.current = libraryCommand.token;
        if (!libraryBusy) refreshLibrary();
      } else {
        const item = library.listing.items.find((candidate) => candidate.item_id === libraryCommand.itemId);
        if (item) { handledLibraryCommand.current = libraryCommand.token; openLibraryItem(item); }
      }
    }
    if (libraryOpenRequest) {
      const item = library.listing.items.find((candidate) => candidate.item_id === libraryOpenRequest);
      if (item) { setLibraryOpenRequest(null); openLibraryItem(item); }
    }
  });
  const [compactToolbar, setCompactToolbar] = useState(false);
  useLayoutEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || typeof ResizeObserver === "undefined") return;
    const measure = () => { const width = viewer.getBoundingClientRect().width; if (width > 0) setCompactToolbar(width <= 420); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(viewer);
    return () => observer.disconnect();
  }, []);
  const reload = (path: string, loadDirectory: (root: ContextRoot, path: string, force?: boolean) => Promise<void>) => {
    spaceListing.reload();
    if (isLibrary) library.reload();
    else void loadDirectory(source.root, path, true);
  };
  const requestLibraryItem = (itemId: string) => {
    setLibraryOpenRequest(itemId);
    if (!isLibrary) chooseRoot(libraryRoot);
  };
  const onLibraryLink = (item: LibraryItemSummary) => {
    if (isLibrary) openLibraryItem(item);
    else requestLibraryItem(item.item_id);
  };
  return { ...operations, providerCredentials, spaceListing, displaySpace, itemSpace,
    noticeItem, noticeAttachment, selectedAttachmentId, selectedLibraryItem,
    libraryActions, attachmentActions, refreshLibrary, startLibraryRefresh, openLibraryItem,
    requestLibraryItem, onLibraryLink, compactToolbar, listing: library.listing, reload };
}
