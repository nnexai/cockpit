import type { LibraryItemSummary, ProjectProvider } from "../../protocol/generated/v1";
import { isConfluencePage, nestUnderParents, type LibraryContainerNode, type LibraryInstanceNode, type LibraryItemNode } from "./libraryState";

/** A Library page, or an ancestor known only by its title. */
type PageNode = { key: string; title: string; item: LibraryItemSummary | null; folder: boolean; children: PageNode[] };

export type LibraryTreeRow =
  | { kind: "instance"; key: string; depth: 0; parent: null; node: LibraryInstanceNode; open: boolean }
  | { kind: "container"; key: string; depth: 1; parent: string; node: LibraryContainerNode; open: boolean }
  | { kind: "ancestor"; key: string; depth: number; parent: string; node: PageNode; open: boolean }
  | { kind: "page"; key: string; depth: number; parent: string; node: PageNode; item: LibraryItemSummary; open: boolean }
  | { kind: "item"; key: string; depth: number; parent: string; item: LibraryItemSummary }
  | { kind: "attachments"; key: string; depth: number; parent: string; item: LibraryItemSummary; open: boolean }
  | { kind: "attachment"; key: string; depth: number; parent: string; item: LibraryItemSummary; attachment: LibraryItemSummary["attachments"][number] };

/** Ancestor chains put each page under its Library parent or its parent's title. */
function pageForest(container: LibraryContainerNode): PageNode[] {
  const nodes = new Map<string, PageNode>();
  const roots: PageNode[] = [];
  for (const item of container.items) {
    const chain = [...item.ancestors, { id: item.canonical_id ?? item.item_id, title: item.title }];
    let parent: PageNode | null = null;
    for (const [index, page] of chain.entries()) {
      let node = nodes.get(page.id);
      if (!node) {
        node = { key: `${container.key}\u0000page:${page.id}`, title: page.title, item: null, folder: false, children: [] };
        nodes.set(page.id, node);
        (parent?.children ?? roots).push(node);
      }
      if (index === chain.length - 1) Object.assign(node, { key: item.item_id, title: item.title, item });
      parent = node;
    }
  }
  const follow = container.follow;
  if (follow && !follow.partial) {
    const excluded = new Set(follow.excluded_ids);
    for (const [id, node] of nodes) node.folder = !node.item && !excluded.has(id);
  }
  return roots;
}

export function pageItemIds(page: PageNode): string[] {
  return [...(page.item ? [page.item.item_id] : []), ...page.children.flatMap(pageItemIds)];
}

/** Issues share the page rows' disclosure, Enter and attachment behavior. */
function issueNode(node: LibraryItemNode): PageNode {
  return { key: node.item.item_id, title: node.item.title, item: node.item, folder: false, children: node.children.map(issueNode) };
}

export function rowLabel(row: LibraryTreeRow): string {
  switch (row.kind) {
    case "item": case "page": return row.item.title;
    case "attachments": return "Attachments";
    case "attachment": return row.attachment.stored_name;
    case "ancestor": return row.node.title;
    default: return row.node.label;
  }
}

/** Visible rows in DOM order; attachment groups start folded, other groups open. */
export function libraryTreeRows(tree: readonly LibraryInstanceNode[], providers: readonly ProjectProvider[], collapsed: ReadonlySet<string>): LibraryTreeRow[] {
  const visible: LibraryTreeRow[] = [];
  const pushPages = (pages: readonly PageNode[], depth: number, parent: string) => {
    for (const page of pages) {
      const open = !collapsed.has(page.key);
      const attachments = page.item?.attachments ?? [];
      if (page.item && page.children.length === 0 && attachments.length === 0) visible.push({ kind: "item", key: page.key, depth, parent, item: page.item });
      else if (page.item) visible.push({ kind: "page", key: page.key, depth, parent, node: page, item: page.item, open });
      else visible.push({ kind: "ancestor", key: page.key, depth, parent, node: page, open });
      if (!open) continue;
      pushPages(page.children, depth + 1, page.key);
      if (!page.item || attachments.length === 0) continue;
      const groupKey = `${page.key}\u0000attachments`;
      const groupOpen = collapsed.has(groupKey);
      visible.push({ kind: "attachments", key: groupKey, depth: depth + 1, parent: page.key, item: page.item, open: groupOpen });
      if (groupOpen) for (const attachment of attachments) visible.push({ kind: "attachment", key: attachment.attachment_id, depth: depth + 2, parent: groupKey, item: page.item, attachment });
    }
  };
  for (const instance of tree) {
    const instanceOpen = !collapsed.has(instance.key);
    visible.push({ kind: "instance", key: instance.key, depth: 0, parent: null, node: instance, open: instanceOpen });
    if (!instanceOpen) continue;
    for (const container of instance.containers) {
      // Folders have no container level.
      if (!container.label) {
        for (const item of container.items) visible.push({ kind: "item", key: item.item_id, depth: 1, parent: instance.key, item });
        continue;
      }
      const containerOpen = !collapsed.has(container.key);
      visible.push({ kind: "container", key: container.key, depth: 1, parent: instance.key, node: container, open: containerOpen });
      if (!containerOpen) continue;
      if (container.items.every(isConfluencePage)) pushPages(pageForest(container), 2, container.key);
      else pushPages(nestUnderParents(container.items, providers).map(issueNode), 2, container.key);
    }
  }
  return visible;
}
