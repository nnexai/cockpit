# Issue #4 workspace trial

The requested end state was reached with workarounds. Cockpit created a real linked worktree and companion, grouped the task Space beneath `cockpit`, displayed the downloaded issue, saved two Review annotations, and closed the task Space. GitHub downloading and opening prepared Context did not work as a direct setup flow.

This was the rebuilt production application with real Herdr data. No mock or fixture served the application. No work on issue #4 was performed. The only worktree edits were two disposable documentation additions and one untracked text file. No agent was started and no comment was sent to GitHub or an agent.

## Runtime and retained resources

- Cockpit source/build: `e51f97eba65685c9b2c515e2727d1791b02056fb`. `cargo build -p cockpit-host --bin cockpit` and `bun run build` passed. The frontend build reported its existing large-bundle warning.
- Herdr: 0.9.0, protocol 22, isolated session `cw-issue4`. Parent Space `w1`, task Space `w2`. User authorization allowed the default session or a clone; this trial used a local clone with an independent Git directory.
- Real gateway: `http://127.0.0.1:4187`, now stopped. Browser: disposable Chrome through Playwright CLI.
- Parent clone: `/tmp/cwf12/repos/cockpit`. Its origin was set to `https://github.com/nnexai/cockpit.git`; no remote push occurred.
- Worktree: `/tmp/cwf12/worktrees/cockpit-issue-4`, branch `study/issue-4-workflow`.
- Companion: `/tmp/cwf12/companions/2ed2cdcc-aa76-4055-9173-624805b3f9fb`.
- Issue body and all three comments are in `issues/github-cockpit-4.md`, with raw JSON and an import receipt beside it. This was an explicit `gh` download workaround, not a Cockpit-managed import.
- Saved Cockpit annotations remain in `/tmp/cwf12/cockpit-state/comments/1e5d2100-1f1b-4cbe-91e2-bfc518fb518d.json`.

After the UI Close action, Herdr contained only `w1`; the linked worktree and companion remained. The isolated Herdr session and gateway were then stopped. The retained files support inspection. Closing a Space was not treated as permission to delete its worktree, context, or review drafts.

The [filesystem proof](filesystem-proof.json), [setup operation](setup-operation.json), [companion manifest](companion-manifest.json), [saved annotations](saved-annotations.json), and [Herdr state after closing](herdr-after-close.json) record those facts. [collect.py](collect.py) can re-read the retained files from this exact run. It does not replay setup.

## The observed flow

| Step | Result and screenshot |
| --- | --- |
| Start from the parent Cockpit Space | [Parent](screenshots/01-parent-space.png). The parent clone and Space were bootstrap prerequisites created through Git and Herdr CLI. Subsequent task setup used Cockpit. |
| Select repository and enter GitHub issue #4 | [Repository and issue](screenshots/02-repository-issue.png). Repository, task name, issue URL, branch, destination, and label were separate inputs. |
| Review worktree and context | [Worktree](screenshots/03-worktree.png), [Context](screenshots/04-context-consent.png). The form has four steps and repeated explanations. |
| Validate the GitHub URL | FAIL. [Rejection](screenshots/05-github-blocked.png): `artifact host is not a configured Gitea provider`. Validation arrived on the Context step after the earlier inputs. The current provider factory instantiates Tea providers only. Adding a GitHub URL to configuration alone would not implement a GitHub importer. |
| Continue without the URL | Workaround. Removed the unsupported URL while retaining the other fields. [Exact effects](screenshots/06-plan-without-issue.png) correctly says there is no artifact. |
| Create the worktree and companion | PASS. [Completed](screenshots/07-setup-completed.png), [child Space](screenshots/08-child-space.png). Git lists a linked worktree. Herdr's repository/worktree grouping places it beneath `cockpit`; no invented parent registry was used. The companion initially contained only `manifest.json`. |
| Open Files, Context, or Review | The fresh isolated session had no plugins enabled. [Disabled commands](screenshots/09-disabled-tools.png). Enabled the installed file-viewer 1.15.0 and Reviewr 0.29.0 for this session and reloaded its configuration. Files and Review recovered. This prerequisite was specific to the fresh session; it is not proof that the user's default session lacks plugins. |
| Open prepared Context | FAIL after setup. [Context still disabled](screenshots/10-context-disabled-after-setup.png). The new context-aware terminal starts in the worktree; Open Context requires its foreground directory to be the companion. The Context buttons have no explanatory tooltip, while other command tooltips expose the reason. |
| Navigate the terminal to the companion | Workaround. Immediate typing after clicking the terminal lost the command prefix; [capture](screenshots/11-context-directory-workaround.png) and Herdr's readback showed only `PIT_CONTEXT_PATH"`. Clearing it and retrying after focus settled worked. [Context enabled](screenshots/12-context-enabled-after-cd.png). Root cause remains unconfirmed. An earlier attempt to click xterm's hidden helper textarea failed in automation and is excluded from product findings. |
| Prepare and read issue context | Manual `gh issue view` downloaded [issue #4](https://github.com/nnexai/cockpit/issues/4), its body, and all three comments into the created companion. Open Context then displayed them: [split](screenshots/13-issue-context-split.png), [zoomed](screenshots/14-issue-context-zoomed.png). The split capture shows the reading column compressed by its tree and controls. |
| Make disposable edits and open Review | PASS. [Review opening](screenshots/15-review-open.png) lists both tracked edits and the untracked file. This initial capture shows a blank diff region before its text arrives. [Actual edits](trial-changes.diff). |
| Annotate | PASS. Saved a new-side comment on `CONTEXT.md:383` and a whole-file comment on the other modified document. [Editor](screenshots/16-review-comment-editor.png), [saved line comment](screenshots/17-review-comment-saved.png), [both annotations](screenshots/18-review-two-annotations.png). |
| Preview and refresh | Preview PASS: [payload](screenshots/19-review-preview.png). Paste correctly stayed disabled with no eligible agent. Refresh FAIL for context preservation: [selected file reset](screenshots/20-refresh-changed-selection.png). README was selected before refresh; `CONTEXT.md` was selected afterward. The comment count briefly became zero, then restored both saved annotations. |
| Switch tabs and renderer | Both annotations survived. Selecting README, leaving for Context, and returning again reset Review to `CONTEXT.md`. The real [herdr-reviewr terminal](screenshots/23-herdr-reviewr-loaded.png) saw the same three changes. Its `Send (0)` is its own annotation store; returning to Cockpit's graphical Review restored the two Cockpit annotations. This is not a cold-start speed comparison. |
| Close the task Space | PASS. [Resource menu](screenshots/24-close-space-menu.png) and a browser-native confirmation closed `w2` and its panes. [Parent afterward](screenshots/25-space-closed.png). Git and disk evidence show that the worktree, companion, and annotations remain. One late presentation request returned 503 after close; no stuck Space or visible failure remained. |

## Measurements and limits

These timings measure individual browser actions in one local run, including the response and UI update. They exclude the agent's investigation, setup preparation, approvals, screenshots, and time between actions. They are not p95 values or a production benchmark.

| Action | Observed |
| --- | --- |
| GitHub URL validation to error, including capture | 365 ms |
| Repository-only review plan, including capture | 264 ms |
| Start to completed setup heading | 2,302 ms |
| Open Review to first changed-file list | 5,828 ms; first useful diff was not separately timed |
| Select another changed file to its diff text | 798 ms |
| Save one line annotation to saved count | 1,832 ms |

The core creation operation is quick. The full issue-to-ready workflow is not yet seamless: GitHub support, prepared-context launch, prerequisite recovery, and selection retention interrupt it.

Real Context and Review were captured at 1440×900, 800×1000, 600×900, and 480×900. At the smaller widths, existing sidebar and viewer chrome consume reading space. [Context 480](screenshots/14-context-480.png), [Review 480](screenshots/21-review-480.png). Body width alone stayed within the viewport; that does not prove readable content. [Screenshot inventory](screenshots.json) records all 32 captures and their hashes.

Production source text in Context and Review both measured 13 px / 19 px. Their fallback stacks differ, and file-list labels differ. The mockup typography concern calls for shared viewer styles. This trial did not find unequal production source sizes.

No native Tauri run, system clipboard test, drag-and-drop test, large-repository benchmark, or provider outage/recovery experiment was performed here. Saved comments were inspected after Space close on disk; recovery into a reopened Space remains an acceptance case. The trial does not establish that existing size limits are solved.

## Changes to the proposals

1. **Packet 12:** make issue-to-ready setup the acceptance workflow. Resolve supported issue identity early, prepare its body and comments in the companion, and open that context without a terminal `cd`. Preserve partial results and distinguish a created worktree from ready context.
2. **Packet 10:** consolidate setup into a compact form, keep the issue source in the primary task flow, preview the actual parent/worktree/companion, and check prerequisites before declaring readiness. Preserve the existing consent and reviewed effects.
3. **Packet 02:** reproduce immediate post-selection input loss. Preserve the intended first command through confirmed focus without duplicate or wrong-pane delivery.
4. **Packets 06 and 07:** use one viewer design and collapse navigation by pane width. Preserve file, hunk, side, scroll, and draft state across refresh and tab/renderer changes. Keep a loading count distinct from an empty batch.
5. **Packet 05:** explain unavailable actions at their location and expose a direct recovery path. Avoid inconsistent tooltips and requiring operators to infer a hidden cwd precondition.

Compare the product's stated loop in `CONTEXT.md` with this evidence: discover repository, create/open worktree, attach companion, gather static issue context, inspect changes, and manage lifecycle. The worktree/companion/annotation primitives exist. The missing part is the connection between those primitives that leaves a task ready to work on.
