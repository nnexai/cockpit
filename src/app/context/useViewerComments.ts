import { useCallback, useEffect, useState, type RefObject } from "react";
import type { ContextRoot, ContextDocument } from "../../protocol/generated/v1";
import type { ContextViewState, ContextFileViewState } from "./ContextViewer";
import { keyFor } from "./viewerState";
export function useViewerComments({ root, document, selectedPath, selectedKey, selectedFileState, value, onChange, updateFile, documentRef }: {
  root: ContextRoot; document: ContextDocument | undefined; selectedPath: string | null; selectedKey: string | null;
  selectedFileState: ContextFileViewState | undefined; value: ContextViewState; onChange: (next: ContextViewState) => void;
  updateFile: (patch: Partial<ContextFileViewState>) => void; documentRef: RefObject<HTMLElement | null>;
}) {
  const [commentStatus, setCommentStatus] = useState({ count: 0, canCreateLines: false, canCreateWholeFile: false });
  const restoreSourceFocus = useCallback(() => {
    requestAnimationFrame(() => {
      const line = selectedFileState?.selectionEnd ?? selectedFileState?.selectionStart ?? 1;
      documentRef.current?.querySelector<HTMLButtonElement>(`[data-line="${line}"]`)?.focus();
    });
  }, [selectedFileState?.selectionEnd, selectedFileState?.selectionStart]);
  const updateCommentStatus = useCallback((next: { count: number; canCreateLines: boolean; canCreateWholeFile: boolean }) => {
    setCommentStatus((current) => current.count === next.count && current.canCreateLines === next.canCreateLines && current.canCreateWholeFile === next.canCreateWholeFile ? current : next);
  }, []);
  useEffect(() => {
    if (!selectedKey || !document || !selectedFileState || selectedFileState.revision === document.revision) return;
    updateFile({ revision: document.revision, selectionStart: null, selectionEnd: null, scrollTop: 0 });
  }, [document?.revision, selectedFileState, selectedKey, updateFile]);
  useEffect(() => {
    const editor = value.commentEditor;
    if (!root || !editor || editor.editor !== "lines" || editor.rootId !== root.root_id) return;
    if (selectedPath !== editor.path) {
      const editorKey = keyFor(root.root_id, editor.path);
      const current = value.files[editorKey] ?? { rootId: root.root_id, path: editor.path, mode: "source" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision: editor.revision };
      onChange({ ...value, rootId: root.root_id, path: editor.path, files: { ...value.files, [editorKey]: { ...current, mode: "source", selectionStart: editor.selection?.start ?? current.selectionStart, selectionEnd: editor.selection?.end ?? current.selectionEnd } } });
      return;
    }
    if (selectedFileState?.mode !== "source") updateFile({ mode: "source" });
  }, [onChange, root, selectedFileState?.mode, selectedPath, updateFile, value]);
  return { commentStatus, updateCommentStatus, restoreSourceFocus };
}
