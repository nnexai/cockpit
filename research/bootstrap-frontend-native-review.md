# Bootstrap frontend and native review

This review covers the bootstrap status operation only. It does not expand into the deferred Herdr mirror, terminal, provider, or workspace work.

## Findings

### 1. [MEDIUM] Runtime validation accepts malformed generated DTOs

- **Evidence:** `src/client/CockpitClient.ts:48-54` accepts any JavaScript `number` for the generated Rust `u32` fields. It does not require those values to be finite, integer, or non-negative. `src/client/CockpitClient.ts:87-92` also treats a missing `herdr.identity` property as valid for the `incompatible` variant, although the generated type at `src/protocol/generated/v1.ts:7` requires the property and permits `null`, not omission.
- **Failure:** A successful transport response such as an incompatible status with no `identity`, or with `protocol: 20.5`, crosses the adapter seam and is rendered as a valid status. That violates the exact generated DTO and the plan's requirement that malformed responses become typed client errors (`BOOTSTRAP_PLAN.md:124-131`, `198-201`).
- **Smallest correction:** Require `identity` to be an own property and accept only `null` or a valid identity. Validate `protocol` and `schema_version` with finite, integer, non-negative `u32` bounds. Add adapter cases for omitted `identity`, fractional values, and negative values.

### 2. [MEDIUM] The native capability grants unused core APIs

- **Evidence:** `src-tauri/capabilities/default.json:7-10` grants `core:app:default`, `core:event:default`, and `core:window:default`. The bootstrap frontend only invokes the custom command (`src/client/native.ts:25-29`) and uses host detection (`src/client/select.ts:1,27`); it does not use app metadata, event listeners, or window controls.
- **Failure:** A compromised or accidentally expanded WebView receives more Tauri core access than the current operation needs. This conflicts with the plan's requirement to grant only required core defaults and no broad permissions (`BOOTSTRAP_PLAN.md:213-218`).
- **Smallest correction:** Remove the three unused `core:*:default` entries and retain `allow-cockpit-status`. Add a narrowly scoped core permission only with the code that demonstrably needs it.

### 3. [LOW] Font rendering is not fixed across browser and native builds

- **Evidence:** `src/app/styles.css:1-3` and `188-191` name IBM Plex and Noto fonts but provide no bundled font files or `@font-face` declarations. The UI design direction explicitly requires bundled fonts so native and browser builds match (`research/ui-design-direction.md:159-166`, `403-407`).
- **Failure:** The status screen falls back to whichever fonts happen to be installed in the browser or WebKitGTK environment, so the two clients can have different metrics and wrapping. This is a visual browser/native mismatch, especially for long compatibility messages.
- **Smallest correction:** Bundle the selected IBM Plex Sans and IBM Plex Mono files in the frontend output and define local `@font-face` rules, keeping the documented fallbacks.

## Non-findings

The UI renders transport data as React text, so status and error strings are not inserted as HTML. It uses one `CockpitClient` seam, selects the adapter once, makes one status request from the effect, and the browser and native adapters both run the same runtime parser. The status screen exposes mode, Cockpit protocol/version, and all Herdr compatibility fields without inventing Spaces, agents, panes, terminals, or disabled controls. Tauri's command manifest, named `main` target, explicit `default` capability selection, `frontendDist: "../dist"`, and absence of shell, process, filesystem, remote-origin, and plugin network permissions match the bootstrap plan.
