# Mock verification

Design artifacts only. Open [the mock](index.html) directly in a browser. No application code, Herdr sessions, repositories, worktrees, extension installations, or provider data were changed by this work. All interactive example state is local to the page and resets on reload.

## Browser checks

The standalone HTML was exercised in Chromium with actual pointer and keyboard input. Final captures were visually inspected.

### Setup and navigation

- Initial issue URL fills repository `cockpit`, branch `issue-4-terminal-width`, and the matching Space name.
- Entering the illustrative PR URL fills branch and default name `ui-polish`.
- Typing branch `fix/sidebar` updates the default Space name.
- Editing the name to `My review`, then changing the branch to `fix/toolbar`, preserves `My review`.
- Existing checkout changes the operation and checkout field. The final action stays disabled because this is not an executable repository operation.
- No repository-actions consent checkbox remains.
- Files and Review both expose collapsible file overviews and file pickers. Both overview toggles work.
- Files picker identifies the issue; Review picker identifies `CONTEXT.md`. Picking the Review file preserves the Review pane.
- No page errors occurred during this final setup/navigation check.

URL resolution uses two local fixtures, not network requests. See [findings](findings.md#setup) for fixture scope and the proposed production behavior.

### Browser annotations

- The seven-control toolbar is 224 CSS pixels wide on desktop.
- Pointer-driven freehand input produces a retained visible stroke.
- Dragging a region produces a visible rectangle and opens its inline editor.
- A region note and an element note were added without opening the sidebar; Ctrl+Enter also commits an inline note.
- The optional overview lists both notes with their respective targets. Selecting the region entry reopens its original text inline.
- Computed toolbar opacity is `0.45` when idle and `1` on hover.
- Keyboard navigation followed by toolbar focus activates `:focus-visible`; computed opacity is `1`.
- Notes, selected regions, and freehand strokes remain fully visible while toolbar chrome fades.

### Layout and comments

- Workbench, Review, Setup, and Browser each fit at widths 1440, 800, and 480 CSS pixels without document-level horizontal overflow or broken images. Heights were 900, 1000, and 900 respectively.
- Desktop evidence uses 1440×1000; portrait evidence uses 480×900.
- The portrait Review composer opens as a modal bottom sheet, 456 pixels wide at left offset 12 within the 480-pixel viewport. Text entry was exercised and visually inspected.
- Earlier interaction checks also exercised the desktop inline composer, newline versus Ctrl/Cmd+Enter, local save/edit/cancel/delete, command filtering/Escape, document details, source view, picker empty state, sidebar collapse, and explicit document expand/restore.

## Terminal preservation

`assets/terminal-original.png` is the 598×840 RGB crop `(236, 59, 834, 899)` from `../current/app-files-source.png`.

The stored asset was compared byte-for-byte against the original crop's decoded RGB pixels: **identical**. The mock displays it at its natural 598×840 CSS dimensions in a scrollable viewport, not as retyped text or a resized image. No original capture was modified.

## Final captures

| Surface | Evidence |
| --- | --- |
| Workbench / Files overview | [Desktop](evidence/workbench-desktop.webp) |
| Review / matching overview | [Desktop](evidence/review-desktop.webp) |
| Compact setup / issue defaults | [Desktop](evidence/setup-desktop.webp) · [480](evidence/setup-480.png) |
| Inline annotations / idle toolbar | [Desktop](evidence/browser-desktop.webp) |
| Same annotations / hovered toolbar | [Desktop](evidence/browser-toolbar-hover.webp) |
| Optional notes overview | [Desktop](evidence/browser-notes-sidebar.webp) |
| Review comment editor | [480](evidence/review-composer-480.webp) |

These replace intermediate mock captures, including the rejected wizard, extra annotation chrome, and consent checkbox.

## Static checks and limits

- `node --check planning/product-atlas-2026-09-12/polish/mock.js` passed.
- Local HTML asset/link targets and Markdown relative links were checked for existence.
- No application build or permanent test suite was needed for these standalone design artifacts.
- This is not proof of native rendering, live Herdr parity, provider lookup, repository mutation, durable note storage, extension installation, capture persistence, or agent delivery. Original screenshot ambiguities are recorded in the [coverage table](findings.md#screenshot-coverage).
