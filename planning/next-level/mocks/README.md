# HTML planning mocks

Open [index.html](index.html) in a browser. It is self-contained and needs no server, package installation, network, credentials, or running Cockpit/Herdr session.

The selected direction detects real file-viewer/Reviewr panes and replaces their renderer without extension IPC. [workbench.html](workbench.html), [setup.html](setup.html), and [reviewr.html](reviewr.html) illustrate it. A/B/C in the historical directions page are superseded. Fonts use installed IBM Plex or the platform fallback; the production implementation should retain Cockpit's bundled fonts.

Interactive examples:

1. Switch between three layouts using the top buttons.
2. Select an issue, personal note, repository source file, or wiki file.
3. Choose Source lines, click a line number, then Shift-click another line number.
4. Add a range comment or use `+ Comment` for the whole file.
5. Collect comments across files, remove an item, and preview the numbered text.
6. Simulate paste. The page changes a sample terminal prompt and clears its sample drafts; it never sends data anywhere.
7. Open the Spaces `+` for a setup sketch, Sources for source states, or Search for a sample result.

This is a design study, not a production prototype. The terminal is static text. The Mermaid diagram is an explicitly labeled static SVG illustration. No actual provider, filesystem, watcher, persistence, paste transport, or Mermaid engine runs. The final implementation must use the story contracts and real acceptance gates.

Checked with locally installed Chromium automation at 1440×900 and 1024×640. The check exercised layout switching, line-range and whole-file comments, multi-file collection, numbered payloads, simulated paste, and the setup review. No JavaScript errors or horizontal page overflow occurred. Actual WebKit and Cockpit rendering remain future implementation checks.

Initial-plan verification also exercised the final Context pane menu/placement/fallback, whole-file review payload without body text, archived simulated paste, setup progress/source retry, both teardown triggers, and dirty-preflight refusal. A missing hidden-state CSS rule in setup and minimum-width overflow in review were found and corrected before committing. The integrated workflow lab now provides the second keyboard/mouse discussion checkpoint.

## Consolidated workflow lab

Open [workflow.html](workflow.html) for the latest interaction proposal. Keep its sibling `workflow.css` and `workflow.js` beside it; no server or network is needed. [Checkpoint notes](../10-interaction-checkpoint.md) document the keymap and prototype limits. Earlier workbench/setup/review mocks remain focused reference studies.

Try Ctrl+P to open a file, Source, Shift+arrows to select lines, C to comment, Ctrl+Enter to collect, and Ctrl+Shift+Enter to paste. The explicit paste action fills a mock textarea without submitting; only your subsequent Enter submits in the mock. Keys & gestures includes the optional always-preview preference. Use the scenario selector for changed sources, two/no same-tab agents, rejected/unknown paste, and disconnection. Reset clears sample drafts and preferences.

Unlike the initial static mocks, this lab saves sample drafts and delivery receipts in localStorage when available. It still has no real transport or filesystem. Chromium checks passed at 1440×900 and 1024×640 for keyboard/mouse layout, exact payloads, old-side comments, unknown delivery, and fast setup. The production keymap remains subject to native/browser collision testing.

Second-checkpoint revision: the latest lab has no persistent comments bottom panel. `N comments` in the pane header or Ctrl+Shift+M opens the overview. Unsent selected-line comments appear inline in source/diffs; whole-file and rendered-document comments appear below the file. They disappear from those views after accepted paste. Pane/ellipsis styling and source setup remain flagged for later refinement.
