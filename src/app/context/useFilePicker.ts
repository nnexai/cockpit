import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ContextRoot, ContextIndexedFile } from "../../protocol/generated/v1";
import type { ContextReader } from "./contextSource";
import { getFileIndex, putFileIndex } from "../input/fileIndexCache";
import { prepareFileCandidates, type FileNavigationCandidate } from "../input/fileNavigation";
const PICKER_REVALIDATE_MS = 8_000;
const PICKER_REVALIDATE_MAX_COST_MS = 2_000;
const PICKER_RETRY_LIMIT = 4;
const PICKER_RETRY_BASE_MS = 1_500;
const FILE_INDEX_WARM_MS = 30_000;
type PickerIndexState = {
  loading: boolean;
  incomplete: boolean;
  files: readonly ContextIndexedFile[];
  mayBeOutOfDate: boolean;
  failed: boolean;
};

export function useFilePicker({ reader, root, bindingId, identityKey }: {
  reader: ContextReader | null; root: ContextRoot; bindingId: string; identityKey: string;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerIndex, setPickerIndex] = useState<PickerIndexState>({ loading: false, incomplete: false, files: [], mayBeOutOfDate: false, failed: false });
  const pickerCandidates = useMemo(() => pickerIndex.files.map((file) => ({
    id: file.path,
    path: file.path,
    detail: file.bytes === null ? undefined : `${file.bytes} B`,
  } satisfies FileNavigationCandidate)), [pickerIndex.files]);
  const preparedPickerCandidates = useMemo(() => prepareFileCandidates(pickerCandidates), [pickerCandidates]);
  const pickerController = useRef<AbortController | null>(null);
  const pickerTimeout = useRef<number | null>(null);
  const pickerGeneration = useRef(0);
  const requestIdentityRef = useRef(identityKey);
  requestIdentityRef.current = identityKey;
  const currentRootRef = useRef(root.root_id);
  currentRootRef.current = root.root_id;
  const closeFilePicker = useCallback(() => {
    pickerController.current?.abort();
    if (pickerTimeout.current !== null) window.clearTimeout(pickerTimeout.current);
    pickerTimeout.current = null;
    pickerController.current = null;
    pickerGeneration.current += 1;
    setPickerOpen(false);
    setPickerIndex({ loading: false, incomplete: false, files: [], mayBeOutOfDate: false, failed: false });
  }, []);
  const openFilePicker = useCallback(() => {
    if (!root || !reader) return;
    pickerController.current?.abort();
    const controller = new AbortController();
    if (pickerTimeout.current !== null) window.clearTimeout(pickerTimeout.current);
    pickerTimeout.current = null;
    pickerController.current = controller;
    const generation = ++pickerGeneration.current;
    const requestIdentity = identityKey;
    const cacheKey = `${bindingId}\u0000${root.root_id}`;
    let freshSettled = false;
    const cachedIndex = getFileIndex(cacheKey);
    const cachedFiles = cachedIndex?.files ?? [];
    setPickerOpen(true);
    setPickerIndex({ loading: true, incomplete: cachedIndex?.truncated ?? false, files: cachedFiles, mayBeOutOfDate: cachedFiles.length > 0, failed: false });
    const current = () => !controller.signal.aborted
      && pickerGeneration.current === generation
      && requestIdentityRef.current === requestIdentity
      && currentRootRef.current === root.root_id;
    const samePaths = (left: readonly ContextIndexedFile[], right: readonly ContextIndexedFile[]) =>
      left.length === right.length && left.every((file, index) => file.path === right[index]?.path);
    // A slow index only flips the status; the request keeps running and heals the list when it lands.
    pickerTimeout.current = window.setTimeout(() => {
      pickerTimeout.current = null;
      if (!current()) return;
      setPickerIndex((previous) => ({
        ...previous,
        loading: false,
        mayBeOutOfDate: previous.files.length > 0,
        failed: true,
      }));
    }, 10_000);
    void reader.fileIndex(root.root_id, "cached", controller.signal).then((result) => {
      if (!current() || freshSettled || result.state !== "cached") return;
      putFileIndex(cacheKey, result.files, result.truncated);
      setPickerIndex((previous) => ({
        loading: previous.failed ? false : true,
        incomplete: result.truncated,
        files: samePaths(previous.files, result.files) ? previous.files : result.files,
        mayBeOutOfDate: true,
        failed: previous.failed,
      }));
    }).catch(() => undefined);
    // Eventually consistent: failures retry with backoff, and an open picker revalidates quietly while
    // indexing stays cheap, so nobody has to reopen it to see new or removed files.
    const fetchFresh = (attempt: number) => {
      const started = performance.now();
      void reader.fileIndex(root.root_id, "fresh", controller.signal).then((result) => {
        if (!current()) return;
        if (pickerTimeout.current !== null) window.clearTimeout(pickerTimeout.current);
        pickerTimeout.current = null;
        if (result.state !== "fresh") throw new Error("File index response is not fresh.");
        freshSettled = true;
        putFileIndex(cacheKey, result.files, result.truncated);
        setPickerIndex((previous) => ({
          loading: false,
          incomplete: result.truncated,
          files: samePaths(previous.files, result.files) ? previous.files : result.files,
          mayBeOutOfDate: false,
          failed: false,
        }));
        if (performance.now() - started < PICKER_REVALIDATE_MAX_COST_MS) {
          window.setTimeout(() => { if (current()) fetchFresh(0); }, PICKER_REVALIDATE_MS);
        }
      }).catch(() => {
        if (!current()) return;
        setPickerIndex((previous) => ({
          ...previous,
          loading: false,
          mayBeOutOfDate: previous.files.length > 0,
          failed: true,
        }));
        if (attempt < PICKER_RETRY_LIMIT) {
          window.setTimeout(() => { if (current()) fetchFresh(attempt + 1); }, PICKER_RETRY_BASE_MS * 2 ** attempt);
        }
      });
    };
    fetchFresh(0);
  }, [bindingId, identityKey, reader, root]);
  // Keep the client-side list warm so the first picker open is instant and already current.
  const warmRootId = root?.root_id;
  useEffect(() => {
    if (!warmRootId || !reader) return;
    const key = `${bindingId}\u0000${warmRootId}`;
    const controller = new AbortController();
    const warm = () => {
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
  useEffect(() => () => {
    pickerController.current?.abort();
    pickerController.current = null;
    if (pickerTimeout.current !== null) window.clearTimeout(pickerTimeout.current);
    pickerTimeout.current = null;
    pickerGeneration.current += 1;
  }, []);
  return { pickerOpen, pickerIndex, pickerCandidates, preparedPickerCandidates, closeFilePicker, openFilePicker };
}
