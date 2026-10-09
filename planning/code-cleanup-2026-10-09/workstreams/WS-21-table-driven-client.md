# WS-21 Table-driven frontend client

Wave 3 · Size M · Depends on: WS-20 · Blocks: –

## Goal
Each `CockpitClient` operation is declared once. The HTTP adapter (browser) and the Tauri adapter (native) are thin interpreters of that table.

## Owns
- `src/client/{browser,native}.ts`
- a new `src/client/operations.ts`
- `src/client/client.test.ts`

## Evidence
- `createBrowserClient` (521 lines, `browser.ts:~729`) and `createNativeClient` (468 lines, `native.ts:~690`) each implement ~80 methods.
- The POST-JSON `getJson(...)` call is repeated about 15 times (`browser.ts:~741-816`). The `invokeAndParse(...)` calls have identical shapes (`native.ts:~706-713`).
- The stream code is parallel: `openBrowserViewStream` (185 lines) and `nativeBrowserViewSubscription` (194 lines).

## Change
1. Add `operations.ts`: a typed table `{name, http: {method, route}, tauri: command, encode, parse, matchIdentity}` built from the WS-20 validators.
2. Implement the `CockpitClient` methods in each adapter by walking the table. Error-shape mapping stays per adapter.
3. Extract a shared stream decoder (ordering, generation, stale/disconnected frames) used by both browser-view stream implementations. Only the socket/channel plumbing differs.

## Keep
- Route paths, HTTP methods and Tauri command names: **no wire change**. WS-22 relies on this.
- Error codes.
- Stream ordering guarantees.

## Acceptance
- `browser.ts` and `native.ts` together drop by at least 50%.
- Adding an operation means one table row plus the backend handler.

## Verify
- `bun run typecheck`, `bun run test -- src/client`.
- Browser and native smoke: startup, terminal attach, Library list, browser-view stream, supervisor action.
