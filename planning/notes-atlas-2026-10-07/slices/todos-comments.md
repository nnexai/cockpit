# Todos and durable comments — evidence slice

**Evidence status:** source-derived only. No runtime proof; no tests/builds were run. Ranges below refer to inspected source. References to existing tests are code references, not claims that they were executed. Surface IDs are stable identifiers for this slice.

## Todo surfaces

### N-TODO-01 — Todos list, completion/reopen, hide/show completed

- **Sources:** `src/app/notes/NotesView.tsx:140-147` (Todos tab panel, add form, toolbar, list, empty state); `src/app/notes/useNotes.ts:35-42,74-105` (`todoIdentity`, `todoSelector`, refresh); `crates/cockpit-core/src/notes/todos.rs:44-106,176-219` (dispatch/list/filter/parser duplicate marking); `crates/cockpit-protocol/src/notes.rs:25-75,99-115` (filter/selector and todo operations).
- **Controls/actions:** ordered task rows, indentation derived from parsed nesting depth, title textarea, completion checkbox (`todo_set_done`), comments/detail entry, optional on-board badge/action. Checkbox checked state is `todo.done`; toggling sends the inverse, thus both complete and reopen. “Hide completed” is a local surface toggle (`surface.hideCompleted`); it changes to “Show completed” and filters only rendered list rows. A separate empty-list affordance “Add a todo” focuses the composer.
- **State/disabled/error:** empty means no todos at all and shows “No todos yet. Add one above.” plus focus action. If all tasks are hidden as completed, `model.todos.length` remains nonzero, so the empty-list affordance does not appear; list is simply visually empty. Checkbox is disabled while a write is busy or outcome is unknown. Errors/unknown outcome are rendered in shared problem area (`NotesView.tsx:152-158`): conflict retains text and offers reload/keep-mine guidance; unknown outcome warns not to replay, preserves drafts, and requires saved-state check/acknowledgment before further writes. Storage errors appear separately. `useNotes.ts:50-56, 110-148` serializes writes per Notes UUID and marks transport/no-confirmation failures as `notes_outcome_unknown`; it refreshes after confirmed writes or failed writes where possible.
- **Navigation/focus/keyboard:** tabs use tablist/tab/tabpanel semantics and left/right/Home/End navigation (`NotesView.tsx:130-138`). Completion is a native checkbox. Empty action focuses `#todoAddInput`. Task rows use todo identity as React key and `data-todo-id`; depth is visually expressed by indentation. No explicit list keyboard navigation is defined in inspected list code.
- **Invariants:** list obtains `filter: all`, so hide-completed is a view-only filter (`useNotes.ts:75-78`); completion flips the Markdown task marker while list refresh updates model. Identity is unique `todo.id` unless duplicate; fallback is `todo.ref` (`useNotes.ts:35-42`).
- **Evidence-backed UX issue:** `NotesView.tsx:143-147` tests the no-todos state using total model count rather than filtered visible count. With hide-completed on and every task complete, the list is blank without an empty/zero-results explanation or immediate “Show completed” empty-state action. This is a code-derived presentation observation, not runtime proof.

### N-TODO-02 — Add todo

- **Sources:** `src/app/notes/NotesView.tsx:140-141`; `crates/cockpit-core/src/notes/todos.rs:629-694`; protocol `crates/cockpit-protocol/src/notes.rs:99-105`; mutation handling `src/app/notes/useNotes.ts:98-137`.
- **Controls/actions:** single-line input `#todoAddInput`, submit button `#todoAddBtn`; Enter follows native form submit. Nonblank trimmed value sends `todo_add` with `lane: null`. On successful response clear only if the current draft still equals the submitted text, preserving typing entered during the request.
- **States:** button disabled when busy, unknown-outcome locked, or trimmed input is empty. Input draft is maintained in Notes drafts. Empty list offers a focus shortcut. Core normalizes CR/LF to spaces, rejects content exceeding 2 KiB, treats normalized empty text as unchanged, rejects 5,000 existing tasks, appends an ID-bearing unchecked Markdown task, preserves inferred line-ending style, and rejects malformed placement/unterminated Markdown blocks (`todos.rs:519-537, 629-694`). Errors surface through shared Notes error panel; unknown write outcome prevents retry until read/review/ack.
- **Invariant:** add appends to the file and refreshes; no implicit nesting or board lane from this form.

### N-TODO-03 — Edit title, retain draft, reconcile conflict, adopt untracked task

- **Sources:** `src/app/notes/TodoTitle.tsx:13-73`; `src/app/notes/useNotes.ts:35-42, 75-105, 110-140`; core `crates/cockpit-core/src/notes/todos.rs:547-627, 696-820`.
- **Controls/actions:** title is an autosizing textarea. Draft changes are stored under `todoIdentity`. Enter without Shift (and not IME composition) saves; explicit “Save title” also appears when dirty. For ID-backed unique items the ordinary edit selector includes draft revision. For ref-backed items it uses the ref. An explicit edit with `text: null` is used for adoption (N-TODO-05). `TodoTitle` does not expose removal.
- **State/error:** dirty title displays “Draft kept.” External revision change sets conflict through `reconcileDraft` (`useNotes.ts:86-90`; draft helpers tested in `notesState.test.ts:8-37`). Conflict shows current saved title and “Keep mine and save” / “Reload (discard)”. Keep mine uses current selector revision; normal save is blocked while conflict exists. Metadata-malformed and lazy-continuation titles are disabled and advise direct Markdown repair. Busy/unknown outcome blocks writes. Successful title save migrates draft key when adoption assigns a new stable ID; typing accrued during save is retained (`TodoTitle.tsx:43-58`).
- **Keyboard/focus:** textarea remembers selection by Notes scope + identity and restores selection when appropriate; `ResizeObserver` resizes it (`TodoTitle.tsx:7-34`). Enter commits except Shift+Enter/IME. No explicit blur-save.
- **Invariant:** conflict resolution is deliberate; title editor does not silently overwrite an external revision. Core normalizes text and rejects malformed ownership/metadata (`todos.rs:696-717`).

### N-TODO-04 — Complete/reopen task from detail; board placement controls

- **Sources:** `src/app/notes/TaskDetail.tsx:180-186`; list checkbox in `NotesView.tsx:143-145`; core `todos.rs:696-820`; protocol `notes.rs:99-128`.
- **Controls/actions:** detail checkbox “Completed” toggles done; lane select sends `kanban_move` to Backlog/Doing/Done. `kanban_promote` adds an unboarded task to Backlog; “Remove from board” uses `kanban_unboard` and explicitly says task/comments are kept. Todo tasks can have no lane; an unknown lane remains unboarded in parsed model until explicit valid lane repair.
- **States/invariants:** these controls disable while busy/unknown. Moving to Done marks done while preserving prior lane; moving to Backlog/Doing reopens task and records that lane. Promotion is a no-op when lane already exists; an unboarded task becomes Backlog. Unboard removes lane metadata. Core refuses malformed/continuation-owned items; unknown lane can be corrected by explicit valid move but attempting Done on a non-board task returns `notes_not_on_board` (`todos.rs:696-793`).

### N-TODO-05 — Adopt an un-IDed task; repair duplicate ID by explicit reference

- **Sources:** open detail `src/app/notes/NotesView.tsx:90-101`; identity/selector `src/app/notes/useNotes.ts:35-42`; adoption `src/app/notes/TodoTitle.tsx:43-72`; core resolution and mutation `crates/cockpit-core/src/notes/todos.rs:527-627, 696-820`; tests `todos.rs:1126-1201` (duplicate repair) and `1204-1239` (adoption).
- **Controls/actions:** detail button label is “Adopt task & comments” for missing ID or duplicate ID. Clicking it sends an explicit `todo_update` by ref with `text: null`; successful todo result with an ID selects that ID and opens detail. Alternatively, user edits/saves title, which adopts as part of the explicit edit. A duplicate is keyed in UI by ref rather than shared duplicate ID; ref selector targets a physical line under current whole-file revision. A mutation by ref of duplicated ID replaces only that item’s ID with fresh random 10-character base36 ID, preserving other metadata fields; a new task lacking metadata gets an ID metadata comment. `require_unique_id` and ID selector reject duplicate ID with `notes_todo_ambiguous` (“select a ref to repair it”).
- **States/guards:** detail action is disabled for malformed metadata/lazy continuation, busy, or unknown outcome. Those ownership/metadata problems require editing `todos.md` directly; Cockpit explicitly refuses to guess. Ref selector fails on changed file revision; row-level ID selector uses per-item revision and can tolerate unrelated sibling edit. Duplicate ID selection and comment attachment require uniqueness. No adoption occurs merely on list/read.
- **Invariant:** stable IDs are not assigned on passive reads; explicit edit/adopt repairs. `todoIdentity` prevents aliasing duplicate-ID rows. `comments::CommentAdd` calls `todos::require_unique_id`, preventing ambiguous task association.
- **Test refs (not run):** `todos.rs:1126-1201` covers ambiguity, ref repair and preservation of unknown metadata fields; `todos.rs:1204-1239` verifies read does not adopt and ref mutation adopts once. `notesState.test.ts:102-106` asserts duplicate rows do not alias identity/gesture.

### N-TODO-06 — Open/close task detail and removed-task recovery

- **Sources:** `src/app/notes/NotesView.tsx:90-119, 140-151`; `src/app/notes/TaskDetail.tsx:11-18, 45-82, 180-187`.
- **Controls/actions:** selected task stored in surface state; details render only while the Todos or Kanban tab is active. On opening, opener is retained. Closing the panel schedules focus return to opener if still connected, otherwise relevant add input; it avoids stealing focus if another live element owns it. Heading receives focus on detail effect. Escape closes detail unless editing/deletion mode consumes it. Removed todo displays a message that comments/drafts remain.
- **State:** details determine the task by `todo.id === selectedTodo`; if removed from the refreshed todo list, `todo` is null but detail remains eligible when selected ID persists, rendering removed-task message. Comment composer then disables because no current todo. (Adoption sets selected ID and displays detail.)
- **Evidence-backed UX issue:** `TaskDetail.tsx:11-18` initializes `loading` true and starts `comment_list` in its effect, but the displayed “Loading comments…” row lives inside `.notes-comment-thread` with `aria-busy={loading}` (`TaskDetail.tsx:45-82, 187-191`). Error is shown separately with retry. Code evidence establishes loading indicator/region; whether its visual hierarchy or announcement is adequate is not verifiable here.
- **Test ref (not run):** `NotesView.behavior.test.tsx:255-275` verifies a late detail-close frame does not steal focus from a newly selected task title.

## Durable comment surfaces

### N-COM-01 — Read/list, live refresh, count and thread navigation

- **Sources:** `src/app/notes/TaskDetail.tsx:9-10,13-82,187-207`; `src/app/notes/NotesView.tsx:62-69,145,151`; core `crates/cockpit-core/src/notes/comments.rs:24-112,149-224`; protocol `crates/cockpit-protocol/src/notes.rs:154-175,220-240`.
- **Read behavior:** detail requests `comment_list`; it polls every 3 seconds while document visible and also on window focus. Reads guard against overlap and stale component completion. Comment count updates parent. Core enumerates UUID `.md` records, validates regular files, applies per-file/body/entry/aggregate bounds, parses metadata/body, and sorts by valid timestamp then UUID; records with unknown timestamps sort after timestamped records.
- **Thread states:** loading has “Loading comments…” and `aria-busy`; empty successful read says “No comments yet” and prompts add-first-comment when task exists; with removed task it says task removed/drafts kept. Read error says “Comments unavailable” with “Retry reading.” Successful count shown; count omitted during loading/error. If newly read comments arrive while scrolled away from bottom, “N new comments below” action appears. If already at bottom, additions auto-scroll. Initial scroll restores saved per Notes/task position, defaulting to bottom.
- **Metadata:** author label shown as explicit unverified identity; absent is “Unattributed.” Timestamp formatted locally when parseable; otherwise “Time not recorded.” Expandable Record details reports unauthenticated label, raw created value or unknown, and `comments/<todoId>/<commentId>.md` path.
- **Focus/keyboard:** heading is focused on detail entry. Thread scroll position stored by scope/task. New-comments button scrolls to bottom. No automatic scroll when reader is away from bottom.

### N-COM-02 — Add comment

- **Sources:** `TaskDetail.tsx:15-16, 102-117, 206-220`; `comments.rs:129-140, 149-191, 225-281`; behavior test references `NotesView.behavior.test.tsx:168-217`.
- **Controls/actions:** Markdown composer persists draft under task ID, optional author label input (maxLength 128), Discard clears body, Comment button / composer save sends `comment_add` with trimmed author or null. Blank/whitespace body is not posted. Success clears body only if unchanged since submission, refreshes thread, scrolls to bottom and returns focus to composer only if focus still belongs to composer/thread (not hidden/inert/modal).
- **States/guards:** button disabled if task no longer exists, body blank, write busy, or outcome unknown. Draft label/body are preserved as local drafts. Button says Saving while busy. Core validates body max 64 KiB; author max 128 bytes/control chars disallowed; requires task ID unique and present; uses UUID filename and creates durable comments directory only after validation. Exceeding file/entry/aggregate bounds yields `notes_too_large`; duplicate ID yields ambiguous error. Comment add is a distinct record and does not mutate task Markdown.
- **Unknown outcome:** failed transport/no authoritative response sets persistent unknown-outcome state; drafts retained and all writes blocked pending saved-state read and explicit acknowledgment. UI warns posting again may duplicate. There is no automatic comment replay. Existing behavior tests cover this discipline (`NotesView.behavior.test.tsx:168-217, 219-253`; not run).

### N-COM-03 — Edit comment and resolve conflict

- **Sources:** `TaskDetail.tsx:11-12, 14-16, 119-141, 195-206`; `comments.rs:129-147, 283-330`; tests `comments.rs:488-497, 542-590, 649-706`.
- **Controls/actions:** Edit creates/reopens draft, focus moves to editor after render. Save comment button and Ctrl/Cmd+Enter save. Cancel / keep edit exits edit mode but keeps draft; “Resume edit · draft kept” re-enters. Discard edit removes draft. Core update uses comment UUID plus expected file revision; preserves original frontmatter exactly and changes body only.
- **Conflict/recovery:** background refresh reconciles comment drafts by ID/revision; dirty edits survive and conflict when saved revision changes. Conflict panel displays current saved version, with Keep mine and save using current comment revision or Reload (discard my edit). Missing comment with dirty draft is displayed in a “Deleted elsewhere. Your edit is kept” recovery record; “Post as new comment” creates a new record and migrates any concurrent typing onto new comment identity; discard removes retained draft. Comment update rejects stale revision (`notes_conflict`) and refuses unterminated frontmatter (`notes_invalid_input`).
- **Keyboard/focus:** Ctrl/Cmd+Enter in editor commits; Escape cancels edit but retains draft. Focus restoration targets edit action; if original comment no longer rendered, falls back to composer. Focus changes are gated on document focus, visibility/inert, modal ownership, and whether user moved focus elsewhere.

### N-COM-04 — Remove comment

- **Sources:** `TaskDetail.tsx:9-10, 88-100, 195-206`; `comments.rs:129-147, 331-350`; tests `comments.rs:498-514, 542-567`.
- **Controls/actions:** per-comment delete action opens inline confirmation. Confirmation shows “Delete comment?” unless observed revision differs, in which case it says changed elsewhere, previews current saved version, and offers “Delete this version.” Keep cancels. Delete sends `comment_remove` with observed revision, then refreshes. On success, focus goes to next comment’s edit action, else prior comment, else composer.
- **State/invariant:** deletion button disabled while busy/unknown. Delete is CAS: a comment changed since displayed revision is refused with conflict; the user must refresh/review current saved version before intentionally deleting it. A successful removal deletes only the comment record. Core comments persist independently from todo Markdown, and comments survive todo unboard/removal.
- **Test refs (not run):** `comments.rs:414-515` verifies comments survive unboard and task removal, remain readable/editable, then removable; `comments.rs:542-590` verifies stale revision refuses update/remove without changing source.

## Shared cross-cutting invariants and review notes

- **Typed operations:** selectors are either `{by: id, id, expected_revision}` or `{by: ref, ref}`; task list and board result are separate operation/result forms (`crates/cockpit-protocol/src/notes.rs:25-70, 90-152`).
- **Concurrency:** ordinary ID-based todo writes compare item revision; ref selector compares entire file revision. Comment edit/remove compare record revision. Confirmed mutation refreshes Notes model; mutation serialization is scoped by Notes UUID (`useNotes.ts:6-7, 110-148`).
- **Unknown outcome vs conflict:** authoritative `notes_*` errors remain actionable errors; transport/uncertain failures become `notes_outcome_unknown` (`useNotes.ts:21-33`). Shared banner asks for state check before acknowledgment; writing is blocked while unknown (`NotesView.tsx:152-158`, `useNotes.ts:110-121`).
- **Durability boundary:** todo text/completion/lane are represented in ordinary Markdown; comments are separate files. Deleting a todo does not delete its comments (tested in core). Detail remains capable of displaying an orphan thread when selected task disappears, but composer is disabled for absent task.
- **Deletion surface gap:** protocol/core implement `todo_remove` (`notes.rs:114-116`; `todos.rs:98,696-745`), including nested-child refusal, but no match for `todo_remove`, “Remove task,” “Delete task,” or `onRemove` was found in `src/app/notes`. Therefore this slice finds no UI control for deleting a todo; source-only scope cannot say whether another surface invokes that operation. This is an evidence-backed surface mismatch, not a recommendation.
- **Potential mismatch:** comment add requires unique todo ID, while UI offers “Adopt task & comments” for missing/duplicate IDs before opening detail. This aligns the UI affordance with backend identity requirement. Direct task removal is unavailable in this inspected UI, while durable comments and retained drafts support removed tasks created externally.

## Existing test map (not executed)

- `src/app/notes/notesState.test.ts:8-37`: draft save/reconciliation, external revision conflict, confirmed-save typing preservation.
- `src/app/notes/notesState.test.ts:38-100`: stale/captured Kanban gesture safeguards; relevant to todo state controls only as adjacent board behavior.
- `src/app/notes/notesState.test.ts:102-106`: duplicate-ID identity non-aliasing.
- `src/app/notes/NotesView.behavior.test.tsx:168-217`: unknown comment outcome requires state check/ack and avoids automatic replay.
- `src/app/notes/NotesView.behavior.test.tsx:219-253`: second uncertain write, failed read supplies no proof, remount does not carry proof.
- `src/app/notes/NotesView.behavior.test.tsx:255-275`: close-detail focus behavior.
- `crates/cockpit-core/src/notes/todos.rs:1126-1239`: duplicate ID repair and adoption without read-side mutation.
- `crates/cockpit-core/src/notes/todos.rs:1242-1295`: unknown lane and malformed metadata refusal/repair.
- `crates/cockpit-core/src/notes/todos.rs:1297-1341`: nested task removal refusal and sibling indentation preservation.
- `crates/cockpit-core/src/notes/comments.rs:414-590`: comment durability after task changes and revision CAS.
- `crates/cockpit-core/src/notes/comments.rs:593-706`: comment sort/frontmatter handling and refusal of unsafe unterminated edits.

## Coverage limit

This is the requested Todos/comment slice, not an exhaustive inventory of other Notes tabs or app-level Notes mounting/loading. React/CSS geometry, assistive technology announcements, actual persistence during a running application, and runtime focus/scroll behavior remain unverified.