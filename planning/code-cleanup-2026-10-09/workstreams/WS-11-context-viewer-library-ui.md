# WS-11 `ContextViewer` split and provider-token entry points

Wave 1 · Size L · Depends on: – · Blocks: WS-24

## Goal
1. `ContextViewer` (a 1,192-line component that serves both the Files viewer and the Library) is split into a shared viewer shell, hooks and a Library-specific module.
2. Provider tokens have one label and fewer, purposeful entry points.

## Owns
- `src/app/context/*`, except `CommentDrafts.test.tsx` (WS-06)
- `src/app/library/*`
- their tests

## Evidence
- `ContextViewer.tsx:~672` has ~30 hooks.
- The token dialog has 9 entry points with 4 labels:
  - the palette command (stays as-is; it lives in `App.tsx`);
  - the Library toolbar menu (`ContextViewer.tsx:~1738`);
  - the item overflow menu (`LibraryItemHeader.tsx:~144`);
  - inline header buttons (`~236`, `~255`);
  - the Add-dialog sign-in failure (`src/app/library/AddContextDialog.tsx:~540`) and "Store a token…" (`~490`);
  - the tree instance menu (`LibraryTree.tsx:~410`);
  - the tree item "Store a token to download attachments…" (`~78`).

## Change
1. **ContextViewer:**
   - extract `useDirectoryTree`, `useDocumentLoader` and `useFilePicker`, plus a `CommentBar` component;
   - move the Library-only branches into `src/app/library/` (e.g. a `LibraryDocumentView`) so `ContextViewer` stops branching on viewer vs Library;
   - split `src/app/library/AddContextDialog.tsx` (433 lines) and `src/app/library/LibraryTree.tsx` (298) along their form steps and row/keyboard handling.
2. **Provider tokens:** use one label, "Provider token…".
   - Keep: the palette, the Library toolbar ⋯ menu, and the contextual prompts where a read or download is blocked (the Add-dialog sign-in failure, the attachments prompt, the Jira state line).
   - Remove: the duplicate generic entries in the item overflow menu and the tree instance menu.
   - Update the tests that assert the old labels (`LibraryTree.test.tsx`, `LibraryAttachments.test.tsx`).

## Keep
- Library selection, preview freshness and pagination.
- Comment semantics.
- Focus return to the opener or its nearest surviving ancestor.
- `TreeSplitter`'s no-rerender drag.
- No CSS edits.

## Acceptance
- No function in `src/app/context` or `src/app/library` is over 300 lines.
- `ContextViewer` is under 300 lines.
- Exactly one token label in the UI.

## Verify
- `bun run test -- src/app/context src/app/library`.
- Browser smoke on a disposable fixture:
  - Files viewer: browse, preview, comment;
  - Library: tree with nested Jira subtasks, preview, Add dialog, open the token dialog from every remaining entry point and confirm focus returns.

## Doc notes for WS-25
The provider-token paragraph in `CODE_GUIDE.md` (~240) lists the entry points.
