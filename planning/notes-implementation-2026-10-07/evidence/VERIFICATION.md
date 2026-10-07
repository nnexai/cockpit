# Final Notes runtime verification — 2026-10-07

## Result and boundaries

The actual isolated rebuilt Notes frontend passed the exercised G1–G19 scenarios, including real CLI/HTTP/store readback, explicit recovery and byte comparisons. This report is browser evidence, not native evidence. No product files were edited by this worker. No shared dist/target writes, installation, product/snapshot commit, push, user/default-session mutation or unapproved compiler/test run occurred.

**Subsequent main visual-review blocker resolved:** Main's actual F43 image inspection identified NR26 author input/Draft kept overlap. Parent seq12 authorized Main's scoped CSS correction and one isolated corrected-overlay rebuild/smoke. The worker made no product edit. Current corrected-source geometry/runtime proof passes; Main personally inspected F26 dirty1440/420, known1440 and unknown420 and accepted the visual correction. Root acceptance remains the parent's review, not inferred from a build. Original66 and targeted-known1440 captures remain explicitly pre-correction evidence.

### Corrected NR26 source/actual-runtime proof

[`composer-correction.json`](composer-correction.json) and [`composer-correction-browser.json`](composer-correction-browser.json) record the authorized sole change: `.notes-view .notes-composer-author` `min-width:min-content`, NotesCSS SHA256 `00827bbf127ce68d345f18ad5d52fda334e7fba83bda730e82d934405eec6652`; all other original Notes hashes, exact NR49 global-rule hash and unchanged helper hash match. Passed tsc/Vite generated and actually loaded new `index-CRpoD7vq.js`, SHA256 `056930d914b87a049d2fcb1c4c71b8db68bd626323d61de9f71408cac6272ede`. Build start UTC2026-10-07T06:32:03.929936+00:00, completed process observed UTC06:32:17.665418+00:00 (conservative CPU exclusion interval; precise earlier exit instant not instrumented). No other compiler/test ran.

One fresh owned fixture `/tmp/cpol-u7_y9k7f`, loaded origin35897, UI-pinned UUID `4f45b8c1-ddba-4363-a382-c505e4ab4a94`; task `ytvx01zu9c`,19 real seeded comments. Actual empty/dirty/known/unknown states were measured at viewport1440/detail360, viewport420/detail420, and viewport360/detail360. Six author-label/unverified/input/Draft-kept/Discard/Comment rectangles have **zero pairwise overlap**; every present node is visible, center-hit unobscured and contained in the footer. Footer bottom stays12px before recovery-slot top, with no composer/recovery intersection. Empty Comment remains disabled; dirty/known enabled; unknown disabled. All420/1440 document overflow0. At360 only existing Supervisor chrome and Decisions tab extend beyond viewport (document379px vs360); corrected footerx14..346 fits360. This unrelated chrome limit is recorded, not fixed.

Caret anchor/head34 and exact composer draft survive all states/layout moves. Within each width, dirty/known/unknown thread scroll is unchanged:600 at1440/detail360 and360,557 at420. The cross-width difference is real Markdown reflow/scroll anchoring, not incorrectly reported as universally600. Known setup is labelled F outgoing date-only adaptation with unchanged real backend400 `notes_invalid_input`; no publication. Unknown setup forwards one actual200 changed=true comment publication, then returns wrong-kind DTO. Existing20 content files remain exact-byte identical; one new comment file makes count19→20,0 decisions. Actual saved-state check shows proof while Comment stays disabled; bounded recovery scrolling makes acknowledgement reachable, explicit ack enables Comment without reposting and retains draft. Continuous controller records exactly one comment_add. `F26-composer-fixed{1440,420,360}.png`, `F26-composer-fixed-known*.png` and `F26-composer-fixed-unknown*.png` are affected-afterproof images, not relabelled old screenshots.


**Previously missing comparator closed by observed evidence:** main review identified missing PLAN334 known-error **enabled Comment** at1440. Parent seq11 explicitly cleared one same-source isolated rebuild and fresh targeted fixture in the CPU BUILD/CHECK window. [`targeted-known-comment.json`](targeted-known-comment.json) records UTC build start2026-10-07T06:25:08.122938+00:00/end06:25:10.149286+00:00, passed tsc/Vite, exact original Notes/global-rule/helper source hashes and byte-identical `index-CNceJCyF.js` asset. The actual gateway returned unchanged HTTP400 `notes_invalid_input` after the labelled validUI2026-10-01→outgoing2026-13-45 date-only adaptation. At viewport1440×1000/container1200, enabled `#postCommentBtn` rect(1331.25,856,94.75,28), center(1378.625,870), resolves exactly to `BUTTON#postCommentBtn.notes-primary`; no panel/slot overlap. Screenshot `F43-known-comment-1440.png` is the actual state. All20 content files remained byte-identical,19 comments/0 decisions; composer draft/caret35 preserved. In the already-mounted known-error detail, clearing the41px slot restores0px while scrollTop600 remains600. The preceding Decisions→Todos tab remount independently reset thread600→0/focused heading; it is recorded, not falsely attributed to slot layout. Check-button removal leaves BODY focus, not alert focus. This closes the named comparator; no wider scenario or native claim is added.

PLAN's failed-read visible-copy assertion was corrected by parent Resolution A (seq9), preserving the existing generic unknown sentence. The failed check must instead preserve the incident/guards/drafts, produce no acknowledgement and publish nothing. The original wording expectation was genuinely not observed and is **not** represented as a pass. All safety behavior was observed. The actual visible generic sentence remains unchanged.

### Evidence classes

- **R:** actual browser frontend → HTTP gateway → Notes core/files, independently read through the real pinned CLI.
- **X:** externally editing only owned synthetic fixture files, still read/refused by the real backend.
- **F:** explicitly controlled browser response/request setup. Not a network-loss, native or backend-fault claim. Unknown tests forwarded a real successful write before corrupting its DTO. Failed-read setup rejected comment_list in the browser. Known-error setup changed a valid UI date to an invalid outgoing date and preserved the backend's real HTTP400. Pending Saving/Loading captures held unchanged real successful HTTP replies; these captures prove presentation under pending responses, not a naturally slow backend.

## Provenance and isolation

[`final-provenance.json`](final-provenance.json) records HEAD `b538420beac5d2bd843f417559b9bde552bdedee`, the complete frozen Notes overlay hashes, helper/executable hashes, fixture ledger, gateway command line and asset hashes. The snapshot `/tmp/cnotes-final-sqy1l4bc` contained archived HEAD plus working Notes files and **only** the exact cleared NR49 global rule. Concurrent dirty Supervisor files were not copied. Installed dependencies and the existing host binary were reused via symlinks; the standard runtime helper was physically copied **unchanged**. Its synthetic initial commit was solely in the expressly permitted disposable fixture repository.

One authorized `bun run build` passed `tsc --noEmit` and Vite (471 transformed modules). Vite's ordinary large-chunk warning was retained. The gateway served this snapshot's dist, and the browser actually loaded `http://127.0.0.1:34591/assets/index-CNceJCyF.js`; this was checked again on the final surface. CLI help and real catalog/scratchpad/board/decision/comment DTO reads established runtime compatibility, not mtimes.

Fixture `/tmp/cpol-g32vk7wo`, session `polish-g32vk7wo`, Notes root `/tmp/cpol-g32vk7wo/data/cockpit/notes`, UI-created UUID-A `fe8bb635-e5ae-4709-9e35-cbbd96dcdaa4`, UUID-B `963fae66-b644-44dc-b116-eff00eac3fae`. UUID and Folder were read from Agent access before pinning CLI writes. Initial w1, w2, w3 came from actual workspace.create results. An early stale Space transition inadvertently created an empty, owned third UUID in w3; rather than force-detach/delete its association, the transfer target was a freshly created **w4** (`Transfer target`). This is a fixture-setup deviation, not a user-store mutation or identity substitution. Transfer was w2→w4 and independently resolved by CLI. UUID-B content hashes did not change.

Measured viewport→Notes widths: 1440→1200, 900→660, 420→420, additional band check1100→860. Screenshots have deviceScaleFactor1.25 (e.g.1440 viewport yields1800 image pixels). [`final-screenshots.json`](final-screenshots.json) inventories all final image hashes/pixel dimensions; viewport widths are from actual DOM measurement, not image filenames alone.

## G1–G19 observations

| Gate | Observed result | Primary evidence |
|---|---|---|
| G1 | PASS R. Picker helper/catalog/actions center exactly840/570/210 at1440/900/420; widths600/600/396; vertically separate. Trusted Tab traversed Attach→Cancel; visual Cancel precedes Attach. | F01-picker-*.png; runtime-results.G1/pickerOrder |
| G2 | PASS R. Transfer group600px equals catalog/action width, lies after catalog; Keep current association initially focused; Keep returns focus to Attach. Explicit Attach here transfers UUID-B, old w2 UI shows No notes and real CLI returns notes_unbound; w4 resolves UUID-B. Content hashes unchanged. | F02-transfer.png; F02-old-space-unbound.png; transfer-readback.json; G2.geometry |
| G3 | PASS R/X. Missing root gives real notes_not_found, separate code line and Retry; Attach disabled. Retry remains actionable; Cancel returns opener. Creating only the owned root yields successful empty catalog, no binding/content creation. | F03-catalog-error.png; F03-empty-catalog.png |
| G4 | PASS R. Wide label/source/Save text edges574px; editor measure532px.900 text edge304;420 edge18;1100 edge404. Source/Preview labels are visible above narrow band and icons-only at420. Actual settled EditorView selection anchor/head43 and scroll600 are identical before/after Source→Preview→Source. Mod-s publishes the typed text. | F04-source-*.png; F04-preview-1440.png; editorSettledBefore/After; scratchpad-mods.json |
| G5 | PASS X/R. Real external append produces conflict; saved preview and alert text both left586px, preview14px/21.7px. Keep/Reload computed bg,border,color identical; consequence shown. Reload leaves external saved bytes unchanged; second conflict explicit Keep mine saves exact captured draft bytes. | F05-conflict.png; G5.alignment/styles; scratchpad-keep-draft.txt |
| G6 | PASS X/R. Explicit hand-written adoption opens details and adds only the target marker; siblings exact-byte unchanged. Duplicate A explicit ref repair changes only its line and leaves Duplicate B untouched. Narrow Adopt label hidden, exact title preserved. | F06-adopted-1440.png; F06-duplicate-adopt-420.png; adoption-bytes.json |
| G7 | PASS R. Resting B/Do/Dn chips carry Backlog/Doing/Done full names and .65 opacity; action columns align. Clicking chip selects Kanban/focuses the matching card title. | G7/G7Done/G7ClickFocus; F19-todos-*.png |
| G8 | PASS R. UUID-B has two actually completed todos. Hide completed retains its name, changes pressed state and shows All tasks are completed and hidden. Toggle leaves its sole todos.md hash unchanged. | F08-filtered-empty.png; G8; transfer-readback content hashes |
| G9 | PASS R. Delete uses trash, wide detail Close uses X; confirmation Keep restores notes-comment-delete focus without deleting. Subsequent browser post/save/edit and separate actual confirmed deletion are read back by CLI. Author/created metadata preserved. | F09-thread-1440.png; G9/threadStyles/commentDelete |
| G10 | PASS R.420 Back to tasks visible, X hidden. Delete Escape cancels; dirty edit Escape parks draft, next Escape closes; reopening Back returns comments opener. Draft explicitly discarded rather than saved. | F10-comment-draft-420.png; G10 |
| G11 | PASS R. Count-free T5 card58.296875px vs baseline96.296875; action row0px vs38. Trusted handle→checkbox→title→comments→summary order preserved. Menu expands in place; pointer/text/grab cursors; selected handle .4. Rapid trusted Backlog→Doing→Done changes only lane/checkbox, preserves source order/remembered Doing. Escape/Tab no-write cancellation, extra-left clamp→Backlog. Settled Tab advances to checkbox. Same-column keyboard/pointer and outside pointer no writes. Real pointer into populated Doing uses collision and changes only lane. | F11-card-menu-1440.png; F11-rapid-done.png; F11-drag-preview.png; F11-pointer-populated-doing.png; G11/pointer* |
| G12 | PASS R. All three plus names Add a card to Backlog; Doing plus focuses kanbanAddInput. | G12/G12Focus |
| G13 | PASS R.900 chip borders visible; Jump to Done changes board scrollLeft; computed right-edge mask and12px status recorded. | F13-board-900.png; G13 |
| G14 | PASS R. Actual sort order and stateful name reverse. Selected current successor remains selected under History with context sentence. History (replaced) option; body query finds Short decision. | F14-filter-context.png; G14 |
| G15 | PASS R. D1 Decided2026-10-01 visible, replacement disabled on replaced record; short record without decided has no invented date. Summary Details & replacement. Text-bottom/footer void40px vs baseline631.0625. D1 predecessor bytes unchanged. Actual browser create/edit/replacement produces linked successor and leaves its predecessor exact hash unchanged. | F15-predecessor.png; F15-short-decision.png; F15-browser-successor.png; final-store.json |
| G16 | PASS F/R under approved Resolution A. Real publication then wrong-kind reply gives2-step unknown; Comment/Add disabled, composer retained; thread scroll3883→3883. Failed Check gets no ack/guards remain. Successful Check gives third step but guards remain until explicit ack. Close/reopen resets proof to2steps and requires new Check. Ack does not repost. Targeted visible add input setup retains both add+composer drafts across fault/remount/ack. One-shot fault controllers count exactly1 dispatched add each; first continuous observer spans fault/read/remount/ack. Readback+files prove+1 per fault. | F16-unknown/proof/failed-read-420.png; F16-unknown/proof-1440.png; F16-both-retained-drafts-1440.png; unknown-publication.json; G16/G16draft/G16wide |
| G17 | PASS X/R. Only T3 externally removed; Scratchpad retained disclosure is singular/warning/collapsed. Readonly textarea selectable; explicit discard removes draft without writing todos. Original bytes restored exactly. | F17-retained-title.png; retained-source.json |
| G18 | PASS R. Initial names Todos8open/Kanban3onboard/Decisions3records(Current and History) match CLI seed counts. Arrow/Home/End rove to expected tab IDs; later counts track real mutations. | G18/rove/home/end; final-seed.json |
| G19 | PASS R. Functional text walk reports no<12px outside precise decorative keep-list across all4panels at1440/420 and expanded420Agent; document overflow0. Every task checkbox label24×24. Trusted click inside label/outside16px input publishes completion; second click restores, confirmed CLI. Actual same-call CDP coarse emulation matches pointer:coarse and all card comment/handle opacities1. | F19-todos-*.png; F19-actual-coarse-reveal.png; typeAll/G19/coarseSameCall |

## All49 findings: explicit disposition/evidence

All rows below describe observed final presentation, not test-name inference. G references refer to exercised scenarios above. NR11/27's rare pending-state images are separately labelled F; native CSS support remains unverified.

| NR | Result / observation |
|---|---|
|01|PASS G1 column centers/stack|
|02|PASS G2 same-width transfer group/warning Attach|
|03|PASS G1/G2 visual Cancel-left, frozen Attach→Cancel Tab|
|04|PASS G3 real error/code/Retry|
|05|PASS R/X: selected owned empty UUID directory temporarily renamed outside root, reopened catalog displays missing-selection hint with disabled Attach; restored exact owned directory. F05-vanished-catalog.png|
|06|PASS G18 exact count-bearing names|
|07|PASS420 header8px padding; header+tab rows retained; Close/Escape exercised|
|08|PASS G10 narrow Back/wide X|
|09|PASS G4 shared532px measure/aligned574px edges|
|10|PASS source/preview labels vs narrow icons, pressed mode and retained EditorView state|
|11|PASS F timing: unchanged real scratchpad write reply held, visible Saving Notes…; released and saved. F11-saving-notes-pending.png|
|12|PASS G5 Saved version(read-only),14px and exact aligned edges|
|13|PASS G5 neutral equivalent choices/consequence/exact explicit overwrite|
|14|PASS composer/edit gutter absent, decision body retains gutter in actual forms|
|15|PASS G7 B/Do/Dn lane chips/full names/reveal|
|16|PASS G6 explicit adoption/ref repair, no new confirmation|
|17|PASS G8 constant Hide completed name|
|18|PASS G8 filtered-empty sentence with zero visible tasks|
|19|PASS computed text cursor on title editors; no new blur-save|
|20|PASS todos.md summary label wide/band860, hidden660/420; popover remains read-only|
|21|PASS final depth0/1/2 and12/24/36px pad match baseline; gradient guides observed wide/narrow, nesting bytes remain canonical|
|22|PASS G9 trash+confirmation+Keep focus; actual delete separately read back|
|23|PASS header12px/unverified suffix/disclosure below body; trusted Edit→Delete→Record details focus, solid outlines|
|24|PASS real invalidUTF8 comment file yields Comments unavailable/Retry; exact bytes restored and explicit Retry returns real content. Empty row0px; plain T3 empty thread centered. F24-real-invalid-utf8.png/F24-empty-thread.png|
|25|PASS real Move/unboard/reboard and retained thread; F25-unboarded.png|
|26|PASS corrected-source visible unverified qualifier and nonoverlapping author/input/status/actions at1440/detail360,420 and360; actual empty/known/unknown guard/draft/caret/scroll/publication evidence in composer-correction.json and F26 images|
|27|PASS reachable real new-below pill/external append; malformed source warning; externally removed-comment retained edit/discard; removed task band and comments retained; unboarded style. Pending loading separately F delayed unchanged real read. F27-*.png|
|28|PASS disabled primary states .6 presentation, actual add/post/save guards preserved|
|29|PASS G11 compact card/menu/focus preserved|
|30|PASS distinct down chevron vs handle glyph; menu rotation and trusted keyboard transitions|
|31|PASS G13 border/pill chip and actual Jump scroll|
|32|PASS G12 allplus Backlog names/focus|
|33|PASS G11 pointer/text/grab cursors/selected handle .4/coarse override|
|34|PASS900 computed edge mask, scrolled Done lane image; no changed scroll tracking|
|35|PASS live status computed12px|
|36|PASS G14 sort wording/names/order flip|
|37|PASS G14 filtered selected-record context|
|38|PASS G15 Recorded vs Decided metadata, no date invention|
|39|PASS G15 Details & replacement and content-footer void40px, additive history intact|
|40|PASS G14 History(replaced) filter|
|41|PASS actual new/edit forms and blocked-color discard hover; F41-new-decision.png/F41-edit-form.png|
|42|PASS420 list first/back/resume retained exact title draft; F42-list-draft-420.png/F42-retained-draft-420.png|
|43|PASS: real400 known alert after labelled request-date adaptation: Save wide/narrow and Comment narrow/wide enabled center hit-tests unobscured; unknown recovery controls actually clicked; empty slot0px. Wide comparator completed by separately authorized same-source rebuild/fresh fixture with exact hit/rect/bytes/draft/scroll evidence in targeted-known-comment.json; no realnetworkloss claim|
|44|PASS G16 two/three recovery steps, guards/proof/ack; approved existing generic copy retained|
|45|PASS G17 singular retained-source summary/selectable textarea/explicit discard|
|46|PASS groups Scratchpad/Todos/Board/Decisions/Comments; all23 labels/order and values compared with baseline. Normalize only root/UUID/revision; sameL6 unadopted parent/no selected decision. Explicit view-preference setup then real reload reads, not DTO/model substitution.23 readonly command inputs, forward/reverse46 input/copy controls; Copy Scratchpad read succeeds with matching clipboard/status. agent-comparison.json/agent-comparator.json|
|47|PASS G19 functional-size walk across all panels and Agent|
|48|PASS G19 target24×24/actual outside-box toggle+restore|
|49|PASS mouse parked/focus away: Notes open bg35,53,76/color137,180,250/border59,71,88/inset2px accent; closed transparent/color186,194,222/border43,53,67/no shadow. Supervisor selected same four exact baseline values unchanged. F49-supervisor-unchanged.png/NR49|

## Publication, durability and request accounting

[`final-seed.json`](final-seed.json), [`final-store.json`](final-store.json), [`adoption-bytes.json`](adoption-bytes.json), [`retained-source.json`](retained-source.json), [`unknown-publication.json`](unknown-publication.json) and [`transfer-readback.json`](transfer-readback.json) carry actual identifiers, revisions and byte hashes. All contents are synthetic owned fixture data.

Thread count sequence: seed18, browser post19, external append20, narrow forwarded fault21, wide forwarded fault22, actual browser delete of wide fault21, CLI deletion of narrow record during retained-edit probe20, targeted both-drafts forwarded fault21. The first fault's Hpre/Hpost differs by exactly one comment file and no existing file. Subsequent+1 reads and controller dispatch counts are recorded independently. Checks/ack/remount never add a comment. A failed one-shot Check setup was initially consumed by background polling; it was replaced with an explicitly armed failed-read wrapper, then successful recovery was exercised. This is F browser setup, not a real-network outage.

Initial draft-retention measurement filled an input while narrow detail was active and did not establish add-draft setup; it is not used as failure/pass evidence. The corrected setup typed into the **visible** add input before opening details; its exact text persists through unknown/remount/ack. Early DOM-selection/focus/wheel measurements were not valid CodeMirror state comparators; settled EditorView selection/scroll equality is the authoritative comparison.

Some browser selector calls initially observed pending old views after async Space/mutation transitions. Subsequent evidence waited for actual header/record/state identity before asserting. One mobile-emulation attempt closed the managed page and reset observation state; it was reported as a tool issue. The browser was reopened against the same owned fixture. Actual coarse media proof was then captured within one CDP call, including matching media and opacity values. Failed coarse attempts are not accepted evidence.

## Limits, exclusions and cleanup

D01–D12 remain excluded; no state/handler/protocol/product changes were made by this worker. Browser evidence does not verify native WebKitGTK :has()/mask rendering, GPU path, pointer touch physics on physical hardware or filesystem-wide external-writer CAS. Pending-state F timing images are not substituted for real publication/read refusal. The extra drag source-change/window-blur/Space-switch cancellation matrix was not exhaustively reproduced; rapid keys, Escape/Tab, clamp, same/outside pointer, populated pointer and Notes-close cancellation were exercised. Layout checks use snapshot/store checkpoints and observed operations, not a per-step hash file for every non-mutating click. No stress, quota, provider or user environment claim is made.

Main reviewed meaningful final screenshots before original cleanup (including catalog, nesting, aligned source, compact menu, short decision, Agent access, known420Save and unknown420guards) and authorized owned cleanup. `cleanup.json` records original resource removal/registry-byte equality. The separately cleared missing-comparator run closes only its owned browser/fresh fixture, while retaining the immutable same-source snapshot/dist `/tmp/cnotes-known1440-r2h_29x3` for parent acceptance/root review; its lifecycle is recorded in `targeted-known-comment.json`. No throwaway driver files were created in the repository. The frontend builds and runtime evidence are independent of acknowledged unrelated dirty Supervisor typecheck failures; those checks were not rerun or fixed.

Corrected NR26 runtime/browser/process cleanup is recorded in `composer-correction.json`. **Both** immutable source/dist snapshots remain for parent root review: `/tmp/cnotes-known1440-r2h_29x3` (pre-correction same-source missing comparator) and `/tmp/cnotes-composer-fixed-0m5egf_c` (corrected source/new asset). No fixture/browser/process should remain. Shared node_modules/host targets are symlinked dependencies and must not be removed by eventual owned-snapshot cleanup.

## Main integration acceptance

- `bun run test -- src/app/notes`: **2 files, 22 tests passed**. The behavior suite uses stable `#f-todos` selectors for the changed count-bearing tab name, removes an incidental catalog-error wording assertion, and checks that a vanished selected catalog target disables Attach without dispatching a write.
- Shared-checkout `bun run typecheck` failed in concurrent, unrelated Supervisor code: `SupervisorView.test.tsx:12` (`Run.bound_omp_process` optional mismatch, TS2322) and `SupervisorView.tsx:312` (`ancestor` implicit/self-referential any, TS7022). Main reported these to the owner, did not alter those files, and did not rerun to confirm them. Isolated Notes overlays passed the expressly authorized tsc/Vite builds above.
- One independent `NotesSafetyReview` round found **no high-severity patch-introduced data-loss, concurrency or security findings**. UUID-pinned requests, draft reconciliation, CAS, adoption, comment/decision writes, focus-return selectors and read-then-explicit-ack/no-replay guards were reviewed. The subsequent correction is only the authorized author-label min-content CSS rule; no handler/state/DOM change followed the review.
- Main personally inspected persisted final catalog, nested420, Source1440/420, compact-menu1440, short-decision, Agent420, known420 Save, unknown420, targeted known1440 Comment, and corrected F26 dirty1440/420, known1440 and unknown420 screenshots. Actual author/status overlap was caught during this review, corrected and exercised rather than accepted as a passing screenshot.
- Sole shared-doc integrator applied the agreed in-flow/frozen-behavior paragraph at canonical `/home/nnex/dev/prj/cockpit/CODE_GUIDE.md:106`; Main's fresh read returned snapshot `896A` and confirmed that paragraph, preserving adjacent widget/Supervisor work.
- PLAN failed-read wording was corrected under parent Resolution A; COVERAGE NR26 records the observed-overlap corrective clearance. The accepted illustrative design and atlas were not modified; D01–D12 remain excluded.
- No product commit, push or installation was performed under the parent's explicit restrictions. Disposable synthetic fixture initial commits used only the unchanged, expressly cleared helper. Retained immutable source/dist paths above are a deliberate parent root-review handoff, not running services or product scaffolds; eventual deletion must preserve shared symlink targets.
