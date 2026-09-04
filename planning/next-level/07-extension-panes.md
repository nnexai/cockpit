# Detect extension panes and replace their GUI

Status: selected architecture from the user checkpoint on 2026-09-04. Detect an existing Herdr extension pane and render Cockpit's complete replacement UI in its rectangle. Do not communicate with the extension, synchronize its internal state, modify its code, or change Herdr-server.

This supersedes the initial side-dock/reading-view study. Context and Reviewr replacements are ordinary Herdr panes from the user's perspective. They move left, right, or below terminals through normal Herdr operations. There are no Build/Review workbench modes or synthetic Context tabs. Artifact type is setup input only.

## Ownership

Herdr owns the real pane, PTY/process lifetime, Space/tab membership, semantic focus, and layout. The original extension continues running underneath. Cockpit chooses a renderer for that pane and owns the replacement's file/diff model, selection, comments, and other GUI state. Normal GUI interaction never sends terminal keystrokes to the hidden TUI.

Context uses the installed `herdr-file-viewer` as a behavioral reference and supported detection target. Review uses `persiyanov.reviewr`. The replacement is a complete Cockpit feature, not a graphical remote control for either TUI. Existing unsent TUI comments are not imported, and GUI comments are not inserted into the TUI's store. “Show terminal view” exposes the original independent TUI without pretending the states were synchronized.

The shared generation-1 client-shell endpoint still receives the complete visible tab surface. Cockpit decodes ordered surfaces/patches but does not mount xterm for a GUI-replaced pane. Neighboring terminal panes retain current rendering/input behavior. A fallback mounts xterm from a fresh full surface; it does not replay GUI actions to the TUI.

Evidence: [pane replacement feasibility](../../research/next-level-pane-replacement.md) and [replacement reference research](../../research/next-level-reviewr-gui-adapter.md). Source inspection establishes the available contract; runtime probes below are required before implementation is declared complete.

## PANE-01: bounded extension detection

Required by the main path. Depends on FND-01/02/03. Can start before context storage or GUI components.

The file-viewer target is verified from the installed manifest `/home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/herdr-plugin.toml`: plugin `herdr-file-viewer`, version 1.15.0, pane entrypoint `file-viewer`, title `Files`, command `./target/release/herdr-file-viewer`. Runtime support still depends on enabled/available capability.

Proposed files: `crates/cockpit-core/src/extensions/{mod,detect}.rs`, `crates/cockpit-herdr/src/extensions.rs`, extension DTOs in `cockpit-protocol`, and `src/app/extensions/extensionReducer.ts`.

Use Herdr's existing APIs, not extension IPC:

- `plugin.list` identifies installed plugin IDs, versions, roots, and declared pane entrypoint commands.
- `plugin.pane.open` returns exact plugin ID, entrypoint, and `PaneInfo` when Cockpit launches it.
- `pane.process_info` provides current foreground process PID/name/argv/cwd when the platform supports that inspection.
- `session.snapshot` supplies pane/terminal identity, current tab/Space, cwd, and layout membership. It does not expose plugin ID/entrypoint for every existing pane.

Define a closed renderer registry with two initial adapters:

| Plugin / entrypoint | GUI replacement | Data authority |
|---|---|---|
| `herdr-file-viewer` / `file-viewer` | Context/file browser | Cockpit core bounded filesystem view and companion metadata |
| `persiyanov.reviewr` / `pane` | Local review UI | Cockpit core Git review snapshot and comments |

Detection returns `verified_launch`, `verified_process`, `candidate`, `none`, or `unsupported`, with a bounded reason. A pane title is only a cheap candidate filter. It never decides replacement by itself.

Implementation sequence:

1. Cache the installed manifest inventory with a bounded refresh policy. Resolve declared executable paths beneath plugin roots, including supported `$HERDR_PLUGIN_ROOT` expansion. Do not execute manifest commands during detection or interpret arbitrary shell strings.
2. For a Cockpit plugin-open response, record its plugin/entrypoint and pane/terminal identity for the current Herdr connection epoch. Verify the returned pane in the next snapshot before rendering.
3. For a pre-existing pane, call `pane.process_info`. Match a supported current foreground executable/argv against the installed entrypoint signature. Names or basenames alone remain ambiguous. Known supported entrypoint wrappers have explicit adapter signatures. For the installed Reviewr `sh -c` wrapper that ends with `exec "$HERDR_PLUGIN_ROOT/bin/herdr-reviewr"`, compare the resulting foreground executable path to `<plugin_root>/bin/herdr-reviewr`; do not build a general shell parser. Unrecognized wrapper scripts, missing argv, multiple matching foreground processes, or unresolvable paths produce a candidate rather than a guessed match.
4. Bind detection to endpoint/session epoch, pane ID, terminal ID, and current process evidence. Recheck at attach/reconnect, relevant lifecycle events, and bounded intervals while a replacement is visible. A process replacement or incompatible evidence must not leave a different terminal application hidden behind an old GUI.
5. Offer a local `Render as → Context / Review / Terminal` override for ambiguous candidates. It binds a presentation choice to the current pane/terminal and approved root, not to every pane with that name. Explicit conversion must not invent plugin provenance.
6. Missing process inspection disables automatic adoption of unknown panes, not the whole app. Known launch receipts and explicit user selection remain available. Do not add a helper handshake as a workaround.

Process details are detection inputs, not authority to read arbitrary paths. Resolve every GUI root through the configured catalog/companion policy. Do not expose raw process argv/cmdline in ordinary UI, logs, or snapshots; arguments can contain secrets. The frontend receives renderer kind, confidence, and a redacted reason.

This does not claim secure executable attestation from `argv0`. In the trusted local-workstation model, the goal is to avoid accidental renderer replacement. Filesystem containment and operation authorization stay independent even if a process spoofs a name. Exact executable evidence is preferred; ambiguous evidence requires a user override.

Tests cover identical titles, unrelated same-name processes, wrapper commands, executable paths with spaces, plugin upgrades, missing process API, restarted/reused IDs, terminal process replacement, and raw-argv redaction. A real probe must detect both installed extensions opened outside Cockpit, then decline an ordinary shell renamed to the same title.

## PANE-02: interchangeable pane renderers and launch

Required. Depends on PANE-01; Context integration additionally needs CTX-01 and VIEW-01. This is the main placement story, replacing an outer dock.

Proposed files: `src/app/extensions/PaneRenderer.tsx`, `src/app/extensions/RendererMenu.tsx`, a narrow `PaneView` edit in `App.tsx`, core launch operations, and fixtures. The integrator owns all terminal lifecycle/geometry changes.

A pane has one active local renderer: terminal, Context, or Review. Renderer state follows authoritative Herdr geometry and visibility. Split, move, swap, resize, zoom, and cross-tab moves use existing Herdr commands. The GUI file-tree divider is local; the outer pane divider is not.

```text
terminal -> detecting -> graphical
                  |         |
                  v         v
              ambiguous   source/detection stale
                  |         |
        explicit renderer   retry or terminal view
```

Keep the last GUI with a local error while revalidating source state. If the backing pane/process identity changed or the user chooses terminal view, obtain a full current client-shell surface before mounting xterm. Never run both input paths at once. A GUI error does not close the pane. If Herdr actually closes it, remove the view and retain unsent GUI drafts in recovery storage.

When its manifest/capability is available, launch “Open Context” through the installed file-viewer plugin pane entrypoint with split placement and companion cwd. Launch “Open Review” through the installed Reviewr pane entrypoint with the primary worktree cwd. Both operations run through the schema-gated Herdr adapter and capture the plugin-open response. GUI availability does not require changing either extension. Missing/disabled plugins get a clear capability result and setup instructions; do not silently install plugins or register automatic hooks.

For ordinary file-viewer panes launched in a repository cwd, the GUI can show that authorized repository root. Add a Context root selector only when the Space has a verified companion. Do not assume every file-viewer pane points at the companion, and do not infer the TUI's currently selected file by scraping output.

### Focus and dimensions

A click on a graphical pane records local intent and sends Herdr pane focus. After confirmation, focus the relevant GUI control. GUI selection, text entry, scroll, and drag within the document never go to xterm or the extension PTY. The magic escape/layout shortcuts keep priority; outside those shortcuts, the focused GUI control owns its input.

External Herdr focus changes clear local control intent while preserving document/draft state. Read-only viewing can continue; editing a comment requires renewed local focus. REF-02 explicitly focuses the chosen same-tab agent, waits for confirmation, revalidates the target, then performs the paste. If focus fails, send nothing. If paste succeeds but DOM focus fails, report accepted text with a focus warning.

Convert authoritative character rectangles through the existing shared cell metrics. Document size must never resize the entire Herdr surface. GUI minimum widths trigger internal collapse/scroll, not fake pane geometry. Real surface-size changes still use the current client-shell handshake/resize path.

Closing the graphical pane invokes the same Herdr pane close operation as the terminal view and terminates that extension pane's process. Closing a Cockpit window only detaches local renderers. Closing a pane does not delete its Space companion or discard unrelated drafts.

Tests and real acceptance: two real terminal neighbors plus one replaced file-viewer/review pane; move/resize/zoom from Cockpit and another Herdr client; tab/Space moves; reconnect and process exit; GUI→terminal→GUI switch; magic escape and clipboard behavior. Verify no GUI keystrokes reach the hidden TUI and no hidden xterm instance remains mounted.

## REV-01: Cockpit-owned local review model

Selectable after the main Context reference loop, but fully planned here. Required for a complete Reviewr GUI replacement. Depends on FND-01/02/03, PANE-01, and REF-01 serialization. It does not depend on an extension bridge or on remote review ingestion.

Proposed files: `crates/cockpit-core/src/review/{mod,snapshot,diff,anchors}.rs`, a narrow Git adapter under the reusable core/provider boundary, protocol `review.snapshot/files/file/refresh`, and Git fixtures. No reads from Reviewr private memory/state files and no terminal scraping.

Define explicit comparison modes inside the review pane:

- Working tree: unstaged tracked changes relative to index, with staged changes separately identified.
- Staged: index relative to HEAD.
- Branch: merge-base with an explicitly selected base ref through selected HEAD; record both immutable commit IDs.
- Untracked: explicit separate files; never silently treat them as tracked diff additions.

These are diff scopes, not global workbench modes. Default to “All local changes” as a grouped view of staged, unstaged, and untracked files, retaining each group's distinct base/target revisions. A partially staged file may appear in two groups with different anchors.

Use a fixed-argv Git operation with `--no-ext-diff`, `--no-textconv`, bounded output, and literal pathspecs. Use NUL-delimited names/status for arbitrary filenames. No user-supplied Git options, external diff drivers, checkout, staging, reset, amend, or commit operations. Record HEAD, index state, and file hashes before/after reading; retry once or return a changed-during-read state rather than presenting an inconsistent diff.

A `ReviewSnapshot` carries review ID, canonical repo/worktree identity, comparison scope, base/head/index/worktree revision tokens, generation, files, limits, and errors. Hunks carry old/new path, change status, side-specific line numbers, exact original lines, and source revisions. Rename detection must be bounded/configurable. Binary, submodule, mode-only, too-large, and unreadable files get explicit rows with safe summaries.

Comment anchors include `review_id`, snapshot generation, file identity, side, base/head or dirty revision, start/end lines, immutable selected text, and reviewer comment. A deleted line references the old side and old path. Refresh flags changed anchors; it never silently remaps old comments to current line numbers. Context and review comments reuse format/delivery code but remain separate batches with an explicit source kind.

Tests: staged-only, unstaged-only, partially staged, untracked, renamed/deleted/binary/mode-only/submodule, empty/unborn HEAD, detached HEAD, base ref missing, filenames with newline/dash/non-UTF8 handling, large diff truncation, changing index, revision-stale anchors, and command injection. Real acceptance uses a disposable Git fixture with each state and verifies source Git/index contents are unchanged after review.

## REV-02: full graphical review replacement

Selectable after REV-01, PANE-02, VIEW-01 source rendering, and REF-01/02. Source ingestion SRC-04 is independent and may provide additional task context later.

Proposed files: `src/app/review/{ReviewPane,ChangedFiles,DiffView,ReviewComments}.tsx`, review reducer/client mappings, and shared comment components. Reuse Context typography, source gutters, selection, queue, and payload preview. Both features use Cockpit-owned durable review/comment state.

Show changed files with status and scope, a unified diff, old/new source gutters, inline unsent comments, and a header comment count opening an on-demand batch overview. Do not reserve a persistent bottom panel for comments. Whole-file and selected-line comments work across files. Add keyboard next/previous file/hunk, source view, expand unchanged lines under limits, and refresh. Side-by-side diff is a separately deferrable renderer inside this story; unified diff must be complete first.

The target picker lists Herdr-detected agents in the same actual tab. Sending uses REF-02's acknowledged paste-only operation. No provider comment posting, Git mutation, or automatic agent submission is part of review. A review/MR URL can set the comparison suggestion and attach context, but the GUI requires a verified local repo/revision match before using remote positions as local anchors.

The original Reviewr TUI continues independently. Switching to terminal view preserves GUI drafts in Cockpit and reveals the TUI's own current state. Show a concise state-boundary notice when unsent GUI drafts exist; do not export or clear them to imitate synchronization. This satisfies detect-and-replace without adding extension IPC.

Acceptance: detect a Reviewr pane opened in Herdr, render the complete local diff UI, collect/edit/remove comments in three files including deleted lines, preview byte-exact payloads, paste without Enter, move the pane through Herdr, and verify both GUI drafts and the original TUI survive renderer switching independently.

## PANE-03: future TUI backport of Cockpit features

Optional after Context and review core contracts stabilize. No current GUI story depends on it. It does not require a bridge to the current extensions.

Reuse the Cockpit file/review/comment services in a dedicated terminal frontend or contribute chosen behavior upstream to the existing extensions. Proposed module `crates/cockpit-tui` is added only when this story is selected; do not scaffold it during main GUI implementation.

Stages: bounded file tree/source preview, local diff/hunks, comment collection, same-tab paste. Mermaid becomes source text or a labeled placeholder unless a concrete terminal rendering capability is selected. Keep domain state in shared core and UI keymaps/rendering in the TUI. Decide whether the new TUI shares Cockpit's durable batches or intentionally remains independent before implementing persistence.

Tests include small cell sizes, resizing, keyboard/magic escape, Unicode/wide glyphs, exact line mapping, and safe paste. A shared-state choice requires concurrent GUI/TUI CAS and delivery-lock tests. An independent-state choice must not claim synchronized drafts. Either can ship without modifying Herdr-server.

## Integration and sequencing

Process inspection is an optional independent capability: absence must not disable plugin-open receipts, explicit renderer overrides, or ordinary terminals.

PANE-01 lands with foundation capabilities, including typed `plugin.list`, `plugin.pane.open`, and `pane.process_info`. PANE-02 and VIEW-01/02 can run in parallel after renderer and file DTOs freeze. REF-01 core is independent of renderer placement; its UI integrates into the actual pane. REV-01 can later run beside provider work; REV-02 follows it. PANE-03 is elective.

The integration owner edits `App.tsx`, protocol generation, host/client operation tables, and terminal input/geometry. Domain contributors own their modules. No implementation lane needs to modify an installed extension or Herdr-server.
