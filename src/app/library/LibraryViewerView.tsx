import type { DocumentViewProps } from "../context/DocumentView";
import type { ViewerModeSlots, ViewerToolbarProps } from "../context/ContextViewerFrame";
import type { LibraryViewerController } from "./useLibraryViewerController";
import type { LibraryViewerSource } from "./useLibraryViewerSource";
import { LibraryDocumentView } from "./LibraryDocumentView";
import { LibraryToolbar } from "./LibraryToolbar";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";
import { LibraryTree } from "./LibraryTree";
import { AttachmentReport } from "./LibraryItemHeader";
import { RefreshReport } from "./RefreshReport";
function LibraryTreePane({ source, controller }: { source: LibraryViewerSource; controller: LibraryViewerController }) {
  const { library } = source;
  const { setLibraryAdd, selectedLibraryItem, selectedAttachmentId, pendingItemIds, libraryActions, setLibraryTreeNotice } = controller;
  return <>
            {!library.listing && library.status !== "error" ? <div className="library-skeleton" role="status" aria-label="Loading Library">{[0, 1, 2, 3, 4, 5].map((index) => <span key={index} className="library-skeleton-row" style={{ width: `${[72, 58, 64, 48, 60, 52][index]}%` }} />)}</div> : null}
            {library.listing?.items.length === 0 ? <div className="library-tree-empty">
              <span>Nothing in the Library yet</span>
              <button type="button" className="library-button is-primary" onClick={() => setLibraryAdd("library")}><UiIcon name="plus" />Add…</button>
            </div> : null}
            {library.listing ? <LibraryTree items={library.listing.items} follows={library.listing.follows} providers={library.providers} selectedItemId={selectedLibraryItem?.item_id ?? null} selectedAttachmentId={selectedAttachmentId} pendingItemIds={pendingItemIds} actions={libraryActions} onNotice={setLibraryTreeNotice} /> : null}
  </>;
}
function LibraryStatusArea({ source, controller }: { source: LibraryViewerSource; controller: LibraryViewerController }) {
  const { library } = source;
  const { dismissedLibraryError, setDismissedLibraryError, attachmentRequest, attachmentOperation,
    libraryItems, beforeAttachments, setAttachmentRequest, attachmentActions, libraryOperation,
    libraryReportDismissed, libraryReportVerb, setLibraryReportDismissed, openLibraryItem,
    startLibraryRefresh, retryLibraryOperation, libraryTreeNotice, setLibraryTreeNotice } = controller;
  return <div className="library-status-area">
        {library.status === "error" && library.error !== dismissedLibraryError ? <ErrorSlot placement="pane"
          message={`Library unavailable: ${library.error}. Space context is unaffected.`}
          actions={<><button type="button" onClick={library.reload}>Retry</button><button type="button" onClick={() => setDismissedLibraryError(library.error)}>Dismiss</button></>} />
        : attachmentRequest ? <AttachmentReport request={attachmentRequest} operation={attachmentOperation.operation} starting={attachmentOperation.starting} error={attachmentOperation.error}
          item={library.status === "error" ? null : libraryItems?.find((item) => item.item_id === attachmentRequest.item_id) ?? null}
          settled={beforeAttachments !== undefined && (library.status === "error" || (library.status === "ready" && library.listing !== beforeAttachments))}
          onCancel={attachmentOperation.cancel} onDismiss={() => setAttachmentRequest(null)}
          onRetry={(ids) => { const item = libraryItems?.find((item) => item.item_id === attachmentRequest.item_id); if (item) attachmentActions.start(item, attachmentRequest.action, ids); }} />
        : libraryOperation.operation && !libraryReportDismissed ? <RefreshReport operation={libraryOperation.operation} verb={libraryReportVerb} error={libraryOperation.error}
          onCancel={libraryOperation.cancel} onDismiss={() => setLibraryReportDismissed(true)}
          onOpenItem={(itemId) => { const item = libraryItems?.find((candidate) => candidate.item_id === itemId); if (item) openLibraryItem(item); }}
          onRetry={(itemIds) => startLibraryRefresh({ scope: "items", item_ids: itemIds }, itemIds)}
          onRetryOperation={retryLibraryOperation}
          onRetryFollow={(followId) => startLibraryRefresh({ scope: "follow", follow_id: followId }, [])} />
        : libraryTreeNotice ? <ErrorSlot placement="pane" error={libraryTreeNotice.failed} message={libraryTreeNotice.text}
          actions={<button type="button" onClick={() => setLibraryTreeNotice(null)}>Dismiss</button>} />
        : <ErrorSlot placement="pane" message={libraryOperation.error} actions={libraryOperation.error ? <>
          <button type="button" onClick={retryLibraryOperation}>Retry</button>
          <button type="button" onClick={libraryOperation.reset}>Dismiss</button>
        </> : null} />}
  </div>;
}
export function libraryViewSlots({ source, controller, toolbar, document, narrow }: {
  source: LibraryViewerSource; controller: LibraryViewerController;
  toolbar: ViewerToolbarProps; document: DocumentViewProps; narrow: boolean;
}): ViewerModeSlots {
  return { className: " is-library", treeLabel: "Library items",
    toolbar: <LibraryToolbar {...toolbar} controller={controller} />,
    treeLead: <LibraryTreePane source={source} controller={controller} />,
    document: <LibraryDocumentView {...document} source={source} controller={controller} narrow={narrow} />,
    status: <LibraryStatusArea source={source} controller={controller} /> };
}
