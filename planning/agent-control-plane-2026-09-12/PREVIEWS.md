# Discussion previews

These are screenshots of proposals. The terminal interiors use archived atlas pixels. They are not captures of an improved production application.

Use the [interactive review board](mocks/review.html) to compare sizes and try the local controls.

| Surface | Desktop | Portrait or narrow pane |
| --- | --- | --- |
| Shell | [1440×900](previews/shell-desktop.png) | [800×1000](previews/shell-800.png), [600×900](previews/shell-600.png), [480×900](previews/shell-480.png) |
| Sidebar comparison | [Persistent rail at 800](previews/rail-800-alternative.png) | [Open drawer at 480](previews/sidebar-480-open.png) |
| Commands and resource actions | [Commands](previews/commands.png), [Space menu](previews/space-menu.png) | [Stale view at 800](previews/stale-800.png) |
| Files and Context | [Desktop reader](previews/context-1440.png) | [480 px](previews/context-480.png), [narrow pane on desktop](previews/context-split-narrow.png) |
| Review | [Desktop diff](previews/review-1440.png) | [480 px](previews/review-480.png) |
| Space setup | [Desktop form](previews/setup-1440.png) | [480 px](previews/setup-480.png) |
| Browser feedback | [Desktop feedback](previews/feedback-1440.png) | [480 px](previews/feedback-480.png) |
| Browser capture | [Browse state](previews/capture-1440.png), [element hover preview](previews/capture-element-preview.png) | [480 px](previews/capture-480.png) |

## Useful comparison points

- A or B at 800 px: is constant sidebar access worth the lost terminal width?
- Sidebar: are the current names, locations, and exact status text readable with these insets and row heights?
- Commands: does grouping direction choices beside the action read better than the current repeated long labels?
- Files: does the reader feel lighter when sources and comments are closed until needed?
- Review: are the scope, changed files, diff, and comment target in the right places?
- Capture: does toolbar visibility make the mode clear, and is the element hover outline enough before a click?

Clipboard, focus correctness, reorder failure, scrollbar behavior, and file/review performance require real reproduction. They are task packets rather than invented visual solutions.
