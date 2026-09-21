# WEB-01 — preserve browser draft lifecycle

## Accepted plan

Accepted by Main, 2026-09-21. Baseline `6593cf4ecba87687c809ee9f3d2a295c757db787` plus returned SYNC-01 and WEB-07 implementation. Their acceptance remains pending; source interfaces are settled for this writing handoff under OBS-011. Original WEB-01 criteria are not reduced.

Observable outcome: Hide releases presentation/input resources without destroying page/draft/editor identity; Show resumes the same live association. Explicit shared-browser Close persists unsent editor state or refuses safely, while saved feedback and exact pending capture recovery remain available. Geometry changes make prior marks stale/recoverable, not deleted or painted onto replacement pages.

### Evidence and decomposition

App currently unmounts BrowserPane when browserVisible becomes false. BrowserPane resets draft/note/pending state on stream rebind and document/target/navigation transitions. BrowserDraftStore::retire_obsolete clears annotations and tombstones every previous identity. Existing durable editor state stores only selected_annotation_id/notes_open, not unsent note text. These concrete boundaries motivate frontend and persistence slices; no new generic store or migration project.

- Frontend worker exclusively owns src/app/browser/BrowserPane.tsx, its focused tests, and App.tsx browser presentation/action regions plus necessary browser integration tests. Preserve returned SYNC-01 cleanup/generation checks. No terminal/session/source code or backend/protocol writes.
- Persistence worker owns crates/cockpit-core/src/browser/drafts.rs, narrowly necessary existing browser feedback/delivery/close integration, and the browser-view protocol definition/handwritten browser validators and constructor migrations. Preserve returned WEB-07 launch changes and existing owner forwarding. No App/BrowserPane/helper/terminal/source/generated edits.
- Main owns generated output and shared integration. TERM-03 cannot edit App.tsx. GLAB-01 edits only source protocol; browser protocol files are disjoint.

### Fixed interface and invariants

1. Extend BrowserViewDraftEditorState with `note_annotation_id: Option<String>` and `note_text: String`, using serde defaults for old durable records and existing optional wire conventions. Empty/no active note is None/empty string. Bound note text consistently with the existing 4000-code-unit editor; validate annotation membership without discarding failed editor text. This extends the existing durable draft, not a new persistence model.
2. BrowserPane gains optional `registerCloseGuard?: (guard: (() => Promise<void>) | null) => void`. App registers it for the exact active session/Space and awaits it before a shared close request. Rejection keeps the association/view and recoverable work, with a real actionable error. Guard removal/late completion cannot authorize closing a replacement association. Presentation Hide never calls shared Close.
3. Frontend retains scoped local dirty draft/editor/pending operation identity until its matching durable acknowledgement. Hide/show must not reset an unchanged draft or increment its revision unnecessarily. Retain the hidden selected-association component, without frame/input resources; do not keep extra live streams/profiles. Do not let later/older save responses overwrite a newer local revision or acknowledge newer text. Serialize existing draft mutations and reconcile exact IDs rather than retrying uncertain capture/navigation automatically.
4. Store retires old geometry by marking it stale while preserving annotations and editor text. Only explicit discard may destroy unsent work; empty unreferenced records may be compacted safely. Preserve the existing bounded draft/capture capacities and surface capacity/recovery rather than evicting dirty work. Stale draft recovery is explicit/read-only until user review; never transplant old marks to a new document by URL/selector guess.
5. Draft read/update/recovery remains scoped to the authenticated association and original browser incarnation/target/document. Listing/recovering old work may cross a now-gone live document only through validated durable identity, never arbitrary draft IDs from another association. Live geometry/capture/input keeps current document/viewport gates; persisting already-owned editor text must not need fresh pixels. Backend may narrowly separate durable editor/list/retry operations from live-document checks where required, without widening live input or browser ownership.
6. Pending capture inventory/retry identifies its original association/incarnation even if its target has gone; retry reuses existing frozen ID/bytes. Saved-feedback lookup/ack must work with the browser closed. Explicit close uses existing authorized owner-forwarding; closing an observer presentation cannot stop the owner browser, and authorized forwarded shared close must not be prohibited merely because the caller is not input controller.
7. Navigation/reload/target switch/helper reconnect/resize cancel unfinished gestures only. Retain earlier marks and unsent notes for explicit recovery; never paint stale geometry. Escape/cancel affects only the active note edit. Failed persistence/close leaves recoverable state, not a warning followed by loss. Preserve Herdr hierarchy/focus authority and existing lifecycle error distinctions.

### Verification

Workers skip all tests/builds/formatters/runtime/services/commits during concurrent writing. Add only meaningful lifecycle/durability regressions. Main integrates once and delegates bounded required checks; final original gateway/native hide/show, pending-save, true-close, stale recovery, observer/owner, saved-feedback, exact retry byte identity, and Herdr parity criteria remain required. Use only the recorded disposable resources and no legacy import/migration. This plan is not acceptance or a commit claim.

## Corrected outgoing-work boundary

Source review demonstrated that retaining a guard closure is insufficient: queued editor saves read mutable current-association refs, and a second unmount can overwrite the single outgoing slot. Main accepts this narrow correction on 2026-09-21:

- Each draft association owns its target, draft revision, editor generation/bytes, pending annotation intents and serialized acknowledgement state. Queued work captures that owner, not mutable refs belonging to a replacement pane. Late acknowledgements update only their original owner; UI updates additionally require the current lifetime.
- App retains one sealed outgoing owner above Workbench epoch replacement. Authoritative Herdr navigation remains immediate. Do not overwrite unresolved outgoing work; keep a visible labelled recovery state and prevent creating a second dirty browser owner until the first is acknowledged or explicitly discarded. No hidden live stream/profile cache.
- Extend the existing authenticated durable recovery action with narrowly scoped annotation upsert/remove, alongside set_editor, using exact original draft ownership, tombstone checks, validated payloads and expected-revision CAS. These operations persist draft metadata only; they do not grant live input, pixels, inspection evidence freshness, capture or delivery authority. This closes the demonstrated inability to save an already-created annotation after presentation teardown.
- A lost acknowledgement is reconciled against exact stored content/revision, never assumed successful or blindly overwritten. Unresolved writes remain visible and retryable; explicit discard is the only destructive resolution. Existing frozen capture bytes/IDs and newer-edit preservation remain unchanged.

No new generic cache, transport or persistence model is authorized. Original acceptance remains pending.
