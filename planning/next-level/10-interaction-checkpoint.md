# Second checkpoint: daily workflow interactions

The initial plan was committed as `b5cc472`. This checkpoint brings its main interactions together in [the workflow lab](mocks/workflow.html). It is a discussion prototype, not implemented Cockpit functionality. The plan index retains separate stories for all deferred capabilities; the lab's “All planned blocks” menu links those concepts into the workbench without pretending they are operational.

## Proposed daily loop

Keep Spaces and Agents separate. Open a real extension pane through Herdr, detect its identity, and replace its terminal presentation with Context or the complete Reviewr GUI. Their headers, placement, focus, and zoom belong to the same pane layout as agents. No extension communication and no global Build/Review switch are introduced.

Reading and collecting comments stays inside the GUI pane. The pane header shows `N comments`; a click or Ctrl+Shift+M opens the batch overview with its same-tab target. There is no persistent bottom comments panel. The retained quick-interaction proposal is: the paste shortcut or an explicit **Paste to [agent]** action in the overview sends the collected text without submitting it. Preview is always available; an “always preview” preference adds that step for users who want it. An invalid/ambiguous target, source revision conflict, or unknown delivery outcome remains visible and cannot be silently retried. This direct-paste default is a second-checkpoint proposal, superseding the initial mock's mandatory preview only if accepted.

The draft editor opens from the selection, saves with one shortcut, and returns focus to the file. Unsent comments appear inline after their source/diff range, or below the file for whole-file comments and rendered documents. Whole-file comments contain the actual path and comment; selected-line comments also contain original numbered lines. Review selections identify old or new revision coordinates. The prototype refuses a selection spanning both sides.

## Candidate keymap

These bindings are proposals to test in the real native and browser applications. Browser/WebKit reservations and terminal input behavior need explicit runtime verification before adoption. GUI bindings do not intercept terminal keys or typing in unrelated editors. Preserve the existing Herdr prefix handler and its priority.

| Action | GUI keyboard proposal | Mouse |
|---|---|---|
| Find a file | Ctrl+P, type, Enter | Open file or tree item |
| Find a command/open a graphical pane | Ctrl+Shift+P, type, Enter | Commands / + Pane |
| Move within source | Arrows, Home, End | Click source gutter |
| Extend a source range | Shift+arrows | Shift-click or drag gutter |
| Comment on range / whole file | C / Shift+C | Selection action / + Comment |
| Collect comment | Ctrl+Enter inside editor | Collect |
| Source/rendered view | Alt+Enter | Source / Rendered |
| Paste collected comments | Ctrl+Shift+Enter, GUI only | Explicit Paste to named target |
| Open comment overview | Ctrl+Shift+M, GUI only | N comments in pane header |
| Preview exact text | Focus Preview, Enter | Preview |
| Cycle major focus regions | F6 / Shift+F6 | Click region |
| Move pane | Existing Herdr layout commands | Header drag or placement menu |
| Zoom pane | Existing Ctrl+B then Z | Double-click header |
| Fast setup | Spaces +, fields, Ctrl+Enter to review, again to create | Same visible actions |
| Dismiss current overlay | Escape | Close / cancel |

F6 is a candidate workbench navigation binding, including escape from the mock terminal; unlike ordinary GUI shortcuts, adopting that behavior in Cockpit requires a deliberate reserved-key decision. The mock's prefix zoom illustrates the graphical pane only. Actual implementation must operate on Herdr's focused pane and retain current terminal-prefix behavior. Do not treat the prototype's local focus model as that authority.

## Efficiency checks for the discussion

Counts below are control actions, excluding typing text, navigation distance, and initial pane creation. They are acceptance targets, not measurements of real Cockpit. Count a keyboard chord as one action and record file-query typing separately.

- From a focused source: select range, C, type comment, Ctrl+Enter, Ctrl+Shift+Enter. No preview or target-selection step when the existing visible target is correct.
- Additional comment in the same file: select range, C, type, Ctrl+Enter. The editor returns to source so the next range is reachable without relocating focus.
- Whole-file comment after quick open: Shift+C, type, Ctrl+Enter. No need to select all lines.
- Switch reading/source view in one action; keep file, scroll position, and selection in the production implementation.
- Setup for a known repository keeps worktree and context defaults collapsed. Review the actual planned effects once before creating resources. An artifact URL never starts mutation by itself.
- Pane placement has a pointer route and an existing Herdr keyboard route; do not require dragging for keyboard users.

Try a three-file batch, a review deletion comment, and a quick setup. Note where focus is unclear, a shortcut is unavailable, or a control forces an unnecessary action. Discuss direct paste versus always-preview, focus cycling, and which setup defaults should be easiest to change in code.

## Prototype coverage and limits

The lab includes Context source/rendered fixtures, an illustrative Mermaid SVG, Reviewr old/new gutters, comment edits/removal, numbered payload preview, same-tab target choices, source-change acknowledgement, rejected/unknown delivery, fast setup, source recovery sketches, pane placement, fallback terminal presentation, and optional-feature entry sketches. Scenarios are selected in the prototype toolbar.

Drafts and delivery records use localStorage when available. A pending receipt restored after reload becomes unknown; marking it pasted archives it without writing text again. The mock agent input is a textarea: explicit paste fills it, and only a later user Enter submits to the mock. This proves prototype behavior, not Herdr or an agent's bracketed-paste semantics.

No real filesystem, provider, Markdown/Mermaid engine, process detection, pane layout, transport, or agent runs. Setup/recovery and optional panels are illustrative; changing a review scope does not calculate a diff. The production stories retain their own tests and runtime gates.

Chromium automation at 1440×900 and 1024×640 exercised quick-open, keyboard range selection, whole-file comments, exact numbered payloads, direct paste without submit, subsequent explicit Enter, pane placement/zoom, old-side review comments, unknown results across reload, marking a batch pasted without another write, and setup review/create. JavaScript syntax and no-horizontal-overflow checks passed. Real browser/native shortcut collision and accessibility checks remain implementation acceptance work.

## Feedback incorporated

The user liked the quick interactions and requested the header-count/on-demand-overview design in place of the comments bottom panel. The revised mock shows unsent range comments inline in source/diffs and file comments beneath content. Its overview is available by click and Ctrl+Shift+M. Pane/ellipsis controls and source setup were called out as awkward but are outside this focused comment revision; their current styling/flow is not final approval.
