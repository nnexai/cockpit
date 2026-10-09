# WS-20 Generated TypeScript response validators

Wave 3 · Size L · Depends on: WS-06, WS-12, WS-13 (the DTOs must be settled) · Blocks: WS-21

## Goal
Runtime response validation in the frontend is generated from the same Rust DTOs as the TypeScript types. Hand-written structural `isRecord`/`isString` chains disappear. Semantic checks stay hand-written: response identity matching and cross-field rules.

## Owns
- `crates/cockpit-protocol/src/typescript.rs` (and splitting `render_v1`, 474 lines)
- `src/protocol/generated/*` (generated output only)
- `src/client/CockpitClient.ts` (parsers only)
- `src/client/*Protocol.ts` (`orchestration`, `project`, `library`, `notes`, `widget`, `comment`, `quota`) and their tests

## Evidence
- `CockpitClient.ts` is ~1,900 lines of hand-written validators, e.g. `parseBrowserViewEvent` (~942), `parseBrowserViewCommandOutcome` (~1311), `parseSessionSnapshotResponse` (~1567), `parseSpaceGitActionResponse` (~1645).
- `orchestrationProtocol.ts` has ~350 more.
- The types are already generated from Rust via `ts-rs` (`=12.0.1`) and the custom exporter.

## Change
1. **Decide the mechanism and record it in your handoff.** Either emit validators from the exporter, or emit a JSON Schema per DTO plus one small in-repo validator. Do not add a heavy runtime dependency without the owner's approval.
2. Validation must be **at least as strict** as today: unknown-key rejection where it exists now, exact enums, and integer and null rules. It must still produce the `malformed_response` error codes.
3. Replace the structural parsers one protocol at a time. Keep the identity/semantic checks as small hand-written functions on top.
4. Split `render_v1` into per-module renderers.
5. Extend the existing "generated file is up to date" check to cover the validators.

## Keep
- Wire shapes.
- Error codes and messages surfaced to the UI.
- Identity matching.

## Acceptance
- No hand-written structural validator remains for a generated DTO.
- `src/client` production lines drop by at least 1,500.
- The client and protocol tests pass, and the strictness tests still reject the same malformed inputs.

## Verify
- `cargo test -p cockpit-protocol`, `bun run typecheck`, `bun run test -- src/client`.
- Browser and native smoke of startup, session switch, Library list, supervisor snapshot, notes and widgets.
