import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type RefObject } from "react";
import type { ContextRoot, ContextEntry, ContextDirectory, ProjectDiagnostic } from "../../protocol/generated/v1";
import type { ContextReader, ContextDirectoryRead } from "./contextSource";
import { keyFor, readableError } from "./viewerState";
const MAX_RETAINED_DIRECTORY_STATES = 128;
const MAX_RETAINED_EXPANDED_DIRECTORIES = 64;
export type DirectoryState = {
  status: "idle" | "loading" | "ready" | "error";
  data?: ContextDirectory;
  error?: string;
};
function retainDirectoryState(current: Record<string, DirectoryState>, key: string, state: DirectoryState, protectedKeys: Set<string>): Record<string, DirectoryState> {
  const next = { ...current };
  delete next[key];
  next[key] = state;
  const keys = Object.keys(next);
  if (keys.length > MAX_RETAINED_DIRECTORY_STATES) {
    const oldest = keys.find((candidate) => candidate !== key && !protectedKeys.has(candidate)) ?? keys.find((candidate) => candidate !== key) ?? keys[0];
    if (oldest !== undefined) delete next[oldest];
  }
  return next;
}
export type ContextTreeRow = {
  entry: ContextEntry;
  path: string;
  depth: number;
  label: string;
  open: boolean;
};

export function isTreeRowEnabled(row: ContextTreeRow): boolean {
  return (row.entry.kind === "file" || row.entry.kind === "directory") && !row.entry.refusal;
}

function contextTreeRows(root: ContextRoot, directories: Record<string, DirectoryState>, expanded: Set<string>): ContextTreeRow[] {
  const rows: ContextTreeRow[] = [];
  const visit = (path: string, depth: number) => {
    const state = directories[keyFor(root.root_id, path)];
    if (!state?.data) return;
    for (const initial of state.data.entries) {
      let entry = initial;
      let entryPath = entry.path ?? (path ? `${path}/${entry.name}` : entry.name);
      let label = entry.name;
      // Compress only directories whose next segment is already loaded and open.
      // This never reads descendants merely to improve presentation.
      while (entry.kind === "directory" && expanded.has(entryPath)) {
        const child = directories[keyFor(root.root_id, entryPath)]?.data;
        if (!child || child.entries.length !== 1 || child.entries[0]?.kind !== "directory") break;
        entry = child.entries[0];
        entryPath = entry.path ?? `${entryPath}/${entry.name}`;
        label += `/${entry.name}`;
      }
      const open = entry.kind === "directory" && expanded.has(entryPath);
      rows.push({ entry, path: entryPath, depth, label, open });
      if (open) visit(entryPath, depth + 1);
    }
  };
  visit("", 0);
  return rows;
}

export function useDirectoryTree({ reader, root, bindingId, identityKey, selectedPath, treeRef, enabled, openFile }: {
  reader: ContextReader | null; root: ContextRoot; bindingId: string; identityKey: string;
  selectedPath: string | null; treeRef: RefObject<HTMLElement | null>; enabled: boolean;
  openFile: (path: string, revision: string | null) => void;
}) {
  const [directories, setDirectories] = useState<Record<string, DirectoryState>>({});
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const protectedDirectoryKeysRef = useRef<Set<string>>(new Set());
  const directoriesRef = useRef(directories);
  directoriesRef.current = directories;
  const directoryRequests = useRef<Record<string, number>>({});
  const directoryControllers = useRef<Record<string, AbortController>>({});
  const directoryRequestSequence = useRef(0);
  const treeFocusPathRef = useRef<{ path: string; restoreAfterLoad: boolean } | null>(null);
  const mountedRef = useRef(true);
  const requestIdentityRef = useRef(identityKey);
  requestIdentityRef.current = identityKey;
  const currentBindingRef = useRef(bindingId);
  currentBindingRef.current = bindingId;
  const currentRootRef = useRef(root.root_id);
  currentRootRef.current = root.root_id;
  useEffect(() => { mountedRef.current = true; return () => {
    mountedRef.current = false;
    for (const controller of Object.values(directoryControllers.current)) controller.abort();
    directoryControllers.current = {};
    directoryRequests.current = {};
  }; }, []);
  const directoryPathForFile = selectedPath?.includes("/") ? selectedPath.slice(0, selectedPath.lastIndexOf("/")) : "";
  const selectedDirectory = root ? directories[keyFor(root.root_id, directoryPathForFile)] : undefined;
  const selectedEntry = selectedDirectory?.data?.entries.find((entry) => (entry.path ?? (directoryPathForFile ? `${directoryPathForFile}/${entry.name}` : entry.name)) === selectedPath);
  const protectedDirectoryKeys = new Set<string>();
  if (root) {
    protectedDirectoryKeys.add(keyFor(root.root_id, ""));
    protectedDirectoryKeys.add(keyFor(root.root_id, directoryPathForFile));
    for (const path of expanded) protectedDirectoryKeys.add(keyFor(root.root_id, path));
  }
  protectedDirectoryKeysRef.current = protectedDirectoryKeys;
  const treeRows = useMemo(() => root ? contextTreeRows(root, directories, expanded) : [], [directories, expanded, root]);
  const loadDirectory = useCallback(async (directoryRoot: ContextRoot, path: string, force = false) => {
    if (!reader) return;
    const key = keyFor(directoryRoot.root_id, path);
    const previous = directoriesRef.current[key]?.data;
    if (!force && directoriesRef.current[key]?.status === "ready" && previous?.next_offset === undefined) return;
    directoryControllers.current[key]?.abort();
    const requestId = ++directoryRequestSequence.current;
    directoryRequests.current[key] = requestId;
    setDirectories((current) => retainDirectoryState(current, key, { status: "loading", data: current[key]?.data }, protectedDirectoryKeysRef.current));
    const controller = new AbortController();
    directoryControllers.current[key] = controller;
    const requestIdentity = identityKey;
    const requestBindingId = bindingId;
    const requestRootId = directoryRoot.root_id;
    try {
      const request: ContextDirectoryRead = {
        root_id: requestRootId,
        path,
        offset: force ? undefined : previous?.next_offset,
        revision: force ? undefined : previous?.revision,
      };
      const data = await reader.directory(request, controller.signal);
      if (!mountedRef.current || controller.signal.aborted || directoryRequests.current[key] !== requestId
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId) return;
      const merged = !force && previous?.revision !== undefined && data.revision === previous.revision
        ? { ...data, entries: [...previous.entries, ...data.entries] }
        : data;
      setDirectories((current) => retainDirectoryState(current, key, { status: "ready", data: merged }, protectedDirectoryKeysRef.current));
    } catch (error) {
      if (!mountedRef.current || controller.signal.aborted || directoryRequests.current[key] !== requestId
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId) return;
      setDirectories((current) => retainDirectoryState(current, key, { status: "error", data: current[key]?.data, error: readableError(error) }, protectedDirectoryKeysRef.current));
    } finally {
      if (directoryControllers.current[key] === controller) delete directoryControllers.current[key];
    }
  }, [bindingId, identityKey, reader]);
  const identityRef = useRef<string | null>(null);
  useEffect(() => {
    if (identityRef.current === identityKey) return;
    identityRef.current = identityKey;
    for (const controller of Object.values(directoryControllers.current)) controller.abort();
    directoryControllers.current = {};
    directoryRequests.current = {};
    directoriesRef.current = {};
    setDirectories({});
    setExpanded(new Set());
    if (enabled) void loadDirectory(root, "", true);
  }, [identityKey, enabled, loadDirectory]);
  const chooseEntry = (entry: ContextEntry) => {
    if (entry.kind !== "file" || entry.refusal || !entry.path) return;
    openFile(entry.path, entry.revision);
  };
  const toggleDirectory = (entry: ContextEntry) => {
    if (!root || !entry.path || entry.kind !== "directory") return;
    const next = new Set(expanded);
    if (next.has(entry.path)) next.delete(entry.path); else {
      next.add(entry.path);
      if (next.size > MAX_RETAINED_EXPANDED_DIRECTORIES) {
        const oldest = next.values().next().value;
        if (typeof oldest === "string") next.delete(oldest);
      }
      void loadDirectory(root, entry.path);
    }
    setExpanded(next);
  };
  const focusTreePath = (path: string, restoreAfterLoad = false) => {
    treeFocusPathRef.current = { path, restoreAfterLoad };
    requestAnimationFrame(() => {
      const request = treeFocusPathRef.current;
      if (!request || request.path !== path) return;
      const target = [...(treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-context-path]") ?? [])]
        .find((button) => button.dataset.contextPath === request.path);
      target?.focus();
      if (!request.restoreAfterLoad) treeFocusPathRef.current = null;
    });
  };
  useEffect(() => {
    const request = treeFocusPathRef.current;
    if (!request?.restoreAfterLoad || !root) return;
    const state = directories[keyFor(root.root_id, request.path)];
    if (!state || state.status === "loading") return;
    const activeElement = globalThis.document.activeElement;
    if (activeElement !== globalThis.document.body && !treeRef.current?.contains(activeElement)) {
      treeFocusPathRef.current = null;
      return;
    }
    const buttons = [...(treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-context-path]") ?? [])];
    const target = buttons.find((button) => button.dataset.contextPath === request.path)
      ?? buttons.find((button) => button.dataset.contextPath?.startsWith(`${request.path}/`));
    target?.focus();
    treeFocusPathRef.current = null;
  }, [directories, root, treeRows]);
  const onTreeKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey || !(event.target instanceof HTMLElement)) return;
    const current = event.target.closest<HTMLButtonElement>("[data-context-path]");
    if (!current) return;
    const index = treeRows.findIndex((row) => row.path === current.dataset.contextPath);
    const row = treeRows[index];
    if (!row) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const enabledRows = treeRows.filter(isTreeRowEnabled);
      const currentIndex = enabledRows.findIndex((candidate) => candidate.path === row.path);
      const next = event.key === "Home" ? enabledRows[0] : event.key === "End" ? enabledRows.at(-1)
        : enabledRows[Math.max(0, Math.min(enabledRows.length - 1, currentIndex + (event.key === "ArrowDown" ? 1 : -1)))];
      focusTreePath(next?.path ?? row.path);
      return;
    }
    if (row.entry.kind === "directory" && event.key === "ArrowRight") {
      event.preventDefault();
      if (!row.open) {
        focusTreePath(row.path, true);
        toggleDirectory(row.entry);
      } else if (treeRows[index + 1]?.depth > row.depth) {
        focusTreePath(treeRows[index + 1]!.path);
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (row.entry.kind === "directory" && row.open) {
        focusTreePath(row.path);
        toggleDirectory(row.entry);
      } else {
        const parent = [...treeRows.slice(0, index)].reverse().find((candidate) => candidate.entry.kind === "directory" && candidate.depth < row.depth);
        focusTreePath(parent?.path ?? row.path);
      }
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (row.entry.kind === "directory") toggleDirectory(row.entry); else chooseEntry(row.entry);
    }
  };
  const rootDirectory = directories[keyFor(root.root_id, "")];
  const rootEmpty = rootDirectory?.status === "ready" && rootDirectory.data?.entries.length === 0 && rootDirectory.data.next_offset === undefined;
  return { directories, expanded, setExpanded, selectedEntry, directoryPathForFile,
    treeRows, loadDirectory, toggleDirectory, chooseEntry, onTreeKeyDown, rootEmpty, rootDirectory };
}

export function useDefaultGuide({ identityKey, hasPath, rootDirectory, chooseEntry }: {
  identityKey: string; hasPath: boolean; rootDirectory: DirectoryState | undefined;
  chooseEntry: (entry: ContextEntry) => void;
}) {
  const openedDefaultRef = useRef<string | null>(null);
  useEffect(() => {
    if (hasPath || !rootDirectory?.data || openedDefaultRef.current === identityKey) return;
    openedDefaultRef.current = identityKey;
    const files = rootDirectory.data.entries.filter((entry) => entry.kind === "file" && !entry.refusal && entry.path);
    const named = (name: string) => files.find((entry) => entry.name.toLowerCase() === name);
    const guide = named("task.md") ?? named("readme.md") ?? files.find((entry) => /\.md$/i.test(entry.name));
    if (guide) chooseEntry(guide);
  });
}

export function useDirectoryDiagnostics(root: ContextRoot, directories: Record<string, DirectoryState>, rootDiagnostics: ProjectDiagnostic[]) {
  return useMemo(() => {
    const seen = new Set<string>();
    return [...rootDiagnostics, ...(directories[keyFor(root.root_id, "")]?.data?.diagnostics ?? [])].filter((diagnostic) => {
      const key = `${diagnostic.code}\u0000${diagnostic.message}\u0000${diagnostic.path ?? ""}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }, [directories, rootDiagnostics, root]);
}
