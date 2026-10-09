import type { DocumentHeaderFacts, DocumentViewProps } from "../context/DocumentView";
import { DocumentView } from "../context/DocumentView";
import { UiIcon } from "../UiIcon";
import { splitSourceLines } from "../context/sourceLines";
import { LibraryItemHeader, LibraryAttachmentNotice } from "./LibraryItemHeader";
import { LibraryDetails } from "./LibraryDetails";
import { attachmentPath } from "./LibraryTree";
import { copyText } from "./clipboard";
import type { LibraryViewerController } from "./useLibraryViewerController";
import type { LibraryViewerSource } from "./useLibraryViewerSource";
export function LibraryDocumentView({ controller, source, narrow, ...props }: DocumentViewProps & {
  controller: LibraryViewerController; source: LibraryViewerSource; narrow: boolean;
}) {
  const { selectedLibraryItem, pendingItemIds, libraryActions, setLibraryConfirm, itemSpace,
    noticeItem, noticeAttachment, attachmentActions, setLibraryAdd } = controller;
  const { library } = source;
  const { context, root, selectedPath } = props;
  let emptyContent;
  if (!library.listing && library.status === "error") emptyContent = null;
  else if (!library.listing) emptyContent = <div className="context-empty">Loading…</div>;
  else if (library.listing.items.length === 0) emptyContent = <div className="context-empty"><div className="context-empty-message"><strong>The Library is empty</strong><span>Add an issue, merge request, pull request, Jira issue, Confluence page, or a folder. The Library keeps it without a Space or session.</span><button type="button" onClick={() => setLibraryAdd("library")}>Add…</button></div></div>;
  else emptyContent = <div className="context-empty library-empty-select"><span className="library-empty-select-badge"><UiIcon name="library" /></span><span>Select an item to read it</span></div>;
  const noticeContent = noticeItem && noticeAttachment ? attachmentPath(noticeItem, noticeAttachment)
    ? <div className="context-notice library-attachment-notice"><span>Downloaded.</span><button type="button" onClick={() => attachmentActions.open(noticeItem, noticeAttachment)}>Open attachment</button></div>
    : <LibraryAttachmentNotice item={noticeItem} attachment={noticeAttachment} attachments={attachmentActions} /> : null;
  const renderHeader = selectedLibraryItem ? ({ document, metadata, facts, frontmatter }: DocumentHeaderFacts) => (
<LibraryItemHeader item={selectedLibraryItem} providers={library.providers} narrow={narrow} rootCrumb={context !== null} pending={pendingItemIds.has(selectedLibraryItem.item_id)} actions={libraryActions} onReplace={(item) => setLibraryConfirm({ kind: "replace", item })} details={<LibraryDetails item={selectedLibraryItem} providers={library.providers} now={Date.now()} document={{ bytes: document.bytes, contentHash: document.content_hash ?? null, mediaType: document.media_type, frontmatter: frontmatter ? splitSourceLines(document.text ?? "").slice(frontmatter.start - 1, frontmatter.end).map((line) => line.raw).join("") : null, diagnostics: document.diagnostics }} root={{ id: root.root_id, kind: root.kind, path: root.path, repositoryId: root.repository_id }} pageUpdate={{ at: metadata.lastModified, by: metadata.lastModifiedBy }} />} space={itemSpace(selectedLibraryItem)} pageUpdate={{ at: metadata.lastModified, by: metadata.lastModifiedBy }} facts={facts.generated ? facts : null} />
  ) : undefined;
  return <DocumentView {...props} renderHeader={renderHeader} emptyContent={emptyContent} noticeContent={noticeContent}
    hiddenTitle={selectedLibraryItem?.title ?? null} libraryItems={controller.libraryItems}
    relativePathAction={selectedPath ? <button type="button" onClick={() => void copyText(selectedPath)}>Copy Library path</button> : null} />;
}
