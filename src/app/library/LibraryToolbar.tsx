import { UiIcon } from "../UiIcon";
import { ariaKeyShortcuts, withShortcut } from "../input/shortcuts";
import { menuAnchor } from "./LibraryTree";
import type { ViewerToolbarProps } from "../context/ContextViewerFrame";
import type { LibraryViewerController } from "./useLibraryViewerController";
export function LibraryToolbar({ controller, ...props }: ViewerToolbarProps & { controller: LibraryViewerController }) {
  const { overview, overviewId, openFilePicker, context, resourcesOpen, setResourcesOpen,
    presentable, selectedFileState, updateFile, wrap, toggleWrap, sourceShown } = props;
  const { compactToolbar, setLibraryAdd, displaySpace, libraryBusy, refreshLibrary,
    libraryToolbarMenu, setLibraryToolbarMenu } = controller;
  const refreshAllDisabled = libraryBusy || !controller.listing || controller.listing.items.length === 0;
  return <>
    <button type="button" className="library-ghost is-icon viewer-overview-trigger" aria-pressed={overview.open} aria-controls={overviewId} aria-label={overview.open ? "Hide file tree" : "Show file tree"} aria-keyshortcuts={ariaKeyShortcuts("focus-file-tree")} title={withShortcut(overview.open ? "Hide file tree" : "Show file tree", "focus-file-tree")} onClick={overview.toggle}><UiIcon name="sidebar" /></button>
    <button type="button" className="library-ghost is-icon viewer-file-picker-trigger" aria-label="Find in Library" aria-keyshortcuts={ariaKeyShortcuts("open-file-picker")} title={withShortcut("Find in Library", "open-file-picker", undefined, "chord")} onClick={openFilePicker}><UiIcon name="search" /></button>
    {context ? <button type="button" className="library-ghost" onClick={() => setResourcesOpen(true)} aria-expanded={resourcesOpen}>Resources</button> : null}
    {compactToolbar ? null : <>
      <span className="library-toolbar-rule" aria-hidden="true" />
      <button type="button" className="library-ghost" title="Add a page, issue, MR/PR or folder to the Library" onClick={() => setLibraryAdd(displaySpace?.live ? "space" : "library")}><UiIcon name="plus" />Add…</button>
      <button type="button" className="library-ghost" aria-disabled={refreshAllDisabled} onClick={() => { if (!refreshAllDisabled) refreshLibrary(); }} title="Refresh every item from its source"><UiIcon name="refresh" />Refresh all</button>
    </>}
    <span className="context-toolbar-spacer" />
    {presentable ? <div className="viewer-segmented" role="group" aria-label="Document presentation"><button type="button" aria-keyshortcuts={ariaKeyShortcuts("toggle-preview")} title={withShortcut("Preview", "toggle-preview")} aria-pressed={selectedFileState?.mode !== "source"} onClick={() => updateFile({ mode: "auto" })}>Preview</button><button type="button" aria-keyshortcuts={ariaKeyShortcuts("toggle-preview")} title={withShortcut("Source", "toggle-preview")} aria-pressed={selectedFileState?.mode === "source"} onClick={() => updateFile({ mode: "source" })}>Source</button></div> : null}
    {sourceShown ? <button type="button" className="library-ghost is-icon viewer-wrap-toggle" aria-pressed={wrap} aria-label="Wrap long lines" aria-keyshortcuts={ariaKeyShortcuts("toggle-wrap")} onClick={toggleWrap} title={withShortcut(wrap ? "Scroll long lines" : "Wrap long lines", "toggle-wrap")}><UiIcon name="wrap" /></button> : null}
    <button type="button" className="library-ghost is-icon library-overflow" aria-label="Library actions" title="Library actions" aria-haspopup="menu" aria-expanded={libraryToolbarMenu !== null} onClick={(event) => setLibraryToolbarMenu(menuAnchor(event.currentTarget))}><UiIcon name="more" /></button>
  </>;
}
