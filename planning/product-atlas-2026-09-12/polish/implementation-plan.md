# Cockpit UI polish implementation plan

Status: approved user direction, implemented as a standalone mock; product implementation remains to be planned and executed. This document is the implementation handoff, not a record of rejected alternatives.

## Start here

- [Interactive mock](index.html): [Workbench / Files](index.html#workbench), [Review](index.html#review), [Setup](index.html#setup), [browser annotations](index.html#browser).
- [Screenshot findings](findings.md): observations and coverage limitations across the 24 original captures.
- [Verification and captures](verification.md): what the mock actually demonstrates, including subsequent refinements.
- [Original atlas](../current/index.html): the pre-polish surface, not the desired end state.

**Planning-agent assignment:** turn the contracts below into file-owned, end-to-end increments against the current implementation. Preserve working behavior, implement the approved changes beyond the four mock scenes, verify the real surfaces, and commit each completed increment. Do not rebuild implemented features from older plans or copy the mock into the application.

### Authority and conflicts

Read [CONTEXT.md](../../../CONTEXT.md), [DECISIONS.md](../../../DECISIONS.md), [design direction](../../../research/ui-design-direction.md), [implementation constraints](../../../research/ui-implementation-constraints.md), and the [existing phase plan](../../next-level/README.md) before defining implementation boundaries.

This handoff supplies the latest UI decisions from the screenshot review. It does not replace the Herdr authority, filesystem, transport, or draft-identity contracts. Resolve historical conflicts as follows:

| Earlier guidance | Instruction for this work |
| --- | --- |
| Synthetic Context tab, extra workbench modes, or a separate dock | Do not implement. Files and Review replace renderers of real Herdr panes. The mock's four scene buttons are demonstration navigation only. |
| Old protocol migration and terminal font/ANSI recommendations | Do not replay them. Later decisions record Herdr 0.9.0 / protocol 22 compatibility. Inspect the currently installed supported runtime; preserve its working terminal path. No runtime upgrade/downgrade is part of UI polish. |
| Comments in a persistent bottom panel or duplicate top toolbar | Keep comments inline, with bottom actions/count and an optional overview. Files and Review both retain the complete workflow. |
| Required repository for every setup operation | New worktree requires a repository. Existing directory requires only a path, accepts non-Git folders, and never initializes Git. This needs a real contract/lifecycle change, not just hidden fields. |
| Repository-action consent checkbox | Remove it. Configured repository actions are always allowed for this personal tool. Do not conflate this with Git ownership overrides or destructive-resource confirmation. |
| Wizard, explanatory steps, annotation slogans, new “Add idea” workflow | Do not implement. Use a compact setup form and the existing small annotation toolbar with direct inline notes. |

At implementation time, update the affected architecture/lifecycle decision entries to record the approved path-only and repository-action policy changes. Keep historical evidence identified as history. Do not change unrelated architecture to make the mock easier to reproduce.

## Governing principles

1. **Keep work visible.** The application fills its window. Content, resource identity, and the next useful action take precedence over framing. No hero, study masthead, scene switcher, page gutter, marketing subtitle, decorative cards, or mock disclaimer belongs in the product.
2. **One resource hierarchy.** Session → Spaces → tabs → panes, with a separate Agents queue. Use existing names and IDs; do not invent new workspace terminology or a Cockpit-owned resource registry.
3. **Improve presentation, preserve authority.** Herdr owns hierarchy, ordering semantics, layout, semantic focus, terminal ownership, and process lifetime. Pointer gestures express intent; authoritative responses/events confirm changes. Local disclosure and scroll state are presentation only.
4. **Make frequent work direct.** Setup is used 5–20 times daily. Default derived values, preserve explicit edits, and put occasional details behind disclosure. Do not make the user acknowledge explanations repeatedly.
5. **Keep actions near their subject.** Pane/Commands stay at the tab bar's right edge. File/review comments belong next to content and in the pane's bottom controls. Error recovery belongs to the failed resource, not a global banner or toast alone.
6. **Use one visual system.** Shared semantic tokens and existing controls, not a new component library or a second set of per-pane sizes. Files and Review share navigation, source typography, comment controls, and responsive rules.
7. **Do not polish terminal contents.** No changes to font, cell metrics, line height, ANSI colors, cursor, glyphs, wrapping, output, scrollback, mouse routing, or renderer lifecycle. Never fade, blur, scale, or reconstruct terminal text. Chrome geometry can change only with real resize/input verification.
8. **Preserve authorship and uncertainty.** A comment save is not paste, submission, or provider publication. Keep drafts when source changes or an outcome is unknown. Never claim persistence or delivery from connection status alone.
9. **Reduce chrome, not capability.** Removing a toolbar layer must relocate its useful actions, not discard search, source selection, comments, failure recovery, or keyboard access.
10. **No implicit expansion of scope.** This is a personal local IDE. No onboarding system, telemetry project, new settings product, remote-access framework, browser-engine replacement, or new agent automation is required.

## Design tokens and usage

### Token ownership

Use the existing [application tokens](../../../src/app/styles.css), [shared viewer tokens](../../../src/app/viewer.css), and [file-navigation styles](../../../src/app/input/fileNavigation.css). The mock's short variable names are prototype-local; do not introduce them alongside production tokens. Consolidate duplicate component literals into the existing semantic names while touching a surface.

The following palette maps the mock's neutral hierarchy onto production names. It is a target for **non-terminal chrome**. Apply it coherently, not as a collection of screenshot-specific overrides.

| Production token | Target | Role |
| --- | --- | --- |
| `--app-bg` | `#0b0e12` | Window/root background |
| `--sidebar-bg` | `#11161e` | Spaces, file trees, optional overview |
| `--surface` | `#161d27` | Pane/control chrome |
| `--surface-raised` | `#1b2431` | Menus, dialogs, inline editors |
| `--surface-hover` | `#222e3d` | Pointer hover only |
| `--surface-selected` | `#23354c` | Selected navigation item |
| `--border` | `#2b3543` | Ordinary one-pixel separator |
| `--border-strong` | retain `#3b4758` | Interactive boundaries/dividers |
| `--text-primary` | `#e5eaf1` | Content and active labels |
| `--text-secondary` | `#a6b3c3` | Supporting identity/status |
| `--text-muted` | `#8996a8` | Nonessential metadata; not the sole label of an action |
| `--accent` | `#8bb9ff` | Selected tab, link, active marker |
| `--focus-strong` | retain `#8db8ff` | Keyboard focus indication |
| `--blocked`, `--working`, `--done`, `--idle` | retain existing `#ff6b78`, `#e7b14a`, `#69a7ff`, `#57c78b` | Existing agent-state semantics; glyph/word as well as color |
| `--terminal-bg`, `--terminal-fg`, ANSI palette | unchanged | Outside the visual retuning scope |

Check actual contrast after integration. Required labels must remain readable in hover, selected, disabled, stale, and narrow states. Do not lower whole-pane opacity to express status. State color marks status; blue selection does not mean Working, ownership, or delivery.

### Typography

Keep the bundled IBM Plex Sans UI and IBM Plex Mono source fonts already used by the application. The mock's system-font fallback is not a font migration requirement. Terminal typography is separate and unchanged.

| Role | Existing token / target |
| --- | --- |
| Compact secondary metadata | `--font-size-2xs` = 11px at the normal 16px root; do not use it for primary actions/body text |
| File/review controls | `--viewer-control-size` 13px / `--viewer-control-line-height` 20px |
| Source/diff text | `--viewer-source-size` 13px / `--viewer-source-line-height` 19px, `--viewer-source-font`; never shrink to fit |
| File/review metadata | `--viewer-meta-size` 12px / `--viewer-meta-line-height` 16px |
| UI labels | existing `--font-size-xs` 13px; 400 normal, 500–600 selected/heading |
| Document body | existing `--font-size-sm` 14px, roughly 22px line height |
| Document title | 20–22px, restrained weight; no large hero title in a narrow pane |
| Paths, IDs, line numbers | existing monospace role, secondary to the human-readable document name |

Retain the existing global font-size scale rather than redefining it independently in each component. Use body/heading roles to resolve density; never lower source or terminal metrics because a toolbar is too wide.

### Spacing, geometry, and responsiveness

- Use 4px spacing increments: 4 / 8 / 12 / 16 / 24. Existing viewer inset is 12px, reducing to 8px in narrow containers. Avoid independent Files and Review inset rules.
- One-pixel dividers; 4px control radius. Primary application regions remain square. Shadows are for floating menus/dialogs, not every pane or content block.
- Compact icon controls: at least 28×28px. Default controls: about 32px. File/review toolbar: minimum 36px. Preserve the existing `--tab-strip-height` 31px initially; the mock is not a reason to enlarge every row.
- Pane focus and keyboard focus are separate stable outlines. Reserve their geometry so hover/focus/pending does not change layout or shift content.
- Retain the current resizable sidebar and tree geometry. Do not overwrite Herdr branch/order presentation with the historical 272px-sidebar study or the mock's illustrative rows.
- Use pane container width, not only window width. Consolidate around the existing shared viewer 520px breakpoint; the mock's 500px tree threshold is illustrative. Both panes retain a file-overview toggle and picker at every width. A narrow overview overlays its own pane; it does not create a new workbench column.
- Keep required action, selected identity, and recovery visible. Hide low-priority metadata before wrapping an entire toolbar into several rows. Source can scroll horizontally; the application page must not.
- Desktop comments stay inline when usable. Below roughly 600px viewport width, or when the containing pane cannot provide a usable editor, use a bounded sheet within the existing app dialog pattern. Preserve draft, selection, anchor, and focus across the transition. Never silently zoom/rearrange the Herdr pane layout.
- The mock setup form is 600px maximum width and fits 480px portrait without a step transition. Actual operation details/progress may scroll; the final action remains reachable without hiding validation.

### Browser annotation tokens

The extension overlay is intentionally light against arbitrary web pages; do not apply the application's dark surface palette to page content.

| Role | Target |
| --- | --- |
| Toolbar surface/text | white / `#29394f`; use the existing extension's scoped styles |
| Toolbar geometry | existing seven tools, approximately 224×38px desktop in the mock; 28px icon controls |
| Idle toolbar opacity | `0.45` |
| Hover / keyboard-visible focus opacity | `1` |
| Freehand | 3 CSS px, rounded caps/joins, non-scaling stroke |
| Region outline | retain 2 CSS px |
| Post-draw simplification | existing `simplifyFreehand`, tolerance 1.5 CSS px; retain endpoints and significant corners |

Fade only the toolbar, never notes, drawings, the page, or terminal panes. Keyboard-visible focus must reveal it; clicking a tool must not leave it permanently opaque after pointer departure. Keep existing annotation colors rather than replacing every mark with the mock's single blue. Reuse [the extension implementation](../../../browser-extension/content.js); it already simplifies freehand in `finishFreehand`. Do not simplify every move or introduce a competing smoothing library.

## Approved behavior contracts

### Files and Review

- Keep the collapsible overview and Go to file picker in both. Reuse the current [FilePicker](../../../src/app/input/FilePicker.tsx) and navigation ordering, rather than making a second picker for imported context.
- Show readable file/resource identity; disclose full original path, provenance, freshness, and diagnostic IDs in Details. Abbreviation is presentation, never the value used for operations or copying.
- Keep Preview/Source, review scope/side, file/hunk/line navigation, search, refresh, and permitted external-open behavior. Compress metadata before hiding essential controls.
- **Preserve full inline comment support in both panes.** Selected-line/range comments appear by their source; whole-file/rendered-document comments can appear below their subject. Bottom actions and count open the optional overview. No action in Files redirects to Review.
- Keep exact original lines, source revision, root/file identity, and Review old/new side. Rendered Markdown selection maps to original source, including frontmatter; do not use visible row numbers as fabricated source coordinates.
- Enter inserts a newline. Ctrl/Cmd+Enter saves. C and Shift+C retain the existing line/whole-file semantics when the viewer, not a text editor or terminal, owns the shortcut.
- Reuse [CommentDrafts](../../../src/app/context/CommentDrafts.tsx), [SourceLines / Markdown rendering](../../../src/app/context/ContextViewer.tsx), and the existing backend draft identity/revision model. One editor/batch model, not a parallel Files store.
- Drafts remain durable and isolated by their actual source/binding identity. Resize, pane refresh, source invalidation, renderer fallback, overview toggling, and session transitions must not silently lose or reassign text. A stale source requires an explicit current-source capture before saving, as existing behavior requires.
- Keep the current batch preview and explicit same-tab agent paste workflow. Save is not paste; paste is not Enter. Pending/rejected/unknown outcomes retain drafts and receipts. Do not add automatic resend.

The mock demonstrates one Summary block and one Review line. It does not limit production to those anchors, one file, one comment, or one mode. Its page-memory storage and shortened source are not implementation patterns.

### Setup: two operations, not one overloaded form

| | New worktree | Existing directory |
| --- | --- | --- |
| Required input | Local repository and new branch/default branch policy | Directory path only |
| Optional shortcut | Issue/MR URL resolves configured local repository and branch metadata | None required; do not demand repository discovery or provider setup |
| Space name | Defaults from branch until explicitly edited | Defaults from directory name until explicitly edited |
| Advanced | Base revision and destination | No branch, base, worktree destination, or Git initialization |
| Filesystem ownership | Record newly created worktree/companion ownership | Directory remains borrowed, whether Git or not |
| Git behavior | Existing supported worktree creation | Detect existing Git metadata if present; never switch branches or turn a plain folder into a repository |

**New worktree defaults:** resolve issue/MR metadata through existing provider boundaries, not URL string guesses. An MR supplies its actual source branch; an issue uses configured branch naming. Map to an already configured local repository. If matching is ambiguous, let the user select the repository; do not clone or guess. Ignore late lookup responses for superseded URLs. Preserve explicit branch/name/destination edits; a manually typed branch drives the default Space name until overridden. Invalid/unavailable provider input leaves manual repository/branch setup usable.

**Existing directory:** offer direct path entry (and the existing platform-appropriate directory picker if available), not a repository catalog disguised as a path selector. Hide and disable repository, URL, branch, base, and worktree-destination inputs. Validate that the path identifies an accessible directory in the core. A missing path, a regular file, or denied access gets a field-local error; none triggers directory creation, cloning, `git init`, or fallback to another repository. Detect Git from the chosen path if present. Non-Git operation is a first-class success case, not a warning or conversion flow. A nested path must remain the requested working directory; discovered Git toplevel is metadata, not permission to replace that path.

**Lifecycle implementation is required:** [SetupDialog.makeRequest](../../../src/app/projects/SetupDialog.tsx) currently emits `repository_id` and a repository-action consent field; its form still models branch-versus-path opening. Removing controls alone is insufficient. Split create/open validation in the shared request/core path so directory opening does not require a repository ID. Use the supported Herdr operation for an ordinary directory Space, not a repository-required worktree call with invented provenance. If the installed API requires a different operation, implement that mapping in the adapter and expose an explicit unsupported error only if the capability truly does not exist. Never satisfy the UI by initializing Git.

Preserve returned resource identity, companion association, explicit environment handoff, operation receipts, partial-state recovery, and authoritative focus. Distinguish owned resources from borrowed directories in the durable lifecycle record: closing the Space or rolling back companion provisioning must never delete the user's directory or remove its existing worktree. Update teardown validation with the creation contract, not in a later cleanup.

**Repository actions:** configured actions run automatically for this personal IDE; there is no consent checkbox or replacement confirmation. Implement the selected policy in core/configuration, not a UI-only hardcoded success. This permission does not grant Git `safe.directory`/ownership overrides, arbitrary destructive cleanup, or implicit provider mutations. No repository means no repository-specific action. Retain separate confirmations for actual destructive operations where already required.

Keep the authoritative plan/effects check, bounded progress, operation identity, and recovery behavior while removing the wizard. Recompute a stale plan when input changes; do not execute a receipt bound to old inputs. Show progress/failure in the same compact form. Unknown outcomes require inspection/reconciliation, not automatic repeat submission. Do not force a separate teaching/review screen just to expose occasional operation details.

### Browser annotations

- Preserve Browse, Freehand, Element, Region, Clear, capture, and toolbar close/reopen. No additional “Add idea” action, guidance banner, descriptive slogan, or persistence panel.
- Element targeting and region selection open the inline editor directly. Freehand draws directly and simplifies on release. Keep page-relative geometry, scrolling/zoom behavior, pointer capture, cancellation, and existing bounds.
- Notes sidebar is closed by default and used only to find/revisit annotations. Selecting a note reveals its original anchor and editor; authoring never requires the sidebar.
- Preserve existing storage and capture semantics behind the small controls. Toolbar simplification is not permission to remove durable storage, pairing, or capture behavior.
- Separate connection, successful capture, stored annotation, recipient eligibility, and delivery. Surface a failure beside the operation that failed; do not make diagnostic state the default authoring experience.
- Do not infer a switch to an embedded browser from the screenshot backdrop. Browser-engine and OSR proof-of-concept work is outside this handoff.

## Transforming surfaces not fully mocked

Apply the same rules to the actual existing surface. These rows are implementation requirements for its polish, not a request to invent missing products.

| Surface | Transformation | Preserve / verify |
| --- | --- | --- |
| Session chooser, startup, compatibility | Compact searchable resource selection; one scoped connection/compatibility notice; no welcome page | Transactional session switch, last-known content, no mixed-session state, actionable failure |
| Populated Spaces tree | Shared row typography and selection markers; compact name/status; natural row/menu/drag targets | Current Herdr ordering/grouping semantics, expand/focus distinction, rename/drop confirmation, long/deep names |
| Populated Agents queue | Existing attention order, concise state word/glyph, owning context; no cards or chat transcript | Real populated states, freshness/unknown state, owning-pane navigation; the atlas only shows an empty queue |
| Tabs, pane menus, split/zoom/move | Consistent headers and icon sizing; Pane/Commands at tab-bar right; group ordinary, advanced, and destructive actions | Herdr layout, stable focus/ownership, pending/rejected mutations, overflow and keyboard navigation |
| Commands | One search field, one independently scrolling result list, short keyboard footer | Existing command set and shortcuts; no clipped final result; empty query/results and Escape/focus return |
| File picker, search, Details | Same picker in both viewers; human-readable identity first, exact path accessible; metadata collapsed | Index loading/incomplete/empty/error states, bounded search, stable visible-tree order, full copyable paths |
| Markdown, source, logs, images, HTML, Mermaid | Shared content inset/type; subdued metadata; local media/error controls | Source mapping, safe isolation/refusals, finite preview limits, exact source text, scroll restoration, no external requests from unsafe preview |
| Review scopes, old/new/source, empty diff, binary/renamed files | Shared file hierarchy and bottom selection/comment controls; explicit side/scope without repeated captions | Existing supported scopes, exact revisions, side-aware ranges, no Git/provider mutation from review |
| Comment overview, preview, paste/recovery | Optional overview with the same draft state; useful batch actions, recovery details collapsed | Durable identity, stale excerpts, target eligibility, receipts, unknown-outcome handling, no auto-submit |
| Setup progress and partial recovery | Same form with concise active step, affected resource/path, recovery action; no repeated explanations | Plan generation, idempotent/reconciled operations, companion failures, manual setup without providers |
| Teardown and recovery | Compact owned-versus-borrowed resource list and clear destructive action | Never delete an opened directory or unrelated resources; preserve required destruction confirmation |
| Source ingestion/import/snapshots | Use common field, disclosure, progress, and local error patterns; do not add separate top-level modes | Provider-neutral identities, bounded imports, ownership, freshness, manual context without an agent/provider |
| Extension popup, pairing, capture failure, recipient state | Short operational status and relevant recovery only; separate “connected” from task success | Existing association and persistence boundaries; no false saved/delivered badge, no automatic duplicate mutation |
| Inline pending/stale/disconnected/unsupported/error states | Stable geometry; short operation-specific message and allowed recovery | Keep usable content visible, state label plus color, no global disabling of unrelated resources |
| Empty states and long-content extremes | One local sentence and a useful available action; truncate labels, not source or operation identity | Full identity on disclosure, long paths/URLs, no-result distinctions, 480px viewport and narrow split panes |

## Existing implementation seams

These are inspected entry points, not a mandate to rewrite entire files. Locate current symbol references and existing contracts before editing; source may have advanced since this handoff.

| Responsibility | Starting points |
| --- | --- |
| Shell, hierarchy, commands, Pane actions | [App.tsx](../../../src/app/App.tsx), [styles.css](../../../src/app/styles.css), [session modules](../../../src/app/session), [keymap](../../../src/app/input/keymap.ts) |
| Shared viewer layout/navigation | [viewer.css](../../../src/app/viewer.css), [FilePicker](../../../src/app/input/FilePicker.tsx), [fileNavigation](../../../src/app/input/fileNavigation.ts) |
| Files and source/Markdown anchoring | [ContextViewer](../../../src/app/context/ContextViewer.tsx), [context styles](../../../src/app/context/context.css), [core context](../../../crates/cockpit-core/src/context.rs) |
| Review | [ReviewPane](../../../src/app/review/ReviewPane.tsx), [ReviewViewer](../../../src/app/review/ReviewViewer.tsx), [review styles](../../../src/app/review/review.css), [core review](../../../crates/cockpit-core/src/review.rs) |
| Shared comments, overview, paste | [CommentDrafts](../../../src/app/context/CommentDrafts.tsx), [comments.css](../../../src/app/context/comments.css), [CommentPasteControls](../../../src/app/context/CommentPasteControls.tsx), [core comments](../../../crates/cockpit-core/src/comments) |
| Setup and ownership lifecycle | [SetupDialog](../../../src/app/projects/SetupDialog.tsx), [setup.css](../../../src/app/projects/setup.css), [projects.rs](../../../crates/cockpit-core/src/projects.rs), [repositories.rs](../../../crates/cockpit-core/src/repositories.rs), [project_store.rs](../../../crates/cockpit-core/src/project_store.rs), [project_teardown.rs](../../../crates/cockpit-core/src/project_teardown.rs) |
| Contract cutover | [CockpitClient](../../../src/client/CockpitClient.ts), [projectProtocol](../../../src/client/projectProtocol.ts), [protocol crate](../../../crates/cockpit-protocol), [native/browser adapters](../../../src/client), [Herdr adapter](../../../crates/cockpit-herdr), [host](../../../crates/cockpit-host), [Tauri](../../../src-tauri/src) |
| Browser annotations and delivery | [content.js](../../../browser-extension/content.js), [background.js](../../../browser-extension/background.js), [popup.js](../../../browser-extension/popup.js), [browser feedback](../../../crates/cockpit-core/src/browser_feedback.rs) |

Extend the current transport-neutral client and shared Rust services. No direct filesystem/provider/Herdr calls from presentation components. Regenerate protocol artifacts through the existing generator and migrate every adapter/caller when the setup contract changes. No deprecated alternate form or compatibility shim for the old branch-based “existing” workflow.

## Implementation sequence

Inventory the current implementation first and keep unrelated work unstaged. Use one observable outcome per increment; do not allocate a broad terminal or architecture rewrite as “preparation.”

| Increment | End-to-end outcome | Gate before commit |
| --- | --- | --- |
| A. Shared chrome and hierarchy | One token system across existing shell, menus, populated trees, and viewers; terminal contents unchanged | Browser hierarchy/menu/keyboard evidence with real Herdr state; geometry and terminal input/resize regression checks |
| B. Files and Review navigation/annotations | Matching overview/picker, lower metadata weight, full inline comments in both, usable narrow editor | Real file/range selection → source-bound save → inline result → overview/edit/delete; stale-source and resize retention; existing paste receipts remain correct |
| C. Path-only setup contract and lifecycle | Plain directory and Git checkout both open without a repository selection; ownership/teardown stay correct | Protocol/core/adapters migrated; disposable Git and non-Git directories; no Git initialization, branch switch, or deletion of borrowed files; browser and native operation evidence |
| D. Compact setup and defaults | No wizard/consent checkbox; URL and manual defaults work; authoritative execution/progress remains | Provider fixture plus supported real provider when available; manual no-provider path; explicit overrides and late-response handling; stale plan/partial/unknown outcome recovery |
| E. Browser annotation polish | Existing direct authoring with thicker optimized freehand and idle toolbar fade | Actual extension on a disposable browser page: pointer, zoom/scroll, cancellation, notes, keyboard focus, optional overview, capture/storage failure |
| F. Remaining surfaces and integration | Non-mocked surfaces follow the matrix without feature loss | Cross-surface state/layout matrix, full action-to-result evidence, scoped native checks, documentation and final owned commit |

B and E can be independently owned after shared token contracts are fixed. C owns setup DTO/core/adapter/ownership changes; D consumes that contract and must not independently invent request semantics. One integration owner controls shared token files and protocol generation. F is not a place to defer named acceptance criteria: each preceding increment proves its affected real path before completion.

### Required acceptance matrix

- Desktop 1440×900, minimum desktop 1024×640, 800×1000, and 480×900; also narrow **panes inside a wide window**. No page-level horizontal overflow, clipped required actions, accidental layout zoom, or source-font shrinking.
- Keyboard and pointer: visible focus, Escape restoration, text input/IME left intact, editor newline/save, tree/file/hunk navigation, Herdr magic-escape priority, and no shortcut leakage into terminal input.
- Every changed operation: user gesture → pending where applicable → authoritative response/event → rendered result; then rejected, stale, or unknown outcome as appropriate. A successful request alone is not UI proof.
- Files and Review: multiple files, whole-file and line/range anchors, old/new Review sides, Markdown-to-source mapping, ordinary folder without companion, source invalidation, persistence across reload/reopen, and retained unsaved edits across resize/refresh.
- Setup: issue URL, MR URL, manual branch, explicit name/destination override, ambiguous/unavailable lookup; Git directory, plain directory, nested directory, inaccessible/non-directory path; owned versus borrowed cleanup and interrupted provisioning. Verify absence of `.git` creation and preserved user files in a disposable plain-directory case.
- Browser annotations: continuous live freehand, release simplification without corner/end loss, 3px freehand versus 2px region, scroll/zoom geometry, pointer cancel, inline notes, sidebar revisit, idle/hover/keyboard opacity, storage/capture failure, and no-agent/unknown-delivery cases where those existing operations are exercised.
- Terminal regression: preserve actual output and cell metrics; exercise selection, keyboard, pointer, scrolling, attachment/focus/ownership, and window resizing. Do not use the mock's terminal image as runtime proof.
- Use uniquely named disposable Herdr sessions and owned fixture directories/browser sessions only. Never send test input to, restart, or change ownership of the user's active session. Clean up only resources the scenario created.
- Shared frontend changes need real browser proof. Native commands/channels, startup/window behavior, platform input, or native-only rendering changes also need an actual native smoke. Report platform limits rather than treating browser results as native evidence.
- Run focused existing tests and static checks for the changed contracts. Keep new regression tests for plausible failures such as source-identity leakage, stale lookup/plan execution, or borrowed-directory deletion—not screenshot wording or mock wiring.

### Definition of complete

All approved behaviors and affected non-mocked surfaces are implemented or already present and verified; old callers/controls are removed; authorities document the selected lifecycle policy; failures remain actionable; no real workflow depends on fixtures. Each increment has its acceptance evidence and an owned commit. The final report links real-surface captures, names exercised failures and native coverage, and distinguishes unexercised capabilities from passing ones.

## What not to copy from the mock

Do not copy its static terminal/page images, local arrays, hardcoded provider metadata or paths, single-note targets, disabled setup submission, demonstration tabs, or direct DOM event/state architecture. Do not use its approximate dimensions as a pixel specification for terminal or existing source metrics. Its purpose is to communicate hierarchy, density, action placement, defaulting, and the resulting interactions. Use the shared product implementations and the contracts above for everything else.
