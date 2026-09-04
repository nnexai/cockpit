# Context viewer and reference comments

Status: proposed implementation stories; no implementation is authorized by this document.
Date: 2026-09-04

This slice adds a bounded, read-only Context viewer and a local reference-comment workflow. It
assumes FND-01 configuration/capabilities, FND-02 typed operations, FND-03 identity/concurrency,
and CTX-01 companion ownership/path policy. It uses PANE-01/02 detection and renderer replacement inside a real Herdr extension pane. Herdr-server is not
modified. Herdr remains authoritative for session, Space, tab, pane, agent, focus, and process
state.

The product loop is: browse bounded companion files beside the active Herdr work, inspect exact
source lines, collect comments across files, review one deterministic payload, then paste it into a
same-tab agent input. Pasting never submits; the user performs submission in the agent terminal.

## Shared invariants

* Context reads are read-only and bounded. The viewer never executes a file, Mermaid diagram,
  image, HTML, SVG, PDF, shell command, or arbitrary URL.
* Every displayed file is a `FileRef` with companion identity, normalized relative path, source
  identity, immutable revision/content hash, size, and status. A readable absolute companion path
  may be shown as metadata or offered through an approved host-mediated external-open action; the
  client never turns it into an unrestricted read path.
* Source mode is the authority for comments. A line comment stores the immutable source revision,
  exact 1-based `start_line`/`end_line`, and exact original lines including frontmatter. A full-file
  comment stores the same revision and an explicit `whole_file` anchor without embedding the file body. External changes invalidate
  the draft; they never silently rebase or replace its quoted lines.
* Draft batches are scoped by client/window, Herdr session epoch, Space, and tab. A new tab or
  session never inherits or merges drafts. The main implementation persists drafts under the
  Cockpit state root by default so a restart does not lose review work; persistence is user-owned
  local state and stores no credentials. Writes are atomic and protected by the FND-03 lock.
* Paste target filtering is stricter than reviewr's workspace-wide behavior: the target must be an
  agent pane in the selected session, selected Space, and selected tab, with confirmed Herdr focus and explicit local paste intent. A pane moving tabs, closing, changing agent identity, or losing control
  between selection and delivery produces an explicit rejected or outcome-unknown result.
* The paste operation is bounded by encoded payload bytes, framing overhead, and an initial Cockpit paste ceiling of
  64 KiB. It never truncates silently. Ordinary terminal input remains the
  existing client-shell `TextCommit` path; the new action uses a separately acknowledged paste
  operation and never pretends `TextCommit` is native paste.

## VIEW-01: tree and bounded file viewer

**Dependencies:** CTX-01 manifest and path policy; FND-01 limits/capabilities; FND-02 `context.tree`
and `context.read`; FND-03 `FileRef`/revision rules. No provider dependency.

**Proposed files:**

* `crates/cockpit-core/src/context/view.rs`: bounded tree/read DTOs, file-kind classification,
  revision checks, and read errors.
* `crates/cockpit-protocol/src/v1.rs`: `context.tree`, `context.read`, `context.open_external`
  requests/responses and typed stale/unsupported/limit failures.
* `src/client/CockpitClient.ts`, `src/client/native.ts`, `src/client/browser.ts`: typed client
  methods and adapter mapping.
* `src/app/context/ContextViewer.tsx`, `ContextTree.tsx`, `SourceView.tsx`, and `context.css`:
  presentation only; mount only for the selected visible Context surface.

**Implementation steps:** enumerate manifest entries plus a bounded directory view; show status,
kind, size, freshness, and refusal reason; page or virtualize large trees; read one bounded regular
file by `FileRef` and revision; render source lines with stable gutters and selection; preserve
scroll and selection by file identity across refresh; expose `Refresh` and approved external-open
only when capability data permits it. Keep companion path display separate from read authorization.

**Tests:** path traversal, absolute/NUL/encoded traversal, symlink escape, special-file refusal,
byte/line/page limits, revision mismatch, malformed DTOs, stale cache, external-open capability,
tree ordering, line-number preservation, frontmatter inclusion, and late response from another
session/tab.

**Real acceptance:** use a disposable companion containing user Markdown, source, frontmatter,
symlink, binary, oversized, and changing files. Verify exact line numbers and bytes, visible inline
failure state, refresh behavior, and that no file or Herdr resource changes.

**Parallel lane:** core/path and protocol owner first; frontend owner can use checked-in fixtures
after DTO freeze. Integration owner owns generated types and adapter parity.

## VIEW-02: Markdown, Mermaid/source mapping, safe media, and optional PDF

**Dependencies:** VIEW-01; FND-01 preview budgets; CTX-01 generated/user separation. No paste
dependency.

**Proposed files:** `crates/cockpit-core/src/context/render.rs` for bounded render classification
and source mappings; `src/app/context/MarkdownView.tsx`, `MermaidView.tsx`, `SafeImage.tsx`,
`PdfNotice.tsx`, and tests/fixtures under `crates/cockpit-core/tests/fixtures/context/`.

**Implementation steps:** keep source mode canonical and make rendered Markdown a derived view;
  map every rendered block back to source line spans; include frontmatter in source and collapse it
  only in rendered view; render Mermaid only through a bounded, sandboxed renderer with a visible
  source mapping and timeout/degraded state; accept only bounded image formats through host handles,
  never active SVG/HTML; present safe text/code fallback when image or diagram capabilities are
  absent; treat PDF as an optional capability with a read-only bounded notice or page renderer.
  Use the selected Herdr extension-pane renderer design in 07-extension-panes.md.

**Tests:** Markdown links/escaping, frontmatter mapping, Mermaid timeout/size/error, diagram click
  back to source lines, image MIME/size/pixel limits, SVG/HTML refusal, PDF unavailable/corrupt/page
  limits, and render failure retaining the source view.

**Real acceptance:** exercise the actual WebKit/Tauri and browser paths with Mermaid, images,
source fences, frontmatter, and PDF fixtures. Confirm source line selection remains exact after
render toggles and that untrusted assets cannot execute or navigate the host.

**Parallel lane:** renderer owner may proceed after VIEW-01 DTOs; security/path owner reviews all
media and external-open decisions. PDF remains optional and cannot block source/Markdown.

## VIEW-03: watcher, search, and revision invalidation

**Dependencies:** VIEW-01; FND-02 `context.search`/`context.subscribe`; FND-03 revisions and
operation cancellation.

**Proposed files:** `crates/cockpit-core/src/context/watch.rs`, `search.rs`; host watcher adapter;
protocol events for bounded invalidation; `src/app/context/contextStore.ts` and `ContextSearch.tsx`.

**Implementation steps:** watch only the owned companion roots; debounce/coalesce events by file;
  emit generation and revision changes; invalidate reads and line selections without rewriting
  drafts; search through a core allowlisted bounded operation with path/line/excerpt results;
  preserve the selected file and search query when safe; cancel stale searches and reject late
  results by session, companion, query generation, and file revision.

**Tests/acceptance:** event storms, rename/delete, file mutation during read, stale search result,
  bounded result/timeout behavior, restart watcher recovery, and a real fixture where a selected
  source changes while a draft remains visible as conflict/unknown.

**Parallel lane:** watcher/search owner after CTX-01 and VIEW-01; UI search may use fixtures after
  event DTO freeze. No provider or Herdr mutation is involved.

## REF-01: draft collection, edit/delete, and exact multi-file payload

**Dependencies:** VIEW-01 exact source reads and revisions; FND-02 `comments.list/upsert/remove/preview`;
FND-03 `DraftBatchId`, persistence, locks, and compare-and-swap. No delivery transport dependency
for collection and preview.

**Proposed files:** `crates/cockpit-core/src/comments/mod.rs`, `draft.rs`, `format.rs`; protocol
`comments.list/upsert/remove/preview`; `src/app/context/CommentDrafts.tsx`; persistent state under
the configured Cockpit state root (exact directory chosen by FND-01).

**Draft shape:** `{draft_id, batch_id, session_epoch, space_id, tab_id, file_ref, revision,
source_kind, start_line, end_line, selected_lines, comment_text, updated_at}`. `selected_lines` is
the exact source text, including frontmatter and original newline normalization metadata. Full-file
comments set `start_line = null`, `end_line = null`, `selected_lines = []`, and `source_kind = whole_file`. They contain path/revision and comment only; only explicit line selections embed source text.

**Implementation steps:** comment whole file or a contiguous line range; capture the current
  immutable revision and exact lines at creation; retain drafts while navigating files, refreshing,
  restarting, or receiving watcher events; mark `stale`/`conflict` if revision or bytes differ;
  support edit/delete with atomic persistence; provide a preview that displays every path, range,
  original line number, exact line, and reviewer text; sort only for preview/export by path then
  start line, with stable draft-id tie-breaker.

**Payload:** one deterministic block per draft, containing the resolved absolute companion file path, optional `start_line-end_line`, source revision/hash, and comment text. Explicit line comments also contain numbered exact original lines. Include a short relative display path separately; agents run in the primary worktree and cannot resolve a bare companion-relative path from there. The referenced snapshot path is the path actually reviewed, not a mutable origin repository path. Keep
the payload bounded after UTF-8 encoding and paste framing. Do not replace stale lines with a fresh
read; require explicit discard/review of a conflict.

**Tests/acceptance:** multi-file order, whole-file and line comments, frontmatter, Unicode and
newline normalization, edit/delete, atomic restart persistence, concurrent window locking,
revision conflict, duplicate preview, exact byte-size accounting, and a real browser/native run
that collects comments from at least three files then verifies the displayed payload byte-for-byte.

**Parallel lane:** comments core owner after VIEW-01; draft UI can proceed from protocol fixtures.
Integration owner reviews persistence and generated protocol changes.

## REF-02: bounded same-tab agent paste, no Enter

**Dependencies:** REF-01 preview/bounds; FND-02 `comments.paste` and operation results; FND-03
Herdr resource/session epoch and operation locks; current Cockpit terminal ownership seam. Requires
the capability audit to expose a dedicated acknowledged paste operation. No Herdr-server change.

**Transport decision:** prefer a narrow Cockpit core task that targets the selected Herdr pane and
uses the installed public `pane.send_text` byte path, wrapping a sanitized batch in bracketed-paste
markers like reviewr. The core/host must return an acknowledgment that the bounded write was
accepted by Herdr, with `rejected` and `outcome_unknown` when target validation, queueing, timeout,
disconnect, or pane movement prevents a reliable result. Do not route this action through the
existing unacknowledged `terminal.input`/`TextCommit` path and label it delivered. The native
Herdr source distinguishes raw `pane.send_text` bytes from its conditional native paste helper;
the exact framed behavior must be proven by a real terminal/agent fixture before implementation.

**Target contract:** resolve and freeze `{session_id, session_epoch, space_id, tab_id, pane_id,
agent_fingerprint, client_surface_id, draft_batch_id}` at preview. Revalidate immediately
before sending that the pane still belongs to the same session, Space, and tab, is still an agent,
and has confirmed Herdr semantic focus plus explicit local paste intent. Current protocol 22 has no exclusive owner lease; do not invent an `agent_revision` or lease field. Build the agent fingerprint from fields actually present and fail conservatively if identity cannot be checked. A race remains possible after revalidation; classify a lost
acknowledgment as `outcome_unknown`, retain drafts, and never auto-retry.

**Implementation steps:** add capability-gated `comments.paste`; sanitize embedded bracket-end
markers; account for UTF-8 payload plus framing under the configured/Herdr byte limit; acquire a
per-target/per-batch lock and dedupe record; send exactly once; persist `pending` before write;
record `accepted` only from the host acknowledgment; preserve drafts on rejected/unknown; allow an
explicit user-directed retry only after target revalidation and a visible duplicate-risk warning.
Before writing text, focus the chosen agent through the acknowledged Herdr focus operation, then revalidate same-tab identity. Focus failure sends nothing and retains drafts. After an accepted paste, a DOM-focus failure is a separate warning and must not trigger another paste. Never append `\n`, `\r`, Enter, or a
second submission command.

**Persistence/dedupe:** retain the pending/accepted/rejected receipt and batch payload hash in the
Cockpit state root. A process crash can leave delivery unknown; on restart show the receipt and
require explicit resolution. Do not claim exactly-once PTY delivery. A cross-process filesystem lock prevents concurrent
duplicate sends across native/browser hosts; persistence handles restart visibility, not recovery proof.

**Tests:** capability absence, same-tab filtering, moved/closed pane, changed agent identity,
ownership loss, session switch, malformed/oversized payload, embedded marker sanitization, exactly
one paste call, no appended submission bytes, acknowledgment states, timeout/disconnect unknown,
lock/dedupe concurrency, restart recovery, and ordinary terminal input regression.

**Real acceptance:** with a disposable same-tab agent fixture, preview and paste comments from
multiple files; inspect the actual agent input and confirm one bracketed paste, exact numbered
lines, no submission, and authoritative accepted state. Repeat with another tab, pane movement,
ownership loss, external file change, timeout, and restart. Each case must leave a visible
pending/rejected/unknown state and preserve drafts where delivery is not confirmed. Use separate
disposable sessions from the user's active session.

**Parallel lane:** transport/core owner after capability audit and REF-01; UI owner can implement
preview and state rendering against fixtures. Integration owner owns the public operation table,
native/browser adapter equivalence, generated types, and final real-session acceptance.

## Integration gate

FND-01/02/03 and CTX-01 must be contract-frozen before merging these lanes. The integration owner
must verify native/browser DTO equivalence, bounded reads/search/rendering, persisted draft recovery,
same-tab target filtering, acknowledged paste outcomes, no-submit behavior, and real runtime
evidence. PANE-01/02 provide detection and real-pane layout. No outer dock, synthetic Context tab, or extension communication is part of this design.


## Draft lifecycle and paste contract details

A source view numbers original physical lines, including frontmatter. Preserve an immutable byte snapshot and CRLF/LF/final-newline metadata. Rendered Markdown selection resolves to exact source spans; until accurate partial-span mapping exists, “Comment on source lines” opens the corresponding source range rather than quoting rendered text as original Markdown. Mermaid comments refer to the original fenced source. Syntax highlighting never changes numbering.

Durable draft identity is independent of the ephemeral client session epoch. Store a stable batch ID, companion/review identity, owner pane/terminal identity, and last-known tab/Space. The epoch guards live responses only. After restart, restore drafts as detached until fresh Herdr/process/provenance checks bind them to the intended pane. Closed panes leave recoverable drafts. A moved extension carries its drafts, but changing tab invalidates the paste target; moving to another Space also requires context reattachment or an explicit “original context” read-only state. Never send a batch from another tab silently.

All GUI batches are Cockpit-owned. Switching to the original file-viewer/Reviewr TUI does not synchronize drafts with that independent process. Two GUI windows use compare-and-swap revisions and a single persisted send receipt; a collision asks the user to reload the newer batch, not overwrite it.

Before send, detect changed/deleted source files and offer explicit re-review/reselect, remove comment, or retain the captured excerpt labeled with its original revision. If the path no longer exists, the outgoing header must state this and preserve the excerpt; do not claim the agent can open a removed file. No automatic line rebasing.

Use 64 KiB as the initial Cockpit batch ceiling, including UTF-8 encoding and bracket framing, until the public `pane.send_text` limit is measured. It is not a claim that the existing client-shell command limit governs every Herdr API. Reject an oversized batch with size/count and let the user remove comments or explicitly choose a smaller subset. Do not silently split one send into multiple writes.

Bracketed-paste framing contains source newlines; these are required content. “No Enter” means no submission key and no extra CR/LF after the closing marker, not removing source newlines. Strip/reject embedded paste terminators and other terminal control characters through a linear bounded sanitizer. Display any sanitization in preview, preserving the original stored excerpt. Reject unsupported byte encodings rather than silently corrupting references.

An accepted Herdr response means its input queue accepted the bytes, not that the agent semantically processed them. Mark the sent batch with a receipt and remove it from the unsent queue, retaining a short local sent history for recovery. Rejection preserves unsent drafts. Timeout/disconnect after dispatch is outcome-unknown, persists the receipt, and never retries automatically. The user inspects the terminal before choosing “Mark pasted” or “Paste again.” A server-side move can race the final check; without a conditional Herdr write API this race cannot be eliminated, so do not promise atomic same-tab validation across arbitrary concurrent clients.

The M0 probe must verify bracketed paste with the installed agent's actual input mode, including multiline and control-character fixtures. A pane no longer recognized as an agent is never a paste target. If a supported agent cannot preserve multiline paste without submission, expose preview/copy with an explicit unavailable-paste reason until a proven path exists.

## Comment presentation: second-checkpoint decision

Keep the batch as shared state, not a permanent bottom panel. The Context/Reviewr header shows `N comments`; clicking or a GUI-scoped shortcut opens an overview across files. Range comments appear beside their original source/diff lines before sending. Whole-file comments and rendered-document comments can appear below that file. Inline, file-bottom, and overview presentations edit the same draft identities. Accepted paste removes the sent drafts from these unsent views and retains delivery history; rejected/unknown delivery preserves them. Changed-source anchors show the captured revision and do not pretend to attach to newly numbered lines. Test switching files/renderers, editing from the overview, reload, and accepted versus unknown delivery.
