# WEB-03 — frame-bound geometry and resize transitions

## Accepted plan

Main/Astra accepts the WEB-03 implementation increment on 2026-09-21 against `8fe09231093ac2ce017017d3a254f321c211b502`, preserving the other campaign worktree changes. TERM-01 is complete. All original WEB-03 changes and acceptance criteria remain required; long browser/Linux-native rounds stay deferred under OBS-011.

Inspected the task brief, architecture identity/JPEG/geometry/resize sections, helper viewport/capture/credit producers, shared transform and presenter decode path. Concrete starting defects:

- `viewportState` reports fixed zero visual offsets, unit scales and fresh geometry rather than confirmed values.
- `enqueueFrame` checks but ignores capture-time scroll metadata, then labels pixels with current mutable state. Capture callbacks can therefore claim newer geometry than their pixels.
- Requested capture dimensions and measured page geometry are conflated. Screencast uses requested CSS dimensions as pixel bounds without recorded DPR semantics.
- Presenter validation occurs before asynchronous decode; its decoded image/object URL is not released on every subsequent validation/presentation failure.

## Contract and ownership

Keep the existing public Rust/TypeScript DTO shapes and IBFV v2 framing. Public coordinate spaces remain Cockpit client CSS, image pixels, Chromium viewport CSS and document CSS. Requested controller surface dimensions are distinct private state from measured visible CSS geometry. Snapshot and frame geometry must describe confirmed values; offsets/scales must not be fabricated. If existing DTOs cannot represent a proved case, pause that boundary for Main rather than inventing an implicit interpretation.

The helper owns capture dimensions, transition generation and CDP-specific unit conversion. A frame descriptor binds to an immutable capture session/baseline; delayed packets are not retagged with current state. Capture metadata must agree with that baseline, or be discarded with original-session credit released and a coalesced geometry reconciliation. Keep page binding, document/frame generation, viewport revision and stream epoch distinct. Retiring capture must never ACK through a replacement CDP connection. Do not introduce polling screenshots or treat static image silence as failure.

The existing `createBrowserTransform` remains the shared client mapping. It must not add host-DPR multiplication or duplicate geometry formulas in consumers. Confirm CSS/DIP/page-scale/visual-offset meanings before changing them. Layout document scroll and visual viewport offset must not be added twice.

Two independent writing slices:

1. Helper geometry owner: `browser-runtime/browser-helper.mjs`, only viewport/capture/rebind/geometry-related guards. Preserve WEB-02 nullable local inspection intent and all unrelated WEB-07 prerequisite work. A single bounded, owned Chromium/CDP prerequisite experiment may establish actual CSS/DPR/page-scale/screencast/input meanings; this is not campaign UI/native acceptance. No project build/test suite or service/Herdr mutation.
2. Presenter owner: `src/app/browser/framePresenter.ts`, `transform.ts` and meaningful existing focused tests if their contract must change. Preserve public DTOs and callback API, revalidate after asynchronous decode before publishing, and close/revoke owned decoded resources on all transition/error paths. No helper, protocol, client transport or BrowserPane edits.

Main owns the later shared BrowserPane integration: pending requested viewport reconciliation on takeover/reconnect, matching-frame eligibility and consumer use of the shared transform. BrowserSurfaceIntegration remains the only current App/BrowserPane writer until its WEB-01/02 handoff. Do not overlap or assume its moving implementation. Main will integrate this boundary after that handoff.

## Verification and queued boundaries

Workers skip formatters, linters, compiler, project tests and runtime acceptance. After integration, a bounded static check and required geometry/capture review precede final browser and Linux-native proof. Existing tests are retained only for meaningful geometry/identity/transition contracts. New unit tests are justified only for an uncertain race/boundary, not to count wiring.

Final scenarios retain original WEB-03 criteria: 800×600 to materially different accepted size; latest resize during takeover/reconnect; requested CSS versus delivered JPEG versus painted client rectangle; controlled host zoom/DPR, page zoom, scroll, sticky/fixed content and letterbox with measured hit/export error ≤2 displayed CSS pixels; two-client resize authority; navigation/reload/target races; pinned capture identity; retired-credit cleanup; resize during drag/wheel/Element/capture; monotonic metadata and visibly stale blocked input.

Keep raw descriptor values, exact build/client identity, authoritative responses, painted geometry and owned cleanup. macOS-specific execution remains user verification—not executed per OBS-013; browser and Linux-native criteria are not waived.

The independently observed pre-decode JPEG dimension-budget and wider queue/lifecycle issues remain queued for WEB-06/WEB-08, not silently accepted by this geometry slice. No completion or runtime claim from this plan or source inspection.
