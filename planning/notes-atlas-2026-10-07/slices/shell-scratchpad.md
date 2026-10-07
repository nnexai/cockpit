## Evidence and scope

This is a code-derived atlas only: no runtime proof, fixture use, tests, builds, or edits. Line ranges below refer to inspected repository source. Notes view owns multiple other surfaces (Todos, Kanban, Decisions, task detail); this slice covers their shared shell/nav and the Scratchpad specifically, not their individual workflows.

## N-SHELL-01 — Workbench entry, placement and shared header

**Evidence:** `src/app/App.tsx:180-191,238-241,490-493,677-678,1098-1100,1188-1191`; symbols `TabStrip`, `closeNotes`, `openNotes`, `notesOpen`, command action `notes:toggle`.

- Notes is a workarea toggle in the top bar (button label “Notes”/“Open Notes”, `aria-controls="cockpit-notes"`, `aria-pressed`) alongside Browser and Library. It is not a Herdr pane or tab. `openNotes` sets `localWorkarea` to `notes`; close clears it. Both update attach-focus suppression.
- A parallel Commands action `notes:toggle` labels itself “Open Notes”/“Close Notes”; it is disabled without a selected Space. The Notes top-bar button has no own no-Space disabled prop, but its enclosing TabStrip renders only when `selection.spaceId` exists (`App.tsx:1188`). NotesView itself still handles an absent target with a select-Space empty state.
- NotesView is keyed by `${sessionId}:${selection.spaceId}`. App renders it instead of the Library/canvas when Notes is open, unless Supervisor is open. `space={librarySpace}` and `client` are passed in.
- NotesView header is a section (`id="cockpit-notes"`, label “Notes”), with Space label or “Select a Space”, and “Close Notes” button. Escape closes unless picker/transfer is active; see N-SHELL-02. Header styling is in `notes.css:1-25`.
- Focus routing: opening Notes causes NotesView to remember the currently focused external HTMLElement and focus its root. On unmount, it returns focus to the still-connected invoker, otherwise the top-bar element selected by `[aria-controls="cockpit-notes"]`; if neither exists no fallback focus is called. App-level pane-scoped shortcuts deliberately exit local workareas before executing (`App.tsx:959-965`); the global `routeWorkbenchKeydown` registration is at `App.tsx:983-984`.
- A Notes top-bar action is visible in the shared toolbar; the shortcut registry (`src/app/input/shortcuts.ts:28-48,118-140`) has no Notes shortcut entry. The Commands palette action is separately assembled in `App.tsx:1098-1100`.

## N-SHELL-02 — Space binding and create

**Evidence:** `src/app/notes/NotesView.tsx:164-239`; symbols `NotesView`, `target`, `resolve`, `bind`; exact binding render/actions in `NotesView.tsx:269-280` (render branches); shared style `notes.css:1-10,239-249` (binding selectors; inspected source owner).

- Resolution uses the Space target `{session_id, space_id}` and `target_resolve`. If target is absent, `loading` is cleared and the view shows “Select a Space to open its notes.” If the Space is not live, it shows `notes_space_unavailable` with explanation that the association needs a live Herdr connection and drafts are kept. A successful `kind:"target"` response stores `NotesTargetInfo`; failures become `notesError` and leave info null.
- On target/session/space/live dependency changes, the effect resets info/error/loading/picker/selected/transfer state before resolving. It retries resolution on window focus and every five seconds while the document is visible, unless binding is pending or picker visible. The resolver effect’s active flag suppresses updates after cleanup.
- On `notes_unbound`, the empty binding panel explains files live outside the repository and exposes **Create notes** and **Attach existing notes…**. Both are disabled while busy or when `space.live` is false.
- Create invokes `target_create`; attach invokes `target_attach` with the selected UUID. Both use one `pending` ref to prevent concurrent bind calls, expose `busy`, clear the prior error, then either install the returned target info and close picker or retain a displayed error. A thrown/failed create does not synthesize a new binding.
- Other resolve errors show “Could not resolve this Space’s notes” plus Retry. Non-`notes_unbound` errors also appear in a `role=alert` with message and code. Initial resolving state is text “Resolving this Space’s notes…”, and root `aria-busy` includes loading and busy.
- Because NotesView is keyed by selected Space/session, switching those host identities remounts the view; the active effect cleanup suppresses stale resolve responses. This is code-derived, not runtime-tested here.

## N-SHELL-03 — Attach catalog and transfer confirmation

**Evidence:** `src/app/notes/NotesView.tsx:179-211,234-280`; symbols `loadCatalog`, `closePicker`, `cancelTransfer`, `candidate`, picker/transfer render branches. Relevant tested behavior: `NotesView.behavior.test.tsx:214-270`.

- `loadCatalog` remembers the opener when entering picker, resets catalog to loading and transfer confirmation, then reads root `catalog_list`. The picker announces “Attach existing notes” and says to choose exact Notes ID; catalog entries show a label (“Last called …” or “Untitled notes”), UUID, and whether attached elsewhere/not attached.
- Catalog state variants: idle (no result content), loading (“Finding existing notes…” and root `aria-busy`), error (alert with error text/code and explicit “Retry reading catalog”), ready-empty (“No existing notes found”), and ready with radios. **Attach** is disabled unless the selected ID still resolves to a candidate, or while busy. Selecting another radio clears transfer confirmation.
- For an unbound candidate, Attach calls `bind(candidate.notes_id)` directly. For an already bound candidate it opens a `role=group` confirmation: “Attach here? The other Space will become unbound. Files are not moved or deleted.” **Keep current association** cancels; **Attach here** submits the target attach. The confirm button is disabled while busy or if candidate is absent. Catalog read errors remain displayed until explicit retry; no automatic retry is triggered by focus/elapsed time in the tested scenario.
- Catalog read has no `pending` gate against another catalog read; each invocation sets loading and resolves into ready/error. Binding itself has the pending guard above.
- Cancel picker clears picker and transfer state, then restores focus to the remembered opener if connected, else attach opener; it only steals focus if active element remains body or the control that initiated closing. Cancel transfer similarly returns focus to the Attach action under the same focus guard. Opening picker focuses selected radio, else first radio, else Cancel; on catalog error it focuses Retry, otherwise root. Transfer confirmation focuses Keep current association when focus is body/root, unless another modal dialog is open.
- Escape on Notes root cancels transfer first, else closes picker, else closes Notes; it does not intercept already-default-prevented or IME-composing Escape. Root is focusable with `tabIndex=-1`.
- Shell unmount returns focus to original invoker or Notes top-bar trigger. The picker/transfer remains inline rather than an ARIA modal dialog; no modal focus trap is implemented in this component.

## N-SHELL-04 — Shared tab navigation, layout and state persistence

**Evidence:** `src/app/notes/NotesView.tsx:17-35,38-53,128-161`; `src/app/notes/notes.css:20-41,230-280`.

- Four tabs, in order: Scratchpad, Todos, Kanban, Decisions. Their tab buttons have `role=tab`, selected state, reciprocal panel IDs/labels, and roving `tabIndex` (selected tab 0; others -1). Left/Right wrap; Home/End go to first/last. Ctrl/Meta-modified navigation keys are not handled. The key handler moves focus to the next tab after changing selected tab.
- Tab state is keyed by Notes UUID in both module memory (`surfaceStates`) and localStorage `cockpit.notes.surface.v1:<notesId>`. Valid saved fields are tab, selected todo/decision IDs and hide-completed; detailOpen is accepted only as true. Invalid JSON/storage access falls back to Kanban, no selection, hide completed false, details closed. Persistence write errors are swallowed with a comment noting draft storage has a separate visible warning.
- Shared panels use flex/overflow layout; inactive sections receive `hidden`, enforced by `.notes-view [hidden]`. Notes header, tab strip and tabs use chrome background; responsive tab layout is in `notes.css:260-280`. This component exposes distinct empty/loading/error states rather than a common loading screen.
- Count badges are hidden from accessibility tree (`aria-hidden`) and show open Todo count, board-assigned count, and Decisions count. They are informational, not controls.
- Agent access disclosure and retained drafts are below the tab-panels, shared across tabs (`NotesView.tsx:159-161`). Error/storage alerts occupy an absolute bottom problem slot in the panels (`notes.css:62`), potentially overlaying panel content; slot caps at `min(160px,45%)` and scrolls.

## N-SHELL-05 — Agent access / CLI reference

**Evidence:** `src/app/notes/AgentAccess.tsx:1-72`; callsite `src/app/notes/NotesView.tsx:159-160`.

- Collapsed native `<details>` disclosure, labeled “Agent access” with `cockpit-cli` meta. Expanded text states these commands use the same Markdown files/revision checks, advises pinning the Notes UUID rather than relying on current Space, says replace placeholders with IDs/revisions from reads, and warns not to retry unconfirmed writes automatically.
- Read-only UUID, folder, and command fields select their full value on click. Command snippets use `COCKPIT_NOTES_ROOT` derived from the Notes folder and a pinned `--notes <UUID>` base. The scratchpad group includes read, append, and replace-from-stdin with current Scratchpad revision or placeholder. The broader list covers Todos, Kanban, Decisions and comments; final text mentions catalog/create/attach and `--ref` for imported Todos.
- Each command has a named Copy button. Clipboard success reports `Copied <label>.`; clipboard failure reports manual-select/copy guidance. A persistent `role=status`, `aria-atomic=true` announces copy feedback. Copy failure does not hide or clear the selected command field.
- State variants: collapsed/expanded native disclosure; initial blank copy status; per-copy success/failure. There is no loading/disabled state or clipboard-permission preflight. Controls are regular keyboard-focusable summary, readonly inputs, and buttons; no custom keyboard routing.
- This is displayed command text, not an agent permission grant or execution surface. Code passes selected todo/decision to construct revision-aware example selectors, but no command is run from this panel.

## N-SCR-01 — Scratchpad initial read and draft model

**Evidence:** `src/app/notes/NotesView.tsx:38-53,113-126,136-139`; `src/app/notes/useNotes.ts:13-18,39-97,137-168`; `src/app/notes/drafts.ts:1-64` (draft type/read evidence); symbols `NotesContent`, `useNotes`, `refresh`, `reconcileDraft`, `readDrafts`.

- NotesContent calls `useNotes(client, info)` under a `NotesEditorScope` whose value is the Notes UUID. The hook starts with persisted UUID-scoped drafts, empty loaded records and possibly a persisted unknown-outcome error. Initial refresh concurrently reads Scratchpad, all Todos and all Decisions; unexpected result kinds are errors. Scratchpad document becomes the saved baseline and is merged with any existing local draft using `reconcileDraft`.
- Refresh/poll occurs immediately, on a three-second interval, on window focus, and on visibility change. Poll resolves target change tokens and refreshes records on token change or force. Request failure is displayed unless an unknown outcome is already latched. Draft updates reload current persisted state, set React state and write storage; storage write error is exposed as `storageError` and rendered as alert by NotesView.
- Scratchpad renders “Loading scratchpad…” until a draft is available. Then it renders editor and status. Saved status is “Saved · scratchpad.md”; modified local text is “Draft kept · not yet saved”; busy status is “Saving…”. Editor `onChange` updates the current Scratchpad draft while retaining its base/revision metadata. The normal state presents explicit **Save scratchpad**, disabled when busy, unknown outcome, or unchanged from base.
- Persisted local draft remains distinct from saved document: a fresh read reconciles the draft with latest saved content/revision. Evidence supports local draft preservation, but no runtime storage persistence was checked.

## N-SCR-02 — Markdown source, preview and editor focus/key behavior

**Evidence:** `src/app/notes/MarkdownEditor.tsx:12-13,29-94`; Scratchpad callsite `NotesView.tsx:136-139`; presentation `notes.css:26-41`.

- The MarkdownEditor surface is toolbar text “markdown” plus two mode buttons, Source and Preview, exposing `aria-pressed`. Preview uses `ReactMarkdown` with GFM, skips raw HTML, opens links in new tab with `noopener noreferrer`, and replaces images with a text `[Image: alt]` label. Markdown preview is rendered within a scrollable region.
- Source uses CodeMirror Markdown language support, history, line numbers, selection drawing, syntax highlighting, line wrapping, placeholder and a multiline textbox label/role. Scratchpad CSS hides gutters and centers a readable source column; preview centers rendered content to 72ch. No separate editor-level disabled prop is passed by Scratchpad (defaults enabled); Save control governs write availability.
- `Mod-s` invokes the current save callback and is consumed even without a callback; repeated Ctrl/Meta-S and Ctrl/Meta-Enter keydown are prevented. Default and history keymaps remain enabled. Editor updates feed `onChange` only on document changes. Incoming value sync replaces document content only when it differs, retaining existing editor state/selection/history per implementation comment.
- Session key is `<Notes UUID>/<draftKey>`; Scratchpad draftKey defaults to label “Scratchpad Markdown”. Module map preserves CodeMirror state, scrollTop and preview preference across editor unmount/remount. It evicts oldest map entry when size exceeds 256. Preview mode state is persisted on unmount and explicitly updated in session map by mode-toggle handlers. Switching to preview unmounts source editor; toggling back reconstructs it from cached state.
- Editor source is a CodeMirror contenteditable textbox; toolbar mode controls are ordinary buttons. This file does not explicitly move focus to editor after mode change or restore focus to a mode button after toggling. Focus behavior on those transitions is therefore not otherwise specified here (no runtime proof).

## N-SCR-03 — Save, revision conflict and recovery

**Evidence:** `src/app/notes/NotesView.tsx:100-125,136-139`; `src/app/notes/useNotes.ts:98-136,137-168`; `src/app/notes/drafts.ts` conflict helpers; style `notes.css:49-61`.

- `saveScratchpad` exits without write if draft/saved doc missing, unknown outcome active, another Notes write busy, unchanged against base, or conflict exists unless explicit keep-mine. Standard save uses draft’s captured revision; keep-mine uses current saved document revision. Successful Scratchpad result is acknowledged against submitted content and returned revision. Hook serializes writes by Notes UUID and refreshes on confirmed success.
- While conflicted, normal Save is replaced by alert `Scratchpad changed elsewhere. Current saved version:` with expandable saved Markdown, **Keep mine and save**, and **Reload (discard my draft)**. Keep-mine is disabled while busy, unknown, or its local saving flag is true; its request uses latest saved revision. Reload replaces draft with current saved content/revision through `changedDraft`.
- Mutation error produces visible error status. For unknown outcome, wording states saved result cannot be confirmed, drafts are kept, and another write may duplicate; Save and Keep-mine actions are disabled. **Check saved state** triggers refresh (and selected comment read if any), and an unknown-outcome acknowledgement is only offered after a successful saved-state read; text requires reviewing state before next write. Error boundary message preserves drafts and warns against retrying blindly.
- `useNotes.mutate` blocks writes while another is in-flight or `outcomeUnknown` is latched. A thrown error with authoritative `notes_*` operation code remains that error; other write failures become `notes_outcome_unknown`, stored in drafts and refreshed best-effort. Confirmed write followed by refresh failure says “Change saved, but refresh failed…”. Per-Notes listeners synchronize busy state across consumers. This is the code path backing Scratchpad retry/unknown-outcome behavior; it is not a generic automatic retry.
- Relevant state variants for Scratchpad: loading, saved, local dirty draft, saving, revision conflict, storage failure, ordinary Notes error, unknown write outcome, and saved-but-refresh-failed. No dedicated empty-Scratchpad message exists; empty content is an editable blank Markdown value if server returns it.

## N-SHELL-06 — Invariants and evidence-backed review observations

**Invariants evidenced by code:** binding and filesystem content are separate: transfer UI explicitly says files are not moved or deleted; CLI reference says UUID remains pinned across Space changes. Scratchpad mutations are revision-conditional; draft text is preserved/reconciled rather than overwritten on changed saved revision. Unknown writes are not automatically replayed; writes are held until checked/acknowledged. Catalog attach is selected by explicit UUID, and already-bound targets require a second confirmation.

**Potential UX issues / review points grounded in inspected code (observations, not proposals):**

- The normal Scratchpad status label is based on `model.busy`, which is shared across all operations for a Notes UUID, not just Scratchpad save (`useNotes.ts:98-106`; `NotesView.tsx:138`). Thus unrelated Notes writes can surface “Saving…” in the Scratchpad status. [INFERENCE from shared busy state and rendering.] 
- Mode switches do not explicitly restore focus to the editor or toolbar control, despite caching source session. The source is unmounted in Preview mode, so keyboard focus/selection transition deserves runtime/design review; no behavior claim beyond code. [INFERENCE.]
- Catalog radio selection is not reset when a fresh picker load begins; an ID absent from the newly returned catalog makes candidate undefined and disables Attach, while the prior selected value can remain in React state. No user-facing “selection no longer available” message is rendered. [INFERENCE from `loadCatalog` and `candidate` logic.]
- Binding resolver retries every five seconds and on focus while not in picker; a persistent non-unbound error can be repeated and overwrite current error state with a later result. This is implementation behavior, not observed runtime behavior. [INFERENCE.]
- Picker empty state shows “No existing notes found” and disabled Attach, but no direct create action in picker; user can Cancel to the prior binding state. [Code-derived.]

## Existing tests (references only; not run)

- `src/app/notes/NotesView.behavior.test.tsx:168-270`: consumer safety, uncertain write handling, requiring saved-state read before acknowledgement, preventing comment replay, catalog failure staying visible despite time/focus/late resolve, transfer cancellation focus restoration, no transfer when canceled.
- `src/app/notes/NotesView.behavior.test.tsx:10-22`: the suite keeps real NotesView/useNotes/TaskDetail/TodoTitle but mocks MarkdownEditor and MarkdownPreview; it does **not** exercise real CodeMirror editor behavior or source/preview focus transitions.
- `src/app/notes/NotesView.behavior.test.tsx:57-84`: fixture target, scratchpad read and catalog response setup; useful reference for the consumer request surface.
- `src/app/notes/notesState.test.ts:8-31` covers draft reconciliation/confirmed-save typing preservation, while `33-92` covers Kanban state/drop semantics; neither proves real Scratchpad editor interaction. No inspected test provided direct verification of MarkdownEditor source/preview, Scratchpad save conflict rendering, or AgentAccess clipboard interaction.

## Source map for follow-on reading

- Shell/bind/catalog/transfer: `src/app/notes/NotesView.tsx:164-281` (`NotesView`; target effect; `bind`; `loadCatalog`; render state branches).
- Shared tabs and Scratchpad: `src/app/notes/NotesView.tsx:17-161` (`readSurface`, `NotesContent`, `saveScratchpad`, tabs/panels).
- Markdown implementation: `src/app/notes/MarkdownEditor.tsx:12-94` (`NotesEditorScope`, `MarkdownPreview`, `MarkdownEditor`).
- Agent access: `src/app/notes/AgentAccess.tsx:6-72` (`AgentAccess`, copy handler, generated commands).
- Persistence/error mechanics: `src/app/notes/useNotes.ts:13-168`; draft parse/update helpers `src/app/notes/drafts.ts:1-64`.
- Shell styling: `src/app/notes/notes.css:1-61,230-280`; host toolbar `src/app/App.tsx:230-241,490-493,677-678,1098-1100,1188-1191`.
- Shortcut registry/host keyboard route: `src/app/input/shortcuts.ts:28-48,118-140`; `src/app/App.tsx:959-965,983-984`.
