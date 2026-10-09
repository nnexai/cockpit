import type { ContextRoot, ContextEntry } from "../../protocol/generated/v1";
import type { DirectoryState, ContextTreeRow } from "./useDirectoryTree";
import { UiIcon } from "../UiIcon";
import { keyFor } from "./viewerState";
import { isTreeRowEnabled } from "./useDirectoryTree";
export function ContextDirectoryRows({ root, directories, rootEmpty, treeRows, selectedPath, toggleDirectory, chooseEntry, loadDirectory }: {
  root: ContextRoot; directories: Record<string, DirectoryState>; rootEmpty: boolean; treeRows: ContextTreeRow[];
  selectedPath: string | null; toggleDirectory: (entry: ContextEntry) => void; chooseEntry: (entry: ContextEntry) => void;
  loadDirectory: (root: ContextRoot, path: string, force?: boolean) => Promise<void>;
}) {
  return <>
          {directories[keyFor(root.root_id, "")]?.status === "loading" ? <div className="context-tree-status">Loading…</div> : null}
          {directories[keyFor(root.root_id, "")]?.status === "error" && !directories[keyFor(root.root_id, "")]?.data ? <div className="context-tree-error">{directories[keyFor(root.root_id, "")]?.error}</div> : null}
          {rootEmpty ? <div className="context-tree-status is-empty">Empty</div> : null}
          {treeRows.map((row) => <div className="context-tree-node" key={row.entry.entry_id}>
            <button type="button" data-context-path={row.path} className={`context-tree-row${selectedPath === row.path ? " is-selected" : ""}`} style={{ paddingLeft: `${8 + row.depth * 16}px` }} disabled={!isTreeRowEnabled(row)} onClick={() => row.entry.kind === "directory" ? toggleDirectory(row.entry) : chooseEntry(row.entry)} aria-label={`${row.label}${row.entry.refusal ? `, refused: ${row.entry.refusal}` : ""}`}>
              <span className="context-tree-disclosure">{row.entry.kind === "directory" ? <UiIcon name={row.open ? "down" : "right"} /> : null}</span>
              <span className="context-tree-icon" aria-hidden="true">{row.entry.kind === "directory" ? null : <UiIcon name="file" />}</span>
              <span className="context-tree-name" title={row.path}>{row.label}</span>
              <span className="context-tree-meta">{row.entry.refusal ?? ""}</span>
            </button>{row.entry.kind === "directory" && directories[keyFor(root.root_id, row.path)]?.data?.next_offset !== undefined ? <button type="button" className="context-tree-more" onClick={() => void loadDirectory(root, row.path)} aria-label={`Load more entries in ${row.path}`}>more</button> : null}
            {row.entry.refusal ? <div className="context-tree-refusal">{row.entry.refusal}</div> : null}
          </div>)}
  </>;
}
