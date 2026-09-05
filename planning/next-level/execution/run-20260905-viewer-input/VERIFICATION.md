# File viewer and input repair

Baseline: `7a8994b`. The worktree was clean before this increment.

## Changes

- Task setup autofocus is tied to opening, not callback changes during parent updates.
- File viewer uses compact, indented, lazily loaded directory rows and conventional tree/source navigation. Ordinary verified folders can own comment batches, preserving companion identities and filesystem/path checks.
- Context and Review share inline line commenting, status actions, and Ctrl/Cmd+Enter save. Enter remains a newline. Source Shift navigation anchors at the focused line; expanded source handles each key once.
- Markdown fills the available document width and restores scroll only when entering a document. HTML renders static content and inline styles in an opaque sandbox, with active/navigation markup removed and resource requests restricted by CSP.

## Verification

Disposable session `ck-fv-20260905`, gateway `127.0.0.1:54851`, browser CDP `127.0.0.1:54852`; all resources/evidence under `/tmp/cc-fv-20260905`. No default session or user gateway mutations.

- Real baseline dialog reproduction: `focus-lost.json` records typing moving to Close during terminal updates.
- Backend regression first failed with `comments_detached` for an ordinary folder, then passed after the change. It captures exact folder source, rejects traversal, and rejects a replaced root. `/tmp/cc-folder-red.log`, `/tmp/cc-folder-green.log`.
- `cargo test --workspace` passed. Backend build passed.
- Frontend suite: 112 tests passed. Follow-up source Shift-selection regression uses actual DOM keyboard events and verifies they do not bubble to the parent.
- Production frontend build passed. Vite retains its existing large-chunk advisory.
- Formatter and diff whitespace checks passed.
- Read-only review covered source identity/paste revalidation and HTML isolation, including inert parsing, SVG animation, noscript, and nested templates.
- Final live dialog proof: `focus-retained.json` records every character of `slow focus proof` staying in the task-name input during repeated output from the fixture-owned terminal.
- Final viewer proof: `viewer-proof.json` records mouse selection, Shift+Arrow range selection, C to comment, Ctrl+Enter saving a multiline inline draft, and persistence after reload. Compressed directory collapse/restore and source navigation passed.
- Markdown mouse-wheel proof used eight downward and two upward movements. Both directions were monotonic, then remained at exactly 2880 and 2160 respectively over 60 animation frames. The rendered body filled 360.73px of its 361px scroll viewport.
- HTML inline styling rendered; script, form, and network markup were removed. No external fixture requests, parent mutation, or navigation occurred.
- Screenshots `viewer-normal-1440x900.png` and `viewer-minimum-1024x640.png` were inspected; split panes fit without page horizontal overflow. Browser script: `prove-focus-and-viewer.mjs`.
- Disposable gateway, browser, and Herdr session were stopped using the ownership-checked fixture cleanup script. Fixtures and evidence remain under `/tmp/cc-fv-20260905`.

Scope boundary: comments are available at the verified browsing root. A companion's additional repository selector remains a read-only source view. Source imports and snapshots remain companion-only. HTML scripts and external assets are intentionally disabled. Native-only code did not change; shared GUI behavior is verified in the browser.
