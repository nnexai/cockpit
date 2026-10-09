# WS-18 Supervisor UI split

Wave 2 · Size L · Depends on: WS-05 · Blocks: WS-24

## Goal
The Supervisor views are composed of focused hooks and components. Focus-restoration plumbing gets its own home.

## Owns
`src/app/supervisor/*` and its tests, **except** `retirementView.ts` and `retirementView.test.ts` (both owned by WS-13).

## Evidence
- `SupervisorView.tsx` is 678 lines. ~25 `useState`/`useRef` declarations sit at `~41-83`, including `detailInvoker`, `panelInvoker`, `counterInvoker`, `returnFocusIntent`, `startFocus`, `focusedTask` and `focusedGraph`.
- `StepsSection` is 606 lines (`SupervisorSteps.tsx:~45`).
- `TaskSourceDialog` is ~203 lines (`SupervisorDialogs.tsx:~183`).

## Change
1. Extract `useDetailFocus` (all invoker/return-focus state), `useStartAgentFlow` and `useArchiveCounts`.
2. Split `StepsSection` by mode (view, edit, add, reorder/subtree) and keep shared row components.
3. Split `TaskSourceDialog` into its form and its preview.

## Keep
- Every surface in `docs/supervisor-surfaces.md`.
- Keyboard and focus behaviour, including delayed saved-row move focus without navigation theft.
- DOM, class names, ARIA.
- No CSS edits.

## Acceptance
- No component over 250 lines in `src/app/supervisor`.
- The existing supervisor tests pass without behavioural edits.

## Verify
- `bun run test -- src/app/supervisor`.
- Browser smoke on a disposable supervisor fixture:
  - walk the Tasks, Graph, Dependencies and Steps views;
  - open and close each dialog with the keyboard and confirm focus returns;
  - start-agent flow.
