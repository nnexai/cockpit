import { UiIcon } from "../UiIcon";
import type { ProviderFamily } from "./libraryState";

const MONOGRAMS: Record<ProviderFamily["key"], string | null> = { confluence: "C", jira: "J", github: "GH", gitlab: "GL", gitea: "Gt", other: null };

/**
 * Provider identity as a monogram tile (design §4.1a), not a brand logo:
 * `C`, `J`, `GH`, `GL`, `Gt`, else the family name's first letter. `folder`
 * marks a Library folder copy, which has no provider.
 */
export function ProviderMark({ family, size, folder = false }: { family: ProviderFamily; size: "tree" | "header"; folder?: boolean }) {
  const letters = MONOGRAMS[family.key] ?? family.name.charAt(0).toUpperCase();
  return <span className={`library-tile is-${size}${size === "header" ? " library-item-tile" : ""}${!folder && letters.length > 1 ? " is-wide" : ""}`} aria-hidden="true" data-family={folder ? "folder" : family.key}>
    {folder ? <UiIcon name="folder" /> : letters}
  </span>;
}
