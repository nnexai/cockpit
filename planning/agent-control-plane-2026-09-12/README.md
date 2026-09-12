# Cockpit UI improvements to hand off

Planning and discussion mockups, 2026-09-12. No application changes are included.

The aim is to remove friction from the current product. Terminal copy/paste, focus, drag and drop, readable navigation, file limits, and a usable command menu take priority over cosmetic changes. Small fixes to spacing, contrast, alignment, and target size belong in the same backlog.

Herdr's terminal content, status semantics, hierarchy, ordering, tabs, and pane layout are fixed constraints. Cockpit's terminal input integration can be repaired. Cockpit-owned Files, Context, Review, setup, and browser interfaces can be redesigned fully within their existing resource bindings.

The central acceptance flow is now [issue #4 to a prepared task Space, Review annotations, and close](workflow/issue-4/README.md). The real application trial created the worktree and companion and saved annotations, but required workarounds for GitHub importing and opening Context. [Packet 12](tasks/12-task-context-readiness.md) connects those capabilities into the intended workflow. No production repairs were implemented during the trial.

## Discuss the direction

Open the [review board](mocks/review.html) to switch surfaces and exact viewport sizes. Start with 800×1000 and compare the original atlas capture with directions A and B.

- [Static preview index](PREVIEWS.md): screenshots if you prefer to review without running a server.
- [Shell comparison](mocks/index.html): original atlas captures, a refined rail, and a portrait drawer. Includes Commands, resource menus, Feedback, and illustrative recovery states.
- [Cockpit-owned interfaces](mocks/surfaces.html): Files and Context, Review, Space setup, browser feedback, and browser capture.
- [Unified file/diff comparison](mocks/viewers.html): shared typography, spacing, gutters, and controls. [Viewer design contract](VIEWERS.md) applies to both implementation packets.
- [Design decisions and states](DESIGN.md): the proposed interaction contract, including automatic control on normal selection and fewer toasts.
- [Evidence and limitations](EVIDENCE.md): user reports, source findings, atlas provenance, and the limits of this planning pass.

The shell mock uses original terminal pixels. It does not simulate terminal reflow, live selection, clipboard, drag and drop, or Herdr mutations. The other mock uses illustrative content. Both are design artifacts, not proof of repaired behavior.

To serve the mockups from the repository root:

```bash
python3 -m http.server 4186 --bind 127.0.0.1
```

Open `http://127.0.0.1:4186/planning/agent-control-plane-2026-09-12/mocks/review.html`. The mockups need no build. The shell references the atlas images in this checkout and uses local Plex fonts when available.

## Send one packet to an implementation agent

Each packet contains the problem, ownership boundary, proposed work, reproduction path, and acceptance conditions. Sending a packet starts that task; this document does not authorize running every task automatically.

| ID | Packet | Reason | Evidence level |
| --- | --- | --- | --- |
| 1 | [01 · Repair terminal copy and paste](tasks/01-terminal-clipboard.md) | Basic input is broken for the user | User report; integration path inspected |
| 2 | [02 · Make focus automatic and quiet](tasks/02-focus-and-feedback.md) | Selection should acquire control without a second action | User report; existing ownership flow inspected |
| 3 | [03 · Make the sidebar readable, collapsible, and resizable](tasks/03-sidebar.md) | Portrait loses useful space and hides information | User report, screenshots, and CSS |
| 4 | [04 · Repair drag and drop and resource menus](tasks/04-resource-interactions.md) | Navigation controls do not behave at their natural targets | User report; handlers exist, failure needs reproduction |
| 5 | [05 · Replace the broken command-menu presentation](tasks/05-command-menu.md) | Actions and shortcut reference compete in a tall overlay | User report, screenshot, and source |
| 6 | [06 · Make Files and Context useful at repository scale](tasks/06-files-context.md) | Restrictive limits and overloaded reading UI | User report; independent limits verified |
| 7 | [07 · Make Review complete and responsive](tasks/07-review.md) | Limits prevent useful reviews; compare directly with herdr-reviewr | User report; eager loading and bounds verified |
| 8 | [08 · Simplify browser annotation](tasks/08-browser-annotation.md) | Mode switching, selection, shortcuts, and element targeting | Concrete user requirements; current extension inspected |
| 9 | [09 · Clarify browser feedback and delivery](tasks/09-browser-feedback.md) | Read, target, send, and recover without ambiguous acknowledgement | Existing contract; atlas evidence is mixed |
| 10 | [10 · Simplify Space setup and recovery](tasks/10-space-setup.md) | Show required inputs and actual effects without excess chrome | Design proposal grounded in existing setup flow |
| 11 | [11 · Replace the terminal-edge scrollbar line](tasks/11-terminal-scroll-affordance.md) | The persistent line conveys little useful information | User report; xterm scrollbar configuration found |
| 12 | [12 · Create a task Space with context ready](tasks/12-task-context-readiness.md) | Issue-to-ready setup is Cockpit's central workflow | Real GitHub/worktree/companion/Review/close trial |

Packets 01–07 and 11 are repairs or direct usability work. Packets 8–10 include larger Cockpit-owned interaction changes. Start with 01–02, then evaluate 11 with the same terminal run. The remaining numbers suggest a sequence, not implementation effort.

Use packet 12's complete workflow to judge readiness. Coordinate it with setup, Files/Context, and Review; completing their individual screens is insufficient if the operator still has to download context or discover hidden terminal-directory requirements manually.

## Coordinate delivery

Keep one integration owner. Packets 01, 02, and 11 share terminal input, focus, and scroll code. Packets 03, 04, 05, and 09 share `App.tsx` or `styles.css`; serialize their edits or assign an explicit extraction before parallel implementation. Packets 06 and 07 share comment and preview contracts; decide those boundaries before separate agents edit them. Packet 08 can run independently in `browser-extension/`. Packet 10 can run independently in `src/app/projects/` if it leaves shared contracts to the integration owner.

Before each packet, reproduce its user-visible failure in the real Cockpit app with a uniquely named disposable Herdr session. Never use `default`, inherited user sockets, or unrelated browser profiles. Record the current build, installed Herdr version, created resources, and cleanup owner. Compare terminal and hierarchy behavior with the same scenario in Herdr. Compare Review with installed herdr-reviewr.

Land small complete increments. Include the fix, removal of replaced UI, appropriate tests, real interaction evidence, and one scoped commit. Do not ship two competing implementations of a control. Do not stage unrelated atlas, tooling, or user changes.

Required viewport checks are 1440×900, 800×1000, 600×900, and 480×900. Cockpit-owned viewers must also be checked inside narrow split panes on a wide desktop. Screen width alone does not describe their available space.

Record results as PASS, FAIL, or INCONCLUSIVE. A screenshot proves appearance at that instant. Clipboard, focus, drag and drop, input routing, scrolling, speed, recovery, and persisted drafts require interaction evidence. Unavailable native or extension coverage remains INCONCLUSIVE.

## Scope held out

No new agent registry, invented task field, status normalization, reordered attention queue, terminal transcript renderer, agent dashboard, or synthetic workbench mode. `working`, `blocked`, `done`, `idle`, and `unknown` keep their actual meanings. Additional status values are displayed only if supplied by the supported contract. Disconnection is not agent failure; `done` is not proof that a task was accepted.

The design does not add a permanent instruction composer. Repair clipboard and terminal focus first. Existing comment and browser delivery flows retain explicit target binding and their existing submission semantics.

## Check the design artifacts

The [mock check result](checks/result.json) records 148 browser checks across the four requested sizes, including matching computed viewer typography and insets. These cover mock geometry and local controls only. They do not verify any production fix. The [check script](checks/browser.js) also regenerates the preview screenshots. The separate [runtime trial](workflow/issue-4/README.md) records actual application behavior and 32 screenshots.

With the local server running, open a disposable Playwright CLI browser and run:

```bash
playwright-cli -s=cockpit-ui-study open http://127.0.0.1:4186/planning/agent-control-plane-2026-09-12/mocks/index.html --browser=chrome
playwright-cli -s=cockpit-ui-study run-code --filename=planning/agent-control-plane-2026-09-12/checks/browser.js
playwright-cli -s=cockpit-ui-study close
```

The [viewer comparison check](checks/viewers.js) separately regenerates the side-by-side image and checks that an empty Review has no delivery footer. Run it with the same `run-code --filename` command. The mock styles fall back to system fonts if local Plex files are unavailable.
