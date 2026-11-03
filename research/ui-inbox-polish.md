# Inbox polish: design spec

Status: design only. No product code, tests or config were changed. Two interactive mocks and one shared renderer sit beside this file in `research/ui-inbox-polish/`. I could not render them (no browser or shell in this assignment); every layout number below is derived from the CSS and from hand-computed contrast, not from screenshots. Main's disposable baseline screenshot (desktop empty: largely blank detail pane, composer anchored at the bottom) is the only rendered evidence and is consistent with the code reading in F1 below.

## 1. Goal & users

Goal: make the Inbox screen read as one product with Workbench (tokens, density, selection language, copy) and fix its visible defects, without changing what any control does.

Users and tasks:
- Human operator triaging work that needs them: scan, pick an item, answer a question or review a result, optionally follow up. Mostly desktop, some phone (the app already ships a 700px phone layout, `inbox.css:158-167`).
- Same operator starting new work with a liaison ("New work").
- Same operator messaging an unmanaged ("direct") agent that has an unseen completion.

Non-goals (per the delivery rules and research direction): no new feature, filter, bulk action, manual-refresh control, notification, illustration, celebratory empty state, or behavior change in selection, acknowledgement, drafts, delivery, retry or identity fences.

## 2. Evidence (existing patterns reused)

Behavior that MUST be preserved (contracts):
- Selecting a direct-agent row acknowledges the captured completion only for a fresh, matching target; selecting never focuses the terminal; "Open session" is a separate action (`Inbox.tsx:155-186`, `docs/supervisor-surfaces.md:16`).
- Nothing auto-selects; mounting does not focus the composer (`Inbox.test.tsx:161-173`). Drafts are per selection key and survive session/screen switches (`Inbox.tsx:41-46,151-153`).
- Enter sends, Shift+Enter and IME composition do not (`Inbox.tsx:302`, test `:183`).
- A question is chosen explicitly; a choice fills the draft and enters Answer mode but never sends (`Inbox.tsx:284`, `docs/verification-log.md:31`).
- Failed or uncertain delivery keeps the draft and offers "Retry original request" with the same identity (`Inbox.tsx:198,303`).
- Mark read is explicit; opening a thread never marks it read (`docs/supervisor-surfaces.md:15`).

Selectors and strings the existing tests and docs depend on (keep, or change tests deliberately): `.inbox`, `.inbox[data-detail]`, `.inbox-list`, `.inbox-row` (+ `.is-quiet`), `.inbox-meta` absent on the quiet-agent fallback row, `.inbox-row-next` text `Space · idle`, `.inbox-others`, `.inbox-detail`, `.inbox-content`, `.inbox-head`, `.inbox-question .inbox-q-head[aria-pressed]`, `[aria-label="Session context"]`, a button whose text is exactly `Open session`, the form `.inbox-compose`, `Direct agent · idle` / `status unavailable` in `.inbox-head`. Hazard: tests find the first `button` whose text includes capital `Work` (`Inbox.test.tsx:203,217,240,256`); do not add a button before the thread rows whose label contains "Work".

Existing product patterns reused by both alternatives:
- Tokens: `src/app/styles/tokens.css` (`--sidebar-bg`, `--chrome-bg`, `--surface*`, `--select-fill`, `--select-edge(-size)`, `--accent`, `--warning`, `--blocked`, `--done`, `--idle`, `--tag-1`, `--font-size-*`, `--radius-*`, `--control-height`). Selection language: fill + 2px accent edge on the content-facing side (`tokens.css:15-17`), already used by `.inbox-row[aria-current=true]` (`inbox.css:31`).
- Focus: 2px `--focus-strong` inset ring (`inbox.css:10`; direction `ui-design-direction.md:189`).
- Sidebar section headings: lowercase 13px/500 `--text-secondary` with an 11px muted count (`research/ui-agent-attention-queue/states.html:41-44`, `Agents.tsx:73`, screenshot of the live sidebar: "spaces", "agents").
- State disc: tinted disc + one distinct shape, state never colour-only (`StateGlyph.tsx:10-27`, direction `:114`).
- View bar: 32px chrome with the accent top edge, count pill (`shell.css:56-123`, `Workbench.tsx:210-214`).
- Detail "Back" with focus handoff: Supervisor detail focuses the back control on open and restores focus to the originating row on Back/Escape (`SupervisorTasks.tsx:78`, `SupervisorView.test.tsx:46,95,189`).
- `aria-current="true" : undefined` convention for selected rows (`Agents.tsx:65`, `Spaces.tsx:393`, `LibraryTree.tsx:211`, `SupervisorView.tsx:131`).
- Empty/loading/error rules: compact local empty, no inbox-zero celebration, no illustrations (`ui-design-direction.md:217-220`); skeleton only for initial load, last-known content preserved on resync (`:201-208`); errors attach to the failed resource and say what remains safe (`:230-240`); never "Something went wrong".

Findings in the current Inbox (each with its evidence; F-numbers are referenced below):
- F1 Empty/New-work detail is a blank pane. `Inbox.tsx:291` renders an unstyled `<h2>` + `<p>` in `.inbox-content`, which is `flex:1 1 auto` (`inbox.css:58`) above a dock pinned to the bottom (`:116`). Nothing fills the middle. Confirmed by Main's screenshot.
- F2 Content and dock misalign on wide windows: content is `max-width:700px` left-aligned (`inbox.css:58`) while the dock and textarea span the whole detail width (`:116,149`). At 1440px wide the list is 420px, the detail 1020px, so 320px are dead on the right of the content and the textarea is 996px.
- F3 Row action pill duplicates the row and mislabels agents. `.inbox-act` is `aria-hidden` (`Inbox.tsx:317`) and consumes about 77px (padding 11+4, "Answer" at 13px/600, 14px icon, border). At the 320px minimum the text column is about 163px (`320 - 14 - 12 - 30 - 2*12 - 77`). Agent rows print "Open ›" (`:312`) but a click selects the detail, it does not open the session.
- F4 State is not visible text in most rows: when `thread.project` exists, the state word is `inbox-sr` only (`Inbox.tsx:317`), so sighted users get icon shape and colour only. Titles are single-line ellipsis (`inbox.css:36`) although they are sentences.
- F5 The first list group has no heading, later groups use an 11px uppercase `h2` (`inbox.css:49`), unlike the lowercase sidebar headings. "Other agents" has no count, "Other work" has one (`Inbox.tsx:277-278`).
- F6 List header scrolls away with the list (`overflow:auto` on the aside, header not sticky, `inbox.css:24-25`), so New work disappears in a long list. Its 18px `h1 Inbox` sits under an active "Inbox" tab and is 57px tall versus the 32px bar.
- F7 Typography bypasses tokens: 13 distinct pixel sizes (11, 11.5, 12, 12.5, 13, 13.5, 14, 14.5, 15, 16, 17, 18, 20) and four literal `#0b0e12` (= `--app-bg`) at `inbox.css:72,80,124,151`, plus a literal shadow `#0008` at `:142`. Root size is 15px while the tokens provide 11/12/13/14/15/16/19/20/23.
- F8 Loading is one text line (`Inbox.tsx:270`) while the detail already shows an enabled-looking New work form whose Send is disabled with no explanation (`:303`). First-load failure prints the raw error in the same unstyled line, with no statement that it retries every 1.5s (`:82,115`). With data present, the failure message is only in a `title` tooltip (`:271`), unavailable on touch and to many AT users.
- F9 Mobile: the focused row/button disappears when the list or detail is `display:none`, and nothing moves focus (`Inbox.tsx:154,158,280`; compare the Supervisor precedent). Back is a text glyph "‹ Inbox", 40px tall (`inbox.css:57`), while `UiIcon` has `back`.
- F10 `aria-current={selected}` emits `aria-current="false"` on every unselected row (`Inbox.tsx:311-313,317`), unlike every other list in the app.
- F11 Choices are a 2-column grid of centred chips (`inbox.css:77-78`). A long choice wraps centred in a ~306px cell at the 700px measure. "Recommended" is an icon plus screen-reader-only text (`Inbox.tsx:320`), so sighted users see an unexplained sparkle. Pressed state is a fill inversion only.
- F12 Question rendered as a bordered 6px-radius card (`inbox.css:68`), against the direction's "no card per resource" and the 4px control radius (`ui-design-direction.md:77,288`).
- F13 Textarea auto-grow is capped at 96px in JS (`Inbox.tsx:302`), which is right for a dock but too small for the New work request.

Contrast (WCAG relative luminance, computed by hand from the token hex values; no tool run):
- `--text-muted` #a6adc8 on `--sidebar-bg` #11161e: 8.2:1; on `--surface-selected` #23354c: 5.6:1; on `--surface-hover` #222e3d: 6.2:1.
- `--accent` #89b4fa and `--app-bg` #0b0e12 (chip/send text on accent): 9.2:1.
- `--warning` #f9e2af on `--sidebar-bg`: 14.3:1; on selected: 9.8:1. `--blocked` #f38ba8 on selected: 5.4:1.
- `.inbox-seg button[aria-disabled=true]` at `opacity:.4`: about 2.3:1. WCAG exempts inactive controls; the reason is also given by the visible note "Pick a question above to answer it." (`Inbox.tsx:299`), so it passes as a state, not as a text-contrast claim.

## 3. Flow

```mermaid
flowchart TD
  A[Open Inbox from view bar] --> L{inbox view loaded?}
  L -- no, first load --> S[List: skeleton + "Loading inbox…"; detail: New work, Send disabled with reason]
  S -- load fails --> E1[List: "Inbox unavailable. {message}" + "Retrying automatically."]
  S -- loads --> P
  E1 -- retry succeeds --> P
  L -- yes --> P{anything needs you?}
  P -- no --> Z["Nothing needs you right now." + Other work / Other agents disclosures; detail = New work"]
  P -- yes --> G["needs you · N" group; no auto-selection; detail = New work"]
  G --> R[Select row]
  R -- thread with question --> Q[Select question / pick option -> draft filled, Answer mode]
  Q --> SEND[Send explicit: Answer]
  R -- result ready --> V[Review; Mark read explicit; Follow up]
  R -- direct agent --> D[Detail + Open session; Send direct only if agent ready]
  Z --> N[New work: choose owner, optional Context, Start]
  G --> N
  SEND -- failure --> F[Alert note; draft kept; Retry original request]
```

Mobile (≤700px): list and detail are separate screens. Row or New work opens detail and moves focus to Back; Back returns to the list and moves focus to the originating row (or New work if there was none).

## 4. Screens/components

Both alternatives keep one markup skeleton (`aside.inbox-list` + `main.inbox-detail`) and the class names above. The mocks (`research/ui-inbox-polish/a-ledger.html`, `b-queue.html`) render nine frames each: desktop populated+selected question, direct agent selected, empty, loading, first-load error, degraded; mobile list, detail (Answer mode) and first-load error. Open them directly in a browser; rows, options, Answer/Follow up, New work and Back are interactive. Sample copy is invented for the mock only (shapes follow `InboxThread`, `InboxQuestion`, `UnmanagedAgent` in `src/protocol/generated/v1.ts:1041-1097`).

### 4.1 Alternative A "Ledger" (evolution)
Keep: 28px state discs, pill New work, bordered question card, 2-column chip grid, bottom dock, chip fill inversion.
Change: sticky 44px list header (`h1` 15px/600, "N need you" count, New work); lowercase group headings with counts; state word always printed in the meta line (coloured by `--kc`) before the project; no `inbox-act` pill; titles clamp to 2 lines; row min-height 72px; content and dock share a centred 720px measure through one custom property (`--gutter:max(24px,calc((100% - 720px)/2))`); New work is a centred empty-state block above the dock; chip grid becomes `repeat(auto-fit,minmax(min(100%,240px),1fr))`; typography moved to tokens; skeleton, error copy, Back 44px with focus handoff.

### 4.2 Alternative B "Queue" (Agents-sidebar vocabulary)
Change in addition to everything A fixes:
- List bar: 32px (`--control-height`) sticky, lowercase `inbox` heading + "N need you", 24px ghost New work button (4px radius). List width `clamp(288px,28%,340px)`.
- Rows: 20px disc (12px icon), min-height 56px, hairline separators, meta line `state word · project · time`, title 14px/500 clamped to 2 lines, preview 12px muted single line. Quiet rows 44px.
- Detail: left-aligned 720px measure shared by content and dock (`padding-right:max(24px,calc(100% - 744px))`); head disc 32px.
- Question: flat section with a top hairline (no card); question text 16px/500; choices are stacked full-width option rows with a leading radio mark (filled dot when pressed + accent edge + `--select-fill`); "Recommended" is a visible 11px tag with the spark icon. Pressed is distinguished by dot, edge and fill, not colour alone.
- New work: `:has(.inbox-compose[data-mode=new])` makes the content block `flex:0 0 auto` and removes the dock chrome, so the heading and a 96–160px request field sit at the top of the pane; the dock for other modes is unchanged.
- 4px radius for seg, send, textarea, buttons (direction: radius for buttons/notices).

### 4.3 States (both alternatives, normative unless marked)

Populated, desktop. List groups in this order: `needs you · N` (threads with `state=needs_you`, then relevant direct agents), `Other work · N` disclosure (Underway then Handled, quiet rows), `Other agents` disclosure (add `· N`). Row content: state word (see tone table), project, relative time; title; preview (`next` → first question → reply, unchanged `Inbox.tsx:317`). Direct agent row: `Direct agent · {workspace} · {tab}`, name, context activity/title else status (existing fallbacks, `Inbox.tsx:309-313`, test `:291-300`).

Tone table (unchanged logic, `Inbox.tsx:28-33`): decision `Needs your decision`/`Needs you` (warning, `help`), review `Result ready` (accent, `eye`), work `Working` (muted, `work`), done `Done` (done colour, `check-circle`), direct agent `Direct agent` (warning, `user`).

Selected. Row: `aria-current="true"` only when selected (attribute absent otherwise), fill `--select-fill` plus 2px `--select-edge` on the left. Detail header: disc + `h2` title + facts (project · time · state word · "AI summary" / "Worker claim" provenance). Selected question: radio mark filled, `aria-pressed="true"`; a chosen option: pressed. Dock `data-mode` colour: answer = warning, new = accent, direct = tag-3, follow-up = neutral (unchanged).

Empty (data loaded, nothing needs you and no relevant agent). List: `check-circle` in `--done` + "Nothing needs you right now." (existing copy and icon; no celebration, no illustration). The disclosures stay available. Detail: New work.
- A: centred block (`New work` head + "Send a request to a liaison.") above the bottom dock.
- B: top-aligned heading with the composer inline directly under it. Copy unchanged. Owner select and Context stay in the composer head.
- No session (`sessionId == null`): existing `Select a session to see your inbox.`; keep, centred in `.inbox-root` with muted `--font-size-sm` text.

Loading (first load, `view == null`). List `aria-busy="true"`; `role="status"` line "Loading inbox…" (existing copy) above four neutral skeleton rows (disc + two bars, `--surface-hover` fill). No shimmer, so nothing to disable under `prefers-reduced-motion`; if animation is added later it must be removed under that media query. Detail: New work, Send disabled (existing logic) plus a visible `inbox-note` "Sending is available once the inbox has loaded." Textarea remains typeable (existing). Never replace last-known content with a skeleton on resync (the loop only sets `view` on success).

Error.
- First load failed (`!view && loadError`): list shows an error note (red edge, `alert` icon): bold "Inbox unavailable." + the message text + muted "Retrying automatically." (the loop retries every 1.5s, `Inbox.tsx:82`). `role="status"` (text is stable between retries so it is not re-announced). No Retry button: there is no manual retry path today and adding one is out of scope (open question Q4).
- Data present, refresh failing (`view && loadError`): keep all content; note "Inbox unavailable. Showing the last loaded items." with the message as a visible second line (today it is in `title` only) and "Retrying automatically."
- `observation.status === "unavailable"`: unchanged note "Can’t see live sessions right now. Replies are paused." Put the error message in visible small text only if non-empty and not an internal code (Q5).
- `attention.availability === "unavailable"`: unchanged "Agent attention unavailable."
- Action errors (dock): `role="alert"` red-edge note, full text visible (remove `title={error}` as the only carrier; the text is already inline). Existing direct-send notes keep `role="status"`. Priority when several apply: error alert, then direct-delivery status, then "Pick a question above…" hint; never stack more than three notes (the dock must not exceed about 40% of a 720px-high phone).
- Pending retry: textarea disabled and the send slot becomes "Retry original request" (unchanged).

Mobile (≤700px, port `@container frame` as `@media (max-width:700px)`, same breakpoint as today).
- One screen at a time through `[data-detail]`. Sticky list header. Detail: Back 44px min height; A prints "‹ Inbox" as today, B uses the `back` icon + "Inbox"; both get `aria-label="Back to inbox"` (visible text "Inbox" is contained in the name). Gutters 16px. Choices: A 1 column; B always stacked. Dock gets `padding-bottom: calc(8px + env(safe-area-inset-bottom))` (exists). Dock controls (seg buttons, destination, pickers, Context) min-height 36px in B (A keeps 30px: Q6).
- New work on mobile is top-aligned in both (A's centred block is desktop only).

### 4.4 Markup deltas (needed by either alternative)
1. `ThreadRow`/`AgentRow`: remove the `.inbox-act` span; print the state word (and `Direct agent`) visibly in `.inbox-meta-t`; drop the sr-only ` · {state}` span; keep `.inbox-meta` absent on the quiet-agent fallback row.
2. List groups: wrap the needs-you rows in `<section aria-labelledby>` with an `h2.inbox-group` (`Needs you` + count); promote the existing `.inbox-others h2` to `.inbox-sub`; add the count to "Other agents".
3. `aria-current={selected ? "true" : undefined}` on rows.
4. List header: `h1` stays; add an optional `.inbox-count` ("N need you") when N > 0. The count equals the view-bar badge value (`Inbox.tsx:144`).
5. Loading/error blocks as in 4.3 (replace `Inbox.tsx:270-271`), and keep `title` only as a supplement.
6. New work detail: use `.inbox-head` structure (`plus` disc, `h2 New work`, fact "Send a request to a liaison.") with `.inbox-content.is-new`.
7. Back: `aria-label="Back to inbox"`; B swaps the glyph for `<UiIcon name="back" />`.
8. B only: choice markup `<span class="inbox-chip-t">text</span>` + visible `<span class="inbox-rec"><UiIcon name="spark" />Recommended</span>`; remove the sr-only "Recommended: ". B also raises the JS auto-grow cap for new-work mode from 96 to 160 (`Inbox.tsx:302`).
9. Optional focus handoff in `SessionInbox` (see section 5).

CSS port notes: replace literal sizes with `--font-size-*` (11→2xs, 12→control, 13→xs, 14→sm, 15→md, 16→base, 17/18/19→lg or xl as in the mocks, 20→xl); `#0b0e12`→`var(--app-bg)`; keep `:where(.inbox)` resets so component classes win without `!important`; keep the `:focus-visible` rule; keep `.inbox[hidden]`, `.inbox-root` height rules (`shell.css:127-128`).

## 5. Interaction & keyboard

Unchanged: Tab order is DOM order (view bar → New work → rows → disclosures → Back → detail controls → dock). Enter/Space activate rows and options. Enter in the textarea sends; Shift+Enter inserts a newline; IME composition is ignored. Details/summary toggle with Enter/Space. Hover uses `--surface-hover` with no transition longer than 80ms (the chevron rotation keeps its 120ms: reduce to 80ms or remove, Q7).

New (small, no new commands):
- Focus handoff on the phone layout only (when `.inbox-back` is displayed): opening a row or New work focuses Back; Back focuses the row that was open (or New work if the selection was New work). On desktop, selecting a row keeps focus on that row and does not move it to the detail or the composer (preserves test `:161`). Implementation hint: store the origin element in a ref when `select`/`selectAgent`/New work runs, and run the focus in an effect keyed on `mobileDetail`, guarded by `backRef.current.offsetParent !== null`.
- Pressing a pressed question header or option a second time keeps today's behavior (header toggles off; options re-select the same text).
- No roving tabindex or arrow-key list navigation is added (Q3).

Pointer/keyboard parity: every pointer action above has a button; no hover-only content (the tooltip-only error text is removed).

## 6. Accessibility

- Landmarks/headings: `section[aria-label=Inbox]` > `aside[aria-label="Work needing attention"]` (`h1 Inbox`, group `h2`s) and `main` (`h2` = selected title). Do not add a second `main` or move `h1`.
- Rows: native `button`, name = visible text (state, project, time, title, preview), `aria-current="true"` when selected only. State is announced as text (F4 fix), never colour alone; disc shape differs per state.
- Groups: `section[aria-labelledby]` + heading gives a count to AT; wrapping rows in `ul/li role=list` is recommended but not required and does not affect `.inbox-row` selectors (Q8).
- Live regions: loading and non-dock errors use `role="status"` with stable text; dock errors `role="alert"`; the "Pick a question above" hint keeps `aria-live="polite"`.
- Choices: group `role="group"` labelled "Choices from {owner}" (unchanged); each option `aria-pressed`; B's visible "Recommended" tag replaces the sr-only prefix (keep one, not both, to avoid double announcement).
- Focus: 2px inset ring on every interactive element (unchanged); focus is never moved on desktop; phone handoff per section 5. Skeleton is `aria-hidden`; the status line carries the semantics.
- Targets: rows ≥ 44px (quiet) / 56–72px; options and chips 44px; Back 44px; dock controls ≥ 30px desktop and 36px phone (WCAG 2.5.8 minimum is 24px).
- Contrast: see section 2; all text and state tokens used on `--sidebar-bg`, `--app-bg`, `--surface-selected` and `--surface-hover` are ≥ 4.5:1 for the colours in use (muted 5.6:1 on selected is the lowest). New or changed surfaces must re-check if a token changes.
- Reflow: `overflow-wrap:anywhere` on content stays; no horizontal scroll at 320px; clamp titles to 2 lines but keep the full title in the detail `h2` (not truncated).
- Motion: no new animation; chevron rotation only. Under `prefers-reduced-motion` the existing `transition:none` rule stays.
- Not verified here: screen reader output, forced-colors mode, 200% zoom, real touch behavior.

## 7. Options considered

Evaluation (1 = weak, 5 = strong; judged from the mock CSS and the code, unrendered):

| Criterion | Current | A Ledger | B Queue |
| --- | --- | --- | --- |
| Fixes F1 blank detail | 1 | 3 (centred block; dock still bottom) | 5 (composer at top) |
| Fixes F3/F4 row clutter, visible state | 1 | 4 | 5 |
| Scan density (rows per 640px list) | about 8 | about 7 (72px rows) | about 10 (56px rows) |
| Match to Agents/Spaces vocabulary (`ui-design-direction.md`) | 2 | 3 | 5 |
| Long choice text (F11) | 2 | 3 (grid collapses to 1 column) | 5 (stacked options) |
| Alignment of content/dock (F2) | 1 | 5 (centred) | 4 (left-aligned) |
| Touch comfort | 4 | 4 | 3 (rows 56px, dock 36px) unless Q6 |
| Implementation risk / diff size | n/a | low (CSS + 5 JSX edits) | medium (same JSX + choices markup + `:has` layout + JS cap) |
| Test impact | n/a | none expected | none expected for selectors in section 2; verify `includes("Work")` finds |

Alternatives rejected:
- Inline answer in the list row (choices as one-tap buttons on the row): faster, but sends or fills drafts without opening the thread and changes the explicit-choice contract. Rejected.
- Single-column feed with accordion expansion on all widths: removes the master-detail split but moves the composer into each item and breaks per-selection drafts and the Back flow. Rejected.
- Auto-select the first needs-you item to avoid the blank detail: harmless for threads but acknowledges completions for direct agents and focuses attention changes; contradicts "nothing auto-selects". Rejected.
- Keep the row action pill but relabel it: it duplicates the row click and is `aria-hidden`; removing it frees about 77px. Rejected.

Recommendation: B. It fixes the reported blank pane at the root (the composer is the content), follows the product's own Agents-queue vocabulary, and handles long choices. If the reviewer wants the smallest diff, ship A and keep B's New work inline layout as a follow-up (it is CSS-only: the single `:has(...)` rule).

Tradeoff for B: smaller touch targets on phone unless the 36px dock rule is kept, and a left-aligned measure leaves empty space at the right on very wide windows (accepted: the Workbench is also left-anchored).

## 8. Open questions (options + recommendation)

1. Centred (A) or left-aligned (B) reading column? Options: centred / left. Recommended: left, consistent with Workbench; revisit if windows regularly exceed 1600px.
2. Lowercase section headings (`needs you`) or sentence case? Options: lowercase (matches `spaces`/`agents`) / sentence case. Recommended: lowercase via CSS `text-transform` so the DOM text stays "Needs you" for AT and tests.
3. Arrow-key navigation in the list (roving, as `useRovingList` in the sidebar)? Options: add / skip. Recommended: skip now; it is a behavior addition and the list is short; reconsider if lists exceed ~30 rows.
4. Manual "Retry now" on first-load failure? Options: add / state automatic retry only. Recommended: state it only; the loop already retries every 1.5s and a button needs new code paths.
5. Show `observation.error.message` visibly? Options: tooltip only / small visible text. Recommended: small visible text when it is a human sentence; keep tooltip as a supplement. [INFERENCE] I did not inspect what `ErrorResponse.message` can contain.
6. Phone dock control height: 30px (current) or 36px? Recommended: 36px, only for ≤700px.
7. Chevron rotation 120ms vs the direction's 80ms ceiling? Recommended: 80ms.
8. `ul/li` list semantics around rows? Recommended: yes when the heading/section wrapper is added; otherwise skip.
9. Show `state_changed_at` time on direct-agent rows? Data exists (`v1.ts:1097`) but is not shown today. Recommended: skip (no behavior or data change requested).

## 9. Acceptance scenarios (observable UI checks, run in a disposable fixture)

1. 1100×640, populated: list header stays pinned while the list scrolls; every row shows a visible state word; no row shows an action pill; a 3-line title wraps to 2 lines with ellipsis; selected row has fill plus 2px left edge and only that row has `aria-current="true"` (no `aria-current="false"` anywhere).
2. 1440×800: content and dock left/right edges are on the same 720px measure (B: left; A: centred).
3. Empty fixture (no needs-you, no relevant agent, 2 other threads): "Nothing needs you right now." with `check-circle`; Other work shows `· 2`; the detail shows New work with no blank band between heading and composer (B) or a centred heading above the dock (A); Send disabled until text.
4. Loading (block `orchestrationInbox`): skeleton rows plus "Loading inbox…" status, `aria-busy="true"`, Send disabled with the visible reason; nothing auto-focuses.
5. First-load failure: the failure message and "Retrying automatically." are visible text; when the call starts succeeding the list replaces it without user action.
6. Refresh failure with data: items stay; note shows "Showing the last loaded items." and the message without hovering.
7. Question flow: select a thread, pick an option, the draft equals the option text, dock reads Answer, nothing sent until Answer is pressed; pressing the question header again deselects and returns to Follow up (existing behavior).
8. Direct agent: selecting does not move focus to a terminal; Open session still moves focus only after the existing confirmation; Send direct disabled when the agent is working (existing note shown).
9. Failed delivery: red alert note with full text, draft kept, button reads "Retry original request"; retry reuses the same message identity (existing test `:174`).
10. 390×844: list only; opening a row shows detail and focus is on Back; Back shows the list and focus is on that row; no horizontal scroll at 320px width; dock stays above the keyboard safe area.
11. Keyboard only: Tab reaches New work, every row, disclosures, Back (mobile), options and dock in DOM order, with the visible ring; Enter/Space activate; Escape behavior unchanged.
12. Regression suite named in the spec (`Inbox.test.tsx`, `App.integration.test.tsx` Inbox cases) still passes without selector edits; Main runs the affected checks, I did not.

## 10. Examples (paths)

- `research/ui-inbox-polish/a-ledger.html`: Alternative A, nine interactive frames.
- `research/ui-inbox-polish/b-queue.html`: Alternative B, nine interactive frames.
- `research/ui-inbox-polish/engine.js`: shared renderer and sample copy used by both (must stay next to them; mocks are not self-contained without it).
- This spec: `research/ui-inbox-polish.md`.
