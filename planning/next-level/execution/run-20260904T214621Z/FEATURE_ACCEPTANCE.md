# Selected feature acceptance — 2026-09-05

This continues the verified `a1fb90c` Context/paste/snapshot/cleanup increment documented in `WORKFLOW_ACCEPTANCE.md`. The implementation follows the selected stories in `state.json`; optional stories were not added. No live installation, external publication, default Herdr automation or manual gateway changes occurred.

## Implemented behavior

| Selected scope | Result |
| --- | --- |
| REV-01/02, PANE-01/02 | Cockpit Review for the real Reviewr pane; local staged, unstaged, untracked and branch comparisons; immutable old/new source captures, unified gutters, full-source expansion, bounded binary/unreadable handling; durable comments and exact acknowledged paste through the existing receipt workflow. |
| VIEW-02 | Companion-authorized PNG/JPEG blobs with byte/pixel/type checks; Mermaid in an opaque sandbox with a nonce CSP, no network access and rendering limits; physical-line comment mapping; inert SVG/HTML source and explicit unavailable PDF preview. |
| SRC-01/02 | Configured Tea login and provider instance matched to the primary checkout origin; bounded issue metadata/comments; immutable semantic-hash cache and current pointers; generated Markdown provenance; explicit refresh and preservation of local edits. |
| SRC-03 | Explicit linked-source hydration for qualified issue/PR/wiki URLs in the same repository. Depth2,32assets,4MiB aggregate and one operation deadline; deduplication/cycles, partial failure, durable reports; unknown revisions remain unknown; inconsistent equal revisions retain evidence and report changed content. |
| SRC-04/05 | PR metadata, read review comments and unverified provider positions; wiki content and the current provider revision. Tea's unsupported diff/structured-file/reply capabilities remain explicit. |
| REPAIR-01/02/03 | Shared stream-order corpus and stale reducer guards; prefix/input routing; cancellation of pending browser/native terminal attachments with late-native-stream cleanup and bounded handshake. |
| REPAIR-04/05 | Finite request/process deadlines, unknown-outcome handling, required capability registry, concrete config/operation/transport ownership. Pane-move no-op results now produce actionable errors. |
| CLEAN-02/03/04 | Frontend session/focus/mutation/layout/keymap modules, Herdr config/operations/transport/capabilities, code guide and consistent Rust formatting. |
| CLEAN-05 | Explicit-base changed scope with content hashes, version probes, normalized metric joins, CRAP/coverage rules, baseline ratchet, mutation accounting, stable exits and hermetic fixtures. Optional metric providers remain unavailable. |

FND/LIFE/CTX/REF/VIEW01/VIEW03 behavior delivered in earlier increments is retained. The current guide is `CODE_GUIDE.md`.

## Real runtime evidence

All tests use stock Homebrew Herdr0.8.2 in the run-owned session `ck-cc-hhnye1fm`, gateway54831, ChromeCDP54832, native Xvfb:196 and localhost Tea fixture54833. The resource ledger records executable/hash/socket/process identities. The fixture has a configured primary repository, a linked task checkout and its authorized companion; no remote cloning or provider write occurred.

- `review-open.mjs`: Open Review launches installed Reviewr in the linked checkout and renders Cockpit's GUI.
- `review-capture-responses.json`, `review-preview.txt`: exact old deleted-line, whole-file and deleted-file captures. Payload1827bytes; receipt `9e4a843f-d18e-4329-b297-52720cf008ed` accepted1839framedbytes. Agent fixture's raw input moved2796→4635bytes without Enter. This proves PTY delivery, not model processing.
- `review-switch-retained.png`, `review-original-tui.png`: an unsent draft survives switching between Cockpit Review and the real Reviewr TUI.
- `review-recapture.json`, `review-edit-delete.json`: switching old/new sources retains editor text but requires explicit current-source capture; saved edits and deletion persist without removing the other draft.
- `review-move-verified.json`, `review-reattach.json`: after unzooming, Herdr moved w2:p4 from w2:t2 to w2:t1; the durable batch was recovered and explicitly reattached. Earlier `review-move*.json` attempts before this fix were no-ops because the tab was zoomed; they are diagnostic evidence only.
- `media-final.png`, `native-media.png`, `native-review.png`: actual browser/native Mermaid, safe image and Review rendering. The diagram's saved comment maps to physical lines6–9, including its fences. Remote image, diagram directive, SVG execution and active PDF rendering are refused/unavailable as designed.
- `source-import-result.json`, `source-refresh-result.json`: actual Tea import, unchanged refresh, changed provider content, and preserved `USER ANNOTATION MUST SURVIVE` local text on conflict.
- `source-pr-wiki.json`, `source-hydration.json`: actual PR/wiki imports and bounded PR→issue2→missingissue3 traversal; successful sources persist despite a secondary404. The source report records completed/skipped/failed/truncated counts, byte budget and diagnostics.
- `source-requests.jsonl`: the localhost provider audit contains GET requests only. The Tea executable is real; the provider server/login are controlled fixtures. No claim is made that production forge permissions or network availability were tested.

The working fixture and scripts are retained under `/tmp/cc-hhnye1fm`. Durable copies and a SHA256 evidence index are stored beside the resource ledger under `continuation-20260905/final-feature-acceptance/`.

## Final hardening

Independent review led to explicit Review editor recapture, cache pointer publication before garbage collection, bounded orphan-object recovery, a durable nonblocking import lease across fetch/cache/companion publication, and one configured deadline for every provider fetch. Generated source publication records a recoverable intent before replacing file bytes; restart recovery accepts only exact recorded hashes and preserves user changes. Focused tests cover those interruption and concurrency contracts.

## Verification and remaining limits

Final check results and the delivery commit are recorded in `state.json` and `NEXT.md`. The full quality gate runs frontend typecheck/tests, Rust formatting/workspace checks/tests, and native compilation. Its strict metric result remains `inconclusive` (exit2): coverage/complexity/mutation providers are not installed, and some integration tests have no exact method mapping. Missing data is never counted as passing coverage.

Explicitly excluded or blocked: expanded terminal redraw/input matrix; stock Herdr application mouse and dependent terminal baseline; active Kitty TGP; optional stories and side-by-side diff; active PDF renderer; Tea0.15.1 PR diff/structured changed-file/reply subcapabilities. Wiki revisions are the shortened SHA exposed by Tea's JSON output. The browser bundle retains the existing size warning; Mermaid is a separate lazy chunk.
