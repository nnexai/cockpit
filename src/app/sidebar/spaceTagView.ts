import type { Space } from "./spaceTree";

export const TAG_MAX_CHARS = 32;
export const SPACES_VIEW_KEY = "cockpit.sidebar.spacesView.v1";
export type SpacesView = "repository" | "tags";

export function readSpacesView(): SpacesView {
  try { return localStorage.getItem(SPACES_VIEW_KEY) === "tags" ? "tags" : "repository"; }
  catch { return "repository"; }
}

export function writeSpacesView(view: SpacesView): void {
  try { localStorage.setItem(SPACES_VIEW_KEY, view); } catch { /* Preferences are optional. */ }
}

export function normalizeTag(input: string): string {
  // Unicode White_Space matches Rust char::is_whitespace (unlike JS \s).
  const tag = input.normalize("NFC").replace(/\p{White_Space}+/gu, " ").replace(/^ | $/g, "");
  return !tag || [...tag].length > TAG_MAX_CHARS || /\p{Cc}/u.test(tag) ? "" : tag;
}

export function tagKey(tag: string): string { return normalizeTag(tag).toLowerCase(); }

export function findTag(catalog: readonly string[], input: string): string | undefined {
  const key = tagKey(input);
  return key ? catalog.find(tag => tagKey(tag) === key) : undefined;
}

export function chipFor(tags: readonly string[], exclude?: string): { primary: string; more: number } | null {
  const remaining = exclude === undefined ? tags : tags.filter(tag => tag !== exclude);
  return remaining.length ? { primary: remaining[0], more: remaining.length - 1 } : null;
}

export const UNTAGGED_KEY = "\u0000untagged";
export type TagListRow =
  | { kind: "group"; rowId: string; groupKey: string; label: string; count: number; expanded: boolean }
  | { kind: "space"; rowId: string; groupKey: string; space: Space; first: boolean; exclude?: string };

/** Catalog-order groups; Space order and identities come only from the snapshot. */
export function projectTagGroups(spaces: readonly Space[], bySpace: ReadonlyMap<string, readonly string[]>,
  catalog: readonly string[], collapsed: ReadonlySet<string>): TagListRow[] {
  const groups = new Map<string, Space[]>(catalog.map(tag => [tag, []]));
  const untagged: Space[] = [];
  for (const space of spaces) {
    const tags = bySpace.get(space.id);
    if (!tags?.length) untagged.push(space);
    else for (const tag of tags) groups.get(tag)?.push(space);
  }
  groups.set(UNTAGGED_KEY, untagged);
  const rows: TagListRow[] = [];
  const visible = new Set<string>();
  for (const [groupKey, members] of groups) {
    if (!members.length) continue;
    const expanded = !collapsed.has(groupKey);
    rows.push({ kind: "group", rowId: `group:${JSON.stringify(groupKey)}`, groupKey,
      label: groupKey === UNTAGGED_KEY ? "Untagged" : groupKey, count: members.length, expanded });
    if (!expanded) continue;
    for (const space of members) {
      rows.push({ kind: "space", rowId: `tag:${JSON.stringify([groupKey, space.id])}`, groupKey, space,
        first: !visible.has(space.id), ...(groupKey === UNTAGGED_KEY ? {} : { exclude: groupKey }) });
      visible.add(space.id);
    }
  }
  return rows;
}
