import { Fragment, useEffect, useId, useRef, useState, type ReactNode } from "react";
import type { LibraryItemSummary, ProjectDiagnostic, ProjectProvider } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { copyText } from "./clipboard";
import { confluenceSite, instanceHost, isConfluencePage, parseLibraryTime, providerFamily, timeDetail, type TimeDetail } from "./libraryState";

/** The open document's own facts, read by the viewer. */
export type LibraryDetailsDocument = {
  bytes: number;
  contentHash: string | null;
  mediaType: string;
  /** The document's frontmatter block, verbatim. */
  frontmatter: string | null;
  diagnostics: readonly ProjectDiagnostic[];
};

const COPIED_MS = 1200;

/** `sha256:6190e992521955…7c2e` becomes `sha256:6190e992…7c2e`: the prefix, the first 8 hex digits and the last 4. */
function shortHash(value: string): string {
  const [prefix, hex] = value.includes(":") ? [value.slice(0, value.indexOf(":") + 1), value.slice(value.indexOf(":") + 1)] : ["", value];
  return hex.length > 12 ? `${prefix}${hex.slice(0, 8)}…${hex.slice(-4)}` : value;
}

/** A path keeping its first two and last segments. */
function middlePath(path: string): string {
  const parts = path.split("/");
  return parts.length > 4 ? `${parts.slice(0, 2).join("/")}/…/${parts.at(-1)}` : path;
}

/**
 * Library item facts in one popover (design §4.6), opened from the item
 * header's `ⓘ`: issues, source, Library copy, then a collapsed Technical
 * section. A non-modal disclosure: Escape closes only it (and returns focus to
 * the trigger), as do a press outside and focus leaving. Identifiers are one
 * line, end- or middle-ellipsis, with the full value as tooltip and a Copy button.
 */
export function LibraryDetails({ item, providers, now, document, root, pageUpdate }: {
  item: LibraryItemSummary;
  providers: readonly ProjectProvider[];
  now: number;
  document: LibraryDetailsDocument;
  root: { id: string; kind: string; path: string; repositoryId: string; companionId: string | null };
  /** A Confluence page's last edit from its frontmatter; the source clock. */
  pageUpdate: { at: string | null; by: string | null };
}) {
  const [open, setOpen] = useState(false);
  const [technical, setTechnical] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const copiedTimer = useRef<number | undefined>(undefined);
  const popoverId = useId();
  useEffect(() => () => window.clearTimeout(copiedTimer.current), []);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!wrapRef.current?.contains(event.target as Node)) setOpen(false); };
    window.addEventListener("pointerdown", outside);
    return () => window.removeEventListener("pointerdown", outside);
  }, [open]);
  // Another item's details start closed.
  useEffect(() => { setOpen(false); setTechnical(false); }, [item.item_id]);

  const page = isConfluencePage(item);
  const folder = item.folder;
  const family = providerFamily(providers, item.provider_id);
  const issues = item.diagnostics.length + item.conflict.length;
  const updated = timeDetail(pageUpdate.at, now);
  const fetchedTime = parseLibraryTime(item.fetched_at);
  const checkedTime = parseLibraryTime(item.checked_at);
  const merged = fetchedTime !== null && checkedTime !== null && Math.abs(fetchedTime - checkedTime) <= 1000;
  const fullPath = `${root.path.replace(/\/$/, "")}/${item.document_path ?? ""}`;
  const documentName = item.document_path?.split("/").at(-1) ?? null;

  const copy = (label: string, value: string) => {
    void copyText(value).then((ok) => {
      if (!ok) return;
      setCopied(label);
      window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(null), COPIED_MS);
    });
  };
  const identifier = ({ label, value, shown = value, mono = true }: { label: string; value: string; shown?: string; mono?: boolean }) => <>
    <span className={`library-detail-value${mono ? " is-mono" : ""}`} title={value}>{shown}</span>
    <button type="button" className="library-copy" aria-label={`Copy ${label}`} title={`Copy ${label}`} onClick={() => copy(label, value)}><UiIcon name={copied === label ? "check" : "copy"} /></button>
  </>;
  const time = (raw: string | null | undefined, detail: TimeDetail | null, suffix?: ReactNode): ReactNode => detail
    ? <span className="library-detail-human" title={detail.iso}>{detail.text}{suffix}{detail.ago ? <span className="library-detail-ago"> · {detail.ago}</span> : null}</span>
    : <span className="library-detail-human is-unknown" title={raw ?? undefined}>Unknown</span>;
  const row = (label: string, value: ReactNode) => <Fragment key={label}><dt>{label}</dt><dd>{value}</dd></Fragment>;

  return <div className="library-details" ref={wrapRef} onKeyDown={(event) => {
    if (event.key !== "Escape" || !open) return;
    event.preventDefault();
    event.stopPropagation();
    setOpen(false);
    triggerRef.current?.focus();
  }} onBlur={(event) => { if (open && event.relatedTarget instanceof Node && !wrapRef.current?.contains(event.relatedTarget)) setOpen(false); }}>
    <button ref={triggerRef} type="button" className="library-icon-button" aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? popoverId : undefined}
      aria-label={issues > 0 ? `Details, ${issues} ${issues === 1 ? "issue" : "issues"}` : "Details"} title="Details" onClick={() => setOpen(!open)}>
      <UiIcon name="info" />
      {issues > 0 ? <span className="library-details-dot" aria-hidden="true" /> : null}
    </button>
    {open ? <div id={popoverId} className="library-details-popover" role="dialog" aria-label="Details">
      {issues > 0 ? <section>
        <h4>Issues</h4>
        <ul className="library-detail-issues">
          {item.conflict.map((file) => <li key={file.path}><span className="library-detail-issue"><UiIcon name="edit" />Edited file</span><span className="library-detail-value is-mono" title={file.path}>{file.path}</span></li>)}
          {item.diagnostics.map((diagnostic, index) => <li key={`${diagnostic.code}:${index}`}><span className="library-detail-issue"><UiIcon name="info" />{diagnostic.code}</span><span>{diagnostic.message}</span></li>)}
        </ul>
      </section> : null}
      <section>
        <h4>Source</h4>
        <dl>
          {folder ? <>
            {row("Copied from", identifier({ label: "origin path", value: folder.origin_path, shown: middlePath(folder.origin_path) }))}
            {row("Inventory", <span className="library-detail-human">{folder.git_working_tree ? "Git tracked and untracked, non-ignored files" : "Regular files with default exclusions"}</span>)}
            {row("Copied", <span className="library-detail-human">{folder.files} files · {folder.bytes} bytes</span>)}
            {row("Skipped symlinks", <span className="library-detail-human">{folder.skipped_symlinks}</span>)}
            {row("Skipped special files", <span className="library-detail-human">{folder.skipped_special}</span>)}
            {row("Skipped ignored files", <span className="library-detail-human">{folder.skipped_ignored}</span>)}
            {row("Skipped other files", <span className="library-detail-human">{folder.skipped_other}</span>)}
            {row("Updates", <span className="library-detail-human">Source edits do not change this copy until an explicit re-copy. Space copies update separately.</span>)}
          </> : <>
            {item.container ? row(page ? "Space" : "Project", <span className="library-detail-human">{item.container.label}</span>) : null}
            {item.canonical_id ? row(page ? "Page ID" : "Source identity", identifier({ label: page ? "page ID" : "source identity", value: item.canonical_id })) : null}
            {page && item.source_revision ? row("Version", <span className="library-detail-human">{item.source_revision}</span>)
              : item.source_revision ? row("Source revision", identifier({ label: "source revision", value: item.source_revision })) : null}
            {page && (pageUpdate.at || pageUpdate.by) ? row("Last updated", time(pageUpdate.at, updated, pageUpdate.by && !/\S+@\S+/.test(pageUpdate.by) ? ` · ${pageUpdate.by}` : null)) : null}
            {item.provider_instance ? row("Provider", <span className="library-detail-human">{family.name} · {family.key === "confluence" ? confluenceSite(item.provider_instance) : instanceHost(item.provider_instance)}</span>) : null}
            {item.source_url ? row("Source link", identifier({ label: "source link", value: item.source_url })) : null}
            {item.original_url && item.original_url !== item.source_url ? row("Added from", identifier({ label: "added-from link", value: item.original_url })) : null}
          </>}
        </dl>
      </section>
      <section>
        <h4>Library copy</h4>
        <dl>
          {merged ? row("Fetched · checked", time(item.fetched_at, timeDetail(item.fetched_at, now)))
            : <>
              {item.fetched_at ? row("Fetched", time(item.fetched_at, timeDetail(item.fetched_at, now))) : null}
              {item.checked_at ? row("Checked", time(item.checked_at, timeDetail(item.checked_at, now))) : null}
            </>}
          {row("Folder", identifier({ label: "folder path", value: item.item_path, shown: middlePath(item.item_path) }))}
          {item.document_path ? row("File", identifier({ label: "file path", value: item.document_path, shown: documentName ?? item.document_path })) : null}
          {row("Size", <span className="library-detail-human">{document.bytes} B</span>)}
          {row("Revision", identifier({ label: "revision", value: item.revision, shown: shortHash(item.revision) }))}
          {row("Media type", <span className="library-detail-human">{document.mediaType || "Unavailable"}</span>)}
        </dl>
      </section>
      <section>
        <button type="button" className="library-detail-toggle" aria-expanded={technical} onClick={() => setTechnical(!technical)}>
          <UiIcon name={technical ? "down" : "right"} />Technical
        </button>
        {technical ? <dl>
          {row("Full path", identifier({ label: "full path", value: fullPath, shown: middlePath(fullPath) }))}
          {row("Root identity", identifier({ label: "root identity", value: root.id }))}
          {row("Provenance", <span className="library-detail-human">{root.kind}{root.repositoryId ? ` · repository ${root.repositoryId}` : ""}{root.companionId ? ` · companion ${root.companionId}` : ""}</span>)}
          {row("Content hash", document.contentHash ? identifier({ label: "content hash", value: document.contentHash, shown: shortHash(document.contentHash) }) : <span className="library-detail-human is-unknown">Unavailable</span>)}
          {document.frontmatter ? row("Frontmatter", <pre className="library-detail-frontmatter">{document.frontmatter}</pre>) : null}
          {document.diagnostics.map((diagnostic, index) => row(`Diagnostic ${index + 1}`, <span className="library-detail-human"><code>{diagnostic.code}</code>{diagnostic.path ? ` · ${diagnostic.path}` : ""} · {diagnostic.message}</span>))}
        </dl> : null}
      </section>
      <span className="sr-only" role="status" aria-live="polite">{copied ? "Copied" : ""}</span>
    </div> : null}
  </div>;
}
