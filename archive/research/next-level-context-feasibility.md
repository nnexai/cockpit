# Next-level context feasibility

Date: 2026-09-04
Scope: planning and feasibility only; no application code, package installation, remote repository download, or Herdr-server change.

## Recommendation

The next context slice is feasible as a local-first, read-only ingestion pipeline. Treat an explicitly supplied artifact URL as the entry point, and require the local primary repository before resolving repository-relative references. Do not discover or clone arbitrary remote repositories. Already-discovered local repositories may be copied into a companion context directory using a CoW attempt with a normal-copy fallback.

The provider boundary should expose independent, capability-detected reads:

* issue metadata and issue comments/timeline;
* pull-request metadata, changed files/diff, reviews, and review-comment replies;
* wiki page listing and page content/revision;
* local repository files and bounded previews/search/watch events.

All fetched assets should retain canonical URL/identifier, provider, retrieval time, source revision when supplied, and a content hash. Every failed secondary asset should remain an explicit per-asset failure; the primary issue/PR artifact can still be browsed. Remote writes are outside this surface. “Collect comments” means pull and normalize existing comments; it does not post, edit, react, resolve, or merge.

## Gitea and Tea capability

Gitea’s current API index groups issue and pull-request conversations together and lists operations for getting an issue, listing all issue comments and events, getting a pull request, retrieving its diff/patch and changed files, listing reviews, and review-comment operations. The same index lists wiki page listing, page retrieval, and revision retrieval. [Gitea API operation index](https://docs.gitea.com/api/). This is sufficient to implement a read-only adapter without scraping HTML.

The API index documents token authentication through the `Authorization: token <value>` header and marks query-token forms deprecated. [Gitea API authentication](https://docs.gitea.com/api/#authentication). The adapter must never put a token in normalized Markdown/frontmatter, logs, URLs, or generated environment values.

The concrete wiki read shape is useful for normalization: page retrieval returns base64 page content, HTML URL, title/sub-URL, and a last-commit object containing SHA/date/message. [Get a wiki page](https://docs.gitea.com/api/next/operations/repo-get-wiki-page/). Page listing is paginated with `page` and `limit` and returns page metadata plus last-commit information; the operation and its pagination fields are listed in the [Gitea API operation index](https://docs.gitea.com/api/).

Tea is the official Gitea CLI and advertises issue listing/commenting and pull-request checkout/review workflows. [Tea overview](https://about.gitea.com/products/tea/). Tea is therefore a reasonable default wrapper where installed, but its command output and feature set are version-dependent. The adapter contract should prefer machine-readable Tea output only after probing the installed version/help, and otherwise report the missing capability. A direct Gitea API implementation would be a separate explicitly configured adapter. Do not make a successful human-oriented Tea table parse the proof of support.

Suggested capability contract:

```text
ProviderCapabilities {
  issue: Get + ListCommentsTimeline,
  pull_request: Get + ListReviews + ListReviewComments + GetDiff + ListChangedFiles,
  wiki: ListPages + GetPage + ListRevisions,
  freshness: SourceRevisionOrEtagOrContentHash,
  writes: Unsupported
}
```

Issue and PR references should be parsed only when the artifact URL or local repository identity resolves the owner/repository unambiguously. Bound expansion by depth, total assets, comments per asset, bytes per asset, and total bytes. Preserve the provider’s IDs and timestamps; do not infer chronology from rendered text.

## Local repository CoW semantics

For a local Git repository, `git clone` already uses local optimizations: object files may be hardlinked, while `--shared` uses an alternates file and leaves the new repository dependent on the source object store. Git explicitly warns that source maintenance/deletion can corrupt a `--shared` clone, and `--no-hardlinks` is the backup-oriented option. [git-clone](https://git-scm.com/docs/git-clone.html)

For companion context snapshots, use filesystem reflinks when the destination filesystem supports them. Linux `FICLONE` shares underlying physical storage while presenting independent file descriptors/files; later writes use copy-on-write. [ioctl_ficlone(2)](https://man7.org/linux/man-pages/man2/ioctl_ficlone.2.html). A reflink failure such as unsupported filesystem, cross-device path, or special file must fall back to a byte-for-byte copy. Verify destination size/hash after copying and use a temporary destination plus atomic rename.

Do not use hardlinks for mutable companion snapshots. A hardlink gives two names for one inode, so writing through either name changes the same data; it is an alias, not copy-on-write isolation. This is materially different from a reflink. Never hardlink files that an agent or user may edit, and never use Git `--shared` alternates for a context artifact whose source may be garbage-collected.

Only copy regular files under an allowlisted local repository root. Exclude `.git` internals, sockets, devices, symlinks that escape the root, and files over configured byte limits. Record `copy_mode = reflink|copy`, source identity (device/inode where available), destination hash, and fallback reason for diagnostics.

## Markdown, Mermaid, and source mapping

Parse Markdown with a CommonMark-compatible parser and keep the original UTF-8 bytes/line table beside the AST. CommonMark exists to make Markdown syntax interoperable and testable. [CommonMark](https://commonmark.org/). The commonmark.js reference implementation exposes block source positions and a safe renderer mode that strips raw HTML and rejects unsafe URL schemes. [commonmark.js source positions and safe rendering](https://github.com/commonmark/commonmark.js#api).

Render only an allowlisted Markdown subset into a sandboxed viewer. Keep each block’s `(start_line, start_column, end_line, end_column)` and map clicks/highlights back to the original asset by byte offsets or line/column ranges; rendered HTML offsets are not a reliable source map. Preserve fenced-code source ranges, including `mermaid` fences, before transforming them into diagrams.

Mermaid’s documented `strict` security level (the default) encodes HTML and disables click functionality; `sandbox` renders in an iframe and prevents JavaScript from running in the diagram context. Mermaid also exposes a maximum diagram text size. [Mermaid securityLevel](https://mermaid.js.org/config/schema-docs/config-properties-securitylevel.html) and [Mermaid configuration](https://mermaid.js.org/config/configuration.html). Use `strict` by default, set a lower diagram-size limit than the global file limit, disable links/click actions, and treat malformed/unsupported diagrams as source code with a visible error. If a sandboxed diagram is enabled, isolate it from the parent origin and enforce the same byte/time budget.

Never pass provider/user Markdown through an arbitrary HTML renderer with scripts enabled. External images and links should be opt-in or rewritten to an explicit external-open action; unsafe schemes are rejected. Frontmatter is parsed separately and displayed as data, never interpolated as HTML.

## Bounded search, watch, and previews

Use core-mediated `ripgrep` for search with an argument vector, fixed root, allowlisted globs, and hard limits on query length, results, output bytes, files visited, and elapsed time. ripgrep’s official guide documents automatic skipping of hidden files, ignored files, and binary files, plus explicit controls such as `--hidden`, `--glob`, `--max-columns`, and preview truncation. [ripgrep guide](https://github.com/BurntSushi/ripgrep/blob/master/GUIDE.md). Cockpit should keep the safe defaults, explicitly exclude `.git`, and return a structured `truncated` result rather than silently dropping matches.

Use filesystem notifications only as an invalidation hint. The `notify` crate provides a cross-platform recommended watcher, but its documentation warns that network filesystems may emit no events and very large directory trees may lose events. [notify documentation](https://docs.rs/notify/latest/notify/). On every event burst, debounce, rescan the bounded companion tree, compare `(path, size, mtime, hash when needed)`, and emit a snapshot update. Overflow, watcher errors, or rescan disagreement must produce a stale/watch-degraded state and trigger a full bounded rescan.

Preview policy should be enforced before opening a file: regular-file check, maximum bytes, maximum lines, UTF-8/text detection, extension allowlist, and total concurrent preview budget. Read only the first configured bytes/lines for text and logs; show an explicit truncation marker. Images should be decoded with pixel and memory limits. Binary, executable, oversized, unreadable, and symlink-escaping files receive metadata plus an external-open action, never an inline preview.

## Actionable implementation contracts and risks

1. Resolve an artifact URL into a typed Gitea issue, PR, or wiki target. Require the configured local primary repository for context placement and reference expansion.
2. Probe provider capabilities and authentication without mutating the server. Store only normalized read snapshots and source metadata.
3. Pull the primary asset first; expand recognized references with depth/count/byte limits and cycle detection. Collect comments/timeline and review material as separate bounded children.
4. Materialize local companion content with reflink, then copy fallback; never hardlink mutable files or use Git alternates.
5. Render Markdown through a safe parser/renderer while preserving source ranges. Render Mermaid only from fenced blocks under strict/sandbox controls.
6. Expose search/watch/preview through bounded core operations. Treat watcher events as hints and rescan after debounce or overflow.

The main risks are provider-version drift (Tea output and Gitea API additions), permission differences for private comments/reviews/wiki pages, source changes during local copying, watcher event loss, and unsafe Markdown/diagram payloads. Capability detection, source revisions/hashes, atomic snapshot replacement, bounded work, and explicit degraded states address these without changing Herdr-server or allowing remote writes.

Evidence correction: an API operation called “Reply to a pull request review comment” is a POST/write operation, not proof of a read-replies capability. Only a documented read response and concrete fixture may establish reply collection. The `/api/next/` wiki link describes prospective API documentation; the installed Gitea/Tea capability must be verified separately before enabling wiki reads.
