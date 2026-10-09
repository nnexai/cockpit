import type { ReactNode, KeyboardEvent } from "react";
import { UiIcon } from "../UiIcon";
import type { ViewerToolbarProps, ViewerModeSlots } from "./ContextViewerFrame";
function FilesToolbar({ overview, overviewId, openFilePicker, context, resourcesOpen, setResourcesOpen,
  presentable, selectedFileState, updateFile, wrap, toggleWrap, refresh }: ViewerToolbarProps) {
  return <>
        <button type="button" className="viewer-overview-trigger" onClick={overview.toggle} aria-expanded={overview.open} aria-controls={overviewId} aria-label="Toggle file overview"><UiIcon name="sidebar" /> Files</button>
        <button type="button" className="viewer-file-picker-trigger" onClick={openFilePicker} aria-label="Choose Context file" title="Choose Context file"><UiIcon name="search" /></button>
        {context ? <button type="button" onClick={() => setResourcesOpen(true)} aria-expanded={resourcesOpen}>Resources</button> : null}
        <span className="context-toolbar-spacer" />
        {presentable ? <div className="viewer-segmented" role="group" aria-label="Document presentation"><button type="button" aria-pressed={selectedFileState?.mode !== "source"} onClick={() => updateFile({ mode: "auto" })}>Preview</button><button type="button" aria-pressed={selectedFileState?.mode === "source"} onClick={() => updateFile({ mode: "source" })}>Source</button></div> : null}

        <button type="button" className="viewer-wrap-toggle" aria-pressed={wrap} onClick={toggleWrap} title={wrap ? "Long lines wrap (Alt+Z)" : "Long lines scroll (Alt+Z)"}><UiIcon name="wrap" /><span className="viewer-wrap-label">Wrap</span></button>
        <button type="button" onClick={refresh} aria-label="Refresh Context files" title="Refresh files"><UiIcon name="refresh" /></button>
  </>;
}
export function filesViewSlots({ toolbar, document, onTreeKeyDown }: {
  toolbar: ViewerToolbarProps; document: ReactNode; onTreeKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
}): ViewerModeSlots {
  return { className: "", treeLabel: "Context files", toolbar: <FilesToolbar {...toolbar} />, document, onTreeKeyDown };
}
