import { useCallback, useRef } from "react";
import type { ContextRoot } from "../../protocol/generated/v1";
import type { ContextViewState, ContextFileViewState } from "./ContextViewer";
import { keyFor, MAX_RETAINED_FILE_STATES } from "./viewerState";
export function useViewerFiles({ root, selectedPath, value, onChange, narrow, clearPrimarySelection }: {
  root: ContextRoot; selectedPath: string | null; value: ContextViewState;
  onChange: (next: ContextViewState) => void; narrow: boolean; clearPrimarySelection: () => void;
}) {
  const latestRevisionKeysRef = useRef(new Set<string>());
  const selectedKey = root && selectedPath ? keyFor(root.root_id, selectedPath) : null;
  const selectedFileState = selectedKey ? value.files[selectedKey] : undefined;
  const openFile = (path: string, revision: string | null) => {
    if (!root) return;
    clearPrimarySelection();
    const fileKey = keyFor(root.root_id, path);
    if (revision === null) latestRevisionKeysRef.current.add(fileKey);
    else latestRevisionKeysRef.current.delete(fileKey);
    const next = value.files[fileKey] ?? { rootId: root.root_id, path, mode: "auto" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision };
    const files = { ...value.files };
    delete files[fileKey];
    files[fileKey] = next;
    const keys = Object.keys(files);
    while (keys.length > MAX_RETAINED_FILE_STATES) {
      const oldest = keys.shift();
      if (oldest === undefined) break;
      latestRevisionKeysRef.current.delete(oldest);
      delete files[oldest];
    }
    onChange({ ...value, rootId: root.root_id, path, files, overviewChoice: narrow ? false : value.overviewChoice });
  };
  const selectSearchResult = useCallback((result: { path: string; line: number; revision: string }) => {
    if (!root) return;
    const fileKey = keyFor(root.root_id, result.path);
    const files = { ...value.files, [fileKey]: {
      ...(value.files[fileKey] ?? {
        rootId: root.root_id,
        path: result.path,
        mode: "auto" as const,
        selectionStart: null,
        selectionEnd: null,
        scrollTop: 0,
      }),
      revision: result.revision,
      selectionStart: result.line,
      selectionEnd: result.line,
    } };
    onChange({ ...value, rootId: root.root_id, path: result.path, files });
  }, [onChange, root, value]);
  return { latestRevisionKeysRef, selectedKey, selectedFileState, openFile, selectSearchResult };
}

export function useSelectedFile({ root, selectedPath, selectedKey, selectedRevision, value, onChange }: {
  root: ContextRoot; selectedPath: string | null; selectedKey: string | null; selectedRevision: string | null;
  value: ContextViewState; onChange: (next: ContextViewState) => void;
}) {
  const updateFile = useCallback((patch: Partial<ContextFileViewState>) => {
    if (!root || !selectedPath || !selectedKey) return;
    const previous = value.files[selectedKey] ?? { rootId: root.root_id, path: selectedPath, mode: "source" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision: selectedRevision };
    const files = { ...value.files };
    delete files[selectedKey];
    files[selectedKey] = { ...previous, ...patch };
    const keys = Object.keys(files);
    while (keys.length > MAX_RETAINED_FILE_STATES) {
      const oldest = keys.shift();
      if (oldest === undefined) break;
      delete files[oldest];
    }
    onChange({ ...value, rootId: root.root_id, path: selectedPath, files });
  }, [onChange, root, selectedKey, selectedPath, value]);
  return updateFile;
}
