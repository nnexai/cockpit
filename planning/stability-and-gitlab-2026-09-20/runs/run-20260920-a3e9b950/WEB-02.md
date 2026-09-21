# WEB-02 — first gesture and read-only element sample

## Accepted implementation plan

Accepted by Main, 2026-09-21 under OBS-011. Original WEB-02 acceptance and ledger completion dependencies remain unchanged. Code may advance only across disjoint settled boundaries; WEB-01 retains frontend ownership until its outgoing-draft repair returns.

Concrete current defects: BrowserPane.command silently drops stale pointer/wheel and stale-input-sequence outcomes; remotePointer maps coordinates before takeover but later uses an arbitrary current location; inspect refuses every request until cursor metadata exists. Helper proofMatches reads only top-level location/context, not nested inspection proof, and inspection returns main-document DOM evidence without a post-await identity check or explicit inaccessible-content boundary.

### Ownership and contract

- Helper worker owns browser-runtime/browser-helper.mjs only: preserve WEB-07 launch/dispatcher fixes; repair first-control/inspection authority and read-only inspection. No frontend/store/protocol writes. Commands remain one ordered path; no replay of dispatched uncertain input.
- Main owns the narrow protocol integration after WEB-01's protocol writer hands off: inspect request and result pointer_sample_sequence become nullable. Null explicitly identifies a valid local read-only pointer sample bound to the supplied current frame/location, not invented cursor metadata. A numeric sample remains checked against real cursor identity. Do not move the remote pointer, alter held buttons, acquire control, or fabricate cursor events merely to inspect.
- Validate nested target/document/viewport/frame geometry and bounded coordinates before inspection and revalidate after asynchronous DOM work. Ordinary supported DOM produces actual bounded evidence; inaccessible frame/shadow/canvas content gets an explicit limitation. Never claim local anchors or infer evidence from JPEG coordinates.
- Frontend ownership follows WEB-01 handoff. Retain one original gesture transaction through confirmed control using original target/document/viewport/geometry and fresh lease; a newer frame sequence alone does not discard it. Changed identity/geometry cancels visibly. Capture immutable event data, preserve ordered down/up/cancel, release on loss, and classify stale/refused/cancelled-before-dispatch/outcome-unknown without replay. Read-only Element intent may wait for its matching readiness barrier, then inspect using the explicit local sample; stale replies cannot annotate another document.
- Main generates types and migrates affected protocol constructors/validators once writers settle. Existing response classifications are reused; no generic retry or transport redesign.

### Verification

Writing workers skip formatters, linters, builds, tests and runtime. Required ownership review and exact first-click/first-Element gateway/native fixtures, negative outcomes, pointer release and TUI handoff remain for final verification. No acceptance, commit or surface proof claimed here.
