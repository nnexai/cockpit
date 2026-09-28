/**
 * Generated provider documents (Jira, GitHub, GitLab, Tea issues and reviews):
 * facts from their frontmatter and the preview's comment cards. The stored
 * Markdown stays plain; only the rendered tree changes, so every block keeps
 * its source position for line mapping, selection and Context comments.
 */

/** Top-level `key: value` lines of a leading `---` frontmatter block, values unparsed. */
export function readFrontmatter(source: string): Map<string, string> {
  const fields = new Map<string, string>();
  let start = 0;
  let first = true;
  while (start <= source.length) {
    const end = source.indexOf("\n", start);
    const line = source.slice(start, end === -1 ? source.length : end).replace(/\r$/, "");
    const marker = line.trim();
    if (first) {
      if (marker !== "---") return fields;
      first = false;
    } else if (marker === "---" || marker === "...") {
      return fields;
    } else {
      const match = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line);
      if (match) fields.set(match[1], match[2].trim());
    }
    if (end === -1) break;
    start = end + 1;
  }
  // Unterminated: not frontmatter.
  return new Map();
}

/** A frontmatter scalar: JSON-quoted or bare; `null`, `~` and empty read as null. */
export function frontmatterScalar(value: string | undefined): string | null {
  if (value === undefined || value === "" || value === "null" || value === "~") return null;
  if (!value.startsWith("\"")) return value;
  try {
    const parsed: unknown = JSON.parse(value);
    return typeof parsed === "string" && parsed !== "" ? parsed : null;
  } catch {
    return value;
  }
}

export type ProviderFacts = {
  /** Written by a provider snapshot (`provider:` and `generated: true`); only these get comment cards. */
  generated: boolean;
  itemType: string | null;
  status: string | null;
  priority: string | null;
  /** `null` is reported as unassigned; `undefined` when the provider has no assignee. */
  assignee: string | null | undefined;
  author: string | null;
  created: string | null;
  updated: string | null;
};

export function providerFacts(source: string): ProviderFacts {
  const fields = readFrontmatter(source);
  const scalar = (key: string) => frontmatterScalar(fields.get(key));
  return {
    generated: scalar("provider") !== null && fields.get("generated") === "true",
    itemType: scalar("item_type"),
    status: scalar("status"),
    priority: scalar("priority"),
    assignee: fields.has("assignee") ? scalar("assignee") : undefined,
    author: scalar("author"),
    created: scalar("created"),
    updated: scalar("updated"),
  };
}

type Point = { line: number; column: number; offset?: number };
type Position = { start: Point; end: Point };
type Node = {
  type: string;
  depth?: number;
  value?: string;
  url?: string;
  children?: Node[];
  position?: Position;
  data?: { hName?: string; hProperties?: Record<string, unknown> };
};

const COMMENTS_HEADING = /^Comments \(\d+(?: of \d+)?\)$/;

function plainText(node: Node): string {
  if (node.type === "text" || node.type === "inlineCode") return node.value ?? "";
  return (node.children ?? []).map(plainText).join("");
}

function element(tag: string, className: string, children: Node[]): Node {
  return { type: "providerElement", children, data: { hName: tag, hProperties: { className: [className] } } };
}

function text(value: string): Node {
  return { type: "text", value };
}

/** The paragraph right after a comment heading: one `#id` link, or plain `#id` text without a URL. */
function isPermalink(node: Node | undefined): node is Node {
  if (node?.type !== "paragraph" || node.children?.length !== 1) return false;
  const only = node.children[0];
  if (only.type === "link") return plainText(only).startsWith("#");
  return only.type === "text" && /^#\S+$/.test(only.value?.trim() ?? "");
}

/**
 * `Author · 2026-09-28 09:21 · edited 2026-09-29 11:00 · review on src/a.rs:12`
 * becomes the card header; the permalink paragraph joins it, right-aligned.
 */
function commentHeader(heading: Node, permalink: Node | undefined): Node {
  const [author = "", date, ...rest] = plainText(heading).split(" · ").map((segment) => segment.trim());
  const children: Node[] = [{ type: "strong", children: [text(author)] }];
  if (date) children.push(element("span", "provider-comment-date", [text(date)]));
  for (const segment of rest) {
    if (!segment) continue;
    children.push(segment.startsWith("edited ")
      ? element("span", "provider-comment-edited", [text(segment)])
      : element("span", "provider-comment-tag", [text(segment)]));
  }
  if (permalink) children.push(element("span", "provider-comment-permalink", permalink.children ?? []));
  const position = heading.position && permalink?.position ? { start: heading.position.start, end: permalink.position.end } : heading.position;
  return { ...heading, children, position, data: { ...heading.data, hProperties: { ...heading.data?.hProperties, className: ["provider-comment-header"] } } };
}

/** Groups each `###` under `## Comments (…)` with its body into one card, up to the next `##` or shallower heading. */
function commentCards(nodes: Node[]): Node[] {
  const out: Node[] = [];
  let inComments = false;
  let card: Node | null = null;
  for (let index = 0; index < nodes.length; index += 1) {
    const node = nodes[index];
    const heading = node.type === "heading" ? node.depth ?? 6 : null;
    if (heading !== null && heading <= 2) {
      card = null;
      inComments = heading === 2 && COMMENTS_HEADING.test(plainText(node).trim());
      out.push(node);
      continue;
    }
    if (inComments && heading === 3) {
      const permalink = isPermalink(nodes[index + 1]) ? nodes[index + 1] : undefined;
      if (permalink) index += 1;
      const header = commentHeader(node, permalink);
      card = { type: "providerElement", children: [header], position: header.position, data: { hName: "article", hProperties: { className: ["provider-comment-card"] } } };
      out.push(card);
      continue;
    }
    if (card) {
      card.children!.push(node);
      if (card.position && node.position) card.position = { start: card.position.start, end: node.position.end };
      continue;
    }
    out.push(node);
  }
  return out;
}

export type ProviderDocumentOptions = {
  /** A generated provider document: `## Comments (…)` sections become cards. */
  comments: boolean;
  /** The title the surrounding header already shows; a leading `# ` heading with this text is not rendered again. */
  hiddenTitle: string | null;
  /**
   * The item kind (`item_type`) the header shows among its facts. The one-line
   * summary right after a hidden title (`**Task** · **To Do** · …`) repeats
   * those facts for raw readers, so it is hidden with the title.
   */
  summaryLead: string | null;
};

export function remarkProviderDocument(options: ProviderDocumentOptions) {
  return (tree: unknown) => {
    const root = tree as Node;
    if (!root.children) return;
    const [first, second] = root.children;
    if (options.hiddenTitle && first?.type === "heading" && first.depth === 1 && plainText(first).trim() === options.hiddenTitle.trim()) {
      const lead = second?.type === "paragraph" ? second.children?.[0] : undefined;
      const summary = Boolean(options.summaryLead && lead?.type === "strong" && plainText(lead).trim() === options.summaryLead);
      root.children = root.children.slice(summary ? 2 : 1);
    }
    if (options.comments) root.children = commentCards(root.children);
  };
}
