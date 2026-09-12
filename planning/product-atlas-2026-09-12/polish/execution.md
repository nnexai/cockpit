# UI polish implementation

Implemented against [the approved contract](implementation-plan.md), starting at `5ade393`. These captures show the product, separately from the design mock. The user requested a quick final check rather than further full verification.

| Increment | Delivery |
| --- | --- |
| A. Chrome and hierarchy | `4447096`: shared palette, compact controls, Pane/Commands, session selection, command scrolling/focus and responsive sidebar |
| B. Files and Review | `b3a65dd`: shared overview/picker, collapsed metadata and full paths, inline comments, responsive editors, modal keyboard routing |
| C/D. Setup and lifecycle | `e806e94`: path-only directories, borrowed/owned lifecycle, compact form, provider defaults, explicit overrides and retained operation receipts |
| E. Browser annotations | `019dd7f`: 3px freehand, 2px region, release simplification, idle/hover/keyboard opacity and optional notes overview |
| F. Integration | Cleanup receipts, stock Herdr close acknowledgement (`9cd5f19`), verification helpers and this record |

Runtime resources belonged to disposable sessions `polish-i11uqx_w` and `polish-von7_69n`. Herdr 0.9.0 / protocol 22 was retained. Pre-existing and concurrently created `poc/` work was left unstaged.

## Checks

- Frontend: **182 tests passed across 25 files**. Type checking and browser production build passed.
- Rust: **274 workspace tests passed**, one existing ignored test. The later focused stock-close acknowledgement regression passed **1/1**.
- Native: `bunx tauri build --debug --no-bundle` passed. Actual Linux Tauri input opened a nested plain directory and rejected a missing path without creating another Space. [Native receipt](implementation-evidence/native-smoke.json).
- Extension: **21/21 actual-extension checks passed** in an isolated Chromium profile. [Receipt](implementation-evidence/extension-smoke.json), [freehand](implementation-evidence/extension-freehand.png), [notes overview](implementation-evidence/extension-notes-open.png).

## Observed behavior

| Path | Evidence |
| --- | --- |
| Comment resizing at 1440×900, 1024×640, 800×1000 and 480×900 | Same textarea, text, focus and selection retained, without document overflow. [Measurements](implementation-evidence/comment-resize.json), [480px editor](implementation-evidence/comments-480.png) |
| Comment identity and editing | Files Markdown paragraph saved at source line 8. Whole-file save/edit/delete worked. Review old/new line 5 drafts stayed separate from Files; old-side save closes its editor. [Review](implementation-evidence/review-final.png), [overview](implementation-evidence/comments-overview.png) |
| Modal keyboard routing | Ctrl+B v on an overview button left the underlying three panes unchanged. Escape restored the comments button's focus. |
| Commands | Search and final-result scrolling fit all four widths. [Measurements](implementation-evidence/commands.json), [480px](implementation-evidence/commands-480.png) |
| Directory opening | Plain, nested plain, Git root and nested Git paths completed through browser setup. Missing/non-directory paths showed local errors and allowed correction. [Results](implementation-evidence/directories.json), [nested Git](implementation-evidence/Polish-nested-Git.png) |
| Owned and borrowed cleanup | Created and removed a clean linked worktree after exact confirmation. Closing borrowed Spaces retained their files; no plain-folder Git initialization. [Removal review](implementation-evidence/teardown-owned-confirmation.png), [results](implementation-evidence/teardown.json), [preserved files](implementation-evidence/borrowed-preserved.json) |
| Terminal | Actual keyboard input/output, focus/zoom, wheel and resizing exercised. Font remained 16px and row height 20px at all widths. [Measurements](implementation-evidence/terminal.json), [terminal](implementation-evidence/terminal-final.png) |
| Annotation authoring | Live stroke 11 points → 3 on release; 3px/2px widths, cancellation, inline note, sidebar revisit/clear, opacity, scroll/zoom retention and offline draft preservation passed. |

Review and runtime checks repaired stale defaults, uncertain setup receipts, mode-specific destination carryover, nested-directory inventory assumptions, legacy Create ownership, nested-checkout selection, stale Review editor publication and successful Herdr `ok` close responses.

The final quick browser check confirmed that Close Space displays the successful backend receipt. The asynchronous tree refresh was not awaited in that final sample. [Result](implementation-evidence/close-final.json), [capture](implementation-evidence/teardown-final-receipt.png). Both run-owned Herdr sessions, the gateway, native app and browser profiles were stopped; fixture files and evidence were retained.

## Limits

Provider matching, ambiguity, issue metadata and review source branches have fixture/test coverage; live authenticated provider lookup was not exercised. Native screenshot capture was unavailable on XWayland/niri, though actual input and results were observed. macOS, every media/error state, source-refresh draft retention and the full terminal mouse/selection matrix were not rerun. The terminal selection probe did not establish selected text. This record does not claim exhaustive acceptance for these paths.

## Reusing the fixtures

Run `python3 scripts/verify/ui_polish_runtime.py start` for a disposable Herdr session, gateway, Git/plain files and HTTP annotation fixture. Read its `runtime.json` and gateway log for exact URLs. Copy installed Files/Review plugins into that root's isolated Herdr configuration for graphical viewer scenarios.

Set `COCKPIT_EXTENSION_FIXTURE` to its `fixture_url`, optionally set `COCKPIT_EXTENSION_EVIDENCE`, then run `node scripts/verify/ui_polish_extension.mjs`. It uses the machine-installed Playwright CLI module and removes its own profile. Run `python3 scripts/verify/ui_polish_runtime.py stop /tmp/cpol-…` to stop only matching fixture processes; evidence remains on disk.

## Visual correction, September 13

The initial implementation did not follow the mock closely enough. This pass applies its 41px tab strip, 33px pane headers, sidebar hierarchy and typography, SVG icons, tab-bar sidebar toggle, menu proportions, file-tree widths, document typography, segmented viewer controls, and aligned compact setup fields. Companion content search and additional Review navigation remain available through disclosures.

Review scroll events now record position without writing it back to the scroller. Focus restoration uses `preventScroll`. The initial file inventory now includes Git line counts, including renamed paths, and bounded readable untracked-file counts. Binary or unavailable counts remain unspecified.

Validation was deliberately limited at the user's request: frontend production build, one focused scroll/focus regression, and one real-Git inventory-count regression passed. Final visual acceptance belongs to the user; no extended browser/native verification session was run.
