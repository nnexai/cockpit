import { useCallback, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextInvalidation, ContextRoot } from "../../protocol/generated/v1";
import type { ContextViewState } from "./ContextViewer";
import type { ContextReader } from "./contextSource";
import type { DocumentState } from "./useDocumentLoader";
import { keyFor } from "./viewerState";
export function useViewerSearch({ root, reader, value, onChange, selectedPath, directoryPathForFile, loadDirectory, setDocuments }: {
  root: ContextRoot; reader: ContextReader | null; value: ContextViewState; onChange: (next: ContextViewState) => void;
  selectedPath: string | null; directoryPathForFile: string;
  loadDirectory: (root: ContextRoot, path: string, force?: boolean) => Promise<void>;
  setDocuments: Dispatch<SetStateAction<Record<string, DocumentState>>>;
}) {
  const [invalidationGeneration, setInvalidationGeneration] = useState(0);
  const searchContext = useCallback((request: Parameters<CockpitClient["contextSearch"]>[2], signal: AbortSignal) =>
    reader?.search ? reader.search(request, signal) : Promise.reject(new Error("Search is unavailable for this root.")), [reader]);
  const pollContext = useCallback((request: Parameters<CockpitClient["contextInvalidate"]>[2], signal: AbortSignal) =>
    reader?.invalidate ? reader.invalidate(request, signal) : Promise.reject(new Error("Change polling is unavailable for this root.")), [reader]);
  const invalidateVisibleFiles = useCallback((invalidations: ContextInvalidation[]) => {
    if (!root || invalidations.length === 0) return;
    const invalidated = new Set(invalidations.map((item) => item.path));
    const files = { ...value.files };
    let changed = false;
    for (const path of invalidated) {
      const key = keyFor(root.root_id, path);
      const state = files[key];
      if (!state) continue;
      const update = invalidations.find((item) => item.path === path);
      files[key] = {
        ...state,
        revision: update?.revision ?? state.revision,
        selectionStart: null,
        selectionEnd: null,
      };
      changed = true;
    }
    setDocuments((current) => {
      const next = { ...current };
      for (const path of invalidated) {
        const key = keyFor(root.root_id, path);
        const state = next[key];
        if (state?.document && state.status !== "error") {
          // A late invalidation must not hide the source read's actionable error.
          next[key] = {
            status: "error",
            document: state.document,
            error: "Source changed; the previous view is retained until it is refreshed.",
          };
        }
      }
      return next;
    });
    if (selectedPath && invalidated.has(selectedPath)) {
      void loadDirectory(root, directoryPathForFile, true);
    }
    if (changed) onChange({ ...value, files });
    setInvalidationGeneration((generation) => generation + 1);
  }, [directoryPathForFile, loadDirectory, onChange, root, selectedPath, value]);
  return { invalidationGeneration, searchContext, pollContext, invalidateVisibleFiles };
}
