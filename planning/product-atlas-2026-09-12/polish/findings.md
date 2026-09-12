# Cockpit UI polish findings

2026-09-12 · Design artifacts only. No application implementation.

[Mock](index.html) · [Original atlas](../current/index.html) · [Verification](verification.md)

## User decisions

These supersede earlier suggestions in the review:

- Do not change terminal content.
- Keep the existing small browser annotation toolbar and inline/overlay notes. Preserve **freehand drawing**.
- The annotation toolbar becomes translucent when idle; hover or keyboard-visible focus restores full opacity. Notes and drawings do not fade.
- Do not add an “Add idea” action, descriptive slogans, explanatory subtitles, or a new capture/persistence workflow. Select an element or region and write the note there; draw directly with freehand.
- A Notes sidebar is optional, for finding and revisiting notes. It is closed by default and is not required for authoring.
- Setup is used **5–20 times per day**. Use a compact single-screen form, not a teaching wizard.
- Issue/MR URLs fill repository, branch, and default Space name. A manually entered branch also becomes the default Space name; an explicitly edited name is preserved.
- Repository actions are always allowed for this personal IDE. Remove the consent checkbox; do not replace it with another confirmation.
- Files and Review both need a collapsible file overview and a file picker. Removing the Files overview was inconsistent.
- Do this work directly, without delegation. No subagent performed this work.

## Changes shown in the mock

### Workbench and Files

Observed: session, tabs, pane titles, and actions have similar visual weight. Files places several toolbar/metadata layers before the document. Generated source names dominate navigation. Large document headings wrap excessively in narrow panes.

Proposed:

- Retain the existing Session → Spaces → tabs → panes hierarchy and separate Agents section.
- Align headers and distinguish selected tab, selected pane, and terminal attachment state.
- Keep Pane and Commands at the tab bar's right edge, as required by the existing decision.
- Keep a collapsible file overview in Files, matching Review. Both have a Files toggle and the same Go to file picker pattern. Use pane width to choose an inline overview or an overlay; keep the toggle available in either case.
- Show a readable document identity while preserving access to the exact path and provenance in Details. The mock abbreviates capture-era storage IDs; an implementation must expose full copyable values.
- Reduce metadata to one compact row, with Preview/Source and Details controls. Remove instructional prose from the content area.
- Use restrained headings and readable body text. Do not shrink terminal or source text to make more columns fit.
- Keep Files/Review comment controls and counts at the bottom, rather than adding a duplicate top toolbar.

Terminal preservation: `assets/terminal-original.png` is an exact 598×840 crop of `../current/app-files-source.png`, rectangle `(236, 59, 834, 899)`. The mock renders it at natural dimensions inside a scrollable viewport. No terminal text, colors, spacing, or glyphs were reconstructed.

### Narrow panes and Review

Observed: the 480px three-pane Review captures give each pane roughly 150px. Controls wrap or lose labels, the document heading becomes a vertical column, and the comment editor is too narrow to use.

Proposed:

- Keep the actual split until the user explicitly expands a pane. Do not silently change Herdr layout at a breakpoint.
- Let both file overviews collapse when pane width is insufficient. Use the same toggle/picker affordances in Files and Review.
- Preserve file, side, original line range, and hunk identity. Scroll source horizontally rather than shrinking text.
- Keep the comment editor inline at usable widths; show a bounded sheet when narrow. This is a proposed presentation change, not an implemented product behavior.
- Preserve Enter for newline and Ctrl/Cmd+Enter for saving a comment. Saving never pastes or submits.

The mock's Review scene is a selected expanded-pane example. Switching study scenes is not a proposal for new product modes.

### Setup

Observed: the current dialog repeats repository identity, configuration details, optional-field explanations, and lifecycle text. The final action is absent from the review capture's visible area.

Revised proposal for frequent use:

- One form: optional issue/MR URL first, repository, new worktree/existing checkout, branch or checkout, and Space name.
- URL metadata fills repository and branch. Issue branch names use a derived naming default; MR/PR metadata uses its source branch. Space name follows the branch until explicitly overridden.
- Selecting a project and entering a branch needs no separate naming step. Updating that branch continues to update the default name; a custom name is retained. Advanced destination defaults also track the branch unless edited.
- No stepper, welcome copy, or repeated explanation of what a worktree is.
- Put base revision and destination in collapsed Advanced settings.
- Put exact effects and configured repository actions in collapsed Operation details. Safety information remains available; it does not require rereading a tutorial each time.
- No actions-consent checkbox. The user explicitly selected automatic repository actions for this personal tool. This revises the earlier design recommendation; application/backend behavior is not changed by this mock.
- Use a stable, specific Create Space/Open Space action. In this mock it is disabled because no operation exists.

The default desktop and portrait form fit without navigating through multiple steps. Opening advanced details may require scrolling. Example paths and effects are not a validated plan or permission to mutate a repository.

The URL autofill demonstration uses local fixture metadata: `https://github.com/nnexai/cockpit/issues/4` fills `cockpit` / `issue-4-terminal-width`; `https://github.com/nnexai/cockpit/pull/7` fills `cockpit` / `ui-polish`. The PR fixture is illustrative, not fetched or claimed to describe a real PR. Unknown URLs do not fabricate remote metadata. An eventual implementation must resolve the source against configured local repositories and use real provider metadata.

### Browser annotation

The existing small toolbar and inline annotation model are the baseline to retain. The earlier mock added too much UI; that direction was rejected.

The revised mock keeps:

1. Browse.
2. Freehand.
3. Element targeting.
4. Region targeting.
5. Clear annotations.
6. Capture-position control.
7. Close.

The toolbar is a compact overlay, not a separate workflow panel. The screenshot/capture-position control opens original evidence in this mock; it does not create or save a capture.

Idle toolbar opacity is 45%; hover and keyboard-visible focus use 100%. Transparency applies only to toolbar chrome, not the page, annotations, or terminal content.

- Element selection opens an inline textarea. The mock has one example element target on the captured page.
- Region drag draws a rectangle and opens a comment beside it.
- Freehand draws directly on the overlay, without a save dialog or recipient step.
- Done closes the editor into an inline note. Multiple notes can be revisited through the optional Notes sidebar.
- The Notes control is secondary, outside the seven-button annotation toolbar. The sidebar is closed by default.
- No “Add idea” button, branding block, descriptive mode subtitle, instruction strip, delivery panel, or persistence dashboard is added.

The backdrop is a cropped original page screenshot, not a live DOM. Freehand/region geometry and note text are local mock state. Notes disappearing on reload is a mock limitation, not a recommendation that the product discard them. Any future durable storage should remain unobtrusive.

### Menus and failure information

Observed: the command palette has crowded/clipped bottom content; the resource menu mixes layout, renderer internals, unavailable actions, and Close. The browser feedback capture combines unknown browser outcome and missing agent, while the popup's Connected state can be mistaken for successful capture/delivery.

Proposed:

- One command search input, one scrolling result list, a small keyboard footer.
- Group resource operations and keep renderer diagnostics under Advanced. Separate destructive actions.
- Do not turn backend state into the default annotation experience. Show relevant failures beside the operation that failed; never claim a draft is safe without evidence.
- If delivery is explicitly requested, distinguish connection, capture, recipient eligibility, and delivery. No eligible agent means no delivery action; unknown mutations must not be blindly retried.

## Constraints retained

- Herdr owns resource identity, hierarchy, ordering, pane layout, semantic focus, and terminal ownership. The mock changes no live state.
- No synthetic Context tab, new product mode, local agent sorting, PTY, or terminal rewriting.
- Pane/Commands placement and bottom comment actions follow the current recorded decisions.
- The mock's review navigation and headers are outside the proposed application surface. Example state is not evidence of live attachment or persistence.
- Only files in this `polish/` directory are owned by this work. Existing atlas captures, manifests, application files, and unrelated work remain untouched.

## Screenshot coverage

All 24 current images were inspected. Links below are original evidence, not replacement mock screenshots.

| Original capture | Finding or limitation |
| --- | --- |
| [Workbench](../current/app-shell-desktop.png) | Session/tab/pane hierarchy; Agents empty |
| [Collapsed sidebar](../current/app-sidebar-collapsed.png) | Ambiguous rail toggle |
| [Drawer 480](../current/app-drawer-480.png) | Drawer frame visible but contents blank in capture |
| [Drawer 800](../current/app-drawer-800.png) | Actually shows drawer closed; toggle overlaps tab area |
| [Commands desktop](../current/app-commands-search-desktop.png) | Clipped/crowded bottom area |
| [Commands 480](../current/app-commands-search-480.png) | Same result/footer hierarchy issue |
| [Resource menu](../current/app-resource-menu.png) | Flat grouping and exposed renderer internals |
| [Setup draft](../current/app-setup-draft.png) | Repeated identity and explanations |
| [Setup review](../current/app-setup-review-plan.png) | Final action outside captured viewport |
| [Files](../current/app-files-source.png) | Metadata stack, generated names; terminal crop source |
| [Files picker](../current/app-files-picker-dialog.png) | Machine filename dominates result |
| [Context issue](../current/app-context-issue-1440.png) | Captured pane is labeled Files |
| [Context split](../current/app-context-split-360.png) | Narrow toolbar wrapping and oversized heading |
| [Context portrait](../current/app-context-zoom-480.png) | Drawer obscures document |
| [Review diff](../current/app-review-diff.png) | Actual 480×900, not listed 1440×900; squeezed split |
| [Review composer](../current/app-review-line-comment-desktop.png) | Actual 480×900, not listed 1440×900; unusably narrow editor |
| [Review saved comment](../current/app-review-line-comment-480.png) | Expanded portrait layout is the usable baseline |
| [Extension toolbar](../current/screenshots/browser-toolbar-desktop.png) | Preserve compact toolbar and freehand; captured save failure needs truthful handling |
| [Extension portrait](../current/screenshots/browser-toolbar-portrait.png) | Preserve small overlay; avoid adding more controls/text |
| [Element hover](../current/screenshots/browser-element-hover-outline.png) | Preserve precise element targeting |
| [Element editor](../current/screenshots/browser-element-comment-editor.png) | Preserve inline authoring; polish sizing and focus |
| [Region comment](../current/screenshots/browser-region-comment.png) | Preserve overlay selection and annotations |
| [Feedback panel](../current/screenshots/browser-cockpit-feedback.png) | Separate unknown outcome from recipient eligibility only when relevant |
| [Popup](../current/screenshots/browser-popup-pending.png) | Do not make storage/capture/delivery the primary annotation workflow |

## Opening and checking the mock

Open `index.html` in a browser; no build or server is required. Keep its sibling files and the original atlas at their relative paths.

- Workbench: toggle Spaces, Files, picker, Preview/Source, Details, Pane, Commands, and explicit document expansion.
- Review: toggle Files, use the picker, select line 381, add/edit/cancel a local comment, and try the narrow editor.
- Setup: change operation, edit inputs, expand Advanced or Operation details. The final action stays disabled.
- Browser: use freehand, select the example element, drag a region, write inline notes, optionally open Notes, clear mock annotations, and close/reopen the toolbar.

[Verification](verification.md) records actual browser checks and final captures. It does not claim application, native, Herdr, extension-installation, provider, or production-persistence verification.
