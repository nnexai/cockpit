# WEB-08 — Verify inline browser security boundaries

## Outcome

Exercise and repair only the smallest necessary gaps in the existing inline browser security boundary. Host/Origin, authentication, target/lease/view identity, frame envelope, loopback, file permissions, resource limits, and page-to-Cockpit isolation must reject hostile or stale inputs without weakening ordinary local browsing. Production adapter behavior remains read-only with respect to external resources; no new security platform is introduced.

This is the final hostile-negative campaign for issue #9 and the inline cutover, after WEB-04–07 behavior and resource contracts settle.

## Evidence and starting points

- Baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; no fresh hostile runtime proof exists in the inventory.
- Umbrella issue: https://github.com/nnexai/cockpit/issues/9.
- Read `planning/inline-space-browser-2026-09-13/01-architecture-and-transport.md` security, identity, input, blocker, and frame sections, plus delivery A12, A13, A15, A22, and A25.
- Source starts: `browser-runtime/browser-helper.mjs` `allowedFrameOrigin`, frame-server handshake/grants, `proofMatches`, `validateNavigationUrl`, envelope/size limits, `requireControl`, target transitions, and detach/cleanup.
- Source starts: `crates/cockpit-core/src/browser.rs` verified CLI/profile/CDP attachment and owner/incarnation identity; `crates/cockpit-host/src/browser_helper.rs` helper process/materialization and capability boundary.
- Source starts: `src/client/browser.ts` and `src/client/native.ts` envelope parsing, response validation, stream identity/cancellation; `src/app/browser/framePresenter.ts` descriptor/area/JPEG bounds; `src/app/browser/BrowserPane.tsx` blocker/status rendering and target/document guards.
- Source starts: `crates/cockpit-core/src/config.rs`, `browser_feedback.rs`, and `browser/delivery.rs` bounded paths, text/PNG validation, operation IDs, and file/feedback access.
- Locks and dependencies are authoritative in `planning/stability-and-gitlab-2026-09-20/tasks.json`; all overlapping browser/helper/transport writes remain serialized.

## Changes

1. Probe wrong/missing/expired credentials, hostile Origin/Host, unsolicited pre-auth bytes, replayed grants, wrong owner/incarnation/target/view/lease, and observer mutation attempts. Reject before pixels, metadata, or input.
2. Probe old lease/input sequences after takeover, stale frame/document/viewport envelopes, malformed/oversized JPEG and metadata/control floods. Keep last safe image marked stale, block location-sensitive actions, and preserve other viewers.
3. Validate loopback binding and native/browser origin/CSP/capability alignment. Ensure remote page JavaScript cannot read tickets, invoke privileged Cockpit endpoints, access CDP/profile secrets, or use file/javscript/arbitrary DevTools navigation paths.
4. Exercise file chooser/download/permission and hostile title/locator/text payloads with bounded rendering and owned destinations. Preserve explicit unsupported/deny semantics rather than broad grants.
5. Check state-root/profile/helper/file permissions and symlink/path traversal behavior for run-owned artifacts; do not broaden permissions merely to make launch pass.
6. Repair only confirmed failures, with a required read-only security review before integration. Do not introduce a new auth service, credential store, or generic remote-access mechanism.

## Non-goals

- No penetration-test marketing claim, multi-user remote-access guarantee, or replacement security architecture.
- No SSRF ban on ordinary remote HTTP(S) page browsing beyond the documented control-endpoint protections.
- No production mutation of GitLab/other external providers or user sessions.
- No reintroduction of extension pairing, raw CDP forwarding, wildcard origin policy, or legacy browser migration.

## Acceptance

1. Wrong/missing/expired credentials, hostile Origin/Host, pre-auth bytes, replayed grants, and wrong association/target/view receive bounded refusal; no frame, metadata, input, or secret leaks.
2. Observer mutation, old lease, stale input sequence, stale target/document/frame envelope, and endpoint replacement cannot affect the current controller/browser; uncertain mutations are not automatically replayed.
3. Malformed/oversized JPEG, invalid dimensions/area, metadata/control floods, hostile strings, and slow peers are bounded and isolated; other views and releases continue.
4. Loopback/private frame endpoints and native/web CSP/origin controls reject unauthorized page JavaScript and remote clients while supported local browsing still works; browser cannot read profile/CDP/tickets or invoke privileged Cockpit actions.
5. File chooser/download/permission blockers never read arbitrary paths, auto-grant, deadlock navigation, or write outside owned configured destinations; unsupported states are visible.
6. State/profile/helper/socket files use least-privilege run-owned permissions and reject traversal/symlink escape; cleanup removes only recorded resources.
7. A25 clean-cutover proof still holds: one helper per owned browser, no extension install/pairing/submit runtime, no external Show dependency, and saved feedback remains readable.

8. Include a positive control for an authorized current view after each hostile case, proving that rejection is scoped and does not break normal local browsing.
9. Check ticket placement and lifetime in logs, URLs, DOM attributes, image evidence, and error messages; none may expose a reusable view credential.
10. Exercise lost-client cleanup while keys/buttons are held and while a frame grant is active. The owner must revoke the grant and release input without relying on frontend cooperation.
11. Bound hostile title, locator, dialog text, frame bytes, and control payloads before rendering, logging, filesystem writes, or cross-process forwarding.
12. Verify the browser page cannot use loopback fetch/WebSocket to reach privileged control routes without the required origin and view authorization; ordinary remote navigation remains supported.
13. If a negative case reveals a real defect, make the smallest source repair and rerun both the negative case and its positive control. If no defect is found, preserve the evidence and do not rewrite correct code.

Security review must distinguish a source-review candidate from a reproduced runtime failure. Do not claim broad penetration-test coverage from this bounded matrix.

## Verification

The integration owner must run a hostile-negative matrix on isolated browser gateway and Linux-native Tauri sessions, with disposable origins, credentials, profiles, sockets, files, targets, and sentinels. Record request class, rejection code/state, bytes/metadata exposure, target/lease identity, file effects, process cleanup, and supported positive controls. Exercise A12, A13, A15, A22, A25 and the architecture security-negative list; source review, loopback binding, or a green build alone is not proof.

The evidence record must include:

- each hostile request class, identity/lease/origin state, response code/state, and bytes/metadata exposure;
- malformed frame/metadata and flood limits, blocker/file effects, loopback/CSP result, and positive authorized control;
- state-root/profile/helper permission and traversal checks with cleanup ownership;
- browser/native process, socket, target, credential, and sentinel cleanup.

Do not mark a security boundary verified from loopback binding or source inspection alone; every claimed protection needs a bounded negative and a supported positive control.

## Handoff

Return changed paths, each confirmed vulnerability and smallest repair, negative/positive evidence, and unresolved risks. The integration owner must attach `runs/<run-id>/WEB-08.md` with durable artifacts and land a real commit after security review. Keep all listed lock groups serialized, update no status fields here, and follow `../ORCHESTRATOR.md` for resource and evidence discipline.
