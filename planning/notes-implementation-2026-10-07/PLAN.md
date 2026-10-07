# Notes C/W/L polish: implementation plan (NR-01…NR-49)

Planner: `NotesImplementationPlan`; source-grounded finalizer: `NotesPlanFinalize`, 2026-10-07, run `21c5c9b2-…` (parent `00ef1b2c-…`). This document is a plan only. No product files were edited; no build, test, lint, formatter, install, fixture, browser, commit or push was executed. Only the existing `PLAN.md` and [`COVERAGE.md`](COVERAGE.md) were updated. COVERAGE holds exactly 49 NR rows, D-01…D-12 exclusions and the G1–G19 map.

Accepted inputs (immutable, read-only): `planning/notes-design-review-2026-10-07/DESIGN.md` (DESIGN) and `PRESENTATION.html`, and `planning/notes-atlas-2026-10-07/` (ATLAS, `evidence/INDEX.md`, `evidence/VERIFICATION.md`). The scout handoff `agent://NotesSourceMap` (`/report`) was used too. Its claims were re-read against current source, and the corrections are noted inline.

**Barrier R0 (STOP). Parent must review this plan and explicitly release execution before ANY fixture launch, build, test, formatter, browser automation, product edit or other runtime work. Stage1 is research/artifacts only. No installs, repository commits or pushes are authorized in any later step either.**

---

## 1. Outcome

- The approved polish is in place, with every NR disposition taken from COVERAGE:
  - 38 ordinary apply items, with their approved C/W classes retained (NR-15 adds only lane wording to an existing chip).
  - 5 layout/focus-review dispositions are applied: 03, 08, 23, 29, 46. These dispositions are execution categories, not changes to DESIGN's classes.
  - NR-21 lands after the nested baseline.
  - NR-43 remains required pending rebuilt-source evidence; nonreproduction escalates to the parent, never automatically cancels the remedy.
  - NR-07 is limited to narrow header padding. NR-39 is limited to the summary rename and void tightening.
  - NR-42 is verify-only.
  - NR-49 is handled outside Notes after evidence and explicit shared-file coordination; confirmed-but-unfixed is not a successful full-scope result.
- D-01…D-12 are not implemented.
- The work is four independent component slices (Shell, Task, Board, Decisions). Main owns `src/app/notes/notes.css` and `src/app/notes/NotesView.behavior.test.tsx` and integrates.
- Existing behavior is unchanged: focus/navigation, counts, adoption, UUID pinning, source order, drafts, CAS, the unknown-outcome flow (read fails or succeeds → explicit ack → survives remount → no replay), comments and decision history. The widget anchor commit `b538420` is untouched.
- Verification runs against an isolated build of HEAD plus the Notes files, served from an isolated static dir to a disposable fixture with pinned UUIDs and byte readback, at 1440/900/420 with the Notes container width measured. Real-integration evidence is kept separate from evidence produced by a controlled DTO fault.

## 2. Evidence (current source, fresh reads 2026-10-07)

### 2.1 Authorities

- `.omp/AGENTS.md:6`: UI changes are verified in the browser build against a disposable fixture. A native run is required only for native-only changes.
- `.omp/RULES.md:3,8`: say "fixed" only after running the scenario. Use only disposable Herdr sessions via `scripts/verify/ui_polish_runtime.py`. The parent's no-commit/no-push instruction overrides the generic commit rule at `:7`.
- `CONTEXT.md:154-166`, `DECISIONS.md:94-102` (Space Notes): UUID binding, source order, additive history, explicit unknown-outcome acknowledgement.
- `CODE_GUIDE.md:98-123` (Durable Notes call flow); `docs/keyboard-shortcuts.md:113-123` (Notes keys).
- DESIGN `:24-31` (classes), `:120-207` (49-row register), `:235-245` (options), `:247-251` (I-1…I-12), `:255-274` (D), `:278-302` (G1–G19), `:304-313` (waves), `:315-322` (uncertainty). ATLAS state matrices (`ATLAS.md:96-112`), focus map (`:146-163`), preservation (`:165-181`).
- Concurrent Supervisor run, from the parent-reviewed corrections in its plan (`planning/supervisor-implementation-2026-10-07/PLAN.md:17-18`): Notes files belong to the Notes worker. App/global styles and UiIcon changes need parent coordination. Vite output must be isolated. `b538420` is preserved.

### 2.2 Current facts

| # | Fact | Source |
|---|---|---|
| E1 | `notes.css` is one rule per line. Breakpoints are Notes container queries (`container:notes`, L1): ≥900 L256, ≤899 L257, ≤719 L258-270, ≤520 L271-285. Coarse-pointer reveal is at L286. Shared reading inset can use existing `cqw` units (L91) and a literal length without font-dependent custom-property registration. Current source-column padding is L37, not L38 (gutters). | `src/app/notes/notes.css:1,27,36-41,91,256-299` |
| E2 | The problem slot is `position:absolute;bottom:0` at L62-64, rendered **inside** `.notes-panels` after TaskDetail. `NotesContent` children are direct flex children of `section.notes-view` (a column). | `src/app/notes/NotesView.tsx:128-160,267-280`; `src/app/notes/notes.css:1,25,62-64` |
| E3 | The empty current slot has no children or padding when neither error exists. The in-flow replacement must retain that zero-height state, not reserve an empty band. | `src/app/notes/NotesView.tsx:152-158`; `src/app/notes/notes.css:62-65` |
| E4 | Mutation errors without authoritative notes codes map to unknown; mutate refuses further writes, incident persists in drafts. Saved-state check/read proof gates explicit ack. | `src/app/notes/useNotes.ts:30-33,98-113`; `src/app/notes/NotesView.tsx:70-90` |
| E5 | Browser POSTs Notes, matches response identity/kind, and wrong-kind response becomes malformed_response (then unknown for a mutation). Real comment success envelope has `result.kind="comment"`. | `src/client/browser.ts:979-986`; `src/client/client.test.ts:1122-1126`; `src/client/notesProtocol.ts:220-224`; `crates/cockpit-protocol/src/notes.rs:224-226` |
| E6 | An invalid decided date is refused authoritatively with `notes_invalid_input` and writes nothing. | `crates/cockpit-core/src/notes/decisions.rs:414-430` |
| E7 | Count bases: Todos !done, Kanban lane !== null, Decisions all loaded records. | `src/app/notes/NotesView.tsx:135`; `src/app/notes/useNotes.ts:77` |
| E8 | Panels are labelled by tab IDs, so tab-name changes also rename panels. | `src/app/notes/NotesView.tsx:137,140,149-150` |
| E9 | Tests find buttons by text or aria-label; Todos lookups at :144/:280 need ID selectors. Recovery strings/Attach/Cancel/Keep/close names remain unchanged. Unknown tests reject comment_add with TypeError. Existing tests mock MarkdownEditor/preview and do not prove real editor geometry. | `src/app/notes/NotesView.behavior.test.tsx:1-17,120-127,144,169-226,258-290` |
| E10 | Card whitespace filter excludes all controls including label; Tab drag recovery queries real enabled inputs; apply refocuses the existing input class. | `src/app/notes/Kanban.tsx:29-60` |
| E11 | Comment focus selectors pin Edit first/Delete last in row actions; do not change those positions. | `src/app/notes/TaskDetail.tsx:10,85-87,203` |
| E12 | Picker Attach/Cancel are direct binding children; binding-actions wrapper belongs only to unbound state; transfer replaces Attach. | `src/app/notes/NotesView.tsx:269-276` |
| E13 | Catalog enumerates UUID **directories**, then enriches them with registry labels/bindings. Removing a fixture-owned directory can make a selected entry disappear on reopening; registry mutation is not required. NR-05 is deterministically pinned by T2, not misrepresented as runtime-unreachable. | `crates/cockpit-core/src/notes/registry.rs:168-202`; `src/app/notes/NotesView.tsx:240-248,266,272-274` |
| E14 | Helper facts: `REPO = Path(__file__).resolve().parents[2]` (L16), so copy the helper physically into the snapshot (do not symlink it). `environment()` filters inherited HERDR_/COCKPIT_ vars and sets isolated HOME/XDG/config (L19-30). TOML contains legacy `companion_root`, not `notes_root` (L116-119). First Space: `workspace.create` (L152). Gateway: `REPO/target/debug/cockpit`, `--static-dir REPO/dist` (L154-155). `start` prints root last (L156). `rpc` guards `cpol-`/`polish-` (L33-43); `stop` checks ledger root and process cmdline/environ before killing matching fixture groups (L159-171). Helper also creates a synthetic fixture-only Git commit at L123-124; see Q3 before launch. | `scripts/verify/ui_polish_runtime.py:16-43,103-124,132-171` |
| E15 | Config: legacy `companion_root` is still parsed and validated (L121, L275-287). The `notes_root` default is `$XDG_DATA_HOME/cockpit/notes` (L312-319), which under the helper is `<fixture>/data/cockpit/notes`. Overlap with the library/state/companion/worktree/cache/repository roots is refused (L320-336). | `crates/cockpit-core/src/config.rs` |
| E16 | Gateway validates static index before binding; CLI static default/Vite output are dist. Build invokes tsc noEmit and Vite; no incremental compiler output. | `crates/cockpit-host/src/server.rs:116-147`; `crates/cockpit-host/src/bin/cockpit.rs:286-287`; `vite.config.ts:12-14`; `package.json:8`; `tsconfig.json:1-20` |
| E17 | `resource_guard.py` is a fail-closed Herdr invocation planner keyed on a run ledger. It starts nothing, and the helper does not use it. | `scripts/verify/resource_guard.py:1-8,698-713` |
| E18 | Notes trigger is tab-strip-action with aria-pressed; generic pressed styling currently only targets tab-icon-button. Supervisor has its own selection rule. Its source line moved during the concurrent run: current :248, not scout :241. Coordinate any future change rather than editing the Supervisor file. | `src/app/App.tsx:239-240`; `src/app/styles.css:1647-1651`; `src/app/supervisor/supervisor.css:247-248` |
| E19 | Icons `trash`, `back`, `down`, `plus` already exist. No new asset is needed. | `src/app/UiIcon.tsx:3,6,13,15` |
| E20 | Tokens exist: `--font-size-control` (0.75rem), `--font-size-2xs`, `--warning`, `--blocked`, `--radius-pill`, `--select-*`, `--compact-control-size`. | `src/app/styles.css:19-34,46-50,68,86` |
| E21 | `package.json` defines build, typecheck and test scripts but no lint/formatter script. Use the existing durable Notes guide for the narrow permanent docs update, not a new changelog convention. | `package.json:5-18`; `CODE_GUIDE.md:98-108` |
| E22 | Atlas bundle index-CssAnWz1.js is not proven current source. S-only findings are 05,11,18,21,27,41,43–46. Source/rendered/illustrative evidence must stay distinct. | `planning/notes-atlas-2026-10-07/evidence/INDEX.md:7-11`; `planning/notes-design-review-2026-10-07/DESIGN.md:39-47,315-322` |
| E23 | `NotesTodo` carries depth; rendered nesting is capped at 6 with +12px padding per level. Source issue markup is `TodoTitle.tsx:72` (not the scout's earlier :57-73 span). | `src/protocol/generated/v1.ts:464`; `src/app/notes/NotesView.tsx:143`; `src/app/notes/TodoTitle.tsx:61-73` |
| E24 | Board chip already derives full Done/Doing/Backlog in its title/name; visible content is only `grid`. NR-15 changes that content to unambiguous abbreviations, not state or navigation. | `src/app/notes/NotesView.tsx:145`; `src/app/notes/notes.css:103-108,286` |
| E25 | NR-27 state owners: removed :186, loading :194 (thread already aria-busy :189), discard :203, retained :212, new-below :214, unboarded :183; item problems :72. New-below is already a sibling below the scrolling thread; no sticky overlay or new behavior is needed. | `src/app/notes/TaskDetail.tsx:183-214`; `src/app/notes/TodoTitle.tsx:72`; `src/app/notes/notes.css:151-184` |
| E26 | Four write sets contain **eight** component files (4+2+1+1); `TodoTitle.tsx` is reserved but unchanged. Agent access has **23** command entries (:25-47), grouped as resolve 1 + Scratchpad 3 + Todos 4 + Board 5 + Decisions 5 + Comments 5. | `src/app/notes/AgentAccess.tsx:24-48`; §5.5–§5.8 |
| E27 | Installed CodeMirror base `.cm-line` has asymmetric 6px-left/2px-right padding. Scratchpad-only zero-inline override makes shared-inset text alignment exact; leave other editors untouched. | `node_modules/@codemirror/view/dist/index.js:6890-6893`; `src/app/notes/MarkdownEditor.tsx:50-71`; `src/app/notes/notes.css:36-41` |
| E28 | Comment collection propagates bounded UTF-8 read failures; TaskDetail poll/read sets visible local error with explicit Retry. Deterministic real-backend fixture-only NR-24 probe needs no product changes. | `crates/cockpit-core/src/notes/comments.rs:74-96`; `crates/cockpit-core/src/notes/fs.rs:213-216`; `src/app/notes/TaskDetail.tsx:42-75,188` |

## 3. Decisions

| # | Decision | Rejected alternatives (why) |
|---|---|---|
| D1 | One batch. Four component slices write only JSX in their own files. Main writes all CSS in `notes.css` and all test edits. Class hooks are fixed up front in COVERAGE/§5.4 so CSS and JSX can proceed in parallel. | Per-slice CSS edits (four writers in one file). A new per-component CSS file (a second convention). |
| D2 | NR-06 changes the tab **accessible name** (`aria-label` and `title`) because G18 requires names. The test lookup moves to `#f-todos`, not the new wording. | A `title`-only description, which keeps names stable but fails G18's "names". A visually hidden suffix, which `aria-label` overrides. |
| D3 | NR-03 uses visual `[Cancel][Attach]` through `flex-direction:row-reverse` while DOM order stays Attach→Cancel, exactly as DESIGN states. | Reordering the DOM breaks the focus contract and tests. Showing `[Attach][Cancel]` deviates from the approved text. |
| D4 | NR-08 adds a second button, `.notes-task-back`, shown only at ≤719 (copying the `Decisions.tsx:108` / `notes.css:221,264` pattern). The ✕ button is hidden there. Both call `onClose`. | One button whose label is swapped by CSS: the accessible name cannot follow a container query. A JS width measurement adds state and a ResizeObserver. |
| D5 | NR-09 uses ordinary unregistered length variables in existing `notes.css`. B6 computes the current wide source measure once from resolved scroller padding, then M1 writes that literal px length as `--notes-measure`; no font-metric guess. All four surfaces share `--notes-column-inset` from the existing Notes `cqw` container and outer padding (24px / 18px). Keep full-width editor/scrollport; align its padding, not a reparented editor. Exact rules are in COVERAGE row 09. | Registered `@property` (new native compatibility requirement, unnecessary); per-element `76ch` (different font metrics); JS resize/state tracking (new behavior); a max-width editor wrapper (moves the scrollport). |
| D6 | NR-16 uses the existing `plus` icon, an "Adopt" label at ≥521 and an exact title, with the same handler and aria-label, in both Todos and Kanban. | Confirmation is D-02. A new icon asset is forbidden. |
| D7 | NR-15 replaces only the existing chip's `grid` glyph with `B` / `Do` / `Dn` for Backlog / Doing / Done. Existing full lane title and aria-label stay verbatim. These disambiguated initials are always visible at .65, full on hover/focus/coarse. Use a fixed 106px trailing action grid (64px comment/adopt, 36px lane) so counts align. | B/D/D (ambiguous); title-only/grid-only remedy (silently omits DESIGN :153 lane indicator); a new indicator, lane tracking or handler (D-class). |
| D8 | NR-29 is CSS only. The menu summary is absolutely positioned top-right, and `:not(:has(…))` collapses the lower row when there is no count, draft, adopt or open menu. Kanban DOM is untouched. | JSX reordering or conditionally rendering the actions row would break the frozen card order or drop the keyboard opener. |
| D9 | NR-43 moves the slot just after `.notes-panels` once B4 documents the current behavior. Focusable sequence remains unchanged because the slot was already last inside panels. If B4 does not reproduce it, preserve the required item and escalate Q4 before its JSX/CSS edits; proceed on independent slices. No automatic skip. | Bottom padding workaround (DESIGN :241 rejects it); unconditional omission after one narrow probe; claiming G16 passes while retaining a confirmed occluding overlay. |
| D10 | NR-44 adds an `<ol>` only for the unknown branch. Strings, buttons and conditions are unchanged. | Restructuring every error, which is unneeded churn for known errors. |
| D11 | NR-46 groups entries under `h4` headings. The flat order, labels, values and copy status are unchanged. | Collapsible groups (new state). |
| D12 | NR-48 wraps the bare checkboxes in a 24×24 `label.notes-check`. The box stays 16px. | Enlarging the checkbox changes it visually. Padding on a native checkbox has unspecified hit-testing ([INFERENCE]). |
| D13 | NR-21 adds a `data-depth` attribute and a CSS gradient guide. Inline padding is unchanged. | CSS cannot read inline padding. Per-level pseudo-elements would need nested DOM. |
| D14 | NR-39 removes the summary's `aria-label` override so the visible text "Details & replacement" becomes the name. Replace is not moved. | Keeping a mismatched override. The relocation option (L), which the contract excludes. |
| D15 | NR-47: the functional 11px rules listed in COVERAGE go to 12px. Counts, the code secondary, the toolbar "markdown" tag and the `cockpit-cli` meta stay 11px. | Raising everything breaks the narrow layout for decorative text. |
| D16 | Archive recorded HEAD into owned snapshot; overlay agreed Notes files, build isolated dist/target. Physically copy helper (E14); only a Q3-cleared snapshot-local no-commit adjustment may differ. Reuse a host read-only after actual CLI/help and fixture DTO checks; rebuild isolated if incompatible. Hashes establish provenance, not protocol compatibility. | Shared dist/target builds; timestamp freshness; shared helper edits; symlinked helper resolving to the user's checkout. |
| D17 | Unknown outcome uses a controlled DTO fault (class F): an available SDK browser with Playwright-compatible routing forwards exactly one `comment_add` to the real gateway, confirms its real success/readback, then returns a wrong-kind body (E5). Optional existing CLI browser automation is equivalent, not required. | Abort before dispatch (no applied write); backend fault injection (product scope); installing a browser CLI or requiring it when the SDK suffices. |
| D18 | Known error (class R) is the real backend `notes_invalid_input` from an invalid decided date (E6). It persists across tab switches until the next mutate or check. | A localStorage failure is a controlled fault. A race-based `notes_conflict` is nondeterministic. |
| D19 | Tests change only meaningful behavior: (1) decouple `button("Todos")` from the tab's accessible name; (2) add one guard test for NR-05 (vanished selection never attaches). No jsdom style assertions and no re-pinned wording. Other acceptance is runtime evidence. | Wiring/label tests for NR-08/18/37 (covered at runtime). Pinning "Todos — n open". |
| D20 | Docs: Main adds one narrow sentence in `CODE_GUIDE.md` §Durable Notes call flow after shared-file clearance. If NR-43 lands, include its zero-height in-flow slot contract; otherwise describe presentation ownership and frozen behavior only. Keyboard docs `:113-123` remain unchanged (no keys change). No new changelog. | Permanent product change with no docs; re-authoring unchanged shortcuts; editing approved design/atlas. |
| D21 | No native run is required for this frontend-only scope (`.omp/AGENTS.md:6`). Browser proof is not native proof; record WebKitGTK `:has()`/mask support as unverified. NR-09 adds no registered-property dependency. No installs or repository commits/pushes. | Treating Chromium screenshots as native compatibility evidence; adding native execution or installation to this polish batch. |

## 4. Open questions (parent)

| # | Question | Options | Recommendation / default |
|---|---|---|---|
| Q1 | NR-49 global style is shared with the active Supervisor run | (a) After B5 confirms it, explicitly clear Main for the one COVERAGE row-49 rule; (b) parent assigns the same rule and acceptance to the coordinated Supervisor owner. | (a). Request coordination if unanswered; keep NR-49 open and do not deliver confirmed-but-unfixed as success. No fallback omission. |
| Q2 | Shared `CODE_GUIDE.md` insertion after :104 | (a) Clear Main for the narrow ownership/frozen-behavior sentence (plus zero-height in-flow slot if NR-43 lands); (b) assign that exact docs update to the coordinated owner. | (a). Await shared clearance for this file only; independent Notes work continues. Approved design/atlas and keyboard docs stay unchanged. |
| Q3 | Mandated helper internally commits synthetic fixture repo (`scripts/verify/ui_polish_runtime.py:123-124`), conflicting with the parent's literal no-commit guard | (a) Parent clears a throwaway snapshot-local helper copy that omits only the commit invocation while retaining init/add and all isolation/launch/stop guards; (b) parent provides an existing compliant disposable fixture path/recipe. | (a). No commit exception/commit option is proposed. Resolve at R0 before helper launch; the repository helper remains untouched. Record exact snapshot-local adjustment; never allow commits/pushes or installs. |
| Q4 | B4 records `NOT_REPRODUCED` | (a) Parent authorizes the already-approved in-flow remedy after reviewing rebuilt-source geometry/known+unknown probes; (b) parent requests one targeted missing-state investigation; (c) parent explicitly changes accepted scope. | (a) keeps G16/full scope. Escalate with evidence; do not decide skip. Until answered, NR-43 remains pending, only its slot edit waits. |

## 5. Slices (dependency-ordered)

```
R0 parent review/release (STOP before ANY execution)
  ─► P0 scoped preflight (M) ─► P1 baseline B1–B6 (M) + shared-clearance requests
  ─► ┌ S1 Shell ┐
      ├ S2 Task  │ parallel, genuine disjoint component write sets
      ├ S3 Board │ no mid-flight builds/tests/lint/formatters/fixtures
      ├ S4 Decis.│
      └ M1 CSS   ┘ (Main only)
  ─► M2 integration + test edits (+NR-49 once cleared)
  ─► P4 final static checks once ─► P5 runtime G1–G19 + extras
  ─► P6 cleared docs/evidence ─► R2 parent acceptance ─► owned cleanup
```

Mid-flight rule: S1–S4 and M1 do not build, lint, test, format or start fixtures. P1 is a single authorized baseline before edits; P4/P5 are Main's single integrated verification pass after all slices land. No unnecessary second global STOP after baseline; only unresolved shared-file clearance/Q4 blocks its own dependent edits. R0 does not authorize installs, commits or pushes.

### 5.1 P0 Preflight (Main, read-only)

1. Record the snapshot's source revision with `git -C "$REPO" rev-parse HEAD`. Preserve widget anchor `b538420`; do not alter App/widget files. Do not run a broad history/status audit as a substitute for actual runtime proof.
2. Read only the eight component files, `notes.css` and the behavior test that will be overlaid, and record their hashes. Coordinate any existing changes with their owner; do not overwrite or restore them. Unrelated Supervisor dirt is not a failure.
3. Confirm existing `node_modules`, Bun and host candidate paths are available. No installs. If a prerequisite is absent, report it to the parent; finish reachable planning/component work rather than fabricate checks.
4. Preserve the user Herdr registry by recording its exact bytes/hash for read-only before/after comparison, not mtime freshness. Never touch the user session.
5. Record candidate gateway executable path/hash. Compatibility is determined later by actual Notes CLI help and fixture HTTP/CLI read DTOs (§5.A.1), never binary/source timestamps.

### 5.2 P1 Baseline (Main; source-current, no product edits)

The baseline captures only the S-only and rebuild-required states plus the comparators used for relative acceptance. It does **not** re-run A01–A19 to confirm findings already established by DESIGN (no rerun to confirm reported failures). Everything goes to `planning/notes-implementation-2026-10-07/evidence/BASELINE.md` with `B-*.png`.

- **B1 Identity.** Build snapshot `baseline` (§5.A.1) from the recorded revision plus agreed working-tree Notes files. Record source revision and overlay hashes, `dist/index.html` module script name plus asset hash, gateway path/hash and whether reused or isolated-built, actual `/proc/<pid>/cmdline` with `--static-dir $SNAP/dist`, Notes CLI/HTTP DTO compatibility results and the script URL actually loaded (SDK browser DOM evaluation). Timestamps are not freshness proof.
- **B2 Container widths.** At viewports 1440×1000, 900×900 and 420×900 record `innerWidth`, `.notes-view` width and band (≥900 / 720–899 / 521–719 / ≤520). Default sidebar 240px and narrow threshold 800px are `src/app/App.tsx:422,426-427`; 900 probably lands at 521–719 ([INFERENCE]; measure). Add one explicitly labelled band-check viewport if 720–899 is missing, for NR-20/34 only.
- **B3 Nested todos (NR-21).** With the seed (§5.A.3), open Todos at 1440 and 420. Capture the screen, then record per row the `data-todo-id`, the computed `padding-inline-start` and the CLI `todo list` `depth`. NR-21 proceeds once B3 exists. It also fixes the guide alignment reference: guides should sit on ancestor checkbox centers.
- **B4 Overlay (NR-43), real known error.** In Decisions → New: title "Probe", decided `2026-13-45`, then Record decision. Expect the alert `notes_invalid_input` with no byte change. Then, keeping the error:
  1. Switch to Scratchpad, type one character so the draft enables Save, and run `document.elementFromPoint` at the center of Save scratchpad.
  2. In Todos, open detail of the **adopted** task T1 (an unadopted one would trigger an adoption write that clears the error), type a composer draft, and run elementFromPoint at the center of `#postCommentBtn`.
  3. In Kanban, check whether the `.notes-live-status` rect intersects the `.notes-problem-slot` rect.
  4. Repeat at 420.
  5. Record `NR43=CONFIRMED` if any probed control center at 1440 or 420 resolves inside `.notes-problem-slot`, else `NOT_REPRODUCED`.
  6. Clean up with Check saved state (a known error clears after refresh, E4), Cancel (discard draft) and discarding the drafts. Hashes must be unchanged.
- **B5 Top-bar pressed (NR-49).** Park the mouse at (0,0) and focus inside Notes. Compare the computed `background-color`, `color` and `box-shadow` of `[aria-controls="cockpit-notes"]` with Notes open (`aria-pressed=true`) against closed. If they are equal, prepare the Q1 clearance request.
- **B6 Comparators and focus sequences** (not gates):
  - Height/focus sequence of T5, the one-line adopted board card with no count (not seeded-comment T1 or non-board T3).
  - Short-decision footer gap (`footer.top − body.bottom`).
  - After fonts settle, record source scroller resolved padding/text edges. With width w and padding pL,pR, require positive inset, freeze M=round(w−pL−pR) as `--notes-measure:Mpx`. If 1440 has no inset, temporarily widen measurement viewport, record and restore. Unmeasurable runtime length escalates; do not guess metrics or introduce @property.
  - Focusable sequences (§5.A.5) for: one card, the picker (on S3), the detail header at 1440 and 420, and the Agent access commands.

### 5.3 Baseline handoff / scoped clearance (not a second global STOP)

Main sends `BASELINE.md`: NR-43 evidence/status, B3 reference, B6 literal reading measure, NR-49 coordination request Q1, docs Q2 and runtime identity. R0 already released independent Notes execution. Begin S1–S4/M1 without waiting for another global review. Await explicit shared-file clearance for NR-49/docs and a Q4 parent decision only if NR-43 was not reproduced; do not silently omit any item.

### 5.4 Shared JSX↔CSS contract (fixed; slices and M1 rely on it)

- **New class and attribute hooks:**
  - S1: `notes-binding-picker`, `notes-picker-actions`, `notes-transfer-attach`, `notes-catalog-missing`, `notes-conflict-saved-label`, `notes-conflict-consequence`, `notes-todos-filtered-empty`, `notes-source-label`, `data-depth`, `notes-check`, `is-adopt`, `notes-adopt-label`, `todo-board-lane`, `notes-unknown`, `notes-unknown-steps`, `notes-editor-mode-label`, `notes-agent-group`, `notes-agent-group-heading`, `notes-agent-group-commands`.
  - S2: `notes-task-back`, `notes-task-close`, `notes-comment-delete`, `notes-comment-author-note`, `notes-task-unboard`, `notes-composer-author-note`.
  - S3: `is-adopt`, `notes-adopt-label`, `notes-check`.
  - S4: `notes-decision-context`, `notes-decision-decided`.
- **Shared strings** (exact): adopt title `Adopt task & comments — adds an id to this task in todos.md`; adopt label `Adopt`.
- **Frozen DOM orders:**
  - Card: handle → checkbox → title → comments → menu.
  - Picker: catalog → (transfer group) → Attach? → Cancel.
  - Todo row `.notes-row-actions`: comments button first.
  - Comment actions: Edit first, Delete last.
  - Detail summary: Completed → Move/Not-on-board → Remove/Add → hint.
  - Alert: message → code → Check → review → ack.
  - Agent commands: the original 23-entry order.
- **Prohibited in every slice:**
  - Changing any handler, state, effect, `disabled` expression, `id`, ref, existing aria-label (other than COVERAGE rows 06/20/32/36/39), `role` or `tabIndex`. Derived render-only constants are allowed; do not alter existing model expressions.
  - Adding focus calls.
  - Editing CSS or tests, or touching another slice's files.
  - Touching `useNotes.ts`, `drafts.ts`, `boardState.ts`, `notesSensors.ts`, `UiIcon.tsx`, `App.tsx`, protocol or Rust.

### 5.5 S1 Shell (`NotesView.tsx`, `MarkdownEditor.tsx`, `RetainedDrafts.tsx`, `AgentAccess.tsx`); runs in parallel with S2–S4 and M1

**Goal:** NR-01, 02, 03, 05, 06, 10, 11, 12, 13, 15 (lane wording), 16 (Todos row), 17, 18, 20, 21, 43 (evidence/parent resolution), 44, 45, 46, 48 (Todos row). Exact edits are in COVERAGE.

**Steps (NotesView.tsx):**
1. Line 269: the picker root becomes ``className={`notes-binding${picker ? " notes-binding-picker" : " notes-binding-empty"}`}``.
2. Lines 272–275: keep the catalog/error/loading branch. After it, insert the NR-05 hint (`catalog.status === "ready" && selected && !candidate`). Then `{confirmTransfer ? <div className="notes-conflict" role="group" …>…</div> : null}` with the same contents, adding `className="notes-transfer-attach"` to Attach here. Then `<div className="notes-picker-actions">{confirmTransfer ? null : <button ref={attachAction} …>Attach</button>}<button ref={pickerCancel} …>Cancel</button></div>`. All refs, handlers and disabled expressions are verbatim.
3. Line 128–135 (tab map): compute `n` with the existing three expressions. `const name = tab === "todos" ? \`Todos — ${n} open\` : tab === "kanban" ? \`Kanban — ${n} on board\` : tab === "decisions" ? \`Decisions — ${n} ${n === 1 ? "record" : "records"} (Current and History)\` : tabLabels[tab];` then `aria-label={name} title={name}`. The count span stays as it is.
4. Line 138: `"Saving…"` → `"Saving Notes…"`. Conflict block: add the saved label inside `<details>` after `<summary>`, remove `notes-primary` from Keep mine, append the consequence `<p>` after Reload. The non-conflict `Save scratchpad` keeps `notes-primary`.
5. Line 142: the toggle text is the constant `Hide completed`. The summary becomes the NR-20 markup.
6. Lines 143–147: define `visibleTodos = model.todos.filter(todo => !surface.hideCompleted || !todo.done)` once above the return and map it. Add `data-depth={Math.min(todo.depth, 6)}` to each `li`. Wrap checkbox in `label.notes-check` (input unchanged). Derive adoption inside the map's render callback and apply COVERAGE row 16; unchanged aria-label/count/handler/disabled. On the existing on-board chip only, replace `grid` with `<span className="todo-board-lane" aria-hidden="true">{todo.done ? "Dn" : todo.lane === "doing" ? "Do" : "B"}</span>`; title/name/handler unchanged, promote icon unchanged. Add NR-18 after `</ul>`; existing `!model.todos.length` prompt stays.
7. Line 152–156:
   - Once B4 confirms the overlay or Q4 explicitly resolves nonreproduction in favor of the approved remedy, move the whole slot after `.notes-panels` and before `<RetainedDrafts>`. If Q4 is pending, complete all independent edits; keep NR-43 explicitly pending.
   - In both cases, restructure the **unknown** branch per NR-44: `className={unknown ? "notes-error notes-unknown" : "notes-error"}`. For unknown, render `<UiIcon name="info" />` then the `<ol>`. For non-unknown, render the current children unchanged.

**MarkdownEditor.tsx (:91):** add mode-label spans after existing icons; no editor/session/effect edits.

**RetainedDrafts.tsx (line 15):** `const count = titles.length + decisions.length;` and the text `{count} unsaved {count === 1 ? "edit" : "edits"} kept — source no longer exists`.

**AgentAccess.tsx (commands :24-48; row renderer :62-65):**
- `const resolve = commands[0]`. `const groups: [string, string[][]][] = [["Scratchpad", commands.slice(1, 4)], ["Todos", commands.slice(4, 8)], ["Board", commands.slice(8, 13)], ["Decisions", commands.slice(13, 18)], ["Comments", commands.slice(18)]]`. Keep `commands` as the single source and slice it, so no entry is retyped.
- Extract a local `renderCommand([label, command])` that returns the existing row markup verbatim.
- Render `.notes-agent-commands` containing `renderCommand(resolve)` followed by the group sections.

**Non-goals:** no CSS/focus logic/catalog-fetch changes; no recovery string, handler or guard change. NR-44 changes only unknown-alert markup as explicitly specified.

**Acceptance (static, by Main at M2):** the diff contains only these edits. Refs and handlers are byte-identical: compare `git diff -U0` hunks against the checklist in §5.9. Runtime acceptance comes at P5.

### 5.6 S2 Task (`TaskDetail.tsx`, `TodoTitle.tsx`); parallel

**Goal:** NR-08, 22, 23, 25, 26. `TodoTitle.tsx` needs no JSX change: NR-19/27/47 are CSS. It is reserved so nothing else writes it.

**Steps:**
1. Line 180: insert the Back button before `<h3>` and add `notes-task-close` to the ✕ button's className (`"notes-icon-button notes-task-close"`).
2. Lines 182–185: the `.notes-task-state` div keeps only the Completed label. Move the `todo.lane ? <label className="notes-task-lane">…</label> : <span className="notes-task-lane is-unboarded">…</span>` expression verbatim to the start of `.notes-task-board-actions`. The lane branch becomes `<>{laneLabel}<button className="notes-task-unboard" …>Remove from board</button><small …/></>`. The else branch becomes `<>{unboardedSpan}<button …><UiIcon name="plus" />Add to board</button></>`.
3. Line 202: when `comment.author` is set, add `<span className="notes-comment-author-note">· unverified label</span>` after the author span.
4. Line 203: on the Delete button, set `className="notes-icon-button notes-comment-delete"` and swap the icon to `trash`. It stays the last child.
5. Line 205: cut the `<details className="notes-comment-reference">` element and paste it right after the line-206–208 edit/preview expression, before the line-209 delete-confirm expression.
6. Line 218: add `<span className="notes-composer-author-note">unverified</span>` after `<span>author</span>`.

**Non-goals:** no change to `commentFocusSelectors`, Escape handling, `post`/`save`/`removeComment`, or the composer button text.

**Acceptance (static):** Delete is still the last button of `.notes-comment-actions`. The detail summary DOM sequence is unchanged. `onClose` is used by exactly two buttons.

### 5.7 S3 Board (`Kanban.tsx`); parallel

**Goal:** NR-16 (card), NR-30, NR-32, NR-48 (card). NR-29/31/33/34/35 are CSS only.

**Steps:**
1. Line 60: wrap the `<input className="kanban-card-check" …/>` verbatim in `<label className="notes-check">…</label>`. The handle `<button>` and the TodoTitle stay where they are.
2. Line 61: comments button. Using `adopt` defined as in S1, ``className={`kanban-card-comments${adopt ? " is-adopt" : ""}`}``, `title={adopt ? ADOPT_TITLE : undefined}`, and the first child `{adopt ? <><UiIcon name="plus" /><span className="notes-adopt-label">Adopt</span></> : <UiIcon name="comment" />}`. The count span, draft dot and aria-label expression are unchanged.
3. Line 61: menu `<summary>`: `<UiIcon name="more" />` → `<UiIcon name="down" />`.
4. Line 185: `aria-label={\`Add a card from ${labels[column]}\`}` → `aria-label="Add a card to Backlog"`.

**Non-goals:** sensors, DnD callbacks, announcements, the click filter, `apply()`, chips JSX.

**Acceptance (static):** the card DOM order is unchanged. `.kanban-card-check` is still on the input.

### 5.8 S4 Decisions (`Decisions.tsx`); parallel

**Goal:** NR-36, 37, 38, 39, 40 (NR-41 is CSS, NR-42 is verify-only).

**Steps:**
1. Line 105: the option text becomes `History (replaced)`. The sort button gets the NR-36 aria-label and visible `Sort: {oldest ? "Oldest" : "Newest"}`.
2. Line 106: chip text `Recorded ${new Date(item.recorded).toLocaleDateString()}`; the unknown text is unchanged.
3. Line 108: insert the NR-37 paragraph after the error-row div.
4. Line 113: add the `Decided` span to the footer meta after the Recorded span. The summary becomes `<summary><UiIcon name="info" />Details & replacement</summary>` (aria-label removed). Replace stays where it is.

**Non-goals:** sorting, filter state, `onSelect`, save/replace logic, the Back button.

**Acceptance (static):** `ordered` and the effects are byte-identical, and the `#replaceBtn` position is unchanged.

### 5.9 M1 CSS (`notes.css`); parallel with S1–S4

Apply the CSS-hooks column of COVERAGE row by row. Grouped by current line:

| Lines | Change |
|---|---|
| L27, L36–L41 and L56 override | NR-09 unregistered literal measure/shared container-relative inset; no `@property` |
| after L7 | NR-28 disabled primary |
| L11 | unchanged (box stays 16px) |
| L14, L147, L150, L166, L173, L180, L193, L203, L216, L220, L233, L249 | `--font-size-2xs` → `--font-size-control` (NR-47; L150 also covers NR-35) |
| L31 + ≤520 block | NR-10 |
| L62–L64 | NR-43 once confirmed or resolved by parent: `.notes-problem-slot{flex:none;max-height:min(160px,45%);overflow:auto}`, drop absolute positioning/insets/z-index/pointer-events; retain empty 0px |
| after L70 | NR-12 scoped reset, `.notes-conflict-saved-label`, `.notes-conflict-consequence` |
| L75 | add `.notes-comment-empty` (NR-24) |
| L88 + ≤719 block | NR-20 |
| L95 | `background` → `background-color` (keeps NR-21 guides) |
| L96, L97, L127 + new `.notes-check`, `.notes-task-completed{min-height:24px}` | NR-48 |
| after L101 | `.notes-todo-title textarea:not(:disabled){cursor:text}` (NR-19) |
| L103, after L107 | NR-15 fixed action grid / visible B, Do, Dn chip |
| new | NR-16 `.is-adopt` opacity and ≤520 label hide |
| new | NR-21 six depth rules + exact repeating border-token guide image from COVERAGE; ancestor centers at 24px + 12px per level, account for NR-48 wrapper; nesting padding unchanged |
| L111, L112 | NR-31 chips |
| L122, L136, L142 + new | NR-29 card compaction, NR-30 chevron rotation, NR-33 cursor and selected-handle .4 |
| L257 block | NR-34 mask |
| new | NR-14 gutters, NR-22 delete tint, NR-23 author note, NR-24 `.notes-task-detail>.notes-error-row:empty{min-height:0}`, NR-25 unboard, NR-26 note, NR-27 seven rules with exact sources (E25/COVERAGE), NR-37 context, NR-41 id tints, NR-44 steps, NR-45 summary tint |
| L234, L268 + new | NR-46 groups |
| near L221, ≤719 block | NR-08 back/close toggles |
| ≤520 block | NR-07 header padding |
| new | NR-01/02/03/04 picker rules (placed after L249 so they win over L239–L246) |

Must keep: L105, L108, L129, L135, L170 hover/focus-within reveals; L286 coarse override; L287–L291 reduced motion; L292–L299 short-viewport rules; all `:focus-visible` behavior.

**Acceptance (static):**
- All new selectors stay within the Notes class namespace and match only `.notes-view` descendants. No global `@property` registration.
- No `!important` is added beyond the NR-08 pair, which mirrors the existing L221/L264 pattern.
- No `:has()` is nested inside `:has()`.

### 5.10 M2 Integration and tests (after S1–S4 and M1 land)

1. **Diff review**, per file `git -C $REPO diff -- <file>`, against §5.4–§5.8:
   - Only listed hunks.
   - No changed handler, disabled, ref, id or role.
   - The frozen DOM orders hold.
   - NR-43 follows B4/explicit Q4 resolution; an unresolved item is not final acceptance.
2. **Tests** (`NotesView.behavior.test.tsx` only; `notesState.test.ts` untouched):
   - T1: lines 144 and 280: `await click(button("Todos"))` → `await click(query<HTMLButtonElement>("#f-todos"))`. This removes the coupling to the NR-06 name and pins no new wording.
   - T2: add one test, "keeps Attach disabled and never attaches when the selected catalog entry disappears":
     1. `resolveTarget` rejects `notes_unbound`.
     2. `readCatalog` resolves once with entry E, then with `entries: []`.
     3. Attach existing → select E → Cancel → Attach existing.
     4. Expect `.notes-catalog-missing` to exist and `button("Attach").disabled === true`.
     5. Expect no call with `operation.op === "target_attach"`.
   - No other test edits. No style assertions in jsdom.
3. **NR-49:** only if the parent cleared Q1, add the single rule from COVERAGE row 49 after `src/app/styles.css:1651`. Do not touch `supervisor.css` (Supervisor-owned) or `App.tsx`.

### 5.11 P4 Final static checks (Main; once)

1. Build snapshot `final` from recorded source plus agreed component/CSS/test files, adding only the cleared NR-49 rule to the snapshot's shared stylesheet. Record source and overlay identities as in B1. `cd "$SNAP" && bun run build` covers typecheck/build once; do not build twice through the runtime kit.
2. `cd "$SNAP" && bun run test -- src/app/notes` runs the two Notes suites against the same integrated source. Both must pass. Do not execute a broad shared-checkout verification storm.
3. Review only this run's owned edits against the agreed source snapshots: eight component files (seven expected changed; TodoTitle reserved unchanged), `notes.css`, behavior test, plus cleared `styles.css`/`CODE_GUIDE.md`. Preserve unrelated existing edits. No broad Git status, widget commit file-set audit or timestamp proof.

No lint or formatter script exists in `package.json`, so none is run.

### 5.12 P5 Final runtime (Main; snapshot `final`, new fixture)

Kit: §5.A. Order matters, because some steps change bindings.

| Step | G / extra | Width(s) | Class | Procedure → pass |
|---|---|---|---|---|
| 1 | Identity + B2 widths | all | — | As in B1/B2. Pass: the loaded script equals the final asset. |
| 2 | G3 + missing empty-catalog capture | 1440 | R + X root setup | Before Create, S1 Attach → genuine notes_not_found alert, code separate/Retry adjacent/Attach disabled; Retry still errors; Cancel restores opener. Then fixture-only create empty Notes root directory (not UI create/binding), reopen Attach → real successful empty catalog with disabled Attach, capture separately, Cancel. CLI catalog entries []; opening writes no content/binding. |
| 3 | Create + seed | 1440 | R | Create notes in S1 → read the UUID from Agent access "Notes UUID" and the folder `== $NOTES_ROOT/$UUID_A` (proves the isolated root). Seed (§5.A.3). Hash `H_seed`. |
| 4 | G1 + NR-03 order | 1440, 900, 420 | R | S3 (unbound) → Attach existing. Pass: helper, catalog and actions share one column (center x within 1px, no two in one row). Focus sequence Attach→Cancel. Visual Cancel left of Attach. Cancel. |
| 5 | G18 | 1440 | R | Snapshot tab names = `Todos — {open} open`, etc., with values equal to CLI-derived counts. Arrow/Home/End still rove. |
| 6 | G4, NR-10 | 1440, 900, 420 | R | Pass: label, first `.cm-line` and Save row lefts within 1px. Source/Preview labels at ≥521, icons only at ≤520. Source→Preview→Source keeps caret and scroll. |
| 7 | G5 (NR-12/13) | 1440 | X+R | Type a draft, then append a line to `scratchpad.md` externally → conflict. Pass: preview left = alert text left, 14px. Keep and Reload computed styles are identical. Consequence line is shown. Reload: no hash change. Repeat the conflict, then Keep mine: `scratchpad.md` bytes == draft, exactly one `scratchpad_replace`. |
| 8 | G7, NR-15 | 1440 | R | Mouse parked: chip opacity .65; B/Do/Dn unambiguously map to unchanged full-lane titles, including checked Done; counts aligned across rows. Chip click → Kanban card title focused. |
| 9 | G6 (NR-16) | 1440, 420 | X+R | Hand-written task: Adopt label (hidden at ≤520) and title. Click → detail opens. Diff: only that line gains `id=`. |
| 10 | NR-21 | 1440, 420 | R | Guides: one per ancestor level, compared with B3. Depths unchanged. |
| 11 | G9, NR-23/24/26 | 1440 | R + X read refusal | T1 Delete trash / wide Close ✕; confirm→Keep restores Delete focus, no deletion. Header 12px, disclosure below body, unverified suffix, empty error row 0px. Open plain T3 for centered empty thread, return T1. Freeze valid fixture T1 comment file, write invalid UTF-8 byte only there, wait existing 3s poll for error/Retry in-flow; restore exact bytes then Retry → successful read/hash identical (E28). No DTO fault or user content. |
| 12 | NR-25, NR-27 states | 1440, 420 | R/X | Board row layout. Move → lane readback; unboard → retained comments and styled `.is-unboarded`. Use a seeded long thread to scroll away from bottom, then fixture CLI comment add → new-below pill (no overlay). Malformed marker line → source issue; dirty comment then fixture CLI remove → retained edit/discard; external todo-line removal with detail open → removed band. Loading stays best-effort, not fabricated R proof. Restore T1 membership with the existing Add to board before G11/G16; restore all fixture source alterations or reseed disposable data with fresh IDs/revisions. |
| 13 | G10 (NR-08) | 420 (+900 if ≤719) | R | Narrow Back visible, detail ✕ hidden. T1 Delete confirm → Escape cancels delete; Edit + unsaved text → Escape parks edit with draft retained/focus Edit; next Escape closes detail/returns opener. Reopen → Back returns opener. Explicitly Discard parked comment draft after capture; no saved-file changes. |
| 14 | G11 (NR-29/30/33) | 1440 | R | T5 adopted no-count board card: height < B6, no lower row, glyphs differ, actual Tab/Shift-Tab order unchanged, menu expands below. Whitespace opens detail; after close park mouse/focus outside card → selected handle .4; hover/focus full, coarse override retained. Pointer/text/grab cursors appropriate. Space→Right→Space changes only T5 lane; Space→Right→Escape bytes unchanged. Restore original lane with fresh pinned revision. |
| 15 | G12, G13, NR-31/34/35 | 1440 / 900 | R | Three plus names = "Add a card to Backlog". Doing plus → focus `#kanbanAddInput`. Chips have a border; Jump scrolls. Right-edge fade (end of scroll unfaded). Live status 12px. |
| 16 | G14, G15, NR-41, NR-42 | 1440, 420 | R | "Sort: Newest" with stateful name, which flips. Select D2 (Current) → History → context line, selection unchanged. "History (replaced)". Decided date on D1. Summary "Details & replacement". Replace disabled on D1. D1 hash unchanged. New and edit forms captured, discard tint on hover. 420 list view captured; Back keeps the record and draft. |
| 17 | NR-43 known error | 1440, 420 | R | Repeat B4 with the real invalid decided-date error. Required pass for landed remedy: hit-test Save/Comment controls unobscured, slot follows panels with no overlap, empty slot 0px after clear. Compare focus and scroller positions before/after. If pending Q4, record unresolved rather than a pass and escalate; no full-scope success yet. |
| 18 | G16 (NR-43/44, I-9) | 1440, 420 | F | §5.A.4: (a) the fault, (b) read fails, (c) read succeeds, (d) remount, (e) explicit ack. Pass: see §5.A.4. |
| 19 | G17 (NR-45) | 1440 | X | Type T3 title draft, freeze todos.md bytes, then remove only T3 externally. Summary singular/warning/collapsed, retained textarea selectable, Discard removes draft. Hashes show only external removal. Restore frozen todo bytes after this probe with no active drafts, refresh/re-read revisions, and verify restoration before G19 uses T3. |
| 20 | NR-46 | 1440, 420 | R | Expand Agent access: resolve row + five groups, 23 entries in original order. Compare command labels/values/order to a pre-implementation expanded-source/runtime capture with the same pinned selections, not B6's focus sequences. Copy and verify existing success/failure status; readonly inputs and UUID/root remain pinned. |
| 21 | G19 (NR-47/48) | 1440, 420 | R | Text-size script (§5.A.5): no functional text under 12px. `scrollWidth == innerWidth` at 420. `.notes-check` ≥24×24. Click inside the label but outside the box toggles T3 (readback), click again restores. |
| 22 | NR-07 | 420 | R | Header padding 8px, two rows, Close/Escape unchanged. |
| 23 | G8 (NR-17/18) | 1440 | R | Select already-created S2 (do not create a duplicate). Explicit Create → UUID-B → pinned CLI adds/completes two tasks. Snapshot hashes after seed. Hide completed toggles state/name/zero-result text without writes; compare before/after toggle hashes. |
| 24 | NR-49 | 1440 | R | After Q1 coordination, pressed selection fill when open, none when closed, toggle unchanged. Existing Supervisor selection styling remains intact (source and runtime comparison). Confirmed unresolved NR-49 blocks full-scope acceptance; a parent-authorized scope change must be explicit. |
| 25 | G2 (NR-02/03), last | 1440 | R | S3 → Attach existing → select UUID-B ("Attached elsewhere") → Attach. Pass: the group sits directly under the card at the same width, focus is on Keep, and the text is byte-equal to source. Keep → focus on Attach. Attach → Attach here → S3 bound to UUID-B; S2 shows "No notes for this Space". UUID-B content hashes unchanged. |

Every step also checks a visible focus ring (computed `outline-style:solid` on the focused control) and, unless the step states an expected change, unchanged hashes (§5.A.2).

### 5.13 P6 Docs, R2 barrier, cleanup

- **Docs:** after Q2 clearance add after `CODE_GUIDE.md:104`: "`notes.css` owns Notes presentation; component polish preserves existing handlers, focus, drafts and durable operations." If NR-43 lands, append: "The shared problem slot is in-flow below panels and zero height when empty." No keyboard/changelog edits.
- **Evidence:** `evidence/VERIFICATION.md` records source/overlay/asset/executable identities, observed steps, byte expectations and limitations. Keep R (real HTTP+CLI/store), X (fixture external edits) and F (controlled reply faults) separate. No staged or confirmed-but-unfixed item is marked passed.
- **R2:** parent reviews the full-scope evidence for acceptance. No repository commit/push option. Missing NR-43/49 resolution remains a parent decision, not success.
- **Cleanup:** close the owned SDK browser/automation context; stop only this fixture via the helper; copy evidence out before deleting the exactly returned `/tmp/cpol-*` root. Remove only this run's `/tmp/cnotes-*` snapshot and its own temporary fault drivers. If the host is symlinked, unlink it; never delete its source/shared target or node_modules. Compare user registry bytes/hash to P0. If R0/Q3 has not released runtime, none of these execution actions occurs.

### 5.A Runtime kit (used by P1 and P5; nothing here is run in this planning stage)

**5.A.1 Isolated snapshot and fixture.**

```sh
REPO=/home/nnex/dev/prj/cockpit; PHASE=baseline   # or final; only after R0/Q3 release
SNAP=$(mktemp -d /tmp/cnotes-$PHASE-XXXXXX)
git -C "$REPO" archive --format=tar HEAD | tar -x -C "$SNAP"
cp -p "$REPO"/src/app/notes/* "$SNAP/src/app/notes/"
ln -s "$REPO/node_modules" "$SNAP/node_modules"   # reuse only; no install
mkdir -p "$SNAP/target/debug"
ln -s "$REPO/target/debug/cockpit" "$SNAP/target/debug/cockpit"  # only if this candidate exists
# Candidate CLI syntax: run notes --help and required subcommand help, record actual results.
# If candidate absent/incompatible, remove only this symlink and build in isolation:
# (cd "$SNAP" && CARGO_TARGET_DIR="$SNAP/target" cargo build -p cockpit-host --bin cockpit)
# Build once per phase here, OR P4 for final, never both:
(cd "$SNAP" && bun run build)
# Apply only Q3-cleared snapshot-local omission of helper's synthetic commit; keep repository helper untouched.
FIX=$(python3 "$SNAP/scripts/verify/ui_polish_runtime.py" start | tail -1)  # only after Q3
# Read gateway.log using the available file tool; take its loopback listening URL.
# Open that URL with the existing SDK browser at 1440x1000 (no browser CLI required).
```

- Before UI mutation, run fixture Notes catalog and read operations through the actual CLI/HTTP gateway and verify current result kinds/identity; inspect logs for protocol/schema failures. Help text, hash or mtime alone is not runtime compatibility. On a stale candidate, stop only the owned failed fixture, build the host with isolated `CARGO_TARGET_DIR` and relaunch that disposable fixture path; do not weaken the contract or use `--test-mode`. Record this recovery. No installs; missing offline dependencies escalate.
- Root isolation: helper HOME/XDG/TOML imply `$FIX/data/cockpit/notes` (E14/E15). Prove it after explicit Create by UI Folder plus actual UUID folder/CLI readback. Config overlap validation is lexical (`config.rs:330`), not permission to trust an arbitrary/symlinked root. Never use user Notes.
- The repository helper is never edited. Run its physically copied snapshot entry points start/rpc/stop only against their printed root; the sole permitted divergence is Q3's explicitly cleared omission of the synthetic fixture commit. Preserve all isolation/process identity guards and record the exact difference before launch.
- `resource_guard.py` is not part of this path (E17).
- Extra Spaces in both phases: use helper `rpc "$FIX" workspace.create` with `{"cwd":"<actual FIX>/repositories/sample","label":"Polish S2","focus":false}` and S3 likewise (`scripts/verify/ui_polish_runtime.py:152`). Record actual returned IDs; S1 denotes the helper's initial Space. In baseline, explicitly Create UUID-A in S1 and seed before B3/B4; create no user bindings. For B5 also capture existing Supervisor pressed/unpressed appearance before changes.

**5.A.2 Pinned CLI readback and hashes.**

```sh
NOTES_ROOT=$FIX/data/cockpit/notes
cli() { env -i PATH="$PATH" HOME="$FIX" XDG_CONFIG_HOME="$FIX/config" XDG_STATE_HOME="$FIX/state" \
  XDG_CACHE_HOME="$FIX/cache" XDG_DATA_HOME="$FIX/data" COCKPIT_CONFIG="$FIX/cockpit.toml" \
  COCKPIT_NOTES_ROOT="$NOTES_ROOT" "$SNAP/target/debug/cockpit" notes --notes "$UUID" "$@"; }
hashes() { (cd "$NOTES_ROOT/$UUID" && find . -type f -print0 | sort -z | xargs -0 sha256sum); }
```

- The UUID always comes from the UI (Agent access "Notes UUID"), never from a guess.
- Before and after every step: `hashes > $FIX/evidence/H-<step>-{pre,post}.txt`, then `diff`.
- Structured readback goes through `cli todo list`, `kanban list`, `decision list --status all` and `comment list --todo ID`, with JSON filtered by `python3 -c` to ids/revisions/lanes/depth/counts.

**5.A.3 Seed (synthetic, fixture-only; the same for baseline and final).**

- **Scratchpad:** `cli scratchpad read` → revision → `cli scratchpad replace --file $FIX/evidence/seed/scratch.md --expected-revision REV` (120 lines, for scroll checks).
- **T1:** `cli kanban add --text 'Adopted board task'` → `cli comment add --todo T1 --text 'Seed comment' --author 'Fixture'`.
- **T2:** `cli kanban add --text 'Doing task'` → `cli kanban move --id T2 --expected-revision R --to doing`.
- **T3:** `cli todo add --text 'Plain todo'`.
- **T4:** `cli todo add --text 'Completed task'` → `cli todo complete --id T4 --expected-revision R`.
- **T5:** `cli kanban add --text 'One-line no-comment board task'`; use this adopted, count-free board card for B6/G11. T3 remains the plain todo used for retained-draft and checkbox probes. Seed enough synthetic T1 comments to overflow the thread for NR-27; record its actual starting count for G16.
- **External append to `todos.md`** (fixture-only, Python byte append, newline-safe): `- [ ] Hand-written parent` / `  - [ ] Nested child one` / `    - [ ] Nested child two` / `- [ ] Hand-written unadopted task with a deliberately long title that wraps across two lines in a narrow lane`. Record the depths reported by `todo list`; the expected depths 0/1/2 are [INFERENCE] until B3 records them.
- **Decisions:**
  - D1: `cli decision create --title 'Keep Markdown files' --file d1.md --decided 2026-10-01`.
  - D2: `cli decision replace --id D1 --expected-revision R --title 'Keep Markdown files, revised' --file d2.md`.
  - D3: `cli decision create --title 'Short decision' --text 'One line.'`.

**5.A.4 Controlled fault (class F; G16).** Use the existing SDK browser/Playwright-compatible page routing (optional already-available CLI is equivalent, never required or installed). Precondition: S1, restored T1 detail open, composer "fault probe", Todos add "next task draft", H_pre and actual comment count c0.

```js
// Execute through an existing SDK browser page with route/fetch/fulfill support.
async page => {
  const ops = []; let faulted = false;
  await page.route('**/api/v1/notes', async route => {
    const body = route.request().postDataJSON(); ops.push(body.operation.op);
    if (body.operation.op === 'comment_add' && !faulted) {
      faulted = true;
      const real = await route.fetch();                        // write reaches the real gateway
      if (real.status() !== 200) throw new Error('Real gateway write did not succeed');
      const saved = await real.json();
      if (!saved.changed || saved.result?.kind !== 'comment') throw new Error('No confirmed real comment write');
      ops.push('forwarded:' + real.status());                   // separately confirm CLI count == c0+1
      return route.fulfill({ status: 200, contentType: 'application/json',   // wrong-kind body (client.test.ts:1122-1126)
        body: JSON.stringify({ notes_id: body.target.notes_id, changed: false,
          result: { kind: 'scratchpad', document: { content: '', revision: 'absent' } } }) });
    }
    return route.continue();
  });
  await page.click('#postCommentBtn');
  await page.waitForSelector('.notes-error code:text("notes_outcome_unknown")');
  await page.unroute('**/api/v1/notes');
  return ops;
}
```

Pass conditions:
- **(a) Fault.**
  - `ops` contains exactly one `comment_add` and one `forwarded:200`.
  - The alert is `.notes-unknown` with two li, in-flow after panels; hit-test the guarded Comment control (or label descendant), Check and later ack. Pending NR-43 is unresolved G16, never a pass.
  - `#postCommentBtn` and `#todoAddBtn` are disabled. The composer and add drafts are retained.
  - CLI count = `c0+1`.
  - Capture thread scrollTop and caret/selection before dispatch and after alert settles; no unintended focus steal to the alert. Because a disabled button may lose focus under existing behavior, compare baseline existing focus semantics rather than require an impossible identical focused disabled control. Scroll anchoring remains within 1px.
- **(b) Read fails.**
  - Through the same SDK page, abort exactly the first saved-state `comment_list`, click Check, then remove only this run's route. Clearly label this failed-read setup F; it is not real network-loss evidence.
  - Capture the failed-read response/error evidence. The unknown alert retains its existing presentation; do not assert incidental visible copy. There is no "I checked saved state; allow next write" button; writes stay disabled, unknown guard and both drafts remain retained, and the real comment count stays `c0+1` with no automatic replay. Parent Resolution A corrects the former contradictory visible-message expectation; NR-44's frozen rendering remains unchanged.
- **(c) Read succeeds.**
  - Click "Check saved state". The third `li` appears with the review sentence and an enabled ack.
  - Writes stay disabled until the ack. CLI count is still `c0+1`.
- **(d) Remount.**
  - Close Notes (header ✕) and reopen it from the top-bar Notes.
  - The alert persists, there is no ack until a new successful check, and writes are disabled.
  - Check, then the ack appears.
- **(e) Explicit ack.**
  - Click the ack. Writes are enabled, and the composer draft is still present (not auto-posted).
  - Keep a request observer alive for all (a)–(e), including across remount, while removing only the fault route. It must record total `comment_add` dispatches = 1; CLI count remains c0+1. Do not infer dispatch count from the one-shot route's returned ops alone.
  - Discard the composer draft. `H_post` differs from `H_pre` only by the one comment file.

Steps (a)–(e) also run at 420 for layout only. That run uses a fresh fault, and its +1 comment is recorded.

**5.A.5 Probes.**

- **Focus sequence:** collect visible enabled controls within each scoped region, excluding hidden/inert ancestors and `tabIndex<0` (except summaries with native tabindex); record stable role/name or existing ID, not only repeated class strings. Compare actual forward/Shift-Tab traversal as well; selector order alone is not browser tab order.
- **Text size (G19):** walk visible text nodes under `.notes-view`. Skip only decorative keep-list text `.notes-tab-count,.notes-count,.notes-comment-count,.kanban-chip span,.notes-error code,.notes-editor-toolbar>span,.notes-agent-summary-meta`. Do **not** exempt the whole `.kanban-card-comments` button: its new Adopt label is functional and must compute ≥12px. M1 sets that label to `--font-size-control`; counts remain 11px. Report any other functional text <12px.
- **Occlusion:** `document.elementFromPoint(cx, cy)` with `el === control || control.contains(el)`.
- **Container:** the `.notes-view` `getBoundingClientRect().width`.

## 6. Risks and verification

| Risk | Where | Mitigation / proof |
|---|---|---|
| Behavior drift (handlers, disabled gates, CAS, adoption, unknown guard) | all slices | §5.4 prohibitions; M2 hunk review; both Notes suites pass (P4); byte expectations per step (§5.12); G16 a–e (I-9) |
| Focus-order regressions | NR-03, 08, 23, 25, 29, 43, 44, 46 | Frozen orders (§5.4); B6 versus final sequences; G10 Escape chain; existing focus tests unchanged except the selector |
| Hover/focus reveal or coarse-pointer loss | NR-15, 16, 29, 33 | M1 keep-list; computed opacity checks with mouse parked and under `:focus-within` |
| Empty in-flow slot reintroduces a band (E3) | NR-43 | Step 17: empty slot height 0 |
| Data loss and fixture safety | runtime | Only owned fixture roots/returned UUIDs. Prove Notes root via UI Folder+CLI. External edits only there. User registry exact bytes/hash unchanged; helper stop process identity guard (E14). Q3 resolved before helper start. |
| Concurrent Supervisor run | build, shared files | Physical snapshot helper, isolated dist/target, scoped overlay; no shared-output writes. Shared styles/docs only after Q1/Q2 coordination. Review only owned hunks, preserve other changes. Widget anchor untouched; no broad Git audit as proof. |
| Stale gateway/protocol mismatch | runtime | Actual CLI/help plus fixture HTTP/CLI result/identity validation before mutations; isolated host rebuild if incompatible. Record executable/source/asset provenance, never timestamp-derived freshness. |
| Decoration mistaken for evidence | evidence | R, X and F sections kept separate; S-only items marked; no re-capture used as proof of other states (DESIGN §9 gaps) |
| Unsupported CSS in some engines | `:has()`, `mask-image`, existing container units | Chromium proof only; native WebKitGTK untested. NR-09 reuses cqw and unregistered literal lengths, removing the avoidable `@property` dependency. |

## 7. Limitations (stated up front)

- **Browser only.** Chromium evidence uses the existing SDK browser or already-available equivalent tooling. No mandatory playwright-cli, install, native Tauri/WebKitGTK, screen-reader, touch hardware or forced-colors pass.
- **Unknown outcome comes from a controlled fault.** It is produced by a forward-then-corrupt route (class F), not by a real network loss. The write itself is real.
- **NR-05 deterministic guard evidence is T**, not a claimed real runtime failure: the real catalog enumerates directories (E13). The new test covers vanished selection without manipulating user content.
- **Some states are hard to capture.** `.notes-thread-loading` (NR-27) and "Saving Notes…" (NR-11) are transient, so their capture is best effort.
- **Visual and focus order differ in NR-03.** The visual order (Cancel, Attach) intentionally differs from DOM/focus order (Attach, Cancel), per DESIGN.
- **NR-06 renames the tab panels.** Panel names follow the new tab names (E8).
- **The 900 viewport band is unconfirmed.** It probably exercises the 521–719 container band; the 720–899 band may need the extra band-check viewport (B2).
- **No automatic scope reduction.** NR-43 nonreproduction escalates Q4; confirmed NR-49 requires Q1 coordination. Both stay open until parent resolution and required acceptance. Only an explicit parent scope change can remove them; there is no confirmed-unfixed success fallback.
