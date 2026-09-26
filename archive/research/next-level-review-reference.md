# Reviewr interaction reference

Research date: 2026-09-04. The installed local plugin is
`/home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8` (`persiyanov.reviewr`,
version 0.29.0), so this note uses its source and specifications as the primary reference. No
Herdr session, pane, or live configuration was changed.

## What reviewr collects

Reviewr has one in-memory `CommentStore` per review pane/worktree session. A comment carries the
repo-relative file, side (`new` or `old`), 1-based `start`/`end`, verbatim marker-prefixed source
lines, reviewer text, and whether it came from a diff or whole-file view
([`src/model.rs:78-118`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/model.rs>)).
The store appends, edits, and deletes comments; refresh does not remove them, while export consumes
 the whole set only after successful delivery ([`src/model.rs:142-166`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/model.rs>),
[`specs/review-model.md:95-104`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/review-model.md>)).

The same review loop works in both the `Changes` diff and `All files` whole-file view. In `All
files`, a comment is anchored to current-file line numbers, uses `new` side and space-prefixed
snippet lines, and exports identically to a diff comment ([`specs/review-model.md:85-93`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/review-model.md)>).
The file list and comment store are not restricted to one file: each comment retains its own file
path and range, and the aggregate export sorts by file then start line
([`src/export.rs:31-36`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/export.rs)>).

Selection is contiguous and may include deletion rows in diff view; folds are hard boundaries
([`specs/input.md:44-55`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/input.md>),
[`specs/diff-view.md:134-138`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/diff-view.md)>). A saved comment is rendered under its selected line/range and can be edited or deleted
in place ([`specs/input.md:68-70`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/input.md)>).

## Export format and exact delivery behavior

Each comment becomes three parts: `path:start-end` (or one line), the original snippet, and the
review text. Old-side comments append ` (removed)` to the location. Blocks are separated by one
blank line and sorted by file then start line ([`specs/review-model.md:106-132`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/review-model.md)>). The implementation is the authoritative detail:
`format_comment` emits location, `lines`, and normalized text; `format_all` sorts references and
joins blocks ([`src/export.rs:15-36`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/export.rs)>). Thus the path and range identify the target, while the verbatim selected original lines remain in the payload for context and anchoring.

`Send` with one candidate calls `herdr pane send-text <pane>`, then calls `herdr agent focus
<pane>`; with multiple candidates it opens a picker. A picker freezes its candidate rows and
returns to the mode/view from which Send was issued. It addresses the pane selected in that
frozen row, and a pane disappearing before delivery leaves every comment intact
([`src/app.rs:3435-3518`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/app.rs>),
[`specs/herdr-host.md:92-136`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/herdr-host.md)>).

Delivery is deliberately paste-only. `send_text` wraps the complete export in bracketed-paste
markers (`ESC[200~` ... `ESC[201~`), strips embedded paste terminators, and sends no Enter
([`src/herdr.rs:408-437`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/herdr.rs)). The plugin specification explicitly says Send injects every block, focuses the agent pane, and never submits; the reviewer can add context and submit from the agent input ([`specs/review-model.md:131-148`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/review-model.md)). `pane send-text` has literal no-Enter semantics in the Herdr API notes ([`docs/herdr-api-notes.md:143-161`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/docs/herdr-api-notes.md)>).

Focus failure does not roll back a successful text injection: `Agent::export` treats focus as a
convenience and returns success once `send_text` succeeds. A send failure leaves the complete
comment set, while a successful export clears it ([`src/export.rs:112-147`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/export.rs>),
[`src/app.rs:3521-3551`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/app.rs)). Clipboard Copy uses the same formatted batch and clears only after the clipboard command succeeds ([`src/export.rs:54-101`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/export.rs)>).

## Target filtering and lifecycle

Candidate resolution happens once before the send: entries must carry an `agent` field, belong to
the review pane's `HERDR_WORKSPACE_ID`, and not be the review pane itself. Reviewr does not narrow
the candidates to the review pane's tab; its own spec says every agent in the workspace is eligible
([`specs/herdr-host.md:94-103`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/herdr-host.md>)).
For Cockpit's requested same-tab interaction, add the stricter predicate `agent.tab_id ===
selectedTabId` (and the same session identity) before showing or arming a target; do not silently
fall back to another tab. Herdr's order is kept;
the picker optionally enriches rows with Herdr agent name/state and tab labels
([`src/herdr.rs:245-269`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/herdr.rs>),
[`src/herdr.rs:391-405`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/src/herdr.rs)). One candidate bypasses the picker; several open it; no candidate or failed enumeration reports a clipboard fallback ([`specs/herdr-host.md:92-106`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/herdr-host.md)>).

The review model is session-local and ephemeral: restart/close loses unexported comments; two
review panes on one worktree do not merge their stores. It has no durable lifecycle, categories,
severities, threads, line-number rebasing, or auto-submit ([`specs/review-model.md:136-148`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/review-model.md)>).

## Transferable design for Cockpit's read-only Context viewer

The useful seam is a viewer-owned draft collection independent of the file viewer: each draft should
carry `{path, start, end, selectedLines, text}` and a source kind (`file` or `diff`). Keep drafts
across file navigation and refresh; render the selected range and file path in each draft card; sort
only at export time so collection order does not matter. Build one deterministic multi-file payload
with explicit path/range headers and the selected original lines. Preserve newlines and send it as a
single paste event.

Cockpit already has the transport-neutral terminal seam needed to implement this. `CockpitClient`
exposes `TerminalStream.send(command)` and `openTerminal`; `TerminalCommand` has a single
`terminal.input` tag with exactly one text or base64-bytes field
([`src/client/CockpitClient.ts:54-82`](../src/client/CockpitClient.ts),
[`src/protocol/generated/v1.ts:59-73`](../src/protocol/generated/v1.ts),
[`crates/cockpit-protocol/src/v1.rs:434-511`](../crates/cockpit-protocol/src/v1.rs)). The native adapter
validates and forwards commands through Tauri, while the Herdr terminal wire converts text input
to `TextCommit`; observe attachments reject input and control attachments accept it
([`crates/cockpit-herdr/src/terminal_wire.rs:421-468`](../crates/cockpit-herdr/src/terminal_wire.rs)).
The current xterm pane forwards `onData`/`onBinary` through that command and only sends while the
focused attachment owns control ([`src/app/TerminalPane.tsx:71-84`](../src/app/TerminalPane.tsx),
[`src/app/TerminalPane.tsx:176-195`](../src/app/TerminalPane.tsx),
[`src/app/TerminalPane.tsx:244-255`](../src/app/TerminalPane.tsx)).

For the requested Context interaction, the target check should therefore be explicit before
delivery: resolve the selected session, owning agent pane, and current writable/control state;
reject with an inline target error if any identity or ownership check fails; use a dedicated acknowledged paste task after the wire-path probe below; never append `\n` or issue a second
Enter command. The current Cockpit terminal command protocol caps text input at 64 KiB, so larger collected reviews need a
visible split/refusal policy rather than silent truncation ([`crates/cockpit-protocol/src/v1.rs:474-511`](../crates/cockpit-protocol/src/v1.rs)).

The `/tmp/herdr-cockpit-master` source shows why this must be phrased as a wire-path requirement,
not an assumed `TextCommit` semantic. `pane.send_text` accepts a string and calls
`runtime.try_send_bytes(Bytes::from(params.text))` directly
([`src/app/api/panes.rs:1801-1817`](</tmp/herdr-cockpit-master/src/app/api/panes.rs>)); it does not call
the runtime's paste helper. Herdr's paste helper conditionally wraps text in bracket markers only
when bracketed-paste mode is enabled ([`src/pane.rs:3119-3132`](</tmp/herdr-cockpit-master/src/pane.rs>)).
By contrast, Cockpit's current `TextCommit` branch builds a structured client input event
([`crates/cockpit-herdr/src/terminal_wire.rs:455-468`](../crates/cockpit-herdr/src/terminal_wire.rs)),
which is a different path from native `RawInputEvent::Paste`; the latter invokes
`try_send_paste` ([`src/server/pane_input.rs:187-200`](</tmp/herdr-cockpit-master/src/server/pane_input.rs>),
[`src/server/pane_input.rs:321-332`](</tmp/herdr-cockpit-master/src/server/pane_input.rs>)). Therefore this
research does not claim that sending literal bracket markers inside Cockpit `TextCommit` is
equivalent to native paste. A robust implementation needs an explicit paste-capable protocol
operation (or a proven exact byte-input path) and an integration test against an agent input that
observes bracketed-paste framing.

The safe delivery state machine is bounded and conservative: validate payload size and same-tab
target identity, issue one paste request with a client operation id, and retain the drafts while
the result is pending or ambiguous. Clear drafts only after an authoritative success response;
never retry automatically after timeout/disconnect because the PTY may already contain the paste.
Offer an explicit retry only after the user chooses how to handle the uncertain outcome (for
example, verify the target input first or discard the pending batch). This gives at-most-once intent
under a known response and avoids claiming exactly-once delivery where PTY writes provide no such
acknowledgment.

This differs from reviewr's direct Herdr CLI target resolution: Cockpit's documented constraints
require Herdr-authoritative session/resource identity, separate DOM focus from semantic focus and
control intent, and route input only through the selected writable terminal attachment
([`research/ui-implementation-constraints.md:21-57`](ui-implementation-constraints.md)). A Context
viewer should remain read-only with respect to files and use the terminal input seam only for the
explicit “paste review to agent input” action; user submission remains a separate ordinary agent
input action.

## Limitations and verification gaps

- Reviewr's line anchor is not rebased when the diff shifts; the snippet is authoritative and the
  stored numeric side/range remains fixed ([`specs/review-model.md:24-28`](</home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/specs/review-model.md)>).
- Reviewr's export is ephemeral and one-shot; it is unsuitable as a durable review database.
- `pane send-text` is a Herdr CLI/plugin path that writes raw bytes; Cockpit's current
  `TerminalStream` `TextCommit` path is a structured input event and has not been proven equivalent
  to native bracketed paste. This read-only review did not run a live Context viewer or send against
  a user session.
- The current Cockpit protocol's 64 KiB limit applies to one input command. A multi-file payload
  must be checked before sending, and a future dedicated comment transport would need to preserve
  the same no-submit semantics explicitly.
- `terminal.input` is `TextCommit` at the Herdr wire layer; bracketed-paste framing is carried as
  text bytes and relies on Herdr/terminal input handling to interpret it. The existing reviewr
  implementation's terminator stripping is a useful security/correctness rule to carry over.
