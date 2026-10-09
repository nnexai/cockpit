# WS-09 `BrowserPane` split

Wave 1 · Size L · Depends on: – · Blocks: WS-24

## Goal
The 1,859-line `BrowserPane` function becomes a thin component composed of named hooks and subcomponents, each owning one concern. The annotation and feedback features stay fully functional (feature decisions are out of scope).

## Owns
`src/app/browser/*` and its tests.

## Evidence
`BrowserPane.tsx:~32` has ~50 hooks/refs, which tangle several concerns:
- the input queue and gestures (`inputJobsRef`, `gestureRef`, `controlPromiseRef`, `remotePointRef`);
- draft persistence (`queueDraftMutation`, `applyDraft`, `associationOwner`);
- the note editor;
- feedback capture;
- frame presentation.

## Change
1. Extract hooks:
   - `useBrowserInputQueue`: ordered input jobs, gestures, remote pointer;
   - `useBrowserDrafts`: draft load/mutate/recovery, association ownership;
   - `useBrowserNoteEditor`;
   - `useBrowserFeedbackCapture`: capture, pending capture, send/ack.
2. Extract subcomponents for the toolbar/chrome and the frame surface.
3. Keep the existing pure model logic in `browserPaneModel.ts` and `framePresenter.ts`, and add to it where logic is pure.

## Keep
- The "retired" document/draft/frame guards and cancelled async attachment disposal (current behaviour, see the inventory).
- Input ordering.
- DOM, class names, ARIA and shortcuts.
- No CSS edits.

## Acceptance
- No function in `src/app/browser/` is over 300 lines.
- `BrowserPane` is under 250 lines.
- The existing browser tests pass without behavioural edits.

## Verify
- `bun run test -- src/app/browser`.
- Browser smoke (`skill://cockpit-browser-smoke-on-disposable-fixture`): open a Browser leaf, navigate, click/type/scroll, draw an annotation, add a note, send feedback, close the tab (discard).
- One native run of the same flow (the native browser-view input path differs).
