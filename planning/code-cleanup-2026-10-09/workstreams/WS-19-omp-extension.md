# WS-19 OMP extension split

Wave 2 · Size M · Depends on: WS-05 · Blocks: –

## Goal
The per-process OMP integration is a few small modules instead of one 520-line closure.

## Owns
- `integrations/omp/cockpit-orchestration.ts` (+ test) and new sibling modules
- `scripts/install-native.py` and `scripts/test_install_native.py`, if packaging must change

## Evidence
`cockpitOrchestration` (`~569-1088`) holds the caller identity, the CLI call wrapper, the wake state, the control loop and the retirement handler in one closure.

## Change
1. Split into `identity`, `cliCall`, `wake`, `controlLoop` and `retirement` modules. The exported entry point wires them together.
2. **First check how the extension is installed.** If the installer copies a single file (receipt-owned installation), keep a single installed artifact by bundling, or update the installer and its test to install the module directory under the same receipt rules.

## Keep
- The fresh per-tool prepare gate.
- Counts-only wake.
- Explicit processed ACK.
- Process/PID evidence on every CLI call.
- No global OMP mutation.

## Acceptance
- No function over 150 lines.
- The installer test passes.
- An installed extension loads in OMP.

## Verify
- `bun run test -- integrations/omp`; `python3 scripts/test_install_native.py`.
- Disposable fixture: a worker launched with the installed extension receives a wake, ACKs it, and reports.
