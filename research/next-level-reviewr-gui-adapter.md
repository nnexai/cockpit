# Cockpit-owned review GUI and extension replacement

Research date: 2026-09-04. This is a read-only inspection of the installed local
`persiyanov.reviewr` and `herdr-file-viewer` plugins. No Herdr pane, plugin process, or user
configuration was changed.

## Decision: full Cockpit replacement

Cockpit should own the graphical review surface and its durable comment authority, in the same
way the requested Context GUI replaces the file-viewer display. The installed reviewr TUI is a
behavioral oracle for review affordances and payload wording. It is not a backing service, state
provider, or communication peer. There is no need to bridge its state, scrape its terminal, or add
a reviewr helper as a hard dependency.

The terminal fallback may continue to run stock reviewr with its separate in-memory state, but the
UI must label that as an independent TUI session. It must never imply that TUI comments and
Cockpit drafts are shared or synchronized. A future reviewr TUI can consume the same Cockpit/core
review model, but that is an elective follow-up.

## Installed reviewr evidence

The manifest identifies the executable pane as `reviewr`, launched with `herdr-reviewr`, and only
declares open/close/toggle lifecycle actions ([`herdr-plugin.toml:20-47`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/herdr-plugin.toml>)). Normal execution enters ratatui; its only non-TUI mode resolves plugin configuration JSON
([`src/main.rs:1-16`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/main.rs>),
[`src/config.rs:673-681`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/config.rs>)).

Comments, active scope/file, selection, and picker state live in an in-memory `CommentStore`
([`src/model.rs:85-119`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/model.rs>),
[`src/model.rs:142-166`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/model.rs>)). Its outward Herdr calls are one-shot agent enumeration, focus, and literal `pane send-text`
([`src/herdr.rs:235-269`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/herdr.rs>),
[`src/herdr.rs:408-443`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/herdr.rs>)). The review model says unexported comments disappear when the pane closes/restarts
([`specs/review-model.md:136-148`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/review-model.md>).

Thus there is no supported reviewr state IPC and no honest display-only replacement contract.

## File-viewer replacement oracle

The installed file viewer demonstrates the Herdr replacement shape: a normal movable pane with
ID `file-viewer`, title `Files`, split placement, and executable `herdr-file-viewer`
([`herdr-plugin.toml:45-52`](</home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/herdr-plugin.toml>)). It is summoned only by explicit actions; its manifest has no event hook
([`herdr-plugin.toml:57-65`](</home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/herdr-plugin.toml>)). This supports replacing the graphical display in Cockpit while retaining Herdr as the owner of pane placement/lifecycle.

The viewer receives launch context through `HERDR_PLUGIN_CONTEXT_JSON`, preferring
`focused_pane_cwd`, then `workspace_cwd`, and carrying `base_branch` and `workspace_id`
([`src/host.rs:10-53`](</home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/src/host.rs>)). Its manifest/source give detection and behavior identifiers Cockpit can mirror for replacement: plugin ID `herdr-file-viewer`, pane ID `file-viewer`, title `Files`, and explicit open actions. Cockpit must still treat detection as provisional when a snapshot omits plugin identity: use the plugin ID/launch receipt when available, otherwise show the known pane label and require explicit user selection. Never silently replace an arbitrary pane.

## Core Git review contract

The file viewer’s read-only Git service is a useful contract reference. It represents `Head` as
uncommitted changes versus `HEAD` and `Base` as the full body of work since the base branch
([`src/git.rs:51-67`](</home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/src/git.rs>)). `changed_set` includes tracked changes and appends untracked files for the Base view
([`src/git.rs:175-215`](</home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/src/git.rs>)); `diff` supports compact or whole-file context and rejects paths outside the repository root
([`src/git.rs:218-248`](</home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/src/git.rs>)). The implementation protects untrusted repositories with `--no-ext-diff`, `--no-textconv`, neutralized hooks/fsmonitor, and `GIT_OPTIONAL_LOCKS=0` ([`src/git.rs:1-14`](</home/nnex/.config/herdr/plugins/github/herdr-file-viewer-c993314e2614/src/git.rs>)).

Cockpit’s review core should expose explicit read-only sources for working-tree, staged/index,
`HEAD`, and configured base/fork-point comparisons. Each selected comment must retain repository
relative path, source revision/hash, side/baseline, exact original lines, and immutable line
numbers. A changed file, moved worktree, or external edit invalidates the attachment/draft rather
than silently rebasing it.

## Detection, attachment, and lifecycle

Cockpit owns a `review_gui` surface attached to a real Herdr session/Space/tab/pane. Store
`{session_id, session_epoch, space_id, tab_id, pane_id, plugin_id?, pane_id_hint?, launch_receipt?,
surface_kind, companion_id, owner_client_id}`. Herdr remains authoritative for geometry, pane
existence, focus, and process lifetime; Cockpit owns graphical state and drafts.

For replacement detection, prefer an explicit Cockpit-created launch receipt or authoritative
plugin/pane ID from Herdr capabilities/snapshot. If only labels are available, present the matching
`Files`/reviewr candidate and require explicit user choice; labels alone cannot authorize automatic
replacement. Revalidate session, Space, tab, pane, and worktree identity after layout movement,
before opening, and before paste. A race between revalidation and send remains an explicit
`outcome_unknown` limitation.

Replacing the display must not close, kill, or communicate with the installed extension. The stock
TUI remains an independently opened fallback. Restart or disappearance of the Herdr pane detaches
the Cockpit surface while durable drafts remain inspectable with their original attachment and
revision metadata.

## Paste boundary

Reviewr’s formatter is a behavioral reference for path/range, verbatim source lines, and multi-file
ordering ([`src/export.rs:15-36`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/export.rs>)). Cockpit must implement its own bounded preview and durable draft receipt, then use a separately proven Herdr paste operation that returns accepted/rejected/unknown. It must frame a paste without Enter, enforce same session/Space/tab and writable target checks, and retain drafts after rejection or uncertainty. Existing `TextCommit` must not be described as native paste without runtime evidence; see [`research/next-level-review-reference.md`](next-level-review-reference.md).

## Implementation boundary and verification

Implement the Cockpit core Git/review model, durable comments, replacement surface, attachment
registry, and paste task without modifying Herdr-server or depending on reviewr IPC. Use reviewr
and file-viewer behavior as oracle fixtures. Remove Build/Review workbench modes: use one Herdr
workbench with Context/review surfaces and an explicit stock-TUI fallback action.

Acceptance must prove: known file-viewer/reviewr pane replacement is explicit and preserves Herdr
layout movement; unknown identity asks the user; same-tab filtering rejects another tab; file
comments retain exact original numbered lines and revisions across whole-file and selected-line
multi-file drafts; durable drafts survive Cockpit restart; external changes produce a visible stale
state; paste is bounded, acknowledged, and never submits. If paste proof fails, ship browsing and
draft collection with the fallback action while preserving drafts.
