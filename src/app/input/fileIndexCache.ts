import type { ContextIndexedFile } from "../../protocol/generated/v1";

const MAX_ENTRIES = 8;
type FileIndexEntry = { files: readonly ContextIndexedFile[]; truncated: boolean; at: number };
const entries = new Map<string, FileIndexEntry>();

export function getFileIndex(key: string): FileIndexEntry | undefined {
  const value = entries.get(key);
  if (value === undefined) return undefined;
  entries.delete(key);
  entries.set(key, value);
  return value;
}

export function putFileIndex(key: string, files: readonly ContextIndexedFile[], truncated: boolean): void {
  entries.delete(key);
  entries.set(key, { files, truncated, at: Date.now() });
  while (entries.size > MAX_ENTRIES) entries.delete(entries.keys().next().value!);
}

