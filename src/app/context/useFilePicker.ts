import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ContextRoot, ContextIndexedFile } from "../../protocol/generated/v1";
import type { ContextReader } from "./contextSource";
import { getFileIndex, putFileIndex } from "../input/fileIndexCache";
import { prepareFileCandidates, type FileNavigationCandidate } from "../input/fileNavigation";

const PICKER_RETRY_LIMIT = 4;
const PICKER_RETRY_BASE_MS = 1_500;
const FILE_INDEX_WARM_MS = 30_000;
type PickerIndexState = {
  loading: boolean;
  incomplete: boolean;
  files: readonly ContextIndexedFile[];
  mayBeOutOfDate: boolean;
  failed: boolean;
  watchUnavailable: boolean;
};
const emptyIndex: PickerIndexState = { loading: false, incomplete: false, files: [], mayBeOutOfDate: false, failed: false, watchUnavailable: false };

export function useFilePicker({ reader, root, bindingId, identityKey }: {
  reader: ContextReader | null; root: ContextRoot; bindingId: string; identityKey: string;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const [visible, setVisible] = useState(() => document.visibilityState !== "hidden");
  const [pickerIndex, setPickerIndex] = useState<PickerIndexState>(emptyIndex);
  const refreshRef = useRef<(() => void) | null>(null);
  const retryWatchRef = useRef<(() => void) | null>(null);
  const identityRef = useRef(identityKey);
  identityRef.current = identityKey;
  const rootRef = useRef(root?.root_id);
  rootRef.current = root?.root_id;
  const pickerCandidates = useMemo(() => pickerIndex.files.map((file) => ({
    id: file.path, path: file.path, detail: file.bytes === null ? undefined : `${file.bytes} B`,
  } satisfies FileNavigationCandidate)), [pickerIndex.files]);
  const preparedPickerCandidates = useMemo(() => prepareFileCandidates(pickerCandidates), [pickerCandidates]);
  const closeFilePicker = useCallback(() => { setPickerOpen(false); setPickerIndex(emptyIndex); }, []);
  const openFilePicker = useCallback(() => { if (root && reader) setPickerOpen(true); }, [reader, root]);
  const refreshPicker = useCallback(() => refreshRef.current?.(), []);
  useEffect(() => {
    const changed = () => setVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", changed);
    return () => document.removeEventListener("visibilitychange", changed);
  }, []);
  const retryPickerWatch = useCallback(() => retryWatchRef.current?.(), []);

  useEffect(() => {
    if (!pickerOpen || !reader || !root || !visible) return undefined;
    const controller = new AbortController();
    const cacheKey = `${bindingId}\u0000${root.root_id}`;
    const cached = getFileIndex(cacheKey);
    let cancelled = false;
    let freshSettled = false;
    let watching = false;
    let running: Promise<void> | null = null;
    let dirty = false;
    let retryTimer: number | null = null;
    let slowTimer: number | null = null;
    setPickerIndex({ loading: true, incomplete: cached?.truncated ?? false, files: cached?.files ?? [], mayBeOutOfDate: Boolean(cached?.files.length), failed: false, watchUnavailable: false });
    const current = () => !cancelled && !controller.signal.aborted
      && identityRef.current === identityKey && rootRef.current === root.root_id;
    const samePaths = (left: readonly ContextIndexedFile[], right: readonly ContextIndexedFile[]) =>
      left.length === right.length && left.every((file, index) => file.path === right[index]?.path);
    void reader.fileIndex(root.root_id, "cached", controller.signal).then((result) => {
      if (!current() || freshSettled || result.state !== "cached") return;
      putFileIndex(cacheKey, result.files, result.truncated);
      setPickerIndex((previous) => ({ ...previous, incomplete: result.truncated,
        files: samePaths(previous.files, result.files) ? previous.files : result.files, mayBeOutOfDate: true }));
    }).catch(() => undefined);
    const fetchFresh = (attempt = 0): Promise<void> => {
      dirty = true;
      if (running) return running;
      if (retryTimer !== null) { window.clearTimeout(retryTimer); retryTimer = null; }
      running = (async () => {
        while (dirty && current()) {
          dirty = false;
          slowTimer = window.setTimeout(() => {
            slowTimer = null;
            if (current()) setPickerIndex((previous) => ({ ...previous, loading: false, mayBeOutOfDate: true, failed: true }));
          }, 10_000);
          try {
            const result = await reader.fileIndex(root.root_id, "fresh", controller.signal);
            if (!current()) return;
            if (result.state !== "fresh") throw new Error("File index response is not fresh.");
            freshSettled = true;
            putFileIndex(cacheKey, result.files, result.truncated);
            setPickerIndex((previous) => ({ ...previous, loading: false, incomplete: result.truncated,
              files: samePaths(previous.files, result.files) ? previous.files : result.files,
              mayBeOutOfDate: !watching, watchUnavailable: watching ? false : previous.watchUnavailable, failed: false }));
            attempt = 0;
          } catch {
            if (!current()) return;
            setPickerIndex((previous) => ({ ...previous, loading: false, mayBeOutOfDate: true, failed: true }));
            if (attempt < PICKER_RETRY_LIMIT) retryTimer = window.setTimeout(() => { retryTimer = null; void fetchFresh(attempt + 1); }, PICKER_RETRY_BASE_MS * 2 ** attempt);
          } finally {
            if (slowTimer !== null) { window.clearTimeout(slowTimer); slowTimer = null; }
          }
        }
      })().finally(() => { running = null; });
      return running;
    };
    const subscription = reader.watchIndex(root.root_id, async (event) => {
      if (event.kind === "unavailable") {
        watching = false;
        if (current()) setPickerIndex((previous) => ({ ...previous, watchUnavailable: true, mayBeOutOfDate: true }));
        if (!freshSettled) await fetchFresh();
        return;
      }
      if (event.kind === "ready") watching = true;
      await fetchFresh();
    }, () => {
      if (!current()) return;
      watching = false;
      setPickerIndex((previous) => ({ ...previous, watchUnavailable: true, mayBeOutOfDate: true }));
      if (!freshSettled) void fetchFresh();
    });
    refreshRef.current = () => { void fetchFresh(); };
    retryWatchRef.current = () => subscription.retry();
    return () => {
      cancelled = true;
      subscription.close();
      controller.abort();
      if (retryTimer !== null) window.clearTimeout(retryTimer);
      if (slowTimer !== null) window.clearTimeout(slowTimer);
      refreshRef.current = null;
      retryWatchRef.current = null;
    };
  }, [pickerOpen, bindingId, identityKey, reader, root, visible]);
  const warmRootId = root?.root_id;
  useEffect(() => {
    if (!warmRootId || !reader) return;
    const key = `${bindingId}\u0000${warmRootId}`;
    const controller = new AbortController();
    const warm = () => {
      if (document.visibilityState === "hidden") return;
      const known = getFileIndex(key);
      if (known && Date.now() - known.at < FILE_INDEX_WARM_MS) return;
      void Promise.resolve().then(() => reader.fileIndex(warmRootId, "fresh", controller.signal)).then((result) => {
        if (!controller.signal.aborted && result.state === "fresh") putFileIndex(key, result.files, result.truncated);
      }).catch(() => undefined);
    };
    warm();
    window.addEventListener("focus", warm);
    return () => { controller.abort(); window.removeEventListener("focus", warm); };
  }, [bindingId, reader, warmRootId]);
  useEffect(() => { closeFilePicker(); }, [identityKey, closeFilePicker]);
  return { pickerOpen, pickerIndex, pickerCandidates, preparedPickerCandidates, closeFilePicker, openFilePicker, refreshPicker, retryPickerWatch };
}
