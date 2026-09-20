# TERM-01 — Accept compatible Herdr patch releases

## Outcome
Cockpit accepts a Herdr installation when the wire protocol, schema, and advertised capabilities satisfy the supported contract, including compatible patch releases. Compatibility must not reject a supported Herdr solely because its human-readable version differs at the patch level. Unsupported protocol/schema/capability combinations remain explicit, actionable incompatibilities.

This is an adapter/status contract change, not a promise to support arbitrary Herdr versions. Preserve the status response shape and the distinction between compatible, unavailable, and incompatible states.

## Evidence and starting points
- Baseline is `6f6222b74e4f552ce697e61364cf653f4b6be29f`.
- Ledger dependencies and ownership are authoritative in [../tasks.json](../tasks.json); this task depends on RUN-01 and owns `herdr-adapter`.
- Issue anchor: [GitHub #13](https://github.com/nnexai/cockpit/issues/13). Treat it as a compatibility report, not permission to add an exact patch-version allowlist.
- `crates/cockpit-core/src/lib.rs` defines `HerdrAdapter`, compatibility caching, status construction, and the installation/session inspection boundary.
- `src/client/CockpitClient.ts` parses `StatusResponse`, `HerdrIdentity`, and capabilities; `src/app/App.tsx` renders compatibility and unavailable states.
- `crates/cockpit-core/tests/status.rs` and `crates/cockpit-host/tests/server.rs` provide existing compatible/unavailable fixtures.
- The protocol authority is the generated v1 schema and the current Herdr inspection response, not stale historical graphics or patch-version prose.

## Changes
- Trace the current status request from adapter inspection through Rust compatibility classification, JSON response, generated TypeScript parsing, and the App gate.
- Define the supported compatibility predicate in protocol/schema/capability terms: required protocol range, schema version, and capability requirements must be explicit and stable.
- Accept a later compatible patch release when those fields remain supported; do not compare exact semantic patch strings as the compatibility rule.
- Keep unknown protocol, schema, or required capability failures incompatible with a reason and machine-readable code.
- Preserve capability-gated behavior such as terminal mouse input; absence must disable only the dependent feature rather than falsely claiming full support.
- Update only the adapter/status contract and its directly affected callers or generated contract artifacts. Do not broaden to browser CLI compatibility (#9 finding 6) or unrelated provider checks.
- Add the required review type: protocol/compatibility review by the integration owner before runtime proof.

## Non-goals
- No exact `0.9.1` (or any other patch) allowlist.
- No restoration of old terminal graphics, old native scale defaults, or legacy browser migration.
- No server-side feature invention when Herdr does not advertise a capability.
- No claim that issue #13 is currently reproduced until a real compatible and incompatible installation is exercised.

## Acceptance
1. A supported Herdr patch release with the same supported protocol/schema and required capabilities reaches the normal session UI instead of the compatibility error.
2. A protocol mismatch, schema mismatch, and missing required capability each produce a clear incompatible status; the App does not start session work for any of them.
3. An unavailable adapter remains an unavailable state with its actionable code/message, distinct from incompatibility.
4. Existing capability-dependent behavior remains honest: terminal mouse input is enabled only when advertised, and no unrelated capability is silently inferred.
5. Browser and native status paths show the same classification and message identity.
6. The TUI oracle confirms the Herdr session remains authoritative for the accepted status; no compatibility decision is based on Cockpit's cached presentation.

## Verification
The integration owner should run the status through the actual browser and native launch surfaces using a disposable named Herdr session and isolated configuration. Exercise one compatible later patch response, each incompatible protocol/schema/capability response, and unavailable Herdr. Capture the raw status fields and rendered state, then compare with the TUI's own protocol/schema/capability report. Existing closed #1/#2/#4 are regression anchors only, not current-failure claims. Static tests may guard the predicate, but they do not replace this runtime proof.

## Handoff
Record the adapter response, protocol/schema/capability values, browser/native screenshots or compact captures, TUI oracle identity, and cleanup in the run evidence. Link the durable evidence path from the ledger. The worker must provide a real commit containing only this task's contract changes; do not mark the task complete from a source review or a passing fixture-only test.
- Before changing the predicate, capture the current status response and identify which field currently causes a supported patch release to fail.
- Keep the adapter's cached installation/session generations coherent when compatibility is rechecked during a running session.
- Check that a session-specific incompatibility cannot be mistaken for installation compatibility, and that a stale status cannot reopen session work.
- Exercise malformed status JSON separately from a well-formed incompatible response; malformed data must remain a transport/schema error.
- Record the exact required capability set and the fallback behavior for optional capabilities in the task evidence.
- Verify retrying the compatibility notice performs a fresh inspection and does not reuse a stale incompatible cache entry.
- Use the generated protocol type as the shared boundary; do not hand-maintain a second status schema in the browser.
- Confirm no source path still compares the Herdr display version as an exact patch string after the repair.
- If a compatibility issue is found in a browser-only prerequisite such as Playwright configuration, leave it for its own task rather than folding it into this predicate.
- A passing unit fixture is useful for the boundary but cannot prove a live Herdr patch release is accepted.

The evidence should distinguish:
- repaired code paths (the predicate, cache invalidation, and status rendering);
- behavior observed live (raw Herdr identity/capabilities and browser/native outcome);
- behavior still unverified (any platform or capability not exercised by the fixture).

Do not close issue #13 or claim broad version support from a single compatible response.

- Include the supported and rejected identities in compact evidence so a later reader can reproduce the decision without a session-local artifact.
- Preserve the exact issue URL and note whether the observed failure was repaired or only source-reviewed.
- Record cleanup of any fake status endpoint, adapter fixture, process, or disposable Herdr session.
- The handoff must name the integration owner responsible for the final browser/native gate.
- A missing compatible patch fixture blocks the live criterion; it does not justify an exact-version fallback.

- Keep the evidence compact enough to review alongside the status contract and ledger entry.
- Do not substitute a mocked identity for the real Herdr compatibility report.
