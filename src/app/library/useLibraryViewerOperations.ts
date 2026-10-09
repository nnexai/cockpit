import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryAttachmentRequest, LibraryItemSummary, LibraryOperation } from "../../protocol/generated/v1";
import { useLibraryOperation, type LibraryListingState, type LibraryOperationState } from "./useLibraryOperation";
import type { LibraryTreeNotice } from "./LibraryTree";
const NO_PENDING_ITEMS: ReadonlySet<string> = new Set();
export interface LibraryViewerOperations {
  libraryPendingIds: ReadonlySet<string>;
  setLibraryPendingIds: Dispatch<SetStateAction<ReadonlySet<string>>>;
  libraryReportVerb: "Refresh" | "Replace" | "Keep";
  setLibraryReportVerb: Dispatch<SetStateAction<"Refresh" | "Replace" | "Keep">>;
  libraryReportDismissed: boolean;
  setLibraryReportDismissed: Dispatch<SetStateAction<boolean>>;
  dismissedLibraryError: string | null;
  setDismissedLibraryError: Dispatch<SetStateAction<string | null>>;
  libraryTreeNotice: LibraryTreeNotice | null;
  setLibraryTreeNotice: Dispatch<SetStateAction<LibraryTreeNotice | null>>;
  libraryAdd: "library" | "space" | null;
  setLibraryAdd: Dispatch<SetStateAction<"library" | "space" | null>>;
  libraryConfirm: { kind: "remove" | "replace"; item: LibraryItemSummary } | null;
  setLibraryConfirm: Dispatch<SetStateAction<{ kind: "remove" | "replace"; item: LibraryItemSummary } | null>>;
  libraryToolbarMenu: { x: number; y: number } | null;
  setLibraryToolbarMenu: Dispatch<SetStateAction<{ x: number; y: number } | null>>;
  attachmentRequest: LibraryAttachmentRequest | null;
  setAttachmentRequest: Dispatch<SetStateAction<LibraryAttachmentRequest | null>>;
  beforeAttachments: LibraryListingState["listing"] | undefined;
  setBeforeAttachments: Dispatch<SetStateAction<LibraryListingState["listing"] | undefined>>;
  attachmentNotice: { itemId: string; attachmentId: string } | null;
  setAttachmentNotice: Dispatch<SetStateAction<{ itemId: string; attachmentId: string } | null>>;
  libraryOperation: LibraryOperationState;
  attachmentOperation: LibraryOperationState;
  startLibraryOperation: (begin: () => Promise<LibraryOperation>) => Promise<LibraryOperation | null>;
  retryLibraryOperation: () => void;
  attachmentBusy: boolean;
  libraryBusy: boolean;
  pendingItemIds: Set<string>;
  libraryItems: LibraryItemSummary[] | undefined;
}
export function useLibraryViewerOperations(client: CockpitClient, library: LibraryListingState, revalidate: () => void) {
  const [libraryPendingIds, setLibraryPendingIds] = useState<ReadonlySet<string>>(NO_PENDING_ITEMS);
  const [libraryReportVerb, setLibraryReportVerb] = useState<"Refresh" | "Replace" | "Keep">("Refresh");
  const [libraryReportDismissed, setLibraryReportDismissed] = useState(false);
  const [dismissedLibraryError, setDismissedLibraryError] = useState<string | null>(null);
  const [libraryTreeNotice, setLibraryTreeNotice] = useState<LibraryTreeNotice | null>(null);
  useEffect(() => {
    if (library.status !== "error") setDismissedLibraryError(null);
  }, [library.status]);
  const [libraryAdd, setLibraryAdd] = useState<"library" | "space" | null>(null);
  const [libraryConfirm, setLibraryConfirm] = useState<{ kind: "remove" | "replace"; item: LibraryItemSummary } | null>(null);
  const [libraryToolbarMenu, setLibraryToolbarMenu] = useState<{ x: number; y: number } | null>(null);
  const libraryOperation = useLibraryOperation(client, () => {
    revalidate();
  });
  const libraryOperationId = libraryOperation.operation?.operation_id;
  // A newer refresh report replaces the follow notice rather than queueing behind it.
  useEffect(() => { setLibraryTreeNotice(null); }, [libraryOperationId]);
  const lastLibraryBegin = useRef<(() => Promise<LibraryOperation>) | null>(null);
  const beginLibraryOperation = libraryOperation.start;
  const startLibraryOperation = useCallback((begin: () => Promise<LibraryOperation>) => {
    lastLibraryBegin.current = begin;
    return beginLibraryOperation(begin);
  }, [beginLibraryOperation]);
  const retryLibraryOperation = () => {
    if (lastLibraryBegin.current) {
      setLibraryReportDismissed(false);
      void startLibraryOperation(lastLibraryBegin.current);
    }
  };
  const [attachmentRequest, setAttachmentRequest] = useState<LibraryAttachmentRequest | null>(null);
  const listingRef = useRef(library.listing);
  listingRef.current = library.listing;
  const [beforeAttachments, setBeforeAttachments] = useState<LibraryListingState["listing"] | undefined>(undefined);
  const attachmentOperation = useLibraryOperation(client, () => setBeforeAttachments(listingRef.current));
  const attachmentBusy = attachmentOperation.running || attachmentOperation.starting;
  const libraryBusy = libraryOperation.running || libraryOperation.starting || attachmentBusy;
  const pendingItemIds = useMemo(() => new Set([...libraryOperation.pendingItemIds, ...(libraryOperation.running || libraryOperation.starting ? libraryPendingIds : []), ...(attachmentBusy && attachmentRequest ? [attachmentRequest.item_id] : [])]), [libraryOperation.running, libraryOperation.starting, libraryOperation.pendingItemIds, libraryPendingIds, attachmentBusy, attachmentRequest]);
  const libraryItems = library.listing?.items;
  const [attachmentNotice, setAttachmentNotice] = useState<{ itemId: string; attachmentId: string } | null>(null);
  return { libraryPendingIds, setLibraryPendingIds, libraryReportVerb, setLibraryReportVerb,
    libraryReportDismissed, setLibraryReportDismissed, dismissedLibraryError, setDismissedLibraryError,
    libraryTreeNotice, setLibraryTreeNotice, libraryAdd, setLibraryAdd, libraryConfirm, setLibraryConfirm,
    libraryToolbarMenu, setLibraryToolbarMenu, libraryOperation, startLibraryOperation, retryLibraryOperation,
    attachmentRequest, setAttachmentRequest, beforeAttachments, setBeforeAttachments, attachmentOperation,
    attachmentBusy, libraryBusy, pendingItemIds, libraryItems, attachmentNotice, setAttachmentNotice };
}
