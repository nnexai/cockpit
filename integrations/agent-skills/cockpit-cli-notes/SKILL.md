---
name: cockpit-cli-notes
description: Use when reading or editing Cockpit Space Notes through cockpit-cli - Scratchpad, todos, Kanban, decisions and todo comments, with explicit targets, revision fences and safe recovery. Not for the read-only Cockpit Library or supervisor canonical tasks.
---

# Cockpit Space Notes with cockpit-cli

Notes are user-owned Markdown under the configured `notes_root` or
`COCKPIT_NOTES_ROOT`. On Linux the default is `$XDG_DATA_HOME/cockpit/notes`,
with `$HOME/.local/share` as the unset-XDG fallback. Notes are separate from
Library items and orchestration tasks.

Use the installed `cockpit-cli`; in Cockpit-launched panes prefer the actual
`$COCKPIT_CLI_PATH` when set, rather than an older binary on PATH. Help works
without a running Cockpit, Herdr pane or Notes target:

```sh
cockpit-cli notes --help
cockpit-cli notes target --help
cockpit-cli notes todo complete --help
```

Read the relevant `notes <area> --help` and verb help before a write. Commands
below are templates: populate variables from real read results, not labels,
titles, cwd or guessed IDs. Writes require the user's requested Notes work.

## 1. Resolve once, then pin UUID and root

```sh
cockpit-cli notes --current target
cockpit-cli notes --space "$SPACE_ID" --herdr-session "$SESSION" --herdr-socket "$SOCKET" target
```

Choose one resolution command. `--current` needs a real Herdr pane
(`HERDR_ENV=1`); explicit `--space` requires both session and socket options.
An unbound Space returns `notes_unbound` (exit 2), not permission to create
content. Only on explicit request use `target --create` or `target --attach
UUID`. Attach can transfer another Space's association; `notes_already_bound`
(exit 9) means leave the existing binding alone. `notes catalog` is read-only
and accepts no target.

Record `notes_id` as `NOTES_ID` and the parent directory of
`result.info.folder` as `NOTES_ROOT`. Pin both for every later content call:

```sh
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" todo list
```

Pinned `--notes` calls need neither Herdr nor a running Cockpit owner. Do not
re-resolve a Space on each write. `--config` / `COCKPIT_CONFIG_PATH` can select
configuration, but must not silently change the pinned root.

## 2. Read first; use the exact returned revision

Revisions are `sha256:` plus 64 lowercase hex digits. Use returned values,
not `target.info.change_tokens`: those are freshness tokens, not write fences.

| Write | Fence and source |
| --- | --- |
| `scratchpad append` | Optional `--expected-revision` from `result.document.revision` in `scratchpad read`; use it for read-modify-write work. `absent` fences a missing file. |
| `scratchpad replace` | Required revision from the same read, including `absent`; accepts `--stdin` or `--file`, not `--text`. |
| `todo update/complete/reopen/remove`, `kanban promote/move/unboard` | Unique non-null todo `id`: `--id ID --expected-revision REV` with that todo's own `revision`. Otherwise use its returned `--ref` alone; never combine ref with id or expected revision. |
| `decision update/replace` | `--expected-revision` from `result.decision.summary.revision` in `decision get` or that summary in `result.decisions` from `decision list`. |
| `comment update/remove` | `--expected-revision` from `result.comment.revision` in `comment get` or the selected entry in `result.comments` from `comment list`. |
| `todo add`, `kanban add`, `decision create`, `comment add` | No revision fence; every invocation creates a new record. Inspect before replaying an uncertain result. |

Todo `ref` has the form `L<n>@sha256:<hash>` and uses the whole-file revision,
not the item's revision. Any edit to that file invalidates it. A ref-selected
write that keeps the todo adopts/repairs its stable ID; remove returns only
the resulting document revision. For a `duplicate_id`
problem, use the returned ref rather than guessing which duplicate to edit;
malformed ownership is refused, not permission to rewrite surrounding prose.

## 3. Common read and write workflows

Set `REVISION` from the specific item's latest read before each fenced write;
these lines are not a batch to execute unchanged. `TODO_REF` is the complete
returned ref. Decision and comment IDs come from their returned records.

```sh
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" scratchpad read
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" scratchpad append --text "- Isolate flaky test" --expected-revision "$REVISION"
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" scratchpad replace --file scratchpad.md --expected-revision "$REVISION"
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" todo list --open
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" todo add --text "Add regression test" --lane backlog
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" todo complete --id "$TODO_ID" --expected-revision "$REVISION"
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" todo complete --ref "$TODO_REF"
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" kanban list
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" kanban move --id "$TODO_ID" --expected-revision "$REVISION" --to doing
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" kanban unboard --id "$TODO_ID" --expected-revision "$REVISION"
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" decision list --status current
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" decision get --id "$DECISION_ID"
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" decision create --title "Use owned fixtures" --file decision.md
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" decision replace --id "$DECISION_ID" --expected-revision "$REVISION" --title "Use disposable fixtures" --file decision.md
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" comment list --todo "$TODO_ID"
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" comment add --todo "$TODO_ID" --text "Repro needs a clean HOME."
COCKPIT_NOTES_ROOT="$NOTES_ROOT" cockpit-cli notes --notes "$NOTES_ID" comment update --todo "$TODO_ID" --comment "$COMMENT_ID" --expected-revision "$REVISION" --file comment.md
```

Payload flags `--text`, `--stdin` and `--file` are mutually exclusive. Prefer
file/stdin for shell-sensitive Markdown. Scratchpad input is bounded to 1 MiB,
decision input to 256 KiB and comments to 64 KiB; decision verbs reject
`--text`. Decision create/replace may omit the body (empty Markdown); update
may change the title without replacing the body. Todo text is bounded to
2 KiB. Other document/collection limits can also reject a write.

Backlog/Doing are todo lane metadata; Done is the checkbox. `kanban add` adds
a Backlog todo. `promote` gives an unboarded todo Backlog without changing
its checkbox; an already boarded todo retains its lane. `move --to done`
checks it. Moves never reorder `todos.md`. `unboard` keeps the todo and comments;
`reopen` restores the remembered lane. Todo removal retains comment threads
and refuses removal with nested tasks. Replacing a decision creates a linked
new record while preserving the old one; already replaced sources refuse
another replacement. List statuses are `current|history|all`; `--query` searches
title/body substrings. `--author` on a comment is an unverified label, not identity.

## 4. Outcomes and recovery

Normal operations print one JSON object on stdout. Success is exit 0 with
`notes_id`, `changed` and tagged `result.kind`; `changed: false` means nothing
was written. Errors print `{"error":{"code":"...","message":"..."}}` on
stdout and `error: CODE: MESSAGE` on stderr. Help is text, not this JSON shape.

| Exit | Meaning |
| --- | --- |
| 2 | Usage, missing target or unbound Space |
| 4 | Not found or not on the board |
| 9 | Conflict, already replaced decision or already bound Space |
| 10 | Ambiguous/malformed todo or removal with nested tasks |
| 11 | Invalid input/encoding/target, too large or unsafe path |
| 12 | Busy |
| 20 | Other error or unknown outcome |

- `notes_conflict`: re-read and re-decide using the new revision, never reuse
  the stale one or silently overwrite someone else's edits.
- `notes_busy`: lock timed out before writing; retry later.
- `notes_outcome_unknown` or lost write response: the write may have landed.
  Re-read the affected content before another write. For unfenced creates,
  look for your record and explicitly account for possible duplication;
  never replay blindly. A failed read does not prove the write failed.
- External editors can ignore Cockpit's locks and race. Revision fences
  protect participating writers, not every filesystem editor.

Keep private note bodies and author labels out of reusable artifacts. Do not
create, attach, rebind or remove unrelated Notes as part of discovery.
