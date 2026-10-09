import { useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { ContextEntry, ContextRoot, LibraryItemSummary } from "../../protocol/generated/v1";
import type { ContextReader } from "./contextSource";
import { resolveContextLink } from "./linkResolver";
export function useDocumentLinks({ root, reader, selectedPath, libraryItems, onLibraryLink, openFile, setExpanded, loadDirectory }: {
  root: ContextRoot; reader: ContextReader | null; selectedPath: string | null; libraryItems: LibraryItemSummary[] | undefined;
  onLibraryLink: (item: LibraryItemSummary) => void; openFile: (path: string, revision: string | null) => void;
  setExpanded: Dispatch<SetStateAction<Set<string>>>; loadDirectory: (root: ContextRoot, path: string, force?: boolean) => Promise<void>;
}) {
  const [linkNotice, setLinkNotice] = useState<string | null>(null);
  const openMarkdownLink = async (href: string) => {
    if (!root || !reader || !selectedPath) return;
    const resolution = resolveContextLink(href, selectedPath, libraryItems ?? []);
    setLinkNotice(null);
    if (resolution.kind === "external" || resolution.kind === "inert" || resolution.kind === "refused") return;
    if (resolution.kind === "library") {
      onLibraryLink(resolution.item);
      return;
    }
    const parts = resolution.path.split("/");
    const parentPath = parts.slice(0, -1).join("/");
    try {
      let offset: number | undefined;
      let revision: string | undefined;
      let found: ContextEntry | undefined;
      do {
        const page = await reader.directory({ root_id: root.root_id, path: parentPath, offset, revision }, new AbortController().signal);
        if (page.root_id !== root.root_id) throw new Error("Context root changed");
        found = page.entries.find((entry) => (entry.path ?? (parentPath ? `${parentPath}/${entry.name}` : entry.name)) === resolution.path);
        offset = page.next_offset;
        revision = page.revision;
      } while (!found && offset !== undefined);
      if (!found || found.refusal) {
        setLinkNotice(`Link target not found: ${resolution.path}`);
        return;
      }
      if (found.kind === "directory") {
        const folders = parts.slice(0, -1).map((_, index) => parts.slice(0, index + 1).join("/"));
        setExpanded((current) => new Set([...current, ...folders, resolution.path]));
        for (const path of [...folders, resolution.path]) void loadDirectory(root, path);
      } else if (found.kind === "file") openFile(resolution.path, found.revision);
      else setLinkNotice(`Link target is unavailable: ${resolution.path}`);
    } catch {
      setLinkNotice(`Unable to resolve link target: ${resolution.path}`);
    }
  };
  return { linkNotice, openMarkdownLink };
}
