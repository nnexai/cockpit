import { useMemo, useState, useRef, useLayoutEffect, useEffect, type MouseEvent as ReactMouseEvent } from "react";
import ReactMarkdown, { type Components, type Options as MarkdownOptions } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { LibraryItemSummary } from "../../protocol/generated/v1";
import type { ContextFileViewState } from "./ContextViewer";
import type { ContextReader } from "./contextSource";
import { remarkBoundDiagrams } from "./markdownPolicy";
import { providerFacts, remarkProviderDocument } from "./providerDocument";
import { sourceLinesForMarkdown } from "./documentMetadata";
import { resolveContextLink } from "./linkResolver";
import { SafeImage } from "./SafeImage";
import { MermaidView } from "./MermaidView";
type SourceSpan = { start: number; end: number };

export function localImagePath(documentPath: string, value: string | undefined): string | null {
  if (!value || /^(?:[A-Za-z][A-Za-z0-9+.-]*:|[/\\])/.test(value) || /[?#]/.test(value)) return null;
  let decoded: string;
  try { decoded = decodeURIComponent(value); } catch { return null; }
  if (/[\x00-\x1f\x7f\\]/.test(decoded) || decoded.startsWith("/")) return null;
  const segments = documentPath.split("/").slice(0, -1);
  for (const segment of decoded.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") { if (!segments.length) return null; segments.pop(); }
    else segments.push(segment);
  }
  return segments.length ? segments.join("/") : null;
}

function nodePosition(node: unknown): { start: number; end: number } | null {
  if (typeof node !== "object" || node === null || !("position" in node)) return null;
  const position = node.position;
  if (typeof position !== "object" || position === null || !("start" in position) || !("end" in position)) return null;
  const startValue = position.start;
  const endValue = position.end;
  if (typeof startValue !== "object" || startValue === null || !("line" in startValue)) return null;
  if (typeof endValue !== "object" || endValue === null || !("line" in endValue)) return null;
  const start = startValue.line;
  const end = endValue.line;
  return typeof start === "number" && typeof end === "number" ? { start, end } : null;
}

function blockData(node: unknown, mapping: number[]): { "data-source-start": number; "data-source-end": number } | undefined {
  const position = nodePosition(node);
  if (!position || mapping.length === 0) return undefined;
  const start = mapping[Math.max(0, Math.min(mapping.length - 1, position.start - 1))];
  const end = mapping[Math.max(0, Math.min(mapping.length - 1, position.end - 1))];
  return start === undefined || end === undefined ? undefined : { "data-source-start": start, "data-source-end": end };
}

function spanFromClick(event: ReactMouseEvent<HTMLDivElement>): SourceSpan | null {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return null;
  const block = target.closest<HTMLElement>("[data-source-start]");
  if (!block) return null;
  const start = Number(block.dataset.sourceStart);
  const end = Number(block.dataset.sourceEnd);
  return Number.isSafeInteger(start) && Number.isSafeInteger(end) ? { start, end } : null;
}
/** An absolute http(s) image address; other schemes are never loaded. */
function remoteImageUrl(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}

/** Markdown image references to web addresses, e.g. `![badge](https://…)`. */
function countExternalImages(markdown: string): number {
  return markdown.match(/!\[[^\]]*\]\(\s*<?https?:\/\//gi)?.length ?? 0;
}

/**
 * A remote image stays blocked until the reader loads external images for the
 * document, as a mail client does. It loads without a referrer; a failure
 * leaves its alt text.
 */
function RemoteImage({ src, alt, allowed }: { src: string; alt: string; allowed: boolean }) {
  const [failed, setFailed] = useState(false);
  if (!allowed || failed) return <span className="context-media-refusal" title={src}>{alt || "image"}</span>;
  return <img className="context-safe-image" src={src} alt={alt} title={alt || undefined} referrerPolicy="no-referrer" loading="lazy" decoding="async" onError={() => setFailed(true)} />;
}

export function MarkdownView({
  text,
  state,
  onSelect,
  onScroll,
  onLink,
  libraryItems,
  media,
  hiddenTitle,
}: {
  media: ContextReader["media"] | null;
  text: string;
  state: ContextFileViewState;
  onSelect: (start: number, end: number) => void;
  onScroll: (scrollTop: number) => void;
  onLink: (href: string) => void;
  libraryItems: readonly LibraryItemSummary[];
  /** The title the item header already shows; a leading equal `# ` heading isn't rendered again. */
  hiddenTitle: string | null;
}) {
  const derived = useMemo(() => sourceLinesForMarkdown(text), [text]);
  const facts = useMemo(() => providerFacts(text), [text]);
  const comments = facts.generated;
  const summaryLead = facts.generated ? facts.itemType : null;
  const remarkPlugins = useMemo((): NonNullable<MarkdownOptions["remarkPlugins"]> => [remarkGfm, remarkBoundDiagrams, [remarkProviderDocument, { comments, hiddenTitle, summaryLead }]], [comments, hiddenTitle, summaryLead]);
  const externalImages = useMemo(() => countExternalImages(derived.text), [derived.text]);
  const documentKey = `${state.rootId}\u0000${state.path}`;
  const [externalAllowedFor, setExternalAllowedFor] = useState<string | null>(null);
  const externalAllowed = externalAllowedFor === documentKey;
  // Listing refreshes rebuild the item array; links only change when an item's identity or address does.
  const linkItemsKey = libraryItems.map((item) => `${item.item_id}\u0000${item.title}\u0000${item.source_url ?? ""}\u0000${item.original_url ?? ""}`).join("\u0001");
  const linkItemsRef = useRef({ key: linkItemsKey, items: libraryItems });
  if (linkItemsRef.current.key !== linkItemsKey) linkItemsRef.current = { key: linkItemsKey, items: libraryItems };
  const linkItems = linkItemsRef.current.items;
  const onLinkRef = useRef(onLink);
  const onScrollRef = useRef(onScroll);
  useLayoutEffect(() => {
    onLinkRef.current = onLink;
    onScrollRef.current = onScroll;
  });
  const scrollRef = useRef<HTMLDivElement>(null);
  const restoredScrollIdentity = useRef<string | null>(null);
  const scrollIdentity = `${state.rootId}\u0000${state.path}\u0000${state.revision ?? ""}\u0000${text}`;
  // The position is committed once scrolling settles: committing per scroll event re-renders the whole viewer every frame.
  const pendingScroll = useRef<number | null>(null);
  const dropPendingScroll = () => {
    if (pendingScroll.current !== null) window.clearTimeout(pendingScroll.current);
    pendingScroll.current = null;
  };
  useEffect(() => dropPendingScroll, []);
  useEffect(() => {
    if (restoredScrollIdentity.current === scrollIdentity) return;
    restoredScrollIdentity.current = scrollIdentity;
    dropPendingScroll();
    if (scrollRef.current) scrollRef.current.scrollTop = state.scrollTop;
  }, [scrollIdentity, state.scrollTop]);
  const scheduleScrollCommit = () => {
    dropPendingScroll();
    pendingScroll.current = window.setTimeout(() => {
      pendingScroll.current = null;
      if (scrollRef.current) onScrollRef.current(scrollRef.current.scrollTop);
    }, 120);
  };
  const components: Components = useMemo(() => ({
    p: ({ node, children, ...props }) => <p {...props} {...blockData(node, derived.sourceLines)}>{children}</p>,
    h1: ({ node, children, ...props }) => <h1 {...props} {...blockData(node, derived.sourceLines)}>{children}</h1>,
    h2: ({ node, children, ...props }) => <h2 {...props} {...blockData(node, derived.sourceLines)}>{children}</h2>,
    h3: ({ node, children, ...props }) => <h3 {...props} {...blockData(node, derived.sourceLines)}>{children}</h3>,
    h4: ({ node, children, ...props }) => <h4 {...props} {...blockData(node, derived.sourceLines)}>{children}</h4>,
    h5: ({ node, children, ...props }) => <h5 {...props} {...blockData(node, derived.sourceLines)}>{children}</h5>,
    h6: ({ node, children, ...props }) => <h6 {...props} {...blockData(node, derived.sourceLines)}>{children}</h6>,
    blockquote: ({ node, children, ...props }) => <blockquote {...props} {...blockData(node, derived.sourceLines)}>{children}</blockquote>,
    ul: ({ node, children, ...props }) => <ul {...props} {...blockData(node, derived.sourceLines)}>{children}</ul>,
    ol: ({ node, children, ...props }) => <ol {...props} {...blockData(node, derived.sourceLines)}>{children}</ol>,
    pre: ({ node, children, ...props }) => {
      const code = node?.children.find(child => child.type === "element" && child.tagName === "code");
      if (code?.type === "element" && Array.isArray(code.properties.className) && code.properties.className.includes("language-mermaid")) {

        const source = code.children.filter(child => child.type === "text").map(child => child.type === "text" ? child.value : "").join("");
        return <div {...blockData(node, derived.sourceLines)}>{code.properties.dataMermaidPreview === true ? <MermaidView source={source} /> : <><p>Only the first four diagrams are previewed.</p><pre>{children}</pre></>}</div>;
      }
      return <pre {...props} {...blockData(node, derived.sourceLines)}>{children}</pre>;
    },
    a: ({ node, href, children, ...props }) => {
      const resolution = resolveContextLink(href, state.path, linkItems);
      if (resolution.kind === "relative" || resolution.kind === "library") {
        return <a {...props} href={href} title={resolution.kind === "library" ? `Open in Library: ${resolution.item.title}` : "Navigates within this root; unsafe paths and symlink targets are refused"} className="context-link-reference" {...blockData(node, derived.sourceLines)} onClick={(event) => { event.preventDefault(); event.stopPropagation(); if (href) onLinkRef.current(href); }}>{children}</a>;
      }
      if (resolution.kind === "refused") {
        const reason = resolution.reason === "root_escape" ? "target escapes the current root" : resolution.reason === "absolute_path" ? "absolute filesystem paths are not allowed" : resolution.reason === "unsupported_scheme" ? "only relative paths are allowed" : "invalid path";
        return <span {...props} title={`Link refused: ${reason}`} {...blockData(node, derived.sourceLines)}>{children}</span>;
      }
      return <span {...props} title={resolution.kind === "external" ? "External link is not available in this viewer" : undefined} {...blockData(node, derived.sourceLines)}>{children}</span>;
    },
    img: ({ node, alt, src }) => {
      const path = localImagePath(state.path, src);
      const remote = !path && remoteImageUrl(src);
      return <span {...blockData(node, derived.sourceLines)}>{path && media ? <SafeImage media={media} request={{ root_id: state.rootId, path, expected_revision: null }} alt={alt ?? "Context image"} className="context-safe-image" /> : remote ? <RemoteImage src={remote} alt={alt ?? ""} allowed={externalAllowed} /> : <span className="context-media-refusal">{alt || "image"}</span>}</span>;
    },
  }), [derived.sourceLines, externalAllowed, linkItems, media, state.rootId, state.path]);
  // Scrolling, selection and sibling state re-render this view; the document tree only changes with its inputs.
  const rendered = useMemo(() => <ReactMarkdown skipHtml remarkPlugins={remarkPlugins} components={components}>{derived.text}</ReactMarkdown>, [components, derived.text, remarkPlugins]);
  useEffect(() => {
    for (const block of scrollRef.current?.querySelectorAll<HTMLElement>("[data-source-start]") ?? []) {
      const selected = Number(block.dataset.sourceStart) === state.selectionStart && Number(block.dataset.sourceEnd) === state.selectionEnd;
      block.classList.toggle("is-selected-block", selected);
    }
  }, [rendered, state.selectionStart, state.selectionEnd]);
  return (
    <div className="context-markdown-scroll" ref={scrollRef} onScroll={scheduleScrollCommit} onClick={(event) => {
      if (event.target instanceof Element && event.target.closest("dialog, button, textarea")) return;
      const span = spanFromClick(event);
      if (span) onSelect(span.start, span.end);
    }}>
      {externalImages > 0 && !externalAllowed ? <div className="context-external-images" role="status"><span>{externalImages === 1 ? "1 external image" : `${externalImages} external images`} not loaded</span><button type="button" onClick={(event) => { event.stopPropagation(); setExternalAllowedFor(documentKey); }}>Load external images</button></div> : null}
      <article className="context-markdown-body">
        {rendered}
      </article>
    </div>
  );
}
