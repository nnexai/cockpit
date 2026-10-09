import type { CSSProperties, KeyboardEvent, ReactNode, RefObject } from "react";
import type { ContextRoot, ProjectDiagnostic } from "../../protocol/generated/v1";
import type { ContextFileViewState } from "./ContextViewer";
import { TreeSplitter, type TreeLayout } from "../viewer/ViewerLayout";
import { isEditingTarget } from "./viewerState";
import { viewerShortcutAction, type ShortcutId } from "../input/shortcuts";
export interface ViewerModeSlots {
  className: string; treeLabel: string; toolbar: ReactNode; treeLead?: ReactNode;
  onTreeKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  document: ReactNode; status?: ReactNode;
}
export interface ViewerToolbarProps {
  overview: { open: boolean; toggle: () => void }; overviewId: string;
  openFilePicker: () => void; context: boolean; resourcesOpen: boolean; setResourcesOpen: (open: boolean) => void;
  presentable: boolean; selectedFileState: ContextFileViewState | undefined;
  updateFile: (patch: Partial<ContextFileViewState>) => void; wrap: boolean; toggleWrap: () => void;
  refresh: () => void; sourceShown: boolean;
}
export function ContextViewerFrame({ view, root, roots, chooseRoot, viewerRef, treeRef, documentRef,
  overview, overviewId, tree, layout, discoveryDiagnostics, linkNotice, search, directoryRows, onShortcut, children }: {
  view: ViewerModeSlots; root: ContextRoot; roots: ContextRoot[]; chooseRoot: (root: ContextRoot) => void;
  viewerRef: RefObject<HTMLElement | null>; treeRef: RefObject<HTMLElement | null>; documentRef: RefObject<HTMLElement | null>;
  overview: { open: boolean; narrow: boolean; close: () => void }; overviewId: string;
  tree: { width: number; setWidth: (width: number, persist?: boolean) => void; style: CSSProperties }; layout: TreeLayout;
  discoveryDiagnostics: ProjectDiagnostic[]; linkNotice: string | null; search: ReactNode; directoryRows: ReactNode;
  onShortcut: (action: ShortcutId) => void; children: ReactNode;
}) {
  return <section className={`context-viewer${view.className}`} aria-label="Context file viewer" ref={viewerRef} onKeyDownCapture={(event) => {
    if (!(event.target instanceof Node) || !event.currentTarget.contains(event.target) || isEditingTarget(event.target)) return;
    const action = viewerShortcutAction(event);
    if (action === null) return;
    event.preventDefault();
    if (action === "open-file-picker") event.stopPropagation();
    onShortcut(action);
  }}>
    <header className="context-toolbar">
        {roots.length > 1 ? <label className="context-root-select"><span className="sr-only">Context root</span><select value={root.root_id} onChange={(event) => { const next = roots.find((candidate) => candidate.root_id === event.target.value); if (next) chooseRoot(next); }}>{roots.map((candidate) => <option value={candidate.root_id} key={candidate.root_id}>{candidate.label}</option>)}</select></label> : null}
      {view.toolbar}
    </header>
      {discoveryDiagnostics.length > 0 ? <div className="context-notice context-notice-warning" role="status">{discoveryDiagnostics.map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : null}
      {linkNotice ? <div className="context-notice context-notice-warning" role="status">{linkNotice}</div> : null}
      <div className={`context-body${overview.open ? " has-file-overview" : ""}`} style={tree.style}>
        {overview.narrow && overview.open ? <button type="button" className="viewer-overview-backdrop" aria-label="Close file overview" onClick={overview.close} /> : null}
      <aside id={overviewId} className={`context-tree${overview.open ? " is-overview-open" : ""}`} aria-label={view.treeLabel} tabIndex={-1} ref={treeRef} onKeyDown={(event) => {
        if (event.key === "Escape" && overview.narrow) {
          event.preventDefault(); event.stopPropagation(); overview.close(); documentRef.current?.focus();
        } else view.onTreeKeyDown?.(event);
      }}>
        {search}{view.treeLead}{directoryRows}
      </aside>
      {overview.open && !overview.narrow ? <TreeSplitter width={tree.width} onChange={tree.setWidth} layout={layout} /> : null}
      <main className="context-document" ref={documentRef} tabIndex={-1}>{view.document}</main>
    </div>
    {view.status}{children}
  </section>;
}
