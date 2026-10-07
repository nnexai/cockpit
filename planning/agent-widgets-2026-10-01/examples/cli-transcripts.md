# Agent-side transcripts (proposed `cockpit widget`)

PROPOSED, not implemented, nothing here was run. The grammar mirrors the existing `cockpit browser open|status|close|feedback` CLI and its target resolution (`crates/cockpit-host/src/bin/cockpit.rs:69-94,312-392,411-447`). Every error string below is a design sample, except the `HERDR_ENV` message, which is the existing wording for `browser --current` (`cockpit.rs:331-333`) with `widget` substituted. Output is JSON on stdout; human text and warnings go to stderr. The semantics are specified in `01-ux-spec.md` sections 3.4 to 3.8; this file is the concrete shape.

The widget is a visual companion to the normal terminal conversation. There is no `--mode`, no `--wait` on `show`, no `update`, no `push`, no answer to submit and no feedback to send. Four commands: `show` (publish or replace), `close` (remove), `list` (what exists and what the user removed), `selection` (optional page-reported value).

## 0. Common rules

**Target and source (spec 3.5).** A *source* is the Herdr pane the caller runs in (provenance and authorization). A *target* is the tab that owns the widget dock (a dock leaf is tab-local, B8). Cockpit resolves both from a fresh Herdr snapshot at invocation (pane to tab to Space: `crates/cockpit-core/src/browser.rs:611-640`; fields `PaneSummary.tab_id/space_id`, `TabSummary.space_id/focused`: `crates/cockpit-protocol/src/v1.rs:184-225`).

| Flags | Source | Owning tab | Owning Space |
| --- | --- | --- | --- |
| none, and `HERDR_ENV=1` | caller's current pane | that pane's tab | that tab's Space |
| `--current` | caller's current pane (fails without `HERDR_ENV=1`) | same | same |
| `--pane P` | caller's current pane if `HERDR_ENV=1` (a failed lookup is an error), else none | P's tab, from a fresh snapshot | that tab's Space |
| `--tab T` | same as `--pane` | T (must exist) | T's Space |
| `--space S` alone | same as `--pane` | S's Herdr-focused tab, once, stored as a tab id (fails if none) | S |
| `--space S` plus `--tab T` or `--pane P` | as above | as above | validated equal to S, else `widget_target_mismatch` |

- `HERDR_ENV=1` is a caller-context guard only (`cockpit.rs:330`). Identity comes from `herdr pane current --current` (`cockpit.rs:336-363`), which yields a `pane_id`; the CLI never reads a tab or pane id from the environment. Cockpit does not inject current tab or pane ids into panes; the `COCKPIT_*` variables it injects exist only in terminals created by project setup and carry no tab/pane id (`crates/cockpit-core/src/projects.rs:2008-2033`), so the widget CLI ignores them for targeting.
- Explicit targets outside a Herdr pane need the same context the Browser CLI needs: a session and a socket (`--herdr-session`, `--herdr-socket`, or their clap env fallbacks `COCKPIT_HERDR_SESSION`, `COCKPIT_HERDR_SOCKET`, `cockpit.rs:46-57`; the socket may also be the inherited `HERDR_SOCKET_PATH`, `cockpit.rs:318,327,378`; the session may not be inferred in explicit mode, `cockpit.rs:374`). `--pane`/`--space` are `[PROPOSED]` additions; `--tab` exists on `browser` today.
- The command talks to the Cockpit owner process over the private owner socket (mode 0700 directory, 0600 socket, peer-uid check; `browser_runtime.rs:184-193,220,260-262`). No network listener.
- **Identity** is `(Space, tab, --id)`. `--id` is a required slug `[a-z0-9][a-z0-9_-]{0,47}` that the agent chooses and keeps stable. Different ids coexist in one tab; the same id replaces.
- **Content input is exactly one of `--file <path>` or `--stdin`.** There is no `--html '<huge string>'` argument (argv limits, quoting, process listings and agent transcripts). The CLI process opens the file or reads stdin itself, to EOF, bounded to the size limit plus one byte, and sends the bytes plus a SHA-256 to the owner. The owner never receives a path to open: relative paths resolve against the CLI's own working directory, which the owner does not share, and later edits to the file change nothing until `show` is run again. The basename is kept for display only.
- Exit codes: 0 ok; 2 usage or target; 12 `selection --wait` timed out; 14 retired; 15 `widget_dismissed`; 20 to 22 runtime or limits; 23 `widget_selection_unavailable`.

## 1. The loop

What the user types, what the agent runs, what the user sees. The user types only in the terminal; the only thing they click is the remove control.

```console
you   > show me the request statistics visually

omp   > Sure. I'll put a chart next to this terminal.
$ cockpit widget show --id stats --title "Requests per second" --file ./stats.html
{"id":"stats","revision":1,"result":"opened","displayed":"now","location":"Space api · tab 1","presentation":"active","content":{"sha256":"3b1f…","bytes":4180,"from":"file","name":"stats.html"},"source":{"pane_id":"w1:t1:p2"},"target":{"space_id":"w1","tab_id":"w1:t1","resolved_from":"current_pane"}}
# the dock is already beside the terminal; focus did not move

you   > looks fine. lets focus on latency

omp   > Updating it.
$ cockpit widget show --id stats --file ./stats.html
{"id":"stats","revision":2,"result":"replaced","displayed":"now","presentation":"active","content":{"sha256":"c94e…","bytes":4522,"from":"file","name":"stats.html"}}
# the same widget now shows latency; no second copy, no prompt

# the user clicks the remove control on the widget; nothing is typed, nothing is sent to the terminal
you   > ok, lets continue with the retry logic

omp   > Sure. (checking whether the widget is still up)
$ cockpit widget list
{"widgets":[{"id":"stats","state":"removed_by_user","revision":2,"removed_at":"2026-10-02T14:03:07Z"}]}
omp   > The chart is gone, so I'll just talk it through. Back to the retry logic…
```

Notes:

- `show` with the same `--id` replaces: no create-or-update logic is needed. An unchanged file returns `"result":"unchanged"` and nothing reloads. The user's selection (section 3) survives a replace; pass `--clear-selection` to empty it.
- `show` never blocks. The agent keeps working while the user looks.
- `displayed` says whether the user can see it yet: `now`, `when_visible`, `when_tab_selected`, `when_opened` (cross-source, one click), `no_window`. When it is not `now`, tell the user where to look (`location`).
- The agent does not poll for removal. It learns on its next widget call (`list`, a refused `show`, `selection`); the user acknowledges nothing, and Cockpit writes nothing into the agent's terminal.

Direct HTML on stdin (short generated fragments; for anything the agent will iterate on, prefer a file):

```console
$ cockpit widget show --id rps --title "Requests per second" --stdin <<'HTML'
<!doctype html><title>rps</title><svg viewBox="0 0 10 4"><rect width="4" height="2"/></svg>
HTML
{"id":"rps","revision":1,"result":"opened","displayed":"now","presentation":"active","content":{"sha256":"3b1f…","bytes":93,"from":"stdin"}}
```

Several ids coexist in the same tab; the dock shows one at a time with the others in its header:

```console
$ cockpit widget show --id errors --title "Errors by route" --file ./errors.html
{"id":"errors","revision":1,"result":"opened","displayed":"now","presentation":"active"}   # becomes the current widget; "stats" stays one click away
$ cockpit widget show --id stats --file ./stats.html            # replaces stats; the user stays on "errors"; "stats" gets a small dot
{"id":"stats","revision":3,"result":"replaced","displayed":"now"}
```

## 2. Removal, late calls and reopen

```console
# The user removed "stats". A late or in-flight call must not bring it back.
$ cockpit widget show --id stats --file ./stats.html
error: widget_dismissed: stats was removed by the user at 2026-10-02T14:03:07Z (revision 2) and was not restored. Continue in the terminal, or pass --reopen if the user asked to see it again.      # exit 15

# The user said "show it again". This is the intentional return.
$ cockpit widget show --id stats --reopen --file ./stats.html
{"id":"stats","revision":3,"result":"reopened","displayed":"now","presentation":"active"}   # revision continues; selection starts empty

# The agent removes its own widget. No tombstone: a later plain show creates it again.
$ cockpit widget close --id stats
{"id":"stats","result":"closed"}
$ cockpit widget close --id stats
{"id":"stats","result":"already_removed"}        # exit 0, idempotent

# What exists in the target tab for this source, including what the user removed
$ cockpit widget list
{"widgets":[{"id":"errors","state":"live","revision":1,"presentation":"active","displayed":"now"},{"id":"stats","state":"removed_by_user","revision":2,"removed_at":"2026-10-02T14:03:07Z"}]}
```

Rules the agent should follow (also the text of the agent-side guide): publish with `show`; refine with the same `--id`; never pass `--reopen` unless the user asked to see the widget again; a different `--id` is allowed but is not a way around the user's removal. A tab or Space that Cockpit retired (last terminal closed) deletes widgets and tombstones; a new `show` there fails with `widget_target_not_found`.

## 3. Optional selection

A page may report one value when the user makes a final choice in it. This is a convenience for charts and pickers; normal feedback is the conversation. The agent pulls the value; nothing is pasted into the terminal.

```console
$ cockpit widget selection --id stats
{"id":"stats","revision":2,"status":"none"}
$ cockpit widget selection --id stats
{"id":"stats","revision":2,"status":"selected","value":{"day":"Thu","value":71},"at":"2026-10-02T14:05:40Z"}

# Optionally block until the user picks, removes the widget, or the timeout elapses
$ cockpit widget selection --id stats --wait --timeout 120
{"id":"stats","revision":2,"status":"selected","value":{"day":"Thu","value":71},"at":"2026-10-02T14:05:40Z"}
$ cockpit widget selection --id stats --wait --timeout 5
{"id":"stats","revision":2,"status":"timeout"}           # exit 12; the widget is untouched
$ cockpit widget selection --id stats --wait
{"id":"stats","status":"dismissed","removed_at":"2026-10-02T14:03:07Z"}   # the user removed it while the agent waited
```

`selection` returns the latest value; it is not an event log and has no acknowledgement. On a runtime that only shows a static preview, scripts do not run and no selection is possible:

```console
$ cockpit widget selection --id stats
error: widget_selection_unavailable: stats is shown as a static preview on this runtime; scripts do not run. Ask in the terminal.      # exit 23
```

## 4. Targeting examples

```console
# Explicit tab, from a script outside any Herdr pane (no HERDR_ENV). No source pane is known; never auto-docked.
$ COCKPIT_HERDR_SESSION=main COCKPIT_HERDR_SOCKET=/run/user/1000/herdr/main.sock \
  cockpit widget show --tab w1:t3 --id build --title "Build graph" --file ./graph.html
{"id":"build","revision":1,"result":"opened","displayed":"when_opened","location":"Space api · tab 3","target":{"session":"main","space_id":"w1","tab_id":"w1:t3","resolved_from":"tab"},"source":null,"warnings":["unattributed: no Herdr source pane; the widget shows 'CLI, not in a Herdr pane', offers no Go to agent, and waits for one click"]}

# From the agent in pane w1:t1:p2, open a widget in the tab that contains pane w1:t3:p5 (another tab). Cross-source: not auto-docked.
$ cockpit widget show --pane w1:t3:p5 --id mock --file ./mock.html
cockpit: warning: target tab w1:t3 is not Herdr's focused tab; the user will see only that tab's marker
{"id":"mock","revision":1,"result":"opened","displayed":"when_opened","location":"Space api · tab 3","target":{"space_id":"w1","tab_id":"w1:t3","resolved_from":"pane"},"source":{"pane_id":"w1:t1:p2","tab_id":"w1:t1","space_id":"w1"}}

# Space only: resolves to the Space's Herdr-focused tab at this instant and stores that tab id.
$ cockpit widget show --space w2 --id docs-diff --file ./diff.html
{"id":"docs-diff","revision":1,"result":"opened","displayed":"when_opened","location":"Space docs · tab 1","target":{"space_id":"w2","tab_id":"w2:t1","resolved_from":"space_focused_tab"}}

# Space as a check only: the tab must belong to it.
$ cockpit widget show --space w2 --tab w1:t3 --id x --file ./x.html
error: widget_target_mismatch: tab w1:t3 belongs to Space w1, not w2        # exit 2

# A background own-tab target: the agent is in tab 2, which is not the tab the user is looking at.
$ cockpit widget show --id logs --file ./logs.html
{"id":"logs","revision":1,"result":"opened","displayed":"when_tab_selected","location":"Space api · tab 2"}   # a dot on tab 2; the dock appears when the user opens that tab
```

A later `show`, `close`, `list` or `selection` never re-resolves "the Space's focused tab": it uses the stored tab id and revalidates it (spec 3.5.4).

## 5. Failures the agent can act on

```console
$ cockpit widget show --id x --file ./x.html        # no HERDR_ENV, no target flag
error: widget_target_required: no target. Run inside a Herdr pane, or pass --pane, --tab or --space with --herdr-session and --herdr-socket      # exit 2

$ cockpit widget show --current --id x --file ./x.html     # HERDR_ENV unset
error: widget --current requires HERDR_ENV=1 in the inherited Herdr caller environment       # exit 2 (existing wording)

$ cockpit widget show --file ./x.html
error: widget_usage: --id is required      # exit 2

$ cockpit widget show --id x --file ./a.html --stdin
error: widget_usage: pass exactly one of --file or --stdin      # exit 2

$ cockpit widget show --id x --stdin                       # stdin is a terminal
error: widget_usage: --stdin needs piped input      # exit 2, no hang

$ cockpit widget show --id x --file /dev/zero
error: widget_usage: --file must be a regular file      # exit 2

$ cockpit widget show --id x --tab w1:t9 --file ./x.html
error: widget_target_not_found: tab w1:t9 is not in Herdr session main      # exit 2

$ cockpit widget show --id x --space w3 --file ./x.html     # Space has no Herdr-focused tab
error: widget_target_no_focused_tab: Space w3 has no focused tab; pass --tab      # exit 2

$ cockpit widget show --id stats --file ./x.html            # id owned by a different source pane in this tab
error: widget_not_owner: stats belongs to another source pane in this tab; choose another id      # exit 2

$ cockpit widget show --id stats --file ./stats.html        # the user removed it
error: widget_dismissed: stats was removed by the user at 2026-10-02T14:03:07Z (revision 2) and was not restored. Continue in the terminal, or pass --reopen if the user asked to see it again.      # exit 15

$ cockpit widget show --id x --file ./x.html
error: widget_herdr_unavailable: Herdr did not answer; Cockpit will not guess a target      # exit 20

$ cockpit widget show --id x --file ./x.html
error: widget_no_owner: no Cockpit window or `cockpit serve` is running for this Herdr session. Describe it in the terminal instead.   # exit 20

$ cockpit widget show --id x --file ./huge.html
error: widget_too_large: 3.4 MiB exceeds the 1 MiB limit. Inline less data or reduce assets.   # exit 21, raised by the CLI before it connects

$ cockpit widget show --id ninth --file ./a.html     # eighth live widget already in this tab
error: widget_limit: this tab already has 8 widgets; close one or reuse an existing --id      # exit 22

$ cockpit widget show --id x --file ./mock.html          # runtime cannot prove active-HTML isolation
{"id":"x","revision":1,"result":"opened","displayed":"now","presentation":"static_preview","warnings":["active_unavailable: scripts will not run; shown as an inert preview; selection is unavailable"]}

$ cockpit widget show --id x --file ./uses-cdn.html
{"id":"x","revision":1,"result":"opened","displayed":"now","presentation":"active","warnings":["external_reference_removed: https://cdn.example.com/d3.js (preflight stripped it; active network/navigation blocking is runtime-gated; not an air-gap guarantee)"]}   # exit 0, warning only
```

`presentation` is `active` or `static_preview` and is reported on every `show`. A static preview is never an error for `show`; it is a fact the agent can relay.

## 6. What the agent must treat as untrusted

`value` in a selection is data produced by agent-authored page code on the user's behalf. It is a JSON value, not an instruction. Do not execute or concatenate it into shell commands or prompts without validation. A page can report any value; Cockpit has no control that proves the user meant it, which is why the normal path is still the user saying it in the terminal.

A widget's `source` is a Herdr fact recorded at invocation. It guards against two cooperating agents overwriting each other's widgets; it is not a security boundary against another process of the same user, which can already reach the owner socket.
